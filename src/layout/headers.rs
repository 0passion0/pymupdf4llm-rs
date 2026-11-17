use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use rayon::prelude::*;

use crate::config::ToMarkdownOptions;
use crate::error::Result;
use crate::pdf::PdfDocument;
use crate::pdf::TextSpan;

pub trait HeaderResolver: Send + Sync {
    fn get_header_id(&self, span: &TextSpan, page_index: usize) -> String;
}

/// Default no-op header resolver when `hdr_info` is disabled.
pub struct NoopHeaders;

impl HeaderResolver for NoopHeaders {
    fn get_header_id(&self, _span: &TextSpan, _page_index: usize) -> String {
        String::new()
    }
}

/// Pure-Rust replica of `pymupdf4llm.IdentifyHeaders`:
/// Scans the document's font-size distribution weighted by character count, identifies the
/// dominant body font size, and maps larger rounded font sizes to `# `..`###### `.
#[derive(Debug, Clone)]
pub struct IdentifyHeaders {
    pub body_limit: f32,
    pub max_levels: usize,
    pub header_sizes: Vec<(i32, usize)>, // (rounded_size_tenths, level 1..=max_levels)
}

impl IdentifyHeaders {
    pub fn from_document(
        doc: &Arc<PdfDocument>,
        max_levels: usize,
        explicit_body_limit: Option<f32>,
        options: &ToMarkdownOptions,
    ) -> Result<Self> {
        let mut counts: BTreeMap<i32, usize> = BTreeMap::new();
        let page_indices: Vec<usize> = match &options.pages {
            Some(p) => p.clone(),
            None => (0..doc.page_count()).collect(),
        };

        let per_page_counts: Vec<BTreeMap<i32, usize>> = page_indices
            .par_iter()
            .map(|&idx| {
                let mut local = BTreeMap::new();
                if let Ok(page) = doc.parse_page(idx, options) {
                    for span in page.spans {
                        let key = (span.font_size * 10.0).round() as i32;
                        let chars = span.text.trim().chars().count();
                        if chars > 0 {
                            *local.entry(key).or_insert(0) += chars;
                        }
                    }
                }
                local
            })
            .collect();

        for local in per_page_counts {
            for (k, v) in local {
                *counts.entry(k).or_insert(0) += v;
            }
        }

        let body_tenths = explicit_body_limit
            .map(|b| (b * 10.0).round() as i32)
            .unwrap_or_else(|| {
                counts
                    .iter()
                    .max_by_key(|(_, &c)| c)
                    .map(|(&size, _)| size)
                    .unwrap_or(110)
            });

        let mut larger_sizes: Vec<i32> = counts
            .keys()
            .copied()
            .filter(|&sz| sz > body_tenths + 5) // > +0.5pt above body size
            .collect();
        larger_sizes.sort_by(|a, b| b.cmp(a)); // descending: largest = H1

        let capped = max_levels.clamp(1, 6);
        let mut header_sizes = Vec::new();
        for (idx, sz) in larger_sizes.into_iter().take(capped).enumerate() {
            header_sizes.push((sz, idx + 1));
        }

        Ok(Self {
            body_limit: body_tenths as f32 / 10.0,
            max_levels: capped,
            header_sizes,
        })
    }
}

impl HeaderResolver for IdentifyHeaders {
    fn get_header_id(&self, span: &TextSpan, _page_index: usize) -> String {
        if span.is_monospace {
            return String::new();
        }
        let tenths = (span.font_size * 10.0).round() as i32;
        for &(sz, level) in &self.header_sizes {
            if (tenths - sz).abs() <= 4 {
                return format!("{} ", "#".repeat(level));
            }
        }
        // Fallback if font exceeds body size by >= 1.5pt even if unseen during sampling
        if span.font_size > self.body_limit + 1.5 {
            let level = self.header_sizes.len().saturating_add(1).min(self.max_levels);
            return format!("{} ", "#".repeat(level.max(1)));
        }
        String::new()
    }
}

/// Pure-Rust replica of `pymupdf4llm.TocHeaders`:
/// Uses PDF native `/Outlines` bookmark tree (`doc.get_toc()`) to determine heading levels.
#[derive(Debug, Clone)]
pub struct TocHeaders {
    /// Map of 0-based page_index -> Vec<(level, normalized_title)>
    pub page_entries: HashMap<usize, Vec<(usize, String)>>,
}

impl TocHeaders {
    pub fn from_document(doc: &PdfDocument) -> Self {
        let mut page_entries: HashMap<usize, Vec<(usize, String)>> = HashMap::new();
        for &(level, ref title, page_1based) in doc.get_toc() {
            let page_idx = page_1based.saturating_sub(1);
            let norm = normalize_title(title);
            if !norm.is_empty() {
                page_entries
                    .entry(page_idx)
                    .or_default()
                    .push((level.clamp(1, 6), norm));
            }
        }
        Self { page_entries }
    }
}

impl HeaderResolver for TocHeaders {
    fn get_header_id(&self, span: &TextSpan, page_index: usize) -> String {
        let Some(entries) = self.page_entries.get(&page_index) else {
            return String::new();
        };
        let norm_span = normalize_title(&span.text);
        if norm_span.len() < 2 {
            return String::new();
        }
        for (level, title) in entries {
            if title == &norm_span || title.starts_with(&norm_span) || norm_span.starts_with(title) {
                return format!("{} ", "#".repeat(*level));
            }
        }
        String::new()
    }
}

fn normalize_title(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
