use pdfcraft_content::Matrix;
use pdfcraft_cutline::{Background, Holes};
use pdfcraft_print::{CutPaint, Placement, Sheet};

use crate::cut::{CutRequest, CutSpec, Outline, cut_requests, place_cuts, placement_scale, request_of};

const MM: f64 = 72.0 / 25.4;

fn placement(page: usize, m: [f64; 6], clip: [f64; 4]) -> Placement {
    Placement { page, matrix: Matrix(m), clip }
}

fn sheet(placed: Vec<Placement>) -> Sheet {
    Sheet { size: (400.0, 400.0), placed, ..Sheet::default() }
}

const FULL: [f64; 4] = [0.0, 0.0, 100.0, 50.0];

fn outline_for(p: &Placement, loops: Vec<Vec<(f64, f64)>>) -> Outline {
    Outline { request: request_of(p), loops }
}

#[test]
fn one_trace_serves_every_copy_of_a_page() {
    let copies: Vec<Placement> = (0..6).map(|i| placement(0, [1.0, 0.0, 0.0, 1.0, 10.0 + 110.0 * i as f64, 20.0], FULL)).collect();
    let requests = cut_requests(&[sheet(copies)]);
    assert_eq!(requests, vec![CutRequest { page: 0, scale: 1.0, clip: FULL }]);
}

#[test]
fn a_different_page_scale_or_visible_part_needs_its_own_trace() {
    let a = placement(0, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], FULL);
    let other_page = placement(1, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], FULL);
    let scaled = placement(0, [2.0, 0.0, 0.0, 2.0, 0.0, 0.0], FULL);
    let cropped = placement(0, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 60.0, 50.0]);
    assert_eq!(cut_requests(&[sheet(vec![a, other_page, scaled, cropped])]).len(), 4);
}

#[test]
fn turning_or_mirroring_does_not_change_the_scale() {
    let turned = placement(0, [0.0, 2.0, -2.0, 0.0, 0.0, 0.0], FULL);
    let mirrored = placement(0, [-2.0, 0.0, 0.0, 2.0, 0.0, 0.0], FULL);
    assert!((placement_scale(&turned) - 2.0).abs() < 1e-12);
    assert!((placement_scale(&mirrored) - 2.0).abs() < 1e-12);
    assert_eq!(cut_requests(&[sheet(vec![turned, mirrored])]).len(), 1);
}

#[test]
fn a_placement_with_no_usable_scale_asks_for_nothing() {
    let flat = placement(0, [0.0; 6], FULL);
    let nan = placement(0, [f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0], FULL);
    assert!(cut_requests(&[sheet(vec![flat, nan])]).is_empty());
}

#[test]
fn outlines_are_moved_to_each_placement() {
    let square = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
    let at = |x: f64| placement(0, [1.0, 0.0, 0.0, 1.0, x, 5.0], FULL);
    let mut sheets = vec![sheet(vec![at(100.0), at(300.0)])];
    place_cuts(&mut sheets, &[outline_for(&at(0.0), vec![square])], &CutPaint::default());
    assert_eq!(sheets[0].cuts.len(), 2);
    assert_eq!(sheets[0].cuts[0], vec![(100.0, 5.0), (110.0, 5.0), (110.0, 15.0), (100.0, 15.0)]);
    assert_eq!(sheets[0].cuts[1][0], (300.0, 5.0));
}

#[test]
fn a_scaled_turned_copy_gets_the_outline_scaled_and_turned_with_it() {
    // 3x, turned a quarter anticlockwise: (x, y) -> (-3y + 200, 3x).
    let m = [0.0, 3.0, -3.0, 0.0, 200.0, 0.0];
    let p = placement(0, m, FULL);
    let mut sheets = vec![sheet(vec![p])];
    place_cuts(&mut sheets, &[outline_for(&p, vec![vec![(10.0, 0.0), (10.0, 20.0), (0.0, 20.0)]])], &CutPaint::default());
    assert_eq!(sheets[0].cuts[0], vec![(200.0, 30.0), (140.0, 30.0), (140.0, 0.0)]);
}

#[test]
fn a_page_without_an_outline_gets_no_cut_and_old_cuts_are_replaced() {
    let p = placement(0, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], FULL);
    let mut s = sheet(vec![p]);
    s.cuts = vec![vec![(1.0, 1.0), (2.0, 2.0), (3.0, 1.0)]];
    let mut sheets = vec![s];
    place_cuts(&mut sheets, &[], &CutPaint { name: "Thru".into(), width: 1.0 });
    assert!(sheets[0].cuts.is_empty());
    assert_eq!(sheets[0].cut_paint.name, "Thru");
}

#[test]
fn the_default_spec_cuts_on_the_artwork_edge_outside_only() {
    let s = CutSpec::new();
    assert_eq!((s.offset, s.holes, s.background), (0.0, Holes::Fill, Background::Auto));
    assert!((s.min_area - 1.0 * MM * MM).abs() < 1e-9, "one square millimetre");
}

/// A 20 pt × 10 pt page drawn at 10 pixels a point: a solid 8 pt × 4 pt box at (6, 3) in display
/// space (y up), blue on a clear page, as straight or premultiplied RGBA.
fn box_raster(straight: bool) -> (Vec<u8>, usize, usize) {
    let (w, h) = (200usize, 100usize);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            // Row 0 is the top: display y = 10 - (y + 0.5) / 10.
            let (px, py) = ((x as f64 + 0.5) / 10.0, 10.0 - (y as f64 + 0.5) / 10.0);
            if (6.0..14.0).contains(&px) && (3.0..7.0).contains(&py) {
                let at = (y * w + x) * 4;
                // Half-transparent blue: the colour is halved when it is premultiplied.
                let blue = if straight { 255 } else { 128 };
                rgba[at..at + 4].copy_from_slice(&[0, 0, blue, 128]);
            }
        }
    }
    (rgba, w, h)
}

fn bounds(loops: &[Vec<(f64, f64)>]) -> [f64; 4] {
    let all = loops.iter().flatten();
    [
        all.clone().map(|p| p.0).fold(f64::MAX, f64::min),
        all.clone().map(|p| p.1).fold(f64::MAX, f64::min),
        all.clone().map(|p| p.0).fold(f64::MIN, f64::max),
        all.map(|p| p.1).fold(f64::MIN, f64::max),
    ]
}

fn assert_box(found: [f64; 4], wanted: [f64; 4]) {
    for (a, b) in found.iter().zip(&wanted) {
        assert!((a - b).abs() < 0.12, "{found:?} is not {wanted:?}");
    }
}

#[test]
fn a_traced_page_lies_in_its_display_space_whichever_way_the_pixels_come() {
    let request = CutRequest { page: 0, scale: 1.0, clip: [0.0, 0.0, 20.0, 10.0] };
    for straight in [true, false] {
        let (rgba, width, height) = box_raster(straight);
        let raster = crate::Raster { width, height, rgba: &rgba, straight, size: (20.0, 10.0) };
        let outline = crate::trace_page(&raster, &request, &CutSpec::new()).expect("trace");
        assert_eq!(outline.request, request);
        assert_box(bounds(&outline.loops), [6.0, 3.0, 14.0, 7.0]);
    }
}

#[test]
fn the_offset_is_on_the_sheet_so_a_scaled_page_moves_it_by_the_inverse() {
    let (rgba, width, height) = box_raster(false);
    let raster = crate::Raster { width, height, rgba: &rgba, straight: false, size: (20.0, 10.0) };
    // Placed at half size, 1 pt of the sheet is 2 pt of the page.
    let request = CutRequest { page: 0, scale: 0.5, clip: [0.0, 0.0, 20.0, 10.0] };
    let spec = CutSpec { offset: 1.0, ..CutSpec::new() };
    let outline = crate::trace_page(&raster, &request, &spec).expect("trace");
    // Rounded corners at +2 pt on the page, so the box's sides are at 4, 1, 16, 9.
    assert_box(bounds(&outline.loops), [4.0, 1.0, 16.0, 9.0]);
}

#[test]
fn only_what_the_placement_shows_is_cut() {
    let (rgba, width, height) = box_raster(false);
    let raster = crate::Raster { width, height, rgba: &rgba, straight: false, size: (20.0, 10.0) };
    // The right half of the page, from x = 10: the box is cut off there.
    let request = CutRequest { page: 0, scale: 1.0, clip: [10.0, 0.0, 20.0, 10.0] };
    let outline = crate::trace_page(&raster, &request, &CutSpec::new()).expect("trace");
    assert_box(bounds(&outline.loops), [10.0, 3.0, 14.0, 7.0]);
}

#[test]
fn a_page_with_no_size_or_a_bad_scale_is_refused() {
    let (rgba, width, height) = box_raster(false);
    let request = CutRequest { page: 0, scale: 1.0, clip: [0.0, 0.0, 20.0, 10.0] };
    let with = |size: (f64, f64)| crate::Raster { width, height, rgba: &rgba, straight: false, size };
    assert!(crate::trace_page(&with((0.0, 10.0)), &request, &CutSpec::new()).is_err());
    assert!(crate::trace_page(&with((f64::NAN, 10.0)), &request, &CutSpec::new()).is_err());
    assert!(crate::trace_page(&with((20.0, 10.0)), &CutRequest { scale: 0.0, ..request }, &CutSpec::new()).is_err());
    let raster = crate::Raster { width: width + 1, ..with((20.0, 10.0)) };
    assert!(crate::trace_page(&raster, &request, &CutSpec::new()).is_err(), "the pixels must match the size");
}

#[test]
fn the_trace_resolution_is_a_tenth_of_a_millimetre_of_the_sheet_and_lower_on_a_huge_page() {
    let request = |scale| CutRequest { page: 0, scale, clip: [0.0; 4] };
    // 1:1, a point is 25.4 / 72 mm, so 10 pixels a millimetre is 10 × 25.4 / 72 pixels a point.
    assert!((crate::trace_resolution((100.0, 100.0), &request(1.0)) - 254.0 / 72.0).abs() < 1e-9);
    assert!((crate::trace_resolution((100.0, 100.0), &request(0.5)) - 254.0 / 72.0 * 0.5).abs() < 1e-9, "a shrunk page needs fewer pixels");
    let huge = crate::trace_resolution((20_000.0, 20_000.0), &request(1.0));
    assert!(huge * 20_000.0 * huge * 20_000.0 <= 9_000_001.0, "{huge}");
}

#[test]
fn a_cut_on_the_edge_is_as_smooth_as_one_offset_from_it() {
    // A disc of radius 40 pt on a 100 pt page at 10 px a point. On the edge the trace follows the
    // pixels, a staircase on every slope; the path must still be a smooth curve, not one tooth per
    // pixel step, or the cutter chatters along it.
    let (w, h) = (1000usize, 1000usize);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f64 + 0.5 - 500.0, y as f64 + 0.5 - 500.0);
            if dx * dx + dy * dy <= 400.0 * 400.0 {
                let at = (y * w + x) * 4;
                rgba[at..at + 4].copy_from_slice(&[0, 0, 255, 255]);
            }
        }
    }
    let raster = crate::Raster { width: w, height: h, rgba: &rgba, straight: true, size: (100.0, 100.0) };
    let request = CutRequest { page: 0, scale: 1.0, clip: [0.0, 0.0, 100.0, 100.0] };
    let points = |offset: f64| {
        let outline = crate::trace_page(&raster, &request, &CutSpec { offset, ..CutSpec::new() }).expect("trace");
        assert_eq!(outline.loops.len(), 1);
        outline.loops[0].len()
    };
    let (edge, grown) = (points(0.0), points(1.0));
    assert!(edge <= grown * 3 / 2, "{edge} points on the edge against {grown} offset by 1 pt");
}

fn cutter_spec() -> crate::CutterSpec {
    crate::CutterSpec { name: "job 42".into(), layout: pdfcraft_cutter::Layout::default() }
}

/// A 480 × 320 mm sheet with one 100 mm square cut line well inside the marks.
fn cut_sheet_inside() -> Sheet {
    let mm = 72.0 / 25.4;
    let mut s = sheet(vec![]);
    s.size = (480.0 * mm, 320.0 * mm);
    s.cuts = vec![vec![(50.0 * mm, 50.0 * mm), (150.0 * mm, 50.0 * mm), (150.0 * mm, 150.0 * mm), (50.0 * mm, 150.0 * mm)]];
    s
}

#[test]
fn sheets_with_cut_lines_get_marks_and_a_named_plt_each() {
    let mut sheets = vec![cut_sheet_inside(), sheet(vec![]), cut_sheet_inside()];
    sheets[1].size = sheets[0].size;
    let files = crate::apply_cutter(&mut sheets, &cutter_spec()).expect("cutter");
    assert_eq!(files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["job42-1.plt", "job42-2.plt"]);
    assert!(files[0].bytes.starts_with(b"CR;NAMEjob42-1.plt;"));
    assert!(files[1].bytes.starts_with(b"CR;NAMEjob42-2.plt;"));
    assert!(!sheets[0].sensor_black.is_empty() && !sheets[2].sensor_black.is_empty());
    assert!(sheets[1].sensor_black.is_empty() && sheets[1].sensor_clear.is_empty(), "a sheet without cut lines is left alone");
    assert_ne!(sheets[0].sensor_black, sheets[2].sensor_black, "each sheet's code names its own file");
    let one = crate::apply_cutter(&mut [cut_sheet_inside()], &cutter_spec()).expect("cutter");
    assert_eq!(one[0].name, "job42.plt");
}

#[test]
fn a_cutter_failure_changes_nothing() {
    let mut sheets = vec![cut_sheet_inside(), cut_sheet_inside()];
    // The second sheet's line runs off the marks.
    sheets[1].cuts[0][0].0 = 1.0;
    let before = sheets.clone();
    assert_eq!(crate::apply_cutter(&mut sheets, &cutter_spec()), Err(pdfcraft_cutter::CutterError::OutsideMarks));
    assert_eq!(sheets, before);
    assert_eq!(crate::apply_cutter(&mut [sheet(vec![])], &cutter_spec()), Err(pdfcraft_cutter::CutterError::NothingToCut));
    let bad = crate::CutterSpec { name: "///".into(), ..cutter_spec() };
    assert_eq!(crate::apply_cutter(&mut [cut_sheet_inside()], &bad), Err(pdfcraft_cutter::CutterError::BadName));
}
