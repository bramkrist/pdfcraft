use super::*;

fn order(n: usize, arrange: Arrange, reverse_evens: bool) -> Vec<usize> {
    shuffle_order(n, &Shuffle { arrange, reverse_evens, turn_evens: false }).into_iter().map(|(p, _)| p).collect()
}

fn sources(sheets: &[Sheet]) -> Vec<usize> {
    sheets.iter().filter_map(|s| s.placed.first().map(|p| p.page)).collect()
}

#[test]
fn reverse_puts_the_last_page_first_unchanged() {
    let s = reverse(&[(100.0, 200.0), (300.0, 400.0), (50.0, 60.0)]).unwrap();
    assert_eq!(sources(&s), vec![2, 1, 0]);
    assert_eq!(s[0].size, (50.0, 60.0));
    assert_eq!(s[2].placed[0].matrix.0, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    assert_eq!(reverse(&[]), Err(ImposeError::NoPages));
}

#[test]
fn odds_first_splits_fronts_from_backs_and_interleave_joins_them_again() {
    assert_eq!(order(5, Arrange::OddsFirst, false), vec![0, 2, 4, 1, 3]);
    assert_eq!(order(5, Arrange::OddsFirst, true), vec![0, 2, 4, 3, 1]);
    assert_eq!(order(6, Arrange::Interleave, false), vec![0, 3, 1, 4, 2, 5]);
    // Backs fed in last page first.
    assert_eq!(order(6, Arrange::Interleave, true), vec![0, 5, 1, 4, 2, 3]);
    // Interleave undoes OddsFirst, with or without reversed backs, for odd and even counts.
    for n in 1..12 {
        for rev in [false, true] {
            let split = order(n, Arrange::OddsFirst, rev);
            let joined: Vec<usize> = order(n, Arrange::Interleave, rev).into_iter().map(|i| split[i]).collect();
            assert_eq!(joined, (0..n).collect::<Vec<_>>(), "n = {n}, reversed backs = {rev}");
        }
    }
}

#[test]
fn keep_only_reverses_or_turns_the_even_pages() {
    assert_eq!(order(6, Arrange::Keep, false), vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(order(6, Arrange::Keep, true), vec![0, 5, 2, 3, 4, 1]);
    assert_eq!(order(5, Arrange::Keep, true), vec![0, 3, 2, 1, 4]);
    let s = shuffle(&[(100.0, 200.0); 3], &Shuffle { turn_evens: true, ..Shuffle::default() }).unwrap();
    assert_eq!(s[0].placed[0].matrix.0, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    assert_eq!(s[1].placed[0].matrix.0, [-1.0, 0.0, 0.0, -1.0, 100.0, 200.0], "page 2 upside down in its own box");
    assert_eq!(s[2].placed[0].matrix.0, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
}

#[test]
fn trimming_shrinks_the_page_and_adding_space_never_uncovers_anything() {
    let trim = TrimShift { trim: Trim::Edges([10.0, 20.0, 30.0, 40.0]), ..TrimShift::default() };
    let s = trim_and_shift(&[(200.0, 300.0)], &trim).unwrap();
    assert_eq!(s[0].size, (160.0, 240.0));
    assert_eq!(s[0].placed[0].matrix.0[4..], [-10.0, -20.0]);
    assert_eq!(s[0].placed[0].clip, [10.0, 20.0, 170.0, 260.0]);
    let grow = TrimShift { trim: Trim::Edges([-10.0, -10.0, -10.0, -10.0]), ..TrimShift::default() };
    let s = trim_and_shift(&[(200.0, 300.0)], &grow).unwrap();
    assert_eq!(s[0].size, (220.0, 320.0));
    assert_eq!(s[0].placed[0].matrix.0[4..], [10.0, 10.0]);
    assert_eq!(s[0].placed[0].clip, [0.0, 0.0, 200.0, 300.0], "the new margin is white, not what lay off the page");
    let all = TrimShift { trim: Trim::Edges([100.0, 0.0, 100.0, 0.0]), ..TrimShift::default() };
    assert!(trim_and_shift(&[(200.0, 300.0)], &all).is_err(), "trimming the whole page away");
}

#[test]
fn same_size_centres_every_page() {
    let spec = TrimShift { trim: Trim::SameSize(200.0, 200.0), ..TrimShift::default() };
    let s = trim_and_shift(&[(100.0, 100.0), (300.0, 300.0)], &spec).unwrap();
    assert_eq!((s[0].size, s[1].size), ((200.0, 200.0), (200.0, 200.0)));
    assert_eq!(s[0].placed[0].matrix.0[4..], [50.0, 50.0]);
    assert_eq!(s[0].placed[0].clip, [0.0, 0.0, 100.0, 100.0]);
    assert_eq!(s[1].placed[0].matrix.0[4..], [-50.0, -50.0]);
    assert_eq!(s[1].placed[0].clip, [50.0, 50.0, 250.0, 250.0], "the larger page is cut round its centre");
}

#[test]
fn shifts_are_fixed_or_spread_across_the_chosen_pages_only() {
    let fixed = TrimShift { pages: PageSet::Even, shift: Shift::Fixed(5.0, 0.0), ..TrimShift::default() };
    let s = trim_and_shift(&[(100.0, 100.0); 4], &fixed).unwrap();
    let dx: Vec<f64> = s.iter().map(|x| x.placed[0].matrix.0[4]).collect();
    assert_eq!(dx, vec![0.0, 5.0, 0.0, 5.0], "only pages 2 and 4 move");
    assert_eq!(s[1].placed[0].clip, [0.0, 0.0, 95.0, 100.0], "what is pushed past the edge is cut");
    let var = TrimShift { pages: PageSet::Odd, shift: Shift::Variable { first: (0.0, 0.0), last: (-4.0, 0.0) }, ..TrimShift::default() };
    let s = trim_and_shift(&[(100.0, 100.0); 5], &var).unwrap();
    let dx: Vec<f64> = s.iter().map(|x| x.placed[0].matrix.0[4]).collect();
    assert_eq!(dx, vec![0.0, 0.0, -2.0, 0.0, -4.0], "pages 1, 3, 5 from 0 to -4 in proportion");
    let off = TrimShift { shift: Shift::Fixed(0.0, 500.0), ..TrimShift::default() };
    assert_eq!(trim_and_shift(&[(100.0, 100.0)], &off), Err(ImposeError::OffSheet { page: 1 }));
    let none = TrimShift { pages: PageSet::Listed(vec![9]), ..TrimShift::default() };
    assert!(trim_and_shift(&[(100.0, 100.0)], &none).is_err(), "a selection with no page of the document");
    let nan = TrimShift { shift: Shift::Fixed(f64::NAN, 0.0), ..TrimShift::default() };
    assert!(trim_and_shift(&[(100.0, 100.0)], &nan).is_err());
}

/// The part of the page a tile shows, in page space.
fn window(s: &Sheet) -> Rect {
    s.placed[0].clip
}

#[test]
fn tiles_cover_the_page_from_the_top_left_and_share_the_overlap() {
    let t = Tile { cols: 2, rows: 2, overlap: 10.0, only_wide: false };
    let s = tile(&[(190.0, 290.0)], &t).unwrap();
    assert_eq!(s.len(), 4);
    for x in &s {
        assert_eq!(x.size, (100.0, 150.0), "2 tiles of 100 overlapping 10 make 190");
        // The window lands on the whole tile: page point (clip x0, clip y0) at the tile's origin.
        let [_, _, _, _, e, f] = x.placed[0].matrix.0;
        assert_eq!((window(x)[0] + e, window(x)[1] + f), (0.0, 0.0));
    }
    assert_eq!(window(&s[0]), [0.0, 140.0, 100.0, 290.0], "first tile = top left");
    assert_eq!(window(&s[1]), [90.0, 140.0, 190.0, 290.0]);
    assert_eq!(window(&s[3]), [90.0, 0.0, 190.0, 150.0], "last = bottom right");
    assert!(tile(&[(100.0, 100.0)], &Tile { cols: 2, rows: 1, overlap: 100.0, only_wide: false }).is_err());
    assert!(tile(&[(100.0, 100.0)], &Tile { cols: 0, rows: 1, overlap: 0.0, only_wide: false }).is_err());
    assert!(tile(&[(100.0, 100.0)], &Tile { cols: 21, rows: 20, overlap: 0.0, only_wide: false }).is_err());
    assert!(tile(&[(100.0, 100.0)], &Tile { cols: usize::MAX, rows: 2, overlap: 0.0, only_wide: false }).is_err());
}

#[test]
fn only_wide_pages_split_spreads_and_keep_single_pages() {
    let t = Tile { cols: 2, rows: 1, overlap: 0.0, only_wide: true };
    let s = tile(&[(100.0, 150.0), (200.0, 150.0), (100.0, 150.0)], &t).unwrap();
    assert_eq!(sources(&s), vec![0, 1, 1, 2]);
    assert!(s.iter().all(|x| x.size == (100.0, 150.0)));
    assert_eq!(window(&s[1]), [0.0, 0.0, 100.0, 150.0]);
    assert_eq!(window(&s[2]), [100.0, 0.0, 200.0, 150.0]);
}
