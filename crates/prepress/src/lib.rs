//! pdfcraft-prepress — preflight for a press room (L4).
//!
//! [`analyze`] reads a document and reports what a prepress desk checks before a file goes to
//! press, without changing anything:
//!
//! - page sizes, `TrimBox`/`BleedBox` and the bleed on each side;
//! - fonts: embedded, subset, Type 3, or missing (a font that is not embedded prints in whatever
//!   the RIP substitutes);
//! - images: effective resolution where they are placed, colour space, and images that are not
//!   in the file at all (external streams, OPI proxies, broken references);
//! - colour: which process inks are used (cyan, magenta, yellow, black), every spot colour by
//!   name, registration colour, RGB, Lab and ICC-based colour;
//! - small text in rich black, hairlines, transparency, overprint, the largest ink total of any
//!   vector fill, the output intent (PDF/X).
//!
//! The content streams are interpreted for the graphics state only (matrix, colours, line width,
//! text size): nothing is rendered, so ink totals inside images are not measured.
//! [`checks`] turns a [`Report`] into pass / warning / fail lines with the thresholds of
//! [`Thresholds`], the same ones PIXELINDIE's upload preflight uses.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod inks;
pub mod outline;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use pdfcraft_content::{Matrix, parse};
use pdfcraft_cos::{Dict, Document, ObjRef, Object};
use serde::Serialize;

const MM: f64 = 72.0 / 25.4;
/// Nested form XObjects followed at most this deep.
const MAX_DEPTH: usize = 16;
/// Operators interpreted per document at most (a defence against pathological files).
const MAX_OPS: usize = 20_000_000;
/// Largest content stream decoded.
/// Largest decoded CMYK image measured for ink total (bytes).
const MAX_IMAGE_BYTES: usize = 192 << 20;
const MAX_STREAM: usize = 256 << 20;

/// Limits a check is judged against.
#[derive(Clone, Debug, PartialEq)]
pub struct Thresholds {
    /// Images below this effective resolution are a warning.
    pub warn_ppi: f64,
    /// Images below this are a failure.
    pub fail_ppi: f64,
    /// Images smaller than this (mm, both sides) are not judged on resolution.
    pub ignore_image_mm: f64,
    /// Strokes thinner than this (points) are hairlines.
    pub min_line_pt: f64,
    /// Text below this size (points) should be 100 % black, not rich black.
    pub small_text_pt: f64,
    /// Highest acceptable ink total (percent).
    pub max_tac: f64,
    /// Bleed expected on every side (mm).
    pub bleed_mm: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds { warn_ppi: 300.0, fail_ppi: 200.0, ignore_image_mm: 20.0, min_line_pt: 0.25, small_text_pt: 10.0, max_tac: 300.0, bleed_mm: 3.0 }
    }
}

/// One page.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct PageInfo {
    /// 1-based.
    pub number: usize,
    /// Display size (crop box, rotated), millimetres.
    pub size_mm: [f64; 2],
    /// Trim size when a `TrimBox` is set.
    pub trim_mm: Option<[f64; 2]>,
    /// Bleed beyond the trim (left, bottom, right, top) when both `TrimBox` and `BleedBox` are set.
    pub bleed_mm: Option<[f64; 4]>,
    pub transparency: bool,
    pub overprint: bool,
    pub hairlines: usize,
    pub rich_black_small_text: usize,
    /// Highest C+M+Y+K of any DeviceCMYK fill or stroke on the page, percent.
    pub max_tac: f64,
    pub annotations: usize,
    /// Bytes of content that could not be read.
    pub unreadable: usize,
}

/// One font.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct FontInfo {
    pub name: String,
    pub subtype: String,
    pub embedded: bool,
    pub subset: bool,
    pub pages: Vec<usize>,
}

/// One placement of an image.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ImageUse {
    pub page: usize,
    pub width_px: i64,
    pub height_px: i64,
    /// Lower of the horizontal and vertical effective resolution where placed.
    pub ppi: f64,
    /// Placed size, millimetres.
    pub size_mm: [f64; 2],
    pub colour: String,
    pub bits: i64,
    pub soft_mask: bool,
    /// Image data not in the file: an external stream (`/F`), an OPI proxy, or a missing object.
    pub missing: Option<String>,
    /// For 8-bit CMYK images: the highest C+M+Y+K of any pixel, percent, and the share of
    /// pixels above the TAC limit.
    pub tac: Option<ImageTac>,
}

/// Ink total inside one CMYK image.
#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
pub struct ImageTac {
    pub max: f64,
    /// 0..1.
    pub over: f64,
}

/// One spot colour.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Spot {
    pub name: String,
    pub pages: Vec<usize>,
}

/// The inks the document asks for.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Inks {
    pub cyan: bool,
    pub magenta: bool,
    pub yellow: bool,
    pub black: bool,
    pub spots: Vec<Spot>,
    /// Objects in registration colour (`/Separation /All`): they print on every plate.
    pub registration: bool,
    pub rgb: bool,
    pub gray: bool,
    pub lab: bool,
    /// Process colour given as ICC-based colour.
    pub icc: bool,
}

/// Everything [`analyze`] found.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Report {
    pub pages: Vec<PageInfo>,
    pub fonts: Vec<FontInfo>,
    pub images: Vec<ImageUse>,
    pub inks: Inks,
    /// `OutputConditionIdentifier` of the first output intent.
    pub output_intent: Option<String>,
    /// `GTS_PDFXVersion` from the document information.
    pub pdfx: Option<String>,
    /// The interpreter stopped early (operator budget).
    pub truncated: bool,
}

/// How a check came out.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Info,
    Na,
    Warn,
    Fail,
}

/// One line of a preflight report.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Check {
    pub id: &'static str,
    pub title: &'static str,
    pub status: Status,
    pub message: String,
}

// ---- colour --------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Gray,
    Rgb,
    Cmyk,
    Lab,
    Separation(Vec<u8>),
    DeviceN(Vec<Vec<u8>>),
    Pattern,
    /// Indexed into a base (colours of the lookup table are not inspected value by value).
    Indexed(Box<Kind>),
    Other,
}

#[derive(Clone, Debug)]
struct Colour {
    kind: Kind,
    values: Vec<f64>,
    icc: bool,
    /// Values stand for "any amount" (images, shadings): inks count, ink totals do not.
    nominal: bool,
}

impl Colour {
    fn gray() -> Self {
        Colour { kind: Kind::Gray, values: vec![0.0], icc: false, nominal: false }
    }
}

fn kind_of(doc: &Document, res: &Dict, o: &Object, depth: usize) -> (Kind, bool) {
    if depth > 4 {
        return (Kind::Other, false);
    }
    let o = doc.resolve(o);
    match &*o {
        Object::Name(n) => match n.as_slice() {
            b"DeviceGray" | b"G" => (Kind::Gray, false),
            b"DeviceRGB" | b"RGB" => (Kind::Rgb, false),
            b"DeviceCMYK" | b"CMYK" => (Kind::Cmyk, false),
            b"Pattern" => (Kind::Pattern, false),
            other => {
                let named = res.get(b"ColorSpace").map(|c| doc.resolve(c)).and_then(|c| c.as_dict().and_then(|d| d.get(other).cloned()));
                match named {
                    Some(c) => kind_of(doc, res, &c, depth + 1),
                    None => (Kind::Other, false),
                }
            }
        },
        Object::Array(a) => {
            let family = a.first().and_then(|f| f.as_name()).unwrap_or(b"");
            match family {
                b"ICCBased" => {
                    let n = a.get(1).map(|s| doc.resolve(s)).and_then(|s| s.as_dict().and_then(|d| d.int(b"N"))).unwrap_or(0);
                    let k = match n {
                        1 => Kind::Gray,
                        3 => Kind::Rgb,
                        4 => Kind::Cmyk,
                        _ => Kind::Other,
                    };
                    (k, true)
                }
                b"CalGray" => (Kind::Gray, false),
                b"CalRGB" => (Kind::Rgb, false),
                b"Lab" => (Kind::Lab, false),
                b"Indexed" | b"I" => {
                    let base = a.get(1).map(|b| kind_of(doc, res, b, depth + 1)).unwrap_or((Kind::Other, false));
                    (Kind::Indexed(Box::new(base.0)), base.1)
                }
                b"Separation" => (Kind::Separation(a.get(1).and_then(|n| doc.resolve(n).as_name().map(<[u8]>::to_vec)).unwrap_or_default()), false),
                b"DeviceN" => {
                    let names = a
                        .get(1)
                        .map(|n| doc.resolve(n))
                        .and_then(|n| n.as_array().map(|v| v.iter().filter_map(|x| doc.resolve(x).as_name().map(<[u8]>::to_vec)).collect()))
                        .unwrap_or_default();
                    (Kind::DeviceN(names), false)
                }
                b"Pattern" => (Kind::Pattern, false),
                _ => (Kind::Other, false),
            }
        }
        _ => (Kind::Other, false),
    }
}

fn components(k: &Kind) -> usize {
    match k {
        Kind::Gray | Kind::Separation(_) | Kind::Indexed(_) => 1,
        Kind::Rgb | Kind::Lab => 3,
        Kind::Cmyk => 4,
        Kind::DeviceN(n) => n.len(),
        _ => 0,
    }
}

/// The initial colour of a colour space (ISO 32000-2 8.6.8 table 73: black, or tint 1).
fn initial(kind: Kind, icc: bool) -> Colour {
    let values = match &kind {
        Kind::Cmyk => vec![0.0, 0.0, 0.0, 1.0],
        Kind::Separation(_) => vec![1.0],
        Kind::DeviceN(n) => vec![1.0; n.len()],
        k => vec![0.0; components(k)],
    };
    Colour { kind, values, icc, nominal: false }
}

// ---- interpretation ------------------------------------------------------------------------

#[derive(Clone)]
struct State {
    ctm: Matrix,
    fill: Colour,
    stroke: Colour,
    line_width: f64,
    font_size: f64,
    text_scale: f64,
    /// The font resource last selected with `Tf`; noted when text is shown with it.
    font: Option<Vec<u8>>,
}

struct Walker<'a> {
    doc: &'a Document,
    report: Report,
    spots: BTreeMap<Vec<u8>, BTreeSet<usize>>,
    fonts: BTreeMap<String, FontInfo>,
    page: usize,
    ops: usize,
    th: Thresholds,
    /// The renderer's view of the same file, opened when the first CMYK image needs decoding.
    pixels: Option<Option<hayro::hayro_interpret::hayro_syntax::Pdf>>,
    /// Image TAC already measured, per image object.
    tacs: HashMap<ObjRef, Option<ImageTac>>,
}

fn num(o: Option<&Object>) -> f64 {
    o.and_then(Object::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0)
}

fn scale_of(m: &Matrix) -> f64 {
    let [a, b, c, d, _, _] = m.0;
    let det = (a * d - b * c).abs();
    if det.is_finite() { det.sqrt() } else { 0.0 }
}

fn name_text(n: &[u8]) -> String {
    String::from_utf8_lossy(n).into_owned()
}

impl<'a> Walker<'a> {
    fn page_info(&mut self) -> Option<&mut PageInfo> {
        self.report.pages.get_mut(self.page.saturating_sub(1))
    }

    fn use_colour(&mut self, c: &Colour) {
        let page = self.page;
        if c.icc {
            self.report.inks.icc = true;
        }
        match &c.kind {
            Kind::Gray => {
                self.report.inks.gray = true;
                if c.values.first().copied().unwrap_or(0.0) < 1.0 {
                    self.report.inks.black = true;
                }
            }
            Kind::Rgb => self.report.inks.rgb = true,
            Kind::Cmyk => {
                let v = |i: usize| c.values.get(i).copied().unwrap_or(0.0);
                let inks = &mut self.report.inks;
                inks.cyan |= v(0) > 0.0;
                inks.magenta |= v(1) > 0.0;
                inks.yellow |= v(2) > 0.0;
                inks.black |= v(3) > 0.0;
                let tac = (v(0) + v(1) + v(2) + v(3)) * 100.0;
                if c.nominal {
                    return;
                }
                if let Some(p) = self.page_info() {
                    p.max_tac = p.max_tac.max(tac);
                }
            }
            Kind::Lab => self.report.inks.lab = true,
            Kind::Separation(n) => self.use_named(n, c.values.first().copied().unwrap_or(1.0), page),
            Kind::DeviceN(names) => {
                for (i, n) in names.clone().iter().enumerate() {
                    self.use_named(n, c.values.get(i).copied().unwrap_or(1.0), page);
                }
            }
            Kind::Indexed(base) => {
                // Lookup table colours are not inspected one by one: count the base family.
                let b = Colour { kind: (**base).clone(), values: vec![1.0; components(base).max(1)], icc: c.icc, nominal: true };
                self.use_colour(&b);
            }
            Kind::Pattern | Kind::Other => {}
        }
    }

    fn use_named(&mut self, name: &[u8], tint: f64, page: usize) {
        if tint <= 0.0 {
            return;
        }
        let inks = &mut self.report.inks;
        match name {
            b"All" => inks.registration = true,
            b"None" => {}
            b"Cyan" => inks.cyan = true,
            b"Magenta" => inks.magenta = true,
            b"Yellow" => inks.yellow = true,
            b"Black" => inks.black = true,
            other => {
                self.spots.entry(other.to_vec()).or_default().insert(page);
            }
        }
    }

    fn note_font(&mut self, res: &Dict, name: &[u8]) {
        let Some(font) = res.get(b"Font").map(|f| self.doc.resolve(f)).and_then(|f| f.as_dict().and_then(|d| d.get(name).cloned())) else { return };
        let font = self.doc.resolve(&font);
        let Some(fd) = font.as_dict() else { return };
        let base = fd.name(b"BaseFont").map(name_text).unwrap_or_else(|| "(tanpa nama)".into());
        let subtype = fd.name(b"Subtype").map(name_text).unwrap_or_default();
        let page = self.page;
        if let Some(f) = self.fonts.get_mut(&base) {
            if !f.pages.contains(&page) {
                f.pages.push(page);
            }
            return;
        }
        let embedded = if subtype == "Type3" {
            true
        } else {
            let descriptor_dict = if subtype == "Type0" {
                fd.get(b"DescendantFonts")
                    .map(|d| self.doc.resolve(d))
                    .and_then(|d| d.as_array().and_then(|a| a.first().cloned()))
                    .map(|d| self.doc.resolve(&d))
                    .and_then(|d| d.as_dict().and_then(|dd| dd.get(b"FontDescriptor").cloned()))
            } else {
                fd.get(b"FontDescriptor").cloned()
            };
            descriptor_dict
                .map(|d| self.doc.resolve(&d))
                .and_then(|d| d.as_dict().map(|dd| dd.contains(b"FontFile") || dd.contains(b"FontFile2") || dd.contains(b"FontFile3")))
                .unwrap_or(false)
        };
        let subset = base.len() > 7 && base.as_bytes().get(6) == Some(&b'+') && base.bytes().take(6).all(|b| b.is_ascii_uppercase());
        self.fonts.insert(base.clone(), FontInfo { name: base, subtype, embedded, subset, pages: vec![page] });
    }

    fn ext_gstate(&mut self, res: &Dict, name: &[u8], st: &mut State) {
        let Some(gs) = res.get(b"ExtGState").map(|g| self.doc.resolve(g)).and_then(|g| g.as_dict().and_then(|d| d.get(name).cloned())) else {
            return;
        };
        let gs = self.doc.resolve(&gs);
        let Some(g) = gs.as_dict() else { return };
        let transparent = g.get(b"CA").and_then(Object::as_f64).is_some_and(|v| v < 1.0)
            || g.get(b"ca").and_then(Object::as_f64).is_some_and(|v| v < 1.0)
            || g.name(b"BM").is_some_and(|b| b != b"Normal" && b != b"Compatible")
            || g.get(b"BM")
                .and_then(|b| b.as_array().map(|a| a.iter().any(|x| x.as_name().is_some_and(|n| n != b"Normal" && n != b"Compatible"))))
                .unwrap_or(false)
            || g.get(b"SMask").is_some_and(|m| m.as_name() != Some(b"None"));
        let overprint = matches!(g.get(b"OP"), Some(Object::Bool(true))) || matches!(g.get(b"op"), Some(Object::Bool(true)));
        if let Some(lw) = g.get(b"LW").and_then(Object::as_f64) {
            st.line_width = lw;
        }
        if let Some(p) = self.page_info() {
            p.transparency |= transparent;
            p.overprint |= overprint;
        }
    }

    fn image(&mut self, dict: &Dict, inline: bool, ctm: &Matrix, fill: &Colour, res: &Dict, (missing, obj): (Option<String>, Option<ObjRef>)) {
        let [a, b, c, d, _, _] = ctm.0;
        let w_pt = (a * a + b * b).sqrt();
        let h_pt = (c * c + d * d).sqrt();
        let get = |long: &[u8], short: &[u8]| {
            dict.get(long).or_else(|| if inline { dict.get(short) } else { None }).map(|o| self.doc.resolve(o)).and_then(|o| o.as_int()).unwrap_or(0)
        };
        let (wpx, hpx) = (get(b"Width", b"W"), get(b"Height", b"H"));
        let is_mask = matches!(dict.get(b"ImageMask").or_else(|| if inline { dict.get(b"IM") } else { None }), Some(Object::Bool(true)));
        let mut cmyk = false;
        let colour = if is_mask {
            self.use_colour(&fill.clone());
            "mask (warna isi)".to_string()
        } else if let Some(cs) = dict.get(b"ColorSpace").or_else(|| if inline { dict.get(b"CS") } else { None }) {
            let (k, icc) = kind_of(self.doc, res, cs, 0);
            let label = match &k {
                Kind::Gray => "Gray",
                Kind::Rgb => "RGB",
                Kind::Cmyk => "CMYK",
                Kind::Lab => "Lab",
                Kind::Separation(_) => "Spot",
                Kind::DeviceN(_) => "DeviceN",
                Kind::Indexed(_) => "Indexed",
                _ => "lain",
            };
            cmyk = k == Kind::Cmyk;
            let c = initial(k, icc);
            // A process image may use any of its inks; count them all.
            let c = Colour { values: vec![1.0; c.values.len()], nominal: true, ..c };
            self.use_colour(&c);
            format!("{label}{}", if icc { " (ICC)" } else { "" })
        } else if dict.name(b"Filter").is_some_and(|f| f == b"JPXDecode") {
            "JPX (warna di dalam gambar)".to_string()
        } else {
            "tidak diketahui".to_string()
        };
        let soft_mask = dict.get(b"SMask").is_some() || dict.get(b"SMaskInData").and_then(Object::as_int).is_some_and(|v| v > 0);
        if soft_mask && let Some(p) = self.page_info() {
            p.transparency = true;
        }
        let ppi = |px: i64, pt: f64| if pt > 1e-9 { px as f64 / (pt / 72.0) } else { 0.0 };
        let ppi = ppi(wpx, w_pt).min(ppi(hpx, h_pt));
        let bits = dict
            .get(b"BitsPerComponent")
            .or_else(|| if inline { dict.get(b"BPC") } else { None })
            .and_then(Object::as_int)
            .unwrap_or(if is_mask { 1 } else { 0 });
        let tac = match obj {
            Some(r) if cmyk && bits == 8 && missing.is_none() => self.image_tac(r, dict, wpx, hpx),
            _ => None,
        };
        self.report.images.push(ImageUse {
            page: self.page,
            width_px: wpx,
            height_px: hpx,
            ppi,
            size_mm: [w_pt / MM, h_pt / MM],
            colour,
            bits,
            soft_mask,
            missing,
            tac,
        });
    }

    /// Decode an 8-bit CMYK image as the renderer does and measure its ink total.
    fn image_tac(&mut self, r: ObjRef, dict: &Dict, w: i64, h: i64) -> Option<ImageTac> {
        if let Some(t) = self.tacs.get(&r) {
            return *t;
        }
        let t = self.measure_tac(r, dict, w, h);
        self.tacs.insert(r, t);
        t
    }

    fn measure_tac(&mut self, r: ObjRef, dict: &Dict, w: i64, h: i64) -> Option<ImageTac> {
        use hayro::hayro_interpret::hayro_syntax::object::stream::ImageDecodeParams;
        use hayro::hayro_interpret::hayro_syntax::object::{ObjectIdentifier, Stream as HStream};
        let pixels = usize::try_from(w).ok()?.checked_mul(usize::try_from(h).ok()?)?;
        let want = pixels.checked_mul(4)?;
        if pixels == 0 || want > MAX_IMAGE_BYTES {
            return None;
        }
        let doc = self.doc;
        let pdf =
            self.pixels.get_or_insert_with(|| hayro::hayro_interpret::hayro_syntax::Pdf::new_with_password(doc.bytes().clone(), "").ok()).as_ref()?;
        let id = ObjectIdentifier::new(i32::try_from(r.num).ok()?, i32::from(r.generation));
        let stream: HStream<'_> = pdf.xref().get(id)?;
        // The image codecs need the dictionary's size, as the renderer passes it.
        let params = ImageDecodeParams {
            bpc: Some(8),
            num_components: Some(4),
            width: u32::try_from(w).ok()?,
            height: u32::try_from(h).ok()?,
            ..ImageDecodeParams::default()
        };
        let decoded = stream.decoded_image(&params).ok()?;
        let data = decoded.data.get(..want)?;
        // A Decode array of [1 0 1 0 1 0 1 0] reads every channel inverted.
        let inverted = dict
            .get(b"Decode")
            .map(|d| doc.resolve(d))
            .and_then(|d| d.as_array().map(|a| a.iter().filter_map(Object::as_f64).collect::<Vec<_>>()))
            .is_some_and(|d| d.len() == 8 && d.chunks(2).all(|p| p.first() == Some(&1.0) && p.get(1) == Some(&0.0)));
        let limit = self.th.max_tac;
        let (mut max, mut over) = (0u32, 0usize);
        for px in data.chunks_exact(4) {
            let sum: u32 = px.iter().map(|v| u32::from(if inverted { 255 - v } else { *v })).sum();
            max = max.max(sum);
            if f64::from(sum) * 100.0 / 255.0 > limit + 0.5 {
                over += 1;
            }
        }
        Some(ImageTac { max: f64::from(max) * 100.0 / 255.0, over: over as f64 / pixels as f64 })
    }

    fn set_colour(&mut self, st: &mut State, fill: bool, ops: &[Object], res: &Dict) {
        let target = if fill { &mut st.fill } else { &mut st.stroke };
        if target.kind == Kind::Pattern {
            // The last operand names a pattern: look at its colour.
            if let Some(name) = ops.last().and_then(Object::as_name) {
                let pat = res.get(b"Pattern").map(|p| self.doc.resolve(p)).and_then(|p| p.as_dict().and_then(|d| d.get(name).cloned()));
                if let Some(p) = pat {
                    let p = self.doc.resolve(&p);
                    if let Some(sh) = p.as_dict().and_then(|d| d.get(b"Shading").cloned()) {
                        self.shading(&sh, res);
                    }
                }
            }
            return;
        }
        target.values = ops.iter().filter_map(Object::as_f64).collect();
    }

    fn shading(&mut self, sh: &Object, res: &Dict) {
        let sh = self.doc.resolve(sh);
        if let Some(cs) = sh.as_dict().and_then(|d| d.get(b"ColorSpace").cloned()) {
            let (k, icc) = kind_of(self.doc, res, &cs, 0);
            let c = initial(k, icc);
            let c = Colour { values: vec![1.0; c.values.len()], nominal: true, ..c };
            self.use_colour(&c);
        }
    }

    fn run(&mut self, content: &[u8], res: &Dict, mut st: State, depth: usize, seen: &mut Vec<ObjRef>) {
        let parsed = parse(content);
        if let Some(p) = self.page_info() {
            p.unreadable += parsed.skipped;
        }
        let mut stack: Vec<State> = Vec::new();
        for op in &parsed.ops {
            self.ops += 1;
            if self.ops > MAX_OPS {
                self.report.truncated = true;
                return;
            }
            let o = &op.operands;
            match op.op.as_slice() {
                b"q" => stack.push(st.clone()),
                b"Q" => {
                    if let Some(s) = stack.pop() {
                        st = s;
                    }
                }
                b"cm" => {
                    if let Some(m) = Matrix::from_operands(o) {
                        st.ctm = m.then(&st.ctm);
                    }
                }
                b"w" => st.line_width = num(o.first()),
                b"gs" => {
                    if let Some(n) = o.first().and_then(Object::as_name) {
                        self.ext_gstate(res, n, &mut st);
                    }
                }
                b"g" => st.fill = Colour { kind: Kind::Gray, values: vec![num(o.first())], icc: false, nominal: false },
                b"G" => st.stroke = Colour { kind: Kind::Gray, values: vec![num(o.first())], icc: false, nominal: false },
                b"rg" => st.fill = Colour { kind: Kind::Rgb, values: o.iter().filter_map(Object::as_f64).collect(), icc: false, nominal: false },
                b"RG" => st.stroke = Colour { kind: Kind::Rgb, values: o.iter().filter_map(Object::as_f64).collect(), icc: false, nominal: false },
                b"k" => st.fill = Colour { kind: Kind::Cmyk, values: o.iter().filter_map(Object::as_f64).collect(), icc: false, nominal: false },
                b"K" => st.stroke = Colour { kind: Kind::Cmyk, values: o.iter().filter_map(Object::as_f64).collect(), icc: false, nominal: false },
                b"cs" | b"CS" => {
                    if let Some(n) = o.first() {
                        let (k, icc) = kind_of(self.doc, res, n, 0);
                        let c = initial(k, icc);
                        if op.op == b"cs" { st.fill = c } else { st.stroke = c }
                    }
                }
                b"sc" | b"scn" => self.set_colour(&mut st, true, o, res),
                b"SC" | b"SCN" => self.set_colour(&mut st, false, o, res),
                b"f" | b"F" | b"f*" => self.use_colour(&st.fill.clone()),
                b"S" | b"s" => self.stroke(&st),
                b"B" | b"B*" | b"b" | b"b*" => {
                    self.use_colour(&st.fill.clone());
                    self.stroke(&st);
                }
                b"sh" => {
                    if let Some(n) = o.first().and_then(Object::as_name) {
                        let sh = res.get(b"Shading").map(|s| self.doc.resolve(s)).and_then(|s| s.as_dict().and_then(|d| d.get(n).cloned()));
                        if let Some(sh) = sh {
                            self.shading(&sh, res);
                        }
                    }
                }
                b"Tf" => {
                    st.font_size = num(o.get(1));
                    st.font = o.first().and_then(Object::as_name).map(<[u8]>::to_vec);
                }
                b"Tm" => {
                    if let Some(m) = Matrix::from_operands(o) {
                        st.text_scale = scale_of(&m);
                    }
                }
                b"BT" => st.text_scale = 1.0,
                b"Tj" | b"TJ" | b"'" | b"\"" => {
                    if let Some(f) = st.font.clone() {
                        self.note_font(res, &f);
                    }
                    self.text(&st);
                }
                b"BI" => {
                    if let Some((dict, _)) = &op.inline {
                        self.image(dict, true, &st.ctm, &st.fill.clone(), res, (None, None));
                    }
                }
                b"Do" => {
                    if let Some(n) = o.first().and_then(Object::as_name) {
                        self.xobject(res, n, &st, depth, seen);
                    }
                }
                _ => {}
            }
        }
    }

    fn stroke(&mut self, st: &State) {
        self.use_colour(&st.stroke.clone());
        let width = st.line_width * scale_of(&st.ctm);
        if width < self.th.min_line_pt
            && let Some(p) = self.page_info()
        {
            p.hairlines += 1;
        }
    }

    fn text(&mut self, st: &State) {
        self.use_colour(&st.fill.clone());
        let size = st.font_size.abs() * st.text_scale * scale_of(&st.ctm);
        if st.fill.kind == Kind::Cmyk && size > 0.0 && size < self.th.small_text_pt {
            let v = |i: usize| st.fill.values.get(i).copied().unwrap_or(0.0);
            if v(3) > 0.0
                && v(0) + v(1) + v(2) > 0.0
                && let Some(p) = self.page_info()
            {
                p.rich_black_small_text += 1;
            }
        }
    }

    fn xobject(&mut self, res: &Dict, name: &[u8], st: &State, depth: usize, seen: &mut Vec<ObjRef>) {
        let Some(xo) = res.get(b"XObject").map(|x| self.doc.resolve(x)).and_then(|x| x.as_dict().and_then(|d| d.get(name).cloned())) else {
            self.missing_image(st, format!("XObject /{} tidak ada di resources", name_text(name)));
            return;
        };
        let target = xo.as_ref();
        let obj = self.doc.resolve(&xo);
        let Object::Stream(s) = &*obj else {
            self.missing_image(st, format!("XObject /{} rusak atau hilang", name_text(name)));
            return;
        };
        match s.dict.name(b"Subtype") {
            Some(b"Image") => {
                let missing = if s.dict.contains(b"F") {
                    Some("gambar eksternal (/F, tidak tertanam di file)".to_string())
                } else if s.dict.contains(b"OPI") {
                    Some("proxy OPI (gambar resolusi tinggi ada di luar file)".to_string())
                } else if s.raw.is_empty() {
                    Some("data gambar kosong".to_string())
                } else {
                    None
                };
                self.image(&s.dict, false, &st.ctm, &st.fill, res, (missing, target));
            }
            Some(b"Form") => {
                if depth >= MAX_DEPTH || target.is_some_and(|r| seen.contains(&r)) {
                    return;
                }
                if s.dict
                    .get(b"Group")
                    .map(|g| self.doc.resolve(g))
                    .and_then(|g| g.as_dict().and_then(|d| d.name(b"S").map(|n| n == b"Transparency")))
                    .unwrap_or(false)
                {
                    // A transparency group alone changes nothing on its own; it only matters with
                    // blending inside, which the inner content reports. Note it softly.
                }
                let Ok(data) = s.decoded_within(MAX_STREAM) else { return };
                let mut inner = st.clone();
                if let Some(m) = s.dict.get(b"Matrix").map(|m| self.doc.resolve(m)).and_then(|m| m.as_array().and_then(|a| Matrix::from_operands(a)))
                {
                    inner.ctm = m.then(&st.ctm);
                }
                let form_res = s.dict.get(b"Resources").map(|r| self.doc.resolve(r)).and_then(|r| r.as_dict().cloned());
                let form_res = form_res.unwrap_or_else(|| res.clone());
                if let Some(r) = target {
                    seen.push(r);
                }
                self.run(&data, &form_res, inner, depth + 1, seen);
                if target.is_some() {
                    seen.pop();
                }
            }
            _ => {}
        }
    }

    fn missing_image(&mut self, st: &State, why: String) {
        let [a, b, c, d, _, _] = st.ctm.0;
        let size_mm = [(a * a + b * b).sqrt() / MM, (c * c + d * d).sqrt() / MM];
        self.report.images.push(ImageUse {
            page: self.page,
            width_px: 0,
            height_px: 0,
            ppi: 0.0,
            size_mm,
            colour: String::new(),
            bits: 0,
            soft_mask: false,
            missing: Some(why),
            tac: None,
        });
    }
}

fn rect(doc: &Document, o: Option<&Object>) -> Option<[f64; 4]> {
    let o = doc.resolve(o?);
    let v: Vec<f64> = o.as_array()?.iter().filter_map(|x| doc.resolve(x).as_f64()).collect();
    let r = [*v.first()?, *v.get(1)?, *v.get(2)?, *v.get(3)?];
    r.iter().all(|x| x.is_finite()).then(|| [r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])])
}

fn page_content(doc: &Document, page: &Dict) -> Vec<u8> {
    let Some(c) = page.get(b"Contents") else { return Vec::new() };
    let c = doc.resolve(c);
    let parts: Vec<Object> = match &*c {
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

/// Read `doc` for everything a press-room preflight looks at. Never changes the document.
pub fn analyze(doc: &Document) -> Report {
    analyze_with(doc, &Thresholds::default())
}

/// [`analyze`] with the hairline and small-text limits of `th`.
pub fn analyze_with(doc: &Document, th: &Thresholds) -> Report {
    let mut w = Walker {
        doc,
        report: Report::default(),
        spots: BTreeMap::new(),
        fonts: BTreeMap::new(),
        page: 0,
        ops: 0,
        th: th.clone(),
        pixels: None,
        tacs: HashMap::new(),
    };
    let pages = pdfcraft_model::pages(doc);
    for (i, page) in pages.iter().enumerate() {
        let (dw, dh) = page.display_size(doc);
        let trim = rect(doc, page.dict.get(b"TrimBox"));
        let bleed = rect(doc, page.dict.get(b"BleedBox"));
        let crop = page.crop(doc);
        let turned = page.rotation(doc).rem_euclid(180) == 90;
        let trim_mm = trim.map(|t| {
            let (tw, thh) = ((t[2] - t[0]) / MM, (t[3] - t[1]) / MM);
            if turned { [thh, tw] } else { [tw, thh] }
        });
        // Bleed: from BleedBox if set, else the crop box is what will be printed around the trim.
        let bleed_mm = trim.map(|t| {
            let b = bleed.unwrap_or(crop);
            [(t[0] - b[0]) / MM, (t[1] - b[1]) / MM, (b[2] - t[2]) / MM, (b[3] - t[3]) / MM]
        });
        let annotations = page.dict.get(b"Annots").map(|a| doc.resolve(a)).and_then(|a| a.as_array().map(Vec::len)).unwrap_or(0);
        w.report.pages.push(PageInfo { number: i + 1, size_mm: [dw / MM, dh / MM], trim_mm, bleed_mm, annotations, ..PageInfo::default() });
        w.page = i + 1;
        let res = page.dict.get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default();
        if page
            .dict
            .get(b"Group")
            .map(|g| doc.resolve(g))
            .and_then(|g| g.as_dict().and_then(|d| d.name(b"S").map(|n| n == b"Transparency")))
            .unwrap_or(false)
        {
            // Page groups are written by most design apps whether or not anything is
            // transparent; blending inside is what the walker reports.
        }
        let content = page_content(doc, &page.dict);
        let st = State {
            ctm: Matrix::default(),
            fill: Colour::gray(),
            stroke: Colour::gray(),
            line_width: 1.0,
            font_size: 0.0,
            text_scale: 1.0,
            font: None,
        };
        let mut seen = Vec::new();
        w.run(&content, &res, st, 0, &mut seen);
        if w.report.truncated {
            break;
        }
    }
    w.report.fonts = w.fonts.into_values().collect();
    w.report.inks.spots = w.spots.into_iter().map(|(n, p)| Spot { name: name_text(&n), pages: p.into_iter().collect() }).collect();
    // Output intent and PDF/X.
    if let Some(root) = doc.trailer().get(b"Root").map(|r| doc.resolve(r)) {
        let intent = root
            .as_dict()
            .and_then(|r| r.get(b"OutputIntents").cloned())
            .map(|o| doc.resolve(&o))
            .and_then(|o| o.as_array().and_then(|a| a.first().cloned()));
        if let Some(i) = intent.map(|i| doc.resolve(&i)) {
            w.report.output_intent = i
                .as_dict()
                .and_then(|d| d.get(b"OutputConditionIdentifier").or_else(|| d.get(b"OutputCondition")).map(|s| doc.resolve(s)))
                .and_then(|s| s.as_string().map(|s| String::from_utf8_lossy(&s.bytes).into_owned()));
        }
    }
    if let Some(info) = doc.trailer().get(b"Info").map(|i| doc.resolve(i)) {
        w.report.pdfx = info
            .as_dict()
            .and_then(|d| d.get(b"GTS_PDFXVersion").map(|v| doc.resolve(v)))
            .and_then(|v| v.as_string().map(|s| String::from_utf8_lossy(&s.bytes).into_owned()));
    }
    w.report
}

fn list<T: AsRef<str>>(items: &[T], max: usize) -> String {
    let mut s: Vec<&str> = items.iter().take(max).map(AsRef::as_ref).collect();
    let more = items.len().saturating_sub(max);
    let tail = if more > 0 { format!(" (+{more} lagi)") } else { String::new() };
    s.dedup();
    format!("{}{tail}", s.join(", "))
}

fn pages_text(p: &[usize]) -> String {
    let v: Vec<String> = p.iter().map(|n| n.to_string()).collect();
    format!("hal. {}", list(&v, 8))
}

/// Production-room spot names that are not an ink to print: cutting and finishing layers.
pub fn is_finishing_layer(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["cut", "dieline", "die line", "kisscut", "kiss cut", "foil", "varnish", "spot uv", "white", "emboss", "crease", "perfor"]
        .iter()
        .any(|k| n.contains(k))
}

/// Ink total: vector colours, and the pixels of 8-bit CMYK images.
fn tac_check(r: &Report, th: &Thresholds) -> (Status, String) {
    let max_tac = r.pages.iter().map(|p| p.max_tac).fold(0.0_f64, f64::max);
    let vec_pages: Vec<usize> = r.pages.iter().filter(|p| p.max_tac > th.max_tac + 0.5).map(|p| p.number).collect();
    // An image counts when more than 0.1 % of its pixels go over: a few dark JPEG pixels do not.
    let measured: Vec<&ImageUse> = r.images.iter().filter(|i| i.tac.is_some()).collect();
    let heavy: Vec<&ImageUse> = measured.iter().copied().filter(|i| i.tac.is_some_and(|t| t.over > 0.001)).collect();
    let img_max = measured.iter().filter_map(|i| i.tac.map(|t| t.max)).fold(0.0_f64, f64::max);
    let mut img_pages: Vec<usize> = heavy.iter().map(|i| i.page).collect();
    img_pages.sort_unstable();
    img_pages.dedup();
    let unmeasured = r.images.iter().filter(|i| i.tac.is_none() && i.missing.is_none() && i.colour.starts_with("CMYK")).count();
    let tail = if unmeasured > 0 { format!(" {unmeasured} gambar CMYK tidak terukur (bukan 8-bit atau terlalu besar).") } else { String::new() };
    if max_tac <= 0.0 && measured.is_empty() {
        (Status::Na, format!("Tidak ada warna CMYK untuk dihitung.{tail}"))
    } else if !vec_pages.is_empty() || !heavy.is_empty() {
        let mut parts = Vec::new();
        if !vec_pages.is_empty() {
            parts.push(format!("warna vektor {:.0}% di {}", max_tac, pages_text(&vec_pages)));
        }
        if !heavy.is_empty() {
            let share = heavy.iter().filter_map(|i| i.tac.map(|t| t.over)).fold(0.0_f64, f64::max);
            parts.push(format!(
                "{} gambar sampai {:.0}% ({:.1}% piksel di atas batas) di {}",
                heavy.len(),
                img_max,
                share * 100.0,
                pages_text(&img_pages)
            ));
        }
        (Status::Warn, format!("Total tinta di atas {:.0}%: {}.{tail}", th.max_tac, parts.join("; ")))
    } else {
        let mut parts = Vec::new();
        if max_tac > 0.0 {
            parts.push(format!("vektor {:.0}%", max_tac));
        }
        if !measured.is_empty() {
            parts.push(format!("{} gambar CMYK, tertinggi {:.0}%", measured.len(), img_max));
        }
        (Status::Pass, format!("Total tinta tertinggi: {}.{tail}", parts.join(", ")))
    }
}

/// Pass / warning / fail lines for `r`, in Indonesian, judged with `th`.
pub fn checks(r: &Report, th: &Thresholds) -> Vec<Check> {
    let mut out = Vec::new();
    let mut push = |id, title, status, message: String| out.push(Check { id, title, status, message });

    // Pages.
    if r.pages.is_empty() {
        push("file_ok", "File bisa dibuka", Status::Fail, "Tidak ada halaman.".into());
        return out;
    }
    let first = r.pages.first().map(|p| p.size_mm).unwrap_or_default();
    let mixed: Vec<usize> =
        r.pages.iter().filter(|p| (p.size_mm[0] - first[0]).abs() > 0.5 || (p.size_mm[1] - first[1]).abs() > 0.5).map(|p| p.number).collect();
    push(
        "page_size",
        "Ukuran halaman",
        if mixed.is_empty() { Status::Pass } else { Status::Warn },
        if mixed.is_empty() {
            format!("{} halaman, semua {:.1} × {:.1} mm.", r.pages.len(), first[0], first[1])
        } else {
            format!("Ukuran halaman campur: hal. 1 {:.1} × {:.1} mm, beda di {}.", first[0], first[1], pages_text(&mixed))
        },
    );

    // Bleed.
    let no_trim: Vec<usize> = r.pages.iter().filter(|p| p.trim_mm.is_none()).map(|p| p.number).collect();
    let short: Vec<usize> =
        r.pages.iter().filter(|p| p.bleed_mm.is_some_and(|b| b.iter().any(|v| *v < th.bleed_mm - 0.05))).map(|p| p.number).collect();
    let (status, msg) = if no_trim.len() == r.pages.len() {
        (
            Status::Warn,
            format!(
                "Tidak ada TrimBox di file: bleed tidak bisa dipastikan. Kalau desain sampai tepi, isi \"Bleed di file\" saat impose atau tambahkan bleed {:.0} mm.",
                th.bleed_mm
            ),
        )
    } else if !short.is_empty() {
        let min = r.pages.iter().filter_map(|p| p.bleed_mm).flat_map(|b| b.into_iter()).fold(f64::MAX, f64::min);
        (Status::Warn, format!("Bleed kurang dari {:.0} mm (terkecil {:.1} mm) di {}.", th.bleed_mm, min.max(0.0), pages_text(&short)))
    } else if !no_trim.is_empty() {
        (Status::Warn, format!("TrimBox tidak ada di {}.", pages_text(&no_trim)))
    } else {
        let min = r.pages.iter().filter_map(|p| p.bleed_mm).flat_map(|b| b.into_iter()).fold(f64::MAX, f64::min);
        (Status::Pass, format!("Bleed {:.1} mm atau lebih di semua sisi.", min))
    };
    push("bleed", "Bleed", status, msg);

    // Fonts.
    let missing: Vec<&str> = r.fonts.iter().filter(|f| !f.embedded).map(|f| f.name.as_str()).collect();
    let type3: Vec<&str> = r.fonts.iter().filter(|f| f.subtype == "Type3").map(|f| f.name.as_str()).collect();
    if r.fonts.is_empty() {
        push("fonts", "Font", Status::Pass, "Tidak ada font: teks sudah outline (atau tidak ada teks).".into());
    } else if !missing.is_empty() {
        push(
            "fonts",
            "Font",
            Status::Fail,
            format!(
                "{} font TIDAK tertanam: {}. Akan diganti font lain oleh mesin. Export ulang dengan font embedded atau outline.",
                missing.len(),
                list(&missing, 5)
            ),
        );
    } else {
        let t3 = if type3.is_empty() { String::new() } else { format!(" Type 3: {}.", list(&type3, 3)) };
        push("fonts", "Font", Status::Pass, format!("Semua {} font tertanam.{t3}", r.fonts.len()));
    }

    // Images that are not in the file.
    let gone: Vec<String> = r.images.iter().filter_map(|i| i.missing.as_ref().map(|m| format!("hal. {}: {m}", i.page))).collect();
    if gone.is_empty() {
        push(
            "images_missing",
            "Gambar tertanam",
            if r.images.is_empty() { Status::Na } else { Status::Pass },
            if r.images.is_empty() { "Tidak ada gambar.".into() } else { format!("Semua {} gambar tertanam di file.", r.images.len()) },
        );
    } else {
        push("images_missing", "Gambar tertanam", Status::Fail, format!("{} gambar tidak tertanam / hilang: {}.", gone.len(), list(&gone, 4)));
    }

    // Resolution.
    let judged: Vec<&ImageUse> = r
        .images
        .iter()
        .filter(|i| i.missing.is_none() && i.ppi > 0.0 && !(i.size_mm[0] < th.ignore_image_mm && i.size_mm[1] < th.ignore_image_mm))
        .collect();
    let low: Vec<&&ImageUse> = judged.iter().filter(|i| i.ppi < th.warn_ppi - 0.5).collect();
    if judged.is_empty() {
        push("images_dpi", "Resolusi gambar", Status::Na, "Tidak ada gambar raster yang perlu dinilai.".into());
    } else if low.is_empty() {
        push("images_dpi", "Resolusi gambar", Status::Pass, format!("Semua {} gambar ≥{:.0} DPI efektif.", judged.len(), th.warn_ppi));
    } else {
        let worst = low.iter().any(|i| i.ppi < th.fail_ppi);
        let items: Vec<String> =
            low.iter().map(|i| format!("hal. {} {:.0} DPI ({:.0}×{:.0} mm)", i.page, i.ppi, i.size_mm[0], i.size_mm[1])).collect();
        push(
            "images_dpi",
            "Resolusi gambar",
            if worst { Status::Fail } else { Status::Warn },
            format!("{} gambar di bawah {:.0} DPI: {}.", low.len(), th.warn_ppi, list(&items, 5)),
        );
    }

    let c = colour_check(r);
    push(c.id, c.title, c.status, c.message);

    let (status, message) = tac_check(r, th);
    push("ink_coverage", "Total tinta (TAC)", status, message);

    // Small rich black text.
    let rich: Vec<usize> = r.pages.iter().filter(|p| p.rich_black_small_text > 0).map(|p| p.number).collect();
    push(
        "rich_black_text",
        "Teks kecil rich black",
        if rich.is_empty() { Status::Pass } else { Status::Warn },
        if rich.is_empty() {
            format!("Tidak ada teks <{:.0} pt dengan campuran CMYK.", th.small_text_pt)
        } else {
            format!(
                "Teks <{:.0} pt pakai campuran CMYK (bukan 100% K) di {}: rawan blur kalau register meleset.",
                th.small_text_pt,
                pages_text(&rich)
            )
        },
    );

    // Hairlines.
    let thin: Vec<usize> = r.pages.iter().filter(|p| p.hairlines > 0).map(|p| p.number).collect();
    push(
        "thin_lines",
        "Garis tipis",
        if thin.is_empty() { Status::Pass } else { Status::Warn },
        if thin.is_empty() {
            format!("Tidak ada garis < {} pt.", th.min_line_pt)
        } else {
            format!("Garis < {} pt di {}: bisa putus atau hilang saat cetak.", th.min_line_pt, pages_text(&thin))
        },
    );

    // Transparency and overprint.
    let tr: Vec<usize> = r.pages.iter().filter(|p| p.transparency).map(|p| p.number).collect();
    let op: Vec<usize> = r.pages.iter().filter(|p| p.overprint).map(|p| p.number).collect();
    let mut m = Vec::new();
    if !tr.is_empty() {
        m.push(format!("Transparansi di {} (aman di RIP modern/APPE; cek preview).", pages_text(&tr)));
    }
    if !op.is_empty() {
        m.push(format!("Overprint di {}: pastikan disengaja (mis. teks hitam).", pages_text(&op)));
    }
    push(
        "transparency_overprint",
        "Transparansi & overprint",
        if m.is_empty() { Status::Pass } else { Status::Info },
        if m.is_empty() { "Tidak ada transparansi atau overprint.".into() } else { m.join(" ") },
    );

    // Annotations.
    let ann: usize = r.pages.iter().map(|p| p.annotations).sum();
    if ann > 0 {
        push(
            "annotations",
            "Komentar & form",
            Status::Info,
            format!("{ann} anotasi/form field. Impose memakai isi halaman + form field; komentar tidak ikut kecuali dipilih."),
        );
    }

    // Output intent.
    match (&r.pdfx, &r.output_intent) {
        (Some(x), Some(o)) => push("pdf_x", "Standar PDF/X", Status::Pass, format!("{x}, kondisi cetak {o}.")),
        (None, Some(o)) => push("pdf_x", "Standar PDF/X", Status::Info, format!("Output intent {o} (bukan PDF/X penuh).")),
        _ => push("pdf_x", "Standar PDF/X", Status::Info, "Bukan PDF/X, tanpa output intent. Tidak wajib.".into()),
    }
    if r.truncated {
        push("partial", "Pemeriksaan", Status::Warn, "File sangat kompleks: pemeriksaan berhenti sebelum semua halaman selesai.".into());
    }
    out
}

/// Inks and colour spaces in one line.
fn colour_check(r: &Report) -> Check {
    let i = &r.inks;
    let process: Vec<&str> =
        [(i.cyan, "Cyan"), (i.magenta, "Magenta"), (i.yellow, "Yellow"), (i.black, "Black")].iter().filter(|(on, _)| *on).map(|(_, n)| *n).collect();
    let spots: Vec<&str> = i.spots.iter().map(|s| s.name.as_str()).filter(|n| !is_finishing_layer(n)).collect();
    let layers: Vec<&str> = i.spots.iter().map(|s| s.name.as_str()).filter(|n| is_finishing_layer(n)).collect();
    let mut msgs = Vec::new();
    let mut status = Status::Pass;
    if i.rgb {
        msgs.push("Ada warna RGB: mesin/RIP yang mengonversi ke CMYK, warna terang bisa jadi kusam.".to_string());
        status = Status::Warn;
    }
    if i.lab {
        msgs.push("Ada warna Lab.".to_string());
        status = Status::Warn;
    }
    if !spots.is_empty() {
        msgs.push(format!("Spot color selain CMYK: {}.", list(&spots, 6)));
        status = Status::Warn;
    }
    if !layers.is_empty() {
        msgs.push(format!("Layer produksi (info): {}.", list(&layers, 6)));
    }
    if i.registration {
        msgs.push("Ada objek warna Registration: tercetak di SEMUA plat (wajar untuk crop mark, tidak wajar untuk desain).".to_string());
        status = Status::Warn;
    }
    if msgs.is_empty() || status == Status::Pass {
        msgs.insert(0, if process.is_empty() { "Tidak ada tinta terdeteksi.".to_string() } else { format!("Tinta proses: {}.", process.join(", ")) });
    }
    Check { id: "color_mode", title: "Mode warna & tinta", status, message: msgs.join(" ") }
}

/// The worst status among `checks`.
pub fn overall(checks: &[Check]) -> Status {
    checks.iter().map(|c| c.status).filter(|s| matches!(s, Status::Warn | Status::Fail)).max().unwrap_or(Status::Pass)
}

/// Count of each status, for a summary line.
pub fn tally(checks: &[Check]) -> HashMap<Status, usize> {
    let mut m = HashMap::new();
    for c in checks {
        *m.entry(c.status).or_insert(0) += 1;
    }
    m
}

#[cfg(test)]
mod tests;
