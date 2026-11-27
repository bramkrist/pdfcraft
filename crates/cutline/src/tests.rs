use super::*;

/// A `w` × `h` mask where `inside(x, y)` is solid.
fn mask(w: usize, h: usize, inside: impl Fn(usize, usize) -> bool) -> Mask {
    let mut data = Vec::new();
    for y in 0..h {
        for x in 0..w {
            data.push(if inside(x, y) { 255 } else { 0 });
        }
    }
    Mask::new(w, h, data).expect("mask")
}

fn outer(loops: &[Loop]) -> Vec<&Loop> {
    loops.iter().filter(|l| !l.hole).collect()
}

fn bounds(l: &Loop) -> (f64, f64, f64, f64) {
    let xs = l.points.iter().map(|p| p.0);
    let ys = l.points.iter().map(|p| p.1);
    (xs.clone().fold(f64::MAX, f64::min), ys.clone().fold(f64::MAX, f64::min), xs.fold(f64::MIN, f64::max), ys.fold(f64::MIN, f64::max))
}

/// A solid square of side 10 with its corner at (5, 5) in a 20 × 20 mask.
fn square() -> Mask {
    mask(20, 20, |x, y| (5..15).contains(&x) && (5..15).contains(&y))
}

fn opts(offset: f64) -> TraceOptions {
    TraceOptions { offset, ..TraceOptions::default() }
}

#[test]
fn a_square_is_traced_at_its_own_edge() {
    let loops = trace(&square(), &opts(0.0)).expect("trace");
    assert_eq!(loops.len(), 1);
    let l = &loops[0];
    assert!(!l.hole, "the shape's own outline is not a hole");
    let (x0, y0, x1, y1) = bounds(l);
    for (got, want) in [(x0, 5.0), (y0, 5.0), (x1, 15.0), (y1, 15.0)] {
        assert!((got - want).abs() < 0.3, "edge {got} should be {want}");
    }
    assert!((l.area() - 100.0).abs() < 2.0, "area {}", l.area());
}

#[test]
fn a_positive_offset_grows_the_shape_by_exactly_that_distance() {
    let loops = trace(&square(), &opts(3.0)).expect("trace");
    let l = &outer(&loops)[0];
    let (x0, y0, x1, y1) = bounds(l);
    for (got, want) in [(x0, 2.0), (y0, 2.0), (x1, 18.0), (y1, 18.0)] {
        assert!((got - want).abs() < 0.3, "edge {got} should be {want}");
    }
    // A square grown by r has rounded corners: 100 + 4 * 10 * r + pi * r^2.
    // The distance field is exact along straight edges and about a fifth of a pixel short at
    // the diagonal of a corner, so the area is a little under the ideal.
    let want = 100.0 + 4.0 * 10.0 * 3.0 + std::f64::consts::PI * 9.0;
    assert!((l.area() - want).abs() / want < 0.03 && l.area() <= want, "area {} vs {want}", l.area());
}

#[test]
fn a_negative_offset_shrinks_the_shape_and_keeps_its_corners() {
    let loops = trace(&square(), &opts(-2.0)).expect("trace");
    let l = &outer(&loops)[0];
    let (x0, _, x1, _) = bounds(l);
    assert!((x0 - 7.0).abs() < 0.3 && (x1 - 13.0).abs() < 0.3);
    assert!((l.area() - 36.0).abs() < 2.0, "area {}", l.area());
}

#[test]
fn shrinking_past_the_shape_leaves_nothing() {
    assert!(trace(&square(), &opts(-6.0)).expect("trace").is_empty());
}

#[test]
fn offsets_are_euclidean_for_a_circle() {
    let circle = mask(100, 100, |x, y| {
        let (dx, dy) = (x as f64 + 0.5 - 50.0, y as f64 + 0.5 - 50.0);
        dx * dx + dy * dy <= 30.0 * 30.0
    });
    for (offset, radius) in [(0.0, 30.0), (6.0, 36.0), (-8.0, 22.0)] {
        let loops = trace(&circle, &opts(offset)).expect("trace");
        assert_eq!(outer(&loops).len(), 1);
        for p in &loops[0].points {
            let r = ((p.0 - 50.0).powi(2) + (p.1 - 50.0).powi(2)).sqrt();
            assert!((r - radius).abs() < 0.6, "offset {offset}: point at radius {r}, want {radius}");
        }
    }
}

/// A ring: a 20 square with a 6 square hole.
fn ring() -> Mask {
    mask(30, 30, |x, y| (5..25).contains(&x) && (5..25).contains(&y) && !((12..18).contains(&x) && (12..18).contains(&y)))
}

#[test]
fn holes_are_filled_unless_kept() {
    let filled = trace(&ring(), &opts(0.0)).expect("trace");
    assert_eq!(filled.len(), 1, "only the outside is cut");
    assert!((filled[0].area() - 400.0).abs() < 4.0);
    let kept = trace(&ring(), &TraceOptions { holes: Holes::Keep, ..opts(0.0) }).expect("trace");
    assert_eq!(kept.len(), 2);
    assert!(!kept[0].hole && kept[1].hole, "the outline first, then its hole");
    assert!((kept[1].area() - 36.0).abs() < 2.0, "hole area {}", kept[1].area());
}

#[test]
fn a_kept_hole_shrinks_when_the_shape_grows() {
    let kept = trace(&ring(), &TraceOptions { holes: Holes::Keep, min_area: 0.0, ..opts(2.0) }).expect("trace");
    let hole = kept.iter().find(|l| l.hole).expect("hole");
    let (x0, _, x1, _) = bounds(hole);
    assert!((x0 - 14.0).abs() < 0.3 && (x1 - 16.0).abs() < 0.3, "{x0}..{x1}");
}

#[test]
fn specks_below_the_minimum_area_are_dropped() {
    let m = mask(30, 30, |x, y| ((5..20).contains(&x) && (5..20).contains(&y)) || (x == 26 && y == 26));
    assert_eq!(trace(&m, &opts(0.0)).expect("trace").len(), 1);
    let all = trace(&m, &TraceOptions { min_area: 0.0, ..opts(0.0) }).expect("trace");
    assert_eq!(all.len(), 2);
}

#[test]
fn separate_shapes_merge_when_the_offset_bridges_the_gap() {
    let m = mask(40, 20, |x, y| (4..14).contains(&y) && ((4..14).contains(&x) || (22..32).contains(&x)));
    assert_eq!(outer(&trace(&m, &opts(0.0)).expect("trace")).len(), 2);
    assert_eq!(outer(&trace(&m, &opts(5.0)).expect("trace")).len(), 1, "an 8 px gap closes under +5");
}

#[test]
fn an_empty_or_solid_mask_traces_cleanly() {
    assert!(trace(&mask(8, 8, |_, _| false), &opts(2.0)).expect("trace").is_empty());
    let solid = trace(&mask(8, 8, |_, _| true), &opts(0.0)).expect("trace");
    assert_eq!(solid.len(), 1);
    let (x0, y0, x1, y1) = bounds(&solid[0]);
    assert!(x0.abs() < 0.3 && y0.abs() < 0.3 && (x1 - 8.0).abs() < 0.3 && (y1 - 8.0).abs() < 0.3);
}

#[test]
fn diagonal_neighbours_never_break_the_outline() {
    // Two pixels touching only at a corner (a saddle in every cell they share).
    let m = mask(8, 8, |x, y| (x == 3 && y == 3) || (x == 4 && y == 4));
    let loops = trace(&m, &TraceOptions { min_area: 0.0, tolerance: 0.0, ..opts(0.0) }).expect("trace");
    assert!(!loops.is_empty());
    assert!(loops.iter().all(|l| l.points.len() >= 3 && l.points.iter().all(|p| p.0.is_finite() && p.1.is_finite())));
}

#[test]
fn simplifying_keeps_the_outline_within_the_tolerance() {
    let circle = mask(100, 100, |x, y| {
        let (dx, dy) = (x as f64 + 0.5 - 50.0, y as f64 + 0.5 - 50.0);
        dx * dx + dy * dy <= 40.0 * 40.0
    });
    let exact = trace(&circle, &TraceOptions { tolerance: 0.0, ..opts(0.0) }).expect("trace");
    let coarse = trace(&circle, &TraceOptions { tolerance: 0.5, ..opts(0.0) }).expect("trace");
    assert!(coarse[0].points.len() * 3 < exact[0].points.len(), "{} vs {}", coarse[0].points.len(), exact[0].points.len());
    for p in &coarse[0].points {
        let r = ((p.0 - 50.0).powi(2) + (p.1 - 50.0).powi(2)).sqrt();
        assert!((r - 40.0).abs() < 1.0, "radius {r}");
    }
}

#[test]
fn the_same_input_always_gives_the_same_outline() {
    let m = ring();
    let a = trace(&m, &TraceOptions { holes: Holes::Keep, ..opts(1.5) }).expect("trace");
    let b = trace(&m, &TraceOptions { holes: Holes::Keep, ..opts(1.5) }).expect("trace");
    assert_eq!(a, b);
}

/// A small deterministic generator so the property test needs no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn random_masks_and_offsets_never_panic_and_always_close() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for round in 0..300 {
        let (w, h) = (1 + (rng.next() % 30) as usize, 1 + (rng.next() % 30) as usize);
        let density = 1 + rng.next() % 9;
        let data: Vec<u8> = (0..w * h).map(|_| if rng.next() % 10 < density { 255 } else { 0 }).collect();
        let m = Mask::new(w, h, data).expect("mask");
        let offset = (rng.next() % 1000) as f64 / 100.0 - 4.0;
        let holes = if round % 2 == 0 { Holes::Keep } else { Holes::Fill };
        let loops = trace(&m, &TraceOptions { offset, holes, min_area: 0.0, tolerance: 0.0, threshold: 128 }).expect("trace");
        for l in &loops {
            assert!(l.points.len() >= 3, "round {round}");
            assert!(l.points.iter().all(|p| p.0.is_finite() && p.1.is_finite()), "round {round}");
            assert!(l.area().is_finite() && l.area() > 0.0, "round {round}: a loop encloses area, got {}", l.area());
        }
    }
}

#[test]
fn the_winding_of_loops_tells_holes_from_shapes() {
    let kept = trace(&ring(), &TraceOptions { holes: Holes::Keep, ..opts(0.0) }).expect("trace");
    let signed = |l: &Loop| simplify::signed_area(&l.points);
    assert!(signed(&kept[0]) < 0.0 && signed(&kept[1]) > 0.0);
}

#[test]
fn nonsense_settings_and_oversize_masks_are_refused() {
    assert_eq!(trace(&square(), &opts(f64::NAN)), Err(CutError::NotFinite));
    assert_eq!(trace(&square(), &opts(1.0e9)), Err(CutError::TooLarge));
    assert_eq!(Mask::new(0, 5, Vec::new()), Err(CutError::BadMask));
    assert_eq!(Mask::new(3, 3, vec![0; 8]), Err(CutError::BadMask));
    let wide = Mask::new(5000, 5000, vec![0; 25_000_000]).expect("mask");
    assert_eq!(trace(&wide, &opts(0.0)), Err(CutError::TooLarge));
}

/// A premultiplied RGBA page: `pixel(x, y)` is `[r, g, b, a]`.
fn rgba(w: usize, h: usize, pixel: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).flat_map(|(x, y)| pixel(x, y)).collect()
}

#[test]
fn transparency_and_white_paper_give_the_same_silhouette() {
    let shape = |x: usize, y: usize| (5..15).contains(&x) && (5..15).contains(&y);
    let alpha = rgba(20, 20, |x, y| if shape(x, y) { [200, 30, 30, 255] } else { [0, 0, 0, 0] });
    let paper = rgba(20, 20, |x, y| if shape(x, y) { [200, 30, 30, 255] } else { [255, 255, 255, 255] });
    let from_alpha = Mask::from_rgba(20, 20, &alpha, Background::Auto).expect("mask");
    let from_paper = Mask::from_rgba(20, 20, &paper, Background::Auto).expect("mask");
    assert_eq!(from_alpha, from_paper);
    // Forced to transparency only, white paper is all artwork.
    let solid = Mask::from_rgba(20, 20, &paper, Background::Transparent).expect("mask");
    assert_ne!(solid, from_paper);
}

#[test]
fn white_inside_artwork_counts_as_a_hole_only_where_it_is_enclosed() {
    // A red square with a white highlight, on white paper.
    let page = rgba(30, 30, |x, y| {
        let body = (5..25).contains(&x) && (5..25).contains(&y);
        let glint = (12..16).contains(&x) && (12..16).contains(&y);
        if body && !glint { [200, 30, 30, 255] } else { [255, 255, 255, 255] }
    });
    let m = Mask::from_rgba(30, 30, &page, Background::Auto).expect("mask");
    let cut = trace(&m, &opts(0.0)).expect("trace");
    assert_eq!(cut.len(), 1, "the highlight is not cut by default");
    assert!((cut[0].area() - 400.0).abs() < 4.0);
}

#[test]
fn the_size_of_the_pixels_must_match() {
    assert_eq!(Mask::from_rgba(2, 2, &[0; 15], Background::Auto), Err(CutError::BadMask));
}

#[test]
fn keeping_only_a_rectangle_cuts_the_artwork_at_its_edge() {
    let mut m = square();
    m.keep_only(10.0, 0.0, 20.0, 20.0);
    let cut = trace(&m, &opts(0.0)).expect("trace");
    let (x0, y0, x1, y1) = bounds(outer(&cut)[0]);
    assert!((x0 - 10.0).abs() < 0.01 && (x1 - 15.0).abs() < 0.01, "x {x0}..{x1}");
    assert!((y0 - 5.0).abs() < 0.01 && (y1 - 15.0).abs() < 0.01, "y {y0}..{y1}");
}

#[test]
fn a_rectangle_that_is_not_one_empties_the_mask() {
    for rect in [(f64::NAN, 0.0, 20.0, 20.0), (15.0, 0.0, 5.0, 20.0), (0.0, 0.0, f64::INFINITY, f64::NAN)] {
        let mut m = square();
        m.keep_only(rect.0, rect.1, rect.2, rect.3);
        assert!(trace(&m, &opts(0.0)).expect("trace").is_empty(), "{rect:?}");
    }
}

#[test]
fn straight_and_premultiplied_pixels_give_the_same_silhouette() {
    // Half-transparent red over white paper, and fully transparent, as both conventions write them.
    let straight: Vec<u8> = [[200u8, 30, 30, 128], [255, 255, 255, 255], [0, 0, 0, 0], [200, 30, 30, 255]].concat();
    let premultiplied: Vec<u8> =
        straight.chunks_exact(4).flat_map(|p| [0, 1, 2].map(|i| (u32::from(p[i]) * u32::from(p[3]) / 255) as u8).into_iter().chain([p[3]])).collect();
    for background in [Background::Transparent, Background::White] {
        let a = Mask::from_straight_rgba(4, 1, &straight, background).expect("straight");
        let b = Mask::from_rgba(4, 1, &premultiplied, background).expect("premultiplied");
        let (a, b) = (a.rows().next().expect("row").to_vec(), b.rows().next().expect("row").to_vec());
        for (x, y) in a.iter().zip(&b) {
            assert!(x.abs_diff(*y) <= 4, "{background:?}: {a:?} against {b:?}");
        }
    }
}
