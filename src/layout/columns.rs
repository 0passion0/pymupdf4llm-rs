use crate::config::Rect;
use crate::layout::TextLine;
use crate::pdf::TextSpan;

/// Recursive X-Y cut reading-order reconstruction for multi-column PDFs.
/// Splits regions at clear vertical gutters (column gaps >= 12pt) and horizontal bands,
/// guaranteeing left-to-right column reading order without interleaving columns.
pub fn order_spans_into_lines(spans: &[TextSpan], bounds: Rect) -> Vec<TextLine> {
    if spans.is_empty() {
        return Vec::new();
    }
    let mut ordered_spans = Vec::with_capacity(spans.len());
    xy_cut_recursive(spans, bounds, 0, &mut ordered_spans);
    group_spans_into_lines(&ordered_spans)
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
    if spans.len() <= 1 || depth >= 8 {
        let mut rest = spans.to_vec();
        sort_spans_top_left(&mut rest, 4.0);
        out.extend(rest);
        return;
    }

    // 1. Try horizontal band split first (separating full-width headers/abstracts from 2-col body)
    if let Some(split_y) = find_widest_horizontal_gutter(spans, bounds) {
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

    // 2. Try vertical column gutter split (>= 14pt clear vertical corridor)
    if let Some(split_x) = find_widest_vertical_gutter(spans, bounds) {
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

    // Base case: sort top-to-bottom, left-to-right with strict total order
    let mut rest = spans.to_vec();
    sort_spans_top_left(&mut rest, 4.0);
    out.extend(rest);
}

fn find_widest_vertical_gutter(spans: &[TextSpan], bounds: Rect) -> Option<f32> {
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

    let mut best_gap = 14.0f32;
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

fn find_widest_horizontal_gutter(spans: &[TextSpan], bounds: Rect) -> Option<f32> {
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

    let mut best_gap = 18.0f32;
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

fn group_spans_into_lines(spans: &[TextSpan]) -> Vec<TextLine> {
    let mut lines: Vec<Vec<TextSpan>> = Vec::new();
    for span in spans {
        if let Some(last) = lines.last_mut() {
            let ref_y = last[0].origin.1;
            let same_baseline = (span.origin.1 - ref_y).abs() <= (last[0].font_size * 0.45).max(3.5);
            let same_column = last.last().is_some_and(|prev_span| {
                (span.bbox.x0 - prev_span.bbox.x1).abs() < 120.0
                    || span.bbox.x0 >= prev_span.bbox.x0 - 4.0
            });
            if same_baseline && same_column {
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
                    if gap > s.font_size * 0.22 && !raw_text.ends_with(' ') && !s.text.starts_with(' ') {
                        raw_text.push(' ');
                    }
                }
                raw_text.push_str(&s.text);
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
