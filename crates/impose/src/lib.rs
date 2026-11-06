//! Print-production imposition (layer L4).
//!
//! Lays source pages out on press sheets and returns them as [`pdfcraft_print::Sheet`]s, which
//! `pdfcraft-print` then writes as a PDF. This crate is geometry only: it never reads or writes
//! PDF objects, so every layout rule is unit-testable with plain numbers.
//!
//! - **Step & repeat** ([`Fill::Repeat`]): each source page is repeated across a grid of cells
//!   (business cards, labels, flyers cut from a larger sheet).
//! - **Cut & stack / unique collate** ([`Fill::CutStack`]): stacks that read in order once the
//!   sheets are cut, optionally repeated across the sheet.
//! - **Gang / n-up** ([`Fill::Sequential`]): consecutive source pages fill the grid in reading
//!   order, with gutters between cells.
//! - **Duplex** ([`Duplex`]): the back side of every sheet is laid out as the mirror image of the
//!   front, so a front cell and its back cell sit exactly behind each other once the sheet is
//!   turned over the long or the short edge.
//! - **Trim marks**: corner or Japanese (tombo) marks around every finished page, optionally running
//!   into the page (negative offset), in black or registration colour.
//! - **Bleed** ([`add_bleed`]): mirrored or scaled margin around every page, with a `TrimBox`.
//! - **Manual** ([`manual()`]): every page placed by hand, turned in quarter turns, with marks
//!   worked out as for step & repeat.
//! - **Booklets** ([`booklet()`]): saddle-stitched, perfect-bound (signatures) and continuous page
//!   order, two pages to a sheet side.
//!
//! All lengths are PDF points (1/72 inch). Sheet space has its origin at the lower left, y up.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod bleed;
mod booklet;
mod manual;

pub use bleed::{AddBleed, BleedMethod, add_bleed};
pub use booklet::{Align, Binding, Booklet, LastSignature, booklet};
pub use manual::{Manual, ManualPage, manual};

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
    /// A manual layout names a page the document does not have (1-based).
    #[error("page {page} does not exist: the document has {pages} page(s)")]
    NoSuchPage { page: usize, pages: usize },
    /// A page of a manual layout lies wholly off the sheet (1-based).
    #[error("page {page} lies wholly off the sheet")]
    OffSheet { page: usize },
}

/// How source pages fill the cells of a sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Fill {
    /// Every cell of a sheet shows the same page (step & repeat).
    #[default]
    Repeat,
    /// Consecutive pages fill the cells in reading order (n-up, ganging).
    Sequential,
    /// Cut & stack ("unique collate"): the grid is cut into stacks and each stack runs on in page
    /// order from one sheet to the next, so after cutting, every stack is already in sequence.
    /// The first `cols / copies` columns hold different stacks and that block is repeated
    /// `copies` times across the sheet. With 16 pages on 4 x 2 cells and 2 copies, sheet 1 reads
    /// 1 5 1 5 / 9 13 9 13, sheet 2 reads 2 6 2 6 / 10 14 10 14, and so on.
    CutStack {
        /// How many times the block of different stacks is repeated across a row (at least 1;
        /// must divide the number of columns).
        copies: usize,
    },
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
    /// Distance from the trim edge to where a mark starts, outward. Negative runs the mark into
    /// the page by that much: offset -3 mm with length 5 mm puts 3 mm inside the trim and 2 mm
    /// outside it.
    pub offset: f64,
    pub style: MarkStyle,
    /// Line width.
    pub width: f64,
    /// Registration colour (every plate) instead of black.
    pub registration: bool,
}

/// How the corner of a page is marked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MarkStyle {
    /// One line along each trim edge at every corner.
    #[default]
    Corner,
    /// Japanese style (tombo): a second line along the bleed edge beside every corner line, so
    /// the press room sees both the trim and the bleed.
    Japanese,
    /// A cross centred on every outer trim corner, `length` long, half on the artwork and half
    /// outside (offset is ignored). Where pages meet at the edge of the layout only the line
    /// along the cut between them is drawn, and none inside the layout: a line is kept only when
    /// its outer end lies clear of every other page.
    Centred,
}

impl Default for Marks {
    fn default() -> Self {
        // 3 mm marks starting 3 mm from the trim, the usual press-room values.
        Self { length: 8.504, offset: 8.504, style: MarkStyle::Corner, width: 0.5, registration: false }
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
    /// Step & repeat only, with automatic columns, rows and cell: turn whole rows of pages a
    /// quarter turn where that fits more copies (as press-room step & repeat does). Rows run
    /// from the top; each row is centred.
    pub rotate: bool,
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
            rotate: false,
        }
    }
}

/// A rectangle `[x0, y0, x1, y1]` in sheet space.
type Rect = [f64; 4];

const EPS: f64 = 1e-6;

/// How far a page may overrun the room it is given and still count as fitting, in points
/// (0.05 pt = 0.018 mm): PDF files round their sizes, so a 105 mm page doubled is 0.005 pt more
/// than a 210 mm sheet, and a layout must not be refused for that.
pub(crate) const FIT_TOLERANCE: f64 = 0.05;

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
        positive(m.width, "mark width")?;
        if !m.offset.is_finite() || m.offset + m.length <= 0.0 {
            return Err(ImposeError::Invalid("mark offset", "finite, and leaving part of the mark outside the trim"));
        }
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
    let n = ((room + gap + FIT_TOLERANCE) / (cell + gap) + EPS).floor();
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

/// A quarter turn of a page in its cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Turn {
    None,
    /// Anticlockwise: the head of the page points left.
    Ccw,
    /// Clockwise: the head points right.
    Cw,
}

/// A page turned a quarter turn, centred in `cell` (which holds its turned footprint).
fn place_turned(page: usize, size: (f64, f64), cell: Rect, turn: Turn) -> (Placement, Rect) {
    let (w, h) = size;
    let ox = cell[0] + ((cell[2] - cell[0]) - h) / 2.0;
    let oy = cell[1] + ((cell[3] - cell[1]) - w) / 2.0;
    let m = match turn {
        Turn::Cw => [0.0, -1.0, 1.0, 0.0, ox, oy + w],
        _ => [0.0, 1.0, -1.0, 0.0, ox + h, oy],
    };
    (Placement { page, matrix: Matrix(m), clip: [0.0, 0.0, w, h] }, [ox, oy, ox + h, oy + w])
}

/// Build one sheet from `cells` and what each holds.
fn build_sheet(spec: &StepRepeat, cells: &[Rect], slots: &[Slot]) -> Sheet {
    build_turned(spec, cells, slots, &[])
}

/// [`build_sheet`] with a [`Turn`] per cell (missing entries are not turned).
fn build_turned(spec: &StepRepeat, cells: &[Rect], slots: &[Slot], turns: &[Turn]) -> Sheet {
    let mut sheet = Sheet { size: spec.sheet, ..Sheet::default() };
    let mut trims: Vec<Rect> = Vec::new();
    for (k, (cell, slot)) in cells.iter().zip(slots).enumerate() {
        let Some((page, size)) = *slot else { continue };
        let turn = turns.get(k).copied().unwrap_or(Turn::None);
        let footprint = if turn == Turn::None {
            let Some((placement, _)) = place_page(page, size, *cell) else { continue };
            let (ox, oy) = (placement.matrix.0[4], placement.matrix.0[5]);
            sheet.placed.push(placement);
            [ox, oy, ox + size.0, oy + size.1]
        } else {
            let (placement, footprint) = place_turned(page, size, *cell, turn);
            sheet.placed.push(placement);
            footprint
        };
        let b = spec.bleed;
        trims.push([footprint[0] + b, footprint[1] + b, footprint[2] - b, footprint[3] - b]);
    }
    if let Some(marks) = spec.marks {
        sheet.lines = trim_marks(&trims, spec.sheet, spec.bleed, marks);
        sheet.marks_paint = pdfcraft_print::MarkPaint { width: marks.width, registration: marks.registration };
    }
    sheet
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a[0] < b[2] - EPS && b[0] < a[2] - EPS && a[1] < b[3] - EPS && b[1] < a[3] - EPS
}

/// Corner marks around every rect in `trims` (the finished pages). As in press-room smart marks,
/// a mark may run over a bleed but never over another page's finished area, nor off the sheet;
/// one that would is left out. A mark with a negative offset runs into its own page by design. A
/// mark two neighbours share is drawn once.
fn trim_marks(trims: &[Rect], sheet: (f64, f64), bleed: f64, marks: Marks) -> Vec<[f64; 4]> {
    if marks.style == MarkStyle::Centred {
        return centred_marks(trims, sheet, marks.length);
    }
    let gap = marks.offset;
    let len = marks.length;
    let mut lines: Vec<[f64; 4]> = Vec::new();
    let mut push = |own: usize, line: [f64; 4]| {
        let norm = [line[0].min(line[2]), line[1].min(line[3]), line[0].max(line[2]), line[1].max(line[3])];
        let inside = norm[0] >= -EPS && norm[1] >= -EPS && norm[2] <= sheet.0 + EPS && norm[3] <= sheet.1 + EPS;
        // A mark is a line; thicken it across its direction (not along it, so a mark that only
        // touches a page edge stays) so the overlap test sees a rectangle.
        let probe = if (norm[1] - norm[3]).abs() < EPS {
            [norm[0], norm[1] - 0.01, norm[2], norm[3] + 0.01]
        } else {
            [norm[0] - 0.01, norm[1], norm[2] + 0.01, norm[3]]
        };
        let over_page = trims.iter().enumerate().any(|(k, p)| k != own && overlaps(probe, *p));
        let seen = lines.iter().any(|l| l.iter().zip(norm.iter()).all(|(a, b)| (a - b).abs() < EPS));
        if inside && !over_page && !seen {
            lines.push(norm);
        }
    };
    // Japanese marks repeat each line along the bleed edge, `bleed` further out.
    let shifts: &[f64] = if marks.style == MarkStyle::Japanese && bleed > EPS { &[0.0, 1.0] } else { &[0.0] };
    for (own, t) in trims.iter().enumerate() {
        let [x0, y0, x1, y1] = *t;
        for &k in shifts {
            let d = k * bleed;
            for (y, out) in [(y0, -d), (y1, d)] {
                push(own, [x0 - gap - len, y + out, x0 - gap, y + out]);
                push(own, [x1 + gap, y + out, x1 + gap + len, y + out]);
            }
            for (x, out) in [(x0, -d), (x1, d)] {
                push(own, [x + out, y0 - gap - len, x + out, y0 - gap]);
                push(own, [x + out, y1 + gap, x + out, y1 + gap + len]);
            }
        }
    }
    lines
}

/// [`MarkStyle::Centred`] marks: per corner a horizontal and a vertical line through the corner,
/// `len` long, from `len / 2` inside the page to `len / 2` outside it.
fn centred_marks(trims: &[Rect], sheet: (f64, f64), len: f64) -> Vec<[f64; 4]> {
    let h = len / 2.0;
    let mut lines: Vec<[f64; 4]> = Vec::new();
    // On or inside page `k` (its edges count).
    let on_page = |k: usize, x: f64, y: f64| trims.get(k).is_some_and(|t| x >= t[0] - EPS && x <= t[2] + EPS && y >= t[1] - EPS && y <= t[3] + EPS);
    for (own, t) in trims.iter().enumerate() {
        let [x0, y0, x1, y1] = *t;
        for (x, sx) in [(x0, -1.0), (x1, 1.0)] {
            for (y, sy) in [(y0, -1.0), (y1, 1.0)] {
                // (inner end, outer end) of the horizontal and the vertical line.
                for (inner, outer) in [((x - sx * h, y), (x + sx * h, y)), ((x, y - sy * h), (x, y + sy * h))] {
                    let line = [inner.0.min(outer.0), inner.1.min(outer.1), inner.0.max(outer.0), inner.1.max(outer.1)];
                    let inside = line[0] >= -EPS && line[1] >= -EPS && line[2] <= sheet.0 + EPS && line[3] <= sheet.1 + EPS;
                    let clear = (0..trims.len()).all(|k| k == own || !on_page(k, outer.0, outer.1));
                    // A line through another page's inside (not just along its edge) would print on it.
                    let crosses = trims
                        .iter()
                        .enumerate()
                        .any(|(k, p)| k != own && line[0] < p[2] - EPS && p[0] < line[2] - EPS && line[1] < p[3] - EPS && p[1] < line[3] - EPS);
                    let seen = lines.iter().any(|l| l.iter().zip(line.iter()).all(|(a, b)| (a - b).abs() < EPS));
                    if inside && clear && !crosses && !seen {
                        lines.push(line);
                    }
                }
            }
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
        Fill::CutStack { copies } => cut_stack_sheets(sizes, spec, copies),
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
        let (cells, turns) = if spec.rotate && spec.cols.is_none() && spec.rows.is_none() && spec.cell.is_none() {
            rotated_rows(spec, front_size)?
        } else {
            let cells = grid(spec, spec.cell.unwrap_or(front_size))?;
            let turns = vec![Turn::None; cells.len()];
            (cells, turns)
        };
        let slots: Vec<Slot> = cells.iter().map(|_| Some((i, front_size))).collect();
        out.push(build_turned(spec, &cells, &slots, &turns));
        if let Some(duplex) = spec.duplex
            && let Some(&back_size) = sizes.get(i + 1)
        {
            let back_cells: Vec<Rect> = cells.iter().map(|c| mirrored(*c, spec.sheet, duplex)).collect();
            let back_slots: Vec<Slot> = back_cells.iter().map(|_| Some((i + 1, back_size))).collect();
            // Turned over the long edge, a page whose head pointed left now has it pointing right:
            // its back turns the other way so heads line up. Over the short edge they stay.
            let back_turns: Vec<Turn> = turns
                .iter()
                .map(|t| match (t, duplex) {
                    (Turn::Ccw, Duplex::LongEdge) => Turn::Cw,
                    (Turn::Cw, Duplex::LongEdge) => Turn::Ccw,
                    (t, _) => *t,
                })
                .collect();
            out.push(build_turned(spec, &back_cells, &back_slots, &back_turns));
        }
        i += step;
    }
    Ok(out)
}

/// Rows of `size` pages, some rows turned a quarter turn, fitting the most copies in the usable
/// area: a block of rows one way from the top, then rows the other way in the space left.
fn rotated_rows(spec: &StepRepeat, size: (f64, f64)) -> Result<(Vec<Rect>, Vec<Turn>), ImposeError> {
    let m = spec.margins;
    let area_w = spec.sheet.0 - m.left - m.right;
    let area_h = spec.sheet.1 - m.top - m.bottom;
    let (gx, gy) = spec.gutter;
    let shapes = [(size, Turn::None), ((size.1, size.0), Turn::Ccw)];
    // (copies, turned copies, first shape, rows of the first shape, rows of the second)
    let mut best: Option<(usize, usize, usize, usize, usize)> = None;
    for first in 0..2 {
        let (a, ta) = shapes[first];
        let (b, _) = shapes[1 - first];
        let (ca, cb) = (fit_count(area_w, a.0, gx), fit_count(area_w, b.0, gx));
        for ra in 0..=fit_count(area_h, a.1, gy) {
            let used = if ra > 0 { ra as f64 * a.1 + (ra - 1) as f64 * gy + gy } else { 0.0 };
            let rb = if area_h - used > 0.0 { fit_count(area_h - used, b.1, gy) } else { 0 };
            let total = ra.saturating_mul(ca).saturating_add(rb.saturating_mul(cb));
            let turned = if ta == Turn::None { rb.saturating_mul(cb) } else { ra.saturating_mul(ca) };
            let better = match best {
                None => total > 0,
                Some((t, tt, ..)) => total > t || (total == t && turned < tt),
            };
            if better {
                best = Some((total, turned, first, ra, rb));
            }
        }
    }
    let Some((total, _, first, ra, rb)) = best else {
        return Err(ImposeError::DoesNotFit { cell_w: size.0, cell_h: size.1, area_w, area_h });
    };
    if total > MAX_CELLS_PER_SHEET {
        return Err(ImposeError::TooLarge("more cells on one sheet than are supported"));
    }
    let rows: Vec<((f64, f64), Turn)> = std::iter::repeat_n(shapes[first], ra).chain(std::iter::repeat_n(shapes[1 - first], rb)).collect();
    let used_h: f64 = rows.iter().map(|(s, _)| s.1).sum::<f64>() + rows.len().saturating_sub(1) as f64 * gy;
    let mut top = spec.sheet.1 - m.top - (area_h - used_h) / 2.0;
    let (mut cells, mut turns) = (Vec::with_capacity(total), Vec::with_capacity(total));
    for (shape, turn) in rows {
        let n = fit_count(area_w, shape.0, gx);
        let row_w = n as f64 * shape.0 + n.saturating_sub(1) as f64 * gx;
        let x0 = m.left + (area_w - row_w) / 2.0;
        for c in 0..n {
            let x = x0 + c as f64 * (shape.0 + gx);
            cells.push([x, top - shape.1, x + shape.0, top]);
            turns.push(turn);
        }
        top -= shape.1 + gy;
    }
    Ok((cells, turns))
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

fn cut_stack_sheets(sizes: &[(f64, f64)], spec: &StepRepeat, copies: usize) -> Result<Vec<Sheet>, ImposeError> {
    let widest = sizes.iter().fold(0.0_f64, |m, s| m.max(s.0));
    let tallest = sizes.iter().fold(0.0_f64, |m, s| m.max(s.1));
    let cell = spec.cell.unwrap_or((widest, tallest));
    let mut cells = grid(spec, cell)?;
    // The grid is row by row from the top: the first row is the cells sharing the first top edge.
    let row_len = |cells: &[Rect]| {
        let top = cells.first().map_or(0.0, |c| c[3]);
        cells.iter().take_while(|c| (c[3] - top).abs() < EPS).count().max(1)
    };
    let mut cols = row_len(&cells);
    // Automatic columns: as many as fit, rounded down to a whole number of repeated blocks.
    if spec.cols.is_none() && copies > 0 && !cols.is_multiple_of(copies) && cols >= copies {
        let mut fixed = spec.clone();
        fixed.cols = Some(cols - cols % copies);
        cells = grid(&fixed, cell)?;
        cols = row_len(&cells);
    }
    if copies == 0 || !cols.is_multiple_of(copies) {
        return Err(ImposeError::Invalid("copies", "at least 1 and dividing the number of columns"));
    }
    let block = cols / copies;
    // Different stacks on one sheet.
    let stacks = (cells.len() / cols) * block;
    // Duplex cuts and stacks leaves (front/back page pairs) instead of single pages.
    let per_unit = if spec.duplex.is_some() { 2 } else { 1 };
    let units = sizes.len().div_ceil(per_unit);
    let sheets = units.div_ceil(stacks.max(1));
    check_sheet_count(sheets.saturating_mul(per_unit))?;
    let slot = |index: usize| -> Slot { sizes.get(index).map(|s| (index, *s)) };
    // The unit in cell `k` of sheet `s`: stack u holds units u * sheets .. u * sheets + sheets.
    let unit = |k: usize, s: usize| -> usize {
        let (r, c) = (k / cols, k % cols);
        (r * block + c % block) * sheets + s
    };
    let mut out = Vec::new();
    for s in 0..sheets {
        let front: Vec<Slot> = (0..cells.len()).map(|k| slot(unit(k, s) * per_unit)).collect();
        out.push(build_sheet(spec, &cells, &front));
        if let Some(duplex) = spec.duplex {
            let back: Vec<Slot> = (0..cells.len()).map(|k| slot(unit(k, s) * per_unit + 1)).collect();
            if back.iter().any(Option::is_some) {
                let back_cells: Vec<Rect> = cells.iter().map(|c| mirrored(*c, spec.sheet, duplex)).collect();
                out.push(build_sheet(spec, &back_cells, &back));
            }
        }
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

/// Lay out every page of `src` as a booklet by `spec` and write the sheet sides as a print-ready
/// PDF (see [`impose_document`] for annotations and encryption).
pub fn booklet_document(src: &Document, spec: &Booklet, content: Content) -> Result<Vec<u8>, DocumentError> {
    let sheets = booklet(&pdfcraft_print::page_sizes(src), spec)?;
    Ok(pdfcraft_print::impose_sheets(src, &sheets, content)?)
}

/// Lay out pages of `src` by hand as `spec` says and write the sheets as a print-ready PDF (see
/// [`impose_document`] for annotations and encryption).
pub fn manual_document(src: &Document, spec: &Manual, content: Content) -> Result<Vec<u8>, DocumentError> {
    let sheets = manual(&pdfcraft_print::page_sizes(src), spec)?;
    Ok(pdfcraft_print::impose_sheets(src, &sheets, content)?)
}

/// Add bleed to every page of `src` by `spec` and write the pages (each with a `TrimBox` at the
/// old edge) as a PDF (see [`impose_document`] for annotations and encryption).
pub fn bleed_document(src: &Document, spec: &AddBleed, content: Content) -> Result<Vec<u8>, DocumentError> {
    let sheets = add_bleed(&pdfcraft_print::page_sizes(src), spec)?;
    Ok(pdfcraft_print::impose_sheets(src, &sheets, content)?)
}

#[cfg(test)]
mod bleed_tests;
#[cfg(test)]
mod booklet_tests;
#[cfg(test)]
mod manual_tests;
#[cfg(test)]
mod tests;
