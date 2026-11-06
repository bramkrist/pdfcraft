//! Manual imposition: every page placed by hand, as in a press room's manual impose. Each
//! placement names a source page, where its lower-left corner goes on the sheet and how many
//! quarter turns it makes; trim marks are worked out from the placed pages the same way as for
//! step & repeat. Pages are still placed, never redrawn.

use pdfcraft_content::Matrix;
use pdfcraft_print::{Placement, Sheet};

use super::{EPS, ImposeError, MAX_CELLS_PER_SHEET, MAX_SHEETS, Marks, Rect, non_negative, positive, trim_marks};

/// One page placed by hand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ManualPage {
    /// 0-based source page.
    pub page: usize,
    /// Lower-left corner of the page's footprint on the sheet (after turning), bleed included.
    pub at: (f64, f64),
    /// Quarter turns anticlockwise, 0 to 3 (other values are taken modulo 4).
    pub turns: u8,
}

/// A manual layout: the sheet, the bleed inside every page, marks, and the pages on each sheet
/// side in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Manual {
    pub sheet: (f64, f64),
    /// Bleed contained in each page's size, on every side; the trim edge lies this far inside.
    pub bleed: f64,
    pub marks: Option<Marks>,
    pub sheets: Vec<Vec<ManualPage>>,
}

/// The placement of `size` turned `turns` quarter turns anticlockwise with its footprint's
/// lower-left corner at `at`, and that footprint.
fn place(page: usize, size: (f64, f64), at: (f64, f64), turns: u8) -> (Placement, Rect) {
    let (w, h) = size;
    let (ox, oy) = at;
    let (m, fw, fh) = match turns % 4 {
        1 => ([0.0, 1.0, -1.0, 0.0, ox + h, oy], h, w),
        2 => ([-1.0, 0.0, 0.0, -1.0, ox + w, oy + h], w, h),
        3 => ([0.0, -1.0, 1.0, 0.0, ox, oy + w], h, w),
        _ => ([1.0, 0.0, 0.0, 1.0, ox, oy], w, h),
    };
    (Placement { page, matrix: Matrix(m), clip: [0.0, 0.0, w, h] }, [ox, oy, ox + fw, oy + fh])
}

fn validate(sizes: &[(f64, f64)], spec: &Manual) -> Result<(), ImposeError> {
    if sizes.is_empty() {
        return Err(ImposeError::NoPages);
    }
    positive(spec.sheet.0, "sheet width")?;
    positive(spec.sheet.1, "sheet height")?;
    non_negative(spec.bleed, "bleed")?;
    if let Some(m) = spec.marks {
        positive(m.length, "mark length")?;
        positive(m.width, "mark width")?;
        if !m.offset.is_finite() || m.offset + m.length <= 0.0 {
            return Err(ImposeError::Invalid("mark offset", "finite, and leaving part of the mark outside the trim"));
        }
    }
    if spec.sheets.is_empty() {
        return Err(ImposeError::Invalid("manual layout", "with at least one sheet"));
    }
    if spec.sheets.len() > MAX_SHEETS {
        return Err(ImposeError::TooLarge("more sheets than are supported"));
    }
    for side in &spec.sheets {
        if side.len() > MAX_CELLS_PER_SHEET {
            return Err(ImposeError::TooLarge("more cells on one sheet than are supported"));
        }
        for p in side {
            let &(w, h) = sizes.get(p.page).ok_or(ImposeError::NoSuchPage { page: p.page.saturating_add(1), pages: sizes.len() })?;
            if !p.at.0.is_finite() || !p.at.1.is_finite() {
                return Err(ImposeError::Invalid("page position", "on the sheet"));
            }
            if spec.bleed * 2.0 >= w || spec.bleed * 2.0 >= h {
                return Err(ImposeError::Invalid("bleed", "less than half of the smallest page side"));
            }
            let (_, f) = place(p.page, (w, h), p.at, p.turns);
            if f[2] <= EPS || f[3] <= EPS || f[0] >= spec.sheet.0 - EPS || f[1] >= spec.sheet.1 - EPS {
                return Err(ImposeError::OffSheet { page: p.page.saturating_add(1) });
            }
        }
    }
    Ok(())
}

/// The sheets of a manual layout of pages of `sizes` (display sizes, in order).
pub fn manual(sizes: &[(f64, f64)], spec: &Manual) -> Result<Vec<Sheet>, ImposeError> {
    validate(sizes, spec)?;
    let mut out = Vec::with_capacity(spec.sheets.len());
    for side in &spec.sheets {
        let mut sheet = Sheet { size: spec.sheet, ..Sheet::default() };
        let mut trims = Vec::with_capacity(side.len());
        for p in side {
            let Some(&size) = sizes.get(p.page) else { continue };
            let (placement, f) = place(p.page, size, p.at, p.turns);
            sheet.placed.push(placement);
            let b = spec.bleed;
            trims.push([f[0] + b, f[1] + b, f[2] - b, f[3] - b]);
        }
        if let Some(marks) = spec.marks {
            sheet.lines = trim_marks(&trims, spec.sheet, spec.bleed, marks);
            sheet.marks_paint = pdfcraft_print::MarkPaint { width: marks.width, registration: marks.registration };
        }
        out.push(sheet);
    }
    Ok(out)
}
