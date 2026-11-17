use std::collections::HashMap;
use crate::config::Rect;

/// Uniform 2D spatial grid index providing O(1) average-case bounding-box intersection queries,
/// eliminating O(N^2) span-to-link, span-to-table, and graphic-clustering scans on dense PDFs.
pub struct SpatialGrid {
    cell_size: f32,
    cells: HashMap<(i32, i32), Vec<usize>>,
}

const MIN_GRID_CELL: f32 = -1024.0;
const MAX_GRID_CELL: f32 = 4096.0;
const MAX_INSERT_CELLS: i64 = 16_384;

impl SpatialGrid {
    pub fn new(cell_size: f32) -> Self {
        Self {
            cell_size: cell_size.max(8.0),
            cells: HashMap::new(),
        }
    }

    #[inline]
    fn cell_bounds(&self, rect: Rect) -> Option<(i32, i32, i32, i32)> {
        if !(rect.x0.is_finite()
            && rect.y0.is_finite()
            && rect.x1.is_finite()
            && rect.y1.is_finite())
        {
            return None;
        }
        let gx0 = (rect.x0 / self.cell_size)
            .floor()
            .clamp(MIN_GRID_CELL, MAX_GRID_CELL) as i32;
        let gy0 = (rect.y0 / self.cell_size)
            .floor()
            .clamp(MIN_GRID_CELL, MAX_GRID_CELL) as i32;
        let gx1 = (rect.x1 / self.cell_size)
            .floor()
            .clamp(MIN_GRID_CELL, MAX_GRID_CELL) as i32;
        let gy1 = (rect.y1 / self.cell_size)
            .floor()
            .clamp(MIN_GRID_CELL, MAX_GRID_CELL) as i32;
        Some((gx0.min(gx1), gy0.min(gy1), gx0.max(gx1), gy0.max(gy1)))
    }

    pub fn insert(&mut self, id: usize, rect: Rect) {
        let Some((gx0, gy0, gx1, gy1)) = self.cell_bounds(rect) else {
            return;
        };
        let span_x = (gx1 - gx0) as i64 + 1;
        let span_y = (gy1 - gy0) as i64 + 1;
        if span_x.saturating_mul(span_y) > MAX_INSERT_CELLS {
            return;
        }
        for gx in gx0..=gx1 {
            for gy in gy0..=gy1 {
                self.cells.entry((gx, gy)).or_default().push(id);
            }
        }
    }

    pub fn query_rect(&self, rect: Rect) -> Vec<usize> {
        let Some((gx0, gy0, gx1, gy1)) = self.cell_bounds(rect) else {
            return Vec::new();
        };
        let span_x = (gx1 - gx0) as i64 + 1;
        let span_y = (gy1 - gy0) as i64 + 1;
        if span_x.saturating_mul(span_y) > MAX_INSERT_CELLS {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for gx in gx0..=gx1 {
            for gy in gy0..=gy1 {
                if let Some(bucket) = self.cells.get(&(gx, gy)) {
                    hits.extend_from_slice(bucket);
                }
            }
        }
        hits.sort_unstable();
        hits.dedup();
        hits
    }

    pub fn query_point(&self, x: f32, y: f32) -> &[usize] {
        if !x.is_finite() || !y.is_finite() {
            return &[];
        }
        let gx = (x / self.cell_size)
            .floor()
            .clamp(MIN_GRID_CELL, MAX_GRID_CELL) as i32;
        let gy = (y / self.cell_size)
            .floor()
            .clamp(MIN_GRID_CELL, MAX_GRID_CELL) as i32;
        self.cells.get(&(gx, gy)).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// Disjoint-set (Union-Find) with path compression and union by rank for O(alpha(N))
/// connected-component segmentation of multiple independent tables and graphic clusters per page.
pub struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl UnionFind {
    pub fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    pub fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    pub fn union(&mut self, a: usize, b: usize) {
        let mut ra = self.find(a);
        let mut rb = self.find(b);
        if ra == rb {
            return;
        }
        if self.rank[ra] < self.rank[rb] {
            std::mem::swap(&mut ra, &mut rb);
        }
        self.parent[rb] = ra;
        if self.rank[ra] == self.rank[rb] {
            self.rank[ra] += 1;
        }
    }
}
