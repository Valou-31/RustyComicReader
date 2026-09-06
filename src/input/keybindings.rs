use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    NextSpread,
    PrevSpread,
    ShiftRight,
    ShiftLeft,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyBindings {
    pub bindings: HashMap<Action, Vec<String>>,
}

impl Default for KeyBindings {
    fn default() -> Self {
        let mut bindings = HashMap::new();
        bindings.insert(Action::NextSpread, vec!["ArrowRight".to_string(), "d".to_string()]);
        bindings.insert(Action::PrevSpread, vec!["ArrowLeft".to_string(), "a".to_string()]);
        bindings.insert(Action::ShiftRight, vec!["e".to_string()]);
        bindings.insert(Action::ShiftLeft, vec!["q".to_string()]);
        
        Self { bindings }
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

    pub fn add_key(&mut self, action: Action, key: String) {
        self.bindings.entry(action).or_insert_with(Vec::new).push(key);
    }

    pub fn remove_key(&mut self, action: Action, key: &str) {
        if let Some(keys) = self.bindings.get_mut(&action) {
            keys.retain(|k| k != key);
        }
    }

    pub fn clear_action(&mut self, action: Action) {
        self.bindings.insert(action, Vec::new());
    }
}