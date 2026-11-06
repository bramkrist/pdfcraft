use super::*;

fn apply(m: &Matrix, (u, v): (f64, f64)) -> (f64, f64) {
    let [a, b, c, d, e, f] = m.0;
    (a * u + c * v + e, b * u + d * v + f)
}

fn spec(sheets: Vec<Vec<ManualPage>>) -> Manual {
    Manual { sheet: (300.0, 200.0), bleed: 0.0, marks: None, sheets }
}

#[test]
fn every_turn_lands_the_page_exactly_on_its_footprint() {
    // A 100 x 40 page at (10, 20): its corners must cover exactly the footprint, and its own
    // bottom-left corner must go where each quarter turn sends it.
    let size = (100.0, 40.0);
    let want_origin = [(10.0, 20.0), (50.0, 20.0), (110.0, 60.0), (10.0, 120.0)];
    for t in 0..4u8 {
        let sheets = manual(&[size], &spec(vec![vec![ManualPage { page: 0, at: (10.0, 20.0), turns: t }]])).expect("layout");
        let p = &sheets[0].placed[0];
        let corners = [(0.0, 0.0), (100.0, 0.0), (0.0, 40.0), (100.0, 40.0)].map(|c| apply(&p.matrix, c));
        let xs = corners.map(|c| c.0);
        let ys = corners.map(|c| c.1);
        let fw = if t % 2 == 1 { 40.0 } else { 100.0 };
        let fh = if t % 2 == 1 { 100.0 } else { 40.0 };
        let min = |v: [f64; 4]| v.iter().copied().fold(f64::MAX, f64::min);
        let max = |v: [f64; 4]| v.iter().copied().fold(f64::MIN, f64::max);
        assert!((min(xs) - 10.0).abs() < EPS && (max(xs) - (10.0 + fw)).abs() < EPS, "turn {t}: x {xs:?}");
        assert!((min(ys) - 20.0).abs() < EPS && (max(ys) - (20.0 + fh)).abs() < EPS, "turn {t}: y {ys:?}");
        let o = corners[0];
        let w = want_origin[t as usize];
        assert!((o.0 - w.0).abs() < EPS && (o.1 - w.1).abs() < EPS, "turn {t}: page origin at {o:?}, want {w:?}");
        // Turning is a rotation, never a reflection.
        let [a, b, c, d, _, _] = p.matrix.0;
        assert!((a * d - b * c - 1.0).abs() < EPS);
    }
}

#[test]
fn marks_come_from_the_trims_of_the_placed_pages() {
    // Two 96 x 60 pages with 3 bleed, side by side with their bleeds touching: the marks between
    // them may not cross either trim, so the inner ones run only in the gap or not at all.
    let mut s = spec(vec![vec![ManualPage { page: 0, at: (20.0, 50.0), turns: 0 }, ManualPage { page: 0, at: (116.0, 50.0), turns: 0 }]]);
    s.bleed = 3.0;
    s.marks = Some(Marks { length: 8.0, offset: 3.0, ..Marks::default() });
    let sheets = manual(&[(96.0, 60.0)], &s).expect("layout");
    let lines = &sheets[0].lines;
    assert!(!lines.is_empty());
    let trims = [[23.0, 53.0, 113.0, 107.0], [119.0, 53.0, 209.0, 107.0]];
    for l in lines {
        let probe = [l[0].min(l[2]) - 0.01, l[1].min(l[3]) - 0.01, l[0].max(l[2]) + 0.01, l[1].max(l[3]) + 0.01];
        assert!(trims.iter().all(|t| !overlaps(probe, *t)), "{l:?} crosses a trim");
    }
    // The outer left marks of page 1 are there.
    assert!(lines.iter().any(|l| (l[2] - 20.0).abs() < EPS && (l[1] - 53.0).abs() < EPS), "{lines:?}");
}

#[test]
fn empty_sheets_stay_as_blank_sides() {
    let sheets = manual(&[(50.0, 50.0)], &spec(vec![vec![ManualPage { page: 0, at: (0.0, 0.0), turns: 0 }], vec![]])).expect("layout");
    assert_eq!(sheets.len(), 2);
    assert!(sheets[1].placed.is_empty());
}

#[test]
fn bad_layouts_are_refused_with_a_reason() {
    let one = |p: ManualPage| spec(vec![vec![p]]);
    assert_eq!(manual(&[(50.0, 50.0)], &one(ManualPage { page: 3, at: (0.0, 0.0), turns: 0 })), Err(ImposeError::NoSuchPage { page: 4, pages: 1 }));
    assert_eq!(manual(&[(50.0, 50.0)], &one(ManualPage { page: 0, at: (300.0, 0.0), turns: 0 })), Err(ImposeError::OffSheet { page: 1 }));
    assert_eq!(manual(&[(50.0, 50.0)], &one(ManualPage { page: 0, at: (-50.0, 0.0), turns: 0 })), Err(ImposeError::OffSheet { page: 1 }));
    assert!(manual(&[(50.0, 50.0)], &one(ManualPage { page: 0, at: (f64::NAN, 0.0), turns: 0 })).is_err());
    assert_eq!(manual(&[], &one(ManualPage { page: 0, at: (0.0, 0.0), turns: 0 })), Err(ImposeError::NoPages));
    assert!(manual(&[(50.0, 50.0)], &spec(vec![])).is_err());
    // Partly off the sheet is allowed: the sheet edge crops it, as on press.
    assert!(manual(&[(50.0, 50.0)], &one(ManualPage { page: 0, at: (-20.0, 0.0), turns: 0 })).is_ok());
}
