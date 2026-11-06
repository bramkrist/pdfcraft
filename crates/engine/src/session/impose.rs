//! Step & repeat and n-up imposition of a document's pages onto press sheets.

use super::{Arc, DocId, EditError, Session, print, set_default_layer_state};

impl Session {
    /// The print-ready PDF of document `id` laid out on press sheets by `spec` (step & repeat,
    /// gang n-up, duplex backs, trim marks; see `pdfcraft-impose`). Printing must be permitted;
    /// layers print as they are shown, as in [`Session::print_pdf`].
    pub fn impose_pdf(&self, id: DocId, spec: &crate::impose::StepRepeat, content: print::Content) -> Result<Vec<u8>, EditError> {
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
        crate::impose::impose_document(&cos, spec, content).map_err(|e| EditError::Print(e.to_string()))
    }
}
