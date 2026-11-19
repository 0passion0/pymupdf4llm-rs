use super::resources::normalize_actual_text;
use crate::pdf::objects::PdfObject;
use crate::pdf::parser::lexer::decode_pdf_name_bytes;
use crate::pdf::parser::Lexer;

const MAX_TOKEN_ARRAY_DEPTH: usize = 32;

#[derive(Debug, Clone)]
pub(super) enum StreamToken {
    Number(f32),
    Name(String),
    String(Vec<u8>),
    Array(Vec<StreamToken>),
    Dict { actual_text: Option<String> },
    Operator(String),
}

pub(super) fn tokenize_content_stream(data: &[u8]) -> Vec<StreamToken> {
    tokenize_content_stream_inner(data, 0)
}

fn tokenize_content_stream_inner(data: &[u8], depth: usize) -> Vec<StreamToken> {
    if depth > MAX_TOKEN_ARRAY_DEPTH {
        return Vec::new();
    }
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
                let mut paren_depth = 1usize;
                while i < data.len() && paren_depth > 0 {
                    let c = data[i];
                    i += 1;
                    match c {
                        b'(' => {
                            paren_depth += 1;
                            s.push(b'(');
                        }
                        b')' => {
                            paren_depth -= 1;
                            if paren_depth > 0 {
                                s.push(b')');
                            }
                        }
                        b'\\' if i < data.len() => {
                            let esc = data[i];
                            i += 1;
                            match esc {
                                b'0'..=b'7' => {
                                    let mut val = (esc - b'0') as u16;
                                    for _ in 0..2 {
                                        if i < data.len() && (b'0'..=b'7').contains(&data[i]) {
                                            val = val * 8 + (data[i] - b'0') as u16;
                                            i += 1;
                                        } else {
                                            break;
                                        }
                                    }
                                    s.push((val & 0xFF) as u8);
                                }
                                b'n' => s.push(b'\n'),
                                b'r' => s.push(b'\r'),
                                b't' => s.push(b'\t'),
                                b'b' => s.push(0x08),
                                b'f' => s.push(0x0C),
                                b'\r' => {
                                    if i < data.len() && data[i] == b'\n' {
                                        i += 1;
                                    }
                                }
                                b'\n' => {}
                                other => s.push(other),
                            }
                        }
                        other => s.push(other),
                    }
                }
                tokens.push(StreamToken::String(s));
            }
            b'<' if data.get(i + 1) == Some(&b'<') => {
                let dict_start = i;
                skip_inline_dict(data, &mut i);
                let actual_text = extract_actual_text_from_inline_dict(&data[dict_start..i]);
                tokens.push(StreamToken::Dict { actual_text });
            }
            b'<' => {
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
                    let sub = tokenize_content_stream_inner(&data[i..sub_end], depth + 1);
                    arr.extend(sub);
                    i = sub_end;
                }
                tokens.push(StreamToken::Array(arr));
            }
            b'/' => {
                i += 1;
                let mut raw_name = Vec::new();
                while i < data.len()
                    && !data[i].is_ascii_whitespace()
                    && !matches!(
                        data[i],
                        b'/' | b'[' | b']' | b'(' | b')' | b'<' | b'>' | b'{' | b'}' | b'%'
                    )
                {
                    if data[i] == b'#' && i + 2 < data.len() {
                        if let Ok(s) = std::str::from_utf8(&data[i + 1..i + 3]) {
                            if let Ok(v) = u8::from_str_radix(s, 16) {
                                raw_name.push(v);
                                i += 3;
                                continue;
                            }
                        }
                    }
                    raw_name.push(data[i]);
                    i += 1;
                }
                tokens.push(StreamToken::Name(decode_pdf_name_bytes(raw_name)));
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                let start = i;
                i += 1;
                while i < data.len() && (data[i].is_ascii_digit() || data[i] == b'.') {
                    i += 1;
                }
                // Tolerate non-compliant scientific notation inside content streams (e.g. `1e-5`)
                if i < data.len() && matches!(data[i], b'e' | b'E') {
                    let mut exp_i = i + 1;
                    if matches!(data.get(exp_i), Some(b'+' | b'-')) {
                        exp_i += 1;
                    }
                    if data.get(exp_i).is_some_and(|c| c.is_ascii_digit()) {
                        while data.get(exp_i).is_some_and(|c| c.is_ascii_digit()) {
                            exp_i += 1;
                        }
                        i = exp_i;
                    }
                }
                let v = std::str::from_utf8(&data[start..i])
                    .ok()
                    .and_then(|s| s.parse::<f32>().ok())
                    .filter(|v| v.is_finite())
                    .unwrap_or(0.0);
                tokens.push(StreamToken::Number(v));
            }
            _ => {
                let start = i;
                while i < data.len()
                    && !data[i].is_ascii_whitespace()
                    && !matches!(
                        data[i],
                        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'/' | b'{' | b'}' | b'%'
                    )
                {
                    i += 1;
                }
                let op = String::from_utf8_lossy(&data[start..i]).into_owned();
                if op == "BI" && skip_inline_image(data, &mut i) {
                    continue;
                }
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

fn extract_actual_text_from_inline_dict(slice: &[u8]) -> Option<String> {
    if !slice.windows(11).any(|w| w == b"/ActualText") {
        return None;
    }
    let mut lexer = Lexer::new(slice, 0);
    let Ok(PdfObject::Dict(dict)) = lexer.parse_object() else {
        return None;
    };
    dict.get("ActualText")
        .and_then(|v| v.as_text_string())
        .and_then(normalize_actual_text)
}

fn skip_inline_dict(data: &[u8], cursor: &mut usize) {
    let mut i = *cursor + 2;
    let mut dict_depth = 1usize;
    while i < data.len() && dict_depth > 0 {
        match data[i] {
            b'(' => {
                i += 1;
                let mut str_depth = 1usize;
                while i < data.len() && str_depth > 0 {
                    match data[i] {
                        b'\\' => i = (i + 2).min(data.len()),
                        b'(' => {
                            str_depth += 1;
                            i += 1;
                        }
                        b')' => {
                            str_depth -= 1;
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
            }
            b'<' if data.get(i + 1) == Some(&b'<') => {
                dict_depth += 1;
                i += 2;
            }
            b'>' if data.get(i + 1) == Some(&b'>') => {
                dict_depth -= 1;
                i += 2;
            }
            _ => i += 1,
        }
    }
    *cursor = i;
}

fn skip_inline_image(data: &[u8], cursor: &mut usize) -> bool {
    let header_end = (*cursor + 4096).min(data.len());
    let mut pos = *cursor;
    let mut img_start = None;
    while pos + 2 <= header_end {
        if data[pos] == b'I'
            && data[pos + 1] == b'D'
            && (pos == *cursor || data[pos - 1].is_ascii_whitespace() || data[pos - 1] == b']')
            && data.get(pos + 2).is_some_and(|b| b.is_ascii_whitespace())
        {
            let mut s = pos + 3;
            if data.get(pos + 2) == Some(&b'\r') && data.get(pos + 3) == Some(&b'\n') {
                s += 1;
            }
            img_start = Some(s.min(data.len()));
            break;
        }
        pos += 1;
    }
    let Some(start) = img_start else {
        return false;
    };
    let mut p = start;
    while p + 2 <= data.len() {
        if data[p] == b'E'
            && data[p + 1] == b'I'
            && (p == start || data[p - 1].is_ascii_whitespace())
            && data.get(p + 2).map_or(true, |b| b.is_ascii_whitespace())
        {
            // Verify following non-whitespace byte (if any) is printable ASCII to avoid
            // matching accidental `EI` sequences inside binary inline image payloads.
            let mut look = p + 2;
            while look < data.len() && data[look].is_ascii_whitespace() {
                look += 1;
            }
            let valid_follow = data.get(look).map_or(true, |&b| (0x21..=0x7E).contains(&b));
            if valid_follow {
                *cursor = p + 2;
                return true;
            }
        }
        p += 1;
    }
    false
}

fn find_array_elem_end(data: &[u8], start: usize) -> usize {
    let b = data[start];
    if b == b'(' {
        let mut i = start + 1;
        let mut depth = 1usize;
        while i < data.len() && depth > 0 {
            if data[i] == b'\\' {
                i = (i + 2).min(data.len());
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
    } else if b == b'[' {
        let mut i = start + 1;
        let mut depth = 1usize;
        while i < data.len() && depth > 0 {
            if data[i] == b'[' {
                depth += 1;
            } else if data[i] == b']' {
                depth -= 1;
            }
            i += 1;
        }
        i
    } else {
        let mut i = start;
        while i < data.len()
            && !data[i].is_ascii_whitespace()
            && data[i] != b']'
            && data[i] != b'('
            && data[i] != b'<'
        {
            i += 1;
        }
        i.max(start + 1)
    }
}
