pub mod content_stream;
pub mod encoding;
pub mod font;
pub mod objects;
pub mod parser;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

use crate::config::{Rect, ToMarkdownOptions};
use crate::error::Result;
use crate::markdown::chunk::TocItem;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub format: String,
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
    pub creator: String,
    pub producer: String,
    pub creation_date: String,
    pub mod_date: String,
    pub encryption: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TextSpan {
    pub text: String,
    pub bbox: Rect,
    pub origin: (f32, f32),
    pub font_name: String,
    pub font_size: f32,
    pub is_bold: bool,
    pub is_italic: bool,
    pub is_monospace: bool,
    pub is_strikeout: bool,
    pub is_superscript: bool,
    pub is_subscript: bool,
    pub color_rgb: (f32, f32, f32),
    pub bg_color_rgb: Option<(f32, f32, f32)>,
    pub alpha: f32,
    pub render_mode: u8,
    pub rotation_deg: f32,
    pub link_uri: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VectorSegment {
    pub p0: (f32, f32),
    pub p1: (f32, f32),
    pub stroke_width: f32,
    pub is_stroked: bool,
    pub is_filled: bool,
    pub color_rgb: (f32, f32, f32),
}

#[derive(Debug, Clone)]
pub struct VectorRect {
    pub rect: Rect,
    pub is_stroked: bool,
    pub is_filled: bool,
    pub stroke_width: f32,
    pub fill_rgb: Option<(f32, f32, f32)>,
}

#[derive(Debug, Clone)]
pub struct RawImagePlacement {
    pub bbox: Rect,
    pub width: u32,
    pub height: u32,
    pub bits_per_component: u8,
    pub color_space: String,
    pub filter: Option<String>,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct LinkAnnotation {
    pub rect: Rect,
    pub uri: Option<String>,
    pub dest_page: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ParsedPage {
    pub page_index: usize,
    pub media_box: Rect,
    pub crop_box: Rect,
    pub rotation: i32,
    pub spans: Vec<TextSpan>,
    pub segments: Vec<VectorSegment>,
    pub rects: Vec<VectorRect>,
    pub images: Vec<RawImagePlacement>,
    pub links: Vec<LinkAnnotation>,
}

#[derive(Debug, Clone, Default)]
pub struct InheritedPageProps {
    pub media_box: Option<Rect>,
    pub crop_box: Option<Rect>,
    pub rotate: Option<i32>,
    pub resources: BTreeMap<String, objects::PdfObject>,
}

/// Thread-safe (`Send + Sync`) immutable PDF document handle with shared concurrent font cache.
pub struct PdfDocument {
    pub file_path: Option<PathBuf>,
    pub pdf_version: String,
    pub objects: BTreeMap<u32, objects::PdfObject>,
    pub trailer: BTreeMap<String, objects::PdfObject>,
    pub page_refs: Vec<(u32, u16)>,
    pub inherited_props: BTreeMap<u32, InheritedPageProps>,
    pub font_cache: dashmap::DashMap<u32, font::PdfFont>,
    pub metadata: DocumentMetadata,
    pub toc: Vec<TocItem>,
}

impl PdfDocument {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path_ref = path.as_ref();
        let bytes = std::fs::read(path_ref)?;
        let mut doc = Self::from_bytes(&bytes)?;
        doc.file_path = Some(path_ref.to_path_buf());
        Ok(doc)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        parser::parse_pdf_bytes(bytes)
    }

    pub fn page_count(&self) -> usize {
        self.page_refs.len()
    }

    pub fn get_toc(&self) -> &[TocItem] {
        &self.toc
    }

    pub fn resolve<'a>(&'a self, obj: &'a objects::PdfObject) -> Option<&'a objects::PdfObject> {
        match obj {
            objects::PdfObject::Reference(num, _) => self.objects.get(num),
            other => Some(other),
        }
    }

    pub fn parse_page(&self, page_idx: usize, options: &ToMarkdownOptions) -> Result<ParsedPage> {
        content_stream::extract_page(self, page_idx, options)
    }
}
