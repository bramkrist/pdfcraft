//! Crop tool, Set Page Boxes and Duplicate pages in the real shell (egui_kittest).

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::{Dialog, PdfCraftApp, QuickTool};

const FIXTURE: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 300 400] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R >> endobj
4 0 obj << /Type /Page /Parent 2 0 R >> endobj
trailer << /Root 1 0 R >>
%%EOF";

fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("doc.pdf", None, FIXTURE.to_vec()).unwrap();
        app.set_option("left", "closed").unwrap();
        app.set_option("zoom", "150").unwrap();
        app
    });
    h.run_steps(6);
    h
}

fn size(h: &Harness<'static, PdfCraftApp>, page: usize) -> (f32, f32) {
    let s = h.state();
    let p = &s.session.get(s.views[0].id).unwrap().info.pages[page];
    (p.width, p.height)
}

#[test]
fn the_crop_tool_crops_to_the_dragged_rectangle_then_returns_to_select() {
    let mut h = harness();
    assert!(h.state_mut().execute("page.crop"));
    assert_eq!(h.state().quick_tool, QuickTool::Crop);
    h.run_steps(2);
    let r = h.state().views[0].page_screen_rect(0).unwrap();
    // The middle half of the page in both directions.
    let (a, b) = (r.min + r.size() * 0.25, r.min + r.size() * 0.75);
    h.hover_at(a);
    h.run_steps(1);
    h.drag_at(a);
    h.run_steps(1);
    for k in 1..=4 {
        h.hover_at(a + (b - a) * (k as f32 / 4.0));
        h.run_steps(1);
    }
    h.drop_at(b);
    h.run_steps(4);
    let (w, hh) = size(&h, 0);
    assert!((w - 150.0).abs() < 2.0 && (hh - 200.0).abs() < 2.0, "{w}×{hh}");
    assert_eq!(size(&h, 1), (300.0, 400.0));
    assert_eq!(h.state().quick_tool, QuickTool::Select);
    assert_eq!(h.state().session.get(h.state().views[0].id).unwrap().can_undo(), Some("Crop page"));
}

#[test]
fn set_page_boxes_dialog_applies_margins_to_the_chosen_pages() {
    let mut h = harness();
    assert!(h.state_mut().execute("page.boxes"));
    h.run_steps(3);
    assert_eq!(h.state().dialog, Some(Dialog::PageBoxes));
    h.get_by_label("Set Page Boxes");
    // One inch off every side (the units default to inches; margins are stored in points).
    h.state_mut().boxes_draft.margins = [72.0; 4];
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert_eq!(h.state().dialog, None);
    assert_eq!(size(&h, 0), (156.0, 256.0));
    assert_eq!(size(&h, 1), (156.0, 256.0), "All pages by default");
    // Set to zero resets the box.
    h.state_mut().execute("page.boxes");
    h.run_steps(3);
    h.get_by_label("Set to zero").click();
    h.run_steps(1);
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert_eq!(size(&h, 0), (300.0, 400.0));
}

#[test]
fn duplicate_pages_copies_the_current_page() {
    let mut h = harness();
    assert!(h.state_mut().execute("page.duplicate"));
    h.run_steps(3);
    let s = h.state();
    assert_eq!(s.session.get(s.views[0].id).unwrap().info.pages.len(), 3);
}

#[test]
fn repeat_pages_dialog_makes_the_chosen_number_of_copies_of_each_page() {
    let mut h = harness();
    assert!(h.state_mut().execute("page.repeat"));
    h.run_steps(3);
    assert_eq!(h.state().dialog, Some(Dialog::RepeatPages));
    h.get_by_label("Repeat pages");
    // Unchanged counts: nothing to apply yet, so Apply is disabled.
    assert_eq!(h.state().repeat_draft.counts, vec![1, 1]);
    h.state_mut().repeat_draft.counts = vec![10, 12];
    h.run_steps(2);
    h.get_by_label("2 pages become 22.");
    h.get_by_label("Apply").click();
    h.run_steps(4);
    assert_eq!(h.state().dialog, None);
    let s = h.state();
    assert_eq!(s.session.get(s.views[0].id).unwrap().info.pages.len(), 22);
    // One undo brings the two pages back, and the dialog starts from one copy of each again.
    assert!(h.state_mut().execute("edit.undo"));
    h.run_steps(2);
    let s = h.state();
    assert_eq!(s.session.get(s.views[0].id).unwrap().info.pages.len(), 2);
    assert!(h.state_mut().execute("page.repeat"));
    h.run_steps(3);
    assert_eq!(h.state().repeat_draft.counts, vec![1, 1]);
}

#[test]
fn repeat_pages_dialog_refuses_to_remove_every_page() {
    let mut h = harness();
    assert!(h.state_mut().execute("page.repeat"));
    h.run_steps(3);
    h.state_mut().repeat_draft.counts = vec![0, 0];
    h.run_steps(2);
    h.get_by_label("Every page would be removed.");
    h.get_by_label("Apply").click();
    h.run_steps(3);
    let s = h.state();
    assert_eq!(s.session.get(s.views[0].id).unwrap().info.pages.len(), 2, "nothing applied");
}

#[test]
fn add_bleed_dialog_opens_a_new_document_with_the_pages_grown_by_the_bleed() {
    let mut h = harness();
    assert!(h.state_mut().execute("impose.add_bleed"));
    h.run_steps(3);
    assert_eq!(h.state().dialog, Some(Dialog::AddBleed));
    h.get_by_label("Add bleed");
    // 5 mm all round.
    h.state_mut().bleed_draft.amount = 5.0 * 72.0 / 25.4;
    h.run_steps(2);
    h.get_by_label("Finish").click();
    h.run_steps(6);
    assert_eq!(h.state().dialog, None);
    let s = h.state();
    assert_eq!(s.views.len(), 2, "the bleed copy opens beside the original");
    let made = s.session.get(s.views[s.views.len() - 1].id).unwrap();
    assert!(made.name.ends_with("_BLEED.pdf"), "{}", made.name);
    assert_eq!(made.info.pages.len(), 2);
    let grown = 10.0 * 72.0 / 25.4;
    assert!((f64::from(made.info.pages[0].width) - (300.0 + grown)).abs() < 0.5, "{}", made.info.pages[0].width);
    assert!((f64::from(made.info.pages[0].height) - (400.0 + grown)).abs() < 0.5, "{}", made.info.pages[0].height);
}

#[test]
fn the_crop_tool_leaves_the_page_grid_so_a_page_can_be_cropped() {
    let mut h = harness();
    // The Organize panel's thumbnail grid is on screen for this view: choosing
    // "Crop pages" from it must leave the grid so the canvas handles a drag on
    // a page instead of returning early to organize_grid.
    h.state_mut().views[0].organize = true;
    assert!(h.state_mut().execute("page.crop"));
    assert_eq!(h.state().quick_tool, QuickTool::Crop);
    assert!(!h.state().views[0].organize, "\"page.crop\" must clear view.organize");
    h.run_steps(2);
    let r = h.state().views[0].page_screen_rect(0).unwrap();
    // The middle half of the page in both directions.
    let (a, b) = (r.min + r.size() * 0.25, r.min + r.size() * 0.75);
    h.hover_at(a);
    h.run_steps(1);
    h.drag_at(a);
    h.run_steps(1);
    for k in 1..=4 {
        h.hover_at(a + (b - a) * (k as f32 / 4.0));
        h.run_steps(1);
    }
    h.drop_at(b);
    h.run_steps(4);
    let (w, hh) = size(&h, 0);
    assert!((w - 150.0).abs() < 2.0 && (hh - 200.0).abs() < 2.0, "{w}×{hh}");
    assert_eq!(size(&h, 1), (300.0, 400.0));
}

#[test]
fn remove_white_margins_crops_pages_to_what_is_drawn_and_leaves_blank_pages() {
    let content = "0 0 0 rg 100 150 50 40 re f";
    let pdf = format!(
        "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 300 400] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /Contents 5 0 R >> endobj
4 0 obj << /Type /Page /Parent 2 0 R >> endobj
5 0 obj << /Length {} >> stream
{content}
endstream endobj
trailer << /Root 1 0 R >>
%%EOF",
        content.len()
    );
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("doc.pdf", None, pdf.clone().into_bytes()).unwrap();
        app.set_option("left", "closed").unwrap();
        app
    });
    h.run_steps(6);
    assert!(h.state_mut().execute("page.boxes"));
    h.run_steps(3);
    h.get_by_label("Remove white margins").click();
    h.run_steps(2);
    assert!(h.state().boxes_draft.white);
    h.get_by_label("OK").click();
    h.run_steps(4);
    let (w, hh) = size(&h, 0);
    // The square is 50 x 40; a point of anti-aliasing is kept on every side.
    assert!((50.0..=54.0).contains(&w) && (40.0..=44.0).contains(&hh), "{w}×{hh}");
    assert_eq!(size(&h, 1), (300.0, 400.0), "a blank page has no content to crop to");
    // One undo gives page 1 back.
    assert!(h.state_mut().execute("edit.undo"));
    h.run_steps(2);
    assert_eq!(size(&h, 0), (300.0, 400.0));
}
