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
    AddBleed, Align, Arrange, Binding, BleedMethod, Booklet, Duplex, Fill, LastSignature, Manual, ManualPage, Margins, MarkStyle, Marks, PageSet,
    PageTool, Shift, Shuffle, StepRepeat, Tile, Trim, TrimShift,
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
    /// `bleed`, `manual`, or a page tool: `shuffle`, `reverse`, `trim-shift`, `tile`.
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
    /// Step & repeat / n-up / cut & stack: page scale in percent (default 100).
    pub scale_percent: Option<f64>,
    /// Step & repeat: scale each page to fill the fixed `cell_mm`, keeping its shape.
    pub fit_cell: Option<bool>,
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
    /// Booklet: creep per sheet of paper of a folded signature, mm (see `Booklet::creep`).
    pub creep_mm: Option<f64>,
    /// Bleed to generate: one amount, or `[left, bottom, right, top]`.
    pub add_bleed_mm: Option<MarginMm>,
    /// Generated bleed: `mirror` (default) or `scale`.
    pub method: Option<String>,
    /// Manual: the pages on each sheet side, placed by hand.
    pub manual: Option<Vec<Vec<ManualCellMm>>>,
    /// The page tools' settings (`shuffle`, `trim-shift`, `tile`).
    pub page_tool: Option<PageToolMm>,
    /// What prints besides page content: `document` (default), `document-and-markups`,
    /// `document-and-stamps`, `form-fields-only`.
    pub comments_forms: Option<String>,
}

/// Settings of the page tools (shuffle even/odd, trim and shift, tile). Lengths in millimetres.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Default)]
#[serde(deny_unknown_fields, default)]
pub struct PageToolMm {
    /// Shuffle: `odds-first` (default), `interleave` or `keep`.
    pub arrange: Option<String>,
    pub reverse_evens: bool,
    pub turn_evens: bool,
    /// Trim and shift: `all` (default), `odd` or `even`.
    pub pages: Option<String>,
    /// Trim and shift: `[left, bottom, right, top]`, positive trims, negative adds white space.
    pub trim_mm: Option<[f64; 4]>,
    /// Trim and shift: every chosen page made this size, its content centred.
    pub same_size_mm: Option<[f64; 2]>,
    /// Trim and shift: `[right, up]`; with `shift_last_mm`, the first chosen page's amount.
    pub shift_mm: Option<[f64; 2]>,
    /// Trim and shift: the last chosen page's shift; the pages between in proportion.
    pub shift_last_mm: Option<[f64; 2]>,
    /// Tile: columns and rows (default 2 x 1).
    pub cols: Option<usize>,
    pub rows: Option<usize>,
    pub overlap_mm: Option<f64>,
    pub only_wide: bool,
}

/// The page tool a `shuffle`, `reverse`, `trim-shift` or `tile` job runs.
fn page_tool(job: &Job) -> Result<PageTool, String> {
    let d = PageToolMm::default();
    let t = job.page_tool.as_ref().unwrap_or(&d);
    let pair = |v: Option<[f64; 2]>| v.map(|[x, y]| (pt(x), pt(y)));
    Ok(match job.mode.as_str() {
        "reverse" => PageTool::Reverse,
        "shuffle" => PageTool::Shuffle(Shuffle {
            arrange: match t.arrange.as_deref().unwrap_or("odds-first") {
                "odds-first" => Arrange::OddsFirst,
                "interleave" => Arrange::Interleave,
                "keep" => Arrange::Keep,
                a => return Err(bad(format!("unknown arrange {a:?}"))),
            },
            reverse_evens: t.reverse_evens,
            turn_evens: t.turn_evens,
        }),
        "trim-shift" => {
            let pages = match t.pages.as_deref().unwrap_or("all") {
                "all" => PageSet::All,
                "odd" => PageSet::Odd,
                "even" => PageSet::Even,
                p => return Err(bad(format!("unknown pages {p:?}"))),
            };
            let trim = match (t.trim_mm, t.same_size_mm) {
                (Some(_), Some(_)) => return Err(bad("give trim_mm or same_size_mm, not both")),
                (Some([l, b, r, top]), None) => Trim::Edges([pt(l), pt(b), pt(r), pt(top)]),
                (None, Some([w, h])) => Trim::SameSize(pt(w), pt(h)),
                (None, None) => Trim::None,
            };
            let shift = match (pair(t.shift_mm), pair(t.shift_last_mm)) {
                (None, None) => Shift::None,
                (Some(s), None) => Shift::Fixed(s.0, s.1),
                (Some(first), Some(last)) => Shift::Variable { first, last },
                (None, Some(_)) => return Err(bad("shift_last_mm needs shift_mm for the first page")),
            };
            if trim == Trim::None && shift == Shift::None {
                return Err(bad("trim and shift needs a trim or a shift"));
            }
            PageTool::TrimShift(TrimShift { pages, trim, shift })
        }
        "tile" => PageTool::Tile(Tile {
            cols: t.cols.unwrap_or(2),
            rows: t.rows.unwrap_or(1),
            overlap: pt(t.overlap_mm.unwrap_or(0.0)),
            only_wide: t.only_wide,
        }),
        m => return Err(bad(format!("{m:?} is not a page tool"))),
    })
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
    s.fit_cell = job.fit_cell.unwrap_or(false);
    if let Some(pct) = job.scale_percent {
        s.scale = pct / 100.0;
    }
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
                    "centred" | "centered" => MarkStyle::Centred,
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
    b.creep = pt(job.creep_mm.unwrap_or(0.0));
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
        "shuffle" | "reverse" | "trim-shift" | "tile" => match page_tool(job)? {
            PageTool::Reverse => pdfcraft_impose::reverse(sizes),
            PageTool::Shuffle(s) => pdfcraft_impose::shuffle(sizes, &s),
            PageTool::TrimShift(t) => pdfcraft_impose::trim_and_shift(sizes, &t),
            PageTool::Tile(t) => pdfcraft_impose::tile(sizes, &t),
        },
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

/// Sticker cut lines asked for with a job. Every length is in millimetres.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Default)]
#[serde(deny_unknown_fields, default)]
pub struct CutMm {
    /// How far the line lies from the artwork's edge, on the sheet: positive outside, negative
    /// inside, 0 on the edge.
    pub offset_mm: f64,
    /// `fill` (default: only the outer outline is cut) or `keep` (empty areas inside are cut too).
    pub holes: Option<String>,
    /// What counts as empty paper: `auto` (default), `transparent` or `white`.
    pub background: Option<String>,
    /// The spot colour's name (default `CutContour`).
    pub spot: Option<String>,
    pub width_pt: Option<f64>,
    /// Pieces of artwork smaller than this are not cut (default 1 mm²).
    pub min_area_mm2: Option<f64>,
    /// A plotter-cutter's sheet: registration marks and a QR code on every sheet with cut lines,
    /// and a PLT file of those lines for each. Needs the cut lines.
    pub cutter: Option<CutterMm>,
}

/// The plotter-cutter's marks and PLT file, in millimetres.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Default)]
#[serde(deny_unknown_fields, default)]
pub struct CutterMm {
    /// The job's name: the QR code's content and the PLT file's name (letters, digits, `-`, `_`).
    pub name: String,
    /// Distance from the sheet's edge to the centre of each mark (default 10).
    pub inset_mm: Option<f64>,
    /// Side of a mark (default 5).
    pub mark_mm: Option<f64>,
    /// Side of the QR code (default 7).
    pub code_mm: Option<f64>,
}

/// A PLT file made for one sheet; the text is plain ASCII.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CutterFile {
    pub name: String,
    pub text: String,
}

fn cutter_spec(c: &CutterMm) -> Result<pdfcraft_impose::CutterSpec, String> {
    let mut layout = pdfcraft_impose::cutter::Layout::default();
    for (what, v, slot, lo, hi) in [
        ("inset_mm", c.inset_mm, &mut layout.inset_mm, 1.0, 100.0),
        ("mark_mm", c.mark_mm, &mut layout.mark_mm, 1.0, 50.0),
        ("code_mm", c.code_mm, &mut layout.code_mm, 3.0, 50.0),
    ] {
        if let Some(v) = v {
            if !(v.is_finite() && (lo..=hi).contains(&v)) {
                return Err(bad(format!("cutter {what} must be between {lo} and {hi}")));
            }
            *slot = v;
        }
    }
    pdfcraft_impose::cutter::clean_name(&c.name).map_err(|e| bad(format!("cutter name: {e}")))?;
    Ok(pdfcraft_impose::CutterSpec { name: c.name.clone(), layout })
}

/// The farthest a cut line may be from the artwork, in millimetres.
const MAX_CUT_OFFSET_MM: f64 = 100.0;

fn cut_spec(cut: &CutMm) -> Result<pdfcraft_impose::CutSpec, String> {
    use pdfcraft_impose::cutline::{Background, Holes};
    if !cut.offset_mm.is_finite() || cut.offset_mm.abs() > MAX_CUT_OFFSET_MM {
        return Err(bad(format!("cut offset_mm must be between -{MAX_CUT_OFFSET_MM} and {MAX_CUT_OFFSET_MM}")));
    }
    let mut spec = pdfcraft_impose::CutSpec::new();
    spec.offset = pt(cut.offset_mm);
    spec.holes = match cut.holes.as_deref().unwrap_or("fill") {
        "fill" => Holes::Fill,
        "keep" => Holes::Keep,
        h => return Err(bad(format!("unknown cut holes {h:?}"))),
    };
    spec.background = match cut.background.as_deref().unwrap_or("auto") {
        "auto" => Background::Auto,
        "transparent" => Background::Transparent,
        "white" => Background::White,
        b => return Err(bad(format!("unknown cut background {b:?}"))),
    };
    if let Some(name) = &cut.spot {
        spec.paint.name.clone_from(name);
    }
    if let Some(width) = cut.width_pt {
        if !(width.is_finite() && width > 0.0 && width <= 100.0) {
            return Err(bad("cut width_pt must be greater than 0 and at most 100"));
        }
        spec.paint.width = width;
    }
    if let Some(area) = cut.min_area_mm2 {
        if !(area.is_finite() && (0.0..=1e6).contains(&area)) {
            return Err(bad("cut min_area_mm2 must be between 0 and 1000000"));
        }
        spec.min_area = pt(pt(area));
    }
    if let Some(c) = &cut.cutter {
        cutter_spec(c)?;
    }
    Ok(spec)
}

/// The modes that can carry cut lines: the ones that place whole pages on a grid.
fn check_cut_mode(job: &Job) -> Result<(), String> {
    match job.mode.as_str() {
        "repeat" | "" | "sequential" | "cut-stack" => Ok(()),
        m => Err(bad(format!("cut lines are for step & repeat, n-up and cut & stack, not {m:?}"))),
    }
}

/// One drawing of a page that cut lines need from the browser: page `page` of the file, whose
/// display size is `size_pt`, drawn `px_per_pt` pixels to the point (more or fewer is fine as long
/// as the whole page is drawn on a clear background, rotated as it displays).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CutNeed {
    /// 0-based source page.
    pub page: usize,
    /// How much the placement scales the page.
    pub scale: f64,
    /// The visible part of the page, `[x0, y0, x1, y1]` points in its display space.
    pub clip: [f64; 4],
    pub size_pt: [f64; 2],
    pub px_per_pt: f64,
}

/// The drawings `job`'s layout of `pdf` needs for `cut`, one for each distinct placement.
pub fn cut_needs(pdf: Vec<u8>, job: &Job, cut: &CutMm) -> Result<Vec<CutNeed>, String> {
    check_cut_mode(job)?;
    cut_spec(cut)?;
    let doc = open(pdf)?;
    let sizes = pdfcraft_print::page_sizes(&doc);
    let sheets = sheets(&sizes, job)?;
    pdfcraft_impose::cut_requests(&sheets)
        .into_iter()
        .map(|r| {
            let size = *sizes.get(r.page).ok_or_else(|| bad(format!("the layout uses page {}", r.page.saturating_add(1))))?;
            Ok(CutNeed {
                page: r.page,
                scale: r.scale,
                clip: r.clip,
                size_pt: [size.0, size.1],
                px_per_pt: pdfcraft_impose::trace_resolution(size, &r),
            })
        })
        .collect()
}

/// The cut lines of one placement: closed polygons in the page's display space (points).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CutOutline {
    pub page: usize,
    pub scale: f64,
    pub clip: [f64; 4],
    pub loops: Vec<Vec<[f64; 2]>>,
}

/// The cut lines for `need`, traced from the browser's drawing of the page: `rgba` is
/// `width` × `height` straight (not premultiplied) pixels, row by row from the top, as a canvas
/// gives them.
pub fn trace_cut(rgba: &[u8], width: usize, height: usize, need: &CutNeed, cut: &CutMm) -> Result<CutOutline, String> {
    let spec = cut_spec(cut)?;
    let request = pdfcraft_impose::CutRequest { page: need.page, scale: need.scale, clip: need.clip };
    let raster = pdfcraft_impose::Raster { width, height, rgba, straight: true, size: (need.size_pt[0], need.size_pt[1]) };
    let found = pdfcraft_impose::trace_page(&raster, &request, &spec).map_err(|e| format!("page {}: {e}", need.page.saturating_add(1)))?;
    let loops = found.loops.iter().map(|l| l.iter().map(|&(x, y)| [x, y]).collect()).collect();
    Ok(CutOutline { page: need.page, scale: need.scale, clip: need.clip, loops })
}

/// What a cut job makes.
#[derive(Debug)]
pub struct CutJob {
    pub pdf: Vec<u8>,
    /// How many cut lines the sheets have.
    pub paths: usize,
    /// The plotter-cutter's PLT files, one per sheet with cut lines (empty without `cutter`).
    pub files: Vec<CutterFile>,
}

/// The imposed PDF with cut lines on their own layer, from the `outlines` traced for every
/// placement [`cut_needs`] listed; with `cutter`, also the marks and QR code on the sheets and the
/// PLT files. Returns the PDF and how many cut lines it has.
pub fn impose_cut(pdf: Vec<u8>, job: &Job, cut: &CutMm, outlines: &[CutOutline]) -> Result<(Vec<u8>, usize), String> {
    impose_cut_job(pdf, job, cut, outlines).map(|j| (j.pdf, j.paths))
}

/// [`impose_cut`] with the plotter-cutter's files too.
pub fn impose_cut_job(pdf: Vec<u8>, job: &Job, cut: &CutMm, outlines: &[CutOutline]) -> Result<CutJob, String> {
    check_cut_mode(job)?;
    let spec = cut_spec(cut)?;
    let cutter = cut.cutter.as_ref().map(cutter_spec).transpose()?;
    let doc = open(pdf)?;
    let mut sheets = sheets(&pdfcraft_print::page_sizes(&doc), job)?;
    let found: Vec<pdfcraft_impose::Outline> = outlines
        .iter()
        .map(|o| pdfcraft_impose::Outline {
            request: pdfcraft_impose::CutRequest { page: o.page, scale: o.scale, clip: o.clip },
            loops: o.loops.iter().map(|l| l.iter().map(|p| (p[0], p[1])).collect()).collect(),
        })
        .collect();
    // A placement nobody traced would silently get no cut line: refuse instead.
    for need in pdfcraft_impose::cut_requests(&sheets) {
        if !found.iter().any(|o| o.request == need) {
            return Err(bad(format!("no cut line was traced for page {} as the layout places it", need.page.saturating_add(1))));
        }
    }
    pdfcraft_impose::place_cuts(&mut sheets, &found, &spec.paint);
    let paths = sheets.iter().map(|s| s.cuts.len()).sum();
    let mut files = Vec::new();
    if let Some(c) = &cutter {
        for f in pdfcraft_impose::apply_cutter(&mut sheets, c).map_err(|e| bad(format!("cutter: {e}")))? {
            // The writer only makes ASCII, so this cannot lose anything.
            files.push(CutterFile { name: f.name, text: String::from_utf8_lossy(&f.bytes).into_owned() });
        }
    }
    let pdf = pdfcraft_print::impose_sheets(&doc, &sheets, content(job)?).map_err(|e| e.to_string())?;
    Ok(CutJob { pdf, paths, files })
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

/// One page's new crop box, in PDF user space: `[left, bottom, right, top]`.
#[derive(Deserialize)]
pub struct CropBox {
    pub page: usize,
    pub rect: [f64; 4],
}

/// Most pages one crop call may name.
const MAX_CROPS: usize = 20_000;

/// Set the crop box of the named pages (0-based) and write the file in full. The browser finds
/// the boxes (from what its own page renderer draws, where the page is rotated or not); here
/// they are only checked and applied, all or none. A box is kept inside its page's media box.
pub fn crop_pages(pdf: Vec<u8>, boxes: &[CropBox]) -> Result<Vec<u8>, String> {
    use pdfcraft_organize as org;
    if boxes.is_empty() {
        return Err(bad("no pages to crop"));
    }
    if boxes.len() > MAX_CROPS {
        return Err(bad(format!("at most {MAX_CROPS} pages")));
    }
    let mut doc = open(pdf)?;
    let count = org::page_count(&doc).map_err(|e| e.to_string())?;
    if let Some(b) = boxes.iter().find(|b| b.page >= count) {
        return Err(bad(format!("the document has no page {}", b.page.saturating_add(1))));
    }
    for b in boxes {
        org::set_page_box(&mut doc, &[b.page], org::PageBox::Crop, org::BoxSpec::Rect(b.rect)).map_err(|e| format!("page {}: {e}", b.page + 1))?;
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

    fn parse<T: serde::de::DeserializeOwned>(what: &str, json: &str) -> Result<T, JsError> {
        serde_json::from_str(json).map_err(|e| JsError::new(&format!("{what}: {e}")))
    }

    /// `cutNeeds(pdfBytes, jobJson, cutJson)`: the page drawings cut lines need, as JSON
    /// `[{page, scale, clip, size_pt, px_per_pt}, …]`. Draw each on a clear background at
    /// `px_per_pt` pixels to the point, then give the pixels to `traceCut`.
    #[wasm_bindgen(js_name = cutNeeds)]
    pub fn cut_needs(pdf: Vec<u8>, job_json: &str, cut_json: &str) -> Result<String, JsError> {
        to_json(&super::cut_needs(pdf, &super::parse_job(job_json).map_err(js)?, &parse("cut", cut_json)?).map_err(js)?)
    }

    /// `traceCut(rgba, width, height, needJson, cutJson)`: the cut lines of one drawing (straight
    /// RGBA from a canvas), as JSON to collect for `imposeCut`.
    #[wasm_bindgen(js_name = traceCut)]
    pub fn trace_cut(rgba: &[u8], width: u32, height: u32, need_json: &str, cut_json: &str) -> Result<String, JsError> {
        let need: super::CutNeed = parse("need", need_json)?;
        to_json(&super::trace_cut(rgba, width as usize, height as usize, &need, &parse("cut", cut_json)?).map_err(js)?)
    }

    /// `imposeCut(pdfBytes, jobJson, cutJson, outlinesJson)`: the imposed PDF with the cut lines.
    #[wasm_bindgen(js_name = imposeCut)]
    pub fn impose_cut(pdf: Vec<u8>, job_json: &str, cut_json: &str, outlines_json: &str) -> Result<Vec<u8>, JsError> {
        let outlines: Vec<super::CutOutline> = parse("outlines", outlines_json)?;
        super::impose_cut(pdf, &super::parse_job(job_json).map_err(js)?, &parse("cut", cut_json)?, &outlines).map(|(pdf, _)| pdf).map_err(js)
    }

    /// `cutterFiles(pdfBytes, jobJson, cutJson, outlinesJson)`: the plotter-cutter's PLT files of
    /// the same job, as JSON `[{name, text}, …]` (empty without `cutter` in `cutJson`).
    #[wasm_bindgen(js_name = cutterFiles)]
    pub fn cutter_files(pdf: Vec<u8>, job_json: &str, cut_json: &str, outlines_json: &str) -> Result<String, JsError> {
        let outlines: Vec<super::CutOutline> = parse("outlines", outlines_json)?;
        let made = super::impose_cut_job(pdf, &super::parse_job(job_json).map_err(js)?, &parse("cut", cut_json)?, &outlines).map_err(js)?;
        to_json(&made.files)
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

    /// `cropPages(pdfBytes, boxesJson)`: `[{"page": 0, "rect": [l, b, r, t]}, …]` in PDF user space
    /// becomes each page's crop box.
    #[wasm_bindgen(js_name = cropPages)]
    pub fn crop_pages(pdf: Vec<u8>, boxes_json: &str) -> Result<Vec<u8>, JsError> {
        let boxes: Vec<super::CropBox> = serde_json::from_str(boxes_json).map_err(|e| JsError::new(&format!("boxes: {e}")))?;
        super::crop_pages(pdf, &boxes).map_err(js)
    }

    /// The engine version, to show in the UI and record with a job.
    #[wasm_bindgen]
    pub fn version() -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

#[cfg(test)]
mod tests;
