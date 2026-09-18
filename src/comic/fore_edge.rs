use egui::{Color32, ColorImage};

/// Row count of every page's strip in the fore-edge composite texture (see
/// `ui::fore_edge`) — matching `thumbnail::LOW_RES_MAX_DIMENSION` (the same
/// resolution the low-res whole-book preview decodes at) means a page's
/// fore-edge sample and its progress-bar thumbnail could in principle share
/// one decode, even though today each is decoded separately (see
/// `comic::archive::ComicArchive::load`, which decodes for the fore-edge
/// during the archive read itself, concurrently with extraction).
pub const FORE_EDGE_HEIGHT: usize = crate::comic::thumbnail::LOW_RES_MAX_DIMENSION as usize;

/// How many of a page's own outermost source columns it contributes to the
/// composite — two adjacent pixels rather than one, so each page reads as
/// an actual (if thin) stripe once the composite is stretched into the
/// on-screen bar, instead of a single texel that's easy to lose entirely to
/// bilinear blending with its neighbors.
pub const EDGE_SAMPLE_WIDTH: usize = 2;

/// The `EDGE_SAMPLE_WIDTH`-wide, full-height strip from one side of `image`
/// — its rightmost two columns if `on_right`, leftmost two otherwise —
/// resampled (nearest source row) to `FORE_EDGE_HEIGHT` rows so every page
/// contributes a strip of the same length regardless of its own decoded
/// resolution. Row-major, ready for a `[EDGE_SAMPLE_WIDTH, FORE_EDGE_HEIGHT]`
/// `ColorImage` — column 0 is always the true outermost pixel, column 1 the
/// one just inside it.
///
/// `ComicArchive::load` samples *both* sides of a page (rather than calling
/// this once with the answer from `edge_on_right`) because, while archive
/// extraction is still in progress, a page's final sorted index — and so
/// which side it'll actually want — isn't known yet; picking between the
/// two already-sampled strips once sorting resolves it is free, versus
/// decoding the page a second time.
pub fn sample_side(image: &ColorImage, on_right: bool) -> Vec<Color32> {
    let [w, h] = image.size;
    if w == 0 || h == 0 {
        return vec![Color32::TRANSPARENT; EDGE_SAMPLE_WIDTH * FORE_EDGE_HEIGHT];
    }
    let (outer, inner) = if on_right { (w - 1, w.saturating_sub(2)) } else { (0, 1.min(w - 1)) };

    let mut pixels = Vec::with_capacity(EDGE_SAMPLE_WIDTH * FORE_EDGE_HEIGHT);
    for row in 0..FORE_EDGE_HEIGHT {
        let src_y = (row * h / FORE_EDGE_HEIGHT).min(h - 1) * w;
        pixels.push(image.pixels[src_y + outer]);
        pixels.push(image.pixels[src_y + inner]);
    }
    pixels
}

/// Whether `page_idx` (0-based) should sample its right edge rather than its
/// left — true for an even 1-based page number (page 2, 4, 6, ...). `pub`
/// so `ComicArchive::load` can apply the same rule once a page's final
/// (sorted) index is known, to pick between its already-sampled `left`/
/// `right` strips (see `sample_side`) rather than re-deriving the rule.
pub fn edge_on_right(page_idx: usize) -> bool {
    (page_idx + 1).is_multiple_of(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_with_edges(width: usize, height: usize, left: Color32, right: Color32) -> ColorImage {
        let mut pixels = vec![Color32::GRAY; width * height];
        for y in 0..height {
            pixels[y * width] = left;
            pixels[y * width + width - 1] = right;
        }
        ColorImage::new([width, height], pixels)
    }

    /// Row `row`'s two samples, as written into the composite (column 0,
    /// then column 1) — a small helper so each test can index by row
    /// without re-deriving the row-major layout.
    fn row(columns: &[Color32], row: usize) -> (Color32, Color32) {
        (columns[row * EDGE_SAMPLE_WIDTH], columns[row * EDGE_SAMPLE_WIDTH + 1])
    }

    #[test]
    fn odd_displayed_page_number_wants_the_left_edge() {
        assert!(!edge_on_right(0)); // page_idx 0 -> displayed page 1, odd
    }

    #[test]
    fn even_displayed_page_number_wants_the_right_edge() {
        assert!(edge_on_right(1)); // page_idx 1 -> displayed page 2, even
    }

    #[test]
    fn sample_side_gives_both_edges_from_one_decode() {
        let image = image_with_edges(10, FORE_EDGE_HEIGHT, Color32::BLACK, Color32::WHITE);
        // Outermost pixel matches the requested side (left=x0/black,
        // right=x(w-1)/white); the second column is the gray fill just
        // inside it either way.
        assert_eq!(row(&sample_side(&image, false), 0), (Color32::BLACK, Color32::GRAY));
        assert_eq!(row(&sample_side(&image, true), 0), (Color32::WHITE, Color32::GRAY));
    }

    #[test]
    fn resamples_to_fore_edge_height_regardless_of_source_height() {
        let image = image_with_edges(4, 10, Color32::BLACK, Color32::WHITE);
        assert_eq!(sample_side(&image, false).len(), EDGE_SAMPLE_WIDTH * FORE_EDGE_HEIGHT);

        let tall = image_with_edges(4, FORE_EDGE_HEIGHT * 3, Color32::BLACK, Color32::WHITE);
        assert_eq!(sample_side(&tall, false).len(), EDGE_SAMPLE_WIDTH * FORE_EDGE_HEIGHT);
    }

    #[test]
    fn empty_image_returns_a_transparent_strip() {
        let image = ColorImage::new([0, 0], Vec::new());
        assert_eq!(sample_side(&image, false), vec![Color32::TRANSPARENT; EDGE_SAMPLE_WIDTH * FORE_EDGE_HEIGHT]);
    }

    #[test]
    fn a_single_pixel_wide_page_reuses_it_for_both_columns() {
        let image = ColorImage::new([1, FORE_EDGE_HEIGHT], vec![Color32::RED; FORE_EDGE_HEIGHT]);
        assert_eq!(row(&sample_side(&image, false), 0), (Color32::RED, Color32::RED));
    }
}
