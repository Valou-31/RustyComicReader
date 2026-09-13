use crate::app::ComicApp;
use egui::Context;
use std::path::PathBuf;

/// Draws the reading-history window when `app.show_history` is set. No-op
/// otherwise. Mirrors `ui::settings::draw_settings`'s window/close pattern.
pub fn draw_history(ctx: &Context, app: &mut ComicApp) {
    if !app.show_history {
        return;
    }

    let mut open = true;
    let mut to_open: Option<PathBuf> = None;
    let mut to_remove: Option<PathBuf> = None;

    egui::Window::new("🕘 Reading History")
        .open(&mut open)
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| {
            if app.history.entries.is_empty() {
                ui.label("Nothing read yet.");
            }

            for entry in &app.history.entries {
                ui.horizontal(|ui| {
                    let missing = !entry.path.exists();
                    ui.add_enabled_ui(!missing, |ui| {
                        if ui.button(&entry.filename).clicked() {
                            to_open = Some(entry.path.clone());
                        }
                    });

                    let progress = if entry.total_pages > 0 {
                        format!("Page {} / {}", entry.last_page + 1, entry.total_pages)
                    } else {
                        String::new()
                    };
                    ui.colored_label(ui.visuals().weak_text_color(), progress);

                    if missing {
                        ui.colored_label(egui::Color32::from_rgb(220, 80, 80), "file not found");
                    }

                    if ui.small_button("✕").clicked() {
                        to_remove = Some(entry.path.clone());
                    }
                });
            }

            ui.separator();
            if ui.button("Close").clicked() {
                app.show_history = false;
            }
        });

    if !open {
        app.show_history = false;
    }

    if let Some(path) = to_open {
        app.show_history = false;
        app.start_loading_path(path);
    }
    if let Some(path) = to_remove {
        app.history.remove(&path);
        if let Err(err) = app.history.save() {
            tracing::warn!("Failed to save history: {err}");
        }
    }
}
