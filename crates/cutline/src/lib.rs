//! pdfcraft-cutline — L0 cut-contour tracing for die-cut and kiss-cut stickers.
//!
//! Given a raster silhouette ([`Mask`]: how much of each pixel the artwork covers) [`trace`]
//! returns the closed outlines of the shape, grown or shrunk by an exact distance
//! ([`TraceOptions::offset`]). The three usual cut lines are one number:
//! - `0`: the cut follows the artwork,
//! - `> 0`: the cut lies that far outside it (a safety margin around the print),
//! - `< 0`: the cut lies that far inside it (to trim a white edge off).
//!
//! The method is a signed Euclidean distance field of the silhouette, contoured at the offset
//! with marching squares. The offset is therefore a true distance (round at convex corners, as a
//! cutter's offset is), exact along straight edges and within about a fifth of a pixel at the
//! diagonal of a corner, and it copes with holes, islands, concavities and shapes that merge or
//! vanish under the offset without any polygon clipping. Written from the public
//! description of the algorithms (Felzenszwalb & Huttenlocher's distance transform, marching
//! squares); nothing is taken from Potrace or any other GPL code.
//!
//! Coordinates are mask pixels: x to the right, y down, pixel `(i, j)` covering
//! `[i, i + 1] × [j, j + 1]`. With a positive offset the outline may lie outside the mask.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod field;
mod march;
mod mask;
mod simplify;
#[cfg(test)]
mod tests;

use std::fmt;

pub use mask::{Background, Mask};

/// The most grid cells (mask pixels plus the padding an offset needs) one trace works on. It
/// bounds the memory: about 8 bytes a cell at the peak.
pub const MAX_CELLS: usize = 16_000_000;

/// The largest offset, in pixels, that [`trace`] accepts.
pub const MAX_OFFSET: f64 = 4096.0;

/// Why a trace was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CutError {
    /// The mask is empty or its pixel count does not match its size.
    BadMask,
    /// A number was NaN or infinite.
    NotFinite,
    /// The mask (with the padding the offset needs) is over [`MAX_CELLS`], or the offset is over
    /// [`MAX_OFFSET`]: trace at a lower resolution.
    TooLarge,
}

impl fmt::Display for CutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CutError::BadMask => f.write_str("the silhouette is empty or its size does not match its pixels"),
            CutError::NotFinite => f.write_str("a cut-line setting is not a finite number"),
            CutError::TooLarge => f.write_str("the silhouette is too large to trace at this resolution"),
        }
    }
}

impl std::error::Error for CutError {}

/// What to do with the empty areas inside the artwork (the counter of an O, a ring).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Holes {
    /// Cut the outside only: holes are filled in before the offset, so none is cut.
    #[default]
    Fill,
    /// Cut around every hole as well.
    Keep,
}

/// How [`trace`] works.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceOptions {
    /// The distance, in pixels, the outline is moved from the silhouette's edge: positive
    /// outwards, negative inwards.
    pub offset: f64,
    /// Coverage (0–255) from which a pixel counts as part of the artwork.
    pub threshold: u8,
    pub holes: Holes,
    /// Outlines enclosing less than this many square pixels are dropped (specks and pinholes).
    pub min_area: f64,
    /// The most, in pixels, a simplified outline may stray from the traced one.
    pub tolerance: f64,
}

impl Default for TraceOptions {
    fn default() -> Self {
        Self { offset: 0.0, threshold: 128, holes: Holes::Fill, min_area: 4.0, tolerance: 0.35 }
    }
}

/// One closed outline: the points in order, the last joined to the first (not repeated).
#[derive(Clone, Debug, PartialEq)]
pub struct Loop {
    pub points: Vec<(f64, f64)>,
    /// The outline of a hole in the shape, not of the shape itself.
    pub hole: bool,
}

impl Loop {
    /// The enclosed area in square pixels.
    pub fn area(&self) -> f64 {
        simplify::signed_area(&self.points).abs()
    }
}

/// The outlines of `mask`'s silhouette moved by `options.offset`: the outer ones first, then
/// the holes (see [`Holes`]). Empty when nothing of the silhouette is left.
///
/// Never panics; the work and memory are bounded by [`MAX_CELLS`].
pub fn trace(mask: &Mask, options: &TraceOptions) -> Result<Vec<Loop>, CutError> {
    if !options.offset.is_finite() || !options.min_area.is_finite() || !options.tolerance.is_finite() {
        return Err(CutError::NotFinite);
    }
    if options.offset.abs() > MAX_OFFSET {
        return Err(CutError::TooLarge);
    }
    let pad = padding(options.offset);
    let grid = field::Grid::new(mask, pad, options)?;
    let Some(signed) = field::signed_distance(&grid) else { return Ok(Vec::new()) };
    let raw = march::loops(&signed, grid.width, options.offset as f32);
    let shift = pad as f64 - 0.5;
    let mut outer = Vec::new();
    let mut holes = Vec::new();
    for lattice in raw {
        let points: Vec<(f64, f64)> = lattice.iter().map(|&(x, y)| (f64::from(x) - shift, f64::from(y) - shift)).collect();
        let area = simplify::signed_area(&points);
        if area.abs() < options.min_area {
            continue;
        }
        let points = simplify::closed(&points, options.tolerance.max(0.0));
        if points.len() < 3 {
            continue;
        }
        // Marching squares walks with the artwork on the left: an outer outline then has a
        // negative shoelace area in these y-down coordinates, a hole a positive one.
        let hole = area > 0.0;
        if hole { holes.push(Loop { points, hole }) } else { outer.push(Loop { points, hole }) }
    }
    outer.append(&mut holes);
    Ok(outer)
}

/// The border of empty cells added around the mask so the outline never touches the grid's
/// edge: room for a positive offset, and a margin for the interpolation.
fn padding(offset: f64) -> usize {
    // `offset` is finite and at most MAX_OFFSET here, so the cast cannot saturate.
    (offset.max(0.0).ceil() as usize).saturating_add(2)
}
