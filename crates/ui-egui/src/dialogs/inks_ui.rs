//! Imposing ▸ Ink manager: the spot colours a document uses, each kept, printed in process
//! (its alternate colour, taken from the file) or printed on another spot's plate. Apply makes a
//! new document through the session (`Session::manage_inks_pdf`); the original is never changed.
//!
//! The choices are plain data ([`InksDraft`]), so what they turn into is tested without a window.

use std::collections::BTreeMap;

use egui::{Align, Layout};
use pdfcraft_engine::inks::InkAction;
use pdfcraft_engine::{DocId, SpotInk};

use crate::theme;
use crate::{PdfCraftApp, widgets};

/// What is to be done with one spot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Choice {
    /// Leave it as a spot.
    #[default]
    Keep,
    /// Print it in its alternate (process) colour.
    Process,
    /// Print it on the plate of the spot at this index.
    Merge(usize),
}

/// The document's spots and what the user chose for each.
#[derive(Debug, Default)]
pub struct InksDraft {
    doc: Option<DocId>,
    spots: Vec<SpotInk>,
    choice: Vec<Choice>,
    /// Why the spots could not be read, when they could not.
    error: Option<String>,
}

impl InksDraft {
    /// Start over: the next [`body`] reads the document's spots again.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The spots of document `id`, every one kept, unless this draft is already for it.
    fn attach(&mut self, id: DocId, spots: Result<Vec<SpotInk>, String>) {
        if self.doc == Some(id) {
            return;
        }
        *self = Self { doc: Some(id), ..Self::default() };
        match spots {
            Ok(spots) => {
                self.choice = vec![Choice::Keep; spots.len()];
                self.spots = spots;
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Choose what to do with spot `i`. A spot never merges into itself, and an index past the
    /// spots is ignored.
    pub fn set(&mut self, i: usize, c: Choice) {
        let ok = match c {
            Choice::Merge(j) => j != i && j < self.spots.len(),
            Choice::Keep | Choice::Process => true,
        };
        if ok && let Some(slot) = self.choice.get_mut(i) {
            *slot = c;
        }
    }

    /// The actions for the engine: spot name to what to do, the kept ones left out.
    pub fn actions(&self) -> BTreeMap<String, InkAction> {
        let mut out = BTreeMap::new();
        for (spot, c) in self.spots.iter().zip(&self.choice) {
            match c {
                Choice::Keep => {}
                Choice::Process => {
                    out.insert(spot.name.clone(), InkAction::ToProcess);
                }
                Choice::Merge(j) => {
                    if let Some(to) = self.spots.get(*j) {
                        out.insert(spot.name.clone(), InkAction::Alias(to.name.clone()));
                    }
                }
            }
        }
        out
    }

    /// Why the choices cannot be applied, or `None` when they can (and when there are none).
    pub fn problem(&self) -> Option<String> {
        // A chain of merges that comes back to where it began: the engine would leave those alone.
        for start in 0..self.spots.len() {
            let mut at = start;
            for _ in 0..=self.spots.len() {
                match self.choice.get(at) {
                    Some(Choice::Merge(j)) => at = *j,
                    _ => break,
                }
                if at == start {
                    return Some(crate::i18n::t("These merges go round in a circle: one spot has to stay or go to process.").to_owned());
                }
            }
        }
        None
    }

    /// Whether there is anything to apply.
    pub fn can_apply(&self) -> bool {
        self.choice.iter().any(|c| *c != Choice::Keep) && self.problem().is_none()
    }
}

/// Draw the dialog; returns (apply, cancel).
pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp) -> (bool, bool) {
    let Some((_, id)) = app.active_ids() else { return (false, true) };
    let spots = app.session.spot_inks(id).map_err(|e| e.to_string());
    let d = &mut app.inks_draft;
    d.attach(id, spots);
    ui.label(egui::RichText::new(tl!("Ink manager")).font(theme::semibold(18.0)));
    ui.add_space(4.0);
    if let Some(e) = &d.error {
        ui.label(egui::RichText::new(e).color(egui::Color32::from_rgb(0xc0, 0x39, 0x2b)));
    } else if d.spots.is_empty() {
        ui.label(tl!("This document uses no spot colours."));
    } else {
        ui.label(
            egui::RichText::new(tl!("\"Process\" prints a spot in the alternate colour the file carries. \"Merge into\" puts it on another spot's plate. A new document is made; this one is not changed."))
                .small()
                .weak(),
        );
        ui.add_space(6.0);
        let names: Vec<String> = d.spots.iter().map(|s| s.name.clone()).collect();
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            egui::Grid::new("inks-grid").num_columns(3).spacing([14.0, 6.0]).show(ui, |ui| {
                for (i, spot) in d.spots.clone().iter().enumerate() {
                    ui.label(&spot.name);
                    let pages = spot.pages.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
                    ui.label(egui::RichText::new(crate::i18n::fmt(tl!("page {pages}"), &[("pages", &pages)])).small().weak());
                    let shown = match d.choice.get(i).copied().unwrap_or_default() {
                        Choice::Keep => tl!("Keep as a spot").to_owned(),
                        Choice::Process => tl!("Process (CMYK)").to_owned(),
                        Choice::Merge(j) => crate::i18n::fmt(tl!("Merge into {name}"), &[("name", names.get(j).map_or("", String::as_str))]),
                    };
                    let name = crate::i18n::fmt(tl!("What to do with {name}"), &[("name", &spot.name)]);
                    let combo = craft_ui::ComboBox::from_id_salt(("inks-choice", i)).selected_text(shown).show_ui(ui, |ui| {
                        let now = d.choice.get(i).copied().unwrap_or_default();
                        if ui.selectable_label(now == Choice::Keep, tl!("Keep as a spot")).clicked() {
                            d.set(i, Choice::Keep);
                        }
                        if ui.selectable_label(now == Choice::Process, tl!("Process (CMYK)")).clicked() {
                            d.set(i, Choice::Process);
                        }
                        for (j, name) in names.iter().enumerate().filter(|(j, _)| *j != i) {
                            let label = crate::i18n::fmt(tl!("Merge into {name}"), &[("name", name)]);
                            if ui.selectable_label(now == Choice::Merge(j), label).clicked() {
                                d.set(i, Choice::Merge(j));
                            }
                        }
                    });
                    combo.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
                    ui.end_row();
                }
            });
        });
    }
    ui.add_space(12.0);
    if let Some(p) = d.problem() {
        ui.label(egui::RichText::new(p).small().color(egui::Color32::from_rgb(0xc0, 0x39, 0x2b)));
    }
    let can = d.can_apply();
    let (mut apply, mut cancel) = (false, false);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if ui.add_enabled_ui(can, |ui| widgets::pill_button(ui, tl!("Apply"), true)).inner.clicked() {
            apply = true;
        }
        if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
            cancel = true;
        }
    });
    (apply, cancel)
}

/// Make the new document and open it in a new tab.
pub(crate) fn finish(app: &mut PdfCraftApp) {
    let Some((_, id)) = app.active_ids() else { return };
    let Some(doc) = app.session.get(id) else { return };
    let stem = doc.name.strip_suffix(".pdf").or_else(|| doc.name.strip_suffix(".PDF")).unwrap_or(&doc.name).to_owned();
    let actions = app.inks_draft.actions();
    match app.session.manage_inks_pdf(id, &actions) {
        Ok((bytes, report)) => {
            if let Err(e) = app.open_bytes(&format!("{stem}_INKS.pdf"), None, bytes) {
                app.notify_error(e);
            } else if !report.kept.is_empty() {
                app.notify_fmt(tl!("{n} uses of a spot were left as they were (gradients and images)."), &[("n", &report.kept.len().to_string())]);
            }
        }
        Err(e) => app.notify_error(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(name: &str) -> SpotInk {
        SpotInk { name: name.into(), pages: vec![1] }
    }

    fn draft(names: &[&str]) -> InksDraft {
        let mut d = InksDraft::default();
        d.attach(DocId(1), Ok(names.iter().map(|n| spot(n)).collect()));
        d
    }

    #[test]
    fn everything_is_kept_until_something_is_chosen() {
        let d = draft(&["Gold", "Silver"]);
        assert!(d.actions().is_empty());
        assert!(!d.can_apply());
    }

    #[test]
    fn choices_become_the_engines_actions_by_name() {
        let mut d = draft(&["Gold", "Silver", "Spot UV"]);
        d.set(0, Choice::Process);
        d.set(2, Choice::Merge(1));
        let a = d.actions();
        assert_eq!(a.len(), 2);
        assert_eq!(a.get("Gold"), Some(&InkAction::ToProcess));
        assert_eq!(a.get("Spot UV"), Some(&InkAction::Alias("Silver".into())));
        assert!(d.can_apply());
    }

    #[test]
    fn a_spot_cannot_merge_into_itself_or_into_nothing() {
        let mut d = draft(&["Gold", "Silver"]);
        d.set(0, Choice::Merge(0));
        d.set(0, Choice::Merge(9));
        d.set(9, Choice::Process);
        assert!(d.actions().is_empty());
    }

    #[test]
    fn merges_in_a_circle_are_refused() {
        let mut d = draft(&["A", "B", "C"]);
        d.set(0, Choice::Merge(1));
        d.set(1, Choice::Merge(2));
        assert!(d.problem().is_none(), "a chain is fine");
        assert!(d.can_apply());
        d.set(2, Choice::Merge(0));
        assert!(d.problem().is_some(), "A to B to C to A");
        assert!(!d.can_apply());
        d.set(2, Choice::Process);
        assert!(d.can_apply(), "ending in process breaks the circle");
    }

    #[test]
    fn the_draft_is_kept_for_the_same_document_and_dropped_for_another() {
        let mut d = draft(&["Gold", "Silver"]);
        d.set(0, Choice::Process);
        d.attach(DocId(1), Ok(vec![spot("Other")]));
        assert_eq!(d.spots.len(), 2, "same document: nothing re-read");
        d.attach(DocId(2), Ok(vec![spot("Other")]));
        assert_eq!((d.spots.len(), d.actions().len()), (1, 0), "another document starts clean");
        d.reset();
        assert!(d.doc.is_none());
    }

    #[test]
    fn a_spot_that_could_not_be_read_is_an_error_and_nothing_is_applied() {
        let mut d = InksDraft::default();
        d.attach(DocId(1), Err("broken".into()));
        assert_eq!(d.error.as_deref(), Some("broken"));
        assert!(!d.can_apply());
    }

    /// One page painting with two spots, `Gold` and `Silver`.
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

    #[test]
    fn the_dialog_lists_the_documents_spots_and_apply_opens_a_copy_without_them() {
        use egui_kittest::kittest::Queryable;
        let mut app = PdfCraftApp::new();
        app.execute("impose.inks");
        assert!(app.dialog.is_none(), "nothing to manage without a document");
        app.open_bytes("art.pdf", None, two_spots()).expect("opens");
        app.execute("impose.inks");
        assert!(matches!(app.dialog, Some(crate::state::Dialog::ImposeInks)));

        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(700.0, 500.0)).build_ui_state(
            move |ui, app: &mut PdfCraftApp| {
                if !fonts {
                    ui.ctx().set_fonts(theme::font_definitions());
                    fonts = true;
                    return;
                }
                body(ui, app);
            },
            app,
        );
        h.run();
        h.get_by_label("Gold");
        h.get_by_label("Silver");
        assert!(!h.state().inks_draft.can_apply(), "nothing chosen yet");

        let (_, id) = h.state().active_ids().expect("a document");
        h.state_mut().inks_draft.set(0, Choice::Process);
        h.state_mut().inks_draft.set(1, Choice::Merge(0));
        let before = h.state().views.len();
        finish(h.state_mut());
        let app = h.state();
        assert_eq!(app.views.len(), before + 1, "the copy opens in a new tab");
        let (_, new_id) = app.active_ids().expect("the new tab");
        assert_eq!(app.session.get(new_id).expect("open").name, "art_INKS.pdf");
        assert!(app.session.spot_inks(new_id).expect("spots").is_empty(), "Gold in process, Silver on its plate");
        let names: Vec<String> = app.session.spot_inks(id).expect("spots").into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["Gold", "Silver"], "the original is untouched");
    }

    #[test]
    fn opening_the_dialog_again_reads_the_spots_again() {
        let mut app = PdfCraftApp::new();
        app.open_bytes("art.pdf", None, two_spots()).expect("opens");
        let (_, id) = app.active_ids().expect("a document");
        let spots = app.session.spot_inks(id).map_err(|e| e.to_string());
        app.inks_draft.attach(id, spots);
        app.inks_draft.set(0, Choice::Process);
        assert!(app.inks_draft.can_apply());
        app.execute("impose.inks");
        assert!(!app.inks_draft.can_apply(), "last time's choices are not carried over");
    }
}
