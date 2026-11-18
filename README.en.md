# pymupdf4llm-rs

[简体中文](./README.md) | **English**

[![PyPI](https://img.shields.io/pypi/v/pymupdf4llm-rs?color=blue&label=PyPI)](https://pypi.org/project/pymupdf4llm-rs/)
[![Python](https://img.shields.io/pypi/pyversions/pymupdf4llm-rs)](https://pypi.org/project/pymupdf4llm-rs/)
[![CI](https://github.com/0passion0/pymupdf4llm-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/0passion0/pymupdf4llm-rs/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-green.svg)](./LICENSE)

**100% Pure-Rust High-Concurrency `pymupdf4llm` PDF-to-Markdown Library & CLI**

`pymupdf4llm-rs` is a zero-C-FFI, pure-Rust reimplementation of [`pymupdf4llm`](https://github.com/pymupdf/pymupdf4llm) engineered for LLM & RAG document ingestion pipelines. Powered by **Tokio + Rayon**, it delivers bounded multi-document/multi-page concurrency, GIL-free Python execution, and deep PDF text hygiene without shipping heavyweight C/C++ MuPDF binaries.

---

## Table of Contents

- [1. Highlights & Advantages](#1-highlights--advantages)
- [2. Parsing Pipeline Architecture](#2-parsing-pipeline-architecture)
- [3. Installation (PyPI / GitHub / Cargo)](#3-installation-pypi--github--cargo)
- [4. Python Quick Start & API Reference](#4-python-quick-start--api-reference)
- [5. Rust Library & Web Singleton (`PdfEngine::global`)](#5-rust-library--web-singleton-pdfengineglobal)
- [6. CLI Usage](#6-cli-usage)
- [7. Key Options & Safety Guards](#7-key-options--safety-guards)
- [8. Repository Structure & Development](#8-repository-structure--development)
- [9. License](#9-license)

---

## 1. Highlights & Advantages

| Dimension | `pymupdf4llm-rs` (Pure Rust) | Traditional Python + C Bindings |
|---|---|---|
| **Zero C/C++ FFI** | **100% pure Rust** PDF 1.0–2.0 binary parser & stream decoders (`flate2` rust_backend, LZW, A85, RLE) | Heavy MuPDF / Poppler shared libraries and cross-platform packaging overhead |
| **GIL Release** | Releases the Python GIL (`py.allow_threads`) across parsing and rendering for true multi-threading | Python wrappers and callbacks often contend on the GIL |
| **Parse Once, Export Many** | `PdfExtractor` holds shared Rust pools; `ParsedPdf` (`open()`) exports MD/chunks/JSON/text from one parse | Repeated calls re-read I/O and rebuild page trees |
| **Bounded Concurrency & Safety** | Built-in `max_pdf_bytes`, 64 MB decompression-bomb limit, `max_pages`, and Tokio semaphores | Malformed compressed streams risk OOM or unbounded thread spawning |
| **Spatial Algorithms** | 2D `SpatialGrid` (amortized `O(1)`) + `UnionFind` (DSU) multi-table separation | `O(N²)` scans on dense tables and hyperlink-heavy pages |
| **Text Hygiene** | Built-in ligature expansion, `3 Tr` invisible OCR removal, watermark/faux-bold filtering, running header cleanup | Requires fragile custom post-processing regex pipelines |

---

## 2. Parsing Pipeline Architecture

```text
PDF Path / Raw Bytes / ParsedPdf Handle
  │
  ▼ [Safety Guards: max_pdf_bytes check & PDF Header validation]
[1. Binary Lexer & Page Tree]  XRef / ObjStm / /Pages attribute inheritance / 64MB decompression caps
  │
  ▼ [Lock-free DashMap Font & ToUnicode CMap Cache]
[2. Content Stream & Decoding] Type0 CID /W widths / beginbfchar & beginbfrange / Form XObject recursion
  │
  ▼ [Rayon Page Workers + Bounded Tokio Semaphores]
[3. Spatial Layout Engine]     SpatialGrid (O(1) lookups) · UnionFind table isolation · X-Y Cut columns
  │
  ▼ [TextHygieneConfig Cleanup]
[4. Hygiene & Markdown Render] Ligatures · 3 Tr OCR strip · Watermarks · Faux-bold · Headers/TOC
  │
  ▼
Markdown String / PageChunk List / JSON / LlamaMarkdownReader Documents
```

---

## 3. Installation (PyPI / GitHub / Cargo)

### Python (`>= 3.9`, Prebuilt `abi3` Wheels)

```bash
# Option 1: Install from PyPI (recommended, prebuilt wheels for Linux / macOS / Windows)
pip install pymupdf4llm-rs

# Option 2: Install directly from GitHub source (requires Rust toolchain)
pip install git+https://github.com/0passion0/pymupdf4llm-rs.git

# Option 3: Install prebuilt abi3 wheel from GitHub Releases (example: v0.4.2 Linux x86_64)
pip install https://github.com/0passion0/pymupdf4llm-rs/releases/download/v0.4.2/pymupdf4llm_rs-0.4.2-cp39-abi3-manylinux2014_x86_64.manylinux_2_17_x86_64.whl
```

### Rust Crate / CLI

```toml
# Cargo.toml
[dependencies]
pymupdf4llm-rs = "0.4.2"
tokio = { version = "1", features = ["full"] }
```

```bash
cargo install --path . --features cli
```

---

## 4. Python Quick Start & API Reference

### 4.1 Recommended: Reuse `PdfExtractor` & Parse-Once `ParsedPdf`

Instantiate `PdfExtractor` once at application startup to share `Arc<PdfEngine>`, Tokio runtime, lock-free `ToUnicode` cache, and safety limits across requests:

```python
from pymupdf4llm_rs import LlamaMarkdownReader, PdfExtractor

# 1. Instantiate once and reuse across HTTP workers / batch pipelines
extractor = PdfExtractor(
    table_strategy="lines_strict",
    clean_mojibake=True,
    strip_watermarks=True,
    ignore_invisible_text=True,
    dedup_faux_bold=True,
    max_concurrent_docs=4,
    max_concurrent_pages=8,
    max_pdf_bytes=64 * 1024 * 1024,  # reject oversized payloads before parsing
    max_pages=500,                   # prevent memory exhaustion on huge documents
)

# 2. Reuse across requests (automatically releases Python GIL)
md_text = extractor.to_markdown("paper.pdf")
md_from_bytes = extractor.to_markdown_bytes(pdf_bytes)
chunks = extractor.to_chunks("paper.pdf", extract_words=True)

# 3. Derive lightweight option variants sharing the same underlying engine pool
text_table_extractor = extractor.with_options(table_strategy="text", ignore_code=True)

# 4. Parse once via ParsedPdf and export multiple formats with zero re-parsing
with extractor.open(pdf_bytes) as doc:
    meta = doc.metadata()
    toc = doc.toc()
    md = doc.to_markdown()
    page_chunks = doc.to_chunks()
    json_str = doc.to_json()
    plain_text = doc.to_text()

# 5. Drop-in LlamaIndex-style reader reusing the same extractor
reader = LlamaMarkdownReader(extractor=extractor)
llama_docs = reader.load_data("paper.pdf", extra_info={"source": "arxiv"})
```

### 4.2 Top-Level Convenience Functions

```python
from pymupdf4llm_rs import to_chunks, to_json, to_markdown, to_text

md = to_markdown("paper.pdf", table_strategy="lines_strict", page_separators=True)
chunks = to_chunks("paper.pdf", extract_words=True)
json_str = to_json("paper.pdf")
plain = to_text("paper.pdf")
```

---

## 5. Rust Library & Web Singleton (`PdfEngine::global`)

```rust
use pymupdf4llm_rs::{
    to_markdown_async, MarkdownOutput, PdfEngine, TableStrategy, ToMarkdownOptions,
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

    // Shared process-wide singleton for Axum / Actix / Tonic servers
    let _shared = PdfEngine::global()
        .extract_async("paper.pdf", ToMarkdownOptions::default())
        .await?;
    Ok(())
}
```

---

## 6. CLI Usage

```bash
cargo run --release --features cli -- paper.pdf --page-chunks --concurrency 8 -o output.json
cargo test --features "async,cli,images"
```

---

## 7. Key Options & Safety Guards

| Option | Values / Type | Description |
|---|---|---|
| `table_strategy` | `"lines_strict"` \| `"lines"` \| `"text"` \| `"explicit"` | Table extraction strategy |
| `hdr_info` | `"identify"` \| `"toc"` \| `"disabled"` \| `None` | Header detection via font histogram or `/Outlines` bookmarks |
| `page_chunks` | `bool` | Emit structured per-page dictionaries (`metadata`, `tables`, `words`, `hygiene`) |
| `clean_mojibake` | `bool` | Expand typographic ligatures, normalize PUA icons, strip unmapped control chars |
| `dehyphenate` | `bool` | Rejoin hyphenated line wraps |
| `dedup_faux_bold` | `bool` | Deduplicate synthetic overprint faux-bold spans |
| `ignore_invisible_text` | `bool` | Strip `3 Tr` invisible OCR text layers |
| `strip_watermarks` | `bool` | Filter rotated or low-opacity watermarks |
| `dedup_running_headers` | `bool` | Remove repeating cross-page running headers/footers |
| `max_concurrent_docs` / `max_concurrent_pages` | `Optional[int]` | Bounded document and page concurrency semaphores |
| `max_pdf_bytes` / `max_pages` | `int` / `Optional[int]` | Safety guards capping max input payload size (default `128 MB`) and page count |

---

## 8. Repository Structure & Development

```text
.
├── Cargo.toml                         # Pure-Rust crate manifest & release LTO profile
├── pyproject.toml                     # Maturin PEP 517 Python extension config (abi3-py39)
├── LICENSE / CHANGELOG.md             # Dual license & changelog
├── README.md / README.en.md           # Chinese (default) / English docs
├── .github/workflows/
│   ├── ci.yml                         # rustfmt + clippy + cargo test + maturin + pytest
│   └── release-wheels.yml             # Linux(x64/ARM64) / macOS(Intel/Apple Silicon) / Windows PyPI release
├── python/pymupdf4llm_rs/             # Python wrapper, .pyi type stubs & LlamaMarkdownReader
├── src/                               # Pure-Rust parser, layout, hygiene, async engine, PyO3 & CLI
└── tests/                             # Rust compatibility tests & Python pytest suite
```

```bash
pip install maturin pytest
maturin develop --release
pytest -v tests/test_python_api.py

cargo fmt --all -- --check
cargo clippy --all-targets --features "async,cli,images,python" -- -D warnings
cargo test --features "async,cli,images"
cargo doc --no-deps --features "async,cli,images"
```

---

## 9. License

Dual-licensed under **[Apache-2.0 OR MIT](LICENSE)**.
