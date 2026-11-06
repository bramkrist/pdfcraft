use std::sync::Arc;

use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, RenderedPage, RequestKind};

use super::*;

/// A page with two spots: `/Cs1` = PANTONE 185 C (Type 2 tint to CMYK, inline) and
/// `/Cs2` = Gold (Type 4 tint to CMYK, by reference), plus a form that paints with `/Cs1`.
fn pdf_with_spots(content: &str) -> Vec<u8> {
    let ps = "{ dup 0.2 mul exch dup 0.4 mul exch dup 0.9 mul exch 0.1 mul }";
    let form = "/Cs1 cs 0.25 scn 0 0 50 50 re f";
    let objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 300] /Resources << /ColorSpace << /Cs1 [/Separation /PANTONE#20185#20C /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0.9 0.8 0] /N 1 >>] /Cs2 [/Separation /Gold /DeviceCMYK 5 0 R] /Cs3 [/Separation /PANTONE#20185#20CV /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0.9 0.8 0] /N 1 >>] >> /XObject << /Fm 6 0 R >> >> /Contents 4 0 R >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        format!("<< /FunctionType 4 /Domain [0 1] /Range [0 1 0 1 0 1 0 1] /Length {} >>\nstream\n{ps}\nendstream", ps.len()),
        format!("<< /Type /XObject /Subtype /Form /BBox [0 0 400 300] /Matrix [1 0 0 1 300 200] /Resources << /ColorSpace << /Cs1 [/Separation /PANTONE#20185#20C /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0.9 0.8 0] /N 1 >>] >> >> /Length {} >>\nstream\n{form}\nendstream", form.len()),
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

const PAGE: &str = "/Cs1 cs 1 scn 0 0 100 100 re f q /Cs1 cs 0.5 scn 100 0 100 100 re f Q 0 0 0 1 k 0 100 50 50 re f /Cs2 CS 0.6 SCN 8 w 20 250 m 380 250 l S /Cs3 cs 1 scn 200 0 100 100 re f /Fm Do";

fn render(bytes: &[u8]) -> RenderedPage {
    let r = PageRenderer::new(Arc::new(bytes.to_vec()), RenderConfig::default()).render(RenderRequest {
        page: 0,
        kind: RequestKind::Pixels,
        tile: None,
        scale: 1.0,
        tag: 0,
        preview: false,
    });
    assert_eq!(r.error, None);
    r
}

fn max_channel_diff(a: &RenderedPage, b: &RenderedPage) -> u8 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    a.rgba.iter().zip(b.rgba.iter()).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
}

fn content_of(bytes: &[u8]) -> String {
    let doc = Document::open(Arc::new(bytes.to_vec())).expect("open");
    let pages = pdfcraft_model::pages(&doc);
    let page = pages.first().expect("a page");
    String::from_utf8_lossy(&page_content(&doc, &page.dict)).into_owned()
}

fn actions(v: &[(&str, InkAction)]) -> BTreeMap<String, InkAction> {
    v.iter().map(|(n, a)| ((*n).to_string(), a.clone())).collect()
}

#[test]
fn spot_to_process_prints_its_own_alternate_and_looks_the_same() {
    let src = pdf_with_spots(PAGE);
    let (out, report) = manage_inks(src.clone(), &actions(&[("PANTONE 185 C", InkAction::ToProcess), ("Gold", InkAction::ToProcess)])).expect("inks");
    assert_eq!(report.kept, Vec::<String>::new(), "{report:?}");
    // Page: cs+scn twice for 185 C, CS+SCN for Gold; form: cs+scn.
    assert_eq!(report.converted.get("PANTONE 185 C"), Some(&6));
    assert_eq!(report.converted.get("Gold"), Some(&2));
    let text = content_of(&out);
    assert!(text.contains("0 0.9 0.8 0 k 0 0 100 100 re f"), "{text}");
    assert!(text.contains("0 0.45 0.4 0 k 100 0"), "half tint: {text}");
    assert!(text.contains("0.12 0.24 0.54 0.06 K"), "type 4 tint on stroke: {text}");
    assert!(text.contains("/Cs3 cs 1 scn"), "the other spot stays: {text}");
    assert!(max_channel_diff(&render(&src), &render(&out)) <= 2, "converted page prints as it showed");
    assert!(out.starts_with(&src[..src.len() - 10]), "saved incrementally");
}

#[test]
fn alias_puts_two_names_on_one_plate_without_touching_values() {
    let src = pdf_with_spots(PAGE);
    let (out, report) = manage_inks(src.clone(), &actions(&[("PANTONE 185 CV", InkAction::Alias("PANTONE 185 C".into()))])).expect("inks");
    assert_eq!(report.aliased.get("PANTONE 185 CV"), Some(&1), "{report:?}");
    let doc = Document::open(Arc::new(out.clone())).expect("open");
    let names: Vec<String> = crate::analyze(&doc).inks.spots.iter().map(|i| i.name.clone()).collect();
    assert!(names.iter().any(|n| n == "PANTONE 185 C") && !names.iter().any(|n| n == "PANTONE 185 CV"), "{names:?}");
    assert_eq!(content_of(&out), content_of(&src), "content streams unchanged");
    assert_eq!(max_channel_diff(&render(&src), &render(&out)), 0);
}

#[test]
fn spot_images_and_shadings_are_kept_and_reported() {
    let src = pdf_with_spots("/Cs1 cs 1 scn 0 0 10 10 re f");
    // Swap in an image XObject painted through the spot, by editing the fixture.
    let s = String::from_utf8_lossy(&src).replace("/XObject << /Fm 6 0 R >>", "/XObject << /Fm 6 0 R >> /Shading << /Sh << /ShadingType 2 /ColorSpace /Cs1 /Coords [0 0 1 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >> >>");
    let s = s.replace("0 0 10 10 re f", "0 0 10 10 re f /Sh sh");
    let fixed = rebuild_xref(s.as_bytes());
    let (_, report) = manage_inks(fixed, &actions(&[("PANTONE 185 C", InkAction::ToProcess)])).expect("inks");
    assert!(report.kept.iter().any(|k| k.contains("gradasi")), "{report:?}");
}

/// Recompute xref offsets after a text edit of a fixture.
fn rebuild_xref(b: &[u8]) -> Vec<u8> {
    let s = String::from_utf8_lossy(b);
    let body = s.split("xref\n").next().unwrap_or("");
    let mut offs = Vec::new();
    let mut i = 1;
    while let Some(at) = body.find(&format!("\n{i} 0 obj\n")) {
        offs.push(at + 1);
        i += 1;
    }
    let mut out = body.as_bytes().to_vec();
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offs.len() + 1).as_bytes());
    for o in &offs {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offs.len() + 1).as_bytes());
    out
}

#[test]
fn an_alias_into_a_spot_made_process_goes_to_process_too() {
    let src = pdf_with_spots(PAGE);
    let acts = actions(&[("PANTONE 185 CV", InkAction::Alias("PANTONE 185 C".into())), ("PANTONE 185 C", InkAction::ToProcess)]);
    let (out, report) = manage_inks(src.clone(), &acts).expect("inks");
    assert_eq!(report.converted.get("PANTONE 185 CV"), Some(&2), "{report:?}");
    assert!(report.aliased.is_empty(), "{report:?}");
    let doc = Document::open(Arc::new(out.clone())).expect("open");
    let names: Vec<String> = crate::analyze(&doc).inks.spots.iter().map(|i| i.name.clone()).collect();
    assert!(!names.iter().any(|n| n.starts_with("PANTONE")), "{names:?}");
    assert!(max_channel_diff(&render(&src), &render(&out)) <= 2);
}

#[test]
fn alias_cycles_change_nothing() {
    let src = pdf_with_spots(PAGE);
    let acts = actions(&[("PANTONE 185 CV", InkAction::Alias("PANTONE 185 C".into())), ("PANTONE 185 C", InkAction::Alias("PANTONE 185 CV".into()))]);
    let (out, _) = manage_inks(src.clone(), &acts).expect("inks");
    assert_eq!(out, src);
}

#[test]
fn nothing_to_do_returns_the_same_bytes() {
    let src = pdf_with_spots(PAGE);
    let (out, _) = manage_inks(src.clone(), &BTreeMap::new()).expect("inks");
    assert_eq!(out, src);
}

#[test]
fn garbage_is_an_error_not_a_panic() {
    assert!(manage_inks(b"%PDF nope".to_vec(), &actions(&[("X", InkAction::ToProcess)])).is_err());
}
