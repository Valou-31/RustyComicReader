use crate::comic::archive::{ComicArchive, PageMeta};
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Condvar;
use std::sync::{Arc, Mutex};

/// A page waiting to be decoded off the UI thread. Carries its own bytes
/// (rather than an index into the app's page list) so the worker doesn't
/// need shared access to app state. `generation` identifies which loaded
/// archive this request belongs to, so a result that arrives after the user
/// has already opened a different file can be told apart from a page that
/// merely shares the same index in the new book.
pub struct DecodeRequest {
    pub generation: u64,
    pub data: Vec<u8>,
    pub max_dimension: Option<u32>,
}

pub struct DecodedPage {
    pub generation: u64,
    pub page_idx: usize,
    pub image: egui::ColorImage,
    pub meta: PageMeta,
}

#[cfg(not(target_arch = "wasm32"))]
struct Shared {
    queue: Mutex<HashMap<usize, DecodeRequest>>,
    work_available: Condvar,
}

/// Handle to the background decode worker's input queue.
///
/// This is a *coalescing* queue, not a FIFO: `reconcile` replaces its
/// contents to match exactly what's currently wanted (the pages near the
/// current spread), dropping requests for pages the user has since scrolled
/// past instead of leaving them to pile up. Without that, a worker that
/// can't keep up with fast page-turning would end up with a ever-growing
/// backlog of queued, never-relevant-anymore page bytes — effectively a
/// second copy of large chunks of the archive sitting in memory on top of
/// `ComicApp::pages`.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct DecodeQueue {
    shared: Arc<Shared>,
}

#[cfg(not(target_arch = "wasm32"))]
impl DecodeQueue {
    /// Makes the queue's contents exactly `wanted`: drops any not-yet-started
    /// request for a page no longer in `wanted`, and adds one (via
    /// `make_request`, called only for pages actually missing from the
    /// queue) for each newly wanted page.
    pub fn reconcile(&self, wanted: &std::collections::HashSet<usize>, mut make_request: impl FnMut(usize) -> DecodeRequest) {
        let mut queue = self.shared.queue.lock().unwrap();
        queue.retain(|page_idx, _| wanted.contains(page_idx));
        for &page_idx in wanted {
            queue.entry(page_idx).or_insert_with(|| make_request(page_idx));
        }
        drop(queue);
        self.shared.work_available.notify_one();
    }

    /// Drops every not-yet-started request. Called when a new archive
    /// finishes loading, so a leftover request from the previous book can't
    /// coincidentally survive `reconcile` just because the new book also
    /// wants a page at the same index.
    pub fn clear(&self) {
        self.shared.queue.lock().unwrap().clear();
    }
}

/// The web build's counterpart to the native `Shared`/`DecodeQueue` above —
/// no queue at all, just which pages have already been decoded-and-sent (so
/// `reconcile`, called every frame with a mostly-unchanged `wanted` set,
/// doesn't redecode a page it already handled).
#[cfg(target_arch = "wasm32")]
struct Shared {
    done: Mutex<std::collections::HashSet<usize>>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
pub struct DecodeQueue {
    shared: Arc<Shared>,
    result_tx: std::sync::mpsc::Sender<DecodedPage>,
}

#[cfg(target_arch = "wasm32")]
impl DecodeQueue {
    /// Synchronously decodes every page in `wanted` not already sent since
    /// the last `clear()`, sending each result immediately on the same
    /// channel `spawn_decode_worker`'s `Receiver` reads.
    pub fn reconcile(&self, wanted: &std::collections::HashSet<usize>, mut make_request: impl FnMut(usize) -> DecodeRequest) {
        let mut done = self.shared.done.lock().unwrap();
        for &page_idx in wanted {
            if !done.insert(page_idx) {
                continue;
            }
            let request = make_request(page_idx);
            if let Ok(image) = ComicArchive::decode_page_image(&request.data, request.max_dimension) {
                let meta = PageMeta::sample(&image);
                let _ = self.result_tx.send(DecodedPage { generation: request.generation, page_idx, image, meta });
            }
        }
    }

    /// Forgets every page decoded so far, so a newly loaded book's pages
    /// (which may reuse the same indices) get decoded again rather than
    /// being skipped as "already done".
    pub fn clear(&self) {
        self.shared.done.lock().unwrap().clear();
    }
}

/// Spawns a single long-lived worker that decodes page images off the UI
/// thread. Used to get pages just ahead of the current spread ready as
/// textures before the user turns to them, so the page turn itself doesn't
/// have to wait on a decode.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_decode_worker() -> (DecodeQueue, Receiver<DecodedPage>) {
    let shared = Arc::new(Shared { queue: Mutex::new(HashMap::new()), work_available: Condvar::new() });
    let worker_shared = Arc::clone(&shared);
    let (result_tx, result_rx) = channel::<DecodedPage>();

    std::thread::spawn(move || {
        loop {
            let mut queue = worker_shared.queue.lock().unwrap();
            while queue.is_empty() {
                queue = worker_shared.work_available.wait(queue).unwrap();
            }
            let page_idx = *queue.keys().next().unwrap();
            let request = queue.remove(&page_idx).unwrap();
            drop(queue);

            if let Ok(image) = ComicArchive::decode_page_image(&request.data, request.max_dimension) {
                let meta = PageMeta::sample(&image);
                let result = DecodedPage { generation: request.generation, page_idx, image, meta };
                if result_tx.send(result).is_err() {
                    break;
                }
            }
        }
    });

    (DecodeQueue { shared }, result_rx)
}

/// The web build's counterpart to the native worker above — `wasm32` has no
/// OS threads (short of a heavyweight `SharedArrayBuffer`/Web Worker pool
/// setup, out of scope for v1), so `reconcile` just decodes synchronously,
/// inline, for whichever requested pages aren't in `done` yet, and sends
/// each result on the same channel type the native `poll_decoded_pages`
/// already reads — no changes needed on the calling side. Trade-off: a page
/// decode briefly blocks the browser tab instead of overlapping with
/// scrolling/input.
#[cfg(target_arch = "wasm32")]
pub fn spawn_decode_worker() -> (DecodeQueue, Receiver<DecodedPage>) {
    let shared = Arc::new(Shared { done: Mutex::new(std::collections::HashSet::new()) });
    let (result_tx, result_rx) = channel::<DecodedPage>();
    (DecodeQueue { shared, result_tx }, result_rx)
}
