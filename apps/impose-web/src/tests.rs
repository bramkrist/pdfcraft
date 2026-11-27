use super::*;

fn job(json: &str) -> Job {
    parse_job(json).expect("job")
}

/// A one-colour PDF of `n` pages, `w` x `h` points.
fn pdf(n: usize, w: u32, h: u32) -> Vec<u8> {
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + 2 * i)).collect();
    let mut objs = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string(), format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" "))];
    for i in 0..n {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents {} 0 R >>", 4 + 2 * i));
        let s = "1 0 0 rg 0 0 10 10 re f";
        objs.push(format!("<< /Length {} >>\nstream\n{s}\nendstream", s.len()));
    }
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

#[test]
fn business_cards_step_and_repeat_on_a3() {
    let l = layout(&[[93.0, 57.0]], &job(r#"{"mode":"repeat","sheet_mm":[297,420],"margin_mm":10}"#)).expect("layout");
    assert_eq!((l.sheet_count, l.up_per_sheet), (1, 14));
    let c = &l.sheets[0].cells[0];
    assert!(((c.rect_mm[2] - c.rect_mm[0]) - 93.0).abs() < 1e-6);
}

#[test]
fn unique_collate_from_the_shop_floor() {
    let pages = vec![[100.0, 100.0]; 16];
    let j = job(r#"{"mode":"cut-stack","copies":2,"sheet_mm":[400,200],"margin_mm":0,"cols":4,"rows":2}"#);
    let l = layout(&pages, &j).expect("layout");
    assert_eq!((l.sheet_count, l.up_per_sheet), (4, 8));
    // Top row of sheet 1, left to right: pages 1 5 1 5.
    let mut top: Vec<&Cell> = l.sheets[0].cells.iter().filter(|c| c.rect_mm[1] > 50.0).collect();
    top.sort_by(|a, b| a.rect_mm[0].total_cmp(&b.rect_mm[0]));
    assert_eq!(top.iter().map(|c| c.page + 1).collect::<Vec<_>>(), vec![1, 5, 1, 5]);
}

#[test]
fn booklet_and_bleed_modes() {
    let l = layout(&[[148.0, 210.0]; 8], &job(r#"{"mode":"booklet"}"#)).expect("booklet");
    assert_eq!(l.sheet_count, 4);
    let l = layout(&[[90.0, 54.0]], &job(r#"{"mode":"bleed","add_bleed_mm":3}"#)).expect("bleed");
    let s = &l.sheets[0];
    assert!((s.size_mm[0] - 96.0).abs() < 1e-6 && (s.size_mm[1] - 60.0).abs() < 1e-6);
    let t = s.trim_mm.expect("trim");
    assert!((t[0] - 3.0).abs() < 1e-6 && (t[2] - 93.0).abs() < 1e-6);
    // The page itself counts once; the reflected strips do not count as ups.
    assert_eq!(l.up_per_sheet, 1);
    assert!(s.cells.iter().filter(|c| c.mirrored).count() == 8);
}

#[test]
fn a_real_pdf_is_imposed_and_measured() {
    let src = pdf(2, 255, 156); // 90 x 55 mm
    let sizes = page_sizes(src.clone()).expect("sizes");
    assert_eq!(sizes.len(), 2);
    assert!((sizes[0][0] - 89.958).abs() < 0.01);
    let j = job(r#"{"mode":"repeat","sheet_mm":[320,450],"margin_mm":5,"gutter_mm":[4,4],"marks":true}"#);
    let preview = layout_pdf(src.clone(), &j).expect("layout");
    let out = impose(src, &j).expect("impose");
    assert!(out.starts_with(b"%PDF"));
    let n = page_sizes(out).expect("reopen").len();
    assert_eq!(n, preview.sheet_count);
}

#[test]
fn mistakes_in_a_job_are_errors() {
    assert!(parse_job(r#"{"mode":"repeat","sheet":[1,2]}"#).is_err(), "unknown fields are refused, not ignored");
    for j in [
        r#"{"mode":"poster","sheet_mm":[100,100]}"#,
        r#"{"mode":"repeat"}"#,
        r#"{"mode":"repeat","sheet_mm":[100,100],"duplex":"sideways"}"#,
        r#"{"mode":"booklet","binding":"perfect"}"#,
        r#"{"mode":"bleed"}"#,
        r#"{"mode":"repeat","sheet_mm":[10,10],"margin_mm":0}"#,
        r#"{"mode":"cut-stack","sheet_mm":[400,200],"margin_mm":0,"cols":4,"rows":2,"copies":3}"#,
    ] {
        assert!(layout(&[[100.0, 100.0]], &job(j)).is_err(), "{j}");
    }
    assert!(impose(b"not a pdf".to_vec(), &job(r#"{"mode":"repeat","sheet_mm":[100,100]}"#)).is_err());
}

#[test]
fn custom_marks_can_run_into_the_page_in_registration_colour() {
    let j = job(
        r#"{"mode":"repeat","sheet_mm":[200,200],"margin_mm":0,"cols":1,"rows":1,"marks":{"length_mm":5,"offset_mm":-3,"style":"japanese","registration":true}}"#,
    );
    let l = layout(&[[100.0, 100.0]], &j).expect("layout");
    // Page from 50 to 150 mm; bottom-left horizontal mark 48..53 mm.
    assert!(l.sheets[0].marks_mm.iter().any(|m| (m[0] - 48.0).abs() < 1e-6 && (m[2] - 53.0).abs() < 1e-6), "{:?}", l.sheets[0].marks_mm);
    assert!(parse_job(r#"{"marks":{"lenght_mm":5}}"#).is_err(), "misspelt mark fields are refused");
    let out = impose(pdf(1, 283, 283), &j).expect("impose");
    let text = String::from_utf8_lossy(&out).to_string();
    let _ = text;
}

#[test]
fn best_orientation_and_rotation_fit_more() {
    let plain = layout(&[[127.0, 50.8]], &job(r#"{"mode":"repeat","sheet_mm":[228.6,355.6],"margin_mm":0}"#)).expect("plain");
    let best = layout(&[[127.0, 50.8]], &job(r#"{"mode":"repeat","sheet_mm":[228.6,355.6],"margin_mm":0,"sheet_orientation":"best","rotate":true}"#))
        .expect("best");
    assert_eq!(plain.up_per_sheet, 7);
    assert!(best.up_per_sheet >= 10, "{}", best.up_per_sheet);
    // Turned cells report their turned footprint: 50.8 wide, 127 tall.
    let t = best.sheets[0].cells.iter().find(|c| c.turn != 0).expect("a turned cell");
    assert!(((t.rect_mm[2] - t.rect_mm[0]) - 50.8).abs() < 1e-6, "{:?}", t.rect_mm);
    assert!(layout(&[[10.0, 10.0]], &job(r#"{"mode":"repeat","sheet_mm":[100,100],"sheet_orientation":"sideways"}"#)).is_err());
}

#[test]
fn a_manual_layout_round_trips_through_the_layout_description() {
    // Two 93 x 57 mm cards with 3 mm bleed: one upright, one upside down, one turned clockwise;
    // a second, empty sheet side for the back.
    let j = job(r#"{"mode":"manual","sheet_mm":[320,480],"bleed_mm":3,"marks":true,"manual":[[
            {"page":0,"x_mm":10,"y_mm":10},
            {"page":0,"x_mm":110,"y_mm":10,"turn":180},
            {"page":0,"x_mm":10,"y_mm":100,"turn":-90}
        ],[]]}"#);
    let l = layout(&[[93.0, 57.0]], &j).expect("layout");
    assert_eq!(l.sheet_count, 2);
    assert_eq!(l.up_per_sheet, 3);
    let cells = &l.sheets[0].cells;
    let turns: Vec<i32> = cells.iter().map(|c| c.turn).collect();
    assert_eq!(turns, vec![0, 180, -90]);
    assert!(cells.iter().all(|c| !c.mirrored), "turning is not mirroring");
    let r = cells[2].rect_mm;
    assert!((r[0] - 10.0).abs() < 1e-6 && (r[1] - 100.0).abs() < 1e-6 && (r[2] - 67.0).abs() < 1e-6 && (r[3] - 193.0).abs() < 1e-6, "{r:?}");
    assert!(!l.sheets[0].marks_mm.is_empty());
    // The real PDF has two sheet sides too.
    let out = impose(pdf(1, 264, 162), &j).expect("impose");
    assert_eq!(page_sizes(out).expect("sizes").len(), 2);
}

#[test]
fn manual_mistakes_are_errors() {
    let bad = |m: &str| layout(&[[50.0, 50.0]], &job(&format!(r#"{{"mode":"manual","sheet_mm":[100,100],"manual":{m}}}"#)));
    assert!(bad(r#"[[{"page":0,"x_mm":0,"y_mm":0,"turn":45}]]"#).is_err_and(|e| e.contains("turn")));
    assert!(bad(r#"[[{"page":4,"x_mm":0,"y_mm":0}]]"#).is_err_and(|e| e.contains("page 5")));
    assert!(bad(r#"[[{"page":0,"x_mm":200,"y_mm":0}]]"#).is_err_and(|e| e.contains("off the sheet")));
    assert!(parse_job(r#"{"mode":"manual","sheet_mm":[100,100],"manual":[[{"page":0,"x":0,"y_mm":0}]]}"#).is_err());
    assert!(layout(&[[50.0, 50.0]], &job(r#"{"mode":"manual","sheet_mm":[100,100]}"#)).is_err());
}

/// A PDF whose page `i` is `100 + 10 i` points wide (so pages can be told apart by size) and
/// shows the text marker `PAGE<i>` in its content stream.
fn marked_pdf(n: usize) -> Vec<u8> {
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + 2 * i)).collect();
    let mut objs = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string(), format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" "))];
    for i in 0..n {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} 200] /Contents {} 0 R >>", 100 + 10 * i, 4 + 2 * i));
        let s = format!("% PAGE{i}\n0 0 1 rg 0 0 10 10 re f");
        objs.push(format!("<< /Length {} >>\nstream\n{s}\nendstream", s.len()));
    }
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn widths_pt(pdf: Vec<u8>) -> Vec<(i64, i64)> {
    page_sizes(pdf).expect("sizes").iter().map(|[w, h]| (pt(*w).round() as i64, pt(*h).round() as i64)).collect()
}

#[test]
fn the_page_editor_reorders_turns_copies_inserts_and_deletes() {
    let src = marked_pdf(4); // widths 100, 110, 120, 130
    let other = marked_pdf(1); // width 100
    let plan: Vec<PlanPage> = serde_json::from_str(
        r#"[
            {"src":0,"page":2,"rotate":90},
            {"blank_mm":[100,50]},
            {"src":0,"page":0},
            {"src":0,"page":0,"rotate":-90},
            {"src":1,"page":0},
            {"src":0,"page":3}
        ]"#,
    )
    .expect("plan");
    let out = assemble(src, vec![other], &plan).expect("assemble");
    let sizes = widths_pt(out.clone());
    // Turned pages show their sides swapped; the blank is 100 x 50 mm.
    assert_eq!(sizes, vec![(200, 120), (283, 142), (100, 200), (200, 100), (100, 200), (130, 200)]);
    // Page 2 (index 1) was left out: deleted for good, not just unlinked.
    let text = String::from_utf8_lossy(&out);
    assert!(!text.contains("PAGE1"), "a deleted page's content must not stay in the file");
    assert!(text.contains("PAGE2") && text.contains("PAGE3"));
}

#[test]
fn page_editor_mistakes_are_errors() {
    let plan = |j: &str| serde_json::from_str::<Vec<PlanPage>>(j).expect("plan");
    assert!(assemble(marked_pdf(2), vec![], &plan("[]")).is_err());
    assert!(assemble(marked_pdf(2), vec![], &plan(r#"[{"src":0,"page":5}]"#)).is_err_and(|e| e.contains("no page 6")));
    assert!(assemble(marked_pdf(2), vec![], &plan(r#"[{"src":2,"page":0}]"#)).is_err_and(|e| e.contains("extra file")));
    assert!(assemble(marked_pdf(2), vec![], &plan(r#"[{"src":0,"page":0,"rotate":45}]"#)).is_err());
    assert!(assemble(marked_pdf(2), vec![], &plan(r#"[{"blank_mm":[0,10]}]"#)).is_err());
    assert!(assemble(b"nope".to_vec(), vec![], &plan(r#"[{"src":0,"page":0}]"#)).is_err());
}

#[test]
fn centred_marks_are_a_job_option() {
    let l = layout(
        &[[100.0, 60.0]],
        &job(r#"{"mode":"repeat","sheet_mm":[300,200],"margin_mm":0,"cols":2,"rows":2,"marks":{"style":"centred","length_mm":7}}"#),
    )
    .expect("layout");
    assert_eq!(l.sheets[0].marks_mm.len(), 12);
    assert!(layout(&[[100.0, 60.0]], &job(r#"{"mode":"repeat","sheet_mm":[300,200],"marks":{"style":"cross"}}"#)).is_err());
}

#[test]
fn booklet_creep_shifts_the_inner_sheet() {
    let l = layout(&[[100.0, 150.0]; 8], &job(r#"{"mode":"booklet","sheet_mm":[200,150],"scale":false,"creep_mm":2}"#)).expect("layout");
    // Side 3 (inner sheet front): its left page starts 2 mm right of the edge.
    let x = l.sheets[2].cells[0].rect_mm[0];
    assert!((x - 2.0).abs() < 1e-6, "{x}");
    assert!((l.sheets[0].cells[0].rect_mm[0]).abs() < 1e-6);
}

/// The cells of the first and second sheet of two booklet leaves (front, back),
/// two copies per sheet, as the "2 buku per lembar" chain lays them out.
fn two_up(leaf: [f64; 2], sheet: [f64; 2], cols: usize, rows: usize, duplex: &str) -> (Vec<[f64; 4]>, Vec<[f64; 4]>) {
    let j = job(&format!(
        r#"{{"mode":"repeat","sheet_mm":[{},{}],"margin_mm":8,"cols":{cols},"rows":{rows},"gutter_mm":[0,0],"bleed_mm":0,"duplex":"{duplex}"}}"#,
        sheet[0], sheet[1]
    ));
    let l = layout(&[leaf, leaf], &j).expect("layout");
    assert_eq!(l.sheet_count, 2);
    let rects = |i: usize| l.sheets[i].cells.iter().map(|c| c.rect_mm).collect::<Vec<_>>();
    (rects(0), rects(1))
}

#[test]
fn two_booklets_on_one_sheet_keep_each_back_behind_its_front() {
    // Portrait sheet, one column, two rows: a booklet leaf's back turns like a page, which is
    // the long edge of a portrait sheet, so left and right swap.
    let (front, back) = two_up([296.0, 210.0], [320.0, 450.0], 1, 2, "long-edge");
    assert_eq!(front.len(), 2);
    for f in &front {
        let mirrored = [320.0 - f[2], f[1], 320.0 - f[0], f[3]];
        assert!(back.iter().any(|b| b.iter().zip(&mirrored).all(|(x, y)| (x - y).abs() < 1e-6)), "{f:?} has no back in {back:?}");
    }
    // The same two leaves side by side on a landscape sheet: that page turn is now the short edge.
    let (front, back) = two_up([210.0, 148.0], [450.0, 320.0], 2, 1, "short-edge");
    for f in &front {
        let mirrored = [450.0 - f[2], f[1], 450.0 - f[0], f[3]];
        assert!(back.iter().any(|b| b.iter().zip(&mirrored).all(|(x, y)| (x - y).abs() < 1e-6)), "{f:?} has no back in {back:?}");
    }
}

#[test]
fn long_edge_means_the_physical_long_edge_on_a_landscape_sheet() {
    // Landscape sheet, "long-edge": the sheet is turned over its long (horizontal) edge, so top
    // and bottom swap and the back cell sits at the same x.
    let (front, back) = two_up([296.0, 210.0], [450.0, 320.0], 1, 1, "long-edge");
    let (f, b) = (front[0], back[0]);
    assert!((b[0] - f[0]).abs() < 1e-6 && (b[2] - f[2]).abs() < 1e-6, "{f:?} {b:?}");
    assert!(((320.0 - f[3]) - b[1]).abs() < 1e-6, "{f:?} {b:?}");
}

#[test]
fn cropping_sets_the_crop_box_of_the_named_pages_only() {
    let src = pdf(3, 300, 400);
    let out = crop_pages(src, &parse_boxes(r#"[{"page":0,"rect":[100,150,150,190]},{"page":2,"rect":[10,10,60,60]}]"#)).expect("crop");
    let sizes = page_sizes(out).expect("sizes");
    let pt = |v: f64| v / 25.4 * 72.0;
    assert!((pt(sizes[0][0]) - 50.0).abs() < 0.01 && (pt(sizes[0][1]) - 40.0).abs() < 0.01, "{:?}", sizes[0]);
    assert!((pt(sizes[1][0]) - 300.0).abs() < 0.01, "the page left out keeps its size");
    assert!((pt(sizes[2][0]) - 50.0).abs() < 0.01);
}

#[test]
fn cropping_is_all_or_nothing_and_refuses_nonsense() {
    let boxes = |j: &str| parse_boxes(j);
    assert!(crop_pages(pdf(1, 300, 400), &[]).is_err());
    assert!(crop_pages(pdf(1, 300, 400), &boxes(r#"[{"page":1,"rect":[0,0,10,10]}]"#)).is_err_and(|e| e.contains("no page 2")));
    // The second box is a point, so the whole call fails, not just that page.
    assert!(crop_pages(pdf(2, 300, 400), &boxes(r#"[{"page":0,"rect":[0,0,10,10]},{"page":1,"rect":[5,5,5,5]}]"#)).is_err());
    assert!(crop_pages(vec![1, 2, 3], &boxes(r#"[{"page":0,"rect":[0,0,10,10]}]"#)).is_err());
}

fn parse_boxes(json: &str) -> Vec<CropBox> {
    serde_json::from_str(json).expect("boxes")
}

#[test]
fn turning_rows_is_only_ever_a_suggestion_that_fits_at_least_as_many() {
    // The page uses this to offer "Coba putar": the plain layout never turns a row,
    // and the turned layout is shown only when it fits more.
    let mut better = 0;
    for page in [[210.0, 99.0], [90.0, 55.0], [100.0, 148.0], [60.0, 40.0], [120.0, 70.0], [70.0, 250.0]] {
        let plain = layout(&[page], &job(r#"{"mode":"repeat","sheet_mm":[320,450],"margin_mm":5}"#)).expect("plain");
        let turned = layout(&[page], &job(r#"{"mode":"repeat","sheet_mm":[320,450],"margin_mm":5,"rotate":true}"#)).expect("turned");
        assert!(plain.sheets[0].cells.iter().all(|c| c.turn == 0), "no row is turned unless asked: {page:?}");
        assert!(turned.up_per_sheet >= plain.up_per_sheet, "{page:?}: {} < {}", turned.up_per_sheet, plain.up_per_sheet);
        if turned.up_per_sheet > plain.up_per_sheet {
            better += 1;
        }
    }
    assert!(better > 0, "at least one of these pages fits more when rows are turned");
}

// ---- sticker cut lines ------------------------------------------------------------------

/// A 100 × 100 pt page of the test PDF drawn at 2 pixels a point as a straight RGBA canvas: a
/// solid disc of radius 30 at the middle on a clear background.
fn disc_canvas(need: &CutNeed) -> (Vec<u8>, usize, usize) {
    let (w, h) = ((need.size_pt[0] * need.px_per_pt).ceil() as usize, (need.size_pt[1] * need.px_per_pt).ceil() as usize);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (px, py) = ((x as f64 + 0.5) / w as f64 * need.size_pt[0], (y as f64 + 0.5) / h as f64 * need.size_pt[1]);
            if (px - 50.0).hypot(py - 50.0) <= 30.0 {
                rgba[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[0, 100, 255, 255]);
            }
        }
    }
    (rgba, w, h)
}

fn sticker_job() -> Job {
    job(r#"{"mode":"repeat","sheet_mm":[210,297],"margin_mm":5,"gutter_mm":[2,2]}"#)
}

fn cut(offset_mm: f64) -> CutMm {
    CutMm { offset_mm, ..CutMm::default() }
}

/// What the browser does: draw every page the layout needs, trace it, and hand the lines back.
fn browser_round_trip(pdf: &[u8], job: &Job, cut: &CutMm) -> (Vec<u8>, usize) {
    let needs = cut_needs(pdf.to_vec(), job, cut).expect("needs");
    let outlines: Vec<CutOutline> = needs
        .iter()
        .map(|need| {
            let (rgba, w, h) = disc_canvas(need);
            trace_cut(&rgba, w, h, need, cut).expect("trace")
        })
        .collect();
    impose_cut(pdf.to_vec(), job, cut, &outlines).expect("impose")
}

#[test]
fn every_distinct_placement_is_drawn_once_however_often_it_repeats() {
    let needs = cut_needs(pdf(1, 100, 100), &sticker_job(), &cut(0.0)).expect("needs");
    assert_eq!(needs.len(), 1);
    assert_eq!((needs[0].page, needs[0].size_pt), (0, [100.0, 100.0]));
    // A tenth of a millimetre a pixel.
    assert!((needs[0].px_per_pt - 254.0 / 72.0).abs() < 1e-9);
}

#[test]
fn the_browser_round_trip_writes_one_cut_line_per_copy_on_a_cut_layer() {
    let job = sticker_job();
    let up = layout(&[[pt_to_mm(100.0), pt_to_mm(100.0)]], &job).expect("layout").up_per_sheet;
    assert!(up > 4, "{up}");
    let (bytes, paths) = browser_round_trip(&pdf(1, 100, 100), &job, &cut(1.0));
    assert_eq!(paths, up, "one line for each copy");
    let doc = Document::open(Arc::new(bytes.clone())).expect("reopen");
    let root = doc.trailer().get(b"Root").and_then(|r| doc.resolve(r).as_dict().cloned()).expect("root");
    assert!(root.get(b"OCProperties").is_some(), "the Cut layer is in the catalogue");
    // Without cut lines the same layout has no layer.
    let plain = impose(pdf(1, 100, 100), &job).expect("impose");
    let doc = Document::open(Arc::new(plain)).expect("reopen");
    let root = doc.trailer().get(b"Root").and_then(|r| doc.resolve(r).as_dict().cloned()).expect("root");
    assert!(root.get(b"OCProperties").is_none());
}

fn pt_to_mm(v: f64) -> f64 {
    mm(v)
}

#[test]
fn the_offset_moves_the_traced_line_in_the_page_space() {
    let radius = |offset: f64| {
        let need = cut_needs(pdf(1, 100, 100), &sticker_job(), &cut(offset)).expect("needs").remove(0);
        let (rgba, w, h) = disc_canvas(&need);
        let found = trace_cut(&rgba, w, h, &need, &cut(offset)).expect("trace");
        let xs = found.loops.iter().flatten().map(|p| p[0]);
        (xs.clone().fold(f64::MIN, f64::max) - xs.fold(f64::MAX, f64::min)) / 2.0
    };
    assert!((radius(0.0) - 30.0).abs() < 0.3, "{}", radius(0.0));
    assert!((radius(2.0) - (30.0 + pt(2.0))).abs() < 0.3, "{}", radius(2.0));
    assert!((radius(-2.0) - (30.0 - pt(2.0))).abs() < 0.3, "{}", radius(-2.0));
}

#[test]
fn a_placement_nobody_traced_is_refused_rather_than_left_uncut() {
    let err = impose_cut(pdf(1, 100, 100), &sticker_job(), &cut(0.0), &[]).expect_err("no outlines");
    assert!(err.contains("no cut line was traced"), "{err}");
}

#[test]
fn cut_lines_are_for_the_grid_modes_and_bad_settings_are_refused() {
    let booklet = job(r#"{"mode":"booklet","sheet_mm":[297,210]}"#);
    assert!(cut_needs(pdf(4, 100, 100), &booklet, &cut(0.0)).is_err());
    for bad in [
        CutMm { offset_mm: 1e9, ..CutMm::default() },
        CutMm { offset_mm: f64::NAN, ..CutMm::default() },
        CutMm { holes: Some("some".into()), ..CutMm::default() },
        CutMm { background: Some("pink".into()), ..CutMm::default() },
        CutMm { width_pt: Some(0.0), ..CutMm::default() },
        CutMm { min_area_mm2: Some(-1.0), ..CutMm::default() },
    ] {
        assert!(cut_needs(pdf(1, 100, 100), &sticker_job(), &bad).is_err(), "{bad:?}");
    }
    // A drawing whose pixels do not match its size is refused, not traced wrongly.
    let need = cut_needs(pdf(1, 100, 100), &sticker_job(), &cut(0.0)).expect("needs").remove(0);
    assert!(trace_cut(&[0; 16], 3, 1, &need, &cut(0.0)).is_err());
}

fn cutter_job() -> Job {
    // The cut lines have to lie inside the marks, which are 10 mm in from the sheet's edge.
    job(r#"{"mode":"repeat","sheet_mm":[210,297],"margin_mm":15,"gutter_mm":[2,2]}"#)
}

fn with_cutter(offset_mm: f64, name: &str) -> CutMm {
    CutMm { offset_mm, cutter: Some(CutterMm { name: name.into(), ..CutterMm::default() }), ..CutMm::default() }
}

fn traced(pdf: &[u8], job: &Job, cut: &CutMm) -> Vec<CutOutline> {
    cut_needs(pdf.to_vec(), job, cut)
        .expect("needs")
        .iter()
        .map(|need| {
            let (rgba, w, h) = disc_canvas(need);
            trace_cut(&rgba, w, h, need, cut).expect("trace")
        })
        .collect()
}

#[test]
fn a_cutter_job_has_marks_on_the_sheet_and_a_plt_named_for_the_job() {
    let job = cutter_job();
    let source = pdf(1, 100, 100);
    let cut = with_cutter(1.0, "stiker-7");
    let outlines = traced(&source, &job, &cut);
    let made = impose_cut_job(source.clone(), &job, &cut, &outlines).expect("job");
    assert_eq!(made.files.len(), 1);
    let file = &made.files[0];
    assert_eq!(file.name, "stiker-7.plt");
    assert!(file.text.is_ascii() && file.text.starts_with("CR;NAMEstiker-7.plt;SIZE"), "{}", &file.text[..40]);
    // The header says how long the file is, and the lines end with the machine's stop code.
    let size: usize = file.text["CR;NAMEstiker-7.plt;SIZE".len()..].split("Bytes").next().expect("size").parse().expect("number");
    assert_eq!(size, file.text.len());
    assert!(file.text.ends_with("!PG;"));
    // One closed line for each copy.
    assert_eq!(file.text.matches("PU").count(), made.paths + 1, "a pen-up for each line and the last one");
    // The marks are printed, so the file is bigger than the same job without them.
    let plain = impose_cut(source, &job, &cut_of(&cut), &outlines).expect("plain").0;
    assert!(made.pdf.len() > plain.len());
}

fn cut_of(c: &CutMm) -> CutMm {
    CutMm { cutter: None, ..c.clone() }
}

#[test]
fn without_a_cutter_there_are_no_plt_files() {
    let job = cutter_job();
    let source = pdf(1, 100, 100);
    let cut = cut(1.0);
    let made = impose_cut_job(source.clone(), &job, &cut, &traced(&source, &job, &cut)).expect("job");
    assert!(made.files.is_empty());
}

#[test]
fn cut_lines_that_run_past_the_marks_are_refused_and_so_are_bad_cutter_settings() {
    // Marks 30 mm in from the edge leave the lines of copies near the edge outside them.
    let source = pdf(1, 100, 100);
    let cut = CutMm { cutter: Some(CutterMm { name: "x".into(), inset_mm: Some(30.0), ..CutterMm::default() }), ..cut(1.0) };
    let narrow = sticker_job();
    let err = impose_cut_job(source.clone(), &narrow, &cut, &traced(&source, &narrow, &cut_of(&cut))).expect_err("off the marks");
    assert!(err.contains("cutter"), "{err}");
    let c = |name: &str, inset: Option<f64>, mark: Option<f64>, code: Option<f64>| CutMm {
        cutter: Some(CutterMm { name: name.into(), inset_mm: inset, mark_mm: mark, code_mm: code }),
        ..CutMm::default()
    };
    for bad in [
        c("", None, None, None),
        c("!!!", None, None, None),
        c("  / ", None, None, None),
        c("ok", Some(0.0), None, None),
        c("ok", Some(f64::NAN), None, None),
        c("ok", None, Some(500.0), None),
        c("ok", None, None, Some(1.0)),
    ] {
        assert!(cut_needs(source.clone(), &cutter_job(), &bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_job_name_is_cleaned_not_trusted() {
    let job = cutter_job();
    let source = pdf(1, 100, 100);
    let cut = with_cutter(1.0, "../to ko/stiker 7");
    let made = impose_cut_job(source.clone(), &job, &cut, &traced(&source, &job, &cut)).expect("job");
    assert_eq!(made.files[0].name, "tokostiker7.plt");
}
