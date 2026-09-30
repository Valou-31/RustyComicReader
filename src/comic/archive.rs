use crate::comic::comic_info::ComicInfo;
use crate::comic::fore_edge;
use anyhow::Result;
use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex};
use std::sync::mpsc;

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
    /// Parsed from the archive's `ComicInfo.xml` entry, if it has one —
    /// `None` when there isn't one, regardless of whether that's because the
    /// book just wasn't tagged or the archive format doesn't carry one.
    pub comic_info: Option<ComicInfo>,
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

/// Cap on the loading-screen preview's longest side (`main.rs` displays it
/// at up to 220x300 anyway) — decoding it at the source resolution risked
/// `egui::Context::load_texture` panicking outright on a very tall page
/// (e.g. a webtoon-format strip several thousand pixels tall), since GPUs
/// commonly cap a texture's side at 8192px.
const PREVIEW_MAX_DIMENSION: u32 = 400;

/// Hard ceiling on either side of a decoded page, independent of
/// `ComicApp::downscale_large_pages` — GPUs (Metal on Apple Silicon
/// included) commonly refuse to create a texture with a side over 8192px,
/// so `decode_image` must never hand back an image taller/wider than this
/// no matter what the user's downscale preference is. Left with headroom
/// under the actual 8192 limit.
const MAX_TEXTURE_SIDE: u32 = 8000;

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
#[cfg(not(target_arch = "wasm32"))]
struct EdgeSamplePool {
    work_tx: mpsc::Sender<(String, Vec<u8>)>,
    result_rx: mpsc::Receiver<(String, EdgeSample)>,
}

#[cfg(not(target_arch = "wasm32"))]
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

/// `wasm32` has no OS threads to sample fore-edges concurrently with
/// extraction (see module docs on `EdgeSamplePool`'s native impl above) —
/// this decodes each sample synchronously the moment it's submitted instead.
/// Same public shape (`spawn`/`submit`/`finish`) so `ComicArchive::load`
/// doesn't need a different code path per target; just blocks the extraction
/// loop a little longer per page instead of overlapping with it.
#[cfg(target_arch = "wasm32")]
struct EdgeSamplePool {
    result_tx: mpsc::Sender<(String, EdgeSample)>,
    result_rx: mpsc::Receiver<(String, EdgeSample)>,
}

#[cfg(target_arch = "wasm32")]
impl EdgeSamplePool {
    fn spawn() -> Self {
        let (result_tx, result_rx) = mpsc::channel();
        Self { result_tx, result_rx }
    }

    fn submit(&self, name: &str, data: &[u8]) {
        if let Ok(image) = ComicArchive::decode_image(data, Some(fore_edge::FORE_EDGE_HEIGHT as u32)) {
            let _ = self.result_tx.send((name.to_string(), EdgeSample::from_image(&image)));
        }
    }

    fn finish(self) -> (HashMap<String, EdgeSample>, mpsc::Receiver<(String, EdgeSample)>) {
        drop(self.result_tx);
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
    ///
    /// Opens `path` itself (native only — the web build has no real
    /// filesystem, see `load_from_bytes`) and dispatches by extension;
    /// `cbz`/`zip`/`cb7`/`7z` go through `load_reader` (generic over any
    /// `Read + Seek`, so the exact same extraction code runs for the web
    /// build's in-memory bytes), `cbr`/`rar` needs a real path (`unrar` has
    /// no wasm32 target) and isn't offered there at all — see
    /// `comic::loader::SUPPORTED_EXTENSIONS`.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn load(
        path: &Path,
        compute_fore_edge: bool,
        mut on_progress: impl FnMut(usize, usize, Option<&egui::ColorImage>),
    ) -> Result<Self> {
        let extension = path.extension().unwrap_or_default().to_str().unwrap_or("").to_lowercase();

        if matches!(extension.as_str(), "cbr" | "rar") {
            let edge_pool = compute_fore_edge.then(EdgeSamplePool::spawn);
            let (pages, comic_info_xml) = Self::load_rar(path, &mut on_progress, edge_pool.as_ref())?;
            return Ok(Self::finish_load(pages, comic_info_xml, edge_pool));
        }

        let file = std::fs::File::open(path)?;
        Self::load_reader(&extension, file, compute_fore_edge, on_progress).await
    }

    /// The web build's entry point — no real filesystem, so the caller
    /// (`comic::loader::spawn_bytes_load`) hands over whatever bytes the
    /// browser's file picker/drag-drop already read into memory, plus the
    /// original filename (used only to dispatch by extension).
    #[cfg(target_arch = "wasm32")]
    pub async fn load_from_bytes(
        filename: &str,
        data: Vec<u8>,
        compute_fore_edge: bool,
        on_progress: impl FnMut(usize, usize, Option<&egui::ColorImage>),
    ) -> Result<Self> {
        let extension = Path::new(filename).extension().unwrap_or_default().to_str().unwrap_or("").to_lowercase();
        Self::load_reader(&extension, std::io::Cursor::new(data), compute_fore_edge, on_progress).await
    }

    /// Shared by both `load` (native, `std::fs::File`) and `load_from_bytes`
    /// (web, `std::io::Cursor<Vec<u8>>`) for every format except RAR (native
    /// only, since `unrar` needs a real file path rather than any reader).
    async fn load_reader<R: Read + Seek>(
        extension: &str,
        reader: R,
        compute_fore_edge: bool,
        mut on_progress: impl FnMut(usize, usize, Option<&egui::ColorImage>),
    ) -> Result<Self> {
        let edge_pool = compute_fore_edge.then(EdgeSamplePool::spawn);

        let (pages, comic_info_xml) = match extension {
            "cbz" | "zip" => Self::load_zip(reader, &mut on_progress, edge_pool.as_ref())?,
            "cb7" | "7z" => Self::load_7z(reader, &mut on_progress, edge_pool.as_ref())?,
            _ => anyhow::bail!("Format non supporté: {extension}"),
        };

        Ok(Self::finish_load(pages, comic_info_xml, edge_pool))
    }

    /// Parses `ComicInfo.xml` (if any), collects whatever fore-edge samples
    /// had already finished, and sorts pages into final reading order —
    /// the tail end shared by every loading path regardless of format or
    /// source.
    fn finish_load(
        pages: Vec<(String, Vec<u8>)>,
        comic_info_xml: Option<Vec<u8>>,
        edge_pool: Option<EdgeSamplePool>,
    ) -> Self {
        let comic_info = comic_info_xml.map(|bytes| ComicInfo::parse(&bytes)).filter(|info| !info.is_empty());

        let (edge_samples, edge_rx) = match edge_pool {
            Some(pool) => {
                let (samples, rx) = pool.finish();
                (samples, Some(rx))
            }
            None => (HashMap::new(), None),
        };
        let (pages, fore_edge_columns, index_by_name) = Self::sort_and_filter(pages, edge_samples, edge_rx.is_some());
        let fore_edge_tail = edge_rx.map(|result_rx| ForeEdgeTail { result_rx, index_by_name });

        ComicArchive { pages, fore_edge_columns, fore_edge_tail, comic_info }
    }

    fn load_zip<R: Read + Seek>(
        reader: R,
        on_progress: &mut ProgressFn<'_>,
        edge_pool: Option<&EdgeSamplePool>,
    ) -> Result<(Vec<(String, Vec<u8>)>, Option<Vec<u8>>)> {
        let mut archive = zip::ZipArchive::new(reader)?;
        let total = archive.file_names().filter(|n| Self::is_image_file(n)).count();
        let mut images = Vec::new();
        let mut comic_info_xml = None;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            if Self::is_image_file(&file.name()) {
                let mut data = Vec::new();
                std::io::Read::read_to_end(&mut file, &mut data)?;
                if !Self::looks_like_image(&data) {
                    continue;
                }
                let preview = if images.is_empty() { Self::decode_image(&data, Some(PREVIEW_MAX_DIMENSION)).ok() } else { None };
                let name = file.name().to_string();
                if let Some(pool) = edge_pool {
                    pool.submit(&name, &data);
                }
                images.push((name, data));
                on_progress(images.len(), total, preview.as_ref());
            } else if comic_info_xml.is_none() && Self::is_comic_info_file(file.name()) {
                let mut data = Vec::new();
                std::io::Read::read_to_end(&mut file, &mut data)?;
                comic_info_xml = Some(data);
            }
        }

        Ok((images, comic_info_xml))
    }

    fn load_7z<R: Read + Seek>(
        mut reader: R,
        on_progress: &mut ProgressFn<'_>,
        edge_pool: Option<&EdgeSamplePool>,
    ) -> Result<(Vec<(String, Vec<u8>)>, Option<Vec<u8>>)> {
        // `SevenZReader::new` wants the stream's length up front rather than
        // asking the reader itself — works the same for a `File` (native) or
        // an in-memory `Cursor<Vec<u8>>` (web), so long as it's rewound
        // afterward (seeking to the end to measure it moves the cursor).
        let len = reader.seek(std::io::SeekFrom::End(0))?;
        reader.seek(std::io::SeekFrom::Start(0))?;
        let mut archive = sevenz_rust::SevenZReader::new(reader, len, sevenz_rust::Password::empty())
            .map_err(|e| anyhow::anyhow!("Erreur lecture 7z: {e}"))?;

        let total = archive
            .archive()
            .files
            .iter()
            .filter(|entry| !entry.is_directory() && Self::is_image_file(entry.name()))
            .count();

        let mut images = Vec::new();
        let mut comic_info_xml: Option<Vec<u8>> = None;
        archive
            .for_each_entries(|entry, reader| {
                if !entry.is_directory() && Self::is_image_file(entry.name()) {
                    let mut data = Vec::new();
                    reader.read_to_end(&mut data)?;
                    if Self::looks_like_image(&data) {
                        let preview = if images.is_empty() { Self::decode_image(&data, Some(PREVIEW_MAX_DIMENSION)).ok() } else { None };
                        let name = entry.name().to_string();
                        if let Some(pool) = edge_pool {
                            pool.submit(&name, &data);
                        }
                        images.push((name, data));
                        on_progress(images.len(), total, preview.as_ref());
                    }
                } else if !entry.is_directory() && comic_info_xml.is_none() && Self::is_comic_info_file(entry.name()) {
                    let mut data = Vec::new();
                    reader.read_to_end(&mut data)?;
                    comic_info_xml = Some(data);
                }
                Ok(true)
            })
            .map_err(|e| anyhow::anyhow!("Erreur lecture 7z: {e}"))?;

        Ok((images, comic_info_xml))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn load_rar(
        path: &Path,
        on_progress: &mut ProgressFn<'_>,
        edge_pool: Option<&EdgeSamplePool>,
    ) -> Result<(Vec<(String, Vec<u8>)>, Option<Vec<u8>>)> {
        let total = unrar::Archive::new(path)
            .open_for_listing()
            .map_err(|e| anyhow::anyhow!("Erreur lecture RAR: {e}"))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| !entry.is_directory() && Self::is_image_file(&entry.filename.to_string_lossy()))
            .count();

        let mut images = Vec::new();
        let mut comic_info_xml: Option<Vec<u8>> = None;

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
            let is_comic_info = !file.entry().is_directory() && comic_info_xml.is_none() && Self::is_comic_info_file(&name);

            if is_image {
                let (data, next) = file
                    .read()
                    .map_err(|e| anyhow::anyhow!("Erreur lecture RAR: {e}"))?;
                if Self::looks_like_image(&data) {
                    let preview = if images.is_empty() { Self::decode_image(&data, Some(PREVIEW_MAX_DIMENSION)).ok() } else { None };
                    if let Some(pool) = edge_pool {
                        pool.submit(&name, &data);
                    }
                    images.push((name, data));
                    on_progress(images.len(), total, preview.as_ref());
                }
                cursor = Some(next);
            } else if is_comic_info {
                let (data, next) = file
                    .read()
                    .map_err(|e| anyhow::anyhow!("Erreur lecture RAR: {e}"))?;
                comic_info_xml = Some(data);
                cursor = Some(next);
            } else {
                cursor = Some(
                    file.skip()
                        .map_err(|e| anyhow::anyhow!("Erreur lecture RAR: {e}"))?,
                );
            }
        }

        Ok((images, comic_info_xml))
    }

    fn is_image_file(filename: &str) -> bool {
        let lower = filename.to_lowercase();
        lower.ends_with(".jpg") || lower.ends_with(".jpeg")
            || lower.ends_with(".png") || lower.ends_with(".webp")
            || lower.ends_with(".gif")
    }

    /// Matches `ComicInfo.xml` by its base filename alone (case-insensitive),
    /// regardless of which directory inside the archive it sits in — taggers
    /// disagree on whether it belongs at the root or alongside the pages.
    fn is_comic_info_file(filename: &str) -> bool {
        Path::new(filename)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("ComicInfo.xml"))
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

    /// Decodes a page's compressed bytes into raw pixels, downscaled (aspect
    /// ratio preserved, uniform box) so neither side exceeds `max_dimension`
    /// when set. Used for thumbnails, fore-edge sampling and the
    /// loading-screen preview — all of which only ever need a genuinely
    /// small image, regardless of the source page's own aspect ratio, so
    /// forcing *both* axes down to `max_dimension` is exactly what they
    /// want. For a page that's actually about to be read at close to native
    /// size, see `decode_page_image` instead.
    pub(crate) fn decode_image(data: &[u8], max_dimension: Option<u32>) -> anyhow::Result<egui::ColorImage> {
        let mut img = image::load_from_memory(data)?;

        if let Some(max_dimension) = max_dimension
            && img.width().max(img.height()) > max_dimension
        {
            img = img.resize(max_dimension, max_dimension, image::imageops::FilterType::Triangle);
        }

        Self::to_color_image(img)
    }

    /// Decodes a page meant to actually be displayed at close to native
    /// width — the page currently on screen in the paginated view or the
    /// Webtoon strip. Called on demand, right before a page's texture is
    /// uploaded — not for the whole archive up front — so decoded (and
    /// GPU-uploaded) pages never outnumber the handful actually near the
    /// current spread.
    ///
    /// Unlike `decode_image`'s uniform box, width and height are capped
    /// independently: `max_dimension` (the "Downscale large pages"
    /// preference, when set) bounds width, and `MAX_TEXTURE_SIDE` bounds
    /// height unconditionally, regardless of that preference. A single
    /// "longest side" box would be wrong here: some webtoon releases ship
    /// an entire chapter as one already-narrow strip many thousands of
    /// pixels tall (e.g. `1654x34859`) — running *that* through a box sized
    /// for a typical page would treat its height as the "longest side" and
    /// crush its already-modest width down to a sliver (~113px, in that
    /// example) to match, for no memory benefit the width ever needed.
    /// Capping each axis independently keeps a normal page's behavior
    /// unchanged (width is almost always its binding dimension) while
    /// leaving a tall strip's native width alone and only trimming its
    /// height down to whatever the GPU will actually accept — capped
    /// unconditionally, since with downscaling turned off entirely
    /// (`max_dimension: None`) an oversized page must still never reach the
    /// GPU at a height it will refuse to allocate.
    pub(crate) fn decode_page_image(data: &[u8], max_dimension: Option<u32>) -> anyhow::Result<egui::ColorImage> {
        let mut img = image::load_from_memory(data)?;

        let width_cap = max_dimension.unwrap_or(MAX_TEXTURE_SIDE).min(MAX_TEXTURE_SIDE);
        let scale = (width_cap as f32 / img.width() as f32)
            .min(MAX_TEXTURE_SIDE as f32 / img.height() as f32)
            .min(1.0);
        if scale < 1.0 {
            let new_width = ((img.width() as f32 * scale).round() as u32).max(1);
            let new_height = ((img.height() as f32 * scale).round() as u32).max(1);
            img = img.resize_exact(new_width, new_height, image::imageops::FilterType::Triangle);
        }

        Self::to_color_image(img)
    }

    /// Decodes a page for the Webtoon strip specifically, split into one or
    /// more vertical slices so a page never has to trade away resolution
    /// just to fit inside one GPU texture the way `decode_page_image` does.
    /// `max_dimension` still caps *width* the same way (the "Downscale
    /// large pages" preference) — width almost never needs more than one
    /// texture's worth of pixels, so there's no reason to slice it, only
    /// height. But once the page (after any width-driven downscale) is
    /// still taller than `MAX_TEXTURE_SIDE`, instead of squeezing it down
    /// to fit — as `decode_page_image` must, since it only ever hands back
    /// one texture — it's cropped into consecutive horizontal bands, each
    /// its own texture at full (post-downscale) resolution. `ui::reader::
    /// draw_webtoon` stacks them back-to-back so they read as one seamless
    /// page again.
    ///
    /// Returns the slices top-to-bottom, plus the whole page's aspect ratio
    /// (width/height, after any width downscale — unaffected by slicing
    /// itself) for `ui::reader`'s layout math, same role `PageMeta::aspect`
    /// plays for `decode_page_image`.
    pub(crate) fn decode_page_slices(data: &[u8], max_dimension: Option<u32>) -> anyhow::Result<(Vec<egui::ColorImage>, f32)> {
        let mut img = image::load_from_memory(data)?;

        let width_cap = max_dimension.unwrap_or(MAX_TEXTURE_SIDE).min(MAX_TEXTURE_SIDE);
        if img.width() > width_cap {
            let scale = width_cap as f32 / img.width() as f32;
            let new_width = width_cap.max(1);
            let new_height = ((img.height() as f32 * scale).round() as u32).max(1);
            img = img.resize_exact(new_width, new_height, image::imageops::FilterType::Triangle);
        }

        let aspect = img.width() as f32 / img.height().max(1) as f32;

        let total_height = img.height();
        let mut slices = Vec::with_capacity((total_height / MAX_TEXTURE_SIDE + 1) as usize);
        let mut y = 0u32;
        while y < total_height {
            let slice_height = (total_height - y).min(MAX_TEXTURE_SIDE);
            let slice = img.crop_imm(0, y, img.width(), slice_height);
            slices.push(Self::to_color_image(slice)?);
            y += slice_height;
        }

        Ok((slices, aspect))
    }

    /// A page's aspect ratio (width/height), read from just the image's
    /// header — orders of magnitude cheaper than a full decode
    /// (`decode_page_slices`/`decode_image`), since it only needs to parse
    /// metadata, not decode any pixels (measured ~1ms even for a
    /// many-thousand-pixel-tall webtoon strip, vs. several hundred ms for a
    /// full decode of the same page). Uniform downscaling — the only kind
    /// `decode_page_slices` ever does to a whole page, see its own docs —
    /// doesn't change an aspect ratio, so this is exactly the value that
    /// decode will eventually produce too; no need to duplicate its
    /// `max_dimension` handling here.
    ///
    /// Used to populate `ComicApp::webtoon_aspect` for the *whole* book
    /// right when it finishes loading, rather than only as each page's full
    /// decode happens to complete — `ui::reader::draw_webtoon`'s layout
    /// math (`total_doc_height`, and so `max_scroll`) depends on knowing
    /// every page's real proportions, not `WEBTOON_DEFAULT_ASPECT`'s guess.
    /// Guessing wrong for a page still waiting on its full decode doesn't
    /// just cause a visible jump once it resolves (already handled — see
    /// `ComicApp::webtoon_anchor_page`'s docs) — for the *last* page
    /// specifically, an underestimated `total_doc_height` means an
    /// underestimated `max_scroll`, which clamps scrolling short of the
    /// book's real end until that page's full decode happens to catch up,
    /// which can read as the rest of the page having gone missing if the
    /// reader stops scrolling before it does.
    pub fn page_dimensions(data: &[u8]) -> anyhow::Result<(u32, u32)> {
        let dims = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format()?.into_dimensions()?;
        Ok(dims)
    }

    fn to_color_image(img: image::DynamicImage) -> anyhow::Result<egui::ColorImage> {
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
    use crate::app::DOWNSCALE_MAX_DIMENSION;

    fn solid_image(width: usize, height: usize, color: egui::Color32) -> egui::ColorImage {
        egui::ColorImage::new([width, height], vec![color; width * height])
    }

    /// PNG-encodes a solid-color image of the given native size — the raw
    /// compressed bytes `decode_image`/`decode_page_image`/
    /// `decode_page_slices` actually take, as opposed to `solid_image`'s
    /// already-decoded pixels.
    fn encode_solid_png(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(width, height, image::Rgba([200, 100, 50, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
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

    #[test]
    fn decode_page_slices_of_a_normal_page_is_a_single_untouched_slice() {
        let data = encode_solid_png(200, 300);
        let (slices, aspect) = ComicArchive::decode_page_slices(&data, Some(DOWNSCALE_MAX_DIMENSION)).unwrap();

        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0].size, [200, 300]);
        assert!((aspect - 200.0 / 300.0).abs() < 0.001);
    }

    #[test]
    fn decode_page_slices_splits_a_tall_strip_without_shrinking_its_width() {
        // Mirrors a real webtoon chapter shipped as one merged strip: narrow
        // enough that `decode_page_image`'s uniform "longest side" downscale
        // would crush its width down to a sliver trying to also cap its
        // height. Slicing should leave the width alone entirely (it was
        // already under the cap) and only split the height.
        let data = encode_solid_png(1000, 20000);
        let (slices, aspect) = ComicArchive::decode_page_slices(&data, Some(DOWNSCALE_MAX_DIMENSION)).unwrap();

        // ceil(20000 / MAX_TEXTURE_SIDE) slices, none over the GPU limit,
        // stacking back up to the original height.
        let expected_slice_count = (20000u32).div_ceil(MAX_TEXTURE_SIDE) as usize;
        assert_eq!(slices.len(), expected_slice_count);
        for slice in &slices {
            assert_eq!(slice.size[0], 1000); // width untouched — under DOWNSCALE_MAX_DIMENSION already
            assert!(slice.size[1] as u32 <= MAX_TEXTURE_SIDE);
        }
        let total_height: usize = slices.iter().map(|s| s.size[1]).sum();
        assert_eq!(total_height, 20000);
        assert!((aspect - 1000.0 / 20000.0).abs() < 0.0001);
    }

    #[test]
    fn decode_page_slices_still_caps_width_when_downscaling_is_on() {
        let data = encode_solid_png(6000, 4000);
        let (slices, aspect) = ComicArchive::decode_page_slices(&data, Some(DOWNSCALE_MAX_DIMENSION)).unwrap();

        assert_eq!(slices.len(), 1); // well under MAX_TEXTURE_SIDE after width downscale
        assert_eq!(slices[0].size[0], DOWNSCALE_MAX_DIMENSION as usize);
        // Aspect ratio preserved by the uniform width-driven downscale.
        assert!((aspect - 6000.0 / 4000.0).abs() < 0.01);
    }

    #[test]
    fn decode_page_slices_never_exceeds_the_gpu_limit_even_with_downscaling_off() {
        let data = encode_solid_png(500, 10000);
        let (slices, _) = ComicArchive::decode_page_slices(&data, None).unwrap();

        for slice in &slices {
            assert!(slice.size[0] as u32 <= MAX_TEXTURE_SIDE);
            assert!(slice.size[1] as u32 <= MAX_TEXTURE_SIDE);
        }
        let total_height: usize = slices.iter().map(|s| s.size[1]).sum();
        assert_eq!(total_height, 10000); // no downscale needed, just sliced
    }

    #[test]
    fn page_dimensions_reads_a_normal_pages_size() {
        let data = encode_solid_png(600, 900);
        assert_eq!(ComicArchive::page_dimensions(&data).unwrap(), (600, 900));
    }

    #[test]
    fn page_dimensions_matches_decode_page_slices_own_aspect_for_an_extreme_page() {
        // The whole point of `page_dimensions` existing: its aspect ratio
        // must agree with what `decode_page_slices` eventually computes
        // (via a full decode) for the exact same page, including for the
        // extreme aspect ratios a merged webtoon-chapter strip has —
        // otherwise `ui::reader::draw_webtoon`'s layout would just jump
        // from one wrong value to another instead of settling once the
        // full decode lands.
        let data = encode_solid_png(1000, 30000);
        let (native_w, native_h) = ComicArchive::page_dimensions(&data).unwrap();
        let header_aspect = native_w as f32 / native_h as f32;

        let (_, decoded_aspect) = ComicArchive::decode_page_slices(&data, Some(DOWNSCALE_MAX_DIMENSION)).unwrap();

        assert!((header_aspect - decoded_aspect).abs() < 0.0001);
    }
}
