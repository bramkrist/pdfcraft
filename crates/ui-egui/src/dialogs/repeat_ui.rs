//! Repeat pages: how many copies of each page, in one go (page 1 ×10, page 2 ×12, …), so a gang
//! of artwork fills a sheet. The copies sit right after their original; 0 removes a page. The
//! work is `Edit::RepeatPages`, one undo step.

use pdfcraft_engine::{DocId, Edit};

use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, widgets};

/// The most copies of one page the dialog offers.
const MAX_PER_PAGE: usize = 9_999;

/// Dialog state: one count per page of the document it was opened on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepeatDraft {
    /// Copies of each page, including the original (1 = unchanged, 0 = remove).
    pub counts: Vec<usize>,
    /// The value the "Set all" button applies.
    pub all: usize,
    /// The document and page count `counts` belongs to; a different one re-seeds it.
    seeded: Option<(DocId, usize)>,
}

impl RepeatDraft {
    /// Start from one copy of every page of `doc`.
    pub fn seed(&mut self, doc: DocId, pages: usize) {
        if self.seeded != Some((doc, pages)) {
            self.counts = vec![1; pages];
            self.all = 1;
            self.seeded = Some((doc, pages));
        }
    }

    /// Forget the counts so the next opening starts from one copy of each page.
    pub fn reset(&mut self) {
        self.seeded = None;
    }

    /// Pages after the repeat (saturating, so a silly draft cannot overflow).
    pub fn total(&self) -> usize {
        self.counts.iter().fold(0usize, |t, &c| t.saturating_add(c))
    }

    /// Whether Apply would change anything.
    pub fn changes(&self) -> bool {
        self.counts.iter().any(|&c| c != 1)
    }

    pub fn edit(&self) -> Edit {
        Edit::RepeatPages { counts: self.counts.clone() }
    }
}

/// Draw the dialog body; returns (apply, cancel).
pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp, _t: &Tokens) -> (bool, bool) {
    let Some((_, id)) = app.active_ids() else { return (false, true) };
    let Some(doc) = app.session.get(id) else { return (false, true) };
    let pages = doc.info.pages.len();
    let d = &mut app.repeat_draft;
    d.seed(id, pages);
    ui.label(egui::RichText::new(tl!("Repeat pages")).font(theme::semibold(18.0)));
    ui.label(egui::RichText::new(tl!("Each copy goes right after its page; 0 removes the page.")).weak());
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(tl!("Set every page to"));
        ui.add(egui::DragValue::new(&mut d.all).range(0..=MAX_PER_PAGE).speed(0.2));
        if ui.button(tl!("Apply to all")).clicked() {
            let n = d.all;
            d.counts.iter_mut().for_each(|c| *c = n);
        }
    });
    ui.add_space(6.0);
    egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
        egui::Grid::new("repeat-grid").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
            for (i, c) in d.counts.iter_mut().enumerate() {
                let l = ui.label(crate::i18n::fmt(tl!("Page {n}"), &[("n", &(i + 1).to_string())]));
                ui.add(egui::DragValue::new(c).range(0..=MAX_PER_PAGE).speed(0.2).prefix("× ")).labelled_by(l.id);
                ui.end_row();
            }
        });
    });
    ui.add_space(8.0);
    let total = d.total();
    let ok = total > 0 && total <= pdfcraft_engine::MAX_REPEAT_PAGES && d.changes();
    let text = if total == 0 {
        tl!("Every page would be removed.").to_owned()
    } else if total > pdfcraft_engine::MAX_REPEAT_PAGES {
        crate::i18n::fmt(tl!("Too many pages (the limit is {max})."), &[("max", &pdfcraft_engine::MAX_REPEAT_PAGES.to_string())])
    } else {
        crate::i18n::fmt(tl!("{from} pages become {to}."), &[("from", &pages.to_string()), ("to", &total.to_string())])
    };
    ui.label(egui::RichText::new(text).weak());
    ui.add_space(8.0);
    let (mut go, mut cancel) = (false, false);
    ui.horizontal(|ui| {
        go = ui.add_enabled(ok, |ui: &mut egui::Ui| widgets::pill_button(ui, tl!("Apply"), true)).clicked();
        cancel = widgets::pill_button(ui, tl!("Cancel"), false).clicked();
    });
    (go, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_draft_changes_nothing_and_counts_every_page_once() {
        let mut d = RepeatDraft::default();
        d.seed(DocId(1), 5);
        assert_eq!(d.counts, vec![1; 5]);
        assert_eq!(d.total(), 5);
        assert!(!d.changes());
    }

    #[test]
    fn a_draft_is_kept_for_the_same_document_and_reset_for_another() {
        let mut d = RepeatDraft::default();
        d.seed(DocId(1), 3);
        d.counts[0] = 10;
        d.seed(DocId(1), 3);
        assert_eq!(d.counts[0], 10);
        d.seed(DocId(1), 4);
        assert_eq!(d.counts, vec![1; 4]);
        d.counts[1] = 7;
        d.reset();
        d.seed(DocId(1), 4);
        assert_eq!(d.counts, vec![1; 4]);
    }

    #[test]
    fn the_total_cannot_overflow() {
        let d = RepeatDraft { counts: vec![usize::MAX, usize::MAX], all: 1, seeded: None };
        assert_eq!(d.total(), usize::MAX);
    }

    #[test]
    fn the_edit_carries_the_counts() {
        let mut d = RepeatDraft::default();
        d.seed(DocId(1), 2);
        d.counts = vec![10, 12];
        assert!(d.changes());
        assert!(matches!(d.edit(), Edit::RepeatPages { counts } if counts == vec![10, 12]));
    }
}
