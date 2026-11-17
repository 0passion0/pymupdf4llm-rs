import { useState } from 'react';
import { Play } from 'lucide-react';
import { generateSamplePdf, SAMPLE_PDF_PRESETS } from '../engine/samplePdfs';
import { runPyMuPdf4LlmPipeline } from '../engine/pymupdf4llmPipeline';
import { DEFAULT_OPTIONS, type WorkerTrace } from '../engine/types';

interface BatchRunSummary {
  batchSize: number;
  concurrency: number;
  totalPages: number;
  totalElapsedMs: number;
  throughputPagesPerSec: number;
  traces: WorkerTrace[];
  docSummaries: Array<{
    docIndex: number;
    fileName: string;
    pages: number;
    elapsedMs: number;
    tables: number;
  }>;
}

export function AsyncPipelineView() {
  const [batchSize, setBatchSize] = useState<number>(8);
  const [concurrency, setConcurrency] = useState<number>(4);
  const [isRunning, setIsRunning] = useState<boolean>(false);
  const [summary, setSummary] = useState<BatchRunSummary | null>(null);

  const runBatchBenchmark = async () => {
    setIsRunning(true);
    try {
      const t0 = performance.now();
      const jobs = Array.from({ length: batchSize }, (_, i) => {
        const preset = SAMPLE_PDF_PRESETS[i % SAMPLE_PDF_PRESETS.length];
        return { idx: i, preset };
      });

      const docSummaries: BatchRunSummary['docSummaries'] = new Array(jobs.length);
      const allTraces: WorkerTrace[] = [];
      let cursor = 0;
      const docWorkers = Math.min(concurrency, jobs.length);

      await Promise.all(
        Array.from({ length: docWorkers }, async (_, workerSlot) => {
          while (cursor < jobs.length) {
            const jobIdx = cursor++;
            const { idx, preset } = jobs[jobIdx];
            const sample = generateSamplePdf(preset.id);
            const docName = `doc_${String(idx + 1).padStart(2, '0')}_${sample.fileName}`;
            const wStart = performance.now() - t0;
            const res = await runPyMuPdf4LlmPipeline(sample.bytes, docName, {
              ...DEFAULT_OPTIONS,
              tableStrategy: preset.recommendedStrategy,
              maxPageConcurrency: concurrency,
            });
            const wEnd = performance.now() - t0;
            docSummaries[jobIdx] = {
              docIndex: idx + 1,
              fileName: docName,
              pages: res.chunks.length,
              elapsedMs: Number((wEnd - wStart).toFixed(2)),
              tables: res.chunks.reduce((acc, c) => acc + c.tables.length, 0),
            };
            for (const tr of res.workerTraces) {
              allTraces.push({
                ...tr,
                workerId: workerSlot,
                startMs: Number((wStart + tr.startMs).toFixed(2)),
                endMs: Number((wStart + tr.endMs).toFixed(2)),
              });
            }
          }
        })
      );

      const totalElapsedMs = Math.max(0.5, performance.now() - t0);
      const totalPages = docSummaries.reduce((acc, d) => acc + d.pages, 0);
      setSummary({
        batchSize,
        concurrency,
        totalPages,
        totalElapsedMs: Number(totalElapsedMs.toFixed(2)),
        throughputPagesPerSec: Number(((totalPages / totalElapsedMs) * 1000).toFixed(1)),
        traces: allTraces,
        docSummaries,
      });
    } finally {
      setIsRunning(false);
    }
  };

  const maxTimelineMs = summary
    ? Math.max(1, ...summary.traces.map((t) => t.endMs))
    : 1;

  return (
    <div className="mx-auto max-w-[1440px] px-4 py-6 sm:px-6">
      <div className="mb-6 flex flex-col justify-between gap-4 border-b border-slate-200 pb-5 md:flex-row md:items-end">
        <div>
          <h1 className="text-2xl font-bold tracking-tight text-slate-900">
            Async &amp; High-Concurrency Batch Pipeline
          </h1>
          <p className="mt-1 text-sm text-slate-600">
            Simulates <code className="font-mono text-xs text-slate-800">batch_to_markdown_async</code> with semaphore-bounded concurrent document and page workers.
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-3">
          <div className="flex items-center gap-2">
            <label htmlFor="batch-size-select" className="text-xs font-medium text-slate-700 whitespace-nowrap">
              Batch Size
            </label>
            <select
              id="batch-size-select"
              value={batchSize}
              onChange={(e) => setBatchSize(Number(e.target.value))}
              className="min-h-10 rounded-lg border border-slate-200 bg-white px-3 py-2 font-mono text-xs text-slate-900"
            >
              <option value={4}>4 PDFs</option>
              <option value={8}>8 PDFs</option>
              <option value={12}>12 PDFs</option>
              <option value={16}>16 PDFs</option>
            </select>
          </div>

          <div className="flex items-center gap-2">
            <label htmlFor="batch-concurrency-select" className="text-xs font-medium text-slate-700 whitespace-nowrap">
              Worker Pool
            </label>
            <select
              id="batch-concurrency-select"
              value={concurrency}
              onChange={(e) => setConcurrency(Number(e.target.value))}
              className="min-h-10 rounded-lg border border-slate-200 bg-white px-3 py-2 font-mono text-xs text-slate-900"
            >
              <option value={1}>1 worker (serial)</option>
              <option value={2}>2 workers</option>
              <option value={4}>4 workers</option>
              <option value={8}>8 workers</option>
              <option value={16}>16 workers</option>
            </select>
          </div>

          <button
            type="button"
            onClick={() => void runBatchBenchmark()}
            disabled={isRunning}
            className="inline-flex min-h-10 items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-xs font-semibold text-white transition-colors hover:bg-blue-700 disabled:opacity-60 whitespace-nowrap shrink-0"
          >
            <Play className="h-3.5 w-3.5" aria-hidden="true" />
            <span>{isRunning ? 'Executing Batch...' : 'Run Concurrent Batch'}</span>
          </button>
        </div>
      </div>

      {!summary ? (
        <div className="rounded-xl border border-slate-200 bg-white p-12 text-center">
          <h2 className="text-base font-semibold text-slate-900">Ready to Execute Concurrent Batch</h2>
          <p className="mx-auto mt-2 max-w-lg text-sm text-slate-600">
            Select a batch size and semaphore concurrency limit, then run the batch to inspect real worker lane scheduling and page throughput.
          </p>
          <button
            type="button"
            onClick={() => void runBatchBenchmark()}
            className="mt-5 inline-flex min-h-10 items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-xs font-semibold text-white hover:bg-blue-700"
          >
            <Play className="h-3.5 w-3.5" aria-hidden="true" />
            <span>Run First Batch</span>
          </button>
        </div>
      ) : (
        <div className="space-y-6">
          <div className="grid grid-cols-2 divide-y divide-slate-200 rounded-xl border border-slate-200 bg-white sm:grid-cols-4 sm:divide-x sm:divide-y-0">
            <div className="px-4 py-3.5">
              <div className="text-xs text-slate-500">Documents Processed</div>
              <div className="mt-1 font-mono text-lg font-semibold tabular-nums text-slate-900">
                {summary.batchSize} PDFs
              </div>
            </div>
            <div className="px-4 py-3.5">
              <div className="text-xs text-slate-500">Total Pages Extracted</div>
              <div className="mt-1 font-mono text-lg font-semibold tabular-nums text-slate-900">
                {summary.totalPages} pages
              </div>
            </div>
            <div className="px-4 py-3.5">
              <div className="text-xs text-slate-500">Total Wall Time</div>
              <div className="mt-1 font-mono text-lg font-semibold tabular-nums text-slate-900">
                {summary.totalElapsedMs} ms
              </div>
            </div>
            <div className="px-4 py-3.5">
              <div className="text-xs text-slate-500">Aggregate Throughput</div>
              <div className="mt-1 font-mono text-lg font-semibold tabular-nums text-emerald-700">
                {summary.throughputPagesPerSec} pages/s
              </div>
            </div>
          </div>

          <div className="rounded-xl border border-slate-200 bg-white p-5">
            <div className="flex items-center justify-between gap-4">
              <div>
                <h2 className="text-sm font-semibold text-slate-900">Worker Lane Execution Timeline</h2>
                <p className="mt-0.5 text-xs text-slate-500">
                  Real task execution spans across {Math.min(summary.concurrency, summary.batchSize)} semaphore-bounded worker lanes.
                </p>
              </div>
              <span className="font-mono text-xs tabular-nums text-slate-500">
                0 ms – {maxTimelineMs.toFixed(1)} ms
              </span>
            </div>

            <div className="mt-4 space-y-2.5">
              {Array.from({ length: Math.min(summary.concurrency, summary.batchSize) }, (_, lane) => {
                const laneTraces = summary.traces.filter((t) => t.workerId === lane);
                return (
                  <div key={lane} className="flex items-center gap-3">
                    <div className="w-20 shrink-0 font-mono text-xs text-slate-600 tabular-nums">
                      Worker {lane}
                    </div>
                    <div className="relative h-7 flex-1 overflow-hidden rounded border border-slate-200 bg-slate-50">
                      {laneTraces.map((tr, i) => {
                        const leftPct = (tr.startMs / maxTimelineMs) * 100;
                        const widthPct = Math.max(1.5, ((tr.endMs - tr.startMs) / maxTimelineMs) * 100);
                        return (
                          <div
                            key={i}
                            title={`${tr.docName} p.${tr.pageIndex + 1} (${tr.durationMs} ms)`}
                            style={{ left: `${leftPct}%`, width: `${widthPct}%` }}
                            className="absolute top-1 bottom-1 rounded-xs bg-blue-600/85 hover:bg-blue-700"
                          />
                        );
                      })}
                    </div>
                  </div>
                );
              })}
            </div>
          </div>

          <div className="overflow-x-auto rounded-xl border border-slate-200 bg-white">
            <table className="w-full border-collapse text-left text-xs">
              <thead className="border-b border-slate-200 bg-slate-50">
                <tr>
                  <th className="px-4 py-3 font-semibold text-slate-900">#</th>
                  <th className="px-4 py-3 font-semibold text-slate-900">Document</th>
                  <th className="px-4 py-3 text-right font-semibold text-slate-900">Pages</th>
                  <th className="px-4 py-3 text-right font-semibold text-slate-900">Tables</th>
                  <th className="px-4 py-3 text-right font-semibold text-slate-900">Duration (ms)</th>
                  <th className="px-4 py-3 font-semibold text-slate-900">Status</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-200 font-mono tabular-nums">
                {summary.docSummaries.map((d) => (
                  <tr key={d.docIndex} className="hover:bg-slate-50">
                    <td className="px-4 py-2.5 text-slate-500">{d.docIndex}</td>
                    <td className="px-4 py-2.5 text-slate-900">{d.fileName}</td>
                    <td className="px-4 py-2.5 text-right text-slate-700">{d.pages}</td>
                    <td className="px-4 py-2.5 text-right text-slate-700">{d.tables}</td>
                    <td className="px-4 py-2.5 text-right text-slate-700">{d.elapsedMs}</td>
                    <td className="px-4 py-2.5 text-emerald-700">Completed</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  );
}
