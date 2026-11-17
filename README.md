# pymupdf4llm-rs

[English](README.md) | [简体中文](README.zh-CN.md)

**100% Pure-Rust `pymupdf4llm` PDF-to-Markdown Engine & Interactive Compatibility Workbench**

`pymupdf4llm-rs` is a zero-C-dependency, pure-Rust reimplementation of [`pymupdf4llm`](https://github.com/pymupdf/pymupdf4llm) designed for high-throughput LLM & RAG document ingestion, paired with a browser-based interactive parser & compatibility workbench.

---

## Core Highlights

- **100% Pure Rust (`no C/C++ FFI`)**: Native PDF 1.0–2.0 binary parser (`XRef`, `ObjStm`, `/Pages` attribute inheritance, `/Subtype /Form` XObject recursion, `FlateDecode`, `LZWDecode`, `ASCIIHexDecode`) with `flate2` (`rust_backend`).
- **Async & High-Concurrency Execution**:
  - Zero-copy `Arc<PdfDocument>` shared across async workers.
  - Bounded `tokio::sync::Semaphore` + `rayon` work-stealing CPU pool for deterministic page-level & batch concurrency (`to_markdown_async`, `to_markdown_stream`, `batch_to_markdown_async`).
  - `DashMap` lock-free font/ToUnicode cache across concurrent page workers.
- **Full `pymupdf4llm` Output & Option Parity**:
  - `to_markdown()`, `to_markdown_with_options()`, `PageChunk` (`metadata`, `toc_items`, `tables`, `images`, `graphics`, `words`, `text`, `hygiene`).
  - `IdentifyHeaders` (character-weighted font-size histogram) & `TocHeaders` (`/Outlines` bookmark mapping).
  - All 4 `table_strategy` modes: `LinesStrict`, `Lines`, `Text`, `Explicit`.
  - Recursive X-Y Cut multi-column reading order segmentation, fenced code blocks, lists, adjacent hyperlink merging, and `LlamaMarkdownReader`.
- **Advanced Text Hygiene & High-Performance Algorithms (`v0.4.0`)**:
  - **2D `SpatialGrid`** for $O(1)$ average-case hyperlink matching and table cell lookups.
  - **`UnionFind` (DSU)** connected components to isolate multiple independent tables per page and cluster vector graphics in $O(N \cdot \alpha(N))$.
  - **Broad Encoding & CJK Support**: `Type0`/CID `/W` width parsing, `beginbfchar` & array `beginbfrange` `ToUnicode` CMaps, `MacRomanEncoding`, `WinAnsiEncoding`, 70+ Adobe Glyph names, and `/Rotate` normalization.
  - **Noise & Mojibake Cleanup**: Ligature expansion (`ﬀ`, `ﬁ`, `ﬂ`, `ﬃ`, `ﬄ`, `ﬅ`), PUA icon normalization & unmapped PUA/control stripping, line-wrap de-hyphenation, `3 Tr` invisible OCR filtering, diagonal/translucent watermark filtering, faux-bold deduplication, and optional cross-page running header/footer removal.

---

## Quick Start

### 1. Rust Crate (`rust/pymupdf4llm-rs`)

```toml
[dependencies]
pymupdf4llm-rs = { path = "./rust/pymupdf4llm-rs" }
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

Run Rust tests and CLI:

```bash
cd rust/pymupdf4llm-rs
cargo test --all-features
cargo run --release --features cli -- ../../sample.pdf --page-chunks --concurrency 8
```

### 2. Interactive Web Workbench (Express + Vite + React)

```bash
npm install
npm run dev
```

Open `http://localhost:3000` to inspect multi-column reading order, `ToMarkdownOptions` toggles, `Text Hygiene & Diagnostics`, `Page Chunks JSON`, and browse/download the full pure-Rust crate source.

Run TypeScript engine checks:

```bash
npm run lint
npx tsx --test src/engine/pdfEngine.test.ts
npm run build
```

---

## Repository Layout

```text
.
├── .github/                  # GitHub Actions CI, Issue Forms, PR Template (EN/ZH)
├── rust/pymupdf4llm-rs/      # 100% pure-Rust crate (Tokio + Rayon + SpatialGrid + UnionFind)
│   ├── src/
│   │   ├── pdf/              # Binary lexer, Flate/LZW decoders, fonts/ToUnicode, content stream
│   │   ├── layout/           # SpatialGrid, UnionFind tables, X-Y Cut columns, header detection
│   │   ├── markdown/         # PageChunk & Markdown renderer, de-hyphenation, link merging
│   │   ├── text_cleaner.rs   # Mojibake, PUA, 3 Tr OCR, watermark, faux-bold & header hygiene
│   │   ├── async_engine.rs   # Tokio semaphore + Rayon bounded concurrency & streaming
│   │   └── bin/              # CLI binary (`pymupdf4llm`)
│   └── tests/                # Compatibility & hygiene integration tests
├── src/
│   ├── engine/               # TypeScript mirror engine + synthetic PDF fixtures + node:test suite
│   └── components/           # Workbench, Async Pipeline, Rust Crate Explorer, Parity Matrix
├── server.ts                 # Express + Vite server & `/api/crate/download` tar.gz packager
├── README.md                 # English documentation
├── README.zh-CN.md           # 简体中文文档
├── CONTRIBUTING.md           # Bilingual contribution guidelines
├── CHANGELOG.md              # Bilingual release history
├── SECURITY.md               # Bilingual security policy
└── LICENSE                   # Apache-2.0 OR MIT dual license
```

---

## License

Dual-licensed under [Apache-2.0 OR MIT](LICENSE).
