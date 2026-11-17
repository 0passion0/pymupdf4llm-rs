import { extractPage, parsePdfBytes, type ParsedPage, type ParsedPdfDocument, type TextSpan } from './pdfBinaryParser';
import { SpatialGrid, UnionFind } from './spatial';
import {
  detectRunningHeadersFooters,
  filterAndDedupSpans,
  normalizeRunningSignature,
  sanitizeSpanText,
  shouldDehyphenateJoin,
} from './textCleaner';
import {
  createEmptyHygieneStats,
  type ExtractedImage,
  type ExtractedWord,
  type PageChunk,
  type ParseExecutionResult,
  type Rect,
  type ToMarkdownOptions,
  type WorkerTrace,
} from './types';

interface TextLine {
  bbox: Rect;
  spans: TextSpan[];
  text: string;
  maxFontSize: number;
  isAllMonospace: boolean;
}

interface DetectedTable {
  bbox: Rect;
  rows: number;
  cols: number;
  markdown: string;
}

interface TableLine {
  isHoriz: boolean;
  coord: number;
  start: number;
  end: number;
}

function rectWidth(r: Rect): number {
  return Math.max(0, r.x1 - r.x0);
}
function rectHeight(r: Rect): number {
  return Math.max(0, r.y1 - r.y0);
}
function rectArea(r: Rect): number {
  return rectWidth(r) * rectHeight(r);
}
function rectCenterX(r: Rect): number {
  return (r.x0 + r.x1) * 0.5;
}
function rectCenterY(r: Rect): number {
  return (r.y0 + r.y1) * 0.5;
}
function rectContainsPt(r: Rect, x: number, y: number): boolean {
  return x >= r.x0 && x <= r.x1 && y >= r.y0 && y <= r.y1;
}
function rectIntersects(a: Rect, b: Rect): boolean {
  return a.x0 < b.x1 && a.x1 > b.x0 && a.y0 < b.y1 && a.y1 > b.y0;
}
function rectUnion(a: Rect, b: Rect): Rect {
  return {
    x0: Math.min(a.x0, b.x0),
    y0: Math.min(a.y0, b.y0),
    x1: Math.max(a.x1, b.x1),
    y1: Math.max(a.y1, b.y1),
  };
}

function clusterCoords(vals: number[], tol: number): number[] {
  const sorted = [...vals].sort((a, b) => a - b);
  const out: number[] = [];
  for (const v of sorted) {
    if (out.length > 0 && Math.abs(v - out[out.length - 1]) <= tol) {
      out[out.length - 1] = (out[out.length - 1] + v) * 0.5;
    } else {
      out.push(v);
    }
  }
  return out;
}

function formatGfmTable(grid: string[][]): string {
  if (grid.length === 0) return '';
  const cols = grid[0].length;
  const fmtRow = (r: string[]) => `|${r.map((c) => c.replace(/\|/g, '\\|').replace(/\n/g, '<br>')).join('|')}|`;
  const lines = [fmtRow(grid[0]), `|${Array(cols).fill('---').join('|')}|`];
  for (let i = 1; i < grid.length; i++) lines.push(fmtRow(grid[i]));
  return lines.join('\n');
}

function linesConnect(a: TableLine, b: TableLine, tol: number): boolean {
  if (a.isHoriz !== b.isHoriz) {
    const h = a.isHoriz ? a : b;
    const v = a.isHoriz ? b : a;
    return v.coord >= h.start - tol && v.coord <= h.end + tol && h.coord >= v.start - tol && h.coord <= v.end + tol;
  }
  return Math.abs(a.coord - b.coord) <= tol && a.start <= b.end + tol && a.end >= b.start - tol;
}

function detectTables(page: ParsedPage, spans: TextSpan[], strategy: ToMarkdownOptions['tableStrategy']): DetectedTable[] {
  if (strategy === 'lines_strict' || strategy === 'lines') {
    const includeFilled = strategy === 'lines';
    const lines: TableLine[] = [];
    for (const seg of page.segments) {
      if (!seg.isStroked && !includeFilled) continue;
      const dx = Math.abs(seg.p1[0] - seg.p0[0]);
      const dy = Math.abs(seg.p1[1] - seg.p0[1]);
      if (dy <= 1.5 && dx >= 12) {
        lines.push({
          isHoriz: true,
          coord: (seg.p0[1] + seg.p1[1]) * 0.5,
          start: Math.min(seg.p0[0], seg.p1[0]),
          end: Math.max(seg.p0[0], seg.p1[0]),
        });
      } else if (dx <= 1.5 && dy >= 8) {
        lines.push({
          isHoriz: false,
          coord: (seg.p0[0] + seg.p1[0]) * 0.5,
          start: Math.min(seg.p0[1], seg.p1[1]),
          end: Math.max(seg.p0[1], seg.p1[1]),
        });
      }
    }
    if (includeFilled) {
      for (const r of page.rects) {
        if (r.isFilled && rectWidth(r.rect) >= 12 && rectHeight(r.rect) >= 8) {
          lines.push({ isHoriz: true, coord: r.rect.y0, start: r.rect.x0, end: r.rect.x1 });
          lines.push({ isHoriz: true, coord: r.rect.y1, start: r.rect.x0, end: r.rect.x1 });
          lines.push({ isHoriz: false, coord: r.rect.x0, start: r.rect.y0, end: r.rect.y1 });
          lines.push({ isHoriz: false, coord: r.rect.x1, start: r.rect.y0, end: r.rect.y1 });
        }
      }
    }
    if (lines.length < 4) return [];

    // Connected-component clustering via SpatialGrid + UnionFind so multiple independent tables per page stay separate
    const tol = 4;
    const lineGrid = new SpatialGrid(32);
    const uf = new UnionFind(lines.length);
    for (let i = 0; i < lines.length; i++) {
      const ln = lines[i];
      const query: Rect = ln.isHoriz
        ? { x0: ln.start - tol, y0: ln.coord - tol, x1: ln.end + tol, y1: ln.coord + tol }
        : { x0: ln.coord - tol, y0: ln.start - tol, x1: ln.coord + tol, y1: ln.end + tol };
      for (const j of lineGrid.queryRect(query)) {
        if (linesConnect(ln, lines[j], tol)) uf.union(i, j);
      }
      lineGrid.insert(i, query);
    }

    const components = new Map<number, TableLine[]>();
    for (let i = 0; i < lines.length; i++) {
      const root = uf.find(i);
      const list = components.get(root);
      if (list) list.push(lines[i]);
      else components.set(root, [lines[i]]);
    }

    const spanGrid = new SpatialGrid(32);
    spans.forEach((s, idx) => spanGrid.insert(idx, s.bbox));

    const tables: DetectedTable[] = [];
    for (const comp of components.values()) {
      const hLines = comp.filter((l) => l.isHoriz).map((l) => l.coord);
      const vLines = comp.filter((l) => !l.isHoriz).map((l) => l.coord);
      if (hLines.length < 2 || vLines.length < 2) continue;
      const ys = clusterCoords(hLines, 2.5);
      const xs = clusterCoords(vLines, 2.5);
      if (ys.length < 2 || xs.length < 2) continue;
      const rows = ys.length - 1;
      const cols = xs.length - 1;
      const grid: string[][] = Array.from({ length: rows }, () => Array(cols).fill(''));
      let nonEmpty = 0;
      for (let r = 0; r < rows; r++) {
        for (let c = 0; c < cols; c++) {
          const cellRect: Rect = { x0: xs[c], y0: ys[r], x1: xs[c + 1], y1: ys[r + 1] };
          const inside = spanGrid
            .queryRect(cellRect)
            .map((idx) => spans[idx])
            .filter((s) => rectContainsPt(cellRect, rectCenterX(s.bbox), rectCenterY(s.bbox)))
            .sort((a, b) => (Math.abs(a.bbox.y0 - b.bbox.y0) <= 3 ? a.bbox.x0 - b.bbox.x0 : a.bbox.y0 - b.bbox.y0));
          const txt = inside.map((s) => s.text.trim()).filter(Boolean).join(' ');
          if (txt) nonEmpty++;
          grid[r][c] = txt;
        }
      }
      if (nonEmpty >= 2) {
        tables.push({
          bbox: { x0: xs[0], y0: ys[0], x1: xs[xs.length - 1], y1: ys[ys.length - 1] },
          rows,
          cols,
          markdown: formatGfmTable(grid),
        });
      }
    }

    return tables.sort((a, b) =>
      Math.abs(a.bbox.y0 - b.bbox.y0) <= 10 ? a.bbox.x0 - b.bbox.x0 : a.bbox.y0 - b.bbox.y0
    );
  }

  if (strategy === 'text') {
    const sorted = [...spans].sort((a, b) => a.bbox.y0 - b.bbox.y0);
    const rowGroups: TextSpan[][] = [];
    for (const s of sorted) {
      const last = rowGroups[rowGroups.length - 1];
      if (last && Math.abs(rectCenterY(s.bbox) - rectCenterY(last[0].bbox)) <= 4.0) {
        last.push(s);
      } else {
        rowGroups.push([s]);
      }
    }
    const tables: DetectedTable[] = [];
    let runStart = -1;
    let runCells: Array<Array<{ x0: number; x1: number; text: string }>> = [];

    const flush = (endIdx: number) => {
      if (runCells.length >= 2) {
        const colStarts: number[] = [];
        for (const r of runCells) {
          for (const c of r) {
            if (!colStarts.some((x) => Math.abs(x - c.x0) < 18)) colStarts.push(c.x0);
          }
        }
        colStarts.sort((a, b) => a - b);
        if (colStarts.length >= 2) {
          const grid = Array.from({ length: runCells.length }, () => Array(colStarts.length).fill(''));
          for (let ri = 0; ri < runCells.length; ri++) {
            for (const c of runCells[ri]) {
              let best = 0;
              let bestDist = Infinity;
              for (let ci = 0; ci < colStarts.length; ci++) {
                const d = Math.abs(colStarts[ci] - c.x0);
                if (d < bestDist) {
                  bestDist = d;
                  best = ci;
                }
              }
              grid[ri][best] = grid[ri][best] ? `${grid[ri][best]} ${c.text}` : c.text;
            }
          }
          let bbox = rowGroups[runStart][0].bbox;
          for (let k = runStart; k < endIdx; k++) {
            for (const sp of rowGroups[k]) bbox = rectUnion(bbox, sp.bbox);
          }
          tables.push({
            bbox,
            rows: grid.length,
            cols: colStarts.length,
            markdown: formatGfmTable(grid),
          });
        }
      }
      runStart = -1;
      runCells = [];
    };

    for (let i = 0; i < rowGroups.length; i++) {
      const row = [...rowGroups[i]].sort((a, b) => a.bbox.x0 - b.bbox.x0);
      const cells: Array<{ x0: number; x1: number; text: string }> = [];
      for (const s of row) {
        const last = cells[cells.length - 1];
        if (last && s.bbox.x0 - last.x1 < 14) {
          last.x1 = Math.max(last.x1, s.bbox.x1);
          last.text = `${last.text} ${s.text.trim()}`.trim();
        } else {
          cells.push({ x0: s.bbox.x0, x1: s.bbox.x1, text: s.text.trim() });
        }
      }
      if (cells.length >= 2) {
        if (runStart === -1) runStart = i;
        runCells.push(cells);
      } else {
        flush(i);
      }
    }
    flush(rowGroups.length);
    return tables;
  }

  return [];
}

function findWidestGutter(spans: TextSpan[], bounds: Rect, axis: 'x' | 'y', minGap: number): number | null {
  const intervals = spans
    .map((s) =>
      axis === 'x'
        ? [Math.max(s.bbox.x0, bounds.x0), Math.min(s.bbox.x1, bounds.x1)]
        : [Math.max(s.bbox.y0, bounds.y0), Math.min(s.bbox.y1, bounds.y1)]
    )
    .filter(([a, b]) => b > a)
    .sort((a, b) => a[0] - b[0]);

  if (intervals.length < 2) return null;
  const merged: Array<[number, number]> = [];
  for (const [a, b] of intervals) {
    const last = merged[merged.length - 1];
    if (last && a <= last[1] + 1) last[1] = Math.max(last[1], b);
    else merged.push([a, b]);
  }
  let bestGap = minGap;
  let bestMid: number | null = null;
  for (let i = 0; i + 1 < merged.length; i++) {
    const gap = merged[i + 1][0] - merged[i][1];
    if (gap > bestGap) {
      bestGap = gap;
      bestMid = (merged[i][1] + merged[i + 1][0]) * 0.5;
    }
  }
  return bestMid;
}

function xyCutOrder(spans: TextSpan[], bounds: Rect, depth = 0): TextSpan[] {
  if (spans.length <= 1 || depth >= 8) {
    return [...spans].sort((a, b) =>
      Math.abs(rectCenterY(a.bbox) - rectCenterY(b.bbox)) <= 4 ? a.bbox.x0 - b.bbox.x0 : a.bbox.y0 - b.bbox.y0
    );
  }
  const splitY = findWidestGutter(spans, bounds, 'y', 18);
  if (splitY !== null) {
    const top = spans.filter((s) => rectCenterY(s.bbox) < splitY);
    const bot = spans.filter((s) => rectCenterY(s.bbox) >= splitY);
    if (top.length > 0 && bot.length > 0) {
      return [
        ...xyCutOrder(top, { ...bounds, y1: splitY }, depth + 1),
        ...xyCutOrder(bot, { ...bounds, y0: splitY }, depth + 1),
      ];
    }
  }
  const splitX = findWidestGutter(spans, bounds, 'x', 14);
  if (splitX !== null) {
    const left = spans.filter((s) => rectCenterX(s.bbox) < splitX);
    const right = spans.filter((s) => rectCenterX(s.bbox) >= splitX);
    if (left.length > 0 && right.length > 0) {
      return [
        ...xyCutOrder(left, { ...bounds, x1: splitX }, depth + 1),
        ...xyCutOrder(right, { ...bounds, x0: splitX }, depth + 1),
      ];
    }
  }
  return [...spans].sort((a, b) =>
    Math.abs(rectCenterY(a.bbox) - rectCenterY(b.bbox)) <= 4 ? a.bbox.x0 - b.bbox.x0 : a.bbox.y0 - b.bbox.y0
  );
}

function groupIntoLines(spans: TextSpan[], bounds: Rect): TextLine[] {
  const ordered = xyCutOrder(spans, bounds);
  const groups: TextSpan[][] = [];
  for (const s of ordered) {
    const last = groups[groups.length - 1];
    if (last) {
      const sameY = Math.abs(s.origin[1] - last[0].origin[1]) <= Math.max(3.5, last[0].fontSize * 0.45);
      const prevSpan = last[last.length - 1];
      const sameCol = Math.abs(s.bbox.x0 - prevSpan.bbox.x1) < 120 || s.bbox.x0 >= prevSpan.bbox.x0 - 4;
      if (sameY && sameCol) {
        last.push(s);
        continue;
      }
    }
    groups.push([s]);
  }
  return groups.map((g) => {
    g.sort((a, b) => a.bbox.x0 - b.bbox.x0);
    let bbox = g[0].bbox;
    let maxFontSize = 0;
    let allMono = true;
    let text = '';
    for (let i = 0; i < g.length; i++) {
      const s = g[i];
      bbox = rectUnion(bbox, s.bbox);
      maxFontSize = Math.max(maxFontSize, s.fontSize);
      if (!s.isMonospace && s.text.trim()) allMono = false;
      if (i > 0 && s.bbox.x0 - g[i - 1].bbox.x1 > s.fontSize * 0.22 && !text.endsWith(' ') && !s.text.startsWith(' ')) {
        text += ' ';
      }
      text += s.text;
    }
    return { bbox, spans: g, text, maxFontSize, isAllMonospace: allMono };
  });
}

function encodeUint8Base64(bytes: Uint8Array): string {
  let bin = '';
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  return btoa(bin);
}

function buildMinimalPngDataUri(img: { width: number; height: number; data: Uint8Array }, fmt: string): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${Math.max(64, img.width)}" height="${Math.max(48, img.height)}" viewBox="0 0 120 80"><rect width="120" height="80" fill="#e2e8f0"/><rect x="8" y="8" width="104" height="64" rx="4" fill="#cbd5e1"/><text x="60" y="44" text-anchor="middle" font-family="monospace" font-size="10" fill="#334155">${img.width}x${img.height} ${fmt.toUpperCase()}</text></svg>`;
  return `data:image/svg+xml;base64,${encodeUint8Base64(new TextEncoder().encode(svg))}`;
}

function styleSpanCore(sp: TextSpan, ignoreCode: boolean): { lead: string; core: string; trail: string } {
  const trimmed = sp.text.trim();
  if (!trimmed) return { lead: '', core: sp.text, trail: '' };
  const lead = sp.text.slice(0, sp.text.length - sp.text.trimStart().length);
  const trail = sp.text.slice(sp.text.trimEnd().length);
  let core = trimmed;
  if (!ignoreCode && sp.isMonospace) core = `\`${trimmed}\``;
  else if (sp.isBold && sp.isItalic) core = `***${trimmed}***`;
  else if (sp.isBold) core = `**${trimmed}**`;
  else if (sp.isItalic) core = `*${trimmed}*`;
  return { lead, core, trail };
}

export async function runPyMuPdf4LlmPipeline(
  bytes: Uint8Array,
  fileName: string,
  options: ToMarkdownOptions
): Promise<ParseExecutionResult> {
  const tStart = performance.now();
  const doc: ParsedPdfDocument = await parsePdfBytes(bytes, fileName);
  const hygieneStats = createEmptyHygieneStats();

  const targetPages =
    options.pages && options.pages.length > 0
      ? options.pages.filter((p) => p >= 0 && p < doc.pageCount)
      : Array.from({ length: doc.pageCount }, (_, i) => i);

  // 1. Parse pages concurrently
  const parsedPages: ParsedPage[] = new Array(targetPages.length);
  const workerTraces: WorkerTrace[] = [];

  let cursor = 0;
  const workerCount = Math.max(1, Math.min(options.maxPageConcurrency, targetPages.length || 1));

  const parseWorkers = Array.from({ length: workerCount }, async (_, workerId) => {
    while (cursor < targetPages.length) {
      const slot = cursor++;
      const pageIdx = targetPages[slot];
      const wStart = performance.now() - tStart;
      parsedPages[slot] = await extractPage(doc, pageIdx, options.fontsizeLimit, options.ignoreImages);
      const wEnd = performance.now() - tStart;
      workerTraces.push({
        workerId,
        docName: fileName,
        pageIndex: pageIdx,
        startMs: Number(wStart.toFixed(2)),
        endMs: Number(wEnd.toFixed(2)),
        durationMs: Number((wEnd - wStart).toFixed(2)),
        charsExtracted: parsedPages[slot].spans.reduce((acc, s) => acc + s.text.length, 0),
        tablesFound: 0,
      });
    }
  });
  await Promise.all(parseWorkers);

  // 2. Sanitize & filter spans per page, then calibrate HeaderResolver on clean spans
  const cleanedPagesData = parsedPages.map((page) => {
    const cb = page.cropBox;
    const contentBox: Rect = {
      x0: cb.x0 + options.margins.left,
      y0: Math.max(cb.y0 + options.margins.top, options.header ? 0 : cb.y0 + rectHeight(cb) * 0.08),
      x1: Math.max(cb.x0 + options.margins.left + 1, cb.x1 - options.margins.right),
      y1: Math.min(cb.y1 - options.margins.bottom, options.footer ? cb.y1 : cb.y1 - rectHeight(cb) * 0.08),
    };

    const sanitized: TextSpan[] = [];
    for (const s of page.spans) {
      if (!rectContainsPt(contentBox, rectCenterX(s.bbox), rectCenterY(s.bbox))) continue;
      if (options.ignoreAlpha && s.alpha < 0.05) continue;
      const cleanedText = sanitizeSpanText(s.text, options.hygiene, hygieneStats);
      if (cleanedText.trim().length === 0) continue;
      sanitized.push({ ...s, text: cleanedText });
    }
    const filteredSpans = filterAndDedupSpans(sanitized, options.hygiene, hygieneStats);
    const tables = detectTables(page, filteredSpans, options.tableStrategy);

    let nonTableSpans = filteredSpans;
    if (tables.length > 0) {
      const tblGrid = new SpatialGrid(64);
      tables.forEach((t, ti) => tblGrid.insert(ti, t.bbox));
      nonTableSpans = filteredSpans.filter((s) => {
        const cx = rectCenterX(s.bbox);
        const cy = rectCenterY(s.bbox);
        return !tblGrid.queryPoint(cx, cy).some((ti) => rectContainsPt(tables[ti].bbox, cx, cy));
      });
    }

    const orderedLines = groupIntoLines(nonTableSpans, contentBox);
    return { page, contentBox, filteredSpans, tables, orderedLines };
  });

  // Optional cross-page running header/footer deduplication across >= 3 pages
  if (options.hygiene.dedupRunningHeaders && cleanedPagesData.length >= 3) {
    const sigInputs = cleanedPagesData.map(({ page, orderedLines }) => {
      const h = Math.max(1, rectHeight(page.cropBox));
      const y0 = page.cropBox.y0;
      return orderedLines.map((ln) => ({
        yRatio: (rectCenterY(ln.bbox) - y0) / h,
        text: ln.text,
      }));
    });
    const repeated = detectRunningHeadersFooters(sigInputs, 3);
    if (repeated.size > 0) {
      for (const item of cleanedPagesData) {
        const h = Math.max(1, rectHeight(item.page.cropBox));
        const y0 = item.page.cropBox.y0;
        const before = item.orderedLines.length;
        item.orderedLines = item.orderedLines.filter((ln) => {
          const yRatio = (rectCenterY(ln.bbox) - y0) / h;
          if (yRatio <= 0.1 || yRatio >= 0.9) {
            return !repeated.has(normalizeRunningSignature(ln.text));
          }
          return true;
        });
        hygieneStats.runningHeadersRemoved += before - item.orderedLines.length;
      }
    }
  }

  // Calibrate IdentifyHeaders on clean non-filtered spans
  const counts = new Map<number, number>();
  for (const { filteredSpans } of cleanedPagesData) {
    for (const s of filteredSpans) {
      const key = Math.round(s.fontSize * 10);
      const len = s.text.trim().length;
      if (len > 0) counts.set(key, (counts.get(key) ?? 0) + len);
    }
  }
  let bodyTenths = 110;
  let maxChars = -1;
  for (const [sz, cnt] of counts.entries()) {
    if (cnt > maxChars) {
      maxChars = cnt;
      bodyTenths = sz;
    }
  }
  const larger = [...counts.keys()]
    .filter((sz) => sz > bodyTenths + 5)
    .sort((a, b) => b - a)
    .slice(0, Math.min(6, Math.max(1, options.maxHeaderLevels)));
  const headerMap = larger.map((sz, i) => ({ fontSize: sz / 10, level: i + 1 }));

  const getHeaderPrefix = (span: TextSpan, pageIdx: number): string => {
    if (options.hdrInfo === 'disabled' || span.isMonospace) return '';
    if (options.hdrInfo === 'toc_headers') {
      const page1 = pageIdx + 1;
      const normSpan = span.text.trim().replace(/\s+/g, ' ').toLowerCase();
      if (normSpan.length < 2) return '';
      for (const [lvl, title, pg] of doc.toc) {
        if (pg === page1) {
          const normTitle = title.trim().replace(/\s+/g, ' ').toLowerCase();
          if (normTitle === normSpan || normTitle.startsWith(normSpan) || normSpan.startsWith(normTitle)) {
            return `${'#'.repeat(Math.min(6, Math.max(1, lvl)))} `;
          }
        }
      }
      return '';
    }
    const tenths = Math.round(span.fontSize * 10);
    for (const { fontSize, level } of headerMap) {
      if (Math.abs(tenths - Math.round(fontSize * 10)) <= 4) {
        return `${'#'.repeat(level)} `;
      }
    }
    return '';
  };

  // 3. Render Markdown & Chunks
  const chunks: PageChunk[] = cleanedPagesData.map(({ page, contentBox, tables, orderedLines }) => {
    const trace = workerTraces.find((t) => t.pageIndex === page.pageIndex);
    if (trace) trace.tablesFound = tables.length;

    // Graphics clustering via SpatialGrid + UnionFind
    const graphics: Array<{ bbox: [number, number, number, number] }> = [];
    if (!options.ignoreGraphics) {
      const totalDrawings = page.segments.length + page.rects.length;
      if (options.graphicsLimit === null || totalDrawings <= options.graphicsLimit) {
        const boxes = page.rects
          .filter(
            (r) =>
              rectIntersects(r.rect, contentBox) &&
              rectWidth(r.rect) > 6 &&
              rectHeight(r.rect) > 6 &&
              rectArea(r.rect) < rectArea(contentBox) * 0.92
          )
          .map((r) => r.rect);
        if (boxes.length > 0) {
          const gGrid = new SpatialGrid(32);
          const uf = new UnionFind(boxes.length);
          for (let i = 0; i < boxes.length; i++) {
            const b = boxes[i];
            const exp = { x0: b.x0 - 8, y0: b.y0 - 8, x1: b.x1 + 8, y1: b.y1 + 8 };
            for (const j of gGrid.queryRect(exp)) {
              const jb = boxes[j];
              const jExp = { x0: jb.x0 - 8, y0: jb.y0 - 8, x1: jb.x1 + 8, y1: jb.y1 + 8 };
              if (rectIntersects(jExp, b)) uf.union(i, j);
            }
            gGrid.insert(i, exp);
          }
          const groups = new Map<number, Rect>();
          boxes.forEach((b, i) => {
            const root = uf.find(i);
            const cur = groups.get(root);
            groups.set(root, cur ? rectUnion(cur, b) : { ...b });
          });
          const minArea = rectArea(contentBox) * options.imageSizeLimit * 0.05;
          for (const g of groups.values()) {
            if (rectArea(g) >= minArea) {
              graphics.push({
                bbox: [Number(g.x0.toFixed(1)), Number(g.y0.toFixed(1)), Number(g.x1.toFixed(1)), Number(g.y1.toFixed(1))],
              });
            }
          }
        }
      }
    }

    const pageArea = Math.max(1, rectArea(page.cropBox));
    const images: ExtractedImage[] = [];
    const imageFlowBlocks: Array<{ y: number; kind: 'image'; markdown: string }> = [];
    if (!options.ignoreImages) {
      page.images
        .filter((img) => rectArea(img.bbox) / pageArea >= options.imageSizeLimit * 0.01 && rectIntersects(img.bbox, contentBox))
        .forEach((img, idx) => {
          const fname = `${fileName}-${page.pageIndex}-${idx}.${options.imageFormat}`;
          images.push({
            number: idx,
            bbox: [
              Number(img.bbox.x0.toFixed(1)),
              Number(img.bbox.y0.toFixed(1)),
              Number(img.bbox.x1.toFixed(1)),
              Number(img.bbox.y1.toFixed(1)),
            ],
            width: img.width,
            height: img.height,
            cs_name: img.colorSpace,
            ext: options.imageFormat,
            ...(options.writeImages ? { file_name: fname } : {}),
          });
          if (options.embedImages) {
            imageFlowBlocks.push({
              y: img.bbox.y0,
              kind: 'image',
              markdown: `![](${buildMinimalPngDataUri(img, options.imageFormat)})`,
            });
          } else if (options.writeImages) {
            const pathPrefix = options.imagePath ? `${options.imagePath.replace(/\/$/, '')}/` : '';
            imageFlowBlocks.push({
              y: img.bbox.y0,
              kind: 'image',
              markdown: `![](${pathPrefix}${fname})`,
            });
          }
        });
    }

    type Flow =
      | { y: number; x0: number; x1: number; kind: 'line'; line: TextLine }
      | { y: number; x0: number; x1: number; kind: 'table'; markdown: string }
      | { y: number; x0: number; x1: number; kind: 'image'; markdown: string };

    const flow: Flow[] = orderedLines.map((line) => ({
      y: line.bbox.y0,
      x0: line.bbox.x0,
      x1: line.bbox.x1,
      kind: 'line' as const,
      line,
    }));

    const mediaBlocks: Flow[] = [
      ...tables.map((t) => ({
        y: t.bbox.y0,
        x0: t.bbox.x0,
        x1: t.bbox.x1,
        kind: 'table' as const,
        markdown: t.markdown,
      })),
      ...imageFlowBlocks.map((img) => ({
        y: img.y,
        x0: contentBox.x0,
        x1: contentBox.x1,
        kind: 'image' as const,
        markdown: img.markdown,
      })),
    ].sort((a, b) => a.y - b.y);

    for (const mb of mediaBlocks) {
      let insertIdx = flow.length;
      for (let idx = 0; idx < flow.length; idx++) {
        const f = flow[idx];
        const horizOverlap = f.x0 < mb.x1 && f.x1 > mb.x0;
        if (horizOverlap && f.y > mb.y) {
          insertIdx = idx;
          break;
        }
      }
      flow.splice(insertIdx, 0, mb);
    }

    let md = '';
    let inCode = false;
    let prevYEnd: number | null = null;
    let prevLineText: string | null = null;

    const formatSpans = (spans: TextSpan[]) => {
      let sOut = '';
      let i = 0;
      while (i < spans.length) {
        const sp = spans[i];
        if (i > 0 && sp.bbox.x0 - spans[i - 1].bbox.x1 > sp.fontSize * 0.22 && !sOut.endsWith(' ') && !sp.text.startsWith(' ')) {
          sOut += ' ';
        }
        if (sp.linkUri) {
          const uri = sp.linkUri;
          let j = i + 1;
          while (j < spans.length && spans[j].linkUri === uri) j++;
          let inner = '';
          let leadWs = '';
          let trailWs = '';
          for (let k = i; k < j; k++) {
            const sk = spans[k];
            if (k > i && sk.bbox.x0 - spans[k - 1].bbox.x1 > sk.fontSize * 0.22 && !inner.endsWith(' ') && !sk.text.startsWith(' ')) {
              inner += ' ';
            }
            const { lead, core, trail } = styleSpanCore(sk, options.ignoreCode);
            if (k === i) leadWs = lead;
            else inner += lead;
            inner += core;
            if (k + 1 === j) trailWs = trail;
            else inner += trail;
          }
          sOut += leadWs + (inner.trim() ? `[${inner.trim()}](${uri})` : inner) + trailWs;
          i = j;
          continue;
        }
        const { lead, core, trail } = styleSpanCore(sp, options.ignoreCode);
        sOut += lead + core + trail;
        i++;
      }
      return sOut;
    };

    for (const blk of flow) {
      if (blk.kind === 'table' || blk.kind === 'image') {
        if (inCode) {
          md += '```\n\n';
          inCode = false;
        }
        if (md && !md.endsWith('\n\n')) md += '\n';
        md += `${blk.markdown}\n\n`;
        prevYEnd = null;
        prevLineText = null;
        continue;
      }
      const line = blk.line;
      const trimmed = line.text.trim();
      if (!trimmed) continue;

      const hdrPrefix = line.spans[0] ? getHeaderPrefix(line.spans[0], page.pageIndex) : '';
      if (!options.ignoreCode && line.isAllMonospace && !hdrPrefix) {
        if (!inCode) {
          if (md && !md.endsWith('\n\n')) md += '\n';
          md += '```\n';
          inCode = true;
        }
        md += `${trimmed}\n`;
        prevYEnd = line.bbox.y1;
        prevLineText = null;
        continue;
      } else if (inCode) {
        md += '```\n\n';
        inCode = false;
      }

      if (hdrPrefix) {
        if (md && !md.endsWith('\n\n')) md += '\n';
        md += `${hdrPrefix}${trimmed}\n\n`;
        prevYEnd = line.bbox.y1;
        prevLineText = null;
        continue;
      }

      const bulletMatch = trimmed.match(/^(?:•|◦|▪|‣|-|\*)\s+(.*)$/);
      if (bulletMatch) {
        if (md && !md.endsWith('\n')) md += '\n';
        md += `- ${bulletMatch[1]}\n`;
        prevYEnd = line.bbox.y1;
        prevLineText = null;
        continue;
      }

      if (prevYEnd !== null) {
        const gap = line.bbox.y0 - prevYEnd;
        if (gap > line.maxFontSize * 0.85) {
          if (!md.endsWith('\n\n')) {
            if (!md.endsWith('\n')) md += '\n';
            md += '\n';
          }
        } else if (
          options.hygiene.dehyphenate &&
          prevLineText !== null &&
          shouldDehyphenateJoin(prevLineText, trimmed) &&
          md.endsWith('-')
        ) {
          md = md.slice(0, -1);
          hygieneStats.dehyphenatedJoins++;
        } else if (md && !md.endsWith('\n') && !md.endsWith(' ')) {
          md += ' ';
        }
      }
      md += formatSpans(line.spans);
      prevYEnd = line.bbox.y1;
      prevLineText = trimmed;
    }
    if (inCode) md += '```\n';
    md = md.replace(/\n+$/, '') + (md.trim() ? '\n' : '');

    const words: ExtractedWord[] = [];
    if (options.extractWords) {
      orderedLines.forEach((line, lIdx) => {
        let wIdx = 0;
        for (const sp of line.spans) {
          const totalC = Math.max(1, sp.text.length);
          const spW = rectWidth(sp.bbox);
          let off = 0;
          for (const part of sp.text.split(/\s+/).filter(Boolean)) {
            const rel = sp.text.indexOf(part, off);
            if (rel !== -1) {
              const wx0 = sp.bbox.x0 + (rel / totalC) * spW;
              const wx1 = sp.bbox.x0 + ((rel + part.length) / totalC) * spW;
              words.push([
                Number(wx0.toFixed(1)),
                Number(sp.bbox.y0.toFixed(1)),
                Number(wx1.toFixed(1)),
                Number(sp.bbox.y1.toFixed(1)),
                part,
                0,
                lIdx,
                wIdx++,
              ]);
              off = rel + part.length;
            }
          }
        }
      });
    }

    const page1 = page.pageIndex + 1;
    return {
      metadata: {
        ...doc.metadata,
        file_path: fileName,
        page_count: doc.pageCount,
        page: page1,
      },
      toc_items: doc.toc.filter((t) => t[2] === page1),
      tables: tables.map((t) => ({
        bbox: [
          Number(t.bbox.x0.toFixed(1)),
          Number(t.bbox.y0.toFixed(1)),
          Number(t.bbox.x1.toFixed(1)),
          Number(t.bbox.y1.toFixed(1)),
        ],
        rows: t.rows,
        columns: t.cols,
      })),
      images,
      graphics,
      text: md,
      ...(options.extractWords ? { words } : {}),
    };
  });

  const sep = options.pageSeparators ? '\n-----\n\n' : '\n';
  const markdown = chunks.map((c) => c.text).join(sep);
  const elapsedMs = Math.max(0.5, performance.now() - tStart);

  return {
    fileName,
    fileSizeBytes: bytes.length,
    pageCount: doc.pageCount,
    markdown,
    chunks,
    elapsedMs: Number(elapsedMs.toFixed(2)),
    throughputPagesPerSec: Number(((chunks.length / elapsedMs) * 1000).toFixed(1)),
    workerTraces: workerTraces.sort((a, b) => a.pageIndex - b.pageIndex),
    detectedHeaderLevels: headerMap,
    bodyFontSize: bodyTenths / 10,
    hygieneStats,
  };
}
