use flate2::read::{DeflateDecoder, ZlibDecoder};
use std::collections::BTreeMap;
use std::io::Read;

use crate::error::{PdfError, Result};
use crate::pdf::objects::PdfObject;

use super::document::resolve_obj_map;

const MAX_DECOMPRESSED_STREAM_BYTES: usize = 64 * 1024 * 1024;

pub fn decode_stream_bytes(dict: &BTreeMap<String, PdfObject>, raw: &[u8]) -> Result<Vec<u8>> {
    let empty = BTreeMap::new();
    decode_stream_bytes_with_objects(&empty, dict, raw)
}

pub fn decode_stream_bytes_with_objects(
    objects: &BTreeMap<u32, PdfObject>,
    dict: &BTreeMap<String, PdfObject>,
    raw: &[u8],
) -> Result<Vec<u8>> {
    let filter_obj = dict
        .get("Filter")
        .or_else(|| dict.get("F"))
        .and_then(|v| resolve_obj_map(objects, v));
    let filters: Vec<String> = match filter_obj {
        None => return Ok(raw.to_vec()),
        Some(PdfObject::Name(n)) => vec![n.clone()],
        Some(PdfObject::Array(arr)) => arr
            .iter()
            .filter_map(|v| {
                resolve_obj_map(objects, v)
                    .and_then(|o| o.as_name())
                    .map(ToString::to_string)
            })
            .collect(),
        _ => return Ok(raw.to_vec()),
    };

    let mut data = raw.to_vec();
    for (idx, filter) in filters.into_iter().enumerate() {
        let parms = decode_parms_for_filter(objects, dict, idx);
        data = match filter.as_str() {
            "FlateDecode" | "Fl" => {
                let out = decompress_flate_resilient(&data)?;
                if let Some(parms) = parms {
                    apply_predictor(objects, parms, &out).unwrap_or(out)
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
                    .and_then(|v| resolve_obj_map(objects, v))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1)
                    != 0;
                let decoded = decode_lzw(&data, early)?;
                if let Some(parms) = parms {
                    apply_predictor(objects, parms, &decoded).unwrap_or(decoded)
                } else {
                    decoded
                }
            }
            _ => data,
        };
    }
    Ok(data)
}

fn decode_parms_for_filter<'a>(
    objects: &'a BTreeMap<u32, PdfObject>,
    dict: &'a BTreeMap<String, PdfObject>,
    idx: usize,
) -> Option<&'a BTreeMap<String, PdfObject>> {
    let raw_parms = dict.get("DecodeParms").or_else(|| dict.get("DP"))?;
    match resolve_obj_map(objects, raw_parms)? {
        PdfObject::Dict(d) => Some(d),
        PdfObject::Array(arr) => arr
            .get(idx)
            .and_then(|v| resolve_obj_map(objects, v))
            .and_then(|v| v.as_dict()),
        _ => None,
    }
}

fn try_zlib_decode(input: &[u8]) -> std::result::Result<Vec<u8>, std::io::Error> {
    let decoder = ZlibDecoder::new(input);
    let mut limited = decoder.take((MAX_DECOMPRESSED_STREAM_BYTES + 1) as u64);
    let mut out = Vec::new();
    match limited.read_to_end(&mut out) {
        Ok(_) => Ok(out),
        Err(e) => {
            if out.len() >= 8 && out.len() <= MAX_DECOMPRESSED_STREAM_BYTES {
                Ok(out)
            } else {
                Err(e)
            }
        }
    }
}

fn try_raw_deflate_decode(input: &[u8]) -> Option<Vec<u8>> {
    let raw_dec = DeflateDecoder::new(input);
    let mut raw_lim = raw_dec.take((MAX_DECOMPRESSED_STREAM_BYTES + 1) as u64);
    let mut raw_out = Vec::new();
    match raw_lim.read_to_end(&mut raw_out) {
        Ok(_) if !raw_out.is_empty() && raw_out.len() <= MAX_DECOMPRESSED_STREAM_BYTES => {
            Some(raw_out)
        }
        Err(_) if raw_out.len() >= 8 && raw_out.len() <= MAX_DECOMPRESSED_STREAM_BYTES => {
            Some(raw_out)
        }
        _ => None,
    }
}

fn is_valid_zlib_header(slice: &[u8]) -> bool {
    if slice.len() < 2 {
        return false;
    }
    let cmf = slice[0];
    let flg = slice[1];
    (cmf & 0x0F) == 8 && (cmf >> 4) <= 7 && (((cmf as u16) << 8) | (flg as u16)) % 31 == 0
}

fn decompress_flate_resilient(input: &[u8]) -> Result<Vec<u8>> {
    let first_err = match try_zlib_decode(input) {
        Ok(out) => {
            if out.len() > MAX_DECOMPRESSED_STREAM_BYTES {
                return Err(PdfError::Decompress {
                    filter: "FlateDecode".into(),
                    message: "Decompressed stream exceeded 64 MB safety limit".into(),
                });
            }
            return Ok(out);
        }
        Err(e) => e,
    };

    // Scan up to 16 leading bytes for a valid RFC 1950 zlib header (handles stray EOL/BOM/padding)
    let scan_limit = input.len().min(16);
    for offset in 1..scan_limit {
        let candidate = &input[offset..];
        if is_valid_zlib_header(candidate) {
            if let Ok(out) = try_zlib_decode(candidate) {
                if out.len() <= MAX_DECOMPRESSED_STREAM_BYTES {
                    return Ok(out);
                }
            }
        }
    }

    let leading_ws = input
        .iter()
        .take(8)
        .take_while(|&&b| matches!(b, b'\r' | b'\n' | b' ' | b'\t'))
        .count();
    let trimmed = &input[leading_ws..];
    if leading_ws > 0 && !trimmed.is_empty() {
        if let Ok(out) = try_zlib_decode(trimmed) {
            if out.len() <= MAX_DECOMPRESSED_STREAM_BYTES {
                return Ok(out);
            }
        }
    }

    if let Some(raw_out) = try_raw_deflate_decode(input) {
        return Ok(raw_out);
    }
    if leading_ws > 0 && !trimmed.is_empty() {
        if let Some(raw_out) = try_raw_deflate_decode(trimmed) {
            return Ok(raw_out);
        }
    }

    Err(PdfError::Decompress {
        filter: "FlateDecode".into(),
        message: first_err.to_string(),
    })
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

fn apply_predictor(
    objects: &BTreeMap<u32, PdfObject>,
    parms: &BTreeMap<String, PdfObject>,
    data: &[u8],
) -> Option<Vec<u8>> {
    let read_i64 = |key: &str, default: i64| -> i64 {
        parms
            .get(key)
            .and_then(|v| resolve_obj_map(objects, v))
            .and_then(|v| v.as_i64())
            .unwrap_or(default)
    };
    let predictor = read_i64("Predictor", 1);
    if predictor <= 1 {
        return None;
    }
    let cols = read_i64("Columns", 1).clamp(1, 1_000_000) as usize;
    let bpc = read_i64("BitsPerComponent", 8).clamp(1, 32) as usize;
    let colors = read_i64("Colors", 1).clamp(1, 64) as usize;
    let bpp = colors.saturating_mul(bpc).div_ceil(8).max(1);
    let row_bytes = cols.saturating_mul(colors).saturating_mul(bpc).div_ceil(8);
    if row_bytes == 0 || row_bytes > MAX_DECOMPRESSED_STREAM_BYTES {
        return None;
    }

    if predictor == 2 {
        if data.len() < row_bytes {
            return None;
        }
        if bpc == 8 {
            let mut out = Vec::with_capacity(data.len());
            for chunk in data.chunks_exact(row_bytes) {
                let mut row = chunk.to_vec();
                for i in bpp..row_bytes {
                    row[i] = row[i].wrapping_add(row[i - bpp]);
                }
                out.extend_from_slice(&row);
            }
            return Some(out);
        } else if bpc == 16 && row_bytes >= colors * 2 {
            let mut out = Vec::with_capacity(data.len());
            for chunk in data.chunks_exact(row_bytes) {
                let mut row = chunk.to_vec();
                for i in (colors * 2..row_bytes).step_by(2) {
                    let cur = u16::from_be_bytes([row[i], row[i + 1]]);
                    let prev = u16::from_be_bytes([row[i - colors * 2], row[i - colors * 2 + 1]]);
                    let sum = cur.wrapping_add(prev).to_be_bytes();
                    row[i] = sum[0];
                    row[i + 1] = sum[1];
                }
                out.extend_from_slice(&row);
            }
            return Some(out);
        }
        return None;
    }

    if predictor < 10 {
        return None;
    }
    let stride = row_bytes.checked_add(1)?;
    if stride <= 1 || data.len() < 2 {
        return None;
    }
    let mut out = Vec::with_capacity((data.len().div_ceil(stride)) * row_bytes);
    let mut prev_row = vec![0u8; row_bytes];
    for chunk in data.chunks(stride) {
        if chunk.len() < 2 {
            break;
        }
        let filter_type = chunk[0];
        let src = &chunk[1..];
        let row_len = src.len();
        let mut row = vec![0u8; row_bytes];
        for i in 0..row_len {
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
        out.extend_from_slice(&row[..row_len]);
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

pub(crate) fn decode_ascii85(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut group: Vec<u64> = Vec::with_capacity(5);
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
            group.push((b - b'!') as u64);
            if group.len() == 5 {
                let val = group
                    .iter()
                    .fold(0u64, |acc, &d| acc * 85 + d)
                    .min(u32::MAX as u64) as u32;
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
        let val = group
            .iter()
            .fold(0u64, |acc, &d| acc * 85 + d)
            .min(u32::MAX as u64) as u32;
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
