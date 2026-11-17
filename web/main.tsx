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
              100% Pure-Rust PDF-to-Markdown Crate &amp; CLI (`Cargo.toml` + `src/` + `tests/`)
            </p>
          </div>
          <a
            href="/api/crate/download"
            className="inline-flex min-h-10 items-center rounded-lg bg-slate-900 px-4 py-2 text-xs font-semibold text-white transition-colors hover:bg-slate-800 whitespace-nowrap shrink-0"
          >
            Download Crate (.tar.gz)
          </a>
        </div>

        <div className="mt-6 space-y-5 text-sm text-slate-700">
          <div>
            <h2 className="text-xs font-semibold tracking-wide text-slate-500 uppercase">
              Web Server Global Singleton Call (`PdfEngine::global`)
            </h2>
            <pre className="mt-2 overflow-x-auto rounded-lg bg-slate-950 p-4 font-mono text-xs leading-relaxed text-slate-100">
{`use pymupdf4llm_rs::{PdfEngine, ToMarkdownOptions};

// Shared across all HTTP handlers: bounds in-flight docs/pages globally
// and reuses parsed Type0 CJK ToUnicode CMaps via content hash.
let output = PdfEngine::global()
    .extract_bytes(pdf_bytes, ToMarkdownOptions::default())
    .await?;`}
            </pre>
          </div>

          <div>
            <h2 className="text-xs font-semibold tracking-wide text-slate-500 uppercase">
              CLI &amp; Rust Tests
            </h2>
            <pre className="mt-2 overflow-x-auto rounded-lg bg-slate-950 p-4 font-mono text-xs leading-relaxed text-slate-100">
{`cargo test --all-features
cargo run --release --features cli -- paper.pdf --page-chunks --concurrency 8`}
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
