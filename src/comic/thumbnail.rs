use crate::comic::archive::ComicArchive;
#[cfg(not(target_arch = "wasm32"))]
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, channel};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Condvar;
use std::sync::{Arc, Mutex};

/// Longest side, in pixels, of the cheap whole-book preview `ComicApp::
/// warm_thumbnail_cache` decodes for every page up front — small enough
/// that holding one per page for the entire book, for the whole session,
/// stays a modest amount of memory even for a very long book.
pub const LOW_RES_MAX_DIMENSION: u32 = 120;
/// Longest side, in pixels, of the sharp preview decoded on demand for
/// whichever page is actually under the cursor (see `ComicApp::
/// request_thumbnail`) — sized to look good at `ui::progress_bar`'s display
/// size. Only ever held for a small, capped set of recently hovered pages,
/// so it's fine for this to cost much more per page than the low-res tier.
pub const HIGH_RES_MAX_DIMENSION: u32 = 400;

/// Which of the two preview qualities a request/result is for — see the
/// module docs on `LOW_RES_MAX_DIMENSION`/`HIGH_RES_MAX_DIMENSION`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThumbnailTier {
    Low,
    High,
}

/// A page's bytes waiting to be decoded into a thumbnail off the UI thread.
/// `generation` identifies which loaded archive this belongs to, so a
/// result that lands after the user has since opened a different book can
/// be told apart from a page that merely shares the same index in the new
/// one (see `comic::prefetch::DecodeRequest`, same idea).
pub struct ThumbnailRequest {
    pub generation: u64,
    pub page_idx: usize,
    pub tier: ThumbnailTier,
    pub max_dimension: u32,
    pub data: Vec<u8>,
}

pub struct DecodedThumbnail {
    pub generation: u64,
    pub page_idx: usize,
    pub tier: ThumbnailTier,
    pub image: egui::ColorImage,
}

#[cfg(not(target_arch = "wasm32"))]
struct Shared {
    queue: Mutex<VecDeque<ThumbnailRequest>>,
    work_available: Condvar,
}

/// Handle to the background thumbnail worker's request queue.
///
/// Two ways in: `prioritize` jumps a single page to the very front — used
/// for whatever's directly under the cursor right now, which should never
/// wait behind anything else — and `extend_low_priority` appends a batch to
/// the back — used for `ComicApp`'s whole-book background preload, so it
/// fills in around the edges without ever delaying a page someone's
/// actually looking at. A page can be queued at both tiers at once (they're
/// different requests, matched on `(page_idx, tier)`, not just `page_idx`).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct ThumbnailQueue {
    shared: Arc<Shared>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ThumbnailQueue {
    /// Moves `request` to the front of the queue, dropping any existing
    /// queued entry for the same page *and tier* first so it isn't left
    /// duplicated further back.
    pub fn prioritize(&self, request: ThumbnailRequest) {
        let mut queue = self.shared.queue.lock().unwrap();
        queue.retain(|r| (r.page_idx, r.tier) != (request.page_idx, request.tier));
        queue.push_front(request);
        drop(queue);
        self.shared.work_available.notify_one();
    }

    /// Appends `requests` to the back of the queue, skipping any page/tier
    /// pair that's already queued (at any position).
    pub fn extend_low_priority(&self, requests: Vec<ThumbnailRequest>) {
        if requests.is_empty() {
            return;
        }
        let mut queue = self.shared.queue.lock().unwrap();
        for request in requests {
            if !queue.iter().any(|r| (r.page_idx, r.tier) == (request.page_idx, request.tier)) {
                queue.push_back(request);
            }
        }
        drop(queue);
        self.shared.work_available.notify_one();
    }

    /// Drops every not-yet-started request — called when a book is closed,
    /// so the worker doesn't keep grinding through a backlog for an archive
    /// that's no longer open.
    pub fn clear(&self) {
        self.shared.queue.lock().unwrap().clear();
    }
}

/// Spawns a single long-lived worker that decodes hover-preview thumbnails
/// off the UI thread — so showing a preview never blocks a frame on
/// decoding a full-resolution scanned page, which is what made the
/// original synchronous-on-hover version feel sluggish.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_thumbnail_worker() -> (ThumbnailQueue, Receiver<DecodedThumbnail>) {
    let shared = Arc::new(Shared { queue: Mutex::new(VecDeque::new()), work_available: Condvar::new() });
    let worker_shared = Arc::clone(&shared);
    let (result_tx, result_rx) = channel::<DecodedThumbnail>();

    std::thread::spawn(move || {
        loop {
            let mut queue = worker_shared.queue.lock().unwrap();
            while queue.is_empty() {
                queue = worker_shared.work_available.wait(queue).unwrap();
            }
            let request = queue.pop_front().unwrap();
            drop(queue);

            if let Ok(image) = ComicArchive::decode_image(&request.data, Some(request.max_dimension)) {
                let result = DecodedThumbnail {
                    generation: request.generation,
                    page_idx: request.page_idx,
                    tier: request.tier,
                    image,
                };
                if result_tx.send(result).is_err() {
                    break;
                }
            }
        }
    });

    (ThumbnailQueue { shared }, result_rx)
}

/// The web build's counterpart to the native worker above — see
/// `comic::prefetch`'s wasm32 twin for why (no OS threads on `wasm32`) and
/// its dedup approach (`done`, keyed here on `(page_idx, tier)` since a page
/// can be queued at both quality tiers independently). Both `prioritize` and
/// `extend_low_priority` decode synchronously, inline, rather than actually
/// queueing — a whole-book `extend_low_priority` preload call will therefore
/// block the tab for however long the whole book takes to decode at
/// low-res, a known v1 trade-off (see `comic::prefetch`'s docs).
#[cfg(target_arch = "wasm32")]
struct Shared {
    done: Mutex<std::collections::HashSet<(usize, ThumbnailTier)>>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
pub struct ThumbnailQueue {
    shared: Arc<Shared>,
    result_tx: std::sync::mpsc::Sender<DecodedThumbnail>,
}

#[cfg(target_arch = "wasm32")]
impl ThumbnailQueue {
    fn decode_and_send(&self, request: ThumbnailRequest, done: &mut std::collections::HashSet<(usize, ThumbnailTier)>) {
        if !done.insert((request.page_idx, request.tier)) {
            return;
        }
        if let Ok(image) = ComicArchive::decode_image(&request.data, Some(request.max_dimension)) {
            let result = DecodedThumbnail { generation: request.generation, page_idx: request.page_idx, tier: request.tier, image };
            let _ = self.result_tx.send(result);
        }
    }

    pub fn prioritize(&self, request: ThumbnailRequest) {
        let mut done = self.shared.done.lock().unwrap();
        // A re-hover after already seeing this page/tier should still show
        // it again immediately rather than silently no-op — forget it first.
        done.remove(&(request.page_idx, request.tier));
        self.decode_and_send(request, &mut done);
    }

    pub fn extend_low_priority(&self, requests: Vec<ThumbnailRequest>) {
        let mut done = self.shared.done.lock().unwrap();
        for request in requests {
            self.decode_and_send(request, &mut done);
        }
    }

    pub fn clear(&self) {
        self.shared.done.lock().unwrap().clear();
    }
}

#[cfg(target_arch = "wasm32")]
pub fn spawn_thumbnail_worker() -> (ThumbnailQueue, Receiver<DecodedThumbnail>) {
    let shared = Arc::new(Shared { done: Mutex::new(std::collections::HashSet::new()) });
    let (result_tx, result_rx) = channel::<DecodedThumbnail>();
    (ThumbnailQueue { shared, result_tx }, result_rx)
}
