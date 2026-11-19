mod line_tables;
mod text_tables;

use crate::config::{TableStrategy, ToMarkdownOptions};
use crate::layout::spatial::SpatialGrid;
use crate::layout::DetectedTable;
use crate::pdf::{ParsedPage, TextSpan};

pub fn detect_tables(
    page: &ParsedPage,
    spans: &[TextSpan],
    options: &ToMarkdownOptions,
) -> Vec<DetectedTable> {
    match options.table_strategy {
        TableStrategy::Explicit => {
            text_tables::detect_explicit_tables(page.page_index, spans, options)
        }
        TableStrategy::LinesStrict => line_tables::detect_line_tables(page, spans, false),
        TableStrategy::Lines => line_tables::detect_line_tables(page, spans, true),
        TableStrategy::Text => text_tables::detect_text_aligned_tables(spans),
    }
}

pub fn format_gfm_table(grid: &[Vec<String>]) -> String {
    if grid.is_empty() {
        return String::new();
    }
    let cols = grid[0].len();
    let mut lines = Vec::with_capacity(grid.len() + 1);

    let format_row = |row: &[String]| -> String {
        let cells: Vec<String> = row
            .iter()
            .map(|c| c.replace('|', "\\|").replace('\n', "<br>"))
            .collect();
        format!("|{}|", cells.join("|"))
    };

    lines.push(format_row(&grid[0]));
    lines.push(format!("|{}|", vec!["---"; cols].join("|")));
    for row in &grid[1..] {
        lines.push(format_row(row));
    }
    lines.join("\n")
}

pub(super) fn build_span_grid(spans: &[TextSpan]) -> SpatialGrid {
    let mut grid = SpatialGrid::new(32.0);
    for (idx, s) in spans.iter().enumerate() {
        grid.insert(idx, s.bbox);
    }
    grid
}

/// Clusters nearby 1D coordinates using true arithmetic cluster means (avoiding exponential drift).
pub(super) fn cluster_coords(mut vals: Vec<f32>, tol: f32) -> Vec<f32> {
    vals.sort_by(f32::total_cmp);
    let mut clusters: Vec<(f32, usize)> = Vec::new();
    for v in vals {
        if let Some((sum, count)) = clusters.last_mut() {
            let mean = *sum / (*count as f32);
            if (v - mean).abs() <= tol {
                *sum += v;
                *count += 1;
                continue;
            }
        }
        clusters.push((v, 1));
    }
    clusters
        .into_iter()
        .map(|(sum, count)| sum / (count as f32))
        .collect()
}

pub(super) fn cluster_column_intervals(
    row_cells: &[Vec<(f32, f32, String)>],
    tol: f32,
) -> Vec<(f32, f32)> {
    let mut intervals: Vec<(f32, f32)> = row_cells
        .iter()
        .flat_map(|r| r.iter().map(|c| (c.0, c.1)))
        .collect();
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));

    let mut merged: Vec<(f32, f32)> = Vec::new();
    for (x0, x1) in intervals {
        if let Some(last) = merged.last_mut() {
            if x0 <= last.1 + tol {
                last.1 = last.1.max(x1);
                continue;
            }
        }
        merged.push((x0, x1));
    }
    merged
}

pub(super) fn assign_column_index(col_intervals: &[(f32, f32)], x0: f32, x1: f32) -> usize {
    let mut best_idx = 0usize;
    let mut best_overlap = -1.0f32;
    for (idx, &(cx0, cx1)) in col_intervals.iter().enumerate() {
        let overlap = (cx1.min(x1) - cx0.max(x0)).max(0.0);
        if overlap > best_overlap {
            best_overlap = overlap;
            best_idx = idx;
        }
    }
    if best_overlap > 0.0 {
        return best_idx;
    }
    let center = (x0 + x1) * 0.5;
    col_intervals
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let da = ((a.0 + a.1) * 0.5 - center).abs();
            let db = ((b.0 + b.1) * 0.5 - center).abs();
            da.total_cmp(&db)
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}

pub(super) fn prune_empty_rows_and_cols(mut grid: Vec<Vec<String>>) -> Vec<Vec<String>> {
    grid.retain(|row| row.iter().any(|c| !c.trim().is_empty()));
    let Some(num_cols) = grid.first().map(|r| r.len()) else {
        return grid;
    };
    let keep_col: Vec<bool> = (0..num_cols)
        .map(|c| grid.iter().any(|row| !row[c].trim().is_empty()))
        .collect();
    if keep_col.iter().all(|&k| k) {
        return grid;
    }
    for row in &mut grid {
        let mut idx = 0usize;
        row.retain(|_| {
            let keep = keep_col.get(idx).copied().unwrap_or(false);
            idx += 1;
            keep
        });
    }
    grid
}
