//! The ink manager through a session: the spots a document uses, and a copy with them changed.

use std::collections::BTreeMap;
use std::sync::Arc;

use pdfcraft_engine::Session;
use pdfcraft_engine::inks::InkAction;

/// One page painting with two spots, `Gold` and `Silver`, each through a Type 2 tint to CMYK.
fn two_spots() -> Vec<u8> {
    let tint = |c1: &str| format!("<< /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [{c1}] /N 1 >>");
    let content = "/Cs1 cs 1 scn 0 0 100 100 re f /Cs2 cs 1 scn 100 0 100 100 re f";
    let objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /ColorSpace << /Cs1 [/Separation /Gold /DeviceCMYK {}] /Cs2 [/Separation /Silver /DeviceCMYK {}] >> >> /Contents 4 0 R >>",
            tint("0 0.2 0.9 0"),
            tint("0 0 0 0.3")
        ),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
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

fn open() -> (Session, pdfcraft_engine::DocId) {
    let mut s = Session::new();
    let id = s.open("spots.pdf", None, Arc::new(two_spots()), None).expect("opens");
    (s, id)
}

#[test]
fn the_documents_spots_are_listed_with_their_pages() {
    let (s, id) = open();
    let spots = s.spot_inks(id).expect("spots");
    let names: Vec<&str> = spots.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, ["Gold", "Silver"]);
    assert!(spots.iter().all(|x| x.pages == [1]));
}

#[test]
fn a_spot_made_process_is_gone_from_the_copy_and_the_original_is_untouched() {
    let (s, id) = open();
    let actions: BTreeMap<String, InkAction> = [("Gold".to_string(), InkAction::ToProcess)].into();
    let (bytes, report) = s.manage_inks_pdf(id, &actions).expect("applies");
    assert!(report.converted.get("Gold").is_some_and(|n| *n > 0), "{report:?}");
    let mut t = Session::new();
    let cid = t.open("copy.pdf", None, Arc::new(bytes), None).expect("the copy opens");
    let names: Vec<String> = t.spot_inks(cid).expect("spots").into_iter().map(|x| x.name).collect();
    assert_eq!(names, ["Silver"], "Gold now prints in process");
    let still: Vec<String> = s.spot_inks(id).expect("spots").into_iter().map(|x| x.name).collect();
    assert_eq!(still, ["Gold", "Silver"], "the open document is not changed");
}

#[test]
fn a_spot_given_to_another_plate_takes_that_name() {
    let (s, id) = open();
    let actions: BTreeMap<String, InkAction> = [("Silver".to_string(), InkAction::Alias("Gold".into()))].into();
    let (bytes, report) = s.manage_inks_pdf(id, &actions).expect("applies");
    assert_eq!(report.aliased.get("Silver"), Some(&1));
    let mut t = Session::new();
    let cid = t.open("copy.pdf", None, Arc::new(bytes), None).expect("opens");
    let names: Vec<String> = t.spot_inks(cid).expect("spots").into_iter().map(|x| x.name).collect();
    assert_eq!(names, ["Gold"], "one plate for both");
}
