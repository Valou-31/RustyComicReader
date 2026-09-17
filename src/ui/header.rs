use crate::app::{ComicApp, UI_HIDE_DELAY, UpdateStatus};
use egui::Ui;

/// Draws the top bar (filename, live page numbers, peek badge, controls) and
/// the page-progress bar right below it for an open comic. Fades out after
/// `UI_HIDE_DELAY` of no mouse movement and fades back in as soon as the
/// mouse moves — skips layout entirely once fully hidden, so it doesn't
/// intercept clicks meant for the reader below.
pub fn draw_header(ui: &mut Ui, app: &mut ComicApp) {
    let visible = app.idle_time() < UI_HIDE_DELAY;
    let alpha = ui.ctx().animate_bool_with_time(
        egui::Id::new("header_fade"),
        visible,
        app.layout.fade_duration().as_secs_f32(),
    );

    if alpha <= 0.01 {
        return;
    }

    ui.scope(|ui| {
        ui.set_opacity(alpha);

        let secondary = app.theme_preset.theme().text_secondary;
        ui.horizontal(|ui| {
            ui.label(&app.filename);
            ui.colored_label(secondary, page_label(app));

            if app.page_offset > 0 {
                ui.colored_label(
                    egui::Color32::from_rgb(230, 180, 40),
                    format!("👁 peek +{}", app.page_offset),
                );
            }

            if ui.button("Load File").clicked() {
                app.start_loading_file();
            }
            if !app.file_queue.is_empty() && ui.button(format!("▶ Next ({})", app.file_queue.len())).clicked() {
                app.open_next_in_queue();
            }
            if ui.button(app.reading_mode.label()).clicked() {
                app.toggle_reading_mode();
            }
            if app.is_zoomed() && ui.button("🔍 Reset Zoom").clicked() {
                app.reset_zoom();
            }
            if ui
                .selectable_label(app.zoom_locked, "🔒 Lock Zoom")
                .on_hover_text(
                    "Keep the current zoom level instead of resetting it to normal on every page \
                     turn or when opening a different book.",
                )
                .clicked()
            {
                app.zoom_locked = !app.zoom_locked;
                app.save_config();
            }
            if ui.button("⚙ Settings").clicked() {
                app.show_settings = true;
            }
            if ui.button("🕘 History").clicked() {
                app.show_history = true;
            }

            draw_update_indicator(ui, app);

            ui.separator();
            ui.colored_label(secondary, "🌙");
            if ui
                .add(egui::Slider::new(&mut app.blue_light_filter, 0.0..=1.0).show_value(false))
                .on_hover_text("Blue light filter")
                .changed()
            {
                app.save_config();
            }
        });

        ui.separator();
        crate::ui::progress_bar::draw_progress_bar(ui, app);
        ui.add_space(4.0);
    });
}

/// A small button that only appears when there's something to act on — a
/// newer release found, mid-download, ready to apply, or failed. Silent the
/// rest of the time (idle, or still checking) so it doesn't clutter the
/// header on every launch. Shared between the reading header and the
/// empty-state screen, since an update can be found before any book is open.
pub fn draw_update_indicator(ui: &mut Ui, app: &mut ComicApp) {
    match app.update_status.clone() {
        UpdateStatus::Available(info) => {
            if ui.button(format!("⬆ Update to v{}", info.version)).clicked() {
                app.start_update_download();
            }
        }
        UpdateStatus::Downloading => {
            ui.add_enabled(false, egui::Button::new("⬇ Downloading update…"));
        }
        UpdateStatus::Ready => {
            if ui
                .button("🔄 Restart to update")
                .on_hover_text("Update downloaded — restart to apply it")
                .clicked()
            {
                app.restart_to_apply_update();
            }
        }
        UpdateStatus::Failed(err) => {
            if ui.button("⚠ Update failed").on_hover_text(format!("{err}\n\nClick to retry")).clicked() {
                app.check_for_updates();
            }
        }
        UpdateStatus::Idle | UpdateStatus::Checking => {}
    }
}

fn page_label(app: &ComicApp) -> String {
    match (app.left_page(), app.right_page()) {
        (Some(l), Some(r)) => format!("Page {}-{} / {}", l + 1, r + 1, app.total_pages),
        (Some(p), None) | (None, Some(p)) => format!("Page {} / {}", p + 1, app.total_pages),
        (None, None) => format!("— / {}", app.total_pages),
    }
}
