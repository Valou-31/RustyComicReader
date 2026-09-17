use crate::app::ComicApp;
use egui::{Align2, Area, Id, Order, Pos2, Rect, Sense, Stroke, Ui, Vec2};

/// Height of the progress bar's track, in points.
const BAR_HEIGHT: f32 = 10.0;
/// Gap, in points, between the bar and the bottom of the preview popup.
const PREVIEW_GAP: f32 = 18.0;
/// Size the preview popup's image area is always held to, whether it's
/// showing the thumbnail or still waiting on one — computed and applied
/// explicitly (`ui.set_min_size` + a manually aspect-scaled `Image`) rather
/// than left to `Image::shrink_to_fit`, whose "available space" inside a
/// freshly created floating `Area` isn't something we control, so relying
/// on it made the box's actual on-screen size unpredictable.
const PREVIEW_IMAGE_SIZE: Vec2 = Vec2::new(320.0, 460.0);

/// Draws the page-progress bar: a thin track filled up to the current
/// page's fraction of the book (page 35 of 100 fills 35%, regardless of
/// reading direction). Hovering previews the page under the cursor —
/// decoded off the UI thread by `comic::thumbnail`, so it never blocks a
/// frame — and clicking jumps straight to it. Meant to be called from
/// inside `draw_header`'s fade scope, right below the header proper, so it
/// shows and hides along with it rather than staying pinned while reading.
///
/// The preview is a hand-rolled `Area`, not `Response::on_hover_ui`/egui's
/// built-in tooltip: tooltips default to a ~0.5s delay and only appear once
/// the pointer holds still (`Style::interaction::tooltip_delay`/
/// `show_tooltips_only_when_still`), which is right for a button's hover
/// hint but wrong for a scrubber — the whole point here is a preview that
/// tracks the cursor continuously while dragging across the bar, with no
/// delay at all.
pub fn draw_progress_bar(ui: &mut Ui, app: &mut ComicApp) {
    if app.total_pages == 0 {
        return;
    }

    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, BAR_HEIGHT), Sense::click());

    let theme = app.theme_preset.theme();
    let painter = ui.painter();
    painter.rect_filled(rect, BAR_HEIGHT / 2.0, theme.panel);

    if let Some(page_number) = app.left_page().or(app.right_page()).map(|p| p + 1) {
        let fraction = (page_number as f32 / app.total_pages as f32).clamp(0.0, 1.0);
        let fill_width = rect.width() * fraction;
        if fill_width > 0.0 {
            let fill_rect = Rect::from_min_size(rect.left_top(), Vec2::new(fill_width, rect.height()));
            painter.rect_filled(fill_rect, BAR_HEIGHT / 2.0, theme.accent);
        }
    }

    let Some(pos) = response.hover_pos() else { return };
    let hover_fraction = ((pos.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
    let hover_page = ((hover_fraction * app.total_pages as f32) as usize).min(app.total_pages - 1);

    ui.painter().line_segment(
        [Pos2::new(pos.x, rect.top()), Pos2::new(pos.x, rect.bottom())],
        Stroke::new(1.5, theme.text_primary),
    );

    Area::new(Id::new("progress_bar_preview"))
        .fixed_pos(Pos2::new(pos.x, rect.top() - PREVIEW_GAP))
        .pivot(Align2::CENTER_BOTTOM)
        .order(Order::Tooltip)
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_size(PREVIEW_IMAGE_SIZE);
                ui.vertical_centered(|ui| {
                    // The sharp on-demand preview, if it's landed yet; the
                    // cheap whole-book one otherwise — so there's near
                    // always *something* to show instantly, upgraded to
                    // the sharp version a few milliseconds later.
                    let hires = app.thumbnail_hires_textures.get(&hover_page).cloned();
                    if hires.is_none() {
                        app.request_thumbnail(hover_page);
                    }
                    let lowres = app.thumbnail_textures.get(&hover_page).cloned();

                    match hires.or(lowres) {
                        Some(texture) => {
                            let size = scale_to_fit(texture.size_vec2(), PREVIEW_IMAGE_SIZE);
                            ui.add(egui::Image::new(&texture).fit_to_exact_size(size));
                        }
                        None => {
                            ui.add_space((PREVIEW_IMAGE_SIZE.y - 32.0) / 2.0);
                            ui.add(egui::Spinner::new().size(32.0));
                            // Keep repainting while waiting so the preview
                            // appears the instant the background decode
                            // lands, rather than only on the next mouse move.
                            ui.ctx().request_repaint();
                        }
                    }
                    ui.label(format!("Page {}", hover_page + 1));
                });
            });
        });

    if response.clicked() {
        app.jump_to_page(hover_page);
    }
}

/// Scales `native` down or up, preserving aspect ratio, to the largest size
/// that fits within `bounds` on both axes.
fn scale_to_fit(native: Vec2, bounds: Vec2) -> Vec2 {
    if native.x <= 0.0 || native.y <= 0.0 {
        return bounds;
    }
    let ratio = (bounds.x / native.x).min(bounds.y / native.y);
    native * ratio
}
