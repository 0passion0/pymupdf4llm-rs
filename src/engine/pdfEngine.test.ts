import test from 'node:test';
import assert from 'node:assert/strict';

import { DEFAULT_OPTIONS } from './types';
import { generateSamplePdf } from './samplePdfs';
import { decodeLzw } from './pdfBinaryParser';
import { runPyMuPdf4LlmPipeline } from './pymupdf4llmPipeline';
import { parseMarkdownToBlocks } from './markdownPreview';

test('Multi-column research paper preserves left-to-right column reading order, headings, code, and stroked table', async () => {
  const { fileName, bytes } = generateSamplePdf('research-paper');
  const res = await runPyMuPdf4LlmPipeline(bytes, fileName, {
    ...DEFAULT_OPTIONS,
    pageChunks: true,
    extractWords: true,
    tableStrategy: 'lines_strict',
    hdrInfo: 'identify_headers',
  });

  assert.equal(res.pageCount, 3);
  assert.equal(res.chunks.length, 3);

  const p1 = res.chunks[0];
  assert.equal(p1.metadata.page, 1);
  assert.equal(p1.metadata.page_count, 3);
  assert.equal(p1.tables.length, 1);
  assert.ok(p1.text.includes('# Pure-Rust High-Concurrency PDF to Markdown'));
  assert.ok(p1.text.includes('## 1. Abstract'));
  assert.ok(p1.text.includes('## 2. Async Architecture'));

  const idxAbstract = p1.text.indexOf('## 1. Abstract');
  const idxSafeRust = p1.text.indexOf('100% safe Rust without C++ FFI.');
  const idxAsync = p1.text.indexOf('## 2. Async Architecture');
  assert.ok(idxAbstract < idxSafeRust && idxSafeRust < idxAsync, 'Left column must finish before right column begins');

  assert.ok(p1.text.includes('```'));
  assert.ok(p1.text.includes('|Engine Mode|Concurrency|Throughput|'));
  assert.ok(p1.words && p1.words.length > 15);

  const p2 = res.chunks[1];
  assert.ok(p2.text.includes('- lines_strict:'));
  assert.ok(p2.text.includes('- lines:'));
  assert.ok(p2.text.includes('- text:'));
});

test('Financial report extracts text-aligned table, graphics, embedded image, and respects header/footer toggles', async () => {
  const { fileName, bytes } = generateSamplePdf('financial-report');

  const withHeaders = await runPyMuPdf4LlmPipeline(bytes, fileName, {
    ...DEFAULT_OPTIONS,
    tableStrategy: 'text',
    embedImages: true,
    header: true,
    footer: true,
    margins: { left: 0, top: 10, right: 0, bottom: 10 },
  });
  assert.equal(withHeaders.chunks[0].tables.length, 1);
  assert.ok(withHeaders.chunks[0].images.length >= 1);
  assert.ok(withHeaders.chunks[0].graphics.length >= 1);
  assert.ok(withHeaders.chunks[0].text.includes('|Region|Nodes|Throughput|P99 Latency|'));
  assert.ok(withHeaders.chunks[0].text.includes('CONFIDENTIAL'));
  assert.ok(withHeaders.chunks[0].text.includes('![](data:image/svg+xml;base64,'));

  const suppressed = await runPyMuPdf4LlmPipeline(bytes, fileName, {
    ...DEFAULT_OPTIONS,
    tableStrategy: 'text',
    header: false,
    footer: false,
    margins: { left: 0, top: 10, right: 0, bottom: 10 },
  });
  assert.ok(!suppressed.chunks[0].text.includes('CONFIDENTIAL'));
  assert.ok(!suppressed.chunks[0].text.includes('Internal Telemetry Archive'));
});

test('TocHeaders and pageSeparators work deterministically on RAG spec PDF', async () => {
  const { fileName, bytes } = generateSamplePdf('rag-spec');
  const res = await runPyMuPdf4LlmPipeline(bytes, fileName, {
    ...DEFAULT_OPTIONS,
    hdrInfo: 'toc_headers',
    pageSeparators: true,
    ignoreCode: true,
    pages: [0, 2],
  });

  assert.equal(res.chunks.length, 2);
  assert.equal(res.chunks[0].metadata.page, 1);
  assert.equal(res.chunks[1].metadata.page, 3);
  assert.ok(res.markdown.includes('\n-----\n'));
  assert.ok(!res.markdown.includes('```\n'));
  assert.ok(res.chunks[0].toc_items.length === 1);
});

test('Noisy CJK multi-table PDF decodes FlateDecode, inherited /Resources, Type0 CJK, Form XObject, hygiene cleanup, and 2 separate tables', async () => {
  const { fileName, bytes } = generateSamplePdf('noisy-cjk-multitable');
  const cleanRes = await runPyMuPdf4LlmPipeline(bytes, fileName, {
    ...DEFAULT_OPTIONS,
    margins: { left: 0, top: 10, right: 0, bottom: 10 },
    hygiene: {
      cleanMojibake: true,
      dehyphenate: true,
      dedupFauxBold: true,
      ignoreInvisibleText: true,
      stripWatermarks: true,
      dedupRunningHeaders: true,
    },
  });

  assert.equal(cleanRes.pageCount, 3);
  const p1 = cleanRes.chunks[0];

  // Inherited /Pages /Resources + FlateDecode + Type0 ToUnicode CJK + beginbfrange array + ligature expansion
  assert.ok(p1.text.includes('中文测试'), 'Should decode Type0 CJK ToUnicode characters');
  assert.ok(p1.text.includes('first-class'), 'Should expand fi ligature cleanly');
  assert.ok(!p1.text.includes('\uE001'), 'Should strip unmapped PUA mojibake');

  // Form XObject recursion
  assert.ok(p1.text.includes('Nested Form XObject payload rendered via /Do.'), 'Should extract nested Form XObject');

  // Invisible OCR (3 Tr), diagonal DRAFT watermark, faux-bold dedup, and dehyphenation
  assert.ok(!p1.text.includes('INVISIBLE_OCR_GARBAGE_LAYER'), 'Should drop 3 Tr invisible text');
  assert.ok(!p1.text.includes('DRAFT'), 'Should strip diagonal rotated DRAFT watermark');
  assert.ok(p1.text.includes('High-concurrency extraction'), 'Should de-hyphenate wrapped line');
  const fauxMatches = p1.text.match(/CleanBold/g) ?? [];
  assert.equal(fauxMatches.length, 1, 'Should deduplicate faux-bold overprint to 1 occurrence');

  // Union-Find connected-component multi-table detection on page 1
  assert.equal(p1.tables.length, 2, 'Should detect 2 independent stroked tables on page 1');
  assert.ok(p1.text.includes('|Filter|Action|'));
  assert.ok(p1.text.includes('|Algorithm|Complexity|'));

  // Adjacent hyperlink span merging on page 2
  const p2 = cleanRes.chunks[1];
  assert.ok(
    p2.text.includes('[**PyMuPDF4LLM** *Rust Toolkit*](https://github.com/pymupdf/pymupdf4llm)'),
    'Should merge consecutive spans sharing the same link URI'
  );

  // Cross-page running header/footer deduplication across 3 pages
  assert.ok(!p1.text.includes('RUNNING HEADER ARCHIVE'));
  assert.ok(!p2.text.includes('Confidential Engineering Spec'));
  assert.ok(cleanRes.hygieneStats.mojibakeCharsRemoved >= 1);
  assert.ok(cleanRes.hygieneStats.ligaturesExpanded >= 1);
  assert.equal(cleanRes.hygieneStats.invisibleSpansFiltered, 1);
  assert.equal(cleanRes.hygieneStats.watermarksFiltered, 1);
  assert.equal(cleanRes.hygieneStats.fauxBoldDeduped, 1);
  assert.ok(cleanRes.hygieneStats.dehyphenatedJoins >= 1);
  assert.equal(cleanRes.hygieneStats.runningHeadersRemoved, 6);

  // Verify disabling hygiene toggles preserves noisy spans
  const rawRes = await runPyMuPdf4LlmPipeline(bytes, fileName, {
    ...DEFAULT_OPTIONS,
    margins: { left: 0, top: 10, right: 0, bottom: 10 },
    hygiene: {
      cleanMojibake: false,
      dehyphenate: false,
      dedupFauxBold: false,
      ignoreInvisibleText: false,
      stripWatermarks: false,
      dedupRunningHeaders: false,
    },
  });
  assert.ok(rawRes.chunks[0].text.includes('INVISIBLE_OCR_GARBAGE_LAYER'));
  assert.ok(rawRes.chunks[0].text.includes('DRAFT'));
  assert.equal((rawRes.chunks[0].text.match(/CleanBold/g) ?? []).length, 2);
});

test('LZWDecode decompressor decodes valid PDF LZW bitstreams', () => {
  // LZW encoding (9-bit MSB-first, earlyChange=1) for "ABAB":
  // 256 (100000000), 65 (001000001), 66 (001000010), 258 (100000010), 257 (100000001) + 000 pad
  const lzwSample = new Uint8Array([0x80, 0x10, 0x48, 0x50, 0x28, 0x08]);
  const decoded = new TextDecoder().decode(decodeLzw(lzwSample, true));
  assert.equal(decoded, 'ABAB');
});

test('Markdown block parser handles headings, tables, lists, and fenced code', () => {
  const blocks = parseMarkdownToBlocks('# Title\n\n|A|B|\n|---|---|\n|1|2|\n\n- item 1\n\n```\nlet x = 1;\n```');
  assert.equal(blocks[0].type, 'heading');
  assert.equal(blocks[1].type, 'table');
  assert.equal(blocks[2].type, 'list');
  assert.equal(blocks[3].type, 'code');
});
