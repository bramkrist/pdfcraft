//! Imposing ▸ Shuffle even/odd pages, Reverse pages, Trim and shift, Tile pages: the page tools an
//! imposition plug-in keeps beside its layouts, in one dialog. Each makes a new document; this
//! one is not changed. The work is `pdfcraft-impose`'s page tools through the session.

use pdfcraft_engine::impose::{self, Arrange, PageSet, PageTool, Shift, Shuffle, Tile, Trim, TrimShift};
use pdfcraft_engine::print::Content;

use crate::units::Unit;
use crate::{PdfCraftApp, theme, widgets};

const MM: f64 = 72.0 / 25.4;

/// Which page tool the dialog shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ToolKind {
    #[default]
    Shuffle,
    Reverse,
    TrimShift,
    Tile,
}

impl ToolKind {
    fn title(self) -> &'static str {
        match self {
            ToolKind::Shuffle => "Shuffle even/odd pages",
            ToolKind::Reverse => "Reverse pages",
            ToolKind::TrimShift => "Trim and shift",
            ToolKind::Tile => "Tile pages",
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            ToolKind::Shuffle => "_SHUFFLE",
            ToolKind::Reverse => "_REVERSE",
            ToolKind::TrimShift => "_TRIMSHIFT",
            ToolKind::Tile => "_TILES",
        }
    }
}

/// How Trim and shift changes the page size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TrimMode {
    #[default]
    None,
    /// Space taken off the edges.
    Trim,
    /// Space added beyond the edges.
    Add,
    /// Every page made one size.
    SameSize,
}

/// How Trim and shift moves the content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShiftMode {
    #[default]
    None,
    Fixed,
    Variable,
}

/// Dialog state. Lengths are in points.
#[derive(Clone, Debug, PartialEq)]
pub struct PageToolsDraft {
    pub kind: ToolKind,
    pub shuffle: Shuffle,
    /// Trim and shift: 0 all pages, 1 odd, 2 even.
    pub which: u8,
    pub trim_mode: TrimMode,
    /// Left, bottom, right, top (always zero or more; `trim_mode` says which way).
    pub edges: [f64; 4],
    pub same: (f64, f64),
    pub shift_mode: ShiftMode,
    /// Right and up (negative: left and down).
    pub shift: (f64, f64),
    pub shift_last: (f64, f64),
    pub tile: Tile,
}

impl Default for PageToolsDraft {
    fn default() -> Self {
        Self {
            kind: ToolKind::Shuffle,
            shuffle: Shuffle { arrange: Arrange::OddsFirst, reverse_evens: false, turn_evens: false },
            which: 0,
            trim_mode: TrimMode::None,
            edges: [0.0; 4],
            same: (210.0 * MM, 297.0 * MM),
            shift_mode: ShiftMode::Fixed,
            shift: (3.0 * MM, 0.0),
            shift_last: (0.0, 0.0),
            tile: Tile { cols: 2, rows: 1, overlap: 0.0, only_wide: false },
        }
    }
}

impl PageToolsDraft {
    /// The page tool Finish runs.
    pub fn tool(&self) -> PageTool {
        match self.kind {
            ToolKind::Reverse => PageTool::Reverse,
            ToolKind::Shuffle => PageTool::Shuffle(self.shuffle),
            ToolKind::Tile => PageTool::Tile(self.tile),
            ToolKind::TrimShift => {
                let pages = match self.which {
                    1 => PageSet::Odd,
                    2 => PageSet::Even,
                    _ => PageSet::All,
                };
                let trim = match self.trim_mode {
                    TrimMode::None => Trim::None,
                    TrimMode::Trim => Trim::Edges(self.edges),
                    TrimMode::Add => Trim::Edges(self.edges.map(|e| -e)),
                    TrimMode::SameSize => Trim::SameSize(self.same.0, self.same.1),
                };
                let shift = match self.shift_mode {
                    ShiftMode::None => Shift::None,
                    ShiftMode::Fixed => Shift::Fixed(self.shift.0, self.shift.1),
                    ShiftMode::Variable => Shift::Variable { first: self.shift, last: self.shift_last },
                };
                PageTool::TrimShift(TrimShift { pages, trim, shift })
            }
        }
    }

    /// What Finish would make from pages of `sizes`: the number of pages and the first page's
    /// size, or why it cannot.
    pub fn preview(&self, sizes: &[(f64, f64)]) -> Result<(usize, (f64, f64)), String> {
        let sheets = match self.tool() {
            PageTool::Reverse => impose::reverse(sizes),
            PageTool::Shuffle(s) => impose::shuffle(sizes, &s),
            PageTool::TrimShift(t) => {
                if t.trim == Trim::None && t.shift == Shift::None {
                    return Err(tl!("Choose a trim or a shift.").to_owned());
                }
                impose::trim_and_shift(sizes, &t)
            }
            PageTool::Tile(t) => impose::tile(sizes, &t),
        }
        .map_err(|e| e.to_string())?;
        Ok((sheets.len(), sheets.first().map_or((0.0, 0.0), |s| s.size)))
    }
}

/// A length field in `unit` holding points, from `min` (which may be negative for a shift).
fn length(ui: &mut egui::Ui, v: &mut f64, unit: Unit, min: f64) {
    let k = unit.per_point();
    let mut shown = *v * k;
    let r = ui.add(
        egui::DragValue::new(&mut shown)
            .speed(unit.speed() / 5.0)
            .range(min * k..=2_000.0 * MM * k)
            .max_decimals(unit.decimals() + 2)
            .suffix(format!(" {}", unit.label())),
    );
    if r.changed() && shown.is_finite() {
        *v = (shown / k).clamp(min, 2_000.0 * MM);
    }
}

fn count(ui: &mut egui::Ui, v: &mut usize) {
    let mut n = u32::try_from(*v).unwrap_or(1);
    if ui.add(egui::DragValue::new(&mut n).range(1..=20)).changed() {
        *v = n as usize;
    }
}

fn shuffle_ui(ui: &mut egui::Ui, d: &mut PageToolsDraft) {
    let s = &mut d.shuffle;
    widgets::radio_value(ui, &mut s.arrange, Arrange::OddsFirst, tl!("All the odd pages first, then all the even pages (fronts and backs apart)"));
    widgets::radio_value(ui, &mut s.arrange, Arrange::Interleave, tl!("Join fronts and backs: the first half are fronts, the rest backs"));
    widgets::radio_value(ui, &mut s.arrange, Arrange::Keep, tl!("Keep the order (only reverse or turn the even pages)"));
    ui.add_space(6.0);
    let reverse = match s.arrange {
        Arrange::Interleave => tl!("The backs are in reverse order (last page first)"),
        _ => tl!("Reverse the order of the even pages"),
    };
    ui.add(widgets::checkbox(&mut s.reverse_evens, reverse));
    ui.add(widgets::checkbox(&mut s.turn_evens, tl!("Turn the even pages upside down")));
}

fn trim_shift_ui(ui: &mut egui::Ui, d: &mut PageToolsDraft, unit: Unit) {
    ui.horizontal(|ui| {
        ui.label(tl!("Pages"));
        widgets::radio_value(ui, &mut d.which, 0, tl!("All"));
        widgets::radio_value(ui, &mut d.which, 1, tl!("Odd"));
        widgets::radio_value(ui, &mut d.which, 2, tl!("Even"));
    });
    ui.label(egui::RichText::new(tl!("The other pages are kept as they are. To treat odd and even pages differently, run it twice.")).weak().small());
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tl!("Page size")).font(theme::semibold(13.0)));
    widgets::radio_value(ui, &mut d.trim_mode, TrimMode::None, tl!("Do not trim"));
    widgets::radio_value(ui, &mut d.trim_mode, TrimMode::Trim, tl!("Trim space from the edges"));
    widgets::radio_value(ui, &mut d.trim_mode, TrimMode::Add, tl!("Add white space around the edges"));
    widgets::radio_value(ui, &mut d.trim_mode, TrimMode::SameSize, tl!("Make every page the same size (content centred)"));
    match d.trim_mode {
        TrimMode::Trim | TrimMode::Add => {
            egui::Grid::new("ts-edges").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
                for pair in [[(tl!("Left"), 0), (tl!("Right"), 2)], [(tl!("Top"), 3), (tl!("Bottom"), 1)]] {
                    for (label, i) in pair {
                        ui.label(label);
                        if let Some(e) = d.edges.get_mut(i) {
                            length(ui, e, unit, 0.0);
                        }
                    }
                    ui.end_row();
                }
            });
        }
        TrimMode::SameSize => {
            ui.horizontal(|ui| {
                length(ui, &mut d.same.0, unit, 1.0);
                ui.label("×");
                length(ui, &mut d.same.1, unit, 1.0);
            });
        }
        TrimMode::None => {}
    }
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tl!("Shift the content")).font(theme::semibold(13.0)));
    widgets::radio_value(ui, &mut d.shift_mode, ShiftMode::None, tl!("Do not shift"));
    widgets::radio_value(ui, &mut d.shift_mode, ShiftMode::Fixed, tl!("By a fixed amount (a binding margin, a printer offset)"));
    widgets::radio_value(ui, &mut d.shift_mode, ShiftMode::Variable, tl!("From one amount on the first page to another on the last"));
    let pair = |ui: &mut egui::Ui, label: &str, v: &mut (f64, f64)| {
        ui.horizontal(|ui| {
            ui.label(label);
            ui.label(tl!("right"));
            length(ui, &mut v.0, unit, -2_000.0 * MM);
            ui.label(tl!("up"));
            length(ui, &mut v.1, unit, -2_000.0 * MM);
        });
    };
    match d.shift_mode {
        ShiftMode::None => {}
        ShiftMode::Fixed => pair(ui, "", &mut d.shift),
        ShiftMode::Variable => {
            pair(ui, tl!("First page"), &mut d.shift);
            pair(ui, tl!("Last page"), &mut d.shift_last);
        }
    }
    ui.label(
        egui::RichText::new(tl!("Negative amounts move left or down. Content pushed past the edge is cut; nothing outside the page is uncovered."))
            .weak()
            .small(),
    );
}

fn tile_ui(ui: &mut egui::Ui, d: &mut PageToolsDraft, unit: Unit) {
    egui::Grid::new("tile-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label(tl!("Columns"));
        count(ui, &mut d.tile.cols);
        ui.end_row();
        ui.label(tl!("Rows"));
        count(ui, &mut d.tile.rows);
        ui.end_row();
        ui.label(tl!("Overlap"));
        length(ui, &mut d.tile.overlap, unit, 0.0);
        ui.end_row();
    });
    ui.add(widgets::checkbox(&mut d.tile.only_wide, tl!("Split only wide pages (spreads among single pages)")));
    ui.label(egui::RichText::new(tl!("Tiles run from the top left, row by row; each is a page of its own.")).weak().small());
}

/// Draw the dialog body; returns (finish, cancel).
pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp) -> (bool, bool) {
    let Some((_, id)) = app.active_ids() else { return (false, true) };
    let Some(doc) = app.session.get(id) else { return (false, true) };
    let sizes: Vec<(f64, f64)> = doc.info.pages.iter().map(|p| (f64::from(p.width), f64::from(p.height))).collect();
    let unit = app.units;
    let d = &mut app.page_tools_draft;
    ui.label(egui::RichText::new(tl!(d.kind.title())).font(theme::semibold(18.0)));
    ui.label(egui::RichText::new(tl!("A new document is made; this one is not changed. Links and bookmarks do not carry over.")).weak());
    ui.add_space(8.0);
    match d.kind {
        ToolKind::Shuffle => shuffle_ui(ui, d),
        ToolKind::Reverse => {
            ui.label(tl!("The pages are put in the opposite order: the last page first."));
        }
        ToolKind::TrimShift => trim_shift_ui(ui, d, unit),
        ToolKind::Tile => tile_ui(ui, d, unit),
    }
    ui.add_space(8.0);
    let preview = d.preview(&sizes);
    match &preview {
        Ok((n, (w, h))) => {
            ui.label(crate::i18n::fmt(tl!("Result: {n} pages, the first {size}"), &[("n", &n.to_string()), ("size", &unit.size(*w, *h))]))
        }
        Err(e) => ui.label(egui::RichText::new(e).color(egui::Color32::from_rgb(0xc0, 0x39, 0x2b))),
    };
    ui.add_space(10.0);
    let (mut go, mut cancel) = (false, false);
    ui.horizontal(|ui| {
        go = ui.add_enabled(preview.is_ok(), |ui: &mut egui::Ui| widgets::pill_button(ui, tl!("Finish"), true)).clicked();
        cancel = widgets::pill_button(ui, tl!("Cancel"), false).clicked();
    });
    (go, cancel)
}

/// Make the new document and open it.
pub(crate) fn finish(app: &mut PdfCraftApp) {
    let Some((_, id)) = app.active_ids() else { return };
    let Some(doc) = app.session.get(id) else { return };
    let stem = doc.name.strip_suffix(".pdf").or_else(|| doc.name.strip_suffix(".PDF")).unwrap_or(&doc.name).to_owned();
    let kind = app.page_tools_draft.kind;
    match app.session.pages_pdf(id, &app.page_tools_draft.tool(), Content::DocumentAndMarkups) {
        Ok(bytes) => {
            if let Err(e) = app.open_bytes(&format!("{stem}{}.pdf", kind.suffix()), None, bytes) {
                app.notify_error(e);
            }
        }
        Err(e) => app.notify_error(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A4: (f64, f64) = (210.0 * MM, 297.0 * MM);

    #[test]
    fn the_dialog_choices_make_the_page_tool() {
        let mut d =
            PageToolsDraft { kind: ToolKind::TrimShift, which: 2, trim_mode: TrimMode::Add, edges: [1.0, 2.0, 3.0, 4.0], ..Default::default() };
        d.shift_mode = ShiftMode::Variable;
        d.shift = (1.0, 0.0);
        d.shift_last = (5.0, 0.0);
        assert_eq!(
            d.tool(),
            PageTool::TrimShift(TrimShift {
                pages: PageSet::Even,
                trim: Trim::Edges([-1.0, -2.0, -3.0, -4.0]),
                shift: Shift::Variable { first: (1.0, 0.0), last: (5.0, 0.0) }
            }),
            "Add space is a negative trim"
        );
        d.trim_mode = TrimMode::Trim;
        assert!(matches!(d.tool(), PageTool::TrimShift(TrimShift { trim: Trim::Edges([1.0, 2.0, 3.0, 4.0]), .. })));
        assert_eq!(PageToolsDraft { kind: ToolKind::Reverse, ..Default::default() }.tool(), PageTool::Reverse);
    }

    #[test]
    fn the_preview_counts_pages_and_refuses_what_finish_would() {
        let d = PageToolsDraft { kind: ToolKind::Tile, tile: Tile { cols: 2, rows: 2, overlap: 0.0, only_wide: false }, ..Default::default() };
        let (n, (w, h)) = d.preview(&[A4; 3]).unwrap();
        assert_eq!(n, 12);
        assert!((w - A4.0 / 2.0).abs() < 1e-9 && (h - A4.1 / 2.0).abs() < 1e-9);
        let nothing = PageToolsDraft { kind: ToolKind::TrimShift, trim_mode: TrimMode::None, shift_mode: ShiftMode::None, ..Default::default() };
        assert!(nothing.preview(&[A4]).is_err(), "a trim and shift that does nothing");
        let off = PageToolsDraft { kind: ToolKind::TrimShift, shift: (500.0 * MM, 0.0), ..Default::default() };
        assert!(off.preview(&[A4]).is_err(), "content shifted wholly off the page");
        let shuffled = PageToolsDraft::default();
        assert_eq!(shuffled.preview(&[A4; 5]).unwrap().0, 5);
    }
}
