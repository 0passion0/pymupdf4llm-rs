pub mod document;
pub mod filters;
pub mod lexer;
pub mod xref;

pub use filters::{decode_stream_bytes, decode_stream_bytes_with_objects};
pub use lexer::Lexer;

use std::collections::{BTreeMap, HashSet};

use crate::error::{PdfError, Result};
use crate::pdf::objects::PdfObject;
use crate::pdf::PdfDocument;

use document::{collect_page_tree, extract_metadata, extract_outlines, resolve_obj_map};
use lexer::{
    find_subsequence, find_valid_classic_trailers, parse_indirect_object_at,
    scan_next_indirect_object,
};
use xref::{collect_object_stream_entries, parse_xref_chain, XrefEntry};

pub fn parse_pdf_bytes(bytes: &[u8]) -> Result<PdfDocument> {
    let header_pos = find_subsequence(bytes, b"%PDF-").ok_or(PdfError::InvalidHeader)?;
    let version_end = bytes[header_pos + 5..]
        .iter()
        .position(|&b| b == b'\r' || b == b'\n' || b == b' ')
        .map(|p| header_pos + 5 + p)
        .unwrap_or((header_pos + 8).min(bytes.len()));
    let pdf_version = String::from_utf8_lossy(&bytes[header_pos + 5..version_end]).into_owned();

    // Fast path: follow `startxref` -> classic `xref` / PDF 1.5+ `/Type /XRef` streams & `/Prev`
    let xref_res = parse_xref_chain(bytes, header_pos);
    let mut objects: BTreeMap<u32, PdfObject> = BTreeMap::new();
    let mut trailer: BTreeMap<String, PdfObject> = BTreeMap::new();
    let mut xref_mismatches = 0usize;

    if xref_res.valid_sections > 0 && !xref_res.entries.is_empty() {
        trailer = xref_res.trailer.clone();
        let mut needed_objstms: HashSet<u32> = HashSet::new();

        for (&obj_num, &entry) in &xref_res.entries {
            match entry {
                XrefEntry::Uncompressed(offset) => {
                    if let Some((parsed_id, _gen, obj)) =
                        parse_indirect_object_at(bytes, header_pos, offset)
                    {
                        if parsed_id == obj_num {
                            objects.insert(obj_num, obj);
                        } else {
                            xref_mismatches += 1;
                        }
                    } else {
                        xref_mismatches += 1;
                    }
                }
                XrefEntry::Compressed { objstm_id } => {
                    needed_objstms.insert(objstm_id);
                }
                XrefEntry::Free => {}
            }
        }

        // Unpack `/ObjStm` object streams referenced by XRef entries
        let mut unpacked_entries: Vec<(u32, u32, PdfObject)> = Vec::new();
        for objstm_id in needed_objstms {
            if let Some(PdfObject::Stream { dict, data }) = objects.get(&objstm_id) {
                if dict.get("Type").and_then(|v| v.as_name()) == Some("ObjStm") {
                    if let Ok(decompressed) = decode_stream_bytes_with_objects(&objects, dict, data)
                    {
                        let n = dict.get("N").and_then(|v| v.as_i64()).unwrap_or(0).max(0) as usize;
                        let first = dict
                            .get("First")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0)
                            .max(0) as usize;
                        let mut batch = Vec::new();
                        collect_object_stream_entries(&decompressed, n, first, &mut batch);
                        for (cid, cobj) in batch {
                            unpacked_entries.push((objstm_id, cid, cobj));
                        }
                    }
                }
            }
        }
        for (source_stm, obj_num, obj) in unpacked_entries {
            let allow_insert = match xref_res.entries.get(&obj_num) {
                Some(XrefEntry::Compressed { objstm_id }) => *objstm_id == source_stm,
                None => !objects.contains_key(&obj_num),
                _ => false,
            };
            if allow_insert {
                objects.insert(obj_num, obj);
            }
        }
    }

    if is_document_encrypted(&objects, &trailer) {
        return Err(PdfError::EncryptedDocument);
    }

    // Verify whether XRef fast path yielded a complete, usable page tree;
    // otherwise seamlessly recover via tolerant linear object scan.
    let fast_path_pages =
        if xref_res.valid_sections > 0 && xref_mismatches == 0 && !objects.is_empty() {
            collect_page_tree(&objects, &trailer)
                .ok()
                .filter(|(pages, _)| {
                    !pages.is_empty() && pages.iter().all(|(id, _)| objects.contains_key(id))
                })
        } else {
            None
        };

    let (page_refs, inherited_props) = if let Some(tree) = fast_path_pages {
        tree
    } else {
        recover_objects_linear_scan(bytes, header_pos, &mut objects, &mut trailer, xref_mismatches);

        if is_document_encrypted(&objects, &trailer) {
            return Err(PdfError::EncryptedDocument);
        }

        collect_page_tree(&objects, &trailer)?
    };

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

fn is_document_encrypted(
    objects: &BTreeMap<u32, PdfObject>,
    trailer: &BTreeMap<String, PdfObject>,
) -> bool {
    let Some(enc_obj) = trailer.get("Encrypt") else {
        return false;
    };
    match resolve_obj_map(objects, enc_obj) {
        Some(PdfObject::Null) => false,
        Some(_) => true,
        None => !matches!(enc_obj, PdfObject::Null),
    }
}

fn recover_objects_linear_scan(
    bytes: &[u8],
    header_pos: usize,
    objects: &mut BTreeMap<u32, PdfObject>,
    trailer: &mut BTreeMap<String, PdfObject>,
    xref_mismatches: usize,
) {
    let mut scanned_objects: BTreeMap<u32, PdfObject> = BTreeMap::new();
    let mut obj_positions: BTreeMap<u32, usize> = BTreeMap::new();
    let mut xref_trailers: Vec<(usize, BTreeMap<String, PdfObject>)> = Vec::new();

    let mut cursor = header_pos;
    while let Some((obj_num, _gen_num, obj, obj_pos, next_pos)) =
        scan_next_indirect_object(bytes, cursor)
    {
        if let PdfObject::Stream { ref dict, .. } = obj {
            if dict.get("Type").and_then(|v| v.as_name()) == Some("XRef") {
                xref_trailers.push((obj_pos, dict.clone()));
            }
        }
        scanned_objects.insert(obj_num, obj);
        obj_positions.insert(obj_num, obj_pos);
        cursor = next_pos;
    }

    let mut classic_trailers = find_valid_classic_trailers(bytes);
    let mut all_trailers: Vec<(usize, BTreeMap<String, PdfObject>)> = Vec::new();
    all_trailers.append(&mut classic_trailers);
    all_trailers.extend(xref_trailers);
    all_trailers.sort_by_key(|(pos, _)| *pos);

    let mut linear_trailer: BTreeMap<String, PdfObject> = BTreeMap::new();
    for (_pos, d) in all_trailers.into_iter().rev() {
        if linear_trailer.is_empty() {
            linear_trailer = d;
        } else {
            for key in ["Root", "Info", "Encrypt"] {
                if !linear_trailer.contains_key(key) {
                    if let Some(val) = d.get(key) {
                        linear_trailer.insert(key.to_string(), val.clone());
                    }
                }
            }
        }
    }

    // Unpack `/ObjStm` streams in ascending file position order so newer `/ObjStm`
    // updates overwrite older objects while never clobbering newer direct updates.
    let mut objstm_ids: Vec<(usize, u32)> = scanned_objects
        .iter()
        .filter_map(|(&id, obj)| match obj {
            PdfObject::Stream { dict, .. }
                if dict.get("Type").and_then(|v| v.as_name()) == Some("ObjStm") =>
            {
                Some((obj_positions.get(&id).copied().unwrap_or(0), id))
            }
            _ => None,
        })
        .collect();
    objstm_ids.sort_by_key(|(pos, id)| (*pos, *id));

    let mut unpacked_entries: Vec<(usize, u32, PdfObject)> = Vec::new();
    for (stm_pos, id) in objstm_ids {
        if let Some(PdfObject::Stream { dict, data }) = scanned_objects.get(&id) {
            if let Ok(decompressed) = decode_stream_bytes_with_objects(&scanned_objects, dict, data)
            {
                let n = dict.get("N").and_then(|v| v.as_i64()).unwrap_or(0).max(0) as usize;
                let first = dict
                    .get("First")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
                    .max(0) as usize;
                let mut batch = Vec::new();
                collect_object_stream_entries(&decompressed, n, first, &mut batch);
                for (cid, cobj) in batch {
                    unpacked_entries.push((stm_pos, cid, cobj));
                }
            }
        }
    }
    for (stm_pos, obj_num, obj) in unpacked_entries {
        let existing_pos = obj_positions.get(&obj_num).copied();
        if existing_pos.map_or(true, |ep| stm_pos >= ep) {
            scanned_objects.insert(obj_num, obj);
            obj_positions.insert(obj_num, stm_pos);
        }
    }

    if xref_mismatches > 0 || objects.is_empty() {
        for (id, obj) in scanned_objects {
            objects.insert(id, obj);
        }
    } else {
        for (id, obj) in scanned_objects {
            objects.entry(id).or_insert(obj);
        }
    }

    for (k, v) in linear_trailer {
        trailer.entry(k).or_insert(v);
    }

    if !trailer.contains_key("Root") {
        for (id, obj) in objects.iter() {
            if let Some(dict) = obj.as_dict() {
                if dict.get("Type").and_then(|v| v.as_name()) == Some("Catalog") {
                    trailer.insert("Root".to_string(), PdfObject::Reference(*id, 0));
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filters::decode_ascii85;
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

        let mut zlib_enc = ZlibEncoder::new(Vec::new(), Compression::default());
        zlib_enc.write_all(payload).unwrap();
        let mut full_zlib = zlib_enc.finish().unwrap();
        full_zlib.pop();
        full_zlib.pop();
        let recovered = decode_stream_bytes(&dict, &full_zlib).unwrap();
        assert_eq!(recovered, payload);

        let mut padded_zlib = b"\r\n".to_vec();
        padded_zlib.extend_from_slice(&full_zlib);
        let recovered_padded = decode_stream_bytes(&dict, &padded_zlib).unwrap();
        assert_eq!(recovered_padded, payload);

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

    #[test]
    fn test_lexer_scientific_notation_depth_guard_and_broken_length() {
        let mut sci = Lexer::new(b"[1.5e-4 -2E+3 42]", 0);
        let arr = sci.parse_object().unwrap();
        let items = arr.as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert!((items[0].as_f64().unwrap() - 1.5e-4).abs() < 1e-9);
        assert!((items[1].as_f64().unwrap() - (-2000.0)).abs() < 1e-6);
        assert_eq!(items[2].as_i64(), Some(42));

        let mut nested = vec![b'['; 100];
        nested.extend(vec![b']'; 100]);
        let mut deep_lexer = Lexer::new(&nested, 0);
        assert!(deep_lexer.parse_object().is_err());

        let broken_stream = b"<< /Length 3 >>\nstream\nBT (Recovered) Tj ET\nendstream";
        let mut s_lex = Lexer::new(broken_stream, 0);
        let obj = s_lex.parse_object().unwrap();
        match obj {
            PdfObject::Stream { data, .. } => assert_eq!(data, b"BT (Recovered) Tj ET"),
            _ => panic!("expected stream"),
        }
    }

    #[test]
    fn test_ascii85_overflow_safe_and_indirect_filter() {
        let decoded = decode_ascii85(b"uuuuu~>").unwrap();
        assert_eq!(decoded, vec![0xFF, 0xFF, 0xFF, 0xFF]);

        let mut objects = BTreeMap::new();
        objects.insert(99, PdfObject::Name("ASCIIHexDecode".into()));
        let mut dict = BTreeMap::new();
        dict.insert("Filter".into(), PdfObject::Reference(99, 0));
        let out = decode_stream_bytes_with_objects(&objects, &dict, b"48656C6C6F>").unwrap();
        assert_eq!(out, b"Hello");
    }
}
