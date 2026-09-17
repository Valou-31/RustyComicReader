use crate::app::ComicApp;
use egui::Context;
use std::path::PathBuf;

/// Draws the bookmarks window when `app.show_bookmarks` is set. No-op
/// otherwise. Mirrors `ui::history::draw_history`'s window/close pattern.
pub fn draw_bookmarks(ctx: &Context, app: &mut ComicApp) {
    if !app.show_bookmarks {
        return;
    }

    let mut open = true;
    let mut to_open: Option<(PathBuf, usize)> = None;
    let mut to_remove: Option<(PathBuf, usize)> = None;

    egui::Window::new("Bookmarks")
        .open(&mut open)
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| {
            if app.bookmarks.entries.is_empty() {
                ui.label("No bookmarks yet — use the Bookmark button while reading to add one.");
            }

            for bookmark in &app.bookmarks.entries {
                ui.horizontal(|ui| {
                    let missing = !bookmark.path.exists();
                    ui.add_enabled_ui(!missing, |ui| {
                        if ui.button(&bookmark.filename).clicked() {
                            to_open = Some((bookmark.path.clone(), bookmark.page));
                        }
                    });

                    let progress = if bookmark.total_pages > 0 {
                        format!("Page {} / {}", bookmark.page + 1, bookmark.total_pages)
                    } else {
                        String::new()
                    };
                    ui.colored_label(ui.visuals().weak_text_color(), progress);

                    if missing {
                        ui.colored_label(egui::Color32::from_rgb(220, 80, 80), "file not found");
                    }

                    if ui.small_button("Remove").clicked() {
                        to_remove = Some((bookmark.path.clone(), bookmark.page));
                    }
                });
            }

            ui.separator();
            if ui.button("Close").clicked() {
                app.show_bookmarks = false;
            }
        });

    if !open {
        app.show_bookmarks = false;
    }

    if let Some((path, page)) = to_open {
        app.show_bookmarks = false;
        if app.current_path.as_deref() == Some(path.as_path()) {
            // Already open: no need to reload, just jump.
            app.jump_to_page(page);
        } else {
            app.start_loading_path_at(path, page);
        }
    }
    if let Some((path, page)) = to_remove {
        app.bookmarks.remove_at(&path, page);
        if let Err(err) = app.bookmarks.save() {
            tracing::warn!("Failed to save bookmarks: {err}");
        }
    }
}
