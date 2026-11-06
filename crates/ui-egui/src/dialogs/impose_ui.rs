//! Imposition wizards: Step and repeat, n-up pages and Create booklet, in the order a press-room
//! imposition plug-in asks its questions — the pages first, then margins and marks, then the
//! sheet, with a running summary of the layout — and Finish making a new document. The layouts
//! come from `pdfcraft-impose` through the session; the original document is never changed.

use egui::{Align, Color32, CornerRadius, Layout, Rect, Stroke, pos2, vec2};
use pdfcraft_engine::impose::{self, Align as BookletAlign, Binding, Booklet, Duplex, Fill, LastSignature, Margins, MarkStyle, Marks, StepRepeat};
use pdfcraft_engine::print::Sheet;

use crate::theme::{self, Tokens};
use crate::units::Unit;
use crate::{PdfCraftApp, widgets};

const MM: f64 = 72.0 / 25.4;

/// Which imposition the wizard makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImposeKind {
    /// Every page repeated to fill its own sheet.
    StepRepeat,
    /// Different pages combined on each sheet (ganging), optionally cut & stack.
    NUp,
    /// Pages in folding order, two to a sheet side.
    Booklet,
}

impl ImposeKind {
    fn title(self) -> &'static str {
        match self {
            ImposeKind::StepRepeat => "Step and repeat",
            ImposeKind::NUp => "n-up pages",
            ImposeKind::Booklet => "Create booklet",
        }
    }

    fn screens(self) -> usize {
        match self {
            ImposeKind::Booklet => 2,
            _ => 3,
        }
    }

    /// The suffix of the new document's name.
    fn suffix(self) -> &'static str {
        match self {
            ImposeKind::StepRepeat => "_STEPREPEAT",
            ImposeKind::NUp => "_NUP",
            ImposeKind::Booklet => "_BOOKLET",
        }
    }
}

/// Margins and spacing: none, one value each, or each side and gutter on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spacing {
    None,
    Simple,
    Advanced,
}

/// Which way the sheet is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orient {
    Tall,
    Wide,
    /// Whichever of tall and wide places more pages.
    Best,
}

/// The sheet sizes offered (millimetres, portrait); the last choice is a custom size.
pub const PRESETS: &[(&str, f64, f64)] =
    &[("A3+ 32 × 48 cm", 320.0, 480.0), ("33 × 48 cm", 330.0, 480.0), ("SRA3", 320.0, 450.0), ("A3", 297.0, 420.0), ("A4", 210.0, 297.0)];

/// The wizard's state. Lengths are in points.
#[derive(Clone, Debug, PartialEq)]
pub struct ImposeDraft {
    /// 0-based screen.
    pub screen: usize,
    // Screen 1: the pages.
    pub bleed: f64,
    pub duplex: Option<Duplex>,
    /// n-up: 0 places pages in reading order; n ≥ 1 cuts and stacks with the block repeated n times.
    pub stack_copies: usize,
    // Screen 2: margins and marks.
    pub spacing: Spacing,
    pub edge: f64,
    pub gap: f64,
    /// Advanced: left, bottom, right, top.
    pub margins: [f64; 4],
    /// Advanced: horizontal, vertical.
    pub gutter: (f64, f64),
    pub marks: bool,
    pub custom_marks: bool,
    pub mark: Marks,
    // Screen 3: the sheet.
    /// Index into [`PRESETS`], or `PRESETS.len()` for a custom size.
    pub preset: usize,
    /// Custom sheet, portrait.
    pub custom: (f64, f64),
    pub orient: Orient,
    /// Most columns and rows (0: as many as fit).
    pub max_cols: usize,
    pub max_rows: usize,
    pub rotate: bool,
    // Booklet.
    pub binding: usize,
    pub signature: usize,
    pub full_last: bool,
    pub rtl: bool,
    pub auto_sheet: bool,
    pub scale: bool,
    pub spine: bool,
    pub creep: f64,
}

impl Default for ImposeDraft {
    fn default() -> Self {
        Self {
            screen: 0,
            bleed: 0.0,
            duplex: None,
            stack_copies: 0,
            spacing: Spacing::Simple,
            edge: 5.0 * MM,
            gap: 0.0,
            margins: [5.0 * MM; 4],
            gutter: (0.0, 0.0),
            marks: true,
            custom_marks: false,
            mark: Marks::default(),
            preset: 0,
            custom: (320.0 * MM, 480.0 * MM),
            orient: Orient::Best,
            max_cols: 0,
            max_rows: 0,
            rotate: true,
            binding: 0,
            signature: 16,
            full_last: false,
            rtl: false,
            auto_sheet: false,
            scale: true,
            spine: false,
            creep: 0.0,
        }
    }
}

/// What a layout came to, for the summary and the preview.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub sheet: (f64, f64),
    pub cols: usize,
    pub rows: usize,
    pub per_sheet: usize,
    /// Columns × rows, row by row from the top where rows differ (turned rows): "5 × 4 + 8 × 1".
    pub layout: String,
    pub sides: usize,
    pub placed: usize,
    /// The first sheet side's page footprints and mark lines, for the preview.
    pub first: Option<Sheet>,
}

/// The four corners of `clip` through `m`, as a box.
fn footprint(m: [f64; 6], clip: [f64; 4]) -> [f64; 4] {
    let [a, b, c, d, e, f] = m;
    let pts = [(clip[0], clip[1]), (clip[2], clip[1]), (clip[0], clip[3]), (clip[2], clip[3])];
    let mut r = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for (x, y) in pts {
        let (px, py) = (a * x + c * y + e, b * x + d * y + f);
        r = [r[0].min(px), r[1].min(py), r[2].max(px), r[3].max(py)];
    }
    r
}

/// Distinct values, half a point apart.
fn distinct(mut v: Vec<f64>) -> usize {
    v.sort_by(f64::total_cmp);
    v.dedup_by(|a, b| (*a - *b).abs() < 0.5);
    v.len()
}

/// Columns × rows of `rects`, a run of rows with the same number of pages at a time, top first.
fn layout_label(rects: &[[f64; 4]]) -> String {
    // Pages per row, rows found by their bottom edge (half a point apart), top row first.
    let mut rows: Vec<(f64, usize)> = Vec::new();
    for r in rects {
        match rows.iter_mut().find(|(y, _)| (*y - r[1]).abs() < 0.5) {
            Some(row) => row.1 += 1,
            None => rows.push((r[1], 1)),
        }
    }
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (_, n) in rows {
        match runs.last_mut() {
            Some((cols, count)) if *cols == n => *count += 1,
            _ => runs.push((n, 1)),
        }
    }
    if runs.is_empty() {
        return "0".to_owned();
    }
    runs.iter().map(|(cols, count)| format!("{cols} × {count}")).collect::<Vec<_>>().join(" + ")
}

fn summarize(sheets: &[Sheet]) -> Summary {
    let first = sheets.first();
    let rects: Vec<[f64; 4]> = first.map(|s| s.placed.iter().map(|p| footprint(p.matrix.0, p.clip)).collect()).unwrap_or_default();
    Summary {
        layout: layout_label(&rects),
        sheet: first.map_or((0.0, 0.0), |s| s.size),
        cols: distinct(rects.iter().map(|r| r[0]).collect()),
        rows: distinct(rects.iter().map(|r| r[1]).collect()),
        per_sheet: rects.len(),
        sides: sheets.len(),
        placed: sheets.iter().map(|s| s.placed.len()).sum(),
        first: first.cloned(),
    }
}

impl ImposeDraft {
    /// The sheet, portrait, in points.
    pub fn portrait(&self) -> (f64, f64) {
        let (w, h) = match PRESETS.get(self.preset) {
            Some(&(_, w, h)) => (w * MM, h * MM),
            None => self.custom,
        };
        (w.min(h), w.max(h))
    }

    fn margins_and_gutter(&self) -> (Margins, (f64, f64)) {
        match self.spacing {
            Spacing::None => (Margins::default(), (0.0, 0.0)),
            Spacing::Simple => (Margins::uniform(self.edge), (self.gap, self.gap)),
            Spacing::Advanced => {
                let [left, bottom, right, top] = self.margins;
                (Margins { left, bottom, right, top }, self.gutter)
            }
        }
    }

    /// The step & repeat / n-up spec for a sheet used `wide` or tall, with as many columns and
    /// rows as fit.
    fn spec(&self, kind: ImposeKind, wide: bool) -> StepRepeat {
        let (w, h) = self.portrait();
        let mut s = StepRepeat::new(if wide { (h, w) } else { (w, h) });
        (s.margins, s.gutter) = self.margins_and_gutter();
        s.fill = match (kind, self.stack_copies) {
            (ImposeKind::StepRepeat, _) => Fill::Repeat,
            (_, 0) => Fill::Sequential,
            (_, copies) => Fill::CutStack { copies },
        };
        s.bleed = self.bleed;
        s.duplex = self.duplex;
        s.marks = self.marks.then_some(self.mark);
        s.rotate = kind == ImposeKind::StepRepeat && self.rotate && self.max_cols == 0 && self.max_rows == 0;
        s
    }

    /// The layout for one orientation: columns and rows as many as fit, then capped by the
    /// maximums (a maximum is never a minimum).
    fn layout_one(&self, kind: ImposeKind, sizes: &[(f64, f64)], wide: bool) -> Result<(StepRepeat, Vec<Sheet>), String> {
        let mut spec = self.spec(kind, wide);
        let mut sheets = impose::step_and_repeat(sizes, &spec).map_err(|e| e.to_string())?;
        if self.max_cols > 0 || self.max_rows > 0 {
            let fit = summarize(&sheets);
            let cols = if self.max_cols > 0 { self.max_cols.min(fit.cols) } else { fit.cols };
            let rows = if self.max_rows > 0 { self.max_rows.min(fit.rows) } else { fit.rows };
            if let Fill::CutStack { copies } = spec.fill
                && cols % copies.max(1) != 0
            {
                return Err(crate::i18n::fmt(
                    tl!("{cols} columns can't hold {n} copies of the block"),
                    &[("cols", &cols.to_string()), ("n", &copies.to_string())],
                ));
            }
            spec.cols = Some(cols.max(1));
            spec.rows = Some(rows.max(1));
            sheets = impose::step_and_repeat(sizes, &spec).map_err(|e| e.to_string())?;
        }
        Ok((spec, sheets))
    }

    /// The step & repeat or n-up layout of pages of `sizes`, with Best fit resolved: the
    /// orientation that places more pages on a sheet, then the one needing fewer sheets, tall on a tie.
    pub fn layout(&self, kind: ImposeKind, sizes: &[(f64, f64)]) -> Result<(StepRepeat, Vec<Sheet>), String> {
        match self.orient {
            Orient::Tall => self.layout_one(kind, sizes, false),
            Orient::Wide => self.layout_one(kind, sizes, true),
            Orient::Best => match (self.layout_one(kind, sizes, false), self.layout_one(kind, sizes, true)) {
                (Ok(t), Ok(w)) => {
                    let (st, sw) = (summarize(&t.1), summarize(&w.1));
                    Ok(if sw.per_sheet > st.per_sheet || (sw.per_sheet == st.per_sheet && sw.sides < st.sides) { w } else { t })
                }
                (Ok(t), Err(_)) => Ok(t),
                (Err(_), Ok(w)) => Ok(w),
                (Err(e), Err(_)) => Err(e),
            },
        }
    }

    pub fn booklet_spec(&self) -> Booklet {
        let (w, h) = self.portrait();
        let mut b = Booklet::new();
        // A booklet sheet holds two pages side by side: use it wide unless asked otherwise.
        b.sheet = (!self.auto_sheet).then_some(if self.orient == Orient::Tall { (w, h) } else { (h, w) });
        b.binding = match self.binding {
            1 => Binding::Perfect {
                signature: self.signature.max(4) / 4 * 4,
                last: if self.full_last { LastSignature::Full } else { LastSignature::MultipleOfFour },
            },
            2 => Binding::Continuous,
            _ => Binding::Saddle,
        };
        b.scale = self.scale;
        b.align = if self.spine { BookletAlign::Spine } else { BookletAlign::Centre };
        b.edge = if self.spacing == Spacing::None { 0.0 } else { self.edge };
        b.right_to_left = self.rtl;
        b.creep = self.creep;
        b
    }

    /// The summary of what Finish would make from pages of `sizes`.
    pub fn summary(&self, kind: ImposeKind, sizes: &[(f64, f64)]) -> Result<Summary, String> {
        match kind {
            ImposeKind::Booklet => impose::booklet(sizes, &self.booklet_spec()).map(|s| summarize(&s)).map_err(|e| e.to_string()),
            _ => self.layout(kind, sizes).map(|(_, s)| summarize(&s)),
        }
    }
}

/// A length field in `unit`, holding points.
fn length(ui: &mut egui::Ui, v: &mut f64, unit: Unit, min: f64) -> egui::Response {
    let k = unit.per_point();
    let mut shown = *v * k;
    let r = ui.add(
        egui::DragValue::new(&mut shown)
            .speed(unit.speed() / 5.0)
            .range(min * k..=5_000.0 * k)
            .max_decimals(unit.decimals() + 2)
            .suffix(format!(" {}", unit.label())),
    );
    if r.changed() && shown.is_finite() {
        *v = (shown / k).max(min);
    }
    r
}

fn count(ui: &mut egui::Ui, v: &mut usize) {
    let mut n = *v as u32;
    if ui
        .add(egui::DragValue::new(&mut n).range(0..=200).custom_formatter(|n, _| if n < 0.5 { tl!("Any").to_owned() } else { format!("{n:.0}") }))
        .changed()
    {
        *v = n as usize;
    }
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).font(theme::semibold(13.0)));
}

fn screen_pages(ui: &mut egui::Ui, d: &mut ImposeDraft, kind: ImposeKind, unit: Unit) {
    heading(ui, tl!("The pages"));
    ui.label(egui::RichText::new(tl!("A new document is made; this one is not changed.")).weak());
    ui.horizontal(|ui| {
        ui.label(tl!("Bleed included in each page"));
        length(ui, &mut d.bleed, unit, 0.0);
    });
    ui.label(egui::RichText::new(tl!("The trim edge lies this far inside every page; marks are drawn at the trim.")).weak().small());
    heading(ui, tl!("Printing"));
    widgets::radio_value(ui, &mut d.duplex, None, tl!("One-sided"));
    widgets::radio_value(ui, &mut d.duplex, Some(Duplex::LongEdge), tl!("Two-sided, turned over the long edge (pages come in front/back pairs)"));
    widgets::radio_value(ui, &mut d.duplex, Some(Duplex::ShortEdge), tl!("Two-sided, turned over the short edge"));
    if kind == ImposeKind::NUp {
        heading(ui, tl!("Page order on the sheet"));
        let mut stacked = d.stack_copies > 0;
        widgets::radio_value(ui, &mut stacked, false, tl!("In reading order (1 2 3 4 on the first sheet)"));
        widgets::radio_value(ui, &mut stacked, true, tl!("Cut & stack: each stack runs on from sheet to sheet"));
        if stacked {
            if d.stack_copies == 0 {
                d.stack_copies = 1;
            }
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.label(tl!("Copies of the block across the sheet"));
                let mut n = d.stack_copies as u32;
                if ui.add(egui::DragValue::new(&mut n).range(1..=20)).changed() {
                    d.stack_copies = n as usize;
                }
            });
        } else {
            d.stack_copies = 0;
        }
    }
}

fn screen_marks(ui: &mut egui::Ui, d: &mut ImposeDraft, kind: ImposeKind, unit: Unit) {
    heading(ui, tl!("Margins and spacing"));
    widgets::radio_value(ui, &mut d.spacing, Spacing::None, tl!("No margins or spacing"));
    widgets::radio_value(ui, &mut d.spacing, Spacing::Simple, tl!("Margins and spacing"));
    if d.spacing == Spacing::Simple {
        egui::Grid::new("impose-simple").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label(tl!("Space at edge of sheet"));
            length(ui, &mut d.edge, unit, 0.0);
            ui.end_row();
            if kind != ImposeKind::Booklet {
                ui.label(tl!("Space between pages"));
                length(ui, &mut d.gap, unit, 0.0);
                ui.end_row();
            }
        });
    }
    if kind != ImposeKind::Booklet {
        widgets::radio_value(ui, &mut d.spacing, Spacing::Advanced, tl!("Advanced margins and spacing"));
        if d.spacing == Spacing::Advanced {
            egui::Grid::new("impose-advanced").num_columns(4).spacing([12.0, 6.0]).show(ui, |ui| {
                for (label, i) in [(tl!("Top"), 3), (tl!("Bottom"), 1), (tl!("Left"), 0), (tl!("Right"), 2)] {
                    ui.label(label);
                    if let Some(m) = d.margins.get_mut(i) {
                        length(ui, m, unit, 0.0);
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
                ui.label(tl!("Horizontal spacing"));
                length(ui, &mut d.gutter.0, unit, 0.0);
                ui.label(tl!("Vertical spacing"));
                length(ui, &mut d.gutter.1, unit, 0.0);
                ui.end_row();
            });
        }
    } else if d.spacing == Spacing::Advanced {
        d.spacing = Spacing::Simple;
    }
    if kind == ImposeKind::Booklet {
        return;
    }
    heading(ui, tl!("Marks"));
    ui.horizontal(|ui| {
        ui.add(widgets::checkbox(&mut d.marks, tl!("Add crop marks")));
        if d.marks && ui.button(if d.custom_marks { tl!("Hide custom") } else { tl!("Custom…") }).clicked() {
            d.custom_marks = !d.custom_marks;
        }
    });
    if d.marks && d.custom_marks {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::Grid::new("impose-marks").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label(tl!("Style"));
                let styles = [
                    (MarkStyle::Corner, tl!("Corner marks")),
                    (MarkStyle::Japanese, tl!("Japanese (trim and bleed)")),
                    (MarkStyle::Centred, tl!("INDIE Crop Mark")),
                ];
                let cur = styles.iter().find(|(s, _)| *s == d.mark.style).map_or("", |(_, l)| *l);
                craft_ui::ComboBox::from_id_salt("impose-mark-style").selected_text(cur).show_ui(ui, |ui| {
                    for (s, l) in styles {
                        ui.selectable_value(&mut d.mark.style, s, l);
                    }
                });
                ui.end_row();
                if d.mark.style != MarkStyle::Centred {
                    ui.label(tl!("Distance from page edge"));
                    let k = unit.per_point();
                    let mut v = d.mark.offset * k;
                    if ui
                        .add(
                            egui::DragValue::new(&mut v)
                                .speed(unit.speed() / 5.0)
                                .range(-50.0 * MM * k..=50.0 * MM * k)
                                .max_decimals(unit.decimals() + 2)
                                .suffix(format!(" {}", unit.label())),
                        )
                        .changed()
                        && v.is_finite()
                    {
                        d.mark.offset = v / k;
                    }
                    ui.end_row();
                }
                ui.label(tl!("Length"));
                length(ui, &mut d.mark.length, unit, 0.5);
                ui.end_row();
                ui.label(tl!("Width of line"));
                length(ui, &mut d.mark.width, unit, 0.05);
                ui.end_row();
                ui.label("");
                ui.add(widgets::checkbox(&mut d.mark.registration, tl!("Registration colour (every plate)")));
                ui.end_row();
                ui.label("");
                if ui.button(tl!("Get default values")).clicked() {
                    d.mark = Marks { style: d.mark.style, ..Marks::default() };
                }
                ui.end_row();
            });
        });
    }
}

fn sheet_choice(ui: &mut egui::Ui, d: &mut ImposeDraft, unit: Unit) {
    let cur = PRESETS.get(d.preset).map_or(tl!("Custom size"), |p| p.0);
    craft_ui::ComboBox::from_id_salt("impose-sheet").selected_text(cur).show_ui(ui, |ui| {
        for (i, p) in PRESETS.iter().enumerate() {
            ui.selectable_value(&mut d.preset, i, p.0);
        }
        ui.selectable_value(&mut d.preset, PRESETS.len(), tl!("Custom size"));
    });
    if d.preset >= PRESETS.len() {
        length(ui, &mut d.custom.0, unit, 10.0);
        ui.label("×");
        length(ui, &mut d.custom.1, unit, 10.0);
    }
}

fn screen_sheet(ui: &mut egui::Ui, d: &mut ImposeDraft, kind: ImposeKind, unit: Unit) {
    heading(ui, tl!("Sheet size"));
    ui.horizontal(|ui| sheet_choice(ui, d, unit));
    ui.horizontal(|ui| {
        widgets::radio_value(ui, &mut d.orient, Orient::Tall, tl!("Tall"));
        widgets::radio_value(ui, &mut d.orient, Orient::Wide, tl!("Wide"));
        widgets::radio_value(ui, &mut d.orient, Orient::Best, tl!("Best fit"));
    });
    heading(ui, tl!("Layout"));
    egui::Grid::new("impose-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label(tl!("Maximum columns"));
        count(ui, &mut d.max_cols);
        ui.end_row();
        ui.label(tl!("Maximum rows"));
        count(ui, &mut d.max_rows);
        ui.end_row();
    });
    if kind == ImposeKind::StepRepeat {
        let auto = d.max_cols == 0 && d.max_rows == 0;
        ui.add_enabled(auto, widgets::checkbox(&mut d.rotate, tl!("Turn rows of pages where that fits more copies")));
        if !auto {
            ui.label(egui::RichText::new(tl!("Turning rows needs Any columns and rows.")).weak().small());
        }
    }
}

fn screen_booklet(ui: &mut egui::Ui, d: &mut ImposeDraft) {
    heading(ui, tl!("Binding"));
    widgets::radio_value(ui, &mut d.binding, 0, tl!("Saddle stitch: one folded signature"));
    widgets::radio_value(ui, &mut d.binding, 1, tl!("Perfect bound: signatures read one after another"));
    if d.binding == 1 {
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.label(tl!("Pages per signature"));
            let mut n = d.signature as u32;
            if ui.add(egui::DragValue::new(&mut n).range(4..=128).speed(0.1)).changed() {
                d.signature = (n as usize).max(4) / 4 * 4;
            }
        });
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.add(widgets::checkbox(&mut d.full_last, tl!("Pad the last signature to full size")));
        });
    }
    widgets::radio_value(ui, &mut d.binding, 2, tl!("Continuous: two pages a side, not folded"));
    ui.add_space(4.0);
    ui.add(widgets::checkbox(&mut d.rtl, tl!("Bound on the right (right-to-left)")));
}

fn screen_booklet_sheet(ui: &mut egui::Ui, d: &mut ImposeDraft, unit: Unit) {
    heading(ui, tl!("Sheet size"));
    widgets::radio_value(ui, &mut d.auto_sheet, true, tl!("Smallest sheet that holds two pages"));
    ui.horizontal(|ui| {
        if ui.add(widgets::radio(!d.auto_sheet, tl!("Sheet"))).clicked() {
            d.auto_sheet = false;
        }
        ui.add_enabled_ui(!d.auto_sheet, |ui| sheet_choice(ui, d, unit));
    });
    heading(ui, tl!("Pages"));
    widgets::radio_value(ui, &mut d.scale, true, tl!("Scale pages to fill their half of the sheet"));
    widgets::radio_value(ui, &mut d.scale, false, tl!("Do not scale pages (100% only)"));
    widgets::radio_value(ui, &mut d.spine, false, tl!("Centre each page in its half"));
    widgets::radio_value(ui, &mut d.spine, true, tl!("Pull pages to the spine"));
    ui.horizontal(|ui| {
        ui.label(tl!("Space at edge of sheet"));
        length(ui, &mut d.edge, unit, 0.0);
    });
    if d.spacing == Spacing::None {
        d.spacing = Spacing::Simple;
    }
    if d.binding != 2 {
        ui.horizontal(|ui| {
            ui.label(tl!("Creep per sheet"));
            length(ui, &mut d.creep, unit, 0.0);
        });
        ui.label(egui::RichText::new(tl!("Inner sheets move this much more towards the spine for each sheet outside them.")).weak().small());
    }
}

/// The summary lines and a small picture of the first sheet side.
fn summary_ui(ui: &mut egui::Ui, s: &Result<Summary, String>, unit: Unit, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(vec2(200.0, 200.0), egui::Sense::hover());
    match s {
        Err(e) => {
            ui.painter().rect_stroke(r.shrink(20.0), CornerRadius::ZERO, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            ui.label(egui::RichText::new(e).color(Color32::from_rgb(0xc0, 0x39, 0x2b)));
        }
        Ok(s) => {
            let (w, h) = (s.sheet.0.max(1.0), s.sheet.1.max(1.0));
            let k = (190.0 / w.max(h)) as f32;
            let sheet = Rect::from_center_size(r.center(), vec2(w as f32 * k, h as f32 * k));
            let p = ui.painter();
            p.rect_filled(sheet, CornerRadius::ZERO, Color32::WHITE);
            p.rect_stroke(sheet, CornerRadius::ZERO, Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
            let to = |x: f64, y: f64| pos2(sheet.left() + x as f32 * k, sheet.bottom() - y as f32 * k);
            if let Some(first) = &s.first {
                for pl in &first.placed {
                    let f = footprint(pl.matrix.0, pl.clip);
                    let cell = Rect::from_two_pos(to(f[0], f[1]), to(f[2], f[3]));
                    p.rect_filled(cell, CornerRadius::ZERO, t.accent.gamma_multiply(0.18));
                    p.rect_stroke(cell, CornerRadius::ZERO, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
                    p.text(cell.center(), egui::Align2::CENTER_CENTER, (pl.page + 1).to_string(), theme::regular(10.0), t.text_faint);
                }
                for l in &first.lines {
                    p.line_segment([to(l[0], l[1]), to(l[2], l[3])], Stroke::new(0.8, Color32::BLACK));
                }
            }
            egui::Grid::new("impose-summary").num_columns(2).spacing([10.0, 2.0]).show(ui, |ui| {
                ui.label(tl!("Sheet"));
                ui.label(unit.size(s.sheet.0, s.sheet.1));
                ui.end_row();
                ui.label(tl!("Layout"));
                ui.label(&s.layout);
                ui.end_row();
                ui.label(tl!("Pages per sheet"));
                ui.label(s.per_sheet.to_string());
                ui.end_row();
                ui.label(tl!("Sheet sides"));
                ui.label(s.sides.to_string());
                ui.end_row();
                ui.label(tl!("Total pages placed"));
                ui.label(s.placed.to_string());
                ui.end_row();
            });
        }
    }
}

/// Draw the wizard; returns (finish, cancel).
pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp, t: &Tokens, kind: ImposeKind) -> (bool, bool) {
    let Some((_, id)) = app.active_ids() else { return (false, true) };
    let Some(doc) = app.session.get(id) else { return (false, true) };
    let sizes: Vec<(f64, f64)> = doc.info.pages.iter().map(|p| (f64::from(p.width), f64::from(p.height))).collect();
    let unit = app.units;
    let d = &mut app.impose_draft;
    let last = kind.screens() - 1;
    d.screen = d.screen.min(last);
    ui.label(egui::RichText::new(tl!(kind.title())).font(theme::semibold(18.0)));
    ui.label(
        egui::RichText::new(crate::i18n::fmt(tl!("Step {n} of {of}"), &[("n", &(d.screen + 1).to_string()), ("of", &(last + 1).to_string())])).weak(),
    );
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(440.0);
            match (kind, d.screen) {
                (ImposeKind::Booklet, 0) => screen_booklet(ui, d),
                (ImposeKind::Booklet, _) => screen_booklet_sheet(ui, d, unit),
                (_, 0) => screen_pages(ui, d, kind, unit),
                (_, 1) => screen_marks(ui, d, kind, unit),
                _ => screen_sheet(ui, d, kind, unit),
            }
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            let s = d.summary(kind, &sizes);
            summary_ui(ui, &s, unit, t);
        });
    });
    ui.add_space(12.0);
    let (mut finish, mut cancel) = (false, false);
    let ok = d.summary(kind, &sizes).is_ok();
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if d.screen == last {
            if ui.add_enabled_ui(ok, |ui| widgets::pill_button(ui, tl!("Finish"), true)).inner.clicked() {
                finish = true;
            }
        } else if widgets::pill_button(ui, tl!("Next"), true).clicked() {
            d.screen += 1;
        }
        if d.screen > 0 && widgets::pill_button(ui, tl!("Back"), false).clicked() {
            d.screen -= 1;
        }
        if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
            cancel = true;
        }
    });
    (finish, cancel)
}

/// Make the imposed document and open it in a new tab.
pub(crate) fn finish(app: &mut PdfCraftApp, kind: ImposeKind) {
    let Some((_, id)) = app.active_ids() else { return };
    let Some(doc) = app.session.get(id) else { return };
    let sizes: Vec<(f64, f64)> = doc.info.pages.iter().map(|p| (f64::from(p.width), f64::from(p.height))).collect();
    let stem = doc.name.strip_suffix(".pdf").or_else(|| doc.name.strip_suffix(".PDF")).unwrap_or(&doc.name).to_owned();
    let content = pdfcraft_engine::print::Content::DocumentAndMarkups;
    let made = match kind {
        ImposeKind::Booklet => app.session.booklet_pdf(id, &app.impose_draft.booklet_spec(), content).map_err(|e| e.to_string()),
        _ => match app.impose_draft.layout(kind, &sizes) {
            Ok((spec, _)) => app.session.impose_pdf(id, &spec, content).map_err(|e| e.to_string()),
            Err(e) => Err(e),
        },
    };
    match made {
        Ok(bytes) => {
            let name = format!("{stem}{}.pdf", kind.suffix());
            match app.open_bytes(&name, None, bytes) {
                Ok(()) => app.impose_draft.screen = 0,
                Err(e) => app.notify_error(e),
            }
        }
        Err(e) => app.notify_error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A5: (f64, f64) = (148.0 * MM, 210.0 * MM);

    #[test]
    fn best_fit_picks_the_orientation_that_places_more() {
        let mut d = ImposeDraft { spacing: Spacing::None, marks: false, rotate: false, ..Default::default() };
        d.preset = 3; // A3 holds 2 x 2 A5 tall, 2 x 1 wide.
        let s = d.summary(ImposeKind::StepRepeat, &[A5]).unwrap();
        assert_eq!((s.cols, s.rows, s.per_sheet), (2, 2, 4));
        // Business cards 90 x 55 on A4: tall 2 x 5 = 10, wide 3 x 3 = 9; best fit takes tall.
        let card = (90.0 * MM, 55.0 * MM);
        d.preset = 4;
        let best = d.summary(ImposeKind::StepRepeat, &[card]).unwrap();
        d.orient = Orient::Wide;
        let wide = d.summary(ImposeKind::StepRepeat, &[card]).unwrap();
        assert_eq!(wide.per_sheet, 9);
        assert_eq!(best.per_sheet, 10);
        assert!(best.sheet.0 < best.sheet.1, "tall sheet chosen");
    }

    #[test]
    fn maximum_columns_and_rows_cap_but_never_add() {
        let mut d = ImposeDraft { spacing: Spacing::None, marks: false, orient: Orient::Tall, preset: 3, ..Default::default() };
        d.max_cols = 1;
        let s = d.summary(ImposeKind::StepRepeat, &[A5]).unwrap();
        assert_eq!((s.cols, s.rows), (1, 2));
        d.max_cols = 9;
        d.max_rows = 9;
        let s = d.summary(ImposeKind::StepRepeat, &[A5]).unwrap();
        assert_eq!((s.cols, s.rows), (2, 2), "a maximum larger than fits is not a minimum");
    }

    #[test]
    fn n_up_counts_sheets_and_cut_stack_needs_columns_for_its_copies() {
        let mut d = ImposeDraft { spacing: Spacing::None, marks: false, orient: Orient::Tall, preset: 3, ..Default::default() };
        let s = d.summary(ImposeKind::NUp, &[A5; 10]).unwrap();
        assert_eq!((s.per_sheet, s.sides, s.placed), (4, 3, 10));
        d.stack_copies = 3;
        d.max_cols = 2;
        assert!(d.summary(ImposeKind::NUp, &[A5; 10]).is_err(), "2 columns can't hold 3 copies of the block");
    }

    #[test]
    fn booklets_pad_to_four_and_marks_draw_on_step_and_repeat() {
        let d = ImposeDraft { auto_sheet: true, ..Default::default() };
        let s = d.summary(ImposeKind::Booklet, &[A5; 6]).unwrap();
        assert_eq!(s.sides, 4, "6 pages saddle-stitched: 8 pages on 2 sheets, 4 sides");
        let d = ImposeDraft { orient: Orient::Tall, preset: 3, ..Default::default() };
        let s = d.summary(ImposeKind::StepRepeat, &[A5]).unwrap();
        assert!(s.first.as_ref().is_some_and(|f| !f.lines.is_empty()), "crop marks are on by default");
    }

    #[test]
    fn turned_rows_are_counted_row_by_row() {
        // Business cards on A3+ wide: straight rows and then a row turned a quarter turn, which
        // has more columns; one "cols × rows" would be wrong.
        let card = (90.0 * MM, 55.0 * MM);
        let d = ImposeDraft { orient: Orient::Wide, ..Default::default() };
        let s = d.summary(ImposeKind::StepRepeat, &[card]).unwrap();
        assert_eq!(s.layout, "5 × 4 + 8 × 1");
        assert_eq!(s.per_sheet, 28);
        let d = ImposeDraft { rotate: false, orient: Orient::Wide, ..Default::default() };
        assert_eq!(d.summary(ImposeKind::StepRepeat, &[card]).unwrap().layout, "5 × 5");
    }

    #[test]
    fn a_page_too_big_for_the_sheet_says_so() {
        let d = ImposeDraft { preset: 4, ..Default::default() };
        assert!(d.summary(ImposeKind::StepRepeat, &[(600.0 * MM, 900.0 * MM)]).is_err());
    }
}
