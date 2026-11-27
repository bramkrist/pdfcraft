//! Cut lines for sticker sheets: where a cutter should follow each placed page.
//!
//! This crate stays geometry only. Tracing a page's silhouette needs a rendering, so the caller
//! (the engine) asks [`cut_requests`] which traces are needed, makes them with
//! `pdfcraft-cutline`, and hands the outlines back to [`place_cuts`], which moves them to
//! where each copy of the page sits on its sheet.

use pdfcraft_cutline::{Background, CutError, Holes, Mask, TraceOptions, trace};

/// How far, in pixels, a simplified cut path may stray from the traced one. A rendered silhouette
/// is a staircase up to ~0.7 px deep along every slope, which a finer tolerance keeps as a jagged
/// path the cutter would chatter along; at the 10 px/mm the trace runs at, 0.8 px is 0.08 mm.
const STAIRCASE_TOLERANCE: f64 = 0.8;
use pdfcraft_print::{CutPaint, Placement, Sheet};

/// How the cut lines are made.
#[derive(Clone, Debug, PartialEq)]
pub struct CutSpec {
    /// The distance from the artwork's edge to the cut line, in points, measured on the sheet
    /// (so after any scaling): positive puts the line outside the artwork, negative inside it,
    /// zero on its edge.
    pub offset: f64,
    /// Whether to cut around empty areas inside the artwork as well.
    pub holes: Holes,
    /// What counts as empty paper on the page.
    pub background: Background,
    /// Pieces of artwork smaller than this many square points get no cut line (specks).
    pub min_area: f64,
    /// The ink and weight of the cut line.
    pub paint: CutPaint,
}

impl CutSpec {
    /// A cut line on the artwork's edge, outer outline only.
    pub fn new() -> Self {
        // One square millimetre.
        Self { offset: 0.0, holes: Holes::Fill, background: Background::Auto, min_area: (72.0 / 25.4) * (72.0 / 25.4), paint: CutPaint::default() }
    }
}

impl Default for CutSpec {
    fn default() -> Self {
        Self::new()
    }
}

/// One trace the layout needs: `page` as it is placed (scaled by `scale`, visible inside `clip`
/// in its own display space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutRequest {
    pub page: usize,
    pub scale: f64,
    pub clip: [f64; 4],
}

/// The outlines found for a [`CutRequest`], as closed polygons in the page's display space
/// (points, origin at the lower left, y up).
#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    pub request: CutRequest,
    pub loops: Vec<Vec<(f64, f64)>>,
}

/// Pixels per point of the sheet that a trace is made at: 254 dpi, a tenth of a millimetre a pixel.
const TRACE_PX_PER_PT: f64 = 254.0 / 72.0;

/// The most pixels a page is traced at. The tracer pads the page by the offset and takes at most
/// 16 million cells, so this leaves room for an offset of a few millimetres on any page.
const MAX_TRACE_PIXELS: f64 = 9_000_000.0;

/// The resolution, in pixels per point of the page (of `size` points), to draw a page at to
/// trace it as `request` places it: a tenth of a millimetre of the sheet a pixel, fewer on a page
/// too big for that. The renderer may lower it further for its own limits.
pub fn trace_resolution(size: (f64, f64), request: &CutRequest) -> f64 {
    let area = (size.0 * size.1).max(1.0);
    (TRACE_PX_PER_PT * request.scale).min((MAX_TRACE_PIXELS / area).sqrt())
}

/// A page drawn for tracing: `width` × `height` pixels of RGBA, row by row from the top, covering
/// the page's `size` (points).
#[derive(Clone, Copy, Debug)]
pub struct Raster<'a> {
    pub width: usize,
    pub height: usize,
    pub rgba: &'a [u8],
    /// The colours are not multiplied by the alpha (a browser canvas), unlike a renderer's.
    pub straight: bool,
    pub size: (f64, f64),
}

/// The cut lines of one placement of a page, from the page drawn as `raster`: moved by
/// `spec.offset` (on the sheet, so `1 / request.scale` as far on the page), limited to what the
/// placement's clip shows, in the page's display space.
pub fn trace_page(raster: &Raster<'_>, request: &CutRequest, spec: &CutSpec) -> Result<Outline, CutError> {
    let (w, h) = raster.size;
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 && request.scale.is_finite() && request.scale > 0.0) {
        return Err(CutError::NotFinite);
    }
    let mut mask = if raster.straight {
        Mask::from_straight_rgba(raster.width, raster.height, raster.rgba, spec.background)?
    } else {
        Mask::from_rgba(raster.width, raster.height, raster.rgba, spec.background)?
    };
    // Pixels per point of the page, each way (the size rounds up to whole pixels).
    let (sx, sy) = (raster.width as f64 / w, raster.height as f64 / h);
    let [x0, y0, x1, y1] = request.clip;
    if !(x0 <= 0.0 && y0 <= 0.0 && x1 >= w && y1 >= h) {
        // Display space is y up, the mask y down.
        mask.keep_only(x0 * sx, (h - y1) * sy, x1 * sx, (h - y0) * sy);
    }
    let options = TraceOptions {
        offset: spec.offset / request.scale * (sx + sy) / 2.0,
        holes: spec.holes,
        min_area: spec.min_area / (request.scale * request.scale) * sx * sy,
        tolerance: STAIRCASE_TOLERANCE,
        ..TraceOptions::default()
    };
    let loops = trace(&mask, &options)?.into_iter().map(|l| l.points.into_iter().map(|(x, y)| (x / sx, h - y / sy)).collect()).collect();
    Ok(Outline { request: *request, loops })
}

/// The factor a placement's matrix scales lengths by (turns and mirroring do not change it).
pub fn placement_scale(p: &Placement) -> f64 {
    let [a, b, c, d, ..] = p.matrix.0;
    (a * d - b * c).abs().sqrt()
}

pub(crate) fn request_of(p: &Placement) -> CutRequest {
    CutRequest { page: p.page, scale: placement_scale(p), clip: p.clip }
}

fn same(a: &CutRequest, b: &CutRequest) -> bool {
    let near = |x: f64, y: f64, tol: f64| (x - y).abs() <= tol * x.abs().max(y.abs()).max(1.0);
    a.page == b.page && near(a.scale, b.scale, 1e-9) && a.clip.iter().zip(&b.clip).all(|(x, y)| near(*x, *y, 1e-9))
}

/// The distinct traces `sheets` need, in order of first use: one per page, scale and visible
/// part, however many times that page is repeated.
pub fn cut_requests(sheets: &[Sheet]) -> Vec<CutRequest> {
    let mut out: Vec<CutRequest> = Vec::new();
    for placement in sheets.iter().flat_map(|s| &s.placed) {
        let request = request_of(placement);
        if request.scale.is_finite() && request.scale > 0.0 && !out.iter().any(|r| same(r, &request)) {
            out.push(request);
        }
    }
    out
}

/// Set every sheet's cut lines to the `outlines` of the pages placed on it, moved to where each
/// placement sits (scaled, turned or mirrored with it). Earlier cut lines are replaced.
pub fn place_cuts(sheets: &mut [Sheet], outlines: &[Outline], paint: &CutPaint) {
    for sheet in sheets {
        let mut cuts = Vec::new();
        for placement in &sheet.placed {
            let request = request_of(placement);
            let Some(found) = outlines.iter().find(|o| same(&o.request, &request)) else { continue };
            let [a, b, c, d, e, f] = placement.matrix.0;
            for outline in &found.loops {
                cuts.push(outline.iter().map(|&(x, y)| (a * x + c * y + e, b * x + d * y + f)).collect());
            }
        }
        sheet.cuts = cuts;
        sheet.cut_paint = paint.clone();
    }
}
