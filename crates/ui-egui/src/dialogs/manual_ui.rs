//! Imposing ▸ Manual: every page placed by hand on the sheet, as a press room's manual
//! imposition does. The sheet is drawn to scale; a page is dragged where it goes and snaps to
//! the sheet's edges and to the other pages' trim and bleed edges, turned in quarter turns,
//! copied and deleted, with as many sheet sides as the job needs. Finish makes a new document
//! through the session (`Session::manual_pdf`); the original is never changed.
//!
//! The layout is plain data in millimetres ([`ManualDraft`]), so what it does is tested without
//! a window; the egui part only draws it and turns the pointer and keys into those operations.

use egui::{Align, Color32, CornerRadius, Layout, Rect, Sense, Stroke, pos2, vec2};
use pdfcraft_engine::DocId;
use pdfcraft_engine::impose::{self, Fill, Manual, ManualPage, Margins, Marks, StepRepeat};
use pdfcraft_engine::print::Placement;

use crate::impose_ui::{PRESETS, footprint as footprint_pt};
use crate::theme;
use crate::{PdfCraftApp, widgets};

const MM: f64 = 72.0 / 25.4;
/// How much two edges may overlap before it counts, in millimetres.
const SLACK: f64 = 0.01;
/// Farthest a page may be put from the sheet's corner, in millimetres.
const FAR: f64 = 10_000.0;
/// Most pages on one sheet side and most sides the editor makes (the engine allows more).
const MAX_ON_SIDE: usize = 500;
const MAX_SIDES: usize = 200;
/// How near an edge must be to pull a dragged page to it, in screen pixels.
const SNAP_PX: f64 = 8.0;

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// `v` kept finite and within [`FAR`] of the corner: nothing typed or dragged may poison a layout.
fn tame(v: f64) -> f64 {
    if v.is_finite() { v.clamp(-FAR, FAR) } else { 0.0 }
}

/// One page placed on a sheet side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    /// 0-based page of the document.
    pub page: usize,
    /// Lower-left corner of the page's footprint (turned, bleed included) from the sheet's, mm.
    pub x: f64,
    pub y: f64,
    /// Quarter turns anticlockwise, 0 to 3.
    pub turns: u8,
}

/// How a placed page stands with the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    Fine,
    /// Its bleed runs over another page's finished area, or it hangs off the sheet.
    Warn,
    /// Its finished area is over another page's.
    Bad,
}

/// What is wrong on a sheet side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problems {
    /// One per placed page, in order.
    pub flags: Vec<Flag>,
    /// Pages partly off the sheet (cropped at its edge).
    pub off: usize,
}

impl Problems {
    pub fn count(&self, f: Flag) -> usize {
        self.flags.iter().filter(|&&x| x == f).count()
    }
}

/// A line the dragged page snapped to, in sheet millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Guide {
    X(f64),
    Y(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Drag {
    i: usize,
    /// Where in the page the pointer took hold, from the page's lower-left corner, mm.
    grab: (f64, f64),
}

/// The editor's state. Lengths are in millimetres.
#[derive(Clone, Debug, PartialEq)]
pub struct ManualDraft {
    /// The document the layout is for; another document starts a fresh layout.
    doc: Option<DocId>,
    /// Index into [`PRESETS`], or `PRESETS.len()` for a custom size.
    pub preset: usize,
    /// Custom sheet, portrait.
    pub custom: (f64, f64),
    pub landscape: bool,
    /// Bleed already inside each page of the document, on every side.
    pub bleed: f64,
    pub marks: bool,
    pub snap: bool,
    /// The pages on each sheet side, in output order; never empty.
    pub sheets: Vec<Vec<Cell>>,
    pub current: usize,
    pub sel: Option<usize>,
    /// The page "Add to sheet" and "Fill" take (0-based).
    pub add_page: usize,
    drag: Option<Drag>,
    pub guides: Vec<Guide>,
}

impl Default for ManualDraft {
    fn default() -> Self {
        Self {
            doc: None,
            preset: 0,
            custom: (320.0, 480.0),
            landscape: true,
            bleed: 0.0,
            marks: true,
            snap: true,
            sheets: vec![Vec::new()],
            current: 0,
            sel: None,
            add_page: 0,
            drag: None,
            guides: Vec::new(),
        }
    }
}

fn size_of(sizes: &[(f64, f64)], page: usize) -> (f64, f64) {
    sizes.get(page).copied().unwrap_or((10.0, 10.0))
}

/// Width and height of a placed page's footprint (turned).
fn footprint(sizes: &[(f64, f64)], c: &Cell) -> (f64, f64) {
    let (w, h) = size_of(sizes, c.page);
    if c.turns.is_multiple_of(2) { (w, h) } else { (h, w) }
}

fn rect_of(sizes: &[(f64, f64)], c: &Cell) -> [f64; 4] {
    let (fw, fh) = footprint(sizes, c);
    [c.x, c.y, c.x + fw, c.y + fh]
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] - SLACK && b[0] < a[2] - SLACK && a[1] < b[3] - SLACK && b[1] < a[3] - SLACK
}

/// Where an edge of a moving page (`pos` its lower edge, `size` its extent, `b` its bleed) can be
/// pulled to meet a target within `tol`: the move to make and the line met.
fn snap_axis(pos: f64, size: f64, b: f64, targets: &[f64], tol: f64) -> Option<(f64, f64)> {
    let mine = [0.0, size, b, size - b, size / 2.0];
    let mut best: Option<(f64, f64)> = None;
    for m in mine {
        for &t in targets {
            let delta = t - (pos + m);
            if delta.abs() <= tol && best.is_none_or(|(d, _)| delta.abs() < d.abs()) {
                best = Some((delta, t));
            }
        }
    }
    best
}

fn to_points(sizes_mm: &[(f64, f64)]) -> Vec<(f64, f64)> {
    sizes_mm.iter().map(|&(w, h)| (w * MM, h * MM)).collect()
}

/// A placement from step & repeat as a cell for page `page`.
fn cell_from(p: &Placement, page: usize) -> Cell {
    let [a, b, ..] = p.matrix.0;
    let turns = if a.abs() > 0.5 {
        if a > 0.0 { 0 } else { 2 }
    } else if b > 0.0 {
        1
    } else {
        3
    };
    let f = footprint_pt(p.matrix.0, p.clip);
    Cell { page, x: round2(f[0] / MM), y: round2(f[1] / MM), turns }
}

impl ManualDraft {
    /// The sheet in millimetres, as it lies (landscape or portrait).
    pub fn sheet(&self) -> (f64, f64) {
        let (w, h) = match PRESETS.get(self.preset) {
            Some(&(_, w, h)) => (w, h),
            None => self.custom,
        };
        let (short, long) = (w.min(h).max(1.0), w.max(h).max(1.0));
        if self.landscape { (long, short) } else { (short, long) }
    }

    /// Keep the invariants: a sheet side exists and `current` is one of them.
    fn fix(&mut self) {
        if self.sheets.is_empty() {
            self.sheets.push(Vec::new());
        }
        self.current = self.current.min(self.sheets.len() - 1);
    }

    /// Tie the draft to document `id` of `pages` pages: another document starts empty, and a
    /// page the document does not have is dropped.
    pub fn attach(&mut self, id: DocId, pages: usize) {
        if self.doc != Some(id) {
            self.doc = Some(id);
            self.sheets = vec![Vec::new()];
            self.current = 0;
            self.sel = None;
            self.add_page = 0;
            self.drag = None;
            self.guides.clear();
        }
        for side in &mut self.sheets {
            side.retain(|c| c.page < pages);
        }
        self.add_page = self.add_page.min(pages.saturating_sub(1));
        self.fix();
        let n = self.side().len();
        if self.sel.is_some_and(|i| i >= n) {
            self.sel = None;
        }
    }

    pub fn side(&self) -> &[Cell] {
        self.sheets.get(self.current).map_or(&[], Vec::as_slice)
    }

    fn side_mut(&mut self) -> &mut Vec<Cell> {
        self.fix();
        let i = self.current;
        // `fix` leaves at least one side and `current` inside them.
        &mut self.sheets[i]
    }

    /// What is wrong on the current side: finished areas over each other, bleed over another
    /// page's finished area, pages off the sheet.
    pub fn problems(&self, sizes: &[(f64, f64)]) -> Problems {
        let (w, h) = self.sheet();
        let b = self.bleed;
        let rects: Vec<[f64; 4]> = self.side().iter().map(|c| rect_of(sizes, c)).collect();
        let trims: Vec<[f64; 4]> = rects.iter().map(|r| [r[0] + b, r[1] + b, r[2] - b, r[3] - b]).collect();
        let mut flags = vec![Flag::Fine; rects.len()];
        let mut off = 0;
        for (i, r) in rects.iter().enumerate() {
            if r[0] < -SLACK || r[1] < -SLACK || r[2] > w + SLACK || r[3] > h + SLACK {
                off += 1;
                flags[i] = Flag::Warn;
            }
            for (j, t) in trims.iter().enumerate() {
                if i == j {
                    continue;
                }
                if overlaps(trims[i], *t) {
                    flags[i] = Flag::Bad;
                    flags[j] = Flag::Bad;
                } else if overlaps(*r, *t) && flags[i] == Flag::Fine {
                    flags[i] = Flag::Warn;
                }
            }
        }
        Problems { flags, off }
    }

    /// A free spot for a footprint of `fw` × `fh`: beside the pages already there, else the
    /// middle of the sheet.
    pub fn free_spot(&self, sizes: &[(f64, f64)], fw: f64, fh: f64) -> (f64, f64) {
        let (w, h) = self.sheet();
        let rects: Vec<[f64; 4]> = self.side().iter().map(|c| rect_of(sizes, c)).collect();
        let mut cands = vec![(5.0, h - 5.0 - fh)];
        for r in &rects {
            cands.extend([(r[2], r[1]), (r[0], r[1] - fh), (r[0], r[3]), (r[0] - fw, r[1])]);
        }
        cands.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.total_cmp(&b.0)));
        for (x, y) in cands {
            let r = [x, y, x + fw, y + fh];
            let inside = r[0] >= -SLACK && r[1] >= -SLACK && r[2] <= w + SLACK && r[3] <= h + SLACK;
            if inside && !rects.iter().any(|o| overlaps(*o, r)) {
                return (round2(x), round2(y));
            }
        }
        (round2((w - fw) / 2.0), round2((h - fh) / 2.0))
    }

    /// Put page `page` on the current side at a free spot and select it. `false`: the side is full.
    pub fn add(&mut self, sizes: &[(f64, f64)], page: usize) -> bool {
        if page >= sizes.len() || self.side().len() >= MAX_ON_SIDE {
            return false;
        }
        let mut c = Cell { page, x: 0.0, y: 0.0, turns: 0 };
        let (fw, fh) = footprint(sizes, &c);
        (c.x, c.y) = self.free_spot(sizes, fw, fh);
        let side = self.side_mut();
        side.push(c);
        self.sel = Some(self.side().len() - 1);
        true
    }

    /// Fill the current side with the chosen page, as step & repeat would (5 mm edge, pages
    /// turned where that fits more).
    pub fn seed(&mut self, sizes: &[(f64, f64)]) -> Result<(), String> {
        let page = self.add_page;
        let size = *sizes.get(page).ok_or_else(|| "The document has no such page".to_owned())?;
        let (w, h) = self.sheet();
        let mut s = StepRepeat::new((w * MM, h * MM));
        s.margins = Margins::uniform(5.0 * MM);
        s.bleed = self.bleed * MM;
        s.rotate = true;
        s.fill = Fill::Repeat;
        let sheets = impose::step_and_repeat(&to_points(&[size]), &s).map_err(|e| e.to_string())?;
        let first = sheets.first().ok_or_else(|| "Nothing fits on the sheet".to_owned())?;
        if first.placed.len() > MAX_ON_SIDE {
            return Err(format!("More than {MAX_ON_SIDE} pages on a sheet"));
        }
        let cells: Vec<Cell> = first.placed.iter().map(|p| cell_from(p, page)).collect();
        *self.side_mut() = cells;
        self.sel = None;
        Ok(())
    }

    pub fn clear(&mut self) {
        self.side_mut().clear();
        self.sel = None;
    }

    pub fn sheet_add(&mut self) {
        if self.sheets.len() >= MAX_SIDES {
            return;
        }
        self.fix();
        self.sheets.insert(self.current + 1, Vec::new());
        self.current += 1;
        self.sel = None;
    }

    pub fn sheet_dup(&mut self) {
        if self.sheets.len() >= MAX_SIDES {
            return;
        }
        let copy = self.side().to_vec();
        self.fix();
        self.sheets.insert(self.current + 1, copy);
        self.current += 1;
        self.sel = None;
    }

    pub fn sheet_del(&mut self) {
        self.fix();
        if self.sheets.len() == 1 {
            self.sheets = vec![Vec::new()];
        } else {
            self.sheets.remove(self.current);
        }
        self.fix();
        self.sel = None;
    }

    pub fn go(&mut self, to: usize) {
        self.fix();
        self.current = to.min(self.sheets.len() - 1);
        self.sel = None;
        self.drag = None;
        self.guides.clear();
    }

    fn sel_cell(&mut self) -> Option<&mut Cell> {
        let i = self.sel?;
        self.fix();
        let cur = self.current;
        self.sheets.get_mut(cur)?.get_mut(i)
    }

    /// Turn the selected page a quarter turn about its centre.
    pub fn rotate(&mut self, sizes: &[(f64, f64)]) {
        let Some(c) = self.sel_cell() else { return };
        let r = rect_of(sizes, c);
        let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
        c.turns = (c.turns + 1) % 4;
        let (fw, fh) = footprint(sizes, c);
        c.x = round2(cx - fw / 2.0);
        c.y = round2(cy - fh / 2.0);
    }

    /// Copy the selected page to a free spot and select the copy.
    pub fn duplicate(&mut self, sizes: &[(f64, f64)]) {
        let Some(&c) = self.sel_cell().map(|c| &*c) else { return };
        if self.side().len() >= MAX_ON_SIDE {
            return;
        }
        let (fw, fh) = footprint(sizes, &c);
        let (x, y) = self.free_spot(sizes, fw, fh);
        self.side_mut().push(Cell { x, y, ..c });
        self.sel = Some(self.side().len() - 1);
    }

    pub fn delete_selected(&mut self) {
        if let Some(i) = self.sel
            && i < self.side().len()
        {
            self.side_mut().remove(i);
        }
        self.sel = None;
    }

    /// Move the selected page by `dx`, `dy` mm.
    pub fn nudge(&mut self, dx: f64, dy: f64) {
        if let Some(c) = self.sel_cell() {
            c.x = round2(tame(c.x + dx));
            c.y = round2(tame(c.y + dy));
        }
    }

    /// The selected page's finished area as a ruler reads it: its distance from the sheet's left
    /// and top edges.
    pub fn trim_from_edges(&self, sizes: &[(f64, f64)]) -> Option<(f64, f64)> {
        let c = self.side().get(self.sel?)?;
        let (_, fh) = footprint(sizes, c);
        let (_, h) = self.sheet();
        Some((round2(c.x + self.bleed), round2(h - (c.y + fh) + self.bleed)))
    }

    pub fn set_trim_left(&mut self, v: f64) {
        let b = self.bleed;
        if let Some(c) = self.sel_cell() {
            c.x = round2(tame(v - b));
        }
    }

    pub fn set_trim_top(&mut self, sizes: &[(f64, f64)], v: f64) {
        let (b, (_, h)) = (self.bleed, self.sheet());
        if let Some(c) = self.sel_cell() {
            let (_, fh) = footprint(sizes, c);
            c.y = round2(tame(h - v + b - fh));
        }
    }

    /// The page on top at `pt` (sheet millimetres), if any.
    pub fn hit(&self, sizes: &[(f64, f64)], pt: (f64, f64)) -> Option<usize> {
        self.side().iter().enumerate().rev().find_map(|(i, c)| {
            let r = rect_of(sizes, c);
            (pt.0 >= r[0] && pt.0 <= r[2] && pt.1 >= r[1] && pt.1 <= r[3]).then_some(i)
        })
    }

    /// Take hold of the page `i` at `pt`.
    fn grab(&mut self, i: usize, pt: (f64, f64)) {
        if let Some(c) = self.side().get(i) {
            self.drag = Some(Drag { i, grab: (pt.0 - c.x, pt.1 - c.y) });
            self.sel = Some(i);
        }
    }

    /// Move the page being dragged so the pointer, at `pt`, stays where it took hold; with `snap`
    /// an edge within `tol` mm of the sheet's or another page's edges is pulled onto it.
    pub fn drag_to(&mut self, sizes: &[(f64, f64)], pt: (f64, f64), snap: bool, tol: f64) {
        let Some(Drag { i, grab }) = self.drag else { return };
        let Some(&c) = self.side().get(i) else { return };
        let (pt, tol) = ((tame(pt.0), tame(pt.1)), if tol.is_finite() { tol.max(0.0) } else { 0.0 });
        let (mut x, mut y) = (pt.0 - grab.0, pt.1 - grab.1);
        let (fw, fh) = footprint(sizes, &c);
        let mut guides = Vec::new();
        let (mut snapped_x, mut snapped_y) = (false, false);
        if snap {
            let b = self.bleed;
            let (w, h) = self.sheet();
            let (mut tx, mut ty) = (vec![0.0, w, w / 2.0], vec![0.0, h, h / 2.0]);
            for (j, o) in self.side().iter().enumerate() {
                if j != i {
                    let r = rect_of(sizes, o);
                    tx.extend([r[0], r[2], r[0] + b, r[2] - b]);
                    ty.extend([r[1], r[3], r[1] + b, r[3] - b]);
                }
            }
            if let Some((delta, line)) = snap_axis(x, fw, b, &tx, tol) {
                x += delta;
                snapped_x = true;
                guides.push(Guide::X(line));
            }
            if let Some((delta, line)) = snap_axis(y, fh, b, &ty, tol) {
                y += delta;
                snapped_y = true;
                guides.push(Guide::Y(line));
            }
        }
        // Free movement goes by tenths of a millimetre; a snapped edge stays exactly on its line.
        let (x, y) = (tame(x), tame(y));
        let (x, y) = (if snapped_x { round2(x) } else { (x * 10.0).round() / 10.0 }, if snapped_y { round2(y) } else { (y * 10.0).round() / 10.0 });
        if let Some(c) = self.side_mut().get_mut(i) {
            c.x = x;
            c.y = y;
        }
        self.guides = guides;
    }

    fn drop_page(&mut self) {
        self.drag = None;
        self.guides.clear();
    }

    /// The layout for the engine (points).
    pub fn spec(&self) -> Manual {
        let (w, h) = self.sheet();
        Manual {
            sheet: (w * MM, h * MM),
            bleed: self.bleed.max(0.0) * MM,
            marks: self.marks.then(Marks::default),
            sheets: self
                .sheets
                .iter()
                .map(|side| side.iter().map(|c| ManualPage { page: c.page, at: (c.x * MM, c.y * MM), turns: c.turns }).collect())
                .collect(),
        }
    }

    /// Whether the engine takes the layout, and the reason when it does not.
    pub fn check(&self, sizes: &[(f64, f64)]) -> Result<(), String> {
        if self.sheets.iter().all(Vec::is_empty) {
            return Err(crate::i18n::t("Put at least one page on a sheet").to_owned());
        }
        impose::manual(&to_points(sizes), &self.spec()).map(|_| ()).map_err(|e| e.to_string())
    }
}

/// Which way the keyboard moved things, collected while the input is borrowed.
enum Key {
    Nudge(f64, f64),
    Delete,
    Rotate,
    Duplicate,
}

fn keys(ui: &mut egui::Ui) -> Vec<Key> {
    use egui::{Key as K, Modifiers as M};
    let mut out = Vec::new();
    ui.input_mut(|i| {
        for (m, step) in [(M::SHIFT, 5.0), (M::NONE, 0.5)] {
            for (key, dx, dy) in [(K::ArrowLeft, -1.0, 0.0), (K::ArrowRight, 1.0, 0.0), (K::ArrowUp, 0.0, 1.0), (K::ArrowDown, 0.0, -1.0)] {
                if i.consume_key(m, key) {
                    out.push(Key::Nudge(dx * step, dy * step));
                }
            }
        }
        for key in [K::Delete, K::Backspace] {
            if i.consume_key(M::NONE, key) {
                out.push(Key::Delete);
            }
        }
        if i.consume_key(M::NONE, K::R) {
            out.push(Key::Rotate);
        }
        if i.consume_key(M::NONE, K::D) {
            out.push(Key::Duplicate);
        }
    });
    out
}

/// The sheet drawn to scale; the pointer and keys edit the draft.
fn canvas(ui: &mut egui::Ui, d: &mut ManualDraft, sizes: &[(f64, f64)]) {
    let (sw, sh) = d.sheet();
    let k = ((430.0 / sw).min(320.0 / sh)) as f32;
    let (rect, resp) = ui.allocate_exact_size(vec2((sw as f32 * k).max(1.0), (sh as f32 * k).max(1.0)), Sense::click_and_drag());
    let to_mm = |p: egui::Pos2| (f64::from((p.x - rect.left()) / k), f64::from((rect.bottom() - p.y) / k));
    let to_px = |x: f64, y: f64| pos2(rect.left() + x as f32 * k, rect.bottom() - y as f32 * k);

    if resp.drag_started() || resp.clicked() {
        resp.request_focus();
        if let Some(p) = resp.interact_pointer_pos() {
            let pt = to_mm(p);
            match d.hit(sizes, pt) {
                Some(i) if resp.drag_started() => d.grab(i, pt),
                hit => d.sel = hit,
            }
        }
    }
    if resp.dragged()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let snap = d.snap && !ui.input(|i| i.modifiers.alt);
        d.drag_to(sizes, to_mm(p), snap, SNAP_PX / f64::from(k));
    }
    if resp.drag_stopped() {
        d.drop_page();
    }
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(resp.id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() });
    });
    if resp.has_focus() {
        for key in keys(ui) {
            match key {
                Key::Nudge(dx, dy) => d.nudge(dx, dy),
                Key::Delete => d.delete_selected(),
                Key::Rotate => d.rotate(sizes),
                Key::Duplicate => d.duplicate(sizes),
            }
        }
    }
    if resp.hovered() {
        let over = resp.hover_pos().is_some_and(|p| d.hit(sizes, to_mm(p)).is_some());
        ui.ctx().set_cursor_icon(if d.drag.is_some() {
            egui::CursorIcon::Grabbing
        } else if over {
            egui::CursorIcon::Grab
        } else {
            egui::CursorIcon::Default
        });
    }

    let p = ui.painter_at(rect);
    p.rect_filled(rect, CornerRadius::ZERO, Color32::WHITE);
    let problems = d.problems(sizes);
    let b = d.bleed;
    let ink = Color32::from_rgb(0x00, 0x40, 0x80);
    let selected = Color32::from_rgb(0xF0, 0x40, 0x00);
    for (i, c) in d.side().iter().enumerate() {
        let r = rect_of(sizes, c);
        let cell = Rect::from_two_pos(to_px(r[0], r[1]), to_px(r[2], r[3]));
        let fill = match problems.flags.get(i) {
            Some(Flag::Bad) => Color32::from_rgba_unmultiplied(180, 35, 24, 56),
            Some(Flag::Warn) => Color32::from_rgba_unmultiplied(240, 208, 80, 90),
            _ => Color32::from_rgb(0xE8, 0xEE, 0xF6),
        };
        p.rect_filled(cell, CornerRadius::ZERO, fill);
        if b > 0.0 {
            p.rect_stroke(cell, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_rgb(0x7A, 0x88, 0x99)), egui::StrokeKind::Inside);
        }
        let trim = Rect::from_two_pos(to_px(r[0] + b, r[1] + b), to_px(r[2] - b, r[3] - b));
        let on = d.sel == Some(i);
        p.rect_stroke(trim, CornerRadius::ZERO, Stroke::new(if on { 2.5 } else { 1.0 }, if on { selected } else { ink }), egui::StrokeKind::Inside);
        p.text(trim.center(), egui::Align2::CENTER_CENTER, (c.page + 1).to_string(), theme::semibold(14.0), ink);
        // A bar along the page's head shows which way up it stands: turned a quarter anticlockwise
        // the head is on the left, and so on.
        let (a, z) = match c.turns % 4 {
            0 => (trim.left_top(), trim.right_top()),
            1 => (trim.left_bottom(), trim.left_top()),
            2 => (trim.left_bottom(), trim.right_bottom()),
            _ => (trim.right_bottom(), trim.right_top()),
        };
        let inward = match c.turns % 4 {
            0 => vec2(0.0, 4.0),
            1 => vec2(4.0, 0.0),
            2 => vec2(0.0, -4.0),
            _ => vec2(-4.0, 0.0),
        };
        p.line_segment([a + inward, z + inward], Stroke::new(3.0, ink));
    }
    // Marks as the engine draws them, when it takes the layout.
    if let Ok(sheets) = impose::manual(&to_points(sizes), &d.spec())
        && let Some(s) = sheets.get(d.current)
    {
        for l in &s.lines {
            p.line_segment([to_px(l[0] / MM, l[1] / MM), to_px(l[2] / MM, l[3] / MM)], Stroke::new(0.8, Color32::BLACK));
        }
    }
    for g in &d.guides {
        let seg = match *g {
            Guide::X(x) => [to_px(x, 0.0), to_px(x, sh)],
            Guide::Y(y) => [to_px(0.0, y), to_px(sw, y)],
        };
        p.line_segment(seg, Stroke::new(1.0, selected));
    }
    // The sheet's edge must show against a white dialog.
    p.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.5, Color32::from_gray(140)), egui::StrokeKind::Inside);
}

fn mm_field(ui: &mut egui::Ui, v: &mut f64, range: std::ops::RangeInclusive<f64>) -> egui::Response {
    ui.add(egui::DragValue::new(v).speed(0.1).range(range).max_decimals(2).suffix(" mm"))
}

/// The sheet and bleed controls.
fn sheet_controls(ui: &mut egui::Ui, d: &mut ManualDraft) {
    egui::Grid::new("manual-sheet").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
        ui.label(tl!("Sheet"));
        let label = PRESETS.get(d.preset).map_or(tl!("Custom"), |p| p.0);
        craft_ui::ComboBox::from_id_salt("manual-preset").selected_text(label).show_ui(ui, |ui| {
            for (i, (name, ..)) in PRESETS.iter().enumerate() {
                ui.selectable_value(&mut d.preset, i, *name);
            }
            ui.selectable_value(&mut d.preset, PRESETS.len(), tl!("Custom"));
        });
        ui.end_row();
        if d.preset >= PRESETS.len() {
            ui.label(tl!("Width"));
            mm_field(ui, &mut d.custom.0, 50.0..=2000.0);
            ui.end_row();
            ui.label(tl!("Height"));
            mm_field(ui, &mut d.custom.1, 50.0..=2000.0);
            ui.end_row();
        }
        ui.label(tl!("Direction"));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut d.landscape, true, tl!("Wide"));
            ui.selectable_value(&mut d.landscape, false, tl!("Tall"));
        });
        ui.end_row();
        ui.label(tl!("Bleed in the file"));
        mm_field(ui, &mut d.bleed, 0.0..=50.0);
        ui.end_row();
    });
    ui.add(widgets::checkbox(&mut d.marks, tl!("Trim marks")));
    ui.add(widgets::checkbox(&mut d.snap, tl!("Snap to the sheet and other pages (Alt: off)")));
}

/// The selected page's position and the buttons that act on it.
fn selection_controls(ui: &mut egui::Ui, d: &mut ManualDraft, sizes: &[(f64, f64)]) {
    let Some(c) = d.sel.and_then(|i| d.side().get(i).copied()) else { return };
    let (fw, fh) = footprint(sizes, &c);
    ui.separator();
    ui.label(
        egui::RichText::new(crate::i18n::fmt(
            tl!("Page {n} · turned {deg}° · {w} × {h} mm"),
            &[
                ("n", &(c.page + 1).to_string()),
                ("deg", &(u32::from(c.turns) * 90).to_string()),
                ("w", &format!("{fw:.1}")),
                ("h", &format!("{fh:.1}")),
            ],
        ))
        .strong(),
    );
    if let Some((mut left, mut top)) = d.trim_from_edges(sizes) {
        egui::Grid::new("manual-pos").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
            ui.label(tl!("From the left (trim)"));
            if mm_field(ui, &mut left, -FAR..=FAR).changed() {
                d.set_trim_left(left);
            }
            ui.end_row();
            ui.label(tl!("From the top (trim)"));
            if mm_field(ui, &mut top, -FAR..=FAR).changed() {
                d.set_trim_top(sizes, top);
            }
            ui.end_row();
        });
    }
    ui.horizontal(|ui| {
        if ui.button(tl!("Turn 90°")).clicked() {
            d.rotate(sizes);
        }
        if ui.button(tl!("Duplicate")).clicked() {
            d.duplicate(sizes);
        }
        if ui.button(tl!("Delete")).clicked() {
            d.delete_selected();
        }
    });
}

/// What the current side comes to, in a line.
fn status(ui: &mut egui::Ui, d: &ManualDraft, sizes: &[(f64, f64)]) {
    let p = d.problems(sizes);
    let n = d.side().len();
    let (bad, warn) = (p.count(Flag::Bad), p.count(Flag::Warn));
    let (text, color) = if bad > 0 {
        (
            crate::i18n::fmt(tl!("{n} pages have finished areas over each other (red)."), &[("n", &bad.to_string())]),
            Color32::from_rgb(0xc0, 0x39, 0x2b),
        )
    } else if warn > 0 {
        let mut s =
            crate::i18n::fmt(tl!("{n} pages: bleed over another page's finished area, or off the sheet (yellow)."), &[("n", &warn.to_string())]);
        if p.off > 0 {
            s.push(' ');
            s.push_str(tl!("What is off the sheet is cropped at its edge."));
        }
        (s, Color32::from_rgb(0xa6, 0x7c, 0x00))
    } else if n > 0 {
        (crate::i18n::fmt(tl!("{n} pages on this sheet, none over another."), &[("n", &n.to_string())]), ui.visuals().text_color())
    } else {
        (tl!("Empty sheet. Add pages, or fill it like step and repeat.").to_owned(), ui.visuals().weak_text_color())
    };
    ui.label(egui::RichText::new(text).small().color(color));
}

/// Draw the editor; returns (finish, cancel).
pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp) -> (bool, bool) {
    let Some((_, id)) = app.active_ids() else { return (false, true) };
    let Some(doc) = app.session.get(id) else { return (false, true) };
    let sizes: Vec<(f64, f64)> = doc.info.pages.iter().map(|p| (f64::from(p.width) / MM, f64::from(p.height) / MM)).collect();
    let d = &mut app.manual_draft;
    d.attach(id, sizes.len());
    let mut seed_error = None;
    ui.label(egui::RichText::new(tl!("Manual")).font(theme::semibold(18.0)));
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(440.0);
            ui.horizontal(|ui| {
                let (cur, n) = (d.current, d.sheets.len());
                if ui.add_enabled(cur > 0, egui::Button::new("◀")).clicked() {
                    d.go(cur - 1);
                }
                ui.label(crate::i18n::fmt(tl!("Sheet side {n} of {of}"), &[("n", &(cur + 1).to_string()), ("of", &n.to_string())]));
                if ui.add_enabled(cur + 1 < n, egui::Button::new("▶")).clicked() {
                    d.go(cur + 1);
                }
            });
            canvas(ui, d, &sizes);
            status(ui, d, &sizes);
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_width(220.0);
            sheet_controls(ui, d);
            ui.separator();
            ui.horizontal(|ui| {
                let pages = sizes.len();
                craft_ui::ComboBox::from_id_salt("manual-page")
                    .selected_text(crate::i18n::fmt(tl!("Page {n}"), &[("n", &(d.add_page + 1).to_string())]))
                    .show_ui(ui, |ui| {
                        for i in 0..pages {
                            ui.selectable_value(&mut d.add_page, i, crate::i18n::fmt(tl!("Page {n}"), &[("n", &(i + 1).to_string())]));
                        }
                    });
                if ui.button(tl!("Add to sheet")).clicked() {
                    d.add(&sizes, d.add_page);
                }
            });
            if ui.button(tl!("Fill like step and repeat")).clicked()
                && let Err(e) = d.seed(&sizes)
            {
                seed_error = Some(e);
            }
            if ui.button(tl!("Empty this sheet side")).clicked() {
                d.clear();
            }
            ui.horizontal(|ui| {
                if ui.button(tl!("+ Side")).clicked() {
                    d.sheet_add();
                }
                if ui.button(tl!("Copy side")).clicked() {
                    d.sheet_dup();
                }
                if ui.button(tl!("Delete side")).clicked() {
                    d.sheet_del();
                }
            });
            selection_controls(ui, d, &sizes);
            ui.add_space(6.0);
            ui.label(egui::RichText::new(tl!("Drag a page. Arrows nudge 0.5 mm (Shift 5 mm), R turns, D copies, Delete removes.")).small().weak());
        });
    });
    if let Some(e) = seed_error {
        app.notify_error(e);
    }
    ui.add_space(12.0);
    let d = &app.manual_draft;
    let verdict = d.check(&sizes);
    if let Err(e) = &verdict {
        ui.label(egui::RichText::new(e).small().color(Color32::from_rgb(0xc0, 0x39, 0x2b)));
    }
    let (mut finish, mut cancel) = (false, false);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if ui.add_enabled_ui(verdict.is_ok(), |ui| widgets::pill_button(ui, tl!("Finish"), true)).inner.clicked() {
            finish = true;
        }
        if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
            cancel = true;
        }
    });
    (finish, cancel)
}

/// Make the imposed document and open it in a new tab.
pub(crate) fn finish(app: &mut PdfCraftApp) {
    let Some((_, id)) = app.active_ids() else { return };
    let Some(doc) = app.session.get(id) else { return };
    let stem = doc.name.strip_suffix(".pdf").or_else(|| doc.name.strip_suffix(".PDF")).unwrap_or(&doc.name).to_owned();
    let content = pdfcraft_engine::print::Content::DocumentAndMarkups;
    match app.session.manual_pdf(id, &app.manual_draft.spec(), content) {
        Ok(bytes) => {
            if let Err(e) = app.open_bytes(&format!("{stem}_MANUAL.pdf"), None, bytes) {
                app.notify_error(e);
            }
        }
        Err(e) => app.notify_error(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A5 pages, 148 × 210 mm.
    fn pages(n: usize) -> Vec<(f64, f64)> {
        vec![(148.0, 210.0); n]
    }

    fn at(page: usize, x: f64, y: f64) -> Cell {
        Cell { page, x, y, turns: 0 }
    }

    /// A draft with a custom 320 × 480 mm sheet lying wide, holding `cells`.
    fn draft(cells: Vec<Cell>) -> ManualDraft {
        ManualDraft { preset: PRESETS.len(), custom: (320.0, 480.0), sheets: vec![cells], ..Default::default() }
    }

    #[test]
    fn the_sheet_lies_as_asked_whatever_order_the_size_was_typed() {
        let mut d = draft(vec![]);
        d.custom = (480.0, 320.0);
        assert_eq!(d.sheet(), (480.0, 320.0));
        d.landscape = false;
        assert_eq!(d.sheet(), (320.0, 480.0));
    }

    #[test]
    fn finished_areas_over_each_other_are_bad_and_bleed_over_a_neighbour_only_a_warning() {
        let s = pages(2);
        let mut d = draft(vec![at(0, 10.0, 10.0), at(1, 100.0, 10.0)]);
        assert_eq!(d.problems(&s).flags, [Flag::Bad, Flag::Bad], "the pages share 58 mm");
        d.sheets[0][1].x = 158.0;
        assert_eq!(d.problems(&s).flags, [Flag::Fine, Flag::Fine], "side by side");
        d.bleed = 3.0;
        d.sheets[0][1].x = 150.0;
        assert_eq!(d.problems(&s).flags, [Flag::Bad, Flag::Bad], "the finished areas overlap by 2 mm");
        d.sheets[0][1].x = 152.0;
        assert_eq!(d.problems(&s).flags, [Flag::Warn, Flag::Warn], "finished areas touch, bleeds run over them");
        d.sheets[0][1].x = 155.0;
        assert_eq!(d.problems(&s).flags, [Flag::Fine, Flag::Fine], "bleeds touch, finished areas apart");
    }

    #[test]
    fn a_page_off_the_sheet_is_a_warning_and_counted() {
        let s = pages(1);
        let d = draft(vec![at(0, 400.0, 10.0)]);
        let p = d.problems(&s);
        assert_eq!((p.flags, p.off), (vec![Flag::Warn], 1));
    }

    #[test]
    fn a_dragged_page_snaps_to_a_neighbours_edge_within_the_distance_and_not_beyond() {
        let s = pages(2);
        let mut d = draft(vec![at(0, 20.0, 20.0), at(1, 300.0, 200.0)]);
        d.drag = Some(Drag { i: 1, grab: (0.0, 0.0) });
        // 1.5 mm short of the first page's right edge (168 mm): pulled to it.
        d.drag_to(&s, (169.5, 200.0), true, 3.0);
        assert_eq!(d.sheets[0][1].x, 168.0);
        assert!(d.guides.contains(&Guide::X(168.0)));
        // 5 mm away with 3 mm to spare: left where it is, by tenths of a millimetre.
        d.drag_to(&s, (173.04, 200.0), true, 3.0);
        assert_eq!(d.sheets[0][1].x, 173.0);
        assert!(!d.guides.iter().any(|g| matches!(g, Guide::X(_))));
        // Snap off: never pulled.
        d.drag_to(&s, (169.5, 200.0), false, 3.0);
        assert_eq!(d.sheets[0][1].x, 169.5);
    }

    #[test]
    fn a_page_snaps_to_the_sheets_edge_and_its_middle() {
        let s = pages(1);
        let mut d = draft(vec![at(0, 50.0, 50.0)]);
        d.drag = Some(Drag { i: 0, grab: (0.0, 0.0) });
        d.drag_to(&s, (1.0, 1.0), true, 3.0);
        assert_eq!((d.sheets[0][0].x, d.sheets[0][0].y), (0.0, 0.0));
        let (w, _) = d.sheet();
        d.drag_to(&s, (w / 2.0 - 74.0 + 1.0, 100.0), true, 3.0);
        assert_eq!(d.sheets[0][0].x, w / 2.0 - 74.0, "centred on the sheet");
    }

    #[test]
    fn a_trim_edge_snaps_to_a_neighbours_trim_edge_when_the_files_carry_bleed() {
        let s = pages(2);
        let mut d = draft(vec![at(0, 10.0, 10.0), at(1, 300.0, 10.0)]);
        d.bleed = 3.0;
        d.drag = Some(Drag { i: 1, grab: (0.0, 0.0) });
        // The first page's trim ends at 155 mm; the second's trim starts 3 mm in from its own edge.
        d.drag_to(&s, (152.8, 10.0), true, 1.0);
        assert_eq!(d.sheets[0][1].x, 152.0, "its trim edge (x + 3) meets 155");
    }

    #[test]
    fn nothing_dragged_or_typed_can_make_a_position_non_finite() {
        let s = pages(1);
        let mut d = draft(vec![at(0, 50.0, 50.0)]);
        d.drag = Some(Drag { i: 0, grab: (0.0, 0.0) });
        d.drag_to(&s, (f64::NAN, f64::INFINITY), true, f64::NAN);
        let c = d.sheets[0][0];
        assert!(c.x.is_finite() && c.y.is_finite());
        d.sel = Some(0);
        d.set_trim_left(f64::NEG_INFINITY);
        d.set_trim_top(&s, f64::NAN);
        d.nudge(f64::NAN, f64::INFINITY);
        let c = d.sheets[0][0];
        assert!(c.x.is_finite() && c.y.is_finite() && c.x.abs() <= FAR && c.y.abs() <= FAR);
    }

    #[test]
    fn a_page_is_added_beside_the_others_and_when_none_is_free_it_is_flagged() {
        let s = pages(1);
        let mut d = draft(vec![]);
        for _ in 0..3 {
            assert!(d.add(&s, 0));
        }
        assert_eq!(d.problems(&s).count(Flag::Fine), 3, "three upright A5 pages fit a 480 × 320 sheet: {:?}", d.side());
        assert_eq!(d.sel, Some(2), "the new page is selected");
        assert!(d.add(&s, 0), "a fourth has no room: it goes in the middle");
        assert!(d.problems(&s).count(Flag::Bad) > 0, "and is flagged, not hidden");
        assert!(!d.add(&s, 5), "no such page");
    }

    #[test]
    fn turning_a_page_keeps_its_centre_and_swaps_its_footprint() {
        let s = pages(1);
        let mut d = draft(vec![at(0, 10.0, 10.0)]);
        d.sel = Some(0);
        d.rotate(&s);
        let c = d.sheets[0][0];
        assert_eq!(c.turns, 1);
        // Centre (84, 115) before; the footprint is now 210 × 148.
        assert_eq!((c.x, c.y), (-21.0, 41.0));
        for _ in 0..3 {
            d.rotate(&s);
        }
        assert_eq!((d.sheets[0][0].turns, d.sheets[0][0].x, d.sheets[0][0].y), (0, 10.0, 10.0));
    }

    #[test]
    fn the_trim_is_read_from_the_sheets_left_and_top_as_a_ruler_does() {
        let s = pages(1);
        let mut d = draft(vec![at(0, 10.0, 100.0)]);
        d.bleed = 3.0;
        d.sel = Some(0);
        // Sheet 480 × 320: the page's top is at 100 + 210 = 310, trim top 307, so 13 from the top.
        assert_eq!(d.trim_from_edges(&s), Some((13.0, 13.0)));
        d.set_trim_left(20.0);
        d.set_trim_top(&s, 30.0);
        assert_eq!((d.sheets[0][0].x, d.sheets[0][0].y), (17.0, 83.0));
        assert_eq!(d.trim_from_edges(&s), Some((20.0, 30.0)));
    }

    #[test]
    fn a_copy_lands_on_a_free_spot_and_deleting_selects_nothing() {
        let s = pages(1);
        let mut d = draft(vec![at(0, 5.0, 105.0)]);
        d.sel = Some(0);
        d.duplicate(&s);
        assert_eq!(d.side().len(), 2);
        assert_eq!(d.sel, Some(1));
        assert_eq!(d.problems(&s).count(Flag::Fine), 2);
        d.delete_selected();
        assert_eq!((d.side().len(), d.sel), (1, None));
    }

    #[test]
    fn sheet_sides_are_added_copied_and_deleted_and_there_is_always_one() {
        let mut d = draft(vec![at(0, 5.0, 5.0)]);
        d.sheet_add();
        assert_eq!((d.sheets.len(), d.current, d.side().len()), (2, 1, 0));
        d.go(0);
        d.sheet_dup();
        assert_eq!((d.sheets.len(), d.current, d.side().len()), (3, 1, 1));
        d.sheet_del();
        d.sheet_del();
        d.sheet_del();
        assert_eq!((d.sheets.len(), d.current), (1, 0));
        assert!(d.side().is_empty());
        d.go(99);
        assert_eq!(d.current, 0);
    }

    #[test]
    fn a_sheet_may_hold_no_more_than_the_editor_allows() {
        let s = pages(1);
        let mut d = draft(vec![at(0, 0.0, 0.0); MAX_ON_SIDE]);
        assert!(!d.add(&s, 0));
        d.sel = Some(0);
        d.duplicate(&s);
        assert_eq!(d.side().len(), MAX_ON_SIDE);
    }

    #[test]
    fn filling_like_step_and_repeat_places_the_chosen_page_without_overlap() {
        let s = vec![(100.0, 100.0), (148.0, 210.0)];
        let mut d = draft(vec![]);
        d.add_page = 1;
        d.seed(&s).expect("fills");
        assert!(!d.side().is_empty());
        assert!(d.side().iter().all(|c| c.page == 1), "every copy is the chosen page");
        assert_eq!(d.problems(&s).count(Flag::Fine), d.side().len(), "{:?}", d.side());
        // 480 × 320 less 5 mm edges: 3 × 1 upright is 444 × 210; turned rows may add more.
        assert!(d.side().len() >= 3);
        assert!(d.check(&s).is_ok());
    }

    #[test]
    fn filling_with_a_page_the_sheet_cannot_hold_says_so_and_changes_nothing() {
        let s = vec![(900.0, 900.0)];
        let mut d = draft(vec![at(0, 1.0, 1.0)]);
        assert!(d.seed(&s).is_err());
        assert_eq!(d.side().len(), 1);
    }

    #[test]
    fn the_engine_takes_the_layout_in_points_and_refuses_what_cannot_be_made() {
        let s = pages(2);
        let mut d = draft(vec![]);
        assert!(d.check(&s).is_err(), "an empty job is not a job");
        d.sheets[0] = vec![at(0, 10.0, 10.0), Cell { page: 1, x: 200.0, y: 20.0, turns: 1 }];
        let spec = d.spec();
        assert!((spec.sheet.0 - 480.0 * MM).abs() < 1e-9);
        assert_eq!(spec.sheets[0][1].turns, 1);
        assert!((spec.sheets[0][1].at.0 - 200.0 * MM).abs() < 1e-9);
        assert!(d.check(&s).is_ok());
        d.sheets[0][0].x = 5000.0;
        assert!(d.check(&s).is_err(), "wholly off the sheet");
        d.sheets[0][0] = at(7, 10.0, 10.0);
        assert!(d.check(&s).is_err(), "a page the document does not have");
        d.sheets[0][0] = at(0, 10.0, 10.0);
        d.bleed = 80.0;
        assert!(d.check(&s).is_err(), "bleed larger than the page");
    }

    #[test]
    fn marks_come_with_the_job_only_when_asked() {
        let mut d = draft(vec![at(0, 10.0, 10.0)]);
        assert!(d.spec().marks.is_some());
        d.marks = false;
        assert!(d.spec().marks.is_none());
    }

    #[test]
    fn another_document_starts_empty_and_missing_pages_are_dropped() {
        let ids = [DocId(1), DocId(2)];
        let mut d = draft(vec![at(0, 5.0, 5.0), at(3, 200.0, 5.0)]);
        d.sel = Some(1);
        d.add_page = 3;
        d.attach(ids[0], 2);
        assert_eq!(d.side().len(), 0, "a first document starts a fresh layout");
        d.sheets[0] = vec![at(0, 5.0, 5.0), at(1, 200.0, 5.0)];
        d.attach(ids[0], 1);
        assert_eq!(d.side().len(), 1, "page 2 is gone");
        assert_eq!(d.add_page, 0);
        d.attach(ids[1], 5);
        assert!(d.side().is_empty());
    }

    /// A one-page PDF of `w` × `h` points.
    fn blank_pdf(w: u32, h: u32) -> Vec<u8> {
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            format!("<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 {w} {h}] >>"),
            "<< /Type /Page /Parent 2 0 R >>".to_owned(),
        ];
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in offsets {
            pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
        pdf
    }

    /// The menu entry opens the editor on an open document, and Finish makes a new tab holding
    /// one sheet per side, the size of the sheet.
    #[test]
    fn the_menu_entry_opens_the_editor_and_finish_makes_a_sheet_per_side() {
        let mut app = PdfCraftApp::new();
        app.execute("impose.manual");
        assert!(app.dialog.is_none(), "nothing to place without a document");
        app.open_bytes("a.pdf", None, blank_pdf(420, 595)).expect("opens");
        app.execute("impose.manual");
        assert!(matches!(app.dialog, Some(crate::state::Dialog::ImposeManual)));
        let (_, id) = app.active_ids().expect("a document");
        let sizes = [(420.0 / MM, 595.0 / MM)];
        let d = &mut app.manual_draft;
        d.attach(id, 1);
        d.preset = 0;
        d.landscape = true;
        assert!(d.add(&sizes, 0));
        d.sheet_add();
        assert!(d.add(&sizes, 0));
        assert!(d.check(&sizes).is_ok());
        let before = app.views.len();
        finish(&mut app);
        assert_eq!(app.views.len(), before + 1, "the imposed document opens in a new tab");
        let (_, new_id) = app.active_ids().expect("the new tab");
        let doc = app.session.get(new_id).expect("open");
        assert_eq!(doc.name, "a_MANUAL.pdf");
        assert_eq!(doc.info.pages.len(), 2, "one sheet per side");
        let (w, h) = (f64::from(doc.info.pages[0].width), f64::from(doc.info.pages[0].height));
        assert!((w - 480.0 * MM).abs() < 0.5 && (h - 320.0 * MM).abs() < 0.5, "{w} × {h}");
    }

    #[test]
    fn hitting_picks_the_page_on_top() {
        let s = pages(2);
        let d = draft(vec![at(0, 10.0, 10.0), at(1, 100.0, 10.0)]);
        assert_eq!(d.hit(&s, (120.0, 50.0)), Some(1), "overlap: the later page is on top");
        assert_eq!(d.hit(&s, (50.0, 50.0)), Some(0));
        assert_eq!(d.hit(&s, (400.0, 300.0)), None);
    }
}
