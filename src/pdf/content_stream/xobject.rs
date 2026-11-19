use std::collections::{BTreeMap, HashMap, HashSet};

use super::interpreter::interpret_content_stream;
use super::resources::{load_fonts, merge_resource_dicts};
use super::text_emitter::{mul_matrix, transform_point, PageCoordMap};
use super::GraphicsState;
use crate::config::{Rect, ToMarkdownOptions};
use crate::pdf::font::PdfFont;
use crate::pdf::objects::PdfObject;
use crate::pdf::parser::decode_stream_bytes_with_objects;
use crate::pdf::{ParsedPage, PdfDocument, RawImagePlacement};

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_xobject(
    doc: &PdfDocument,
    resources: &BTreeMap<String, PdfObject>,
    parent_fonts: &HashMap<String, PdfFont>,
    xname: &str,
    gs: &GraphicsState,
    coord_map: PageCoordMap,
    options: &ToMarkdownOptions,
    depth: usize,
    active_xobjects: &mut HashSet<u32>,
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
    let ref_obj_num = xobj_ref.as_reference().map(|(id, _)| id);
    let Some(PdfObject::Stream { dict, data }) = doc.resolve(xobj_ref) else {
        return;
    };
    match dict
        .get("Subtype")
        .and_then(|v| doc.resolve(v))
        .and_then(|v| v.as_name())
    {
        Some("Image") if !options.ignore_images => {
            let (x0, y0) = transform_point(gs.ctm, 0.0, 0.0);
            let (x1, y1) = transform_point(gs.ctm, 1.0, 1.0);
            let (ax, ay) = coord_map.map_point(x0.min(x1), y0.max(y1));
            let (bx, by) = coord_map.map_point(x0.max(x1), y0.min(y1));
            let bbox = Rect::new(ax, ay, bx, by);
            let width = dict
                .get("Width")
                .and_then(|v| doc.resolve(v))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as u32;
            let height = dict
                .get("Height")
                .and_then(|v| doc.resolve(v))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as u32;
            let bpc = dict
                .get("BitsPerComponent")
                .and_then(|v| doc.resolve(v))
                .and_then(|v| v.as_i64())
                .unwrap_or(8) as u8;
            let color_space = match dict.get("ColorSpace").and_then(|v| doc.resolve(v)) {
                Some(PdfObject::Name(n)) => n.clone(),
                Some(PdfObject::Array(arr)) => arr
                    .first()
                    .and_then(|v| doc.resolve(v))
                    .and_then(|v| v.as_name())
                    .unwrap_or("DeviceRGB")
                    .to_string(),
                _ => "DeviceRGB".to_string(),
            };
            let filter = match dict.get("Filter").and_then(|v| doc.resolve(v)) {
                Some(PdfObject::Name(n)) => Some(n.clone()),
                Some(PdfObject::Array(arr)) => arr
                    .first()
                    .and_then(|v| doc.resolve(v))
                    .and_then(|v| v.as_name())
                    .map(ToString::to_string),
                _ => None,
            };
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
            if let Some(id) = ref_obj_num {
                if !active_xobjects.insert(id) {
                    // Break cyclic Form XObject self-recursion
                    return;
                }
            }
            let Ok(decoded) = decode_stream_bytes_with_objects(&doc.objects, dict, data) else {
                if let Some(id) = ref_obj_num {
                    active_xobjects.remove(&id);
                }
                return;
            };
            let mut form_gs = gs.clone();
            if let Some(arr) = dict
                .get("Matrix")
                .and_then(|o| doc.resolve(o))
                .and_then(|o| o.as_array())
            {
                if arr.len() == 6 {
                    let fm = [
                        doc.resolve(&arr[0])
                            .and_then(|v| v.as_f32())
                            .unwrap_or(1.0),
                        doc.resolve(&arr[1])
                            .and_then(|v| v.as_f32())
                            .unwrap_or(0.0),
                        doc.resolve(&arr[2])
                            .and_then(|v| v.as_f32())
                            .unwrap_or(0.0),
                        doc.resolve(&arr[3])
                            .and_then(|v| v.as_f32())
                            .unwrap_or(1.0),
                        doc.resolve(&arr[4])
                            .and_then(|v| v.as_f32())
                            .unwrap_or(0.0),
                        doc.resolve(&arr[5])
                            .and_then(|v| v.as_f32())
                            .unwrap_or(0.0),
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
                merge_resource_dicts(doc, &mut form_resources, fres);
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
                coord_map,
                depth + 1,
                active_xobjects,
                page,
            );

            if let Some(id) = ref_obj_num {
                active_xobjects.remove(&id);
            }
        }
        _ => {}
    }
}
