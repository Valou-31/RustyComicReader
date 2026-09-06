use egui::Color32;

#[derive(Clone, Debug)]
pub struct Theme {
    pub bg_dark: Color32,
    pub accent: Color32,
    pub separator: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub button_hover: Color32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            bg_dark: Color32::from_rgb(15, 23, 42),
            accent: Color32::from_rgb(6, 182, 212),
            separator: Color32::from_rgba_unmultiplied(148, 163, 184, 77),
            text_primary: Color32::from_rgb(226, 232, 240),
            text_secondary: Color32::from_rgb(148, 163, 184),
            button_hover: Color32::from_rgb(14, 165, 233),
        }
    }
}