//! The signed distance field of a silhouette (Felzenszwalb & Huttenlocher's exact Euclidean
//! distance transform), on a grid padded so the outline never reaches the edge.

use crate::{CutError, Holes, MAX_CELLS, Mask, TraceOptions};

/// The binary silhouette on its padded grid.
pub(crate) struct Grid {
    pub width: usize,
    pub height: usize,
    /// Row by row; true where the artwork is.
    pub inside: Vec<bool>,
}

impl Grid {
    /// `mask` thresholded and surrounded by `pad` empty cells on every side (holes filled in when
    /// `options` say so). Refused when the grid would pass [`MAX_CELLS`].
    pub(crate) fn new(mask: &Mask, pad: usize, options: &TraceOptions) -> Result<Self, CutError> {
        let margin = pad.checked_mul(2).ok_or(CutError::TooLarge)?;
        let width = mask.width().checked_add(margin).ok_or(CutError::TooLarge)?;
        let height = mask.height().checked_add(margin).ok_or(CutError::TooLarge)?;
        let cells = width.checked_mul(height).filter(|c| *c <= MAX_CELLS).ok_or(CutError::TooLarge)?;
        let threshold = options.threshold.max(1);
        let mut inside = vec![false; cells];
        for (row, source) in inside.chunks_exact_mut(width).skip(pad).zip(mask.rows()) {
            for (cell, coverage) in row.iter_mut().skip(pad).zip(source) {
                *cell = *coverage >= threshold;
            }
        }
        if options.holes == Holes::Fill {
            fill_holes(&mut inside, width, height);
        }
        Ok(Self { width, height, inside })
    }
}

/// Mark every empty cell that the outside cannot reach (4-connected) as artwork.
fn fill_holes(inside: &mut [bool], width: usize, height: usize) {
    // The padding makes the first cell empty and outside.
    if inside.first() != Some(&false) {
        return;
    }
    let mut reached = vec![false; inside.len()];
    if let Some(r) = reached.first_mut() {
        *r = true;
    }
    let mut stack = vec![0usize];
    while let Some(i) = stack.pop() {
        let (x, y) = (i % width, i / width);
        let mut visit = |j: usize| {
            if inside.get(j) == Some(&false)
                && let Some(r) = reached.get_mut(j)
                && !*r
            {
                *r = true;
                stack.push(j);
            }
        };
        if x > 0 {
            visit(i - 1);
        }
        if x + 1 < width {
            visit(i + 1);
        }
        if y > 0 {
            visit(i - width);
        }
        if y + 1 < height {
            visit(i + width);
        }
    }
    for (cell, reached) in inside.iter_mut().zip(&reached) {
        if !*reached {
            *cell = true;
        }
    }
}

/// The signed distance of every cell centre from the silhouette's edge: negative inside,
/// positive outside, zero half-way between an artwork cell and an empty one. `None` when
/// there is no artwork.
pub(crate) fn signed_distance(grid: &Grid) -> Option<Vec<f32>> {
    if !grid.inside.iter().any(|&b| b) {
        return None;
    }
    let mut out = vec![0.0f32; grid.inside.len()];
    let to_artwork = squared_distance(&grid.inside, grid.width, grid.height);
    for ((o, d), inside) in out.iter_mut().zip(&to_artwork).zip(&grid.inside) {
        if !*inside {
            *o = d.sqrt() - 0.5;
        }
    }
    drop(to_artwork);
    let outside: Vec<bool> = grid.inside.iter().map(|b| !b).collect();
    let to_empty = squared_distance(&outside, grid.width, grid.height);
    for ((o, d), inside) in out.iter_mut().zip(&to_empty).zip(&grid.inside) {
        if *inside {
            *o = 0.5 - d.sqrt();
        }
    }
    Some(out)
}

/// Stands for "no site anywhere" in the transform; finite so the arithmetic stays defined.
const FAR: f32 = 1.0e18;

/// For every cell, the squared distance to the nearest cell of `sites`.
fn squared_distance(sites: &[bool], width: usize, height: usize) -> Vec<f32> {
    let mut f: Vec<f32> = sites.iter().map(|&s| if s { 0.0 } else { FAR }).collect();
    let mut scratch = Scratch::default();
    let mut line = vec![0.0f32; height];
    let mut result = vec![0.0f32; height];
    for x in 0..width {
        for (l, v) in line.iter_mut().zip(f.iter().skip(x).step_by(width.max(1))) {
            *l = *v;
        }
        transform_line(&line, &mut result, &mut scratch);
        for (v, r) in f.iter_mut().skip(x).step_by(width.max(1)).zip(&result) {
            *v = *r;
        }
    }
    let mut line = vec![0.0f32; width];
    let mut result = vec![0.0f32; width];
    for row in f.chunks_exact_mut(width.max(1)) {
        line.copy_from_slice(row);
        transform_line(&line, &mut result, &mut scratch);
        row.copy_from_slice(&result);
    }
    f
}

#[derive(Default)]
struct Scratch {
    /// Positions of the parabolas on the lower envelope.
    v: Vec<usize>,
    /// Where each envelope parabola starts to be the lowest.
    z: Vec<f32>,
}

/// The one-dimensional squared distance transform: `out[q] = min over p of (q - p)² + f[p]`.
fn transform_line(f: &[f32], out: &mut [f32], s: &mut Scratch) {
    let at = |i: usize| f.get(i).copied().unwrap_or(FAR);
    let sq = |i: usize| (i as f32) * (i as f32);
    s.v.clear();
    s.z.clear();
    if f.is_empty() {
        return;
    }
    s.v.push(0);
    s.z.push(f32::NEG_INFINITY);
    s.z.push(f32::INFINITY);
    let mut k = 0usize;
    for q in 1..f.len() {
        loop {
            let p = s.v.get(k).copied().unwrap_or(0);
            let cross = ((at(q) + sq(q)) - (at(p) + sq(p))) / (2.0 * (q as f32 - p as f32));
            if k > 0 && cross <= s.z.get(k).copied().unwrap_or(f32::INFINITY) {
                k -= 1;
                continue;
            }
            k += 1;
            s.v.truncate(k);
            s.z.truncate(k);
            s.v.push(q);
            s.z.push(cross);
            s.z.push(f32::INFINITY);
            break;
        }
    }
    let mut k = 0usize;
    for (q, o) in out.iter_mut().enumerate() {
        while s.z.get(k + 1).is_some_and(|z| *z < q as f32) {
            k += 1;
        }
        let p = s.v.get(k).copied().unwrap_or(0);
        let d = q as f32 - p as f32;
        *o = d * d + at(p);
    }
}
