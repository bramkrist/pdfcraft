use super::*;

const MM: f64 = PT_PER_MM;

/// A rounded-corner-free box `w` × `h` mm at (`x`, `y`) mm of the sheet, counter-clockwise.
fn boxed(x: f64, y: f64, w: f64, h: f64) -> Vec<(f64, f64)> {
    let (x0, y0, x1, y1) = (x * MM, y * MM, (x + w) * MM, (y + h) * MM);
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

/// Four columns of three boxes, laid out as in a working job: 109 × 80 mm on a 480 × 320 mm sheet
/// whose marks are 10 mm in.
fn grid() -> Vec<Vec<(f64, f64)>> {
    let mut loops = Vec::new();
    for col in 0..4 {
        for row in 0..3 {
            loops.push(boxed(10.0 + 3.0 + f64::from(col) * 115.0, 10.0 + 25.0 + f64::from(row) * 85.0, 109.0, 80.0));
        }
    }
    loops
}

fn job(loops: &[Vec<(f64, f64)>]) -> Job<'_> {
    Job { name: "271739", sheet: (480.0 * MM, 320.0 * MM), layout: Layout::default(), loops }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn the_header_names_the_marks_the_job_and_its_own_size() {
    let loops = grid();
    let file = plt(&job(&loops)).expect("plt");
    let head = text(&file[..HEADER_BYTES]);
    assert!(head.starts_with("CR;NAME271739.plt;SIZE"), "{head}");
    assert!(head.contains(";SC1;MN4;MK1;MR1;TB25;18400,12000,400,400;CT1;"), "{head}");
    assert!(head[head.find("CT1;").unwrap_or(0) + 4..].bytes().all(|b| b == b' '), "padded with spaces only");
    let size: usize = head.split("SIZE").nth(1).and_then(|s| s.split("Bytes").next()).and_then(|n| n.parse().ok()).expect("size");
    assert_eq!(size, file.len(), "the header's size is the file's");
    assert!(text(&file[HEADER_BYTES..]).starts_with("IN;PA;PU"));
    assert!(text(&file).ends_with("PU18400,0;!PG;"));
}

#[test]
fn outlines_are_clockwise_from_the_leftmost_point_and_closed() {
    let loops = vec![boxed(20.0, 20.0, 10.0, 6.0)];
    let file = text(&plt(&job(&loops)).expect("plt"));
    let body = &file[HEADER_BYTES..];
    // Origin at the lower-left mark (10, 10) mm: the box is at (10..20, 10..16) mm, 40 units a mm.
    // The leftmost x is 400; of its two corners the lower, 400, starts the shape.
    let commands: Vec<&str> = body.split(';').filter(|c| !c.is_empty()).collect();
    assert_eq!(commands.first(), Some(&"IN"));
    let first_pu = commands.iter().position(|c| c.starts_with("PU")).expect("pu");
    let start = commands[first_pu].trim_start_matches("PU").to_string();
    assert_eq!(commands[first_pu + 1], format!("PD{start}"), "the first point is drawn to as well");
    assert_eq!(commands.iter().rfind(|c| c.starts_with("PD")).copied(), Some(format!("PD{start}").as_str()), "and the shape is closed");
    // Clockwise with y up: from the left side it goes up first.
    let pts: Vec<(i64, i64)> = commands[first_pu + 1..]
        .iter()
        .take_while(|c| c.starts_with("PD"))
        .filter_map(|c| {
            let (x, y) = c[2..].split_once(',')?;
            Some((x.parse().ok()?, y.parse().ok()?))
        })
        .collect();
    assert_eq!(pts.len(), 5, "the start, three more corners, back to the start: {pts:?}");
    assert_eq!(pts[0], (400, 400), "{pts:?}");
    assert!(area(&pts[..pts.len() - 1]) < 0, "clockwise: {pts:?}");
    assert!(pts.iter().all(|p| (400..=800).contains(&p.0) && (0..=2000).contains(&p.1)), "{pts:?}");
}

#[test]
fn a_counter_clockwise_loop_is_reversed_and_a_repeated_end_point_is_dropped() {
    let mut l = boxed(20.0, 20.0, 10.0, 6.0);
    l.push(l[0]);
    let a = plt(&job(&[l.clone()])).expect("plt");
    l.reverse();
    let b = plt(&job(&[l])).expect("plt");
    assert!(a == b, "the same file whichever way the loop ran");
}

#[test]
fn shapes_are_cut_in_a_serpentine_column_by_column() {
    let loops = grid();
    let file = text(&plt(&job(&loops)).expect("plt"));
    let starts: Vec<(i64, i64)> = file[HEADER_BYTES..]
        .split(';')
        .filter(|c| c.starts_with("PU") && c.contains(','))
        .filter_map(|c| {
            let (x, y) = c[2..].split_once(',')?;
            Some((x.parse().ok()?, y.parse().ok()?))
        })
        .filter(|p| p.1 != 0)
        .collect();
    assert_eq!(starts.len(), 12);
    // The left edge of every column: 119-ish units from the origin, then 4600 apart; the shapes'
    // rows go up the first column, down the second, up the third.
    let col = |i: usize| (starts[i].0 + 2300) / 4600;
    assert_eq!([col(0), col(3), col(6), col(9)], [0, 1, 2, 3], "{starts:?}");
    let rows: Vec<i64> = starts.iter().map(|p| (p.1 - 800) / 3400).collect();
    assert_eq!(rows, vec![0, 1, 2, 2, 1, 0, 0, 1, 2, 2, 1, 0], "{starts:?}");
}

#[test]
fn nothing_to_cut_and_bad_input_are_errors_not_panics() {
    assert_eq!(plt(&job(&[])), Err(CutterError::NothingToCut));
    assert_eq!(plt(&job(&[vec![(1.0, 1.0), (2.0, 2.0)]])), Err(CutterError::NothingToCut), "a line is not a shape");
    let nan = vec![(f64::NAN, 0.0), (1.0, 0.0), (1.0, 1.0)];
    assert!(matches!(plt(&job(&[nan])), Err(CutterError::BadNumber(_))));
    let far = vec![(1e300, 0.0), (1.0, 0.0), (1.0, 1.0)];
    assert!(matches!(plt(&job(&[far])), Err(CutterError::BadNumber(_))));
    let loops = grid();
    for name in ["", "   ", "../.."] {
        let name = if name == "../.." { "///" } else { name };
        assert_eq!(plt(&Job { name, ..job(&loops) }), Err(CutterError::BadName), "{name:?}");
    }
    let mut tight = job(&loops);
    tight.layout.inset_mm = 3.0;
    assert_eq!(plt(&tight), Err(CutterError::NoRoom));
    tight.layout = Layout { inset_mm: f64::NAN, ..Layout::default() };
    assert!(matches!(plt(&tight), Err(CutterError::BadNumber(_))));
    tight.layout = Layout::default();
    tight.sheet = (0.0, 100.0);
    assert!(matches!(plt(&tight), Err(CutterError::BadNumber(_))));
}

#[test]
fn a_job_name_is_made_safe() {
    assert_eq!(clean_name("271 739/../x.plt").as_deref(), Ok("271739xplt"));
    assert_eq!(clean_name(&"a".repeat(80)).map(|n| n.len()), Ok(MAX_NAME));
}

#[test]
fn four_marks_sit_in_the_corners_and_the_code_in_the_bottom_margin() {
    let m = marks((480.0 * MM, 320.0 * MM), "271739", &Layout::default()).expect("marks");
    let squares = &m.black[..4];
    let centres: Vec<(f64, f64)> = squares.iter().map(|r| ((r[0] + r[2]) / 2.0 / MM, (r[1] + r[3]) / 2.0 / MM)).collect();
    for (got, want) in centres.iter().zip([(10.0, 10.0), (470.0, 10.0), (10.0, 310.0), (470.0, 310.0)]) {
        assert!((got.0 - want.0).abs() < 1e-9 && (got.1 - want.1).abs() < 1e-9, "{centres:?}");
    }
    for r in squares {
        assert!(((r[2] - r[0]) / MM - 5.0).abs() < 1e-9 && ((r[3] - r[1]) / MM - 5.0).abs() < 1e-9);
    }
    let code = &m.black[4..];
    assert!(code.len() > 20, "a QR code has many dark runs: {}", code.len());
    let clear = m.clear[0];
    for r in code {
        assert!(r[0] >= clear[0] && r[1] >= clear[1] && r[2] <= clear[2] && r[3] <= clear[3], "inside its clear border");
        assert!(r[3] / MM <= 10.0 - 2.5 || r[0] / MM > 15.0, "clear of the lower marks");
    }
    assert!(clear[1] > 0.0 && clear[3] / MM < 10.0, "in the margin, inside the sheet: {clear:?}");
}

#[test]
fn the_code_names_the_job() {
    let a = marks((480.0 * MM, 320.0 * MM), "271739", &Layout::default()).expect("marks");
    let b = marks((480.0 * MM, 320.0 * MM), "271740", &Layout::default()).expect("marks");
    assert_ne!(a.black, b.black);
    assert_eq!(a, marks((480.0 * MM, 320.0 * MM), "271739", &Layout::default()).expect("marks"));
}

#[test]
fn a_cut_line_outside_the_marks_is_refused() {
    // 8 mm from the sheet's edge is outside the marks (10 mm in); 20 mm is inside.
    assert_eq!(plt(&job(&[boxed(8.0, 50.0, 20.0, 20.0)])), Err(CutterError::OutsideMarks));
    assert_eq!(plt(&job(&[boxed(50.0, 311.0, 20.0, 20.0)])), Err(CutterError::OutsideMarks));
    assert!(plt(&job(&[boxed(10.0, 10.0, 460.0, 300.0)])).is_ok(), "exactly on the marks' rectangle is inside it");
}
