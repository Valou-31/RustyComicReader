use crate::input::keybindings::KeyBindings;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub enum ReadingMode {
    LTR, // Left-to-Right
    RTL, // Right-to-Left
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemappingAction {
    NextSpread,
    PrevSpread,
    ShiftRight,
    ShiftLeft,
}

pub struct ComicApp {
    pub pages: Vec<egui::ColorImage>,  // ✅ egui::ColorImage, pas image::ColorImage
    pub current_spread: usize,
    pub total_pages: usize,
    pub page_offset: i32,
    pub reading_mode: ReadingMode,
    pub filename: String,
    pub show_settings: bool,
    pub ui_hidden: bool,
    pub last_mouse_move: std::time::Instant,
    pub fullscreen: bool,
    pub keybindings: KeyBindings,
    pub remapping_action: Option<RemappingAction>,
    pub textures: HashMap<usize, egui::TextureHandle>,
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
            ui_hidden: false,
            last_mouse_move: std::time::Instant::now(),
            fullscreen: false,
            keybindings: KeyBindings::default(),
            remapping_action: None,
            textures: HashMap::new(),
        }
    }
}

impl ComicApp {
    pub fn next_spread(&mut self) {
        if self.current_spread * 2 + 1 < self.total_pages {
            self.current_spread += 1;
            self.page_offset = 0;
        }
    }

    pub fn prev_spread(&mut self) {
        if self.current_spread > 0 {
            self.current_spread -= 1;
            self.page_offset = 0;
        }
    }

    pub fn left_page(&self) -> Option<usize> {
        match self.reading_mode {
            ReadingMode::LTR => Some(self.current_spread * 2 + self.page_offset as usize),
            ReadingMode::RTL => {
                let page = self.current_spread * 2 + 1 + self.page_offset as usize;
                if page < self.total_pages { Some(page) } else { None }
            }
        }
    }

    pub fn right_page(&self) -> Option<usize> {
        match self.reading_mode {
            ReadingMode::LTR => Some(self.current_spread * 2 + 1 + self.page_offset as usize),
            ReadingMode::RTL => Some(self.current_spread * 2 + self.page_offset as usize),
        }
    }
}