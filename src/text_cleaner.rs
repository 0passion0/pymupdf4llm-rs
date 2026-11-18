use crate::config::TextHygieneConfig;
use crate::pdf::TextSpan;
use std::collections::HashMap;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct HygieneStats {
    pub mojibake_chars_removed: usize,
    pub ligatures_expanded: usize,
    pub faux_bold_deduped: usize,
    pub watermarks_filtered: usize,
    pub invisible_spans_filtered: usize,
    pub dehyphenated_joins: usize,
    pub running_headers_removed: usize,
}

/// Maps known Symbol / Wingdings PUA codepoints to standard Unicode, expands typographic ligatures,
/// strips control bytes, zero-width noise, and unmapped PUA/replacement glyphs.
pub fn sanitize_span_text(
    input: &str,
    cfg: &TextHygieneConfig,
    stats: &mut HygieneStats,
) -> String {
    if !cfg.clean_mojibake {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            // Known typographic ligatures
            '\u{FB00}' => {
                out.push_str("ff");
                stats.ligatures_expanded += 1;
            }
            '\u{FB01}' => {
                out.push_str("fi");
                stats.ligatures_expanded += 1;
            }
            '\u{FB02}' => {
                out.push_str("fl");
                stats.ligatures_expanded += 1;
            }
            '\u{FB03}' => {
                out.push_str("ffi");
                stats.ligatures_expanded += 1;
            }
            '\u{FB04}' => {
                out.push_str("ffl");
                stats.ligatures_expanded += 1;
            }
            '\u{FB05}' | '\u{FB06}' => {
                out.push_str("st");
                stats.ligatures_expanded += 1;
            }
            // Non-breaking / exotic spaces -> standard space
            '\u{00A0}' | '\u{2002}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => {
                out.push(' ');
            }
            // Common Wingdings / Symbol PUA bullets & checkmarks
            '\u{F0B7}' | '\u{F0A7}' | '\u{F076}' => out.push('•'),
            '\u{F0FC}' | '\u{F052}' => out.push('✓'),
            '\u{F0E0}' | '\u{F0E8}' => out.push_str("->"),
            // Strip soft hyphens, zero-width chars, BOM, replacement char, and unmapped PUA
            '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{FEFF}' | '\u{FFFD}' => {
                stats.mojibake_chars_removed += 1;
            }
            '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{FFFFD}' | '\u{100000}'..='\u{10FFFD}' => {
                stats.mojibake_chars_removed += 1;
            }
            c if c.is_control() && c != '\n' && c != '\t' => {
                stats.mojibake_chars_removed += 1;
            }
            c => out.push(c),
        }
    }
    out
}

/// Filters invisible text (`3 Tr`), rotated diagonal watermarks, and deduplicates
/// faux-bold double-struck spans (same text within <= 1.2pt offset).
pub fn filter_and_dedup_spans(
    spans: Vec<TextSpan>,
    cfg: &TextHygieneConfig,
    stats: &mut HygieneStats,
) -> Vec<TextSpan> {
    let mut kept: Vec<TextSpan> = Vec::with_capacity(spans.len());

    for span in spans {
        if cfg.ignore_invisible_text && span.render_mode == 3 {
            stats.invisible_spans_filtered += 1;
            continue;
        }
        if cfg.strip_watermarks && span.rotation_deg.abs() > 12.0 && span.alpha < 0.75 {
            stats.watermarks_filtered += 1;
            continue;
        }
        if cfg.strip_watermarks && span.rotation_deg.abs() > 20.0 && span.font_size >= 24.0 {
            let upper = span.text.trim().to_ascii_uppercase();
            if matches!(
                upper.as_str(),
                "CONFIDENTIAL"
                    | "DRAFT"
                    | "WATERMARK"
                    | "DO NOT DISTRIBUTE"
                    | "SAMPLE"
                    | "INTERNAL USE ONLY"
            ) {
                stats.watermarks_filtered += 1;
                continue;
            }
        }

        if cfg.dedup_faux_bold {
            let trimmed = span.text.trim();
            if !trimmed.is_empty() {
                let is_dup = kept.iter().rev().take(12).any(|prev| {
                    prev.text.trim() == trimmed
                        && (prev.bbox.x0 - span.bbox.x0).abs() <= 1.5
                        && (prev.bbox.y0 - span.bbox.y0).abs() <= 1.5
                        && (prev.font_size - span.font_size).abs() <= 0.5
                });
                if is_dup {
                    stats.faux_bold_deduped += 1;
                    continue;
                }
            }
        }
        kept.push(span);
    }

    kept
}

/// Cross-page running header/footer detector: identifies lines appearing in the top 10% or
/// bottom 10% of >= 3 pages with normalized digit-masking (e.g. `Page 1 of 10` == `Page 2 of 10`).
pub fn detect_running_headers_footers(
    pages: &[Vec<(f32, f32, String)>], // per page: (y_ratio_0_to_1, x0, text)
    min_pages: usize,
) -> HashMap<String, usize> {
    let mut freq: HashMap<String, usize> = HashMap::new();
    if pages.len() < min_pages {
        return freq;
    }
    for page_lines in pages {
        let mut seen_this_page = std::collections::HashSet::new();
        for (y_ratio, _x0, text) in page_lines {
            if *y_ratio <= 0.10 || *y_ratio >= 0.90 {
                let sig = normalize_running_signature(text);
                if sig.len() >= 3 && seen_this_page.insert(sig.clone()) {
                    *freq.entry(sig).or_insert(0) += 1;
                }
            }
        }
    }
    freq.retain(|_, count| *count >= min_pages);
    freq
}

pub fn normalize_running_signature(text: &str) -> String {
    text.trim()
        .chars()
        .map(|c| {
            if c.is_ascii_digit() {
                '#'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Joins hyphenated line wraps (`architec-` + `ture` -> `architecture`) when the next token
/// starts with a lowercase alphabetic character.
pub fn should_dehyphenate_join(prev_line: &str, next_line: &str) -> bool {
    let prev = prev_line.trim_end();
    let next = next_line.trim_start();
    if !prev.ends_with('-') || prev.ends_with("--") || prev.ends_with(" -") {
        return false;
    }
    let before_hyphen = prev[..prev.len() - 1].chars().last();
    let first_next = next.chars().next();
    matches!(
        (before_hyphen, first_next),
        (Some(a), Some(b)) if a.is_alphabetic() && b.is_lowercase()
    )
}
