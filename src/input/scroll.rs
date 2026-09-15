use crate::app::{ComicApp, ReadingMode};
use egui::{Event, TouchPhase};

/// Horizontal drag distance (in points) that maps to a full `0.0..=1.0`
/// page-turn progress — roughly one comfortable two-finger swipe.
const DRAG_FULL_DISTANCE: f32 = 220.0;

/// Two-finger horizontal trackpad scroll turns pages, tracking the fingers
/// live rather than jumping straight to the next spread: the page follows
/// the drag distance, and only commits (or settles back) once the trackpad
/// gesture actually ends — signaled by the OS via `TouchPhase::End`/`Cancel`
/// on the scroll event itself, not guessed from a pause in movement. Holding
/// the fingers still mid-swipe (without lifting) keeps sending `Move`-phase
/// events, so it doesn't get mistaken for a release and snap the page back.
///
/// The "natural" mapping follows the book's reading direction: in manga
/// (RTL) mode, dragging left-to-right advances and right-to-left goes back;
/// in traditional (LTR) mode it's the opposite. `scroll_inverted` flips this
/// globally, independent of reading mode — the same "natural vs. inverted"
/// choice macOS itself offers for trackpad scroll.
pub fn handle_scroll(app: &mut ComicApp, ctx: &egui::Context) {
    if app.pages.is_empty() || app.show_settings || app.show_history || app.remapping_action.is_some() {
        return;
    }

    let (delta_sum, gesture_ended) = ctx.input(|i| {
        i.events.iter().fold((0.0f32, false), |(sum, ended), event| match event {
            Event::MouseWheel { phase: TouchPhase::End | TouchPhase::Cancel, .. } => (sum, true),
            Event::MouseWheel { delta, .. } => (sum + delta.x, ended),
            _ => (sum, ended),
        })
    });

    if delta_sum != 0.0 {
        let delta_x = if app.scroll_inverted { -delta_sum } else { delta_sum };
        let sample_forward = wants_forward(delta_x, app.reading_mode);

        if !app.is_dragging() && app.page_transition.is_none() {
            app.start_page_drag(sample_forward);
        }

        if let Some(drag_forward) = app.dragging_forward() {
            let signed = if sample_forward == drag_forward { delta_x.abs() } else { -delta_x.abs() };
            app.drag_page_by(signed / DRAG_FULL_DISTANCE);
        }
    }

    if gesture_ended && app.is_dragging() {
        app.end_page_drag();
    }
}

/// Whether a horizontal delta (positive = left-to-right, already adjusted
/// for `scroll_inverted`) means "advance" for the given reading mode.
fn wants_forward(delta_x: f32, reading_mode: ReadingMode) -> bool {
    let left_to_right = delta_x > 0.0;
    match reading_mode {
        ReadingMode::RTL => left_to_right,
        ReadingMode::LTR => !left_to_right,
    }
}
