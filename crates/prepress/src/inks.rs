//! Ink manager: merge one spot colour into another (alias) or print it in process colours.
//!
//! - **Alias** renames the colorant in every `Separation` and `DeviceN` colour space, so two
//!   names for one ink (`PANTONE 185 C`, `PANTONE 185 CV`) come out on one plate. Colour values
//!   are not touched.
//! - **To process** replaces each use of the spot in content streams with its own alternate
//!   colour, computed from the colour space's tint transform, so it prints exactly as the file
//!   already shows it on screen and as a RIP without that ink would print it. Only colour
//!   operators change; the rest of each stream is copied byte for byte and the file is saved
//!   incrementally. Images, shadings, patterns and `DeviceN` mixes that use the spot, and spots
//!   whose alternate is not DeviceCMYK, DeviceRGB or DeviceGray, are left as they are and
//!   reported.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use hayro::hayro_interpret::Function;
use hayro::hayro_interpret::hayro_syntax::Pdf;
use hayro::hayro_interpret::hayro_syntax::object::{Object as HObject, ObjectIdentifier};
use pdfcraft_content::parse;
use pdfcraft_cos::{Dict, Document, ObjRef, Object, SaveOptions, Stream, write_incremental};
use serde::{Deserialize, Serialize};

const MAX_STREAM: usize = 256 << 20;
const MAX_DEPTH: usize = 16;

/// What to do with one spot colour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "action", content = "into")]
pub enum InkAction {
    /// Print it in its alternate (process) colour.
    ToProcess,
    /// Print it on the plate of this other spot.
    Alias(String),
}

/// What [`manage_inks`] did.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct InkReport {
    /// Colour operators replaced by their process equivalent, per spot.
    pub converted: BTreeMap<String, usize>,
    /// Colour spaces renamed, per spot.
    pub aliased: BTreeMap<String, usize>,
    /// Uses left as they were, and why.
    pub kept: Vec<String>,
}

/// The alternate family of a spot and how to compute it.
#[derive(Clone)]
enum Tint {
    /// `C0 + t^N (C1 - C0)`.
    Exponential { c0: Vec<f64>, c1: Vec<f64>, n: f64 },
    /// Any other function, through the renderer's evaluator.
    Function(Function),
}

#[derive(Clone)]
struct SpotSpace {
    name: String,
    /// `k`, `rg` or `g`: the operator its alternate is painted with (lower case for fill).
    family: &'static str,
    tint: Tint,
}

impl SpotSpace {
    fn eval(&self, t: f64) -> Option<Vec<f64>> {
        let t = t.clamp(0.0, 1.0);
        let v: Vec<f64> = match &self.tint {
            Tint::Exponential { c0, c1, n } => c0.iter().zip(c1).map(|(a, b)| a + t.powf(*n) * (b - a)).collect(),
            Tint::Function(f) => {
                let mut input = smallvec::SmallVec::<[f32; 4]>::new();
                input.push(t as f32);
                f.eval(input)?.iter().map(|v| f64::from(*v)).collect()
            }
        };
        let want = match self.family {
            "k" => 4,
            "rg" => 3,
            _ => 1,
        };
        (v.len() == want).then(|| v.into_iter().map(|x| x.clamp(0.0, 1.0)).collect())
    }

    /// The operator that paints `t` of this spot in its alternate colour.
    fn operator(&self, t: f64, stroke: bool) -> Option<String> {
        let v = self.eval(t)?;
        let mut s: Vec<String> = v.iter().map(|x| fmt(*x)).collect();
        let op = if stroke { self.family.to_uppercase() } else { self.family.to_string() };
        s.push(op);
        Some(s.join(" "))
    }
}

fn fmt(v: f64) -> String {
    let s = format!("{:.5}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".into() } else { s.into() }
}

struct Ctx<'a> {
    doc: &'a Document,
    pdf: Option<&'a Pdf>,
    targets: &'a HashSet<String>,
    report: InkReport,
    page: usize,
}

impl Ctx<'_> {
    /// The spot this colour space is, when it is one of the targets.
    fn spot(&mut self, res: &Dict, cs: &Object) -> Option<SpotSpace> {
        let arr_obj = match self.doc.resolve(cs).as_ref() {
            Object::Name(n) => res.get(b"ColorSpace").map(|c| self.doc.resolve(c)).and_then(|c| c.as_dict().and_then(|d| d.get(n).cloned()))?,
            _ => cs.clone(),
        };
        let arr = self.doc.resolve(&arr_obj);
        let a = arr.as_array()?;
        match a.first().and_then(Object::as_name) {
            Some(b"Separation") => {}
            Some(b"DeviceN") => {
                let names: Vec<String> = a
                    .get(1)
                    .map(|n| self.doc.resolve(n))
                    .and_then(|n| {
                        n.as_array().map(|v| v.iter().filter_map(|x| x.as_name().map(|b| String::from_utf8_lossy(b).into_owned())).collect())
                    })
                    .unwrap_or_default();
                if names.iter().any(|n| self.targets.contains(n)) {
                    self.report.kept.push(format!("hal. {}: campuran DeviceN berisi spot, dibiarkan", self.page));
                }
                return None;
            }
            _ => return None,
        }
        let name = a.get(1).map(|n| self.doc.resolve(n)).and_then(|n| n.as_name().map(|b| String::from_utf8_lossy(b).into_owned()))?;
        if !self.targets.contains(&name) {
            return None;
        }
        let alt = a.get(2).map(|x| self.doc.resolve(x))?;
        let family = match alt.as_name() {
            Some(b"DeviceCMYK") => "k",
            Some(b"DeviceRGB") => "rg",
            Some(b"DeviceGray") => "g",
            _ => {
                self.report.kept.push(format!("hal. {}: {name} punya warna alternatif bukan CMYK/RGB/Gray, dibiarkan", self.page));
                return None;
            }
        };
        let f = a.get(3)?;
        let tint = match f {
            Object::Ref(r) => {
                let pdf = self.pdf?;
                let id = ObjectIdentifier::new(i32::try_from(r.num).ok()?, i32::from(r.generation));
                let obj: HObject<'_> = pdf.xref().get(id)?;
                Tint::Function(Function::new(&obj)?)
            }
            Object::Dict(d) if d.int(b"FunctionType") == Some(2) => {
                let nums = |k: &[u8], dflt: Vec<f64>| {
                    d.get(k).and_then(|v| v.as_array().map(|a| a.iter().filter_map(Object::as_f64).collect())).unwrap_or(dflt)
                };
                Tint::Exponential { c0: nums(b"C0", vec![0.0]), c1: nums(b"C1", vec![1.0]), n: d.get(b"N").and_then(Object::as_f64).unwrap_or(1.0) }
            }
            _ => {
                self.report.kept.push(format!("hal. {}: fungsi warna {name} tidak bisa dibaca, dibiarkan", self.page));
                return None;
            }
        };
        Some(SpotSpace { name, family, tint })
    }

    /// Rewrite one content stream; returns the new bytes when anything changed.
    fn stream(&mut self, data: &[u8], res: &Dict, depth: usize, forms: &mut Vec<(ObjRef, Vec<u8>)>, done: &mut HashSet<ObjRef>) -> Option<Vec<u8>> {
        let parsed = parse(data);
        let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
        let (mut fill, mut stroke): (Option<SpotSpace>, Option<SpotSpace>) = (None, None);
        let mut stack = Vec::new();
        for op in &parsed.ops {
            let o = &op.operands;
            match op.op.as_slice() {
                b"q" => stack.push((fill.clone(), stroke.clone())),
                b"Q" => {
                    if let Some((f, s)) = stack.pop() {
                        fill = f;
                        stroke = s;
                    }
                }
                b"cs" | b"CS" => {
                    let is_stroke = op.op == b"CS";
                    let sp = o.first().and_then(|cs| self.spot(res, cs));
                    if let Some(sp) = &sp
                        && let Some(text) = sp.operator(1.0, is_stroke)
                    {
                        edits.push((op.span.clone(), text));
                        *self.report.converted.entry(sp.name.clone()).or_insert(0) += 1;
                    }
                    if is_stroke { stroke = sp } else { fill = sp }
                }
                b"sc" | b"scn" | b"SC" | b"SCN" => {
                    let is_stroke = op.op.first() == Some(&b'S');
                    let current = if is_stroke { &stroke } else { &fill };
                    if let Some(sp) = current {
                        let t = o.first().and_then(Object::as_f64).unwrap_or(1.0);
                        if let Some(text) = sp.operator(t, is_stroke) {
                            edits.push((op.span.clone(), text));
                            *self.report.converted.entry(sp.name.clone()).or_insert(0) += 1;
                        }
                    }
                }
                // Device colour operators leave the spot.
                b"g" | b"rg" | b"k" => fill = None,
                b"G" | b"RG" | b"K" => stroke = None,
                b"sh" => {
                    let sh = o
                        .first()
                        .and_then(|n| n.as_name())
                        .and_then(|n| res.get(b"Shading").map(|s| self.doc.resolve(s)).and_then(|s| s.as_dict().and_then(|d| d.get(n).cloned())));
                    if let Some(cs) = sh.map(|s| self.doc.resolve(&s)).and_then(|s| s.as_dict().and_then(|d| d.get(b"ColorSpace").cloned()))
                        && let Some(sp) = self.spot(res, &cs)
                    {
                        self.report.kept.push(format!("hal. {}: gradasi {} dibiarkan", self.page, sp.name));
                    }
                }
                b"Do" => {
                    let Some(n) = o.first().and_then(|n| n.as_name()) else { continue };
                    let Some(xo) = res.get(b"XObject").map(|x| self.doc.resolve(x)).and_then(|x| x.as_dict().and_then(|d| d.get(n).cloned())) else {
                        continue;
                    };
                    let r = xo.as_ref();
                    let obj = self.doc.resolve(&xo);
                    let Object::Stream(s) = &*obj else { continue };
                    match s.dict.name(b"Subtype") {
                        Some(b"Image") => {
                            if let Some(cs) = s.dict.get(b"ColorSpace")
                                && let Some(sp) = self.spot(res, cs)
                            {
                                self.report.kept.push(format!("hal. {}: gambar dalam {} dibiarkan", self.page, sp.name));
                            }
                        }
                        Some(b"Form") if depth < MAX_DEPTH => {
                            let Some(r) = r else { continue };
                            if !done.insert(r) {
                                continue;
                            }
                            let Ok(d) = s.decoded_within(MAX_STREAM) else { continue };
                            let fres = s
                                .dict
                                .get(b"Resources")
                                .map(|x| self.doc.resolve(x))
                                .and_then(|x| x.as_dict().cloned())
                                .unwrap_or_else(|| res.clone());
                            if let Some(new) = self.stream(&d, &fres, depth + 1, forms, done) {
                                forms.push((r, new));
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        if edits.is_empty() {
            return None;
        }
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
        Some(out)
    }
}

/// Rename colorant `from` to `to` in a colour space array; `None` when it isn't there or the
/// rename would list a colorant twice.
fn renamed(a: &[Object], from: &[u8], to: &[u8], doc: &Document) -> Option<Vec<Object>> {
    match a.first().and_then(Object::as_name) {
        Some(b"Separation") if a.get(1).and_then(Object::as_name) == Some(from) => {
            let mut v = a.to_vec();
            if let Some(slot) = v.get_mut(1) {
                *slot = Object::Name(to.to_vec());
            }
            Some(v)
        }
        Some(b"DeviceN") => {
            let names = doc.resolve(a.get(1)?);
            let names = names.as_array()?;
            if !names.iter().any(|n| n.as_name() == Some(from)) || names.iter().any(|n| n.as_name() == Some(to)) {
                return None;
            }
            let mut v = a.to_vec();
            if let Some(slot) = v.get_mut(1) {
                *slot = Object::Array(names.iter().map(|n| if n.as_name() == Some(from) { Object::Name(to.to_vec()) } else { n.clone() }).collect());
            }
            Some(v)
        }
        _ => None,
    }
}

/// Apply every alias in `map` inside `o`; counts renames.
fn alias_in(o: &Object, map: &BTreeMap<Vec<u8>, Vec<u8>>, doc: &Document, count: &mut BTreeMap<String, usize>, depth: usize) -> Option<Object> {
    if depth > 32 {
        return None;
    }
    match o {
        Object::Array(a) => {
            let mut changed = None;
            for (from, to) in map {
                let cur = changed.as_deref().unwrap_or(a.as_slice());
                if let Some(v) = renamed(cur, from, to, doc) {
                    *count.entry(String::from_utf8_lossy(from).into_owned()).or_insert(0) += 1;
                    changed = Some(v);
                }
            }
            let base = changed.clone().unwrap_or_else(|| a.clone());
            let mut inner_changed = false;
            let items: Vec<Object> = base
                .iter()
                .map(|x| match alias_in(x, map, doc, count, depth + 1) {
                    Some(n) => {
                        inner_changed = true;
                        n
                    }
                    None => x.clone(),
                })
                .collect();
            (changed.is_some() || inner_changed).then_some(Object::Array(items))
        }
        Object::Dict(d) => {
            let mut nd = d.clone();
            let mut any = false;
            for (k, v) in d.iter() {
                if let Some(n) = alias_in(v, map, doc, count, depth + 1) {
                    nd.set(k.clone(), n);
                    any = true;
                }
            }
            any.then_some(Object::Dict(nd))
        }
        _ => None,
    }
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

/// Apply `actions` (spot name → what to do) to `pdf`.
pub fn manage_inks(pdf: Vec<u8>, actions: &BTreeMap<String, InkAction>) -> Result<(Vec<u8>, InkReport), String> {
    let bytes = Arc::new(pdf);
    let mut doc = Document::open(bytes.clone()).map_err(|e| format!("cannot open the PDF: {e}"))?;
    let hpdf = Pdf::new_with_password(bytes.clone(), "").ok();
    let targets: HashSet<String> = actions.iter().filter(|(_, a)| **a == InkAction::ToProcess).map(|(n, _)| n.clone()).collect();
    let aliases: BTreeMap<Vec<u8>, Vec<u8>> = actions
        .iter()
        .filter_map(|(n, a)| match a {
            InkAction::Alias(to) if to != n => Some((n.as_bytes().to_vec(), to.as_bytes().to_vec())),
            _ => None,
        })
        .collect();
    let mut report = InkReport::default();
    let mut changed = false;

    // Process conversions first, while every spot still has its own name.
    if !targets.is_empty() {
        let pages = pdfcraft_model::pages(&doc);
        let mut page_writes = Vec::new();
        let mut form_writes = Vec::new();
        let mut done = HashSet::new();
        let mut ctx = Ctx { doc: &doc, pdf: hpdf.as_ref(), targets: &targets, report: InkReport::default(), page: 0 };
        for (i, page) in pages.iter().enumerate() {
            ctx.page = i + 1;
            let data = page_content(&doc, &page.dict);
            let res = page.dict.get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default();
            if let Some(new) = ctx.stream(&data, &res, 0, &mut form_writes, &mut done) {
                page_writes.push((page.obj, new));
            }
        }
        report = ctx.report;
        changed |= !page_writes.is_empty() || !form_writes.is_empty();
        for (r, data) in form_writes {
            let obj = doc.resolve(&Object::Ref(r));
            if let Object::Stream(s) = &*obj {
                let mut dict = s.dict.clone();
                for k in [b"Filter".as_slice(), b"DecodeParms", b"Length", b"DL"] {
                    dict.remove(k);
                }
                doc.set(r, Object::Stream(Stream::flate(dict, &data)));
            }
        }
        for (r, data) in page_writes {
            let contents = doc.add(Object::Stream(Stream::flate(Dict::new(), &data)));
            doc.update_dict(r, |d| d.set(b"Contents".to_vec(), Object::Ref(contents))).map_err(|e| e.to_string())?;
        }
    }

    // Aliases: rename the colorant wherever a colour space names it.
    if !aliases.is_empty() {
        let mut count = BTreeMap::new();
        for num in doc.object_numbers() {
            let r = ObjRef { num, generation: 0 };
            let obj = doc.resolve(&Object::Ref(r));
            let new = match &*obj {
                Object::Stream(s) => alias_in(&Object::Dict(s.dict.clone()), &aliases, &doc, &mut count, 0).and_then(|d| match d {
                    Object::Dict(nd) => Some(Object::Stream(Stream { dict: nd, raw: s.raw.clone() })),
                    _ => None,
                }),
                other => alias_in(other, &aliases, &doc, &mut count, 0),
            };
            if let Some(n) = new {
                doc.set(r, n);
                changed = true;
            }
        }
        report.aliased = count;
    }
    if !changed {
        return Ok((bytes.to_vec(), report));
    }
    let out = write_incremental(&doc, &SaveOptions::default()).map_err(|e| e.to_string())?;
    Ok((out, report))
}

#[cfg(test)]
#[path = "inks_tests.rs"]
mod tests;
