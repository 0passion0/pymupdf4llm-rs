use std::collections::{BTreeMap, HashMap};

use crate::config::{Rect, ToMarkdownOptions};
use crate::error::{PdfError, Result};
use crate::layout::spatial::SpatialGrid;
use crate::pdf::font::PdfFont;
use crate::pdf::objects::PdfObject;
use crate::pdf::parser::decode_stream_bytes;
use crate::pdf::{
    LinkAnnotation, ParsedPage, PdfDocument, RawImagePlacement, TextSpan, VectorRect,
    VectorSegment,
};

#[derive(Debug, Clone)]
enum StreamToken {
    Number(f32),
    Name(String),
    String(Vec<u8>),
    Array(Vec<StreamToken>),
    Operator(String),
}

#[derive(Debug, Clone)]
struct GraphicsState {
    ctm: [f32; 6],
    font_name: String,
    font_size: f32,
    char_spacing: f32,
    word_spacing: f32,
    horiz_scale: f32,
    leading: f32,
    text_rise: f32,
    line_width: f32,
    fill_rgb: (f32, f32, f32),
    stroke_rgb: (f32, f32, f32),
    alpha: f32,
    render_mode: u8,
}

impl Default for GraphicsState {
    fn default() -> Self {
        Self {
            ctm: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            font_name: String::new(),
            font_size: 12.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            horiz_scale: 100.0,
            leading: 0.0,
            text_rise: 0.0,
            line_width: 1.0,
            fill_rgb: (0.0, 0.0, 0.0),
            stroke_rgb: (0.0, 0.0, 0.0),
            alpha: 1.0,
            render_mode: 0,
        }
    }
}

pub fn extract_page(
    doc: &PdfDocument,
    page_idx: usize,
    options: &ToMarkdownOptions,
) -> Result<ParsedPage> {
    let page_ref = *doc
        .page_refs
        .get(page_idx)
        .ok_or(PdfError::PageOutOfBounds {
            index: page_idx,
            total: doc.page_count(),
        })?;

    let page_dict = doc
        .objects
        .get(&page_ref.0)
        .and_then(|o| o.as_dict())
        .ok_or(PdfError::MissingObject {
            obj_num: page_ref.0,
            gen_num: page_ref.1,
        })?;

    let inherited = doc.inherited_props.get(&page_ref.0);

    let media_box = parse_rect_from_dict(doc, page_dict, "MediaBox")
        .or_else(|| inherited.and_then(|p| p.media_box))
        .unwrap_or(Rect::new(
            0.0,
            0.0,
            options.page_width,
            options.page_height.unwrap_or(792.0),
        ));
    let raw_crop_box = parse_rect_from_dict(doc, page_dict, "CropBox")
        .or_else(|| inherited.and_then(|p| p.crop_box))
        .unwrap_or(media_box);
    let rotation = page_dict
        .get("Rotate")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)
        .or_else(|| inherited.and_then(|p| p.rotate))
        .unwrap_or(0)
        .rem_euclid(360);

    let crop_box = if rotation == 90 || rotation == 270 {
        Rect::new(0.0, 0.0, raw_crop_box.height(), raw_crop_box.width())
    } else {
        raw_crop_box
    };

    let mut resources = inherited
        .map(|p| p.resources.clone())
        .unwrap_or_default();
    if let Some(page_res) = page_dict
        .get("Resources")
        .and_then(|r| doc.resolve(r))
        .and_then(|r| r.as_dict())
    {
        merge_resource_dicts(&mut resources, page_res);
    }

    let fonts = load_fonts(doc, &resources);
    let links = load_link_annotations(doc, page_dict, raw_crop_box.height());

    let mut content_bytes = Vec::new();
    if let Some(contents) = page_dict.get("Contents") {
        collect_content_bytes(doc, contents, &mut content_bytes)?;
    }

    let mut page = ParsedPage {
        page_index: page_idx,
        media_box,
        crop_box,
        rotation,
        spans: Vec::new(),
        segments: Vec::new(),
        rects: Vec::new(),
        images: Vec::new(),
        links,
    };

    interpret_content_stream(
        doc,
        &content_bytes,
        &resources,
        &fonts,
        options,
        &GraphicsState::default(),
        raw_crop_box.height(),
        0,
        &mut page,
    );

    if rotation != 0 {
        normalize_page_rotation(&mut page, raw_crop_box.width(), raw_crop_box.height(), rotation);
    }

    // Attach link annotations to overlapping spans using O(1) SpatialGrid lookup
    if !page.links.is_empty() && !page.spans.is_empty() {
        let mut link_grid = SpatialGrid::new(48.0);
        for (idx, link) in page.links.iter().enumerate() {
            link_grid.insert(idx, link.rect);
        }
        for span in &mut page.spans {
            if span.link_uri.is_none() {
                for link_idx in link_grid.query_rect(span.bbox) {
                    if let Some(link) = page.links.get(link_idx) {
                        if span.bbox.intersects(&link.rect) {
                            if let Some(uri) = &link.uri {
                                span.link_uri = Some(uri.clone());
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(page)
}

fn merge_resource_dicts(
    base: &mut BTreeMap<String, PdfObject>,
    overlay: &BTreeMap<String, PdfObject>,
) {
    for (k, v) in overlay {
        base.insert(k.clone(), v.clone());
    }
}

fn collect_content_bytes(
    doc: &PdfDocument,
    obj: &PdfObject,
    out: &mut Vec<u8>,
) -> Result<()> {
    match doc.resolve(obj) {
        Some(PdfObject::Stream { dict, data }) => {
            let decoded = decode_stream_bytes(dict, data)?;
            out.extend_from_slice(&decoded);
            out.push(b'\n');
        }
        Some(PdfObject::Array(arr)) => {
            for item in arr {
                collect_content_bytes(doc, item, out)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn load_fonts(doc: &PdfDocument, resources: &BTreeMap<String, PdfObject>) -> HashMap<String, PdfFont> {
    let mut map = HashMap::new();
    let Some(font_dict) = resources
        .get("Font")
        .and_then(|o| doc.resolve(o))
        .and_then(|o| o.as_dict())
    else {
        return map;
    };
    for (name, font_obj) in font_dict {
        map.insert(name.clone(), PdfFont::from_dict(doc, font_obj));
    }
    map
}

fn load_ext_gstate_alphas(
    doc: &PdfDocument,
    resources: &BTreeMap<String, PdfObject>,
) -> HashMap<String, f32> {
    let mut map = HashMap::new();
    let Some(gs_dict) = resources
        .get("ExtGState")
        .and_then(|o| doc.resolve(o))
        .and_then(|o| o.as_dict())
    else {
        return map;
    };
    for (name, obj) in gs_dict {
        if let Some(d) = doc.resolve(obj).and_then(|o| o.as_dict()) {
            if let Some(alpha) = d
                .get("ca")
                .or_else(|| d.get("CA"))
                .and_then(|v| v.as_f32())
            {
                map.insert(name.clone(), alpha.clamp(0.0, 1.0));
            }
        }
    }
    map
}

fn load_link_annotations(
    doc: &PdfDocument,
    page_dict: &BTreeMap<String, PdfObject>,
    page_height: f32,
) -> Vec<LinkAnnotation> {
    let mut links = Vec::new();
    let Some(annots) = page_dict
        .get("Annots")
        .and_then(|o| doc.resolve(o))
        .and_then(|o| o.as_array())
    else {
        return links;
    };
    for annot_obj in annots {
        let Some(dict) = doc.resolve(annot_obj).and_then(|o| o.as_dict()) else {
            continue;
        };
        if dict.get("Subtype").and_then(|v| v.as_name()) != Some("Link") {
            continue;
        }
        let Some(raw_rect) = dict.get("Rect").and_then(|v| v.as_array()) else {
            continue;
        };
        if raw_rect.len() != 4 {
            continue;
        }
        let x0 = raw_rect[0].as_f32().unwrap_or(0.0);
        let y0_pdf = raw_rect[1].as_f32().unwrap_or(0.0);
        let x1 = raw_rect[2].as_f32().unwrap_or(0.0);
        let y1_pdf = raw_rect[3].as_f32().unwrap_or(0.0);
        let rect = Rect::new(
            x0.min(x1),
            page_height - y1_pdf.max(y0_pdf),
            x0.max(x1),
            page_height - y0_pdf.min(y1_pdf),
        );
        let uri = dict
            .get("A")
            .and_then(|a| doc.resolve(a))
            .and_then(|a| a.as_dict())
            .and_then(|ad| ad.get("URI"))
            .and_then(|u| u.as_text_string());
        links.push(LinkAnnotation {
            rect,
            uri,
            dest_page: None,
        });
    }
    links
}

#[allow(clippy::too_many_arguments)]
fn interpret_content_stream(
    doc: &PdfDocument,
    stream: &[u8],
    resources: &BTreeMap<String, PdfObject>,
    fonts: &HashMap<String, PdfFont>,
    options: &ToMarkdownOptions,
    initial_gs: &GraphicsState,
    page_height: f32,
    depth: usize,
    page: &mut ParsedPage,
) {
    if depth > 8 {
        return;
    }
    let tokens = tokenize_content_stream(stream);
    let ext_alphas = load_ext_gstate_alphas(doc, resources);
    let mut stack: Vec<GraphicsState> = Vec::new();
    let mut gs = initial_gs.clone();
    let mut text_matrix = [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut line_matrix = [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut operands: Vec<StreamToken> = Vec::new();

    let mut path_points: Vec<(f32, f32)> = Vec::new();
    let mut path_rects: Vec<Rect> = Vec::new();

    for tok in tokens {
        match tok {
            StreamToken::Operator(op) => {
                match op.as_str() {
                    "q" => stack.push(gs.clone()),
                    "Q" => {
                        if let Some(prev) = stack.pop() {
                            gs = prev;
                        }
                    }
                    "cm" if operands.len() >= 6 => {
                        let m = pop_matrix(&operands);
                        gs.ctm = mul_matrix(m, gs.ctm);
                    }
                    "w" if !operands.is_empty() => {
                        gs.line_width = as_num(&operands[0]);
                    }
                    "gs" if !operands.is_empty() => {
                        if let StreamToken::Name(name) = &operands[0] {
                            if let Some(&a) = ext_alphas.get(name) {
                                gs.alpha = a;
                            }
                        }
                    }
                    "rg" if operands.len() >= 3 => {
                        gs.fill_rgb = (as_num(&operands[0]), as_num(&operands[1]), as_num(&operands[2]));
                    }
                    "RG" if operands.len() >= 3 => {
                        gs.stroke_rgb = (as_num(&operands[0]), as_num(&operands[1]), as_num(&operands[2]));
                    }
                    "g" if !operands.is_empty() => {
                        let v = as_num(&operands[0]);
                        gs.fill_rgb = (v, v, v);
                    }
                    "G" if !operands.is_empty() => {
                        let v = as_num(&operands[0]);
                        gs.stroke_rgb = (v, v, v);
                    }
                    "BT" => {
                        text_matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                        line_matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                    }
                    "ET" => {}
                    "Tf" if operands.len() >= 2 => {
                        if let StreamToken::Name(name) = &operands[0] {
                            gs.font_name = name.clone();
                        }
                        gs.font_size = as_num(&operands[1]).abs().max(1.0);
                    }
                    "Tr" if !operands.is_empty() => {
                        gs.render_mode = (as_num(&operands[0]).round() as i32).clamp(0, 7) as u8;
                    }
                    "Tc" if !operands.is_empty() => gs.char_spacing = as_num(&operands[0]),
                    "Tw" if !operands.is_empty() => gs.word_spacing = as_num(&operands[0]),
                    "Tz" if !operands.is_empty() => gs.horiz_scale = as_num(&operands[0]),
                    "TL" if !operands.is_empty() => gs.leading = as_num(&operands[0]),
                    "Ts" if !operands.is_empty() => gs.text_rise = as_num(&operands[0]),
                    "Tm" if operands.len() >= 6 => {
                        let m = pop_matrix(&operands);
                        text_matrix = m;
                        line_matrix = m;
                    }
                    "Td" if operands.len() >= 2 => {
                        let tx = as_num(&operands[0]);
                        let ty = as_num(&operands[1]);
                        line_matrix = mul_matrix([1.0, 0.0, 0.0, 1.0, tx, ty], line_matrix);
                        text_matrix = line_matrix;
                    }
                    "TD" if operands.len() >= 2 => {
                        let tx = as_num(&operands[0]);
                        let ty = as_num(&operands[1]);
                        gs.leading = -ty;
                        line_matrix = mul_matrix([1.0, 0.0, 0.0, 1.0, tx, ty], line_matrix);
                        text_matrix = line_matrix;
                    }
                    "T*" => {
                        line_matrix = mul_matrix([1.0, 0.0, 0.0, 1.0, 0.0, -gs.leading], line_matrix);
                        text_matrix = line_matrix;
                    }
                    "Tj" | "'" | "\"" => {
                        if op == "'" || op == "\"" {
                            line_matrix = mul_matrix([1.0, 0.0, 0.0, 1.0, 0.0, -gs.leading], line_matrix);
                            text_matrix = line_matrix;
                        }
                        if let Some(StreamToken::String(bytes)) = operands.last() {
                            emit_text_bytes(
                                bytes,
                                &gs,
                                &mut text_matrix,
                                fonts,
                                page_height,
                                options,
                                &mut page.spans,
                            );
                        }
                    }
                    "TJ" => {
                        if let Some(StreamToken::Array(arr)) = operands.last() {
                            for item in arr {
                                match item {
                                    StreamToken::String(bytes) => {
                                        emit_text_bytes(
                                            bytes,
                                            &gs,
                                            &mut text_matrix,
                                            fonts,
                                            page_height,
                                            options,
                                            &mut page.spans,
                                        );
                                    }
                                    StreamToken::Number(adj) => {
                                        let dx = (-adj / 1000.0) * gs.font_size * (gs.horiz_scale / 100.0);
                                        text_matrix[4] += dx;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    "m" | "l" if operands.len() >= 2 => {
                        let (x, y) = transform_point(gs.ctm, as_num(&operands[0]), as_num(&operands[1]));
                        path_points.push((x, page_height - y));
                    }
                    "re" if operands.len() >= 4 => {
                        let rx = as_num(&operands[0]);
                        let ry = as_num(&operands[1]);
                        let rw = as_num(&operands[2]);
                        let rh = as_num(&operands[3]);
                        let (p0x, p0y) = transform_point(gs.ctm, rx, ry);
                        let (p1x, p1y) = transform_point(gs.ctm, rx + rw, ry + rh);
                        let rect = Rect::new(
                            p0x.min(p1x),
                            page_height - p0y.max(p1y),
                            p0x.max(p1x),
                            page_height - p0y.min(p1y),
                        );
                        path_rects.push(rect);
                    }
                    "S" | "s" | "f" | "F" | "f*" | "B" | "b" | "B*" | "b*" => {
                        let is_stroked = matches!(op.as_str(), "S" | "s" | "B" | "b" | "B*" | "b*");
                        let is_filled = matches!(op.as_str(), "f" | "F" | "f*" | "B" | "b" | "B*" | "b*");
                        for pair in path_points.windows(2) {
                            page.segments.push(VectorSegment {
                                p0: pair[0],
                                p1: pair[1],
                                stroke_width: gs.line_width,
                                is_stroked,
                                is_filled,
                                color_rgb: if is_stroked { gs.stroke_rgb } else { gs.fill_rgb },
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
                        path_points.clear();
                    }
                    "n" => {
                        path_points.clear();
                        path_rects.clear();
                    }
                    "Do" if !operands.is_empty() => {
                        if let StreamToken::Name(xname) = &operands[0] {
                            handle_xobject(
                                doc,
                                resources,
                                fonts,
                                xname,
                                &gs,
                                page_height,
                                options,
                                depth,
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

fn emit_text_bytes(
    bytes: &[u8],
    gs: &GraphicsState,
    text_matrix: &mut [f32; 6],
    fonts: &HashMap<String, PdfFont>,
    page_height: f32,
    options: &ToMarkdownOptions,
    out: &mut Vec<TextSpan>,
) {
    let font = fonts.get(&gs.font_name).cloned().unwrap_or_default();
    let (decoded, glyph_units) = font.decode_bytes(bytes);
    if decoded.is_empty() {
        return;
    }

    let combined = mul_matrix(*text_matrix, gs.ctm);
    let scale_x = combined[0].hypot(combined[1]).max(1.0);
    let effective_size = (gs.font_size * scale_x).max(1.0);
    if effective_size < options.fontsize_limit {
        return;
    }

    if options.ignore_alpha && gs.alpha < 0.05 {
        return;
    }

    let rotation_deg = combined[1].atan2(combined[0]).to_degrees();
    let advance = (glyph_units / 1000.0) * gs.font_size * (gs.horiz_scale / 100.0)
        + (decoded.chars().count() as f32 * gs.char_spacing);

    let x0 = combined[4];
    let baseline_y = page_height - combined[5];
    let x1 = x0 + advance * (gs.ctm[0].abs().max(1.0));
    let y0 = baseline_y - effective_size * 0.85;
    let y1 = baseline_y + effective_size * 0.2;

    text_matrix[4] += advance;

    out.push(TextSpan {
        text: decoded,
        bbox: Rect::new(x0.min(x1), y0.min(y1), x0.max(x1).max(x0.min(x1) + 1.0), y0.max(y1)),
        origin: (x0, baseline_y),
        font_name: font.base_font,
        font_size: effective_size,
        is_bold: font.is_bold,
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

#[allow(clippy::too_many_arguments)]
fn handle_xobject(
    doc: &PdfDocument,
    resources: &BTreeMap<String, PdfObject>,
    parent_fonts: &HashMap<String, PdfFont>,
    xname: &str,
    gs: &GraphicsState,
    page_height: f32,
    options: &ToMarkdownOptions,
    depth: usize,
    page: &mut ParsedPage,
) {
    let Some(xobjs) = resources
        .get("XObject")
        .and_then(|o| doc.resolve(o))
        .and_then(|o| o.as_dict())
    else {
        return;
    };
    let Some(xobj_ref) = xobjs.get(xname) else {
        return;
    };
    let Some(PdfObject::Stream { dict, data }) = doc.resolve(xobj_ref) else {
        return;
    };
    match dict.get("Subtype").and_then(|v| v.as_name()) {
        Some("Image") if !options.ignore_images => {
            let (x0, y0) = transform_point(gs.ctm, 0.0, 0.0);
            let (x1, y1) = transform_point(gs.ctm, 1.0, 1.0);
            let bbox = Rect::new(
                x0.min(x1),
                page_height - y0.max(y1),
                x0.max(x1),
                page_height - y0.min(y1),
            );
            let width = dict.get("Width").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
            let height = dict.get("Height").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
            let bpc = dict.get("BitsPerComponent").and_then(|v| v.as_i64()).unwrap_or(8) as u8;
            let color_space = dict
                .get("ColorSpace")
                .and_then(|v| v.as_name())
                .unwrap_or("DeviceRGB")
                .to_string();
            let filter = dict
                .get("Filter")
                .and_then(|v| v.as_name())
                .map(ToString::to_string);
            page.images.push(RawImagePlacement {
                bbox,
                width,
                height,
                bits_per_component: bpc,
                color_space,
                filter,
                data: data.clone(),
            });
        }
        Some("Form") => {
            let Ok(decoded) = decode_stream_bytes(dict, data) else {
                return;
            };
            let mut form_gs = gs.clone();
            if let Some(arr) = dict.get("Matrix").and_then(|o| doc.resolve(o)).and_then(|o| o.as_array()) {
                if arr.len() == 6 {
                    let fm = [
                        arr[0].as_f32().unwrap_or(1.0),
                        arr[1].as_f32().unwrap_or(0.0),
                        arr[2].as_f32().unwrap_or(0.0),
                        arr[3].as_f32().unwrap_or(1.0),
                        arr[4].as_f32().unwrap_or(0.0),
                        arr[5].as_f32().unwrap_or(0.0),
                    ];
                    form_gs.ctm = mul_matrix(fm, gs.ctm);
                }
            }

            let mut form_resources = resources.clone();
            if let Some(fres) = dict
                .get("Resources")
                .and_then(|o| doc.resolve(o))
                .and_then(|o| o.as_dict())
            {
                merge_resource_dicts(&mut form_resources, fres);
            }

            let mut form_fonts = parent_fonts.clone();
            for (k, v) in load_fonts(doc, &form_resources) {
                form_fonts.insert(k, v);
            }

            interpret_content_stream(
                doc,
                &decoded,
                &form_resources,
                &form_fonts,
                options,
                &form_gs,
                page_height,
                depth + 1,
                page,
            );
        }
        _ => {}
    }
}

fn rotate_point(x: f32, y: f32, w: f32, h: f32, rotation: i32) -> (f32, f32) {
    match rotation {
        90 => (h - y, x),
        180 => (w - x, h - y),
        270 => (y, w - x),
        _ => (x, y),
    }
}

fn rotate_rect(r: Rect, w: f32, h: f32, rotation: i32) -> Rect {
    let (ax, ay) = rotate_point(r.x0, r.y0, w, h, rotation);
    let (bx, by) = rotate_point(r.x1, r.y1, w, h, rotation);
    Rect::new(ax.min(bx), ay.min(by), ax.max(bx), ay.max(by))
}

fn normalize_page_rotation(page: &mut ParsedPage, orig_w: f32, orig_h: f32, rotation: i32) {
    for span in &mut page.spans {
        span.bbox = rotate_rect(span.bbox, orig_w, orig_h, rotation);
        span.origin = rotate_point(span.origin.0, span.origin.1, orig_w, orig_h, rotation);
        span.rotation_deg = (span.rotation_deg - rotation as f32).rem_euclid(360.0);
        if span.rotation_deg > 180.0 {
            span.rotation_deg -= 360.0;
        }
    }
    for seg in &mut page.segments {
        seg.p0 = rotate_point(seg.p0.0, seg.p0.1, orig_w, orig_h, rotation);
        seg.p1 = rotate_point(seg.p1.0, seg.p1.1, orig_w, orig_h, rotation);
    }
    for vr in &mut page.rects {
        vr.rect = rotate_rect(vr.rect, orig_w, orig_h, rotation);
    }
    for img in &mut page.images {
        img.bbox = rotate_rect(img.bbox, orig_w, orig_h, rotation);
    }
    for link in &mut page.links {
        link.rect = rotate_rect(link.rect, orig_w, orig_h, rotation);
    }
}

fn parse_rect_from_dict(doc: &PdfDocument, dict: &BTreeMap<String, PdfObject>, key: &str) -> Option<Rect> {
    let arr = dict.get(key).and_then(|o| doc.resolve(o)).and_then(|o| o.as_array())?;
    if arr.len() != 4 {
        return None;
    }
    Some(Rect::new(
        arr[0].as_f32()?,
        arr[1].as_f32()?,
        arr[2].as_f32()?,
        arr[3].as_f32()?,
    ))
}

#[inline]
fn transform_point(m: [f32; 6], x: f32, y: f32) -> (f32, f32) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

#[inline]
fn mul_matrix(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
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
fn pop_matrix(ops: &[StreamToken]) -> [f32; 6] {
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
fn as_num(tok: &StreamToken) -> f32 {
    match tok {
        StreamToken::Number(v) => *v,
        _ => 0.0,
    }
}

fn tokenize_content_stream(data: &[u8]) -> Vec<StreamToken> {
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'%' {
            while i < data.len() && data[i] != b'\n' && data[i] != b'\r' {
                i += 1;
            }
            continue;
        }
        match b {
            b'(' => {
                i += 1;
                let mut s = Vec::new();
                let mut depth = 1usize;
                while i < data.len() && depth > 0 {
                    let c = data[i];
                    i += 1;
                    match c {
                        b'(' => {
                            depth += 1;
                            s.push(b'(');
                        }
                        b')' => {
                            depth -= 1;
                            if depth > 0 {
                                s.push(b')');
                            }
                        }
                        b'\\' if i < data.len() => {
                            let esc = data[i];
                            i += 1;
                            match esc {
                                b'n' => s.push(b'\n'),
                                b'r' => s.push(b'\r'),
                                b't' => s.push(b'\t'),
                                other => s.push(other),
                            }
                        }
                        other => s.push(other),
                    }
                }
                tokens.push(StreamToken::String(s));
            }
            b'<' if data.get(i + 1) != Some(&b'<') => {
                i += 1;
                let mut nibbles = Vec::new();
                while i < data.len() && data[i] != b'>' {
                    if let Some(v) = match data[i] {
                        b'0'..=b'9' => Some(data[i] - b'0'),
                        b'a'..=b'f' => Some(data[i] - b'a' + 10),
                        b'A'..=b'F' => Some(data[i] - b'A' + 10),
                        _ => None,
                    } {
                        nibbles.push(v);
                    }
                    i += 1;
                }
                if i < data.len() {
                    i += 1;
                }
                let mut out = Vec::new();
                for ch in nibbles.chunks(2) {
                    out.push((ch[0] << 4) | ch.get(1).copied().unwrap_or(0));
                }
                tokens.push(StreamToken::String(out));
            }
            b'[' => {
                i += 1;
                let mut arr = Vec::new();
                while i < data.len() {
                    while i < data.len() && data[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    if i >= data.len() || data[i] == b']' {
                        i += 1;
                        break;
                    }
                    let sub_end = find_array_elem_end(data, i);
                    let sub = tokenize_content_stream(&data[i..sub_end]);
                    arr.extend(sub);
                    i = sub_end;
                }
                tokens.push(StreamToken::Array(arr));
            }
            b'/' => {
                i += 1;
                let start = i;
                while i < data.len() && !data[i].is_ascii_whitespace() && !matches!(data[i], b'/' | b'[' | b']' | b'(' | b'<') {
                    i += 1;
                }
                tokens.push(StreamToken::Name(String::from_utf8_lossy(&data[start..i]).into_owned()));
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                let start = i;
                i += 1;
                while i < data.len() && (data[i].is_ascii_digit() || data[i] == b'.') {
                    i += 1;
                }
                let v = std::str::from_utf8(&data[start..i])
                    .ok()
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(0.0);
                tokens.push(StreamToken::Number(v));
            }
            _ => {
                let start = i;
                while i < data.len() && !data[i].is_ascii_whitespace() && !matches!(data[i], b'(' | b'<' | b'[' | b'/' | b']') {
                    i += 1;
                }
                let op = String::from_utf8_lossy(&data[start..i]).into_owned();
                if !op.is_empty() {
                    tokens.push(StreamToken::Operator(op));
                } else {
                    i += 1;
                }
            }
        }
    }
    tokens
}

fn find_array_elem_end(data: &[u8], start: usize) -> usize {
    let b = data[start];
    if b == b'(' {
        let mut i = start + 1;
        let mut depth = 1usize;
        while i < data.len() && depth > 0 {
            if data[i] == b'\\' {
                i += 2;
                continue;
            }
            if data[i] == b'(' {
                depth += 1;
            } else if data[i] == b')' {
                depth -= 1;
            }
            i += 1;
        }
        i
    } else if b == b'<' {
        data[start..]
            .iter()
            .position(|&c| c == b'>')
            .map(|p| start + p + 1)
            .unwrap_or(data.len())
    } else {
        let mut i = start;
        while i < data.len() && !data[i].is_ascii_whitespace() && data[i] != b']' && data[i] != b'(' && data[i] != b'<' {
            i += 1;
        }
        i.max(start + 1)
    }
}
