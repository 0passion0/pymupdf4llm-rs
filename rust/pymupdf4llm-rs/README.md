# pymupdf4llm-rs

[English](README.md) | [简体中文](README.zh-CN.md)

100% pure-Rust implementation of [pymupdf4llm](https://github.com/pymupdf/pymupdf4llm) with API/output compatibility, advanced mojibake/noise hygiene, `SpatialGrid` + `UnionFind` layout algorithms, and native Tokio + Rayon high-concurrency async processing.

## Key Capabilities

- **100% Pure Rust**: Zero C/C++ bindings (`no mupdf-sys`, `no pdfium-render`). Uses `flate2` (`rust_backend`), native PDF 1.0–2.0 binary/XRef/ObjStm parser, inherited `/Pages` tree `/MediaBox` & `/Resources`, `/Subtype /Form` XObject recursion, `LZWDecode` + `FlateDecode`, `Type0`/CID `/W` widths, `ToUnicode` CMap (`beginbfchar` + array `beginbfrange`), `MacRomanEncoding`, 70+ Adobe Glyph names, and `/Rotate` normalization.
- **Full `pymupdf4llm` Parity**:
  - `to_markdown()` & `to_markdown_with_options()`
  - `page_chunks = true` producing compatible `PageChunk` structs (`metadata`, `toc_items`, `tables`, `images`, `graphics`, `words`, `text`, `hygiene`)
  - `IdentifyHeaders` (font-size histogram clustering + `max_levels`) and `TocHeaders` (`/Outlines` bookmark mapping)
  - All 4 `table_strategy` modes: `LinesStrict`, `Lines`, `Text`, `Explicit`
  - Recursive X-Y Cut multi-column reading order + `SpatialGrid` $O(1)$ link/cell queries + `UnionFind` multi-table isolation & graphic clustering
  - Bold, italic, bold-italic, strikethrough, inline code, fenced code blocks (`ignore_code`), merged consecutive same-URI links, lists, blockquotes, and `page_separators`
  - Raster image extraction + vector graphic clustering (`write_images`, `embed_images`, `image_path`, `image_format`, `dpi`, `image_size_limit`, `graphics_limit`)
  - `LlamaMarkdownReader` for RAG / LlamaIndex document nodes
- **Text Hygiene & Noise Cleanup (`TextHygieneConfig`)**:
  - Ligature expansion (`ﬀ`, `ﬁ`, `ﬂ`, `ﬃ`, `ﬄ`, `ﬅ`), PUA bullet normalization, unmapped PUA/control/replacement stripping
  - Line-wrap de-hyphenation (`dehyphenate`)
  - `3 Tr` invisible OCR layer suppression (`ignore_invisible_text`)
  - Rotated/translucent watermark removal (`strip_watermarks`)
  - Faux-bold overprint deduplication (`dedup_faux_bold`)
  - Optional cross-page running header/footer deduplication (`dedup_running_headers`)
- **Async & High-Concurrency**:
  - Bounded `tokio::sync::Semaphore` + `rayon` work-stealing CPU pool for parallel page & batch extraction
  - Streaming page chunks via `to_markdown_stream()` (`futures::Stream`)
  - Zero-copy `Arc<PdfDocument>` + lock-free `DashMap` font cache shared across page workers

## Quick Start

```rust
use pymupdf4llm_rs::{
    to_markdown_async, MarkdownOutput, TableStrategy, ToMarkdownOptions,
};

#[tokio::main]
async fn main() -> pymupdf4llm_rs::Result<()> {
    let opts = ToMarkdownOptions::builder()
        .page_chunks(true)
        .table_strategy(TableStrategy::LinesStrict)
        .write_images(false)
        .extract_words(true)
        .max_page_concurrency(8)
        .build();

    match to_markdown_async("paper.pdf", opts).await? {
        MarkdownOutput::Chunks(chunks) => {
            for chunk in chunks {
                println!(
                    "Page {}: {} chars, {} tables",
                    chunk.metadata.page,
                    chunk.text.len(),
                    chunk.tables.len()
                );
            }
        }
        MarkdownOutput::Text(md) => println!("{md}"),
    }
    Ok(())
}
```
