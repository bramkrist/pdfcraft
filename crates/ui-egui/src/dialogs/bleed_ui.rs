//! Imposing ▸ Add bleed: every page grows by the bleed on each side, filled by mirroring its edge
//! or by scaling the page up, and gets a `TrimBox` at its old edge. A new document is made; this
//! one is not changed. The work is `pdfcraft-impose`'s `add_bleed` through the session.

use pdfcraft_engine::impose::{AddBleed, BleedMethod};
use pdfcraft_engine::print::Content;

use crate::theme::Tokens;
use crate::units::Unit;
use crate::{PdfCraftApp, theme, widgets};

const MM: f64 = 72.0 / 25.4;

/// Dialog state. Lengths are in points.
#[derive(Clone, Debug, PartialEq)]
pub struct BleedDraft {
    /// The same bleed on every side.
    pub amount: f64,
    /// Left, bottom, right, top, when the sides differ.
    pub sides: [f64; 4],
    pub per_side: bool,
    pub method: BleedMethod,
}

impl Default for BleedDraft {
    fn default() -> Self {
        Self { amount: 3.0 * MM, sides: [3.0 * MM; 4], per_side: false, method: BleedMethod::Mirror }
    }
}

impl BleedDraft {
    pub fn spec(&self) -> AddBleed {
        let amount = if self.per_side { self.sides } else { [self.amount; 4] };
        AddBleed { amount, method: self.method }
    }

    /// Whether Finish has anything to add.
    pub fn adds_bleed(&self) -> bool {
        self.spec().amount.iter().any(|a| a.is_finite() && *a > 0.0)
    }
}

fn length(ui: &mut egui::Ui, v: &mut f64, unit: Unit) {
    let k = unit.per_point();
    let mut shown = *v * k;
    let r = ui.add(
        egui::DragValue::new(&mut shown)
            .speed(unit.speed() / 5.0)
            .range(0.0..=500.0 * k)
            .max_decimals(unit.decimals() + 2)
            .suffix(format!(" {}", unit.label())),
    );
    if r.changed() && shown.is_finite() {
        *v = (shown / k).max(0.0);
    }
}

/// Draw the dialog body; returns (finish, cancel).
pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp, _t: &Tokens) -> (bool, bool) {
    if app.active_ids().is_none() {
        return (false, true);
    }
    let unit = app.units;
    let d = &mut app.bleed_draft;
    ui.label(egui::RichText::new(tl!("Add bleed")).font(theme::semibold(18.0)));
    ui.label(egui::RichText::new(tl!("A new document is made; this one is not changed.")).weak());
    ui.add_space(8.0);
    egui::Grid::new("bleed-grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        ui.label(tl!("Bleed to add"));
        ui.vertical(|ui| {
            ui.add_enabled_ui(!d.per_side, |ui| length(ui, &mut d.amount, unit));
            ui.add(widgets::checkbox(&mut d.per_side, tl!("Set each side on its own")));
        });
        ui.end_row();
        if d.per_side {
            for (i, label) in [tl!("Left"), tl!("Bottom"), tl!("Right"), tl!("Top")].into_iter().enumerate() {
                ui.label(label);
                length(ui, &mut d.sides[i], unit);
                ui.end_row();
            }
        }
        ui.label(tl!("Fill the bleed by"));
        ui.vertical(|ui| {
            widgets::radio_value(ui, &mut d.method, BleedMethod::Mirror, tl!("Mirroring the edge (the artwork continues as its own reflection)"));
            widgets::radio_value(ui, &mut d.method, BleedMethod::Scale, tl!("Enlarging the page (the trim crops into the artwork)"));
        });
        ui.end_row();
    });
    ui.add_space(10.0);
    let (mut go, mut cancel) = (false, false);
    ui.horizontal(|ui| {
        go = ui.add_enabled(d.adds_bleed(), |ui: &mut egui::Ui| widgets::pill_button(ui, tl!("Finish"), true)).clicked();
        cancel = widgets::pill_button(ui, tl!("Cancel"), false).clicked();
    });
    (go, cancel)
}

/// Make the new document with the bleed added and open it.
pub(crate) fn finish(app: &mut PdfCraftApp) {
    let Some((_, id)) = app.active_ids() else { return };
    let Some(doc) = app.session.get(id) else { return };
    let stem = doc.name.strip_suffix(".pdf").or_else(|| doc.name.strip_suffix(".PDF")).unwrap_or(&doc.name).to_owned();
    let made = app.session.bleed_pdf(id, &app.bleed_draft.spec(), Content::DocumentAndMarkups).map_err(|e| e.to_string());
    match made {
        Ok(bytes) => {
            if let Err(e) = app.open_bytes(&format!("{stem}_BLEED.pdf"), None, bytes) {
                app.notify_error(e);
            }
        }
        Err(e) => app.notify_error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_three_millimetres_mirrored_on_every_side() {
        let d = BleedDraft::default();
        assert!(d.adds_bleed());
        let spec = d.spec();
        assert_eq!(spec.method, BleedMethod::Mirror);
        assert!(spec.amount.iter().all(|a| (a - 3.0 * MM).abs() < 1e-9));
    }

    #[test]
    fn each_side_on_its_own_replaces_the_single_amount() {
        let d = BleedDraft { per_side: true, sides: [1.0, 2.0, 3.0, 4.0], amount: 99.0, ..Default::default() };
        assert_eq!(d.spec().amount, [1.0, 2.0, 3.0, 4.0]);
        let none = BleedDraft { amount: 0.0, ..Default::default() };
        assert!(!none.adds_bleed());
        let nan = BleedDraft { amount: f64::NAN, ..Default::default() };
        assert!(!nan.adds_bleed());
    }
}
