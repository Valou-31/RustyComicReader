use crate::app::{ReadingMode, WEBTOON_SCROLL_SPEED_DOC, ZoomTarget, ZOOM_MIN};
use crate::input::keybindings::KeyBindings;
use crate::ui::layout::LayoutConfig;
use crate::ui::theme::ThemePreset;
use anyhow::Result;
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub reading_mode: ReadingMode,
    pub webtoon_page_width_pct: f32,
    pub webtoon_scroll_speed: f32,
    pub webtoon_wheel_sensitivity: f32,
    pub webtoon_scroll_inverted: bool,
    pub webtoon_page_gap: f32,
    pub keybindings: KeyBindings,
    pub theme: ThemePreset,
    pub layout: LayoutConfig,
    pub blue_light_filter: f32,
    pub scroll_inverted: bool,
    pub scroll_sensitivity: f32,
    pub one_turn_per_swipe: bool,
    pub show_page_preview: bool,
    pub show_fore_edge: bool,
    pub zoom_spread: f32,
    pub zoom_left: f32,
    pub zoom_right: f32,
    pub zoom_locked: bool,
    pub zoom_target: ZoomTarget,
    pub downscale_large_pages: bool,
    pub resume_last_session: bool,
    pub auto_check_updates: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            reading_mode: ReadingMode::LTR,
            webtoon_page_width_pct: 70.0,
            webtoon_scroll_speed: WEBTOON_SCROLL_SPEED_DOC,
            webtoon_wheel_sensitivity: 1.0,
            webtoon_scroll_inverted: false,
            webtoon_page_gap: 0.0,
            keybindings: KeyBindings::default(),
            theme: ThemePreset::default(),
            layout: LayoutConfig::default(),
            blue_light_filter: 0.0,
            scroll_inverted: false,
            scroll_sensitivity: 1.0,
            one_turn_per_swipe: true,
            show_page_preview: true,
            show_fore_edge: true,
            zoom_spread: ZOOM_MIN,
            zoom_left: ZOOM_MIN,
            zoom_right: ZOOM_MIN,
            zoom_locked: false,
            zoom_target: ZoomTarget::Spread,
            downscale_large_pages: true,
            resume_last_session: true,
            auto_check_updates: true,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
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

/// The web build has no `dirs::config_dir()`/real filesystem to persist
/// to — settings simply reset to defaults every session (see the
/// project-level decision to drop persistence for the first web version).
#[cfg(target_arch = "wasm32")]
impl Config {
    pub fn load() -> Result<Self> {
        Ok(Self::default())
    }

    pub fn save(&self) -> Result<()> {
        Ok(())
    }
}
