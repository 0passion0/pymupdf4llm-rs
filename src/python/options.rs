use std::path::PathBuf;

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

use crate::config::{HeaderMode, Margins, TableStrategy, ToMarkdownOptions};

pub(super) struct ParsedOptions {
    pub(super) opts: ToMarkdownOptions,
    pub(super) max_docs: Option<usize>,
    pub(super) max_pages_concurrency: Option<usize>,
    pub(super) max_pdf_bytes: Option<usize>,
    pub(super) max_pages: Option<Option<usize>>,
}

fn parse_margins(val: &Bound<'_, PyAny>) -> PyResult<Margins> {
    if let Ok(scalar) = val.extract::<f32>() {
        if !scalar.is_finite() || scalar < 0.0 {
            return Err(PyValueError::new_err(
                "margins scalar must be finite and >= 0",
            ));
        }
        return Ok(Margins::from(scalar));
    }
    if let Ok((h, v)) = val.extract::<(f32, f32)>() {
        if !h.is_finite() || !v.is_finite() || h < 0.0 || v < 0.0 {
            return Err(PyValueError::new_err(
                "margins values must be finite and >= 0",
            ));
        }
        return Ok(Margins::from((h, v)));
    }
    if let Ok((l, t, r, b)) = val.extract::<(f32, f32, f32, f32)>() {
        if ![l, t, r, b].iter().all(|x| x.is_finite() && *x >= 0.0) {
            return Err(PyValueError::new_err(
                "margins values must be finite and >= 0",
            ));
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

pub(super) fn table_strategy_str(ts: TableStrategy) -> &'static str {
    match ts {
        TableStrategy::LinesStrict => "lines_strict",
        TableStrategy::Lines => "lines",
        TableStrategy::Text => "text",
        TableStrategy::Explicit => "explicit",
    }
}

pub(super) fn apply_option_kwargs(
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
                let lim = v.extract::<Option<f32>>()?;
                if let Some(val) = lim {
                    if !val.is_finite() || val <= 0.0 {
                        return Err(PyValueError::new_err(
                            "header_body_limit must be finite and > 0",
                        ));
                    }
                }
                hdr_body_limit = Some(lim);
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
            "image_size_limit" => {
                let lim: f32 = v.extract()?;
                if !lim.is_finite() || lim < 0.0 {
                    return Err(PyValueError::new_err(
                        "image_size_limit must be finite and >= 0",
                    ));
                }
                opts.image_size_limit = lim;
            }
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
            "page_height" => {
                let ph: Option<f32> = v.extract()?;
                if let Some(val) = ph {
                    if !val.is_finite() || val <= 0.0 {
                        return Err(PyValueError::new_err("page_height must be finite and > 0"));
                    }
                }
                opts.page_height = ph;
            }
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
                    return Err(PyValueError::new_err(
                        "fontsize_limit must be finite and >= 0",
                    ));
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

pub(super) fn options_to_pydict<'py>(
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
