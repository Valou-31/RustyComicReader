use crate::app::ReadingMode;
use crate::input::keybindings::KeyBindings;
use crate::ui::layout::LayoutConfig;
use crate::ui::theme::ThemePreset;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub reading_mode: ReadingMode,
    pub keybindings: KeyBindings,
    pub theme: ThemePreset,
    pub layout: LayoutConfig,
    pub blue_light_filter: f32,
    pub scroll_inverted: bool,
    pub scroll_sensitivity: f32,
    pub one_turn_per_swipe: bool,
    pub show_page_preview: bool,
    pub downscale_large_pages: bool,
    pub resume_last_session: bool,
    pub auto_check_updates: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            reading_mode: ReadingMode::LTR,
            keybindings: KeyBindings::default(),
            theme: ThemePreset::default(),
            layout: LayoutConfig::default(),
            blue_light_filter: 0.0,
            scroll_inverted: false,
            scroll_sensitivity: 1.0,
            one_turn_per_swipe: true,
            show_page_preview: true,
            downscale_large_pages: true,
            resume_last_session: true,
            auto_check_updates: true,
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
