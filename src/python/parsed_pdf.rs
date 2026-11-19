use std::sync::Arc;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use super::convert::{chunks_to_pylist, metadata_to_pydict};
use super::options::apply_option_kwargs;
use super::run_doc_extract;
use crate::async_engine::PdfEngine;
use crate::config::ToMarkdownOptions;
use crate::markdown::renderer::strip_markdown_syntax;
use crate::pdf::PdfDocument;
use crate::MarkdownOutput;

/// Immutable, thread-safe parsed PDF handle holding shared Rust resources (`Arc<PdfDocument>`).
/// Parse once via `extractor.open(path)` or `extractor.open_bytes(data)` and run multiple
/// extraction views (`to_markdown`, `to_chunks`, `to_json`, `to_text`) with zero re-parsing.
#[pyclass(frozen, module = "pymupdf4llm_rs._native")]
pub struct ParsedPdf {
    pub(super) doc: Arc<PdfDocument>,
    pub(super) engine: Arc<PdfEngine>,
    pub(super) default_options: Arc<ToMarkdownOptions>,
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
    fn to_markdown(
        &self,
        py: Python<'_>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyObject> {
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
            return Err(PyRuntimeError::new_err(
                "Unexpected text output for to_chunks",
            ));
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
