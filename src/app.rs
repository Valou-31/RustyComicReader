use crate::input::keybindings::{Action, KeyBindings};
use crate::storage::config::Config;
use crate::ui::layout::LayoutConfig;
use crate::ui::theme::ThemePreset;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// How long the header stays visible after the last mouse movement.
pub const UI_HIDE_DELAY: Duration = Duration::from_secs(3);

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
    pub pages: Vec<egui::ColorImage>,  // ✅ egui::ColorImage, pas image::ColorImage
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
}

impl Default for ComicApp {
    fn default() -> Self {
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
            load_progress: (0, 0),
            load_preview: None,
            preview_texture: None,
            window_title: "Comic Reader".to_string(),
            title_dirty: false,
            logo_texture: None,
            pending_load: None,
        }
    }
}

impl ComicApp {
    /// Like `default()`, but starts from whatever was last saved to disk
    /// (reading mode, keybindings), falling back to defaults if there's
    /// nothing saved yet or it can't be read.
    pub fn new() -> Self {
        let mut app = Self::default();
        if let Ok(config) = Config::load() {
            app.reading_mode = config.reading_mode;
            app.keybindings = config.keybindings;
            app.theme_preset = config.theme;
            app.layout = config.layout;
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
        }
    }

    pub fn prev_spread(&mut self) {
        let position = self.current_spread * 2 + self.page_offset;
        if position >= 2 {
            let position = position - 2;
            self.current_spread = position / 2;
            self.page_offset = position % 2;
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
    }

    pub fn shift_left(&mut self) {
        self.page_offset = self.page_offset.saturating_sub(1);
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

    /// Opens the native file picker and loads the chosen archive in the background.
    pub fn start_loading_file(&mut self) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.load_error = None;
        self.load_progress = (0, 0);
        self.load_preview = None;
        self.preview_texture = None;
        self.pending_load = Some(crate::comic::loader::spawn_file_picker());
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
                    self.current_spread = 0;
                    self.page_offset = 0;
                    self.loading = false;
                    self.load_preview = None;
                    self.preview_texture = None;
                    self.pending_load = None;
                    self.window_title = format!("Comic Reader — {}", self.filename);
                    self.title_dirty = true;
                    break;
                }
                Ok(LoadEvent::Failed(err)) => {
                    self.load_error = Some(err);
                    self.loading = false;
                    self.load_preview = None;
                    self.preview_texture = None;
                    self.pending_load = None;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // Dialog was cancelled or the worker thread panicked.
                    self.loading = false;
                    self.load_preview = None;
                    self.preview_texture = None;
                    self.pending_load = None;
                    break;
                }
            }
        }
    }
}