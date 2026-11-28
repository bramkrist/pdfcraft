//! Named manual layouts: kept in the settings, and shared as `*.impose.json`, the file the light
//! app (`apps/impose-web`) writes and reads.
//!
//! The file and the settings are untrusted input: every number is checked, a cell that makes no
//! sense is dropped and counted, and the size of what is taken is bounded.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{Cell, MAX_ON_SIDE, MAX_SIDES, ManualDraft, PRESETS, round2, tame};

/// Most templates kept.
pub const MAX_TEMPLATES: usize = 100;
/// Longest template name, in characters.
pub const MAX_NAME_CHARS: usize = 60;
/// Most placed pages in one template, over all its sheet sides.
pub const MAX_CELLS: usize = 2000;
/// Largest file taken as a template.
#[cfg(not(target_arch = "wasm32"))]
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// The sheet sizes the editor's fields take, millimetres.
const SHEET_RANGE: (f64, f64) = (50.0, 2000.0);
const BLEED_MAX: f64 = 50.0;

/// A manual layout without the document it was made for: the sheet, the bleed in the file, the
/// marks and the pages placed on each sheet side (0-based page numbers).
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    /// The sheet as it lies, millimetres.
    pub sheet: (f64, f64),
    pub bleed: f64,
    pub marks: bool,
    pub sheets: Vec<Vec<Cell>>,
}

impl Template {
    /// The layout in the editor, as it is now.
    pub fn from_draft(d: &ManualDraft) -> Result<Self, String> {
        let total: usize = d.sheets.iter().map(Vec::len).sum();
        if total > MAX_CELLS {
            return Err(crate::i18n::fmt(tl!("A template holds at most {n} pages."), &[("n", &MAX_CELLS.to_string())]));
        }
        Ok(Self { sheet: d.sheet(), bleed: d.bleed, marks: d.marks, sheets: d.sheets.clone() })
    }

    /// Put the layout in the editor. A page the document does not have is left out; returns how
    /// many were. The view goes back to the first sheet side with nothing selected.
    pub fn apply(&self, d: &mut ManualDraft, pages: usize) -> usize {
        let (w, h) = self.sheet;
        let (short, long) = (w.min(h), w.max(h));
        d.landscape = w >= h;
        d.preset =
            PRESETS.iter().position(|&(_, pw, ph)| (pw.min(ph) - short).abs() < 0.01 && (pw.max(ph) - long).abs() < 0.01).unwrap_or(PRESETS.len());
        if d.preset >= PRESETS.len() {
            d.custom = (short, long);
        }
        d.bleed = self.bleed;
        d.marks = self.marks;
        let mut skipped = 0;
        d.sheets = self
            .sheets
            .iter()
            .map(|side| {
                side.iter()
                    .filter(|c| {
                        let there = c.page < pages;
                        skipped += usize::from(!there);
                        there
                    })
                    .copied()
                    .collect()
            })
            .collect();
        d.current = 0;
        d.sel = None;
        d.drag = None;
        d.guides.clear();
        d.fix();
        skipped
    }

    /// The file's form, which is the light app's: `{ v, name?, sheet_mm, bleed_mm, marks, sheets }`
    /// with a quarter turn written as `turn` in degrees.
    pub fn to_json(&self, name: Option<&str>) -> Value {
        let sheets: Vec<Value> = self
            .sheets
            .iter()
            .map(|side| side.iter().map(|c| json!({ "page": c.page, "x": c.x, "y": c.y, "turn": u32::from(c.turns % 4) * 90 })).collect())
            .collect();
        // The Pro editor draws its marks with the app's usual values: 3 mm lines from 3 mm out.
        let marks = self.marks.then(|| json!({ "length_mm": 3, "offset_mm": 3, "style": "corner", "width_pt": 0.5, "registration": false }));
        let mut v = json!({ "v": 1, "sheet_mm": [self.sheet.0, self.sheet.1], "bleed_mm": self.bleed, "marks": marks, "sheets": sheets });
        if let (Some(name), Some(o)) = (name, v.as_object_mut()) {
            o.insert("name".to_owned(), Value::String(name.to_owned()));
        }
        v
    }

    /// Read a template from a file's or the settings' JSON. Returns it with the number of cells
    /// that were left out because they make no sense (a page that is not a whole number, a
    /// position that is not a number, a turn that is not a quarter turn).
    pub fn from_json(v: &Value) -> Result<(Self, usize), String> {
        let sheet = v["sheet_mm"].as_array().filter(|a| a.len() >= 2).and_then(|a| Some((a[0].as_f64()?, a[1].as_f64()?)));
        let Some((w, h)) = sheet.filter(|&(w, h)| w.is_finite() && h.is_finite()) else {
            return Err(tl!("This is not an imposition template (no sheet size).").to_owned());
        };
        let Some(sides) = v["sheets"].as_array() else {
            return Err(tl!("This is not an imposition template (no sheets).").to_owned());
        };
        if sides.len() > MAX_SIDES {
            return Err(tl!("The template has too many sheet sides.").to_owned());
        }
        let (mut dropped, mut total) = (0usize, 0usize);
        let mut sheets = Vec::with_capacity(sides.len());
        for side in sides {
            let cells = side.as_array().map_or(&[][..], Vec::as_slice);
            if cells.len() > MAX_ON_SIDE {
                return Err(tl!("The template has too many pages on a sheet side.").to_owned());
            }
            let mut out = Vec::with_capacity(cells.len());
            for c in cells {
                match cell_from_json(c) {
                    Some(cell) => out.push(cell),
                    None => dropped += 1,
                }
            }
            total += out.len();
            if total > MAX_CELLS {
                return Err(crate::i18n::fmt(tl!("A template holds at most {n} pages."), &[("n", &MAX_CELLS.to_string())]));
            }
            sheets.push(out);
        }
        if sheets.is_empty() {
            sheets.push(Vec::new());
        }
        let clamp = |x: f64| x.clamp(SHEET_RANGE.0, SHEET_RANGE.1);
        let bleed = v["bleed_mm"].as_f64().filter(|b| b.is_finite()).map_or(0.0, |b| b.clamp(0.0, BLEED_MAX));
        let marks = !matches!(v["marks"], Value::Null | Value::Bool(false));
        Ok((Self { sheet: (clamp(w), clamp(h)), bleed, marks, sheets }, dropped))
    }
}

/// One placed page of the file, or `None` when it makes no sense.
fn cell_from_json(c: &Value) -> Option<Cell> {
    let page = usize::try_from(c["page"].as_u64()?).ok()?;
    let (x, y) = (c["x"].as_f64().filter(|v| v.is_finite())?, c["y"].as_f64().filter(|v| v.is_finite())?);
    let turns = match c["turn"].as_i64()? {
        0 => 0,
        90 => 1,
        180 => 2,
        270 => 3,
        _ => return None,
    };
    Some(Cell { page, x: round2(tame(x)), y: round2(tame(y)), turns })
}

/// A name as kept: no control characters, trimmed, at most [`MAX_NAME_CHARS`] characters.
pub fn clean_name(name: &str) -> String {
    let kept: String = name.chars().filter(|c| !c.is_control()).collect();
    kept.trim().chars().take(MAX_NAME_CHARS).collect::<String>().trim().to_owned()
}

/// A file name stem for `name`: letters, digits and `_ . -`, runs of anything else become `_`.
#[cfg(not(target_arch = "wasm32"))]
pub fn file_stem(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() || matches!(c, '_' | '.' | '-') {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_start_matches('.').to_owned();
    if out.is_empty() { "layout-manual".to_owned() } else { out }
}

/// The templates kept in the settings, by name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Templates(BTreeMap<String, Template>);

impl Templates {
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    pub fn get(&self, name: &str) -> Option<&Template> {
        self.0.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Keep `t` as `name` (replacing a template of that name). Returns the name as kept.
    pub fn save(&mut self, name: &str, t: Template) -> Result<String, String> {
        let name = clean_name(name);
        if name.is_empty() {
            return Err(tl!("Give the template a name first.").to_owned());
        }
        if !self.0.contains_key(&name) && self.0.len() >= MAX_TEMPLATES {
            return Err(crate::i18n::fmt(tl!("At most {n} templates are kept. Delete one first."), &[("n", &MAX_TEMPLATES.to_string())]));
        }
        self.0.insert(name.clone(), t);
        Ok(name)
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.0.remove(name).is_some()
    }

    /// For the settings file.
    pub fn to_json(&self) -> Value {
        Value::Object(self.0.iter().map(|(k, t)| (k.clone(), t.to_json(None))).collect())
    }

    /// From the settings file; an entry that is not a template is left out.
    pub fn from_json(v: &Value) -> Self {
        let mut out = Self::default();
        for (name, t) in v.as_object().into_iter().flatten() {
            if let Ok((t, _)) = Template::from_json(t) {
                let _ = out.save(name, t);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(page: usize, x: f64, y: f64, turns: u8) -> Cell {
        Cell { page, x, y, turns }
    }

    fn draft() -> ManualDraft {
        ManualDraft {
            sheets: vec![vec![cell(0, 5.0, 6.0, 0), cell(1, 50.0, 6.0, 3)], vec![cell(2, 1.5, 2.5, 2)]],
            bleed: 3.0,
            marks: false,
            ..ManualDraft::default()
        }
    }

    #[test]
    fn a_layout_survives_the_trip_through_a_template() {
        let d = draft();
        let t = Template::from_draft(&d).expect("fits");
        let (back, dropped) = Template::from_json(&t.to_json(Some("n"))).expect("reads");
        assert_eq!((dropped, &back), (0, &t));
        let mut e = ManualDraft::default();
        assert_eq!(back.apply(&mut e, 3), 0);
        assert_eq!((e.sheets.clone(), e.sheet(), e.bleed, e.marks), (d.sheets.clone(), d.sheet(), 3.0, false));
    }

    #[test]
    fn the_file_has_the_light_apps_shape() {
        let v = Template::from_draft(&draft()).expect("fits").to_json(Some("A3"));
        assert_eq!(v["v"], 1);
        assert_eq!(v["name"], "A3");
        assert_eq!(v["sheet_mm"], json!([480.0, 320.0]));
        assert!(v["marks"].is_null());
        assert_eq!(v["sheets"][0][1], json!({ "page": 1, "x": 50.0, "y": 6.0, "turn": 270 }));
        let mut d = draft();
        d.marks = true;
        let v = Template::from_draft(&d).expect("fits").to_json(None);
        assert_eq!(v["marks"]["style"], "corner");
        assert!(v.get("name").is_none());
    }

    #[test]
    fn a_file_the_light_app_wrote_is_read() {
        let file = json!({
            "name": "x", "v": 1, "sheet_mm": [450, 320], "bleed_mm": 2,
            "marks": { "length_mm": 5, "offset_mm": 3, "style": "centred", "width_pt": 0.5, "registration": true },
            "sheets": [[{ "page": 0, "x": 10, "y": 20, "turn": 90 }], []]
        });
        let (t, dropped) = Template::from_json(&file).expect("reads");
        assert_eq!(dropped, 0);
        assert_eq!((t.sheet, t.bleed, t.marks), ((450.0, 320.0), 2.0, true));
        assert_eq!(t.sheets, vec![vec![cell(0, 10.0, 20.0, 1)], vec![]]);
        let mut d = ManualDraft::default();
        t.apply(&mut d, 1);
        // 450 x 320 is the SRA3 preset, wide.
        assert_eq!(PRESETS[d.preset].0, "SRA3");
        assert!(d.landscape);
    }

    #[test]
    fn cells_that_make_no_sense_are_dropped_and_counted() {
        let file = json!({ "sheet_mm": [320, 480], "sheets": [[
            { "page": 0, "x": 1, "y": 1, "turn": 0 },
            { "page": -1, "x": 1, "y": 1, "turn": 0 },
            { "page": 1.5, "x": 1, "y": 1, "turn": 0 },
            { "page": 1, "x": "a", "y": 1, "turn": 0 },
            { "page": 1, "x": 1, "y": 1, "turn": 45 },
            { "page": 1, "x": 1, "y": 1 },
            "junk", null
        ]]});
        let (t, dropped) = Template::from_json(&file).expect("reads");
        assert_eq!((t.sheets[0].len(), dropped), (1, 7));
    }

    #[test]
    fn what_is_not_a_template_is_an_error_and_numbers_are_kept_sane() {
        for bad in [
            json!(null),
            json!([]),
            json!({}),
            json!({ "sheets": [] }),
            json!({ "sheet_mm": [1, 2] }),
            json!({ "sheet_mm": [1], "sheets": [] }),
            json!({ "sheet_mm": ["a", 2], "sheets": [] }),
        ] {
            assert!(Template::from_json(&bad).is_err(), "{bad}");
        }
        let (t, _) =
            Template::from_json(&json!({ "sheet_mm": [1e9, -5], "bleed_mm": 1e9, "sheets": [[{ "page": 0, "x": 1e30, "y": -1e30, "turn": 0 }]] }))
                .expect("reads");
        assert_eq!(t.sheet, (SHEET_RANGE.1, SHEET_RANGE.0));
        assert_eq!(t.bleed, BLEED_MAX);
        assert_eq!(t.sheets[0][0], cell(0, 10_000.0, -10_000.0, 0));
        let (t, _) = Template::from_json(&json!({ "sheet_mm": [320, 480], "bleed_mm": "x", "sheets": [] })).expect("reads");
        assert_eq!((t.bleed, t.sheets.len()), (0.0, 1));
    }

    #[test]
    fn a_template_that_is_too_big_is_refused() {
        let cell = json!({ "page": 0, "x": 0, "y": 0, "turn": 0 });
        let over_side = json!({ "sheet_mm": [320, 480], "sheets": [vec![cell.clone(); MAX_ON_SIDE + 1]] });
        assert!(Template::from_json(&over_side).is_err());
        let over_total = json!({ "sheet_mm": [320, 480], "sheets": vec![vec![cell.clone(); 400]; MAX_CELLS / 400 + 1] });
        assert!(Template::from_json(&over_total).is_err());
        let at_total = json!({ "sheet_mm": [320, 480], "sheets": vec![vec![cell.clone(); 400]; MAX_CELLS / 400] });
        assert!(Template::from_json(&at_total).is_ok());
        let over_sides = json!({ "sheet_mm": [320, 480], "sheets": vec![json!([]); MAX_SIDES + 1] });
        assert!(Template::from_json(&over_sides).is_err());
        let d = ManualDraft { sheets: vec![vec![Cell { page: 0, x: 0.0, y: 0.0, turns: 0 }; MAX_CELLS + 1]], ..ManualDraft::default() };
        assert!(Template::from_draft(&d).is_err());
    }

    #[test]
    fn applying_leaves_out_pages_the_document_lacks_and_resets_the_view() {
        let t = Template::from_draft(&draft()).expect("fits");
        let mut d = ManualDraft { current: 1, sel: Some(0), sheets: vec![Vec::new(), Vec::new()], ..ManualDraft::default() };
        let skipped = t.apply(&mut d, 2);
        assert_eq!(skipped, 1);
        assert_eq!(d.sheets, vec![vec![cell(0, 5.0, 6.0, 0), cell(1, 50.0, 6.0, 3)], vec![]]);
        assert_eq!((d.current, d.sel), (0, None));
        // No page at all still leaves a sheet side to work on.
        let mut e = ManualDraft::default();
        assert_eq!(t.apply(&mut e, 0), 3);
        assert_eq!(e.sheets.len(), 2);
    }

    #[test]
    fn a_sheet_that_is_no_preset_becomes_a_custom_one() {
        let (t, _) = Template::from_json(&json!({ "sheet_mm": [500, 300], "sheets": [[]] })).expect("reads");
        let mut d = ManualDraft::default();
        t.apply(&mut d, 1);
        assert_eq!(d.preset, PRESETS.len());
        assert_eq!(d.sheet(), (500.0, 300.0));
        let (t, _) = Template::from_json(&json!({ "sheet_mm": [300, 500], "sheets": [[]] })).expect("reads");
        t.apply(&mut d, 1);
        assert_eq!(d.sheet(), (300.0, 500.0));
    }

    #[test]
    fn names_are_cleaned_and_a_template_of_the_same_name_is_replaced() {
        assert_eq!(clean_name("  a\u{7}b\n "), "ab");
        assert_eq!(clean_name(&"é".repeat(100)).chars().count(), MAX_NAME_CHARS);
        assert_eq!(clean_name("   "), "");
        let t = Template::from_draft(&draft()).expect("fits");
        let mut all = Templates::default();
        assert!(all.save("  ", t.clone()).is_err());
        assert_eq!(all.save(" A3 ", t.clone()).as_deref(), Ok("A3"));
        let mut other = t.clone();
        other.bleed = 7.0;
        all.save("A3", other).expect("replaces");
        assert_eq!(all.names().collect::<Vec<_>>(), ["A3"]);
        assert_eq!(all.get("A3").map(|t| t.bleed), Some(7.0));
        assert!(all.remove("A3"));
        assert!(!all.remove("A3"));
    }

    #[test]
    fn the_count_of_templates_is_capped_but_a_replacement_is_not_refused() {
        let t = Template::from_draft(&draft()).expect("fits");
        let mut all = Templates::default();
        for i in 0..MAX_TEMPLATES {
            all.save(&format!("t{i}"), t.clone()).expect("room");
        }
        assert!(all.save("one more", t.clone()).is_err());
        assert!(all.save("t0", t).is_ok());
        assert_eq!(all.names().count(), MAX_TEMPLATES);
    }

    #[test]
    fn the_settings_round_trip_and_a_bad_entry_is_left_out() {
        let t = Template::from_draft(&draft()).expect("fits");
        let mut all = Templates::default();
        all.save("one", t.clone()).expect("room");
        all.save("two", t).expect("room");
        let mut v = all.to_json();
        assert_eq!(Templates::from_json(&v), all);
        v["bad"] = json!("nope");
        v["worse"] = json!({ "sheets": 3 });
        assert_eq!(Templates::from_json(&v), all);
        assert_eq!(Templates::from_json(&json!([1, 2])), Templates::default());
    }

    #[test]
    fn a_file_name_comes_from_the_template_name() {
        assert_eq!(file_stem("A3 two-up / back"), "A3_two-up_back");
        assert_eq!(file_stem("..hidden"), "hidden");
        assert_eq!(file_stem("***"), "_");
        assert_eq!(file_stem(""), "layout-manual");
        assert_eq!(file_stem("kartu nama é"), "kartu_nama_é");
    }
}
