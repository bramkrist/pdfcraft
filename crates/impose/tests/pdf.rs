//! End to end: a real PDF in, a laid-out PDF out, checked by rendering it.

use std::sync::Arc;

use pdfcraft_cos::Document;
use pdfcraft_impose::{Duplex, Manual, ManualPage, Margins, StepRepeat, impose_document, manual_document};
use pdfcraft_print::Content;
use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, RenderedPage, RequestKind};

/// A PDF with one page per entry of `pages`: (width, height, content stream).
fn pdf(pages: &[(u32, u32, &str)]) -> Vec<u8> {
    pdf_with_root(pages, "")
}

/// Like [`pdf`], with `root_extra` added to the catalog.
fn pdf_with_root(pages: &[(u32, u32, &str)], root_extra: &str) -> Vec<u8> {
    let mut objs: Vec<String> = vec![format!("<< /Type /Catalog /Pages 2 0 R {root_extra} >>")];
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

/// The catalog entry `key` of an imposed file, written back out as text for comparison.
fn catalog_entry(bytes: &[u8], key: &[u8]) -> Option<String> {
    let doc = Document::open(Arc::new(bytes.to_vec())).expect("open output");
    let root = doc.trailer().get(b"Root")?;
    let entry = doc.resolve(root).as_dict()?.get(key)?.clone();
    Some(format!("{:?}", doc.resolve(&entry)))
}

#[test]
fn the_output_intent_of_the_source_survives_imposition() {
    let intent = "/OutputIntents [<< /Type /OutputIntent /S /GTS_PDFX /OutputConditionIdentifier (CGATS TR 001) >>]";
    let src = pdf_with_root(&[(100, 60, "0 1 1 0 k 0 0 100 60 re f")], intent);
    let doc = Document::open(Arc::new(src.clone())).expect("open");
    let out = impose_document(&doc, &StepRepeat::new((300.0, 200.0)), Content::Document).expect("impose");
    let before = catalog_entry(&src, b"OutputIntents").expect("the fixture has an output intent");
    let kept = catalog_entry(&out, b"OutputIntents").expect("the catalog lost its output intent");
    assert_eq!(kept, before, "the output condition was changed");
    // And a source without one gains none.
    let plain = Document::open(Arc::new(pdf(&[(100, 60, "0 0 100 60 re f")]))).expect("open");
    let out = impose_document(&plain, &StepRepeat::new((300.0, 200.0)), Content::Document).expect("impose");
    assert!(catalog_entry(&out, b"OutputIntents").is_none());
}

#[test]
fn a_saddle_stitched_booklet_puts_each_page_where_the_fold_needs_it() {
    const GREEN: [u8; 3] = [0, 255, 0];
    // Four pages, one colour each: red, green, blue, black.
    let src = pdf(&[
        (100, 60, "1 0 0 rg 0 0 100 60 re f"),
        (100, 60, "0 1 0 rg 0 0 100 60 re f"),
        (100, 60, "0 0 1 rg 0 0 100 60 re f"),
        (100, 60, "0 0 0 rg 0 0 100 60 re f"),
    ]);
    let doc = Document::open(Arc::new(src)).expect("open");
    let out = pdfcraft_impose::booklet_document(&doc, &pdfcraft_impose::Booklet::new(), Content::Document).expect("booklet");
    let (front, back) = (render(&out, 0), render(&out, 1));
    assert_eq!((front.width, front.height), (200, 60));
    // Outside of the sheet: page 4 | page 1. Inside: page 2 | page 3.
    assert_eq!((rgb(&front, 50, 30), rgb(&front, 150, 30)), ([0, 0, 0], RED));
    assert_eq!((rgb(&back, 50, 30), rgb(&back, 150, 30)), (GREEN, BLUE));
}

/// The first page's entry `key`, as text for comparison.
fn first_page_entry(bytes: &[u8], key: &[u8]) -> Option<String> {
    let doc = Document::open(Arc::new(bytes.to_vec())).expect("open output");
    let root = doc.resolve(doc.trailer().get(b"Root")?).as_dict()?.clone();
    let pages = doc.resolve(root.get(b"Pages")?).as_dict()?.clone();
    let kids = doc.resolve(pages.get(b"Kids")?).as_array()?.to_vec();
    let page = doc.resolve(kids.first()?).as_dict()?.clone();
    Some(format!("{:?}", doc.resolve(page.get(key)?)))
}

#[test]
fn a_mirrored_bleed_continues_the_artwork_as_its_own_reflection_and_sets_the_trim_box() {
    const GREEN: [u8; 3] = [0, 255, 0];
    // Left edge: red for the first 5 pt, then green for 5 more; the rest is white.
    let src = pdf(&[(100, 60, "1 0 0 rg 0 0 5 60 re f 0 1 0 rg 5 0 5 60 re f")]);
    let doc = Document::open(Arc::new(src)).expect("open");
    let spec = pdfcraft_impose::AddBleed::uniform(10.0, pdfcraft_impose::BleedMethod::Mirror);
    let out = pdfcraft_impose::bleed_document(&doc, &spec, Content::Document).expect("bleed");
    let page = render(&out, 0);
    assert_eq!((page.width, page.height), (120, 80));
    // Sheet x 10..15 is the page's red, 15..20 its green; the bleed reflects them across the edge.
    for (x, want) in [(2, GREEN), (7, RED), (12, RED), (17, GREEN), (60, WHITE), (115, WHITE)] {
        assert_eq!(rgb(&page, x, 40), want, "x = {x}");
    }
    // The corners reflect across both edges and the strips above and below carry the same colours.
    assert_eq!(rgb(&page, 2, 2), GREEN);
    assert_eq!(rgb(&page, 12, 78), RED);
    // TrimBox at the old edge, BleedBox the whole page.
    // Whole numbers are written as integers.
    let boxed = |v: [i64; 4]| format!("{:?}", pdfcraft_cos::Object::Array(v.iter().map(|x| pdfcraft_cos::Object::Int(*x)).collect()));
    assert_eq!(first_page_entry(&out, b"TrimBox"), Some(boxed([10, 10, 110, 70])));
    assert_eq!(first_page_entry(&out, b"BleedBox"), Some(boxed([0, 0, 120, 80])));
}

#[test]
fn registration_marks_render_black_and_ink_every_plate() {
    let src = pdf(&[(100, 100, "1 1 1 rg 0 0 100 100 re f")]);
    let doc = Document::open(Arc::new(src)).expect("open");
    let mut spec = StepRepeat::new((200.0, 200.0));
    spec.cols = Some(1);
    spec.rows = Some(1);
    spec.marks = Some(pdfcraft_impose::Marks { length: 20.0, offset: 5.0, width: 2.0, registration: true, ..Default::default() });
    let out = impose_document(&doc, &spec, Content::Document).expect("impose");
    // Page at 50..150: the bottom-left horizontal mark runs from x 25 to 45 on y 50.
    let sheet = render(&out, 0);
    assert_eq!(rgb(&sheet, 35, 150), [0, 0, 0], "mark in registration colour shows as black");
    // The colour space is a Separation named All.
    let page = first_page_entry(&out, b"Resources").expect("resources");
    let name = |n: &str| format!("Name({:?})", n.as_bytes());
    assert!(page.contains(&name("Separation")) && page.contains(&name("All")), "{page}");
}

#[test]
fn a_turned_page_renders_turned_not_clipped() {
    // 100 x 40 pt page: red left half, blue right half. On a 60 x 100 sheet it fits only turned.
    let src = pdf(&[(100, 40, "1 0 0 rg 0 0 50 40 re f 0 0 1 rg 50 0 50 40 re f")]);
    let doc = Document::open(Arc::new(src)).expect("open");
    let mut spec = StepRepeat::new((40.0, 100.0));
    spec.rotate = true;
    let out = impose_document(&doc, &spec, Content::Document).expect("impose");
    let sheet = render(&out, 0);
    assert_eq!((sheet.width, sheet.height), (40, 100));
    // Turned anticlockwise: the page's left (red) is now at the bottom, its right (blue) on top.
    assert_eq!(rgb(&sheet, 20, 80), RED, "bottom of the sheet");
    assert_eq!(rgb(&sheet, 20, 20), BLUE, "top of the sheet");
}

#[test]
fn manual_pages_render_where_and_how_they_were_placed() {
    // 100 x 40 pt page: red left half, blue right half. Sheet 200 x 200: upright at the bottom
    // left, turned 180 at the top right, a quarter turn clockwise in between.
    let src = pdf(&[(100, 40, "1 0 0 rg 0 0 50 40 re f 0 0 1 rg 50 0 50 40 re f")]);
    let doc = Document::open(Arc::new(src)).expect("open");
    let spec = Manual {
        sheet: (200.0, 200.0),
        bleed: 0.0,
        marks: None,
        sheets: vec![vec![
            ManualPage { page: 0, at: (0.0, 0.0), turns: 0 },
            ManualPage { page: 0, at: (100.0, 160.0), turns: 2 },
            ManualPage { page: 0, at: (110.0, 30.0), turns: 3 },
        ]],
    };
    let out = manual_document(&doc, &spec, Content::Document).expect("impose");
    let s = render(&out, 0);
    assert_eq!((s.width, s.height), (200, 200));
    // Image y runs down: sheet y = 200 - image y.
    assert_eq!(rgb(&s, 20, 180), RED, "upright: left half red");
    assert_eq!(rgb(&s, 80, 180), BLUE, "upright: right half blue");
    assert_eq!(rgb(&s, 120, 20), BLUE, "turned 180: its right half now on the left");
    assert_eq!(rgb(&s, 180, 20), RED);
    // Clockwise: the page's left (red) goes to the top, its right (blue) to the bottom.
    assert_eq!(rgb(&s, 130, 200 - 120), RED, "clockwise: red on top");
    assert_eq!(rgb(&s, 130, 200 - 40), BLUE, "clockwise: blue at the bottom");
    assert_eq!(rgb(&s, 60, 100), WHITE, "nothing placed there");
}

#[test]
fn two_booklets_on_one_sheet_repeat_every_side_of_the_booklet_and_keep_the_big_sheet_portrait() {
    use pdfcraft_impose::{Booklet, TwoUp, booklet_two_up_document};
    const MM: f64 = 72.0 / 25.4;
    // Eight A5 pages: a saddle-stitched booklet of 2 sheets = 4 sides of 296 x 210 mm.
    let pages: Vec<(u32, u32, &str)> = (0..8).map(|_| (420, 595, "1 0 0 rg 0 0 420 595 re f")).collect();
    let src = Document::open(Arc::new(pdf(&pages))).unwrap();
    let two = TwoUp::new((450.0 * MM, 320.0 * MM));
    let out = booklet_two_up_document(&src, &Booklet::new(), &two, Content::DocumentAndMarkups).unwrap();
    let doc = Document::open(Arc::new(out.clone())).unwrap();
    let sizes = pdfcraft_print::page_sizes(&doc);
    assert_eq!(sizes.len(), 4, "one big-sheet side per booklet side");
    for (w, h) in &sizes {
        assert!((w - 320.0 * MM).abs() < 0.5 && (h - 450.0 * MM).abs() < 0.5, "{w} x {h}: stays portrait");
    }
    // Both booklets print: the top and the bottom half of the first side are both red.
    let p = render(&out, 0);
    let (x, third) = (p.width as usize / 2, p.height as usize / 4);
    assert_eq!(rgb(&p, x, third), RED);
    assert_eq!(rgb(&p, x, third * 3), RED);
}

#[test]
fn a_scaled_page_is_drawn_smaller_in_the_middle_of_its_cell() {
    // A 100 x 100 red page at 50 % on a 200 x 200 sheet, one cell fixed at 100 x 100.
    let src = Document::open(Arc::new(pdf(&[(100, 100, "1 0 0 rg 0 0 100 100 re f")]))).unwrap();
    let spec = StepRepeat { cell: Some((200.0, 200.0)), scale: 0.5, ..StepRepeat::new((200.0, 200.0)) };
    let out = impose_document(&src, &spec, Content::Document).expect("impose");
    let p = render(&out, 0);
    let at = |fx: f64, fy: f64| rgb(&p, (p.width as f64 * fx) as usize, (p.height as f64 * fy) as usize);
    assert_eq!(at(0.5, 0.5), RED, "the scaled page covers the middle");
    assert_eq!(at(0.4, 0.5), RED, "the half-size page spans 50 to 150 pt, so 40 % across is still inside it");
    assert_ne!(at(0.1, 0.5), RED, "but 10 % across is outside it: the page no longer fills its cell");
    assert_ne!(at(0.5, 0.9), RED);
}

#[test]
fn page_tools_reverse_turn_trim_and_tile_real_pages() {
    use pdfcraft_impose::{Arrange, PageTool, Shuffle, Tile, Trim, TrimShift, pages_document};
    // Page 1 red, page 2 blue with a red bottom strip; 100 x 60 pt each.
    let src = pdf(&[(100, 60, "1 0 0 rg 0 0 100 60 re f"), (100, 60, "0 0 1 rg 0 0 100 60 re f 1 0 0 rg 0 0 100 10 re f")]);
    let doc = Document::open(Arc::new(src)).expect("open");
    let rev = pages_document(&doc, &PageTool::Reverse, Content::Document).unwrap();
    assert_eq!(rgb(&render(&rev, 0), 50, 30), BLUE, "page 2 first");
    assert_eq!(rgb(&render(&rev, 1), 50, 30), RED);
    // Page 2 upside down: its red strip now at the top.
    let turned = Shuffle { arrange: Arrange::Keep, reverse_evens: false, turn_evens: true };
    let out = pages_document(&doc, &PageTool::Shuffle(turned), Content::Document).unwrap();
    let p2 = render(&out, 1);
    assert_eq!(rgb(&p2, 50, 3), RED, "strip at the top");
    assert_eq!(rgb(&p2, 50, 57), BLUE);
    // Trim 10 off the bottom of page 2: the red strip is gone and the page is 50 tall.
    let trim = TrimShift { trim: Trim::Edges([0.0, 10.0, 0.0, 0.0]), ..TrimShift::default() };
    let out = pages_document(&doc, &PageTool::TrimShift(trim), Content::Document).unwrap();
    let p2 = render(&out, 1);
    assert_eq!((p2.width, p2.height), (100, 50));
    assert_eq!(rgb(&p2, 50, 47), BLUE, "the strip was trimmed off");
    // Two tiles across: 50 x 60 each, the right one showing the page's right half.
    let out = pages_document(&doc, &PageTool::Tile(Tile { cols: 2, rows: 1, overlap: 0.0, only_wide: false }), Content::Document).unwrap();
    let back = Document::open(Arc::new(out.clone())).unwrap();
    assert_eq!(pdfcraft_print::page_sizes(&back).len(), 4);
    let t = render(&out, 3);
    assert_eq!((t.width, t.height), (50, 60));
    assert_eq!(rgb(&t, 25, 55), RED, "page 2's bottom strip");
    assert_eq!(rgb(&t, 25, 20), BLUE);
}
