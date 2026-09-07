use crate::app::{ComicApp, ReadingMode};
use crate::input::keybindings::{Action, Preset};
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
                        ui.label(&key);
                        if ui.small_button("✕").clicked() {
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
            if ui.button("Close").clicked() {
                app.show_settings = false;
            }
        });

    if !open {
        app.show_settings = false;
        app.remapping_action = None;
    }
}
