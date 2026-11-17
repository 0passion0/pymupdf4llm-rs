use std::collections::HashMap;

/// PDF simple-font base encoding per ISO 32000-1 Annex D.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BaseEncoding {
    #[default]
    WinAnsi,
    MacRoman,
    Standard,
    Symbol,
    ZapfDingbats,
}

impl BaseEncoding {
    pub fn from_pdf_name(name: &str) -> Self {
        match name {
            "MacRomanEncoding" => Self::MacRoman,
            "StandardEncoding" => Self::Standard,
            "SymbolEncoding" => Self::Symbol,
            "ZapfDingbatsEncoding" => Self::ZapfDingbats,
            _ => Self::WinAnsi,
        }
    }

    pub fn decode_byte(self, b: u8) -> Option<String> {
        match self {
            Self::WinAnsi => Some(winansi_byte_to_char(b).to_string()),
            Self::MacRoman => Some(macroman_byte_to_char(b).to_string()),
            Self::Standard => standard_byte_to_str(b).map(ToString::to_string),
            Self::Symbol => {
                if let Some(s) = symbol_code_to_str(b as u32) {
                    Some(s.to_string())
                } else if (0x20..=0x7E).contains(&b) {
                    Some((b as char).to_string())
                } else {
                    None
                }
            }
            Self::ZapfDingbats => zapf_dingbats_byte_to_str(b).map(ToString::to_string),
        }
    }
}

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
                        if let (CMapToken::Hex(s), CMapToken::Hex(e)) =
                            (&tokens[i], &tokens[i + 1])
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
                                parsed.mappings.insert(code, mapped);
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
                                                            parsed.mappings.insert(code, mapped);
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
                                            start,
                                            end,
                                            dst_hex,
                                            &mut parsed.mappings,
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
                    && !matches!(bytes[i], b'<' | b'>' | b'[' | b']' | b'%')
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
    start: u32,
    end: u32,
    dst_hex: &str,
    out: &mut HashMap<u32, String>,
) {
    let clean: String = dst_hex.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if clean.len() < 2 {
        return;
    }
    let capped_end = end.min(start.saturating_add(4096));
    // Handle UTF-16BE surrogate pairs (8 hex digits) as well as single BMP code units
    if clean.len() == 8 {
        if let (Ok(hi), Ok(lo_start)) = (
            u16::from_str_radix(&clean[0..4], 16),
            u16::from_str_radix(&clean[4..8], 16),
        ) {
            if (0xD800..=0xDBFF).contains(&hi) && (0xDC00..=0xDFFF).contains(&lo_start) {
                for (offset, code) in (start..=capped_end).enumerate() {
                    let lo = lo_start.wrapping_add(offset as u16);
                    if (0xDC00..=0xDFFF).contains(&lo) {
                        let s = String::from_utf16_lossy(&[hi, lo]);
                        if !s.is_empty() {
                            out.insert(code, s);
                        }
                    }
                }
                return;
            }
        }
    }
    if let Ok(mut dst_code) = u32::from_str_radix(&clean, 16) {
        for code in start..=capped_end {
            if let Some(ch) = char::from_u32(dst_code) {
                out.insert(code, ch.to_string());
            }
            dst_code = dst_code.saturating_add(1);
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

/// Decodes an Adobe / Type1C / Advent / Dingbats glyph name into Unicode.
pub fn glyph_name_to_unicode(name: &str, base_font: &str) -> Option<String> {
    if let Some(s) = static_glyph_name_to_unicode(name) {
        return Some(s.to_string());
    }

    // Single-character ASCII glyph names (e.g. /A, /a, /0)
    if name.len() == 1 {
        let b = name.as_bytes()[0];
        if b.is_ascii_alphanumeric() {
            return Some((b as char).to_string());
        }
    }

    // Adobe Glyph List `uniXXXX` (4-hex-digit sequences) and `uXXXXX` scalar syntax
    if let Some(hex) = name.strip_prefix("uni") {
        if hex.len() >= 4 && hex.len() % 4 == 0 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            let mut units = Vec::with_capacity(hex.len() / 4);
            let mut valid = true;
            for chunk in hex.as_bytes().chunks_exact(4) {
                if let Ok(s) = std::str::from_utf8(chunk) {
                    if let Ok(u) = u16::from_str_radix(s, 16) {
                        units.push(u);
                        continue;
                    }
                }
                valid = false;
                break;
            }
            if valid {
                let decoded = String::from_utf16_lossy(&units);
                if !decoded.is_empty() && !decoded.chars().all(|c| c.is_control()) {
                    return Some(decoded);
                }
            }
        }
    }
    if let Some(hex) = name.strip_prefix('u') {
        if (4..=6).contains(&hex.len()) && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            if let Ok(cp) = u32::from_str_radix(hex, 16) {
                if cp >= 0x20 {
                    if let Some(ch) = char::from_u32(cp) {
                        if !ch.is_control() {
                            return Some(ch.to_string());
                        }
                    }
                }
            }
        }
    }

    // Embedded Type1C / Advent 3B2 / TeX subset `/C<decimal>` glyph names
    if let Some(num_str) = name.strip_prefix('C') {
        if !num_str.is_empty() && num_str.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(code) = num_str.parse::<u32>() {
                return decode_numeric_c_glyph(code, base_font);
            }
        }
    }

    // ZapfDingbats `/a<decimal>` glyph names (checkmarks, bullets, stars, arrows)
    if let Some(num_str) = name.strip_prefix('a') {
        if !num_str.is_empty() && num_str.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(code) = num_str.parse::<u32>() {
                if let Some(ch) = zapf_dingbats_a_glyph(code) {
                    return Some(ch.to_string());
                }
            }
        }
    }

    // Variant suffix stripping per Adobe Glyph List spec (e.g. `fi.liga`, `A.sc`)
    if !name.starts_with('.') {
        if let Some((base, _variant)) = name.split_once('.') {
            if !base.is_empty() {
                return glyph_name_to_unicode(base, base_font);
            }
        }
    }

    // Compound ligature names separated by `_` (e.g. `T_h`, `uni0066_uni0069`)
    if name.contains('_') && !name.starts_with('_') && !name.ends_with('_') {
        let mut combined = String::new();
        let mut all_ok = true;
        for part in name.split('_') {
            if part.is_empty() {
                all_ok = false;
                break;
            }
            if let Some(sub) = glyph_name_to_unicode(part, base_font) {
                combined.push_str(&sub);
            } else {
                all_ok = false;
                break;
            }
        }
        if all_ok && !combined.is_empty() {
            return Some(combined);
        }
    }

    None
}

fn decode_numeric_c_glyph(code: u32, base_font: &str) -> Option<String> {
    // Font names with `+<HH>` high-byte Unicode page suffix (e.g. `AdvTT5843c571+20` + `/C19` -> U+2013 '–')
    if code < 256 {
        if let Some((_, suffix)) = base_font.rsplit_once('+') {
            if suffix.len() == 2 && suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
                if let Ok(hi) = u32::from_str_radix(suffix, 16) {
                    if hi > 0 {
                        if let Some(ch) = char::from_u32((hi << 8) | code) {
                            if !ch.is_control() {
                                return Some(ch.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    let lower = base_font.to_ascii_lowercase();
    if lower.contains("advpi") || lower == "symbol" || lower.ends_with("+symbol") {
        if let Some(sym) = symbol_code_to_str(code) {
            return Some(sym.to_string());
        }
    }
    if lower.contains("advt001") && code == 223 {
        return Some("©".to_string());
    }

    if (32..=126).contains(&code) {
        return Some((code as u8 as char).to_string());
    }

    if let Some(std_ch) = adobe_standard_high_to_str(code) {
        return Some(std_ch.to_string());
    }

    if code >= 256 {
        if let Some(ch) = char::from_u32(code) {
            if !ch.is_control() {
                return Some(ch.to_string());
            }
        }
    }

    None
}

fn zapf_dingbats_a_glyph(code: u32) -> Option<&'static str> {
    match code {
        1 => Some("✁"),
        2 => Some("✂"),
        3 => Some("✃"),
        4 => Some("✄"),
        19 => Some("✓"),
        20 => Some("✔"),
        21 => Some("✕"),
        22 => Some("✖"),
        23 => Some("✗"),
        24 => Some("✘"),
        35 => Some("★"),
        36 => Some("☆"),
        71 => Some("●"),
        72 => Some("❍"),
        73 => Some("■"),
        74 => Some("❏"),
        75 => Some("❐"),
        76 => Some("❑"),
        77 => Some("❒"),
        108 => Some("●"),
        109 => Some("❍"),
        110 => Some("■"),
        160 => Some("→"),
        161 => Some("➔"),
        165 => Some("➜"),
        _ => None,
    }
}

fn zapf_dingbats_byte_to_str(b: u8) -> Option<&'static str> {
    match b {
        0x20 => Some(" "),
        0x21 => Some("✁"),
        0x22 => Some("✂"),
        0x23 => Some("✃"),
        0x24 => Some("✄"),
        0x25 => Some("☎"),
        0x26 => Some("✆"),
        0x27 => Some("✇"),
        0x28 => Some("✈"),
        0x29 => Some("✉"),
        0x2A => Some("☛"),
        0x2B => Some("☞"),
        0x2C => Some("✌"),
        0x2D => Some("✍"),
        0x2E => Some("✎"),
        0x2F => Some("✏"),
        0x33 => Some("✔"),
        0x34 => Some("✔"),
        0x35 => Some("✕"),
        0x36 => Some("✖"),
        0x37 => Some("✗"),
        0x38 => Some("✘"),
        0x48 => Some("★"),
        0x49 => Some("☆"),
        0x6C => Some("●"),
        0x6D => Some("❍"),
        0x6E => Some("■"),
        0x6F => Some("❏"),
        0x70 => Some("❐"),
        0x71 => Some("❑"),
        0x72 => Some("❒"),
        0x73 => Some("▲"),
        0x74 => Some("▼"),
        0x75 => Some("◆"),
        0xD4 => Some("→"),
        0xD5 => Some("➔"),
        0xD9 => Some("➜"),
        _ => None,
    }
}

pub fn symbol_code_to_str(code: u32) -> Option<&'static str> {
    match code {
        32..=64 => None,
        65 => Some("Α"),
        66 => Some("Β"),
        67 => Some("Χ"),
        68 => Some("Δ"),
        69 => Some("Ε"),
        70 => Some("Φ"),
        71 => Some("Γ"),
        72 => Some("Η"),
        73 => Some("Ι"),
        74 => Some("ϑ"),
        75 => Some("Κ"),
        76 => Some("Λ"),
        77 => Some("Μ"),
        78 => Some("Ν"),
        79 => Some("Ο"),
        80 => Some("Π"),
        81 => Some("Θ"),
        82 => Some("Ρ"),
        83 => Some("Σ"),
        84 => Some("Τ"),
        85 => Some("Υ"),
        86 => Some("ς"),
        87 => Some("Ω"),
        88 => Some("Ξ"),
        89 => Some("Ψ"),
        90 => Some("Ζ"),
        97 => Some("α"),
        98 => Some("β"),
        99 => Some("χ"),
        100 => Some("δ"),
        101 => Some("ε"),
        102 => Some("φ"),
        103 => Some("γ"),
        104 => Some("η"),
        105 => Some("ι"),
        106 => Some("ϕ"),
        107 => Some("κ"),
        108 => Some("λ"),
        109 => Some("µ"),
        110 => Some("ν"),
        111 => Some("ο"),
        112 => Some("π"),
        113 => Some("θ"),
        114 => Some("ρ"),
        115 => Some("σ"),
        116 => Some("τ"),
        117 => Some("υ"),
        118 => Some("ϖ"),
        119 => Some("ω"),
        120 => Some("ξ"),
        121 => Some("ψ"),
        122 => Some("ζ"),
        161 => Some("ϒ"),
        162 => Some("′"),
        163 => Some("≤"),
        164 => Some("⁄"),
        165 => Some("∞"),
        166 => Some("ƒ"),
        167 => Some("♣"),
        168 => Some("♦"),
        169 => Some("♥"),
        170 => Some("♠"),
        171 => Some("↔"),
        172 => Some("←"),
        173 => Some("↑"),
        174 => Some("→"),
        175 => Some("↓"),
        176 => Some("°"),
        177 => Some("±"),
        178 => Some("″"),
        179 => Some("≥"),
        180 => Some("×"),
        181 => Some("∝"),
        182 => Some("∂"),
        183 => Some("•"),
        184 => Some("÷"),
        185 => Some("≠"),
        186 => Some("≡"),
        187 => Some("≈"),
        188 => Some("…"),
        196 => Some("⊗"),
        197 => Some("⊕"),
        198 => Some("∅"),
        199 => Some("∩"),
        200 => Some("∪"),
        201 => Some("⊃"),
        202 => Some("⊇"),
        203 => Some("⊄"),
        204 => Some("⊂"),
        205 => Some("⊆"),
        206 => Some("∈"),
        207 => Some("∉"),
        208 => Some("∠"),
        209 => Some("∇"),
        210 => Some("®"),
        211 => Some("©"),
        212 => Some("™"),
        213 => Some("∏"),
        214 => Some("√"),
        215 => Some("·"),
        216 => Some("¬"),
        217 => Some("∧"),
        218 => Some("∨"),
        219 => Some("⇔"),
        220 => Some("⇐"),
        221 => Some("⇑"),
        222 => Some("⇒"),
        223 => Some("⇓"),
        224 => Some("◊"),
        229 => Some("∑"),
        242 => Some("∫"),
        _ => None,
    }
}

fn adobe_standard_high_to_str(code: u32) -> Option<&'static str> {
    match code {
        161 => Some("¡"),
        162 => Some("¢"),
        163 => Some("£"),
        164 => Some("/"),
        165 => Some("¥"),
        166 => Some("ƒ"),
        167 => Some("§"),
        168 => Some("¤"),
        169 => Some("'"),
        170 => Some("“"),
        171 => Some("«"),
        172 => Some("‹"),
        173 => Some("›"),
        174 => Some("fi"),
        175 => Some("fl"),
        177 => Some("–"),
        178 => Some("†"),
        179 => Some("‡"),
        180 => Some("·"),
        182 => Some("¶"),
        183 => Some("•"),
        184 => Some("‚"),
        185 => Some("„"),
        186 => Some("”"),
        187 => Some("»"),
        188 => Some("…"),
        189 => Some("‰"),
        191 => Some("¿"),
        193 => Some("\u{0300}"),
        194 => Some("\u{0301}"),
        195 => Some("\u{0302}"),
        196 => Some("\u{0303}"),
        197 => Some("\u{0304}"),
        198 => Some("\u{0306}"),
        199 => Some("\u{0307}"),
        200 => Some("\u{0308}"),
        202 => Some("\u{030A}"),
        203 => Some("\u{0327}"),
        205 => Some("\u{030B}"),
        206 => Some("\u{0328}"),
        207 => Some("\u{030C}"),
        208 => Some("—"),
        225 => Some("Æ"),
        227 => Some("ª"),
        232 => Some("Ł"),
        233 => Some("Ø"),
        234 => Some("Œ"),
        235 => Some("º"),
        241 => Some("æ"),
        245 => Some("ı"),
        248 => Some("ł"),
        249 => Some("ø"),
        250 => Some("œ"),
        251 => Some("ß"),
        _ => None,
    }
}

fn standard_byte_to_str(b: u8) -> Option<&'static str> {
    match b {
        0x27 => Some("’"),
        0x60 => Some("‘"),
        0x20..=0x7E => None,
        0xA1..=0xFF => adobe_standard_high_to_str(b as u32),
        _ => None,
    }
    .or_else(|| {
        if (0x20..=0x7E).contains(&b) {
            // Static ASCII slice via byte table lookup
            ASCII_SINGLE_STRS.get((b - 0x20) as usize).copied()
        } else {
            None
        }
    })
}

const ASCII_SINGLE_STRS: [&str; 95] = [
    " ", "!", "\"", "#", "$", "%", "&", "'", "(", ")", "*", "+", ",", "-", ".", "/",
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", ":", ";", "<", "=", ">", "?",
    "@", "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O",
    "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "[", "\\", "]", "^", "_",
    "`", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o",
    "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z", "{", "|", "}", "~",
];

fn static_glyph_name_to_unicode(name: &str) -> Option<&'static str> {
    match name {
        "space" | "nbspace" | "nonbreakingspace" => Some(" "),
        "exclam" => Some("!"),
        "exclamdown" => Some("¡"),
        "quotedbl" => Some("\""),
        "numbersign" => Some("#"),
        "dollar" => Some("$"),
        "percent" => Some("%"),
        "ampersand" => Some("&"),
        "quotesingle" => Some("'"),
        "parenleft" => Some("("),
        "parenright" => Some(")"),
        "asterisk" => Some("*"),
        "plus" => Some("+"),
        "comma" => Some(","),
        "hyphen" | "minus" | "sfthyphen" | "hyphensoft" | "nonbreakinghyphen" => Some("-"),
        "period" => Some("."),
        "slash" | "fraction" => Some("/"),
        "zero" | "zerooldstyle" => Some("0"),
        "one" | "oneoldstyle" => Some("1"),
        "two" | "twooldstyle" => Some("2"),
        "three" | "threeoldstyle" => Some("3"),
        "four" | "fourextra" | "fouroldstyle" => Some("4"),
        "five" | "fiveoldstyle" => Some("5"),
        "six" | "sixoldstyle" => Some("6"),
        "seven" | "sevenoldstyle" => Some("7"),
        "eight" | "eightoldstyle" => Some("8"),
        "nine" | "nineoldstyle" => Some("9"),
        "colon" => Some(":"),
        "semicolon" => Some(";"),
        "less" => Some("<"),
        "equal" => Some("="),
        "greater" => Some(">"),
        "question" => Some("?"),
        "questiondown" => Some("¿"),
        "at" => Some("@"),
        "bracketleft" => Some("["),
        "backslash" => Some("\\"),
        "bracketright" => Some("]"),
        "asciicircum" => Some("^"),
        "underscore" => Some("_"),
        "grave" => Some("`"),
        "braceleft" => Some("{"),
        "bar" => Some("|"),
        "braceright" => Some("}"),
        "asciitilde" => Some("~"),
        "bullet" | "bulletoperator" => Some("•"),
        "endash" | "figuredash" => Some("–"),
        "emdash" => Some("—"),
        "quotedblleft" => Some("“"),
        "quotedblright" => Some("”"),
        "quotedblbase" => Some("„"),
        "quoteleft" => Some("‘"),
        "quoteright" => Some("’"),
        "quotesinglbase" => Some("‚"),
        "guillemotleft" => Some("«"),
        "guillemotright" => Some("»"),
        "guilsinglleft" => Some("‹"),
        "guilsinglright" => Some("›"),
        "ellipsis" => Some("…"),
        "fi" | "f_i" => Some("fi"),
        "fl" | "f_l" => Some("fl"),
        "ff" | "f_f" => Some("ff"),
        "ffi" | "f_f_i" => Some("ffi"),
        "ffl" | "f_f_l" => Some("ffl"),
        "Euro" => Some("€"),
        "sterling" => Some("£"),
        "yen" => Some("¥"),
        "cent" => Some("¢"),
        "section" => Some("§"),
        "paragraph" => Some("¶"),
        "degree" => Some("°"),
        "plusminus" => Some("±"),
        "multiply" => Some("×"),
        "divide" => Some("÷"),
        "periodcentered" | "middot" => Some("·"),
        "copyright" => Some("©"),
        "registered" => Some("®"),
        "trademark" => Some("™"),
        "dagger" => Some("†"),
        "daggerdbl" => Some("‡"),
        "perthousand" => Some("‰"),
        "lessequal" => Some("≤"),
        "greaterequal" => Some("≥"),
        "notequal" => Some("≠"),
        "approxequal" => Some("≈"),
        "infinity" => Some("∞"),
        "partialdiff" => Some("∂"),
        "summation" => Some("∑"),
        "product" => Some("∏"),
        "radical" => Some("√"),
        "integral" => Some("∫"),
        "arrowleft" => Some("←"),
        "arrowright" => Some("→"),
        "arrowup" => Some("↑"),
        "arrowdown" => Some("↓"),
        "checkmark" => Some("✓"),
        "dieresis" => Some("¨"),
        "Aacute" => Some("Á"), "aacute" => Some("á"),
        "Agrave" => Some("À"), "agrave" => Some("à"),
        "Acircumflex" => Some("Â"), "acircumflex" => Some("â"),
        "Adieresis" => Some("Ä"), "adieresis" => Some("ä"),
        "Atilde" => Some("Ã"), "atilde" => Some("ã"),
        "Aring" => Some("Å"), "aring" => Some("å"),
        "AE" => Some("Æ"), "ae" => Some("æ"),
        "Ccedilla" => Some("Ç"), "ccedilla" => Some("ç"),
        "Eacute" => Some("É"), "eacute" => Some("é"),
        "Egrave" => Some("È"), "egrave" => Some("è"),
        "Ecircumflex" => Some("Ê"), "ecircumflex" => Some("ê"),
        "Edieresis" => Some("Ë"), "edieresis" => Some("ë"),
        "Iacute" => Some("Í"), "iacute" => Some("í"),
        "Igrave" => Some("Ì"), "igrave" => Some("ì"),
        "Icircumflex" => Some("Î"), "icircumflex" => Some("î"),
        "Idieresis" => Some("Ï"), "idieresis" => Some("ï"),
        "Ntilde" => Some("Ñ"), "ntilde" => Some("ñ"),
        "Oacute" => Some("Ó"), "oacute" => Some("ó"),
        "Ograve" => Some("Ò"), "ograve" => Some("ò"),
        "Ocircumflex" => Some("Ô"), "ocircumflex" => Some("ô"),
        "Odieresis" => Some("Ö"), "odieresis" => Some("ö"),
        "Otilde" => Some("Õ"), "otilde" => Some("õ"),
        "Oslash" => Some("Ø"), "oslash" => Some("ø"),
        "OE" => Some("Œ"), "oe" => Some("œ"),
        "Uacute" => Some("Ú"), "uacute" => Some("ú"),
        "Ugrave" => Some("Ù"), "ugrave" => Some("ù"),
        "Ucircumflex" => Some("Û"), "ucircumflex" => Some("û"),
        "Udieresis" => Some("Ü"), "udieresis" => Some("ü"),
        "Yacute" => Some("Ý"), "yacute" => Some("ý"),
        "ydieresis" => Some("ÿ"), "Ydieresis" => Some("Ÿ"),
        "germandbls" => Some("ß"),
        "Lslash" => Some("Ł"), "lslash" => Some("ł"),
        "Scaron" => Some("Š"), "scaron" => Some("š"),
        "Zcaron" => Some("Ž"), "zcaron" => Some("ž"),
        "Eth" => Some("Ð"), "eth" => Some("ð"),
        "Thorn" => Some("Þ"), "thorn" => Some("þ"),
        "ordfeminine" => Some("ª"), "ordmasculine" => Some("º"),
        "onesuperior" => Some("¹"), "twosuperior" => Some("²"), "threesuperior" => Some("³"),
        "onequarter" => Some("¼"), "onehalf" => Some("½"), "threequarters" => Some("¾"),
        "brokenbar" => Some("¦"), "currency" => Some("¤"), "florin" => Some("ƒ"),
        "lozenge" => Some("◊"), "logicalnot" => Some("¬"), "macron" => Some("¯"),
        "prime" | "minute" => Some("′"),
        "second" => Some("″"),
        "equivalence" => Some("≡"), "propersubset" => Some("⊂"), "propersuperset" => Some("⊃"),
        "reflexsubset" => Some("⊆"), "reflexsuperset" => Some("⊇"), "notsubset" => Some("⊄"),
        "element" => Some("∈"), "notelement" => Some("∉"), "suchthat" => Some("∋"),
        "emptyset" => Some("∅"), "universal" => Some("∀"), "existential" => Some("∃"),
        "gradient" => Some("∇"), "angle" => Some("∠"), "perpendicular" => Some("⊥"),
        "intersection" => Some("∩"), "union" => Some("∪"), "proportional" => Some("∝"),
        "similar" => Some("∼"), "congruent" => Some("≅"), "therefore" => Some("∴"),
        "logicaland" => Some("∧"), "logicalor" => Some("∨"),
        "arrowboth" => Some("↔"), "carriagereturn" => Some("↵"),
        "arrowdblleft" => Some("⇐"), "arrowdblright" => Some("⇒"),
        "arrowdblup" => Some("⇑"), "arrowdbldown" => Some("⇓"), "arrowdblboth" => Some("⇔"),
        "circlemultiply" => Some("⊗"), "circleplus" => Some("⊕"),
        "asteriskmath" => Some("∗"), "dotmath" => Some("⋅"),
        "aleph" => Some("ℵ"), "Weierstrass" => Some("℘"),
        "Ifraktur" => Some("ℑ"), "Rfraktur" => Some("ℜ"),
        "club" => Some("♣"), "diamond" => Some("♦"), "heart" => Some("♥"), "spade" => Some("♠"),
        "H18533" => Some("●"),
        "H18543" => Some("○"),
        "H18551" => Some("▪"),
        "H18552" => Some("▫"),
        "Alpha" => Some("Α"), "Beta" => Some("Β"), "Gamma" => Some("Γ"),
        "Delta" => Some("Δ"), "Epsilon" => Some("Ε"), "Zeta" => Some("Ζ"),
        "Eta" => Some("Η"), "Theta" => Some("Θ"), "Iota" => Some("Ι"),
        "Kappa" => Some("Κ"), "Lambda" => Some("Λ"), "Mu" => Some("Μ"),
        "Nu" => Some("Ν"), "Xi" => Some("Ξ"), "Omicron" => Some("Ο"),
        "Pi" => Some("Π"), "Rho" => Some("Ρ"), "Sigma" => Some("Σ"),
        "Tau" => Some("Τ"), "Upsilon" => Some("Υ"), "Upsilon1" => Some("ϒ"),
        "Phi" => Some("Φ"), "Chi" => Some("Χ"), "Psi" => Some("Ψ"), "Omega" => Some("Ω"),
        "alpha" => Some("α"), "beta" => Some("β"), "gamma" => Some("γ"),
        "delta" => Some("δ"), "epsilon" => Some("ε"), "zeta" => Some("ζ"),
        "eta" => Some("η"), "theta" => Some("θ"), "theta1" => Some("ϑ"),
        "iota" => Some("ι"), "kappa" => Some("κ"), "lambda" => Some("λ"),
        "mu" | "mu1" | "micro" => Some("µ"), "nu" => Some("ν"), "xi" => Some("ξ"),
        "omicron" => Some("ο"), "pi" => Some("π"), "rho" => Some("ρ"),
        "sigma" => Some("σ"), "sigma1" => Some("ς"), "tau" => Some("τ"),
        "upsilon" => Some("υ"), "phi" => Some("φ"), "phi1" => Some("ϕ"),
        "chi" => Some("χ"), "psi" => Some("ψ"), "omega" => Some("ω"), "omega1" => Some("ϖ"),
        _ => None,
    }
}

pub fn winansi_byte_to_char(b: u8) -> char {
    match b {
        0x80 => '€',
        0x82 => '‚',
        0x83 => 'ƒ',
        0x84 => '„',
        0x85 => '…',
        0x86 => '†',
        0x87 => '‡',
        0x88 => 'ˆ',
        0x89 => '‰',
        0x8A => 'Š',
        0x8B => '‹',
        0x8C => 'Œ',
        0x8E => 'Ž',
        0x91 => '‘',
        0x92 => '’',
        0x93 => '“',
        0x94 => '”',
        0x95 => '•',
        0x96 => '–',
        0x97 => '—',
        0x98 => '˜',
        0x99 => '™',
        0x9A => 'š',
        0x9B => '›',
        0x9C => 'œ',
        0x9E => 'ž',
        0x9F => 'Ÿ',
        _ => b as char,
    }
}

const MACROMAN_HIGH: [char; 128] = [
    'Ä', 'Å', 'Ç', 'É', 'Ñ', 'Ö', 'Ü', 'á', 'à', 'â', 'ä', 'ã', 'å', 'ç', 'é', 'è',
    'ê', 'ë', 'í', 'ì', 'î', 'ï', 'ñ', 'ó', 'ò', 'ô', 'ö', 'õ', 'ú', 'ù', 'û', 'ü',
    '†', '°', '¢', '£', '§', '•', '¶', 'ß', '®', '©', '™', '´', '¨', '≠', 'Æ', 'Ø',
    '∞', '±', '≤', '≥', '¥', 'µ', '∂', '∑', '∏', 'π', '∫', 'ª', 'º', 'Ω', 'æ', 'ø',
    '¿', '¡', '¬', '√', 'ƒ', '≈', '∆', '«', '»', '…', '\u{00A0}', 'À', 'Ã', 'Õ', 'Œ', 'œ',
    '–', '—', '“', '”', '‘', '’', '÷', '◊', 'ÿ', 'Ÿ', '⁄', '€', '‹', '›', 'ﬁ', 'ﬂ',
    '‡', '·', '‚', '„', '‰', 'Â', 'Ê', 'Á', 'Ë', 'È', 'Í', 'Î', 'Ï', 'Ì', 'Ó', 'Ô',
    '\u{F8FF}', 'Ò', 'Ú', 'Û', 'Ù', 'ı', 'ˆ', '˜', '¯', '˘', '˙', '˚', '¸', '˝', '˛', 'ˇ',
];

pub fn macroman_byte_to_char(b: u8) -> char {
    if b >= 0x80 {
        MACROMAN_HIGH[(b - 0x80) as usize]
    } else {
        b as char
    }
}
