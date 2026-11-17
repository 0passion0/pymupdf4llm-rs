import { useEffect, useState } from 'react';
import { Check, Copy, FileCode2 } from 'lucide-react';
import type { CrateFileEntry } from '../engine/types';

interface CrateApiResponse {
  crateName: string;
  version: string;
  fileCount: number;
  totalBytes: number;
  totalLines: number;
  files: Array<CrateFileEntry & { content: string }>;
}

export function RustCrateView() {
  const [crateData, setCrateData] = useState<CrateApiResponse | null>(null);
  const [selectedPath, setSelectedPath] = useState<string>('Cargo.toml');
  const [copied, setCopied] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetch('/api/crate/files')
      .then((res) => {
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        return res.json() as Promise<CrateApiResponse>;
      })
      .then((data) => {
        setCrateData(data);
        if (data.files.length > 0 && !data.files.some((f) => f.path === 'Cargo.toml')) {
          setSelectedPath(data.files[0].path);
        }
      })
      .catch((err) => {
        setError(err instanceof Error ? err.message : 'Unable to load Rust crate sources.');
      });
  }, []);

  const activeFile = crateData?.files.find((f) => f.path === selectedPath) ?? crateData?.files[0];

  const handleCopy = async () => {
    if (!activeFile) return;
    await navigator.clipboard.writeText(activeFile.content);
    setCopied(true);
    setTimeout(() => setCopied(false), 1800);
  };

  const [cliCopied, setCliCopied] = useState<boolean>(false);
  const handleCopyCli = async () => {
    await navigator.clipboard.writeText('cargo run --release --features cli -- paper.pdf --page-chunks --concurrency 8');
    setCliCopied(true);
    setTimeout(() => setCliCopied(false), 1800);
  };

  return (
    <div className="mx-auto max-w-[1440px] px-4 py-6 sm:px-6">
      <div className="mb-6 flex flex-col justify-between gap-4 border-b border-slate-200 pb-5 md:flex-row md:items-end">
        <div>
          <h1 className="text-2xl font-bold tracking-tight text-slate-900">
            Pure-Rust Crate Source (<code className="font-mono text-xl">pymupdf4llm-rs</code>)
          </h1>
          <p className="mt-1 text-sm text-slate-600">
            100% pure Rust implementation with zero C/C++ bindings · Powered by <code className="font-mono text-xs text-slate-800">flate2</code>, <code className="font-mono text-xs text-slate-800">tokio</code>, and <code className="font-mono text-xs text-slate-800">rayon</code>.
          </p>
        </div>

        <button
          type="button"
          onClick={handleCopyCli}
          className="inline-flex min-h-10 items-center gap-2 rounded-lg border border-slate-200 bg-white px-4 py-2 text-xs font-medium text-slate-700 transition-colors hover:bg-slate-50 whitespace-nowrap shrink-0"
        >
          {cliCopied ? (
            <>
              <Check className="h-3.5 w-3.5 text-emerald-600" aria-hidden="true" />
              <span>CLI Command Copied</span>
            </>
          ) : (
            <>
              <Copy className="h-3.5 w-3.5" aria-hidden="true" />
              <span>Copy Cargo CLI Command</span>
            </>
          )}
        </button>
      </div>

      {error && (
        <div role="alert" className="rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700">
          {error}
        </div>
      )}

      {!crateData && !error ? (
        <div className="rounded-xl border border-slate-200 bg-white p-12 text-center text-sm text-slate-500">
          Loading pure-Rust crate files...
        </div>
      ) : crateData ? (
        <div className="space-y-4">
          <div className="grid grid-cols-2 divide-y divide-slate-200 rounded-xl border border-slate-200 bg-white sm:grid-cols-4 sm:divide-x sm:divide-y-0">
            <div className="px-4 py-3">
              <div className="text-xs text-slate-500">Crate Files</div>
              <div className="mt-1 font-mono text-base font-semibold tabular-nums text-slate-900">
                {crateData.fileCount} files
              </div>
            </div>
            <div className="px-4 py-3">
              <div className="text-xs text-slate-500">Total Source Lines</div>
              <div className="mt-1 font-mono text-base font-semibold tabular-nums text-slate-900">
                {crateData.totalLines.toLocaleString()} LOC
              </div>
            </div>
            <div className="px-4 py-3">
              <div className="text-xs text-slate-500">FFI / C Dependencies</div>
              <div className="mt-1 font-mono text-base font-semibold text-emerald-700">
                0 (100% Safe Rust)
              </div>
            </div>
            <div className="px-4 py-3">
              <div className="text-xs text-slate-500">Async &amp; Parallel Runtime</div>
              <div className="mt-1 font-mono text-base font-semibold text-slate-900">
                Tokio + Rayon
              </div>
            </div>
          </div>

          <div className="grid grid-cols-1 gap-6 lg:grid-cols-12">
            <aside className="rounded-xl border border-slate-200 bg-white p-4 lg:col-span-4 xl:col-span-3">
              <h2 className="px-2 text-xs font-semibold text-slate-900">Crate File Tree</h2>
              <nav aria-label="Rust crate files" className="mt-3 space-y-1">
                {crateData.files.map((file) => {
                  const active = activeFile?.path === file.path;
                  return (
                    <button
                      key={file.path}
                      type="button"
                      onClick={() => setSelectedPath(file.path)}
                      className={`flex w-full items-center justify-between gap-2 rounded-lg px-2.5 py-2 text-left font-mono text-xs transition-colors ${
                        active
                          ? 'bg-blue-50 font-semibold text-blue-700'
                          : 'text-slate-700 hover:bg-slate-50 hover:text-slate-900'
                      }`}
                    >
                      <span className="flex min-w-0 items-center gap-2">
                        <FileCode2 className="h-3.5 w-3.5 shrink-0 text-slate-400" aria-hidden="true" />
                        <span className="truncate">{file.path}</span>
                      </span>
                      <span className="shrink-0 text-[11px] tabular-nums text-slate-400">
                        {file.lines}L
                      </span>
                    </button>
                  );
                })}
              </nav>
            </aside>

            <section className="flex flex-col overflow-hidden rounded-xl border border-slate-200 bg-white lg:col-span-8 xl:col-span-9">
              {activeFile && (
                <>
                  <div className="flex items-center justify-between gap-4 border-b border-slate-200 px-4 py-3">
                    <div className="min-w-0">
                      <div className="truncate font-mono text-sm font-semibold text-slate-900">
                        {activeFile.path}
                      </div>
                      <div className="mt-0.5 text-xs tabular-nums text-slate-500">
                        {activeFile.lines} lines · {(activeFile.sizeBytes / 1024).toFixed(1)} KB
                      </div>
                    </div>
                    <button
                      type="button"
                      onClick={handleCopy}
                      className="inline-flex min-h-9 items-center gap-1.5 rounded-lg border border-slate-200 bg-white px-3 py-1.5 text-xs font-medium text-slate-700 hover:bg-slate-50 whitespace-nowrap shrink-0"
                    >
                      {copied ? (
                        <>
                          <Check className="h-3.5 w-3.5 text-emerald-600" aria-hidden="true" />
                          <span>Copied</span>
                        </>
                      ) : (
                        <>
                          <Copy className="h-3.5 w-3.5" aria-hidden="true" />
                          <span>Copy File</span>
                        </>
                      )}
                    </button>
                  </div>

                  <div className="max-h-[680px] overflow-auto bg-slate-950 p-4">
                    <pre className="font-mono text-[12.5px] leading-relaxed text-slate-100">
                      <code>{activeFile.content}</code>
                    </pre>
                  </div>
                </>
              )}
            </section>
          </div>
        </div>
      ) : null}
    </div>
  );
}
