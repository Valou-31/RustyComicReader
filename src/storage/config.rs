use crate::app::ReadingMode;
use crate::input::keybindings::KeyBindings;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub reading_mode: ReadingMode,
    pub keybindings: KeyBindings,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            reading_mode: ReadingMode::LTR,
            keybindings: KeyBindings::default(),
        }
    }
}

impl Config {
    pub fn config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Impossible de trouver le dossier config"))?;
        Ok(config_dir.join("comic-reader").join("config.json"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        let content = serde_json::to_string_pretty(&self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}