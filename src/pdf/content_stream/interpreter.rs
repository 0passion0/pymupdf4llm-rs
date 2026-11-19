use std::collections::{BTreeMap, HashMap, HashSet};

use super::resources::{load_ext_gstate_props, lookup_property_actual_text};
use super::text_emitter::{
    as_num, cmyk_to_rgb, emit_text_bytes, emit_tj_array, mul_matrix, pop_matrix, transform_point,
    PageCoordMap,
};
use super::tokenizer::{tokenize_content_stream, StreamToken};
use super::xobject::handle_xobject;
use super::GraphicsState;
use crate::config::{Rect, ToMarkdownOptions};
use crate::pdf::font::PdfFont;
use crate::pdf::objects::PdfObject;
use crate::pdf::{ParsedPage, PdfDocument, VectorRect, VectorSegment};

const MAX_GS_STACK: usize = 256;
const MAX_MC_STACK: usize = 256;

#[derive(Debug, Clone)]
struct MarkedContentFrame {
    actual_text: Option<String>,
    span_start_idx: usize,
}

fn color_from_sc_operands(operands: &[StreamToken]) -> Option<(f32, f32, f32)> {
    let nums: Vec<f32> = operands
        .iter()
        .filter_map(|t| match t {
            StreamToken::Number(v) if v.is_finite() => Some(*v),
            _ => None,
        })
        .collect();
    match nums.len() {
        1 => {
            let v = nums[0].clamp(0.0, 1.0);
            Some((v, v, v))
        }
        3 => Some((
            nums[0].clamp(0.0, 1.0),
            nums[1].clamp(0.0, 1.0),
            nums[2].clamp(0.0, 1.0),
        )),
        4 => Some(cmyk_to_rgb(nums[0], nums[1], nums[2], nums[3])),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn interpret_content_stream(
    doc: &PdfDocument,
    stream: &[u8],
    resources: &BTreeMap<String, PdfObject>,
    base_fonts: &HashMap<String, PdfFont>,
    options: &ToMarkdownOptions,
    initial_gs: &GraphicsState,
    coord_map: PageCoordMap,
    depth: usize,
    active_xobjects: &mut HashSet<u32>,
    page: &mut ParsedPage,
) {
    if depth > 8 {
        return;
    }
    let tokens = tokenize_content_stream(stream);
    let ext_gstates = load_ext_gstate_props(doc, resources);
    let mut local_fonts: Option<HashMap<String, PdfFont>> = None;
    let mut stack: Vec<GraphicsState> = Vec::new();
    let mut mc_stack: Vec<MarkedContentFrame> = Vec::new();
    let mut gs = initial_gs.clone();
    let mut text_matrix = [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut line_matrix = [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut operands: Vec<StreamToken> = Vec::new();

    let mut path_segments: Vec<((f32, f32), (f32, f32))> = Vec::new();
    let mut path_rects: Vec<Rect> = Vec::new();
    let mut subpath_start: Option<(f32, f32)> = None;
    let mut cur_pt: Option<(f32, f32)> = None;

    for tok in tokens {
        match tok {
            StreamToken::Operator(op) => {
                let fonts_ref = local_fonts.as_ref().unwrap_or(base_fonts);
                match op.as_str() {
                    "q" => {
                        if stack.len() < MAX_GS_STACK {
                            stack.push(gs.clone());
                        }
                    }
                    "Q" => {
                        if let Some(prev) = stack.pop() {
                            gs = prev;
                        }
                    }
                    "BMC" => {
                        if mc_stack.len() < MAX_MC_STACK {
                            mc_stack.push(MarkedContentFrame {
                                actual_text: None,
                                span_start_idx: page.spans.len(),
                            });
                        }
                    }
                    "BDC" => {
                        if mc_stack.len() < MAX_MC_STACK {
                            let actual_text = match operands.last() {
                                Some(StreamToken::Dict { actual_text }) => actual_text.clone(),
                                Some(StreamToken::Name(prop_name)) if operands.len() >= 2 => {
                                    lookup_property_actual_text(doc, resources, prop_name)
                                }
                                _ => None,
                            };
                            mc_stack.push(MarkedContentFrame {
                                actual_text,
                                span_start_idx: page.spans.len(),
                            });
                        }
                    }
                    "EMC" => {
                        if let Some(frame) = mc_stack.pop() {
                            if let Some(actual) = frame.actual_text {
                                if frame.span_start_idx < page.spans.len() {
                                    let mut merged = page.spans[frame.span_start_idx].clone();
                                    for s in &page.spans[frame.span_start_idx + 1..] {
                                        merged.bbox = merged.bbox.union(&s.bbox);
                                    }
                                    merged.text = actual;
                                    page.spans.truncate(frame.span_start_idx);
                                    page.spans.push(merged);
                                }
                            }
                        }
                    }
                    "cm" if operands.len() >= 6 => {
                        let m = pop_matrix(&operands);
                        gs.ctm = mul_matrix(m, gs.ctm);
                    }
                    "w" if !operands.is_empty() => {
                        let n = operands.len();
                        gs.line_width = as_num(&operands[n - 1]).abs();
                    }
                    "gs" if !operands.is_empty() => {
                        let n = operands.len();
                        if let StreamToken::Name(name) = &operands[n - 1] {
                            if let Some(props) = ext_gstates.get(name) {
                                if let Some(a) = props.alpha {
                                    gs.alpha = a;
                                }
                                if let Some((syn_name, font, sz)) = &props.font_override {
                                    let map = local_fonts.get_or_insert_with(|| base_fonts.clone());
                                    map.insert(syn_name.clone(), font.clone());
                                    gs.font_name = syn_name.clone();
                                    gs.font_size = *sz;
                                }
                            }
                        }
                    }
                    "rg" if operands.len() >= 3 => {
                        let n = operands.len();
                        gs.fill_rgb = (
                            as_num(&operands[n - 3]).clamp(0.0, 1.0),
                            as_num(&operands[n - 2]).clamp(0.0, 1.0),
                            as_num(&operands[n - 1]).clamp(0.0, 1.0),
                        );
                    }
                    "RG" if operands.len() >= 3 => {
                        let n = operands.len();
                        gs.stroke_rgb = (
                            as_num(&operands[n - 3]).clamp(0.0, 1.0),
                            as_num(&operands[n - 2]).clamp(0.0, 1.0),
                            as_num(&operands[n - 1]).clamp(0.0, 1.0),
                        );
                    }
                    "k" if operands.len() >= 4 => {
                        let n = operands.len();
                        gs.fill_rgb = cmyk_to_rgb(
                            as_num(&operands[n - 4]),
                            as_num(&operands[n - 3]),
                            as_num(&operands[n - 2]),
                            as_num(&operands[n - 1]),
                        );
                    }
                    "K" if operands.len() >= 4 => {
                        let n = operands.len();
                        gs.stroke_rgb = cmyk_to_rgb(
                            as_num(&operands[n - 4]),
                            as_num(&operands[n - 3]),
                            as_num(&operands[n - 2]),
                            as_num(&operands[n - 1]),
                        );
                    }
                    "g" if !operands.is_empty() => {
                        let n = operands.len();
                        let v = as_num(&operands[n - 1]).clamp(0.0, 1.0);
                        gs.fill_rgb = (v, v, v);
                    }
                    "G" if !operands.is_empty() => {
                        let n = operands.len();
                        let v = as_num(&operands[n - 1]).clamp(0.0, 1.0);
                        gs.stroke_rgb = (v, v, v);
                    }
                    "sc" | "scn" => {
                        if let Some(rgb) = color_from_sc_operands(&operands) {
                            gs.fill_rgb = rgb;
                        }
                    }
                    "SC" | "SCN" => {
                        if let Some(rgb) = color_from_sc_operands(&operands) {
                            gs.stroke_rgb = rgb;
                        }
                    }
                    "BT" => {
                        text_matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                        line_matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                    }
                    "ET" => {}
                    "Tf" if operands.len() >= 2 => {
                        let n = operands.len();
                        if let StreamToken::Name(name) = &operands[n - 2] {
                            gs.font_name = name.clone();
                        }
                        let sz = as_num(&operands[n - 1]).abs();
                        gs.font_size = if sz > 1e-4 { sz } else { 1.0 };
                    }
                    "Tr" if !operands.is_empty() => {
                        let n = operands.len();
                        gs.render_mode =
                            (as_num(&operands[n - 1]).round() as i32).clamp(0, 7) as u8;
                    }
                    "Tc" if !operands.is_empty() => {
                        let n = operands.len();
                        gs.char_spacing = as_num(&operands[n - 1]);
                    }
                    "Tw" if !operands.is_empty() => {
                        let n = operands.len();
                        gs.word_spacing = as_num(&operands[n - 1]);
                    }
                    "Tz" if !operands.is_empty() => {
                        let n = operands.len();
                        gs.horiz_scale = as_num(&operands[n - 1]);
                    }
                    "TL" if !operands.is_empty() => {
                        let n = operands.len();
                        gs.leading = as_num(&operands[n - 1]);
                    }
                    "Ts" if !operands.is_empty() => {
                        let n = operands.len();
                        gs.text_rise = as_num(&operands[n - 1]);
                    }
                    "Tm" if operands.len() >= 2 => {
                        let m = if operands.len() >= 6 {
                            pop_matrix(&operands)
                        } else {
                            let n = operands.len();
                            [
                                1.0,
                                0.0,
                                0.0,
                                1.0,
                                as_num(&operands[n - 2]),
                                as_num(&operands[n - 1]),
                            ]
                        };
                        text_matrix = m;
                        line_matrix = m;
                    }
                    "Td" if operands.len() >= 2 => {
                        let n = operands.len();
                        let tx = as_num(&operands[n - 2]);
                        let ty = as_num(&operands[n - 1]);
                        line_matrix = mul_matrix([1.0, 0.0, 0.0, 1.0, tx, ty], line_matrix);
                        text_matrix = line_matrix;
                    }
                    "TD" if operands.len() >= 2 => {
                        let n = operands.len();
                        let tx = as_num(&operands[n - 2]);
                        let ty = as_num(&operands[n - 1]);
                        gs.leading = -ty;
                        line_matrix = mul_matrix([1.0, 0.0, 0.0, 1.0, tx, ty], line_matrix);
                        text_matrix = line_matrix;
                    }
                    "T*" => {
                        line_matrix =
                            mul_matrix([1.0, 0.0, 0.0, 1.0, 0.0, -gs.leading], line_matrix);
                        text_matrix = line_matrix;
                    }
                    "Tj" | "'" | "\"" => {
                        if op == "\"" && operands.len() >= 3 {
                            let n = operands.len();
                            gs.word_spacing = as_num(&operands[n - 3]);
                            gs.char_spacing = as_num(&operands[n - 2]);
                        }
                        if op == "'" || op == "\"" {
                            line_matrix =
                                mul_matrix([1.0, 0.0, 0.0, 1.0, 0.0, -gs.leading], line_matrix);
                            text_matrix = line_matrix;
                        }
                        if let Some(StreamToken::String(bytes)) = operands.last() {
                            emit_text_bytes(
                                bytes,
                                &gs,
                                &mut text_matrix,
                                fonts_ref,
                                coord_map,
                                options,
                                &mut page.spans,
                            );
                        }
                    }
                    "TJ" => {
                        if let Some(StreamToken::Array(arr)) = operands.last() {
                            emit_tj_array(
                                arr,
                                &gs,
                                &mut text_matrix,
                                fonts_ref,
                                coord_map,
                                options,
                                &mut page.spans,
                            );
                        }
                    }
                    "m" if operands.len() >= 2 => {
                        let n = operands.len();
                        let (x, y) = transform_point(
                            gs.ctm,
                            as_num(&operands[n - 2]),
                            as_num(&operands[n - 1]),
                        );
                        let pt = coord_map.map_point(x, y);
                        cur_pt = Some(pt);
                        subpath_start = Some(pt);
                    }
                    "l" if operands.len() >= 2 => {
                        let n = operands.len();
                        let (x, y) = transform_point(
                            gs.ctm,
                            as_num(&operands[n - 2]),
                            as_num(&operands[n - 1]),
                        );
                        let pt = coord_map.map_point(x, y);
                        if let Some(prev) = cur_pt {
                            path_segments.push((prev, pt));
                        }
                        cur_pt = Some(pt);
                        if subpath_start.is_none() {
                            subpath_start = Some(pt);
                        }
                    }
                    "c" | "v" | "y" if operands.len() >= 4 => {
                        let n = operands.len();
                        let (x, y) = transform_point(
                            gs.ctm,
                            as_num(&operands[n - 2]),
                            as_num(&operands[n - 1]),
                        );
                        cur_pt = Some(coord_map.map_point(x, y));
                    }
                    "h" => {
                        if let (Some(prev), Some(start)) = (cur_pt, subpath_start) {
                            if prev != start {
                                path_segments.push((prev, start));
                            }
                            cur_pt = Some(start);
                        }
                    }
                    "re" if operands.len() >= 4 => {
                        let n = operands.len();
                        let rx = as_num(&operands[n - 4]);
                        let ry = as_num(&operands[n - 3]);
                        let rw = as_num(&operands[n - 2]);
                        let rh = as_num(&operands[n - 1]);
                        let (p0x, p0y) = transform_point(gs.ctm, rx, ry);
                        let (p1x, p1y) = transform_point(gs.ctm, rx + rw, ry + rh);
                        let (ax, ay) = coord_map.map_point(p0x.min(p1x), p0y.max(p1y));
                        let (bx, by) = coord_map.map_point(p0x.max(p1x), p0y.min(p1y));
                        path_rects.push(Rect::new(ax, ay, bx, by));
                    }
                    "S" | "s" | "f" | "F" | "f*" | "B" | "b" | "B*" | "b*" => {
                        let is_stroked = matches!(op.as_str(), "S" | "s" | "B" | "b" | "B*" | "b*");
                        let is_filled =
                            matches!(op.as_str(), "f" | "F" | "f*" | "B" | "b" | "B*" | "b*");
                        if matches!(op.as_str(), "s" | "b" | "b*") {
                            if let (Some(prev), Some(start)) = (cur_pt, subpath_start) {
                                if prev != start {
                                    path_segments.push((prev, start));
                                }
                            }
                        }
                        for (p0, p1) in path_segments.drain(..) {
                            page.segments.push(VectorSegment {
                                p0,
                                p1,
                                stroke_width: gs.line_width,
                                is_stroked,
                                is_filled,
                                color_rgb: if is_stroked {
                                    gs.stroke_rgb
                                } else {
                                    gs.fill_rgb
                                },
                            });
                        }
                        for r in path_rects.drain(..) {
                            if is_stroked {
                                page.segments.push(VectorSegment {
                                    p0: (r.x0, r.y0),
                                    p1: (r.x1, r.y0),
                                    stroke_width: gs.line_width,
                                    is_stroked: true,
                                    is_filled,
                                    color_rgb: gs.stroke_rgb,
                                });
                                page.segments.push(VectorSegment {
                                    p0: (r.x0, r.y1),
                                    p1: (r.x1, r.y1),
                                    stroke_width: gs.line_width,
                                    is_stroked: true,
                                    is_filled,
                                    color_rgb: gs.stroke_rgb,
                                });
                                page.segments.push(VectorSegment {
                                    p0: (r.x0, r.y0),
                                    p1: (r.x0, r.y1),
                                    stroke_width: gs.line_width,
                                    is_stroked: true,
                                    is_filled,
                                    color_rgb: gs.stroke_rgb,
                                });
                                page.segments.push(VectorSegment {
                                    p0: (r.x1, r.y0),
                                    p1: (r.x1, r.y1),
                                    stroke_width: gs.line_width,
                                    is_stroked: true,
                                    is_filled,
                                    color_rgb: gs.stroke_rgb,
                                });
                            }
                            page.rects.push(VectorRect {
                                rect: r,
                                is_stroked,
                                is_filled,
                                stroke_width: gs.line_width,
                                fill_rgb: if is_filled { Some(gs.fill_rgb) } else { None },
                            });
                        }
                        cur_pt = None;
                        subpath_start = None;
                    }
                    "n" => {
                        path_segments.clear();
                        path_rects.clear();
                        cur_pt = None;
                        subpath_start = None;
                    }
                    "Do" if !operands.is_empty() => {
                        let n = operands.len();
                        if let StreamToken::Name(xname) = &operands[n - 1] {
                            handle_xobject(
                                doc,
                                resources,
                                fonts_ref,
                                xname,
                                &gs,
                                coord_map,
                                options,
                                depth,
                                active_xobjects,
                                page,
                            );
                        }
                    }
                    _ => {}
                }
                operands.clear();
            }
            other => operands.push(other),
        }
    }
}
