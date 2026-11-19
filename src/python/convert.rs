use std::path::PathBuf;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::markdown::chunk::PageChunk;
use crate::pdf::DocumentMetadata;

pub(super) fn metadata_to_pydict<'py>(
    py: Python<'py>,
    meta: &DocumentMetadata,
    page_count: usize,
    file_path: Option<&PathBuf>,
) -> PyResult<Bound<'py, PyDict>> {
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
            chunk
                .words
                .iter()
                .map(|w| (w.0, w.1, w.2, w.3, w.4.as_str(), w.5, w.6, w.7)),
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

pub(super) fn chunks_to_pylist<'py>(
    py: Python<'py>,
    chunks: &[PageChunk],
) -> PyResult<Bound<'py, PyList>> {
    let mut items = Vec::with_capacity(chunks.len());
    for c in chunks {
        items.push(chunk_to_pydict(py, c)?);
    }
    PyList::new(py, items)
}
