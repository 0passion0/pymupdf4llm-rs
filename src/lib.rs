//! # pymupdf4llm-rs
//!
//! 100% 纯 Rust 实现的高性能 [`pymupdf4llm`](https://github.com/pymupdf/pymupdf4llm) PDF 转 Markdown 核心库，
//! 专为大语言模型（LLM）与检索增强生成（RAG）文档解析管线打造，零 C/C++ 外部依赖。
//!
//! ## 核心能力
//!
//! - **多栏版面与阅读顺序**：基于递归 X-Y Cut 与二维 [`layout::spatial::SpatialGrid`] 空间索引还原自然阅读流。
//! - **表格与标题识别**：完整支持 `LinesStrict`、`Lines`、`Text`、`Explicit` 四种表格提取策略与并查集分离，以及基于字号直方图 / `/Outlines` 书签的标题分级。
//! - **编码与乱码治理**：支持 Type0/CID `/W`、数组型 `ToUnicode` CMap、Form XObject 递归、Pure-Rust `LZWDecode`/`FlateDecode`（64 MB 防解压炸弹上限），以及 `TextHygieneConfig` 水印/隐形 OCR/连字清洗。
//! - **高并发调度与 Python 绑定**：支持 `async`（Tokio 信号量 + Rayon 工作窃取线程池）与 `python`（PyO3 `abi3-py39` 释放 GIL 绑定）。
//!
//! ## Feature Flags
//!
//! - `async`：启用 [`async_engine`] 异步引擎、[`PdfEngine::global`] 单例与批量并发调度（依赖 `tokio`、`futures`）。
//! - `cli`：启用 `pymupdf4llm` 命令行可执行文件（自动开启 `async`）。
//! - `images`：启用嵌入图像采样尺寸探测与导出支持（依赖 `image`）。
//! - `python` / `extension-module`：启用 PyO3 原生 Python 扩展模块封装。
//!
//! ## 快速示例
//!
//! ```no_run
//! use pymupdf4llm_rs::{to_markdown, MarkdownOutput, ToMarkdownOptions};
//!
//! let out = to_markdown("paper.pdf", ToMarkdownOptions::default())?;
//! match out {
//!     MarkdownOutput::Text(md) => println!("{md}"),
//!     MarkdownOutput::Chunks(chunks) => println!("共解析 {} 页", chunks.len()),
//! }
//! # Ok::<(), pymupdf4llm_rs::PdfError>(())
//! ```
//!
//! ## MSRV 与开源协议
//!
//! - **MSRV**：Rust `1.78`
//! - **开源协议**：`Apache-2.0 OR MIT`

#![deny(rustdoc::broken_intra_doc_links)]

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

use rayon::prelude::*;
use std::path::Path;
use std::sync::Arc;

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

/// Markdown 提取统一返回结果，对应普通全文模式或按页分块模式。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum MarkdownOutput {
    /// 整份文档合并后的完整 Markdown 文本。
    Text(String),
    /// 开启 `page_chunks` 后生成的逐页结构化分块列表。
    Chunks(Vec<PageChunk>),
}

impl MarkdownOutput {
    /// 若输出为纯文本模式则返回文本切片，否则返回 `None`。
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            Self::Chunks(_) => None,
        }
    }

    /// 若输出为分块模式则返回 [`PageChunk`] 切片，否则返回 `None`。
    pub fn as_chunks(&self) -> Option<&[PageChunk]> {
        match self {
            Self::Chunks(c) => Some(c),
            Self::Text(_) => None,
        }
    }

    /// 消费自身并统一提取为单个 Markdown 字符串（分块模式按换行拼接各页文本）。
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

/// 统一输入源抽象，兼容磁盘路径、原始字节切片或已解析共享的 [`PdfDocument`]。
pub enum PdfSource<'a> {
    /// 本地 PDF 文件路径。
    Path(&'a Path),
    /// 内存中的 PDF 原始字节流。
    Bytes(&'a [u8]),
    /// 已解析并跨请求共享的 [`PdfDocument`] 引用计数句柄。
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

pub(crate) fn resolve_target_pages(
    doc: &PdfDocument,
    options: &ToMarkdownOptions,
) -> Result<Vec<usize>> {
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
    let chunk =
        markdown::render_page_chunk(doc, &parsed_page, &layout_page, options, header_resolver);
    Ok(chunk)
}
