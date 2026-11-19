mod interpreter;
mod resources;
mod text_emitter;
mod tokenizer;
mod xobject;

use std::collections::HashSet;

use interpreter::interpret_content_stream;
use resources::{
    collect_content_bytes, load_fonts, load_link_annotations, merge_resource_dicts,
    parse_rect_from_dict,
};
use text_emitter::PageCoordMap;

use crate::config::{Rect, ToMarkdownOptions};
use crate::error::{PdfError, Result};
use crate::layout::spatial::SpatialGrid;
use crate::pdf::{ParsedPage, PdfDocument};

#[derive(Debug, Clone)]
pub(super) struct GraphicsState {
    pub(super) ctm: [f32; 6],
    pub(super) font_name: String,
    pub(super) font_size: f32,
    pub(super) char_spacing: f32,
    pub(super) word_spacing: f32,
    pub(super) horiz_scale: f32,
    pub(super) leading: f32,
    pub(super) text_rise: f32,
    pub(super) line_width: f32,
    pub(super) fill_rgb: (f32, f32, f32),
    pub(super) stroke_rgb: (f32, f32, f32),
    pub(super) alpha: f32,
    pub(super) render_mode: u8,
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
        .and_then(|v| doc.resolve(v))
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)
        .or_else(|| inherited.and_then(|p| p.rotate))
        .unwrap_or(0)
        .rem_euclid(360);

    // Normalize non-zero-origin CropBoxes to `(0, 0, width, height)` in top-left page space
    let coord_map = PageCoordMap::from_crop_box(raw_crop_box);
    let crop_box = if rotation == 90 || rotation == 270 {
        Rect::new(0.0, 0.0, raw_crop_box.height(), raw_crop_box.width())
    } else {
        Rect::new(0.0, 0.0, raw_crop_box.width(), raw_crop_box.height())
    };

    let mut resources = inherited.map(|p| p.resources.clone()).unwrap_or_default();
    if let Some(page_res) = page_dict
        .get("Resources")
        .and_then(|r| doc.resolve(r))
        .and_then(|r| r.as_dict())
    {
        merge_resource_dicts(doc, &mut resources, page_res);
    }

    let fonts = load_fonts(doc, &resources);
    let links = load_link_annotations(doc, page_dict, coord_map);

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

    let mut active_xobjects = HashSet::new();
    interpret_content_stream(
        doc,
        &content_bytes,
        &resources,
        &fonts,
        options,
        &GraphicsState::default(),
        coord_map,
        0,
        &mut active_xobjects,
        &mut page,
    );

    if rotation != 0 {
        normalize_page_rotation(
            &mut page,
            raw_crop_box.width(),
            raw_crop_box.height(),
            rotation,
        );
    }

    attach_link_annotations(&mut page);
    attach_strikethrough_and_bg(&mut page);

    Ok(page)
}

/// Attaches link annotations to overlapping spans via O(1) SpatialGrid lookup.
fn attach_link_annotations(page: &mut ParsedPage) {
    if page.links.is_empty() || page.spans.is_empty() {
        return;
    }
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

/// Detects strikethrough rules crossing text spans (both stroked segments and thin filled rects)
/// and records filled background rectangle colors behind text spans using O(1) SpatialGrid queries.
fn attach_strikethrough_and_bg(page: &mut ParsedPage) {
    if page.spans.is_empty() || (page.segments.is_empty() && page.rects.is_empty()) {
        return;
    }

    // Collect candidate horizontal strike lines: `(x0, x1, y_mid)`
    let mut strike_lines: Vec<(f32, f32, f32)> = Vec::new();
    for seg in &page.segments {
        if seg.is_stroked
            && (seg.p0.1 - seg.p1.1).abs() <= 1.2
            && (seg.p0.0 - seg.p1.0).abs() >= 6.0
        {
            strike_lines.push((
                seg.p0.0.min(seg.p1.0),
                seg.p0.0.max(seg.p1.0),
                (seg.p0.1 + seg.p1.1) * 0.5,
            ));
        }
    }
    let mut bg_rects: Vec<(Rect, (f32, f32, f32))> = Vec::new();
    for vr in &page.rects {
        if vr.is_filled {
            if vr.rect.height() <= 2.5 && vr.rect.width() >= 6.0 {
                strike_lines.push((vr.rect.x0, vr.rect.x1, vr.rect.center_y()));
            } else if let Some(rgb) = vr.fill_rgb {
                if vr.rect.width() >= 6.0 && vr.rect.height() >= 6.0 {
                    bg_rects.push((vr.rect, rgb));
                }
            }
        }
    }

    if !strike_lines.is_empty() {
        let mut strike_grid = SpatialGrid::new(32.0);
        for (idx, &(x0, x1, y)) in strike_lines.iter().enumerate() {
            strike_grid.insert(idx, Rect::new(x0, y - 1.0, x1, y + 1.0));
        }
        for span in &mut page.spans {
            if span.rotation_deg.abs() > 10.0 || span.bbox.width() < 4.0 {
                continue;
            }
            let h = span.bbox.height();
            let mid_lo = span.bbox.y0 + h * 0.30;
            let mid_hi = span.bbox.y0 + h * 0.70;
            for idx in strike_grid.query_rect(span.bbox) {
                let (lx0, lx1, ly) = strike_lines[idx];
                if ly >= mid_lo && ly <= mid_hi {
                    let overlap = (lx1.min(span.bbox.x1) - lx0.max(span.bbox.x0)).max(0.0);
                    if overlap >= span.bbox.width() * 0.72 {
                        span.is_strikeout = true;
                        break;
                    }
                }
            }
        }
    }

    if !bg_rects.is_empty() {
        let mut bg_grid = SpatialGrid::new(64.0);
        for (idx, (rect, _)) in bg_rects.iter().enumerate() {
            bg_grid.insert(idx, *rect);
        }
        for span in &mut page.spans {
            let cx = span.bbox.center_x();
            let cy = span.bbox.center_y();
            for &idx in bg_grid.query_point(cx, cy) {
                let (rect, rgb) = bg_rects[idx];
                if rect.contains_point(cx, cy) {
                    span.bg_color_rgb = Some(rgb);
                }
            }
        }
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
