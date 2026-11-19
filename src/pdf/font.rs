use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use dashmap::DashMap;

use crate::pdf::encoding::{
    glyph_name_to_unicode, parse_tounicode_cmap, symbol_code_to_str, BaseEncoding,
    ParsedToUnicodeCMap, PredefinedCMap,
};
use crate::pdf::objects::PdfObject;
use crate::pdf::parser::decode_stream_bytes_with_objects;
use crate::pdf::PdfDocument;

const MAX_GLOBAL_CMAP_ENTRIES: usize = 512;

fn global_cmap_cache() -> &'static DashMap<(u64, usize), Arc<ParsedToUnicodeCMap>> {
    static CACHE: OnceLock<DashMap<(u64, usize), Arc<ParsedToUnicodeCMap>>> = OnceLock::new();
    CACHE.get_or_init(DashMap::new)
}

fn hash_cmap_bytes(bytes: &[u8]) -> (u64, usize) {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    (hasher.finish(), bytes.len())
}

#[derive(Debug, Clone)]
pub struct PdfFont {
    pub base_font: String,
    pub is_bold: bool,
    pub is_italic: bool,
    pub is_monospace: bool,
    pub is_two_byte: bool,
    pub base_encoding: BaseEncoding,
    pub predefined_cmap: PredefinedCMap,
    pub to_unicode: Arc<ParsedToUnicodeCMap>,
    pub differences: Arc<HashMap<u8, String>>,
    pub widths: Arc<HashMap<u32, f32>>,
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
            base_encoding: BaseEncoding::WinAnsi,
            predefined_cmap: PredefinedCMap::None,
            to_unicode: Arc::new(ParsedToUnicodeCMap::default()),
            differences: Arc::new(HashMap::new()),
            widths: Arc::new(HashMap::new()),
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
            .and_then(|v| doc.resolve(v))
            .and_then(|v| v.as_name())
            .unwrap_or("Helvetica")
            .to_string();
        let lower = base_font.to_ascii_lowercase();
        let subtype = dict
            .get("Subtype")
            .and_then(|v| doc.resolve(v))
            .and_then(|v| v.as_name())
            .unwrap_or("");
        let enc_obj_opt = dict.get("Encoding").and_then(|v| doc.resolve(v));
        let enc_name = enc_obj_opt
            .and_then(|o| o.as_name())
            .or_else(|| {
                enc_obj_opt
                    .and_then(|o| o.as_dict())
                    .and_then(|d| d.get("BaseEncoding"))
                    .and_then(|v| doc.resolve(v))
                    .and_then(|v| v.as_name())
            })
            .unwrap_or("");
        let predefined_cmap = PredefinedCMap::from_encoding_name(enc_name);
        let is_two_byte = subtype == "Type0"
            || enc_name.contains("Identity-H")
            || enc_name.contains("Identity-V")
            || predefined_cmap != PredefinedCMap::None
            || dict.contains_key("DescendantFonts");
        let base_encoding = if enc_name.is_empty() {
            if lower == "symbol" || lower.ends_with("+symbol") || lower.ends_with(",symbol") {
                BaseEncoding::Symbol
            } else if lower.contains("zapfdingbats") {
                BaseEncoding::ZapfDingbats
            } else if is_tex_ot1_font(&lower) {
                BaseEncoding::Ot1TeX
            } else {
                BaseEncoding::WinAnsi
            }
        } else {
            BaseEncoding::from_pdf_name(enc_name)
        };

        let mut is_bold = lower.contains("bold")
            || lower.contains("black")
            || lower.contains("heavy")
            || lower.contains("semibold")
            || lower.contains("demi")
            || lower.ends_with("-bd")
            || lower.contains("simhei")
            || base_font.contains("黑体")
            || base_font.contains("粗体")
            || base_font.contains("加粗")
            || base_font.contains("中粗");
        let mut is_italic = lower.contains("italic")
            || lower.contains("oblique")
            || lower.contains("slant")
            || base_font.contains("斜体");
        let mut is_monospace = lower.contains("courier")
            || lower.contains("mono")
            || lower.contains("consolas")
            || lower.contains("menlo")
            || lower.contains("inconsolata")
            || lower.contains("cmtt")
            || (lower.contains("fira") && lower.contains("code"))
            || base_font.contains("等宽");

        let cid_dict_opt = dict
            .get("DescendantFonts")
            .and_then(|o| doc.resolve(o))
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|o| doc.resolve(o))
            .and_then(|v| v.as_dict());

        // Per ISO 32000-1 §9.7, Type0 composite fonts place `/FontDescriptor` on the
        // descendant CIDFont dictionary rather than on the top-level Type0 dictionary.
        let mut missing_width: Option<f32> = None;
        let desc_obj_opt = dict
            .get("FontDescriptor")
            .or_else(|| cid_dict_opt.and_then(|cd| cd.get("FontDescriptor")));
        if let Some(desc_obj) = desc_obj_opt {
            if let Some(desc) = doc.resolve(desc_obj).and_then(|o| o.as_dict()) {
                let flags = desc
                    .get("Flags")
                    .and_then(|v| doc.resolve(v))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                if (flags & 1) != 0 {
                    is_monospace = true;
                }
                if (flags & (1 << 6)) != 0 {
                    is_italic = true;
                }
                if (flags & (1 << 18)) != 0 {
                    is_bold = true;
                }
                if let Some(weight) = desc
                    .get("FontWeight")
                    .and_then(|v| doc.resolve(v))
                    .and_then(|v| v.as_f32())
                {
                    if weight >= 600.0 {
                        is_bold = true;
                    }
                }
                if let Some(angle) = desc
                    .get("ItalicAngle")
                    .and_then(|v| doc.resolve(v))
                    .and_then(|v| v.as_f32())
                {
                    if angle.abs() > 1.0 {
                        is_italic = true;
                    }
                }
                if let Some(stem_v) = desc
                    .get("StemV")
                    .and_then(|v| doc.resolve(v))
                    .and_then(|v| v.as_f32())
                {
                    if stem_v >= 110.0 {
                        is_bold = true;
                    }
                }
                if let Some(mw) = desc
                    .get("MissingWidth")
                    .and_then(|v| doc.resolve(v))
                    .and_then(|v| v.as_f32())
                {
                    if mw > 0.0 {
                        missing_width = Some(mw);
                    }
                }
            }
        }

        let mut to_unicode: Option<Arc<ParsedToUnicodeCMap>> = None;
        let tu_obj_opt = dict
            .get("ToUnicode")
            .or_else(|| cid_dict_opt.and_then(|cd| cd.get("ToUnicode")));
        if let Some(tu_obj) = tu_obj_opt {
            if let Some(PdfObject::Stream { dict: sdict, data }) = doc.resolve(tu_obj) {
                if let Ok(cmap_bytes) = decode_stream_bytes_with_objects(&doc.objects, sdict, data)
                {
                    let key = hash_cmap_bytes(&cmap_bytes);
                    let cache = global_cmap_cache();
                    if let Some(hit) = cache.get(&key) {
                        to_unicode = Some(Arc::clone(hit.value()));
                    } else {
                        let parsed_cmap = parse_tounicode_cmap(&cmap_bytes);
                        let shared = Arc::new(parsed_cmap);
                        if cache.len() < MAX_GLOBAL_CMAP_ENTRIES {
                            cache.insert(key, Arc::clone(&shared));
                        }
                        to_unicode = Some(shared);
                    }
                }
            }
        }
        let to_unicode = to_unicode.unwrap_or_else(|| Arc::new(ParsedToUnicodeCMap::default()));

        let mut differences = HashMap::new();
        if let Some(enc_dict) = enc_obj_opt.and_then(|o| o.as_dict()) {
            if let Some(diffs) = enc_dict
                .get("Differences")
                .and_then(|v| doc.resolve(v))
                .and_then(|v| v.as_array())
            {
                let mut code: u8 = 0;
                for item in diffs {
                    match item {
                        PdfObject::Int(n) => code = (*n).clamp(0, 255) as u8,
                        PdfObject::Name(name) => {
                            if let Some(ch) = glyph_name_to_unicode(name, &base_font) {
                                differences.insert(code, ch);
                            }
                            code = code.wrapping_add(1);
                        }
                        _ => {}
                    }
                }
            }
        }

        // Type3 fonts store widths in their own `/FontMatrix` coordinate system rather than 1/1000 em
        let width_scale = if subtype == "Type3" {
            dict.get("FontMatrix")
                .and_then(|v| doc.resolve(v))
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_f32())
                .map(|sx| sx.abs() * 1000.0)
                .filter(|&s| s.is_finite() && s > 1e-4)
                .unwrap_or(1.0)
        } else {
            1.0
        };

        let mut widths = HashMap::new();
        let mut default_width = missing_width
            .filter(|w| w.is_finite() && *w > 0.0)
            .map(|w| (w * width_scale).clamp(1.0, 10_000.0))
            .unwrap_or(if is_monospace {
                600.0
            } else if is_two_byte {
                1000.0
            } else {
                520.0
            });

        let first_char = dict
            .get("FirstChar")
            .and_then(|v| doc.resolve(v))
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .clamp(0, u32::MAX as i64) as u32;
        if let Some(arr) = dict
            .get("Widths")
            .and_then(|o| doc.resolve(o))
            .and_then(|v| v.as_array())
        {
            for (idx, w) in arr.iter().enumerate() {
                if let Some(val) = w.as_f32() {
                    let scaled = val * width_scale;
                    if scaled.is_finite() {
                        widths.insert(
                            first_char.saturating_add(idx as u32),
                            scaled.clamp(0.0, 10_000.0),
                        );
                    }
                }
            }
        }

        // Parse Type0 /DescendantFonts CID widths (/DW & /W)
        if let Some(cid_dict) = cid_dict_opt {
            if let Some(dw) = cid_dict
                .get("DW")
                .and_then(|o| doc.resolve(o))
                .and_then(|v| v.as_f32())
            {
                if dw.is_finite() && dw > 0.0 {
                    default_width = dw.clamp(1.0, 10_000.0);
                }
            }
            if let Some(w_arr) = cid_dict
                .get("W")
                .and_then(|o| doc.resolve(o))
                .and_then(|v| v.as_array())
            {
                parse_cid_widths(w_arr, &mut widths);
            }
        }

        Self {
            base_font,
            is_bold,
            is_italic,
            is_monospace,
            is_two_byte,
            base_encoding,
            predefined_cmap,
            to_unicode,
            differences: Arc::new(differences),
            widths: Arc::new(widths),
            default_width,
        }
    }

    pub fn decode_bytes(&self, bytes: &[u8]) -> (String, f32) {
        let (out, total_width, _, _) = self.decode_bytes_with_stats(bytes);
        (out, total_width)
    }

    pub fn decode_bytes_with_stats(&self, bytes: &[u8]) -> (String, f32, usize, usize) {
        let mut out = String::new();
        let mut total_width = 0.0f32;
        let mut glyph_count = 0usize;
        let mut space_count = 0usize;

        // Simple fonts (Type1/MMType1/TrueType/Type3) always use single-byte codes in content
        // streams even when authoring tools (e.g. Adobe InDesign) emit `<0000> <FFFF>` in ToUnicode.
        if self.is_two_byte && !self.to_unicode.codespaces.is_empty() {
            let mut i = 0usize;
            while i < bytes.len() {
                let mut matched: Option<(u32, usize)> = None;
                for cs in &self.to_unicode.codespaces {
                    if let Some(code) = cs.matches(&bytes[i..]) {
                        matched = Some((code, cs.len as usize));
                        break;
                    }
                }
                let (code, step) = matched.unwrap_or_else(|| {
                    if self.predefined_cmap == PredefinedCMap::EucVariable && bytes[i] < 0x80 {
                        (bytes[i] as u32, 1)
                    } else if i + 1 < bytes.len() {
                        (u16::from_be_bytes([bytes[i], bytes[i + 1]]) as u32, 2)
                    } else {
                        (bytes[i] as u32, 1)
                    }
                });

                glyph_count += 1;
                if step == 1 && bytes[i] == b' ' {
                    space_count += 1;
                }

                if let Some(mapped) = self.to_unicode.lookup(step as u8, code) {
                    if let Some(norm) = self.normalize_tounicode_mapping(code, step, mapped) {
                        out.push_str(&norm);
                    } else if step == 1 {
                        self.decode_single_byte_fallback(bytes[i], &mut out);
                    }
                } else if step == 1 {
                    self.decode_single_byte_fallback(bytes[i], &mut out);
                } else if let Some(ch) = char::from_u32(code) {
                    if !ch.is_control() || ch == '\n' || ch == '\t' {
                        out.push(ch);
                    }
                }
                total_width += self
                    .widths
                    .get(&code)
                    .copied()
                    .unwrap_or(self.default_width);
                i += step.max(1);
            }
            return (out, total_width, glyph_count, space_count);
        }

        // PDF 2.0 UTF-8 predefined CMaps (e.g. `UniGB-UTF8-H`) without explicit ToUnicode
        if self.is_two_byte
            && self.predefined_cmap == PredefinedCMap::Utf8
            && self.to_unicode.mappings.is_empty()
        {
            let text = String::from_utf8_lossy(bytes);
            for ch in text.chars() {
                if !ch.is_control() || ch == '\n' || ch == '\t' {
                    out.push(ch);
                }
                glyph_count += 1;
                if ch == ' ' {
                    space_count += 1;
                }
                let cp = ch as u32;
                let fallback_w = if cp < 0x80 { 500.0 } else { self.default_width };
                total_width += self.widths.get(&cp).copied().unwrap_or(fallback_w);
            }
            return (out, total_width, glyph_count, space_count);
        }

        // Variable-width EUC/Shift-JIS/Big5 CMaps: keep 1-byte ASCII aligned cleanly
        if self.is_two_byte && self.predefined_cmap == PredefinedCMap::EucVariable {
            let mut i = 0usize;
            while i < bytes.len() {
                let b = bytes[i];
                if b < 0x80 {
                    let code = b as u32;
                    glyph_count += 1;
                    if b == b' ' {
                        space_count += 1;
                    }
                    if let Some(mapped) = self.to_unicode.lookup(1, code) {
                        if let Some(norm) = self.normalize_tounicode_mapping(code, 1, mapped) {
                            out.push_str(&norm);
                        }
                    } else if (0x20..=0x7E).contains(&b) || b == b'\n' || b == b'\t' {
                        out.push(b as char);
                    }
                    total_width += self.widths.get(&code).copied().unwrap_or(500.0);
                    i += 1;
                } else if i + 1 < bytes.len() {
                    let code = u16::from_be_bytes([b, bytes[i + 1]]) as u32;
                    glyph_count += 1;
                    if let Some(mapped) = self.to_unicode.lookup(2, code) {
                        if let Some(norm) = self.normalize_tounicode_mapping(code, 2, mapped) {
                            out.push_str(&norm);
                        }
                    }
                    total_width += self
                        .widths
                        .get(&code)
                        .copied()
                        .unwrap_or(self.default_width);
                    i += 2;
                } else {
                    break;
                }
            }
            return (out, total_width, glyph_count, space_count);
        }

        if self.is_two_byte && bytes.len() >= 2 {
            let mut i = 0;
            while i + 1 < bytes.len() {
                let code = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as u32;
                glyph_count += 1;
                if let Some(mapped) = self.to_unicode.lookup(2, code) {
                    if let Some(norm) = self.normalize_tounicode_mapping(code, 2, mapped) {
                        out.push_str(&norm);
                    }
                } else if (0xD800..=0xDBFF).contains(&code) && i + 3 < bytes.len() {
                    let lo = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]);
                    if (0xDC00..=0xDFFF).contains(&lo) {
                        let s = String::from_utf16_lossy(&[code as u16, lo]);
                        out.push_str(&s);
                        total_width += self
                            .widths
                            .get(&code)
                            .copied()
                            .unwrap_or(self.default_width);
                        i += 4;
                        continue;
                    }
                } else if let Some(ch) = char::from_u32(code) {
                    if !ch.is_control() || ch == '\n' || ch == '\t' {
                        out.push(ch);
                    }
                }
                total_width += self
                    .widths
                    .get(&code)
                    .copied()
                    .unwrap_or(self.default_width);
                i += 2;
            }
        } else {
            for &b in bytes {
                let code = b as u32;
                glyph_count += 1;
                if b == b' ' {
                    space_count += 1;
                }
                if let Some(mapped) = self.to_unicode.lookup(1, code) {
                    if let Some(norm) = self.normalize_tounicode_mapping(code, 1, mapped) {
                        out.push_str(&norm);
                    } else {
                        self.decode_single_byte_fallback(b, &mut out);
                    }
                } else {
                    self.decode_single_byte_fallback(b, &mut out);
                }
                total_width += self
                    .widths
                    .get(&code)
                    .copied()
                    .unwrap_or(self.default_width);
            }
        }
        (out, total_width, glyph_count, space_count)
    }

    fn normalize_tounicode_mapping(&self, code: u32, step: usize, mapped: &str) -> Option<String> {
        if mapped.is_empty()
            || matches!(
                mapped,
                "\u{0000}" | "\u{FFFD}" | "\u{FFFE}" | "\u{FFFF}" | "\u{FEFF}"
            )
        {
            return None;
        }
        let mut chars = mapped.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            let cp = ch as u32;
            if (0xE000..=0xF8FF).contains(&cp) {
                if (0xF020..=0xF0FF).contains(&cp) {
                    let lower = self.base_font.to_ascii_lowercase();
                    if lower.contains("symbol") || lower.contains("advpi") {
                        let byte_cp = cp & 0xFF;
                        if let Some(sym) = symbol_code_to_str(byte_cp) {
                            return Some(sym.to_string());
                        }
                        if (0x20..=0x7E).contains(&byte_cp) {
                            return Some((byte_cp as u8 as char).to_string());
                        }
                    }
                }
                match cp {
                    0xF001 => return Some("fi".to_string()),
                    0xF002 => return Some("fl".to_string()),
                    0xF020 => return Some(" ".to_string()),
                    0xF02D => return Some("-".to_string()),
                    0xF0B7 | 0xF076 => return Some("•".to_string()),
                    0xF0A7 => return Some("▪".to_string()),
                    0xF0FC | 0xF052 => return Some("✓".to_string()),
                    _ => {}
                }
                if step == 1 {
                    if let Some(diff) = self.differences.get(&(code as u8)) {
                        return Some(diff.clone());
                    }
                }
            }
        }
        Some(mapped.to_string())
    }

    #[inline]
    fn decode_single_byte_fallback(&self, b: u8, out: &mut String) {
        if let Some(diff) = self.differences.get(&b) {
            out.push_str(diff);
        } else if b < 0x20 && b != b'\n' && b != b'\t' && !self.differences.is_empty() {
            // Custom-remapped font with an unmapped control byte: drop cleanly instead of emitting mojibake
        } else if let Some(s) = self.base_encoding.decode_byte(b) {
            out.push_str(&s);
        }
    }
}

fn is_tex_ot1_font(lower_base_font: &str) -> bool {
    let core = lower_base_font
        .rsplit_once('+')
        .map(|(_, s)| s)
        .unwrap_or(lower_base_font);
    ["cmr", "cmbx", "cmti", "cmsl", "cmss", "cmcsc"]
        .iter()
        .any(|prefix| {
            core.starts_with(prefix)
                && core[prefix.len()..]
                    .bytes()
                    .next()
                    .is_some_and(|b| b.is_ascii_digit())
        })
}

fn parse_cid_widths(arr: &[PdfObject], out: &mut HashMap<u32, f32>) {
    let mut i = 0;
    while i < arr.len() {
        let Some(c_start_raw) = arr[i].as_i64() else {
            i += 1;
            continue;
        };
        if i + 1 >= arr.len() {
            break;
        }
        let c_start = c_start_raw.clamp(0, u32::MAX as i64) as u32;
        match &arr[i + 1] {
            PdfObject::Array(widths) => {
                for (offset, w) in widths.iter().enumerate() {
                    if let Some(val) = w.as_f32() {
                        if val.is_finite() {
                            out.insert(
                                c_start.saturating_add(offset as u32),
                                val.clamp(0.0, 10_000.0),
                            );
                        }
                    }
                }
                i += 2;
            }
            PdfObject::Int(c_end_raw) => {
                if i + 2 < arr.len() {
                    if let Some(w) = arr[i + 2].as_f32() {
                        if w.is_finite() {
                            let clamped = w.clamp(0.0, 10_000.0);
                            let c_end = (*c_end_raw).clamp(0, u32::MAX as i64) as u32;
                            if c_end >= c_start {
                                for cid in c_start..=c_end.min(c_start.saturating_add(4096)) {
                                    out.insert(cid, clamped);
                                }
                            }
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
