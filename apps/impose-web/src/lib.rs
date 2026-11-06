//! `pdfcraft-impose` for the browser.
//!
//! One job description (JSON, millimetres) drives both a layout preview — which needs only page
//! sizes, so the PIXELINDIE POS can simulate a job before any file exists — and the imposed PDF
//! itself, made locally from the user's file. Both come from the same `pdfcraft-impose` geometry,
//! so the numbers on a POS receipt and the sheets in the PDF cannot disagree.
//!
//! The functions below are plain Rust (tested natively); the `wasm` module only adapts them to
//! JavaScript.

#![forbid(unsafe_code)]

use std::sync::Arc;

use pdfcraft_cos::Document;
use pdfcraft_impose::{
    AddBleed, Align, Binding, BleedMethod, Booklet, Duplex, Fill, LastSignature, Manual, ManualPage, Margins, MarkStyle, Marks, StepRepeat,
};
use pdfcraft_print::{Content, Sheet};
use serde::{Deserialize, Serialize};

/// Points per millimetre.
const PT_PER_MM: f64 = 72.0 / 25.4;

fn pt(mm: f64) -> f64 {
    mm * PT_PER_MM
}

fn mm(pt: f64) -> f64 {
    pt / PT_PER_MM
}

/// What to make. Every length is in millimetres.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Job {
    /// `repeat` (step & repeat), `sequential` (n-up), `cut-stack` (unique collate), `booklet`,
    /// `bleed` or `manual`.
    pub mode: String,
    /// Press sheet width and height (portrait or landscape as given).
    pub sheet_mm: Option<[f64; 2]>,
    /// One margin for every side, or `[left, bottom, right, top]`.
    pub margin_mm: Option<MarginMm>,
    pub cols: Option<usize>,
    pub rows: Option<usize>,
    /// Horizontal and vertical space between cells.
    pub gutter_mm: Option<[f64; 2]>,
    /// Fixed cell size (default: the page size, or the largest page).
    pub cell_mm: Option<[f64; 2]>,
    /// Bleed already inside each page (step & repeat, n-up, cut & stack).
    pub bleed_mm: Option<f64>,
    /// Trim marks: `true` for 3 mm marks 3 mm off the trim, or explicit sizes.
    pub marks: Option<MarksMm>,
    /// `long-edge` or `short-edge`.
    pub duplex: Option<String>,
    /// Cut & stack: times the block of different stacks repeats across a row.
    pub copies: Option<usize>,
    /// Step & repeat: turn whole rows a quarter turn where that fits more copies.
    pub rotate: Option<bool>,
    /// `best`: try the sheet both ways round and keep the one with more copies per sheet.
    pub sheet_orientation: Option<String>,
    /// Booklet: `saddle` (default), `perfect` or `continuous`.
    pub binding: Option<String>,
    pub signature: Option<usize>,
    /// Booklet, perfect binding: `full` or `multiple-of-four` (default).
    pub last_signature: Option<String>,
    /// Booklet: scale pages to fill half the sheet (default true).
    pub scale: Option<bool>,
    /// Booklet: `centre` (default) or `spine`.
    pub align: Option<String>,
    pub edge_mm: Option<f64>,
    pub right_to_left: Option<bool>,
    /// Bleed to generate: one amount, or `[left, bottom, right, top]`.
    pub add_bleed_mm: Option<MarginMm>,
    /// Generated bleed: `mirror` (default) or `scale`.
    pub method: Option<String>,
    /// Manual: the pages on each sheet side, placed by hand.
    pub manual: Option<Vec<Vec<ManualCellMm>>>,
    /// What prints besides page content: `document` (default), `document-and-markups`,
    /// `document-and-stamps`, `form-fields-only`.
    pub comments_forms: Option<String>,
}

/// One page of a manual layout.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ManualCellMm {
    /// 0-based source page.
    pub page: usize,
    /// Lower-left corner of the page's footprint (bleed included, after turning), from the
    /// sheet's lower-left corner.
    pub x_mm: f64,
    pub y_mm: f64,
    /// Degrees anticlockwise: 0, 90, 180 or 270 (-90 is 270).
    #[serde(default)]
    pub turn: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum MarginMm {
    All(f64),
    Sides([f64; 4]),
}

impl MarginMm {
    fn sides(&self) -> [f64; 4] {
        match self {
            MarginMm::All(v) => [*v; 4],
            MarginMm::Sides(s) => *s,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum MarksMm {
    On(bool),
    /// Every field optional; missing ones take the 3 mm / 3 mm / 0.5 pt black corner defaults.
    Custom(MarksSpec),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Default)]
#[serde(deny_unknown_fields, default)]
pub struct MarksSpec {
    pub length_mm: Option<f64>,
    /// Negative runs into the page: -3 with length 5 = 3 mm inside, 2 mm outside.
    pub offset_mm: Option<f64>,
    /// `corner` (default) or `japanese`.
    pub style: Option<String>,
    pub width_pt: Option<f64>,
    pub registration: Option<bool>,
}

/// One page's visible part on a sheet.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Cell {
    /// 0-based source page.
    pub page: usize,
    /// `[x0, y0, x1, y1]` on the sheet, millimetres from the lower-left corner.
    pub rect_mm: [f64; 4],
    /// The page is reflected left-right or top-bottom (generated bleed strips).
    pub mirrored: bool,
    /// Turn of the page in degrees: 90 anticlockwise, -90 clockwise, 180, or 0 upright.
    pub turn: i32,
}

/// One sheet side.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SheetOut {
    pub size_mm: [f64; 2],
    pub cells: Vec<Cell>,
    /// Trim marks, `[x0, y0, x1, y1]` lines in millimetres.
    pub marks_mm: Vec<[f64; 4]>,
    /// The finished area of a page with generated bleed.
    pub trim_mm: Option<[f64; 4]>,
}

/// A whole layout and the numbers a press room asks for first.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Layout {
    pub sheets: Vec<SheetOut>,
    /// Sheet sides produced.
    pub sheet_count: usize,
    /// Most pages placed on one sheet side.
    pub up_per_sheet: usize,
}

fn bad(msg: impl Into<String>) -> String {
    msg.into()
}

fn content(job: &Job) -> Result<Content, String> {
    Ok(match job.comments_forms.as_deref().unwrap_or("document") {
        "document" => Content::Document,
        "document-and-markups" => Content::DocumentAndMarkups,
        "document-and-stamps" => Content::DocumentAndStamps,
        "form-fields-only" => Content::FormFieldsOnly,
        c => return Err(bad(format!("unknown comments_forms {c:?}"))),
    })
}

fn step_repeat(job: &Job, fill: Fill) -> Result<StepRepeat, String> {
    let [w, h] = job.sheet_mm.ok_or_else(|| bad("sheet_mm is required"))?;
    let mut s = StepRepeat::new((pt(w), pt(h)));
    let [l, b, r, t] = job.margin_mm.as_ref().map_or([10.0; 4], MarginMm::sides);
    s.margins = Margins { left: pt(l), bottom: pt(b), right: pt(r), top: pt(t) };
    s.fill = fill;
    s.cols = job.cols;
    s.rows = job.rows;
    let [gx, gy] = job.gutter_mm.unwrap_or([0.0, 0.0]);
    s.gutter = (pt(gx), pt(gy));
    s.cell = job.cell_mm.map(|[cw, ch]| (pt(cw), pt(ch)));
    s.bleed = pt(job.bleed_mm.unwrap_or(0.0));
    s.duplex = match job.duplex.as_deref() {
        None => None,
        Some("long-edge") => Some(Duplex::LongEdge),
        Some("short-edge") => Some(Duplex::ShortEdge),
        Some(d) => return Err(bad(format!("unknown duplex {d:?}"))),
    };
    s.rotate = job.rotate.unwrap_or(false);
    s.marks = marks(job)?;
    Ok(s)
}

fn marks(job: &Job) -> Result<Option<Marks>, String> {
    Ok(match &job.marks {
        None | Some(MarksMm::On(false)) => None,
        Some(MarksMm::On(true)) => Some(Marks::default()),
        Some(MarksMm::Custom(m)) => {
            let d = Marks::default();
            Some(Marks {
                length: m.length_mm.map_or(d.length, pt),
                offset: m.offset_mm.map_or(d.offset, pt),
                style: match m.style.as_deref().unwrap_or("corner") {
                    "corner" => MarkStyle::Corner,
                    "japanese" => MarkStyle::Japanese,
                    x => return Err(bad(format!("unknown mark style {x:?}"))),
                },
                width: m.width_pt.unwrap_or(d.width),
                registration: m.registration.unwrap_or(false),
            })
        }
    })
}

fn manual_spec(job: &Job) -> Result<Manual, String> {
    let [w, h] = job.sheet_mm.ok_or_else(|| bad("sheet_mm is required"))?;
    let sheets = job.manual.as_ref().ok_or_else(|| bad("manual: the pages of each sheet are required"))?;
    let sheets = sheets
        .iter()
        .map(|side| {
            side.iter()
                .map(|c| {
                    let turns = match c.turn.rem_euclid(360) {
                        0 => 0,
                        90 => 1,
                        180 => 2,
                        270 => 3,
                        t => return Err(bad(format!("manual: turn must be 0, 90, 180 or 270, not {t}"))),
                    };
                    Ok(ManualPage { page: c.page, at: (pt(c.x_mm), pt(c.y_mm)), turns })
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Manual { sheet: (pt(w), pt(h)), bleed: pt(job.bleed_mm.unwrap_or(0.0)), marks: marks(job)?, sheets })
}

fn booklet(job: &Job) -> Result<Booklet, String> {
    let mut b = Booklet::new();
    b.sheet = job.sheet_mm.map(|[w, h]| (pt(w), pt(h)));
    b.binding = match job.binding.as_deref().unwrap_or("saddle") {
        "saddle" => Binding::Saddle,
        "continuous" => Binding::Continuous,
        "perfect" => Binding::Perfect {
            signature: job.signature.ok_or_else(|| bad("perfect binding needs signature"))?,
            last: match job.last_signature.as_deref().unwrap_or("multiple-of-four") {
                "full" => LastSignature::Full,
                "multiple-of-four" => LastSignature::MultipleOfFour,
                l => return Err(bad(format!("unknown last_signature {l:?}"))),
            },
        },
        x => return Err(bad(format!("unknown binding {x:?}"))),
    };
    b.scale = job.scale.unwrap_or(true);
    b.align = match job.align.as_deref().unwrap_or("centre") {
        "centre" | "center" => Align::Centre,
        "spine" => Align::Spine,
        a => return Err(bad(format!("unknown align {a:?}"))),
    };
    b.edge = pt(job.edge_mm.unwrap_or(0.0));
    b.right_to_left = job.right_to_left.unwrap_or(false);
    Ok(b)
}

fn add_bleed(job: &Job) -> Result<AddBleed, String> {
    let [l, b, r, t] = job.add_bleed_mm.as_ref().ok_or_else(|| bad("add_bleed_mm is required"))?.sides();
    Ok(AddBleed {
        amount: [pt(l), pt(b), pt(r), pt(t)],
        method: match job.method.as_deref().unwrap_or("mirror") {
            "mirror" => BleedMethod::Mirror,
            "scale" => BleedMethod::Scale,
            m => return Err(bad(format!("unknown method {m:?}"))),
        },
    })
}

/// The sheets `job` makes of pages of these sizes (points). With `sheet_orientation: "best"`,
/// the sheet is also tried the other way round and the layout with more copies on its first
/// sheet wins (the first way on a tie).
fn sheets(sizes: &[(f64, f64)], job: &Job) -> Result<Vec<Sheet>, String> {
    match job.sheet_orientation.as_deref() {
        None | Some("as-given") => sheets_as_given(sizes, job),
        Some("best") => {
            let first = sheets_as_given(sizes, job);
            let mut turned = job.clone();
            turned.sheet_mm = job.sheet_mm.map(|[w, h]| [h, w]);
            let second = sheets_as_given(sizes, &turned);
            let ups = |r: &Result<Vec<Sheet>, String>| r.as_ref().ok().and_then(|s| s.first()).map_or(0, |s| s.placed.len());
            if ups(&second) > ups(&first) { second } else { first }
        }
        Some(o) => Err(bad(format!("unknown sheet_orientation {o:?}"))),
    }
}

fn sheets_as_given(sizes: &[(f64, f64)], job: &Job) -> Result<Vec<Sheet>, String> {
    let r = match job.mode.as_str() {
        "repeat" | "" => pdfcraft_impose::step_and_repeat(sizes, &step_repeat(job, Fill::Repeat)?),
        "sequential" => pdfcraft_impose::step_and_repeat(sizes, &step_repeat(job, Fill::Sequential)?),
        "cut-stack" => pdfcraft_impose::step_and_repeat(sizes, &step_repeat(job, Fill::CutStack { copies: job.copies.unwrap_or(1) })?),
        "booklet" => pdfcraft_impose::booklet(sizes, &booklet(job)?),
        "bleed" => pdfcraft_impose::add_bleed(sizes, &add_bleed(job)?),
        "manual" => pdfcraft_impose::manual(sizes, &manual_spec(job)?),
        m => return Err(bad(format!("unknown mode {m:?}"))),
    };
    r.map_err(|e| e.to_string())
}

/// `strips`: the sheets are pages with generated bleed, whose reflected parts are not ups.
fn describe(sheets: &[Sheet], strips: bool) -> Layout {
    let out: Vec<SheetOut> = sheets
        .iter()
        .map(|s| SheetOut {
            size_mm: [mm(s.size.0), mm(s.size.1)],
            cells: s
                .placed
                .iter()
                .map(|p| {
                    let [a, b, c, d, e, f] = p.matrix.0;
                    let [u0, v0, u1, v1] = p.clip;
                    let pts = [(u0, v0), (u1, v0), (u0, v1), (u1, v1)].map(|(u, v)| (a * u + c * v + e, b * u + d * v + f));
                    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                    for (x, y) in pts {
                        x0 = x0.min(x);
                        y0 = y0.min(y);
                        x1 = x1.max(x);
                        y1 = y1.max(y);
                    }
                    // Quarter turns: 90 anticlockwise, -90 clockwise, 180 upside down, 0 upright.
                    let turn = if b > 0.5 {
                        90
                    } else if b < -0.5 {
                        -90
                    } else if a < -0.5 && d < -0.5 && !strips {
                        180
                    } else {
                        0
                    };
                    // Generated bleed strips are reflections of the page (a corner strip in both
                    // directions); anything else upside down is a half turn.
                    let mirrored = if strips { turn == 0 && (a < 0.0 || d < 0.0) } else { a * d - b * c < 0.0 };
                    Cell { page: p.page, rect_mm: [mm(x0), mm(y0), mm(x1), mm(y1)], mirrored, turn }
                })
                .collect(),
            marks_mm: s.lines.iter().map(|l| [mm(l[0]), mm(l[1]), mm(l[2]), mm(l[3])]).collect(),
            trim_mm: s.trim.map(|t| [mm(t[0]), mm(t[1]), mm(t[2]), mm(t[3])]),
        })
        .collect();
    Layout {
        sheet_count: out.len(),
        up_per_sheet: out.iter().map(|s| s.cells.iter().filter(|c| !c.mirrored).count()).max().unwrap_or(0),
        sheets: out,
    }
}

/// The layout of `job` for pages of these sizes (millimetres), without any PDF.
pub fn layout(pages_mm: &[[f64; 2]], job: &Job) -> Result<Layout, String> {
    let sizes: Vec<(f64, f64)> = pages_mm.iter().map(|[w, h]| (pt(*w), pt(*h))).collect();
    Ok(describe(&sheets(&sizes, job)?, job.mode == "bleed"))
}

fn open(pdf: Vec<u8>) -> Result<Document, String> {
    Document::open(Arc::new(pdf)).map_err(|e| format!("cannot open the PDF: {e}"))
}

/// The display size of every page of `pdf`, in millimetres.
pub fn page_sizes(pdf: Vec<u8>) -> Result<Vec<[f64; 2]>, String> {
    let doc = open(pdf)?;
    Ok(pdfcraft_print::page_sizes(&doc).into_iter().map(|(w, h)| [mm(w), mm(h)]).collect())
}

/// The layout `job` makes of `pdf`, for a preview before imposing.
pub fn layout_pdf(pdf: Vec<u8>, job: &Job) -> Result<Layout, String> {
    let doc = open(pdf)?;
    Ok(describe(&sheets(&pdfcraft_print::page_sizes(&doc), job)?, job.mode == "bleed"))
}

/// The imposed PDF. Page content, images, fonts and colour spaces are placed, never rewritten.
pub fn impose(pdf: Vec<u8>, job: &Job) -> Result<Vec<u8>, String> {
    let doc = open(pdf)?;
    let s = sheets(&pdfcraft_print::page_sizes(&doc), job)?;
    pdfcraft_print::impose_sheets(&doc, &s, content(job)?).map_err(|e| e.to_string())
}

/// Press-room preflight of `pdf`: what was found, each check, and the worst outcome.
#[derive(Serialize)]
pub struct Preflight {
    pub overall: pdfcraft_prepress::Status,
    pub checks: Vec<pdfcraft_prepress::Check>,
    pub report: pdfcraft_prepress::Report,
}

pub fn preflight(pdf: Vec<u8>) -> Result<Preflight, String> {
    let doc = open(pdf)?;
    let th = pdfcraft_prepress::Thresholds::default();
    let report = pdfcraft_prepress::analyze_with(&doc, &th);
    let checks = pdfcraft_prepress::checks(&report, &th);
    Ok(Preflight { overall: pdfcraft_prepress::overall(&checks), checks, report })
}

/// Text turned into outlines (see `pdfcraft_prepress::outline`): the new file and what was done.
pub fn outline(pdf: Vec<u8>) -> Result<(Vec<u8>, pdfcraft_prepress::outline::OutlineReport), String> {
    pdfcraft_prepress::outline::outline_text(pdf)
}

/// Spot colours merged or printed in process (see `pdfcraft_prepress::inks`). `actions_json` maps
/// a spot name to `{"action": "to-process"}` or `{"action": "alias", "into": "<other spot>"}`.
pub fn inks(pdf: Vec<u8>, actions_json: &str) -> Result<(Vec<u8>, pdfcraft_prepress::inks::InkReport), String> {
    let actions: std::collections::BTreeMap<String, pdfcraft_prepress::inks::InkAction> =
        serde_json::from_str(actions_json).map_err(|e| format!("inks: {e}"))?;
    pdfcraft_prepress::inks::manage_inks(pdf, &actions)
}

/// One page of the document the page editor makes.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, untagged)]
pub enum PlanPage {
    /// Page `page` (0-based) of source `src` (0 = the document itself, k = the k-th extra file),
    /// turned `rotate` degrees clockwise on top of the rotation it has (a multiple of 90).
    Page {
        src: usize,
        page: usize,
        #[serde(default)]
        rotate: i64,
    },
    /// A blank page of this size.
    Blank { blank_mm: [f64; 2] },
}

/// Most pages a page-editor plan may make.
const MAX_PLAN_PAGES: usize = 20_000;

/// The page editor: a new document whose pages are `plan` in order. Pages are moved, copied
/// and turned as page objects (their links, comments, form fields and bookmarks go with them);
/// pages of the document left out are deleted for good, and the file is written in full so
/// nothing of them stays behind. Pages from `extras` are imported with their fields renamed
/// where names clash.
pub fn assemble(pdf: Vec<u8>, extras: Vec<Vec<u8>>, plan: &[PlanPage]) -> Result<Vec<u8>, String> {
    use pdfcraft_organize as org;
    let err = |e: org::OrganizeError| e.to_string();
    if plan.is_empty() {
        return Err(bad("the new document needs at least one page"));
    }
    if plan.len() > MAX_PLAN_PAGES {
        return Err(bad(format!("at most {MAX_PLAN_PAGES} pages")));
    }
    let mut doc = open(pdf)?;
    let extras: Vec<Document> = extras.into_iter().map(open).collect::<Result<_, _>>()?;
    let original: Vec<pdfcraft_cos::ObjRef> = org::pages(&doc).map_err(err)?.into_iter().map(|p| p.obj).collect();
    let extra_counts: Vec<usize> = extras.iter().map(|d| org::page_count(d).map_err(err)).collect::<Result<_, _>>()?;
    // 1. Make every page the plan needs, in plan order, and remember which object it is.
    let mut used = vec![false; original.len()];
    let mut target = Vec::with_capacity(plan.len());
    let mut turns = Vec::with_capacity(plan.len());
    for (i, entry) in plan.iter().enumerate() {
        let end = org::page_count(&doc).map_err(err)?;
        match *entry {
            PlanPage::Page { src, page, rotate } => {
                if rotate % 90 != 0 {
                    return Err(bad(format!("page {}: rotation must be a multiple of 90", i + 1)));
                }
                let obj = if src == 0 {
                    let &obj = original.get(page).ok_or_else(|| bad(format!("page {}: the document has no page {}", i + 1, page + 1)))?;
                    match used.get_mut(page) {
                        Some(u) if !*u => {
                            *u = true;
                            obj
                        }
                        _ => {
                            // A second use: a copy of the page, sharing its fonts and images.
                            let now = org::pages(&doc).map_err(err)?.iter().position(|p| p.obj == obj).ok_or_else(|| bad("page lost"))?;
                            let snapshot = doc.clone();
                            *org::import_pages(&mut doc, &snapshot, &[now], end).map_err(err)?.first().ok_or_else(|| bad("copy failed"))?
                        }
                    }
                } else {
                    let ex = extras.get(src - 1).ok_or_else(|| bad(format!("page {}: there is no extra file {src}", i + 1)))?;
                    if page >= extra_counts.get(src - 1).copied().unwrap_or(0) {
                        return Err(bad(format!("page {}: extra file {src} has no page {}", i + 1, page + 1)));
                    }
                    let out = org::import_pages_with(&mut doc, ex, &[page], end, org::FieldConflicts::Rename).map_err(err)?;
                    *out.pages.first().ok_or_else(|| bad("import failed"))?
                };
                target.push(obj);
                turns.push(rotate);
            }
            PlanPage::Blank { blank_mm: [w, h] } => {
                if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 && w <= 5080.0 && h <= 5080.0) {
                    return Err(bad(format!("page {}: a blank page needs a size between 0 and 5080 mm", i + 1)));
                }
                target.push(org::insert_blank_page(&mut doc, end, pt(w), pt(h)).map_err(err)?);
                turns.push(0);
            }
        }
    }
    // 2. Delete the document's pages the plan leaves out.
    let unused: Vec<usize> = used.iter().enumerate().filter(|(_, u)| !**u).map(|(i, _)| i).collect();
    if !unused.is_empty() {
        org::delete_pages(&mut doc, &unused).map_err(err)?;
    }
    // 3. Put the pages in plan order.
    for (i, obj) in target.iter().enumerate() {
        let now = org::pages(&doc).map_err(err)?.iter().position(|p| p.obj == *obj).ok_or_else(|| bad("page lost while ordering"))?;
        if now != i {
            org::move_pages(&mut doc, &[now], i).map_err(err)?;
        }
    }
    // 4. Turn them.
    for (i, t) in turns.iter().enumerate() {
        if t.rem_euclid(360) != 0 {
            org::rotate_pages(&mut doc, &[i], *t).map_err(err)?;
        }
    }
    pdfcraft_cos::write_full(&doc, &pdfcraft_cos::SaveOptions::default()).map_err(|e| e.to_string())
}

fn parse_job(json: &str) -> Result<Job, String> {
    serde_json::from_str(json).map_err(|e| format!("job: {e}"))
}

/// JavaScript entry points. Errors become thrown `Error`s with the message above.
mod wasm {
    use wasm_bindgen::prelude::*;

    fn js(e: String) -> JsError {
        JsError::new(&e)
    }

    fn to_json<T: serde::Serialize>(v: &T) -> Result<String, JsError> {
        serde_json::to_string(v).map_err(|e| JsError::new(&e.to_string()))
    }

    /// `layout(pagesJson, jobJson)`: pages as `[[w, h], ...]` in mm; returns the layout as JSON.
    #[wasm_bindgen]
    pub fn layout(pages_json: &str, job_json: &str) -> Result<String, JsError> {
        let pages: Vec<[f64; 2]> = serde_json::from_str(pages_json).map_err(|e| JsError::new(&format!("pages: {e}")))?;
        to_json(&super::layout(&pages, &super::parse_job(job_json).map_err(js)?).map_err(js)?)
    }

    /// `pageSizes(pdfBytes)`: `[[w, h], ...]` in mm, as JSON.
    #[wasm_bindgen(js_name = pageSizes)]
    pub fn page_sizes(pdf: Vec<u8>) -> Result<String, JsError> {
        to_json(&super::page_sizes(pdf).map_err(js)?)
    }

    /// `layoutPdf(pdfBytes, jobJson)`: the layout of a real file, as JSON.
    #[wasm_bindgen(js_name = layoutPdf)]
    pub fn layout_pdf(pdf: Vec<u8>, job_json: &str) -> Result<String, JsError> {
        to_json(&super::layout_pdf(pdf, &super::parse_job(job_json).map_err(js)?).map_err(js)?)
    }

    /// `impose(pdfBytes, jobJson)`: the imposed PDF bytes.
    #[wasm_bindgen]
    pub fn impose(pdf: Vec<u8>, job_json: &str) -> Result<Vec<u8>, JsError> {
        super::impose(pdf, &super::parse_job(job_json).map_err(js)?).map_err(js)
    }

    /// `preflight(pdfBytes)`: `{ overall, checks: [{id, title, status, message}], report }` as JSON.
    #[wasm_bindgen]
    pub fn preflight(pdf: Vec<u8>) -> Result<String, JsError> {
        to_json(&super::preflight(pdf).map_err(js)?)
    }

    /// A changed file and the report of what changed, as JSON.
    #[wasm_bindgen]
    pub struct Converted {
        bytes: Vec<u8>,
        report: String,
    }

    #[wasm_bindgen]
    impl Converted {
        #[wasm_bindgen(getter)]
        pub fn bytes(&self) -> Vec<u8> {
            self.bytes.clone()
        }
        #[wasm_bindgen(getter)]
        pub fn report(&self) -> String {
            self.report.clone()
        }
    }

    /// `outlineText(pdfBytes)`: `{ bytes, report }`, every embedded-font glyph as a path.
    #[wasm_bindgen(js_name = outlineText)]
    pub fn outline_text(pdf: Vec<u8>) -> Result<Converted, JsError> {
        let (bytes, report) = super::outline(pdf).map_err(js)?;
        Ok(Converted { bytes, report: to_json(&report)? })
    }

    /// `manageInks(pdfBytes, actionsJson)`: `{ bytes, report }` with spots merged or made process.
    #[wasm_bindgen(js_name = manageInks)]
    pub fn manage_inks(pdf: Vec<u8>, actions_json: &str) -> Result<Converted, JsError> {
        let (bytes, report) = super::inks(pdf, actions_json).map_err(js)?;
        Ok(Converted { bytes, report: to_json(&report)? })
    }

    /// `assemblePages(pdfBytes, extrasBytes, extraLengths, planJson)`: the page editor's new
    /// document. `extrasBytes` holds the other files one after another, `extraLengths` their sizes.
    #[wasm_bindgen(js_name = assemblePages)]
    pub fn assemble_pages(pdf: Vec<u8>, extras: Vec<u8>, extra_lengths: Vec<u32>, plan_json: &str) -> Result<Vec<u8>, JsError> {
        let plan: Vec<super::PlanPage> = serde_json::from_str(plan_json).map_err(|e| JsError::new(&format!("plan: {e}")))?;
        let mut files = Vec::with_capacity(extra_lengths.len());
        let mut at = 0usize;
        for n in extra_lengths {
            let end = at.checked_add(n as usize).ok_or_else(|| JsError::new("extra files: bad lengths"))?;
            files.push(extras.get(at..end).ok_or_else(|| JsError::new("extra files: bad lengths"))?.to_vec());
            at = end;
        }
        super::assemble(pdf, files, &plan).map_err(js)
    }

    /// The engine version, to show in the UI and record with a job.
    #[wasm_bindgen]
    pub fn version() -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

#[cfg(test)]
mod tests;
