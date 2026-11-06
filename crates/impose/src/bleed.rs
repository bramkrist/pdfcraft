//! Generated bleed: a page grown outward past its edge so the artwork reaches beyond the trim.
//!
//! Both methods are placements of the original page (see [`pdfcraft_print::Placement`]): nothing
//! is rasterised and the page content, its images and its fonts are not rewritten.

use pdfcraft_content::Matrix;
use pdfcraft_print::{Placement, Sheet};

use crate::{ImposeError, check_sheet_count, non_negative, positive};

/// How the new margin is filled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BleedMethod {
    /// Each edge strip is reflected outward across the edge, so the artwork continues as its own
    /// mirror image (the usual way to cover a bleed behind a fine edge detail).
    #[default]
    Mirror,
    /// The whole page is enlarged, about its centre, until it covers the page plus the bleed. The
    /// trim edge then crops slightly into the artwork.
    Scale,
}

/// Bleed to add outside the edge of every page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AddBleed {
    /// Left, bottom, right and top amounts, in points.
    pub amount: [f64; 4],
    pub method: BleedMethod,
}

impl AddBleed {
    /// The same bleed on all four sides.
    #[must_use]
    pub fn uniform(amount: f64, method: BleedMethod) -> Self {
        AddBleed { amount: [amount; 4], method }
    }
}

fn place(page: usize, m: [f64; 6], clip: [f64; 4]) -> Placement {
    Placement { page, matrix: Matrix(m), clip }
}

/// One sheet per page: the page with its bleed, whose `TrimBox` is the original page edge.
pub fn add_bleed(sizes: &[(f64, f64)], spec: &AddBleed) -> Result<Vec<Sheet>, ImposeError> {
    if sizes.is_empty() {
        return Err(ImposeError::NoPages);
    }
    check_sheet_count(sizes.len())?;
    let [bl, bb, br, bt] = spec.amount;
    for (v, what) in [(bl, "left bleed"), (bb, "bottom bleed"), (br, "right bleed"), (bt, "top bleed")] {
        non_negative(v, what)?;
    }
    if spec.amount.iter().all(|v| *v == 0.0) {
        return Err(ImposeError::Invalid("bleed", "greater than zero on at least one side"));
    }
    let mut out = Vec::with_capacity(sizes.len());
    for (page, &(w, h)) in sizes.iter().enumerate() {
        positive(w, "page width")?;
        positive(h, "page height")?;
        let (sw, sh) = (bl + w + br, bb + h + bt);
        let trim = [bl, bb, bl + w, bb + h];
        let placed = match spec.method {
            BleedMethod::Mirror => {
                // A reflected strip can be no wider than the page it is reflected from.
                if bl >= w || br >= w || bb >= h || bt >= h {
                    return Err(ImposeError::Invalid("bleed", "smaller than the page it is added to"));
                }
                let mut v = vec![place(page, [1.0, 0.0, 0.0, 1.0, bl, bb], [0.0, 0.0, w, h])];
                let x = |far: bool| if far { bl + 2.0 * w } else { bl };
                let y = |far: bool| if far { bb + 2.0 * h } else { bb };
                if bl > 0.0 {
                    v.push(place(page, [-1.0, 0.0, 0.0, 1.0, x(false), bb], [0.0, 0.0, bl, h]));
                }
                if br > 0.0 {
                    v.push(place(page, [-1.0, 0.0, 0.0, 1.0, x(true), bb], [w - br, 0.0, w, h]));
                }
                if bb > 0.0 {
                    v.push(place(page, [1.0, 0.0, 0.0, -1.0, bl, y(false)], [0.0, 0.0, w, bb]));
                }
                if bt > 0.0 {
                    v.push(place(page, [1.0, 0.0, 0.0, -1.0, bl, y(true)], [0.0, h - bt, w, h]));
                }
                // Corners: reflected across both edges, so they continue the neighbouring strips.
                if bl > 0.0 && bb > 0.0 {
                    v.push(place(page, [-1.0, 0.0, 0.0, -1.0, x(false), y(false)], [0.0, 0.0, bl, bb]));
                }
                if br > 0.0 && bb > 0.0 {
                    v.push(place(page, [-1.0, 0.0, 0.0, -1.0, x(true), y(false)], [w - br, 0.0, w, bb]));
                }
                if bl > 0.0 && bt > 0.0 {
                    v.push(place(page, [-1.0, 0.0, 0.0, -1.0, x(false), y(true)], [0.0, h - bt, bl, h]));
                }
                if br > 0.0 && bt > 0.0 {
                    v.push(place(page, [-1.0, 0.0, 0.0, -1.0, x(true), y(true)], [w - br, h - bt, w, h]));
                }
                v
            }
            BleedMethod::Scale => {
                let k = (sw / w).max(sh / h);
                if !(k.is_finite() && k > 0.0) {
                    return Err(ImposeError::Invalid("bleed", "a finite amount"));
                }
                // The enlarged page is centred on the grown page, then cut to the sheet.
                let (ox, oy) = ((sw - w * k) / 2.0, (sh - h * k) / 2.0);
                let clip = [(-ox / k).max(0.0), (-oy / k).max(0.0), ((sw - ox) / k).min(w), ((sh - oy) / k).min(h)];
                vec![place(page, [k, 0.0, 0.0, k, ox, oy], clip)]
            }
        };
        out.push(Sheet { size: (sw, sh), placed, trim: Some(trim), ..Sheet::default() });
    }
    Ok(out)
}
