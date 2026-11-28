//! Page tools a press-room imposition plug-in keeps beside its layouts: reverse the pages,
//! shuffle the even and odd pages, trim or enlarge pages and shift their content, and tile big
//! pages into smaller ones. Each makes a new document of one sheet per output page; pages are
//! placed, never redrawn, so their content, images and fonts are not rewritten. As with the other
//! layouts, links and bookmarks do not carry over.

use pdfcraft_content::Matrix;
use pdfcraft_print::{Placement, Sheet};

use crate::{EPS, ImposeError, Rect, check_sheet_count, non_negative, positive};

/// Most tiles one page may be split into.
pub const MAX_TILES_PER_PAGE: usize = 400;

/// A page as it is, on a sheet of its own size.
fn whole(page: usize, (w, h): (f64, f64)) -> Sheet {
    Sheet {
        size: (w, h),
        placed: vec![Placement { page, matrix: Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]), clip: [0.0, 0.0, w, h] }],
        ..Sheet::default()
    }
}

/// A page turned upside down, on a sheet of its own size.
fn upside_down(page: usize, (w, h): (f64, f64)) -> Sheet {
    Sheet { size: (w, h), placed: vec![Placement { page, matrix: Matrix([-1.0, 0.0, 0.0, -1.0, w, h]), clip: [0.0, 0.0, w, h] }], ..Sheet::default() }
}

fn check_sizes(sizes: &[(f64, f64)]) -> Result<(), ImposeError> {
    if sizes.is_empty() {
        return Err(ImposeError::NoPages);
    }
    check_sheet_count(sizes.len())?;
    for &(w, h) in sizes {
        positive(w, "page width")?;
        positive(h, "page height")?;
    }
    Ok(())
}

/// The pages in the opposite order.
pub fn reverse(sizes: &[(f64, f64)]) -> Result<Vec<Sheet>, ImposeError> {
    check_sizes(sizes)?;
    Ok(sizes.iter().enumerate().rev().map(|(p, s)| whole(p, *s)).collect())
}

/// How [`shuffle`] reorders the pages. Pages are counted from 1, so the odd pages are the
/// fronts of a two-sided document and the even pages its backs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Arrange {
    /// The order is kept; only the even pages are reversed or turned, if asked.
    #[default]
    Keep,
    /// All the odd pages first, then all the even pages: fronts and backs apart, for a printer
    /// that prints one side at a time.
    OddsFirst,
    /// The reverse of `OddsFirst`: the first half (rounded up) are fronts, the rest backs, and
    /// they are put back together front, back, front, back. Joins fronts and backs that were
    /// printed or scanned apart.
    Interleave,
}

/// Shuffle even/odd pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Shuffle {
    pub arrange: Arrange,
    /// The even pages (the backs) are taken in reverse order: with `OddsFirst` they come out last
    /// page first; with `Interleave` the backs are read as having come in last page first.
    pub reverse_evens: bool,
    /// The even pages are turned upside down.
    pub turn_evens: bool,
}

/// The new order of `n` pages under `spec`: each output page's source page (0-based) and whether
/// it is an even page (1-based), which `turn_evens` turns.
pub fn shuffle_order(n: usize, spec: &Shuffle) -> Vec<(usize, bool)> {
    let odds: Vec<usize> = (0..n).step_by(2).collect();
    let mut evens: Vec<usize> = (1..n).step_by(2).collect();
    match spec.arrange {
        Arrange::Keep => {
            if spec.reverse_evens {
                evens.reverse();
            }
            // The even positions take the evens in their (maybe reversed) order.
            let mut out = Vec::with_capacity(n);
            let mut e = evens.into_iter();
            for i in 0..n {
                if i.is_multiple_of(2) {
                    out.push((i, false));
                } else if let Some(p) = e.next() {
                    out.push((p, true));
                }
            }
            out
        }
        Arrange::OddsFirst => {
            if spec.reverse_evens {
                evens.reverse();
            }
            odds.into_iter().map(|p| (p, false)).chain(evens.into_iter().map(|p| (p, true))).collect()
        }
        Arrange::Interleave => {
            let fronts = n.div_ceil(2);
            let mut backs: Vec<usize> = (fronts..n).collect();
            if spec.reverse_evens {
                backs.reverse();
            }
            let mut out = Vec::with_capacity(n);
            for i in 0..fronts {
                out.push((i, false));
                if let Some(&b) = backs.get(i) {
                    out.push((b, true));
                }
            }
            out
        }
    }
}

/// The pages in the order `spec` gives.
pub fn shuffle(sizes: &[(f64, f64)], spec: &Shuffle) -> Result<Vec<Sheet>, ImposeError> {
    check_sizes(sizes)?;
    let mut out = Vec::with_capacity(sizes.len());
    for (page, even) in shuffle_order(sizes.len(), spec) {
        let Some(&size) = sizes.get(page) else { continue };
        out.push(if even && spec.turn_evens { upside_down(page, size) } else { whole(page, size) });
    }
    Ok(out)
}

/// Which pages a page tool works on; the others are kept as they are.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum PageSet {
    #[default]
    All,
    /// Pages 1, 3, 5, …
    Odd,
    /// Pages 2, 4, 6, …
    Even,
    /// These pages (0-based).
    Listed(Vec<usize>),
}

impl PageSet {
    fn contains(&self, page: usize) -> bool {
        match self {
            PageSet::All => true,
            PageSet::Odd => page.is_multiple_of(2),
            PageSet::Even => !page.is_multiple_of(2),
            PageSet::Listed(v) => v.contains(&page),
        }
    }
}

/// How [`trim_and_shift`] changes the size of a page.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Trim {
    #[default]
    None,
    /// Left, bottom, right and top amounts, in points: positive trims space off that edge,
    /// negative adds space beyond it (white: nothing outside the page is uncovered).
    Edges([f64; 4]),
    /// Every page made this size (points), its content centred: cut where it is larger, white
    /// margins where it is smaller.
    SameSize(f64, f64),
}

/// How [`trim_and_shift`] moves the content of a page (points; x to the right, y up).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Shift {
    #[default]
    None,
    /// Every page's content by the same amount (a binding margin, a printer offset).
    Fixed(f64, f64),
    /// From the first page's amount to the last's, the pages in between in proportion.
    Variable { first: (f64, f64), last: (f64, f64) },
}

/// Trim and shift.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TrimShift {
    pub pages: PageSet,
    pub trim: Trim,
    pub shift: Shift,
}

/// Each page trimmed or enlarged and its content shifted as `spec` says. Content never shows
/// past the page's old edge (nothing outside the page is uncovered); the space a trim takes away
/// or a shift moves off is gone. Pages outside `spec.pages` are kept as they are.
pub fn trim_and_shift(sizes: &[(f64, f64)], spec: &TrimShift) -> Result<Vec<Sheet>, ImposeError> {
    check_sizes(sizes)?;
    match spec.trim {
        Trim::None => {}
        Trim::Edges(e) => {
            if e.iter().any(|v| !v.is_finite()) {
                return Err(ImposeError::Invalid("trim", "finite"));
            }
        }
        Trim::SameSize(w, h) => {
            positive(w, "page width")?;
            positive(h, "page height")?;
        }
    }
    let finite = |(x, y): (f64, f64)| x.is_finite() && y.is_finite();
    match spec.shift {
        Shift::None => {}
        Shift::Fixed(x, y) if finite((x, y)) => {}
        Shift::Variable { first, last } if finite(first) && finite(last) => {}
        _ => return Err(ImposeError::Invalid("shift", "finite")),
    }
    let chosen: Vec<usize> = (0..sizes.len()).filter(|p| spec.pages.contains(*p)).collect();
    if chosen.is_empty() {
        return Err(ImposeError::Invalid("page selection", "with at least one page of the document"));
    }
    let span = chosen.len().saturating_sub(1).max(1) as f64;
    let mut out = Vec::with_capacity(sizes.len());
    let mut k = 0usize;
    for (page, &(w, h)) in sizes.iter().enumerate() {
        if !spec.pages.contains(page) {
            out.push(whole(page, (w, h)));
            continue;
        }
        let t = k as f64 / span;
        k += 1;
        let (sx, sy) = match spec.shift {
            Shift::None => (0.0, 0.0),
            Shift::Fixed(x, y) => (x, y),
            Shift::Variable { first, last } => (first.0 + (last.0 - first.0) * t, first.1 + (last.1 - first.1) * t),
        };
        // The new page and where the old page's origin lands on it.
        let (nw, nh, ox, oy) = match spec.trim {
            Trim::None => (w, h, 0.0, 0.0),
            Trim::Edges([l, b, r, top]) => (w - l - r, h - b - top, -l, -b),
            Trim::SameSize(tw, th) => (tw, th, (tw - w) / 2.0, (th - h) / 2.0),
        };
        if !(nw > EPS && nh > EPS) {
            return Err(ImposeError::Invalid("trim", "leaving some of every page"));
        }
        let (ox, oy) = (ox + sx, oy + sy);
        // What of the old page shows on the new one: the new page seen in old-page space, cut to
        // the old page.
        let clip: Rect = [(-ox).max(0.0), (-oy).max(0.0), (nw - ox).min(w), (nh - oy).min(h)];
        if !(clip[2] - clip[0] > EPS && clip[3] - clip[1] > EPS) {
            return Err(ImposeError::OffSheet { page: page.saturating_add(1) });
        }
        out.push(Sheet { size: (nw, nh), placed: vec![Placement { page, matrix: Matrix([1.0, 0.0, 0.0, 1.0, ox, oy]), clip }], ..Sheet::default() });
    }
    Ok(out)
}

/// Tile pages.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    pub cols: usize,
    pub rows: usize,
    /// How much neighbouring tiles share (points), to help put the pieces back together.
    pub overlap: f64,
    /// Only pages wider than they are tall are split (spreads among single pages); the others are
    /// kept whole.
    pub only_wide: bool,
}

/// Each page split into `cols` × `rows` equal tiles that overlap by `spec.overlap`, from the top
/// left, row by row. Each tile is a page of its own.
pub fn tile(sizes: &[(f64, f64)], spec: &Tile) -> Result<Vec<Sheet>, ImposeError> {
    check_sizes(sizes)?;
    non_negative(spec.overlap, "overlap")?;
    if spec.cols == 0 || spec.rows == 0 {
        return Err(ImposeError::Invalid("columns and rows", "at least 1"));
    }
    if spec.cols.checked_mul(spec.rows).is_none_or(|n| n > MAX_TILES_PER_PAGE) {
        return Err(ImposeError::TooLarge("more tiles on one page than are supported"));
    }
    let (cols, rows) = (spec.cols as f64, spec.rows as f64);
    let mut out = Vec::new();
    for (page, &(w, h)) in sizes.iter().enumerate() {
        if spec.only_wide && w <= h {
            out.push(whole(page, (w, h)));
            continue;
        }
        // n tiles of width t overlapping by o cover n t - (n - 1) o = w.
        let (tw, th) = ((w + (cols - 1.0) * spec.overlap) / cols, (h + (rows - 1.0) * spec.overlap) / rows);
        if (spec.cols > 1 && spec.overlap >= tw - EPS) || (spec.rows > 1 && spec.overlap >= th - EPS) {
            return Err(ImposeError::Invalid("overlap", "smaller than a tile"));
        }
        for r in 0..spec.rows {
            for c in 0..spec.cols {
                let x0 = c as f64 * (tw - spec.overlap);
                let y1 = h - r as f64 * (th - spec.overlap);
                let clip = [x0, y1 - th, x0 + tw, y1];
                out.push(Sheet {
                    size: (tw, th),
                    placed: vec![Placement { page, matrix: Matrix([1.0, 0.0, 0.0, 1.0, -x0, th - y1]), clip }],
                    ..Sheet::default()
                });
            }
        }
        check_sheet_count(out.len())?;
    }
    Ok(out)
}
