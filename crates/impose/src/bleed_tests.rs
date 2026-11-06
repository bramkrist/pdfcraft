use super::*;

/// Where a placement puts its visible part on the sheet: the clip rectangle through the matrix
/// (axis-aligned: these layouts only scale, flip and shift).
fn covered(p: &pdfcraft_print::Placement) -> Rect {
    let [a, _, _, d, e, f] = p.matrix.0;
    let (x0, x1) = (a * p.clip[0] + e, a * p.clip[2] + e);
    let (y0, y1) = (d * p.clip[1] + f, d * p.clip[3] + f);
    [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]
}

fn area(r: Rect) -> f64 {
    (r[2] - r[0]) * (r[3] - r[1])
}

/// The visible parts tile the whole sheet: all inside it, none overlapping, areas adding up.
fn assert_tiles(sheet: &Sheet) {
    let whole = [0.0, 0.0, sheet.size.0, sheet.size.1];
    let rects: Vec<Rect> = sheet.placed.iter().map(covered).collect();
    for (i, r) in rects.iter().enumerate() {
        assert!(r[0] >= -EPS && r[1] >= -EPS && r[2] <= whole[2] + EPS && r[3] <= whole[3] + EPS, "{r:?} leaves the sheet");
        for q in rects.iter().skip(i + 1) {
            assert!(!overlaps(*r, *q), "{r:?} overlaps {q:?}");
        }
    }
    let total: f64 = rects.iter().map(|r| area(*r)).sum();
    assert!((total - area(whole)).abs() < 1e-6, "the parts cover {total} of {}", area(whole));
}

#[test]
fn a_mirrored_bleed_grows_the_page_and_marks_the_old_edge_as_the_trim() {
    let sheets = add_bleed(&[(100.0, 60.0)], &AddBleed::uniform(10.0, BleedMethod::Mirror)).expect("layout");
    assert_eq!(sheets.len(), 1);
    assert_eq!(sheets[0].size, (120.0, 80.0));
    assert_eq!(sheets[0].trim, Some([10.0, 10.0, 110.0, 70.0]));
    // The page, four strips and four corners.
    assert_eq!(sheets[0].placed.len(), 9);
    assert_tiles(&sheets[0]);
}

#[test]
fn the_parts_tile_the_sheet_for_any_mix_of_sides() {
    for amount in [[10.0, 0.0, 0.0, 0.0], [0.0, 5.0, 0.0, 9.0], [3.0, 4.0, 5.0, 6.0], [20.0, 20.0, 20.0, 20.0], [0.0, 0.0, 7.0, 7.0]] {
        let spec = AddBleed { amount, method: BleedMethod::Mirror };
        let sheets = add_bleed(&[(100.0, 60.0), (80.0, 80.0)], &spec).expect("layout");
        for s in &sheets {
            assert_tiles(s);
            let (w, h) = if s.size.0 - amount[0] - amount[2] == 100.0 { (100.0, 60.0) } else { (80.0, 80.0) };
            assert_eq!(s.size, (w + amount[0] + amount[2], h + amount[1] + amount[3]));
        }
    }
}

#[test]
fn a_bleed_as_wide_as_the_page_cannot_be_mirrored() {
    for amount in [[0.0, 60.0, 0.0, 0.0], [0.0, 0.0, 0.0, 70.0], [100.0, 1.0, 1.0, 1.0]] {
        let spec = AddBleed { amount, method: BleedMethod::Mirror };
        assert!(matches!(add_bleed(&[(100.0, 60.0)], &spec), Err(ImposeError::Invalid("bleed", _))), "{amount:?}");
    }
}

#[test]
fn a_scaled_bleed_enlarges_the_page_about_its_centre_to_cover_the_new_edge() {
    let sheets = add_bleed(&[(100.0, 60.0)], &AddBleed::uniform(10.0, BleedMethod::Scale)).expect("layout");
    let s = &sheets[0];
    assert_eq!((s.size, s.trim, s.placed.len()), ((120.0, 80.0), Some([10.0, 10.0, 110.0, 70.0]), 1));
    let p = &s.placed[0];
    // The larger of 120/100 and 80/60.
    assert!((p.matrix.0[0] - 80.0 / 60.0).abs() < 1e-9 && p.matrix.0[0] == p.matrix.0[3]);
    assert_tiles(s);
}

#[test]
fn bad_bleeds_are_errors_never_panics() {
    let page = [(100.0, 60.0)];
    assert_eq!(add_bleed(&[], &AddBleed::uniform(3.0, BleedMethod::Mirror)), Err(ImposeError::NoPages));
    for f in [f64::NAN, f64::INFINITY, -1.0] {
        for method in [BleedMethod::Mirror, BleedMethod::Scale] {
            assert!(add_bleed(&page, &AddBleed::uniform(f, method)).is_err(), "{f} {method:?}");
        }
    }
    for method in [BleedMethod::Mirror, BleedMethod::Scale] {
        assert!(add_bleed(&page, &AddBleed::uniform(0.0, method)).is_err(), "no bleed at all is not a job");
        assert!(add_bleed(&[(0.0, 60.0)], &AddBleed::uniform(3.0, method)).is_err());
    }
}
