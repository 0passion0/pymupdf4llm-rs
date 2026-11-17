use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum PdfObject {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Name(String),
    String(Vec<u8>),
    Array(Vec<PdfObject>),
    Dict(BTreeMap<String, PdfObject>),
    Stream {
        dict: BTreeMap<String, PdfObject>,
        data: Vec<u8>,
    },
    Reference(u32, u16),
}

impl PdfObject {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            Self::Real(r) => Some(*r as i64),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Self::Int(i) => Some(*i as f32),
            Self::Real(r) => Some(*r as f32),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int(i) => Some(*i as f64),
            Self::Real(r) => Some(*r),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&str> {
        match self {
            Self::Name(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::String(b) => Some(b.as_slice()),
            _ => None,
        }
    }

    pub fn as_text_string(&self) -> Option<String> {
        let bytes = self.as_bytes()?;
        Some(decode_pdf_text_string(bytes))
    }

    pub fn as_array(&self) -> Option<&[PdfObject]> {
        match self {
            Self::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&BTreeMap<String, PdfObject>> {
        match self {
            Self::Dict(d) => Some(d),
            Self::Stream { dict, .. } => Some(dict),
            _ => None,
        }
    }

    pub fn as_reference(&self) -> Option<(u32, u16)> {
        match self {
            Self::Reference(o, g) => Some((*o, *g)),
            _ => None,
        }
    }
}

/// Decodes PDFDocEncoding or UTF-16BE / UTF-8 text strings per ISO 32000-1 §7.9.2.2.
pub fn decode_pdf_text_string(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let mut units = Vec::with_capacity((bytes.len() - 2) / 2);
        let mut i = 2;
        while i + 1 < bytes.len() {
            units.push(u16::from_be_bytes([bytes[i], bytes[i + 1]]));
            i += 2;
        }
        return String::from_utf16_lossy(&units);
    }
    if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    if let Ok(utf8) = std::str::from_utf8(bytes) {
        return utf8.to_string();
    }
    bytes.iter().map(|&b| b as char).collect()
}
