use crate::comic::archive::ComicArchive;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

pub struct LoadResult {
    pub path: PathBuf,
    pub filename: String,
    /// Each page's original compressed bytes, in reading order — decoded
    /// lazily by the UI as pages come into view.
    pub pages: Vec<Vec<u8>>,
}

pub enum LoadEvent {
    /// Sent after each page is decoded. `first_page` carries a copy of the
    /// very first decoded image (archive order, not final reading order),
    /// so the UI can show a preview while the rest keeps loading.
    Progress {
        loaded: usize,
        total: usize,
        first_page: Option<egui::ColorImage>,
    },
    Finished(LoadResult),
    Failed(String),
}

/// Sent once the native file dialog closes.
pub enum PickEvent {
    Picked(Vec<PathBuf>),
    Cancelled,
}

pub(crate) const SUPPORTED_EXTENSIONS: &[&str] = &["cbz", "cb7", "cbr", "zip", "7z", "rar"];

/// Opens a native file picker (multi-select) on a background thread, so the
/// (possibly slow) dialog doesn't block the UI. Sends `Cancelled` if the user
/// closes it without picking anything.
pub fn spawn_file_picker() -> Receiver<PickEvent> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let paths = rfd::FileDialog::new()
            .add_filter("Comic archives", SUPPORTED_EXTENSIONS)
            .pick_files();

        let event = match paths {
            Some(paths) if !paths.is_empty() => PickEvent::Picked(paths),
            _ => PickEvent::Cancelled,
        };
        let _ = tx.send(event);
    });

    rx
}

/// Decodes the archive at `path` on a background thread. Used both after the
/// file picker returns a path and whenever a path is already known (reading
/// history, the multi-file queue, resuming last session, CLI arguments).
pub fn spawn_file_load(path: PathBuf) -> Receiver<LoadEvent> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let filename = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Fichier inconnu".to_string());

        let progress_tx = tx.clone();
        let on_progress = move |loaded: usize, total: usize, preview: Option<&egui::ColorImage>| {
            let _ = progress_tx.send(LoadEvent::Progress { loaded, total, first_page: preview.cloned() });
        };

        let event = match pollster::block_on(ComicArchive::load(&path, on_progress)) {
            Ok(archive) => LoadEvent::Finished(LoadResult { path, filename, pages: archive.pages }),
            Err(err) => LoadEvent::Failed(err.to_string()),
        };
        let _ = tx.send(event);
    });

    rx
}
