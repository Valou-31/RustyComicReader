use crate::app::{ComicApp, ReadingMode};
use crate::input::keybindings::{Action, Preset};
use crate::ui::layout::LayoutConfig;
use crate::ui::theme::ThemePreset;
use egui::{Color32, Context};

/// Draws the settings window (reading direction, key remapping, presets)
/// when `app.show_settings` is set. No-op otherwise.
pub fn draw_settings(ctx: &Context, app: &mut ComicApp) {
    if !app.show_settings {
        return;
    }

    let mut open = true;
    egui::Window::new("⚙ Settings")
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.heading("Reading Direction");
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(app.reading_mode == ReadingMode::LTR, "➡ LTR (Western)")
                    .clicked()
                    && app.reading_mode != ReadingMode::LTR
                {
                    app.toggle_reading_mode();
                }
                if ui
                    .selectable_label(app.reading_mode == ReadingMode::RTL, "⬅ RTL (Manga)")
                    .clicked()
                    && app.reading_mode != ReadingMode::RTL
                {
                    app.toggle_reading_mode();
                }
            });

            ui.separator();
            ui.heading("Key Remapping");

            if let Some(action) = app.remapping_action {
                ui.colored_label(
                    Color32::from_rgb(230, 180, 40),
                    format!("Press a key to bind to \"{}\"… (Esc to cancel)", action.label()),
                );
            }

            for action in Action::ALL {
                ui.horizontal(|ui| {
                    ui.label(action.label());

                    let keys = app.keybindings.bindings.get(&action).cloned().unwrap_or_default();
                    for key in keys {
                        if draw_key_chip(ui, &key) {
                            app.keybindings.remove_key(action, &key);
                            app.save_config();
                        }
                    }

                    let remapping_this = app.remapping_action == Some(action);
                    if ui
                        .add_enabled(!remapping_this, egui::Button::new("+ Add Key"))
                        .clicked()
                    {
                        app.remapping_action = Some(action);
                    }
                });
            }
            ui.label("Fullscreen (F) is not remappable.");

            ui.separator();
            ui.heading("Presets");
            ui.horizontal(|ui| {
                for preset in Preset::ALL {
                    if ui.button(preset.label()).clicked() {
                        app.keybindings = preset.bindings();
                        app.remapping_action = None;
                        app.save_config();
                    }
                }
            });

            ui.separator();
            ui.heading("Theme");
            ui.horizontal(|ui| {
                for preset in ThemePreset::ALL {
                    if ui.selectable_label(app.theme_preset == preset, preset.label()).clicked()
                        && app.theme_preset != preset
                    {
                        app.theme_preset = preset;
                        app.save_config();
                    }
                }
            });

            ui.separator();
            ui.heading("Layout");
            let mut layout_changed = false;
            ui.horizontal(|ui| {
                ui.label("Spine width");
                layout_changed |= ui.add(egui::Slider::new(&mut app.layout.spine_width, 0.0..=6.0)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("Spine opacity");
                layout_changed |= ui.add(egui::Slider::new(&mut app.layout.spine_opacity, 0.0..=1.0)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("Page gap");
                layout_changed |= ui.add(egui::Slider::new(&mut app.layout.page_gap, 0.0..=80.0)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("Fade speed (ms)");
                layout_changed |= ui.add(egui::Slider::new(&mut app.layout.fade_duration_ms, 50..=1000)).changed();
            });
            if ui.button("Reset layout to defaults").clicked() {
                app.layout = LayoutConfig::default();
                layout_changed = true;
            }
            if layout_changed {
                app.save_config();
            }

            ui.separator();
            if ui.button("Close").clicked() {
                app.show_settings = false;
            }
        });

    if !open {
        app.show_settings = false;
        app.remapping_action = None;
    }
}

/// Draws a bound key as a single rounded chip — the key text with a small
/// "×" at the end, both inside one bordered rectangle instead of a separate
/// label + button. Returns `true` if the "×" was clicked (caller removes it).
fn draw_key_chip(ui: &mut egui::Ui, key: &str) -> bool {
    let mut remove = false;
    egui::Frame::NONE
        .fill(ui.visuals().widgets.inactive.weak_bg_fill)
        .stroke(ui.visuals().widgets.inactive.bg_stroke)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(key);
                if ui
                    .add(egui::Button::new(egui::RichText::new("×").size(13.0)).small().frame(false))
                    .clicked()
                {
                    remove = true;
                }
            });
        });
    remove
}
