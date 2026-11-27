//! Cut lines for sticker sheets: the traces that [`pdfcraft_impose::cut_requests`] asks for.
//!
//! Each distinct placement of a page (its scale and visible part) is drawn the way the print
//! draws it, on a clear page; what is drawn is the artwork, and the line that goes round it is
//! traced at the signed offset the cut spec asks for, measured on the sheet. `pdfcraft-impose`
//! then moves the outlines to every copy of the page.

use std::sync::Arc;

use pdfcraft_impose::{CutRequest, CutSpec, Outline, Raster, trace_page, trace_resolution};
use pdfcraft_render::{PRINT_TAG, PageRenderer, RenderConfig, RenderRequest, effective_scale};

/// What a cut-line run found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CutStats {
    /// Distinct placements traced.
    pub traces: usize,
    /// Cut lines on the sheets, each copy of a page counted.
    pub paths: usize,
    /// Placements with nothing to cut (blank, or the offset left nothing).
    pub empty: usize,
}

/// The outline of every request in `requests`, from the PDF `bytes` whose pages are `sizes`
/// (display size in points). `config` renders them; it is made transparent here.
pub(crate) fn trace_outlines(
    bytes: Arc<Vec<u8>>,
    config: &RenderConfig,
    sizes: &[(f64, f64)],
    requests: &[CutRequest],
    spec: &CutSpec,
) -> Result<Vec<Outline>, String> {
    let mut renderer = PageRenderer::new(bytes, RenderConfig { transparent: true, hide_comments: false, ..config.clone() });
    requests.iter().map(|request| outline_of(&mut renderer, sizes, request, spec)).collect()
}

fn outline_of(renderer: &mut PageRenderer, sizes: &[(f64, f64)], request: &CutRequest, spec: &CutSpec) -> Result<Outline, String> {
    let label = request.page.saturating_add(1);
    let &size = sizes.get(request.page).ok_or_else(|| format!("page {label} does not exist"))?;
    if !(size.0.is_finite() && size.1.is_finite() && size.0 > 0.0 && size.1 > 0.0) {
        return Err(format!("page {label} has no size to trace"));
    }
    let scale = effective_scale(size.0 as f32, size.1 as f32, trace_resolution(size, request) as f32);
    let page = renderer.render(RenderRequest { page: request.page, scale, tag: PRINT_TAG, ..Default::default() });
    if let Some(e) = page.error {
        return Err(format!("page {label}: {e}"));
    }
    let raster = Raster { width: page.width as usize, height: page.height as usize, rgba: &page.rgba, straight: false, size };
    trace_page(&raster, request, spec).map_err(|e| format!("page {label}: {e}"))
}

/// `src` laid out by `spec` with a cut line round every placed page: the layout and the traces
/// of what it places, as sheets that carry the lines.
pub(crate) fn cut_sheets(
    src: &pdfcraft_cos::Document,
    config: &RenderConfig,
    spec: &pdfcraft_impose::StepRepeat,
    cut: &CutSpec,
    content: pdfcraft_print::Content,
) -> Result<(Vec<pdfcraft_print::Sheet>, CutStats), String> {
    let sizes = pdfcraft_print::page_sizes(src);
    let mut sheets = pdfcraft_impose::step_and_repeat(&sizes, spec).map_err(|e| e.to_string())?;
    let requests = pdfcraft_impose::cut_requests(&sheets);
    // The traces see what prints: annotations that don't print are gone from this copy.
    let mut view = src.clone();
    pdfcraft_print::keep_printed(&mut view, content);
    let bytes = pdfcraft_cos::write_incremental(&view, &pdfcraft_cos::SaveOptions::default()).map_err(|e| e.to_string())?;
    let outlines = trace_outlines(Arc::new(bytes), config, &sizes, &requests, cut)?;
    pdfcraft_impose::place_cuts(&mut sheets, &outlines, &cut.paint);
    let stats = CutStats {
        traces: outlines.len(),
        paths: sheets.iter().map(|s| s.cuts.len()).sum(),
        empty: outlines.iter().filter(|o| o.loops.is_empty()).count(),
    };
    Ok((sheets, stats))
}

/// [`cut_sheets`], written as a print-ready PDF.
pub(crate) fn impose_with_cuts(
    src: &pdfcraft_cos::Document,
    config: &RenderConfig,
    spec: &pdfcraft_impose::StepRepeat,
    cut: &CutSpec,
    content: pdfcraft_print::Content,
) -> Result<(Vec<u8>, CutStats), String> {
    let (sheets, stats) = cut_sheets(src, config, spec, cut, content)?;
    let pdf = pdfcraft_print::impose_sheets(src, &sheets, content).map_err(|e| e.to_string())?;
    Ok((pdf, stats))
}

/// A cut-line run's output: the print-ready PDF, what was cut and, when a plotter-cutter's marks
/// were asked for, the PLT file of each sheet that has cut lines.
#[derive(Clone, Debug, PartialEq)]
pub struct CutJob {
    pub pdf: Vec<u8>,
    pub stats: CutStats,
    pub files: Vec<pdfcraft_impose::CutterFile>,
}

/// [`impose_with_cuts`], with `cutter`'s marks and code on every sheet that has cut lines and
/// their PLT files. Nothing is made when the cutter's output cannot be (a cut line off the marks,
/// no room for the marks, nothing cut).
pub(crate) fn cut_job(
    src: &pdfcraft_cos::Document,
    config: &RenderConfig,
    spec: &pdfcraft_impose::StepRepeat,
    cut: &CutSpec,
    cutter: Option<&pdfcraft_impose::CutterSpec>,
    content: pdfcraft_print::Content,
) -> Result<CutJob, String> {
    let (mut sheets, stats) = cut_sheets(src, config, spec, cut, content)?;
    let files = match cutter {
        Some(cutter) => pdfcraft_impose::apply_cutter(&mut sheets, cutter).map_err(|e| e.to_string())?,
        None => Vec::new(),
    };
    let pdf = pdfcraft_print::impose_sheets(src, &sheets, content).map_err(|e| e.to_string())?;
    Ok(CutJob { pdf, stats, files })
}
