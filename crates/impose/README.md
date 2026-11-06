# pdfcraft-impose

Print-production imposition (layer L4): step & repeat, gang n-up with gutters, duplex back
mirroring and corner trim marks. The crate lays pages out as `pdfcraft_print::Sheet`s from page
sizes alone (plain numbers, no PDF objects), and `impose_document` hands them to
`pdfcraft_print::impose_sheets` to write the PDF, so pages stay vector.

Lengths are PDF points; the `doc_impose` tool takes millimetres.

Not yet: creep, work-and-turn/tumble, perfect-bound signatures, cell rotation, colour bars,
registration marks. See `parity/acrobat-features.toml` (`print.step-repeat-impose`).
