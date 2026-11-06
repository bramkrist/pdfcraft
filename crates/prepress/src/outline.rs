//! Text to outlines: every glyph drawn from an embedded font becomes a filled (or stroked) path
//! with the same shape, position and paint, so the press never needs the font.
//!
//! How it stays exact:
//!
//! 1. The page is interpreted by the renderer's own interpreter (hayro) with a device that only
//!    records glyphs: the outline of each glyph and the matrix it is drawn with, in the order they
//!    are drawn. That is where the font program, encodings, CID maps, spacing, scaling, rise and
//!    `TJ` offsets are all resolved, the same way the page renders.
//! 2. The content streams are walked again here in the same order (page, then each form at its
//!    `Do`), counting how many glyph draws each `BT … ET` block makes. Each block takes its run of
//!    recorded glyphs. A page whose counts do not add up exactly is left alone.
//! 3. A block is replaced by its glyphs as paths, keeping every colour, graphics-state and
//!    marked-content operator of the block in place; positioning operators that only work
//!    inside a text object (`Td`, `TD`, `Tm`, `T*`) are dropped, and text-state operators that
//!    persist past `ET` (`Tf`, `Tc`, `Tw`, `Tz`, `TL`, `Ts`, `Tr`) are kept. A block with clipping
//!    text, a Type 3 font, or a font that is not embedded is kept as text and reported.
//!
//! Everything outside the replaced blocks is copied byte for byte, and the file is saved
//! incrementally, so images, other fonts, colour spaces and the rest of the file are untouched.

use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use hayro::hayro_interpret::font::Glyph;
use hayro::hayro_interpret::hayro_syntax::Pdf;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, interpret_page,
};
use kurbo::{Affine, BezPath, PathEl, Rect};
use pdfcraft_content::{Matrix, Op, parse};
use pdfcraft_cos::{Dict, Document, ObjRef, Object, SaveOptions, Stream, write_incremental};
use serde::Serialize;

/// Largest content stream rewritten.
const MAX_STREAM: usize = 256 << 20;
const MAX_DEPTH: usize = 16;
/// How far the interpreter's matrix may differ from ours before a page is left alone.
const MATRIX_TOLERANCE: f64 = 1e-3;

/// What [`outline_text`] did.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct OutlineReport {
    /// Text objects turned into paths.
    pub outlined: usize,
    /// Glyphs turned into paths.
    pub glyphs: usize,
    /// Text objects left as text, with the page and why.
    pub kept: Vec<Kept>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Kept {
    pub page: usize,
    pub reason: String,
}

#[derive(Clone, Debug)]
enum Mark {
    /// A filled glyph: its outline in glyph space, which font and glyph it is, and where it goes.
    Fill {
        key: (u128, u32),
        path: BezPath,
        at: Affine,
    },
    Stroke(BezPath),
    /// Drawn but not painted (render mode 3).
    Invisible,
    /// Cannot become a path here: clipping text, a Type 3 glyph.
    Unsupported(&'static str),
}

struct Recorded {
    mark: Mark,
    ctm: Affine,
}

/// A device that keeps the glyphs and nothing else.
#[derive(Default)]
struct GlyphRecorder {
    glyphs: Vec<Recorded>,
}

impl<'a> Device<'a> for GlyphRecorder {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph(&mut self, glyph: &Glyph<'a>, transform: Affine, glyph_transform: Affine, _: &Paint<'a>, mode: &GlyphDrawMode) {
        let mark = match (glyph, mode) {
            (_, GlyphDrawMode::Invisible) => Mark::Invisible,
            (_, GlyphDrawMode::Clip) => Mark::Unsupported("teks sebagai clipping path"),
            (Glyph::Type3(_), _) => Mark::Unsupported("font Type 3"),
            (Glyph::Outline(o), GlyphDrawMode::Fill) => {
                Mark::Fill { key: (o.font_cache_key(), o.glyph_id().to_u32()), path: o.outline(), at: glyph_transform }
            }
            (Glyph::Outline(o), GlyphDrawMode::Stroke(_)) => Mark::Stroke(glyph_transform * o.outline()),
        };
        self.glyphs.push(Recorded { mark, ctm: transform });
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn draw_rect(&mut self, _: &Rect, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
}

/// The glyphs of every page, in drawing order, in each page's user space.
fn record(bytes: &Arc<Vec<u8>>) -> Option<Vec<Vec<Recorded>>> {
    catch_unwind(AssertUnwindSafe(|| {
        let pdf = Pdf::new_with_password(bytes.clone(), "").ok()?;
        let settings = InterpreterSettings { render_annotations: false, ..InterpreterSettings::default() };
        let mut out = Vec::new();
        for page in pdf.pages().iter() {
            let cache = InterpreterCache::new();
            let (w, h) = page.render_dimensions();
            let mut ctx = Context::new(Affine::IDENTITY, Rect::new(0.0, 0.0, f64::from(w), f64::from(h)), &cache, page.xref(), settings.clone());
            let mut dev = GlyphRecorder::default();
            interpret_page(page, &mut ctx, &mut dev);
            out.push(dev.glyphs);
        }
        Some(out)
    }))
    .ok()
    .flatten()
}

/// Text state the walk needs to count glyph draws.
#[derive(Clone)]
struct TextState {
    font: Option<std::rc::Rc<LoadedFont>>,
    render_mode: i64,
}

/// A font resource and the code splitting of its strings, read once per font object.
struct LoadedFont {
    dict: Dict,
    metrics: pdfcraft_fonts::pdf::Metrics,
}

#[derive(Clone)]
struct GState {
    ctm: Matrix,
    text: TextState,
}

/// One stream's planned rewrite: byte ranges of `BT … ET` blocks and what replaces them.
#[derive(Default)]
struct Plan {
    edits: Vec<(std::ops::Range<usize>, String)>,
    /// Glyph forms the rewritten stream places (resource names).
    uses: std::collections::BTreeSet<String>,
}

struct Walker<'a> {
    doc: &'a Document,
    glyphs: Vec<Recorded>,
    next: usize,
    page: usize,
    report: OutlineReport,
    /// Streams already planned (forms used more than once are rewritten once).
    planned: HashSet<ObjRef>,
    plans: Vec<(Option<ObjRef>, Plan)>,
    /// Forms seen on this page: their plans only apply if the whole page adds up.
    failed: bool,
    fonts: std::collections::HashMap<ObjRef, std::rc::Rc<LoadedFont>>,
    /// Every distinct glyph shape becomes one form XObject, placed wherever the glyph is shown.
    shapes: Shapes,
}

/// Glyph shapes collected for form XObjects: name, bounding box, path in glyph space.
#[derive(Default)]
struct Shapes {
    by_key: std::collections::HashMap<(u128, u32), Vec<usize>>,
    list: Vec<(String, Rect, BezPath)>,
}

impl Shapes {
    /// Forget the shapes added after the first `n` (a page that was left alone).
    fn truncate(&mut self, n: usize) {
        self.list.truncate(n);
        for v in self.by_key.values_mut() {
            v.retain(|&i| i < n);
        }
        self.by_key.retain(|_, v| !v.is_empty());
    }

    /// The form name for this glyph shape, adding it when new. Two glyphs with the same font
    /// and id but different outlines (synthetic styles) get separate forms.
    fn name(&mut self, key: (u128, u32), path: &BezPath) -> String {
        let bbox = kurbo::Shape::bounding_box(path);
        let same = |r: &Rect| {
            (r.x0 - bbox.x0).abs() < 1e-6 && (r.y0 - bbox.y0).abs() < 1e-6 && (r.x1 - bbox.x1).abs() < 1e-6 && (r.y1 - bbox.y1).abs() < 1e-6
        };
        if let Some(found) = self.by_key.get(&key).and_then(|v| v.iter().find(|&&i| self.list.get(i).is_some_and(|(_, r, _)| same(r))))
            && let Some((n, _, _)) = self.list.get(*found)
        {
            return n.clone();
        }
        let name = format!("OlG{}", self.list.len());
        self.by_key.entry(key).or_default().push(self.list.len());
        self.list.push((name.clone(), bbox, path.clone()));
        name
    }
}

/// A coordinate to 3 decimals without a trailing zero (glyph geometry; 0.001 pt is far below
/// anything a press resolves).
fn push_number(out: &mut String, v: f64) {
    use std::fmt::Write;
    let v = if v.is_finite() { v } else { 0.0 };
    let m = (v * 1000.0).round() as i64;
    if m == 0 {
        out.push('0');
        return;
    }
    if m < 0 {
        out.push('-');
    }
    let a = m.unsigned_abs();
    let _ = write!(out, "{}", a / 1000);
    let frac = a % 1000;
    if frac != 0 {
        let mut f = format!("{frac:03}");
        while f.ends_with('0') {
            f.pop();
        }
        out.push('.');
        out.push_str(&f);
    }
}

/// A matrix scale term to 7 decimals.
fn push_fine(out: &mut String, v: f64) {
    let v = if v.is_finite() { v } else { 0.0 };
    let s = format!("{v:.7}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    out.push_str(if s == "-0" || s.is_empty() { "0" } else { s });
}

fn number(v: f64) -> String {
    let mut s = String::new();
    push_number(&mut s, v);
    s
}

fn path_ops(path: &BezPath, out: &mut String) {
    let mut last = kurbo::Point::ZERO;
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                out.push_str(&format!("{} {} m\n", number(p.x), number(p.y)));
                last = p;
            }
            PathEl::LineTo(p) => {
                out.push_str(&format!("{} {} l\n", number(p.x), number(p.y)));
                last = p;
            }
            PathEl::QuadTo(c, p) => {
                // Quadratic to cubic: control points two thirds of the way to the quadratic one.
                let c1 = last + (c - last) * (2.0 / 3.0);
                let c2 = p + (c - p) * (2.0 / 3.0);
                out.push_str(&format!("{} {} {} {} {} {} c\n", number(c1.x), number(c1.y), number(c2.x), number(c2.y), number(p.x), number(p.y)));
                last = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                out.push_str(&format!("{} {} {} {} {} {} c\n", number(c1.x), number(c1.y), number(c2.x), number(c2.y), number(p.x), number(p.y)));
                last = p;
            }
            PathEl::ClosePath => out.push_str("h\n"),
        }
    }
}

fn affine(m: &Matrix) -> Affine {
    Affine::new(m.0)
}

fn close(a: Affine, b: Affine) -> bool {
    a.as_coeffs().iter().zip(b.as_coeffs()).all(|(x, y)| (x - y).abs() <= MATRIX_TOLERANCE * (1.0 + x.abs().max(y.abs())))
}

/// How many glyph draws a string makes in `ts`.
fn draws(ts: &TextState, s: &[u8]) -> usize {
    let codes = match &ts.font {
        Some(f) => f.metrics.codes(s).len(),
        None => s.len(),
    };
    let per = match ts.render_mode {
        2 | 6 => 2,
        _ => 1,
    };
    codes.saturating_mul(per)
}

fn shown_strings(op: &Op) -> Vec<&[u8]> {
    match op.op.as_slice() {
        b"Tj" | b"'" => op.operands.first().and_then(|o| o.as_string()).map(|s| vec![s.bytes.as_slice()]).unwrap_or_default(),
        b"\"" => op.operands.get(2).and_then(|o| o.as_string()).map(|s| vec![s.bytes.as_slice()]).unwrap_or_default(),
        b"TJ" => op
            .operands
            .first()
            .and_then(|o| o.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_string().map(|s| s.bytes.as_slice())).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn font_embedded(doc: &Document, font: &Dict) -> bool {
    match font.name(b"Subtype") {
        Some(b"Type3") => true,
        Some(b"Type0") => font
            .get(b"DescendantFonts")
            .map(|d| doc.resolve(d))
            .and_then(|d| d.as_array().and_then(|a| a.first().cloned()))
            .map(|d| doc.resolve(&d))
            .and_then(|d| d.as_dict().and_then(|dd| dd.get(b"FontDescriptor").cloned()))
            .map(|fd| doc.resolve(&fd))
            .and_then(|fd| fd.as_dict().map(|d| d.contains(b"FontFile") || d.contains(b"FontFile2") || d.contains(b"FontFile3")))
            .unwrap_or(false),
        _ => font
            .get(b"FontDescriptor")
            .map(|fd| doc.resolve(fd))
            .and_then(|fd| fd.as_dict().map(|d| d.contains(b"FontFile") || d.contains(b"FontFile2") || d.contains(b"FontFile3")))
            .unwrap_or(false),
    }
}

/// Operators that may stand outside a text object and so survive when its glyphs become paths.
fn kept_outside_text(op: &[u8]) -> bool {
    matches!(
        op,
        b"g" | b"G"
            | b"rg"
            | b"RG"
            | b"k"
            | b"K"
            | b"cs"
            | b"CS"
            | b"sc"
            | b"SC"
            | b"scn"
            | b"SCN"
            | b"gs"
            | b"w"
            | b"J"
            | b"j"
            | b"M"
            | b"d"
            | b"ri"
            | b"i"
            | b"Tf"
            | b"Tc"
            | b"Tw"
            | b"Tz"
            | b"TL"
            | b"Ts"
            | b"Tr"
            | b"BMC"
            | b"BDC"
            | b"EMC"
            | b"MP"
            | b"DP"
    )
}

impl<'a> Walker<'a> {
    /// The next `n` recorded glyphs, as a range into `glyphs`.
    fn take(&mut self, n: usize) -> Option<std::ops::Range<usize>> {
        let end = self.next.checked_add(n)?;
        if end > self.glyphs.len() {
            return None;
        }
        let r = self.next..end;
        self.next = end;
        Some(r)
    }

    fn keep(&mut self, why: &str) {
        self.report.kept.push(Kept { page: self.page, reason: why.to_string() });
    }

    /// Walk one stream; plan its rewrite when `target` is a stream not planned before.
    #[allow(clippy::too_many_lines)]
    fn walk(&mut self, data: &[u8], res: &Dict, mut st: GState, depth: usize, target: Option<Option<ObjRef>>, seen: &mut Vec<ObjRef>) {
        let parsed = parse(data);
        let mut plan = Plan::default();
        let mut stack: Vec<GState> = Vec::new();
        let ops = &parsed.ops;
        let mut i = 0;
        while i < ops.len() {
            let Some(op) = ops.get(i) else { break };
            match op.op.as_slice() {
                b"q" => stack.push(st.clone()),
                b"Q" => {
                    if let Some(s) = stack.pop() {
                        st = s;
                    }
                }
                b"cm" => {
                    if let Some(m) = Matrix::from_operands(&op.operands) {
                        st.ctm = m.then(&st.ctm);
                    }
                }
                b"Tf" => st.text.font = op.operands.first().and_then(|n| n.as_name()).and_then(|n| self.font(res, n)),
                b"Tr" => st.text.render_mode = op.operands.first().and_then(Object::as_int).unwrap_or(0),
                b"Do" => {
                    if let Some(n) = op.operands.first().and_then(|n| n.as_name()) {
                        self.form(res, n, &st, depth, seen);
                    }
                }
                b"BT" => {
                    // The block runs to the matching ET.
                    let start = i;
                    let mut end = i;
                    while end < ops.len() && ops.get(end).is_some_and(|o| o.op != b"ET") {
                        end += 1;
                    }
                    let block = ops.get(start..=end.min(ops.len().saturating_sub(1))).unwrap_or(&[]);
                    self.block(block, data, res, &mut st, target.is_some(), &mut plan);
                    i = end;
                }
                _ => {}
            }
            i += 1;
        }
        if let Some(t) = target {
            self.plans.push((t, plan));
        }
    }

    fn font(&mut self, res: &Dict, name: &[u8]) -> Option<std::rc::Rc<LoadedFont>> {
        let f = res.get(b"Font").map(|f| self.doc.resolve(f)).and_then(|f| f.as_dict().and_then(|d| d.get(name).cloned()))?;
        if let Some(r) = f.as_ref()
            && let Some(hit) = self.fonts.get(&r)
        {
            return Some(hit.clone());
        }
        let dict = self.doc.resolve(&f).as_dict().cloned()?;
        let loaded = std::rc::Rc::new(LoadedFont { metrics: pdfcraft_fonts::pdf::Metrics::from_dict(self.doc, &dict), dict });
        if let Some(r) = f.as_ref() {
            self.fonts.insert(r, loaded.clone());
        }
        Some(loaded)
    }

    /// One `BT … ET` block: take its glyphs and, when every one can be a path and `rewrite`,
    /// plan its replacement.
    fn block(&mut self, block: &[Op], data: &[u8], res: &Dict, st: &mut GState, rewrite: bool, plan: &mut Plan) {
        let mut out = String::new();
        let mut uses: Vec<String> = Vec::new();
        let mut why: Option<String> = None;
        let ctm = affine(&st.ctm);
        let mut ts = st.text.clone();
        for op in block {
            match op.op.as_slice() {
                b"BT" | b"ET" | b"Td" | b"TD" | b"Tm" | b"T*" => {}
                b"Tj" | b"TJ" | b"'" | b"\"" => {
                    if let Some(f) = &ts.font
                        && !font_embedded(self.doc, &f.dict)
                    {
                        why.get_or_insert_with(|| "font tidak tertanam".to_string());
                    }
                    let n: usize = shown_strings(op).iter().map(|s| draws(&ts, s)).sum();
                    let Some(range) = self.take(n) else {
                        self.failed = true;
                        return;
                    };
                    for g in self.glyphs.get(range).unwrap_or(&[]) {
                        if !close(g.ctm, ctm) {
                            self.failed = true;
                            return;
                        }
                        match &g.mark {
                            Mark::Fill { key, path, at } => {
                                let name = self.shapes.name(*key, path);
                                out.push_str("q ");
                                // Glyph space is 1000 units to the em, so the scale terms are
                                // small numbers: they keep 7 decimals, the offsets 3.
                                let c = at.as_coeffs();
                                for v in c.iter().take(4) {
                                    push_fine(&mut out, *v);
                                    out.push(' ');
                                }
                                for v in c.iter().skip(4) {
                                    push_number(&mut out, *v);
                                    out.push(' ');
                                }
                                out.push_str("cm /");
                                out.push_str(&name);
                                out.push_str(" Do Q\n");
                                uses.push(name);
                            }
                            Mark::Stroke(p) => {
                                path_ops(p, &mut out);
                                out.push_str("S\n");
                            }
                            Mark::Invisible => {}
                            Mark::Unsupported(r) => {
                                why.get_or_insert_with(|| r.to_string());
                            }
                        }
                    }
                }
                other => {
                    if other == b"Tf" {
                        ts.font = op.operands.first().and_then(|n| n.as_name()).and_then(|n| self.font(res, n));
                    }
                    if other == b"Tr" {
                        ts.render_mode = op.operands.first().and_then(Object::as_int).unwrap_or(0);
                    }
                    if kept_outside_text(other) {
                        // The operator exactly as written in the source.
                        out.push_str(&String::from_utf8_lossy(data.get(op.span.clone()).unwrap_or(&[])));
                        out.push('\n');
                    } else {
                        why.get_or_insert_with(|| format!("operator {} di dalam teks", String::from_utf8_lossy(other)));
                    }
                }
            }
        }
        // Text state set inside the block lasts after it.
        st.text = ts;
        let (Some(first), Some(last)) = (block.first(), block.last()) else { return };
        if let Some(w) = why {
            self.keep(&w);
            return;
        }
        if rewrite {
            plan.uses.extend(uses);
            plan.edits.push((first.span.start..last.span.end, out));
            self.report.outlined += 1;
        }
    }

    fn form(&mut self, res: &Dict, name: &[u8], st: &GState, depth: usize, seen: &mut Vec<ObjRef>) {
        let Some(xo) = res.get(b"XObject").map(|x| self.doc.resolve(x)).and_then(|x| x.as_dict().and_then(|d| d.get(name).cloned())) else { return };
        let r = xo.as_ref();
        let obj = self.doc.resolve(&xo);
        let Object::Stream(s) = &*obj else { return };
        if s.dict.name(b"Subtype") != Some(b"Form") || depth >= MAX_DEPTH || r.is_some_and(|r| seen.contains(&r)) {
            return;
        }
        let Ok(data) = s.decoded_within(MAX_STREAM) else { return };
        let mut inner = st.clone();
        if let Some(m) = s.dict.get(b"Matrix").map(|m| self.doc.resolve(m)).and_then(|m| m.as_array().and_then(|a| Matrix::from_operands(a))) {
            inner.ctm = m.then(&st.ctm);
        }
        let form_res = s.dict.get(b"Resources").map(|x| self.doc.resolve(x)).and_then(|x| x.as_dict().cloned()).unwrap_or_else(|| res.clone());
        // A form is rewritten once, from its first use, and only when it is its own object.
        let target = match r {
            Some(r) if self.planned.insert(r) => Some(Some(r)),
            _ => None,
        };
        if let Some(r) = r {
            seen.push(r);
        }
        self.walk(&data, &form_res, inner, depth + 1, target, seen);
        if r.is_some() {
            seen.pop();
        }
    }
}

fn apply(data: &[u8], plan: &Plan) -> Vec<u8> {
    let mut edits: Vec<&(std::ops::Range<usize>, String)> = plan.edits.iter().collect();
    edits.sort_by_key(|e| e.0.start);
    let mut out = Vec::with_capacity(data.len());
    let mut at = 0;
    for (range, text) in edits {
        if range.start < at || range.end > data.len() {
            continue;
        }
        out.extend_from_slice(data.get(at..range.start).unwrap_or(&[]));
        out.extend_from_slice(text.as_bytes());
        at = range.end;
    }
    out.extend_from_slice(data.get(at..).unwrap_or(&[]));
    out
}

fn page_content(doc: &Document, page: &Dict) -> Vec<u8> {
    let Some(c) = page.get(b"Contents") else { return Vec::new() };
    let parts: Vec<Object> = match &*doc.resolve(c) {
        Object::Array(a) => a.clone(),
        other => vec![other.clone()],
    };
    let mut out = Vec::new();
    for p in parts {
        if let Object::Stream(s) = &*doc.resolve(&p)
            && let Ok(d) = s.decoded_within(MAX_STREAM)
        {
            out.extend_from_slice(&d);
            out.push(b'\n');
        }
    }
    out
}

/// Turn the text of `pdf` into outlines (see the module documentation). Returns the new file
/// (the original with the changed streams appended) and what was done; a file with no text to
/// outline comes back unchanged.
pub fn outline_text(pdf: Vec<u8>) -> Result<(Vec<u8>, OutlineReport), String> {
    let bytes = Arc::new(pdf);
    let mut doc = Document::open(bytes.clone()).map_err(|e| format!("cannot open the PDF: {e}"))?;
    let recorded = record(&bytes).ok_or("the pages could not be interpreted")?;
    let pages = pdfcraft_model::pages(&doc);
    let mut report = OutlineReport::default();
    let mut planned = HashSet::new();
    // (object, new content, glyph forms it places) for pages and for forms.
    let mut page_writes: Vec<(ObjRef, Vec<u8>, Vec<String>, Dict)> = Vec::new();
    let mut form_writes: Vec<(ObjRef, Vec<u8>, Vec<String>)> = Vec::new();
    let mut fonts_cache = std::collections::HashMap::new();
    let mut shapes = Shapes::default();
    for (i, (page, glyphs)) in pages.iter().zip(recorded).enumerate() {
        let data = page_content(&doc, &page.dict);
        let res = page.dict.get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default();
        let total = glyphs.len();
        let shapes_before = shapes.list.len();
        let mut w = Walker {
            doc: &doc,
            glyphs,
            next: 0,
            page: i + 1,
            report: OutlineReport::default(),
            planned: planned.clone(),
            plans: Vec::new(),
            failed: false,
            fonts: std::mem::take(&mut fonts_cache),
            shapes: std::mem::take(&mut shapes),
        };
        let st = GState { ctm: Matrix::default(), text: TextState { font: None, render_mode: 0 } };
        let mut seen = Vec::new();
        w.walk(&data, &res, st, 0, Some(None), &mut seen);
        fonts_cache = std::mem::take(&mut w.fonts);
        shapes = std::mem::take(&mut w.shapes);
        if w.failed || w.next != total {
            shapes.truncate(shapes_before);
            report.kept.push(Kept { page: i + 1, reason: "susunan teks halaman ini tidak bisa dipetakan dengan pasti; teks dibiarkan".into() });
            continue;
        }
        planned = w.planned;
        report.outlined += w.report.outlined;
        report.glyphs += total;
        report.kept.extend(w.report.kept);
        for (target, plan) in w.plans {
            if plan.edits.is_empty() {
                continue;
            }
            let uses: Vec<String> = plan.uses.iter().cloned().collect();
            match target {
                None => page_writes.push((page.obj, apply(&data, &plan), uses, res.clone())),
                Some(r) => {
                    let obj = doc.resolve(&Object::Ref(r));
                    if let Object::Stream(s) = &*obj
                        && let Ok(d) = s.decoded_within(MAX_STREAM)
                    {
                        form_writes.push((r, apply(&d, &plan), uses));
                    }
                }
            }
        }
    }
    if page_writes.is_empty() && form_writes.is_empty() {
        return Ok((bytes.to_vec(), report));
    }
    // One form XObject per glyph shape, in glyph space; placed with the glyph's matrix it fills
    // in the colour current where it is drawn.
    let mut refs: std::collections::HashMap<String, ObjRef> = std::collections::HashMap::new();
    for (name, bbox, path) in &shapes.list {
        let mut content = String::new();
        path_ops(path, &mut content);
        content.push_str("f\n");
        let mut d = Dict::new();
        d.set(b"Type".to_vec(), Object::name("XObject"));
        d.set(b"Subtype".to_vec(), Object::name("Form"));
        // A generous box: its edge is a clip, and an antialiased clip edge would nick the glyph.
        let pad = 50.0_f64.max((bbox.width().max(bbox.height())) * 0.1);
        let b = [(bbox.x0 - pad).floor(), (bbox.y0 - pad).floor(), (bbox.x1 + pad).ceil(), (bbox.y1 + pad).ceil()];
        d.set(b"BBox".to_vec(), Object::Array(b.iter().map(|v| Object::Real(*v)).collect()));
        refs.insert(name.clone(), doc.add(Object::Stream(Stream::flate(d, content.as_bytes()))));
    }
    let with_glyphs = |doc: &Document, res: &Dict, uses: &[String]| -> Dict {
        let mut res = res.clone();
        let mut xo = res.get(b"XObject").map(|x| doc.resolve(x)).and_then(|x| x.as_dict().cloned()).unwrap_or_default();
        for u in uses {
            if let Some(r) = refs.get(u) {
                xo.set(u.as_bytes().to_vec(), Object::Ref(*r));
            }
        }
        res.set(b"XObject".to_vec(), Object::Dict(xo));
        res
    };
    for (r, data, uses) in form_writes {
        let obj = doc.resolve(&Object::Ref(r));
        if let Object::Stream(s) = &*obj {
            let mut dict = s.dict.clone();
            for k in [b"Filter".as_slice(), b"DecodeParms", b"Length", b"DL"] {
                dict.remove(k);
            }
            let res = dict.get(b"Resources").map(|x| doc.resolve(x)).and_then(|x| x.as_dict().cloned()).unwrap_or_default();
            dict.set(b"Resources".to_vec(), Object::Dict(with_glyphs(&doc, &res, &uses)));
            doc.set(r, Object::Stream(Stream::flate(dict, &data)));
        }
    }
    for (r, data, uses, res) in page_writes {
        let contents = doc.add(Object::Stream(Stream::flate(Dict::new(), &data)));
        let res = with_glyphs(&doc, &res, &uses);
        doc.update_dict(r, |d| {
            d.set(b"Contents".to_vec(), Object::Ref(contents));
            d.set(b"Resources".to_vec(), Object::Dict(res));
        })
        .map_err(|e| e.to_string())?;
    }
    let out = write_incremental(&doc, &SaveOptions::default()).map_err(|e| e.to_string())?;
    Ok((out, report))
}

#[cfg(test)]
#[path = "outline_tests.rs"]
mod tests;
