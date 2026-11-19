use std::collections::HashMap;

use super::tokenizer::StreamToken;
use super::GraphicsState;
use crate::config::{Rect, ToMarkdownOptions};
use crate::layout::columns::is_cjk_char;
use crate::pdf::font::PdfFont;
use crate::pdf::TextSpan;

#[derive(Debug, Clone, Copy)]
pub(super) struct PageCoordMap {
    crop_x0: f32,
    crop_y1: f32,
}

impl PageCoordMap {
    #[inline]
    pub(super) fn from_crop_box(raw_crop_box: Rect) -> Self {
        Self {
            crop_x0: raw_crop_box.x0,
            crop_y1: raw_crop_box.y1,
        }
    }

    #[inline]
    pub(super) fn map_point(self, pdf_x: f32, pdf_y: f32) -> (f32, f32) {
        (pdf_x - self.crop_x0, self.crop_y1 - pdf_y)
    }
}

#[inline]
pub(super) fn cmyk_to_rgb(c: f32, m: f32, y: f32, k: f32) -> (f32, f32, f32) {
    let c = c.clamp(0.0, 1.0);
    let m = m.clamp(0.0, 1.0);
    let y = y.clamp(0.0, 1.0);
    let k = k.clamp(0.0, 1.0);
    let inv_k = 1.0 - k;
    ((1.0 - c) * inv_k, (1.0 - m) * inv_k, (1.0 - y) * inv_k)
}

#[inline]
pub(super) fn transform_point(m: [f32; 6], x: f32, y: f32) -> (f32, f32) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

#[inline]
pub(super) fn mul_matrix(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

#[inline]
pub(super) fn pop_matrix(ops: &[StreamToken]) -> [f32; 6] {
    let n = ops.len();
    [
        as_num(&ops[n - 6]),
        as_num(&ops[n - 5]),
        as_num(&ops[n - 4]),
        as_num(&ops[n - 3]),
        as_num(&ops[n - 2]),
        as_num(&ops[n - 1]),
    ]
}

#[inline]
pub(super) fn as_num(tok: &StreamToken) -> f32 {
    match tok {
        StreamToken::Number(v) if v.is_finite() => *v,
        _ => 0.0,
    }
}

#[inline]
fn text_render_matrix(text_matrix: [f32; 6], ctm: [f32; 6], text_rise: f32) -> [f32; 6] {
    if text_rise.abs() > 1e-4 {
        mul_matrix(
            mul_matrix([1.0, 0.0, 0.0, 1.0, 0.0, text_rise], text_matrix),
            ctm,
        )
    } else {
        mul_matrix(text_matrix, ctm)
    }
}

#[inline]
fn effective_font_size(combined: [f32; 6], font_size: f32) -> f32 {
    let scale_y = combined[2].hypot(combined[3]);
    let scale_x = combined[0].hypot(combined[1]);
    let scale = if scale_y.is_finite() && scale_y > 1e-3 {
        scale_y
    } else if scale_x.is_finite() && scale_x > 1e-3 {
        scale_x
    } else {
        1.0
    };
    (font_size * scale).clamp(0.1, 1000.0)
}

#[allow(clippy::too_many_arguments)]
fn push_span_from_matrices(
    text: String,
    font: &PdfFont,
    gs: &GraphicsState,
    start_combined: [f32; 6],
    end_combined: [f32; 6],
    effective_size: f32,
    coord_map: PageCoordMap,
    options: &ToMarkdownOptions,
    out: &mut Vec<TextSpan>,
) {
    if text.is_empty() {
        return;
    }
    if !effective_size.is_finite() || effective_size < options.fontsize_limit {
        return;
    }
    if options.ignore_alpha && gs.alpha < 0.05 {
        return;
    }

    let rotation_deg = start_combined[1].atan2(start_combined[0]).to_degrees();
    let (x0, baseline_y) = coord_map.map_point(start_combined[4], start_combined[5]);
    let (end_x, end_y) = coord_map.map_point(end_combined[4], end_combined[5]);
    if !x0.is_finite() || !baseline_y.is_finite() || !end_x.is_finite() || !end_y.is_finite() {
        return;
    }
    let bbox = if (end_y - baseline_y).abs() < 1.0 {
        let bx1 = end_x.max(x0).max(x0.min(end_x) + 1.0);
        let bx0 = x0.min(end_x);
        let y0 = baseline_y - effective_size * 0.85;
        let y1 = baseline_y + effective_size * 0.2;
        Rect::new(bx0, y0, bx1, y1)
    } else {
        let pad = effective_size * 0.5;
        Rect::new(
            x0.min(end_x) - pad,
            baseline_y.min(end_y) - pad,
            x0.max(end_x) + pad,
            baseline_y.max(end_y) + pad,
        )
    };

    // PDF rendering mode 2 (Fill + Stroke) is widely used for faux-bold typography
    let is_bold =
        font.is_bold || (gs.render_mode == 2 && gs.line_width > 0.0 && gs.line_width <= 2.5);

    out.push(TextSpan {
        text,
        bbox,
        origin: (x0, baseline_y),
        font_name: font.base_font.clone(),
        font_size: effective_size,
        is_bold,
        is_italic: font.is_italic,
        is_monospace: font.is_monospace,
        is_strikeout: false,
        is_superscript: gs.text_rise > effective_size * 0.25,
        is_subscript: gs.text_rise < -effective_size * 0.15,
        color_rgb: gs.fill_rgb,
        bg_color_rgb: None,
        alpha: gs.alpha,
        render_mode: gs.render_mode,
        rotation_deg,
        link_uri: None,
    });
}

pub(super) fn emit_text_bytes(
    bytes: &[u8],
    gs: &GraphicsState,
    text_matrix: &mut [f32; 6],
    fonts: &HashMap<String, PdfFont>,
    coord_map: PageCoordMap,
    options: &ToMarkdownOptions,
    out: &mut Vec<TextSpan>,
) {
    let font = fonts.get(&gs.font_name).cloned().unwrap_or_default();
    let (decoded, glyph_units, char_count, space_count) = font.decode_bytes_with_stats(bytes);

    let start_combined = text_render_matrix(*text_matrix, gs.ctm, gs.text_rise);
    let effective_size = effective_font_size(start_combined, gs.font_size);

    let advance = ((glyph_units / 1000.0) * gs.font_size
        + (char_count as f32 * gs.char_spacing)
        + (space_count as f32 * gs.word_spacing))
        * (gs.horiz_scale / 100.0);

    text_matrix[4] += advance * text_matrix[0];
    text_matrix[5] += advance * text_matrix[1];

    let end_combined = text_render_matrix(*text_matrix, gs.ctm, gs.text_rise);
    push_span_from_matrices(
        decoded,
        &font,
        gs,
        start_combined,
        end_combined,
        effective_size,
        coord_map,
        options,
        out,
    );
}

/// Coalesces kerning-split fragments inside a single `TJ` array into contiguous spans while
/// flushing on genuine column/tab jumps so table alignment stays crisp.
pub(super) fn emit_tj_array(
    arr: &[StreamToken],
    gs: &GraphicsState,
    text_matrix: &mut [f32; 6],
    fonts: &HashMap<String, PdfFont>,
    coord_map: PageCoordMap,
    options: &ToMarkdownOptions,
    out: &mut Vec<TextSpan>,
) {
    let font = fonts.get(&gs.font_name).cloned().unwrap_or_default();
    let mut run_text = String::new();
    let mut run_start_combined: Option<[f32; 6]> = None;
    let mut run_end_combined: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut run_effective_size = gs.font_size.max(1.0);
    let mut pending_gap_em = 0.0f32;

    let flush_run = |text: &mut String,
                     start: &mut Option<[f32; 6]>,
                     end: [f32; 6],
                     eff_size: f32,
                     gap_em: &mut f32,
                     out: &mut Vec<TextSpan>| {
        if let Some(sc) = start.take() {
            if !text.is_empty() {
                let s = std::mem::take(text);
                push_span_from_matrices(s, &font, gs, sc, end, eff_size, coord_map, options, out);
            }
        }
        text.clear();
        *gap_em = 0.0;
    };

    for item in arr {
        match item {
            StreamToken::String(bytes) => {
                let (decoded, glyph_units, char_count, space_count) =
                    font.decode_bytes_with_stats(bytes);
                let start_combined = text_render_matrix(*text_matrix, gs.ctm, gs.text_rise);
                let effective_size = effective_font_size(start_combined, gs.font_size);

                let advance = ((glyph_units / 1000.0) * gs.font_size
                    + (char_count as f32 * gs.char_spacing)
                    + (space_count as f32 * gs.word_spacing))
                    * (gs.horiz_scale / 100.0);

                text_matrix[4] += advance * text_matrix[0];
                text_matrix[5] += advance * text_matrix[1];

                let end_combined = text_render_matrix(*text_matrix, gs.ctm, gs.text_rise);
                if !decoded.is_empty() {
                    if run_start_combined.is_none() {
                        run_start_combined = Some(start_combined);
                        run_effective_size = effective_size;
                    } else if pending_gap_em > 0.16
                        && should_insert_tj_space(&run_text, &decoded, pending_gap_em)
                    {
                        run_text.push(' ');
                    }
                    pending_gap_em = 0.0;
                    run_text.push_str(&decoded);
                    run_end_combined = end_combined;
                }
            }
            StreamToken::Number(adj) => {
                let dx = (-adj / 1000.0) * gs.font_size * (gs.horiz_scale / 100.0);
                text_matrix[4] += dx * text_matrix[0];
                text_matrix[5] += dx * text_matrix[1];

                if run_start_combined.is_some() {
                    let abs_font = gs.font_size.max(1.0);
                    let gap_em = dx / abs_font;
                    if gap_em > 0.65 || gap_em < -0.35 {
                        // Significant tab/column jump or carriage rewind: split into distinct spans
                        flush_run(
                            &mut run_text,
                            &mut run_start_combined,
                            run_end_combined,
                            run_effective_size,
                            &mut pending_gap_em,
                            out,
                        );
                    } else if gap_em > 0.0 {
                        pending_gap_em += gap_em;
                    }
                }
            }
            _ => {}
        }
    }

    flush_run(
        &mut run_text,
        &mut run_start_combined,
        run_end_combined,
        run_effective_size,
        &mut pending_gap_em,
        out,
    );
}

fn should_insert_tj_space(prev: &str, next: &str, gap_em: f32) -> bool {
    let (Some(last_c), Some(first_c)) = (prev.chars().last(), next.chars().next()) else {
        return false;
    };
    if last_c.is_whitespace() || first_c.is_whitespace() {
        return false;
    }
    if is_cjk_char(last_c) && is_cjk_char(first_c) {
        return gap_em > 0.55;
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
        return gap_em > 0.45;
    }
    if matches!(last_c, '(' | '[' | '{' | '（' | '【' | '《') {
        return gap_em > 0.45;
    }
    true
}
