use anyhow::Result;
use std::path::Path;

/// An opened comic archive. Pages are kept as their original *compressed*
/// bytes (a few hundred KB each, typically) rather than decoded pixels —
/// decoding every page up front would mean holding the whole book as raw
/// RGBA in memory (tens of MB per page) even though only a couple of pages
/// are ever on screen at once. Callers decode a page (via `decode_image`)
/// only when they're about to display it.
pub struct ComicArchive {
    pub pages: Vec<Vec<u8>>,
}

/// Called after each page is extracted, in archive order (not final reading
/// order — that's only known once every entry has been read and sorted).
/// Carries a decoded preview image only for the very first entry (so the UI
/// has something to show while the rest of the archive is still extracting);
/// `None` for every entry after that.
type ProgressFn<'a> = dyn FnMut(usize, usize, Option<&egui::ColorImage>) + 'a;

impl ComicArchive {
    pub async fn load(
        path: &Path,
        mut on_progress: impl FnMut(usize, usize, Option<&egui::ColorImage>),
    ) -> Result<Self> {
        let extension = path.extension().unwrap_or_default().to_str().unwrap_or("").to_lowercase();

        let pages = match extension.as_str() {
            "cbz" | "zip" => Self::load_zip(path, &mut on_progress).await?,
            "cb7" | "7z" => Self::load_7z(path, &mut on_progress).await?,
            "cbr" | "rar" => Self::load_rar(path, &mut on_progress).await?,
            _ => anyhow::bail!("Format non supporté: {}", extension),
        };

        Ok(ComicArchive {
            pages: Self::sort_and_filter(pages),
        })
    }

    async fn load_zip(path: &Path, on_progress: &mut ProgressFn<'_>) -> Result<Vec<(String, Vec<u8>)>> {
        let file = std::fs::File::open(path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        let total = archive.file_names().filter(|n| Self::is_image_file(n)).count();
        let mut images = Vec::new();

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            if Self::is_image_file(&file.name()) {
                let mut data = Vec::new();
                std::io::Read::read_to_end(&mut file, &mut data)?;
                if !Self::looks_like_image(&data) {
                    continue;
                }
                let preview = if images.is_empty() { Self::decode_image(&data, None).ok() } else { None };
                images.push((file.name().to_string(), data));
                on_progress(images.len(), total, preview.as_ref());
            }
        }

        Ok(images)
    }

    async fn load_7z(path: &Path, on_progress: &mut ProgressFn<'_>) -> Result<Vec<(String, Vec<u8>)>> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        let mut archive = sevenz_rust::SevenZReader::new(file, len, sevenz_rust::Password::empty())
            .map_err(|e| anyhow::anyhow!("Erreur lecture 7z: {e}"))?;

        let total = archive
            .archive()
            .files
            .iter()
            .filter(|entry| !entry.is_directory() && Self::is_image_file(entry.name()))
            .count();

        let mut images = Vec::new();
        archive
            .for_each_entries(|entry, reader| {
                if !entry.is_directory() && Self::is_image_file(entry.name()) {
                    let mut data = Vec::new();
                    reader.read_to_end(&mut data)?;
                    if Self::looks_like_image(&data) {
                        let preview = if images.is_empty() { Self::decode_image(&data, None).ok() } else { None };
                        images.push((entry.name().to_string(), data));
                        on_progress(images.len(), total, preview.as_ref());
                    }
                }
                Ok(true)
            })
            .map_err(|e| anyhow::anyhow!("Erreur lecture 7z: {e}"))?;

        Ok(images)
    }

    async fn load_rar(path: &Path, on_progress: &mut ProgressFn<'_>) -> Result<Vec<(String, Vec<u8>)>> {
        let total = unrar::Archive::new(path)
            .open_for_listing()
            .map_err(|e| anyhow::anyhow!("Erreur lecture RAR: {e}"))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| !entry.is_directory() && Self::is_image_file(&entry.filename.to_string_lossy()))
            .count();

        let mut images = Vec::new();

        let archive = unrar::Archive::new(path)
            .open_for_processing()
            .map_err(|e| anyhow::anyhow!("Erreur lecture RAR: {e}"))?;

        let mut cursor = Some(archive);
        while let Some(current) = cursor.take() {
            let Some(file) = current
                .read_header()
                .map_err(|e| anyhow::anyhow!("Erreur lecture RAR: {e}"))?
            else {
                break;
            };

            let name = file.entry().filename.to_string_lossy().into_owned();
            let is_image = !file.entry().is_directory() && Self::is_image_file(&name);

            if is_image {
                let (data, next) = file
                    .read()
                    .map_err(|e| anyhow::anyhow!("Erreur lecture RAR: {e}"))?;
                if Self::looks_like_image(&data) {
                    let preview = if images.is_empty() { Self::decode_image(&data, None).ok() } else { None };
                    images.push((name, data));
                    on_progress(images.len(), total, preview.as_ref());
                }
                cursor = Some(next);
            } else {
                cursor = Some(
                    file.skip()
                        .map_err(|e| anyhow::anyhow!("Erreur lecture RAR: {e}"))?,
                );
            }
        }

        Ok(images)
    }

    fn is_image_file(filename: &str) -> bool {
        let lower = filename.to_lowercase();
        lower.ends_with(".jpg") || lower.ends_with(".jpeg")
            || lower.ends_with(".png") || lower.ends_with(".webp")
            || lower.ends_with(".gif")
    }

    /// Sniffs the magic bytes to confirm an entry is really an image, not
    /// just named like one — archives (especially zips made on macOS) can
    /// carry `__MACOSX/._*` AppleDouble metadata files that share the real
    /// image's extension but aren't image data. Cheap (no pixel decode), so
    /// it's fine to run on every entry even though we no longer decode all
    /// of them up front.
    fn looks_like_image(data: &[u8]) -> bool {
        image::guess_format(data).is_ok()
    }

    /// Decodes a page's compressed bytes into raw pixels. Called on demand,
    /// right before a page's texture is uploaded — not for the whole archive
    /// up front — so decoded (and GPU-uploaded) pages never outnumber the
    /// handful actually near the current spread.
    ///
    /// `max_dimension`, when set, downscales the image so neither side
    /// exceeds it (aspect ratio preserved) — trades a little sharpness on
    /// very high-res scans for a lot less RAM/VRAM per page.
    pub(crate) fn decode_image(data: &[u8], max_dimension: Option<u32>) -> anyhow::Result<egui::ColorImage> {
        let mut img = image::load_from_memory(data)?;

        if let Some(max_dimension) = max_dimension
            && img.width().max(img.height()) > max_dimension
        {
            img = img.resize(max_dimension, max_dimension, image::imageops::FilterType::Triangle);
        }

        let rgba = img.to_rgba8();
        Ok(egui::ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            &rgba.into_raw(),
        ))
    }

    fn sort_and_filter(mut images: Vec<(String, Vec<u8>)>) -> Vec<Vec<u8>> {
        images.sort_by(|a, b| {
            alphanumeric_sort::compare_str(&a.0, &b.0)
        });
        images.into_iter().map(|(_, data)| data).collect()
    }
}

/// A decoded page's representative edge colors — the leftmost and rightmost
/// strip of pixels, each summarized over the full height. Used to fill the
/// gap between two facing pages with a tint that blends into each page's
/// own edge (so an all-white page keeps a white gap, an all-black one a
/// black gap, and two different pages a gradient between them) instead of a
/// flat background color showing through.
#[derive(Clone, Copy, Debug)]
pub struct EdgeColors {
    pub left: egui::Color32,
    pub right: egui::Color32,
}

impl EdgeColors {
    /// Wide enough that the per-channel median below (see `dominant`) has
    /// enough samples to be meaningful, without sampling so far in from the
    /// edge that it stops representing "the edge".
    const SAMPLE_WIDTH: usize = 24;

    pub fn sample(image: &egui::ColorImage) -> Self {
        let [w, h] = image.size;
        if w == 0 || h == 0 {
            return Self { left: egui::Color32::WHITE, right: egui::Color32::WHITE };
        }
        let sample_w = Self::SAMPLE_WIDTH.min(w);
        Self { left: Self::dominant(image, 0..sample_w), right: Self::dominant(image, (w - sample_w)..w) }
    }

    /// Per-channel median over the given columns (full height). A comic
    /// page's outer edge is routinely crossed by a panel's black border
    /// line for part of its height — a plain average gets dragged toward
    /// gray by that minority of dark pixels even when the edge reads as
    /// white overall, while the median only moves once a color actually
    /// covers more than half the sampled strip.
    fn dominant(image: &egui::ColorImage, columns: std::ops::Range<usize>) -> egui::Color32 {
        let [w, h] = image.size;
        let mut reds = Vec::with_capacity(h * columns.len());
        let mut greens = Vec::with_capacity(h * columns.len());
        let mut blues = Vec::with_capacity(h * columns.len());
        for y in 0..h {
            let row = y * w;
            for x in columns.clone() {
                let px = image.pixels[row + x];
                reds.push(px.r());
                greens.push(px.g());
                blues.push(px.b());
            }
        }
        egui::Color32::from_rgb(median(&mut reds), median(&mut greens), median(&mut blues))
    }
}

/// The middle value of `values` once sorted. `values` must be non-empty.
fn median(values: &mut [u8]) -> u8 {
    values.sort_unstable();
    values[values.len() / 2]
}

/// What's known about a decoded page beyond its pixels — computed once,
/// off the UI thread, alongside decoding (see `prefetch::spawn_decode_worker`
/// and `ComicArchive::decode_image`'s callers).
#[derive(Clone, Copy, Debug)]
pub struct PageMeta {
    pub edge: EdgeColors,
    /// True for a page scanned as a single wide image spanning what would
    /// normally be two facing pages (landscape orientation — width clearly
    /// exceeds height, unlike a normal comic page). Shown alone, spanning
    /// the full reader width, instead of being paired with another page.
    pub is_spread: bool,
}

impl PageMeta {
    pub fn sample(image: &egui::ColorImage) -> Self {
        let [w, h] = image.size;
        Self { edge: EdgeColors::sample(image), is_spread: w > h }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_image(width: usize, height: usize, color: egui::Color32) -> egui::ColorImage {
        egui::ColorImage::new([width, height], vec![color; width * height])
    }

    #[test]
    fn portrait_page_is_not_a_spread() {
        let image = solid_image(600, 900, egui::Color32::WHITE);
        assert!(!PageMeta::sample(&image).is_spread);
    }

    #[test]
    fn landscape_page_is_a_spread() {
        let image = solid_image(1800, 900, egui::Color32::WHITE);
        assert!(PageMeta::sample(&image).is_spread);
    }

    #[test]
    fn edge_colors_sample_each_side_independently() {
        let width = 40; // wider than SAMPLE_WIDTH so left/right strips don't overlap
        let mut image = solid_image(width, 20, egui::Color32::WHITE);
        for y in 0..20 {
            image.pixels[y * width] = egui::Color32::BLACK; // one column, left edge only
        }
        let edge = EdgeColors::sample(&image);
        // A thin border line is a small minority of the sampled strip, so
        // the median (unlike a plain average) isn't dragged off white by it.
        assert_eq!(edge.left, egui::Color32::WHITE);
        assert_eq!(edge.right, egui::Color32::WHITE);
    }

    #[test]
    fn edge_color_follows_the_majority_not_a_minority_border() {
        let width = 40;
        let mut image = solid_image(width, 20, egui::Color32::WHITE);
        // A panel's black border covering most (not all) of the page's
        // height at its left edge should register as black, not white.
        for y in 0..14 {
            for x in 0..EdgeColors::SAMPLE_WIDTH {
                image.pixels[y * width + x] = egui::Color32::BLACK;
            }
        }
        let edge = EdgeColors::sample(&image);
        assert_eq!(edge.left, egui::Color32::BLACK);
        assert_eq!(edge.right, egui::Color32::WHITE);
    }
}
