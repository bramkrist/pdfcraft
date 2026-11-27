//! Polygon helpers: area and Douglas–Peucker simplification of a closed outline.

type Point = (f64, f64);

/// The shoelace area of a closed polygon: negative or positive by its winding (see `trace`).
pub(crate) fn signed_area(points: &[Point]) -> f64 {
    let twice: f64 = points.iter().zip(points.iter().cycle().skip(1)).map(|(a, b)| a.0 * b.1 - b.0 * a.1).sum();
    twice / 2.0
}

/// `points` (a closed outline) with every point removed that lies within `tolerance` of the
/// line between its neighbours that are kept.
pub(crate) fn closed(points: &[Point], tolerance: f64) -> Vec<Point> {
    if points.len() <= 3 || tolerance <= 0.0 {
        return points.to_vec();
    }
    let Some(&first) = points.first() else { return Vec::new() };
    // Split the loop at the point furthest from its start; both halves are open chains.
    let far = points
        .iter()
        .enumerate()
        .map(|(i, p)| (i, (p.0 - first.0).powi(2) + (p.1 - first.1).powi(2)))
        .fold((0usize, 0.0f64), |best, cur| if cur.1 > best.1 { cur } else { best })
        .0;
    let (head, tail) = points.split_at(far);
    let mut second: Vec<Point> = tail.to_vec();
    second.push(first);
    let mut first_half: Vec<Point> = head.to_vec();
    if let Some(&mid) = tail.first() {
        first_half.push(mid);
    }
    let mut out = chain(&first_half, tolerance);
    out.pop();
    let mut rest = chain(&second, tolerance);
    rest.pop();
    out.extend(rest);
    out
}

/// Ramer–Douglas–Peucker over an open chain; the ends are always kept.
fn chain(points: &[Point], tolerance: f64) -> Vec<Point> {
    let n = points.len();
    if n < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; n];
    for end in [0, n - 1] {
        if let Some(k) = keep.get_mut(end) {
            *k = true;
        }
    }
    let mut stack = vec![(0usize, n - 1)];
    while let Some((lo, hi)) = stack.pop() {
        if hi <= lo + 1 {
            continue;
        }
        let (a, b) = (at(points, lo), at(points, hi));
        let (mut worst, mut worst_d) = (lo, 0.0f64);
        for i in lo + 1..hi {
            let d = distance_to_segment(at(points, i), a, b);
            if d > worst_d {
                (worst, worst_d) = (i, d);
            }
        }
        if worst_d > tolerance {
            if let Some(k) = keep.get_mut(worst) {
                *k = true;
            }
            stack.push((lo, worst));
            stack.push((worst, hi));
        }
    }
    points.iter().zip(&keep).filter(|(_, k)| **k).map(|(p, _)| *p).collect()
}

fn at(points: &[Point], i: usize) -> Point {
    points.get(i).copied().unwrap_or((0.0, 0.0))
}

fn distance_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}
