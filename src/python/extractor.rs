use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyBytes, PyDict, PyList};

use super::convert::chunks_to_pylist;
use super::options::{apply_option_kwargs, options_to_pydict, table_strategy_str};
use super::parsed_pdf::ParsedPdf;
use super::{
    check_file_size, check_page_limit, extract_pdf_bytes, pdf_err_to_py, run_doc_extract,
    shared_tokio_runtime, DEFAULT_MAX_PDF_BYTES,
};
use crate::async_engine::PdfEngine;
use crate::config::ToMarkdownOptions;
use crate::error::PdfError;
use crate::markdown::renderer::strip_markdown_syntax;
use crate::pdf::PdfDocument;
use crate::MarkdownOutput;

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
    pub(super) fn to_markdown(
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
    pub(super) fn to_markdown_bytes(
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
    pub(super) fn to_chunks<'py>(
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
            return Err(PyRuntimeError::new_err(
                "Unexpected text output for to_chunks",
            ));
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
            return Err(PyRuntimeError::new_err(
                "Unexpected text output for to_chunks_bytes",
            ));
        };
        chunks_to_pylist(py, &chunks)
    }

    /// Extracts pretty-printed JSON string matching `PageChunk` schema.
    #[pyo3(signature = (source, **kwargs))]
    pub(super) fn to_json(
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
    pub(super) fn to_text(
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
                    let p_display = p.to_string_lossy().into_owned();
                    let handle = tokio::spawn(async move {
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
                    });
                    tasks.push((p_display, handle));
                }

                let mut collected = Vec::with_capacity(tasks.len());
                for (path_str, t) in tasks {
                    match t.await {
                        Ok(item) => collected.push(item),
                        Err(e) => collected.push(ItemOutcome {
                            path: path_str,
                            output: Err(format!("Task execution error: {e}")),
                            elapsed_ms: 0.0,
                        }),
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

pub(super) fn global_extractor() -> &'static PdfExtractor {
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
