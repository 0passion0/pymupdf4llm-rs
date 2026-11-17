//! # pymupdf4llm-rs
//!
//! 100% pure-Rust implementation of `pymupdf4llm` optimized for LLM & RAG pipelines,
//! supporting multi-column layout ordering, vector & text table extraction (via SpatialGrid + UnionFind),
//! font-histogram and TOC-based header identification, Unicode/mojibake hygiene cleaning,
//! Type0/CJK ToUnicode + Form XObject + LZW decoding, and high-concurrency async execution via Tokio + Rayon.

pub mod config;
pub mod error;
pub mod layout;
pub mod llama_reader;
pub mod markdown;
pub mod pdf;
pub mod text_cleaner;

#[cfg(feature = "async")]
pub mod async_engine;

#[cfg(feature = "python")]
mod python;

use std::path::Path;
use std::sync::Arc;
use rayon::prelude::*;

pub use config::{
    ConcurrencyConfig, HeaderMode, ImageFormat, Margins, ProgressCallback, Rect, TableStrategy,
    TextHygieneConfig, ToMarkdownOptions, ToMarkdownOptionsBuilder,
};
pub use error::{PdfError, Result};
pub use layout::headers::{HeaderResolver, IdentifyHeaders, TocHeaders};
pub use llama_reader::{LlamaDocument, LlamaMarkdownReader};
pub use markdown::chunk::{
    ExtractedGraphic, ExtractedImage, ExtractedTable, ExtractedWord, PageChunk, PageMetadata,
    TocItem,
};
pub use pdf::{DocumentMetadata, PdfDocument};
pub use text_cleaner::HygieneStats;

#[cfg(feature = "async")]
pub use async_engine::{
    batch_to_markdown_async, to_markdown_async, to_markdown_bytes_async, to_markdown_stream,
    BatchItemResult, PdfEngine,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum MarkdownOutput {
    Text(String),
    Chunks(Vec<PageChunk>),
}

impl MarkdownOutput {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            Self::Chunks(_) => None,
        }
    }

    pub fn as_chunks(&self) -> Option<&[PageChunk]> {
        match self {
            Self::Chunks(c) => Some(c),
            Self::Text(_) => None,
        }
    }

    pub fn into_text(self) -> String {
        match self {
            Self::Text(s) => s,
            Self::Chunks(chunks) => chunks
                .into_iter()
                .map(|c| c.text)
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

/// Input source abstraction matching pymupdf4llm's `doc` parameter (file path, bytes, or opened document).
pub enum PdfSource<'a> {
    Path(&'a Path),
    Bytes(&'a [u8]),
    Document(Arc<PdfDocument>),
}

impl<'a> From<&'a str> for PdfSource<'a> {
    fn from(s: &'a str) -> Self {
        Self::Path(Path::new(s))
    }
}

impl<'a> From<&'a Path> for PdfSource<'a> {
    fn from(p: &'a Path) -> Self {
        Self::Path(p)
    }
}

impl<'a> From<&'a std::path::PathBuf> for PdfSource<'a> {
    fn from(p: &'a std::path::PathBuf) -> Self {
        Self::Path(p.as_path())
    }
}

impl<'a> From<&'a [u8]> for PdfSource<'a> {
    fn from(b: &'a [u8]) -> Self {
        Self::Bytes(b)
    }
}

impl<'a> From<Arc<PdfDocument>> for PdfSource<'a> {
    fn from(d: Arc<PdfDocument>) -> Self {
        Self::Document(d)
    }
}

/// Synchronous `to_markdown` matching `pymupdf4llm.to_markdown(doc, ...)` using Rayon parallel page workers.
pub fn to_markdown<'a>(
    source: impl Into<PdfSource<'a>>,
    options: ToMarkdownOptions,
) -> Result<MarkdownOutput> {
    let doc = match source.into() {
        PdfSource::Path(path) => Arc::new(PdfDocument::open(path)?),
        PdfSource::Bytes(bytes) => Arc::new(PdfDocument::from_bytes(bytes)?),
        PdfSource::Document(doc) => doc,
    };
    process_document_sync(&doc, &options)
}

/// Synchronous `to_json` matching `pymupdf4llm.to_json(doc, ...)`.
pub fn to_json<'a>(source: impl Into<PdfSource<'a>>, options: ToMarkdownOptions) -> Result<String> {
    let mut opts = options;
    opts.page_chunks = true;
    let out = to_markdown(source, opts)?;
    serde_json::to_string_pretty(&out).map_err(|e| PdfError::Syntax {
        offset: 0,
        message: format!("JSON serialization error: {e}"),
    })
}

/// Synchronous `to_text` matching `pymupdf4llm.to_text(doc, ...)` (plain text extraction with reading order).
pub fn to_text<'a>(source: impl Into<PdfSource<'a>>, options: ToMarkdownOptions) -> Result<String> {
    let doc = match source.into() {
        PdfSource::Path(path) => Arc::new(PdfDocument::open(path)?),
        PdfSource::Bytes(bytes) => Arc::new(PdfDocument::from_bytes(bytes)?),
        PdfSource::Document(doc) => doc,
    };
    let mut opts = options;
    opts.page_chunks = true;
    let MarkdownOutput::Chunks(chunks) = process_document_sync(&doc, &opts)? else {
        unreachable!();
    };
    Ok(chunks
        .into_iter()
        .map(|c| markdown::renderer::strip_markdown_syntax(&c.text))
        .collect::<Vec<_>>()
        .join("\n\n"))
}

pub(crate) fn resolve_target_pages(doc: &PdfDocument, options: &ToMarkdownOptions) -> Result<Vec<usize>> {
    let total = doc.page_count();
    match &options.pages {
        None => Ok((0..total).collect()),
        Some(pages) => {
            for &idx in pages {
                if idx >= total {
                    return Err(PdfError::PageOutOfBounds { index: idx, total });
                }
            }
            Ok(pages.clone())
        }
    }
}

pub(crate) fn build_header_resolver(
    doc: &Arc<PdfDocument>,
    options: &ToMarkdownOptions,
) -> Result<Arc<dyn HeaderResolver>> {
    match &options.hdr_info {
        HeaderMode::IdentifyHeaders {
            max_levels,
            body_limit,
        } => {
            let resolver = IdentifyHeaders::from_document(doc, *max_levels, *body_limit, options)?;
            Ok(Arc::new(resolver))
        }
        HeaderMode::TocHeaders => Ok(Arc::new(TocHeaders::from_document(doc))),
        HeaderMode::Custom(custom) => Ok(Arc::clone(custom)),
        HeaderMode::Disabled => Ok(Arc::new(layout::headers::NoopHeaders)),
    }
}

pub(crate) fn strip_running_headers_from_pages(
    pages: &mut [(pdf::ParsedPage, layout::LayoutPage)],
    options: &ToMarkdownOptions,
) {
    if !options.hygiene.dedup_running_headers || pages.len() < 3 {
        return;
    }
    let sig_inputs: Vec<Vec<(f32, f32, String)>> = pages
        .iter()
        .map(|(parsed, layout)| {
            let h = parsed.crop_box.height().max(1.0);
            let y0 = parsed.crop_box.y0;
            layout
                .ordered_lines
                .iter()
                .map(|ln| ((ln.bbox.center_y() - y0) / h, ln.bbox.x0, ln.text.clone()))
                .collect()
        })
        .collect();

    let repeated = text_cleaner::detect_running_headers_footers(&sig_inputs, 3);
    if repeated.is_empty() {
        return;
    }

    for (parsed, layout) in pages.iter_mut() {
        let h = parsed.crop_box.height().max(1.0);
        let y0 = parsed.crop_box.y0;
        let before = layout.ordered_lines.len();
        layout.ordered_lines.retain(|ln| {
            let y_ratio = (ln.bbox.center_y() - y0) / h;
            if y_ratio <= 0.10 || y_ratio >= 0.90 {
                let sig = text_cleaner::normalize_running_signature(&ln.text);
                !repeated.contains_key(&sig)
            } else {
                true
            }
        });
        let removed = before.saturating_sub(layout.ordered_lines.len());
        layout.hygiene_stats.running_headers_removed += removed;
    }
}

pub(crate) fn process_document_sync(
    doc: &Arc<PdfDocument>,
    options: &ToMarkdownOptions,
) -> Result<MarkdownOutput> {
    let target_pages = resolve_target_pages(doc, options)?;
    let header_resolver = build_header_resolver(doc, options)?;
    let total_pages = target_pages.len();

    let completed = std::sync::atomic::AtomicUsize::new(0);
    let mut chunks: Vec<PageChunk> = if options.hygiene.dedup_running_headers && total_pages >= 3 {
        let mut analyzed: Vec<(pdf::ParsedPage, layout::LayoutPage)> = target_pages
            .par_iter()
            .map(|&page_idx| {
                let parsed = doc.parse_page(page_idx, options)?;
                let layout = layout::analyze_page_layout(&parsed, options);
                Ok((parsed, layout))
            })
            .collect::<Result<Vec<_>>>()?;

        strip_running_headers_from_pages(&mut analyzed, options);

        analyzed
            .into_par_iter()
            .map(|(parsed, layout)| {
                let chunk = markdown::render_page_chunk(
                    doc,
                    &parsed,
                    &layout,
                    options,
                    header_resolver.as_ref(),
                );
                let done = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if let Some(cb) = &options.progress_callback {
                    cb(done, total_pages);
                }
                chunk
            })
            .collect()
    } else {
        target_pages
            .par_iter()
            .map(|&page_idx| {
                let chunk = process_single_page(doc, page_idx, options, header_resolver.as_ref())?;
                let done = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if let Some(cb) = &options.progress_callback {
                    cb(done, total_pages);
                }
                Ok(chunk)
            })
            .collect::<Result<Vec<_>>>()?
    };

    chunks.sort_by_key(|c| c.metadata.page);

    if options.page_chunks {
        Ok(MarkdownOutput::Chunks(chunks))
    } else {
        let separator = if options.page_separators {
            "\n-----\n\n"
        } else {
            "\n"
        };
        let combined = chunks
            .into_iter()
            .map(|c| c.text)
            .collect::<Vec<_>>()
            .join(separator);
        Ok(MarkdownOutput::Text(combined))
    }
}

pub(crate) fn process_single_page(
    doc: &PdfDocument,
    page_idx: usize,
    options: &ToMarkdownOptions,
    header_resolver: &dyn HeaderResolver,
) -> Result<PageChunk> {
    let parsed_page = doc.parse_page(page_idx, options)?;
    let layout_page = layout::analyze_page_layout(&parsed_page, options);
    let chunk = markdown::render_page_chunk(doc, &parsed_page, &layout_page, options, header_resolver);
    Ok(chunk)
}
