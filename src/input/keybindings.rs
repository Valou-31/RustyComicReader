use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    NextSpread,
    PrevSpread,
    ShiftRight,
    ShiftLeft,
}

impl Action {
    pub const ALL: [Action; 4] = [Action::NextSpread, Action::PrevSpread, Action::ShiftRight, Action::ShiftLeft];

    pub fn label(&self) -> &'static str {
        match self {
            Action::NextSpread => "Next Spread",
            Action::PrevSpread => "Previous Spread",
            Action::ShiftRight => "Shift Right (peek)",
            Action::ShiftLeft => "Shift Left (peek)",
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
                bindings.insert(Action::ShiftRight, vec!["E".to_string()]);
                bindings.insert(Action::ShiftLeft, vec!["Q".to_string()]);
            }
            Preset::LeftHand => {
                bindings.insert(Action::NextSpread, vec!["D".to_string()]);
                bindings.insert(Action::PrevSpread, vec!["A".to_string()]);
                bindings.insert(Action::ShiftRight, vec!["E".to_string()]);
                bindings.insert(Action::ShiftLeft, vec!["Q".to_string()]);
            }
            Preset::RightHand => {
                bindings.insert(Action::NextSpread, vec!["ArrowRight".to_string()]);
                bindings.insert(Action::PrevSpread, vec!["ArrowLeft".to_string()]);
                bindings.insert(Action::ShiftRight, vec!["ArrowUp".to_string()]);
                bindings.insert(Action::ShiftLeft, vec!["ArrowDown".to_string()]);
            }
            Preset::Numpad => {
                bindings.insert(Action::NextSpread, vec!["Num6".to_string()]);
                bindings.insert(Action::PrevSpread, vec!["Num4".to_string()]);
                bindings.insert(Action::ShiftRight, vec!["Num9".to_string()]);
                bindings.insert(Action::ShiftLeft, vec!["Num7".to_string()]);
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
