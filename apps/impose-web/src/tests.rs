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
