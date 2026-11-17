import { SpatialGrid } from './spatial';
import type { PageMetadata, Rect, TocItem } from './types';

export interface TextSpan {
  text: string;
  bbox: Rect;
  origin: [number, number];
  fontName: string;
  fontSize: number;
  isBold: boolean;
  isItalic: boolean;
  isMonospace: boolean;
  isStrikeout: boolean;
  alpha: number;
  renderMode: number;
  rotationDeg: number;
  linkUri: string | null;
}

export interface VectorSegment {
  p0: [number, number];
  p1: [number, number];
  strokeWidth: number;
  isStroked: boolean;
  isFilled: boolean;
}

export interface VectorRect {
  rect: Rect;
  isStroked: boolean;
  isFilled: boolean;
}

export interface RawImagePlacement {
  bbox: Rect;
  width: number;
  height: number;
  colorSpace: string;
  data: Uint8Array;
}

export interface ParsedPage {
  pageIndex: number;
  mediaBox: Rect;
  cropBox: Rect;
  rotation: number;
  spans: TextSpan[];
  segments: VectorSegment[];
  rects: VectorRect[];
  images: RawImagePlacement[];
}

export interface InheritedProps {
  mediaBox?: PdfVal;
  cropBox?: PdfVal;
  rotate?: number;
  resources: Map<string, PdfVal>;
}

export interface ParsedPdfDocument {
  fileName: string;
  pdfVersion: string;
  pageCount: number;
  metadata: Omit<PageMetadata, 'file_path' | 'page_count' | 'page'>;
  toc: TocItem[];
  pagesRaw: Array<{
    pageObjId: number;
    dict: Map<string, PdfVal>;
    inherited: InheritedProps;
  }>;
  objects: Map<number, PdfVal>;
  fontCache: Map<number, FontInfo>;
}

type PdfVal =
  | null
  | boolean
  | number
  | string
  | { kind: 'name'; value: string }
  | { kind: 'bytes'; value: Uint8Array }
  | { kind: 'ref'; num: number; gen: number }
  | PdfVal[]
  | Map<string, PdfVal>
  | { kind: 'stream'; dict: Map<string, PdfVal>; data: Uint8Array };

const textDecoder = new TextDecoder('latin1');
const utf8Decoder = new TextDecoder('utf-8', { fatal: false });

function indexOfBytes(haystack: Uint8Array, needle: Uint8Array, from = 0): number {
  outer: for (let i = from; i <= haystack.length - needle.length; i++) {
    for (let j = 0; j < needle.length; j++) {
      if (haystack[i + j] !== needle[j]) continue outer;
    }
    return i;
  }
  return -1;
}

function lastIndexOfBytes(haystack: Uint8Array, needle: Uint8Array): number {
  outer: for (let i = haystack.length - needle.length; i >= 0; i--) {
    for (let j = 0; j < needle.length; j++) {
      if (haystack[i + j] !== needle[j]) continue outer;
    }
    return i;
  }
  return -1;
}

function isWs(b: number): boolean {
  return b === 0 || b === 9 || b === 10 || b === 12 || b === 13 || b === 32;
}

function isDelimiter(b: number): boolean {
  return (
    b === 40 || // (
    b === 41 || // )
    b === 60 || // <
    b === 62 || // >
    b === 91 || // [
    b === 93 || // ]
    b === 123 || // {
    b === 125 || // }
    b === 47 || // /
    b === 37 // %
  );
}

class PdfLexer {
  pos: number;
  constructor(private data: Uint8Array, pos = 0) {
    this.pos = pos;
  }

  skipWs(): void {
    while (this.pos < this.data.length) {
      const b = this.data[this.pos];
      if (isWs(b)) {
        this.pos++;
      } else if (b === 37) {
        while (this.pos < this.data.length && this.data[this.pos] !== 10 && this.data[this.pos] !== 13) {
          this.pos++;
        }
      } else {
        break;
      }
    }
  }

  parseObject(): PdfVal {
    this.skipWs();
    if (this.pos >= this.data.length) return null;
    const b = this.data[this.pos];

    if (b === 60 && this.data[this.pos + 1] === 60) {
      const dict = this.parseDict();
      const saved = this.pos;
      this.skipWs();
      const kw = textDecoder.decode(this.data.subarray(this.pos, this.pos + 6));
      if (kw === 'stream') {
        this.pos += 6;
        if (this.data[this.pos] === 13) this.pos++;
        if (this.data[this.pos] === 10) this.pos++;
        const start = this.pos;
        const lenVal = dict.get('Length');
        let end = -1;
        if (typeof lenVal === 'number' && lenVal >= 0 && start + lenVal <= this.data.length) {
          end = start + lenVal;
        } else {
          end = indexOfBytes(this.data, new TextEncoder().encode('endstream'), start);
          if (end === -1) end = this.data.length;
        }
        const streamData = this.data.slice(start, end);
        this.pos = end;
        const endKw = indexOfBytes(this.data, new TextEncoder().encode('endstream'), this.pos);
        if (endKw !== -1) this.pos = endKw + 9;
        return { kind: 'stream', dict, data: streamData };
      }
      this.pos = saved;
      return dict;
    }

    if (b === 60) return this.parseHexString();
    if (b === 40) return this.parseLiteralString();
    if (b === 91) return this.parseArray();
    if (b === 47) return this.parseName();

    const slice4 = textDecoder.decode(this.data.subarray(this.pos, this.pos + 5));
    if (slice4.startsWith('true')) {
      this.pos += 4;
      return true;
    }
    if (slice4.startsWith('false')) {
      this.pos += 5;
      return false;
    }
    if (slice4.startsWith('null')) {
      this.pos += 4;
      return null;
    }

    if (b === 43 || b === 45 || b === 46 || (b >= 48 && b <= 57)) {
      return this.parseNumberOrRef();
    }

    this.pos++;
    return null;
  }

  private parseDict(): Map<string, PdfVal> {
    this.pos += 2;
    const dict = new Map<string, PdfVal>();
    while (this.pos < this.data.length) {
      this.skipWs();
      if (this.data[this.pos] === 62 && this.data[this.pos + 1] === 62) {
        this.pos += 2;
        break;
      }
      if (this.data[this.pos] !== 47) break;
      const keyObj = this.parseName();
      const key =
        typeof keyObj === 'object' && keyObj && 'kind' in keyObj && keyObj.kind === 'name'
          ? keyObj.value
          : '';
      const val = this.parseObject();
      if (key) dict.set(key, val);
    }
    return dict;
  }

  private parseArray(): PdfVal[] {
    this.pos += 1;
    const arr: PdfVal[] = [];
    while (this.pos < this.data.length) {
      this.skipWs();
      if (this.data[this.pos] === 93) {
        this.pos += 1;
        break;
      }
      const prev = this.pos;
      arr.push(this.parseObject());
      if (this.pos === prev) this.pos++;
    }
    return arr;
  }

  private parseName(): PdfVal {
    this.pos += 1;
    let out = '';
    while (this.pos < this.data.length) {
      const b = this.data[this.pos];
      if (isWs(b) || isDelimiter(b)) break;
      if (b === 35 && this.pos + 2 < this.data.length) {
        const hex = textDecoder.decode(this.data.subarray(this.pos + 1, this.pos + 3));
        const parsed = Number.parseInt(hex, 16);
        if (!Number.isNaN(parsed)) {
          out += String.fromCharCode(parsed);
          this.pos += 3;
          continue;
        }
      }
      out += String.fromCharCode(b);
      this.pos++;
    }
    return { kind: 'name', value: out };
  }

  private parseLiteralString(): PdfVal {
    this.pos += 1;
    const out: number[] = [];
    let depth = 1;
    while (this.pos < this.data.length && depth > 0) {
      const b = this.data[this.pos++];
      if (b === 40) {
        depth++;
        out.push(40);
      } else if (b === 41) {
        depth--;
        if (depth > 0) out.push(41);
      } else if (b === 92 && this.pos < this.data.length) {
        const esc = this.data[this.pos++];
        if (esc === 110) out.push(10);
        else if (esc === 114) out.push(13);
        else if (esc === 116) out.push(9);
        else if (esc === 98) out.push(8);
        else if (esc === 102) out.push(12);
        else if (esc >= 48 && esc <= 55) {
          let oct = esc - 48;
          for (let k = 0; k < 2; k++) {
            const d = this.data[this.pos];
            if (d >= 48 && d <= 55) {
              oct = oct * 8 + (d - 48);
              this.pos++;
            } else break;
          }
          out.push(oct & 0xff);
        } else {
          out.push(esc);
        }
      } else {
        out.push(b);
      }
    }
    return { kind: 'bytes', value: new Uint8Array(out) };
  }

  private parseHexString(): PdfVal {
    this.pos += 1;
    const nibbles: number[] = [];
    while (this.pos < this.data.length) {
      const b = this.data[this.pos++];
      if (b === 62) break;
      const ch = String.fromCharCode(b);
      const val = Number.parseInt(ch, 16);
      if (!Number.isNaN(val)) nibbles.push(val);
    }
    const bytes = new Uint8Array(Math.ceil(nibbles.length / 2));
    for (let i = 0; i < nibbles.length; i += 2) {
      bytes[i / 2] = (nibbles[i] << 4) | (nibbles[i + 1] ?? 0);
    }
    return { kind: 'bytes', value: bytes };
  }

  private parseNumberOrRef(): PdfVal {
    const start = this.pos;
    let hasDot = false;
    if (this.data[this.pos] === 43 || this.data[this.pos] === 45) this.pos++;
    while (this.pos < this.data.length) {
      const b = this.data[this.pos];
      if (b === 46) {
        hasDot = true;
        this.pos++;
      } else if (b >= 48 && b <= 57) {
        this.pos++;
      } else break;
    }
    const numStr = textDecoder.decode(this.data.subarray(start, this.pos));
    const num = hasDot ? Number.parseFloat(numStr) : Number.parseInt(numStr, 10);
    if (!hasDot && Number.isFinite(num)) {
      const saved = this.pos;
      this.skipWs();
      const gStart = this.pos;
      while (this.pos < this.data.length && this.data[this.pos] >= 48 && this.data[this.pos] <= 57) {
        this.pos++;
      }
      if (this.pos > gStart) {
        const gen = Number.parseInt(textDecoder.decode(this.data.subarray(gStart, this.pos)), 10);
        this.skipWs();
        if (
          this.data[this.pos] === 82 &&
          (isWs(this.data[this.pos + 1]) || isDelimiter(this.data[this.pos + 1]) || this.pos + 1 >= this.data.length)
        ) {
          this.pos++;
          return { kind: 'ref', num, gen };
        }
      }
      this.pos = saved;
    }
    return Number.isFinite(num) ? num : 0;
  }
}

function asDict(val: PdfVal | undefined): Map<string, PdfVal> | undefined {
  if (!val || typeof val !== 'object') return undefined;
  if (val instanceof Map) return val;
  if ('kind' in val && val.kind === 'stream') return val.dict;
  return undefined;
}

function asName(val: PdfVal | undefined): string | undefined {
  if (val && typeof val === 'object' && 'kind' in val && val.kind === 'name') {
    return val.value;
  }
  return undefined;
}

function asRef(val: PdfVal | undefined): { num: number; gen: number } | undefined {
  if (val && typeof val === 'object' && 'kind' in val && val.kind === 'ref') {
    return { num: val.num, gen: val.gen };
  }
  return undefined;
}

function asText(val: PdfVal | undefined): string {
  if (!val || typeof val !== 'object' || !('kind' in val) || val.kind !== 'bytes') return '';
  const b = val.value;
  if (b.length >= 2 && b[0] === 0xfe && b[1] === 0xff) {
    let out = '';
    for (let i = 2; i + 1 < b.length; i += 2) {
      out += String.fromCharCode((b[i] << 8) | b[i + 1]);
    }
    return out;
  }
  return utf8Decoder.decode(b);
}

/**
 * Pure-TS PDF LZWDecode decompressor (variable 9-12 bit codes, clear=256, EOD=257).
 */
export function decodeLzw(data: Uint8Array, earlyChange = true): Uint8Array {
  const out: number[] = [];
  let dict: number[][] = Array.from({ length: 258 }, (_, i) => (i < 256 ? [i] : []));
  let codeLen = 9;
  let nextCode = 258;
  let bitBuf = 0;
  let bitCount = 0;
  let bytePos = 0;
  let prevEntry: number[] | null = null;

  const readCode = (): number | null => {
    while (bitCount < codeLen && bytePos < data.length) {
      bitBuf = (bitBuf << 8) | data[bytePos++];
      bitCount += 8;
    }
    if (bitCount < codeLen) return null;
    bitCount -= codeLen;
    return (bitBuf >> bitCount) & ((1 << codeLen) - 1);
  };

  while (true) {
    const code = readCode();
    if (code === null || code === 257) break;
    if (code === 256) {
      dict = Array.from({ length: 258 }, (_, i) => (i < 256 ? [i] : []));
      codeLen = 9;
      nextCode = 258;
      prevEntry = null;
      continue;
    }
    let entry: number[];
    if (code < dict.length && dict[code].length > 0) {
      entry = dict[code];
    } else if (code === nextCode && prevEntry) {
      entry = [...prevEntry, prevEntry[0]];
    } else {
      break;
    }
    for (const b of entry) out.push(b);
    if (prevEntry && nextCode < 4096) {
      dict[nextCode] = [...prevEntry, entry[0]];
      nextCode++;
      const thresh = earlyChange ? nextCode + 1 : nextCode;
      if (thresh === 512 && codeLen < 10) codeLen = 10;
      else if (thresh === 1024 && codeLen < 11) codeLen = 11;
      else if (thresh === 2048 && codeLen < 12) codeLen = 12;
    }
    prevEntry = entry;
  }
  return new Uint8Array(out);
}

async function decodeStream(dict: Map<string, PdfVal>, data: Uint8Array): Promise<Uint8Array> {
  const filterVal = dict.get('Filter');
  const filters: string[] = [];
  if (asName(filterVal)) filters.push(asName(filterVal)!);
  else if (Array.isArray(filterVal)) {
    for (const item of filterVal) {
      const n = asName(item);
      if (n) filters.push(n);
    }
  }
  const decodeParms = asDict(dict.get('DecodeParms'));
  const earlyChangeVal = decodeParms?.get('EarlyChange');
  const earlyChange = typeof earlyChangeVal === 'number' ? earlyChangeVal !== 0 : true;

  let current = data;
  for (const f of filters) {
    if (f === 'FlateDecode' || f === 'Fl') {
      current = await inflateFlate(current);
    } else if (f === 'LZWDecode' || f === 'LZW') {
      current = decodeLzw(current, earlyChange);
    } else if (f === 'ASCIIHexDecode' || f === 'AHx') {
      const hex = textDecoder.decode(current).replace(/[^0-9a-fA-F]/g, '');
      const out = new Uint8Array(Math.ceil(hex.length / 2));
      for (let i = 0; i < hex.length; i += 2) {
        out[i / 2] = Number.parseInt(hex.slice(i, i + 2).padEnd(2, '0'), 16);
      }
      current = out;
    }
  }
  return current;
}

async function inflateFlate(bytes: Uint8Array): Promise<Uint8Array> {
  if (typeof DecompressionStream !== 'undefined') {
    for (const fmt of ['deflate', 'deflate-raw'] as const) {
      try {
        const ds = new DecompressionStream(fmt);
        const bufPromise = new Response(ds.readable).arrayBuffer();
        const writer = ds.writable.getWriter();
        writer.write(new Uint8Array(bytes)).catch(() => {});
        writer.close().catch(() => {});
        const buf = await bufPromise;
        return new Uint8Array(buf);
      } catch {
        // try fallback raw deflate
      }
    }
  }
  return bytes;
}

export async function parsePdfBytes(bytes: Uint8Array, fileName = 'document.pdf'): Promise<ParsedPdfDocument> {
  const headerMarker = new TextEncoder().encode('%PDF-');
  const headerIdx = indexOfBytes(bytes, headerMarker, 0);
  if (headerIdx === -1) {
    throw new Error('Invalid PDF file: missing %PDF- header');
  }
  let vEnd = headerIdx + 5;
  while (vEnd < bytes.length && bytes[vEnd] !== 10 && bytes[vEnd] !== 13 && bytes[vEnd] !== 32) vEnd++;
  const pdfVersion = textDecoder.decode(bytes.subarray(headerIdx + 5, vEnd));

  const objects = new Map<number, PdfVal>();
  const objPattern = new TextEncoder().encode('obj');
  let cursor = headerIdx;

  while (cursor < bytes.length) {
    const idx = indexOfBytes(bytes, objPattern, cursor);
    if (idx === -1) break;
    const prevOk = idx === 0 || isWs(bytes[idx - 1]);
    const nextOk = idx + 3 >= bytes.length || isWs(bytes[idx + 3]) || bytes[idx + 3] === 60 || bytes[idx + 3] === 91;
    if (prevOk && nextOk) {
      let lineStart = idx - 1;
      while (lineStart > 0 && bytes[lineStart - 1] !== 10 && bytes[lineStart - 1] !== 13) lineStart--;
      const head = textDecoder.decode(bytes.subarray(lineStart, idx)).trim();
      const parts = head.split(/\s+/);
      if (parts.length >= 2) {
        const objNum = Number.parseInt(parts[parts.length - 2], 10);
        const genNum = Number.parseInt(parts[parts.length - 1], 10);
        if (Number.isFinite(objNum) && Number.isFinite(genNum)) {
          const lexer = new PdfLexer(bytes, idx + 3);
          const obj = lexer.parseObject();
          objects.set(objNum, obj);
          const endObjIdx = indexOfBytes(bytes, new TextEncoder().encode('endobj'), lexer.pos);
          cursor = endObjIdx !== -1 ? endObjIdx + 6 : lexer.pos;
          continue;
        }
      }
    }
    cursor = idx + 3;
  }

  let trailer = new Map<string, PdfVal>();
  const trIdx = lastIndexOfBytes(bytes, new TextEncoder().encode('trailer'));
  if (trIdx !== -1) {
    const lexer = new PdfLexer(bytes, trIdx + 7);
    const parsedTr = lexer.parseObject();
    if (parsedTr instanceof Map) trailer = parsedTr;
  }

  if (trailer.size === 0) {
    for (const [id, obj] of objects.entries()) {
      const d = asDict(obj);
      if (d && asName(d.get('Type')) === 'Catalog') {
        trailer.set('Root', { kind: 'ref', num: id, gen: 0 });
        break;
      }
    }
  }

  for (const [, obj] of [...objects.entries()]) {
    if (obj && typeof obj === 'object' && 'kind' in obj && obj.kind === 'stream') {
      if (asName(obj.dict.get('Type')) === 'ObjStm') {
        const dec = await decodeStream(obj.dict, obj.data);
        const n = Number(obj.dict.get('N') ?? 0);
        const first = Number(obj.dict.get('First') ?? 0);
        const headerNums = textDecoder
          .decode(dec.subarray(0, first))
          .trim()
          .split(/\s+/)
          .map((x) => Number.parseInt(x, 10));
        for (let i = 0; i < n; i++) {
          const id = headerNums[i * 2];
          const off = headerNums[i * 2 + 1];
          if (Number.isFinite(id) && Number.isFinite(off)) {
            const lex = new PdfLexer(dec, first + off);
            objects.set(id, lex.parseObject());
          }
        }
      }
    }
  }

  const rootRef = asRef(trailer.get('Root'));
  if (!rootRef) throw new Error('Invalid PDF: missing /Root catalog');
  const catalog = asDict(objects.get(rootRef.num));
  if (!catalog) throw new Error('Invalid PDF: missing Catalog object');
  const pagesRef = asRef(catalog.get('Pages'));
  if (!pagesRef) throw new Error('Invalid PDF: missing /Pages tree');

  const pagesRaw: ParsedPdfDocument['pagesRaw'] = [];
  const resolveRaw = (val: PdfVal | undefined): PdfVal | undefined => {
    const r = asRef(val);
    return r ? objects.get(r.num) : val;
  };

  const walkPages = (refId: number, parentInherited: InheritedProps) => {
    const d = asDict(objects.get(refId));
    if (!d) return;
    const nextInherited: InheritedProps = {
      mediaBox: d.get('MediaBox') ?? parentInherited.mediaBox,
      cropBox: d.get('CropBox') ?? parentInherited.cropBox,
      rotate: typeof d.get('Rotate') === 'number' ? (d.get('Rotate') as number) : parentInherited.rotate,
      resources: new Map(parentInherited.resources),
    };
    const resDict = asDict(resolveRaw(d.get('Resources')));
    if (resDict) {
      for (const [k, v] of resDict.entries()) {
        nextInherited.resources.set(k, v);
      }
    }
    const type = asName(d.get('Type'));
    if (type === 'Page') {
      pagesRaw.push({ pageObjId: refId, dict: d, inherited: nextInherited });
      return;
    }
    const kids = d.get('Kids');
    if (Array.isArray(kids)) {
      for (const kid of kids) {
        const kr = asRef(kid);
        if (kr) walkPages(kr.num, nextInherited);
      }
    }
  };
  walkPages(pagesRef.num, { resources: new Map() });

  const metadata: Omit<PageMetadata, 'file_path' | 'page_count' | 'page'> = {
    format: `PDF ${pdfVersion}`,
    title: '',
    author: '',
    subject: '',
    keywords: '',
    creator: '',
    producer: '',
    creation_date: '',
    mod_date: '',
    encryption: trailer.has('Encrypt') ? 'Standard' : null,
  };
  const infoRef = asRef(trailer.get('Info'));
  if (infoRef) {
    const info = asDict(objects.get(infoRef.num));
    if (info) {
      metadata.title = asText(info.get('Title'));
      metadata.author = asText(info.get('Author'));
      metadata.subject = asText(info.get('Subject'));
      metadata.keywords = asText(info.get('Keywords'));
      metadata.creator = asText(info.get('Creator'));
      metadata.producer = asText(info.get('Producer'));
      metadata.creation_date = asText(info.get('CreationDate'));
      metadata.mod_date = asText(info.get('ModDate'));
    }
  }

  const toc: TocItem[] = [];
  const outlinesRef = asRef(catalog.get('Outlines'));
  if (outlinesRef) {
    const outlinesDict = asDict(objects.get(outlinesRef.num));
    const firstRef = asRef(outlinesDict?.get('First'));
    const walkOutline = (refNum: number, level: number) => {
      let cur: number | undefined = refNum;
      let guard = 0;
      while (cur !== undefined && guard++ < 512) {
        const od = asDict(objects.get(cur));
        if (!od) break;
        const title = asText(od.get('Title'));
        let pageNum = 1;
        const dest = od.get('Dest');
        if (Array.isArray(dest) && dest[0]) {
          const targetRef = asRef(dest[0]);
          if (targetRef) {
            const foundIdx = pagesRaw.findIndex((p) => p.pageObjId === targetRef.num);
            if (foundIdx !== -1) pageNum = foundIdx + 1;
          }
        }
        if (title) toc.push([level, title, pageNum]);
        const childRef = asRef(od.get('First'));
        if (childRef) walkOutline(childRef.num, level + 1);
        cur = asRef(od.get('Next'))?.num;
      }
    };
    if (firstRef) walkOutline(firstRef.num, 1);
  }

  return {
    fileName,
    pdfVersion,
    pageCount: pagesRaw.length,
    metadata,
    toc,
    pagesRaw,
    objects,
    fontCache: new Map(),
  };
}

export interface FontInfo {
  baseFont: string;
  isBold: boolean;
  isItalic: boolean;
  isMonospace: boolean;
  isTwoByte: boolean;
  isMacRoman: boolean;
  toUnicode: Map<number, string>;
  differences: Map<number, string>;
  widths: Map<number, number>;
  defaultWidth: number;
}

const WIN_ANSI_HIGH: Record<number, string> = {
  0x80: '€', 0x82: '‚', 0x83: 'ƒ', 0x84: '„', 0x85: '…', 0x86: '†', 0x87: '‡',
  0x88: 'ˆ', 0x89: '‰', 0x8a: 'Š', 0x8b: '‹', 0x8c: 'Œ', 0x8e: 'Ž', 0x91: '‘',
  0x92: '’', 0x93: '“', 0x94: '”', 0x95: '•', 0x96: '–', 0x97: '—', 0x98: '˜',
  0x99: '™', 0x9a: 'š', 0x9b: '›', 0x9c: 'œ', 0x9e: 'ž', 0x9f: 'Ÿ',
};

const MAC_ROMAN_HIGH: Record<number, string> = {
  0x80: 'Ä', 0x81: 'Å', 0x82: 'Ç', 0x83: 'É', 0x84: 'Ñ', 0x85: 'Ö', 0x86: 'Ü', 0x87: 'á',
  0x88: 'à', 0x89: 'â', 0x8a: 'ä', 0x8b: 'ã', 0x8c: 'å', 0x8d: 'ç', 0x8e: 'é', 0x8f: 'è',
  0x90: 'ê', 0x91: 'ë', 0x92: 'í', 0x93: 'ì', 0x94: 'î', 0x95: 'ï', 0x96: 'ñ', 0x97: 'ó',
  0x98: 'ò', 0x99: 'ô', 0x9a: 'ö', 0x9b: 'õ', 0x9c: 'ú', 0x9d: 'ù', 0x9e: 'û', 0x9f: 'ü',
  0xa0: '†', 0xa1: '°', 0xa2: '¢', 0xa3: '£', 0xa4: '§', 0xa5: '•', 0xa6: '¶', 0xa7: 'ß',
  0xa8: '®', 0xa9: '©', 0xaa: '™', 0xab: '´', 0xac: '¨', 0xad: '≠', 0xae: 'Æ', 0xaf: 'Ø',
  0xb0: '∞', 0xb1: '±', 0xb2: '≤', 0xb3: '≥', 0xb4: '¥', 0xb5: 'µ', 0xb6: '∂', 0xb7: '∑',
  0xb8: '∏', 0xb9: 'π', 0xba: '∫', 0xbb: 'ª', 0xbc: 'º', 0xbd: 'Ω', 0xbe: 'æ', 0xbf: 'ø',
  0xc7: '«', 0xc8: '»', 0xc9: '…', 0xca: ' ', 0xd0: '–', 0xd1: '—', 0xd2: '“', 0xd3: '”',
  0xd4: '‘', 0xd5: '’', 0xdb: '€',
};

const ADOBE_GLYPH_MAP: Record<string, string> = {
  space: ' ', exclam: '!', quotedbl: '"', numbersign: '#', dollar: '$', percent: '%',
  ampersand: '&', quotesingle: "'", parenleft: '(', parenright: ')', asterisk: '*',
  plus: '+', comma: ',', hyphen: '-', period: '.', slash: '/', colon: ':', semicolon: ';',
  less: '<', equal: '=', greater: '>', question: '?', at: '@', bracketleft: '[',
  backslash: '\\', bracketright: ']', asciicircum: '^', underscore: '_', grave: '`',
  braceleft: '{', bar: '|', braceright: '}', asciitilde: '~', bullet: '•', endash: '–',
  emdash: '—', ellipsis: '…', quotedblleft: '“', quotedblright: '”', quoteleft: '‘',
  quoteright: '’', quotesinglbase: '‚', quotedblbase: '„', guillemotleft: '«',
  guillemotright: '»', dagger: '†', daggerdbl: '‡', trademark: '™', copyright: '©',
  registered: '®', Euro: '€', sterling: '£', yen: '¥', cent: '¢', currency: '¤',
  section: '§', paragraph: '¶', degree: '°', plusminus: '±', multiply: '×', divide: '÷',
  minus: '−', approxequal: '≈', notequal: '≠', lessequal: '≤', greaterequal: '≥',
  fi: 'ﬁ', fl: 'ﬂ', ff: 'ﬀ', ffi: 'ﬃ', ffl: 'ﬄ', germandbls: 'ß', ae: 'æ', AE: 'Æ',
  oe: 'œ', OE: 'Œ', oslash: 'ø', Oslash: 'Ø', aacute: 'á', agrave: 'à', acircumflex: 'â',
  adieresis: 'ä', atilde: 'ã', aring: 'å', ccedilla: 'ç', eacute: 'é', egrave: 'è',
  ecircumflex: 'ê', edieresis: 'ë', iacute: 'í', igrave: 'ì', icircumflex: 'î',
  idieresis: 'ï', ntilde: 'ñ', oacute: 'ó', ograve: 'ò', ocircumflex: 'ô',
  odieresis: 'ö', otilde: 'õ', uacute: 'ú', ugrave: 'ù', ucircumflex: 'û', udieresis: 'ü',
  alpha: 'α', beta: 'β', gamma: 'γ', delta: 'δ', mu: 'μ', pi: 'π', Sigma: 'Σ', Omega: 'Ω',
};

function glyphNameToUnicode(name: string): string | null {
  if (ADOBE_GLYPH_MAP[name]) return ADOBE_GLYPH_MAP[name];
  if (name.length === 1) return name;
  if (name.startsWith('uni') && name.length >= 7) {
    const code = Number.parseInt(name.slice(3, 7), 16);
    if (Number.isFinite(code)) return String.fromCodePoint(code);
  }
  return null;
}

function hexToUtf16String(hexClean: string): string {
  if (hexClean.length === 0) return '';
  const padded = hexClean.length % 2 === 1 ? `0${hexClean}` : hexClean;
  if (padded.length === 2) {
    return String.fromCodePoint(Number.parseInt(padded, 16));
  }
  let out = '';
  for (let i = 0; i + 3 < padded.length; i += 4) {
    out += String.fromCharCode(Number.parseInt(padded.slice(i, i + 4), 16));
  }
  return out;
}

function parseToUnicodeCmap(bytes: Uint8Array, map: Map<number, string>): void {
  const text = textDecoder.decode(bytes);
  const bfcharRe = /beginbfchar([\s\S]*?)endbfchar/g;
  let m: RegExpExecArray | null;
  while ((m = bfcharRe.exec(text)) !== null) {
    const pairRe = /<([0-9a-fA-F]+)>\s*<([0-9a-fA-F]+)>/g;
    let pm: RegExpExecArray | null;
    while ((pm = pairRe.exec(m[1])) !== null) {
      const src = Number.parseInt(pm[1], 16);
      const dst = hexToUtf16String(pm[2]);
      if (Number.isFinite(src) && dst) map.set(src, dst);
    }
  }

  const bfrangeRe = /beginbfrange([\s\S]*?)endbfrange/g;
  while ((m = bfrangeRe.exec(text)) !== null) {
    const lines = m[1].split(/\r?\n/);
    for (const line of lines) {
      const arrMatch = line.match(/^\s*<([0-9a-fA-F]+)>\s*<([0-9a-fA-F]+)>\s*\[([^\]]+)\]/);
      if (arrMatch) {
        const start = Number.parseInt(arrMatch[1], 16);
        const end = Number.parseInt(arrMatch[2], 16);
        const items = [...arrMatch[3].matchAll(/<([0-9a-fA-F]+)>/g)];
        for (let k = 0; k < items.length && start + k <= end; k++) {
          const dst = hexToUtf16String(items[k][1]);
          if (dst) map.set(start + k, dst);
        }
        continue;
      }
      const singleMatch = line.match(/^\s*<([0-9a-fA-F]+)>\s*<([0-9a-fA-F]+)>\s*<([0-9a-fA-F]+)>/);
      if (singleMatch) {
        const start = Number.parseInt(singleMatch[1], 16);
        const end = Number.parseInt(singleMatch[2], 16);
        const dstStart = Number.parseInt(singleMatch[3], 16);
        const span = Math.min(end - start, 4096);
        for (let k = 0; k <= span; k++) {
          map.set(start + k, String.fromCodePoint(dstStart + k));
        }
      }
    }
  }
}

async function parseFontInfo(doc: ParsedPdfDocument, fontVal: PdfVal): Promise<FontInfo> {
  const r = asRef(fontVal);
  if (r && doc.fontCache.has(r.num)) {
    return doc.fontCache.get(r.num)!;
  }
  const fd = asDict(resolveObj(doc, fontVal));
  const baseFont = fd ? (asName(fd.get('BaseFont')) ?? 'Helvetica') : 'Helvetica';
  const lower = baseFont.toLowerCase();
  const subtype = fd ? (asName(fd.get('Subtype')) ?? '') : '';
  const encName = fd ? (asName(fd.get('Encoding')) ?? '') : '';
  const isTwoByte =
    subtype === 'Type0' ||
    encName.includes('Identity-H') ||
    encName.includes('Identity-V') ||
    encName.includes('UCS2') ||
    encName.includes('UTF16');
  const isMacRoman = encName === 'MacRomanEncoding';

  let isBold = lower.includes('bold') || lower.includes('black') || lower.includes('heavy') || lower.includes('semibold');
  let isItalic = lower.includes('italic') || lower.includes('oblique') || lower.includes('slant');
  let isMonospace =
    lower.includes('courier') ||
    lower.includes('mono') ||
    lower.includes('consolas') ||
    lower.includes('menlo') ||
    lower.includes('inconsolata');

  const desc = fd ? asDict(resolveObj(doc, fd.get('FontDescriptor'))) : undefined;
  if (desc) {
    const flags = Number(desc.get('Flags') ?? 0);
    if ((flags & 1) !== 0) isMonospace = true;
    if ((flags & (1 << 6)) !== 0) isItalic = true;
    if ((flags & (1 << 18)) !== 0) isBold = true;
  }

  const toUnicode = new Map<number, string>();
  const tuObj = fd ? resolveObj(doc, fd.get('ToUnicode')) : undefined;
  if (tuObj && typeof tuObj === 'object' && 'kind' in tuObj && tuObj.kind === 'stream') {
    const cmapBytes = await decodeStream(tuObj.dict, tuObj.data);
    parseToUnicodeCmap(cmapBytes, toUnicode);
  }

  const differences = new Map<number, string>();
  const encDict = fd ? asDict(resolveObj(doc, fd.get('Encoding'))) : undefined;
  if (encDict) {
    const diffs = resolveObj(doc, encDict.get('Differences'));
    if (Array.isArray(diffs)) {
      let code = 0;
      for (const item of diffs) {
        if (typeof item === 'number') {
          code = Math.max(0, Math.min(255, Math.floor(item)));
        } else {
          const gname = asName(item);
          if (gname) {
            const mapped = glyphNameToUnicode(gname);
            if (mapped) differences.set(code, mapped);
            code = (code + 1) & 0xff;
          }
        }
      }
    }
  }

  const widths = new Map<number, number>();
  let defaultWidth = isMonospace ? 600 : 520;
  if (fd) {
    const firstChar = Number(fd.get('FirstChar') ?? 0);
    const wArr = resolveObj(doc, fd.get('Widths'));
    if (Array.isArray(wArr)) {
      wArr.forEach((w, idx) => {
        if (typeof w === 'number') widths.set(firstChar + idx, w);
      });
    }
    const descendants = resolveObj(doc, fd.get('DescendantFonts'));
    if (Array.isArray(descendants) && descendants[0]) {
      const cidFont = asDict(resolveObj(doc, descendants[0]));
      if (cidFont) {
        if (typeof cidFont.get('DW') === 'number') defaultWidth = cidFont.get('DW') as number;
        const wCid = resolveObj(doc, cidFont.get('W'));
        if (Array.isArray(wCid)) {
          let i = 0;
          while (i + 1 < wCid.length) {
            const cFirst = Number(wCid[i]);
            const second = wCid[i + 1];
            if (Array.isArray(second)) {
              second.forEach((v, k) => {
                if (typeof v === 'number') widths.set(cFirst + k, v);
              });
              i += 2;
            } else if (typeof second === 'number' && typeof wCid[i + 2] === 'number') {
              const cLast = Math.min(cFirst + 4096, second);
              const wVal = wCid[i + 2] as number;
              for (let cid = cFirst; cid <= cLast; cid++) widths.set(cid, wVal);
              i += 3;
            } else {
              i++;
            }
          }
        }
      }
    }
  }

  const info: FontInfo = {
    baseFont,
    isBold,
    isItalic,
    isMonospace,
    isTwoByte,
    isMacRoman,
    toUnicode,
    differences,
    widths,
    defaultWidth,
  };
  if (r) doc.fontCache.set(r.num, info);
  return info;
}

function decodeBytesWithFont(raw: Uint8Array, font: FontInfo): { text: string; widthUnits: number } {
  if (font.isTwoByte) {
    let text = '';
    let widthUnits = 0;
    for (let i = 0; i + 1 < raw.length; i += 2) {
      const code = (raw[i] << 8) | raw[i + 1];
      widthUnits += font.widths.get(code) ?? (code < 128 ? 520 : 1000);
      const mapped = font.toUnicode.get(code);
      if (mapped) {
        text += mapped;
      } else if (code >= 0x20) {
        text += String.fromCharCode(code);
      }
    }
    return { text, widthUnits };
  }

  let hasToUnicode = false;
  let hasDiff = false;
  let widthUnits = 0;
  for (let i = 0; i < raw.length; i++) {
    const b = raw[i];
    widthUnits += font.widths.get(b) ?? font.defaultWidth;
    if (font.toUnicode.has(b)) hasToUnicode = true;
    if (font.differences.has(b)) hasDiff = true;
  }

  if (hasToUnicode || hasDiff || font.isMacRoman) {
    let text = '';
    for (let i = 0; i < raw.length; i++) {
      const b = raw[i];
      if (font.toUnicode.has(b)) text += font.toUnicode.get(b)!;
      else if (font.differences.has(b)) text += font.differences.get(b)!;
      else if (font.isMacRoman && b >= 0x80 && MAC_ROMAN_HIGH[b]) text += MAC_ROMAN_HIGH[b];
      else if (b >= 0x80 && b <= 0x9f && WIN_ANSI_HIGH[b]) text += WIN_ANSI_HIGH[b];
      else text += String.fromCharCode(b);
    }
    return { text, widthUnits };
  }

  let hasHighWinAnsi = false;
  for (let i = 0; i < raw.length; i++) {
    if (raw[i] >= 0x80 && raw[i] <= 0x9f) {
      hasHighWinAnsi = true;
      break;
    }
  }
  if (!hasHighWinAnsi) {
    return { text: utf8Decoder.decode(raw), widthUnits };
  }
  let text = '';
  for (let i = 0; i < raw.length; i++) {
    const b = raw[i];
    text += WIN_ANSI_HIGH[b] ?? String.fromCharCode(b);
  }
  return { text, widthUnits };
}

function resolveObj(doc: ParsedPdfDocument, val: PdfVal | undefined): PdfVal | undefined {
  if (!val) return undefined;
  const r = asRef(val);
  if (r) return doc.objects.get(r.num);
  return val;
}

function parseRectVal(doc: ParsedPdfDocument, val: PdfVal | undefined): Rect | null {
  const arr = resolveObj(doc, val);
  if (!Array.isArray(arr) || arr.length !== 4) return null;
  const nums = arr.map((x) => (typeof x === 'number' ? x : 0));
  return { x0: nums[0], y0: nums[1], x1: nums[2], y1: nums[3] };
}

function mulMat(a: number[], b: number[]): number[] {
  return [
    a[0] * b[0] + a[1] * b[2],
    a[0] * b[1] + a[1] * b[3],
    a[2] * b[0] + a[3] * b[2],
    a[2] * b[1] + a[3] * b[3],
    a[4] * b[0] + a[5] * b[2] + b[4],
    a[4] * b[1] + a[5] * b[3] + b[5],
  ];
}

function txPt(m: number[], x: number, y: number): [number, number] {
  return [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
}

type StreamTok =
  | { kind: 'num'; v: number }
  | { kind: 'name'; v: string }
  | { kind: 'bytes'; v: Uint8Array }
  | { kind: 'arr'; v: StreamTok[] }
  | { kind: 'op'; v: string };

function tokenizeStream(data: Uint8Array): StreamTok[] {
  const tokens: StreamTok[] = [];
  let i = 0;
  while (i < data.length) {
    const b = data[i];
    if (isWs(b)) {
      i++;
      continue;
    }
    if (b === 37) {
      while (i < data.length && data[i] !== 10 && data[i] !== 13) i++;
      continue;
    }
    if (b === 40) {
      i++;
      const bytes: number[] = [];
      let depth = 1;
      while (i < data.length && depth > 0) {
        const c = data[i++];
        if (c === 40) {
          depth++;
          bytes.push(40);
        } else if (c === 41) {
          depth--;
          if (depth > 0) bytes.push(41);
        } else if (c === 92 && i < data.length) {
          const esc = data[i++];
          if (esc === 110) bytes.push(10);
          else if (esc === 114) bytes.push(13);
          else if (esc === 116) bytes.push(9);
          else bytes.push(esc);
        } else {
          bytes.push(c);
        }
      }
      tokens.push({ kind: 'bytes', v: new Uint8Array(bytes) });
    } else if (b === 60 && data[i + 1] !== 60) {
      i++;
      let hex = '';
      while (i < data.length && data[i] !== 62) {
        hex += String.fromCharCode(data[i++]);
      }
      i++;
      const clean = hex.replace(/[^0-9a-fA-F]/g, '');
      const out = new Uint8Array(Math.ceil(clean.length / 2));
      for (let k = 0; k < clean.length; k += 2) {
        out[k / 2] = Number.parseInt(clean.slice(k, k + 2).padEnd(2, '0'), 16);
      }
      tokens.push({ kind: 'bytes', v: out });
    } else if (b === 91) {
      i++;
      const start = i;
      let depth = 1;
      while (i < data.length && depth > 0) {
        if (data[i] === 40) {
          i++;
          let d = 1;
          while (i < data.length && d > 0) {
            if (data[i] === 92) {
              i += 2;
              continue;
            }
            if (data[i] === 40) d++;
            else if (data[i] === 41) d--;
            i++;
          }
          continue;
        }
        if (data[i] === 91) depth++;
        else if (data[i] === 93) depth--;
        i++;
      }
      tokens.push({ kind: 'arr', v: tokenizeStream(data.subarray(start, i - 1)) });
    } else if (b === 47) {
      i++;
      const start = i;
      while (i < data.length && !isWs(data[i]) && !isDelimiter(data[i])) i++;
      tokens.push({ kind: 'name', v: textDecoder.decode(data.subarray(start, i)) });
    } else if (b === 43 || b === 45 || b === 46 || (b >= 48 && b <= 57)) {
      const start = i++;
      while (i < data.length && ((data[i] >= 48 && data[i] <= 57) || data[i] === 46)) i++;
      const v = Number.parseFloat(textDecoder.decode(data.subarray(start, i)));
      tokens.push({ kind: 'num', v: Number.isFinite(v) ? v : 0 });
    } else {
      const start = i;
      while (i < data.length && !isWs(data[i]) && !isDelimiter(data[i])) i++;
      const op = textDecoder.decode(data.subarray(start, i));
      if (op) tokens.push({ kind: 'op', v: op });
      else i++;
    }
  }
  return tokens;
}

interface GraphicsState {
  ctm: number[];
  fontName: string;
  fontSize: number;
  charSpacing: number;
  horizScale: number;
  leading: number;
  lineWidth: number;
  alpha: number;
  renderMode: number;
}

function rotatePoint(x: number, y: number, w: number, h: number, rotation: number): [number, number] {
  if (rotation === 90) return [h - y, x];
  if (rotation === 180) return [w - x, h - y];
  if (rotation === 270) return [y, w - x];
  return [x, y];
}

function rotateRect(r: Rect, w: number, h: number, rotation: number): Rect {
  const [ax, ay] = rotatePoint(r.x0, r.y0, w, h, rotation);
  const [bx, by] = rotatePoint(r.x1, r.y1, w, h, rotation);
  return { x0: Math.min(ax, bx), y0: Math.min(ay, by), x1: Math.max(ax, bx), y1: Math.max(ay, by) };
}

export async function extractPage(
  doc: ParsedPdfDocument,
  pageIndex: number,
  fontsizeLimit: number,
  ignoreImages: boolean
): Promise<ParsedPage> {
  const rawPage = doc.pagesRaw[pageIndex];
  if (!rawPage) throw new Error(`Page ${pageIndex} out of bounds`);

  const mediaBox =
    parseRectVal(doc, rawPage.dict.get('MediaBox') ?? rawPage.inherited.mediaBox) ??
    { x0: 0, y0: 0, x1: 612, y1: 792 };
  const rawCropBox =
    parseRectVal(doc, rawPage.dict.get('CropBox') ?? rawPage.inherited.cropBox) ?? mediaBox;
  const rawRot =
    typeof rawPage.dict.get('Rotate') === 'number'
      ? (rawPage.dict.get('Rotate') as number)
      : (rawPage.inherited.rotate ?? 0);
  const rotation = ((rawRot % 360) + 360) % 360;
  const rawHeight = rawCropBox.y1 - rawCropBox.y0;
  const rawWidth = rawCropBox.x1 - rawCropBox.x0;
  const cropBox: Rect =
    rotation === 90 || rotation === 270
      ? { x0: 0, y0: 0, x1: rawHeight, y1: rawWidth }
      : rawCropBox;

  const resources = new Map<string, PdfVal>(rawPage.inherited.resources);
  const pageRes = asDict(resolveObj(doc, rawPage.dict.get('Resources')));
  if (pageRes) {
    for (const [k, v] of pageRes.entries()) resources.set(k, v);
  }

  const loadFontsFromResources = async (res: Map<string, PdfVal>): Promise<Map<string, FontInfo>> => {
    const fonts = new Map<string, FontInfo>();
    const fontDict = asDict(resolveObj(doc, res.get('Font')));
    if (fontDict) {
      for (const [k, fVal] of fontDict.entries()) {
        fonts.set(k, await parseFontInfo(doc, fVal));
      }
    }
    return fonts;
  };

  const loadAlphas = (res: Map<string, PdfVal>): Map<string, number> => {
    const alphas = new Map<string, number>();
    const gsDict = asDict(resolveObj(doc, res.get('ExtGState')));
    if (gsDict) {
      for (const [k, v] of gsDict.entries()) {
        const d = asDict(resolveObj(doc, v));
        const a = d?.get('ca') ?? d?.get('CA');
        if (typeof a === 'number') alphas.set(k, Math.max(0, Math.min(1, a)));
      }
    }
    return alphas;
  };

  const pageFonts = await loadFontsFromResources(resources);

  const links: Array<{ rect: Rect; uri: string }> = [];
  const annots = resolveObj(doc, rawPage.dict.get('Annots'));
  if (Array.isArray(annots)) {
    for (const a of annots) {
      const ad = asDict(resolveObj(doc, a));
      if (!ad || asName(ad.get('Subtype')) !== 'Link') continue;
      const r = parseRectVal(doc, ad.get('Rect'));
      const act = asDict(resolveObj(doc, ad.get('A')));
      const uri = act ? asText(act.get('URI')) : '';
      if (r && uri) {
        links.push({
          rect: {
            x0: Math.min(r.x0, r.x1),
            y0: rawHeight - Math.max(r.y0, r.y1),
            x1: Math.max(r.x0, r.x1),
            y1: rawHeight - Math.min(r.y0, r.y1),
          },
          uri,
        });
      }
    }
  }

  const contentChunks: Uint8Array[] = [];
  const collectContent = async (val: PdfVal | undefined) => {
    const resolved = resolveObj(doc, val);
    if (!resolved) return;
    if (Array.isArray(resolved)) {
      for (const item of resolved) await collectContent(item);
    } else if (typeof resolved === 'object' && 'kind' in resolved && resolved.kind === 'stream') {
      contentChunks.push(await decodeStream(resolved.dict, resolved.data));
    }
  };
  await collectContent(rawPage.dict.get('Contents'));

  const totalLen = contentChunks.reduce((acc, c) => acc + c.length + 1, 0);
  const merged = new Uint8Array(totalLen);
  let pos = 0;
  for (const c of contentChunks) {
    merged.set(c, pos);
    pos += c.length;
    merged[pos++] = 10;
  }

  const spans: TextSpan[] = [];
  const segments: VectorSegment[] = [];
  const rects: VectorRect[] = [];
  const images: RawImagePlacement[] = [];

  const interpretStream = async (
    streamBytes: Uint8Array,
    res: Map<string, PdfVal>,
    fonts: Map<string, FontInfo>,
    initialGs: GraphicsState,
    depth: number
  ) => {
    if (depth > 8) return;
    const tokens = tokenizeStream(streamBytes);
    const extAlphas = loadAlphas(res);
    const stack: GraphicsState[] = [];
    let gs: GraphicsState = { ...initialGs, ctm: [...initialGs.ctm] };
    let tm = [1, 0, 0, 1, 0, 0];
    let lm = [1, 0, 0, 1, 0, 0];
    let operands: StreamTok[] = [];
    let pathPts: Array<[number, number]> = [];
    let pathRects: Rect[] = [];

    const emitBytes = (raw: Uint8Array) => {
      const font = fonts.get(gs.fontName) ?? {
        baseFont: 'Helvetica',
        isBold: false,
        isItalic: false,
        isMonospace: false,
        isTwoByte: false,
        isMacRoman: false,
        toUnicode: new Map(),
        differences: new Map(),
        widths: new Map(),
        defaultWidth: 520,
      };
      const { text, widthUnits } = decodeBytesWithFont(raw, font);
      if (!text) return;
      const combined = mulMat(tm, gs.ctm);
      const scale = Math.max(1, Math.hypot(combined[0], combined[1]));
      const effSize = Math.max(1, gs.fontSize * scale);
      if (effSize < fontsizeLimit) return;

      const rotationDeg = (Math.atan2(combined[1], combined[0]) * 180) / Math.PI;
      const adv = (widthUnits / 1000) * gs.fontSize * (gs.horizScale / 100) + text.length * gs.charSpacing;
      const x0 = combined[4];
      const baseY = rawHeight - combined[5];
      const x1 = x0 + adv * Math.max(1, Math.abs(gs.ctm[0]));
      const y0 = baseY - effSize * 0.85;
      const y1 = baseY + effSize * 0.2;
      tm[4] += adv;
      spans.push({
        text,
        bbox: {
          x0: Math.min(x0, x1),
          y0: Math.min(y0, y1),
          x1: Math.max(Math.max(x0, x1), Math.min(x0, x1) + 1),
          y1: Math.max(y0, y1),
        },
        origin: [x0, baseY],
        fontName: font.baseFont,
        fontSize: effSize,
        isBold: font.isBold,
        isItalic: font.isItalic,
        isMonospace: font.isMonospace,
        isStrikeout: false,
        alpha: gs.alpha,
        renderMode: gs.renderMode,
        rotationDeg,
        linkUri: null,
      });
    };

    for (const tok of tokens) {
      if (tok.kind !== 'op') {
        operands.push(tok);
        continue;
      }
      const op = tok.v;
      if (op === 'q') {
        stack.push({ ...gs, ctm: [...gs.ctm] });
      } else if (op === 'Q') {
        const prev = stack.pop();
        if (prev) gs = prev;
      } else if (op === 'cm' && operands.length >= 6) {
        const n = operands.length;
        const m = operands.slice(n - 6).map((x) => (x.kind === 'num' ? x.v : 0));
        gs.ctm = mulMat(m, gs.ctm);
      } else if (op === 'w' && operands[0]?.kind === 'num') {
        gs.lineWidth = operands[0].v;
      } else if (op === 'gs' && operands[0]?.kind === 'name') {
        const a = extAlphas.get(operands[0].v);
        if (a !== undefined) gs.alpha = a;
      } else if (op === 'BT') {
        tm = [1, 0, 0, 1, 0, 0];
        lm = [1, 0, 0, 1, 0, 0];
      } else if (op === 'Tf' && operands.length >= 2) {
        if (operands[0].kind === 'name') gs.fontName = operands[0].v;
        if (operands[1].kind === 'num') gs.fontSize = Math.max(1, Math.abs(operands[1].v));
      } else if (op === 'Tr' && operands[0]?.kind === 'num') {
        gs.renderMode = Math.max(0, Math.min(7, Math.round(operands[0].v)));
      } else if (op === 'Tc' && operands[0]?.kind === 'num') {
        gs.charSpacing = operands[0].v;
      } else if (op === 'Tz' && operands[0]?.kind === 'num') {
        gs.horizScale = operands[0].v;
      } else if (op === 'TL' && operands[0]?.kind === 'num') {
        gs.leading = operands[0].v;
      } else if (op === 'Tm' && operands.length >= 6) {
        const n = operands.length;
        const m = operands.slice(n - 6).map((x) => (x.kind === 'num' ? x.v : 0));
        tm = [...m];
        lm = [...m];
      } else if (op === 'Td' && operands.length >= 2) {
        const tx = operands[0].kind === 'num' ? operands[0].v : 0;
        const ty = operands[1].kind === 'num' ? operands[1].v : 0;
        lm = mulMat([1, 0, 0, 1, tx, ty], lm);
        tm = [...lm];
      } else if (op === 'TD' && operands.length >= 2) {
        const tx = operands[0].kind === 'num' ? operands[0].v : 0;
        const ty = operands[1].kind === 'num' ? operands[1].v : 0;
        gs.leading = -ty;
        lm = mulMat([1, 0, 0, 1, tx, ty], lm);
        tm = [...lm];
      } else if (op === 'T*') {
        lm = mulMat([1, 0, 0, 1, 0, -gs.leading], lm);
        tm = [...lm];
      } else if (op === 'Tj' || op === "'" || op === '"') {
        if (op === "'" || op === '"') {
          lm = mulMat([1, 0, 0, 1, 0, -gs.leading], lm);
          tm = [...lm];
        }
        const last = operands[operands.length - 1];
        if (last?.kind === 'bytes') emitBytes(last.v);
      } else if (op === 'TJ') {
        const last = operands[operands.length - 1];
        if (last?.kind === 'arr') {
          for (const item of last.v) {
            if (item.kind === 'bytes') emitBytes(item.v);
            else if (item.kind === 'num') tm[4] += (-item.v / 1000) * gs.fontSize * (gs.horizScale / 100);
          }
        }
      } else if ((op === 'm' || op === 'l') && operands.length >= 2) {
        const x = operands[0].kind === 'num' ? operands[0].v : 0;
        const y = operands[1].kind === 'num' ? operands[1].v : 0;
        const [px, py] = txPt(gs.ctm, x, y);
        pathPts.push([px, rawHeight - py]);
      } else if (op === 're' && operands.length >= 4) {
        const rx = operands[0].kind === 'num' ? operands[0].v : 0;
        const ry = operands[1].kind === 'num' ? operands[1].v : 0;
        const rw = operands[2].kind === 'num' ? operands[2].v : 0;
        const rh = operands[3].kind === 'num' ? operands[3].v : 0;
        const [p0x, p0y] = txPt(gs.ctm, rx, ry);
        const [p1x, p1y] = txPt(gs.ctm, rx + rw, ry + rh);
        pathRects.push({
          x0: Math.min(p0x, p1x),
          y0: rawHeight - Math.max(p0y, p1y),
          x1: Math.max(p0x, p1x),
          y1: rawHeight - Math.min(p0y, p1y),
        });
      } else if (['S', 's', 'f', 'F', 'f*', 'B', 'b', 'B*', 'b*'].includes(op)) {
        const isStroked = ['S', 's', 'B', 'b', 'B*', 'b*'].includes(op);
        const isFilled = ['f', 'F', 'f*', 'B', 'b', 'B*', 'b*'].includes(op);
        for (let k = 0; k + 1 < pathPts.length; k++) {
          segments.push({ p0: pathPts[k], p1: pathPts[k + 1], strokeWidth: gs.lineWidth, isStroked, isFilled });
        }
        for (const r of pathRects) {
          if (isStroked) {
            segments.push({ p0: [r.x0, r.y0], p1: [r.x1, r.y0], strokeWidth: gs.lineWidth, isStroked: true, isFilled });
            segments.push({ p0: [r.x0, r.y1], p1: [r.x1, r.y1], strokeWidth: gs.lineWidth, isStroked: true, isFilled });
            segments.push({ p0: [r.x0, r.y0], p1: [r.x0, r.y1], strokeWidth: gs.lineWidth, isStroked: true, isFilled });
            segments.push({ p0: [r.x1, r.y0], p1: [r.x1, r.y1], strokeWidth: gs.lineWidth, isStroked: true, isFilled });
          }
          rects.push({ rect: r, isStroked, isFilled });
        }
        pathPts = [];
        pathRects = [];
      } else if (op === 'n') {
        pathPts = [];
        pathRects = [];
      } else if (op === 'Do' && operands[0]?.kind === 'name') {
        const xobjs = asDict(resolveObj(doc, res.get('XObject')));
        const xobj = xobjs ? resolveObj(doc, xobjs.get(operands[0].v)) : undefined;
        if (xobj && typeof xobj === 'object' && 'kind' in xobj && xobj.kind === 'stream') {
          const subtype = asName(xobj.dict.get('Subtype'));
          if (subtype === 'Image' && !ignoreImages) {
            const [p0x, p0y] = txPt(gs.ctm, 0, 0);
            const [p1x, p1y] = txPt(gs.ctm, 1, 1);
            images.push({
              bbox: {
                x0: Math.min(p0x, p1x),
                y0: rawHeight - Math.max(p0y, p1y),
                x1: Math.max(p0x, p1x),
                y1: rawHeight - Math.min(p0y, p1y),
              },
              width: Number(xobj.dict.get('Width') ?? 64),
              height: Number(xobj.dict.get('Height') ?? 64),
              colorSpace: asName(xobj.dict.get('ColorSpace')) ?? 'DeviceRGB',
              data: xobj.data,
            });
          } else if (subtype === 'Form') {
            const formBytes = await decodeStream(xobj.dict, xobj.data);
            const formGs: GraphicsState = { ...gs, ctm: [...gs.ctm] };
            const matArr = resolveObj(doc, xobj.dict.get('Matrix'));
            if (Array.isArray(matArr) && matArr.length === 6) {
              const fm = matArr.map((v) => (typeof v === 'number' ? v : 0));
              formGs.ctm = mulMat(fm, gs.ctm);
            }
            const formRes = new Map<string, PdfVal>(res);
            const xRes = asDict(resolveObj(doc, xobj.dict.get('Resources')));
            if (xRes) {
              for (const [rk, rv] of xRes.entries()) formRes.set(rk, rv);
            }
            const formFonts = new Map(fonts);
            const extraFonts = await loadFontsFromResources(formRes);
            for (const [fk, fv] of extraFonts.entries()) formFonts.set(fk, fv);
            await interpretStream(formBytes, formRes, formFonts, formGs, depth + 1);
          }
        }
      }
      operands = [];
    }
  };

  await interpretStream(
    merged,
    resources,
    pageFonts,
    {
      ctm: [1, 0, 0, 1, 0, 0],
      fontName: '',
      fontSize: 12,
      charSpacing: 0,
      horizScale: 100,
      leading: 0,
      lineWidth: 1,
      alpha: 1,
      renderMode: 0,
    },
    0
  );

  if (rotation !== 0) {
    for (const s of spans) {
      s.bbox = rotateRect(s.bbox, rawWidth, rawHeight, rotation);
      s.origin = rotatePoint(s.origin[0], s.origin[1], rawWidth, rawHeight, rotation);
      let r = ((s.rotationDeg - rotation) % 360 + 360) % 360;
      if (r > 180) r -= 360;
      s.rotationDeg = r;
    }
    for (const seg of segments) {
      seg.p0 = rotatePoint(seg.p0[0], seg.p0[1], rawWidth, rawHeight, rotation);
      seg.p1 = rotatePoint(seg.p1[0], seg.p1[1], rawWidth, rawHeight, rotation);
    }
    for (const vr of rects) vr.rect = rotateRect(vr.rect, rawWidth, rawHeight, rotation);
    for (const img of images) img.bbox = rotateRect(img.bbox, rawWidth, rawHeight, rotation);
    for (const lnk of links) lnk.rect = rotateRect(lnk.rect, rawWidth, rawHeight, rotation);
  }

  // Attach links in O(1) average time via SpatialGrid
  if (links.length > 0 && spans.length > 0) {
    const linkGrid = new SpatialGrid(48);
    links.forEach((l, idx) => linkGrid.insert(idx, l.rect));
    for (const s of spans) {
      for (const idx of linkGrid.queryRect(s.bbox)) {
        const l = links[idx];
        const inter =
          s.bbox.x0 < l.rect.x1 && s.bbox.x1 > l.rect.x0 && s.bbox.y0 < l.rect.y1 && s.bbox.y1 > l.rect.y0;
        if (inter) {
          s.linkUri = l.uri;
          break;
        }
      }
    }
  }

  return { pageIndex, mediaBox, cropBox, rotation, spans, segments, rects, images };
}
