# pymupdf4llm-rs

100% pure-Rust implementation of [pymupdf4llm](https://github.com/pymupdf/pymupdf4llm) with complete API/output compatibility and native Tokio + Rayon high-concurrency async processing.

## Key Capabilities

- **100% Pure Rust**: Zero C/C++ bindings (`no mupdf-sys`, `no pdfium-render`). Uses `flate2` (`rust_backend`), native PDF 1.0–2.0 binary/XRef/ObjStm parser, ToUnicode CMap decoder, and content stream state machine.
- **Full `pymupdf4llm` Parity**:
  - `to_markdown()` & `to_markdown_with_options()`
  - `page_chunks = true` producing identical `PageChunk` structs (`metadata`, `toc_items`, `tables`, `images`, `graphics`, `words`, `text`)
  - `IdentifyHeaders` (font-size histogram clustering + `max_levels`) and `TocHeaders` (PDF bookmark/outline mapping)
  - All 4 `table_strategy` modes: `LinesStrict`, `Lines`, `Text`, `Explicit`
  - Multi-column reading order via recursive X-Y cut column segmentation
  - Bold, italic, bold-italic, strikethrough, inline code, fenced code blocks (`ignore_code`), links, lists, blockquotes, and `page_separators`
  - Raster image extraction + vector graphic clustering (`write_images`, `embed_images`, `image_path`, `image_format`, `dpi`, `image_size_limit`, `graphics_limit`)
  - `LlamaMarkdownReader` for RAG / LlamaIndex document nodes
- **Async & High-Concurrency**:
  - Bounded `tokio::sync::Semaphore` + `rayon` work-stealing CPU pool for parallel page & batch extraction
  - Streaming page chunks via `to_markdown_stream()` (`futures::Stream`)
  - Zero-copy `Arc<PdfDocument>` shared across page workers

## Quick Start

```rust
use pymupdf4llm_rs::{
    to_markdown, to_markdown_async, batch_to_markdown_async,
    ToMarkdownOptions, TableStrategy, MarkdownOutput,
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
                println!("Page {}: {} chars, {} tables",
                    chunk.metadata.page, chunk.text.len(), chunk.tables.len());
            }
        }
        MarkdownOutput::Text(md) => println!("{md}"),
    }
    Ok(())
}
```
