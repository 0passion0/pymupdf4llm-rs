use std::collections::HashMap;

/// Fixed-size CMap codespace range (`1..=4` bytes) avoiding per-range heap allocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodespaceRange {
    pub start: [u8; 4],
    pub end: [u8; 4],
    pub len: u8,
}

impl CodespaceRange {
    pub fn from_hex(start_hex: &str, end_hex: &str) -> Option<Self> {
        let s_bytes = decode_hex_bytes(start_hex)?;
        let e_bytes = decode_hex_bytes(end_hex)?;
        if s_bytes.is_empty() || s_bytes.len() != e_bytes.len() || s_bytes.len() > 4 {
            return None;
        }
        let mut start = [0u8; 4];
        let mut end = [0u8; 4];
        start[..s_bytes.len()].copy_from_slice(&s_bytes);
        end[..e_bytes.len()].copy_from_slice(&e_bytes);
        Some(Self {
            start,
            end,
            len: s_bytes.len() as u8,
        })
    }

    #[inline]
    pub fn matches(&self, input: &[u8]) -> Option<u32> {
        let len = self.len as usize;
        if input.len() < len || len == 0 {
            return None;
        }
        let mut code = 0u32;
        for idx in 0..len {
            let b = input[idx];
            if b < self.start[idx] || b > self.end[idx] {
                return None;
            }
            code = (code << 8) | (b as u32);
        }
        Some(code)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ParsedToUnicodeCMap {
    pub codespaces: Vec<CodespaceRange>,
    pub mappings: HashMap<u32, String>,
    pub sized_mappings: HashMap<(u8, u32), String>,
}

impl ParsedToUnicodeCMap {
    /// Looks up a Unicode mapping preferring an exact `(byte_len, code)` match before
    /// falling back to the legacy byte-length-agnostic table.
    #[inline]
    pub fn lookup(&self, byte_len: u8, code: u32) -> Option<&str> {
        self.sized_mappings
            .get(&(byte_len, code))
            .or_else(|| self.mappings.get(&code))
            .map(String::as_str)
    }

    #[inline]
    fn insert_mapping(&mut self, byte_len: u8, code: u32, mapped: String) {
        if byte_len > 0 {
            self.sized_mappings.insert((byte_len, code), mapped.clone());
        }
        self.mappings.insert(code, mapped);
    }
}

#[inline]
fn hex_byte_len(hex: &str) -> u8 {
    let digits = hex.bytes().filter(|b| b.is_ascii_hexdigit()).count();
    digits.div_ceil(2).clamp(1, 4) as u8
}

/// Token-based Adobe `/ToUnicode` CMap parser supporting `codespacerange`,
/// multi-pair/single-line `beginbfchar`, and multiline `[ ... ]` `beginbfrange`.
pub fn parse_tounicode_cmap(bytes: &[u8]) -> ParsedToUnicodeCMap {
    let mut parsed = ParsedToUnicodeCMap::default();
    let tokens = tokenize_cmap(bytes);
    let mut i = 0usize;

    while i < tokens.len() {
        match &tokens[i] {
            CMapToken::Word(w) if w == "begincodespacerange" => {
                i += 1;
                while i < tokens.len() {
                    if matches!(&tokens[i], CMapToken::Word(w) if w == "endcodespacerange") {
                        i += 1;
                        break;
                    }
                    if i + 1 < tokens.len() {
                        if let (CMapToken::Hex(s), CMapToken::Hex(e)) = (&tokens[i], &tokens[i + 1])
                        {
                            if let Some(range) = CodespaceRange::from_hex(s, e) {
                                parsed.codespaces.push(range);
                            }
                            i += 2;
                            continue;
                        }
                    }
                    i += 1;
                }
            }
            CMapToken::Word(w) if w == "beginbfchar" => {
                i += 1;
                while i < tokens.len() {
                    if matches!(&tokens[i], CMapToken::Word(w) if w == "endbfchar") {
                        i += 1;
                        break;
                    }
                    if i + 1 < tokens.len() {
                        if let (CMapToken::Hex(src), CMapToken::Hex(dst)) =
                            (&tokens[i], &tokens[i + 1])
                        {
                            if let (Ok(code), Some(mapped)) =
                                (u32::from_str_radix(src, 16), decode_cmap_utf16be(dst))
                            {
                                parsed.insert_mapping(hex_byte_len(src), code, mapped);
                            }
                            i += 2;
                            continue;
                        }
                    }
                    i += 1;
                }
            }
            CMapToken::Word(w) if w == "beginbfrange" => {
                i += 1;
                while i < tokens.len() {
                    if matches!(&tokens[i], CMapToken::Word(w) if w == "endbfrange") {
                        i += 1;
                        break;
                    }
                    if i + 2 < tokens.len() {
                        if let (CMapToken::Hex(start_hex), CMapToken::Hex(end_hex)) =
                            (&tokens[i], &tokens[i + 1])
                        {
                            if let (Ok(start), Ok(end)) = (
                                u32::from_str_radix(start_hex, 16),
                                u32::from_str_radix(end_hex, 16),
                            ) {
                                let code_len = hex_byte_len(start_hex);
                                match &tokens[i + 2] {
                                    CMapToken::LBracket => {
                                        i += 3;
                                        let mut idx = 0u32;
                                        while i < tokens.len() {
                                            match &tokens[i] {
                                                CMapToken::RBracket => {
                                                    i += 1;
                                                    break;
                                                }
                                                CMapToken::Hex(dst_hex) => {
                                                    let code = start.saturating_add(idx);
                                                    if code <= end {
                                                        if let Some(mapped) =
                                                            decode_cmap_utf16be(dst_hex)
                                                        {
                                                            parsed.insert_mapping(
                                                                code_len, code, mapped,
                                                            );
                                                        }
                                                    }
                                                    idx = idx.saturating_add(1);
                                                    i += 1;
                                                }
                                                _ => i += 1,
                                            }
                                        }
                                        continue;
                                    }
                                    CMapToken::Hex(dst_hex) => {
                                        insert_bfrange_sequential(
                                            code_len,
                                            start,
                                            end,
                                            dst_hex,
                                            &mut parsed,
                                        );
                                        i += 3;
                                        continue;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }

    parsed.codespaces.sort_by_key(|cs| cs.len);
    parsed
}

#[derive(Debug, Clone)]
enum CMapToken {
    Hex(String),
    LBracket,
    RBracket,
    Word(String),
}

fn tokenize_cmap(bytes: &[u8]) -> Vec<CMapToken> {
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() || b == 0 {
            i += 1;
            continue;
        }
        if b == b'%' {
            while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
                i += 1;
            }
            continue;
        }
        match b {
            b'(' => {
                // Skip PostScript literal string inside CMap headers (e.g. `(Adobe)`)
                i += 1;
                let mut depth = 1usize;
                while i < bytes.len() && depth > 0 {
                    match bytes[i] {
                        b'\\' => i = (i + 2).min(bytes.len()),
                        b'(' => {
                            depth += 1;
                            i += 1;
                        }
                        b')' => {
                            depth -= 1;
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
            }
            b'<' if bytes.get(i + 1) == Some(&b'<') => {
                // Skip PostScript dictionary `<< ... >>` (e.g. `/CIDSystemInfo << ... >>`)
                i += 2;
                let mut dict_depth = 1usize;
                while i < bytes.len() && dict_depth > 0 {
                    match bytes[i] {
                        b'(' => {
                            i += 1;
                            let mut str_depth = 1usize;
                            while i < bytes.len() && str_depth > 0 {
                                match bytes[i] {
                                    b'\\' => i = (i + 2).min(bytes.len()),
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
                        b'<' if bytes.get(i + 1) == Some(&b'<') => {
                            dict_depth += 1;
                            i += 2;
                        }
                        b'>' if bytes.get(i + 1) == Some(&b'>') => {
                            dict_depth -= 1;
                            i += 2;
                        }
                        _ => i += 1,
                    }
                }
            }
            b'<' => {
                i += 1;
                let mut hex = String::new();
                while i < bytes.len() && bytes[i] != b'>' {
                    if bytes[i].is_ascii_hexdigit() {
                        hex.push(bytes[i] as char);
                    }
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
                if !hex.is_empty() {
                    tokens.push(CMapToken::Hex(hex));
                }
            }
            b'[' => {
                tokens.push(CMapToken::LBracket);
                i += 1;
            }
            b']' => {
                tokens.push(CMapToken::RBracket);
                i += 1;
            }
            _ => {
                let start = i;
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && !matches!(bytes[i], b'<' | b'>' | b'[' | b']' | b'(' | b')' | b'%')
                {
                    i += 1;
                }
                let word = String::from_utf8_lossy(&bytes[start..i]).into_owned();
                if !word.is_empty() {
                    tokens.push(CMapToken::Word(word));
                } else {
                    i += 1;
                }
            }
        }
    }
    tokens
}

fn insert_bfrange_sequential(
    code_len: u8,
    start: u32,
    end: u32,
    dst_hex: &str,
    out: &mut ParsedToUnicodeCMap,
) {
    let clean: String = dst_hex.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if clean.len() < 2 {
        return;
    }
    let capped_end = end.min(start.saturating_add(4096));

    // Decode destination hex as UTF-16BE code units so single BMP scalars, UTF-16
    // surrogate pairs, AND multi-character ligatures (e.g. `<00660069>` -> "fi") all work.
    let mut base_units: Vec<u16> = Vec::with_capacity(clean.len().div_ceil(4));
    if clean.len() >= 4 && clean.len() % 4 == 0 {
        for chunk in clean.as_bytes().chunks_exact(4) {
            if let Ok(s) = std::str::from_utf8(chunk) {
                if let Ok(u) = u16::from_str_radix(s, 16) {
                    base_units.push(u);
                }
            }
        }
    } else if let Ok(v) = u32::from_str_radix(&clean, 16) {
        if let Ok(u) = u16::try_from(v) {
            base_units.push(u);
        } else if let Some(ch) = char::from_u32(v) {
            let mut buf = [0u16; 2];
            base_units.extend_from_slice(ch.encode_utf16(&mut buf));
        }
    }
    if base_units.is_empty() {
        return;
    }

    let is_surrogate_pair = base_units.len() == 2
        && (0xD800..=0xDBFF).contains(&base_units[0])
        && (0xDC00..=0xDFFF).contains(&base_units[1]);
    let last_idx = base_units.len() - 1;
    let last_start = base_units[last_idx];

    for (offset, code) in (start..=capped_end).enumerate() {
        let incremented = last_start.wrapping_add(offset as u16);
        if is_surrogate_pair && !(0xDC00..=0xDFFF).contains(&incremented) {
            break;
        }
        base_units[last_idx] = incremented;
        let decoded = String::from_utf16_lossy(&base_units);
        if !decoded.is_empty() && !decoded.contains('\u{FFFD}') {
            out.insert_mapping(code_len, code, decoded);
        }
    }
}

fn decode_hex_bytes(hex: &str) -> Option<Vec<u8>> {
    let clean: Vec<u8> = hex.bytes().filter(|b| b.is_ascii_hexdigit()).collect();
    if clean.is_empty() || clean.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(clean.len() / 2);
    for pair in clean.chunks_exact(2) {
        let s = std::str::from_utf8(pair).ok()?;
        out.push(u8::from_str_radix(s, 16).ok()?);
    }
    Some(out)
}

pub fn decode_cmap_utf16be(hex: &str) -> Option<String> {
    let clean: String = hex.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if clean.len() < 2 {
        return None;
    }
    let mut units = Vec::with_capacity(clean.len() / 4);
    let mut i = 0;
    while i + 3 < clean.len() {
        if let Ok(u) = u16::from_str_radix(&clean[i..i + 4], 16) {
            units.push(u);
        }
        i += 4;
    }
    if units.is_empty() {
        if let Ok(v) = u32::from_str_radix(&clean, 16) {
            return char::from_u32(v).map(|c| c.to_string());
        }
    }
    Some(String::from_utf16_lossy(&units))
}
