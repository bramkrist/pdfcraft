//! Booklet imposition: two pages side by side on every sheet side, in the order that makes a
//! folded and stitched stack read 1, 2, 3 ...
//!
//! The page order is plain integer arithmetic and the placement is one uniform scale and a
//! translation per page, so the page content itself is never touched.

use pdfcraft_content::Matrix;
use pdfcraft_print::{Placement, Sheet};

use crate::{ImposeError, check_sheet_count, non_negative, positive};

/// How the printed sheets are bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Binding {
    /// Pages simply run 1, 2, 3, 4 ...; two to a sheet side, never folded.
    Continuous,
    /// One signature folded in half: the first sheet carries the first and the last pages. A
    /// page count that is not a multiple of four is padded with blanks at the end.
    #[default]
    Saddle,
    /// A series of saddle-stitched signatures read one after another.
    Perfect {
        /// Pages per signature: a multiple of four, at least four.
        signature: usize,
        /// What the last signature is padded to.
        last: LastSignature,
    },
}

/// How a perfect-bound document's last, short signature is padded with blank pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LastSignature {
    /// To a full signature.
    Full,
    /// Only to a multiple of four pages.
    #[default]
    MultipleOfFour,
}

/// Where a page sits inside its half of the sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    /// Centred in its half.
    #[default]
    Centre,
    /// Pulled against the spine (the middle of the sheet).
    Spine,
}

/// What to make of a document's pages.
#[derive(Clone, Debug, PartialEq)]
pub struct Booklet {
    /// The flat sheet, or `None` for the smallest sheet that holds two of the largest page.
    pub sheet: Option<(f64, f64)>,
    pub binding: Binding,
    /// Scale pages up or down to fill their half of the sheet. Off: pages keep 100 % and the
    /// layout is refused when they do not fit.
    pub scale: bool,
    pub align: Align,
    /// Space left empty around the edge of the sheet.
    pub edge: f64,
    /// Bound on the right (right-to-left scripts): left and right pages swap places.
    pub right_to_left: bool,
}

impl Booklet {
    #[must_use]
    pub fn new() -> Self {
        Booklet { sheet: None, binding: Binding::Saddle, scale: true, align: Align::Centre, edge: 0.0, right_to_left: false }
    }
}

impl Default for Booklet {
    fn default() -> Self {
        Self::new()
    }
}

/// One sheet side: the pages on its left and right halves (`None` is a blank).
type Side = (Option<usize>, Option<usize>);

/// The sides of one saddle-stitched signature of `padded` pages (a multiple of four), whose
/// pages are `base + 0 ..` of which only the first `real` exist.
fn saddle_sides(base: usize, real: usize, padded: usize, out: &mut Vec<Side>) {
    let page = |i: usize| if i < real { Some(base + i) } else { None };
    for k in 0..padded / 4 {
        out.push((page(padded - 1 - 2 * k), page(2 * k)));
        out.push((page(2 * k + 1), page(padded - 2 - 2 * k)));
    }
}

fn ceil4(n: usize) -> usize {
    n.div_ceil(4) * 4
}

fn sides(pages: usize, binding: Binding) -> Result<Vec<Side>, ImposeError> {
    let mut out = Vec::new();
    match binding {
        Binding::Continuous => {
            for j in 0..pages.div_ceil(2) {
                let page = |i: usize| if i < pages { Some(i) } else { None };
                out.push((page(2 * j), page(2 * j + 1)));
            }
        }
        Binding::Saddle => saddle_sides(0, pages, ceil4(pages), &mut out),
        Binding::Perfect { signature, last } => {
            if signature < 4 || !signature.is_multiple_of(4) {
                return Err(ImposeError::Invalid("signature size", "a multiple of four, at least four"));
            }
            let mut base = 0;
            while base < pages {
                let real = (pages - base).min(signature);
                let padded = if real == signature || last == LastSignature::Full { signature } else { ceil4(real) };
                saddle_sides(base, real, padded, &mut out);
                base += signature;
            }
        }
    }
    Ok(out)
}

/// The sheet sides that turn `sizes` (the display size of every source page) into a booklet by
/// `spec`: for saddle and perfect binding, a front and a back for every sheet of paper, in order.
pub fn booklet(sizes: &[(f64, f64)], spec: &Booklet) -> Result<Vec<Sheet>, ImposeError> {
    if sizes.is_empty() {
        return Err(ImposeError::NoPages);
    }
    non_negative(spec.edge, "space at the edge of the sheet")?;
    let (mut max_w, mut max_h) = (0.0_f64, 0.0_f64);
    for &(w, h) in sizes {
        positive(w, "page width")?;
        positive(h, "page height")?;
        max_w = max_w.max(w);
        max_h = max_h.max(h);
    }
    let sheet = match spec.sheet {
        Some((w, h)) => {
            positive(w, "sheet width")?;
            positive(h, "sheet height")?;
            (w, h)
        }
        None => (2.0 * max_w + 2.0 * spec.edge, max_h + 2.0 * spec.edge),
    };
    let (half_w, area_h) = ((sheet.0 - 2.0 * spec.edge) / 2.0, sheet.1 - 2.0 * spec.edge);
    if !(half_w > 0.0 && area_h > 0.0) {
        return Err(ImposeError::Invalid("space at the edge of the sheet", "less than half of the sheet"));
    }
    // One scale for every page, taken from the largest, so pages keep their relative sizes.
    let scale = if spec.scale { (half_w / max_w).min(area_h / max_h) } else { 1.0 };
    if !(scale.is_finite() && scale > 0.0) {
        return Err(ImposeError::Invalid("scale", "greater than zero"));
    }
    if !spec.scale && (max_w > half_w + crate::EPS || max_h > area_h + crate::EPS) {
        return Err(ImposeError::DoesNotFit { cell_w: max_w, cell_h: max_h, area_w: half_w, area_h });
    }
    let all = sides(sizes.len(), spec.binding)?;
    check_sheet_count(all.len())?;

    let place = |page: usize, right_half: bool| -> Placement {
        // `sizes` was indexed by the same page numbers `sides` produced, all below `sizes.len()`;
        // `get` keeps that an invariant of the data rather than of a panic.
        let (w, h) = sizes.get(page).copied().unwrap_or((max_w, max_h));
        let (sw, sh) = (w * scale, h * scale);
        let half_x = spec.edge + if right_half { half_w } else { 0.0 };
        let ox = match (spec.align, right_half) {
            (Align::Centre, _) => half_x + (half_w - sw) / 2.0,
            (Align::Spine, false) => half_x + half_w - sw,
            (Align::Spine, true) => half_x,
        };
        let oy = spec.edge + (area_h - sh) / 2.0;
        Placement { page, matrix: Matrix([scale, 0.0, 0.0, scale, ox, oy]), clip: [0.0, 0.0, w, h] }
    };
    Ok(all
        .into_iter()
        .map(|(left, right)| {
            let (left, right) = if spec.right_to_left { (right, left) } else { (left, right) };
            let mut placed = Vec::with_capacity(2);
            placed.extend(left.map(|p| place(p, false)));
            placed.extend(right.map(|p| place(p, true)));
            Sheet { size: sheet, placed, borders: Vec::new(), lines: Vec::new() }
        })
        .collect())
}
