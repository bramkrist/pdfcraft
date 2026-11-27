//! pdfcraft-cutter — L0 output for a plotter-cutter that finds its sheet by registration marks.
//!
//! The sheet carries four registration marks, one near each corner and a fixed distance
//! (`inset`) in from the sheet's edges, and a QR code naming the job. The cutter reads the marks,
//! reads the code, loads the PLT file of that name and cuts it relative to the marks. This crate
//! makes both halves from the same numbers, so that they cannot drift apart:
//!
//! - [`marks`]: the black squares and the QR code as rectangles in sheet space (points, y up),
//!   to be drawn by the imposition,
//! - [`plt`]: the cut file, in the layout a working job file of such a cutter has: HPGL at
//!   40 units a millimetre, the origin at the lower-left mark, the header naming the distance
//!   between the marks and their inset, closed outlines drawn clockwise from their leftmost point,
//!   in serpentine order, column by column.
//!
//! What a working file shows, and what it does not: the header `W,H,inset,inset` and the contour
//! coordinates fix the geometry above; the mark's shape and size and the code's content are not
//! in the cut file. They are choices here ([`Layout`]), to be checked against the machine once.
//!
//! Never panics: every number is checked, sizes are bounded, and failures are [`CutterError`]s.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::fmt;

use qrcode::{Color, EcLevel, QrCode};

#[cfg(test)]
mod tests;

/// HPGL plotter units in a millimetre.
const UNITS_PER_MM: f64 = 40.0;
/// Points in a millimetre.
const PT_PER_MM: f64 = 72.0 / 25.4;
/// The header is padded to this many bytes before the commands, as a working file's is.
const HEADER_BYTES: usize = 1103;
/// Most characters of a job name (it is a file name and the code's content).
const MAX_NAME: usize = 32;
/// Most points a PLT job may hold in all (a bound on the work and on the file).
const MAX_POINTS: usize = 4_000_000;
/// A sheet side bound (mm): the coordinates stay far inside what the file format holds.
const MAX_SHEET_MM: f64 = 5_000.0;

/// Why output could not be made.
#[derive(Clone, Debug, PartialEq)]
pub enum CutterError {
    /// A number was NaN or infinite, or a size was out of range.
    BadNumber(&'static str),
    /// The job name has nothing usable in it (letters, digits, `-` and `_` are kept).
    BadName,
    /// The margin between the sheet edge and the marks cannot hold the mark and the code.
    NoRoom,
    /// There is nothing to cut.
    NothingToCut,
    /// The job is larger than the file format or this crate's bounds allow.
    TooLarge,
    /// A cut line lies outside the rectangle of the marks, where the cutter has no reference and
    /// the marks and the code are printed.
    OutsideMarks,
}

impl fmt::Display for CutterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadNumber(what) => write!(f, "{what} is not a usable number"),
            Self::BadName => f.write_str("the job name has no letters or digits"),
            Self::NoRoom => f.write_str("the margin around the marks is too small for the mark and the QR code"),
            Self::NothingToCut => f.write_str("there are no cut lines to write"),
            Self::TooLarge => f.write_str("the job is too large"),
            Self::OutsideMarks => f.write_str("a cut line lies outside the registration marks: keep the artwork and its cut line inside the margin"),
        }
    }
}

impl std::error::Error for CutterError {}

/// Where the marks and the code go, in millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// Distance from the sheet's edge to the centre of each mark (and the PLT's origin offset).
    pub inset_mm: f64,
    /// Side of a mark, a black square.
    pub mark_mm: f64,
    /// Side of the QR code's dark-and-light area, without its clear border.
    pub code_mm: f64,
}

impl Default for Layout {
    fn default() -> Self {
        Self { inset_mm: 10.0, mark_mm: 5.0, code_mm: 7.0 }
    }
}

impl Layout {
    fn check(&self, sheet: (f64, f64)) -> Result<(), CutterError> {
        for (what, v) in [("the inset", self.inset_mm), ("the mark size", self.mark_mm), ("the code size", self.code_mm)] {
            if !v.is_finite() || v <= 0.0 {
                return Err(CutterError::BadNumber(what));
            }
        }
        let (w, h) = (sheet.0 / PT_PER_MM, sheet.1 / PT_PER_MM);
        if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 || w > MAX_SHEET_MM || h > MAX_SHEET_MM {
            return Err(CutterError::BadNumber("the sheet size"));
        }
        // The marks must fit with room between them, and the code (with a 1 mm clear border) must
        // lie in the margin, clear of the marks.
        if self.inset_mm * 2.0 >= w.min(h) || self.mark_mm / 2.0 + 1.0 > self.inset_mm || self.code_mm + 2.0 > self.inset_mm {
            return Err(CutterError::NoRoom);
        }
        Ok(())
    }
}

/// What to draw for the marks and the code, in sheet space: points, y up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marks {
    /// Clear rectangles `[x0, y0, x1, y1]`, drawn first (the code's quiet zone).
    pub clear: Vec<[f64; 4]>,
    /// Black rectangles `[x0, y0, x1, y1]`: the four marks and the code's dark modules.
    pub black: Vec<[f64; 4]>,
}

/// A job name made safe: letters, digits, `-` and `_` only, at most [`MAX_NAME`] of them.
pub fn clean_name(name: &str) -> Result<String, CutterError> {
    let name: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(MAX_NAME).collect();
    if name.is_empty() { Err(CutterError::BadName) } else { Ok(name) }
}

/// The marks and the QR code (content: the cleaned `name`) for a sheet of `sheet` points.
pub fn marks(sheet: (f64, f64), name: &str, layout: &Layout) -> Result<Marks, CutterError> {
    layout.check(sheet)?;
    let name = clean_name(name)?;
    let (w, h) = sheet;
    let (inset, half) = (layout.inset_mm * PT_PER_MM, layout.mark_mm * PT_PER_MM / 2.0);
    let mut out = Marks::default();
    for (cx, cy) in [(inset, inset), (w - inset, inset), (inset, h - inset), (w - inset, h - inset)] {
        out.black.push([cx - half, cy - half, cx + half, cy + half]);
    }
    // The code sits on the bottom margin, centred between the lower marks, 1 mm clear all round.
    let code = QrCode::with_error_correction_level(name.as_bytes(), EcLevel::M).map_err(|_| CutterError::TooLarge)?;
    let modules = code.width();
    let side = layout.code_mm * PT_PER_MM;
    let module = side / modules as f64;
    let clear = PT_PER_MM;
    let (x0, y0) = (w / 2.0 - side / 2.0, clear * 1.5);
    out.clear.push([x0 - clear, y0 - clear, x0 + side + clear, y0 + side + clear]);
    let colors = code.to_colors();
    for row in 0..modules {
        let mut run: Option<usize> = None;
        for col in 0..=modules {
            let dark = col < modules && colors.get(row * modules + col) == Some(&Color::Dark);
            match (dark, run) {
                (true, None) => run = Some(col),
                (false, Some(start)) => {
                    // Row 0 of the code is its top.
                    let top = y0 + side - row as f64 * module;
                    out.black.push([x0 + start as f64 * module, top - module, x0 + col as f64 * module, top]);
                    run = None;
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

/// One cut job: closed outlines on a sheet, in points, y up, the sheet's lower-left corner the origin.
#[derive(Clone, Copy, Debug)]
pub struct Job<'a> {
    pub name: &'a str,
    pub sheet: (f64, f64),
    pub layout: Layout,
    pub loops: &'a [Vec<(f64, f64)>],
}

type Pt = (i64, i64);

/// The PLT file for `job`.
pub fn plt(job: &Job<'_>) -> Result<Vec<u8>, CutterError> {
    job.layout.check(job.sheet)?;
    let name = clean_name(job.name)?;
    if job.loops.iter().map(Vec::len).sum::<usize>() > MAX_POINTS {
        return Err(CutterError::TooLarge);
    }
    let inset = job.layout.inset_mm * PT_PER_MM;
    let units = |v: f64| -> Result<i64, CutterError> {
        let u = (v / PT_PER_MM * UNITS_PER_MM).round();
        if u.is_finite() && u.abs() < 1.0e9 { Ok(u as i64) } else { Err(CutterError::BadNumber("a cut point")) }
    };
    let mut shapes: Vec<Vec<Pt>> = Vec::new();
    for l in job.loops {
        let mut pts: Vec<Pt> = Vec::with_capacity(l.len());
        for &(x, y) in l {
            let p = (units(x - inset)?, units(y - inset)?);
            if pts.last() != Some(&p) {
                pts.push(p);
            }
        }
        while pts.len() > 1 && pts.first() == pts.last() {
            pts.pop();
        }
        if pts.len() >= 3 {
            shapes.push(clockwise_from_left(pts));
        }
    }
    if shapes.is_empty() {
        return Err(CutterError::NothingToCut);
    }
    let order = serpentine(&shapes);
    let (width, height) = (units(job.sheet.0 - 2.0 * inset)?, units(job.sheet.1 - 2.0 * inset)?);
    // One unit (0.025 mm) of slack: a line on the very edge is not outside it.
    if shapes.iter().flatten().any(|&(x, y)| x < -1 || y < -1 || x > width + 1 || y > height + 1) {
        return Err(CutterError::OutsideMarks);
    }
    let mut body = String::from("IN;PA;");
    for &i in &order {
        let Some(shape) = shapes.get(i) else { continue };
        let Some(&(sx, sy)) = shape.first() else { continue };
        body.push_str(&format!("PU{sx},{sy};PD{sx},{sy};"));
        for &(x, y) in shape.iter().skip(1) {
            body.push_str(&format!("PD{x},{y};"));
        }
        body.push_str(&format!("PD{sx},{sy};"));
    }
    body.push_str(&format!("PU{width},0;!PG;"));
    // The header names the file's own size, which depends on the digits of that size.
    // The layout is checked: the inset is positive and under half the sheet, so this is exact.
    let edge = (job.layout.inset_mm * UNITS_PER_MM).round() as i64;
    let mut size = HEADER_BYTES + body.len();
    for _ in 0..4 {
        let head = header(&name, size, width, height, edge);
        let next = HEADER_BYTES.max(head.len()) + body.len();
        if next == size {
            break;
        }
        size = next;
    }
    let mut out = header(&name, size, width, height, edge).into_bytes();
    out.resize(HEADER_BYTES.max(out.len()), b' ');
    out.extend_from_slice(body.as_bytes());
    Ok(out)
}

fn header(name: &str, size: usize, width: i64, height: i64, inset: i64) -> String {
    format!("CR;NAME{name}.plt;SIZE{size}Bytes;SC1;MN4;MK1;MR1;TB25;{width},{height},{inset},{inset};CT1;")
}

/// Signed area (shoelace) of a closed shape; negative is clockwise with y up.
fn area(pts: &[Pt]) -> i128 {
    let mut a = 0i128;
    for (i, &(x0, y0)) in pts.iter().enumerate() {
        let (x1, y1) = pts.get(i + 1).copied().or(pts.first().copied()).unwrap_or((x0, y0));
        a += i128::from(x0) * i128::from(y1) - i128::from(x1) * i128::from(y0);
    }
    a
}

/// The shape drawn clockwise, starting at its leftmost point (the lowest when several share that x).
fn clockwise_from_left(mut pts: Vec<Pt>) -> Vec<Pt> {
    if area(&pts) > 0 {
        pts.reverse();
    }
    // Leftmost, and the lowest of those: a clockwise run up the left side then begins at its foot.
    let start = pts.iter().enumerate().min_by_key(|(_, p)| (p.0, p.1)).map_or(0, |(i, _)| i);
    pts.rotate_left(start);
    pts
}

/// Shape indices in cutting order: column by column from the left, up the first column, down the
/// next, and so on, so the blade travels the least between shapes.
fn serpentine(shapes: &[Vec<Pt>]) -> Vec<usize> {
    struct Box_ {
        i: usize,
        cx: i64,
        cy: i64,
        w: i64,
    }
    let mut boxes: Vec<Box_> = shapes
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let (x0, x1) = (s.iter().map(|p| p.0).min().unwrap_or(0), s.iter().map(|p| p.0).max().unwrap_or(0));
            let (y0, y1) = (s.iter().map(|p| p.1).min().unwrap_or(0), s.iter().map(|p| p.1).max().unwrap_or(0));
            Box_ { i, cx: (x0 + x1) / 2, cy: (y0 + y1) / 2, w: x1 - x0 }
        })
        .collect();
    boxes.sort_by_key(|b| (b.cx, b.cy, b.i));
    let mut columns: Vec<Vec<Box_>> = Vec::new();
    for b in boxes {
        // Same column when the centre is within half a shape's width of the column's first.
        let fits = columns.last().and_then(|c| c.first()).is_some_and(|f| (b.cx - f.cx).abs() * 2 <= f.w.max(b.w));
        match (fits, columns.last_mut()) {
            (true, Some(c)) => c.push(b),
            _ => columns.push(vec![b]),
        }
    }
    let mut order = Vec::with_capacity(shapes.len());
    for (n, mut c) in columns.into_iter().enumerate() {
        c.sort_by_key(|b| (b.cy, b.i));
        if n % 2 == 1 {
            c.reverse();
        }
        order.extend(c.into_iter().map(|b| b.i));
    }
    order
}
