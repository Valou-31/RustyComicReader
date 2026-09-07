use egui::{Color32, Visuals};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemePreset {
    Dark,
    Light,
    Midnight,
    Sepia,
}

impl Default for ThemePreset {
    fn default() -> Self {
        ThemePreset::Dark
    }
}

impl ThemePreset {
    pub const ALL: [ThemePreset; 4] =
        [ThemePreset::Dark, ThemePreset::Light, ThemePreset::Midnight, ThemePreset::Sepia];

    pub fn label(&self) -> &'static str {
        match self {
            ThemePreset::Dark => "🌑 Dark",
            ThemePreset::Light => "☀ Light",
            ThemePreset::Midnight => "🌌 Midnight",
            ThemePreset::Sepia => "📜 Sepia",
        }
    }

    pub fn theme(&self) -> Theme {
        match self {
            ThemePreset::Dark => Theme {
                dark_mode: true,
                bg: Color32::from_rgb(24, 24, 27),
                panel: Color32::from_rgb(15, 23, 42),
                accent: Color32::from_rgb(6, 182, 212),
                separator: Color32::from_rgb(148, 163, 184),
                text_primary: Color32::from_rgb(226, 232, 240),
                text_secondary: Color32::from_rgb(148, 163, 184),
                button_hover: Color32::from_rgb(14, 165, 233),
            },
            ThemePreset::Light => Theme {
                dark_mode: false,
                bg: Color32::from_rgb(250, 250, 250),
                panel: Color32::from_rgb(240, 240, 243),
                accent: Color32::from_rgb(37, 99, 235),
                separator: Color32::from_rgb(100, 100, 110),
                text_primary: Color32::from_rgb(24, 24, 27),
                text_secondary: Color32::from_rgb(90, 90, 100),
                button_hover: Color32::from_rgb(59, 130, 246),
            },
            ThemePreset::Midnight => Theme {
                dark_mode: true,
                bg: Color32::from_rgb(8, 8, 16),
                panel: Color32::from_rgb(4, 4, 10),
                accent: Color32::from_rgb(129, 90, 224),
                separator: Color32::from_rgb(90, 80, 140),
                text_primary: Color32::from_rgb(220, 215, 240),
                text_secondary: Color32::from_rgb(140, 130, 170),
                button_hover: Color32::from_rgb(129, 90, 224),
            },
            ThemePreset::Sepia => Theme {
                dark_mode: false,
                bg: Color32::from_rgb(240, 224, 195),
                panel: Color32::from_rgb(224, 205, 169),
                accent: Color32::from_rgb(140, 92, 42),
                separator: Color32::from_rgb(120, 95, 60),
                text_primary: Color32::from_rgb(59, 41, 21),
                text_secondary: Color32::from_rgb(100, 76, 48),
                button_hover: Color32::from_rgb(160, 110, 55),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub dark_mode: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub accent: Color32,
    pub separator: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub button_hover: Color32,
}

impl Theme {
    /// Applies this palette to egui's global visuals for the current frame —
    /// called once per frame, before any widgets are drawn, so every default
    /// widget (buttons, windows, sliders...) picks it up automatically.
    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = if self.dark_mode { Visuals::dark() } else { Visuals::light() };
        visuals.override_text_color = Some(self.text_primary);
        visuals.panel_fill = self.panel;
        visuals.window_fill = self.panel;
        visuals.faint_bg_color = self.bg;
        visuals.extreme_bg_color = self.bg;
        visuals.selection.bg_fill = self.accent;
        visuals.hyperlink_color = self.accent;
        visuals.widgets.hovered.bg_fill = self.button_hover;
        visuals.widgets.hovered.weak_bg_fill = self.button_hover;
        visuals.widgets.active.bg_fill = self.accent;
        ctx.set_visuals(visuals);
    }
}
