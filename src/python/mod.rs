mod convert;
mod extractor;
mod options;
mod parsed_pdf;

use std::path::Path;
use std::sync::{Arc, OnceLock};

use pyo3::exceptions::{PyFileNotFoundError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyBytes, PyDict, PyList};

use crate::async_engine::PdfEngine;
use crate::config::ToMarkdownOptions;
use crate::error::PdfError;
use crate::pdf::PdfDocument;
use crate::MarkdownOutput;

use extractor::{global_extractor, PdfExtractor};
use parsed_pdf::ParsedPdf;

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

fn run_doc_extract(
    engine: &Arc<PdfEngine>,
    doc: Arc<PdfDocument>,
    options: Arc<ToMarkdownOptions>,
) -> PyResult<MarkdownOutput> {
    let rt = shared_tokio_runtime().map_err(PyRuntimeError::new_err)?;
    rt.block_on(engine.extract_doc_shared(doc, options))
        .map_err(pdf_err_to_py)
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
