use flate2::read::{DeflateDecoder, ZlibDecoder};
use std::collections::{BTreeMap, HashSet};
use std::io::Read;

use crate::config::Rect;
use crate::error::{PdfError, Result};
use crate::markdown::chunk::TocItem;
use crate::pdf::objects::PdfObject;
use crate::pdf::{DocumentMetadata, InheritedPageProps, PdfDocument};

const MAX_TREE_DEPTH: usize = 64;

pub fn parse_pdf_bytes(bytes: &[u8]) -> Result<PdfDocument> {
    let header_pos = find_subsequence(bytes, b"%PDF-").ok_or(PdfError::InvalidHeader)?;
    let version_end = bytes[header_pos + 5..]
        .iter()
        .position(|&b| b == b'\r' || b == b'\n' || b == b' ')
        .map(|p| header_pos + 5 + p)
        .unwrap_or((header_pos + 8).min(bytes.len()));
    let pdf_version = String::from_utf8_lossy(&bytes[header_pos + 5..version_end]).into_owned();

    let mut objects: BTreeMap<u32, PdfObject> = BTreeMap::new();
    let mut trailer: BTreeMap<String, PdfObject> = BTreeMap::new();
    let mut xref_trailers: Vec<(usize, BTreeMap<String, PdfObject>)> = Vec::new();

    // Scan indirect objects (`<num> <gen> obj ... endobj`) across the PDF buffer
    // to support classic xref tables, incremental updates, and XRef streams.
    let mut cursor = header_pos;
    while let Some((obj_num, _gen_num, obj, obj_pos, next_pos)) =
        scan_next_indirect_object(bytes, cursor)
    {
        if let PdfObject::Stream { ref dict, .. } = obj {
            if dict.get("Type").and_then(|v| v.as_name()) == Some("XRef") {
                xref_trailers.push((obj_pos, dict.clone()));
            }
        }
        objects.insert(obj_num, obj);
        cursor = next_pos;
    }

    // Collect valid classic `trailer << ... >>` dictionaries and merge with any /XRef stream
    // trailers in reverse file order so incremental updates keep the latest entries while
    // inheriting Root/Info/Encrypt if omitted in an incremental trailer.
    let mut classic_trailers = find_valid_classic_trailers(bytes);
    let mut all_trailers: Vec<(usize, BTreeMap<String, PdfObject>)> = Vec::new();
    all_trailers.append(&mut classic_trailers);
    all_trailers.extend(xref_trailers);
    all_trailers.sort_by_key(|(pos, _)| *pos);

    for (_pos, d) in all_trailers.into_iter().rev() {
        if trailer.is_empty() {
            trailer = d;
        } else {
            for key in ["Root", "Info", "Encrypt"] {
                if !trailer.contains_key(key) {
                    if let Some(val) = d.get(key) {
                        trailer.insert(key.to_string(), val.clone());
                    }
                }
            }
        }
    }

    // Unpack /ObjStm (Object Streams, PDF 1.5+) without cloning non-ObjStm streams
    let obj_ids: Vec<u32> = objects.keys().copied().collect();
    let mut unpacked_entries: Vec<(u32, PdfObject)> = Vec::new();
    for id in obj_ids {
        if let Some(PdfObject::Stream { dict, data }) = objects.get(&id) {
            if dict.get("Type").and_then(|v| v.as_name()) == Some("ObjStm") {
                if let Ok(decompressed) = decode_stream_bytes(dict, data) {
                    let n = dict.get("N").and_then(|v| v.as_i64()).unwrap_or(0).max(0) as usize;
                    let first = dict
                        .get("First")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0)
                        .max(0) as usize;
                    collect_object_stream_entries(&decompressed, n, first, &mut unpacked_entries);
                }
            }
        }
    }
    for (obj_num, obj) in unpacked_entries {
        objects.entry(obj_num).or_insert(obj);
    }

    if !trailer.contains_key("Root") {
        // Fallback: synthesize /Root from /Catalog object if trailer token was omitted
        for (id, obj) in &objects {
            if let Some(dict) = obj.as_dict() {
                if dict.get("Type").and_then(|v| v.as_name()) == Some("Catalog") {
                    trailer.insert("Root".to_string(), PdfObject::Reference(*id, 0));
                    break;
                }
            }
        }
    }

    if trailer.contains_key("Encrypt") {
        return Err(PdfError::EncryptedDocument);
    }

    let (page_refs, inherited_props) = collect_page_tree(&objects, &trailer)?;
    let metadata = extract_metadata(&objects, &trailer, &pdf_version);
    let toc = extract_outlines(&objects, &trailer, &page_refs);

    Ok(PdfDocument {
        file_path: None,
        pdf_version,
        objects,
        trailer,
        page_refs,
        inherited_props,
        font_cache: dashmap::DashMap::new(),
        metadata,
        toc,
    })
}

fn collect_object_stream_entries(
    data: &[u8],
    n: usize,
    first: usize,
    out: &mut Vec<(u32, PdfObject)>,
) {
    if first > data.len() {
        return;
    }
    let header_str = String::from_utf8_lossy(&data[..first]);
    let nums: Vec<usize> = header_str
        .split_whitespace()
        .filter_map(|t| t.parse::<usize>().ok())
        .collect();
    for i in 0..n {
        let idx = i * 2;
        if idx + 1 >= nums.len() {
            break;
        }
        let obj_num = nums[idx] as u32;
        let rel_offset = nums[idx + 1];
        let abs_offset = first + rel_offset;
        if abs_offset < data.len() {
            let mut lexer = Lexer::new(data, abs_offset);
            if let Ok(obj) = lexer.parse_object() {
                out.push((obj_num, obj));
            }
        }
    }
}

type PageTreeResult = (Vec<(u32, u16)>, BTreeMap<u32, InheritedPageProps>);

fn collect_page_tree(
    objects: &BTreeMap<u32, PdfObject>,
    trailer: &BTreeMap<String, PdfObject>,
) -> Result<PageTreeResult> {
    let root_ref = trailer
        .get("Root")
        .and_then(|v| v.as_reference())
        .ok_or_else(|| PdfError::InvalidXref("Missing /Root catalog in trailer".into()))?;

    let catalog =
        objects
            .get(&root_ref.0)
            .and_then(|o| o.as_dict())
            .ok_or(PdfError::MissingObject {
                obj_num: root_ref.0,
                gen_num: root_ref.1,
            })?;

    let pages_ref = catalog
        .get("Pages")
        .and_then(|v| v.as_reference())
        .ok_or_else(|| PdfError::InvalidXref("Missing /Pages in catalog".into()))?;

    let mut pages = Vec::new();
    let mut inherited = BTreeMap::new();
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
    Ok((pages, inherited))
}

fn resolve_obj_map<'a>(
    objects: &'a BTreeMap<u32, PdfObject>,
    obj: &'a PdfObject,
) -> Option<&'a PdfObject> {
    match obj {
        PdfObject::Reference(id, _) => objects.get(id),
        other => Some(other),
    }
}

fn parse_rect_raw(objects: &BTreeMap<u32, PdfObject>, obj: &PdfObject) -> Option<Rect> {
    let arr = resolve_obj_map(objects, obj)?.as_array()?;
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
    if let Some(rot) = dict.get("Rotate").and_then(|v| v.as_i64()) {
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
    if let Some(kids) = dict.get("Kids").and_then(|v| v.as_array()) {
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

fn extract_metadata(
    objects: &BTreeMap<u32, PdfObject>,
    trailer: &BTreeMap<String, PdfObject>,
    version: &str,
) -> DocumentMetadata {
    let mut meta = DocumentMetadata {
        format: format!("PDF {version}"),
        ..Default::default()
    };
    let Some(info_ref) = trailer.get("Info").and_then(|v| v.as_reference()) else {
        return meta;
    };
    let Some(info) = objects.get(&info_ref.0).and_then(|o| o.as_dict()) else {
        return meta;
    };
    meta.title = info
        .get("Title")
        .and_then(|v| v.as_text_string())
        .unwrap_or_default();
    meta.author = info
        .get("Author")
        .and_then(|v| v.as_text_string())
        .unwrap_or_default();
    meta.subject = info
        .get("Subject")
        .and_then(|v| v.as_text_string())
        .unwrap_or_default();
    meta.keywords = info
        .get("Keywords")
        .and_then(|v| v.as_text_string())
        .unwrap_or_default();
    meta.creator = info
        .get("Creator")
        .and_then(|v| v.as_text_string())
        .unwrap_or_default();
    meta.producer = info
        .get("Producer")
        .and_then(|v| v.as_text_string())
        .unwrap_or_default();
    meta.creation_date = info
        .get("CreationDate")
        .and_then(|v| v.as_text_string())
        .unwrap_or_default();
    meta.mod_date = info
        .get("ModDate")
        .and_then(|v| v.as_text_string())
        .unwrap_or_default();
    meta
}

fn extract_outlines(
    objects: &BTreeMap<u32, PdfObject>,
    trailer: &BTreeMap<String, PdfObject>,
    page_refs: &[(u32, u16)],
) -> Vec<TocItem> {
    let mut toc = Vec::new();
    let Some(root_ref) = trailer.get("Root").and_then(|v| v.as_reference()) else {
        return toc;
    };
    let Some(catalog) = objects.get(&root_ref.0).and_then(|o| o.as_dict()) else {
        return toc;
    };
    let Some(outlines_ref) = catalog.get("Outlines").and_then(|v| v.as_reference()) else {
        return toc;
    };
    let Some(outlines) = objects.get(&outlines_ref.0).and_then(|o| o.as_dict()) else {
        return toc;
    };
    if let Some(first_ref) = outlines.get("First").and_then(|v| v.as_reference()) {
        let mut visited = HashSet::new();
        walk_outline_siblings(objects, first_ref, 1, &mut visited, page_refs, &mut toc);
    }
    toc
}

fn walk_outline_siblings(
    objects: &BTreeMap<u32, PdfObject>,
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
            .and_then(|v| v.as_text_string())
            .unwrap_or_default();
        let page_num = resolve_dest_page(objects, dict, page_refs).unwrap_or(1);
        if !title.is_empty() {
            out.push((level, title, page_num));
        }
        if let Some(child_ref) = dict.get("First").and_then(|v| v.as_reference()) {
            walk_outline_siblings(objects, child_ref, level + 1, visited, page_refs, out);
        }
        match dict.get("Next").and_then(|v| v.as_reference()) {
            Some(next_ref) => current_ref = next_ref,
            None => break,
        }
    }
}

fn resolve_dest_page(
    objects: &BTreeMap<u32, PdfObject>,
    dict: &BTreeMap<String, PdfObject>,
    page_refs: &[(u32, u16)],
) -> Option<usize> {
    if let Some(dest) = dict.get("Dest").and_then(|v| v.as_array()) {
        if let Some((obj_num, _)) = dest.first().and_then(|v| v.as_reference()) {
            return page_refs
                .iter()
                .position(|(p, _)| *p == obj_num)
                .map(|i| i + 1);
        }
    }
    if let Some(action) = dict.get("A") {
        let action_dict = match action {
            PdfObject::Reference(id, _) => objects.get(id).and_then(|o| o.as_dict()),
            PdfObject::Dict(d) => Some(d),
            _ => None,
        }?;
        if let Some(dest) = action_dict.get("D").and_then(|v| v.as_array()) {
            if let Some((obj_num, _)) = dest.first().and_then(|v| v.as_reference()) {
                return page_refs
                    .iter()
                    .position(|(p, _)| *p == obj_num)
                    .map(|i| i + 1);
            }
        }
    }
    None
}

const MAX_DECOMPRESSED_STREAM_BYTES: usize = 64 * 1024 * 1024;

fn decode_parms_for_filter(
    dict: &BTreeMap<String, PdfObject>,
    idx: usize,
) -> Option<&BTreeMap<String, PdfObject>> {
    match dict.get("DecodeParms") {
        Some(PdfObject::Dict(d)) => Some(d),
        Some(PdfObject::Array(arr)) => arr.get(idx).and_then(|v| v.as_dict()),
        _ => None,
    }
}

fn decompress_flate_resilient(input: &[u8]) -> Result<Vec<u8>> {
    let decoder = ZlibDecoder::new(input);
    let mut limited = decoder.take((MAX_DECOMPRESSED_STREAM_BYTES + 1) as u64);
    let mut out = Vec::new();
    match limited.read_to_end(&mut out) {
        Ok(_) => {
            if out.len() > MAX_DECOMPRESSED_STREAM_BYTES {
                return Err(PdfError::Decompress {
                    filter: "FlateDecode".into(),
                    message: "Decompressed stream exceeded 64 MB safety limit".into(),
                });
            }
            Ok(out)
        }
        Err(e) => {
            if out.len() > MAX_DECOMPRESSED_STREAM_BYTES {
                return Err(PdfError::Decompress {
                    filter: "FlateDecode".into(),
                    message: "Decompressed stream exceeded 64 MB safety limit".into(),
                });
            }
            // Recover truncated zlib streams if valid content was already decoded
            if out.len() >= 8 {
                return Ok(out);
            }
            // Fallback for non-standard raw deflate streams lacking zlib header
            let raw_dec = DeflateDecoder::new(input);
            let mut raw_lim = raw_dec.take((MAX_DECOMPRESSED_STREAM_BYTES + 1) as u64);
            let mut raw_out = Vec::new();
            if raw_lim.read_to_end(&mut raw_out).is_ok()
                && raw_out.len() <= MAX_DECOMPRESSED_STREAM_BYTES
                && !raw_out.is_empty()
            {
                return Ok(raw_out);
            }
            Err(PdfError::Decompress {
                filter: "FlateDecode".into(),
                message: e.to_string(),
            })
        }
    }
}

pub fn decode_stream_bytes(dict: &BTreeMap<String, PdfObject>, raw: &[u8]) -> Result<Vec<u8>> {
    let filters: Vec<String> = match dict.get("Filter") {
        None => return Ok(raw.to_vec()),
        Some(PdfObject::Name(n)) => vec![n.clone()],
        Some(PdfObject::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_name().map(ToString::to_string))
            .collect(),
        _ => return Ok(raw.to_vec()),
    };

    let mut data = raw.to_vec();
    for (idx, filter) in filters.into_iter().enumerate() {
        let parms = decode_parms_for_filter(dict, idx);
        data = match filter.as_str() {
            "FlateDecode" | "Fl" => {
                let out = decompress_flate_resilient(&data)?;
                if let Some(parms) = parms {
                    apply_predictor(parms, &out).unwrap_or(out)
                } else {
                    out
                }
            }
            "ASCIIHexDecode" | "AHx" => decode_ascii_hex(&data),
            "ASCII85Decode" | "A85" => decode_ascii85(&data)?,
            "RunLengthDecode" | "RL" => decode_run_length(&data)?,
            "LZWDecode" | "LZW" => {
                let early = parms
                    .and_then(|d| d.get("EarlyChange"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1)
                    != 0;
                let decoded = decode_lzw(&data, early)?;
                if let Some(parms) = parms {
                    apply_predictor(parms, &decoded).unwrap_or(decoded)
                } else {
                    decoded
                }
            }
            _ => data,
        };
    }
    Ok(data)
}

fn decode_lzw(data: &[u8], early_change: bool) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut dict: Vec<Vec<u8>> = (0..=255).map(|b| vec![b as u8]).collect();
    dict.push(Vec::new()); // 256 = Clear
    dict.push(Vec::new()); // 257 = EOD

    let mut code_size = 9usize;
    let mut bit_pos = 0usize;

    let read_bits = |pos: &mut usize, width: usize| -> Option<u32> {
        if pos.saturating_add(width) > data.len().saturating_mul(8) {
            return None;
        }
        let mut val = 0u32;
        for _ in 0..width {
            let byte_idx = *pos / 8;
            let bit_idx = 7 - (*pos % 8);
            val = (val << 1) | (((data[byte_idx] >> bit_idx) & 1) as u32);
            *pos += 1;
        }
        Some(val)
    };

    let mut prev: Option<Vec<u8>> = None;
    while let Some(code) = read_bits(&mut bit_pos, code_size) {
        if code == 256 {
            dict.truncate(258);
            code_size = 9;
            prev = None;
            continue;
        }
        if code == 257 {
            break;
        }
        let entry = if (code as usize) < dict.len() {
            dict[code as usize].clone()
        } else if code as usize == dict.len() {
            let Some(ref p) = prev else { break };
            let mut e = p.clone();
            if let Some(&first) = p.first() {
                e.push(first);
            }
            e
        } else {
            break;
        };
        if out.len().saturating_add(entry.len()) > MAX_DECOMPRESSED_STREAM_BYTES {
            return Err(PdfError::Decompress {
                filter: "LZWDecode".into(),
                message: "Decompressed stream exceeded 64 MB safety limit".into(),
            });
        }
        out.extend_from_slice(&entry);
        if let Some(ref p) = prev {
            if dict.len() < 4096 && !entry.is_empty() {
                let mut new_entry = p.clone();
                new_entry.push(entry[0]);
                dict.push(new_entry);
                let threshold = if early_change {
                    (1usize << code_size) - 1
                } else {
                    1usize << code_size
                };
                if dict.len() >= threshold && code_size < 12 {
                    code_size += 1;
                }
            }
        }
        prev = Some(entry);
    }
    Ok(out)
}

fn apply_predictor(parms: &BTreeMap<String, PdfObject>, data: &[u8]) -> Option<Vec<u8>> {
    let predictor = parms.get("Predictor").and_then(|v| v.as_i64()).unwrap_or(1);
    if predictor <= 1 {
        return None;
    }
    let cols = parms
        .get("Columns")
        .and_then(|v| v.as_i64())
        .unwrap_or(1)
        .clamp(1, 1_000_000) as usize;
    let bpc = parms
        .get("BitsPerComponent")
        .and_then(|v| v.as_i64())
        .unwrap_or(8)
        .clamp(1, 32) as usize;
    let colors = parms
        .get("Colors")
        .and_then(|v| v.as_i64())
        .unwrap_or(1)
        .clamp(1, 64) as usize;
    let bpp = colors.saturating_mul(bpc).div_ceil(8).max(1);
    let row_bytes = cols.saturating_mul(colors).saturating_mul(bpc).div_ceil(8);
    if row_bytes == 0 || row_bytes > MAX_DECOMPRESSED_STREAM_BYTES {
        return None;
    }

    if predictor == 2 && bpc == 8 {
        if data.len() < row_bytes {
            return None;
        }
        let mut out = Vec::with_capacity(data.len());
        for chunk in data.chunks_exact(row_bytes) {
            let mut row = chunk.to_vec();
            for i in bpp..row_bytes {
                row[i] = row[i].wrapping_add(row[i - bpp]);
            }
            out.extend_from_slice(&row);
        }
        return Some(out);
    }

    if predictor < 10 {
        return None;
    }
    let stride = row_bytes.checked_add(1)?;
    if stride <= 1 || data.len() < stride {
        return None;
    }
    let mut out = Vec::with_capacity((data.len() / stride) * row_bytes);
    let mut prev_row = vec![0u8; row_bytes];
    for chunk in data.chunks_exact(stride) {
        let filter_type = chunk[0];
        let src = &chunk[1..];
        let mut row = vec![0u8; row_bytes];
        for i in 0..row_bytes {
            let left = if i >= bpp { row[i - bpp] } else { 0 };
            let up = prev_row[i];
            let up_left = if i >= bpp { prev_row[i - bpp] } else { 0 };
            row[i] = match filter_type {
                0 => src[i],
                1 => src[i].wrapping_add(left),
                2 => src[i].wrapping_add(up),
                3 => src[i].wrapping_add(((left as u16 + up as u16) / 2) as u8),
                4 => src[i].wrapping_add(paeth(left, up, up_left)),
                _ => src[i],
            };
        }
        out.extend_from_slice(&row);
        prev_row = row;
    }
    Some(out)
}

#[inline]
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let a = a as i32;
    let b = b as i32;
    let c = c as i32;
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

fn decode_ascii_hex(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut nibbles = Vec::new();
    for &b in data {
        if b == b'>' {
            break;
        }
        if let Some(v) = match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        } {
            nibbles.push(v);
        }
    }
    for pair in nibbles.chunks(2) {
        let hi = pair[0];
        let lo = pair.get(1).copied().unwrap_or(0);
        out.push((hi << 4) | lo);
    }
    out
}

fn decode_ascii85(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut group = Vec::with_capacity(5);
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        i += 1;
        if b == b'~' && i < data.len() && data[i] == b'>' {
            break;
        }
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'z' && group.is_empty() {
            if out.len().saturating_add(4) > MAX_DECOMPRESSED_STREAM_BYTES {
                return Err(PdfError::Decompress {
                    filter: "ASCII85Decode".into(),
                    message: "Decompressed stream exceeded 64 MB safety limit".into(),
                });
            }
            out.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        if (b'!'..=b'u').contains(&b) {
            group.push((b - b'!') as u32);
            if group.len() == 5 {
                let val = group.iter().fold(0u32, |acc, &d| acc * 85 + d);
                if out.len().saturating_add(4) > MAX_DECOMPRESSED_STREAM_BYTES {
                    return Err(PdfError::Decompress {
                        filter: "ASCII85Decode".into(),
                        message: "Decompressed stream exceeded 64 MB safety limit".into(),
                    });
                }
                out.extend_from_slice(&val.to_be_bytes());
                group.clear();
            }
        }
    }
    if !group.is_empty() {
        let n = group.len();
        while group.len() < 5 {
            group.push(84);
        }
        let val = group.iter().fold(0u32, |acc, &d| acc * 85 + d);
        let bytes = val.to_be_bytes();
        let extra = n - 1;
        if out.len().saturating_add(extra) > MAX_DECOMPRESSED_STREAM_BYTES {
            return Err(PdfError::Decompress {
                filter: "ASCII85Decode".into(),
                message: "Decompressed stream exceeded 64 MB safety limit".into(),
            });
        }
        out.extend_from_slice(&bytes[..extra]);
    }
    Ok(out)
}

fn decode_run_length(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let len = data[i];
        i += 1;
        if len == 128 {
            break;
        } else if len < 128 {
            let count = len as usize + 1;
            let end = (i + count).min(data.len());
            let added = end - i;
            if out.len().saturating_add(added) > MAX_DECOMPRESSED_STREAM_BYTES {
                return Err(PdfError::Decompress {
                    filter: "RunLengthDecode".into(),
                    message: "Decompressed stream exceeded 64 MB safety limit".into(),
                });
            }
            out.extend_from_slice(&data[i..end]);
            i = end;
        } else {
            let count = 257 - len as usize;
            if i < data.len() {
                if out.len().saturating_add(count) > MAX_DECOMPRESSED_STREAM_BYTES {
                    return Err(PdfError::Decompress {
                        filter: "RunLengthDecode".into(),
                        message: "Decompressed stream exceeded 64 MB safety limit".into(),
                    });
                }
                out.resize(out.len() + count, data[i]);
                i += 1;
            }
        }
    }
    Ok(out)
}

fn find_valid_classic_trailers(bytes: &[u8]) -> Vec<(usize, BTreeMap<String, PdfObject>)> {
    let mut found = Vec::new();
    let mut pos = 0usize;
    let needle = b"trailer";
    while pos + needle.len() <= bytes.len() {
        let Some(rel) = find_subsequence(&bytes[pos..], needle) else {
            break;
        };
        let kw_pos = pos + rel;
        let prev_ok = kw_pos == 0 || bytes[kw_pos - 1].is_ascii_whitespace();
        let next_pos = kw_pos + needle.len();
        let next_ok = next_pos < bytes.len()
            && (bytes[next_pos].is_ascii_whitespace() || bytes[next_pos] == b'<');
        if prev_ok && next_ok {
            let mut p = Lexer::new(bytes, next_pos);
            if let Ok(PdfObject::Dict(d)) = p.parse_object() {
                if d.contains_key("Root") || d.contains_key("Size") || d.contains_key("Prev") {
                    found.push((kw_pos, d));
                }
            }
        }
        pos = next_pos;
    }
    found
}

fn scan_next_indirect_object(
    bytes: &[u8],
    start: usize,
) -> Option<(u32, u16, PdfObject, usize, usize)> {
    let mut pos = start;
    while pos < bytes.len() {
        let rel = find_subsequence(&bytes[pos..], b"obj")?;
        let obj_kw = pos + rel;
        let prev_ok = obj_kw == 0 || bytes[obj_kw - 1].is_ascii_whitespace();
        let next_ok = obj_kw + 3 >= bytes.len()
            || bytes[obj_kw + 3].is_ascii_whitespace()
            || bytes[obj_kw + 3] == b'<'
            || bytes[obj_kw + 3] == b'[';
        if !prev_ok || !next_ok {
            pos = obj_kw + 3;
            continue;
        }

        // Parse `<obj_num> <gen_num>` immediately preceding `obj`
        let line_start = bytes[..obj_kw]
            .iter()
            .rposition(|&b| b == b'\n' || b == b'\r')
            .map(|p| p + 1)
            .unwrap_or(0);
        let header_slice = String::from_utf8_lossy(&bytes[line_start..obj_kw]);
        let parts: Vec<&str> = header_slice.split_whitespace().collect();
        if parts.len() >= 2 {
            if let (Ok(obj_num), Ok(gen_num)) = (
                parts[parts.len() - 2].parse::<u32>(),
                parts[parts.len() - 1].parse::<u16>(),
            ) {
                let mut lexer = Lexer::new(bytes, obj_kw + 3);
                if let Ok(obj) = lexer.parse_object() {
                    let next_pos = find_subsequence(&bytes[lexer.pos..], b"endobj")
                        .map(|p| lexer.pos + p + 6)
                        .unwrap_or(lexer.pos);
                    return Some((obj_num, gen_num, obj, obj_kw, next_pos));
                }
            }
        }
        pos = obj_kw + 3;
    }
    None
}

pub struct Lexer<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    pub fn skip_ws_and_comments(&mut self) {
        while self.pos < self.data.len() {
            let b = self.data[self.pos];
            if b.is_ascii_whitespace() || b == 0 {
                self.pos += 1;
            } else if b == b'%' {
                while self.pos < self.data.len()
                    && self.data[self.pos] != b'\n'
                    && self.data[self.pos] != b'\r'
                {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    pub fn parse_object(&mut self) -> Result<PdfObject> {
        self.skip_ws_and_comments();
        if self.pos >= self.data.len() {
            return Err(PdfError::Syntax {
                offset: self.pos,
                message: "Unexpected EOF".into(),
            });
        }
        let b = self.data[self.pos];
        match b {
            b'<' if self.peek_at(1) == Some(b'<') => {
                let dict = self.parse_dict()?;
                let saved = self.pos;
                self.skip_ws_and_comments();
                if self.data[self.pos..].starts_with(b"stream") {
                    self.pos += 6;
                    while matches!(self.data.get(self.pos), Some(b' ' | b'\t')) {
                        self.pos += 1;
                    }
                    if self.data.get(self.pos) == Some(&b'\r') {
                        self.pos += 1;
                    }
                    if self.data.get(self.pos) == Some(&b'\n') {
                        self.pos += 1;
                    }
                    let stream_start = self.pos;
                    let length = dict
                        .get("Length")
                        .and_then(|v| v.as_i64())
                        .map(|l| l.max(0) as usize);
                    let mut trim_eol = false;
                    let stream_end = if let Some(len) = length {
                        if stream_start + len <= self.data.len() {
                            stream_start + len
                        } else {
                            trim_eol = true;
                            find_subsequence(&self.data[stream_start..], b"endstream")
                                .map(|p| stream_start + p)
                                .unwrap_or(self.data.len())
                        }
                    } else {
                        trim_eol = true;
                        find_subsequence(&self.data[stream_start..], b"endstream")
                            .map(|p| stream_start + p)
                            .unwrap_or(self.data.len())
                    };
                    let mut end_slice = stream_end;
                    if trim_eol && end_slice > stream_start {
                        if self.data[end_slice - 1] == b'\n' {
                            end_slice -= 1;
                        }
                        if end_slice > stream_start && self.data[end_slice - 1] == b'\r' {
                            end_slice -= 1;
                        }
                    }
                    let raw = self.data[stream_start..end_slice].to_vec();
                    self.pos = stream_end;
                    if let Some(p) = find_subsequence(&self.data[self.pos..], b"endstream") {
                        self.pos += p + 9;
                    }
                    Ok(PdfObject::Stream { dict, data: raw })
                } else {
                    self.pos = saved;
                    Ok(PdfObject::Dict(dict))
                }
            }
            b'<' => self.parse_hex_string(),
            b'(' => self.parse_literal_string(),
            b'[' => self.parse_array(),
            b'/' => self.parse_name(),
            b't' if self.data[self.pos..].starts_with(b"true")
                && self.is_token_boundary(self.pos + 4) =>
            {
                self.pos += 4;
                Ok(PdfObject::Bool(true))
            }
            b'f' if self.data[self.pos..].starts_with(b"false")
                && self.is_token_boundary(self.pos + 5) =>
            {
                self.pos += 5;
                Ok(PdfObject::Bool(false))
            }
            b'n' if self.data[self.pos..].starts_with(b"null")
                && self.is_token_boundary(self.pos + 4) =>
            {
                self.pos += 4;
                Ok(PdfObject::Null)
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => self.parse_number_or_ref(),
            _ => Err(PdfError::Syntax {
                offset: self.pos,
                message: format!("Unexpected byte 0x{:02x}", b),
            }),
        }
    }

    fn peek_at(&self, delta: usize) -> Option<u8> {
        self.data.get(self.pos + delta).copied()
    }

    fn is_token_boundary(&self, idx: usize) -> bool {
        match self.data.get(idx) {
            None => true,
            Some(&b) => {
                b.is_ascii_whitespace()
                    || matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'/' | b'%')
            }
        }
    }

    fn parse_dict(&mut self) -> Result<BTreeMap<String, PdfObject>> {
        self.pos += 2; // <<
        let mut dict = BTreeMap::new();
        loop {
            self.skip_ws_and_comments();
            if self.pos + 1 < self.data.len()
                && self.data[self.pos] == b'>'
                && self.data[self.pos + 1] == b'>'
            {
                self.pos += 2;
                break;
            }
            if self.pos >= self.data.len() {
                break;
            }
            if self.data[self.pos] != b'/' {
                return Err(PdfError::Syntax {
                    offset: self.pos,
                    message: "Expected dictionary key name".into(),
                });
            }
            let PdfObject::Name(key) = self.parse_name()? else {
                return Err(PdfError::Syntax {
                    offset: self.pos,
                    message: "Invalid dictionary key".into(),
                });
            };
            let val = self.parse_object()?;
            dict.insert(key, val);
        }
        Ok(dict)
    }

    fn parse_array(&mut self) -> Result<PdfObject> {
        self.pos += 1; // [
        let mut items = Vec::new();
        loop {
            self.skip_ws_and_comments();
            if self.pos >= self.data.len() {
                break;
            }
            if self.data[self.pos] == b']' {
                self.pos += 1;
                break;
            }
            items.push(self.parse_object()?);
        }
        Ok(PdfObject::Array(items))
    }

    fn parse_name(&mut self) -> Result<PdfObject> {
        self.pos += 1; // /
        let mut out = String::new();
        while self.pos < self.data.len() {
            let b = self.data[self.pos];
            if b.is_ascii_whitespace()
                || matches!(
                    b,
                    b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
                )
            {
                break;
            }
            if b == b'#' && self.pos + 2 < self.data.len() {
                if let Ok(s) = std::str::from_utf8(&self.data[self.pos + 1..self.pos + 3]) {
                    if let Ok(v) = u8::from_str_radix(s, 16) {
                        out.push(v as char);
                        self.pos += 3;
                        continue;
                    }
                }
            }
            out.push(b as char);
            self.pos += 1;
        }
        Ok(PdfObject::Name(out))
    }

    fn parse_literal_string(&mut self) -> Result<PdfObject> {
        self.pos += 1; // (
        let mut out = Vec::new();
        let mut depth = 1usize;
        while self.pos < self.data.len() && depth > 0 {
            let b = self.data[self.pos];
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b'(');
                }
                b')' => {
                    depth -= 1;
                    if depth > 0 {
                        out.push(b')');
                    }
                }
                b'\\' if self.pos < self.data.len() => {
                    let esc = self.data[self.pos];
                    self.pos += 1;
                    match esc {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'(' | b')' | b'\\' => out.push(esc),
                        b'\r' => {
                            if self.data.get(self.pos) == Some(&b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut oct = esc - b'0';
                            for _ in 0..2 {
                                if let Some(&d @ b'0'..=b'7') = self.data.get(self.pos) {
                                    oct = oct.wrapping_mul(8).wrapping_add(d - b'0');
                                    self.pos += 1;
                                } else {
                                    break;
                                }
                            }
                            out.push(oct);
                        }
                        other => out.push(other),
                    }
                }
                other => out.push(other),
            }
        }
        Ok(PdfObject::String(out))
    }

    fn parse_hex_string(&mut self) -> Result<PdfObject> {
        self.pos += 1; // <
        let mut nibbles = Vec::new();
        while self.pos < self.data.len() {
            let b = self.data[self.pos];
            self.pos += 1;
            if b == b'>' {
                break;
            }
            if let Some(v) = match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                b'A'..=b'F' => Some(b - b'A' + 10),
                _ => None,
            } {
                nibbles.push(v);
            }
        }
        let mut out = Vec::with_capacity(nibbles.len().div_ceil(2));
        for chunk in nibbles.chunks(2) {
            out.push((chunk[0] << 4) | chunk.get(1).copied().unwrap_or(0));
        }
        Ok(PdfObject::String(out))
    }

    fn parse_number_or_ref(&mut self) -> Result<PdfObject> {
        let start = self.pos;
        let mut has_dot = false;
        if matches!(self.data[self.pos], b'+' | b'-') {
            self.pos += 1;
        }
        while self.pos < self.data.len() {
            let b = self.data[self.pos];
            if b == b'.' {
                has_dot = true;
                self.pos += 1;
            } else if b.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        let token = std::str::from_utf8(&self.data[start..self.pos]).unwrap_or("0");
        if has_dot {
            let val = token.parse::<f64>().unwrap_or(0.0);
            return Ok(PdfObject::Real(val));
        }
        let int_val = token.parse::<i64>().unwrap_or(0);
        // Look ahead for `<int> <gen> R` indirect reference
        let saved = self.pos;
        self.skip_ws_and_comments();
        let g_start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.pos > g_start {
            if let Ok(gen_num) = std::str::from_utf8(&self.data[g_start..self.pos])
                .unwrap_or("")
                .parse::<u16>()
            {
                self.skip_ws_and_comments();
                if self.data.get(self.pos) == Some(&b'R')
                    && self
                        .data
                        .get(self.pos + 1)
                        .map(|b| !b.is_ascii_alphanumeric())
                        .unwrap_or(true)
                {
                    self.pos += 1;
                    return Ok(PdfObject::Reference(
                        int_val.clamp(0, u32::MAX as i64) as u32,
                        gen_num,
                    ));
                }
            }
        }
        self.pos = saved;
        Ok(PdfObject::Int(int_val))
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::{DeflateEncoder, ZlibEncoder};
    use flate2::Compression;
    use std::io::Write;

    #[test]
    fn test_decode_stream_raw_deflate_truncated_zlib_and_tiff_predictor() {
        let payload = b"BT /F1 12 Tf (Hello Resilient Deflate) Tj ET";
        let mut raw_enc = DeflateEncoder::new(Vec::new(), Compression::default());
        raw_enc.write_all(payload).unwrap();
        let raw_deflate = raw_enc.finish().unwrap();

        let mut dict = BTreeMap::new();
        dict.insert("Filter".into(), PdfObject::Name("FlateDecode".into()));
        let decoded_raw = decode_stream_bytes(&dict, &raw_deflate).unwrap();
        assert_eq!(decoded_raw, payload);

        // Truncated zlib stream (missing Adler-32 trailer) should still recover valid output
        let mut zlib_enc = ZlibEncoder::new(Vec::new(), Compression::default());
        zlib_enc.write_all(payload).unwrap();
        let mut full_zlib = zlib_enc.finish().unwrap();
        full_zlib.pop();
        full_zlib.pop();
        let recovered = decode_stream_bytes(&dict, &full_zlib).unwrap();
        assert_eq!(recovered, payload);

        // TIFF Predictor 2 with array-typed Filter & DecodeParms
        let tiff_encoded = vec![10u8, 2u8, 3u8, 20u8, 5u8, 1u8];
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(&tiff_encoded).unwrap();
        let z_bytes = z.finish().unwrap();

        let mut parms = BTreeMap::new();
        parms.insert("Predictor".into(), PdfObject::Int(2));
        parms.insert("Columns".into(), PdfObject::Int(3));
        parms.insert("BitsPerComponent".into(), PdfObject::Int(8));
        parms.insert("Colors".into(), PdfObject::Int(1));

        let mut tiff_dict = BTreeMap::new();
        tiff_dict.insert(
            "Filter".into(),
            PdfObject::Array(vec![PdfObject::Name("FlateDecode".into())]),
        );
        tiff_dict.insert(
            "DecodeParms".into(),
            PdfObject::Array(vec![PdfObject::Dict(parms)]),
        );
        let tiff_out = decode_stream_bytes(&tiff_dict, &z_bytes).unwrap();
        assert_eq!(tiff_out, vec![10, 12, 15, 20, 25, 26]);
    }
}
