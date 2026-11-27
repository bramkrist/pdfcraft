//! Marching squares over a signed distance field, joined into closed loops.
//!
//! A cell's four corners are the centres of four neighbouring pixels. A corner is inside the
//! shape when its value is below the level. Where an edge of the cell joins an inside corner to
//! an outside one the outline crosses it. Each cell links the crossing it is entered by to the
//! crossing it is left by, always travelling with the shape on the left; the crossing a cell
//! leaves by is the one its neighbour is entered by (same edge, same key), so following the
//! links walks a whole outline.

use std::collections::BTreeMap;

/// The position of a cell and the width of the field it is in.
struct Cell {
    x: u64,
    y: u64,
    wide: u64,
}

/// The closed outlines of `{field < level}` as points in lattice coordinates (cell centres at
/// whole numbers). `field` is `width` cells wide.
pub(crate) fn loops(field: &[f32], width: usize, level: f32) -> Vec<Vec<(f32, f32)>> {
    let width = width.max(1);
    let wide = width as u64;
    let mut links = BTreeMap::new();
    let rows = field.chunks_exact(width);
    for (y, (top, bottom)) in rows.clone().zip(rows.skip(1)).enumerate() {
        for (x, (a, b)) in top.windows(2).zip(bottom.windows(2)).enumerate() {
            if let ([tl, tr], [bl, br]) = (a, b) {
                link_cell(&Cell { x: x as u64, y: y as u64, wide }, [*tl, *tr, *br, *bl], level, &mut links);
            }
        }
    }
    walk(links, field, wide, level)
}

/// The key of the edge from lattice point `(x, y)` to its right neighbour, or (when `vertical`)
/// to the one below.
fn edge_key(x: u64, y: u64, wide: u64, vertical: bool) -> u64 {
    ((y.saturating_mul(wide).saturating_add(x)) << 1) | u64::from(vertical)
}

/// Link the crossings of one cell. `corners` run clockwise from the top left.
fn link_cell(cell: &Cell, corners: [f32; 4], level: f32, links: &mut BTreeMap<u64, u64>) {
    let [tl, tr, br, bl] = corners;
    let (x, y, wide) = (cell.x, cell.y, cell.wide);
    // The cell's edges clockwise, each from its first corner to its second.
    let edges = [
        (edge_key(x, y, wide, false), tl, tr),
        (edge_key(x + 1, y, wide, true), tr, br),
        (edge_key(x, y + 1, wide, false), br, bl),
        (edge_key(x, y, wide, true), bl, tl),
    ];
    // Crossings in that order: (edge key, whether the outline leaves the cell there). It leaves
    // where the edge goes from an inside corner to an outside one.
    let mut crossings = [(0u64, false); 4];
    let mut count = 0;
    for (key, from, to) in edges {
        let (inside_from, inside_to) = (from < level, to < level);
        if inside_from != inside_to {
            if let Some(slot) = crossings.get_mut(count) {
                *slot = (key, inside_from);
            }
            count += 1;
        }
    }
    match count {
        2 => {
            let [first, second, ..] = crossings;
            let (entry, exit) = if first.1 { (second.0, first.0) } else { (first.0, second.0) };
            links.insert(entry, exit);
        }
        4 => {
            // A saddle: two inside corners face each other. The cell's centre decides whether
            // they are joined (then the outline cuts off the two outside corners).
            let joined = (tl + tr + br + bl) / 4.0 < level;
            for (i, (key, leaves)) in crossings.iter().enumerate() {
                if *leaves {
                    continue;
                }
                let partner = if joined { (i + 3) % 4 } else { (i + 1) % 4 };
                if let Some((to, _)) = crossings.get(partner) {
                    links.insert(*key, *to);
                }
            }
        }
        _ => {}
    }
}

/// Follow the links into loops, in the order of their smallest key.
fn walk(mut links: BTreeMap<u64, u64>, field: &[f32], wide: u64, level: f32) -> Vec<Vec<(f32, f32)>> {
    let mut out = Vec::new();
    while let Some((start, mut current)) = links.pop_first() {
        let mut keys = vec![start];
        let mut closed = false;
        loop {
            if current == start {
                closed = true;
                break;
            }
            keys.push(current);
            match links.remove(&current) {
                Some(next) => current = next,
                None => break,
            }
        }
        if closed && keys.len() >= 3 {
            out.push(keys.iter().map(|&k| crossing_point(k, field, wide, level)).collect());
        }
    }
    out
}

/// Where the outline crosses the edge `key`: along it, linearly between the two corner values.
fn crossing_point(key: u64, field: &[f32], wide: u64, level: f32) -> (f32, f32) {
    let wide = wide.max(1);
    let vertical = key & 1 == 1;
    let index = key >> 1;
    let (x, y) = (index % wide, index / wide);
    let (x2, y2) = if vertical { (x, y + 1) } else { (x + 1, y) };
    let value =
        |px: u64, py: u64| usize::try_from(py.saturating_mul(wide).saturating_add(px)).ok().and_then(|i| field.get(i)).copied().unwrap_or(level);
    let (a, b) = (value(x, y), value(x2, y2));
    let t = if (b - a).abs() > f32::EPSILON { ((level - a) / (b - a)).clamp(0.0, 1.0) } else { 0.5 };
    (x as f32 + t * (x2 - x) as f32, y as f32 + t * (y2 - y) as f32)
}
