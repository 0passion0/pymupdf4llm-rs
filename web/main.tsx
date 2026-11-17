import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import './index.css';

function App() {
  return (
    <main className="mx-auto max-w-3xl px-6 py-12 text-slate-900">
      <div className="rounded-xl border border-slate-200 bg-white p-8 shadow-xs">
        <div className="flex flex-wrap items-center justify-between gap-4 border-b border-slate-200 pb-6">
          <div>
            <h1 className="font-mono text-2xl font-bold tracking-tight text-slate-900">
              pymupdf4llm-rs
            </h1>
            <p className="mt-1 text-sm text-slate-600">
              100% Pure-Rust PDF-to-Markdown Core + Reusable Python Binding (PyO3 / maturin)
            </p>
          </div>
          <a
            href="/api/crate/download"
            className="inline-flex min-h-10 items-center rounded-lg bg-slate-900 px-4 py-2 text-xs font-semibold text-white transition-colors hover:bg-slate-800 whitespace-nowrap shrink-0"
          >
            Download Source (.tar.gz)
          </a>
        </div>

        <div className="mt-6 space-y-5 text-sm text-slate-700">
          <div>
            <h2 className="text-xs font-semibold tracking-wide text-slate-500 uppercase">
              Python Reusable Class (PdfExtractor + ParsedPdf, GIL Released)
            </h2>
            <pre className="mt-2 overflow-x-auto rounded-lg bg-slate-950 p-4 font-mono text-xs leading-relaxed text-slate-100">
{`from pymupdf4llm_rs import PdfExtractor

# Instantiate once: holds Arc<PdfEngine>, bounded semaphores, & CMap cache
extractor = PdfExtractor(
    table_strategy="lines_strict",
    clean_mojibake=True,
    max_concurrent_docs=4,
    max_concurrent_pages=8,
    max_pdf_bytes=64 * 1024 * 1024,
)

# Reuse across requests or parse once for multiple views
md = extractor.to_markdown_bytes(pdf_bytes)
with extractor.open(pdf_bytes) as doc:
    chunks = doc.to_chunks(extract_words=True)
    plain = doc.to_text()`}
            </pre>
          </div>

          <div>
            <h2 className="text-xs font-semibold tracking-wide text-slate-500 uppercase">
              Rust Global Singleton (PdfEngine::global)
            </h2>
            <pre className="mt-2 overflow-x-auto rounded-lg bg-slate-950 p-4 font-mono text-xs leading-relaxed text-slate-100">
{`use pymupdf4llm_rs::{PdfEngine, ToMarkdownOptions};

let output = PdfEngine::global()
    .extract_bytes(pdf_bytes, ToMarkdownOptions::default())
    .await?;`}
            </pre>
          </div>

          <div>
            <h2 className="text-xs font-semibold tracking-wide text-slate-500 uppercase">
              Build &amp; Test Commands
            </h2>
            <pre className="mt-2 overflow-x-auto rounded-lg bg-slate-950 p-4 font-mono text-xs leading-relaxed text-slate-100">
{`maturin develop --release && pytest -v tests/test_python_api.py
cargo test --features "async,cli,images"`}
            </pre>
          </div>
        </div>
      </div>
    </main>
  );
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>
);
