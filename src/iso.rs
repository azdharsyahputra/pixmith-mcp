//! The isometric grid of the Pixmith app (Select > Isometric Tile): diamond cells of tile width `w` and height
//! `h` (default 32 x 16) and the exact pixel mask of one cell. Ported from the app's `ui/iso.rs`, which is not part
//! of `pixmith-core`.
//!
//! Layout. Cell (0, 0) has its bounding box at `(x, y)` with size `w` x `h`, so its corners are
//! `(x + w/2, y)`, `(x + w, y + h/2)`, `(x + w/2, y + h)` and `(x, y + h/2)`. Cell (i, j) is that diamond moved by
//! `(i - j) * w/2` across and `(i + j) * h/2` down; the cells tile the plane without gaps. A pixel belongs to the
//! cell its center lies in (a center on a border goes to the cell below or to the right), so a 32 x 16 cell has
//! `w * h / 2` = 256 pixels.

pub const DEFAULT_W: i32 = 32;
pub const DEFAULT_H: i32 = 16;
pub const MIN_SIDE: i32 = 4;
pub const MAX_SIDE: i32 = 4096;

/// A sanitized isometric grid: even sizes of at least 4 and the offset of cell (0, 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Iso {
    pub w: i64,
    pub h: i64,
    pub x: i64,
    pub y: i64,
}

/// Rounds a tile side to the nearest allowed value: even, 4 to 4096.
pub fn clamp_side(v: i32) -> i32 {
    (v.clamp(MIN_SIDE, MAX_SIDE) + 1) & !1
}

impl Iso {
    pub fn new(w: i32, h: i32, x: i32, y: i32) -> Iso {
        Iso { w: clamp_side(w) as i64, h: clamp_side(h) as i64, x: x as i64, y: y as i64 }
    }

    /// The cell `(i, j)` that contains pixel `(px, py)`.
    pub fn cell_at(&self, px: i32, py: i32) -> (i64, i64) {
        let (w, h) = (self.w, self.h);
        // Doubled coordinates of the pixel's center, relative to the center of the top corner of cell (0, 0).
        let dx = 2 * px as i64 + 1 - 2 * self.x - w;
        let dy = 2 * py as i64 + 1 - 2 * self.y;
        // a = dx / w and b = dy / (h/2) are the coordinates in units of half a tile; cell (i, j) has its top
        // corner at a = i - j, b = i + j, so i and j are the rounded half sums and differences.
        let (sum, diff) = (h * dx + w * (dy - h) + w * h, w * (dy - h) - h * dx + w * h);
        (sum.div_euclid(2 * w * h), diff.div_euclid(2 * w * h))
    }

    /// The pixel rows `(y, left, right)` (inclusive) of cell `(i, j)`, top to bottom; rows without a pixel are left out.
    pub fn cell_rows(&self, cell: (i64, i64)) -> Vec<(i32, i32, i32)> {
        let (i, j) = cell;
        let left = self.x + (i - j) * self.w / 2;
        let top = self.y + (i + j) * self.h / 2;
        let mut rows = Vec::new();
        for y in top..top + self.h {
            let mut span: Option<(i32, i32)> = None;
            for x in left..left + self.w {
                if self.cell_at(x as i32, y as i32) == cell {
                    span = Some(span.map_or((x as i32, x as i32), |(l, _)| (l, x as i32)));
                }
            }
            if let Some((l, r)) = span {
                rows.push((y as i32, l, r));
            }
        }
        rows
    }

    /// A closed polygon whose pixels (outline plus even-odd interior, as the core's polygon selection takes
    /// them) are exactly the cell's mask. The path runs down the left edge and back up the right one, turning
    /// inside the cell, so no outline pixel falls outside it.
    pub fn cell_polygon(&self, cell: (i64, i64)) -> Vec<(i32, i32)> {
        let rows = self.cell_rows(cell);
        let Some(&(y0, l0, _)) = rows.first() else { return Vec::new() };
        let mut pts = vec![(l0, y0)];
        for pair in rows.windows(2) {
            let ((yp, lp, _), (yr, lr, _)) = (pair[0], pair[1]);
            pts.push(if lr <= lp { (lp, yr) } else { (lr, yp) });
            pts.push((lr, yr));
        }
        let &(yb, _, rb) = rows.last().unwrap();
        pts.push((rb, yb));
        for pair in rows.windows(2).rev() {
            let ((yr, _, rr), (yq, _, rq)) = (pair[0], pair[1]);
            pts.push(if rr >= rq { (rq, yr) } else { (rr, yq) });
            pts.push((rr, yr));
        }
        pts.dedup();
        if pts.len() > 1 && pts.first() == pts.last() {
            pts.pop();
        }
        pts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn sides_are_even_and_at_least_four() {
        assert_eq!([clamp_side(0), clamp_side(5), clamp_side(32), clamp_side(99999)], [4, 6, 32, 4096]);
    }

    #[test]
    fn a_32_by_16_cell_is_the_classic_diamond() {
        let iso = Iso::new(32, 16, 0, 0);
        let rows = iso.cell_rows((0, 0));
        let widths: Vec<i32> = rows.iter().map(|&(_, l, r)| r - l + 1).collect();
        assert_eq!(widths, [2, 6, 10, 14, 18, 22, 26, 30, 30, 26, 22, 18, 14, 10, 6, 2]);
        assert_eq!(iso.cell_at(16, 8), (0, 0));
        assert_eq!(iso.cell_at(32, 16), (1, 0));
        assert_eq!(iso.cell_at(0, 16), (0, 1));
    }

    #[test]
    fn cells_partition_the_plane_and_follow_the_offset() {
        let iso = Iso::new(32, 16, 5, -3);
        let mut seen = HashSet::new();
        for cell in [(0, 0), (1, 0), (0, 1), (-1, 2), (2, 2)] {
            for (y, l, r) in iso.cell_rows(cell) {
                for x in l..=r {
                    assert!(seen.insert((x, y)));
                    assert_eq!(iso.cell_at(x, y), cell);
                }
            }
        }
    }
}
