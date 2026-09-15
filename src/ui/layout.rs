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
    /// Opacity of the spine line, `0.0`..=`1.0` — also the peak opacity of
    /// the spine shadow gradient, at its center.
    pub spine_opacity: f32,
    /// How far the spine's shadow gradient fades out on each side, in
    /// points — mimics the way a physical book's pages curve away into
    /// shadow near the binding. `0.0` disables it, leaving just the line.
    pub spine_shadow_width: f32,
    /// Color of the spine line and shadow, independent of the active theme.
    pub spine_color: [u8; 3],
    /// The last few distinct colors picked for `spine_color`, most recent
    /// first, for quick reuse — see `record_spine_color`.
    pub spine_color_history: Vec<[u8; 3]>,
    /// Gap in points between the two pages. `0.0` keeps them touching
    /// (the "sticked together" book-spread look).
    pub page_gap: f32,
    /// Duration in milliseconds of the header show/hide fade.
    pub fade_duration_ms: u64,
    /// Duration in milliseconds of a full page-turn slide (keyboard/queue
    /// navigation, or a trackpad drag's commit/cancel settle).
    pub page_transition_ms: u64,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            spine_width: 1.0,
            spine_opacity: 0.24,
            spine_shadow_width: 28.0,
            spine_color: [20, 20, 20],
            spine_color_history: Vec::new(),
            page_gap: 0.0,
            fade_duration_ms: 300,
            page_transition_ms: 220,
        }
    }
}

impl LayoutConfig {
    pub fn fade_duration(&self) -> Duration {
        Duration::from_millis(self.fade_duration_ms)
    }

    pub fn page_transition_duration(&self) -> Duration {
        Duration::from_millis(self.page_transition_ms)
    }

    /// Records `color` as the most recently used spine color, moving it to
    /// the front if it's already in the history, capped at 5 entries.
    pub fn record_spine_color(&mut self, color: [u8; 3]) {
        self.spine_color_history.retain(|&c| c != color);
        self.spine_color_history.insert(0, color);
        self.spine_color_history.truncate(5);
    }
}
