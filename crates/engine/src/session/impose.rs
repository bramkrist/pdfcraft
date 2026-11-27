//! Step & repeat and n-up imposition of a document's pages onto press sheets.

use super::{Arc, DocId, EditError, Session, print, set_default_layer_state};

impl Session {
    /// The print-ready PDF of document `id` laid out on press sheets by `spec` (step & repeat,
    /// gang n-up, duplex backs, trim marks; see `pdfcraft-impose`). Printing must be permitted;
    /// layers print as they are shown, as in [`Session::print_pdf`].
    pub fn impose_pdf(&self, id: DocId, spec: &crate::impose::StepRepeat, content: print::Content) -> Result<Vec<u8>, EditError> {
        let cos = self.imposition_source(id)?;
        crate::impose::impose_document(&cos, spec, content).map_err(|e| EditError::Print(e.to_string()))
    }

    /// [`Session::impose_pdf`] with cut lines (sticker die-cut and kiss-cut contours): each copy
    /// of a page gets a line traced round its artwork at `cut.offset` from its edge, drawn last
    /// in one spot colour on its own layer. Returns the PDF and what was cut.
    pub fn impose_cut_pdf(
        &self,
        id: DocId,
        spec: &crate::impose::StepRepeat,
        cut: &crate::impose::CutSpec,
        content: print::Content,
    ) -> Result<(Vec<u8>, crate::cutting::CutStats), EditError> {
        let cos = self.imposition_source(id)?;
        let config = self.get(id).ok_or(EditError::NoDocument)?.config.clone();
        crate::guard(|| crate::cutting::impose_with_cuts(&cos, &config, spec, cut, content)).map_err(EditError::Print)?.map_err(EditError::Print)
    }

    /// [`Session::impose_cut_pdf`] for a plotter-cutter: with `cutter`, every sheet that has cut
    /// lines also gets registration marks and a QR code, and a PLT file of its lines comes back
    /// beside the PDF. Fails, making nothing, when the cutter's output cannot be made.
    pub fn impose_cut_job(
        &self,
        id: DocId,
        spec: &crate::impose::StepRepeat,
        cut: &crate::impose::CutSpec,
        cutter: Option<&crate::impose::CutterSpec>,
        content: print::Content,
    ) -> Result<crate::cutting::CutJob, EditError> {
        let cos = self.imposition_source(id)?;
        let config = self.get(id).ok_or(EditError::NoDocument)?.config.clone();
        crate::guard(|| crate::cutting::cut_job(&cos, &config, spec, cut, cutter, content)).map_err(EditError::Print)?.map_err(EditError::Print)
    }

    /// The print-ready PDF of document `id` made into a booklet by `spec` (saddle-stitched,
    /// perfect-bound or continuous; see `pdfcraft-impose`). The same permissions and layer rules
    /// apply as for [`Session::impose_pdf`].
    pub fn booklet_pdf(&self, id: DocId, spec: &crate::impose::Booklet, content: print::Content) -> Result<Vec<u8>, EditError> {
        let cos = self.imposition_source(id)?;
        crate::impose::booklet_document(&cos, spec, content).map_err(|e| EditError::Print(e.to_string()))
    }

    /// The print-ready PDF of document `id` made into a booklet by `spec` with two booklet sheet
    /// sides laid on each side of the big sheet of `two` (two copies of the booklet per press
    /// sheet; see `pdfcraft_impose::two_up_spec`). The same permissions and layer rules apply as
    /// for [`Session::impose_pdf`].
    pub fn booklet_two_up_pdf(
        &self,
        id: DocId,
        spec: &crate::impose::Booklet,
        two: &crate::impose::TwoUp,
        content: print::Content,
    ) -> Result<Vec<u8>, EditError> {
        let cos = self.imposition_source(id)?;
        crate::impose::booklet_two_up_document(&cos, spec, two, content).map_err(|e| EditError::Print(e.to_string()))
    }

    /// The print-ready PDF of document `id` laid out by hand by `spec` (see `pdfcraft-impose`).
    /// The same permissions and layer rules apply as for [`Session::impose_pdf`].
    pub fn manual_pdf(&self, id: DocId, spec: &crate::impose::Manual, content: print::Content) -> Result<Vec<u8>, EditError> {
        let cos = self.imposition_source(id)?;
        crate::impose::manual_document(&cos, spec, content).map_err(|e| EditError::Print(e.to_string()))
    }

    /// The print-ready PDF of document `id` with bleed added around every page (`pdfcraft-impose`):
    /// each page grows by `spec` and gets a `TrimBox` at its old edge. The same permissions and
    /// layer rules apply as for [`Session::impose_pdf`].
    pub fn bleed_pdf(&self, id: DocId, spec: &crate::impose::AddBleed, content: print::Content) -> Result<Vec<u8>, EditError> {
        let cos = self.imposition_source(id)?;
        crate::impose::bleed_document(&cos, spec, content).map_err(|e| EditError::Print(e.to_string()))
    }

    /// The document as imposition reads it: printing permitted, layers as shown.
    fn imposition_source(&self, id: DocId) -> Result<pdfcraft_cos::Document, EditError> {
        let doc = self.get(id).ok_or(EditError::NoDocument)?;
        if !doc.allows_printing() {
            return Err(EditError::NotPermitted("printing"));
        }
        // Table 22, bit 12 clear: only a low-level representation may be printed. Imposition
        // keeps the vector pages, so it can't honour that: refuse rather than exceed the permission.
        if doc.permissions().is_some_and(|p| !p.print_high_quality()) {
            return Err(EditError::NotPermitted("high-quality printing"));
        }
        let mut cos = match doc.editor.as_ref() {
            Some(e) => e.cos.clone(),
            None => pdfcraft_cos::Document::open_with_password(Arc::clone(&doc.bytes), doc.password.as_deref())
                .map_err(|e| EditError::Write(e.to_string()))?,
        };
        if !doc.config.layers.is_empty() {
            set_default_layer_state(&mut cos, &doc.info.layers);
        }
        Ok(cos)
    }
}
