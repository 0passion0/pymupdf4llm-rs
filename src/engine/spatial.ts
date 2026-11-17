import type { Rect } from './types';

/**
 * Uniform 2D spatial grid index providing O(1) average-case bounding-box intersection
 * queries for span-to-link, span-to-table, and graphic-clustering lookups.
 */
export class SpatialGrid {
  private readonly cellSize: number;
  private readonly cells = new Map<string, number[]>();

  constructor(cellSize = 32) {
    this.cellSize = Math.max(8, cellSize);
  }

  private key(gx: number, gy: number): string {
    return `${gx}:${gy}`;
  }

  insert(id: number, rect: Rect): void {
    const gx0 = Math.floor(rect.x0 / this.cellSize);
    const gy0 = Math.floor(rect.y0 / this.cellSize);
    const gx1 = Math.floor(rect.x1 / this.cellSize);
    const gy1 = Math.floor(rect.y1 / this.cellSize);
    for (let gx = gx0; gx <= gx1; gx++) {
      for (let gy = gy0; gy <= gy1; gy++) {
        const k = this.key(gx, gy);
        const bucket = this.cells.get(k);
        if (bucket) bucket.push(id);
        else this.cells.set(k, [id]);
      }
    }
  }

  queryRect(rect: Rect): number[] {
    const gx0 = Math.floor(rect.x0 / this.cellSize);
    const gy0 = Math.floor(rect.y0 / this.cellSize);
    const gx1 = Math.floor(rect.x1 / this.cellSize);
    const gy1 = Math.floor(rect.y1 / this.cellSize);
    const seen = new Set<number>();
    for (let gx = gx0; gx <= gx1; gx++) {
      for (let gy = gy0; gy <= gy1; gy++) {
        const bucket = this.cells.get(this.key(gx, gy));
        if (bucket) {
          for (const id of bucket) seen.add(id);
        }
      }
    }
    return [...seen].sort((a, b) => a - b);
  }

  queryPoint(x: number, y: number): number[] {
    const gx = Math.floor(x / this.cellSize);
    const gy = Math.floor(y / this.cellSize);
    return this.cells.get(this.key(gx, gy)) ?? [];
  }
}

/**
 * Disjoint-set (Union-Find) with path halving and union by rank for O(alpha(N))
 * connected-component segmentation of multiple independent tables and graphic clusters per page.
 */
export class UnionFind {
  private readonly parent: Int32Array;
  private readonly rank: Uint8Array;

  constructor(n: number) {
    this.parent = new Int32Array(n);
    this.rank = new Uint8Array(n);
    for (let i = 0; i < n; i++) this.parent[i] = i;
  }

  find(x: number): number {
    let cur = x;
    while (this.parent[cur] !== cur) {
      this.parent[cur] = this.parent[this.parent[cur]];
      cur = this.parent[cur];
    }
    return cur;
  }

  union(a: number, b: number): void {
    let ra = this.find(a);
    let rb = this.find(b);
    if (ra === rb) return;
    if (this.rank[ra] < this.rank[rb]) {
      const tmp = ra;
      ra = rb;
      rb = tmp;
    }
    this.parent[rb] = ra;
    if (this.rank[ra] === this.rank[rb]) {
      this.rank[ra]++;
    }
  }
}
