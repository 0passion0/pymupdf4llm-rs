use std::collections::{BTreeMap, HashMap};

use super::text_emitter::PageCoordMap;
use crate::config::Rect;
use crate::error::Result;
use crate::pdf::font::PdfFont;
use crate::pdf::objects::PdfObject;
use crate::pdf::parser::decode_stream_bytes_with_objects;
use crate::pdf::{LinkAnnotation, PdfDocument};

#[derive(Debug, Clone)]
pub(super) struct ExtGStateProps {
    pub(super) alpha: Option<f32>,
    pub(super) font_override: Option<(String, PdfFont, f32)>,
}

pub(super) fn merge_resource_dicts(
    doc: &PdfDocument,
    base: &mut BTreeMap<String, PdfObject>,
    overlay: &BTreeMap<String, PdfObject>,
) {
    for (k, v) in overlay {
        let resolved_overlay = doc.resolve(v).unwrap_or(v);
        match (base.get_mut(k), resolved_overlay) {
            (Some(PdfObject::Dict(existing)), PdfObject::Dict(overlay_dict)) => {
                for (sk, sv) in overlay_dict {
                    existing.insert(sk.clone(), sv.clone());
                }
            }
            (Some(existing_ref @ PdfObject::Reference(_, _)), PdfObject::Dict(overlay_dict)) => {
                if let Some(PdfObject::Dict(base_dict)) = doc.resolve(existing_ref) {
                    let mut merged = base_dict.clone();
                    for (sk, sv) in overlay_dict {
                        merged.insert(sk.clone(), sv.clone());
                    }
                    base.insert(k.clone(), PdfObject::Dict(merged));
                } else {
                    base.insert(k.clone(), PdfObject::Dict(overlay_dict.clone()));
                }
            }
            (_, PdfObject::Dict(overlay_dict)) => {
                base.insert(k.clone(), PdfObject::Dict(overlay_dict.clone()));
            }
            _ => {
                base.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Collects and decompresses page `/Contents` stream(s). For multi-stream arrays,
/// tolerates individual corrupted sub-streams so valid sibling streams are still salvaged.
pub(super) fn collect_content_bytes(
    doc: &PdfDocument,
    obj: &PdfObject,
    out: &mut Vec<u8>,
) -> Result<()> {
    let mut last_err = None;
    let mut decoded_count = 0usize;
    collect_content_bytes_inner(doc, obj, out, &mut decoded_count, &mut last_err);
    if decoded_count == 0 {
        if let Some(err) = last_err {
            return Err(err);
        }
    }
    Ok(())
}

fn collect_content_bytes_inner(
    doc: &PdfDocument,
    obj: &PdfObject,
    out: &mut Vec<u8>,
    decoded_count: &mut usize,
    last_err: &mut Option<crate::error::PdfError>,
) {
    match doc.resolve(obj) {
        Some(PdfObject::Stream { dict, data }) => {
            match decode_stream_bytes_with_objects(&doc.objects, dict, data) {
                Ok(decoded) => {
                    out.extend_from_slice(&decoded);
                    out.push(b'\n');
                    *decoded_count += 1;
                }
                Err(e) => {
                    *last_err = Some(e);
                }
            }
        }
        Some(PdfObject::Array(arr)) => {
            for item in arr {
                collect_content_bytes_inner(doc, item, out, decoded_count, last_err);
            }
        }
        _ => {}
    }
}

pub(super) fn load_fonts(
    doc: &PdfDocument,
    resources: &BTreeMap<String, PdfObject>,
) -> HashMap<String, PdfFont> {
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

pub(super) fn load_ext_gstate_props(
    doc: &PdfDocument,
    resources: &BTreeMap<String, PdfObject>,
) -> HashMap<String, ExtGStateProps> {
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
            let alpha = d
                .get("ca")
                .or_else(|| d.get("CA"))
                .and_then(|v| doc.resolve(v))
                .and_then(|v| v.as_f32())
                .map(|a| a.clamp(0.0, 1.0));
            let font_override = d
                .get("Font")
                .and_then(|v| doc.resolve(v))
                .and_then(|v| v.as_array())
                .and_then(|arr| {
                    if arr.len() < 2 {
                        return None;
                    }
                    let font = PdfFont::from_dict(doc, &arr[0]);
                    let size = doc
                        .resolve(&arr[1])
                        .and_then(|v| v.as_f32())
                        .map(f32::abs)
                        .filter(|&s| s.is_finite() && s > 1e-4)
                        .unwrap_or(12.0);
                    let synthetic_key = format!("__extgs_{name}");
                    Some((synthetic_key, font, size))
                });
            if alpha.is_some() || font_override.is_some() {
                map.insert(
                    name.clone(),
                    ExtGStateProps {
                        alpha,
                        font_override,
                    },
                );
            }
        }
    }
    map
}

pub(super) fn lookup_property_actual_text(
    doc: &PdfDocument,
    resources: &BTreeMap<String, PdfObject>,
    prop_name: &str,
) -> Option<String> {
    let props = resources
        .get("Properties")
        .and_then(|o| doc.resolve(o))
        .and_then(|o| o.as_dict())?;
    let entry = props
        .get(prop_name)
        .and_then(|o| doc.resolve(o))
        .and_then(|o| o.as_dict())?;
    entry
        .get("ActualText")
        .and_then(|o| doc.resolve(o))
        .and_then(|o| o.as_text_string())
        .and_then(normalize_actual_text)
}

pub(super) fn normalize_actual_text(raw: String) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|&c| c != '\u{FEFF}' && (!c.is_control() || c == '\n' || c == '\t'))
        .collect();
    if cleaned.trim().is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

pub(super) fn load_link_annotations(
    doc: &PdfDocument,
    page_dict: &BTreeMap<String, PdfObject>,
    coord_map: PageCoordMap,
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
        let Some(raw_rect) = dict
            .get("Rect")
            .and_then(|v| doc.resolve(v))
            .and_then(|v| v.as_array())
        else {
            continue;
        };
        if raw_rect.len() != 4 {
            continue;
        }
        let x0 = doc
            .resolve(&raw_rect[0])
            .and_then(|v| v.as_f32())
            .unwrap_or(0.0);
        let y0_pdf = doc
            .resolve(&raw_rect[1])
            .and_then(|v| v.as_f32())
            .unwrap_or(0.0);
        let x1 = doc
            .resolve(&raw_rect[2])
            .and_then(|v| v.as_f32())
            .unwrap_or(0.0);
        let y1_pdf = doc
            .resolve(&raw_rect[3])
            .and_then(|v| v.as_f32())
            .unwrap_or(0.0);
        let (ax, ay) = coord_map.map_point(x0.min(x1), y1_pdf.max(y0_pdf));
        let (bx, by) = coord_map.map_point(x0.max(x1), y0_pdf.min(y1_pdf));
        let rect = Rect::new(ax, ay, bx, by);
        let uri = dict
            .get("A")
            .and_then(|a| doc.resolve(a))
            .and_then(|a| a.as_dict())
            .and_then(|ad| ad.get("URI"))
            .and_then(|u| doc.resolve(u))
            .and_then(|u| u.as_text_string());
        links.push(LinkAnnotation {
            rect,
            uri,
            dest_page: None,
        });
    }
    links
}

pub(super) fn parse_rect_from_dict(
    doc: &PdfDocument,
    dict: &BTreeMap<String, PdfObject>,
    key: &str,
) -> Option<Rect> {
    let arr = dict
        .get(key)
        .and_then(|o| doc.resolve(o))
        .and_then(|o| o.as_array())?;
    if arr.len() != 4 {
        return None;
    }
    Some(Rect::new(
        doc.resolve(&arr[0])?.as_f32()?,
        doc.resolve(&arr[1])?.as_f32()?,
        doc.resolve(&arr[2])?.as_f32()?,
        doc.resolve(&arr[3])?.as_f32()?,
    ))
}
