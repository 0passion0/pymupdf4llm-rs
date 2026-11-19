use std::collections::BTreeMap;

use crate::error::{PdfError, Result};
use crate::pdf::objects::PdfObject;

const MAX_PARSE_DEPTH: usize = 64;

fn is_nesting_depth_error(err: &PdfError) -> bool {
    matches!(
        err,
        PdfError::Syntax { message, .. } if message.starts_with("Maximum nesting depth exceeded")
    )
}

pub(crate) fn skip_ws_and_comments_slice(bytes: &[u8], pos: &mut usize) {
    while *pos < bytes.len() {
        let b = bytes[*pos];
        if b.is_ascii_whitespace() || b == 0 {
            *pos += 1;
        } else if b == b'%' {
            while *pos < bytes.len() && bytes[*pos] != b'\n' && bytes[*pos] != b'\r' {
                *pos += 1;
            }
        } else {
            break;
        }
    }
}

pub(crate) fn read_ascii_usize(bytes: &[u8], pos: &mut usize) -> Option<usize> {
    let start = *pos;
    while *pos < bytes.len() && bytes[*pos].is_ascii_digit() {
        *pos += 1;
    }
    if *pos == start {
        return None;
    }
    std::str::from_utf8(&bytes[start..*pos])
        .ok()?
        .parse::<usize>()
        .ok()
}

pub(crate) fn parse_indirect_object_at(
    bytes: &[u8],
    header_pos: usize,
    offset: usize,
) -> Option<(u32, u16, PdfObject)> {
    if let Some(pos) = header_pos.checked_add(offset) {
        if let Some(parsed) = parse_indirect_object_at_raw(bytes, pos) {
            return Some(parsed);
        }
    }
    if header_pos > 0 {
        return parse_indirect_object_at_raw(bytes, offset);
    }
    None
}

pub(crate) fn parse_indirect_object_at_raw(
    bytes: &[u8],
    start: usize,
) -> Option<(u32, u16, PdfObject)> {
    if start >= bytes.len() {
        return None;
    }
    let mut pos = start;
    skip_ws_and_comments_slice(bytes, &mut pos);
    let obj_num = read_ascii_usize(bytes, &mut pos)? as u32;
    skip_ws_and_comments_slice(bytes, &mut pos);
    let gen_num = u16::try_from(read_ascii_usize(bytes, &mut pos)?).ok()?;
    skip_ws_and_comments_slice(bytes, &mut pos);
    if !bytes[pos..].starts_with(b"obj") {
        return None;
    }
    pos += 3;
    if bytes.get(pos).is_some_and(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let mut lexer = Lexer::new(bytes, pos);
    let obj = lexer.parse_object().ok()?;
    Some((obj_num, gen_num, obj))
}

pub(crate) fn find_valid_classic_trailers(
    bytes: &[u8],
) -> Vec<(usize, BTreeMap<String, PdfObject>)> {
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

pub(crate) fn scan_next_indirect_object(
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
            || matches!(bytes[obj_kw + 3], b'<' | b'[' | b'(' | b'/');
        if !prev_ok || !next_ok {
            pos = obj_kw + 3;
            continue;
        }

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

fn has_endstream_near(data: &[u8], expected_end: usize) -> bool {
    let mut p = expected_end;
    let limit = (expected_end + 8).min(data.len());
    while p < limit && matches!(data[p], b'\r' | b'\n' | b' ' | b'\t') {
        p += 1;
    }
    data[p..].starts_with(b"endstream")
}

pub struct Lexer<'a> {
    pub data: &'a [u8],
    pub pos: usize,
    depth: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(data: &'a [u8], pos: usize) -> Self {
        Self {
            data,
            pos,
            depth: 0,
        }
    }

    pub fn skip_ws_and_comments(&mut self) {
        skip_ws_and_comments_slice(self.data, &mut self.pos);
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
                        if stream_start.saturating_add(len) <= self.data.len()
                            && has_endstream_near(self.data, stream_start + len)
                        {
                            stream_start + len
                        } else {
                            trim_eol = true;
                            find_subsequence(&self.data[stream_start..], b"endstream")
                                .or_else(|| find_subsequence(&self.data[stream_start..], b"endobj"))
                                .map(|p| stream_start + p)
                                .unwrap_or(self.data.len())
                        }
                    } else {
                        trim_eol = true;
                        find_subsequence(&self.data[stream_start..], b"endstream")
                            .or_else(|| find_subsequence(&self.data[stream_start..], b"endobj"))
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

    fn skip_stray_token(&mut self) {
        if self.pos >= self.data.len() {
            return;
        }
        let b = self.data[self.pos];
        if matches!(b, b'(' | b'<' | b'[' | b'/' | b'>' | b']' | b')') {
            self.pos += 1;
            return;
        }
        while self.pos < self.data.len() && !self.is_token_boundary(self.pos) {
            self.pos += 1;
        }
    }

    fn parse_dict(&mut self) -> Result<BTreeMap<String, PdfObject>> {
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(PdfError::Syntax {
                offset: self.pos,
                message: "Maximum nesting depth exceeded in dictionary".into(),
            });
        }
        self.depth += 1;
        let res = self.parse_dict_inner();
        self.depth -= 1;
        res
    }

    fn parse_dict_inner(&mut self) -> Result<BTreeMap<String, PdfObject>> {
        self.pos += 2; // <<
        let mut dict = BTreeMap::new();
        let mut stray_budget = 16usize;
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
            if self.data[self.pos..].starts_with(b"endobj")
                || self.data[self.pos..].starts_with(b"stream")
                || self.data[self.pos..].starts_with(b"endstream")
            {
                break;
            }
            if self.data[self.pos] != b'/' {
                if stray_budget == 0 {
                    break;
                }
                stray_budget -= 1;
                self.skip_stray_token();
                continue;
            }
            let PdfObject::Name(key) = self.parse_name()? else {
                break;
            };
            self.skip_ws_and_comments();
            if self.pos >= self.data.len()
                || (self.pos + 1 < self.data.len()
                    && self.data[self.pos] == b'>'
                    && self.data[self.pos + 1] == b'>')
            {
                break;
            }
            match self.parse_object() {
                Ok(val) => {
                    dict.insert(key, val);
                }
                Err(e) if is_nesting_depth_error(&e) => return Err(e),
                Err(_) => break,
            }
        }
        Ok(dict)
    }

    fn parse_array(&mut self) -> Result<PdfObject> {
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(PdfError::Syntax {
                offset: self.pos,
                message: "Maximum nesting depth exceeded in array".into(),
            });
        }
        self.depth += 1;
        let res = self.parse_array_inner();
        self.depth -= 1;
        res
    }

    fn parse_array_inner(&mut self) -> Result<PdfObject> {
        self.pos += 1; // [
        let mut items = Vec::new();
        let mut stray_budget = 16usize;
        loop {
            self.skip_ws_and_comments();
            if self.pos >= self.data.len() {
                break;
            }
            if self.data[self.pos] == b']' {
                self.pos += 1;
                break;
            }
            if (self.pos + 1 < self.data.len()
                && self.data[self.pos] == b'>'
                && self.data[self.pos + 1] == b'>')
                || self.data[self.pos..].starts_with(b"endobj")
                || self.data[self.pos..].starts_with(b"stream")
                || self.data[self.pos..].starts_with(b"endstream")
            {
                break;
            }
            match self.parse_object() {
                Ok(obj) => items.push(obj),
                Err(e) if is_nesting_depth_error(&e) => return Err(e),
                Err(_) => {
                    if stray_budget == 0 {
                        break;
                    }
                    stray_budget -= 1;
                    self.skip_stray_token();
                }
            }
        }
        Ok(PdfObject::Array(items))
    }

    fn parse_name(&mut self) -> Result<PdfObject> {
        self.pos += 1; // /
        let mut raw = Vec::new();
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
                        raw.push(v);
                        self.pos += 3;
                        continue;
                    }
                }
            }
            raw.push(b);
            self.pos += 1;
        }
        Ok(PdfObject::Name(decode_pdf_name_bytes(raw)))
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
        let mut has_exp = false;
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
        if self.pos < self.data.len() && matches!(self.data[self.pos], b'e' | b'E') {
            let mut exp_pos = self.pos + 1;
            if matches!(self.data.get(exp_pos), Some(b'+' | b'-')) {
                exp_pos += 1;
            }
            if self.data.get(exp_pos).is_some_and(|b| b.is_ascii_digit()) {
                has_exp = true;
                while self.data.get(exp_pos).is_some_and(|b| b.is_ascii_digit()) {
                    exp_pos += 1;
                }
                self.pos = exp_pos;
            }
        }

        let token = std::str::from_utf8(&self.data[start..self.pos]).unwrap_or("0");
        if has_dot || has_exp {
            let val = token
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .unwrap_or(0.0);
            return Ok(PdfObject::Real(val));
        }
        let int_val = token.parse::<i64>().unwrap_or(0);
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
                        .map_or(true, |b| !b.is_ascii_alphanumeric())
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

pub(crate) fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

pub(crate) fn rfind_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(haystack.len());
    }
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

/// Decodes PDF Name raw bytes (after `#XX` unescaping) as UTF-8 per ISO 32000-1 §7.3.5,
/// falling back losslessly to Latin-1 for legacy non-UTF-8 names.
pub(crate) fn decode_pdf_name_bytes(raw: Vec<u8>) -> String {
    match String::from_utf8(raw) {
        Ok(s) => s,
        Err(e) => e.into_bytes().into_iter().map(|b| b as char).collect(),
    }
}
