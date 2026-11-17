export type TableStrategy = 'lines_strict' | 'lines' | 'text' | 'explicit';
export type HeaderMode = 'identify_headers' | 'toc_headers' | 'disabled';
export type ImageFormat = 'png' | 'jpeg' | 'webp';

export interface Margins {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface Rect {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

export interface TextHygieneConfig {
  cleanMojibake: boolean;
  dehyphenate: boolean;
  dedupFauxBold: boolean;
  ignoreInvisibleText: boolean;
  stripWatermarks: boolean;
  dedupRunningHeaders: boolean;
}

export const DEFAULT_HYGIENE_CONFIG: TextHygieneConfig = {
  cleanMojibake: true,
  dehyphenate: true,
  dedupFauxBold: true,
  ignoreInvisibleText: true,
  stripWatermarks: true,
  dedupRunningHeaders: false,
};

export interface HygieneStats {
  mojibakeCharsRemoved: number;
  ligaturesExpanded: number;
  fauxBoldDeduped: number;
  watermarksFiltered: number;
  invisibleSpansFiltered: number;
  dehyphenatedJoins: number;
  runningHeadersRemoved: number;
}

export function createEmptyHygieneStats(): HygieneStats {
  return {
    mojibakeCharsRemoved: 0,
    ligaturesExpanded: 0,
    fauxBoldDeduped: 0,
    watermarksFiltered: 0,
    invisibleSpansFiltered: 0,
    dehyphenatedJoins: 0,
    runningHeadersRemoved: 0,
  };
}

export interface ToMarkdownOptions {
  pages: number[] | null;
  hdrInfo: HeaderMode;
  maxHeaderLevels: number;
  writeImages: boolean;
  embedImages: boolean;
  dpi: number;
  imagePath: string;
  imageFormat: ImageFormat;
  imageSizeLimit: number;
  forceText: boolean;
  margins: Margins;
  pageChunks: boolean;
  pageSeparators: boolean;
  pageWidth: number;
  pageHeight: number | null;
  tableStrategy: TableStrategy;
  graphicsLimit: number | null;
  ignoreCode: boolean;
  ignoreImages: boolean;
  ignoreGraphics: boolean;
  extractWords: boolean;
  fontsizeLimit: number;
  useGlyphs: boolean;
  detectBgColor: boolean;
  ignoreAlpha: boolean;
  header: boolean;
  footer: boolean;
  showProgress: boolean;
  maxPageConcurrency: number;
  maxDocConcurrency: number;
  hygiene: TextHygieneConfig;
}

export const DEFAULT_OPTIONS: ToMarkdownOptions = {
  pages: null,
  hdrInfo: 'identify_headers',
  maxHeaderLevels: 6,
  writeImages: false,
  embedImages: false,
  dpi: 150,
  imagePath: '',
  imageFormat: 'png',
  imageSizeLimit: 0.05,
  forceText: true,
  margins: { left: 0, top: 50, right: 0, bottom: 50 },
  pageChunks: false,
  pageSeparators: false,
  pageWidth: 612,
  pageHeight: null,
  tableStrategy: 'lines_strict',
  graphicsLimit: null,
  ignoreCode: false,
  ignoreImages: false,
  ignoreGraphics: false,
  extractWords: false,
  fontsizeLimit: 3.0,
  useGlyphs: false,
  detectBgColor: true,
  ignoreAlpha: false,
  header: true,
  footer: true,
  showProgress: true,
  maxPageConcurrency: 4,
  maxDocConcurrency: 4,
  hygiene: { ...DEFAULT_HYGIENE_CONFIG },
};

export type TocItem = [number, string, number];
export type ExtractedWord = [number, number, number, number, string, number, number, number];

export interface PageMetadata {
  format: string;
  title: string;
  author: string;
  subject: string;
  keywords: string;
  creator: string;
  producer: string;
  creation_date: string;
  mod_date: string;
  encryption: string | null;
  file_path: string;
  page_count: number;
  page: number;
}

export interface ExtractedTable {
  bbox: [number, number, number, number];
  rows: number;
  columns: number;
}

export interface ExtractedImage {
  number: number;
  bbox: [number, number, number, number];
  width: number;
  height: number;
  cs_name: string;
  ext: string;
  file_name?: string;
}

export interface ExtractedGraphic {
  bbox: [number, number, number, number];
}

export interface PageChunk {
  metadata: PageMetadata;
  toc_items: TocItem[];
  tables: ExtractedTable[];
  images: ExtractedImage[];
  graphics: ExtractedGraphic[];
  text: string;
  words?: ExtractedWord[];
}

export interface WorkerTrace {
  workerId: number;
  docName: string;
  pageIndex: number;
  startMs: number;
  endMs: number;
  durationMs: number;
  charsExtracted: number;
  tablesFound: number;
}

export interface ParseExecutionResult {
  fileName: string;
  fileSizeBytes: number;
  pageCount: number;
  markdown: string;
  chunks: PageChunk[];
  elapsedMs: number;
  throughputPagesPerSec: number;
  workerTraces: WorkerTrace[];
  detectedHeaderLevels: Array<{ fontSize: number; level: number }>;
  bodyFontSize: number;
  hygieneStats: HygieneStats;
}

export interface CrateFileEntry {
  path: string;
  sizeBytes: number;
  lines: number;
  content?: string;
}

export interface CompatibilityRow {
  pythonParam: string;
  rustField: string;
  defaultValue: string;
  category: 'Core' | 'Headers' | 'Tables' | 'Media' | 'Async' | 'Output' | 'Hygiene';
  description: string;
}
