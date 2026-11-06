use super::*;

const MM: f64 = 72.0 / 25.4;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// A3 sheet, 10 mm margins.
fn a3() -> StepRepeat {
    let mut s = StepRepeat::new((297.0 * MM, 420.0 * MM));
    s.margins = Margins::uniform(10.0 * MM);
    s
}

fn card() -> (f64, f64) {
    (93.0 * MM, 57.0 * MM) // 90 x 54 mm trim + 3 mm bleed
}

fn pages_of(sheet: &Sheet) -> Vec<usize> {
    sheet.placed.iter().map(|p| p.page).collect()
}

/// Where a placement's page origin sits on the sheet.
fn origin(p: &Placement) -> (f64, f64) {
    (p.matrix.0[4], p.matrix.0[5])
}

#[test]
fn repeat_fits_as_many_copies_as_the_sheet_holds() {
    let sheets = step_and_repeat(&[card()], &a3()).expect("layout");
    assert_eq!(sheets.len(), 1);
    // usable 277 x 400 mm; cell 93 x 57 mm -> 2 columns (186 <= 277, 279 > 277), 7 rows (399 <= 400)
    assert_eq!(sheets[0].placed.len(), 2 * 7);
    assert!(pages_of(&sheets[0]).iter().all(|p| *p == 0));
}

#[test]
fn every_cell_lies_inside_the_usable_area_and_none_overlap() {
    let mut s = a3();
    s.gutter = (4.0 * MM, 4.0 * MM);
    let sheets = step_and_repeat(&[card()], &s).expect("layout");
    let rects: Vec<Rect> = sheets[0].placed.iter().map(|p| [origin(p).0, origin(p).1, origin(p).0 + card().0, origin(p).1 + card().1]).collect();
    let m = s.margins.left;
    for r in &rects {
        assert!(r[0] >= m - EPS && r[1] >= m - EPS);
        assert!(r[2] <= s.sheet.0 - m + EPS && r[3] <= s.sheet.1 - m + EPS);
    }
    for (i, a) in rects.iter().enumerate() {
        for b in rects.iter().skip(i + 1) {
            assert!(!overlaps(*a, *b), "{a:?} overlaps {b:?}");
        }
    }
}

#[test]
fn gutter_is_the_exact_space_between_neighbours() {
    let mut s = a3();
    s.cols = Some(2);
    s.rows = Some(1);
    s.gutter = (10.0, 0.0);
    let sheets = step_and_repeat(&[(100.0, 100.0)], &s).expect("layout");
    let xs: Vec<f64> = sheets[0].placed.iter().map(|p| origin(p).0).collect();
    assert_eq!(xs.len(), 2);
    assert!(close(xs[1] - xs[0] - 100.0, 10.0), "{xs:?}");
}

#[test]
fn the_grid_is_centred_in_the_usable_area() {
    let mut s = StepRepeat::new((500.0, 400.0));
    s.cols = Some(1);
    s.rows = Some(1);
    let sheets = step_and_repeat(&[(100.0, 50.0)], &s).expect("layout");
    let p = &sheets[0].placed[0];
    assert!(close(origin(p).0, 200.0) && close(origin(p).1, 175.0), "{:?}", origin(p));
}

#[test]
fn a_fixed_grid_that_does_not_fit_is_an_error_not_a_clip() {
    let mut s = StepRepeat::new((200.0, 200.0));
    s.cols = Some(3);
    s.rows = Some(1);
    let err = step_and_repeat(&[(100.0, 100.0)], &s).expect_err("3 x 100 pt in 200 pt");
    assert!(matches!(err, ImposeError::GridTooBig { cols: 3, rows: 1, .. }), "{err:?}");
}

#[test]
fn a_page_larger_than_the_sheet_is_reported() {
    let s = StepRepeat::new((200.0, 200.0));
    let err = step_and_repeat(&[(300.0, 100.0)], &s).expect_err("too wide");
    assert!(matches!(err, ImposeError::DoesNotFit { .. }), "{err:?}");
}

#[test]
fn each_page_of_a_repeat_job_gets_its_own_sheet() {
    let sheets = step_and_repeat(&[card(), card(), card()], &a3()).expect("layout");
    assert_eq!(sheets.len(), 3);
    for (i, sheet) in sheets.iter().enumerate() {
        assert!(pages_of(sheet).iter().all(|p| *p == i));
    }
}

#[test]
fn duplex_back_cell_sits_exactly_behind_its_front_cell() {
    for (duplex, check) in [(Duplex::LongEdge, true), (Duplex::ShortEdge, false)] {
        let mut s = a3();
        s.duplex = Some(duplex);
        s.margins = Margins { left: 20.0, bottom: 30.0, right: 50.0, top: 10.0 }; // asymmetric on purpose
        let sheets = step_and_repeat(&[card(), card()], &s).expect("layout");
        assert_eq!(sheets.len(), 2);
        let (front, back) = (&sheets[0], &sheets[1]);
        assert_eq!(front.placed.len(), back.placed.len());
        for (f, b) in front.placed.iter().zip(&back.placed) {
            assert_eq!((f.page, b.page), (0, 1));
            let (fx, fy) = origin(f);
            let (bx, by) = origin(b);
            if check {
                // Long edge: x mirrors about the sheet centre, y stays.
                assert!(close(bx, s.sheet.0 - fx - card().0) && close(by, fy), "{duplex:?}");
            } else {
                assert!(close(bx, fx) && close(by, s.sheet.1 - fy - card().1), "{duplex:?}");
            }
        }
    }
}

#[test]
fn an_unpaired_last_page_makes_a_front_only() {
    let mut s = a3();
    s.duplex = Some(Duplex::LongEdge);
    let sheets = step_and_repeat(&[card(), card(), card()], &s).expect("layout");
    assert_eq!(sheets.len(), 3);
    assert!(pages_of(&sheets[2]).iter().all(|p| *p == 2));
}

#[test]
fn sequential_fills_cells_in_reading_order_and_leaves_the_rest_empty() {
    let mut s = StepRepeat::new((300.0, 200.0));
    s.margins = Margins::default();
    s.fill = Fill::Sequential;
    s.cols = Some(3);
    s.rows = Some(2);
    let sizes = vec![(100.0, 100.0); 7];
    let sheets = step_and_repeat(&sizes, &s).expect("layout");
    assert_eq!(sheets.len(), 2);
    assert_eq!(pages_of(&sheets[0]), vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(pages_of(&sheets[1]), vec![6]);
    // Reading order: left to right, then the next row down.
    let o: Vec<(f64, f64)> = sheets[0].placed.iter().map(origin).collect();
    assert!(o[0].0 < o[1].0 && o[1].0 < o[2].0);
    assert!(o[0].1 > o[3].1 && close(o[0].0, o[3].0));
}

#[test]
fn sequential_duplex_puts_the_next_chunk_on_the_back_in_the_mirrored_cell() {
    let mut s = StepRepeat::new((300.0, 200.0));
    s.margins = Margins::default();
    s.fill = Fill::Sequential;
    s.cols = Some(3);
    s.rows = Some(1);
    s.duplex = Some(Duplex::LongEdge);
    let sheets = step_and_repeat(&[(100.0, 100.0); 6], &s).expect("layout");
    assert_eq!(sheets.len(), 2);
    assert_eq!(pages_of(&sheets[0]), vec![0, 1, 2]);
    assert_eq!(pages_of(&sheets[1]), vec![3, 4, 5]);
    // Page 0 is at the left of the front, so page 3 (its back) is at the right of the back.
    assert!(close(origin(&sheets[0].placed[0]).0, 0.0));
    assert!(close(origin(&sheets[1].placed[0]).0, 200.0));
}

#[test]
fn a_smaller_page_is_centred_in_a_larger_cell_and_a_larger_page_is_clipped() {
    let mut s = StepRepeat::new((400.0, 400.0));
    s.cols = Some(1);
    s.rows = Some(1);
    s.cell = Some((100.0, 100.0));
    let small = step_and_repeat(&[(60.0, 40.0)], &s).expect("layout");
    assert_eq!(small[0].placed[0].clip, [0.0, 0.0, 60.0, 40.0]);
    let large = step_and_repeat(&[(140.0, 100.0)], &s).expect("layout");
    assert_eq!(large[0].placed[0].clip, [20.0, 0.0, 120.0, 100.0]);
}

#[test]
fn marks_start_at_the_offset_from_the_trim_and_may_cross_the_bleed() {
    let mut s = StepRepeat::new((400.0, 400.0));
    s.cols = Some(1);
    s.rows = Some(1);
    s.bleed = 9.0;
    s.marks = Some(Marks { length: 20.0, offset: 5.0, ..Marks::default() });
    let sheets = step_and_repeat(&[(100.0, 100.0)], &s).expect("layout");
    // Page at 150..250, trim at 159..241: marks start 5 pt off the trim, over the bleed (allowed,
    // as smart marks do), and never over the finished page.
    let lines = &sheets[0].lines;
    assert_eq!(lines.len(), 8);
    for l in lines {
        assert!(!overlaps(*l, [159.0, 159.0, 241.0, 241.0]), "{l:?}");
    }
    assert!(lines.iter().any(|l| close(l[0], 134.0) && close(l[2], 154.0) && close(l[1], 159.0)));
}

#[test]
fn a_negative_offset_runs_the_mark_into_the_page() {
    let mut s = StepRepeat::new((400.0, 400.0));
    s.cols = Some(1);
    s.rows = Some(1);
    // -3 deep, 5 long: 3 inside the trim, 2 outside.
    s.marks = Some(Marks { length: 5.0, offset: -3.0, ..Marks::default() });
    let sheets = step_and_repeat(&[(100.0, 100.0)], &s).expect("layout");
    let lines = &sheets[0].lines;
    assert_eq!(lines.len(), 8);
    // Left edge x = 150: the bottom-left horizontal mark spans 148..153 on y = 150.
    assert!(lines.iter().any(|l| close(l[0], 148.0) && close(l[2], 153.0) && close(l[1], 150.0)), "{lines:?}");
    // A mark wholly inside the page is refused.
    s.marks = Some(Marks { length: 3.0, offset: -3.0, ..Marks::default() });
    assert!(matches!(step_and_repeat(&[(100.0, 100.0)], &s), Err(ImposeError::Invalid("mark offset", _))));
}

#[test]
fn a_mark_into_one_page_is_still_kept_off_its_neighbour() {
    let mut s = StepRepeat::new((300.0, 200.0));
    s.margins = Margins::uniform(20.0);
    s.cols = Some(2);
    s.rows = Some(1);
    s.marks = Some(Marks { length: 5.0, offset: -3.0, ..Marks::default() });
    // Butted pages: the inner horizontal marks would run into the neighbour, so they go.
    let sheets = step_and_repeat(&[(100.0, 100.0)], &s).expect("layout");
    let pages: Vec<Rect> = sheets[0].placed.iter().map(|p| [origin(p).0, origin(p).1, origin(p).0 + 100.0, origin(p).1 + 100.0]).collect();
    for l in &sheets[0].lines {
        let crossing = pages.iter().filter(|p| overlaps([l[0] - 0.01, l[1] - 0.01, l[2] + 0.01, l[3] + 0.01], **p)).count();
        assert!(crossing <= 1, "{l:?} runs over two pages");
    }
}

#[test]
fn japanese_marks_add_a_line_along_the_bleed() {
    let mut s = StepRepeat::new((400.0, 400.0));
    s.cols = Some(1);
    s.rows = Some(1);
    s.bleed = 9.0;
    s.marks = Some(Marks { style: MarkStyle::Japanese, ..Marks::default() });
    let sheets = step_and_repeat(&[(100.0, 100.0)], &s).expect("layout");
    assert_eq!(sheets[0].lines.len(), 16);
    // Bleed edge at 150: a horizontal mark lies on it as well as on the trim line 159.
    assert!(sheets[0].lines.iter().any(|l| close(l[1], 150.0) && close(l[3], 150.0)));
}

#[test]
fn mark_paint_reaches_the_sheet() {
    let mut s = StepRepeat::new((400.0, 400.0));
    s.marks = Some(Marks { width: 0.25, registration: true, ..Marks::default() });
    let sheets = step_and_repeat(&[(100.0, 100.0)], &s).expect("layout");
    assert_eq!(sheets[0].marks_paint, pdfcraft_print::MarkPaint { width: 0.25, registration: true });
}

#[test]
fn unique_collate_rounds_automatic_columns_to_whole_blocks() {
    // 150 mm pages on a 480 x 320 sheet fit 3 columns; with copies 2 that becomes 2.
    let mm = 72.0 / 25.4;
    let mut s = StepRepeat::new((480.0 * mm, 320.0 * mm));
    s.margins = Margins::uniform(5.0 * mm);
    s.fill = Fill::CutStack { copies: 2 };
    let sheets = step_and_repeat(&[(150.0 * mm, 150.0 * mm); 18], &s).expect("layout");
    assert_eq!(sheets[0].placed.len(), 4);
}

#[test]
fn marks_shared_by_neighbours_are_drawn_once() {
    let mut s = StepRepeat::new((600.0, 400.0));
    s.cols = Some(2);
    s.rows = Some(1);
    s.gutter = (60.0, 0.0);
    s.bleed = 0.0;
    s.marks = Some(Marks { length: 10.0, offset: 5.0, ..Marks::default() });
    let sheets = step_and_repeat(&[(100.0, 100.0)], &s).expect("layout");
    let lines = &sheets[0].lines;
    for (i, a) in lines.iter().enumerate() {
        assert!(!lines.iter().skip(i + 1).any(|b| a == b), "duplicate {a:?}");
    }
    // 2 cells x 8 marks; the two marks facing each other across the gutter do not coincide
    // here, so none are merged and none run over a page.
    assert_eq!(lines.len(), 16);
}

#[test]
fn marks_that_would_leave_the_sheet_are_dropped() {
    let mut s = StepRepeat::new((120.0, 120.0));
    s.cols = Some(1);
    s.rows = Some(1);
    s.marks = Some(Marks { length: 50.0, offset: 5.0, ..Marks::default() });
    let sheets = step_and_repeat(&[(100.0, 100.0)], &s).expect("layout");
    assert!(sheets[0].lines.is_empty());
}

#[test]
fn bad_numbers_are_errors_never_panics() {
    let ok = StepRepeat::new((300.0, 300.0));
    let page = [(100.0, 100.0)];
    assert_eq!(step_and_repeat(&[], &ok), Err(ImposeError::NoPages));
    for f in [f64::NAN, f64::INFINITY, -1.0, 0.0] {
        let mut s = ok.clone();
        s.sheet = (f, 300.0);
        assert!(step_and_repeat(&page, &s).is_err(), "sheet width {f}");
        assert!(step_and_repeat(&[(f, 100.0)], &ok).is_err(), "page width {f}");
    }
    for f in [f64::NAN, f64::INFINITY, -1.0] {
        let mut s = ok.clone();
        s.gutter = (f, 0.0);
        assert!(step_and_repeat(&page, &s).is_err(), "gutter {f}");
        let mut s = ok.clone();
        s.margins.left = f;
        assert!(step_and_repeat(&page, &s).is_err(), "margin {f}");
        let mut s = ok.clone();
        s.bleed = f;
        assert!(step_and_repeat(&page, &s).is_err(), "bleed {f}");
    }
    let mut s = ok.clone();
    s.cols = Some(0);
    assert!(step_and_repeat(&page, &s).is_err());
    let mut s = ok;
    s.bleed = 50.0; // half of 100 pt: nothing would be left inside the trim
    assert!(step_and_repeat(&page, &s).is_err());
}

#[test]
fn a_runaway_grid_is_refused() {
    let mut s = StepRepeat::new((1.0e6, 1.0e6));
    s.margins = Margins::default();
    let err = step_and_repeat(&[(1.0, 1.0)], &s).expect_err("10^12 cells");
    assert_eq!(err, ImposeError::TooLarge("more cells on one sheet than are supported"));
}

/// The page numbers (1-based) of a sheet, row by row, as the attachment from the shop floor shows them.
fn grid_of(sheet: &Sheet, cols: usize) -> Vec<Vec<usize>> {
    let mut placed: Vec<&Placement> = sheet.placed.iter().collect();
    placed.sort_by(|a, b| origin(b).1.total_cmp(&origin(a).1).then(origin(a).0.total_cmp(&origin(b).0)));
    placed.chunks(cols).map(|row| row.iter().map(|p| p.page + 1).collect()).collect()
}

#[test]
fn unique_collate_matches_the_shop_floor_example() {
    // 16 pages, 4 x 2 cells, the block of 2 different stacks repeated twice across each row.
    let mut s = StepRepeat::new((400.0, 200.0));
    s.margins = Margins::default();
    s.fill = Fill::CutStack { copies: 2 };
    s.cols = Some(4);
    s.rows = Some(2);
    let sheets = step_and_repeat(&[(100.0, 100.0); 16], &s).expect("layout");
    assert_eq!(sheets.len(), 4);
    assert_eq!(grid_of(&sheets[0], 4), vec![vec![1, 5, 1, 5], vec![9, 13, 9, 13]]);
    assert_eq!(grid_of(&sheets[1], 4), vec![vec![2, 6, 2, 6], vec![10, 14, 10, 14]]);
    assert_eq!(grid_of(&sheets[2], 4), vec![vec![3, 7, 3, 7], vec![11, 15, 11, 15]]);
    assert_eq!(grid_of(&sheets[3], 4), vec![vec![4, 8, 4, 8], vec![12, 16, 12, 16]]);
}

#[test]
fn every_cut_stack_reads_in_page_order_through_the_sheets() {
    for (n, copies) in [(16, 1), (13, 1), (20, 2), (7, 4)] {
        let mut s = StepRepeat::new((400.0, 200.0));
        s.margins = Margins::default();
        s.fill = Fill::CutStack { copies };
        s.cols = Some(4);
        s.rows = Some(2);
        let sheets = step_and_repeat(&vec![(100.0, 100.0); n], &s).expect("layout");
        // Each cell position, read down the stack of sheets, is an unbroken run of pages, and the
        // stacks of the first block, read one after another, are pages 1..=n exactly once.
        let mut all = Vec::new();
        for k in (0..8).filter(|k| k % 4 < 4 / copies) {
            let stack: Vec<usize> = sheets.iter().filter_map(|sh| cells_in_order(sh).get(k).copied().flatten()).collect();
            for w in stack.windows(2) {
                assert_eq!(w[1], w[0] + 1, "{n} pages, {copies} copies, cell {k}: {stack:?}");
            }
            all.extend(stack);
        }
        all.sort_unstable();
        assert_eq!(all, (0..n).collect::<Vec<_>>(), "{n} pages, {copies} copies");
    }
}

/// The page in each of the 8 cells of a 4 x 2 grid (0-based, reading order), `None` if empty.
fn cells_in_order(sheet: &Sheet) -> Vec<Option<usize>> {
    let xs = [0.0, 100.0, 200.0, 300.0];
    let ys = [100.0, 0.0];
    let mut out = Vec::new();
    for y in ys {
        for x in xs {
            out.push(sheet.placed.iter().find(|p| close(origin(p).0, x) && close(origin(p).1, y)).map(|p| p.page));
        }
    }
    out
}

#[test]
fn cut_stack_copies_must_divide_the_columns() {
    for copies in [0, 3] {
        let mut s = StepRepeat::new((400.0, 200.0));
        s.margins = Margins::default();
        s.fill = Fill::CutStack { copies };
        s.cols = Some(4);
        s.rows = Some(2);
        assert!(matches!(step_and_repeat(&[(100.0, 100.0); 8], &s), Err(ImposeError::Invalid("copies", _))), "{copies}");
    }
}

#[test]
fn duplex_cut_stack_keeps_each_back_behind_its_front() {
    let mut s = StepRepeat::new((400.0, 200.0));
    s.margins = Margins::default();
    s.fill = Fill::CutStack { copies: 1 };
    s.cols = Some(4);
    s.rows = Some(2);
    s.duplex = Some(Duplex::LongEdge);
    let sheets = step_and_repeat(&[(100.0, 100.0); 32], &s).expect("layout");
    assert_eq!(sheets.len(), 4);
    for pair in sheets.chunks(2) {
        for f in &pair[0].placed {
            let (fx, fy) = origin(f);
            let b =
                pair[1].placed.iter().find(|b| close(origin(b).0, 400.0 - fx - 100.0) && close(origin(b).1, fy)).expect("a back behind every front");
            assert_eq!(b.page, f.page + 1);
        }
    }
}

#[test]
fn rotation_fits_more_copies_like_the_press_room_example() {
    // 5 x 2 in pages on a 9 x 14 in sheet, no margins: 8 without turning (wide), more with.
    let inch = 72.0;
    let mut s = StepRepeat::new((9.0 * inch, 14.0 * inch));
    s.margins = Margins::default();
    let plain = step_and_repeat(&[(5.0 * inch, 2.0 * inch)], &s).expect("layout");
    assert_eq!(plain[0].placed.len(), 7, "tall pages, upright: 1 column x 7 rows");
    s.rotate = true;
    let turned = step_and_repeat(&[(5.0 * inch, 2.0 * inch)], &s).expect("layout");
    assert!(turned[0].placed.len() >= 10, "{}", turned[0].placed.len());
    // Nothing overlaps and everything stays on the sheet.
    let rects: Vec<Rect> = turned[0]
        .placed
        .iter()
        .map(|p| {
            let [a, b, c, d, e, f] = p.matrix.0;
            let pts =
                [(0.0, 0.0), (5.0 * inch, 0.0), (0.0, 2.0 * inch), (5.0 * inch, 2.0 * inch)].map(|(u, v)| (a * u + c * v + e, b * u + d * v + f));
            let xs = pts.map(|p| p.0);
            let ys = pts.map(|p| p.1);
            [
                xs.iter().cloned().fold(f64::MAX, f64::min),
                ys.iter().cloned().fold(f64::MAX, f64::min),
                xs.iter().cloned().fold(f64::MIN, f64::max),
                ys.iter().cloned().fold(f64::MIN, f64::max),
            ]
        })
        .collect();
    for (i, r) in rects.iter().enumerate() {
        assert!(r[0] >= -EPS && r[1] >= -EPS && r[2] <= 9.0 * inch + EPS && r[3] <= 14.0 * inch + EPS, "{r:?}");
        for q in rects.iter().skip(i + 1) {
            assert!(!overlaps(*r, *q), "{r:?} {q:?}");
        }
    }
}

#[test]
fn a_turned_front_has_its_back_turned_to_match() {
    let inch = 72.0;
    let mut s = StepRepeat::new((9.0 * inch, 14.0 * inch));
    s.margins = Margins::default();
    s.rotate = true;
    s.duplex = Some(Duplex::LongEdge);
    let sheets = step_and_repeat(&[(5.0 * inch, 2.0 * inch); 2], &s).expect("layout");
    let (front, back) = (&sheets[0], &sheets[1]);
    assert_eq!(front.placed.len(), back.placed.len());
    for (f, b) in front.placed.iter().zip(&back.placed) {
        // Over the long edge a page turned anticlockwise backs onto one turned clockwise.
        if f.matrix.0[1] > 0.5 {
            assert!(b.matrix.0[1] < -0.5, "{:?} / {:?}", f.matrix, b.matrix);
        }
    }
}

#[test]
fn rotation_is_off_by_default_and_ignored_for_a_fixed_grid() {
    let inch = 72.0;
    let mut s = StepRepeat::new((9.0 * inch, 14.0 * inch));
    s.margins = Margins::default();
    s.rotate = true;
    s.cols = Some(1);
    s.rows = Some(2);
    let sheets = step_and_repeat(&[(5.0 * inch, 2.0 * inch)], &s).expect("layout");
    assert_eq!(sheets[0].placed.len(), 2);
    assert!(sheets[0].placed.iter().all(|p| p.matrix.0[1] == 0.0));
}

#[test]
fn centred_marks_cross_the_outer_corners_and_mark_the_cuts_between_pages() {
    // 2 x 2 pages of 100 x 60 with no gutter on a 300 x 200 sheet, 7-unit centred marks: the
    // layout of the PAULUS sample.
    let mut s = StepRepeat::new((300.0, 200.0));
    s.cols = Some(2);
    s.rows = Some(2);
    s.marks = Some(Marks { style: MarkStyle::Centred, length: 7.0, ..Marks::default() });
    let sheets = step_and_repeat(&[(100.0, 60.0)], &s).expect("layout");
    let lines = &sheets[0].lines;
    // Pages cover x 50..250, y 40..160; the cuts are x = 50, 150, 250 and y = 40, 100, 160.
    let has = |l: [f64; 4]| lines.iter().any(|m| m.iter().zip(l.iter()).all(|(a, b)| (a - b).abs() < 1e-6));
    // Outer corner: a full cross through (50, 40).
    assert!(has([46.5, 40.0, 53.5, 40.0]) && has([50.0, 36.5, 50.0, 43.5]), "{lines:?}");
    // Top middle: only the vertical line along the cut x = 150; the horizontal one would run
    // onto the neighbour's edge.
    assert!(has([150.0, 156.5, 150.0, 163.5]));
    assert!(!has([146.5, 160.0, 153.5, 160.0]) && !has([150.0, 160.0, 153.5, 160.0]));
    // Left middle: only the horizontal line along the cut y = 100.
    assert!(has([46.5, 100.0, 53.5, 100.0]));
    assert!(!has([50.0, 96.5, 50.0, 103.5]));
    // Nothing in the middle of the layout.
    assert!(lines.iter().all(|l| !(l[0] > 60.0 && l[2] < 240.0 && l[1] > 50.0 && l[3] < 150.0)), "{lines:?}");
    // 4 outer corners x 2 + 2 cut lines on each of 4 sides / 2 = 8 + 4.
    assert_eq!(lines.len(), 12, "{lines:?}");
}
