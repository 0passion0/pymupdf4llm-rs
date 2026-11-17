import { useState } from 'react';
import { COMPATIBILITY_MATRIX } from '../engine/compatibilityMatrix';
import type { CompatibilityRow } from '../engine/types';

type CategoryFilter = 'All' | CompatibilityRow['category'];

const CATEGORIES: CategoryFilter[] = ['All', 'Core', 'Headers', 'Tables', 'Media', 'Hygiene', 'Output', 'Async'];

export function CompatibilityView() {
  const [filter, setFilter] = useState<CategoryFilter>('All');
  const [search, setSearch] = useState<string>('');

  const rows = COMPATIBILITY_MATRIX.filter((r) => {
    const matchesCat = filter === 'All' || r.category === filter;
    const q = search.trim().toLowerCase();
    const matchesSearch =
      !q ||
      r.pythonParam.toLowerCase().includes(q) ||
      r.rustField.toLowerCase().includes(q) ||
      r.description.toLowerCase().includes(q);
    return matchesCat && matchesSearch;
  });

  return (
    <div className="mx-auto max-w-[1440px] px-4 py-6 sm:px-6">
      <div className="mb-6 border-b border-slate-200 pb-5">
        <h1 className="text-2xl font-bold tracking-tight text-slate-900">
          pymupdf4llm Compatibility &amp; API Parity Specification
        </h1>
        <p className="mt-1 text-sm text-slate-600">
          Direct 1:1 parameter mapping between Python <code className="font-mono text-xs text-slate-800">pymupdf4llm</code> and pure-Rust <code className="font-mono text-xs text-slate-800">pymupdf4llm-rs</code>.
        </p>
      </div>

      {/* Side-by-side Code Parity */}
      <div className="mb-6 grid grid-cols-1 gap-6 lg:grid-cols-2">
        <section className="flex flex-col overflow-hidden rounded-xl border border-slate-200 bg-white">
          <div className="border-b border-slate-200 px-4 py-3">
            <h2 className="text-sm font-semibold text-slate-900">Python pymupdf4llm (Reference)</h2>
          </div>
          <pre className="flex-1 overflow-x-auto bg-slate-950 p-4 font-mono text-[12.5px] leading-relaxed text-slate-100">
            <code>{`import pymupdf4llm

chunks = pymupdf4llm.to_markdown(
    "research_paper.pdf",
    pages=[0, 1, 2],
    page_chunks=True,
    table_strategy="lines_strict",
    write_images=True,
    image_path="assets/",
    extract_words=True,
    margins=(0, 50, 0, 50),
)`}</code>
          </pre>
        </section>

        <section className="flex flex-col overflow-hidden rounded-xl border border-slate-200 bg-white">
          <div className="border-b border-slate-200 px-4 py-3">
            <h2 className="text-sm font-semibold text-slate-900">Rust pymupdf4llm-rs (Sync &amp; Async Parallel)</h2>
          </div>
          <pre className="flex-1 overflow-x-auto bg-slate-950 p-4 font-mono text-[12.5px] leading-relaxed text-slate-100">
            <code>{`use pymupdf4llm_rs::{to_markdown_async, TableStrategy, ToMarkdownOptions};

let opts = ToMarkdownOptions::builder()
    .pages(vec![0, 1, 2])
    .page_chunks(true)
    .table_strategy(TableStrategy::LinesStrict)
    .write_images(true)
    .image_path("assets/")
    .extract_words(true)
    .margins((0.0, 50.0, 0.0, 50.0))
    .max_page_concurrency(8)
    .build();

let chunks = to_markdown_async("research_paper.pdf", opts).await?;`}</code>
          </pre>
        </section>
      </div>

      {/* Filter Controls + Matrix Table */}
      <div className="rounded-xl border border-slate-200 bg-white">
        <div className="flex flex-wrap items-center justify-between gap-4 border-b border-slate-200 px-4 py-3">
          <div className="flex flex-wrap items-center gap-1 rounded-lg bg-slate-100 p-1">
            {CATEGORIES.map((cat) => (
              <button
                key={cat}
                type="button"
                onClick={() => setFilter(cat)}
                className={`min-h-8 rounded-md px-3 py-1.5 text-xs font-medium transition-colors whitespace-nowrap shrink-0 ${
                  filter === cat
                    ? 'bg-white text-slate-900 shadow-xs'
                    : 'text-slate-600 hover:text-slate-900'
                }`}
              >
                {cat}
              </button>
            ))}
          </div>

          <div className="w-full sm:w-64">
            <label htmlFor="compat-search" className="sr-only">
              Search parameters
            </label>
            <input
              id="compat-search"
              type="search"
              placeholder="Filter parameters..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="min-h-9 w-full rounded-lg border border-slate-200 bg-white px-3 py-1.5 text-xs text-slate-900 focus:border-blue-600 focus:outline-none"
            />
          </div>
        </div>

        <div className="overflow-x-auto">
          <table className="w-full border-collapse text-left text-xs">
            <thead className="border-b border-slate-200 bg-slate-50">
              <tr>
                <th className="px-4 py-3 font-semibold text-slate-900">Python Parameter</th>
                <th className="px-4 py-3 font-semibold text-slate-900">Rust Struct Field / API</th>
                <th className="px-4 py-3 font-semibold text-slate-900">Default</th>
                <th className="px-4 py-3 font-semibold text-slate-900">Category</th>
                <th className="px-4 py-3 font-semibold text-slate-900">Behavior</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-200 bg-white">
              {rows.map((row) => (
                <tr key={row.pythonParam} className="hover:bg-slate-50">
                  <td className="px-4 py-3 font-mono font-medium text-slate-900">{row.pythonParam}</td>
                  <td className="px-4 py-3 font-mono text-blue-700">{row.rustField}</td>
                  <td className="px-4 py-3 font-mono text-slate-600">{row.defaultValue}</td>
                  <td className="px-4 py-3 text-slate-600">{row.category}</td>
                  <td className="px-4 py-3 text-slate-700">{row.description}</td>
                </tr>
              ))}
              {rows.length === 0 && (
                <tr>
                  <td colSpan={5} className="px-4 py-8 text-center text-slate-500">
                    No matching parameters found.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  );
}
