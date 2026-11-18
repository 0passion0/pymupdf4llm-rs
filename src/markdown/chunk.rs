use crate::text_cleaner::HygieneStats;
use serde::{Deserialize, Serialize};

/// Matches `doc.get_toc()` entry: `(level, title, page_1based)`
pub type TocItem = (usize, String, usize);

/// Matches `page.get_text("words")` word tuple: `(x0, y0, x1, y1, word, block_no, line_no, word_no)`
pub type ExtractedWord = (f32, f32, f32, f32, String, usize, usize, usize);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageMetadata {
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
    pub file_path: String,
    pub page_count: usize,
    pub page: usize, // 1-based page number matching pymupdf4llm
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedTable {
    pub bbox: [f32; 4],
    pub rows: usize,
    pub columns: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedImage {
    pub number: usize,
    pub bbox: [f32; 4],
    pub width: u32,
    pub height: u32,
    pub cs_name: String,
    pub ext: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedGraphic {
    pub bbox: [f32; 4],
}

/// 100% schema-compatible replica of `pymupdf4llm.to_markdown(..., page_chunks=True)` dictionary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageChunk {
    pub metadata: PageMetadata,
    pub toc_items: Vec<TocItem>,
    pub tables: Vec<ExtractedTable>,
    pub images: Vec<ExtractedImage>,
    pub graphics: Vec<ExtractedGraphic>,
    pub text: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub words: Vec<ExtractedWord>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hygiene: Option<HygieneStats>,
}
