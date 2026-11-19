use super::{
    assign_column_index, cluster_column_intervals, format_gfm_table, prune_empty_rows_and_cols,
};
use crate::config::{Rect, ToMarkdownOptions};
use crate::layout::DetectedTable;
use crate::pdf::TextSpan;

pub(super) fn detect_explicit_tables(
    page_idx: usize,
    spans: &[TextSpan],
    options: &ToMarkdownOptions,
) -> Vec<DetectedTable> {
    options
        .explicit_tables
        .iter()
        .filter(|(p, _)| *p == page_idx)
        .filter_map(|(_, bbox)| build_table_from_grid_bbox(*bbox, spans))
        .collect()
}

pub(super) fn detect_text_aligned_tables(spans: &[TextSpan]) -> Vec<DetectedTable> {
    if spans.len() < 4 {
        return Vec::new();
    }
    let mut sorted = spans.to_vec();
    sorted.sort_by(|a, b| {
        a.bbox
            .y0
            .total_cmp(&b.bbox.y0)
            .then_with(|| a.bbox.x0.total_cmp(&b.bbox.x0))
    });

    let mut rows: Vec<Vec<TextSpan>> = Vec::new();
    for s in sorted {
        if let Some(last_row) = rows.last_mut() {
            let ref_y = last_row[0].bbox.center_y();
            if (s.bbox.center_y() - ref_y).abs() <= 4.0 {
                last_row.push(s);
                continue;
            }
        }
        rows.push(vec![s]);
    }

    let mut tables = Vec::new();
    let mut run_start: Option<usize> = None;
    let mut row_cells: Vec<Vec<(f32, f32, String)>> = Vec::new();

    for row in &mut rows {
        row.sort_by(|a, b| a.bbox.x0.total_cmp(&b.bbox.x0));
    }

    for (idx, row) in rows.iter().enumerate() {
        let mut cells: Vec<(f32, f32, String)> = Vec::new();
        for s in row.iter() {
            if let Some(last) = cells.last_mut() {
                if s.bbox.x0 - last.1 < 14.0 {
                    last.1 = last.1.max(s.bbox.x1);
                    if !last.2.ends_with(' ') && !s.text.starts_with(' ') {
                        last.2.push(' ');
                    }
                    last.2.push_str(s.text.trim());
                    continue;
                }
            }
            cells.push((s.bbox.x0, s.bbox.x1, s.text.trim().to_string()));
        }

        if cells.len() >= 2 {
            if run_start.is_none() {
                run_start = Some(idx);
            }
            row_cells.push(cells);
        } else {
            if row_cells.len() >= 2 {
                if let Some(start_idx) = run_start {
                    if let Some(t) = finalize_text_table(&rows[start_idx..idx], &row_cells) {
                        tables.push(t);
                    }
                }
            }
            run_start = None;
            row_cells.clear();
        }
    }

    if row_cells.len() >= 2 {
        if let Some(start_idx) = run_start {
            if let Some(t) = finalize_text_table(&rows[start_idx..], &row_cells) {
                tables.push(t);
            }
        }
    }

    tables
}

fn finalize_text_table(
    raw_rows: &[Vec<TextSpan>],
    row_cells: &[Vec<(f32, f32, String)>],
) -> Option<DetectedTable> {
    let col_count = row_cells.iter().map(|r| r.len()).max()?;
    if col_count < 2 {
        return None;
    }

    // Use 1D interval projection so right-aligned financial columns align reliably
    let col_intervals = cluster_column_intervals(row_cells, 6.0);
    let num_cols = col_intervals.len();
    if num_cols < 2 {
        return None;
    }

    let mut grid = vec![vec![String::new(); num_cols]; row_cells.len()];
    for (r_idx, row) in row_cells.iter().enumerate() {
        for cell in row {
            let c_idx = assign_column_index(&col_intervals, cell.0, cell.1);
            if !grid[r_idx][c_idx].is_empty() {
                grid[r_idx][c_idx].push(' ');
            }
            grid[r_idx][c_idx].push_str(&cell.2);
        }
    }

    let grid = prune_empty_rows_and_cols(grid);
    let final_rows = grid.len();
    let final_cols = grid.first().map(|r| r.len()).unwrap_or(0);
    if final_rows < 2 || final_cols < 2 {
        return None;
    }

    let mut bbox = raw_rows.first()?.first()?.bbox;
    for r in raw_rows {
        for s in r {
            bbox = bbox.union(&s.bbox);
        }
    }

    let markdown = format_gfm_table(&grid);
    Some(DetectedTable {
        bbox,
        rows: final_rows,
        cols: final_cols,
        cells: grid,
        markdown,
    })
}

fn build_table_from_grid_bbox(bbox: Rect, spans: &[TextSpan]) -> Option<DetectedTable> {
    let inside: Vec<TextSpan> = spans
        .iter()
        .filter(|s| bbox.contains_point(s.bbox.center_x(), s.bbox.center_y()))
        .cloned()
        .collect();
    detect_text_aligned_tables(&inside).into_iter().next()
}
