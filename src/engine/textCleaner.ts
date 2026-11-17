import type { HygieneStats, TextHygieneConfig } from './types';
import type { TextSpan } from './pdfBinaryParser';

const LIGATURE_MAP: Record<string, string> = {
  '\uFB00': 'ff',
  '\uFB01': 'fi',
  '\uFB02': 'fl',
  '\uFB03': 'ffi',
  '\uFB04': 'ffl',
  '\uFB05': 'st',
  '\uFB06': 'st',
};

const WATERMARK_WORDS = new Set([
  'CONFIDENTIAL',
  'DRAFT',
  'WATERMARK',
  'DO NOT DISTRIBUTE',
  'SAMPLE',
  'INTERNAL USE ONLY',
]);

/**
 * Expands typographic ligatures, normalizes Symbol/Wingdings PUA bullets & checkmarks,
 * and strips soft hyphens, zero-width noise, control bytes, and unmapped PUA/replacement glyphs.
 */
export function sanitizeSpanText(
  input: string,
  cfg: TextHygieneConfig,
  stats: HygieneStats
): string {
  if (!cfg.cleanMojibake) return input;
  let out = '';
  for (const ch of input) {
    const lig = LIGATURE_MAP[ch];
    if (lig) {
      out += lig;
      stats.ligaturesExpanded++;
      continue;
    }
    const code = ch.codePointAt(0) ?? 0;
    if (
      code === 0x00a0 ||
      (code >= 0x2002 && code <= 0x200a) ||
      code === 0x202f ||
      code === 0x205f ||
      code === 0x3000
    ) {
      out += ' ';
      continue;
    }
    if (code === 0xf0b7 || code === 0xf0a7 || code === 0xf076) {
      out += '•';
      continue;
    }
    if (code === 0xf0fc || code === 0xf052) {
      out += '✓';
      continue;
    }
    if (code === 0xf0e0 || code === 0xf0e8) {
      out += '->';
      continue;
    }
    if (
      code === 0x00ad ||
      (code >= 0x200b && code <= 0x200f) ||
      code === 0xfeff ||
      code === 0xfffd
    ) {
      stats.mojibakeCharsRemoved++;
      continue;
    }
    if (
      (code >= 0xe000 && code <= 0xf8ff) ||
      (code >= 0xf0000 && code <= 0xffffd) ||
      (code >= 0x100000 && code <= 0x10fffd)
    ) {
      stats.mojibakeCharsRemoved++;
      continue;
    }
    if ((code < 0x20 || (code >= 0x7f && code <= 0x9f)) && code !== 0x0a && code !== 0x09) {
      stats.mojibakeCharsRemoved++;
      continue;
    }
    out += ch;
  }
  return out;
}

/**
 * Filters invisible OCR text (`3 Tr`), rotated diagonal watermarks, and deduplicates
 * faux-bold overprinted spans within <= 1.5pt horizontal/vertical offset.
 */
export function filterAndDedupSpans(
  spans: TextSpan[],
  cfg: TextHygieneConfig,
  stats: HygieneStats
): TextSpan[] {
  const kept: TextSpan[] = [];
  for (const span of spans) {
    if (cfg.ignoreInvisibleText && span.renderMode === 3) {
      stats.invisibleSpansFiltered++;
      continue;
    }
    if (cfg.stripWatermarks && Math.abs(span.rotationDeg) > 12 && span.alpha < 0.75) {
      stats.watermarksFiltered++;
      continue;
    }
    if (cfg.stripWatermarks && Math.abs(span.rotationDeg) > 20 && span.fontSize >= 24) {
      const upper = span.text.trim().toUpperCase();
      if (WATERMARK_WORDS.has(upper)) {
        stats.watermarksFiltered++;
        continue;
      }
    }
    if (cfg.dedupFauxBold) {
      const trimmed = span.text.trim();
      if (trimmed.length > 0) {
        let isDup = false;
        const start = Math.max(0, kept.length - 12);
        for (let i = kept.length - 1; i >= start; i--) {
          const prev = kept[i];
          if (
            prev.text.trim() === trimmed &&
            Math.abs(prev.bbox.x0 - span.bbox.x0) <= 1.5 &&
            Math.abs(prev.bbox.y0 - span.bbox.y0) <= 1.5 &&
            Math.abs(prev.fontSize - span.fontSize) <= 0.5
          ) {
            isDup = true;
            break;
          }
        }
        if (isDup) {
          stats.fauxBoldDeduped++;
          continue;
        }
      }
    }
    kept.push(span);
  }
  return kept;
}

export function normalizeRunningSignature(text: string): string {
  return text
    .trim()
    .toLowerCase()
    .replace(/\d/g, '#')
    .replace(/\s+/g, ' ');
}

export function detectRunningHeadersFooters(
  pages: Array<Array<{ yRatio: number; text: string }>>,
  minPages = 3
): Set<string> {
  const freq = new Map<string, number>();
  if (pages.length < minPages) return new Set();
  for (const lines of pages) {
    const seenPage = new Set<string>();
    for (const ln of lines) {
      if (ln.yRatio <= 0.1 || ln.yRatio >= 0.9) {
        const sig = normalizeRunningSignature(ln.text);
        if (sig.length >= 3 && !seenPage.has(sig)) {
          seenPage.add(sig);
          freq.set(sig, (freq.get(sig) ?? 0) + 1);
        }
      }
    }
  }
  const repeated = new Set<string>();
  for (const [sig, count] of freq.entries()) {
    if (count >= minPages) repeated.add(sig);
  }
  return repeated;
}

export function shouldDehyphenateJoin(prevLine: string, nextLine: string): boolean {
  const prev = prevLine.trimEnd();
  const next = nextLine.trimStart();
  if (!prev.endsWith('-') || prev.endsWith('--') || prev.endsWith(' -') || prev.length < 2 || next.length === 0) {
    return false;
  }
  const beforeHyphen = prev[prev.length - 2];
  const firstNext = next[0];
  return /[a-zA-Z]/.test(beforeHyphen) && /[a-z]/.test(firstNext);
}
