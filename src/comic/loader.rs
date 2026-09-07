use crate::comic::archive::ComicArchive;
use std::sync::mpsc::Receiver;

pub struct LoadResult {
    pub filename: String,
    pub pages: Vec<egui::ColorImage>,
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

const SUPPORTED_EXTENSIONS: &[&str] = &["cbz", "cb7", "cbr", "zip", "7z", "rar"];

/// Opens a native file picker and decodes the chosen archive on a background
/// thread, so neither the (possibly slow) dialog nor the decoding blocks the
/// UI thread. Sends nothing if the user cancels the dialog.
pub fn spawn_file_picker() -> Receiver<LoadEvent> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Comic archives", SUPPORTED_EXTENSIONS)
            .pick_file()
        else {
            return;
        };

        let filename = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Fichier inconnu".to_string());

        let progress_tx = tx.clone();
        let mut first_page_sent = false;
        let on_progress = move |loaded: usize, total: usize, image: &egui::ColorImage| {
            let first_page = if first_page_sent {
                None
            } else {
                first_page_sent = true;
                Some(image.clone())
            };
            let _ = progress_tx.send(LoadEvent::Progress { loaded, total, first_page });
        };

        let event = match pollster::block_on(ComicArchive::load(&path, on_progress)) {
            Ok(archive) => LoadEvent::Finished(LoadResult { filename, pages: archive.pages }),
            Err(err) => LoadEvent::Failed(err.to_string()),
        };
        let _ = tx.send(event);
    });

    rx
}
