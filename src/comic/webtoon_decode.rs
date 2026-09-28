use crate::comic::archive::ComicArchive;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Condvar, Mutex};

/// A page waiting to be sliced for the Webtoon strip, off the UI thread —
/// same shape as `comic::prefetch::DecodeRequest`, just routed to
/// `ComicArchive::decode_page_slices` instead of `decode_page_image` by the
/// worker below.
pub struct WebtoonDecodeRequest {
    pub generation: u64,
    pub data: Vec<u8>,
    pub max_dimension: Option<u32>,
}

pub struct WebtoonDecodedPage {
    pub generation: u64,
    pub page_idx: usize,
    /// Top-to-bottom slices — see `ComicArchive::decode_page_slices`.
    pub slices: Vec<egui::ColorImage>,
    pub aspect: f32,
}

struct Shared {
    queue: Mutex<HashMap<usize, WebtoonDecodeRequest>>,
    work_available: Condvar,
}

/// Handle to the background Webtoon-slice worker's input queue — a
/// coalescing queue, not a FIFO, for exactly the reason `comic::prefetch::
/// DecodeQueue` is: `reconcile` keeps its contents matching exactly what's
/// currently wanted, so a page scrolled past before the worker gets to it
/// is dropped rather than left to pile up.
#[derive(Clone)]
pub struct WebtoonDecodeQueue {
    shared: Arc<Shared>,
}

impl WebtoonDecodeQueue {
    /// Makes the queue's contents exactly `wanted` — see
    /// `comic::prefetch::DecodeQueue::reconcile`, same behavior.
    pub fn reconcile(
        &self,
        wanted: &std::collections::HashSet<usize>,
        mut make_request: impl FnMut(usize) -> WebtoonDecodeRequest,
    ) {
        let mut queue = self.shared.queue.lock().unwrap();
        queue.retain(|page_idx, _| wanted.contains(page_idx));
        for &page_idx in wanted {
            queue.entry(page_idx).or_insert_with(|| make_request(page_idx));
        }
        drop(queue);
        self.shared.work_available.notify_one();
    }

    /// Drops every not-yet-started request — called when a new archive
    /// finishes loading, same as `comic::prefetch::DecodeQueue::clear`.
    pub fn clear(&self) {
        self.shared.queue.lock().unwrap().clear();
    }
}

/// Spawns a single long-lived worker that slices Webtoon pages off the UI
/// thread — see `comic::archive::ComicArchive::decode_page_slices` and
/// `ComicApp::webtoon_textures` for why this needs its own worker rather
/// than sharing `comic::prefetch`'s: a request here can produce more than
/// one texture's worth of pixels.
pub fn spawn_webtoon_decode_worker() -> (WebtoonDecodeQueue, Receiver<WebtoonDecodedPage>) {
    let shared = Arc::new(Shared { queue: Mutex::new(HashMap::new()), work_available: Condvar::new() });
    let worker_shared = Arc::clone(&shared);
    let (result_tx, result_rx) = channel::<WebtoonDecodedPage>();

    std::thread::spawn(move || {
        loop {
            let mut queue = worker_shared.queue.lock().unwrap();
            while queue.is_empty() {
                queue = worker_shared.work_available.wait(queue).unwrap();
            }
            let page_idx = *queue.keys().next().unwrap();
            let request = queue.remove(&page_idx).unwrap();
            drop(queue);

            if let Ok((slices, aspect)) = ComicArchive::decode_page_slices(&request.data, request.max_dimension) {
                let result = WebtoonDecodedPage { generation: request.generation, page_idx, slices, aspect };
                if result_tx.send(result).is_err() {
                    break;
                }
            }
        }
    });

    (WebtoonDecodeQueue { shared }, result_rx)
}
