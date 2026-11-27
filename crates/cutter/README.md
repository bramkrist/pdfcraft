# pdfcraft-cutter

A plotter-cutter's sheet (layer L0, standalone; depends only on `qrcode`): the registration
marks and QR code to print, and the PLT (HPGL) file of the cut lines.

```text
Job { name, sheet, layout, loops }  →  plt()    →  bytes of name.plt
sheet, name, layout                 →  marks()  →  rectangles to print (clear, black)
```

- `plt` follows a working sample from the shop's machine, byte for byte: a header padded to 1103
  bytes with `SIZE` equal to the real file size, 40 units to the millimetre, origin at the
  lower-left mark, every contour closed, clockwise, starting at its leftmost point, shapes in
  a column-by-column serpentine order, and `PU W,0;!PG;` at the end. A cut line outside the
  marks' rectangle is refused (`OutsideMarks`).
- `marks` puts a square mark at each corner (`Layout::inset_mm` from the sheet's edge to the
  mark's centre) and the QR code at the bottom centre, with a clear border so it reads on any
  paper. The QR holds the job's name, the same name the PLT file carries.
- `clean_name` keeps letters, digits, `-` and `_` (at most 32), so a name is safe as a file
  name and as the code's content.

**Inferred, not measured:** the PLT's header gives the marks' distance (W,H) and the inset, but
the sample has no print file with the marks. The mark's shape and size (a 5 mm black square), the
QR's content and place, and reading the origin as the lower-left mark's centre are choices made
from that header. Print one sheet and cut it before trusting a run.

No `unsafe`, no panics: every number is checked, names are cleaned, and an error leaves the
caller's sheets as they were (`pdfcraft_impose::apply_cutter` is all-or-nothing).
