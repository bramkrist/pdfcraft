# pdfcraft-cutline

Cut lines for stickers (layer L0, standalone, no dependencies): the closed outline a cutter
should follow round a rendered page, at a signed distance from the artwork's edge.

```text
Mask (coverage per pixel)  →  trace(offset, holes, …)  →  Vec<Loop> (closed polygons, pixels)
```

- `Mask::from_rgba` / `from_straight_rgba` turn a rendering (premultiplied, or a browser canvas'
  straight pixels) into a silhouette. `Background::Auto` takes transparency as empty paper when
  the page has any, otherwise near-white. `Mask::keep_only` limits it to what a placement shows.
- `trace` fills the holes inside the artwork (or keeps them), takes the exact Euclidean distance
  to the silhouette (Felzenszwalb–Huttenlocher), and follows the iso-line at the offset with
  marching squares (positive outside, 0 on the edge, negative inside). Outer outlines come first,
  clockwise holes after; points are simplified by Douglas–Peucker to `tolerance` (default 0.35 px; callers tracing a rendered page use more, as a binary mask's edge is a staircase up to ~0.7 px deep).
- Straight edges come out exact; a corner diagonal ends about 0.2 px short.

Never panics on any input; the work and memory are bounded by `MAX_CELLS` (16 million cells,
with the padding the offset needs) and `MAX_OFFSET`. The same input always gives the same
polygons. A 9-megapixel mask traces in about 0.75 s.

`pdfcraft-impose` places the outlines on sheets (`place_cuts`, `trace_page`), `pdfcraft-print`
writes them as a spot colour on a layer called Cut, and the engine, the `doc_impose` tool, the
Pro wizard and the light web app drive it. Not here: PLT/HPGL export and registration marks (see
`parity/acrobat-features.toml`, `print.step-repeat-impose`).
