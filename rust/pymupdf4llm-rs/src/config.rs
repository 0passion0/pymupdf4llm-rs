use std::path::PathBuf;
use std::sync::Arc;
use serde::{Deserialize, Serialize};

use crate::layout::headers::HeaderResolver;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableStrategy {
    LinesStrict,
    Lines,
    Text,
    Explicit,
}

impl Default for TableStrategy {
    fn default() -> Self {
        Self::LinesStrict
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

impl Default for ImageFormat {
    fn default() -> Self {
        Self::Png
    }
}

impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }

    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }
}

/// Margins matching PyMuPDF4LLM's (left, top, right, bottom) or scalar/2-tuple semantics.
/// Default in pymupdf4llm is (0.0, 50.0, 0.0, 50.0).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Margins {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Default for Margins {
    fn default() -> Self {
        Self {
            left: 0.0,
            top: 50.0,
            right: 0.0,
            bottom: 50.0,
        }
    }
}

impl From<f32> for Margins {
    fn from(v: f32) -> Self {
        Self {
            left: v,
            top: v,
            right: v,
            bottom: v,
        }
    }
}

impl From<(f32, f32)> for Margins {
    fn from((h, v): (f32, f32)) -> Self {
        Self {
            left: h,
            top: v,
            right: h,
            bottom: v,
        }
    }
}

impl From<(f32, f32, f32, f32)> for Margins {
    fn from((left, top, right, bottom): (f32, f32, f32, f32)) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    pub const fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    #[inline]
    pub fn width(&self) -> f32 {
        (self.x1 - self.x0).max(0.0)
    }

    #[inline]
    pub fn height(&self) -> f32 {
        (self.y1 - self.y0).max(0.0)
    }

    #[inline]
    pub fn area(&self) -> f32 {
        self.width() * self.height()
    }

    #[inline]
    pub fn center_x(&self) -> f32 {
        (self.x0 + self.x1) * 0.5
    }

    #[inline]
    pub fn center_y(&self) -> f32 {
        (self.y0 + self.y1) * 0.5
    }

    #[inline]
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x0 < other.x1 && self.x1 > other.x0 && self.y0 < other.y1 && self.y1 > other.y0
    }

    #[inline]
    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }

    #[inline]
    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }
}

#[derive(Clone)]
pub enum HeaderMode {
    IdentifyHeaders { max_levels: usize, body_limit: Option<f32> },
    TocHeaders,
    Custom(Arc<dyn HeaderResolver>),
    Disabled,
}

impl std::fmt::Debug for HeaderMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IdentifyHeaders { max_levels, body_limit } => f
                .debug_struct("IdentifyHeaders")
                .field("max_levels", max_levels)
                .field("body_limit", body_limit)
                .finish(),
            Self::TocHeaders => write!(f, "TocHeaders"),
            Self::Custom(_) => write!(f, "Custom(<HeaderResolver>)"),
            Self::Disabled => write!(f, "Disabled"),
        }
    }
}

impl Default for HeaderMode {
    fn default() -> Self {
        Self::IdentifyHeaders {
            max_levels: 6,
            body_limit: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConcurrencyConfig {
    pub max_page_concurrency: usize,
    pub max_doc_concurrency: usize,
    pub channel_buffer: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextHygieneConfig {
    pub clean_mojibake: bool,
    pub dehyphenate: bool,
    pub dedup_faux_bold: bool,
    pub ignore_invisible_text: bool,
    pub strip_watermarks: bool,
    pub dedup_running_headers: bool,
}

impl Default for TextHygieneConfig {
    fn default() -> Self {
        Self {
            clean_mojibake: true,
            dehyphenate: true,
            dedup_faux_bold: true,
            ignore_invisible_text: true,
            strip_watermarks: true,
            dedup_running_headers: false,
        }
    }
}

impl Default for ConcurrencyConfig {
    fn default() -> Self {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        Self {
            max_page_concurrency: cpus.max(4),
            max_doc_concurrency: cpus.max(2),
            channel_buffer: 64,
        }
    }
}

pub type ProgressCallback = Arc<dyn Fn(usize, usize) + Send + Sync>;

#[derive(Clone)]
pub struct ToMarkdownOptions {
    pub pages: Option<Vec<usize>>,
    pub hdr_info: HeaderMode,
    pub write_images: bool,
    pub embed_images: bool,
    pub dpi: u32,
    pub image_path: PathBuf,
    pub image_format: ImageFormat,
    pub image_size_limit: f32,
    pub force_text: bool,
    pub margins: Margins,
    pub page_chunks: bool,
    pub page_separators: bool,
    pub page_width: f32,
    pub page_height: Option<f32>,
    pub table_strategy: TableStrategy,
    pub explicit_tables: Vec<(usize, Rect)>,
    pub graphics_limit: Option<usize>,
    pub ignore_code: bool,
    pub ignore_images: bool,
    pub ignore_graphics: bool,
    pub extract_words: bool,
    pub fontsize_limit: f32,
    pub use_glyphs: bool,
    pub detect_bg_color: bool,
    pub ignore_alpha: bool,
    pub header: bool,
    pub footer: bool,
    pub show_progress: bool,
    pub progress_callback: Option<ProgressCallback>,
    pub concurrency: ConcurrencyConfig,
    pub hygiene: TextHygieneConfig,
}

impl std::fmt::Debug for ToMarkdownOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToMarkdownOptions")
            .field("pages", &self.pages)
            .field("hdr_info", &self.hdr_info)
            .field("write_images", &self.write_images)
            .field("embed_images", &self.embed_images)
            .field("dpi", &self.dpi)
            .field("image_path", &self.image_path)
            .field("image_format", &self.image_format)
            .field("image_size_limit", &self.image_size_limit)
            .field("force_text", &self.force_text)
            .field("margins", &self.margins)
            .field("page_chunks", &self.page_chunks)
            .field("page_separators", &self.page_separators)
            .field("page_width", &self.page_width)
            .field("page_height", &self.page_height)
            .field("table_strategy", &self.table_strategy)
            .field("graphics_limit", &self.graphics_limit)
            .field("ignore_code", &self.ignore_code)
            .field("ignore_images", &self.ignore_images)
            .field("ignore_graphics", &self.ignore_graphics)
            .field("extract_words", &self.extract_words)
            .field("fontsize_limit", &self.fontsize_limit)
            .field("use_glyphs", &self.use_glyphs)
            .field("detect_bg_color", &self.detect_bg_color)
            .field("ignore_alpha", &self.ignore_alpha)
            .field("header", &self.header)
            .field("footer", &self.footer)
            .field("show_progress", &self.show_progress)
            .field("concurrency", &self.concurrency)
            .finish()
    }
}

impl Default for ToMarkdownOptions {
    fn default() -> Self {
        Self {
            pages: None,
            hdr_info: HeaderMode::default(),
            write_images: false,
            embed_images: false,
            dpi: 150,
            image_path: PathBuf::new(),
            image_format: ImageFormat::Png,
            image_size_limit: 0.05,
            force_text: true,
            margins: Margins::default(),
            page_chunks: false,
            page_separators: false,
            page_width: 612.0,
            page_height: None,
            table_strategy: TableStrategy::LinesStrict,
            explicit_tables: Vec::new(),
            graphics_limit: None,
            ignore_code: false,
            ignore_images: false,
            ignore_graphics: false,
            extract_words: false,
            fontsize_limit: 3.0,
            use_glyphs: false,
            detect_bg_color: true,
            ignore_alpha: false,
            header: true,
            footer: true,
            show_progress: false,
            progress_callback: None,
            concurrency: ConcurrencyConfig::default(),
            hygiene: TextHygieneConfig::default(),
        }
    }
}

impl ToMarkdownOptions {
    pub fn builder() -> ToMarkdownOptionsBuilder {
        ToMarkdownOptionsBuilder {
            inner: Self::default(),
        }
    }
}

pub struct ToMarkdownOptionsBuilder {
    inner: ToMarkdownOptions,
}

impl ToMarkdownOptionsBuilder {
    pub fn pages(mut self, pages: Vec<usize>) -> Self {
        self.inner.pages = Some(pages);
        self
    }

    pub fn hdr_info(mut self, hdr_info: HeaderMode) -> Self {
        self.inner.hdr_info = hdr_info;
        self
    }

    pub fn write_images(mut self, write_images: bool) -> Self {
        self.inner.write_images = write_images;
        self
    }

    pub fn embed_images(mut self, embed_images: bool) -> Self {
        self.inner.embed_images = embed_images;
        self
    }

    pub fn dpi(mut self, dpi: u32) -> Self {
        self.inner.dpi = dpi;
        self
    }

    pub fn image_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.inner.image_path = path.into();
        self
    }

    pub fn image_format(mut self, format: ImageFormat) -> Self {
        self.inner.image_format = format;
        self
    }

    pub fn image_size_limit(mut self, limit: f32) -> Self {
        self.inner.image_size_limit = limit;
        self
    }

    pub fn force_text(mut self, force_text: bool) -> Self {
        self.inner.force_text = force_text;
        self
    }

    pub fn margins(mut self, margins: impl Into<Margins>) -> Self {
        self.inner.margins = margins.into();
        self
    }

    pub fn page_chunks(mut self, page_chunks: bool) -> Self {
        self.inner.page_chunks = page_chunks;
        self
    }

    pub fn page_separators(mut self, page_separators: bool) -> Self {
        self.inner.page_separators = page_separators;
        self
    }

    pub fn page_width(mut self, page_width: f32) -> Self {
        self.inner.page_width = page_width;
        self
    }

    pub fn page_height(mut self, page_height: Option<f32>) -> Self {
        self.inner.page_height = page_height;
        self
    }

    pub fn table_strategy(mut self, strategy: TableStrategy) -> Self {
        self.inner.table_strategy = strategy;
        self
    }

    pub fn explicit_tables(mut self, tables: Vec<(usize, Rect)>) -> Self {
        self.inner.explicit_tables = tables;
        self
    }

    pub fn graphics_limit(mut self, limit: Option<usize>) -> Self {
        self.inner.graphics_limit = limit;
        self
    }

    pub fn ignore_code(mut self, ignore_code: bool) -> Self {
        self.inner.ignore_code = ignore_code;
        self
    }

    pub fn ignore_images(mut self, ignore_images: bool) -> Self {
        self.inner.ignore_images = ignore_images;
        self
    }

    pub fn ignore_graphics(mut self, ignore_graphics: bool) -> Self {
        self.inner.ignore_graphics = ignore_graphics;
        self
    }

    pub fn extract_words(mut self, extract_words: bool) -> Self {
        self.inner.extract_words = extract_words;
        self
    }

    pub fn fontsize_limit(mut self, limit: f32) -> Self {
        self.inner.fontsize_limit = limit;
        self
    }

    pub fn use_glyphs(mut self, use_glyphs: bool) -> Self {
        self.inner.use_glyphs = use_glyphs;
        self
    }

    pub fn detect_bg_color(mut self, detect: bool) -> Self {
        self.inner.detect_bg_color = detect;
        self
    }

    pub fn ignore_alpha(mut self, ignore: bool) -> Self {
        self.inner.ignore_alpha = ignore;
        self
    }

    pub fn header(mut self, include_header: bool) -> Self {
        self.inner.header = include_header;
        self
    }

    pub fn footer(mut self, include_footer: bool) -> Self {
        self.inner.footer = include_footer;
        self
    }

    pub fn show_progress(mut self, show: bool) -> Self {
        self.inner.show_progress = show;
        self
    }

    pub fn max_page_concurrency(mut self, n: usize) -> Self {
        self.inner.concurrency.max_page_concurrency = n.max(1);
        self
    }

    pub fn max_doc_concurrency(mut self, n: usize) -> Self {
        self.inner.concurrency.max_doc_concurrency = n.max(1);
        self
    }

    pub fn clean_mojibake(mut self, clean: bool) -> Self {
        self.inner.hygiene.clean_mojibake = clean;
        self
    }

    pub fn dehyphenate(mut self, dehyphenate: bool) -> Self {
        self.inner.hygiene.dehyphenate = dehyphenate;
        self
    }

    pub fn dedup_faux_bold(mut self, dedup: bool) -> Self {
        self.inner.hygiene.dedup_faux_bold = dedup;
        self
    }

    pub fn ignore_invisible_text(mut self, ignore: bool) -> Self {
        self.inner.hygiene.ignore_invisible_text = ignore;
        self
    }

    pub fn strip_watermarks(mut self, strip: bool) -> Self {
        self.inner.hygiene.strip_watermarks = strip;
        self
    }

    pub fn dedup_running_headers(mut self, dedup: bool) -> Self {
        self.inner.hygiene.dedup_running_headers = dedup;
        self
    }

    pub fn build(self) -> ToMarkdownOptions {
        self.inner
    }
}
