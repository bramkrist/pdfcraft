//! Where a page has something drawn on it: the box Acrobat's "Remove White Margins" crops to.
//! Found from a low-resolution render, so it is what a viewer shows that counts.

use crate::{PageInfo, PageRenderer, RenderRequest, RequestKind};

/// The longest side of the picture the content is looked for in, in pixels: a larger page is
/// searched at lower resolution rather than in a huge raster.
const MAX_SIDE_PX: f32 = 2400.0;
/// A pixel counts as drawn when it is not opaque, or a colour channel is darker than this.
const PAPER: u8 = 250;
/// Padding past this is not a margin any more.
pub const MAX_PADDING_PT: f64 = 1000.0;

/// The smallest pixel box `[left, top, right, bottom]` (right and bottom exclusive) that holds
/// every drawn pixel of a premultiplied RGBA8 raster, or `None` when nothing is drawn (or the
/// buffer does not match its size). A fully transparent pixel is paper; an opaque one is paper
/// when every channel is at least [`PAPER`].
pub fn content_pixels(rgba: &[u8], width: usize, height: usize) -> Option<[usize; 4]> {
    let row = width.checked_mul(4)?;
    if width == 0 || height == 0 || rgba.len() != row.checked_mul(height)? {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for (y, line) in rgba.chunks_exact(row).enumerate() {
        for (x, px) in line.chunks_exact(4).enumerate() {
            let [r, g, b, a] = [px[0], px[1], px[2], px[3]];
            // Premultiplied: paper is transparent, or opaque and light in every channel. A
            // translucent pixel carries some ink, however light, so it is drawn.
            let paper = a == 0 || (a == 255 && r >= PAPER && g >= PAPER && b >= PAPER);
            if !paper {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x0 != usize::MAX).then_some([x0, y0, x1, y1])
}

/// A pixel box as a box in points of the displayed page, grown by one pixel (the edge of an
/// anti-aliased stroke) and by `padding`, and kept inside the page; `None` when it comes out
/// empty.
pub fn content_rect(px: [usize; 4], scale: f32, page: (f32, f32), padding: f64) -> Option<[f64; 4]> {
    let s = f64::from(scale);
    if !(s.is_finite() && s > 0.0) {
        return None;
    }
    let grow = 1.0 / s + padding;
    let (pw, ph) = (f64::from(page.0), f64::from(page.1));
    let r = [
        (px[0] as f64 / s - grow).max(0.0),
        (px[1] as f64 / s - grow).max(0.0),
        (px[2] as f64 / s + grow).min(pw),
        (px[3] as f64 / s + grow).min(ph),
    ];
    (r.iter().all(|v| v.is_finite()) && r[2] - r[0] >= 1.0 && r[3] - r[1] >= 1.0).then_some(r)
}

/// What [`content_boxes`] found: the pages with something on them and the box to crop each to
/// (0-based page, `[left, bottom, right, top]` in PDF user space), and the pages with nothing.
pub type ContentBoxes = (Vec<(usize, [f64; 4])>, Vec<usize>);

/// The pages to crop, found from renders of `pages` (0-based), and which of them have nothing on
/// them: each cropped page comes with the box to set in PDF user space, `[left, bottom, right,
/// top]`, grown by `padding` points and kept on the page.
///
/// # Errors
/// A page that cannot be rendered, or a padding outside `0..=MAX_PADDING_PT`.
pub fn content_boxes(renderer: &mut PageRenderer, infos: &[PageInfo], pages: &[usize], padding: f64) -> Result<ContentBoxes, String> {
    if !(0.0..=MAX_PADDING_PT).contains(&padding) {
        return Err(format!("padding must be between 0 and {MAX_PADDING_PT} points"));
    }
    let (mut boxes, mut blank) = (Vec::new(), Vec::new());
    for &page in pages {
        let Some(info) = infos.get(page) else { continue };
        let scale = (MAX_SIDE_PX / info.width.max(info.height).max(1.0)).min(1.0);
        let out = renderer.render(RenderRequest { page, kind: RequestKind::Pixels, tile: None, scale, tag: 0, preview: false });
        if let Some(e) = out.error {
            return Err(format!("page {} could not be rendered: {e}", page + 1));
        }
        let found = content_pixels(&out.rgba, out.width as usize, out.height as usize)
            .and_then(|px| content_rect(px, scale, (info.width, info.height), padding));
        let Some(r) = found else {
            blank.push(page);
            continue;
        };
        // View space (top-left origin, after /Rotate) to user space.
        let (u0, u1) = (info.view_to_user(r[0] as f32, r[1] as f32), info.view_to_user(r[2] as f32, r[3] as f32));
        boxes.push((page, [u0[0].min(u1[0]), u0[1].min(u1[1]), u0[0].max(u1[0]), u0[1].max(u1[1])].map(f64::from)));
    }
    Ok((boxes, blank))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raster(w: usize, h: usize, fill: [u8; 4]) -> Vec<u8> {
        fill.iter().copied().cycle().take(w * h * 4).collect()
    }

    fn put(buf: &mut [u8], w: usize, x: usize, y: usize, px: [u8; 4]) {
        buf[(y * w + x) * 4..][..4].copy_from_slice(&px);
    }

    #[test]
    fn finds_the_box_around_what_is_drawn() {
        let mut p = raster(10, 8, [255, 255, 255, 255]);
        put(&mut p, 10, 2, 3, [0, 0, 0, 255]);
        put(&mut p, 10, 6, 5, [10, 200, 30, 255]);
        assert_eq!(content_pixels(&p, 10, 8), Some([2, 3, 7, 6]));
    }

    #[test]
    fn paper_is_white_or_transparent_and_a_faint_ink_still_counts() {
        assert_eq!(content_pixels(&raster(4, 4, [255, 255, 255, 255]), 4, 4), None);
        assert_eq!(content_pixels(&raster(4, 4, [0, 0, 0, 0]), 4, 4), None);
        let mut p = raster(4, 4, [255, 255, 255, 255]);
        put(&mut p, 4, 1, 1, [248, 255, 255, 255]);
        assert_eq!(content_pixels(&p, 4, 4), Some([1, 1, 2, 2]), "a hairline lighter than 250 is drawn");
        let mut q = raster(4, 4, [0, 0, 0, 0]);
        put(&mut q, 4, 3, 0, [1, 1, 1, 1]);
        assert_eq!(content_pixels(&q, 4, 4), Some([3, 0, 4, 1]), "a translucent pixel is ink");
    }

    #[test]
    fn a_buffer_that_does_not_match_its_size_is_not_searched() {
        assert_eq!(content_pixels(&[0; 12], 2, 2), None);
        assert_eq!(content_pixels(&[], 0, 0), None);
        assert_eq!(content_pixels(&[0; 16], 0, 4), None);
        assert_eq!(content_pixels(&[0; 16], usize::MAX, 2), None, "no overflow");
    }

    #[test]
    fn the_rect_grows_by_a_pixel_and_the_padding_and_stays_on_the_page() {
        // Scale 1: pixel (10,20)-(30,60) on a 100 x 100 page, 5 pt padding, 1 px of edge.
        assert_eq!(content_rect([10, 20, 30, 60], 1.0, (100.0, 100.0), 5.0), Some([4.0, 14.0, 36.0, 66.0]));
        // Padding never takes the box off the page.
        assert_eq!(content_rect([0, 0, 100, 100], 1.0, (100.0, 100.0), 50.0), Some([0.0, 0.0, 100.0, 100.0]));
        // Half resolution: a pixel is 2 pt.
        assert_eq!(content_rect([10, 10, 20, 20], 0.5, (100.0, 100.0), 0.0), Some([18.0, 18.0, 42.0, 42.0]));
        assert_eq!(content_rect([1, 1, 2, 2], 0.0, (100.0, 100.0), 0.0), None);
        assert_eq!(content_rect([1, 1, 2, 2], f32::NAN, (100.0, 100.0), 0.0), None);
    }
}
