use std::collections::{BTreeMap, HashSet};

use crate::pdf::objects::PdfObject;

use super::filters::decode_stream_bytes;
use super::lexer::{
    parse_indirect_object_at_raw, read_ascii_usize, rfind_subsequence, skip_ws_and_comments_slice,
    Lexer,
};

const MAX_XREF_CHAIN: usize = 64;
const MAX_XREF_SUBSECTION_COUNT: usize = 2_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum XrefEntry {
    Free,
    Uncompressed(usize),
    Compressed { objstm_id: u32 },
}

#[derive(Default)]
pub(crate) struct XrefParseResult {
    pub(crate) entries: BTreeMap<u32, XrefEntry>,
    pub(crate) trailer: BTreeMap<String, PdfObject>,
    pub(crate) valid_sections: usize,
}

pub(crate) fn parse_xref_chain(bytes: &[u8], header_pos: usize) -> XrefParseResult {
    let mut result = XrefParseResult::default();
    let Some(start_offset) = find_startxref_offset(bytes, header_pos) else {
        return result;
    };

    let mut visited: HashSet<usize> = HashSet::new();
    let mut current_offset = Some(start_offset);

    while let Some(offset) = current_offset {
        if result.valid_sections >= MAX_XREF_CHAIN || !visited.insert(offset) {
            break;
        }
        let Some(pos) = resolve_xref_pos(bytes, header_pos, offset) else {
            break;
        };

        if bytes[pos..].starts_with(b"xref") {
            let Some(trailer_dict) = parse_classic_xref_section(bytes, pos, &mut result.entries)
            else {
                break;
            };
            result.valid_sections += 1;

            if let Some(stm_off) = trailer_dict
                .get("XRefStm")
                .and_then(|v| v.as_i64())
                .and_then(|v| usize::try_from(v).ok())
            {
                if visited.insert(stm_off) {
                    if let Some(stm_pos) = resolve_xref_pos(bytes, header_pos, stm_off) {
                        if let Some((_, _, PdfObject::Stream { dict, data })) =
                            parse_indirect_object_at_raw(bytes, stm_pos)
                        {
                            if dict.get("Type").and_then(|v| v.as_name()) == Some("XRef") {
                                parse_xref_stream_entries(&dict, &data, &mut result.entries);
                            }
                        }
                    }
                }
            }

            let prev = trailer_dict
                .get("Prev")
                .and_then(|v| v.as_i64())
                .and_then(|v| usize::try_from(v).ok());
            for (k, v) in trailer_dict {
                result.trailer.entry(k).or_insert(v);
            }
            current_offset = prev;
        } else if let Some((_, _, PdfObject::Stream { dict, data })) =
            parse_indirect_object_at_raw(bytes, pos)
        {
            if dict.get("Type").and_then(|v| v.as_name()) != Some("XRef") {
                break;
            }
            parse_xref_stream_entries(&dict, &data, &mut result.entries);
            result.valid_sections += 1;
            let prev = dict
                .get("Prev")
                .and_then(|v| v.as_i64())
                .and_then(|v| usize::try_from(v).ok());
            for (k, v) in dict {
                result.trailer.entry(k).or_insert(v);
            }
            current_offset = prev;
        } else {
            break;
        }
    }

    result
}

fn find_startxref_offset(bytes: &[u8], header_pos: usize) -> Option<usize> {
    let tail_start = bytes.len().saturating_sub(8192).max(header_pos);
    let rel = rfind_subsequence(&bytes[tail_start..], b"startxref")?;
    let mut pos = tail_start + rel + b"startxref".len();
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    let num_start = pos;
    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
        pos += 1;
    }
    if pos == num_start {
        return None;
    }
    std::str::from_utf8(&bytes[num_start..pos])
        .ok()?
        .parse::<usize>()
        .ok()
}

fn resolve_xref_pos(bytes: &[u8], header_pos: usize, offset: usize) -> Option<usize> {
    let candidates: [Option<usize>; 2] = [
        header_pos.checked_add(offset),
        if header_pos > 0 { Some(offset) } else { None },
    ];
    for cand in candidates.into_iter().flatten() {
        if cand >= bytes.len() {
            continue;
        }
        let mut p = cand;
        while p < bytes.len() && bytes[p].is_ascii_whitespace() {
            p += 1;
        }
        if p < bytes.len() && (bytes[p..].starts_with(b"xref") || bytes[p].is_ascii_digit()) {
            return Some(p);
        }
    }
    None
}

fn parse_classic_xref_section(
    bytes: &[u8],
    start: usize,
    entries: &mut BTreeMap<u32, XrefEntry>,
) -> Option<BTreeMap<String, PdfObject>> {
    let mut pos = start + 4; // skip "xref"
    loop {
        skip_ws_and_comments_slice(bytes, &mut pos);
        if pos >= bytes.len() {
            return None;
        }
        if bytes[pos..].starts_with(b"trailer")
            && bytes
                .get(pos + 7)
                .map_or(true, |b| b.is_ascii_whitespace() || *b == b'<')
        {
            pos += 7;
            let mut lexer = Lexer::new(bytes, pos);
            return match lexer.parse_object() {
                Ok(PdfObject::Dict(d)) => Some(d),
                _ => None,
            };
        }

        let first_id = read_ascii_usize(bytes, &mut pos)? as u32;
        skip_ws_and_comments_slice(bytes, &mut pos);
        let count = read_ascii_usize(bytes, &mut pos)?;
        if count > MAX_XREF_SUBSECTION_COUNT {
            return None;
        }

        for i in 0..count {
            skip_ws_and_comments_slice(bytes, &mut pos);
            let offset = read_ascii_usize(bytes, &mut pos)?;
            skip_ws_and_comments_slice(bytes, &mut pos);
            let _gen_num = read_ascii_usize(bytes, &mut pos)?;
            skip_ws_and_comments_slice(bytes, &mut pos);
            let flag = *bytes.get(pos)?;
            pos += 1;
            let obj_num = first_id.saturating_add(i as u32);
            match flag {
                b'n' => {
                    entries
                        .entry(obj_num)
                        .or_insert(XrefEntry::Uncompressed(offset));
                }
                b'f' => {
                    entries.entry(obj_num).or_insert(XrefEntry::Free);
                }
                _ => return None,
            }
        }
    }
}

fn parse_xref_stream_entries(
    dict: &BTreeMap<String, PdfObject>,
    data: &[u8],
    entries: &mut BTreeMap<u32, XrefEntry>,
) {
    let Ok(decoded) = decode_stream_bytes(dict, data) else {
        return;
    };
    let Some(w_arr) = dict.get("W").and_then(|v| v.as_array()) else {
        return;
    };
    if w_arr.len() != 3 {
        return;
    }
    let w1 = w_arr[0].as_i64().unwrap_or(0).clamp(0, 8) as usize;
    let w2 = w_arr[1].as_i64().unwrap_or(0).clamp(0, 8) as usize;
    let w3 = w_arr[2].as_i64().unwrap_or(0).clamp(0, 8) as usize;
    let entry_size = w1 + w2 + w3;
    if entry_size == 0 {
        return;
    }

    let size = dict
        .get("Size")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        .clamp(0, u32::MAX as i64) as u32;

    let mut index_pairs: Vec<(u32, u32)> = Vec::new();
    if let Some(idx_arr) = dict.get("Index").and_then(|v| v.as_array()) {
        for pair in idx_arr.chunks_exact(2) {
            if let (Some(first), Some(count)) = (pair[0].as_i64(), pair[1].as_i64()) {
                if first >= 0 && count > 0 {
                    index_pairs.push((
                        first.min(u32::MAX as i64) as u32,
                        count.min(MAX_XREF_SUBSECTION_COUNT as i64) as u32,
                    ));
                }
            }
        }
    }
    if index_pairs.is_empty() && size > 0 {
        index_pairs.push((0, size.min(MAX_XREF_SUBSECTION_COUNT as u32)));
    }

    let mut cursor = 0usize;
    for (first, count) in index_pairs {
        for i in 0..count {
            if cursor + entry_size > decoded.len() {
                return;
            }
            let f1 = if w1 == 0 {
                1
            } else {
                read_be_u64(&decoded[cursor..cursor + w1])
            };
            let f2 = read_be_u64(&decoded[cursor + w1..cursor + w1 + w2]);
            cursor += entry_size;

            let obj_num = first.saturating_add(i);
            match f1 {
                0 => {
                    entries.entry(obj_num).or_insert(XrefEntry::Free);
                }
                1 => {
                    entries
                        .entry(obj_num)
                        .or_insert(XrefEntry::Uncompressed(f2 as usize));
                }
                2 => {
                    entries.entry(obj_num).or_insert(XrefEntry::Compressed {
                        objstm_id: f2 as u32,
                    });
                }
                _ => {}
            }
        }
    }
}

#[inline]
fn read_be_u64(slice: &[u8]) -> u64 {
    let mut val = 0u64;
    for &b in slice {
        val = (val << 8) | (b as u64);
    }
    val
}

pub(crate) fn collect_object_stream_entries(
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
