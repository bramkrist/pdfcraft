use super::*;

fn pages(n: usize) -> Vec<(f64, f64)> {
    vec![(100.0, 200.0); n]
}

/// The pages on one sheet side as (left, right), by the x position of their placement.
fn side(s: &Sheet) -> (Option<usize>, Option<usize>) {
    let mid = s.size.0 / 2.0;
    let on = |right: bool| s.placed.iter().find(|p| (p.matrix.0[4] >= mid - 1e-9) == right).map(|p| p.page);
    (on(false), on(true))
}

fn order(sheets: &[Sheet]) -> Vec<(Option<usize>, Option<usize>)> {
    sheets.iter().map(side).collect()
}

#[test]
fn saddle_stitch_of_eight_pages_pairs_first_with_last() {
    let sheets = booklet(&pages(8), &Booklet::new()).expect("layout");
    // Pages are 0-based: sheet 1 front is 8 | 1, its back 2 | 7, sheet 2 front 6 | 3, back 4 | 5.
    assert_eq!(order(&sheets), vec![(Some(7), Some(0)), (Some(1), Some(6)), (Some(5), Some(2)), (Some(3), Some(4))]);
}

#[test]
fn every_page_lands_once_and_every_sheet_side_adds_up_for_any_saddle_size() {
    for n in (4..=64).step_by(4) {
        let sheets = booklet(&pages(n), &Booklet::new()).expect("layout");
        assert_eq!(sheets.len(), n / 2, "{n} pages");
        let mut seen = vec![0; n];
        for (i, s) in sheets.iter().enumerate() {
            let (l, r) = side(s);
            let (l, r) = (l.expect("left"), r.expect("right"));
            seen[l] += 1;
            seen[r] += 1;
            // Both sides of one leaf share the same pair sum, and the back follows the front.
            assert_eq!(l + r, n - 1, "{n} pages, side {i}");
        }
        assert!(seen.iter().all(|c| *c == 1), "{n} pages: {seen:?}");
        for k in 0..n / 4 {
            let (_, front_right) = side(&sheets[2 * k]);
            let (back_left, _) = side(&sheets[2 * k + 1]);
            assert_eq!(back_left, front_right.map(|p| p + 1), "{n} pages, sheet {k}: the back's left page follows the front's right");
        }
    }
}

#[test]
fn a_page_count_that_is_not_a_multiple_of_four_is_padded_with_blanks_at_the_end() {
    let sheets = booklet(&pages(6), &Booklet::new()).expect("layout");
    assert_eq!(sheets.len(), 4);
    assert_eq!(order(&sheets), vec![(None, Some(0)), (Some(1), None), (Some(5), Some(2)), (Some(3), Some(4))]);
    // A blank is simply nothing placed.
    assert_eq!(sheets[0].placed.len(), 1);
    assert_eq!(sheets[1].placed.len(), 1);
}

#[test]
fn a_single_page_is_a_one_leaf_booklet() {
    let sheets = booklet(&pages(1), &Booklet::new()).expect("layout");
    assert_eq!(order(&sheets), vec![(None, Some(0)), (None, None)]);
}

#[test]
fn perfect_binding_makes_saddle_signatures_read_one_after_another() {
    let mut spec = Booklet::new();
    spec.binding = Binding::Perfect { signature: 16, last: LastSignature::MultipleOfFour };
    let sheets = booklet(&pages(40), &spec).expect("layout");
    // 16 + 16 + 8 pages = 8 + 8 + 4 sheet sides... two sides per leaf: 16 + 16 + 8.
    assert_eq!(sheets.len(), 8 + 8 + 4);
    // Signature 2 starts with its own last page and first page: 32 | 17 in 1-based numbers.
    assert_eq!(side(&sheets[8]), (Some(31), Some(16)));
    // The last signature holds pages 33..40 and no blanks.
    assert_eq!(side(&sheets[16]), (Some(39), Some(32)));
    spec.binding = Binding::Perfect { signature: 16, last: LastSignature::Full };
    let padded = booklet(&pages(40), &spec).expect("layout");
    assert_eq!(padded.len(), 8 + 8 + 8);
    assert_eq!(side(&padded[16]), (None, Some(32)));
}

#[test]
fn perfect_binding_signatures_must_be_multiples_of_four() {
    for bad in [0, 2, 6, 15] {
        let mut spec = Booklet::new();
        spec.binding = Binding::Perfect { signature: bad, last: LastSignature::Full };
        assert!(matches!(booklet(&pages(8), &spec), Err(ImposeError::Invalid("signature size", _))), "{bad}");
    }
}

#[test]
fn continuous_binding_runs_the_pages_in_order() {
    let mut spec = Booklet::new();
    spec.binding = Binding::Continuous;
    let sheets = booklet(&pages(5), &spec).expect("layout");
    assert_eq!(order(&sheets), vec![(Some(0), Some(1)), (Some(2), Some(3)), (Some(4), None)]);
}

#[test]
fn the_automatic_sheet_holds_two_of_the_largest_page_at_full_size() {
    let sheets = booklet(&[(100.0, 200.0), (90.0, 180.0), (100.0, 200.0), (100.0, 200.0)], &Booklet::new()).expect("layout");
    assert_eq!(sheets[0].size, (200.0, 200.0));
    for p in sheets.iter().flat_map(|s| &s.placed) {
        assert_eq!(p.matrix.0[0], 1.0);
    }
    // Page 1 is the left page of the first back: centred in its half, (100 - 90) / 2 = 5 in.
    let small = sheets.iter().flat_map(|s| &s.placed).find(|p| p.page == 1).expect("page 1 is placed");
    assert_eq!((small.matrix.0[4], small.matrix.0[5]), (5.0, 10.0));
}

#[test]
fn pages_are_scaled_by_one_factor_to_fill_the_chosen_sheet() {
    let mut spec = Booklet::new();
    spec.sheet = Some((400.0, 200.0));
    let sheets = booklet(&pages(4), &spec).expect("layout");
    // Half sheet 200 x 200; page 100 x 200: the height is the limit, so the scale is 1.
    assert_eq!(sheets[0].placed[0].matrix.0[0], 1.0);
    spec.sheet = Some((800.0, 800.0));
    let sheets = booklet(&pages(4), &spec).expect("layout");
    // Half 400 x 800: width limits, scale 4; the page is 400 x 800 and fills the half exactly.
    for p in &sheets[0].placed {
        assert_eq!((p.matrix.0[0], p.matrix.0[3]), (4.0, 4.0));
    }
    assert_eq!(sheets[0].placed.iter().map(|p| p.matrix.0[4]).fold(f64::INFINITY, f64::min), 0.0);
}

#[test]
fn without_scaling_pages_keep_their_size_or_the_layout_is_refused() {
    let mut spec = Booklet::new();
    spec.scale = false;
    spec.sheet = Some((400.0, 400.0));
    let sheets = booklet(&pages(4), &spec).expect("layout");
    assert!(sheets[0].placed.iter().all(|p| p.matrix.0[0] == 1.0));
    spec.sheet = Some((150.0, 200.0));
    assert!(matches!(booklet(&pages(4), &spec), Err(ImposeError::DoesNotFit { .. })));
}

#[test]
fn spine_alignment_pulls_each_page_against_the_middle_of_the_sheet() {
    let mut spec = Booklet::new();
    spec.scale = false;
    spec.sheet = Some((400.0, 200.0));
    spec.align = Align::Spine;
    let sheets = booklet(&pages(4), &spec).expect("layout");
    let mut xs: Vec<f64> = sheets[0].placed.iter().map(|p| p.matrix.0[4]).collect();
    xs.sort_by(f64::total_cmp);
    // 100 wide pages meet at x = 200.
    assert_eq!(xs, vec![100.0, 200.0]);
    spec.align = Align::Centre;
    let sheets = booklet(&pages(4), &spec).expect("layout");
    let mut xs: Vec<f64> = sheets[0].placed.iter().map(|p| p.matrix.0[4]).collect();
    xs.sort_by(f64::total_cmp);
    assert_eq!(xs, vec![50.0, 250.0]);
}

#[test]
fn space_at_the_edge_of_the_sheet_is_left_empty() {
    let mut spec = Booklet::new();
    spec.edge = 10.0;
    let sheets = booklet(&pages(4), &spec).expect("layout");
    assert_eq!(sheets[0].size, (220.0, 220.0));
    let mut xs: Vec<f64> = sheets[0].placed.iter().map(|p| p.matrix.0[4]).collect();
    xs.sort_by(f64::total_cmp);
    assert_eq!(xs, vec![10.0, 110.0]);
    assert!(sheets[0].placed.iter().all(|p| p.matrix.0[5] == 10.0));
}

#[test]
fn binding_on_the_right_swaps_the_halves() {
    let mut spec = Booklet::new();
    spec.right_to_left = true;
    let sheets = booklet(&pages(8), &spec).expect("layout");
    assert_eq!(side(&sheets[0]), (Some(0), Some(7)));
}

#[test]
fn bad_numbers_are_errors_never_panics() {
    assert_eq!(booklet(&[], &Booklet::new()), Err(ImposeError::NoPages));
    for f in [f64::NAN, f64::INFINITY, -1.0, 0.0] {
        assert!(booklet(&[(f, 100.0)], &Booklet::new()).is_err(), "page {f}");
        let mut s = Booklet::new();
        s.sheet = Some((f, 100.0));
        assert!(booklet(&pages(4), &s).is_err(), "sheet {f}");
    }
    for f in [f64::NAN, f64::INFINITY, -1.0] {
        let mut s = Booklet::new();
        s.edge = f;
        assert!(booklet(&pages(4), &s).is_err(), "edge {f}");
    }
    let mut s = Booklet::new();
    s.sheet = Some((100.0, 100.0));
    s.edge = 50.0; // nothing left inside the edge
    assert!(booklet(&pages(4), &s).is_err());
}
