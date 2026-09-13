use crate::comic::prefetch::{DecodeQueue, DecodeRequest};
use crate::input::keybindings::{Action, KeyBindings};
use crate::storage::config::Config;
use crate::storage::history::History;
use crate::ui::layout::LayoutConfig;
use crate::ui::theme::ThemePreset;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::Duration;

/// How long the header stays visible after the last mouse movement.
pub const UI_HIDE_DELAY: Duration = Duration::from_secs(3);

/// Minimum time between debounced history saves triggered by page
/// navigation — an exit or a freshly-loaded book flushes immediately
/// regardless of this.
pub const HISTORY_SAVE_DEBOUNCE: Duration = Duration::from_secs(5);

/// When downscaling is enabled, no decoded page dimension exceeds this —
/// comic scans are routinely 3000-6000px on a side, far beyond what any
/// display can show, so this trades invisible sharpness for a large RAM/VRAM
/// saving per page.
pub const DOWNSCALE_MAX_DIMENSION: u32 = 2400;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadingMode {
    LTR, // Left-to-Right
    RTL, // Right-to-Left
}

impl ReadingMode {
    pub fn label(&self) -> &'static str {
        match self {
            ReadingMode::LTR => "➡ LTR (Western)",
            ReadingMode::RTL => "⬅ RTL (Manga)",
        }
    }
}

pub struct ComicApp {
    /// Each page's original compressed bytes (JPEG/PNG/etc.), not decoded
    /// pixels — decoding happens on demand in the reader UI, only for pages
    /// near the current spread, so memory doesn't scale with book length.
    pub pages: Vec<Vec<u8>>,
    pub current_spread: usize,
    pub total_pages: usize,
    pub page_offset: usize,
    pub reading_mode: ReadingMode,
    pub filename: String,
    pub show_settings: bool,
    pub last_mouse_move: std::time::Instant,
    pub fullscreen: bool,
    pub fullscreen_dirty: bool,
    pub keybindings: KeyBindings,
    pub theme_preset: ThemePreset,
    pub layout: LayoutConfig,
    /// The action currently waiting for its next key press to be bound to it.
    pub remapping_action: Option<Action>,
    pub textures: HashMap<usize, egui::TextureHandle>,
    pub loading: bool,
    pub load_error: Option<String>,
    /// Full path of the archive currently loaded, if any — `None` on the
    /// empty-state screen. Used to key reading-history updates.
    pub current_path: Option<PathBuf>,
    /// Files picked (or passed on the command line) alongside the one
    /// currently loading/loaded, waiting their turn.
    pub file_queue: VecDeque<PathBuf>,
    pub history: History,
    pub show_history: bool,
    /// Set true by navigation, cleared once `flush_history` runs.
    history_dirty: bool,
    history_last_saved: std::time::Instant,
    /// Whether to reopen the last-read book (at its last page) on startup.
    /// Persisted; toggled from the settings panel.
    pub resume_last_session: bool,
    /// Page to jump to once the in-flight load finishes — set when that
    /// load is a session resume or a history reopen, `None` for a fresh
    /// file that should just start at page 0.
    resume_to_page: Option<usize>,
    pending_pick: Option<std::sync::mpsc::Receiver<crate::comic::loader::PickEvent>>,
    /// (pages decoded so far, total pages in the archive), while `loading` is true.
    pub load_progress: (usize, usize),
    /// The first decoded page, handed off once so the UI can upload it to a
    /// texture; `None` once picked up or when there is nothing new to show.
    pub load_preview: Option<egui::ColorImage>,
    pub preview_texture: Option<egui::TextureHandle>,
    pub window_title: String,
    pub title_dirty: bool,
    /// Lazily uploaded the first time the empty-state screen is drawn.
    pub logo_texture: Option<egui::TextureHandle>,
    pending_load: Option<std::sync::mpsc::Receiver<crate::comic::loader::LoadEvent>>,
    /// Downscale decoded pages to `DOWNSCALE_MAX_DIMENSION` to save RAM/VRAM,
    /// at some cost to sharpness on very high-res scans. User-toggleable.
    pub downscale_large_pages: bool,
    /// Identifies the currently loaded archive, bumped every time a new file
    /// finishes loading — lets `poll_decoded_pages` discard a background
    /// decode that was still in flight for the *previous* file.
    load_generation: u64,
    /// The background decode worker's input queue. `request_prefetch`
    /// reconciles it to exactly the pages currently wanted every frame, so a
    /// page the user has scrolled past gets dropped from it instead of
    /// piling up behind a worker that can't keep up.
    decode_queue: DecodeQueue,
    decode_result_rx: std::sync::mpsc::Receiver<crate::comic::prefetch::DecodedPage>,
}

impl Default for ComicApp {
    fn default() -> Self {
        let (decode_queue, decode_result_rx) = crate::comic::prefetch::spawn_decode_worker();
        Self {
            pages: Vec::new(),
            current_spread: 0,
            total_pages: 0,
            page_offset: 0,
            reading_mode: ReadingMode::LTR,
            filename: "Aucun fichier".to_string(),
            show_settings: false,
            last_mouse_move: std::time::Instant::now(),
            fullscreen: false,
            fullscreen_dirty: false,
            keybindings: KeyBindings::default(),
            theme_preset: ThemePreset::default(),
            layout: LayoutConfig::default(),
            remapping_action: None,
            textures: HashMap::new(),
            loading: false,
            load_error: None,
            current_path: None,
            file_queue: VecDeque::new(),
            history: History::default(),
            show_history: false,
            history_dirty: false,
            history_last_saved: std::time::Instant::now(),
            resume_last_session: true,
            resume_to_page: None,
            pending_pick: None,
            load_progress: (0, 0),
            load_preview: None,
            preview_texture: None,
            window_title: "Comic Reader".to_string(),
            title_dirty: false,
            logo_texture: None,
            pending_load: None,
            downscale_large_pages: true,
            load_generation: 0,
            decode_queue,
            decode_result_rx,
        }
    }
}

impl ComicApp {
    /// Loads whatever was last saved to disk (reading mode, keybindings,
    /// theme, layout, reading history) into `self`, falling back to
    /// whatever's already there if a file is missing or unreadable.
    fn load_persisted(&mut self) {
        if let Ok(config) = Config::load() {
            self.reading_mode = config.reading_mode;
            self.keybindings = config.keybindings;
            self.theme_preset = config.theme;
            self.layout = config.layout;
            self.downscale_large_pages = config.downscale_large_pages;
            self.resume_last_session = config.resume_last_session;
        }
        if let Ok(history) = History::load() {
            self.history = history;
        }
    }

    /// Like `default()`, but starts from whatever was last saved to disk,
    /// and — if `resume_last_session` is on and the file still exists —
    /// starts loading the most recently read book back to its last page.
    pub fn new() -> Self {
        let mut app = Self::default();
        app.load_persisted();
        if app.resume_last_session
            && let Some(entry) = app.history.entries.first()
            && entry.path.exists()
        {
            let path = entry.path.clone();
            app.resume_to_page = Some(entry.last_page);
            app.start_loading_path(path);
        }
        app
    }

    /// Like `new()`, but opens the given files instead of resuming the last
    /// session — used when the app was launched with file arguments (double-
    /// click / "Open With" / a comic archive passed on the command line).
    /// The first file loads immediately; the rest join the queue.
    pub fn new_with_files(paths: Vec<PathBuf>) -> Self {
        let mut app = Self::default();
        app.load_persisted();
        let mut paths = paths.into_iter();
        if let Some(first) = paths.next() {
            app.file_queue.extend(paths);
            app.start_loading_path(first);
        }
        app
    }

    /// Persists the current reading mode, keybindings, theme and layout.
    /// Failures are logged, not surfaced — losing a settings save shouldn't
    /// interrupt reading.
    pub fn save_config(&self) {
        let config = Config {
            reading_mode: self.reading_mode,
            keybindings: self.keybindings.clone(),
            theme: self.theme_preset,
            layout: self.layout.clone(),
            downscale_large_pages: self.downscale_large_pages,
            resume_last_session: self.resume_last_session,
        };
        if let Err(err) = config.save() {
            tracing::warn!("Failed to save config: {err}");
        }
    }

    /// Advances by exactly 2 pages from wherever the peek offset currently
    /// has us looking — not from the un-offset spread boundary — so a peek
    /// (`shift_right`/`shift_left`) carries forward instead of being discarded.
    pub fn next_spread(&mut self) {
        let position = self.current_spread * 2 + self.page_offset + 2;
        if position < self.total_pages {
            self.current_spread = position / 2;
            self.page_offset = position % 2;
            self.mark_history_dirty();
        }
    }

    pub fn prev_spread(&mut self) {
        let position = self.current_spread * 2 + self.page_offset;
        if position >= 2 {
            let position = position - 2;
            self.current_spread = position / 2;
            self.page_offset = position % 2;
            self.mark_history_dirty();
        }
    }

    /// How long since the mouse last moved — drives the header auto-hide fade.
    pub fn idle_time(&self) -> Duration {
        self.last_mouse_move.elapsed()
    }

    pub fn toggle_reading_mode(&mut self) {
        self.reading_mode = match self.reading_mode {
            ReadingMode::LTR => ReadingMode::RTL,
            ReadingMode::RTL => ReadingMode::LTR,
        };
        self.page_offset = 0;
        self.save_config();
    }

    /// Largest offset for which the spread's left page still stays in bounds.
    fn max_page_offset(&self) -> usize {
        self.total_pages.saturating_sub(self.current_spread * 2 + 1)
    }

    pub fn shift_right(&mut self) {
        self.page_offset = (self.page_offset + 1).min(self.max_page_offset());
        self.mark_history_dirty();
    }

    pub fn shift_left(&mut self) {
        self.page_offset = self.page_offset.saturating_sub(1);
        self.mark_history_dirty();
    }

    fn mark_history_dirty(&mut self) {
        if self.current_path.is_some() {
            self.history_dirty = true;
        }
    }

    pub fn left_page(&self) -> Option<usize> {
        let page = match self.reading_mode {
            ReadingMode::LTR => self.current_spread * 2 + self.page_offset,
            ReadingMode::RTL => self.current_spread * 2 + 1 + self.page_offset,
        };
        if page < self.total_pages { Some(page) } else { None }
    }

    pub fn right_page(&self) -> Option<usize> {
        let page = match self.reading_mode {
            ReadingMode::LTR => self.current_spread * 2 + 1 + self.page_offset,
            ReadingMode::RTL => self.current_spread * 2 + self.page_offset,
        };
        if page < self.total_pages { Some(page) } else { None }
    }

    /// Opens the native file picker (multi-select); once it returns,
    /// `poll_picking` starts loading the first pick and queues the rest.
    pub fn start_loading_file(&mut self) {
        if self.loading {
            return;
        }
        self.pending_pick = Some(crate::comic::loader::spawn_file_picker());
    }

    /// Starts decoding an already-known path in the background — used by the
    /// file picker, reading history, the multi-file queue, session resume,
    /// and files passed on the command line.
    pub fn start_loading_path(&mut self, path: PathBuf) {
        self.loading = true;
        self.load_error = None;
        self.load_progress = (0, 0);
        self.load_preview = None;
        self.preview_texture = None;
        self.pending_load = Some(crate::comic::loader::spawn_file_load(path));
    }

    /// Pops the next queued file (if any) and starts loading it.
    pub fn open_next_in_queue(&mut self) {
        if let Some(path) = self.file_queue.pop_front() {
            self.start_loading_path(path);
        }
    }

    /// Call once per frame: picks up the file picker's result, if any.
    pub fn poll_picking(&mut self) {
        use crate::comic::loader::PickEvent;
        use std::sync::mpsc::TryRecvError;

        let Some(receiver) = &self.pending_pick else {
            return;
        };

        match receiver.try_recv() {
            Ok(PickEvent::Picked(paths)) => {
                self.pending_pick = None;
                let mut paths = paths.into_iter();
                if let Some(first) = paths.next() {
                    self.file_queue.extend(paths);
                    self.start_loading_path(first);
                }
            }
            Ok(PickEvent::Cancelled) | Err(TryRecvError::Disconnected) => {
                self.pending_pick = None;
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    /// Call once per frame: drains every background-load event queued since
    /// the last poll (progress ticks can arrive faster than frames).
    pub fn poll_loading(&mut self) {
        use crate::comic::loader::LoadEvent;
        use std::sync::mpsc::TryRecvError;

        let Some(receiver) = &self.pending_load else {
            return;
        };

        loop {
            match receiver.try_recv() {
                Ok(LoadEvent::Progress { loaded, total, first_page }) => {
                    self.load_progress = (loaded, total);
                    if first_page.is_some() {
                        self.load_preview = first_page;
                    }
                }
                Ok(LoadEvent::Finished(result)) => {
                    self.textures.clear();
                    self.total_pages = result.pages.len();
                    self.pages = result.pages;
                    self.filename = result.filename;
                    self.current_path = Some(result.path.clone());

                    if let Some(page) = self.resume_to_page.take() {
                        self.current_spread = page / 2;
                        self.page_offset = page % 2;
                    } else {
                        self.current_spread = 0;
                        self.page_offset = 0;
                    }

                    self.loading = false;
                    // Any decode still in flight for the previous archive is
                    // now moot; `load_generation` lets `poll_decoded_pages`
                    // tell such a late result apart from one for this book.
                    self.load_generation += 1;
                    self.decode_queue.clear();
                    self.load_preview = None;
                    self.preview_texture = None;
                    self.pending_load = None;
                    self.window_title = format!("Comic Reader — {}", self.filename);
                    self.title_dirty = true;

                    let last_page = self.current_spread * 2 + self.page_offset;
                    self.history.touch(&result.path, &self.filename, last_page, self.total_pages);
                    self.history_dirty = false;
                    self.history_last_saved = std::time::Instant::now();
                    if let Err(err) = self.history.save() {
                        tracing::warn!("Failed to save history: {err}");
                    }
                    break;
                }
                Ok(LoadEvent::Failed(err)) => {
                    self.load_error = Some(err);
                    self.loading = false;
                    self.load_preview = None;
                    self.preview_texture = None;
                    self.pending_load = None;
                    self.resume_to_page = None;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // Dialog was cancelled or the worker thread panicked.
                    self.loading = false;
                    self.load_preview = None;
                    self.preview_texture = None;
                    self.pending_load = None;
                    self.resume_to_page = None;
                    break;
                }
            }
        }
    }

    /// Call once per frame: flushes the reading position to disk if it's
    /// changed and enough time has passed since the last save (debounced —
    /// navigating rapidly shouldn't hit disk every frame). `flush_history`
    /// bypasses the debounce for exit and fresh-load saves.
    pub fn poll_history_save(&mut self) {
        if self.history_dirty && self.history_last_saved.elapsed() >= HISTORY_SAVE_DEBOUNCE {
            self.flush_history();
        }
    }

    /// Records the current page against `current_path` (if any) and writes
    /// history to disk immediately. Safe to call with nothing loaded or
    /// nothing dirty — becomes a cheap no-op / redundant save.
    pub fn flush_history(&mut self) {
        if let Some(path) = &self.current_path {
            let last_page = self.current_spread * 2 + self.page_offset;
            self.history.update_page(path, last_page);
        }
        if let Err(err) = self.history.save() {
            tracing::warn!("Failed to save history: {err}");
        }
        self.history_dirty = false;
        self.history_last_saved = std::time::Instant::now();
    }

    /// The dimension cap to decode pages at, or `None` for full source
    /// resolution — depends on the user's downscale setting.
    pub fn decode_max_dimension(&self) -> Option<u32> {
        self.downscale_large_pages.then_some(DOWNSCALE_MAX_DIMENSION)
    }

    /// Makes the background decode queue want exactly the pages in
    /// `[low, high]` that don't have a texture yet — pages just ahead of (or
    /// behind) the current spread, so their textures are ready by the time
    /// the user turns to them instead of decoding synchronously on that
    /// frame. A page that leaves this range before the worker gets to it is
    /// dropped from the queue rather than decoded anyway — see `DecodeQueue`.
    pub fn request_prefetch(&self, low: usize, high: usize) {
        if self.pages.is_empty() {
            return;
        }
        let max_dimension = self.decode_max_dimension();
        let high = high.min(self.pages.len() - 1);
        let generation = self.load_generation;
        let pages = &self.pages;

        let wanted: HashSet<usize> = (low..=high).filter(|idx| !self.textures.contains_key(idx)).collect();
        self.decode_queue.reconcile(&wanted, |page_idx| DecodeRequest {
            generation,
            data: pages[page_idx].clone(),
            max_dimension,
        });
    }

    /// Call once per frame: turns every background-decoded page that's
    /// finished since the last poll into a GPU texture. Cheap no-op when
    /// nothing has arrived.
    pub fn poll_decoded_pages(&mut self, ctx: &egui::Context) {
        while let Ok(decoded) = self.decode_result_rx.try_recv() {
            if decoded.generation != self.load_generation {
                continue; // stale — belonged to a since-replaced archive
            }
            self.textures.entry(decoded.page_idx).or_insert_with(|| {
                ctx.load_texture(format!("page_{}", decoded.page_idx), decoded.image, egui::TextureOptions::LINEAR)
            });
        }
    }
}