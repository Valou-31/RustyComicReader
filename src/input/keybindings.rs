use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    NextSpread,
    PrevSpread,
    IsolatePage,
    /// Held (not pressed) to scroll the Webtoon strip upward — see
    /// `input::keyboard::handle_keyboard`, which polls this every frame
    /// rather than reacting to a single press.
    WebtoonScrollUp,
    WebtoonScrollDown,
    /// Opens the next sibling archive in the current file's own folder —
    /// see `ComicApp::open_sibling_volume`. The keyboard counterpart of
    /// Webtoon's swipe-past-the-bottom gesture (`ui::reader::WebtoonEdgeDrag`),
    /// but not itself Webtoon-specific: a no-op if there's no book open or
    /// no next file in the folder, same as the swipe.
    NextVolume,
}

impl Action {
    pub const ALL: [Action; 6] = [
        Action::NextSpread,
        Action::PrevSpread,
        Action::IsolatePage,
        Action::WebtoonScrollUp,
        Action::WebtoonScrollDown,
        Action::NextVolume,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Action::NextSpread => "Next Spread",
            Action::PrevSpread => "Previous Spread",
            Action::IsolatePage => "Isolate Page (single view)",
            Action::WebtoonScrollUp => "Webtoon: Scroll Up",
            Action::WebtoonScrollDown => "Webtoon: Scroll Down",
            Action::NextVolume => "Next Volume",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    DualMode,
    LeftHand,
    RightHand,
    Numpad,
}

impl Preset {
    pub const ALL: [Preset; 4] = [Preset::DualMode, Preset::LeftHand, Preset::RightHand, Preset::Numpad];

    pub fn label(&self) -> &'static str {
        match self {
            Preset::DualMode => "Dual-Mode (arrows + WASD)",
            Preset::LeftHand => "Left-Hand (WASD)",
            Preset::RightHand => "Right-Hand (arrows)",
            Preset::Numpad => "Numpad",
        }
    }

    pub fn bindings(&self) -> KeyBindings {
        let mut bindings = HashMap::new();
        match self {
            Preset::DualMode => {
                bindings.insert(Action::NextSpread, vec!["ArrowRight".to_string(), "D".to_string()]);
                bindings.insert(Action::PrevSpread, vec!["ArrowLeft".to_string(), "A".to_string()]);
                bindings.insert(Action::IsolatePage, vec!["E".to_string()]);
                bindings.insert(Action::WebtoonScrollUp, vec!["ArrowUp".to_string()]);
                bindings.insert(Action::WebtoonScrollDown, vec!["ArrowDown".to_string()]);
                bindings.insert(Action::NextVolume, vec!["N".to_string()]);
            }
            Preset::LeftHand => {
                bindings.insert(Action::NextSpread, vec!["D".to_string()]);
                bindings.insert(Action::PrevSpread, vec!["A".to_string()]);
                bindings.insert(Action::IsolatePage, vec!["E".to_string()]);
                // Keeps the hand on WASD rather than reaching for the arrows.
                bindings.insert(Action::WebtoonScrollUp, vec!["W".to_string()]);
                bindings.insert(Action::WebtoonScrollDown, vec!["S".to_string()]);
                bindings.insert(Action::NextVolume, vec!["N".to_string()]);
            }
            Preset::RightHand => {
                bindings.insert(Action::NextSpread, vec!["ArrowRight".to_string()]);
                bindings.insert(Action::PrevSpread, vec!["ArrowLeft".to_string()]);
                bindings.insert(Action::IsolatePage, vec!["E".to_string()]);
                bindings.insert(Action::WebtoonScrollUp, vec!["ArrowUp".to_string()]);
                bindings.insert(Action::WebtoonScrollDown, vec!["ArrowDown".to_string()]);
                bindings.insert(Action::NextVolume, vec!["N".to_string()]);
            }
            Preset::Numpad => {
                bindings.insert(Action::NextSpread, vec!["Num6".to_string()]);
                bindings.insert(Action::PrevSpread, vec!["Num4".to_string()]);
                bindings.insert(Action::IsolatePage, vec!["E".to_string()]);
                // Matches the numpad's own up/down, alongside 4/6 for
                // left/right.
                bindings.insert(Action::WebtoonScrollUp, vec!["Num8".to_string()]);
                bindings.insert(Action::WebtoonScrollDown, vec!["Num2".to_string()]);
                bindings.insert(Action::NextVolume, vec!["N".to_string()]);
            }
        }
        KeyBindings { bindings }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyBindings {
    pub bindings: HashMap<Action, Vec<String>>,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Preset::DualMode.bindings()
    }
}

impl KeyBindings {
    pub fn action_for_key(&self, key: &str) -> Option<Action> {
        for (action, keys) in &self.bindings {
            if keys.iter().any(|k| k.eq_ignore_ascii_case(key)) {
                return Some(*action);
            }
        }
        None
    }

    /// No-ops if `key` is already bound to `action` (case-insensitive).
    pub fn add_key(&mut self, action: Action, key: String) {
        let keys = self.bindings.entry(action).or_insert_with(Vec::new);
        if !keys.iter().any(|k| k.eq_ignore_ascii_case(&key)) {
            keys.push(key);
        }
    }

    pub fn remove_key(&mut self, action: Action, key: &str) {
        if let Some(keys) = self.bindings.get_mut(&action) {
            keys.retain(|k| k != key);
        }
    }
}
