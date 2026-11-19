mod cmap;
mod glyph_names;
mod tables;

pub use cmap::{decode_cmap_utf16be, parse_tounicode_cmap, CodespaceRange, ParsedToUnicodeCMap};
pub use glyph_names::glyph_name_to_unicode;
pub use tables::{
    macroman_byte_to_char, pdfdoc_byte_to_char, symbol_code_to_str, winansi_byte_to_char,
};

/// PDF simple-font base encoding per ISO 32000-1 Annex D (plus legacy TeX OT1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BaseEncoding {
    #[default]
    WinAnsi,
    MacRoman,
    MacExpert,
    PdfDoc,
    Standard,
    Symbol,
    ZapfDingbats,
    Ot1TeX,
}

impl BaseEncoding {
    pub fn from_pdf_name(name: &str) -> Self {
        match name {
            "MacRomanEncoding" => Self::MacRoman,
            "MacExpertEncoding" => Self::MacExpert,
            "PDFDocEncoding" => Self::PdfDoc,
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
            Self::MacExpert => tables::macexpert_byte_to_str(b).map(ToString::to_string),
            Self::PdfDoc => pdfdoc_byte_to_char(b).map(|c| c.to_string()),
            Self::Standard => tables::standard_byte_to_str(b).map(ToString::to_string),
            Self::Symbol => {
                if let Some(s) = symbol_code_to_str(b as u32) {
                    Some(s.to_string())
                } else if (0x20..=0x7E).contains(&b) {
                    Some((b as char).to_string())
                } else {
                    None
                }
            }
            Self::ZapfDingbats => tables::zapf_dingbats_byte_to_str(b).map(ToString::to_string),
            Self::Ot1TeX => tables::ot1_tex_byte_to_str(b).map(ToString::to_string),
        }
    }
}

/// Predefined Type0 CJK / Unicode CMap families per ISO 32000-1 §9.7.5.2 & PDF 2.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PredefinedCMap {
    #[default]
    None,
    /// PDF 2.0 UTF-8 CMaps (`UniGB-UTF8-*`, `UniCNS-UTF8-*`, `UniJIS-UTF8-*`, `UniKS-UTF8-*`).
    Utf8,
    /// Unicode UTF-16BE / UCS-2 CMaps (`UniGB-UTF16-*`, `UniGB-UCS2-*`, `Adobe-*-UCS2`, etc.).
    Utf16Be,
    /// Legacy variable-width (1-byte ASCII + 2-byte CJK) CMaps (`GBK-EUC-*`, `90ms-RKSJ-*`, `B5pc-*`, etc.).
    EucVariable,
}

impl PredefinedCMap {
    pub fn from_encoding_name(name: &str) -> Self {
        if name.is_empty() {
            return Self::None;
        }
        let upper = name.to_ascii_uppercase();
        if upper.contains("UTF8") || upper.contains("UTF-8") {
            Self::Utf8
        } else if upper.contains("UTF16")
            || upper.contains("UCS2")
            || upper.starts_with("UNIGB-")
            || upper.starts_with("UNICNS-")
            || upper.starts_with("UNIJIS-")
            || upper.starts_with("UNIKS-")
        {
            Self::Utf16Be
        } else if upper.contains("GBK")
            || upper.contains("GB-EUC")
            || upper.contains("GBPC-EUC")
            || upper.contains("RKSJ")
            || upper.contains("EUC-H")
            || upper.contains("EUC-V")
            || upper.contains("B5PC")
            || upper.contains("ETEN-B5")
            || upper.contains("KSCMS-UHC")
        {
            Self::EucVariable
        } else {
            Self::None
        }
    }
}
