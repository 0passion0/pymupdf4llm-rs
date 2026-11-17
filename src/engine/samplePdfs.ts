export interface SamplePdfPreset {
  id: string;
  title: string;
  subtitle: string;
  pages: number;
  recommendedStrategy: 'lines_strict' | 'lines' | 'text';
}

export const SAMPLE_PDF_PRESETS: SamplePdfPreset[] = [
  {
    id: 'research-paper',
    title: 'Multi-Column Rust PDF Research Paper',
    subtitle: '2-column layout · Stroked tables · Monospace code · Outlines TOC',
    pages: 3,
    recommendedStrategy: 'lines_strict',
  },
  {
    id: 'financial-report',
    title: 'Q3 Infrastructure & Latency Report',
    subtitle: 'Borderless text tables · Vector charts · Embedded raster image · Header/Footer',
    pages: 2,
    recommendedStrategy: 'text',
  },
  {
    id: 'rag-spec',
    title: 'LLM Chunking & Compatibility Guide',
    subtitle: 'Multi-level headings · Bullet lists · Hyperlink annotations · Stroked tables',
    pages: 4,
    recommendedStrategy: 'lines',
  },
  {
    id: 'noisy-cjk-multitable',
    title: 'Noisy CJK, Form XObject & Multi-Table PDF',
    subtitle: 'FlateDecode · Inherited /Resources · Type0 CJK · 2 tables · Watermarks & Mojibake',
    pages: 3,
    recommendedStrategy: 'lines_strict',
  },
];

function escapePdfStr(s: string): string {
  return s.replace(/\\/g, '\\\\').replace(/\(/g, '\\(').replace(/\)/g, '\\)');
}

/**
 * Wraps raw bytes in a valid zlib/FlateDecode stored-block container (RFC 1950 + RFC 1951)
 * synchronously with zero Node/browser polyfill dependencies.
 */
function zlibStoredFlate(inputStr: string): string {
  const data: number[] = [];
  for (let i = 0; i < inputStr.length; i++) data.push(inputStr.charCodeAt(i) & 0xff);
  const blocks: number[] = [0x78, 0x9c]; // CMF=0x78 (32K deflate), FLG=0x9c
  let offset = 0;
  do {
    const chunkLen = Math.min(65535, data.length - offset);
    const isFinal = offset + chunkLen >= data.length ? 1 : 0;
    blocks.push(isFinal);
    blocks.push(chunkLen & 0xff, (chunkLen >> 8) & 0xff);
    const nlen = chunkLen ^ 0xffff;
    blocks.push(nlen & 0xff, (nlen >> 8) & 0xff);
    for (let i = 0; i < chunkLen; i++) blocks.push(data[offset + i]);
    offset += chunkLen;
  } while (offset < data.length);

  // Adler-32 checksum (big-endian)
  let s1 = 1;
  let s2 = 0;
  for (const b of data) {
    s1 = (s1 + b) % 65521;
    s2 = (s2 + s1) % 65521;
  }
  const adler = ((s2 << 16) | s1) >>> 0;
  blocks.push((adler >>> 24) & 0xff, (adler >>> 16) & 0xff, (adler >>> 8) & 0xff, adler & 0xff);

  let out = '';
  for (const b of blocks) out += String.fromCharCode(b);
  return out;
}

function compilePdf(opts: {
  title: string;
  author: string;
  subject: string;
  pages: Array<{ stream: string; hasImage?: boolean; links?: Array<{ rect: [number, number, number, number]; uri: string }> }>;
  outlines?: Array<{ title: string; level: number; pageIdx: number }>;
}): Uint8Array {
  const objects: string[] = [];
  const baseCount = 8;
  const pageObjIds: number[] = [];
  const pageContentIds: number[] = [];
  const annotIdsPerPage: number[][] = [];

  let nextId = baseCount + 1;
  for (const p of opts.pages) {
    const pid = nextId++;
    const cid = nextId++;
    pageObjIds.push(pid);
    pageContentIds.push(cid);
    const aIds: number[] = [];
    if (p.links) {
      for (const _ of p.links) aIds.push(nextId++);
    }
    annotIdsPerPage.push(aIds);
  }

  let outlinesId: number | null = null;
  const outlineItemIds: number[] = [];
  if (opts.outlines && opts.outlines.length > 0) {
    outlinesId = nextId++;
    for (const _ of opts.outlines) outlineItemIds.push(nextId++);
  }

  const catalogExtra = outlinesId ? ` /Outlines ${outlinesId} 0 R` : '';
  objects.push(`1 0 obj\n<< /Type /Catalog /Pages 2 0 R${catalogExtra} >>\nendobj\n`);
  objects.push(`2 0 obj\n<< /Type /Pages /Kids [${pageObjIds.map((id) => `${id} 0 R`).join(' ')}] /Count ${pageObjIds.length} >>\nendobj\n`);
  objects.push(`3 0 obj\n<< /Title (${escapePdfStr(opts.title)}) /Author (${escapePdfStr(opts.author)}) /Subject (${escapePdfStr(opts.subject)}) /Creator (pymupdf4llm-rs synthetic fixture) /Producer (Rust Pure-PDF Generator) >>\nendobj\n`);
  objects.push(`4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n`);
  objects.push(`5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>\nendobj\n`);
  objects.push(`6 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Oblique >>\nendobj\n`);
  objects.push(`7 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>\nendobj\n`);
  const imgBytes = '\x25\x63\xeb\x1e\x29\x3b\x0f\x17\x2a\x3b\x82\xf6\x16\xa3\x4a\x25\x63\xeb\xe2\xe8\xf0\x94\xa3\xb8\x47\x55\x69\x0f\x17\x2a\x25\x63\xeb\x16\xa3\x4a\xd9\x77\x06\x25\x63\xeb\x33\x41\x55\x0f\x17\x2a';
  objects.push(`8 0 obj\n<< /Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length ${imgBytes.length} >>\nstream\n${imgBytes}\nendstream\nendobj\n`);

  opts.pages.forEach((p, idx) => {
    const pid = pageObjIds[idx];
    const cid = pageContentIds[idx];
    const aIds = annotIdsPerPage[idx];
    const xobjStr = p.hasImage ? ' /XObject << /Im1 8 0 R >>' : '';
    const annotsStr = aIds.length > 0 ? ` /Annots [${aIds.map((id) => `${id} 0 R`).join(' ')}]` : '';
    objects.push(
      `${pid} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R /F2 5 0 R /F3 6 0 R /F4 7 0 R >>${xobjStr} >>${annotsStr} /Contents ${cid} 0 R >>\nendobj\n`
    );
    objects.push(`${cid} 0 obj\n<< /Length ${p.stream.length} >>\nstream\n${p.stream}endstream\nendobj\n`);
    if (p.links) {
      p.links.forEach((lnk, li) => {
        objects.push(
          `${aIds[li]} 0 obj\n<< /Type /Annot /Subtype /Link /Rect [${lnk.rect.join(' ')}] /Border [0 0 0] /A << /S /URI /URI (${escapePdfStr(lnk.uri)}) >> >>\nendobj\n`
        );
      });
    }
  });

  if (outlinesId && opts.outlines) {
    const firstId = outlineItemIds[0];
    const lastId = outlineItemIds[outlineItemIds.length - 1];
    objects.push(`${outlinesId} 0 obj\n<< /Type /Outlines /First ${firstId} 0 R /Last ${lastId} 0 R /Count ${outlineItemIds.length} >>\nendobj\n`);
    opts.outlines.forEach((item, i) => {
      const id = outlineItemIds[i];
      const prev = i > 0 ? ` /Prev ${outlineItemIds[i - 1]} 0 R` : '';
      const next = i + 1 < outlineItemIds.length ? ` /Next ${outlineItemIds[i + 1]} 0 R` : '';
      const targetPage = pageObjIds[item.pageIdx] ?? pageObjIds[0];
      objects.push(
        `${id} 0 obj\n<< /Title (${escapePdfStr(item.title)}) /Parent ${outlinesId} 0 R${prev}${next} /Dest [${targetPage} 0 R /Fit] >>\nendobj\n`
      );
    });
  }

  return serializeRawObjects(objects, '1.4', '<< /Size ' + (objects.length + 1) + ' /Root 1 0 R /Info 3 0 R >>');
}

function serializeRawObjects(objects: string[], version: string, trailerDict: string): Uint8Array {
  let body = `%PDF-${version}\n`;
  const offsets: number[] = [];
  for (const obj of objects) {
    offsets.push(body.length);
    body += obj;
  }
  const xrefPos = body.length;
  body += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const off of offsets) {
    body += `${String(off).padStart(10, '0')} 00000 n \n`;
  }
  body += `trailer\n${trailerDict}\nstartxref\n${xrefPos}\n%%EOF\n`;

  const out = new Uint8Array(body.length);
  for (let i = 0; i < body.length; i++) out[i] = body.charCodeAt(i) & 0xff;
  return out;
}

function buildNoisyCjkMultiTablePdf(): { fileName: string; bytes: Uint8Array } {
  const cmap = [
    '/CIDInit /ProcSet findresource begin',
    '12 dict begin begincmap',
    '/CMapType 2 def',
    '1 begincodespacerange <0000> <FFFF> endcodespacerange',
    '4 beginbfchar',
    '<0001> <4E2D>', // 中
    '<0002> <6587>', // 文
    '<0003> <FB01>', // ﬁ ligature -> expanded to fi
    '<0004> <E001>', // PUA mojibake -> stripped
    'endbfchar',
    '1 beginbfrange',
    '<0010> <0011> [<6D4B> <8BD5>]', // 测试
    'endbfrange',
    'endcmap CMapName currentdict /CMap defineresource pop end end',
  ].join('\n');

  const formStream = 'BT /F1 11 Tf 1 0 0 1 56 632 Tm (Nested Form XObject payload rendered via /Do.) Tj ET\n';

  // Page 1: FlateDecode compressed, inherited /Resources & /MediaBox, Type0 CJK, Tr=3 invisible layer,
  // faux-bold duplicate, diagonal DRAFT watermark, hyphenation, and TWO side-by-side stroked tables!
  const p1Raw = [
    'BT',
    '/F1 9 Tf 1 0 0 1 56 765 Tm (ACME SYSTEMS · RUNNING HEADER ARCHIVE) Tj',
    '/F2 18 Tf 1 0 0 1 56 716 Tm (Noisy CJK, Form XObject & Multi-Table Report) Tj',
    '/F4 12 Tf 1 0 0 1 56 692 Tm <000100020010001100030004> Tj',
    '/F1 11 Tf 1 0 0 1 94 692 Tm (rst-class Unicode hygiene & FlateDecode parsing.) Tj',
    '/F1 11 Tf 3 Tr 1 0 0 1 56 672 Tm (INVISIBLE_OCR_GARBAGE_LAYER) Tj',
    '/F1 11 Tf 0 Tr 1 0 0 1 56 672 Tm (High-concurren-) Tj',
    '/F1 11 Tf 1 0 0 1 56 656 Tm (cy extraction eliminates duplicate faux-bold glyphs.) Tj',
    '/F2 11 Tf 1 0 0 1 56 608 Tm (CleanBold) Tj',
    '/F2 11 Tf 1 0 0 1 56.4 608 Tm (CleanBold) Tj',
    '/F1 36 Tf 0.707 0.707 -0.707 0.707 210 380 Tm (DRAFT) Tj',
    '/F1 9 Tf 1 0 0 1 56 26 Tm (Page 1 of 3 · Confidential Engineering Spec) Tj',
    'ET',
    '/Form1 Do',
    // Left Table (x: 56..270, y_pdf: 490..550)
    '0.7 w 56 550 m 270 550 l S 56 520 m 270 520 l S 56 490 m 270 490 l S',
    '56 550 m 56 490 l S 163 550 m 163 490 l S 270 550 m 270 490 l S',
    'BT /F2 10 Tf 1 0 0 1 64 531 Tm (Filter) Tj 1 0 0 1 171 531 Tm (Action) Tj',
    '/F1 10 Tf 1 0 0 1 64 501 Tm (PUA/Noise) Tj 1 0 0 1 171 501 Tm (Sanitized) Tj ET',
    // Right Table (x: 320..556, y_pdf: 490..550) — separate connected component via Union-Find
    '0.7 w 320 550 m 556 550 l S 320 520 m 556 520 l S 320 490 m 556 490 l S',
    '320 550 m 320 490 l S 438 550 m 438 490 l S 556 550 m 556 490 l S',
    'BT /F2 10 Tf 1 0 0 1 328 531 Tm (Algorithm) Tj 1 0 0 1 446 531 Tm (Complexity) Tj',
    '/F1 10 Tf 1 0 0 1 328 501 Tm (SpatialGrid) Tj 1 0 0 1 446 501 Tm (O(1) query) Tj ET',
  ].join('\n') + '\n';

  const p1Compressed = zlibStoredFlate(p1Raw);

  const p2Raw = [
    'BT',
    '/F1 9 Tf 1 0 0 1 56 765 Tm (ACME SYSTEMS · RUNNING HEADER ARCHIVE) Tj',
    '/F2 14 Tf 1 0 0 1 56 706 Tm (2. Hyperlink Merging & MacRoman / Differences) Tj',
    '/F1 11 Tf 1 0 0 1 56 676 Tm (Consecutive linked spans: ) Tj /F2 11 Tf (PyMuPDF4LLM ) Tj /F3 11 Tf (Rust Toolkit) Tj /F1 11 Tf (.) Tj',
    '/F1 11 Tf 1 0 0 1 56 650 Tm (WinAnsi em-dash \x97 and typographic quotes \x93clean\x94 remain intact.) Tj',
    '/F1 9 Tf 1 0 0 1 56 26 Tm (Page 2 of 3 · Confidential Engineering Spec) Tj',
    'ET',
  ].join('\n') + '\n';

  const p3Raw = [
    'BT',
    '/F1 9 Tf 1 0 0 1 56 765 Tm (ACME SYSTEMS · RUNNING HEADER ARCHIVE) Tj',
    '/F2 14 Tf 1 0 0 1 56 706 Tm (3. Cross-Page Header Deduplication) Tj',
    '/F1 11 Tf 1 0 0 1 56 676 Tm (Enable dedup_running_headers to strip repeated running banners across 3+ pages automatically.) Tj',
    '/F1 9 Tf 1 0 0 1 56 26 Tm (Page 3 of 3 · Confidential Engineering Spec) Tj',
    'ET',
  ].join('\n') + '\n';

  // Build objects demonstrating inherited /Pages /MediaBox & /Resources!
  const objects = [
    `1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n`,
    // Object 2: /Pages holds inherited /MediaBox and /Resources!
    `2 0 obj\n<< /Type /Pages /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R /F2 5 0 R /F3 6 0 R /F4 7 0 R >> /XObject << /Form1 9 0 R >> >> /Kids [10 0 R 12 0 R 14 0 R] /Count 3 >>\nendobj\n`,
    `3 0 obj\n<< /Title (Noisy CJK & Multi-Table Benchmark) /Author (Core Parsing Engine) /Subject (Hygiene & Algorithms) >>\nendobj\n`,
    `4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n`,
    `5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>\nendobj\n`,
    `6 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Oblique >>\nendobj\n`,
    `7 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /NotoSansCJKsc-Regular /Encoding /Identity-H /ToUnicode 8 0 R >>\nendobj\n`,
    `8 0 obj\n<< /Length ${cmap.length} >>\nstream\n${cmap}\nendstream\nendobj\n`,
    `9 0 obj\n<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] /Length ${formStream.length} >>\nstream\n${formStream}endstream\nendobj\n`,
    // Page 1 (10) omits /MediaBox & /Resources intentionally to verify /Pages inheritance!
    `10 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 11 0 R >>\nendobj\n`,
    `11 0 obj\n<< /Filter /FlateDecode /Length ${p1Compressed.length} >>\nstream\n${p1Compressed}endstream\nendobj\n`,
    // Page 2 (12) + Link Annot (15) spanning PyMuPDF4LLM + Rust Toolkit
    `12 0 obj\n<< /Type /Page /Parent 2 0 R /Annots [15 0 R] /Contents 13 0 R >>\nendobj\n`,
    `13 0 obj\n<< /Length ${p2Raw.length} >>\nstream\n${p2Raw}endstream\nendobj\n`,
    // Page 3 (14)
    `14 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 16 0 R >>\nendobj\n`,
    `15 0 obj\n<< /Type /Annot /Subtype /Link /Rect [206 668 340 688] /Border [0 0 0] /A << /S /URI /URI (https://github.com/pymupdf/pymupdf4llm) >> >>\nendobj\n`,
    `16 0 obj\n<< /Length ${p3Raw.length} >>\nstream\n${p3Raw}endstream\nendobj\n`,
  ];

  return {
    fileName: 'noisy_cjk_multitable.pdf',
    bytes: serializeRawObjects(objects, '1.6', `<< /Size ${objects.length + 1} /Root 1 0 R /Info 3 0 R >>`),
  };
}

export function generateSamplePdf(presetId: string): { fileName: string; bytes: Uint8Array } {
  if (presetId === 'noisy-cjk-multitable') {
    return buildNoisyCjkMultiTablePdf();
  }

  if (presetId === 'financial-report') {
    const p1 = [
      'BT',
      '/F1 9 Tf 1 0 0 1 48 762 Tm (CONFIDENTIAL · Q3 INFRASTRUCTURE REPORT) Tj',
      '/F2 20 Tf 1 0 0 1 48 710 Tm (Q3 Compute & Storage Utilization) Tj',
      '/F1 11 Tf 1 0 0 1 48 684 Tm (Quarterly summary covering ) Tj /F2 11 Tf (Rust async workers) Tj /F1 11 Tf ( and GPU ingestion throughput.) Tj',
      '/F2 14 Tf 1 0 0 1 48 640 Tm (Regional Cluster Breakdown) Tj',
      '/F2 10 Tf 1 0 0 1 56 608 Tm (Region) Tj 1 0 0 1 180 608 Tm (Nodes) Tj 1 0 0 1 280 608 Tm (Throughput) Tj 1 0 0 1 410 608 Tm (P99 Latency) Tj',
      '/F1 10 Tf 1 0 0 1 56 588 Tm (eu-west-2) Tj 1 0 0 1 180 588 Tm (128) Tj 1 0 0 1 280 588 Tm (482 pages/s) Tj 1 0 0 1 410 588 Tm (4.2 ms) Tj',
      '/F1 10 Tf 1 0 0 1 56 570 Tm (us-east-1) Tj 1 0 0 1 180 570 Tm (256) Tj 1 0 0 1 280 570 Tm (910 pages/s) Tj 1 0 0 1 410 570 Tm (3.8 ms) Tj',
      '/F1 10 Tf 1 0 0 1 56 552 Tm (ap-northeast-1) Tj 1 0 0 1 180 552 Tm (96) Tj 1 0 0 1 280 552 Tm (345 pages/s) Tj 1 0 0 1 410 552 Tm (5.1 ms) Tj',
      '/F1 9 Tf 1 0 0 1 48 30 Tm (Page 1 of 2 · Internal Telemetry Archive) Tj',
      'ET',
      '0.2 0.35 0.75 rg 60 410 85 95 re f',
      '0.1 0.65 0.35 rg 175 410 85 115 re f',
      '0.85 0.45 0.1 rg 290 410 85 75 re f',
      'q 180 0 0 120 395 395 cm /Im1 Do Q',
    ].join('\n') + '\n';

    const p2 = [
      'BT',
      '/F1 9 Tf 1 0 0 1 48 762 Tm (CONFIDENTIAL · Q3 INFRASTRUCTURE REPORT) Tj',
      '/F2 15 Tf 1 0 0 1 48 705 Tm (Operational Highlights) Tj',
      '/F1 11 Tf 1 0 0 1 54 675 Tm (- Zero-copy Arc<PdfDocument> eliminated redundant page tree parsing.) Tj',
      '/F1 11 Tf 1 0 0 1 54 655 Tm (- Tokio semaphore bounds stabilized memory under 400 MB at 16 workers.) Tj',
      '/F1 11 Tf 1 0 0 1 54 635 Tm (- Rayon work-stealing reduced tail latency by 61% across multi-column PDFs.) Tj',
      '/F1 9 Tf 1 0 0 1 48 30 Tm (Page 2 of 2 · Internal Telemetry Archive) Tj',
      'ET',
    ].join('\n') + '\n';

    return {
      fileName: 'q3_infrastructure_report.pdf',
      bytes: compilePdf({
        title: 'Q3 Infrastructure & Latency Report',
        author: 'Platform Engineering',
        subject: 'PDF Extraction Benchmark',
        pages: [
          { stream: p1, hasImage: true },
          { stream: p2 },
        ],
        outlines: [
          { title: 'Q3 Compute & Storage Utilization', level: 1, pageIdx: 0 },
          { title: 'Regional Cluster Breakdown', level: 2, pageIdx: 0 },
          { title: 'Operational Highlights', level: 2, pageIdx: 1 },
        ],
      }),
    };
  }

  if (presetId === 'rag-spec') {
    const pages = [1, 2, 3, 4].map((n) => ({
      stream: [
        'BT',
        `/F2 18 Tf 1 0 0 1 56 710 Tm (${n}. RAG Chunking Pipeline Stage ${n}) Tj`,
        '/F1 11 Tf 1 0 0 1 56 676 Tm (Every page emits a structured ) Tj /F4 10 Tf (PageChunk) Tj /F1 11 Tf ( containing metadata, tables, and Markdown.) Tj',
        '/F2 13 Tf 1 0 0 1 56 640 Tm (Configuration Flags) Tj',
        '/F1 11 Tf 1 0 0 1 60 616 Tm (\x95 Supports ) Tj /F4 10 Tf (page_chunks=true) Tj /F1 11 Tf ( for per-page vector indexing.) Tj',
        '/F1 11 Tf 1 0 0 1 60 596 Tm (\x95 Preserves hyperlinks such as ) Tj /F3 11 Tf (PyMuPDF4LLM Upstream Spec) Tj /F1 11 Tf (.) Tj',
        '/F4 10 Tf 1 0 0 1 60 558 Tm (let opts = ToMarkdownOptions::builder().page_chunks(true).build();) Tj',
        '/F4 10 Tf 1 0 0 1 60 542 Tm (let out = to_markdown_async("spec.pdf", opts).await?;) Tj',
        'ET',
        '0.7 w 56 500 m 540 500 l S 56 476 m 540 476 l S 56 452 m 540 452 l S',
        '56 500 m 56 452 l S 220 500 m 220 452 l S 540 500 m 540 452 l S',
        'BT',
        '/F2 10 Tf 1 0 0 1 64 484 Tm (Parameter) Tj 1 0 0 1 228 484 Tm (Rust Equivalent) Tj',
        `/F1 10 Tf 1 0 0 1 64 460 Tm (stage_${n}_mode) Tj 1 0 0 1 228 460 Tm (TableStrategy::Lines) Tj`,
        'ET',
      ].join('\n') + '\n',
      links: n === 1 ? [{ rect: [220, 588, 390, 608] as [number, number, number, number], uri: 'https://github.com/pymupdf/pymupdf4llm' }] : [],
    }));
    return {
      fileName: 'rag_chunking_spec.pdf',
      bytes: compilePdf({
        title: 'LLM Chunking & Compatibility Guide',
        author: 'RAG Architecture Team',
        subject: 'pymupdf4llm-rs Specification',
        pages,
        outlines: [
          { title: '1. RAG Chunking Pipeline Stage 1', level: 1, pageIdx: 0 },
          { title: '2. RAG Chunking Pipeline Stage 2', level: 1, pageIdx: 1 },
          { title: '3. RAG Chunking Pipeline Stage 3', level: 1, pageIdx: 2 },
          { title: '4. RAG Chunking Pipeline Stage 4', level: 1, pageIdx: 3 },
        ],
      }),
    };
  }

  const p1 = [
    'BT',
    '/F2 20 Tf 1 0 0 1 56 718 Tm (Pure-Rust High-Concurrency PDF to Markdown) Tj',
    '/F3 11 Tf 1 0 0 1 56 694 Tm (Deterministic X-Y Cut Column Ordering & Vector Table Synthesis) Tj',
    '/F2 14 Tf 1 0 0 1 56 650 Tm (1. Abstract) Tj',
    '/F1 10.5 Tf 1 0 0 1 56 628 Tm (Modern LLM ingestion pipelines require) Tj',
    '/F1 10.5 Tf 1 0 0 1 56 613 Tm (clean Markdown with ) Tj /F2 10.5 Tf (exact reading order) Tj',
    '/F1 10.5 Tf 1 0 0 1 56 598 Tm (across multi-column layouts and tables.) Tj',
    '/F1 10.5 Tf 1 0 0 1 56 572 Tm (Our crate mirrors ) Tj /F4 9.5 Tf (pymupdf4llm) Tj /F1 10.5 Tf ( in) Tj',
    '/F1 10.5 Tf 1 0 0 1 56 557 Tm (100% safe Rust without C++ FFI.) Tj',
    '/F2 14 Tf 1 0 0 1 324 650 Tm (2. Async Architecture) Tj',
    '/F1 10.5 Tf 1 0 0 1 324 628 Tm (Each PDF document is parsed once into) Tj',
    '/F1 10.5 Tf 1 0 0 1 324 613 Tm (an immutable ) Tj /F4 9.5 Tf (Arc<PdfDocument>) Tj /F1 10.5 Tf ( handle.) Tj',
    '/F1 10.5 Tf 1 0 0 1 324 587 Tm (Tokio semaphores bound concurrent page) Tj',
    '/F1 10.5 Tf 1 0 0 1 324 572 Tm (tasks across Rayon worker threads.) Tj',
    '/F4 9.5 Tf 1 0 0 1 56 510 Tm (let md = to_markdown_async("paper.pdf", opts).await?;) Tj',
    '/F4 9.5 Tf 1 0 0 1 56 496 Tm (println!("Pages parsed: {}", md.len());) Tj',
    'ET',
    '0.8 w 56 450 m 556 450 l S 56 424 m 556 424 l S 56 398 m 556 398 l S 56 372 m 556 372 l S',
    '56 450 m 56 372 l S 210 450 m 210 372 l S 380 450 m 380 372 l S 556 450 m 556 372 l S',
    'BT',
    '/F2 10 Tf 1 0 0 1 64 433 Tm (Engine Mode) Tj 1 0 0 1 218 433 Tm (Concurrency) Tj 1 0 0 1 388 433 Tm (Throughput) Tj',
    '/F1 10 Tf 1 0 0 1 64 407 Tm (Single-Thread Sync) Tj 1 0 0 1 218 407 Tm (1 worker) Tj 1 0 0 1 388 407 Tm (64 pages/sec) Tj',
    '/F1 10 Tf 1 0 0 1 64 381 Tm (Tokio + Rayon Async) Tj 1 0 0 1 218 381 Tm (8 workers) Tj 1 0 0 1 388 381 Tm (490 pages/sec) Tj',
    'ET',
  ].join('\n') + '\n';

  const p2 = [
    'BT',
    '/F2 14 Tf 1 0 0 1 56 706 Tm (3. Table Detection Strategies) Tj',
    '/F1 11 Tf 1 0 0 1 56 678 Tm (\x95 lines_strict: Detects stroked ruling segments while ignoring shaded backgrounds.) Tj',
    '/F1 11 Tf 1 0 0 1 56 658 Tm (\x95 lines: Combines stroked paths and filled cell rectangles.) Tj',
    '/F1 11 Tf 1 0 0 1 56 638 Tm (\x95 text: Infers column gutters from horizontal whitespace alignment.) Tj',
    '/F1 11 Tf 1 0 0 1 56 610 Tm (Refer to the ) Tj /F3 11 Tf (Official PyMuPDF4LLM Repository) Tj /F1 11 Tf ( for compatibility benchmarks.) Tj',
    'ET',
  ].join('\n') + '\n';

  const p3 = [
    'BT',
    '/F2 14 Tf 1 0 0 1 56 706 Tm (4. Header Identification & Benchmarks) Tj',
    '/F1 11 Tf 1 0 0 1 56 678 Tm (IdentifyHeaders builds a character-weighted font-size histogram across the entire PDF) Tj',
    '/F1 11 Tf 1 0 0 1 56 662 Tm (to calibrate the dominant body size and assign H1-H6 headings deterministically.) Tj',
    '/F4 9.5 Tf 1 0 0 1 56 626 Tm (#[tokio::test]) Tj',
    '/F4 9.5 Tf 1 0 0 1 56 612 Tm (async fn verify_chunks() { assert!(chunks.len() == 3); }) Tj',
    'ET',
  ].join('\n') + '\n';

  return {
    fileName: 'pymupdf4llm_rust_architecture.pdf',
    bytes: compilePdf({
      title: 'Pure-Rust High-Concurrency PDF to Markdown',
      author: 'Systems Engineering',
      subject: 'pymupdf4llm-rs Benchmark',
      pages: [
        { stream: p1 },
        { stream: p2, links: [{ rect: [130, 602, 260, 622], uri: 'https://github.com/pymupdf/pymupdf4llm' }] },
        { stream: p3 },
      ],
      outlines: [
        { title: 'Pure-Rust High-Concurrency PDF to Markdown', level: 1, pageIdx: 0 },
        { title: '1. Abstract', level: 2, pageIdx: 0 },
        { title: '2. Async Architecture', level: 2, pageIdx: 0 },
        { title: '3. Table Detection Strategies', level: 2, pageIdx: 1 },
        { title: '4. Header Identification & Benchmarks', level: 2, pageIdx: 2 },
      ],
    }),
  };
}
