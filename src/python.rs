use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use pyo3::exceptions::{PyFileNotFoundError, PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyBytes, PyDict, PyList, PyTuple};

use crate::async_engine::PdfEngine;
use crate::config::{HeaderMode, Margins, TableStrategy, ToMarkdownOptions};
use crate::error::PdfError;
use crate::markdown::chunk::PageChunk;
use crate::markdown::renderer::strip_markdown_syntax;
use crate::pdf::{DocumentMetadata, PdfDocument};
use crate::MarkdownOutput;

const DEFAULT_MAX_PDF_BYTES: usize = 128 * 1024 * 1024; // 128 MB per PDF guard

fn shared_tokio_runtime() -> std::result::Result<&'static tokio::runtime::Runtime, String> {
    static RT: OnceLock<std::result::Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    let res = RT.get_or_init(|| {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(cpus.clamp(2, 8))
            .max_blocking_threads((cpus * 4).max(16))
            .thread_name("pymupdf4llm-worker")
            .enable_all()
            .build()
            .map_err(|e| format!("Failed to initialize Tokio runtime: {e}"))
    });
    res.as_ref().map_err(|e| e.clone())
}

fn pdf_err_to_py(err: PdfError) -> PyErr {
    match err {
        PdfError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => {
            PyFileNotFoundError::new_err(e.to_string())
        }
        PdfError::PageOutOfBounds { .. } | PdfError::InvalidHeader => {
            PyValueError::new_err(err.to_string())
        }
        other => PyRuntimeError::new_err(other.to_string()),
    }
}

fn check_file_size(path: &Path, max_pdf_bytes: usize) -> PyResult<()> {
    let meta = std::fs::metadata(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            PyFileNotFoundError::new_err(format!("PDF file not found: {}", path.display()))
        } else {
            PyRuntimeError::new_err(format!("Failed to stat PDF file {}: {e}", path.display()))
        }
    })?;
    if meta.len() > max_pdf_bytes as u64 {
        return Err(PyValueError::new_err(format!(
            "PDF file '{}' size ({} bytes) exceeds max_pdf_bytes ({max_pdf_bytes} bytes)",
            path.display(),
            meta.len()
        )));
    }
    Ok(())
}

fn check_page_limit(doc: &PdfDocument, max_pages: Option<usize>) -> crate::Result<()> {
    if let Some(limit) = max_pages {
        let total = doc.page_count();
        if total > limit {
            return Err(PdfError::PageOutOfBounds {
                index: limit,
                total,
            });
        }
    }
    Ok(())
}

fn extract_pdf_bytes(obj: &Bound<'_, PyAny>, max_pdf_bytes: usize) -> PyResult<Vec<u8>> {
    if let Ok(b) = obj.downcast::<PyBytes>() {
        let slice = b.as_bytes();
        if slice.len() > max_pdf_bytes {
            return Err(PyValueError::new_err(format!(
                "PDF payload ({} bytes) exceeds max_pdf_bytes ({max_pdf_bytes} bytes)",
                slice.len()
            )));
        }
        return Ok(slice.to_vec());
    }
    if let Ok(ba) = obj.downcast::<PyByteArray>() {
        let len = ba.len();
        if len > max_pdf_bytes {
            return Err(PyValueError::new_err(format!(
                "PDF payload ({len} bytes) exceeds max_pdf_bytes ({max_pdf_bytes} bytes)"
            )));
        }
        return Ok(ba.to_vec());
    }
    if obj.hasattr("tobytes")? {
        if let Ok(len) = obj.len() {
            if len > max_pdf_bytes {
                return Err(PyValueError::new_err(format!(
                    "PDF payload ({len} bytes) exceeds max_pdf_bytes ({max_pdf_bytes} bytes)"
                )));
            }
        }
        let bytes_obj = obj.call_method0("tobytes")?;
        if let Ok(b) = bytes_obj.downcast::<PyBytes>() {
            let slice = b.as_bytes();
            if slice.len() > max_pdf_bytes {
                return Err(PyValueError::new_err(format!(
                    "PDF payload ({} bytes) exceeds max_pdf_bytes ({max_pdf_bytes} bytes)",
                    slice.len()
                )));
            }
            return Ok(slice.to_vec());
        }
    }
    let vec = obj.extract::<Vec<u8>>()?;
    if vec.len() > max_pdf_bytes {
        return Err(PyValueError::new_err(format!(
            "PDF payload ({} bytes) exceeds max_pdf_bytes ({max_pdf_bytes} bytes)",
            vec.len()
        )));
    }
    Ok(vec)
}

fn parse_margins(val: &Bound<'_, PyAny>) -> PyResult<Margins> {
    if let Ok(scalar) = val.extract::<f32>() {
        if !scalar.is_finite() || scalar < 0.0 {
            return Err(PyValueError::new_err("margins scalar must be finite and >= 0"));
        }
        return Ok(Margins::from(scalar));
    }
    if let Ok((h, v)) = val.extract::<(f32, f32)>() {
        if !h.is_finite() || !v.is_finite() || h < 0.0 || v < 0.0 {
            return Err(PyValueError::new_err("margins values must be finite and >= 0"));
        }
        return Ok(Margins::from((h, v)));
    }
    if let Ok((l, t, r, b)) = val.extract::<(f32, f32, f32, f32)>() {
        if ![l, t, r, b].iter().all(|x| x.is_finite() && *x >= 0.0) {
            return Err(PyValueError::new_err("margins values must be finite and >= 0"));
        }
        return Ok(Margins::from((l, t, r, b)));
    }
    Err(PyTypeError::new_err(
        "margins must be a float, a 2-tuple (h, v), or a 4-tuple (left, top, right, bottom)",
    ))
}

fn parse_table_strategy(s: &str) -> PyResult<TableStrategy> {
    match s.to_ascii_lowercase().as_str() {
        "lines_strict" => Ok(TableStrategy::LinesStrict),
        "lines" => Ok(TableStrategy::Lines),
        "text" => Ok(TableStrategy::Text),
        "explicit" => Ok(TableStrategy::Explicit),
        other => Err(PyValueError::new_err(format!(
            "Unknown table_strategy '{other}'. Expected 'lines_strict', 'lines', 'text', or 'explicit'"
        ))),
    }
}

fn table_strategy_str(ts: TableStrategy) -> &'static str {
    match ts {
        TableStrategy::LinesStrict => "lines_strict",
        TableStrategy::Lines => "lines",
        TableStrategy::Text => "text",
        TableStrategy::Explicit => "explicit",
    }
}

struct ParsedOptions {
    opts: ToMarkdownOptions,
    max_docs: Option<usize>,
    max_pages_concurrency: Option<usize>,
    max_pdf_bytes: Option<usize>,
    max_pages: Option<Option<usize>>,
}

fn apply_option_kwargs(
    base: &ToMarkdownOptions,
    kwargs: Option<&Bound<'_, PyDict>>,
    allow_engine_keys: bool,
) -> PyResult<ParsedOptions> {
    let mut opts = base.clone();
    let mut max_docs = None;
    let mut max_pages_concurrency = None;
    let mut max_pdf_bytes = None;
    let mut max_pages = None;

    let Some(dict) = kwargs else {
        return Ok(ParsedOptions {
            opts,
            max_docs,
            max_pages_concurrency,
            max_pdf_bytes,
            max_pages,
        });
    };

    let mut hdr_mode_name: Option<String> = None;
    let mut hdr_max_levels: Option<usize> = None;
    let mut hdr_body_limit: Option<Option<f32>> = None;

    for (k_any, v) in dict.iter() {
        let key: String = k_any.extract()?;
        match key.as_str() {
            "pages" => {
                opts.pages = v.extract::<Option<Vec<usize>>>()?;
            }
            "hdr_info" => {
                hdr_mode_name = v.extract::<Option<String>>()?;
                if hdr_mode_name.is_none() {
                    opts.hdr_info = HeaderMode::Disabled;
                }
            }
            "header_max_levels" => {
                let lv: usize = v.extract()?;
                if lv == 0 || lv > 6 {
                    return Err(PyValueError::new_err("header_max_levels must be in 1..=6"));
                }
                hdr_max_levels = Some(lv);
            }
            "header_body_limit" => {
                hdr_body_limit = Some(v.extract::<Option<f32>>()?);
            }
            "write_images" => opts.write_images = v.extract()?,
            "embed_images" => opts.embed_images = v.extract()?,
            "dpi" => {
                let dpi: u32 = v.extract()?;
                if dpi == 0 {
                    return Err(PyValueError::new_err("dpi must be > 0"));
                }
                opts.dpi = dpi;
            }
            "image_path" => {
                let p: PathBuf = v.extract()?;
                opts.image_path = p;
            }
            "image_size_limit" => opts.image_size_limit = v.extract()?,
            "force_text" => opts.force_text = v.extract()?,
            "margins" => opts.margins = parse_margins(&v)?,
            "page_chunks" => opts.page_chunks = v.extract()?,
            "page_separators" => opts.page_separators = v.extract()?,
            "page_width" => {
                let pw: f32 = v.extract()?;
                if !pw.is_finite() || pw <= 0.0 {
                    return Err(PyValueError::new_err("page_width must be finite and > 0"));
                }
                opts.page_width = pw;
            }
            "page_height" => opts.page_height = v.extract()?,
            "table_strategy" => {
                let s: String = v.extract()?;
                opts.table_strategy = parse_table_strategy(&s)?;
            }
            "graphics_limit" => opts.graphics_limit = v.extract()?,
            "ignore_code" => opts.ignore_code = v.extract()?,
            "ignore_images" => opts.ignore_images = v.extract()?,
            "ignore_graphics" => opts.ignore_graphics = v.extract()?,
            "extract_words" => opts.extract_words = v.extract()?,
            "fontsize_limit" => {
                let fl: f32 = v.extract()?;
                if !fl.is_finite() || fl < 0.0 {
                    return Err(PyValueError::new_err("fontsize_limit must be finite and >= 0"));
                }
                opts.fontsize_limit = fl;
            }
            "use_glyphs" => opts.use_glyphs = v.extract()?,
            "detect_bg_color" => opts.detect_bg_color = v.extract()?,
            "ignore_alpha" => opts.ignore_alpha = v.extract()?,
            "header" => opts.header = v.extract()?,
            "footer" => opts.footer = v.extract()?,
            "clean_mojibake" => opts.hygiene.clean_mojibake = v.extract()?,
            "dehyphenate" => opts.hygiene.dehyphenate = v.extract()?,
            "dedup_faux_bold" => opts.hygiene.dedup_faux_bold = v.extract()?,
            "ignore_invisible_text" => opts.hygiene.ignore_invisible_text = v.extract()?,
            "strip_watermarks" => opts.hygiene.strip_watermarks = v.extract()?,
            "dedup_running_headers" => opts.hygiene.dedup_running_headers = v.extract()?,
            "max_concurrent_docs" if allow_engine_keys => {
                if let Some(n) = v.extract::<Option<usize>>()? {
                    if n == 0 {
                        return Err(PyValueError::new_err("max_concurrent_docs must be >= 1"));
                    }
                    max_docs = Some(n);
                }
            }
            "max_concurrent_pages" if allow_engine_keys => {
                if let Some(n) = v.extract::<Option<usize>>()? {
                    if n == 0 {
                        return Err(PyValueError::new_err("max_concurrent_pages must be >= 1"));
                    }
                    max_pages_concurrency = Some(n);
                }
            }
            "max_pdf_bytes" if allow_engine_keys => {
                let limit: usize = v.extract()?;
                if limit == 0 {
                    return Err(PyValueError::new_err("max_pdf_bytes must be > 0"));
                }
                max_pdf_bytes = Some(limit);
            }
            "max_pages" if allow_engine_keys => {
                let limit = v.extract::<Option<usize>>()?;
                if matches!(limit, Some(0)) {
                    return Err(PyValueError::new_err("max_pages must be >= 1 or None"));
                }
                max_pages = Some(limit);
            }
            other => {
                return Err(PyValueError::new_err(format!("Unknown option: '{other}'")));
            }
        }
    }

    if let Some(mode) = hdr_mode_name {
        match mode.to_ascii_lowercase().as_str() {
            "identify" | "identify_headers" => {
                let (cur_lv, cur_bl) = match opts.hdr_info {
                    HeaderMode::IdentifyHeaders {
                        max_levels,
                        body_limit,
                    } => (max_levels, body_limit),
                    _ => (6, None),
                };
                opts.hdr_info = HeaderMode::IdentifyHeaders {
                    max_levels: hdr_max_levels.unwrap_or(cur_lv),
                    body_limit: hdr_body_limit.unwrap_or(cur_bl),
                };
            }
            "toc" | "toc_headers" => opts.hdr_info = HeaderMode::TocHeaders,
            "none" | "disabled" | "off" => opts.hdr_info = HeaderMode::Disabled,
            other => {
                return Err(PyValueError::new_err(format!(
                    "Unknown hdr_info '{other}'. Expected 'identify', 'toc', or 'disabled'"
                )));
            }
        }
    } else if hdr_max_levels.is_some() || hdr_body_limit.is_some() {
        let (cur_lv, cur_bl) = match opts.hdr_info {
            HeaderMode::IdentifyHeaders {
                max_levels,
                body_limit,
            } => (max_levels, body_limit),
            _ => (6, None),
        };
        opts.hdr_info = HeaderMode::IdentifyHeaders {
            max_levels: hdr_max_levels.unwrap_or(cur_lv),
            body_limit: hdr_body_limit.unwrap_or(cur_bl),
        };
    }

    Ok(ParsedOptions {
        opts,
        max_docs,
        max_pages_concurrency,
        max_pdf_bytes,
        max_pages,
    })
}

fn metadata_to_pydict<'py>(py: Python<'py>, meta: &DocumentMetadata, page_count: usize, file_path: Option<&PathBuf>) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("format", &meta.format)?;
    d.set_item("title", &meta.title)?;
    d.set_item("author", &meta.author)?;
    d.set_item("subject", &meta.subject)?;
    d.set_item("keywords", &meta.keywords)?;
    d.set_item("creator", &meta.creator)?;
    d.set_item("producer", &meta.producer)?;
    d.set_item("creation_date", &meta.creation_date)?;
    d.set_item("mod_date", &meta.mod_date)?;
    d.set_item("encryption", meta.encryption.as_deref())?;
    d.set_item(
        "file_path",
        file_path
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
    )?;
    d.set_item("page_count", page_count)?;
    Ok(d)
}

fn chunk_to_pydict<'py>(py: Python<'py>, chunk: &PageChunk) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);

    let meta = PyDict::new(py);
    meta.set_item("format", &chunk.metadata.format)?;
    meta.set_item("title", &chunk.metadata.title)?;
    meta.set_item("author", &chunk.metadata.author)?;
    meta.set_item("subject", &chunk.metadata.subject)?;
    meta.set_item("keywords", &chunk.metadata.keywords)?;
    meta.set_item("creator", &chunk.metadata.creator)?;
    meta.set_item("producer", &chunk.metadata.producer)?;
    meta.set_item("creation_date", &chunk.metadata.creation_date)?;
    meta.set_item("mod_date", &chunk.metadata.mod_date)?;
    meta.set_item("encryption", chunk.metadata.encryption.as_deref())?;
    meta.set_item("file_path", &chunk.metadata.file_path)?;
    meta.set_item("page_count", chunk.metadata.page_count)?;
    meta.set_item("page", chunk.metadata.page)?;
    d.set_item("metadata", meta)?;

    let toc_items = PyList::new(
        py,
        chunk
            .toc_items
            .iter()
            .map(|(lvl, title, page)| (*lvl, title.as_str(), *page)),
    )?;
    d.set_item("toc_items", toc_items)?;

    let mut table_dicts = Vec::with_capacity(chunk.tables.len());
    for tbl in &chunk.tables {
        let td = PyDict::new(py);
        td.set_item("bbox", tbl.bbox.to_vec())?;
        td.set_item("rows", tbl.rows)?;
        td.set_item("columns", tbl.columns)?;
        table_dicts.push(td);
    }
    d.set_item("tables", PyList::new(py, table_dicts)?)?;

    let mut image_dicts = Vec::with_capacity(chunk.images.len());
    for img in &chunk.images {
        let id = PyDict::new(py);
        id.set_item("number", img.number)?;
        id.set_item("bbox", img.bbox.to_vec())?;
        id.set_item("width", img.width)?;
        id.set_item("height", img.height)?;
        id.set_item("cs_name", &img.cs_name)?;
        id.set_item("ext", &img.ext)?;
        if let Some(ref name) = img.file_name {
            id.set_item("file_name", name)?;
        }
        image_dicts.push(id);
    }
    d.set_item("images", PyList::new(py, image_dicts)?)?;

    let mut graphic_dicts = Vec::with_capacity(chunk.graphics.len());
    for gr in &chunk.graphics {
        let gd = PyDict::new(py);
        gd.set_item("bbox", gr.bbox.to_vec())?;
        graphic_dicts.push(gd);
    }
    d.set_item("graphics", PyList::new(py, graphic_dicts)?)?;

    d.set_item("text", &chunk.text)?;

    if !chunk.words.is_empty() {
        let words = PyList::new(
            py,
            chunk.words.iter().map(|w| {
                (
                    w.0,
                    w.1,
                    w.2,
                    w.3,
                    w.4.as_str(),
                    w.5,
                    w.6,
                    w.7,
                )
            }),
        )?;
        d.set_item("words", words)?;
    } else {
        d.set_item("words", PyList::empty(py))?;
    }

    if let Some(ref h) = chunk.hygiene {
        let hd = PyDict::new(py);
        hd.set_item("mojibake_chars_removed", h.mojibake_chars_removed)?;
        hd.set_item("ligatures_expanded", h.ligatures_expanded)?;
        hd.set_item("faux_bold_deduped", h.faux_bold_deduped)?;
        hd.set_item("watermarks_filtered", h.watermarks_filtered)?;
        hd.set_item("invisible_spans_filtered", h.invisible_spans_filtered)?;
        hd.set_item("dehyphenated_joins", h.dehyphenated_joins)?;
        hd.set_item("running_headers_removed", h.running_headers_removed)?;
        d.set_item("hygiene", hd)?;
    }

    Ok(d)
}

fn chunks_to_pylist<'py>(py: Python<'py>, chunks: &[PageChunk]) -> PyResult<Bound<'py, PyList>> {
    let mut items = Vec::with_capacity(chunks.len());
    for c in chunks {
        items.push(chunk_to_pydict(py, c)?);
    }
    PyList::new(py, items)
}

fn options_to_pydict<'py>(
    py: Python<'py>,
    opts: &ToMarkdownOptions,
    max_pdf_bytes: usize,
    max_pages: Option<usize>,
    max_concurrent_docs: usize,
    max_concurrent_pages: usize,
    uses_global_engine: bool,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("pages", opts.pages.clone())?;
    let (hdr_str, hdr_levels, hdr_limit) = match &opts.hdr_info {
        HeaderMode::IdentifyHeaders {
            max_levels,
            body_limit,
        } => ("identify", Some(*max_levels), *body_limit),
        HeaderMode::TocHeaders => ("toc", None, None),
        HeaderMode::Disabled | HeaderMode::Custom(_) => ("disabled", None, None),
    };
    d.set_item("hdr_info", hdr_str)?;
    d.set_item("header_max_levels", hdr_levels)?;
    d.set_item("header_body_limit", hdr_limit)?;
    d.set_item("table_strategy", table_strategy_str(opts.table_strategy))?;
    d.set_item(
        "margins",
        PyTuple::new(
            py,
            [
                opts.margins.left,
                opts.margins.top,
                opts.margins.right,
                opts.margins.bottom,
            ],
        )?,
    )?;
    d.set_item("page_chunks", opts.page_chunks)?;
    d.set_item("page_separators", opts.page_separators)?;
    d.set_item("page_width", opts.page_width)?;
    d.set_item("page_height", opts.page_height)?;
    d.set_item("extract_words", opts.extract_words)?;
    d.set_item("ignore_code", opts.ignore_code)?;
    d.set_item("ignore_images", opts.ignore_images)?;
    d.set_item("ignore_graphics", opts.ignore_graphics)?;
    d.set_item("force_text", opts.force_text)?;
    d.set_item("fontsize_limit", opts.fontsize_limit)?;
    d.set_item("use_glyphs", opts.use_glyphs)?;
    d.set_item("detect_bg_color", opts.detect_bg_color)?;
    d.set_item("ignore_alpha", opts.ignore_alpha)?;
    d.set_item("header", opts.header)?;
    d.set_item("footer", opts.footer)?;
    d.set_item("dpi", opts.dpi)?;
    d.set_item("image_size_limit", opts.image_size_limit)?;
    d.set_item("graphics_limit", opts.graphics_limit)?;
    d.set_item("clean_mojibake", opts.hygiene.clean_mojibake)?;
    d.set_item("dehyphenate", opts.hygiene.dehyphenate)?;
    d.set_item("dedup_faux_bold", opts.hygiene.dedup_faux_bold)?;
    d.set_item("ignore_invisible_text", opts.hygiene.ignore_invisible_text)?;
    d.set_item("strip_watermarks", opts.hygiene.strip_watermarks)?;
    d.set_item("dedup_running_headers", opts.hygiene.dedup_running_headers)?;
    d.set_item("max_pdf_bytes", max_pdf_bytes)?;
    d.set_item("max_pages", max_pages)?;
    d.set_item("max_concurrent_docs", max_concurrent_docs)?;
    d.set_item("max_concurrent_pages", max_concurrent_pages)?;
    d.set_item("uses_global_engine", uses_global_engine)?;
    Ok(d)
}

fn run_doc_extract(
    engine: &Arc<PdfEngine>,
    doc: Arc<PdfDocument>,
    options: Arc<ToMarkdownOptions>,
) -> PyResult<MarkdownOutput> {
    let rt = shared_tokio_runtime().map_err(PyRuntimeError::new_err)?;
    rt.block_on(engine.extract_doc_shared(doc, options))
        .map_err(pdf_err_to_py)
}

/// Immutable, thread-safe parsed PDF handle holding shared Rust resources (`Arc<PdfDocument>`).
/// Parse once via `extractor.open(path)` or `extractor.open_bytes(data)` and run multiple
/// extraction views (`to_markdown`, `to_chunks`, `to_json`, `to_text`) with zero re-parsing.
#[pyclass(frozen, module = "pymupdf4llm_rs._native")]
pub struct ParsedPdf {
    doc: Arc<PdfDocument>,
    engine: Arc<PdfEngine>,
    default_options: Arc<ToMarkdownOptions>,
}

impl ParsedPdf {
    fn resolve_opts(
        &self,
        kwargs: Option<&Bound<'_, PyDict>>,
        force_chunks: Option<bool>,
    ) -> PyResult<Arc<ToMarkdownOptions>> {
        if kwargs.is_none() && force_chunks.is_none() {
            return Ok(Arc::clone(&self.default_options));
        }
        let parsed = apply_option_kwargs(&self.default_options, kwargs, false)?;
        let mut opts = parsed.opts;
        if let Some(flag) = force_chunks {
            opts.page_chunks = flag;
        }
        Ok(Arc::new(opts))
    }
}

#[pymethods]
impl ParsedPdf {
    #[getter]
    fn page_count(&self) -> usize {
        self.doc.page_count()
    }

    #[getter]
    fn file_path(&self) -> Option<String> {
        self.doc
            .file_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
    }

    fn metadata<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        metadata_to_pydict(
            py,
            &self.doc.metadata,
            self.doc.page_count(),
            self.doc.file_path.as_ref(),
        )
    }

    fn toc<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        PyList::new(
            py,
            self.doc
                .get_toc()
                .iter()
                .map(|(lvl, title, page)| (*lvl, title.as_str(), *page)),
        )
    }

    #[pyo3(signature = (**kwargs))]
    fn to_markdown(&self, py: Python<'_>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<PyObject> {
        let opts = self.resolve_opts(kwargs, None)?;
        let doc = Arc::clone(&self.doc);
        let engine = Arc::clone(&self.engine);
        let out = py.allow_threads(|| run_doc_extract(&engine, doc, opts))?;
        match out {
            MarkdownOutput::Text(s) => Ok(s.into_pyobject(py)?.into_any().unbind()),
            MarkdownOutput::Chunks(chunks) => {
                Ok(chunks_to_pylist(py, &chunks)?.into_any().unbind())
            }
        }
    }

    #[pyo3(signature = (**kwargs))]
    fn to_chunks<'py>(
        &self,
        py: Python<'py>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Bound<'py, PyList>> {
        let opts = self.resolve_opts(kwargs, Some(true))?;
        let doc = Arc::clone(&self.doc);
        let engine = Arc::clone(&self.engine);
        let out = py.allow_threads(|| run_doc_extract(&engine, doc, opts))?;
        let MarkdownOutput::Chunks(chunks) = out else {
            return Err(PyRuntimeError::new_err("Unexpected text output for to_chunks"));
        };
        chunks_to_pylist(py, &chunks)
    }

    #[pyo3(signature = (**kwargs))]
    fn to_json(&self, py: Python<'_>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<String> {
        let opts = self.resolve_opts(kwargs, Some(true))?;
        let doc = Arc::clone(&self.doc);
        let engine = Arc::clone(&self.engine);
        py.allow_threads(move || {
            let out = run_doc_extract(&engine, doc, opts)?;
            serde_json::to_string_pretty(&out)
                .map_err(|e| PyRuntimeError::new_err(format!("JSON serialization error: {e}")))
        })
    }

    #[pyo3(signature = (**kwargs))]
    fn to_text(&self, py: Python<'_>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<String> {
        let opts = self.resolve_opts(kwargs, Some(true))?;
        let doc = Arc::clone(&self.doc);
        let engine = Arc::clone(&self.engine);
        py.allow_threads(move || {
            let out = run_doc_extract(&engine, doc, opts)?;
            let MarkdownOutput::Chunks(chunks) = out else {
                return Err(PyRuntimeError::new_err("Unexpected output variant"));
            };
            Ok(chunks
                .into_iter()
                .map(|c| strip_markdown_syntax(&c.text))
                .collect::<Vec<_>>()
                .join("\n\n"))
        })
    }

    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    #[pyo3(signature = (_exc_type=None, _exc_val=None, _exc_tb=None))]
    fn __exit__(
        &self,
        _exc_type: Option<&Bound<'_, PyAny>>,
        _exc_val: Option<&Bound<'_, PyAny>>,
        _exc_tb: Option<&Bound<'_, PyAny>>,
    ) -> bool {
        false
    }

    fn __repr__(&self) -> String {
        format!(
            "ParsedPdf(pages={}, file_path={:?})",
            self.doc.page_count(),
            self.doc.file_path
        )
    }
}

/// Reusable Rust-backed PDF extraction engine & parameter preset.
///
/// Instantiate once at application/worker startup to hold shared Rust resources:
/// - Bounded `Arc<PdfEngine>` concurrency semaphores (`max_concurrent_docs`, `max_concurrent_pages`)
/// - Reusable Tokio + Rayon worker pools & global lock-free `ToUnicode` CMap cache
/// - Pre-validated `Arc<ToMarkdownOptions>` defaults and safety caps (`max_pdf_bytes`, `max_pages`)
#[pyclass(frozen, module = "pymupdf4llm_rs._native")]
pub struct PdfExtractor {
    engine: Arc<PdfEngine>,
    default_options: Arc<ToMarkdownOptions>,
    max_pdf_bytes: usize,
    max_pages: Option<usize>,
    max_concurrent_docs: usize,
    max_concurrent_pages: usize,
    uses_global_engine: bool,
}

impl PdfExtractor {
    fn resolve_opts(
        &self,
        kwargs: Option<&Bound<'_, PyDict>>,
        force_chunks: Option<bool>,
    ) -> PyResult<Arc<ToMarkdownOptions>> {
        if kwargs.is_none() && force_chunks.is_none() {
            return Ok(Arc::clone(&self.default_options));
        }
        let parsed = apply_option_kwargs(&self.default_options, kwargs, false)?;
        let mut opts = parsed.opts;
        if let Some(flag) = force_chunks {
            opts.page_chunks = flag;
        }
        Ok(Arc::new(opts))
    }

    fn parse_doc_from_any(
        &self,
        py: Python<'_>,
        source: &Bound<'_, PyAny>,
    ) -> PyResult<Arc<PdfDocument>> {
        if let Ok(parsed) = source.extract::<PyRef<'_, ParsedPdf>>() {
            if let Some(limit) = self.max_pages {
                if parsed.doc.page_count() > limit {
                    return Err(pdf_err_to_py(PdfError::PageOutOfBounds {
                        index: limit,
                        total: parsed.doc.page_count(),
                    }));
                }
            }
            return Ok(Arc::clone(&parsed.doc));
        }

        if source.is_instance_of::<PyBytes>()
            || source.is_instance_of::<PyByteArray>()
            || source.hasattr("tobytes")?
        {
            let bytes = extract_pdf_bytes(source, self.max_pdf_bytes)?;
            let max_pages = self.max_pages;
            return py.allow_threads(move || {
                let doc = PdfDocument::from_bytes(&bytes).map_err(pdf_err_to_py)?;
                check_page_limit(&doc, max_pages).map_err(pdf_err_to_py)?;
                Ok(Arc::new(doc))
            });
        }

        let path: PathBuf = source.extract()?;
        check_file_size(&path, self.max_pdf_bytes)?;
        let max_pages = self.max_pages;
        py.allow_threads(move || {
            let doc = PdfDocument::open(&path).map_err(pdf_err_to_py)?;
            check_page_limit(&doc, max_pages).map_err(pdf_err_to_py)?;
            Ok(Arc::new(doc))
        })
    }
}

#[pymethods]
impl PdfExtractor {
    #[new]
    #[pyo3(signature = (**kwargs))]
    fn new(kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        let base = ToMarkdownOptions::default();
        let parsed = apply_option_kwargs(&base, kwargs, true)?;

        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let default_docs = cpus.max(4);
        let default_pages = (cpus * 2).max(8);

        let (engine, uses_global_engine, actual_docs, actual_pages) =
            if parsed.max_docs.is_none() && parsed.max_pages_concurrency.is_none() {
                (
                    Arc::clone(PdfEngine::global()),
                    true,
                    default_docs,
                    default_pages,
                )
            } else {
                let docs = parsed.max_docs.unwrap_or(default_docs);
                let pages = parsed.max_pages_concurrency.unwrap_or(default_pages);
                (Arc::new(PdfEngine::new(docs, pages)), false, docs, pages)
            };

        Ok(Self {
            engine,
            default_options: Arc::new(parsed.opts),
            max_pdf_bytes: parsed.max_pdf_bytes.unwrap_or(DEFAULT_MAX_PDF_BYTES),
            max_pages: parsed.max_pages.flatten(),
            max_concurrent_docs: actual_docs,
            max_concurrent_pages: actual_pages,
            uses_global_engine,
        })
    }

    /// Creates a lightweight derived `PdfExtractor` sharing the SAME underlying `Arc<PdfEngine>`
    /// worker/semaphore resources while overriding selected extraction options.
    #[pyo3(signature = (**kwargs))]
    fn with_options(&self, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        let parsed = apply_option_kwargs(&self.default_options, kwargs, true)?;
        Ok(Self {
            engine: Arc::clone(&self.engine),
            default_options: Arc::new(parsed.opts),
            max_pdf_bytes: parsed.max_pdf_bytes.unwrap_or(self.max_pdf_bytes),
            max_pages: parsed.max_pages.unwrap_or(self.max_pages),
            max_concurrent_docs: self.max_concurrent_docs,
            max_concurrent_pages: self.max_concurrent_pages,
            uses_global_engine: self.uses_global_engine,
        })
    }

    #[getter]
    fn options<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        options_to_pydict(
            py,
            &self.default_options,
            self.max_pdf_bytes,
            self.max_pages,
            self.max_concurrent_docs,
            self.max_concurrent_pages,
            self.uses_global_engine,
        )
    }

    /// Opens and parses a PDF path or bytes once into a reusable `ParsedPdf` handle.
    fn open(&self, py: Python<'_>, source: &Bound<'_, PyAny>) -> PyResult<ParsedPdf> {
        let doc = self.parse_doc_from_any(py, source)?;
        Ok(ParsedPdf {
            doc,
            engine: Arc::clone(&self.engine),
            default_options: Arc::clone(&self.default_options),
        })
    }

    /// Parses in-memory PDF bytes (`bytes | bytearray | memoryview`) once into a `ParsedPdf` handle.
    fn open_bytes(&self, py: Python<'_>, data: &Bound<'_, PyAny>) -> PyResult<ParsedPdf> {
        let bytes = extract_pdf_bytes(data, self.max_pdf_bytes)?;
        let max_pages = self.max_pages;
        let doc = py.allow_threads(move || {
            let doc = PdfDocument::from_bytes(&bytes).map_err(pdf_err_to_py)?;
            check_page_limit(&doc, max_pages).map_err(pdf_err_to_py)?;
            Ok::<Arc<PdfDocument>, PyErr>(Arc::new(doc))
        })?;
        Ok(ParsedPdf {
            doc,
            engine: Arc::clone(&self.engine),
            default_options: Arc::clone(&self.default_options),
        })
    }

    /// Extracts Markdown (or chunk dicts if `page_chunks=True`) from `str | Path | bytes | ParsedPdf`.
    #[pyo3(signature = (source, **kwargs))]
    fn to_markdown(
        &self,
        py: Python<'_>,
        source: &Bound<'_, PyAny>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyObject> {
        let opts = self.resolve_opts(kwargs, None)?;
        let doc = self.parse_doc_from_any(py, source)?;
        let engine = Arc::clone(&self.engine);
        let out = py.allow_threads(|| run_doc_extract(&engine, doc, opts))?;
        match out {
            MarkdownOutput::Text(s) => Ok(s.into_pyobject(py)?.into_any().unbind()),
            MarkdownOutput::Chunks(chunks) => {
                Ok(chunks_to_pylist(py, &chunks)?.into_any().unbind())
            }
        }
    }

    /// Extracts Markdown directly from in-memory PDF bytes while releasing the Python GIL.
    #[pyo3(signature = (data, **kwargs))]
    fn to_markdown_bytes(
        &self,
        py: Python<'_>,
        data: &Bound<'_, PyAny>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyObject> {
        let opts = self.resolve_opts(kwargs, None)?;
        let bytes = extract_pdf_bytes(data, self.max_pdf_bytes)?;
        let max_pages = self.max_pages;
        let engine = Arc::clone(&self.engine);
        let out = py.allow_threads(move || {
            let doc = PdfDocument::from_bytes(&bytes).map_err(pdf_err_to_py)?;
            check_page_limit(&doc, max_pages).map_err(pdf_err_to_py)?;
            drop(bytes); // Drop raw buffer eagerly before page extraction begins
            run_doc_extract(&engine, Arc::new(doc), opts)
        })?;
        match out {
            MarkdownOutput::Text(s) => Ok(s.into_pyobject(py)?.into_any().unbind()),
            MarkdownOutput::Chunks(chunks) => {
                Ok(chunks_to_pylist(py, &chunks)?.into_any().unbind())
            }
        }
    }

    /// Extracts structured page chunk dicts (`list[dict]`) directly into Python objects without JSON round-trip.
    #[pyo3(signature = (source, **kwargs))]
    fn to_chunks<'py>(
        &self,
        py: Python<'py>,
        source: &Bound<'_, PyAny>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Bound<'py, PyList>> {
        let opts = self.resolve_opts(kwargs, Some(true))?;
        let doc = self.parse_doc_from_any(py, source)?;
        let engine = Arc::clone(&self.engine);
        let out = py.allow_threads(|| run_doc_extract(&engine, doc, opts))?;
        let MarkdownOutput::Chunks(chunks) = out else {
            return Err(PyRuntimeError::new_err("Unexpected text output for to_chunks"));
        };
        chunks_to_pylist(py, &chunks)
    }

    /// Extracts structured page chunk dicts directly from in-memory bytes.
    #[pyo3(signature = (data, **kwargs))]
    fn to_chunks_bytes<'py>(
        &self,
        py: Python<'py>,
        data: &Bound<'_, PyAny>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Bound<'py, PyList>> {
        let opts = self.resolve_opts(kwargs, Some(true))?;
        let bytes = extract_pdf_bytes(data, self.max_pdf_bytes)?;
        let max_pages = self.max_pages;
        let engine = Arc::clone(&self.engine);
        let out = py.allow_threads(move || {
            let doc = PdfDocument::from_bytes(&bytes).map_err(pdf_err_to_py)?;
            check_page_limit(&doc, max_pages).map_err(pdf_err_to_py)?;
            drop(bytes);
            run_doc_extract(&engine, Arc::new(doc), opts)
        })?;
        let MarkdownOutput::Chunks(chunks) = out else {
            return Err(PyRuntimeError::new_err("Unexpected text output for to_chunks_bytes"));
        };
        chunks_to_pylist(py, &chunks)
    }

    /// Extracts pretty-printed JSON string matching `PageChunk` schema.
    #[pyo3(signature = (source, **kwargs))]
    fn to_json(
        &self,
        py: Python<'_>,
        source: &Bound<'_, PyAny>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<String> {
        let opts = self.resolve_opts(kwargs, Some(true))?;
        let doc = self.parse_doc_from_any(py, source)?;
        let engine = Arc::clone(&self.engine);
        py.allow_threads(move || {
            let out = run_doc_extract(&engine, doc, opts)?;
            serde_json::to_string_pretty(&out)
                .map_err(|e| PyRuntimeError::new_err(format!("JSON serialization error: {e}")))
        })
    }

    /// Extracts plain reading-order text stripped of Markdown formatting.
    #[pyo3(signature = (source, **kwargs))]
    fn to_text(
        &self,
        py: Python<'_>,
        source: &Bound<'_, PyAny>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<String> {
        let opts = self.resolve_opts(kwargs, Some(true))?;
        let doc = self.parse_doc_from_any(py, source)?;
        let engine = Arc::clone(&self.engine);
        py.allow_threads(move || {
            let out = run_doc_extract(&engine, doc, opts)?;
            let MarkdownOutput::Chunks(chunks) = out else {
                return Err(PyRuntimeError::new_err("Unexpected output variant"));
            };
            Ok(chunks
                .into_iter()
                .map(|c| strip_markdown_syntax(&c.text))
                .collect::<Vec<_>>()
                .join("\n\n"))
        })
    }

    /// Concurrently processes a batch of PDF file paths using this instance's bounded `PdfEngine`.
    /// Returns a list of dicts: `{"path", "ok", "output", "error", "elapsed_ms"}`.
    #[pyo3(signature = (paths, **kwargs))]
    fn batch_to_markdown<'py>(
        &self,
        py: Python<'py>,
        paths: Vec<PathBuf>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Bound<'py, PyList>> {
        let opts = self.resolve_opts(kwargs, None)?;
        let engine = Arc::clone(&self.engine);
        let max_pdf_bytes = self.max_pdf_bytes;
        let max_pages = self.max_pages;

        struct ItemOutcome {
            path: String,
            output: std::result::Result<MarkdownOutput, String>,
            elapsed_ms: f64,
        }

        let outcomes = py.allow_threads(move || -> PyResult<Vec<ItemOutcome>> {
            let rt = shared_tokio_runtime().map_err(PyRuntimeError::new_err)?;
            let res = rt.block_on(async move {
                let mut tasks = Vec::with_capacity(paths.len());
                for p in paths {
                    let eng = Arc::clone(&engine);
                    let opt = Arc::clone(&opts);
                    tasks.push(tokio::spawn(async move {
                        let t0 = std::time::Instant::now();
                        let path_str = p.to_string_lossy().into_owned();
                        let meta_res = tokio::fs::metadata(&p).await;
                        let out = match meta_res {
                            Err(e) => Err(e.to_string()),
                            Ok(meta) if meta.len() > max_pdf_bytes as u64 => Err(format!(
                                "PDF file size ({} bytes) exceeds max_pdf_bytes ({max_pdf_bytes} bytes)",
                                meta.len()
                            )),
                            Ok(_) => match tokio::fs::read(&p).await {
                                Err(e) => Err(e.to_string()),
                                Ok(bytes) => {
                                    let p_clone = p.clone();
                                    let parsed = tokio::task::spawn_blocking(move || {
                                        let mut doc = PdfDocument::from_bytes(&bytes)?;
                                        check_page_limit(&doc, max_pages)?;
                                        doc.file_path = Some(p_clone);
                                        Ok::<Arc<PdfDocument>, PdfError>(Arc::new(doc))
                                    })
                                    .await;
                                    match parsed {
                                        Err(e) => Err(e.to_string()),
                                        Ok(Err(e)) => Err(e.to_string()),
                                        Ok(Ok(doc)) => eng
                                            .extract_doc_shared(doc, opt)
                                            .await
                                            .map_err(|e| e.to_string()),
                                    }
                                }
                            },
                        };
                        ItemOutcome {
                            path: path_str,
                            output: out,
                            elapsed_ms: t0.elapsed().as_secs_f64() * 1000.0,
                        }
                    }));
                }

                let mut collected = Vec::with_capacity(tasks.len());
                for t in tasks {
                    if let Ok(item) = t.await {
                        collected.push(item);
                    }
                }
                collected
            });
            Ok(res)
        })?;

        let mut dicts = Vec::with_capacity(outcomes.len());
        for item in outcomes {
            let d = PyDict::new(py);
            d.set_item("path", &item.path)?;
            d.set_item("elapsed_ms", item.elapsed_ms)?;
            match item.output {
                Ok(MarkdownOutput::Text(s)) => {
                    d.set_item("ok", true)?;
                    d.set_item("output", s)?;
                    d.set_item("error", py.None())?;
                }
                Ok(MarkdownOutput::Chunks(chunks)) => {
                    d.set_item("ok", true)?;
                    d.set_item("output", chunks_to_pylist(py, &chunks)?)?;
                    d.set_item("error", py.None())?;
                }
                Err(err) => {
                    d.set_item("ok", false)?;
                    d.set_item("output", py.None())?;
                    d.set_item("error", err)?;
                }
            }
            dicts.push(d);
        }
        PyList::new(py, dicts)
    }

    fn close(&self) {
        // Resources are ref-counted and freed deterministically when references drop.
    }

    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    #[pyo3(signature = (_exc_type=None, _exc_val=None, _exc_tb=None))]
    fn __exit__(
        &self,
        _exc_type: Option<&Bound<'_, PyAny>>,
        _exc_val: Option<&Bound<'_, PyAny>>,
        _exc_tb: Option<&Bound<'_, PyAny>>,
    ) -> bool {
        false
    }

    fn __repr__(&self) -> String {
        format!(
            "PdfExtractor(table_strategy='{}', max_concurrent_docs={}, max_concurrent_pages={}, max_pdf_bytes={}, max_pages={:?}, global_engine={})",
            table_strategy_str(self.default_options.table_strategy),
            self.max_concurrent_docs,
            self.max_concurrent_pages,
            self.max_pdf_bytes,
            self.max_pages,
            self.uses_global_engine
        )
    }
}

fn global_extractor() -> &'static PdfExtractor {
    static DEFAULT_EXTRACTOR: OnceLock<PdfExtractor> = OnceLock::new();
    DEFAULT_EXTRACTOR.get_or_init(|| {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        PdfExtractor {
            engine: Arc::clone(PdfEngine::global()),
            default_options: Arc::clone(PdfEngine::default_options()),
            max_pdf_bytes: DEFAULT_MAX_PDF_BYTES,
            max_pages: None,
            max_concurrent_docs: cpus.max(4),
            max_concurrent_pages: (cpus * 2).max(8),
            uses_global_engine: true,
        }
    })
}

#[pyfunction]
#[pyo3(signature = (source, **kwargs))]
fn to_markdown(
    py: Python<'_>,
    source: &Bound<'_, PyAny>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<PyObject> {
    global_extractor().to_markdown(py, source, kwargs)
}

#[pyfunction]
#[pyo3(signature = (data, **kwargs))]
fn to_markdown_bytes(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<PyObject> {
    global_extractor().to_markdown_bytes(py, data, kwargs)
}

#[pyfunction]
#[pyo3(signature = (source, **kwargs))]
fn to_chunks<'py>(
    py: Python<'py>,
    source: &Bound<'_, PyAny>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<Bound<'py, PyList>> {
    global_extractor().to_chunks(py, source, kwargs)
}

#[pyfunction]
#[pyo3(signature = (source, **kwargs))]
fn to_json(
    py: Python<'_>,
    source: &Bound<'_, PyAny>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<String> {
    global_extractor().to_json(py, source, kwargs)
}

#[pyfunction]
#[pyo3(signature = (source, **kwargs))]
fn to_text(
    py: Python<'_>,
    source: &Bound<'_, PyAny>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<String> {
    global_extractor().to_text(py, source, kwargs)
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<PdfExtractor>()?;
    m.add_class::<ParsedPdf>()?;
    m.add_function(wrap_pyfunction!(to_markdown, m)?)?;
    m.add_function(wrap_pyfunction!(to_markdown_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(to_chunks, m)?)?;
    m.add_function(wrap_pyfunction!(to_json, m)?)?;
    m.add_function(wrap_pyfunction!(to_text, m)?)?;
    Ok(())
}
