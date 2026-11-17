use std::collections::BTreeMap;

use crate::config::{Rect, TableStrategy, ToMarkdownOptions};
use crate::layout::spatial::{SpatialGrid, UnionFind};
use crate::layout::DetectedTable;
use crate::pdf::{ParsedPage, TextSpan};

pub fn detect_tables(
    page: &ParsedPage,
    spans: &[TextSpan],
    options: &ToMarkdownOptions,
) -> Vec<DetectedTable> {
    match options.table_strategy {
        TableStrategy::Explicit => detect_explicit_tables(page.page_index, spans, options),
        TableStrategy::LinesStrict => detect_line_tables(page, spans, false),
        TableStrategy::Lines => detect_line_tables(page, spans, true),
        TableStrategy::Text => detect_text_aligned_tables(spans),
    }
}

fn detect_explicit_tables(
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

#[derive(Clone, Copy)]
struct TableLine {
    is_horiz: bool,
    coord: f32, // y for horiz, x for vert
    start: f32, // x0 for horiz, y0 for vert
    end: f32,   // x1 for horiz, y1 for vert
}

impl TableLine {
    fn padded_bbox(&self, pad: f32) -> Rect {
        if self.is_horiz {
            Rect::new(self.start - pad, self.coord - pad, self.end + pad, self.coord + pad)
        } else {
            Rect::new(self.coord - pad, self.start - pad, self.coord + pad, self.end + pad)
        }
    }
}

/// Detects multiple independent line-ruled tables per page using a 2D SpatialGrid + Union-Find
/// connected-component clustering over intersecting/nearby horizontal and vertical segments.
fn detect_line_tables(
    page: &ParsedPage,
    spans: &[TextSpan],
    include_filled_rects: bool,
) -> Vec<DetectedTable> {
    let mut lines: Vec<TableLine> = Vec::new();

    for seg in &page.segments {
        if !seg.is_stroked && !include_filled_rects {
            continue;
        }
        let dx = (seg.p1.0 - seg.p0.0).abs();
        let dy = (seg.p1.1 - seg.p0.1).abs();
        if dy <= 1.5 && dx >= 12.0 {
            lines.push(TableLine {
                is_horiz: true,
                coord: (seg.p0.1 + seg.p1.1) * 0.5,
                start: seg.p0.0.min(seg.p1.0),
                end: seg.p0.0.max(seg.p1.0),
            });
        } else if dx <= 1.5 && dy >= 8.0 {
            lines.push(TableLine {
                is_horiz: false,
                coord: (seg.p0.0 + seg.p1.0) * 0.5,
                start: seg.p0.1.min(seg.p1.1),
                end: seg.p0.1.max(seg.p1.1),
            });
        }
    }

    if include_filled_rects {
        for r in &page.rects {
            if r.is_filled && r.rect.width() >= 12.0 && r.rect.height() >= 8.0 {
                lines.push(TableLine { is_horiz: true, coord: r.rect.y0, start: r.rect.x0, end: r.rect.x1 });
                lines.push(TableLine { is_horiz: true, coord: r.rect.y1, start: r.rect.x0, end: r.rect.x1 });
                lines.push(TableLine { is_horiz: false, coord: r.rect.x0, start: r.rect.y0, end: r.rect.y1 });
                lines.push(TableLine { is_horiz: false, coord: r.rect.x1, start: r.rect.y0, end: r.rect.y1 });
            }
        }
    }

    if lines.len() < 4 {
        return Vec::new();
    }

    let mut line_grid = SpatialGrid::new(32.0);
    let mut uf = UnionFind::new(lines.len());
    let tol = 4.0f32;

    for (i, ln) in lines.iter().enumerate() {
        let query = ln.padded_bbox(tol);
        for j in line_grid.query_rect(query) {
            if lines_connect(ln, &lines[j], tol) {
                uf.union(i, j);
            }
        }
        line_grid.insert(i, query);
    }

    let mut components: BTreeMap<usize, Vec<&TableLine>> = BTreeMap::new();
    for (i, ln) in lines.iter().enumerate() {
        let root = uf.find(i);
        components.entry(root).or_default().push(ln);
    }

    let span_grid = build_span_grid(spans);
    let mut detected = Vec::new();

    for comp in components.into_values() {
        let h_coords: Vec<f32> = comp.iter().filter(|l| l.is_horiz).map(|l| l.coord).collect();
        let v_coords: Vec<f32> = comp.iter().filter(|l| !l.is_horiz).map(|l| l.coord).collect();
        if h_coords.len() < 2 || v_coords.len() < 2 {
            continue;
        }
        let ys = cluster_coords(h_coords, 2.5);
        let xs = cluster_coords(v_coords, 2.5);
        if ys.len() < 2 || xs.len() < 2 {
            continue;
        }
        let bbox = Rect::new(
            *xs.first().unwrap(),
            *ys.first().unwrap(),
            *xs.last().unwrap(),
            *ys.last().unwrap(),
        );
        if let Some(tbl) = populate_grid_indexed(&xs, &ys, bbox, spans, &span_grid) {
            detected.push(tbl);
        }
    }

    detected.sort_by(|a, b| {
        let dy = (a.bbox.y0 - b.bbox.y0).abs();
        if dy <= 10.0 {
            a.bbox.x0.partial_cmp(&b.bbox.x0).unwrap()
        } else {
            a.bbox.y0.partial_cmp(&b.bbox.y0).unwrap()
        }
    });

    detected
}

#[inline]
fn lines_connect(a: &TableLine, b: &TableLine, tol: f32) -> bool {
    match (a.is_horiz, b.is_horiz) {
        (true, false) => {
            b.coord >= a.start - tol
                && b.coord <= a.end + tol
                && a.coord >= b.start - tol
                && a.coord <= b.end + tol
        }
        (false, true) => {
            a.coord >= b.start - tol
                && a.coord <= b.end + tol
                && b.coord >= a.start - tol
                && b.coord <= a.end + tol
        }
        (true, true) => {
            (a.coord - b.coord).abs() <= tol
                && a.start <= b.end + tol
                && a.end >= b.start - tol
        }
        (false, false) => {
            (a.coord - b.coord).abs() <= tol
                && a.start <= b.end + tol
                && a.end >= b.start - tol
        }
    }
}

fn build_span_grid(spans: &[TextSpan]) -> SpatialGrid {
    let mut grid = SpatialGrid::new(32.0);
    for (idx, s) in spans.iter().enumerate() {
        grid.insert(idx, s.bbox);
    }
    grid
}

fn detect_text_aligned_tables(spans: &[TextSpan]) -> Vec<DetectedTable> {
    if spans.len() < 4 {
        return Vec::new();
    }
    let mut sorted = spans.to_vec();
    sorted.sort_by(|a, b| a.bbox.y0.partial_cmp(&b.bbox.y0).unwrap());

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
    let mut run_start = None;
    let mut row_cells: Vec<Vec<(f32, f32, String)>> = Vec::new();

    for row in &mut rows {
        row.sort_by(|a, b| a.bbox.x0.partial_cmp(&b.bbox.x0).unwrap());
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
                if let Some(t) = finalize_text_table(&rows[run_start.unwrap()..idx], &row_cells) {
                    tables.push(t);
                }
            }
            run_start = None;
            row_cells.clear();
        }
    }

    if row_cells.len() >= 2 {
        if let Some(t) = finalize_text_table(&rows[run_start.unwrap()..], &row_cells) {
            tables.push(t);
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

    let mut col_starts: Vec<f32> = Vec::new();
    for r in row_cells {
        for c in r {
            if !col_starts.iter().any(|&x| (x - c.0).abs() < 18.0) {
                col_starts.push(c.0);
            }
        }
    }
    col_starts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let num_cols = col_starts.len();
    if num_cols < 2 {
        return None;
    }

    let mut grid = vec![vec![String::new(); num_cols]; row_cells.len()];
    for (r_idx, row) in row_cells.iter().enumerate() {
        for cell in row {
            let c_idx = col_starts
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| (*a - cell.0).abs().partial_cmp(&(*b - cell.0).abs()).unwrap())
                .map(|(i, _)| i)
                .unwrap_or(0);
            if !grid[r_idx][c_idx].is_empty() {
                grid[r_idx][c_idx].push(' ');
            }
            grid[r_idx][c_idx].push_str(&cell.2);
        }
    }

    let mut bbox = raw_rows[0][0].bbox;
    for r in raw_rows {
        for s in r {
            bbox = bbox.union(&s.bbox);
        }
    }

    let markdown = format_gfm_table(&grid);
    Some(DetectedTable {
        bbox,
        rows: grid.len(),
        cols: num_cols,
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

fn populate_grid_indexed(
    xs: &[f32],
    ys: &[f32],
    bbox: Rect,
    spans: &[TextSpan],
    span_grid: &SpatialGrid,
) -> Option<DetectedTable> {
    let rows = ys.len().saturating_sub(1);
    let cols = xs.len().saturating_sub(1);
    if rows == 0 || cols == 0 {
        return None;
    }

    let mut grid = vec![vec![String::new(); cols]; rows];
    let mut non_empty_cells = 0usize;

    for r in 0..rows {
        for c in 0..cols {
            let cell_rect = Rect::new(xs[c], ys[r], xs[c + 1], ys[r + 1]);
            let candidates = span_grid.query_rect(cell_rect);
            let mut cell_spans: Vec<&TextSpan> = candidates
                .into_iter()
                .filter_map(|idx| spans.get(idx))
                .filter(|s| cell_rect.contains_point(s.bbox.center_x(), s.bbox.center_y()))
                .collect();
            cell_spans.sort_by(|a, b| {
                let dy = (a.bbox.y0 - b.bbox.y0).abs();
                if dy <= 3.0 {
                    a.bbox.x0.partial_cmp(&b.bbox.x0).unwrap()
                } else {
                    a.bbox.y0.partial_cmp(&b.bbox.y0).unwrap()
                }
            });
            let text = cell_spans
                .iter()
                .map(|s| s.text.trim())
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if !text.is_empty() {
                non_empty_cells += 1;
            }
            grid[r][c] = text;
        }
    }

    if non_empty_cells < 2 {
        return None;
    }

    let markdown = format_gfm_table(&grid);
    Some(DetectedTable {
        bbox,
        rows,
        cols,
        cells: grid,
        markdown,
    })
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

fn cluster_coords(mut vals: Vec<f32>, tol: f32) -> Vec<f32> {
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut clustered: Vec<f32> = Vec::new();
    for v in vals {
        if let Some(last) = clustered.last_mut() {
            if (v - *last).abs() <= tol {
                *last = (*last + v) * 0.5;
                continue;
            }
        }
        clustered.push(v);
    }
    clustered
}
