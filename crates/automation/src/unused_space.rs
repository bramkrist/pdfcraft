//! `page_remove_unused_space`: crop pages to what is drawn on them (Acrobat's "Remove White
//! Margins"); the search itself is `pdfcraft_render::content_boxes`.

use pdfcraft_engine::{BoxSpec, Edit, PageBox};
use serde_json::{Value, json};

use crate::json::Args;
use crate::{Automation, Result, ToolError, failed};

impl Automation {
    /// `page_remove_unused_space`: set each page's crop box to the area that has anything drawn
    /// on it, plus `padding_pt`. Blank pages are left alone and listed.
    pub(crate) fn page_remove_unused_space(&mut self, a: &Args) -> Result<Value> {
        let padding = a.opt_num("padding_pt")?.unwrap_or(0.0);
        if !(0.0..=pdfcraft_render::MAX_PADDING_PT).contains(&padding) {
            return Err(ToolError::InvalidArgs(format!("padding_pt must be between 0 and {}", pdfcraft_render::MAX_PADDING_PT)));
        }
        let doc = self.doc(a)?;
        let id = doc.id;
        let infos = doc.info.pages.clone();
        let pages = match a.opt_ints("pages")? {
            Some(_) => self.pages(a, "pages")?,
            None => (0..infos.len()).collect(),
        };
        let (boxes, blank) = pdfcraft_render::content_boxes(self.renderer(id)?, &infos, &pages, padding).map_err(failed)?;
        let blank: Vec<usize> = blank.iter().map(|p| p + 1).collect();
        if boxes.is_empty() {
            return Ok(json!({ "cropped": [], "blank_pages": blank }));
        }
        let cropped: Vec<usize> = boxes.iter().map(|(p, _)| p + 1).collect();
        let edits = boxes.into_iter().map(|(page, r)| Edit::SetPageBox { pages: vec![page], which: PageBox::Crop, spec: BoxSpec::Rect(r) }).collect();
        let mut out = self.apply(a, Edit::Batch { label: "Remove unused space".into(), edits })?;
        out["cropped"] = json!(cropped);
        out["blank_pages"] = json!(blank);
        out["page_sizes"] = json!(self.doc(a)?.info.pages.iter().map(|p| [p.width, p.height]).collect::<Vec<_>>());
        Ok(out)
    }
}
