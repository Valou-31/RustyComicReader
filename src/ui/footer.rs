use crate::app::{ComicApp, UI_HIDE_DELAY};
use crate::ui::layout::ToolbarArea;
use egui::{Align2, Context};

/// Draws the floating footer bar — a second toolbar location pinned to the
/// bottom edge of the window, independent of the header at the top, and
/// floating above the reader rather than reserving space from it. Empty by
/// default; only appears once at least one control is placed in
/// `ToolbarArea::Footer`. Fades with the same idle timer as the header.
/// Only ever called outside `app.toolbar_edit_mode` — editing has its own
/// full-screen replacement for the header/reader/footer entirely, see
/// `ui::toolbar::draw_toolbar_editor`.
pub fn draw_footer(ctx: &Context, app: &mut ComicApp) {
    let row_count = app.layout.toolbar_row_count(ToolbarArea::Footer);
    if row_count == 0 {
        return;
    }
    // A floating `Order::Foreground` area always draws above a plain
    // `egui::Window` (`Order::Middle`, what Settings/History/Bookmarks use)
    // regardless of call order, so it would otherwise mask part of
    // whichever of those is open.
    if app.any_modal_panel_open() {
        return;
    }

    let visible = app.idle_time() < UI_HIDE_DELAY;
    let alpha =
        ctx.animate_bool_with_time(egui::Id::new("footer_fade"), visible, app.layout.fade_duration().as_secs_f32());
    if alpha <= 0.01 {
        return;
    }

    let screen_width = ctx.input(|i| i.viewport_rect()).width();
    let bg = app.layout.menu_bg_fill(app.theme_preset.theme().bg, alpha);

    egui::Area::new(egui::Id::new("toolbar_footer"))
        .order(egui::Order::Foreground)
        .anchor(Align2::LEFT_BOTTOM, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(screen_width);
            egui::Frame::NONE.fill(bg).inner_margin(4.0).show(ui, |ui| {
                ui.set_width(screen_width - 8.0);
                ui.set_opacity(app.layout.menu_content_opacity(alpha));
                ui.separator();
                for row in 0..row_count.max(1) {
                    crate::ui::toolbar::draw_toolbar_row(
                        ui,
                        app,
                        ToolbarArea::Footer,
                        row,
                        crate::ui::header::draw_toolbar_item,
                    );
                }
            });
        });
}
