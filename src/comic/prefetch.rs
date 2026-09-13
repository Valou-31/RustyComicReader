use crate::comic::archive::ComicArchive;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Condvar, Mutex};

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
}

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
#[derive(Clone)]
pub struct DecodeQueue {
    shared: Arc<Shared>,
}

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

/// Spawns a single long-lived worker that decodes page images off the UI
/// thread. Used to get pages just ahead of the current spread ready as
/// textures before the user turns to them, so the page turn itself doesn't
/// have to wait on a decode.
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

            if let Ok(image) = ComicArchive::decode_image(&request.data, request.max_dimension) {
                let result = DecodedPage { generation: request.generation, page_idx, image };
                if result_tx.send(result).is_err() {
                    break;
                }
            }
        }
    });

    (DecodeQueue { shared }, result_rx)
}
