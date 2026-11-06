//! `doc_impose` (step & repeat, gang n-up) and `doc_booklet` (saddle, perfect-bound and continuous
//! booklets) onto press sheets, `doc_impose_manual` (every page placed by hand) and `doc_bleed`
//! (generated bleed).

use pdfcraft_engine::impose::{
    self, AddBleed, Align, Binding, BleedMethod, Booklet, Duplex, Fill, LastSignature, Manual, ManualPage, Margins, MarkStyle, Marks, StepRepeat,
};
use pdfcraft_engine::print::Content;
use serde_json::{Value, json};

use crate::{Args, Automation, Result, ToolError, failed, write_atomic};

const MM: f64 = 72.0 / 25.4;

/// Sheet sizes in millimetres (width, height; portrait) by name.
const SHEETS: [(&str, (f64, f64)); 8] = [
    ("A5", (148.0, 210.0)),
    ("A4", (210.0, 297.0)),
    ("A3", (297.0, 420.0)),
    ("A3+", (329.0, 483.0)),
    ("SRA4", (225.0, 320.0)),
    ("SRA3", (320.0, 450.0)),
    ("12x18", (304.8, 457.2)),
    ("13x19", (330.2, 482.6)),
];

fn bad(m: impl Into<String>) -> ToolError {
    ToolError::InvalidArgs(m.into())
}

/// The sheet in points: a named size or `sheet_width_mm` x `sheet_height_mm`, turned to landscape
/// when `landscape` is set.
fn sheet_size(a: &Args) -> Result<(f64, f64)> {
    sheet_size_oriented(a, false)
}

/// [`sheet_size`] with the orientation `landscape` defaults to.
fn sheet_size_oriented(a: &Args, landscape_default: bool) -> Result<(f64, f64)> {
    let (w, h) = match (a.opt_str("sheet")?, a.opt_num("sheet_width_mm")?, a.opt_num("sheet_height_mm")?) {
        (Some(name), None, None) => SHEETS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, s)| *s).ok_or_else(|| {
            bad(format!("unknown sheet {name:?}; choose one of {} or pass sheet_width_mm and sheet_height_mm", SHEETS.map(|(n, _)| n).join(", ")))
        })?,
        (None, Some(w), Some(h)) => (w, h),
        _ => return Err(bad("pass either sheet (a name) or both sheet_width_mm and sheet_height_mm")),
    };
    let (w, h) = (w * MM, h * MM);
    Ok(if a.opt_bool("landscape")?.unwrap_or(landscape_default) { (w.max(h), w.min(h)) } else { (w.min(h), w.max(h)) })
}

/// A count argument: at least 1.
fn count(a: &Args, key: &str) -> Result<Option<usize>> {
    match a.opt_int(key)? {
        None => Ok(None),
        Some(n) if n >= 1 => Ok(Some(n as usize)),
        Some(_) => Err(bad(format!("{key} must be at least 1"))),
    }
}

/// `key` in millimetres as points.
fn mm(a: &Args, key: &str) -> Result<Option<f64>> {
    Ok(a.opt_num(key)?.map(|v| v * MM))
}

/// What prints besides the page content: `comments_forms`.
fn content_arg(a: &Args) -> Result<Content> {
    Ok(match a.opt_str("comments_forms")?.unwrap_or("document") {
        "document" => Content::Document,
        "document-and-markups" => Content::DocumentAndMarkups,
        "document-and-stamps" => Content::DocumentAndStamps,
        "form-fields-only" => Content::FormFieldsOnly,
        c => return Err(bad(format!("unknown comments_forms {c:?}"))),
    })
}

/// Trim marks from `marks` and the `mark_*` arguments; `None` unless `marks` is true.
fn marks_arg(a: &Args) -> Result<Option<Marks>> {
    if !a.opt_bool("marks")?.unwrap_or(false) {
        return Ok(None);
    }
    let default = Marks::default();
    Ok(Some(Marks {
        length: mm(a, "mark_length_mm")?.unwrap_or(default.length),
        offset: mm(a, "mark_offset_mm")?.unwrap_or(default.offset),
        style: match a.opt_str("mark_style")?.unwrap_or("corner") {
            "corner" => MarkStyle::Corner,
            "japanese" => MarkStyle::Japanese,
            "centred" | "centered" => MarkStyle::Centred,
            m => return Err(bad(format!("unknown mark_style {m:?}"))),
        },
        width: a.opt_num("mark_width_pt")?.unwrap_or(default.width),
        registration: a.opt_bool("mark_registration")?.unwrap_or(false),
    }))
}

/// The `sheets` argument of `doc_impose_manual`: an array of sheet sides, each an array of
/// `{page (1-based), x_mm, y_mm, turn}`.
fn manual_sheets(a: &Args) -> Result<Vec<Vec<ManualPage>>> {
    let sides = a.get("sheets").and_then(Value::as_array).ok_or_else(|| bad("sheets: an array of sheet sides is required"))?;
    sides
        .iter()
        .enumerate()
        .map(|(i, side)| {
            let cells = side.as_array().ok_or_else(|| bad(format!("sheets[{i}] must be an array of pages")))?;
            cells
                .iter()
                .map(|c| {
                    let n = |k: &str| c.get(k).and_then(Value::as_f64);
                    let page = c
                        .get("page")
                        .and_then(Value::as_u64)
                        .filter(|p| *p >= 1)
                        .ok_or_else(|| bad(format!("sheets[{i}]: every page needs page (1-based)")))?;
                    let (x, y) = n("x_mm").zip(n("y_mm")).ok_or_else(|| bad(format!("sheets[{i}]: every page needs x_mm and y_mm")))?;
                    let turns = match c.get("turn").and_then(Value::as_i64).unwrap_or(0).rem_euclid(360) {
                        0 => 0,
                        90 => 1,
                        180 => 2,
                        270 => 3,
                        t => return Err(bad(format!("sheets[{i}]: turn must be 0, 90, 180 or 270, not {t}"))),
                    };
                    let page = usize::try_from(page - 1).map_err(|_| bad("page out of range"))?;
                    Ok(ManualPage { page, at: (x * MM, y * MM), turns })
                })
                .collect()
        })
        .collect()
}

impl Automation {
    /// `doc_impose`: lay the document's pages out on press sheets and save the print-ready PDF.
    pub(crate) fn doc_impose(&mut self, a: &Args) -> Result<Value> {
        let doc = self.doc(a)?;
        let id = doc.id;
        let sizes: Vec<(f64, f64)> = doc.info.pages.iter().map(|p| (p.width as f64, p.height as f64)).collect();
        let gutter = mm(a, "gutter_mm")?.unwrap_or(0.0);
        let mut spec = StepRepeat::new(sheet_size(a)?);
        spec.margins = Margins::uniform(mm(a, "margin_mm")?.unwrap_or(10.0 * MM));
        spec.fill = match a.opt_str("fill")?.unwrap_or("repeat") {
            "repeat" => Fill::Repeat,
            "sequential" => Fill::Sequential,
            "cut-stack" => Fill::CutStack { copies: count(a, "copies")?.unwrap_or(1) },
            f => return Err(bad(format!("unknown fill {f:?}"))),
        };
        spec.cols = count(a, "cols")?;
        spec.rows = count(a, "rows")?;
        spec.gutter = (mm(a, "gutter_x_mm")?.unwrap_or(gutter), mm(a, "gutter_y_mm")?.unwrap_or(gutter));
        spec.cell = match (mm(a, "cell_width_mm")?, mm(a, "cell_height_mm")?) {
            (None, None) => None,
            (Some(w), Some(h)) => Some((w, h)),
            _ => return Err(bad("pass both cell_width_mm and cell_height_mm, or neither")),
        };
        spec.bleed = mm(a, "bleed_mm")?.unwrap_or(0.0);
        spec.duplex = match a.opt_str("duplex")?.unwrap_or("off") {
            "off" => None,
            "long-edge" => Some(Duplex::LongEdge),
            "short-edge" => Some(Duplex::ShortEdge),
            d => return Err(bad(format!("unknown duplex {d:?}"))),
        };
        spec.rotate = a.opt_bool("rotate")?.unwrap_or(false);
        spec.marks = marks_arg(a)?;
        let content = content_arg(a)?;
        let sheets = impose::step_and_repeat(&sizes, &spec).map_err(|e| bad(e.to_string()))?;
        let up = sheets.iter().map(|s| s.placed.len()).max().unwrap_or(0);
        let bytes = self.session.impose_pdf(id, &spec, content).map_err(failed)?;
        let path = self.resolve(a.str("path")?, true)?;
        write_atomic(&path, &bytes)?;
        Ok(json!({ "sheets": sheets.len(), "up": up, "bytes": bytes.len(), "path": path.to_string_lossy() }))
    }

    /// `doc_booklet`: make the document into a booklet and save the print-ready PDF.
    pub(crate) fn doc_booklet(&mut self, a: &Args) -> Result<Value> {
        let doc = self.doc(a)?;
        let id = doc.id;
        let sizes: Vec<(f64, f64)> = doc.info.pages.iter().map(|p| (p.width as f64, p.height as f64)).collect();
        let mut spec = Booklet::new();
        // A booklet sheet is wide: two pages side by side.
        spec.sheet = match (a.opt_str("sheet")?, a.opt_num("sheet_width_mm")?, a.opt_num("sheet_height_mm")?) {
            (None, None, None) => None,
            _ => Some(sheet_size_oriented(a, true)?),
        };
        spec.binding = match a.opt_str("binding")?.unwrap_or("saddle") {
            "saddle" => Binding::Saddle,
            "continuous" => Binding::Continuous,
            "perfect" => {
                let signature =
                    count(a, "signature")?.ok_or_else(|| bad("perfect binding needs signature (pages per signature, a multiple of 4)"))?;
                let last = match a.opt_str("last_signature")?.unwrap_or("multiple-of-four") {
                    "full" => LastSignature::Full,
                    "multiple-of-four" => LastSignature::MultipleOfFour,
                    l => return Err(bad(format!("unknown last_signature {l:?}"))),
                };
                Binding::Perfect { signature, last }
            }
            b => return Err(bad(format!("unknown binding {b:?}"))),
        };
        spec.scale = a.opt_bool("scale")?.unwrap_or(true);
        spec.align = match a.opt_str("align")?.unwrap_or("centre") {
            "centre" | "center" => Align::Centre,
            "spine" => Align::Spine,
            v => return Err(bad(format!("unknown align {v:?}"))),
        };
        spec.edge = mm(a, "edge_mm")?.unwrap_or(0.0);
        spec.right_to_left = a.opt_bool("right_to_left")?.unwrap_or(false);
        let content = content_arg(a)?;
        let sides = impose::booklet(&sizes, &spec).map_err(|e| bad(e.to_string()))?;
        let bytes = self.session.booklet_pdf(id, &spec, content).map_err(failed)?;
        let path = self.resolve(a.str("path")?, true)?;
        write_atomic(&path, &bytes)?;
        Ok(json!({ "pages": sizes.len(), "sides": sides.len(), "bytes": bytes.len(), "path": path.to_string_lossy() }))
    }

    /// `doc_impose_manual`: place pages on sheets by hand and save the print-ready PDF.
    pub(crate) fn doc_impose_manual(&mut self, a: &Args) -> Result<Value> {
        let doc = self.doc(a)?;
        let id = doc.id;
        let sizes: Vec<(f64, f64)> = doc.info.pages.iter().map(|p| (p.width as f64, p.height as f64)).collect();
        let spec = Manual { sheet: sheet_size(a)?, bleed: mm(a, "bleed_mm")?.unwrap_or(0.0), marks: marks_arg(a)?, sheets: manual_sheets(a)? };
        let content = content_arg(a)?;
        let sheets = impose::manual(&sizes, &spec).map_err(|e| bad(e.to_string()))?;
        let bytes = self.session.manual_pdf(id, &spec, content).map_err(failed)?;
        let path = self.resolve(a.str("path")?, true)?;
        write_atomic(&path, &bytes)?;
        let placed: usize = sheets.iter().map(|s| s.placed.len()).sum();
        Ok(json!({ "sheets": sheets.len(), "placed": placed, "bytes": bytes.len(), "path": path.to_string_lossy() }))
    }

    /// `doc_bleed`: add bleed around every page and save the print-ready PDF.
    pub(crate) fn doc_bleed(&mut self, a: &Args) -> Result<Value> {
        let doc = self.doc(a)?;
        let id = doc.id;
        let sizes: Vec<(f64, f64)> = doc.info.pages.iter().map(|p| (p.width as f64, p.height as f64)).collect();
        let all = mm(a, "bleed_mm")?;
        let side = |key: &str| -> Result<f64> { Ok(mm(a, key)?.or(all).unwrap_or(0.0)) };
        let spec = AddBleed {
            amount: [side("left_mm")?, side("bottom_mm")?, side("right_mm")?, side("top_mm")?],
            method: match a.opt_str("method")?.unwrap_or("mirror") {
                "mirror" => BleedMethod::Mirror,
                "scale" => BleedMethod::Scale,
                m => return Err(bad(format!("unknown method {m:?}"))),
            },
        };
        let content = content_arg(a)?;
        let sheets = impose::add_bleed(&sizes, &spec).map_err(|e| bad(e.to_string()))?;
        let bytes = self.session.bleed_pdf(id, &spec, content).map_err(failed)?;
        let path = self.resolve(a.str("path")?, true)?;
        write_atomic(&path, &bytes)?;
        let first = sheets.first().map(|s| s.size).unwrap_or_default();
        Ok(json!({ "pages": sheets.len(), "first_page_mm": [first.0 / MM, first.1 / MM], "bytes": bytes.len(), "path": path.to_string_lossy() }))
    }
}
