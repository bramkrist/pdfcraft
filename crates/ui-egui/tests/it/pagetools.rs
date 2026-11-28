//! Imposing ▸ Shuffle even/odd pages, Reverse pages, Trim and shift, Tile pages in the real shell
//! (egui_kittest): the menu command opens the dialog on its tool, the controls are there by name,
//! and Finish opens the new document in a tab of its own while the original is unchanged.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use pdfcraft_ui_egui::{Dialog, PdfCraftApp};

const FIXTURE: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 400 300] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R >> endobj
4 0 obj << /Type /Page /Parent 2 0 R >> endobj
trailer << /Root 1 0 R >>
%%EOF";

fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("doc.pdf", None, FIXTURE.to_vec()).unwrap();
        app
    });
    h.run_steps(6);
    h
}

/// The open documents' names and page counts, in tab order.
fn tabs(h: &Harness<'static, PdfCraftApp>) -> Vec<(String, usize)> {
    let s = h.state();
    s.views.iter().filter_map(|v| s.session.get(v.id).map(|d| (d.name.clone(), d.info.pages.len()))).collect()
}

#[test]
fn tile_pages_splits_every_page_into_a_new_document() {
    let mut h = harness();
    assert!(h.state_mut().execute("impose.tile"));
    h.run_steps(3);
    assert_eq!(h.state().dialog, Some(Dialog::PageTools));
    h.get_by_label("Tile pages");
    h.get_by_label("Split only wide pages (spreads among single pages)");
    h.get_by_label("Result: 4 pages, the first 2.78 × 4.17 in");
    h.get_by_label("Finish").click();
    h.run_steps(6);
    assert_eq!(h.state().dialog, None);
    assert_eq!(tabs(&h), vec![("doc.pdf".to_owned(), 2), ("doc_TILES.pdf".to_owned(), 4)]);
}

#[test]
fn shuffle_and_trim_and_shift_open_on_their_own_tool() {
    let mut h = harness();
    assert!(h.state_mut().execute("impose.shuffle"));
    h.run_steps(3);
    h.get_by_label("Shuffle even/odd pages");
    h.get_by_label("Turn the even pages upside down").click();
    h.run_steps(2);
    assert!(h.state().page_tools_draft.shuffle.turn_evens);
    h.get_by_label("Finish").click();
    h.run_steps(6);
    assert_eq!(tabs(&h).last(), Some(&("doc_SHUFFLE.pdf".to_owned(), 2)));
    // Trim and shift: a shift wholly off the page is refused before Finish.
    h.state_mut().execute("impose.trim_shift");
    h.run_steps(3);
    h.get_by_label("Trim and shift");
    h.state_mut().page_tools_draft.shift = (5000.0, 0.0);
    h.run_steps(2);
    assert!(h.get_by_label("Finish").accesskit_node().is_disabled(), "nothing would be left on the page");
    h.state_mut().page_tools_draft.shift = (9.0, 0.0);
    h.run_steps(2);
    h.get_by_label("Finish").click();
    h.run_steps(6);
    // It works on the tab in front: the shuffled one Finish opened.
    assert_eq!(tabs(&h).last(), Some(&("doc_SHUFFLE_TRIMSHIFT.pdf".to_owned(), 2)));
    assert_eq!(tabs(&h).len(), 3);
}
