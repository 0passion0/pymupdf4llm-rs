import React from "react";
import { createRoot } from "react-dom/client";
import "./index.css";

function App() {
  return (
    <main className="min-h-screen bg-slate-50 text-slate-900 flex items-center justify-center p-6">
      <section className="max-w-2xl w-full bg-white border border-slate-200 rounded-xl shadow-sm p-8 space-y-5">
        <div className="space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight text-slate-900">
            pymupdf4llm-rs
          </h1>
          <p className="text-sm text-slate-600">
            100% Pure-Rust PDF-to-Markdown engine with PyO3 Python bindings, Tokio + Rayon
            concurrency, and CJK / table extraction.
          </p>
        </div>

        <div className="rounded-lg bg-slate-900 text-slate-100 p-4 font-mono text-xs overflow-x-auto leading-relaxed">
          <pre>{`pip install pymupdf4llm-rs

from pymupdf4llm_rs import PdfExtractor

extractor = PdfExtractor(
    table_strategy="lines_strict",
    clean_mojibake=True,
    strip_watermarks=True,
    ignore_invisible_text=True,
)
md = extractor.to_markdown("document.pdf")`}</pre>
        </div>
      </section>
    </main>
  );
}

const rootEl = document.getElementById("root");
if (rootEl) {
  createRoot(rootEl).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
}
