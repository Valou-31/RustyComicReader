use crate::comic::archive::{ComicArchive, ForeEdgeTail};
use crate::comic::comic_info::ComicInfo;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

pub struct LoadResult {
    pub path: PathBuf,
    pub filename: String,
    /// Each page's original compressed bytes, in reading order — decoded
    /// lazily by the UI as pages come into view.
    pub pages: Vec<Vec<u8>>,
    /// Parsed from the archive's `ComicInfo.xml` entry, if it had one worth
    /// showing — see `comic::comic_info::ComicInfo`.
    pub comic_info: Option<ComicInfo>,
    /// Every page's fore-edge strip that had already finished sampling by
    /// the time the archive finished loading — see
    /// `comic::archive::ComicArchive::fore_edge_columns`. Empty when
    /// `spawn_file_load`'s `compute_fore_edge` was `false`.
    pub fore_edge_columns: Vec<Vec<egui::Color32>>,
    /// Whatever fore-edge sampling was still in flight — see
    /// `comic::archive::ComicArchive::fore_edge_tail`. The UI keeps polling
    /// this to fill in `fore_edge_columns`'s remaining blanks as they land.
    pub fore_edge_tail: Option<ForeEdgeTail>,
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

/// Whether `path`'s extension is one `ComicArchive::load` can actually open —
/// shared by every non-file-picker way a path reaches the app (CLI args,
/// macOS Open-With/Dock-drop events, a window drag-and-drop) so they all
/// agree on what counts as "a comic archive" without duplicating the check.
pub(crate) fn is_supported_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| SUPPORTED_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
}

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
///
/// `compute_fore_edge` (the user's "book thickness" setting) gates the
/// fore-edge sampling `ComicArchive::load` does concurrently with
/// extraction — when `false`, no extra decoding happens and
/// `LoadResult::fore_edge_columns` comes back empty.
pub fn spawn_file_load(path: PathBuf, compute_fore_edge: bool) -> Receiver<LoadEvent> {
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

        let event = match pollster::block_on(ComicArchive::load(&path, compute_fore_edge, on_progress)) {
            Ok(archive) => LoadEvent::Finished(LoadResult {
                path,
                filename,
                pages: archive.pages,
                comic_info: archive.comic_info,
                fore_edge_columns: archive.fore_edge_columns,
                fore_edge_tail: archive.fore_edge_tail,
            }),
            Err(err) => LoadEvent::Failed(err.to_string()),
        };
        let _ = tx.send(event);
    });

    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_every_supported_extension_case_insensitively() {
        for ext in SUPPORTED_EXTENSIONS {
            assert!(is_supported_path(Path::new(&format!("book.{ext}"))));
            assert!(is_supported_path(Path::new(&format!("book.{}", ext.to_uppercase()))));
        }
    }

    #[test]
    fn rejects_an_unsupported_extension() {
        assert!(!is_supported_path(Path::new("notes.txt")));
    }

    #[test]
    fn rejects_a_path_with_no_extension() {
        assert!(!is_supported_path(Path::new("book")));
    }
}
