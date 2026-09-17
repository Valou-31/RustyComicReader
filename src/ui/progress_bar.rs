use crate::app::ComicApp;
use egui::{Align2, Area, Id, Order, Pos2, Rect, Sense, Stroke, Ui, Vec2};

/// Height of the progress bar's track, in points.
const BAR_HEIGHT: f32 = 10.0;
/// Gap, in points, between the bar and the top of the preview popup.
const PREVIEW_GAP: f32 = 12.0;
/// Size the preview popup's image area is always held to, whether it's
/// showing the thumbnail or still waiting on one — computed and applied
/// explicitly (`ui.set_min_size` + a manually aspect-scaled `Image`) rather
/// than left to `Image::shrink_to_fit`, whose "available space" inside a
/// freshly created floating `Area` isn't something we control, so relying
/// on it made the box's actual on-screen size unpredictable.
const PREVIEW_IMAGE_SIZE: Vec2 = Vec2::new(260.0, 375.0);

/// Draws the page-progress bar: a thin track filled up to the current
/// page's fraction of the book (page 35 of 100 fills 35%, regardless of
/// reading direction). Hovering always shows the page number under the
/// cursor, and — if `ComicApp::show_page_preview` is on — a preview image
/// too, decoded off the UI thread by `comic::thumbnail` so it never blocks
/// a frame. The page only actually changes on release — a plain click
/// jumps immediately (release right where you pressed), and pressing then
/// dragging along the bar keeps previewing and only jumps to wherever you
/// let go, like scrubbing a video's seek bar. Meant to be called from
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
    // `click_and_drag`, not just `click`: a plain click jumps on release
    // like before, but this also lets a press-then-drag be tracked below
    // via `interact_pointer_pos` even once the cursor strays off the bar's
    // thin rect — `hover_pos` alone goes `None` the instant that happens,
    // which is what made the preview disappear mid-scrub.
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, BAR_HEIGHT), Sense::click_and_drag());

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

    // While a press/drag that started on the bar is ongoing this keeps
    // reporting the pointer even off the bar's rect; otherwise it's plain
    // hover, `None` once the cursor truly isn't over the bar at all.
    let Some(pos) = response.interact_pointer_pos().or_else(|| response.hover_pos()) else { return };
    let hover_page = page_at_x(pos.x, rect, app.total_pages);
    // Clamped so the marker/popup stay anchored to the bar itself even
    // while scrubbing well above or below it.
    let marker_x = pos.x.clamp(rect.left(), rect.right());

    // Holding the cursor still over the bar (e.g. reading page numbers
    // while looking for a specific one) doesn't move the pointer, so it
    // wouldn't otherwise count as activity — without this, the header (and
    // the bar along with it) would fade out from under the cursor mid-use.
    app.last_mouse_move = std::time::Instant::now();

    ui.painter().line_segment(
        [Pos2::new(marker_x, rect.top()), Pos2::new(marker_x, rect.bottom())],
        Stroke::new(1.5, theme.text_primary),
    );

    draw_hover_popup(ui, app, marker_x, rect, hover_page);

    // `clicked` for a plain press-and-release with little movement,
    // `drag_stopped` for a press-then-drag ending anywhere (even off the
    // bar) — either way `hover_page` above already reflects wherever the
    // pointer is on this very release frame.
    if response.clicked() || response.drag_stopped() {
        app.jump_to_page(hover_page);
    }
}

/// The page index the horizontal position `x` (in the same space as
/// `rect`) maps to — clamped to the bar's own bounds, so a position beyond
/// either edge (dragging off the bar) still resolves to the first/last
/// page rather than going out of range.
fn page_at_x(x: f32, rect: Rect, total_pages: usize) -> usize {
    let fraction = ((x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
    ((fraction * total_pages as f32) as usize).min(total_pages - 1)
}

/// Draws the floating popup below `x` (already clamped to the bar's own
/// bounds) for `hover_page`: just the page number if
/// `ComicApp::show_page_preview` is off, or the number plus a preview image
/// if it's on. Split out from `draw_progress_bar` purely for readability.
fn draw_hover_popup(ui: &mut Ui, app: &mut ComicApp, x: f32, rect: Rect, hover_page: usize) {
    Area::new(Id::new("progress_bar_preview"))
        .fixed_pos(Pos2::new(x, rect.bottom() + PREVIEW_GAP))
        .pivot(Align2::CENTER_TOP)
        .order(Order::Tooltip)
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                if app.show_page_preview {
                    ui.set_min_size(PREVIEW_IMAGE_SIZE);
                }
                ui.vertical_centered(|ui| {
                    if app.show_page_preview {
                        // The sharp on-demand preview, if it's landed yet;
                        // the cheap whole-book one otherwise — so there's
                        // near always *something* to show instantly,
                        // upgraded to the sharp version a few milliseconds
                        // later.
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
                                // Keep repainting while waiting so the
                                // preview appears the instant the
                                // background decode lands, rather than
                                // only on the next mouse move.
                                ui.ctx().request_repaint();
                            }
                        }
                    }
                    ui.label(format!("Page {}", hover_page + 1));
                });
            });
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn bar_rect() -> Rect {
        Rect::from_min_size(Pos2::new(100.0, 0.0), Vec2::new(200.0, BAR_HEIGHT))
    }

    #[test]
    fn page_at_x_maps_the_left_and_right_edges_to_the_first_and_last_page() {
        let rect = bar_rect();
        assert_eq!(page_at_x(rect.left(), rect, 10), 0);
        assert_eq!(page_at_x(rect.right(), rect, 10), 9);
    }

    #[test]
    fn page_at_x_maps_the_midpoint_to_a_middle_page() {
        let rect = bar_rect();
        let mid = rect.left() + rect.width() / 2.0;
        assert_eq!(page_at_x(mid, rect, 10), 5);
    }

    #[test]
    fn page_at_x_clamps_positions_beyond_either_edge() {
        // Scrubbing well off the bar (up into the preview, or just past
        // its ends) should still resolve to a valid page, not go out of
        // range or panic.
        let rect = bar_rect();
        assert_eq!(page_at_x(rect.left() - 500.0, rect, 10), 0);
        assert_eq!(page_at_x(rect.right() + 500.0, rect, 10), 9);
    }
}
