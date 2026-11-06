//! Print-production imposition (layer L4).
//!
//! Lays source pages out on press sheets and returns them as [`pdfcraft_print::Sheet`]s, which
//! `pdfcraft-print` then writes as a PDF. This crate is geometry only: it never reads or writes
//! PDF objects, so every layout rule is unit-testable with plain numbers.
//!
//! - **Step & repeat** ([`Fill::Repeat`]): each source page is repeated across a grid of cells
//!   (business cards, labels, flyers cut from a larger sheet).
//! - **Gang / n-up** ([`Fill::Sequential`]): consecutive source pages fill the grid in reading
//!   order, with gutters between cells.
//! - **Duplex** ([`Duplex`]): the back side of every sheet is laid out as the mirror image of the
//!   front, so a front cell and its back cell sit exactly behind each other once the sheet is
//!   turned over the long or the short edge.
//! - **Trim marks**: corner marks around every trimmed page, placed outside the bleed.
//!
//! All lengths are PDF points (1/72 inch). Sheet space has its origin at the lower left, y up.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use pdfcraft_content::Matrix;
use pdfcraft_cos::Document;
use pdfcraft_print::{Content, Placement, PrintError, Sheet};

/// Cells on one sheet that this crate will lay out, so a typo in a spec can't allocate without bound.
pub const MAX_CELLS_PER_SHEET: usize = 10_000;
/// Sheets that one call will produce.
pub const MAX_SHEETS: usize = 100_000;

/// Why a layout could not be made.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ImposeError {
    /// A length in the spec is negative, zero where a size is needed, or not a finite number.
    #[error("invalid {0}: it must be a finite number {1}")]
    Invalid(&'static str, &'static str),
    /// There are no source pages to lay out.
    #[error("there are no pages to impose")]
    NoPages,
    /// Not even one cell fits the usable area of the sheet.
    #[error("a cell of {cell_w:.2} x {cell_h:.2} pt does not fit the usable {area_w:.2} x {area_h:.2} pt of the sheet")]
    DoesNotFit { cell_w: f64, cell_h: f64, area_w: f64, area_h: f64 },
    /// The requested number of columns and rows is larger than the sheet can hold.
    #[error("{cols} x {rows} cells need {need_w:.2} x {need_h:.2} pt but the usable area is {area_w:.2} x {area_h:.2} pt")]
    GridTooBig { cols: usize, rows: usize, need_w: f64, need_h: f64, area_w: f64, area_h: f64 },
    /// The layout would be larger than [`MAX_CELLS_PER_SHEET`] or [`MAX_SHEETS`].
    #[error("the layout is too large: {0}")]
    TooLarge(&'static str),
}

/// How source pages fill the cells of a sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Fill {
    /// Every cell of a sheet shows the same page (step & repeat).
    #[default]
    Repeat,
    /// Consecutive pages fill the cells in reading order (n-up, ganging).
    Sequential,
}

/// How the printed sheet is turned over to print its back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Duplex {
    /// Turned over the long edge of the sheet (left and right swap).
    LongEdge,
    /// Turned over the short edge of the sheet (top and bottom swap).
    ShortEdge,
}

/// Distances from the sheet edge to the area that may carry pages.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Margins {
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub top: f64,
}

impl Margins {
    /// The same margin on all four sides.
    pub fn uniform(v: f64) -> Self {
        Self { left: v, bottom: v, right: v, top: v }
    }
}

/// Trim marks around each page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Marks {
    /// Length of each mark line.
    pub length: f64,
    /// Gap between the trim edge and the start of a mark. A mark never starts inside the bleed:
    /// the gap is at least [`StepRepeat::bleed`].
    pub offset: f64,
}

impl Default for Marks {
    fn default() -> Self {
        // 3 mm marks starting 3 mm from the trim, the usual press-room values.
        Self { length: 8.504, offset: 8.504 }
    }
}

/// The spec of a step-and-repeat or n-up layout.
#[derive(Clone, Debug, PartialEq)]
pub struct StepRepeat {
    /// Sheet width and height.
    pub sheet: (f64, f64),
    pub margins: Margins,
    pub fill: Fill,
    /// Columns; `None` fits as many as the sheet holds.
    pub cols: Option<usize>,
    /// Rows; `None` fits as many as the sheet holds.
    pub rows: Option<usize>,
    /// Space between neighbouring cells (horizontal, vertical).
    pub gutter: (f64, f64),
    /// Cell size; `None` uses the page size ([`Fill::Repeat`]) or the largest page ([`Fill::Sequential`]).
    pub cell: Option<(f64, f64)>,
    /// Bleed contained in each page's size, on every side. The trim edge lies this far inside.
    pub bleed: f64,
    /// Duplex printing: pages come in front/back sets and the back is mirrored. `None` is single-sided.
    pub duplex: Option<Duplex>,
    /// Trim marks; `None` draws none.
    pub marks: Option<Marks>,
}

impl StepRepeat {
    /// A single-sided step-and-repeat on `sheet` that fits as many copies as possible.
    pub fn new(sheet: (f64, f64)) -> Self {
        Self {
            sheet,
            margins: Margins::default(),
            fill: Fill::Repeat,
            cols: None,
            rows: None,
            gutter: (0.0, 0.0),
            cell: None,
            bleed: 0.0,
            duplex: None,
            marks: None,
        }
    }
}

/// A rectangle `[x0, y0, x1, y1]` in sheet space.
type Rect = [f64; 4];

const EPS: f64 = 1e-6;

fn positive(v: f64, what: &'static str) -> Result<(), ImposeError> {
    if v.is_finite() && v > 0.0 { Ok(()) } else { Err(ImposeError::Invalid(what, "greater than zero")) }
}

fn non_negative(v: f64, what: &'static str) -> Result<(), ImposeError> {
    if v.is_finite() && v >= 0.0 { Ok(()) } else { Err(ImposeError::Invalid(what, "zero or more")) }
}

fn validate(spec: &StepRepeat, sizes: &[(f64, f64)]) -> Result<(), ImposeError> {
    positive(spec.sheet.0, "sheet width")?;
    positive(spec.sheet.1, "sheet height")?;
    non_negative(spec.margins.left, "left margin")?;
    non_negative(spec.margins.bottom, "bottom margin")?;
    non_negative(spec.margins.right, "right margin")?;
    non_negative(spec.margins.top, "top margin")?;
    non_negative(spec.gutter.0, "horizontal gutter")?;
    non_negative(spec.gutter.1, "vertical gutter")?;
    non_negative(spec.bleed, "bleed")?;
    if let Some((w, h)) = spec.cell {
        positive(w, "cell width")?;
        positive(h, "cell height")?;
    }
    if let Some(m) = spec.marks {
        positive(m.length, "mark length")?;
        non_negative(m.offset, "mark offset")?;
    }
    for &(w, h) in sizes {
        positive(w, "page width")?;
        positive(h, "page height")?;
        if spec.bleed * 2.0 >= w || spec.bleed * 2.0 >= h {
            return Err(ImposeError::Invalid("bleed", "less than half of the smallest page side"));
        }
    }
    if spec.cols == Some(0) || spec.rows == Some(0) {
        return Err(ImposeError::Invalid("column or row count", "at least one"));
    }
    Ok(())
}

/// How many cells of `cell` plus `gap` fit in `room`.
fn fit_count(room: f64, cell: f64, gap: f64) -> usize {
    let n = ((room + gap) / (cell + gap) + EPS).floor();
    if n.is_finite() && n >= 1.0 { n as usize } else { 0 }
}

/// The grid of cell rectangles for `cell` size, row by row from the top, left to right.
fn grid(spec: &StepRepeat, cell: (f64, f64)) -> Result<Vec<Rect>, ImposeError> {
    let m = spec.margins;
    let area_w = spec.sheet.0 - m.left - m.right;
    let area_h = spec.sheet.1 - m.top - m.bottom;
    let fits_cols = fit_count(area_w, cell.0, spec.gutter.0);
    let fits_rows = fit_count(area_h, cell.1, spec.gutter.1);
    if fits_cols == 0 || fits_rows == 0 {
        return Err(ImposeError::DoesNotFit { cell_w: cell.0, cell_h: cell.1, area_w, area_h });
    }
    let cols = spec.cols.unwrap_or(fits_cols);
    let rows = spec.rows.unwrap_or(fits_rows);
    if cols > fits_cols || rows > fits_rows {
        return Err(ImposeError::GridTooBig {
            cols,
            rows,
            need_w: cols as f64 * cell.0 + cols.saturating_sub(1) as f64 * spec.gutter.0,
            need_h: rows as f64 * cell.1 + rows.saturating_sub(1) as f64 * spec.gutter.1,
            area_w,
            area_h,
        });
    }
    if cols.checked_mul(rows).is_none_or(|n| n > MAX_CELLS_PER_SHEET) {
        return Err(ImposeError::TooLarge("more cells on one sheet than are supported"));
    }
    let used_w = cols as f64 * cell.0 + (cols - 1) as f64 * spec.gutter.0;
    let used_h = rows as f64 * cell.1 + (rows - 1) as f64 * spec.gutter.1;
    // The grid is centred in the usable area.
    let x0 = m.left + (area_w - used_w) / 2.0;
    let top = spec.sheet.1 - m.top - (area_h - used_h) / 2.0;
    let mut cells = Vec::with_capacity(cols * rows);
    for r in 0..rows {
        for c in 0..cols {
            let x = x0 + c as f64 * (cell.0 + spec.gutter.0);
            let y1 = top - r as f64 * (cell.1 + spec.gutter.1);
            cells.push([x, y1 - cell.1, x + cell.0, y1]);
        }
    }
    Ok(cells)
}

/// `rect` as it appears from the other side of the sheet.
fn mirrored(rect: Rect, sheet: (f64, f64), duplex: Duplex) -> Rect {
    match duplex {
        Duplex::LongEdge => [sheet.0 - rect[2], rect[1], sheet.0 - rect[0], rect[3]],
        Duplex::ShortEdge => [rect[0], sheet.1 - rect[3], rect[2], sheet.1 - rect[1]],
    }
}

/// A page of display size `size` centred in `cell`: where it lands and the part of the page that
/// is visible inside the cell. `None` when the page and the cell do not overlap.
fn place_page(page: usize, size: (f64, f64), cell: Rect) -> Option<(Placement, Rect)> {
    let ox = cell[0] + ((cell[2] - cell[0]) - size.0) / 2.0;
    let oy = cell[1] + ((cell[3] - cell[1]) - size.1) / 2.0;
    // Visible part, in the page's own display space.
    let clip = [(cell[0] - ox).max(0.0), (cell[1] - oy).max(0.0), (cell[2] - ox).min(size.0), (cell[3] - oy).min(size.1)];
    if clip[2] - clip[0] <= EPS || clip[3] - clip[1] <= EPS {
        return None;
    }
    let visible = [ox + clip[0], oy + clip[1], ox + clip[2], oy + clip[3]];
    Some((Placement { page, matrix: Matrix([1.0, 0.0, 0.0, 1.0, ox, oy]), clip }, visible))
}

/// What goes into one cell of a sheet: the page and its size.
type Slot = Option<(usize, (f64, f64))>;

/// Build one sheet from `cells` and what each holds.
fn build_sheet(spec: &StepRepeat, cells: &[Rect], slots: &[Slot]) -> Sheet {
    let mut sheet = Sheet { size: spec.sheet, ..Sheet::default() };
    let mut trims: Vec<Rect> = Vec::new();
    let mut visible: Vec<Rect> = Vec::new();
    for (cell, slot) in cells.iter().zip(slots) {
        let Some((page, size)) = *slot else { continue };
        let Some((placement, vis)) = place_page(page, size, *cell) else { continue };
        sheet.placed.push(placement);
        visible.push(vis);
        let ox = placement.matrix.0[4];
        let oy = placement.matrix.0[5];
        trims.push([ox + spec.bleed, oy + spec.bleed, ox + size.0 - spec.bleed, oy + size.1 - spec.bleed]);
    }
    if let Some(marks) = spec.marks {
        sheet.lines = trim_marks(&trims, &visible, spec.sheet, spec.bleed, marks);
    }
    sheet
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a[0] < b[2] - EPS && b[0] < a[2] - EPS && a[1] < b[3] - EPS && b[1] < a[3] - EPS
}

/// Corner marks around every rect in `trims`. A mark that would run over a page or off the sheet
/// is left out, and a mark that two neighbours share is drawn once.
fn trim_marks(trims: &[Rect], pages: &[Rect], sheet: (f64, f64), bleed: f64, marks: Marks) -> Vec<[f64; 4]> {
    let gap = marks.offset.max(bleed);
    let len = marks.length;
    let mut lines: Vec<[f64; 4]> = Vec::new();
    let mut push = |line: [f64; 4]| {
        let norm = [line[0].min(line[2]), line[1].min(line[3]), line[0].max(line[2]), line[1].max(line[3])];
        let inside = norm[0] >= -EPS && norm[1] >= -EPS && norm[2] <= sheet.0 + EPS && norm[3] <= sheet.1 + EPS;
        // A mark is a line; thicken it across its direction (not along it, so a mark that only
        // touches a page edge stays) so the overlap test sees a rectangle.
        let probe = if (norm[1] - norm[3]).abs() < EPS {
            [norm[0], norm[1] - 0.01, norm[2], norm[3] + 0.01]
        } else {
            [norm[0] - 0.01, norm[1], norm[2] + 0.01, norm[3]]
        };
        let over_page = pages.iter().any(|p| overlaps(probe, *p));
        let seen = lines.iter().any(|l| l.iter().zip(norm.iter()).all(|(a, b)| (a - b).abs() < EPS));
        if inside && !over_page && !seen {
            lines.push(norm);
        }
    };
    for t in trims {
        let [x0, y0, x1, y1] = *t;
        for y in [y0, y1] {
            push([x0 - gap - len, y, x0 - gap, y]);
            push([x1 + gap, y, x1 + gap + len, y]);
        }
        for x in [x0, x1] {
            push([x, y0 - gap - len, x, y0 - gap]);
            push([x, y1 + gap, x, y1 + gap + len]);
        }
    }
    lines
}

/// Lay out `sizes` (the display size of every source page, in order) as press sheets.
pub fn step_and_repeat(sizes: &[(f64, f64)], spec: &StepRepeat) -> Result<Vec<Sheet>, ImposeError> {
    if sizes.is_empty() {
        return Err(ImposeError::NoPages);
    }
    validate(spec, sizes)?;
    match spec.fill {
        Fill::Repeat => repeat_sheets(sizes, spec),
        Fill::Sequential => sequential_sheets(sizes, spec),
    }
}

fn check_sheet_count(n: usize) -> Result<(), ImposeError> {
    if n > MAX_SHEETS { Err(ImposeError::TooLarge("more sheets than are supported")) } else { Ok(()) }
}

fn repeat_sheets(sizes: &[(f64, f64)], spec: &StepRepeat) -> Result<Vec<Sheet>, ImposeError> {
    // Each page gets a grid made for its own size, unless the spec fixes the cell.
    let step = if spec.duplex.is_some() { 2 } else { 1 };
    check_sheet_count(sizes.len())?;
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < sizes.len() {
        let Some(&front_size) = sizes.get(i) else { break };
        let cell = spec.cell.unwrap_or(front_size);
        let cells = grid(spec, cell)?;
        let slots: Vec<Slot> = cells.iter().map(|_| Some((i, front_size))).collect();
        out.push(build_sheet(spec, &cells, &slots));
        if let Some(duplex) = spec.duplex
            && let Some(&back_size) = sizes.get(i + 1)
        {
            let back_cells: Vec<Rect> = cells.iter().map(|c| mirrored(*c, spec.sheet, duplex)).collect();
            let back_slots: Vec<Slot> = back_cells.iter().map(|_| Some((i + 1, back_size))).collect();
            out.push(build_sheet(spec, &back_cells, &back_slots));
        }
        i += step;
    }
    Ok(out)
}

fn sequential_sheets(sizes: &[(f64, f64)], spec: &StepRepeat) -> Result<Vec<Sheet>, ImposeError> {
    let widest = sizes.iter().fold(0.0_f64, |m, s| m.max(s.0));
    let tallest = sizes.iter().fold(0.0_f64, |m, s| m.max(s.1));
    let cells = grid(spec, spec.cell.unwrap_or((widest, tallest)))?;
    let n = cells.len();
    // A duplex set is a front sheet of `n` pages followed by the back sheet of the next `n`.
    let per_set = if spec.duplex.is_some() { n.saturating_mul(2) } else { n };
    check_sheet_count(sizes.len().div_ceil(n.max(1)))?;
    let slot = |index: usize| -> Slot { sizes.get(index).map(|s| (index, *s)) };
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < sizes.len() {
        let front: Vec<Slot> = (0..n).map(|k| slot(start + k)).collect();
        out.push(build_sheet(spec, &cells, &front));
        if let Some(duplex) = spec.duplex {
            let back: Vec<Slot> = (0..n).map(|k| slot(start + n + k)).collect();
            if back.iter().any(Option::is_some) {
                let back_cells: Vec<Rect> = cells.iter().map(|c| mirrored(*c, spec.sheet, duplex)).collect();
                out.push(build_sheet(spec, &back_cells, &back));
            }
        }
        start = start.saturating_add(per_set.max(1));
    }
    Ok(out)
}

/// Why [`impose_document`] failed.
#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error(transparent)]
    Layout(#[from] ImposeError),
    #[error(transparent)]
    Print(#[from] PrintError),
}

/// Lay out every page of `src` by `spec` and write the sheets as a print-ready PDF. Annotations
/// print as `content` says; encryption is not carried over (see [`pdfcraft_print::impose`]).
pub fn impose_document(src: &Document, spec: &StepRepeat, content: Content) -> Result<Vec<u8>, DocumentError> {
    let sheets = step_and_repeat(&pdfcraft_print::page_sizes(src), spec)?;
    Ok(pdfcraft_print::impose_sheets(src, &sheets, content)?)
}

#[cfg(test)]
mod tests;
