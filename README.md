# pymupdf4llm-rs

[English](README.md) | [简体中文](README.zh-CN.md)

**100% Pure-Rust `pymupdf4llm` PDF-to-Markdown Library & CLI Tool**

`pymupdf4llm-rs` is a zero-C-dependency, pure-Rust reimplementation of [`pymupdf4llm`](https://github.com/pymupdf/pymupdf4llm) designed for high-throughput LLM & RAG document ingestion with Tokio + Rayon concurrency.

---

## Core Highlights

- **100% Pure Rust (`no C/C++ FFI`)**: Built-in PDF 1.0–2.0 binary parser (`XRef`, `ObjStm`, `/Pages` attribute inheritance, `/Subtype /Form` XObject recursion, `FlateDecode`, `LZWDecode`, `ASCIIHexDecode`) using pure-Rust `flate2` (`rust_backend`).
- **Async & High-Concurrency Execution**:
  - Zero-copy `Arc<PdfDocument>` shared across async page tasks.
  - Bounded `tokio::sync::Semaphore` + `rayon` work-stealing CPU pool (`to_markdown_async`, `to_markdown_stream`, `batch_to_markdown_async`).
  - Lock-free `DashMap` font & `ToUnicode` cache across concurrent workers.
- **Full `pymupdf4llm` Parity**:
  - `to_markdown()`, `to_markdown_with_options()`, `to_json()`, `to_text()`, `LlamaMarkdownReader`.
  - `page_chunks = true` emitting `PageChunk` (`metadata`, `toc_items`, `tables`, `images`, `graphics`, `words`, `text`, `hygiene`).
  - `IdentifyHeaders` (character-weighted font histogram) & `TocHeaders` (`/Outlines` bookmark mapping).
  - All 4 `table_strategy` modes (`LinesStrict`, `Lines`, `Text`, `Explicit`), recursive X-Y Cut multi-column reading order, code blocks, lists, and `page_separators`.
- **Advanced Text Hygiene & High-Performance Algorithms (`v0.4.0`)**:
  - **2D `SpatialGrid`** for $O(1)$ average-case hyperlink matching and table cell lookups.
  - **`UnionFind` (DSU)** connected components to isolate multiple independent tables per page and cluster vector graphics in $O(N \cdot \alpha(N))$.
  - **Broad Encoding & CJK Support**: `Type0`/CID `/W` widths, `beginbfchar` & array `beginbfrange` `ToUnicode` CMaps, `MacRomanEncoding`, `WinAnsiEncoding`, 70+ Adobe Glyph names, and `/Rotate` normalization.
  - **Noise & Mojibake Cleanup (`TextHygieneConfig`)**: Ligature expansion (`ﬀ`, `ﬁ`, `ﬂ`, `ﬃ`, `ﬄ`, `ﬅ`), PUA icon normalization & unmapped PUA/control stripping, line-wrap de-hyphenation, `3 Tr` invisible OCR filtering, rotated/translucent watermark filtering, faux-bold deduplication, and optional cross-page running header/footer removal.

---

## Quick Start

### Library Usage

```toml
[dependencies]
pymupdf4llm-rs = { path = "." }
tokio = { version = "1", features = ["full"] }
```

```rust
use pymupdf4llm_rs::{
    to_markdown_async, MarkdownOutput, TableStrategy, ToMarkdownOptions,
};

#[tokio::main]
async fn main() -> pymupdf4llm_rs::Result<()> {
    let opts = ToMarkdownOptions::builder()
        .page_chunks(true)
        .table_strategy(TableStrategy::LinesStrict)
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

### CLI Tool Usage

```bash
cargo run --release --features cli -- paper.pdf --page-chunks --concurrency 8 -o output.json
cargo test --all-features
```

---

## Crate Structure

```text
.
├── Cargo.toml
├── src/
│   ├── lib.rs                 # Public API (to_markdown, to_json, to_text, PageChunk)
│   ├── async_engine.rs        # Tokio semaphore + Rayon bounded concurrency & streaming
│   ├── config.rs              # ToMarkdownOptions, TableStrategy, TextHygieneConfig
│   ├── text_cleaner.rs        # Mojibake, PUA, 3 Tr OCR, watermark, faux-bold & header hygiene
│   ├── llama_reader.rs        # LlamaMarkdownReader integration
│   ├── pdf/                   # Binary lexer, Flate/LZW decoders, fonts/ToUnicode, content stream
│   ├── layout/                # SpatialGrid, UnionFind tables, X-Y Cut columns, header detection
│   ├── markdown/              # PageChunk & Markdown renderer, de-hyphenation, link merging
│   └── bin/pymupdf4llm.rs     # CLI binary
└── tests/
    └── compatibility_tests.rs # Integration tests (multi-column, CJK, Form XObject, hygiene, async)
```

---

## License

Dual-licensed under [Apache-2.0 OR MIT](LICENSE).
