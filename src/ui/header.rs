use crate::app::{ComicApp, UI_HIDE_DELAY, UpdateStatus};
use crate::ui::layout::{ToolbarArea, ToolbarItem};
use egui::{Align2, Context, Ui};

/// Draws the top bar in its normal, space-reserving form: a fixed
/// filename/page-number line, then every row placed in
/// `ToolbarArea::Header` (see `ui::toolbar`), then the page-progress bar —
/// pushing the reader down to make room for it rather than floating over
/// it (that's `draw_header_overlay`, used instead when
/// `app.layout.header_floats_over_reader` is on). Fades out after
/// `UI_HIDE_DELAY` of no mouse movement and fades back in as soon as the
/// mouse moves — skips layout entirely once fully hidden, so it doesn't
/// intercept clicks meant for the reader below. Only ever called outside
/// `app.toolbar_edit_mode` — editing has its own full-screen replacement
/// for the header/reader/footer entirely, see
/// `ui::toolbar::draw_toolbar_editor`.
pub fn draw_header(ui: &mut Ui, app: &mut ComicApp) {
    let Some(fade_alpha) = header_fade_alpha(ui.ctx(), app) else { return };

    egui::Frame::default().inner_margin(egui::Margin::symmetric(10, 0)).show(ui, |ui| {
        draw_header_content(ui, app, fade_alpha);
    });
}

/// Draws the top bar floating over the reader, in its own `egui::Area`
/// (`Order::Foreground`, anchored to the top edge) instead of the normal
/// document flow — used in place of `draw_header` when
/// `app.layout.header_floats_over_reader` is on, so the page always fills
/// the whole window instead of shrinking to make room for the header
/// whenever it's shown. Otherwise identical: same content, same idle-hide
/// fade, same "never called while editing" rule. Mirrors
/// `ui::footer::draw_footer`'s existing floating pattern.
pub fn draw_header_overlay(ctx: &Context, app: &mut ComicApp) {
    // A floating `Order::Foreground` area always draws above a plain
    // `egui::Window` (`Order::Middle`, what Settings/History/Bookmarks use)
    // regardless of call order, so it would otherwise mask part of
    // whichever of those is open. `draw_header` (the non-floating form)
    // doesn't need this — normal content already sits below `Middle`.
    if app.any_modal_panel_open() {
        return;
    }
    let Some(fade_alpha) = header_fade_alpha(ctx, app) else { return };

    let screen_width = ctx.input(|i| i.viewport_rect()).width();
    let bg = app.layout.menu_bg_fill(app.theme_preset.theme().bg, fade_alpha);

    egui::Area::new(egui::Id::new("header_overlay"))
        .order(egui::Order::Foreground)
        .anchor(Align2::LEFT_TOP, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(screen_width);
            egui::Frame::NONE.fill(bg).inner_margin(egui::Margin::symmetric(10, 4)).show(ui, |ui| {
                ui.set_width(screen_width - 20.0);
                draw_header_content(ui, app, fade_alpha);
            });
        });
}

/// Whether the header should render at all this frame and, if so, at what
/// idle-hide fade (`0.0`..=`1.0`) — shared by `draw_header` and
/// `draw_header_overlay` so the two stay in lockstep (same timer, same
/// `Id`, same early-out once fully hidden).
fn header_fade_alpha(ctx: &Context, app: &ComicApp) -> Option<f32> {
    let visible = app.idle_time() < UI_HIDE_DELAY;
    let alpha =
        ctx.animate_bool_with_time(egui::Id::new("header_fade"), visible, app.layout.fade_duration().as_secs_f32());
    (alpha > 0.01).then_some(alpha)
}

/// The header's actual content — filename/page line, toolbar rows,
/// progress bar — shared by `draw_header` (normal, space-reserving) and
/// `draw_header_overlay` (floating over the page). `fade_alpha` is the
/// idle-hide fade in `0.0`..=`1.0`; button/text opacity is
/// `app.layout.menu_content_opacity(fade_alpha)` (which folds in the
/// user's configured floor-respecting button opacity), not `fade_alpha`
/// directly — the background panel's own opacity is handled separately by
/// whoever calls this (a `Frame` fill in `draw_header_overlay`; `draw_header`
/// has no fill at all, since it never overlaps anything that would need
/// one).
fn draw_header_content(ui: &mut Ui, app: &mut ComicApp, fade_alpha: f32) {
    ui.set_opacity(app.layout.menu_content_opacity(fade_alpha));

    let secondary = app.theme_preset.theme().text_secondary;
    ui.horizontal(|ui| {
        ui.label(&app.filename);
        ui.colored_label(secondary, page_label(app));

        if app.is_current_page_isolated() {
            ui.colored_label(egui::Color32::from_rgb(230, 180, 40), "🔀 isolated");
        }
    });

    let row_count = app.layout.toolbar_row_count(ToolbarArea::Header);
    for row in 0..row_count.max(1) {
        crate::ui::toolbar::draw_toolbar_row(ui, app, ToolbarArea::Header, row, draw_toolbar_item);
    }

    ui.separator();
    crate::ui::progress_bar::draw_progress_bar(ui, app);
    ui.add_space(4.0);
}

/// Draws one entry of `app.layout.toolbar_items` — see `ToolbarItem`. Shared
/// by `ui::footer`, since footer items behave identically to header ones.
pub(crate) fn draw_toolbar_item(ui: &mut Ui, app: &mut ComicApp, item: ToolbarItem) {
    match item {
        ToolbarItem::Separator => {
            ui.separator();
        }
        ToolbarItem::LoadFile => {
            if ui.button("📂 Load File").clicked() {
                app.start_loading_file();
            }
        }
        ToolbarItem::NextInQueue => {
            if !app.file_queue.is_empty() && ui.button(format!("▶ Next ({})", app.file_queue.len())).clicked() {
                app.open_next_in_queue();
            }
        }
        ToolbarItem::ReadingMode => {
            if ui.button(app.reading_mode.label()).clicked() {
                app.toggle_reading_mode();
            }
        }
        ToolbarItem::Bookmark => {
            // Icon-only ("🔖" alone) didn't read clearly, so it keeps its
            // label like every other toolbar control — an icon on its own
            // depends entirely on that one glyph rendering recognizably,
            // with no fallback if it doesn't.
            if ui
                .selectable_label(app.is_current_page_bookmarked(), "🔖 Bookmark")
                .on_hover_text("Bookmark this spread, so you can jump straight back to it later.")
                .clicked()
            {
                app.toggle_bookmark();
            }
        }
        ToolbarItem::ResetZoom => {
            if app.is_zoomed() && ui.button("Reset Zoom").clicked() {
                app.reset_zoom();
            }
        }
        ToolbarItem::LockZoom => {
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
        }
        ToolbarItem::Settings => {
            if ui.button("⚙ Settings").clicked() {
                app.show_settings = true;
            }
        }
        ToolbarItem::History => {
            if ui.button("🕘 History").clicked() {
                app.show_history = true;
            }
        }
        ToolbarItem::Bookmarks => {
            if ui.button("📑 Bookmarks").clicked() {
                app.show_bookmarks = true;
            }
        }
        ToolbarItem::UpdateStatus => draw_update_indicator(ui, app),
        ToolbarItem::BlueLightFilter => {
            let secondary = app.theme_preset.theme().text_secondary;
            ui.colored_label(secondary, "🌙");
            if ui
                .add(egui::Slider::new(&mut app.blue_light_filter, 0.0..=1.0).show_value(false))
                .on_hover_text("Blue light filter")
                .changed()
            {
                app.save_config();
            }
        }
    }
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
