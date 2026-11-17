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

### Web Service / High-Concurrency Singleton (`PdfEngine::global`)

For web backends (Axum / Actix / Tonic) handling many concurrent uploads, use the shared `PdfEngine` singleton (`OnceLock<Arc<PdfEngine>>`) to cap global in-flight documents/pages and reuse identical parsed `ToUnicode` CMaps safely across requests:

```rust
use pymupdf4llm_rs::{PdfEngine, ToMarkdownOptions};

let output = PdfEngine::global()
    .extract_bytes(pdf_bytes, ToMarkdownOptions::default())
    .await?;
```

### Python Package (PyO3 + `maturin`, Reusable Class & GIL-Released)

Build and install locally into your Python environment (`Python >= 3.9`, `abi3`):

```bash
pip install maturin pytest
maturin develop --release
pytest -v tests/test_python_api.py
```

Instantiate `PdfExtractor` once per worker/service to hold shared Rust resources (`Arc<PdfEngine>`, bounded semaphores, Tokio runtime, global `ToUnicode` CMap cache, and `max_pdf_bytes` / `max_pages` safety limits), and reuse its methods across requests:

```python
from pymupdf4llm_rs import PdfExtractor

# Instantiate once and reuse across HTTP requests / batch workers
extractor = PdfExtractor(
    table_strategy="lines_strict",
    clean_mojibake=True,
    strip_watermarks=True,
    ignore_invisible_text=True,
    max_concurrent_docs=4,
    max_concurrent_pages=8,
    max_pdf_bytes=64 * 1024 * 1024,  # reject oversized payloads before parsing
    max_pages=500,
)

# Reuse configured defaults directly (Python GIL released during Rust parsing)
md = extractor.to_markdown("paper.pdf")
md_from_bytes = extractor.to_markdown_bytes(pdf_bytes)
chunks = extractor.to_chunks("paper.pdf", extract_words=True)

# Lightweight derived preset sharing the SAME underlying Arc<PdfEngine>
fast_text_extractor = extractor.with_options(table_strategy="text", ignore_code=True)

# Or parse once into ParsedPdf and extract multiple views without re-parsing
with extractor.open(pdf_bytes) as doc:
    meta = doc.metadata()
    toc = doc.toc()
    md = doc.to_markdown()
    chunks = doc.to_chunks()
    json_str = doc.to_json()
    plain = doc.to_text()
```

### CLI Tool Usage

```bash
cargo run --release --features cli -- paper.pdf --page-chunks --concurrency 8 -o output.json
cargo test --features "async,cli,images"
```

---

## Crate Structure

```text
.
├── Cargo.toml
├── pyproject.toml                 # Maturin build configuration for Python wheel (abi3-py39)
├── python/pymupdf4llm_rs/         # Python wrapper, PEP 484 .pyi stubs, LlamaMarkdownReader
├── src/
│   ├── lib.rs                     # Public API (to_markdown, to_json, to_text, PageChunk)
│   ├── python.rs                  # PyO3 native extension (PdfExtractor, ParsedPdf, GIL release)
│   ├── async_engine.rs            # Tokio semaphore + Rayon bounded concurrency & streaming
│   ├── config.rs                  # ToMarkdownOptions, TableStrategy, TextHygieneConfig
│   ├── text_cleaner.rs            # Mojibake, PUA, 3 Tr OCR, watermark, faux-bold & header hygiene
│   ├── llama_reader.rs            # LlamaMarkdownReader integration
│   ├── pdf/                       # Binary lexer, Flate/LZW/A85/RLE decoders (64MB capped), CMap cache
│   ├── layout/                    # SpatialGrid, UnionFind tables, X-Y Cut columns, header detection
│   ├── markdown/                  # PageChunk & Markdown renderer, de-hyphenation, link merging
│   └── bin/pymupdf4llm.rs         # CLI binary
└── tests/
    ├── compatibility_tests.rs     # Rust integration tests (multi-column, CJK, Form XObject, async)
    └── test_python_api.py         # Pytest suite for PdfExtractor, ParsedPdf, safety bounds & threads
```

---

## License

Dual-licensed under [Apache-2.0 OR MIT](LICENSE).
