import React, { useEffect, useState } from 'react';
import { Copy, Check, FileUp, Play, Download } from 'lucide-react';
import {
  DEFAULT_OPTIONS,
  type HeaderMode,
  type ParseExecutionResult,
  type TableStrategy,
  type TextHygieneConfig,
  type ToMarkdownOptions,
} from '../engine/types';
import { generateSamplePdf, SAMPLE_PDF_PRESETS } from '../engine/samplePdfs';
import { runPyMuPdf4LlmPipeline } from '../engine/pymupdf4llmPipeline';
import { MarkdownRenderer } from './MarkdownRenderer';

type OutputTab = 'preview' | 'raw' | 'chunks' | 'inspection';

export function WorkbenchView() {
  const [selectedPreset, setSelectedPreset] = useState<string>('research-paper');
  const [uploadedFile, setUploadedFile] = useState<{ name: string; bytes: Uint8Array } | null>(null);
  const [options, setOptions] = useState<ToMarkdownOptions>(DEFAULT_OPTIONS);
  const [pagesInput, setPagesInput] = useState<string>('');
  const [outputTab, setOutputTab] = useState<OutputTab>('preview');
  const [result, setResult] = useState<ParseExecutionResult | null>(null);
  const [isParsing, setIsParsing] = useState<boolean>(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [copied, setCopied] = useState<boolean>(false);

  const runParse = async (
    nextOptions: ToMarkdownOptions = options,
    presetId: string = selectedPreset,
    customFile: { name: string; bytes: Uint8Array } | null = uploadedFile
  ) => {
    setIsParsing(true);
    setErrorMsg(null);
    try {
      const payload = customFile
        ? { fileName: customFile.name, bytes: customFile.bytes }
        : generateSamplePdf(presetId);

      const parsedPages = pagesInput
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean)
        .map((s) => Number.parseInt(s, 10))
        .filter((n) => Number.isFinite(n) && n >= 0);

      const effectiveOpts: ToMarkdownOptions = {
        ...nextOptions,
        pages: parsedPages.length > 0 ? parsedPages : null,
      };

      const res = await runPyMuPdf4LlmPipeline(payload.bytes, payload.fileName, effectiveOpts);
      setResult(res);
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : 'Failed to parse PDF bytes.');
    } finally {
      setIsParsing(false);
    }
  };

  useEffect(() => {
    void runParse(DEFAULT_OPTIONS, 'research-paper', null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleSelectPreset = (presetId: string) => {
    setUploadedFile(null);
    setSelectedPreset(presetId);
    const preset = SAMPLE_PDF_PRESETS.find((p) => p.id === presetId);
    const tightMargins = presetId === 'financial-report' || presetId === 'noisy-cjk-multitable';
    const nextOpts: ToMarkdownOptions = {
      ...options,
      tableStrategy: preset?.recommendedStrategy ?? options.tableStrategy,
      margins: tightMargins ? { left: 0, top: 10, right: 0, bottom: 10 } : DEFAULT_OPTIONS.margins,
      hygiene:
        presetId === 'noisy-cjk-multitable'
          ? { ...options.hygiene, dedupRunningHeaders: true }
          : options.hygiene,
    };
    setOptions(nextOpts);
    void runParse(nextOpts, presetId, null);
  };

  const handleFileUpload = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const buffer = await file.arrayBuffer();
    const custom = { name: file.name, bytes: new Uint8Array(buffer) };
    setUploadedFile(custom);
    void runParse(options, selectedPreset, custom);
  };

  const updateOption = <K extends keyof ToMarkdownOptions>(key: K, value: ToMarkdownOptions[K]) => {
    const next = { ...options, [key]: value };
    setOptions(next);
    void runParse(next, selectedPreset, uploadedFile);
  };

  const updateHygiene = <K extends keyof TextHygieneConfig>(key: K, value: boolean) => {
    const next: ToMarkdownOptions = {
      ...options,
      hygiene: { ...options.hygiene, [key]: value },
    };
    setOptions(next);
    void runParse(next, selectedPreset, uploadedFile);
  };

  const handleCopy = async () => {
    if (!result) return;
    const textToCopy =
      outputTab === 'chunks'
        ? JSON.stringify(result.chunks, null, 2)
        : result.markdown;
    await navigator.clipboard.writeText(textToCopy);
    setCopied(true);
    setTimeout(() => setCopied(false), 1800);
  };

  const handleDownloadSamplePdf = () => {
    const sample = uploadedFile
      ? { fileName: uploadedFile.name, bytes: uploadedFile.bytes }
      : generateSamplePdf(selectedPreset);
    const blob = new Blob([new Uint8Array(sample.bytes)], { type: 'application/pdf' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = sample.fileName;
    a.click();
    URL.revokeObjectURL(url);
  };

  const handleDownloadMarkdown = () => {
    if (!result) return;
    const blob = new Blob([result.markdown], { type: 'text/markdown;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = result.fileName.replace(/\.pdf$/i, '.md');
    a.click();
    URL.revokeObjectURL(url);
  };

  const totalTables = result?.chunks.reduce((acc, c) => acc + c.tables.length, 0) ?? 0;
  const totalWords = result?.chunks.reduce((acc, c) => acc + (c.words?.length ?? 0), 0) ?? 0;
  const hs = result?.hygieneStats;
  const hygieneCleanedTotal = hs
    ? hs.mojibakeCharsRemoved +
      hs.ligaturesExpanded +
      hs.fauxBoldDeduped +
      hs.watermarksFiltered +
      hs.invisibleSpansFiltered +
      hs.dehyphenatedJoins +
      hs.runningHeadersRemoved
    : 0;

  return (
    <div className="mx-auto max-w-[1440px] px-4 py-6 sm:px-6">
      <div className="mb-6 flex flex-col justify-between gap-4 border-b border-slate-200 pb-5 md:flex-row md:items-end">
        <div>
          <h1 className="text-2xl font-bold tracking-tight text-slate-900">
            PDF-to-Markdown Compatibility Workbench
          </h1>
          <p className="mt-1 text-sm text-slate-600">
            Live pure-binary parser mirroring <code className="font-mono text-xs text-slate-800">pymupdf4llm-rs</code> multi-column X-Y cut ordering, Union-Find multi-table grids, CJK ToUnicode, and mojibake hygiene.
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-3">
          <button
            type="button"
            onClick={handleDownloadSamplePdf}
            className="inline-flex min-h-10 items-center gap-2 rounded-lg border border-slate-200 bg-white px-4 py-2 text-xs font-medium text-slate-700 transition-colors hover:bg-slate-50 whitespace-nowrap shrink-0"
          >
            <Download className="h-3.5 w-3.5" aria-hidden="true" />
            <span>Export Active PDF</span>
          </button>
          <button
            type="button"
            onClick={() => void runParse()}
            disabled={isParsing}
            className="inline-flex min-h-10 items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-xs font-semibold text-white transition-colors hover:bg-blue-700 disabled:opacity-60 whitespace-nowrap shrink-0"
          >
            <Play className="h-3.5 w-3.5" aria-hidden="true" />
            <span>{isParsing ? 'Parsing...' : 'Run Extraction'}</span>
          </button>
        </div>
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-12">
        <aside className="space-y-6 lg:col-span-4 xl:col-span-3">
          <section className="rounded-xl border border-slate-200 bg-white p-5">
            <h2 className="text-sm font-semibold text-slate-900">01. PDF Input Source</h2>
            <div className="mt-3 space-y-2">
              {SAMPLE_PDF_PRESETS.map((preset) => {
                const active = !uploadedFile && selectedPreset === preset.id;
                return (
                  <button
                    key={preset.id}
                    type="button"
                    onClick={() => handleSelectPreset(preset.id)}
                    className={`w-full rounded-lg border p-3 text-left transition-colors ${
                      active
                        ? 'border-blue-600 bg-blue-50/50 text-slate-900'
                        : 'border-slate-200 bg-white text-slate-700 hover:bg-slate-50'
                    }`}
                  >
                    <div className="text-xs font-semibold text-slate-900">{preset.title}</div>
                    <div className="mt-1 text-xs text-slate-500">
                      {preset.pages} pages · {preset.subtitle}
                    </div>
                  </button>
                );
              })}
            </div>

            <div className="mt-4 border-t border-slate-200 pt-4">
              <label
                htmlFor="pdf-file-upload"
                className="flex min-h-10 cursor-pointer items-center justify-center gap-2 rounded-lg border border-dashed border-slate-300 bg-slate-50 px-3 py-2 text-xs font-medium text-slate-700 transition-colors hover:border-blue-500 hover:bg-blue-50/40"
              >
                <FileUp className="h-4 w-4 text-slate-500" aria-hidden="true" />
                <span className="truncate">
                  {uploadedFile ? `Active: ${uploadedFile.name}` : 'Upload Custom .pdf Binary'}
                </span>
              </label>
              <input
                id="pdf-file-upload"
                type="file"
                accept=".pdf,application/pdf"
                onChange={handleFileUpload}
                className="sr-only"
              />
            </div>
          </section>

          <section className="rounded-xl border border-slate-200 bg-white p-5">
            <h2 className="text-sm font-semibold text-slate-900">02. ToMarkdownOptions</h2>

            <div className="mt-4 space-y-4">
              <div>
                <label htmlFor="table-strategy-select" className="block text-xs font-medium text-slate-700">
                  table_strategy
                </label>
                <select
                  id="table-strategy-select"
                  value={options.tableStrategy}
                  onChange={(e) => updateOption('tableStrategy', e.target.value as TableStrategy)}
                  className="mt-1.5 min-h-10 w-full rounded-lg border border-slate-200 bg-white px-3 py-2 font-mono text-xs text-slate-900 focus:border-blue-600 focus:outline-none"
                >
                  <option value="lines_strict">lines_strict (default)</option>
                  <option value="lines">lines</option>
                  <option value="text">text</option>
                  <option value="explicit">explicit</option>
                </select>
              </div>

              <div>
                <label htmlFor="hdr-info-select" className="block text-xs font-medium text-slate-700">
                  hdr_info
                </label>
                <select
                  id="hdr-info-select"
                  value={options.hdrInfo}
                  onChange={(e) => updateOption('hdrInfo', e.target.value as HeaderMode)}
                  className="mt-1.5 min-h-10 w-full rounded-lg border border-slate-200 bg-white px-3 py-2 font-mono text-xs text-slate-900 focus:border-blue-600 focus:outline-none"
                >
                  <option value="identify_headers">IdentifyHeaders (font histogram)</option>
                  <option value="toc_headers">TocHeaders (/Outlines bookmarks)</option>
                  <option value="disabled">None (disabled)</option>
                </select>
              </div>

              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label htmlFor="pages-filter-input" className="block text-xs font-medium text-slate-700">
                    pages (0-based)
                  </label>
                  <input
                    id="pages-filter-input"
                    type="text"
                    placeholder="all (e.g. 0,1)"
                    value={pagesInput}
                    onChange={(e) => setPagesInput(e.target.value)}
                    onBlur={() => void runParse()}
                    className="mt-1.5 min-h-10 w-full rounded-lg border border-slate-200 bg-white px-3 py-2 font-mono text-xs text-slate-900 focus:border-blue-600 focus:outline-none"
                  />
                </div>
                <div>
                  <label htmlFor="concurrency-select" className="block text-xs font-medium text-slate-700">
                    concurrency
                  </label>
                  <select
                    id="concurrency-select"
                    value={options.maxPageConcurrency}
                    onChange={(e) => updateOption('maxPageConcurrency', Number(e.target.value))}
                    className="mt-1.5 min-h-10 w-full rounded-lg border border-slate-200 bg-white px-3 py-2 font-mono text-xs text-slate-900 focus:border-blue-600 focus:outline-none"
                  >
                    <option value={1}>1 worker</option>
                    <option value={2}>2 workers</option>
                    <option value={4}>4 workers</option>
                    <option value={8}>8 workers</option>
                    <option value={16}>16 workers</option>
                  </select>
                </div>
              </div>

              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label htmlFor="margin-top-input" className="block text-xs font-medium text-slate-700">
                    margins.top (pt)
                  </label>
                  <input
                    id="margin-top-input"
                    type="number"
                    value={options.margins.top}
                    onChange={(e) =>
                      updateOption('margins', {
                        ...options.margins,
                        top: Number(e.target.value) || 0,
                      })
                    }
                    className="mt-1.5 min-h-10 w-full rounded-lg border border-slate-200 bg-white px-3 py-2 font-mono text-xs tabular-nums text-slate-900 focus:border-blue-600 focus:outline-none"
                  />
                </div>
                <div>
                  <label htmlFor="margin-bottom-input" className="block text-xs font-medium text-slate-700">
                    margins.bottom (pt)
                  </label>
                  <input
                    id="margin-bottom-input"
                    type="number"
                    value={options.margins.bottom}
                    onChange={(e) =>
                      updateOption('margins', {
                        ...options.margins,
                        bottom: Number(e.target.value) || 0,
                      })
                    }
                    className="mt-1.5 min-h-10 w-full rounded-lg border border-slate-200 bg-white px-3 py-2 font-mono text-xs tabular-nums text-slate-900 focus:border-blue-600 focus:outline-none"
                  />
                </div>
              </div>

              <div className="space-y-2 border-t border-slate-200 pt-3">
                {(
                  [
                    ['pageChunks', 'page_chunks'],
                    ['pageSeparators', 'page_separators'],
                    ['extractWords', 'extract_words'],
                    ['ignoreCode', 'ignore_code'],
                    ['embedImages', 'embed_images'],
                    ['writeImages', 'write_images'],
                    ['header', 'header (top region)'],
                    ['footer', 'footer (bottom region)'],
                  ] as const
                ).map(([key, label]) => (
                  <label
                    key={key}
                    className="flex min-h-8 cursor-pointer items-center justify-between gap-3 text-xs text-slate-700"
                  >
                    <span className="font-mono">{label}</span>
                    <input
                      type="checkbox"
                      checked={Boolean(options[key])}
                      onChange={(e) => {
                        const checked = e.target.checked;
                        if (key === 'pageChunks' && checked) setOutputTab('chunks');
                        updateOption(key, checked);
                      }}
                      className="h-4 w-4 rounded border-slate-300 text-blue-600 focus:ring-blue-600"
                    />
                  </label>
                ))}
              </div>

              <div className="space-y-2 border-t border-slate-200 pt-3">
                <div className="text-xs font-semibold text-slate-900">03. Text Hygiene &amp; Cleanup</div>
                {(
                  [
                    ['cleanMojibake', 'clean_mojibake'],
                    ['dehyphenate', 'dehyphenate'],
                    ['dedupFauxBold', 'dedup_faux_bold'],
                    ['ignoreInvisibleText', 'ignore_invisible_text'],
                    ['stripWatermarks', 'strip_watermarks'],
                    ['dedupRunningHeaders', 'dedup_running_headers'],
                  ] as const
                ).map(([key, label]) => (
                  <label
                    key={key}
                    className="flex min-h-8 cursor-pointer items-center justify-between gap-3 text-xs text-slate-700"
                  >
                    <span className="font-mono">{label}</span>
                    <input
                      type="checkbox"
                      checked={options.hygiene[key]}
                      onChange={(e) => updateHygiene(key, e.target.checked)}
                      className="h-4 w-4 rounded border-slate-300 text-blue-600 focus:ring-blue-600"
                    />
                  </label>
                ))}
              </div>
            </div>
          </section>
        </aside>

        <section className="flex flex-col space-y-4 lg:col-span-8 xl:col-span-9">
          {errorMsg && (
            <div role="alert" className="rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700">
              Parsing error: {errorMsg}
            </div>
          )}

          <div className="grid grid-cols-2 divide-y divide-slate-200 rounded-xl border border-slate-200 bg-white sm:grid-cols-4 sm:divide-x sm:divide-y-0">
            <div className="px-4 py-3.5">
              <div className="text-xs text-slate-500">Pages Parsed</div>
              <div className="mt-1 font-mono text-lg font-semibold tabular-nums text-slate-900">
                {result ? `${result.chunks.length} / ${result.pageCount}` : '—'}
              </div>
            </div>
            <div className="px-4 py-3.5">
              <div className="text-xs text-slate-500">Tables · Hygiene Actions</div>
              <div className="mt-1 font-mono text-lg font-semibold tabular-nums text-slate-900">
                {result ? `${totalTables} · ${hygieneCleanedTotal}` : '—'}
              </div>
            </div>
            <div className="px-4 py-3.5">
              <div className="text-xs text-slate-500">Body Font Calibration</div>
              <div className="mt-1 font-mono text-lg font-semibold tabular-nums text-slate-900">
                {result ? `${result.bodyFontSize} pt` : '—'}
              </div>
            </div>
            <div className="px-4 py-3.5">
              <div className="text-xs text-slate-500">Parse Latency / Throughput</div>
              <div className="mt-1 font-mono text-lg font-semibold tabular-nums text-slate-900">
                {result ? `${result.elapsedMs} ms · ${result.throughputPagesPerSec} p/s` : '—'}
              </div>
            </div>
          </div>

          <div className="flex flex-col rounded-xl border border-slate-200 bg-white">
            <div className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-200 px-4 py-3">
              <div role="tablist" aria-label="Extraction output views" className="flex items-center gap-1 rounded-lg bg-slate-100 p-1">
                {(
                  [
                    ['preview', 'Rendered Preview'],
                    ['raw', 'Raw Markdown'],
                    ['chunks', 'Page Chunks JSON'],
                    ['inspection', 'Words & Tables'],
                  ] as const
                ).map(([id, label]) => (
                  <button
                    key={id}
                    type="button"
                    role="tab"
                    aria-selected={outputTab === id}
                    onClick={() => setOutputTab(id)}
                    className={`min-h-8 rounded-md px-3 py-1.5 text-xs font-medium transition-colors whitespace-nowrap shrink-0 ${
                      outputTab === id
                        ? 'bg-white text-slate-900 shadow-xs'
                        : 'text-slate-600 hover:text-slate-900'
                    }`}
                  >
                    {label}
                  </button>
                ))}
              </div>

              <div className="flex items-center gap-2">
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
                      <span>Copy Output</span>
                    </>
                  )}
                </button>
                <button
                  type="button"
                  onClick={handleDownloadMarkdown}
                  className="inline-flex min-h-9 items-center gap-1.5 rounded-lg border border-slate-200 bg-white px-3 py-1.5 text-xs font-medium text-slate-700 hover:bg-slate-50 whitespace-nowrap shrink-0"
                >
                  <Download className="h-3.5 w-3.5" aria-hidden="true" />
                  <span>Save .md</span>
                </button>
              </div>
            </div>

            <div className="min-h-[540px] p-6">
              {!result ? (
                <div className="py-16 text-center text-sm text-slate-500">Parsing PDF document...</div>
              ) : outputTab === 'preview' ? (
                <MarkdownRenderer markdown={result.markdown} />
              ) : outputTab === 'raw' ? (
                <pre className="max-h-[640px] overflow-auto rounded-lg border border-slate-200 bg-slate-950 p-4 font-mono text-[13px] leading-relaxed text-slate-100">
                  {result.markdown}
                </pre>
              ) : outputTab === 'chunks' ? (
                <pre className="max-h-[640px] overflow-auto rounded-lg border border-slate-200 bg-slate-950 p-4 font-mono text-[12px] leading-relaxed text-slate-100">
                  {JSON.stringify(result.chunks, null, 2)}
                </pre>
              ) : (
                <div className="space-y-6">
                  {hs && (
                    <div>
                      <h3 className="text-sm font-semibold text-slate-900">
                        Text Hygiene &amp; Diagnostics
                      </h3>
                      <div className="mt-3 overflow-x-auto rounded-lg border border-slate-200">
                        <table className="w-full border-collapse text-left text-xs">
                          <thead className="border-b border-slate-200 bg-slate-50">
                            <tr>
                              <th className="px-3.5 py-2.5 font-semibold text-slate-900">Mojibake Stripped</th>
                              <th className="px-3.5 py-2.5 font-semibold text-slate-900">Ligatures Expanded</th>
                              <th className="px-3.5 py-2.5 font-semibold text-slate-900">Faux-Bold Deduped</th>
                              <th className="px-3.5 py-2.5 font-semibold text-slate-900">Watermarks Filtered</th>
                              <th className="px-3.5 py-2.5 font-semibold text-slate-900">3 Tr Invisible</th>
                              <th className="px-3.5 py-2.5 font-semibold text-slate-900">Dehyphenations</th>
                              <th className="px-3.5 py-2.5 font-semibold text-slate-900">Running Headers</th>
                            </tr>
                          </thead>
                          <tbody className="bg-white font-mono tabular-nums">
                            <tr>
                              <td className="px-3.5 py-2.5 text-slate-900">{hs.mojibakeCharsRemoved}</td>
                              <td className="px-3.5 py-2.5 text-slate-900">{hs.ligaturesExpanded}</td>
                              <td className="px-3.5 py-2.5 text-slate-900">{hs.fauxBoldDeduped}</td>
                              <td className="px-3.5 py-2.5 text-slate-900">{hs.watermarksFiltered}</td>
                              <td className="px-3.5 py-2.5 text-slate-900">{hs.invisibleSpansFiltered}</td>
                              <td className="px-3.5 py-2.5 text-slate-900">{hs.dehyphenatedJoins}</td>
                              <td className="px-3.5 py-2.5 text-slate-900">{hs.runningHeadersRemoved}</td>
                            </tr>
                          </tbody>
                        </table>
                      </div>
                    </div>
                  )}

                  <div>
                    <h3 className="text-sm font-semibold text-slate-900">
                      Detected Tables ({totalTables}) &amp; Graphics
                    </h3>
                    <p className="mt-1 text-xs text-slate-500">
                      Per-page bounding boxes `[x0, y0, x1, y1]` segmented by SpatialGrid + Union-Find connected components.
                    </p>
                    <div className="mt-3 overflow-x-auto rounded-lg border border-slate-200">
                      <table className="w-full border-collapse text-left text-xs">
                        <thead className="border-b border-slate-200 bg-slate-50">
                          <tr>
                            <th className="px-3.5 py-2.5 font-semibold text-slate-900">Page</th>
                            <th className="px-3.5 py-2.5 font-semibold text-slate-900">Tables</th>
                            <th className="px-3.5 py-2.5 font-semibold text-slate-900">Images</th>
                            <th className="px-3.5 py-2.5 font-semibold text-slate-900">Graphics</th>
                            <th className="px-3.5 py-2.5 font-semibold text-slate-900">TOC Items</th>
                          </tr>
                        </thead>
                        <tbody className="divide-y divide-slate-200 bg-white font-mono tabular-nums">
                          {result.chunks.map((c) => (
                            <tr key={c.metadata.page}>
                              <td className="px-3.5 py-2.5 text-slate-900">Page {c.metadata.page}</td>
                              <td className="px-3.5 py-2.5 text-slate-700">
                                {c.tables.length > 0
                                  ? c.tables.map((t) => `${t.rows}x${t.columns} [${t.bbox.join(', ')}]`).join('; ')
                                  : '0'}
                              </td>
                              <td className="px-3.5 py-2.5 text-slate-700">{c.images.length}</td>
                              <td className="px-3.5 py-2.5 text-slate-700">{c.graphics.length}</td>
                              <td className="px-3.5 py-2.5 text-slate-700">
                                {c.toc_items.length > 0 ? c.toc_items.map((t) => t[1]).join(' · ') : 'None'}
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  </div>

                  <div>
                    <h3 className="text-sm font-semibold text-slate-900">
                      Extracted Words ({options.extractWords ? totalWords : 'Disabled'})
                    </h3>
                    {!options.extractWords ? (
                      <div className="mt-2 rounded-lg border border-slate-200 bg-slate-50 p-4 text-xs text-slate-600">
                        Enable <code className="font-mono text-slate-800">extract_words</code> in the left sidebar to inspect word-level coordinate tuples.
                      </div>
                    ) : (
                      <div className="mt-3 max-h-72 overflow-auto rounded-lg border border-slate-200">
                        <table className="w-full border-collapse text-left text-xs">
                          <thead className="sticky top-0 border-b border-slate-200 bg-slate-50">
                            <tr>
                              <th className="px-3 py-2 font-semibold text-slate-900">Word</th>
                              <th className="px-3 py-2 font-semibold text-slate-900">BBox (x0, y0, x1, y1)</th>
                              <th className="px-3 py-2 text-right font-semibold text-slate-900">Line</th>
                              <th className="px-3 py-2 text-right font-semibold text-slate-900">Word #</th>
                            </tr>
                          </thead>
                          <tbody className="divide-y divide-slate-200 bg-white font-mono tabular-nums">
                            {(result.chunks[0]?.words ?? []).slice(0, 80).map((w, idx) => (
                              <tr key={idx}>
                                <td className="px-3 py-1.5 text-slate-900">{w[4]}</td>
                                <td className="px-3 py-1.5 text-slate-600">
                                  [{w[0]}, {w[1]}, {w[2]}, {w[3]}]
                                </td>
                                <td className="px-3 py-1.5 text-right text-slate-600">{w[6]}</td>
                                <td className="px-3 py-1.5 text-right text-slate-600">{w[7]}</td>
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      </div>
                    )}
                  </div>
                </div>
              )}
            </div>
          </div>
        </section>
      </div>
    </div>
  );
}
