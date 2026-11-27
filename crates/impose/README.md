# pdfcraft-impose

Print-production imposition (layer L4): step & repeat, gang n-up with gutters, duplex back
mirroring and corner trim marks. The crate lays pages out as `pdfcraft_print::Sheet`s from page
sizes alone (plain numbers, no PDF objects), and `impose_document` hands them to
`pdfcraft_print::impose_sheets` to write the PDF, so pages stay vector.

Sticker cut lines: `cut_requests` lists which placements of a page need a trace, the caller draws
them and `trace_page` (with `pdfcraft-cutline`) finds the line at a signed offset on the sheet,
and `place_cuts` puts one on every copy. The engine and the web app each draw the pages their own
way; the geometry is shared.

Plotter-cutter: `apply_cutter` (with `pdfcraft-cutter`) puts the registration marks and the QR
code on every sheet that has cut lines and returns one PLT file for each (`name.plt`, or
`name-1.plt`, `name-2.plt`, … for several). It is all-or-nothing: on an error no sheet changes.

Lengths are PDF points; the `doc_impose` tool takes millimetres.

Not yet: creep, work-and-turn/tumble, perfect-bound signatures, cell rotation, colour bars,
registration marks, PLT export. See `parity/acrobat-features.toml` (`print.step-repeat-impose`).
