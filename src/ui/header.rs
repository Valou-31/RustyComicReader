use crate::app::{ComicApp, UI_FADE_DURATION, UI_HIDE_DELAY};
use egui::Ui;

/// Draws the top bar (filename, live page numbers, peek badge, controls) for
/// an open comic. Fades out after `UI_HIDE_DELAY` of no mouse movement and
/// fades back in as soon as the mouse moves — skips layout entirely once
/// fully hidden, so it doesn't intercept clicks meant for the reader below.
pub fn draw_header(ui: &mut Ui, app: &mut ComicApp) {
    let visible = app.idle_time() < UI_HIDE_DELAY;
    let alpha = ui.ctx().animate_bool_with_time(
        egui::Id::new("header_fade"),
        visible,
        UI_FADE_DURATION.as_secs_f32(),
    );

    if alpha <= 0.01 {
        return;
    }

    ui.scope(|ui| {
        ui.set_opacity(alpha);

        ui.horizontal(|ui| {
            ui.label(&app.filename);
            ui.label(page_label(app));

            if app.page_offset > 0 {
                ui.colored_label(
                    egui::Color32::from_rgb(230, 180, 40),
                    format!("👁 peek +{}", app.page_offset),
                );
            }

            if ui.button("Load File").clicked() {
                app.start_loading_file();
            }
            if ui.button(app.reading_mode.label()).clicked() {
                app.toggle_reading_mode();
            }
        });

        ui.separator();
    });
}

fn page_label(app: &ComicApp) -> String {
    match (app.left_page(), app.right_page()) {
        (Some(l), Some(r)) => format!("Page {}-{} / {}", l + 1, r + 1, app.total_pages),
        (Some(p), None) | (None, Some(p)) => format!("Page {} / {}", p + 1, app.total_pages),
        (None, None) => format!("— / {}", app.total_pages),
    }
}
