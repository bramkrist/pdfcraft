//! The silhouette a trace starts from: per-pixel coverage.

use crate::CutError;

/// Which pixels of a rendered page are artwork.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Background {
    /// Transparent where the page is empty when it has any transparent pixel (a vector page, a
    /// PNG with an alpha channel); otherwise white paper (a JPEG, a scan).
    #[default]
    Auto,
    /// Only transparency separates the artwork from the page.
    Transparent,
    /// Near-white pixels are paper, whatever their alpha.
    White,
}

/// Per-pixel coverage of the artwork, 0 (empty) to 255 (solid), row by row from the top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    width: usize,
    height: usize,
    coverage: Vec<u8>,
}

impl Mask {
    /// A mask of `width` × `height` pixels; `coverage` must hold exactly that many.
    pub fn new(width: usize, height: usize, coverage: Vec<u8>) -> Result<Self, CutError> {
        if width == 0 || height == 0 || width.checked_mul(height) != Some(coverage.len()) {
            return Err(CutError::BadMask);
        }
        Ok(Self { width, height, coverage })
    }

    /// The silhouette of premultiplied RGBA pixels (4 bytes each, row by row from the top), as
    /// a rendered page comes: `background` says what counts as empty.
    pub fn from_rgba(width: usize, height: usize, rgba: &[u8], background: Background) -> Result<Self, CutError> {
        Self::from_pixels(width, height, rgba, background, true)
    }

    /// [`Mask::from_rgba`] for straight (not premultiplied) pixels, as a browser canvas gives them.
    pub fn from_straight_rgba(width: usize, height: usize, rgba: &[u8], background: Background) -> Result<Self, CutError> {
        Self::from_pixels(width, height, rgba, background, false)
    }

    fn from_pixels(width: usize, height: usize, rgba: &[u8], background: Background, premultiplied: bool) -> Result<Self, CutError> {
        let bytes = width.checked_mul(height).and_then(|n| n.checked_mul(4)).ok_or(CutError::BadMask)?;
        if bytes != rgba.len() {
            return Err(CutError::BadMask);
        }
        let background = match background {
            Background::Auto if rgba.chunks_exact(4).any(|p| p.get(3).is_some_and(|a| *a < 250)) => Background::Transparent,
            Background::Auto => Background::White,
            other => other,
        };
        let coverage = rgba.chunks_exact(4).map(|p| pixel_coverage(p, background, premultiplied)).collect();
        Self::new(width, height, coverage)
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// Empty every pixel whose centre lies outside the rectangle `x0..x1` × `y0..y1` (pixel
    /// units, y down from the top edge): what a clipped placement shows is all that is cut.
    /// A non-finite or inverted rectangle empties the whole mask.
    pub fn keep_only(&mut self, x0: f64, y0: f64, x1: f64, y1: f64) {
        let width = self.width.max(1);
        let inside = |lo: f64, hi: f64, i: usize| {
            let centre = i as f64 + 0.5;
            centre >= lo && centre < hi
        };
        for (y, row) in self.coverage.chunks_exact_mut(width).enumerate() {
            let row_in = inside(y0, y1, y);
            for (x, value) in row.iter_mut().enumerate() {
                if !(row_in && inside(x0, x1, x)) {
                    *value = 0;
                }
            }
        }
    }

    /// The rows from the top, `width` coverage values each.
    pub(crate) fn rows(&self) -> std::slice::ChunksExact<'_, u8> {
        self.coverage.chunks_exact(self.width.max(1))
    }
}

/// The artwork's coverage of one RGBA pixel, premultiplied or straight.
fn pixel_coverage(pixel: &[u8], background: Background, premultiplied: bool) -> u8 {
    let Ok([r, g, b, a]) = <[u8; 4]>::try_from(pixel) else { return 0 };
    if background != Background::White {
        return a;
    }
    if a == 0 {
        return 0;
    }
    // The palest channel as it would be on opaque paper; the further it is from white, the more
    // is ink. A quarter of the way from white is already solid, so a pale tint counts and paper
    // noise does not.
    let palest = u32::from(r.min(g).min(b));
    let palest = if premultiplied { (palest * 255 / u32::from(a)).min(255) } else { palest };
    let ink = (255u32.saturating_sub(palest) * 4).min(255);
    (ink * u32::from(a) / 255) as u8
}
