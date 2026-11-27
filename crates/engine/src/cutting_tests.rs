//! Cut lines through the engine: a disc of known size, traced and placed on sheets.

use std::sync::Arc;

use pdfcraft_impose::{CutSpec, StepRepeat};
use pdfcraft_render::RenderConfig;

use crate::{DocId, Session, print};

/// Points in a millimetre.
const MM: f64 = 72.0 / 25.4;

/// A one-page 100 × 100 pt document: a blue disc of radius 30 at the centre, nothing else.
fn disc_pdf() -> Vec<u8> {
    let (cx, cy, r, k) = (50.0, 50.0, 30.0, 0.552_284_75 * 30.0);
    let content = format!(
        "0 0.4 1 rg {} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c f",
        cx + r,
        cy,
        cx + r,
        cy + k,
        cx + k,
        cy + r,
        cx,
        cy + r,
        cx - k,
        cy + r,
        cx - r,
        cy + k,
        cx - r,
        cy,
        cx - r,
        cy - k,
        cx - k,
        cy - r,
        cx,
        cy - r,
        cx + k,
        cy - r,
        cx + r,
        cy - k,
        cx + r,
        cy,
    );
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        bytes.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    bytes
}

fn cos() -> pdfcraft_cos::Document {
    pdfcraft_cos::Document::open_with_password(Arc::new(disc_pdf()), None).expect("open")
}

/// Nine 100 × 100 cells on a 300 × 300 sheet.
fn nine_up() -> StepRepeat {
    StepRepeat::new((300.0, 300.0))
}

fn at(offset_mm: f64) -> CutSpec {
    CutSpec { offset: offset_mm * MM, ..CutSpec::new() }
}

fn cut_sheets(spec: &StepRepeat, cut: &CutSpec) -> (Vec<print::Sheet>, crate::cutting::CutStats) {
    crate::cutting::cut_sheets(&cos(), &RenderConfig::default(), spec, cut, print::Content::default()).expect("cut sheets")
}

/// The bounding box of a polygon.
fn bounds(path: &[(f64, f64)]) -> [f64; 4] {
    let xs = path.iter().map(|p| p.0);
    let ys = path.iter().map(|p| p.1);
    [xs.clone().fold(f64::MAX, f64::min), ys.clone().fold(f64::MAX, f64::min), xs.fold(f64::MIN, f64::max), ys.fold(f64::MIN, f64::max)]
}

fn assert_near(actual: f64, expected: f64, what: &str) {
    assert!((actual - expected).abs() < 0.4, "{what}: {actual} is not near {expected}");
}

#[test]
fn every_copy_gets_one_cut_line_from_a_single_trace() {
    let (sheets, stats) = cut_sheets(&nine_up(), &at(0.0));
    assert_eq!(stats.traces, 1, "one page, one scale, one trace");
    assert_eq!(stats.paths, 9);
    assert_eq!(stats.empty, 0);
    assert_eq!(sheets.len(), 1);
    assert_eq!(sheets[0].cuts.len(), 9);
}

#[test]
fn the_offset_moves_the_line_out_of_and_into_the_artwork() {
    // The disc's radius is 30 pt, so on a cell at the sheet's origin it spans 20..80 each way.
    let radius = |mm: f64| {
        let (sheets, _) = cut_sheets(&nine_up(), &at(mm));
        let b = sheets[0].cuts.iter().map(|c| bounds(c)).find(|b| b[0] < 100.0 && b[1] < 100.0).expect("the lower-left copy");
        assert_near(b[2] - b[0], b[3] - b[1], "the line is round");
        (b[2] - b[0]) / 2.0
    };
    assert_near(radius(0.0), 30.0, "on the edge");
    assert_near(radius(2.0), 30.0 + 2.0 * MM, "2 mm outside");
    assert_near(radius(-2.0), 30.0 - 2.0 * MM, "2 mm inside");
}

#[test]
fn the_offset_is_measured_on_the_sheet_after_scaling() {
    let spec = StepRepeat { scale: 0.5, ..nine_up() };
    let (sheets, stats) = cut_sheets(&spec, &at(2.0));
    assert!(stats.paths > 9, "a half-size page fits more copies: {}", stats.paths);
    let b = bounds(&sheets[0].cuts[0]);
    // 15 pt radius on the sheet, plus 2 mm on the sheet (not 1 mm, which is 2 mm on the page).
    assert_near((b[2] - b[0]) / 2.0, 15.0 + 2.0 * MM, "radius on the sheet");
}

#[test]
fn an_offset_that_leaves_nothing_cuts_nothing() {
    let (sheets, stats) = cut_sheets(&nine_up(), &at(-12.0));
    assert!(sheets.iter().all(|s| s.cuts.is_empty()));
    assert_eq!((stats.paths, stats.empty), (0, 1));
}

/// Whether the sheet PDF `bytes` has the cut spot colour on its first page and a layer in its catalogue.
fn has_cut_layer(bytes: &[u8]) -> (bool, bool) {
    let doc = pdfcraft_cos::Document::open_with_password(Arc::new(bytes.to_vec()), None).expect("reopen");
    let page = pdfcraft_model::pages(&doc).remove(0);
    let res = page.dict.get(b"Resources").and_then(|r| doc.resolve(r).as_dict().cloned()).unwrap_or_default();
    let spot = res.get(b"ColorSpace").and_then(|c| doc.resolve(c).as_dict().cloned()).is_some_and(|d| d.get(b"CSCut").is_some());
    let root = doc.trailer().get(b"Root").and_then(|r| doc.resolve(r).as_dict().cloned()).unwrap_or_default();
    (spot, root.get(b"OCProperties").is_some())
}

#[test]
fn the_session_writes_the_cut_layer() {
    let mut session = Session::new();
    let id: DocId = session.open_new("disc.pdf", Arc::new(disc_pdf())).expect("open");
    let (pdf, stats) = session.impose_cut_pdf(id, &nine_up(), &at(1.0), print::Content::default()).expect("impose");
    assert_eq!(stats.paths, 9);
    assert_eq!(has_cut_layer(&pdf), (true, true));
    // Without the feature the same document has neither.
    let plain = session.impose_pdf(id, &nine_up(), print::Content::default()).expect("impose");
    assert_eq!(has_cut_layer(&plain), (false, false));
}

#[test]
fn the_cut_line_is_drawn_round_the_disc_when_the_sheet_is_rendered() {
    let mut session = Session::new();
    let id: DocId = session.open_new("disc.pdf", Arc::new(disc_pdf())).expect("open");
    let (pdf, _) = session.impose_cut_pdf(id, &nine_up(), &at(2.0), print::Content::default()).expect("impose");
    let mut renderer = pdfcraft_render::PageRenderer::new(Arc::new(pdf), RenderConfig::default());
    let page = renderer.render(pdfcraft_render::RenderRequest { page: 0, scale: 2.0, tag: pdfcraft_render::PRINT_TAG, ..Default::default() });
    assert!(page.error.is_none(), "{:?}", page.error);
    let (w, h) = (page.width as usize, page.height as usize);
    // Pixels of the line's stand-in colour (magenta) on the row through the centre of the
    // lower-left copy: two crossings, one each side of the disc, 2 mm outside its edge.
    let row = (h as f64 - 50.0 * 2.0) as usize;
    let px = |x: usize| page.rgba.get((row * w + x) * 4..(row * w + x) * 4 + 4).map(|p| (p[0], p[1], p[2])).unwrap_or_default();
    let magenta: Vec<usize> = (0..w / 3)
        .filter(|&x| {
            let (r, g, b) = px(x);
            r > 180 && b > 180 && g < 120
        })
        .collect();
    assert!(magenta.len() >= 2, "no cut line on the row: {magenta:?}");
    let (first, last) = (*magenta.first().unwrap_or(&0) as f64 / 2.0, *magenta.last().unwrap_or(&0) as f64 / 2.0);
    assert_near(last - first, 2.0 * (30.0 + 2.0 * MM), "the line's width on the sheet");
}

/// 4 × 3 discs on a 480 × 320 mm sheet, kept 14 mm in from the edges: inside the marks (10 mm in).
fn cutter_sheet() -> StepRepeat {
    let mut spec = StepRepeat::new((480.0 * MM, 320.0 * MM));
    spec.margins = pdfcraft_impose::Margins::uniform(14.0 * MM);
    spec
}

fn cutter() -> pdfcraft_impose::CutterSpec {
    pdfcraft_impose::CutterSpec { name: "271739".into(), layout: pdfcraft_impose::cutter::Layout::default() }
}

#[test]
fn a_cutter_job_has_marks_on_the_sheet_and_a_plt_of_the_cut_lines() {
    let mut session = Session::new();
    let id: DocId = session.open_new("disc.pdf", Arc::new(disc_pdf())).expect("open");
    let job = session.impose_cut_job(id, &cutter_sheet(), &at(1.0), Some(&cutter()), print::Content::default()).expect("job");
    assert_eq!(job.files.len(), 1);
    assert_eq!(job.files[0].name, "271739.plt");
    let plt = String::from_utf8_lossy(&job.files[0].bytes).into_owned();
    assert!(plt.starts_with("CR;NAME271739.plt;SIZE"), "{}", &plt[..60]);
    // Every disc has a closed PU/PD shape in the file: as many as the sheet has cut lines.
    assert_eq!(plt.matches("PU").count(), job.stats.paths + 1, "one shape each, and the final park");
    // The lower-left mark is black where the sheet is rendered (10 mm in from both edges).
    let mut renderer = pdfcraft_render::PageRenderer::new(Arc::new(job.pdf), RenderConfig::default());
    let page = renderer.render(pdfcraft_render::RenderRequest { page: 0, scale: 1.0, tag: pdfcraft_render::PRINT_TAG, ..Default::default() });
    assert!(page.error.is_none(), "{:?}", page.error);
    let (w, h) = (page.width as usize, page.height as usize);
    let at_mm = |x_mm: f64, y_mm: f64| {
        let (x, y) = ((x_mm * MM) as usize, h - (y_mm * MM) as usize);
        page.rgba.get((y * w + x) * 4..(y * w + x) * 4 + 3).map(|p| p.iter().map(|&v| u32::from(v)).sum::<u32>()).unwrap_or(u32::MAX)
    };
    for (x, y) in [(10.0, 10.0), (470.0, 10.0), (10.0, 310.0), (470.0, 310.0)] {
        assert!(at_mm(x, y) < 150, "a mark at ({x}, {y}) mm: {}", at_mm(x, y));
    }
    assert!(at_mm(240.0, 100.0) > 300, "no mark in the middle of the sheet");
    // Without a cutter, no files and no marks.
    let plain = session.impose_cut_job(id, &cutter_sheet(), &at(1.0), None, print::Content::default()).expect("job");
    assert!(plain.files.is_empty());
}

#[test]
fn a_cutter_job_whose_cut_lines_run_off_the_marks_is_refused() {
    let mut session = Session::new();
    let id: DocId = session.open_new("disc.pdf", Arc::new(disc_pdf())).expect("open");
    // Nine-up on a 106 mm sheet: the outer discs' lines come within 7 mm of its edge, outside the
    // marks (10 mm in).
    let err = session.impose_cut_job(id, &nine_up(), &at(1.0), Some(&cutter()), print::Content::default()).expect_err("refused");
    assert!(err.to_string().contains("outside the registration marks"), "{err}");
}
