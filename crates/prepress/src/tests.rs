use std::sync::Arc;

use super::*;

/// A PDF from object bodies (1-based, object 1 the catalog) whose pages are `pages`
/// (`(resources, content, extra page entries)`), each 595 x 842 pt unless `extra` says otherwise.
fn build(pages: &[(&str, &str, &str)], extra_objects: &[&str], catalog_extra: &str) -> Vec<u8> {
    let n_extra = extra_objects.len();
    // Layout: 1 catalog, 2 pages, 3.. extra objects, then page + content pairs.
    let first_page = 3 + n_extra;
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", first_page + 2 * i)).collect();
    let mut objs = vec![
        format!("<< /Type /Catalog /Pages 2 0 R {catalog_extra} >>"),
        format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()),
    ];
    objs.extend(extra_objects.iter().map(|s| s.to_string()));
    for (i, (res, content, extra)) in pages.iter().enumerate() {
        let media = if extra.contains("/MediaBox") { String::new() } else { "/MediaBox [0 0 595 842]".to_string() };
        objs.push(format!("<< /Type /Page /Parent 2 0 R {media} /Resources {res} /Contents {} 0 R {extra} >>", first_page + 2 * i + 1));
        objs.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
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

fn open(bytes: Vec<u8>) -> Document {
    Document::open(Arc::new(bytes)).expect("open")
}

fn check<'a>(c: &'a [Check], id: &str) -> &'a Check {
    c.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no check {id}: {c:?}"))
}

#[test]
fn a_clean_cmyk_file_with_bleed_passes() {
    // 90 x 54 mm trim with 3 mm bleed; a CMYK box, 100% K text in an embedded font.
    let doc = open(build(
        &[(
            "<< /Font << /F1 3 0 R >> >>",
            "0 0 0 1 k BT /F1 12 Tf 20 20 Td (Hi) Tj ET 0.2 0.3 0 0 k 0 0 100 100 re f",
            "/MediaBox [0 0 272.13 170.08] /TrimBox [8.504 8.504 263.622 161.575] /BleedBox [0 0 272.13 170.08]",
        )],
        &[
            "<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+Inter /FontDescriptor 4 0 R >>",
            "<< /Type /FontDescriptor /FontName /ABCDEF+Inter /FontFile2 5 0 R >>",
            "<< /Length 0 >>\nstream\n\nendstream",
        ],
        "",
    ));
    let r = analyze(&doc);
    let c = checks(&r, &Thresholds::default());
    assert_eq!(check(&c, "bleed").status, Status::Pass, "{:?}", r.pages);
    assert_eq!(check(&c, "fonts").status, Status::Pass);
    assert!(r.fonts.first().is_some_and(|f| f.embedded && f.subset));
    assert_eq!(check(&c, "color_mode").status, Status::Pass);
    assert!(r.inks.cyan && r.inks.magenta && r.inks.black && !r.inks.yellow);
    assert_eq!(overall(&c), Status::Pass, "{c:#?}");
}

#[test]
fn the_usual_problems_are_all_found() {
    let res = "<< /Font << /F1 3 0 R >> /ColorSpace << /P185 [/Separation /PANTONE#20185#20C /DeviceCMYK 4 0 R] /Cut [/Separation /CutContour /DeviceCMYK 4 0 R] /Reg [/Separation /All /DeviceCMYK 4 0 R] >> /ExtGState << /T << /ca 0.5 >> /O << /OP true /op true >> >> >>";
    let content = "1 0 0 rg 0 0 50 50 re f \
        0.6 0.5 0.4 1 k BT /F1 7 Tf 10 10 Td (small) Tj ET \
        0.1 w 0 0 0 1 K 0 0 m 100 100 l S \
        /P185 cs 1 scn 100 100 50 50 re f \
        /Cut CS 1 SCN 0 0 m 10 0 l S \
        /Reg cs 1 scn 200 200 5 5 re f \
        /T gs /O gs 1 1 1 1 k 300 300 20 20 re f";
    let doc = open(build(
        &[(res, content, "")],
        &["<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>", "<< /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 1 1 0] /N 1 >>"],
        "",
    ));
    let r = analyze(&doc);
    let c = checks(&r, &Thresholds::default());
    assert_eq!(check(&c, "fonts").status, Status::Fail, "Helvetica is not embedded");
    let colour = check(&c, "color_mode");
    assert_eq!(colour.status, Status::Warn);
    assert!(
        colour.message.contains("RGB") && colour.message.contains("PANTONE 185 C") && colour.message.contains("Registration"),
        "{}",
        colour.message
    );
    assert!(colour.message.contains("CutContour") && colour.message.contains("Layer produksi"), "cut lines are a production layer, not an ink");
    assert_eq!(r.inks.spots.len(), 2);
    assert_eq!(check(&c, "rich_black_text").status, Status::Warn);
    assert_eq!(check(&c, "thin_lines").status, Status::Warn);
    assert_eq!(check(&c, "ink_coverage").status, Status::Warn, "{:?}", r.pages);
    let t = check(&c, "transparency_overprint");
    assert!(t.message.contains("Transparansi") && t.message.contains("Overprint"), "{}", t.message);
    assert_eq!(check(&c, "bleed").status, Status::Warn, "no TrimBox");
    assert_eq!(overall(&c), Status::Fail);
}

#[test]
fn image_resolution_is_measured_where_the_image_is_placed() {
    // One 300 x 300 px image placed at 2 x 2 in (150 ppi, fail) and at 1 x 1 in (300 ppi, pass);
    // an external image (/F) is reported as missing.
    let res = "<< /XObject << /Im 3 0 R /Ext 4 0 R >> >>";
    let content = "q 144 0 0 144 0 0 cm /Im Do Q q 72 0 0 72 200 200 cm /Im Do Q q 72 0 0 72 300 300 cm /Ext Do Q";
    let img =
        "<< /Type /XObject /Subtype /Image /Width 300 /Height 300 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 3 >>\nstream\nabc\nendstream";
    let ext = "<< /Type /XObject /Subtype /Image /Width 300 /Height 300 /ColorSpace /DeviceCMYK /BitsPerComponent 8 /F (photo.tif) /Length 0 >>\nstream\n\nendstream";
    let doc = open(build(&[(res, content, "")], &[img, ext], ""));
    let r = analyze(&doc);
    assert_eq!(r.images.len(), 3);
    let ppis: Vec<f64> = r.images.iter().map(|i| i.ppi.round()).collect();
    assert_eq!(&ppis[..2], &[150.0, 300.0]);
    let c = checks(&r, &Thresholds::default());
    assert_eq!(check(&c, "images_dpi").status, Status::Fail);
    let m = check(&c, "images_missing");
    assert_eq!(m.status, Status::Fail);
    assert!(m.message.contains("eksternal"), "{}", m.message);
    assert!(r.inks.rgb);
}

#[test]
fn forms_are_followed_with_their_matrix_and_cycles_stop() {
    // A form that draws an image at 1 x 1 unit, scaled by the form matrix to 1 x 1 in, and that
    // (wrongly) names itself again inside: the walk must stop, not loop.
    let res = "<< /XObject << /Fm 3 0 R >> >>";
    let form = "<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Matrix [72 0 0 72 0 0] /Resources << /XObject << /Im 4 0 R /Fm 3 0 R >> >> /Length 13 >>\nstream\n/Im Do /Fm Do\nendstream";
    let img =
        "<< /Type /XObject /Subtype /Image /Width 600 /Height 600 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 3 >>\nstream\nabc\nendstream";
    let doc = open(build(&[(res, "/Fm Do", "")], &[form, img], ""));
    let r = analyze(&doc);
    assert_eq!(r.images.len(), 1);
    assert_eq!(r.images.first().map(|i| i.ppi.round()), Some(600.0));
}

#[test]
fn an_output_intent_is_read() {
    let doc = open(build(&[("<< >>", "", "")], &[], "/OutputIntents [<< /Type /OutputIntent /S /GTS_PDFX /OutputConditionIdentifier (FOGRA39) >>]"));
    let r = analyze(&doc);
    assert_eq!(r.output_intent.as_deref(), Some("FOGRA39"));
}

#[test]
fn broken_input_never_panics() {
    for content in [
        "q q q Q",
        "1 0 0 rg f f f",
        "/X Do /Y gs /Z cs 1 scn",
        "BT /F9 Tf (x) Tj ET",
        "0 0 0 cm 1 w 0 0 m 1 1 l S",
        "BI /W 10 /H 10 /CS /G /BPC 8 ID xxxxxxxxxx EI",
    ] {
        let doc = open(build(&[("<< /XObject << /X 99 0 R >> >>", content, "")], &[], ""));
        let r = analyze(&doc);
        let _ = checks(&r, &Thresholds::default());
    }
}

#[test]
fn process_images_count_as_inks_but_not_as_ink_totals() {
    let res = "<< /XObject << /Im 3 0 R >> >>";
    let img =
        "<< /Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceCMYK /BitsPerComponent 8 /Length 3 >>\nstream\nabc\nendstream";
    let doc = open(build(&[(res, "q 100 0 0 100 0 0 cm /Im Do Q", "")], &[img], ""));
    let r = analyze(&doc);
    assert!(r.inks.cyan && r.inks.black);
    assert_eq!(r.pages.first().map(|p| p.max_tac), Some(0.0));
}

#[test]
fn a_font_that_is_selected_but_never_shows_text_is_not_a_problem() {
    // After outlining, `Tf` may remain outside text objects with nothing drawn in that font.
    let doc = open(build(
        &[("<< /Font << /F1 3 0 R >> >>", "/F1 12 Tf 0 0 10 10 re f", "")],
        &["<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"],
        "",
    ));
    let r = analyze(&doc);
    assert!(r.fonts.is_empty(), "{:?}", r.fonts);
}

#[test]
fn ink_total_inside_cmyk_images_is_measured() {
    // 2 x 2 px CMYK: one pixel at 400 %, three at 0. With a [1 0 ...] Decode the same bytes read
    // as three pixels at 400 % and one at 0.
    let res = "<< /XObject << /Im 3 0 R /Inv 4 0 R >> >>";
    let hex = "FFFFFFFF000000000000000000000000>";
    let img = |decode: &str| {
        format!(
            "<< /Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceCMYK /BitsPerComponent 8 {decode} /Filter /ASCIIHexDecode /Length {} >>\nstream\n{hex}\nendstream",
            hex.len()
        )
    };
    let (plain, inv) = (img(""), img("/Decode [1 0 1 0 1 0 1 0]"));
    let doc = open(build(&[(res, "q 100 0 0 100 0 0 cm /Im Do Q q 100 0 0 100 0 200 cm /Inv Do Q", "")], &[&plain, &inv], ""));
    let r = analyze(&doc);
    let tacs: Vec<_> = r.images.iter().map(|i| i.tac).collect();
    assert_eq!(tacs, vec![Some(ImageTac { max: 400.0, over: 0.25 }), Some(ImageTac { max: 400.0, over: 0.75 })]);
    let c = checks(&r, &Thresholds::default());
    let tac = check(&c, "ink_coverage");
    assert_eq!(tac.status, Status::Warn, "{tac:?}");
    assert!(tac.message.contains("2 gambar"), "{}", tac.message);
}

#[test]
fn a_cmyk_image_under_the_limit_passes() {
    let res = "<< /XObject << /Im 3 0 R >> >>";
    let hex = "40404040>";
    let img = format!(
        "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceCMYK /BitsPerComponent 8 /Filter /ASCIIHexDecode /Length {} >>\nstream\n{hex}\nendstream",
        hex.len()
    );
    let doc = open(build(&[(res, "q 100 0 0 100 0 0 cm /Im Do Q", "")], &[&img], ""));
    let c = checks(&analyze(&doc), &Thresholds::default());
    let tac = check(&c, "ink_coverage");
    assert_eq!(tac.status, Status::Pass, "{tac:?}");
}
