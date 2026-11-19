pub mod columns;
pub mod headers;
pub mod spatial;
pub mod tables;

use std::collections::BTreeMap;

use crate::config::{Rect, ToMarkdownOptions};
use crate::layout::spatial::{SpatialGrid, UnionFind};
use crate::pdf::{ParsedPage, RawImagePlacement, TextSpan};
use crate::text_cleaner::{filter_and_dedup_spans, sanitize_span_text, HygieneStats};

#[derive(Debug, Clone)]
pub struct TextLine {
    pub bbox: Rect,
    pub spans: Vec<TextSpan>,
    pub text: String,
    pub max_font_size: f32,
    pub is_all_bold: bool,
    pub is_all_monospace: bool,
}

#[derive(Debug, Clone)]
pub struct DetectedTable {
    pub bbox: Rect,
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<Vec<String>>,
    pub markdown: String,
}

#[derive(Debug, Clone)]
pub struct LayoutPage {
    pub content_bbox: Rect,
    pub ordered_lines: Vec<TextLine>,
    pub tables: Vec<DetectedTable>,
    pub graphics: Vec<Rect>,
    pub images: Vec<RawImagePlacement>,
    pub hygiene_stats: HygieneStats,
}

pub fn analyze_page_layout(page: &ParsedPage, options: &ToMarkdownOptions) -> LayoutPage {
    let cb = page.crop_box;
    let mut content_bbox = Rect::new(
        cb.x0 + options.margins.left,
        cb.y0 + options.margins.top,
        (cb.x1 - options.margins.right).max(cb.x0 + options.margins.left + 1.0),
        (cb.y1 - options.margins.bottom).max(cb.y0 + options.margins.top + 1.0),
    );

    if !options.header {
        content_bbox.y0 = content_bbox.y0.max(cb.y0 + cb.height() * 0.08);
    }
    if !options.footer {
        content_bbox.y1 = content_bbox.y1.min(cb.y1 - cb.height() * 0.08);
    }

    let mut hygiene_stats = HygieneStats::default();

    let sanitized_spans: Vec<TextSpan> = page
        .spans
        .iter()
        .filter_map(|s| {
            if !s.bbox.intersects(&content_bbox) {
                return None;
            }
            let cleaned = sanitize_span_text(&s.text, &options.hygiene, &mut hygiene_stats);
            if cleaned.trim().is_empty() {
                return None;
            }
            let mut copy = s.clone();
            copy.text = cleaned;
            Some(copy)
        })
        .collect();

    let filtered_spans =
        filter_and_dedup_spans(sanitized_spans, &options.hygiene, &mut hygiene_stats);

    // 1. Detect tables (multiple independent tables per page via Union-Find)
    let tables = tables::detect_tables(page, &filtered_spans, options);

    // 2. Filter out spans consumed by tables using SpatialGrid if tables exist
    let non_table_spans: Vec<TextSpan> = if tables.is_empty() {
        filtered_spans
    } else {
        let mut table_grid = SpatialGrid::new(64.0);
        for (idx, t) in tables.iter().enumerate() {
            table_grid.insert(idx, t.bbox);
        }
        filtered_spans
            .into_iter()
            .filter(|s| {
                let cx = s.bbox.center_x();
                let cy = s.bbox.center_y();
                let hits = table_grid.query_point(cx, cy);
                !hits
                    .iter()
                    .any(|&ti| tables[ti].bbox.contains_point(cx, cy))
            })
            .collect()
    };

    // 3. Reconstruct multi-column reading order via X-Y cut and group spans into TextLines
    let ordered_lines = columns::order_spans_into_lines(&non_table_spans, content_bbox);

    // 4. Cluster vector graphics via SpatialGrid + UnionFind
    let graphics = if options.ignore_graphics {
        Vec::new()
    } else {
        cluster_vector_graphics(page, content_bbox, options)
    };

    // 5. Filter raster images by area ratio (image_size_limit)
    let page_area = cb.area().max(1.0);
    let images = if options.ignore_images {
        Vec::new()
    } else {
        page.images
            .iter()
            .filter(|img| {
                let ratio = img.bbox.area() / page_area;
                ratio >= options.image_size_limit * 0.01 && img.bbox.intersects(&content_bbox)
            })
            .cloned()
            .collect()
    };

    LayoutPage {
        content_bbox,
        ordered_lines,
        tables,
        graphics,
        images,
        hygiene_stats,
    }
}

fn cluster_vector_graphics(
    page: &ParsedPage,
    content_bbox: Rect,
    options: &ToMarkdownOptions,
) -> Vec<Rect> {
    let total_drawings = page.segments.len() + page.rects.len();
    if let Some(limit) = options.graphics_limit {
        if total_drawings > limit {
            return Vec::new();
        }
    }

    let boxes: Vec<Rect> = page
        .rects
        .iter()
        .filter(|r| {
            r.rect.intersects(&content_bbox)
                && r.rect.width() > 6.0
                && r.rect.height() > 6.0
                && r.rect.area() < content_bbox.area() * 0.92
        })
        .map(|r| r.rect)
        .collect();

    if boxes.is_empty() {
        return Vec::new();
    }

    let mut grid = SpatialGrid::new(32.0);
    let mut uf = UnionFind::new(boxes.len());

    for (i, b) in boxes.iter().enumerate() {
        let expanded = Rect::new(b.x0 - 8.0, b.y0 - 8.0, b.x1 + 8.0, b.y1 + 8.0);
        for j in grid.query_rect(expanded) {
            let jb = boxes[j];
            let j_exp = Rect::new(jb.x0 - 8.0, jb.y0 - 8.0, jb.x1 + 8.0, jb.y1 + 8.0);
            if j_exp.intersects(b) {
                uf.union(i, j);
            }
        }
        grid.insert(i, expanded);
    }

    let mut groups: BTreeMap<usize, Rect> = BTreeMap::new();
    for (i, b) in boxes.iter().enumerate() {
        let root = uf.find(i);
        groups
            .entry(root)
            .and_modify(|acc| *acc = acc.union(b))
            .or_insert(*b);
    }

    let min_area = content_bbox.area() * options.image_size_limit.max(0.0) * 0.05;
    groups
        .into_values()
        .filter(|b| b.area() >= min_area)
        .collect()
}
