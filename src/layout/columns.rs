use unicode_normalization::UnicodeNormalization;

use crate::config::Rect;
use crate::layout::TextLine;
use crate::pdf::TextSpan;

/// Recursive XY-Cut++ reading-order reconstruction for multi-column PDFs.
/// Uses adaptive font-scaled gutters and spanning-band isolation so full-width headers,
/// abstracts, or captions never force multi-column bodies to interleave.
pub fn order_spans_into_lines(spans: &[TextSpan], bounds: Rect) -> Vec<TextLine> {
    if spans.is_empty() {
        return Vec::new();
    }
    let mut ordered_spans = Vec::with_capacity(spans.len());
    xy_cut_recursive(spans, bounds, 0, &mut ordered_spans);
    group_spans_into_lines(&ordered_spans)
}

fn median_font_size(spans: &[TextSpan]) -> f32 {
    if spans.is_empty() {
        return 11.0;
    }
    let mut sizes: Vec<f32> = spans
        .iter()
        .map(|s| s.font_size)
        .filter(|s| s.is_finite() && *s > 0.0)
        .collect();
    if sizes.is_empty() {
        return 11.0;
    }
    sizes.sort_by(f32::total_cmp);
    sizes[sizes.len() / 2]
}

fn sort_spans_top_left(rest: &mut [TextSpan], row_tol: f32) {
    if rest.len() <= 1 {
        return;
    }
    rest.sort_by(|a, b| {
        a.bbox
            .center_y()
            .total_cmp(&b.bbox.center_y())
            .then_with(|| a.bbox.x0.total_cmp(&b.bbox.x0))
    });

    let mut start = 0;
    while start < rest.len() {
        let ref_y = rest[start].bbox.center_y();
        let mut end = start + 1;
        while end < rest.len() && (rest[end].bbox.center_y() - ref_y).abs() <= row_tol {
            end += 1;
        }
        if end - start > 1 {
            rest[start..end].sort_by(|a, b| a.bbox.x0.total_cmp(&b.bbox.x0));
        }
        start = end;
    }
}

fn xy_cut_recursive(spans: &[TextSpan], bounds: Rect, depth: usize, out: &mut Vec<TextSpan>) {
    if spans.len() <= 1 || depth >= 10 {
        let mut rest = spans.to_vec();
        sort_spans_top_left(&mut rest, 4.0);
        out.extend(rest);
        return;
    }

    let med_fs = median_font_size(spans);
    let min_h_gap = (med_fs * 1.45).clamp(12.0, 28.0);
    let min_v_gap = (med_fs * 1.15).clamp(11.0, 24.0);

    // 1. Try horizontal band split first (separating full-width headers/abstracts from body)
    if let Some(split_y) = find_widest_horizontal_gutter(spans, bounds, min_h_gap) {
        let (top, bottom): (Vec<TextSpan>, Vec<TextSpan>) = spans
            .iter()
            .cloned()
            .partition(|s| s.bbox.center_y() < split_y);
        if !top.is_empty() && !bottom.is_empty() {
            xy_cut_recursive(
                &top,
                Rect::new(bounds.x0, bounds.y0, bounds.x1, split_y),
                depth + 1,
                out,
            );
            xy_cut_recursive(
                &bottom,
                Rect::new(bounds.x0, split_y, bounds.x1, bounds.y1),
                depth + 1,
                out,
            );
            return;
        }
    }

    // 2. Try vertical column gutter split
    if let Some(split_x) = find_widest_vertical_gutter(spans, bounds, min_v_gap) {
        let (left, right): (Vec<TextSpan>, Vec<TextSpan>) = spans
            .iter()
            .cloned()
            .partition(|s| s.bbox.center_x() < split_x);
        if !left.is_empty() && !right.is_empty() {
            xy_cut_recursive(
                &left,
                Rect::new(bounds.x0, bounds.y0, split_x, bounds.y1),
                depth + 1,
                out,
            );
            xy_cut_recursive(
                &right,
                Rect::new(split_x, bounds.y0, bounds.x1, bounds.y1),
                depth + 1,
                out,
            );
            return;
        }
    }

    // 3. XY-Cut++ spanning-band isolation: if wide spanning elements (title/caption/footnote)
    // bridge across an otherwise clear multi-column body without a wide horizontal gutter,
    // peel off the topmost spanning band so the multi-column subregions split cleanly.
    if spans.len() >= 4 && bounds.width() > 160.0 {
        let wide_threshold = bounds.width() * 0.58;
        let (wide, narrow): (Vec<TextSpan>, Vec<TextSpan>) = spans
            .iter()
            .cloned()
            .partition(|s| s.bbox.width() > wide_threshold);

        if !wide.is_empty()
            && narrow.len() >= 2
            && find_widest_vertical_gutter(&narrow, bounds, min_v_gap).is_some()
        {
            let mut sorted_wide = wide;
            sorted_wide.sort_by(|a, b| a.bbox.y0.total_cmp(&b.bbox.y0));
            let mut band_y0 = sorted_wide[0].bbox.y0;
            let mut band_y1 = sorted_wide[0].bbox.y1;
            for ws in &sorted_wide[1..] {
                if ws.bbox.y0 <= band_y1 + med_fs * 0.6 {
                    band_y0 = band_y0.min(ws.bbox.y0);
                    band_y1 = band_y1.max(ws.bbox.y1);
                } else {
                    break;
                }
            }

            let mut above = Vec::new();
            let mut band = Vec::new();
            let mut below = Vec::new();
            for s in spans {
                let cy = s.bbox.center_y();
                if cy < band_y0 - 1.0 {
                    above.push(s.clone());
                } else if cy > band_y1 + 1.0 {
                    below.push(s.clone());
                } else {
                    band.push(s.clone());
                }
            }

            let non_empty_parts = usize::from(!above.is_empty())
                + usize::from(!band.is_empty())
                + usize::from(!below.is_empty());
            if non_empty_parts >= 2 {
                if !above.is_empty() {
                    xy_cut_recursive(
                        &above,
                        Rect::new(bounds.x0, bounds.y0, bounds.x1, band_y0.max(bounds.y0)),
                        depth + 1,
                        out,
                    );
                }
                if !band.is_empty() {
                    let mut band_sorted = band;
                    sort_spans_top_left(&mut band_sorted, (med_fs * 0.4).max(3.5));
                    out.extend(band_sorted);
                }
                if !below.is_empty() {
                    xy_cut_recursive(
                        &below,
                        Rect::new(bounds.x0, band_y1.min(bounds.y1), bounds.x1, bounds.y1),
                        depth + 1,
                        out,
                    );
                }
                return;
            }
        }
    }

    // Base case: sort top-to-bottom, left-to-right with strict total order
    let mut rest = spans.to_vec();
    sort_spans_top_left(&mut rest, (med_fs * 0.4).max(3.5));
    out.extend(rest);
}

fn find_widest_vertical_gutter(spans: &[TextSpan], bounds: Rect, min_gap: f32) -> Option<f32> {
    let mut x_intervals: Vec<(f32, f32)> = spans
        .iter()
        .map(|s| (s.bbox.x0.max(bounds.x0), s.bbox.x1.min(bounds.x1)))
        .filter(|(a, b)| b > a)
        .collect();
    if x_intervals.len() < 2 {
        return None;
    }
    x_intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));

    let mut merged: Vec<(f32, f32)> = Vec::new();
    for (a, b) in x_intervals {
        if let Some(last) = merged.last_mut() {
            if a <= last.1 + 1.0 {
                last.1 = last.1.max(b);
                continue;
            }
        }
        merged.push((a, b));
    }

    let mut best_gap = min_gap;
    let mut best_mid = None;
    for pair in merged.windows(2) {
        let gap = pair[1].0 - pair[0].1;
        if gap > best_gap {
            best_gap = gap;
            best_mid = Some((pair[0].1 + pair[1].0) * 0.5);
        }
    }
    best_mid
}

fn find_widest_horizontal_gutter(spans: &[TextSpan], bounds: Rect, min_gap: f32) -> Option<f32> {
    let mut y_intervals: Vec<(f32, f32)> = spans
        .iter()
        .map(|s| (s.bbox.y0.max(bounds.y0), s.bbox.y1.min(bounds.y1)))
        .filter(|(a, b)| b > a)
        .collect();
    if y_intervals.len() < 2 {
        return None;
    }
    y_intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));

    let mut merged: Vec<(f32, f32)> = Vec::new();
    for (a, b) in y_intervals {
        if let Some(last) = merged.last_mut() {
            if a <= last.1 + 1.0 {
                last.1 = last.1.max(b);
                continue;
            }
        }
        merged.push((a, b));
    }

    let mut best_gap = min_gap;
    let mut best_mid = None;
    for pair in merged.windows(2) {
        let gap = pair[1].0 - pair[0].1;
        if gap > best_gap {
            best_gap = gap;
            best_mid = Some((pair[0].1 + pair[1].0) * 0.5);
        }
    }
    best_mid
}

#[inline]
pub(crate) fn is_cjk_char(c: char) -> bool {
    matches!(
        c as u32,
        0x3000..=0x30FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
            | 0xFF00..=0xFFEF
    )
}

pub(crate) fn should_insert_span_space(
    prev_text: &str,
    next_text: &str,
    gap: f32,
    font_size: f32,
) -> bool {
    let fs = font_size.max(1.0);
    if gap <= fs * 0.12 {
        return false;
    }
    let (Some(last_c), Some(first_c)) = (prev_text.chars().last(), next_text.chars().next()) else {
        return false;
    };
    if last_c.is_whitespace() || first_c.is_whitespace() {
        return false;
    }
    if is_cjk_char(last_c) && is_cjk_char(first_c) {
        return gap > fs * 0.55;
    }
    if matches!(
        first_c,
        ',' | '.'
            | ';'
            | ':'
            | '!'
            | '?'
            | ')'
            | ']'
            | '}'
            | '，'
            | '。'
            | '；'
            | '：'
            | '！'
            | '？'
    ) {
        return gap > fs * 0.45;
    }
    if matches!(last_c, '(' | '[' | '{' | '（' | '【' | '《') {
        return gap > fs * 0.45;
    }
    true
}

fn group_spans_into_lines(spans: &[TextSpan]) -> Vec<TextLine> {
    let mut lines: Vec<Vec<TextSpan>> = Vec::new();
    for span in spans {
        if let Some(last) = lines.last_mut() {
            let ref_y = last[0].origin.1;
            let line_max_fs = last
                .iter()
                .map(|s| s.font_size)
                .fold(0.0f32, f32::max)
                .max(1.0);
            let same_baseline = (span.origin.1 - ref_y).abs() <= (line_max_fs * 0.45).max(3.5);

            // Keep superscript/subscript/small footnote markers attached to their parent line
            let script_attached = !same_baseline
                && (span.is_superscript
                    || span.is_subscript
                    || span.font_size <= line_max_fs * 0.82)
                && {
                    let line_y0 = last.iter().map(|s| s.bbox.y0).fold(f32::INFINITY, f32::min);
                    let line_y1 = last
                        .iter()
                        .map(|s| s.bbox.y1)
                        .fold(f32::NEG_INFINITY, f32::max);
                    span.bbox.y1 >= line_y0 - 2.0 && span.bbox.y0 <= line_y1 + 2.0
                };

            let same_column = last.last().is_some_and(|prev_span| {
                (span.bbox.x0 - prev_span.bbox.x1).abs() < 120.0
                    || span.bbox.x0 >= prev_span.bbox.x0 - 4.0
            });
            if (same_baseline || script_attached) && same_column {
                last.push(span.clone());
                continue;
            }
        }
        lines.push(vec![span.clone()]);
    }

    lines
        .into_iter()
        .map(|mut line_spans| {
            line_spans.sort_by(|a, b| a.bbox.x0.total_cmp(&b.bbox.x0));
            let mut bbox = line_spans[0].bbox;
            let mut max_font_size = 0.0f32;
            let mut all_bold = true;
            let mut all_mono = true;
            let mut raw_text = String::new();

            for (i, s) in line_spans.iter().enumerate() {
                bbox = bbox.union(&s.bbox);
                max_font_size = max_font_size.max(s.font_size);
                if !s.is_bold && !s.text.trim().is_empty() {
                    all_bold = false;
                }
                if !s.is_monospace && !s.text.trim().is_empty() {
                    all_mono = false;
                }
                if i > 0 {
                    let gap = s.bbox.x0 - line_spans[i - 1].bbox.x1;
                    let ref_fs = s.font_size.min(line_spans[i - 1].font_size);
                    if should_insert_span_space(&raw_text, &s.text, gap, ref_fs) {
                        raw_text.push(' ');
                    }
                }
                raw_text.push_str(&s.text);
            }
            if !raw_text.is_ascii() {
                raw_text = raw_text.nfc().collect();
            }

            TextLine {
                bbox,
                spans: line_spans,
                text: raw_text,
                max_font_size,
                is_all_bold: all_bold,
                is_all_monospace: all_mono,
            }
        })
        .collect()
}
