//! End to end: a real PDF in, a laid-out PDF out, checked by rendering it.

use std::sync::Arc;

use pdfcraft_cos::Document;
use pdfcraft_impose::{Duplex, Margins, StepRepeat, impose_document};
use pdfcraft_print::Content;
use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, RenderedPage, RequestKind};

/// A PDF with one page per entry of `pages`: (width, height, content stream).
fn pdf(pages: &[(u32, u32, &str)]) -> Vec<u8> {
    let mut objs: Vec<String> = vec!["<< /Type /Catalog /Pages 2 0 R >>".into()];
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 3 + 2 * i)).collect();
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()));
    for (i, (w, h, body)) in pages.iter().enumerate() {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents {} 0 R >>", 4 + 2 * i));
        objs.push(format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()));
    }
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let x = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn render(bytes: &[u8], page: usize) -> RenderedPage {
    let r = PageRenderer::new(Arc::new(bytes.to_vec()), RenderConfig::default()).render(RenderRequest {
        page,
        kind: RequestKind::Pixels,
        tile: None,
        scale: 1.0,
        tag: 0,
        preview: false,
    });
    assert_eq!(r.error, None);
    r
}

/// Red, green, blue of the pixel at image coordinates (x from the left, y from the top).
fn rgb(p: &RenderedPage, x: usize, y: usize) -> [u8; 3] {
    let o = (y * p.width as usize + x) * 4;
    [p.rgba[o], p.rgba[o + 1], p.rgba[o + 2]]
}

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const WHITE: [u8; 3] = [255, 255, 255];

#[test]
fn step_and_repeat_puts_the_card_in_every_cell_and_nothing_in_the_gutter() {
    let src = pdf(&[(100, 60, "1 0 0 rg 0 0 100 60 re f")]);
    let doc = Document::open(Arc::new(src)).expect("open");
    let mut spec = StepRepeat::new((300.0, 200.0));
    spec.margins = Margins::uniform(0.0);
    spec.gutter = (20.0, 20.0);
    let out = impose_document(&doc, &spec, Content::Document).expect("impose");
    let page = render(&out, 0);
    assert_eq!((page.width, page.height), (300, 200));
    // 2 x 2 cells: columns 40..140 and 160..260, rows 30..90 and 110..170 from the top.
    for (x, y) in [(90, 60), (210, 60), (90, 140), (210, 140)] {
        assert_eq!(rgb(&page, x, y), RED, "cell at {x},{y}");
    }
    for (x, y) in [(150, 60), (150, 140), (90, 100), (210, 100), (10, 10), (290, 190)] {
        assert_eq!(rgb(&page, x, y), WHITE, "gutter or margin at {x},{y}");
    }
}

#[test]
fn the_back_page_is_printed_behind_the_front_cell_when_the_sheet_is_turned() {
    // Front: red; back: blue. 1 x 2 cells side by side on a landscape sheet.
    let src = pdf(&[(100, 60, "1 0 0 rg 0 0 100 60 re f"), (100, 60, "0 0 1 rg 0 0 100 60 re f")]);
    let doc = Document::open(Arc::new(src)).expect("open");
    let mut spec = StepRepeat::new((300.0, 100.0));
    spec.margins = Margins::uniform(0.0);
    spec.cell = Some((100.0, 60.0));
    spec.cols = Some(2);
    spec.rows = Some(1);
    spec.gutter = (20.0, 0.0);
    spec.duplex = Some(Duplex::LongEdge);
    let out = impose_document(&doc, &spec, Content::Document).expect("impose");
    let front = render(&out, 0);
    let back = render(&out, 1);
    // Front cells at x 40..140 and 160..260; the back cells are the same two spans mirrored.
    assert_eq!(rgb(&front, 90, 50), RED);
    assert_eq!(rgb(&front, 210, 50), RED);
    assert_eq!(rgb(&back, 90, 50), BLUE);
    assert_eq!(rgb(&back, 210, 50), BLUE);
    assert_eq!(rgb(&back, 150, 50), WHITE);
}
