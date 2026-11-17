use std::collections::HashMap;
use crate::config::Rect;

/// Uniform 2D spatial grid index providing O(1) average-case bounding-box intersection queries,
/// eliminating O(N^2) span-to-link, span-to-table, and graphic-clustering scans on dense PDFs.
pub struct SpatialGrid {
    cell_size: f32,
    cells: HashMap<(i32, i32), Vec<usize>>,
}

impl SpatialGrid {
    pub fn new(cell_size: f32) -> Self {
        Self {
            cell_size: cell_size.max(8.0),
            cells: HashMap::new(),
        }
    }

    pub fn insert(&mut self, id: usize, rect: Rect) {
        let gx0 = (rect.x0 / self.cell_size).floor() as i32;
        let gy0 = (rect.y0 / self.cell_size).floor() as i32;
        let gx1 = (rect.x1 / self.cell_size).floor() as i32;
        let gy1 = (rect.y1 / self.cell_size).floor() as i32;
        for gx in gx0..=gx1 {
            for gy in gy0..=gy1 {
                self.cells.entry((gx, gy)).or_default().push(id);
            }
        }
    }

    pub fn query_rect(&self, rect: Rect) -> Vec<usize> {
        let gx0 = (rect.x0 / self.cell_size).floor() as i32;
        let gy0 = (rect.y0 / self.cell_size).floor() as i32;
        let gx1 = (rect.x1 / self.cell_size).floor() as i32;
        let gy1 = (rect.y1 / self.cell_size).floor() as i32;
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
        let gx = (x / self.cell_size).floor() as i32;
        let gy = (y / self.cell_size).floor() as i32;
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
