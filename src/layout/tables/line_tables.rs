use std::collections::BTreeMap;

use super::{
    assign_column_index, build_span_grid, cluster_column_intervals, cluster_coords,
    format_gfm_table, prune_empty_rows_and_cols,
};
use crate::config::Rect;
use crate::layout::columns::should_insert_span_space;
use crate::layout::spatial::{SpatialGrid, UnionFind};
use crate::layout::DetectedTable;
use crate::pdf::{ParsedPage, TextSpan};

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
            Rect::new(
                self.start - pad,
                self.coord - pad,
                self.end + pad,
                self.coord + pad,
            )
        } else {
            Rect::new(
                self.coord - pad,
                self.start - pad,
                self.coord + pad,
                self.end + pad,
            )
        }
    }
}

/// Detects multiple independent tables per page:
/// - Tier 1: Fully-ruled grids using 2D SpatialGrid + Union-Find connected components.
/// - Tier 2: Horizontal-rule / three-line academic ("booktabs" / 三线表) tables where
///   vertical column borders are omitted and columns are inferred via 1D projection valleys.
pub(super) fn detect_line_tables(
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

    // Thin filled rectangles (`re f`) are the primary way Word, Chrome, LibreOffice, and
    // LaTeX `booktabs` render horizontal and vertical table rules.
    for r in &page.rects {
        if !r.is_filled {
            continue;
        }
        let w = r.rect.width();
        let h = r.rect.height();
        if h <= 4.0 && w >= 12.0 {
            lines.push(TableLine {
                is_horiz: true,
                coord: r.rect.center_y(),
                start: r.rect.x0,
                end: r.rect.x1,
            });
        } else if w <= 4.0 && h >= 8.0 {
            lines.push(TableLine {
                is_horiz: false,
                coord: r.rect.center_x(),
                start: r.rect.y0,
                end: r.rect.y1,
            });
        } else if include_filled_rects && w >= 12.0 && h >= 8.0 {
            lines.push(TableLine {
                is_horiz: true,
                coord: r.rect.y0,
                start: r.rect.x0,
                end: r.rect.x1,
            });
            lines.push(TableLine {
                is_horiz: true,
                coord: r.rect.y1,
                start: r.rect.x0,
                end: r.rect.x1,
            });
            lines.push(TableLine {
                is_horiz: false,
                coord: r.rect.x0,
                start: r.rect.y0,
                end: r.rect.y1,
            });
            lines.push(TableLine {
                is_horiz: false,
                coord: r.rect.x1,
                start: r.rect.y0,
                end: r.rect.y1,
            });
        }
    }

    if lines.len() < 2 {
        return Vec::new();
    }

    let span_grid = build_span_grid(spans);
    let mut detected = Vec::new();

    // Tier 1: Fully-ruled tables (horizontal + vertical segments)
    if lines.len() >= 4 {
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

        for comp in components.into_values() {
            let h_coords: Vec<f32> = comp
                .iter()
                .filter(|l| l.is_horiz)
                .map(|l| l.coord)
                .collect();
            let v_coords: Vec<f32> = comp
                .iter()
                .filter(|l| !l.is_horiz)
                .map(|l| l.coord)
                .collect();
            if h_coords.len() < 2 || v_coords.len() < 2 {
                continue;
            }
            let ys = cluster_coords(h_coords, 2.5);
            let xs = cluster_coords(v_coords, 2.5);
            let (Some(&x0), Some(&x1), Some(&y0), Some(&y1)) =
                (xs.first(), xs.last(), ys.first(), ys.last())
            else {
                continue;
            };
            if ys.len() < 2 || xs.len() < 2 {
                continue;
            }
            let bbox = Rect::new(x0, y0, x1, y1);
            if let Some(tbl) = populate_grid_indexed(&xs, &ys, bbox, spans, &span_grid) {
                detected.push(tbl);
            }
        }
    }

    // Tier 2: Three-line / horizontally-ruled academic tables (`booktabs` / 三线表)
    let horiz_rules: Vec<TableLine> = lines
        .iter()
        .copied()
        .filter(|l| {
            l.is_horiz
                && (l.end - l.start) >= 80.0
                && !detected.iter().any(|t| {
                    let expanded = Rect::new(
                        t.bbox.x0 - 4.0,
                        t.bbox.y0 - 4.0,
                        t.bbox.x1 + 4.0,
                        t.bbox.y1 + 4.0,
                    );
                    expanded.contains_point((l.start + l.end) * 0.5, l.coord)
                })
        })
        .collect();

    if horiz_rules.len() >= 2 {
        let mut booktabs = detect_three_line_tables(&horiz_rules, spans, &span_grid);
        detected.append(&mut booktabs);
    }

    detected.sort_by(|a, b| {
        a.bbox
            .y0
            .total_cmp(&b.bbox.y0)
            .then_with(|| a.bbox.x0.total_cmp(&b.bbox.x0))
    });
    let mut start = 0;
    while start < detected.len() {
        let ref_y = detected[start].bbox.y0;
        let mut end = start + 1;
        while end < detected.len() && (detected[end].bbox.y0 - ref_y).abs() <= 10.0 {
            end += 1;
        }
        if end - start > 1 {
            detected[start..end].sort_by(|a, b| a.bbox.x0.total_cmp(&b.bbox.x0));
        }
        start = end;
    }

    detected
}

/// Detects academic three-line ("booktabs" / 三线表) tables consisting of aligned
/// horizontal rules with no vertical rules by inferring column boundaries from 1D
/// vertical whitespace projection across the contained text spans.
fn detect_three_line_tables(
    horiz_rules: &[TableLine],
    spans: &[TextSpan],
    span_grid: &SpatialGrid,
) -> Vec<DetectedTable> {
    let mut sorted = horiz_rules.to_vec();
    sorted.sort_by(|a, b| {
        a.coord
            .total_cmp(&b.coord)
            .then_with(|| a.start.total_cmp(&b.start))
    });

    // Merge collinear/double horizontal rules at nearly identical Y
    let mut merged_rules: Vec<TableLine> = Vec::new();
    for rule in sorted {
        if let Some(last) = merged_rules.last_mut() {
            if (rule.coord - last.coord).abs() <= 3.0
                && rule.start <= last.end + 6.0
                && rule.end >= last.start - 6.0
            {
                last.start = last.start.min(rule.start);
                last.end = last.end.max(rule.end);
                last.coord = (last.coord + rule.coord) * 0.5;
                continue;
            }
        }
        merged_rules.push(rule);
    }

    // Group vertically adjacent rules sharing >= 75% horizontal span overlap
    let mut groups: Vec<Vec<TableLine>> = Vec::new();
    let mut used = vec![false; merged_rules.len()];

    for i in 0..merged_rules.len() {
        if used[i] {
            continue;
        }
        used[i] = true;
        let mut grp = vec![merged_rules[i]];
        let mut cur_x0 = merged_rules[i].start;
        let mut cur_x1 = merged_rules[i].end;
        let mut cur_y = merged_rules[i].coord;

        for j in (i + 1)..merged_rules.len() {
            if used[j] {
                continue;
            }
            let cand = merged_rules[j];
            if cand.coord - cur_y > 360.0 {
                break;
            }
            let overlap = (cur_x1.min(cand.end) - cur_x0.max(cand.start)).max(0.0);
            let max_w = (cur_x1 - cur_x0).max(cand.end - cand.start).max(1.0);
            if overlap / max_w >= 0.75 {
                used[j] = true;
                cur_x0 = cur_x0.min(cand.start);
                cur_x1 = cur_x1.max(cand.end);
                cur_y = cand.coord;
                grp.push(cand);
            }
        }

        if grp.len() >= 2 {
            groups.push(grp);
        }
    }

    let mut tables = Vec::new();
    for grp in groups {
        let Some(first_rule) = grp.first() else {
            continue;
        };
        let Some(last_rule) = grp.last() else {
            continue;
        };
        let x0 = grp.iter().map(|r| r.start).fold(f32::INFINITY, f32::min);
        let x1 = grp.iter().map(|r| r.end).fold(f32::NEG_INFINITY, f32::max);
        let y0 = first_rule.coord;
        let y1 = last_rule.coord;
        if y1 - y0 < 20.0 || x1 - x0 < 80.0 {
            continue;
        }
        let bbox = Rect::new(x0, y0, x1, y1);
        let query_rect = Rect::new(x0 - 4.0, y0 - 2.0, x1 + 4.0, y1 + 2.0);
        let inside_spans: Vec<&TextSpan> = span_grid
            .query_rect(query_rect)
            .into_iter()
            .filter_map(|idx| spans.get(idx))
            .filter(|s| query_rect.contains_point(s.bbox.center_x(), s.bbox.center_y()))
            .collect();

        if let Some(tbl) = infer_three_line_table_from_spans(bbox, &inside_spans) {
            tables.push(tbl);
        }
    }

    tables
}

fn infer_three_line_table_from_spans(bbox: Rect, spans: &[&TextSpan]) -> Option<DetectedTable> {
    if spans.len() < 4 {
        return None;
    }

    let mut sorted: Vec<&TextSpan> = spans.to_vec();
    sorted.sort_by(|a, b| {
        a.bbox
            .center_y()
            .total_cmp(&b.bbox.center_y())
            .then_with(|| a.bbox.x0.total_cmp(&b.bbox.x0))
    });

    // Cluster into visual rows
    let mut rows: Vec<Vec<&TextSpan>> = Vec::new();
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
    if rows.len() < 2 {
        return None;
    }

    // Build cell fragments within each row (merging intra-cell words, splitting column gaps >= 8pt)
    let mut row_cells: Vec<Vec<(f32, f32, String)>> = Vec::with_capacity(rows.len());
    for row in &mut rows {
        row.sort_by(|a, b| a.bbox.x0.total_cmp(&b.bbox.x0));
        let mut cells: Vec<(f32, f32, String)> = Vec::new();
        for s in row.iter() {
            let trimmed = s.text.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(last) = cells.last_mut() {
                let gap = s.bbox.x0 - last.1;
                if gap < (s.font_size * 0.95).clamp(7.0, 14.0) {
                    last.1 = last.1.max(s.bbox.x1);
                    if should_insert_span_space(&last.2, trimmed, gap, s.font_size) {
                        last.2.push(' ');
                    }
                    last.2.push_str(trimmed);
                    continue;
                }
            }
            cells.push((s.bbox.x0, s.bbox.x1, trimmed.to_string()));
        }
        if !cells.is_empty() {
            row_cells.push(cells);
        }
    }
    if row_cells.len() < 2 {
        return None;
    }

    // 1D vertical projection: cluster overlapping cell X-intervals across all rows
    let col_intervals = cluster_column_intervals(&row_cells, 4.0);
    if col_intervals.len() < 2 {
        return None;
    }

    // Guard against prose paragraphs framed by horizontal rules: require >= 60% of rows
    // to occupy at least 2 distinct columns.
    let multi_col_rows = row_cells
        .iter()
        .filter(|r| {
            let mut cols_hit = vec![false; col_intervals.len()];
            for cell in *r {
                let c_idx = assign_column_index(&col_intervals, cell.0, cell.1);
                cols_hit[c_idx] = true;
            }
            cols_hit.into_iter().filter(|&b| b).count() >= 2
        })
        .count();
    if multi_col_rows * 5 < row_cells.len() * 3 {
        return None;
    }

    let mut grid = vec![vec![String::new(); col_intervals.len()]; row_cells.len()];
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

    let markdown = format_gfm_table(&grid);
    Some(DetectedTable {
        bbox,
        rows: final_rows,
        cols: final_cols,
        cells: grid,
        markdown,
    })
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
            (a.coord - b.coord).abs() <= tol && a.start <= b.end + tol && a.end >= b.start - tol
        }
        (false, false) => {
            (a.coord - b.coord).abs() <= tol && a.start <= b.end + tol && a.end >= b.start - tol
        }
    }
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
                a.bbox
                    .y0
                    .total_cmp(&b.bbox.y0)
                    .then_with(|| a.bbox.x0.total_cmp(&b.bbox.x0))
            });
            let mut row_start = 0;
            while row_start < cell_spans.len() {
                let ref_y = cell_spans[row_start].bbox.y0;
                let mut row_end = row_start + 1;
                while row_end < cell_spans.len()
                    && (cell_spans[row_end].bbox.y0 - ref_y).abs() <= 3.0
                {
                    row_end += 1;
                }
                if row_end - row_start > 1 {
                    cell_spans[row_start..row_end].sort_by(|a, b| a.bbox.x0.total_cmp(&b.bbox.x0));
                }
                row_start = row_end;
            }
            let mut text = String::new();
            let mut prev_span: Option<&TextSpan> = None;
            for s in &cell_spans {
                let piece = s.text.trim();
                if piece.is_empty() {
                    continue;
                }
                if let Some(prev) = prev_span {
                    let same_line = (s.bbox.y0 - prev.bbox.y0).abs() <= 3.0;
                    if same_line {
                        let gap = s.bbox.x0 - prev.bbox.x1;
                        let ref_fs = s.font_size.min(prev.font_size);
                        if should_insert_span_space(&text, piece, gap, ref_fs) {
                            text.push(' ');
                        }
                    } else {
                        let (last_c, first_c) = (text.chars().last(), piece.chars().next());
                        let both_cjk = matches!(
                            (last_c, first_c),
                            (Some(a), Some(b))
                                if crate::layout::columns::is_cjk_char(a)
                                    && crate::layout::columns::is_cjk_char(b)
                        );
                        if !both_cjk && !text.ends_with(' ') {
                            text.push(' ');
                        }
                    }
                }
                text.push_str(piece);
                prev_span = Some(s);
            }
            if !text.is_empty() {
                non_empty_cells += 1;
            }
            grid[r][c] = text;
        }
    }

    if non_empty_cells < 2 {
        return None;
    }

    let grid = prune_empty_rows_and_cols(grid);
    let final_rows = grid.len();
    let final_cols = grid.first().map(|r| r.len()).unwrap_or(0);
    if final_rows == 0 || final_cols == 0 || (final_rows == 1 && final_cols == 1) {
        return None;
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
