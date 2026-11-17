use std::collections::HashMap;
use crate::pdf::objects::PdfObject;
use crate::pdf::parser::decode_stream_bytes;
use crate::pdf::PdfDocument;

#[derive(Debug, Clone)]
pub struct PdfFont {
    pub base_font: String,
    pub is_bold: bool,
    pub is_italic: bool,
    pub is_monospace: bool,
    pub is_two_byte: bool,
    pub is_mac_roman: bool,
    pub to_unicode: HashMap<u32, String>,
    pub differences: HashMap<u8, String>,
    pub widths: HashMap<u32, f32>,
    pub default_width: f32,
}

impl Default for PdfFont {
    fn default() -> Self {
        Self {
            base_font: "Helvetica".to_string(),
            is_bold: false,
            is_italic: false,
            is_monospace: false,
            is_two_byte: false,
            is_mac_roman: false,
            to_unicode: HashMap::new(),
            differences: HashMap::new(),
            widths: HashMap::new(),
            default_width: 550.0,
        }
    }
}

impl PdfFont {
    pub fn from_dict(doc: &PdfDocument, font_obj: &PdfObject) -> Self {
        if let PdfObject::Reference(obj_num, _) = font_obj {
            if let Some(cached) = doc.font_cache.get(obj_num) {
                return cached.clone();
            }
        }
        let parsed = Self::parse_uncached(doc, font_obj);
        if let PdfObject::Reference(obj_num, _) = font_obj {
            doc.font_cache.insert(*obj_num, parsed.clone());
        }
        parsed
    }

    fn parse_uncached(doc: &PdfDocument, font_obj: &PdfObject) -> Self {
        let Some(dict) = doc.resolve(font_obj).and_then(|o| o.as_dict()) else {
            return Self::default();
        };

        let base_font = dict
            .get("BaseFont")
            .and_then(|v| v.as_name())
            .unwrap_or("Helvetica")
            .to_string();
        let lower = base_font.to_ascii_lowercase();
        let subtype = dict.get("Subtype").and_then(|v| v.as_name()).unwrap_or("");
        let enc_name = dict.get("Encoding").and_then(|v| v.as_name()).unwrap_or("");
        let is_two_byte = subtype == "Type0"
            || enc_name.contains("Identity-H")
            || enc_name.contains("Identity-V")
            || enc_name.contains("UCS2")
            || enc_name.contains("UTF16");
        let is_mac_roman = enc_name == "MacRomanEncoding";

        let mut is_bold = lower.contains("bold") || lower.contains("black") || lower.contains("heavy") || lower.contains("semibold");
        let mut is_italic = lower.contains("italic") || lower.contains("oblique") || lower.contains("slant");
        let mut is_monospace = lower.contains("courier")
            || lower.contains("mono")
            || lower.contains("consolas")
            || lower.contains("menlo")
            || lower.contains("inconsolata")
            || lower.contains("cmtt")
            || (lower.contains("fira") && lower.contains("code"));

        if let Some(desc_obj) = dict.get("FontDescriptor") {
            if let Some(desc) = doc.resolve(desc_obj).and_then(|o| o.as_dict()) {
                let flags = desc.get("Flags").and_then(|v| v.as_i64()).unwrap_or(0);
                if (flags & 1) != 0 {
                    is_monospace = true;
                }
                if (flags & (1 << 6)) != 0 {
                    is_italic = true;
                }
                if (flags & (1 << 18)) != 0 {
                    is_bold = true;
                }
            }
        }

        let mut to_unicode = HashMap::new();
        if let Some(tu_obj) = dict.get("ToUnicode") {
            if let Some(PdfObject::Stream { dict: sdict, data }) = doc.resolve(tu_obj) {
                if let Ok(cmap_bytes) = decode_stream_bytes(sdict, data) {
                    parse_tounicode_cmap(&cmap_bytes, &mut to_unicode);
                }
            }
        }

        let mut differences = HashMap::new();
        if let Some(enc_obj) = dict.get("Encoding") {
            if let Some(enc_dict) = doc.resolve(enc_obj).and_then(|o| o.as_dict()) {
                if let Some(diffs) = enc_dict.get("Differences").and_then(|v| v.as_array()) {
                    let mut code: u8 = 0;
                    for item in diffs {
                        match item {
                            PdfObject::Int(n) => code = (*n).clamp(0, 255) as u8,
                            PdfObject::Name(name) => {
                                if let Some(ch) = glyph_name_to_unicode(name) {
                                    differences.insert(code, ch.to_string());
                                }
                                code = code.wrapping_add(1);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        let mut widths = HashMap::new();
        let mut default_width = if is_monospace { 600.0 } else if is_two_byte { 1000.0 } else { 520.0 };

        let first_char = dict.get("FirstChar").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
        if let Some(arr) = dict.get("Widths").and_then(|o| doc.resolve(o)).and_then(|v| v.as_array()) {
            for (idx, w) in arr.iter().enumerate() {
                if let Some(val) = w.as_f32() {
                    widths.insert(first_char + idx as u32, val);
                }
            }
        }

        // Parse Type0 /DescendantFonts CID widths (/DW & /W)
        if let Some(descendants) = dict.get("DescendantFonts").and_then(|o| doc.resolve(o)).and_then(|v| v.as_array()) {
            if let Some(cid_dict) = descendants.first().and_then(|o| doc.resolve(o)).and_then(|v| v.as_dict()) {
                if let Some(dw) = cid_dict.get("DW").and_then(|v| v.as_f32()) {
                    default_width = dw;
                }
                if let Some(w_arr) = cid_dict.get("W").and_then(|o| doc.resolve(o)).and_then(|v| v.as_array()) {
                    parse_cid_widths(w_arr, &mut widths);
                }
            }
        }

        Self {
            base_font,
            is_bold,
            is_italic,
            is_monospace,
            is_two_byte,
            is_mac_roman,
            to_unicode,
            differences,
            widths,
            default_width,
        }
    }

    pub fn decode_bytes(&self, bytes: &[u8]) -> (String, f32) {
        let mut out = String::new();
        let mut total_width = 0.0f32;
        if self.is_two_byte && bytes.len() >= 2 {
            let mut i = 0;
            while i + 1 < bytes.len() {
                let code = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as u32;
                if let Some(mapped) = self.to_unicode.get(&code) {
                    out.push_str(mapped);
                } else if let Some(ch) = char::from_u32(code) {
                    if !ch.is_control() || ch == '\n' || ch == '\t' {
                        out.push(ch);
                    }
                }
                total_width += self.widths.get(&code).copied().unwrap_or(self.default_width);
                i += 2;
            }
        } else {
            for &b in bytes {
                let code = b as u32;
                if let Some(mapped) = self.to_unicode.get(&code) {
                    out.push_str(mapped);
                } else if let Some(diff) = self.differences.get(&b) {
                    out.push_str(diff);
                } else if self.is_mac_roman {
                    out.push(macroman_byte_to_char(b));
                } else {
                    out.push(winansi_byte_to_char(b));
                }
                total_width += self.widths.get(&code).copied().unwrap_or(self.default_width);
            }
        }
        (out, total_width)
    }
}

fn parse_cid_widths(arr: &[PdfObject], out: &mut HashMap<u32, f32>) {
    let mut i = 0;
    while i < arr.len() {
        let Some(c_start) = arr[i].as_i64() else {
            i += 1;
            continue;
        };
        if i + 1 >= arr.len() {
            break;
        }
        match &arr[i + 1] {
            PdfObject::Array(widths) => {
                for (offset, w) in widths.iter().enumerate() {
                    if let Some(val) = w.as_f32() {
                        out.insert((c_start as u32) + offset as u32, val);
                    }
                }
                i += 2;
            }
            PdfObject::Int(c_end) => {
                if i + 2 < arr.len() {
                    if let Some(w) = arr[i + 2].as_f32() {
                        for cid in (c_start as u32)..=(*c_end as u32).min(c_start as u32 + 4096) {
                            out.insert(cid, w);
                        }
                    }
                    i += 3;
                } else {
                    break;
                }
            }
            _ => i += 1,
        }
    }
}

fn parse_tounicode_cmap(bytes: &[u8], out: &mut HashMap<u32, String>) {
    let text = String::from_utf8_lossy(bytes);
    let mut in_bfchar = false;
    let mut in_bfrange = false;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.ends_with("beginbfchar") {
            in_bfchar = true;
            continue;
        }
        if line == "endbfchar" {
            in_bfchar = false;
            continue;
        }
        if line.ends_with("beginbfrange") {
            in_bfrange = true;
            continue;
        }
        if line == "endbfrange" {
            in_bfrange = false;
            continue;
        }

        if in_bfchar {
            let tokens = extract_hex_tokens(line);
            for pair in tokens.chunks_exact(2) {
                if let (Ok(code), Some(dst)) = (
                    u32::from_str_radix(&pair[0], 16),
                    decode_cmap_utf16be(&pair[1]),
                ) {
                    out.insert(code, dst);
                }
            }
        } else if in_bfrange {
            let tokens = extract_hex_tokens(line);
            if line.contains('[') && tokens.len() >= 3 {
                if let (Ok(start), Ok(end)) = (
                    u32::from_str_radix(&tokens[0], 16),
                    u32::from_str_radix(&tokens[1], 16),
                ) {
                    for (idx, hex_dst) in tokens[2..].iter().enumerate() {
                        let code = start + idx as u32;
                        if code > end {
                            break;
                        }
                        if let Some(dst) = decode_cmap_utf16be(hex_dst) {
                            out.insert(code, dst);
                        }
                    }
                }
            } else if tokens.len() >= 3 {
                if let (Ok(start), Ok(end), Ok(mut dst_code)) = (
                    u32::from_str_radix(&tokens[0], 16),
                    u32::from_str_radix(&tokens[1], 16),
                    u32::from_str_radix(&tokens[2], 16),
                ) {
                    for code in start..=end.min(start + 2048) {
                        if let Some(ch) = char::from_u32(dst_code) {
                            out.insert(code, ch.to_string());
                        }
                        dst_code += 1;
                    }
                }
            }
        }
    }
}

fn extract_hex_tokens(line: &str) -> Vec<String> {
    let mut res = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('<') {
        if let Some(close) = rest[open + 1..].find('>') {
            res.push(rest[open + 1..open + 1 + close].trim().to_string());
            rest = &rest[open + 1 + close + 1..];
        } else {
            break;
        }
    }
    res
}

fn decode_cmap_utf16be(hex: &str) -> Option<String> {
    let clean: String = hex.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if clean.len() < 2 {
        return None;
    }
    let mut units = Vec::new();
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

fn glyph_name_to_unicode(name: &str) -> Option<&'static str> {
    match name {
        "space" | "nbspace" => Some(" "),
        "exclam" => Some("!"),
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
        "hyphen" | "minus" => Some("-"),
        "period" => Some("."),
        "slash" => Some("/"),
        "colon" => Some(":"),
        "semicolon" => Some(";"),
        "less" => Some("<"),
        "equal" => Some("="),
        "greater" => Some(">"),
        "question" => Some("?"),
        "at" => Some("@"),
        "bracketleft" => Some("["),
        "backslash" => Some("\\"),
        "bracketright" => Some("]"),
        "underscore" => Some("_"),
        "bullet" => Some("•"),
        "endash" => Some("–"),
        "emdash" => Some("—"),
        "quotedblleft" => Some("“"),
        "quotedblright" => Some("”"),
        "quoteleft" => Some("‘"),
        "quoteright" => Some("’"),
        "guillemotleft" => Some("«"),
        "guillemotright" => Some("»"),
        "ellipsis" => Some("…"),
        "fi" => Some("fi"),
        "fl" => Some("fl"),
        "ff" => Some("ff"),
        "ffi" => Some("ffi"),
        "ffl" => Some("ffl"),
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
        "periodcentered" => Some("·"),
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
        "arrowleft" => Some("←"),
        "arrowright" => Some("→"),
        "arrowup" => Some("↑"),
        "arrowdown" => Some("↓"),
        "checkmark" => Some("✓"),
        _ => None,
    }
}

fn winansi_byte_to_char(b: u8) -> char {
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

fn macroman_byte_to_char(b: u8) -> char {
    match b {
        0xA5 => '•',
        0xAA => '™',
        0xA8 => '®',
        0xA9 => '©',
        0xC9 => '…',
        0xD0 => '–',
        0xD1 => '—',
        0xD2 => '“',
        0xD3 => '”',
        0xD4 => '‘',
        0xD5 => '’',
        0xDB => '€',
        _ => b as char,
    }
}
