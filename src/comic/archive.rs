use crate::comic::fore_edge;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};

/// An opened comic archive. Pages are kept as their original *compressed*
/// bytes (a few hundred KB each, typically) rather than decoded pixels —
/// decoding every page up front would mean holding the whole book as raw
/// RGBA in memory (tens of MB per page) even though only a couple of pages
/// are ever on screen at once. Callers decode a page (via `decode_image`)
/// only when they're about to display it.
pub struct ComicArchive {
    pub pages: Vec<Vec<u8>>,
    /// Every page's fore-edge strip (see `comic::fore_edge`) that had
    /// already finished sampling by the time `load` returned — resolved to
    /// the correct side, in final (sorted) page order, transparent
    /// placeholders standing in for whatever hadn't finished yet. Empty if
    /// `load` was called with `compute_fore_edge: false`.
    pub fore_edge_columns: Vec<Vec<egui::Color32>>,
    /// Whatever fore-edge sampling was still in flight when `load` returned
    /// — `load` doesn't block on the full book finishing (that would delay
    /// the archive being usable at all by however long the slowest pages
    /// take), so on a longer book this is typically non-empty even though
    /// sampling started back when extraction did. The caller keeps polling
    /// it (see `ForeEdgeTail::poll`) to fill in the rest of
    /// `fore_edge_columns`'s placeholders as they land.
    pub fore_edge_tail: Option<ForeEdgeTail>,
}

/// `sort_and_filter`'s result: pages in final order, their fore-edge
/// columns (see `ComicArchive::fore_edge_columns`), and the name→final-index
/// map a `ForeEdgeTail` needs to resolve pages sampled after the fact.
type SortedPagesWithForeEdge = (Vec<Vec<u8>>, Vec<Vec<egui::Color32>>, HashMap<String, usize>);

/// Called after each page is extracted, in archive order (not final reading
/// order — that's only known once every entry has been read and sorted).
/// Carries a decoded preview image only for the very first entry (so the UI
/// has something to show while the rest of the archive is still extracting);
/// `None` for every entry after that.
type ProgressFn<'a> = dyn FnMut(usize, usize, Option<&egui::ColorImage>) + 'a;

/// One page's fore-edge sample, decoded once but keeping *both* possible
/// sides (see `EdgeSamplePool`) since which one is wanted — the rule is by
/// final, sorted page number (`comic::fore_edge::edge_on_right`) — isn't
/// known until every archive entry has been read.
struct EdgeSample {
    left: Vec<egui::Color32>,
    right: Vec<egui::Color32>,
}

impl EdgeSample {
    fn from_image(image: &egui::ColorImage) -> Self {
        Self { left: fore_edge::sample_side(image, false), right: fore_edge::sample_side(image, true) }
    }
}

/// A small pool of worker threads that sample each page's fore-edge strip
/// (see `EdgeSample`) as its bytes come off the archive, concurrently with
/// the (I/O-bound, single-threaded) extraction loop still reading further
/// entries — so this CPU-bound decode work overlaps the archive load
/// instead of only starting once it's entirely done. `submit` is cheap
/// (just queues a clone of the page's bytes) and never blocks extraction on
/// decode; `finish` hands back whatever's already arrived plus a
/// `ForeEdgeTail` for the rest — it does *not* wait for outstanding work,
/// since blocking `load` until the slowest page finishes decoding would
/// delay the archive becoming usable at all by however long that takes
/// (measured on a real 259-page volume: roughly 1s to extract, but 2s more
/// for every page's sample to finish decoding — `load` shouldn't make
/// opening the book wait on the second number).
struct EdgeSamplePool {
    work_tx: mpsc::Sender<(String, Vec<u8>)>,
    result_rx: mpsc::Receiver<(String, EdgeSample)>,
}

impl EdgeSamplePool {
    fn spawn() -> Self {
        let (work_tx, work_rx) = mpsc::channel::<(String, Vec<u8>)>();
        let work_rx = Arc::new(Mutex::new(work_rx));
        let (result_tx, result_rx) = mpsc::channel();

        let worker_count = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
        for _ in 0..worker_count {
            let work_rx = Arc::clone(&work_rx);
            let result_tx = result_tx.clone();
            // Not kept as a `JoinHandle`: these run to completion on their
            // own (their queue is finite — `finish` closes it below — so
            // they can't run forever), sending results into `result_rx`
            // for as long as anyone's still receiving. If a newer book
            // supersedes this one before that finishes, the receiving end
            // (`ForeEdgeTail`) is simply dropped and these harmlessly fail
            // to send the rest and exit once their queue drains.
            std::thread::spawn(move || {
                loop {
                    let job = work_rx.lock().unwrap().recv();
                    let Ok((name, data)) = job else { break };
                    if let Ok(image) = ComicArchive::decode_image(&data, Some(fore_edge::FORE_EDGE_HEIGHT as u32)) {
                        let _ = result_tx.send((name, EdgeSample::from_image(&image)));
                    }
                }
            });
        }

        Self { work_tx, result_rx }
    }

    /// Queues `name`/`data` for background sampling — never blocks.
    fn submit(&self, name: &str, data: &[u8]) {
        let _ = self.work_tx.send((name.to_string(), data.to_vec()));
    }

    /// Closes the work queue (so workers exit once it drains) and does one
    /// *non-blocking* sweep of whatever samples have already arrived —
    /// typically a good chunk of the book, since these workers had a head
    /// start throughout extraction. Returns those plus a `ForeEdgeTail` for
    /// the rest, still being decoded by the now-detached workers.
    fn finish(self) -> (HashMap<String, EdgeSample>, mpsc::Receiver<(String, EdgeSample)>) {
        drop(self.work_tx);
        let mut samples = HashMap::new();
        while let Ok((name, sample)) = self.result_rx.try_recv() {
            samples.insert(name, sample);
        }
        (samples, self.result_rx)
    }
}

/// Fore-edge sampling still in flight when `ComicArchive::load` returned —
/// see `EdgeSamplePool::finish`. The caller (`ComicApp::poll_fore_edge`)
/// polls this once per frame until it reports it's done, applying each
/// `(page index, columns)` pair to fill in the corresponding still-blank
/// slot of the fore-edge composite texture.
pub struct ForeEdgeTail {
    result_rx: mpsc::Receiver<(String, EdgeSample)>,
    /// Final (sorted) page index for every name the tail might still hear
    /// about — built once, alongside the sort itself, so resolving a late
    /// arrival to its column position is just a lookup.
    index_by_name: HashMap<String, usize>,
}

impl ForeEdgeTail {
    /// Non-blocking: every `(page index, columns)` pair that's landed since
    /// this was last called, resolved to the side (`comic::fore_edge::
    /// edge_on_right`) that index actually wants — ready to write straight
    /// into the composite — plus whether every worker has now exited and
    /// there's nothing left to ever report (in which case the caller can
    /// drop this). Both come from the one drain so a message can't be lost
    /// between a separate "any more?" check and the next `poll`.
    pub fn poll(&mut self) -> (Vec<(usize, Vec<egui::Color32>)>, bool) {
        let mut updates = Vec::new();
        loop {
            match self.result_rx.try_recv() {
                Ok((name, sample)) => {
                    if let Some(&page_idx) = self.index_by_name.get(&name) {
                        updates.push((page_idx, if fore_edge::edge_on_right(page_idx) { sample.right } else { sample.left }));
                    }
                }
                Err(mpsc::TryRecvError::Empty) => return (updates, false),
                Err(mpsc::TryRecvError::Disconnected) => return (updates, true),
            }
        }
    }
}

impl ComicArchive {
    /// `compute_fore_edge` gates the `EdgeSamplePool` above entirely — when
    /// off (the user's "book thickness" setting is disabled), no extra
    /// decoding happens at all and `fore_edge_columns` comes back empty.
    pub async fn load(
        path: &Path,
        compute_fore_edge: bool,
        mut on_progress: impl FnMut(usize, usize, Option<&egui::ColorImage>),
    ) -> Result<Self> {
        let extension = path.extension().unwrap_or_default().to_str().unwrap_or("").to_lowercase();

        let edge_pool = compute_fore_edge.then(EdgeSamplePool::spawn);

        let pages = match extension.as_str() {
            "cbz" | "zip" => Self::load_zip(path, &mut on_progress, edge_pool.as_ref()).await?,
            "cb7" | "7z" => Self::load_7z(path, &mut on_progress, edge_pool.as_ref()).await?,
            "cbr" | "rar" => Self::load_rar(path, &mut on_progress, edge_pool.as_ref()).await?,
            _ => anyhow::bail!("Format non supporté: {}", extension),
        };

        let (edge_samples, edge_rx) = match edge_pool {
            Some(pool) => {
                let (samples, rx) = pool.finish();
                (samples, Some(rx))
            }
            None => (HashMap::new(), None),
        };
        let (pages, fore_edge_columns, index_by_name) = Self::sort_and_filter(pages, edge_samples, edge_rx.is_some());
        let fore_edge_tail = edge_rx.map(|result_rx| ForeEdgeTail { result_rx, index_by_name });

        Ok(ComicArchive { pages, fore_edge_columns, fore_edge_tail })
    }

    async fn load_zip(path: &Path, on_progress: &mut ProgressFn<'_>, edge_pool: Option<&EdgeSamplePool>) -> Result<Vec<(String, Vec<u8>)>> {
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
                let name = file.name().to_string();
                if let Some(pool) = edge_pool {
                    pool.submit(&name, &data);
                }
                images.push((name, data));
                on_progress(images.len(), total, preview.as_ref());
            }
        }

        Ok(images)
    }

    async fn load_7z(path: &Path, on_progress: &mut ProgressFn<'_>, edge_pool: Option<&EdgeSamplePool>) -> Result<Vec<(String, Vec<u8>)>> {
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
                        let name = entry.name().to_string();
                        if let Some(pool) = edge_pool {
                            pool.submit(&name, &data);
                        }
                        images.push((name, data));
                        on_progress(images.len(), total, preview.as_ref());
                    }
                }
                Ok(true)
            })
            .map_err(|e| anyhow::anyhow!("Erreur lecture 7z: {e}"))?;

        Ok(images)
    }

    async fn load_rar(path: &Path, on_progress: &mut ProgressFn<'_>, edge_pool: Option<&EdgeSamplePool>) -> Result<Vec<(String, Vec<u8>)>> {
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
                    if let Some(pool) = edge_pool {
                        pool.submit(&name, &data);
                    }
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

    /// Sorts into final reading order and, alongside, resolves each page's
    /// fore-edge sample (already decoded — see `EdgeSamplePool`) to the side
    /// (`comic::fore_edge::edge_on_right`) its *final* index actually calls
    /// for. A page missing from `edge_samples` (its own sample never
    /// finished, or `compute_fore_edge` was off) falls back to a transparent
    /// strip, keeping `fore_edge_columns` the same length/shape as `pages`
    /// either way — except when `edge_samples` is empty, where it's skipped
    /// entirely (nothing was asked for) and the result is an empty `Vec`.
    /// Sorts into final reading order and, alongside, resolves each page's
    /// already-finished fore-edge sample (if any — see `EdgeSamplePool`) to
    /// the side its *final* index actually calls for. `compute_fore_edge`
    /// (not merely whether `edge_samples` happens to be non-empty — nothing
    /// may have finished decoding yet even when the feature is on, if
    /// extraction was fast enough to outrun the sampling pool) says whether
    /// to build `fore_edge_columns`/`index_by_name` at all; when it's off
    /// both come back empty. A page missing from `edge_samples` falls back
    /// to a transparent strip, filled in later if its sample turns up
    /// through the returned name→index map (see `ForeEdgeTail`).
    fn sort_and_filter(
        mut images: Vec<(String, Vec<u8>)>,
        edge_samples: HashMap<String, EdgeSample>,
        compute_fore_edge: bool,
    ) -> SortedPagesWithForeEdge {
        images.sort_by(|a, b| alphanumeric_sort::compare_str(&a.0, &b.0));

        if !compute_fore_edge {
            return (images.into_iter().map(|(_, data)| data).collect(), Vec::new(), HashMap::new());
        }

        let transparent = || vec![egui::Color32::TRANSPARENT; fore_edge::EDGE_SAMPLE_WIDTH * fore_edge::FORE_EDGE_HEIGHT];
        let mut pages = Vec::with_capacity(images.len());
        let mut fore_edge_columns = Vec::with_capacity(images.len());
        let mut index_by_name = HashMap::with_capacity(images.len());
        for (page_idx, (name, data)) in images.into_iter().enumerate() {
            let columns = edge_samples
                .get(&name)
                .map(|sample| if fore_edge::edge_on_right(page_idx) { sample.right.clone() } else { sample.left.clone() })
                .unwrap_or_else(transparent);
            fore_edge_columns.push(columns);
            index_by_name.insert(name, page_idx);
            pages.push(data);
        }
        (pages, fore_edge_columns, index_by_name)
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

    #[test]
    fn sort_and_filter_resolves_each_page_to_the_side_its_final_index_calls_for() {
        let images = vec![
            ("b.jpg".to_string(), vec![2u8]),
            ("a.jpg".to_string(), vec![1u8]),
        ];
        let mut edge_samples = HashMap::new();
        edge_samples.insert(
            "a.jpg".to_string(),
            EdgeSample { left: vec![egui::Color32::BLACK; 2], right: vec![egui::Color32::WHITE; 2] },
        );
        edge_samples.insert(
            "b.jpg".to_string(),
            EdgeSample { left: vec![egui::Color32::RED; 2], right: vec![egui::Color32::BLUE; 2] },
        );

        let (pages, columns, index_by_name) = ComicArchive::sort_and_filter(images, edge_samples, true);

        // Sorted order is a.jpg (page_idx 0, odd displayed page -> left
        // edge) then b.jpg (page_idx 1, even displayed page -> right edge).
        assert_eq!(pages, vec![vec![1u8], vec![2u8]]);
        assert_eq!(columns[0], vec![egui::Color32::BLACK; 2]);
        assert_eq!(columns[1], vec![egui::Color32::BLUE; 2]);
        assert_eq!(index_by_name.get("a.jpg"), Some(&0));
        assert_eq!(index_by_name.get("b.jpg"), Some(&1));
    }

    #[test]
    fn a_page_missing_from_edge_samples_falls_back_to_a_transparent_strip() {
        let images = vec![("a.jpg".to_string(), vec![1u8]), ("b.jpg".to_string(), vec![2u8])];
        let mut edge_samples = HashMap::new();
        edge_samples.insert("a.jpg".to_string(), EdgeSample { left: vec![egui::Color32::BLACK; 2], right: vec![egui::Color32::WHITE; 2] });
        // "b.jpg" hadn't finished decoding by the time `load` returned —
        // still resolvable later through `index_by_name` via `ForeEdgeTail`.

        let (_, columns, index_by_name) = ComicArchive::sort_and_filter(images, edge_samples, true);

        assert_eq!(columns[1], vec![egui::Color32::TRANSPARENT; fore_edge::EDGE_SAMPLE_WIDTH * fore_edge::FORE_EDGE_HEIGHT]);
        assert_eq!(index_by_name.get("b.jpg"), Some(&1));
    }

    #[test]
    fn fore_edge_disabled_produces_empty_columns_and_index() {
        let images = vec![("a.jpg".to_string(), vec![1u8])];
        let (pages, columns, index_by_name) = ComicArchive::sort_and_filter(images, HashMap::new(), false);
        assert_eq!(pages, vec![vec![1u8]]);
        assert!(columns.is_empty());
        assert!(index_by_name.is_empty());
    }
}
