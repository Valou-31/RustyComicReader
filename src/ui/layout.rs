use serde::{Deserialize, Serialize};
use std::time::Duration;

/// User-tunable layout knobs for the reader (README: "Configurable Layout —
/// separator width/opacity, spacing, transition speed").
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
    /// Width in points of the spine line painted between the two pages.
    /// `0.0` hides it entirely.
    pub spine_width: f32,
    /// Opacity of the spine line, `0.0`..=`1.0`.
    pub spine_opacity: f32,
    /// Gap in points between the two pages. `0.0` keeps them touching
    /// (the "sticked together" book-spread look).
    pub page_gap: f32,
    /// Duration in milliseconds of the header show/hide fade.
    pub fade_duration_ms: u64,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            spine_width: 1.0,
            spine_opacity: 0.24,
            page_gap: 0.0,
            fade_duration_ms: 300,
        }
    }
}

impl LayoutConfig {
    pub fn fade_duration(&self) -> Duration {
        Duration::from_millis(self.fade_duration_ms)
    }
}
