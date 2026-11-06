use std::sync::Arc;

use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, RenderedPage, RequestKind};

use super::*;

/// A PDF whose page shows `content` with /F1 = Inter embedded as a TrueType font (FontFile2),
/// /F2 = Helvetica not embedded, and a form /Fm that shows text too.
fn pdf_with_text(content: &str) -> Vec<u8> {
    let font = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
    let widths: Vec<String> = (32..=126).map(|_| "600".to_string()).collect();
    let form_content = "BT /F1 20 Tf 0 0 Td (Form text) Tj ET";
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 300] /Resources << /Font << /F1 5 0 R /F2 8 0 R >> /XObject << /Fm 9 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).into_bytes(),
        format!("<< /Type /Font /Subtype /TrueType /BaseFont /AAAAAA+Inter /FirstChar 32 /LastChar 126 /Widths [{}] /Encoding /WinAnsiEncoding /FontDescriptor 6 0 R >>", widths.join(" ")).into_bytes(),
        b"<< /Type /FontDescriptor /FontName /AAAAAA+Inter /Flags 32 /FontBBox [-500 -300 1500 1000] /ItalicAngle 0 /Ascent 900 /Descent -200 /CapHeight 700 /StemV 80 /FontFile2 7 0 R >>".to_vec(),
    ];
    let mut ff = format!("<< /Length {} /Length1 {} >>\nstream\n", font.len(), font.len()).into_bytes();
    ff.extend_from_slice(font);
    ff.extend_from_slice(b"\nendstream");
    objs.push(ff);
    objs.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    objs.push(format!("<< /Type /XObject /Subtype /Form /BBox [0 0 400 300] /Matrix [1 0 0 1 20 20] /Resources << /Font << /F1 5 0 R >> >> /Length {} >>\nstream\n{form_content}\nendstream", form_content.len()).into_bytes());
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn render(bytes: &[u8]) -> RenderedPage {
    let r = PageRenderer::new(Arc::new(bytes.to_vec()), RenderConfig::default()).render(RenderRequest {
        page: 0,
        kind: RequestKind::Pixels,
        tile: None,
        scale: 2.0,
        tag: 0,
        preview: false,
    });
    assert_eq!(r.error, None);
    r
}

/// Share of pixels that differ visibly between two renderings of the same size.
fn difference(a: &RenderedPage, b: &RenderedPage) -> f64 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let px = |p: &RenderedPage| p.rgba.chunks(4).map(|c| [c[0], c[1], c[2]]).collect::<Vec<_>>();
    let (pa, pb) = (px(a), px(b));
    let differ = pa.iter().zip(&pb).filter(|(x, y)| x.iter().zip(y.iter()).any(|(m, n)| m.abs_diff(*n) > 48)).count();
    differ as f64 / pa.len().max(1) as f64
}

fn inked(p: &RenderedPage) -> usize {
    p.rgba.chunks(4).filter(|c| c[0] < 200 || c[1] < 200 || c[2] < 200).count()
}

fn content_of(bytes: &[u8]) -> String {
    let doc = Document::open(Arc::new(bytes.to_vec())).expect("open");
    let pages = pdfcraft_model::pages(&doc);
    let page = pages.first().expect("a page");
    String::from_utf8_lossy(&page_content(&doc, &page.dict)).into_owned()
}

#[test]
fn embedded_text_becomes_paths_that_look_the_same() {
    let src = pdf_with_text(
        "0 0 1 0 k BT /F1 36 Tf 20 200 Td (Hello PIXELINDIE) Tj 0 -50 Td 1 0 0 rg [(Kerned) -500 (TJ)] TJ ET q 2 0 0 2 0 0 cm BT /F1 12 Tf 1 Tr 0 0 1 RG 10 20 Td (stroke) Tj ET Q /Fm Do",
    );
    let (out, report) = outline_text(src.clone()).expect("outline");
    assert_eq!(report.kept, vec![], "{report:?}");
    assert_eq!(report.outlined, 3, "two page blocks and the form's");
    let before = render(&src);
    let after = render(&out);
    assert!(inked(&before) > 1000, "the fixture shows text");
    let d = difference(&before, &after);
    assert!(d < 0.002, "outlined page differs in {:.3}% of pixels", d * 100.0);
    let text = content_of(&out);
    assert!(!text.contains("Tj") && !text.contains("TJ") && !text.contains("BT"), "no text left on the page:\n{text}");
    assert!(text.contains("0 0 1 0 k") && text.contains("1 0 0 rg") && text.contains("0 0 1 RG"), "colours stay where they were");
    // The form was rewritten too, once.
    let doc = Document::open(Arc::new(out.clone())).expect("open");
    let form = doc.resolve(&Object::Ref(ObjRef { num: 9, generation: 0 }));
    let Object::Stream(s) = &*form else { panic!("form") };
    let form_text = String::from_utf8_lossy(&s.decoded().expect("decode")).into_owned();
    assert!(!form_text.contains("Tj") && form_text.contains(" Do Q"), "{form_text}");
    // The original bytes are all still there: the save was incremental.
    assert!(out.starts_with(&src[..src.len() - 10]));
}

#[test]
fn text_in_a_font_that_is_not_embedded_stays_text_and_is_reported() {
    let src = pdf_with_text("BT /F2 24 Tf 20 200 Td (Helvetica) Tj ET BT /F1 24 Tf 20 100 Td (Inter) Tj ET");
    let (out, report) = outline_text(src).expect("outline");
    assert_eq!(report.outlined, 1);
    assert!(report.kept.iter().any(|k| k.reason.contains("tidak tertanam")), "{report:?}");
    let text = content_of(&out);
    assert!(text.contains("(Helvetica) Tj") && !text.contains("(Inter) Tj"), "{text}");
}

#[test]
fn clipping_and_invisible_text() {
    // Invisible text (OCR layers) disappears as paths would show nothing; clipping text is kept.
    let src = pdf_with_text("BT /F1 24 Tf 3 Tr 20 200 Td (ocr) Tj ET BT /F1 24 Tf 7 Tr 20 100 Td (clip) Tj ET 0 0 400 300 re f");
    let (out, report) = outline_text(src).expect("outline");
    assert!(report.kept.iter().any(|k| k.reason.contains("clipping")), "{report:?}");
    let text = content_of(&out);
    assert!(text.contains("(clip) Tj") && !text.contains("(ocr) Tj"), "{text}");
}

#[test]
fn a_file_without_text_comes_back_unchanged() {
    let src = pdf_with_text("0 0 100 100 re f");
    let (out, report) = outline_text(src.clone()).expect("outline");
    assert_eq!(out, src);
    assert_eq!(report.outlined, 0);
}

#[test]
fn garbage_is_an_error_not_a_panic() {
    assert!(outline_text(b"not a pdf".to_vec()).is_err());
}
