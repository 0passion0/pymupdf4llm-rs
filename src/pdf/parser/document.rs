use std::collections::{BTreeMap, HashSet};

use crate::config::Rect;
use crate::error::{PdfError, Result};
use crate::markdown::chunk::TocItem;
use crate::pdf::objects::PdfObject;
use crate::pdf::{DocumentMetadata, InheritedPageProps};

const MAX_TREE_DEPTH: usize = 64;

pub(crate) type PageTreeResult = (Vec<(u32, u16)>, BTreeMap<u32, InheritedPageProps>);

pub(crate) fn resolve_obj_map<'a>(
    objects: &'a BTreeMap<u32, PdfObject>,
    obj: &'a PdfObject,
) -> Option<&'a PdfObject> {
    let mut current = obj;
    for _ in 0..8 {
        match current {
            PdfObject::Reference(id, _) => {
                current = objects.get(id)?;
            }
            other => return Some(other),
        }
    }
    None
}

pub(crate) fn parse_rect_raw(objects: &BTreeMap<u32, PdfObject>, obj: &PdfObject) -> Option<Rect> {
    let arr = resolve_obj_map(objects, obj)?.as_array()?;
    if arr.len() != 4 {
        return None;
    }
    Some(Rect::new(
        resolve_obj_map(objects, &arr[0])?.as_f32()?,
        resolve_obj_map(objects, &arr[1])?.as_f32()?,
        resolve_obj_map(objects, &arr[2])?.as_f32()?,
        resolve_obj_map(objects, &arr[3])?.as_f32()?,
    ))
}

pub(crate) fn collect_page_tree(
    objects: &BTreeMap<u32, PdfObject>,
    trailer: &BTreeMap<String, PdfObject>,
) -> Result<PageTreeResult> {
    let mut pages = Vec::new();
    let mut inherited = BTreeMap::new();

    if let Some(catalog) = trailer
        .get("Root")
        .and_then(|v| resolve_obj_map(objects, v))
        .and_then(|o| o.as_dict())
    {
        if let Some(pages_ref) = catalog.get("Pages").and_then(|v| v.as_reference()) {
            let mut visited = HashSet::new();
            walk_pages_node(
                objects,
                pages_ref,
                InheritedPageProps::default(),
                0,
                &mut visited,
                &mut pages,
                &mut inherited,
            );
        }
    }

    if !pages.is_empty() {
        return Ok((pages, inherited));
    }

    // Salvage fallback for damaged PDFs with missing/corrupt Catalog or Pages tree
    for (&obj_num, obj) in objects {
        if let Some(dict) = obj.as_dict() {
            if dict.get("Type").and_then(|v| v.as_name()) == Some("Page") {
                let mut props = InheritedPageProps::default();
                if let Some(mb) = dict
                    .get("MediaBox")
                    .and_then(|o| parse_rect_raw(objects, o))
                {
                    props.media_box = Some(mb);
                }
                if let Some(cb) = dict.get("CropBox").and_then(|o| parse_rect_raw(objects, o)) {
                    props.crop_box = Some(cb);
                }
                if let Some(rot) = dict
                    .get("Rotate")
                    .and_then(|o| resolve_obj_map(objects, o))
                    .and_then(|v| v.as_i64())
                {
                    props.rotate = Some(rot as i32);
                }
                inherited.insert(obj_num, props);
                pages.push((obj_num, 0));
            }
        }
    }

    if !pages.is_empty() {
        return Ok((pages, inherited));
    }

    let root_ref = trailer
        .get("Root")
        .and_then(|v| v.as_reference())
        .ok_or_else(|| PdfError::InvalidXref("Missing /Root catalog in trailer".into()))?;
    let catalog = objects
        .get(&root_ref.0)
        .and_then(|o| o.as_dict())
        .ok_or(PdfError::MissingObject {
            obj_num: root_ref.0,
            gen_num: root_ref.1,
        })?;
    if catalog.get("Pages").and_then(|v| v.as_reference()).is_none() {
        return Err(PdfError::InvalidXref("Missing /Pages in catalog".into()));
    }
    Ok((pages, inherited))
}

fn walk_pages_node(
    objects: &BTreeMap<u32, PdfObject>,
    node_ref: (u32, u16),
    mut parent_props: InheritedPageProps,
    depth: usize,
    visited: &mut HashSet<u32>,
    out: &mut Vec<(u32, u16)>,
    inherited_map: &mut BTreeMap<u32, InheritedPageProps>,
) {
    if depth > MAX_TREE_DEPTH || !visited.insert(node_ref.0) {
        return;
    }
    let Some(dict) = objects.get(&node_ref.0).and_then(|o| o.as_dict()) else {
        return;
    };
    if let Some(mb) = dict
        .get("MediaBox")
        .and_then(|o| parse_rect_raw(objects, o))
    {
        parent_props.media_box = Some(mb);
    }
    if let Some(cb) = dict.get("CropBox").and_then(|o| parse_rect_raw(objects, o)) {
        parent_props.crop_box = Some(cb);
    }
    if let Some(rot) = dict
        .get("Rotate")
        .and_then(|o| resolve_obj_map(objects, o))
        .and_then(|v| v.as_i64())
    {
        parent_props.rotate = Some(rot as i32);
    }
    if let Some(res_dict) = dict
        .get("Resources")
        .and_then(|o| resolve_obj_map(objects, o))
        .and_then(|o| o.as_dict())
    {
        for (k, v) in res_dict {
            match (
                parent_props.resources.get_mut(k),
                resolve_obj_map(objects, v),
            ) {
                (Some(PdfObject::Dict(existing)), Some(PdfObject::Dict(overlay))) => {
                    for (sk, sv) in overlay {
                        existing.insert(sk.clone(), sv.clone());
                    }
                }
                _ => {
                    if let Some(PdfObject::Dict(overlay)) = resolve_obj_map(objects, v) {
                        parent_props
                            .resources
                            .insert(k.clone(), PdfObject::Dict(overlay.clone()));
                    } else {
                        parent_props.resources.insert(k.clone(), v.clone());
                    }
                }
            }
        }
    }

    let node_type = dict.get("Type").and_then(|v| v.as_name());
    if node_type == Some("Page") {
        inherited_map.insert(node_ref.0, parent_props);
        out.push(node_ref);
        return;
    }
    if let Some(kids) = dict
        .get("Kids")
        .and_then(|o| resolve_obj_map(objects, o))
        .and_then(|v| v.as_array())
    {
        for kid in kids {
            if let Some(kid_ref) = kid.as_reference() {
                walk_pages_node(
                    objects,
                    kid_ref,
                    parent_props.clone(),
                    depth + 1,
                    visited,
                    out,
                    inherited_map,
                );
            }
        }
    } else if node_type != Some("Pages") {
        inherited_map.insert(node_ref.0, parent_props);
        out.push(node_ref);
    }
}

pub(crate) fn extract_metadata(
    objects: &BTreeMap<u32, PdfObject>,
    trailer: &BTreeMap<String, PdfObject>,
    version: &str,
) -> DocumentMetadata {
    let mut meta = DocumentMetadata {
        format: format!("PDF {version}"),
        ..Default::default()
    };
    let Some(info) = trailer
        .get("Info")
        .and_then(|v| resolve_obj_map(objects, v))
        .and_then(|o| o.as_dict())
    else {
        return meta;
    };
    let read_field = |key: &str| -> String {
        info.get(key)
            .and_then(|v| resolve_obj_map(objects, v))
            .and_then(|v| v.as_text_string())
            .unwrap_or_default()
    };
    meta.title = read_field("Title");
    meta.author = read_field("Author");
    meta.subject = read_field("Subject");
    meta.keywords = read_field("Keywords");
    meta.creator = read_field("Creator");
    meta.producer = read_field("Producer");
    meta.creation_date = read_field("CreationDate");
    meta.mod_date = read_field("ModDate");
    meta
}

pub(crate) fn extract_outlines(
    objects: &BTreeMap<u32, PdfObject>,
    trailer: &BTreeMap<String, PdfObject>,
    page_refs: &[(u32, u16)],
) -> Vec<TocItem> {
    let mut toc = Vec::new();
    let Some(catalog) = trailer
        .get("Root")
        .and_then(|v| resolve_obj_map(objects, v))
        .and_then(|o| o.as_dict())
    else {
        return toc;
    };
    let Some(outlines) = catalog
        .get("Outlines")
        .and_then(|v| resolve_obj_map(objects, v))
        .and_then(|o| o.as_dict())
    else {
        return toc;
    };
    if let Some(first_ref) = outlines.get("First").and_then(|v| v.as_reference()) {
        let mut visited = HashSet::new();
        walk_outline_siblings(
            objects,
            catalog,
            first_ref,
            1,
            &mut visited,
            page_refs,
            &mut toc,
        );
    }
    toc
}

fn walk_outline_siblings(
    objects: &BTreeMap<u32, PdfObject>,
    catalog: &BTreeMap<String, PdfObject>,
    mut current_ref: (u32, u16),
    level: usize,
    visited: &mut HashSet<u32>,
    page_refs: &[(u32, u16)],
    out: &mut Vec<TocItem>,
) {
    if level > MAX_TREE_DEPTH {
        return;
    }
    let mut guard = 0usize;
    while guard < 4096 && visited.insert(current_ref.0) {
        guard += 1;
        let Some(dict) = objects.get(&current_ref.0).and_then(|o| o.as_dict()) else {
            break;
        };
        let title = dict
            .get("Title")
            .and_then(|v| resolve_obj_map(objects, v))
            .and_then(|v| v.as_text_string())
            .unwrap_or_default();
        let page_num = resolve_dest_page(objects, catalog, dict, page_refs).unwrap_or(1);
        if !title.is_empty() {
            out.push((level, title, page_num));
        }
        if let Some(child_ref) = dict.get("First").and_then(|v| v.as_reference()) {
            walk_outline_siblings(
                objects,
                catalog,
                child_ref,
                level + 1,
                visited,
                page_refs,
                out,
            );
        }
        match dict.get("Next").and_then(|v| v.as_reference()) {
            Some(next_ref) => current_ref = next_ref,
            None => break,
        }
    }
}

fn resolve_dest_page(
    objects: &BTreeMap<u32, PdfObject>,
    catalog: &BTreeMap<String, PdfObject>,
    dict: &BTreeMap<String, PdfObject>,
    page_refs: &[(u32, u16)],
) -> Option<usize> {
    if let Some(dest_obj) = dict.get("Dest") {
        if let Some(p) = resolve_dest_object(objects, catalog, dest_obj, page_refs, 0) {
            return Some(p);
        }
    }
    if let Some(action) = dict.get("A") {
        let action_dict = resolve_obj_map(objects, action)?.as_dict()?;
        if let Some(dest_obj) = action_dict.get("D") {
            return resolve_dest_object(objects, catalog, dest_obj, page_refs, 0);
        }
    }
    None
}

fn resolve_dest_object(
    objects: &BTreeMap<u32, PdfObject>,
    catalog: &BTreeMap<String, PdfObject>,
    dest_obj: &PdfObject,
    page_refs: &[(u32, u16)],
    depth: usize,
) -> Option<usize> {
    if depth > 4 {
        return None;
    }
    let resolved = resolve_obj_map(objects, dest_obj)?;
    match resolved {
        PdfObject::Array(dest) => {
            let (obj_num, _) = dest.first()?.as_reference()?;
            page_refs
                .iter()
                .position(|(p, _)| *p == obj_num)
                .map(|i| i + 1)
        }
        PdfObject::Dict(d) => {
            let inner = d.get("D")?;
            resolve_dest_object(objects, catalog, inner, page_refs, depth + 1)
        }
        PdfObject::Name(name) => {
            let target = lookup_named_dest(objects, catalog, name)?;
            resolve_dest_object(objects, catalog, target, page_refs, depth + 1)
        }
        PdfObject::String(_) => {
            let name = resolved.as_text_string()?;
            let target = lookup_named_dest(objects, catalog, &name)?;
            resolve_dest_object(objects, catalog, target, page_refs, depth + 1)
        }
        _ => None,
    }
}

fn lookup_named_dest<'a>(
    objects: &'a BTreeMap<u32, PdfObject>,
    catalog: &'a BTreeMap<String, PdfObject>,
    dest_name: &str,
) -> Option<&'a PdfObject> {
    if let Some(dests_dict) = catalog
        .get("Dests")
        .and_then(|o| resolve_obj_map(objects, o))
        .and_then(|o| o.as_dict())
    {
        if let Some(target) = dests_dict.get(dest_name) {
            return Some(target);
        }
    }
    let names_dict = catalog
        .get("Names")
        .and_then(|o| resolve_obj_map(objects, o))
        .and_then(|o| o.as_dict())?;
    let dests_tree = names_dict
        .get("Dests")
        .and_then(|o| resolve_obj_map(objects, o))
        .and_then(|o| o.as_dict())?;
    lookup_name_tree(objects, dests_tree, dest_name, 0)
}

fn lookup_name_tree<'a>(
    objects: &'a BTreeMap<u32, PdfObject>,
    node: &'a BTreeMap<String, PdfObject>,
    key: &str,
    depth: usize,
) -> Option<&'a PdfObject> {
    if depth > MAX_TREE_DEPTH {
        return None;
    }
    if let Some(names) = node
        .get("Names")
        .and_then(|o| resolve_obj_map(objects, o))
        .and_then(|o| o.as_array())
    {
        for pair in names.chunks_exact(2) {
            if let Some(k) = resolve_obj_map(objects, &pair[0]).and_then(|o| o.as_text_string()) {
                if k == key {
                    return Some(&pair[1]);
                }
            }
        }
    }
    if let Some(kids) = node
        .get("Kids")
        .and_then(|o| resolve_obj_map(objects, o))
        .and_then(|o| o.as_array())
    {
        for kid in kids {
            if let Some(kid_dict) = resolve_obj_map(objects, kid).and_then(|o| o.as_dict()) {
                if let Some(limits) = kid_dict
                    .get("Limits")
                    .and_then(|o| resolve_obj_map(objects, o))
                    .and_then(|o| o.as_array())
                {
                    if limits.len() == 2 {
                        let lo = resolve_obj_map(objects, &limits[0])
                            .and_then(|o| o.as_text_string());
                        let hi = resolve_obj_map(objects, &limits[1])
                            .and_then(|o| o.as_text_string());
                        if let (Some(lo_s), Some(hi_s)) = (lo, hi) {
                            if key < lo_s.as_str() || key > hi_s.as_str() {
                                continue;
                            }
                        }
                    }
                }
                if let Some(found) = lookup_name_tree(objects, kid_dict, key, depth + 1) {
                    return Some(found);
                }
            }
        }
    }
    None
}
