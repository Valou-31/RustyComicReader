use crate::app::{ComicApp, ReadingMode};
use egui::{Event, TouchPhase};

/// Horizontal drag distance (in points) that maps to a full `0.0..=1.0`
/// page-turn progress at the default sensitivity — roughly one comfortable
/// two-finger swipe. Divided by `ComicApp::scroll_sensitivity` before use, so
/// a higher sensitivity needs less physical movement for the same turn.
const DRAG_FULL_DISTANCE: f32 = 220.0;

/// Two-finger horizontal trackpad scroll turns pages, tracking the fingers
/// live rather than jumping straight to the next spread: the page follows
/// the drag distance, and only commits (or settles back) once the trackpad
/// gesture actually ends — signaled by the OS via `TouchPhase::End`/`Cancel`
/// on the scroll event itself, not guessed from a pause in movement. Holding
/// the fingers still mid-swipe (without lifting) keeps sending `Move`-phase
/// events, so it doesn't get mistaken for a release and snap the page back.
/// At release, `ComicApp::end_page_drag` hands off to the settle spring
/// (`step_transition`) with the drag's last velocity, so a fast flick keeps
/// gliding instead of the settle starting dead still.
///
/// The "natural" mapping follows the book's reading direction: in manga
/// (RTL) mode, dragging left-to-right advances and right-to-left goes back;
/// in traditional (LTR) mode it's the opposite. `scroll_inverted` flips this
/// globally, independent of reading mode — the same "natural vs. inverted"
/// choice macOS itself offers for trackpad scroll.
///
/// With `one_turn_per_swipe` on, a swipe that already resolved (commit or
/// cancel) locks out further page-turns — see `ComicApp::swipe_locked` —
/// until a fresh `TouchPhase::Start` shows a genuinely new gesture began.
/// This is what stops one strong, fast swipe from chaining through several
/// spreads: trackpad momentum keeps delivering `Move`-phase deltas well
/// after the fingers actually lift, with no `Start` of its own, so without
/// this lock those trailing deltas just look like more swiping and can
/// accumulate into a second (or third) commit.
pub fn handle_scroll(app: &mut ComicApp, ctx: &egui::Context) {
    if app.pages.is_empty()
        || app.show_settings
        || app.show_history
        || app.show_bookmarks
        || app.toolbar_edit_mode
        || app.remapping_action.is_some()
    {
        return;
    }

    let (delta_sum, gesture_ended, gesture_started) = ctx.input(|i| {
        i.events.iter().fold((0.0f32, false, false), |(sum, ended, started), event| match event {
            // A scroll held with the zoom modifier (Cmd on macOS, Ctrl
            // elsewhere) is `ui::reader`'s ctrl/Cmd-scroll-to-zoom, not a
            // page-turn swipe — ignore it here entirely so the two don't
            // fire at once.
            Event::MouseWheel { modifiers, .. } if modifiers.command => (sum, ended, started),
            Event::MouseWheel { phase: TouchPhase::Start, delta, .. } => (sum + delta.x, ended, true),
            Event::MouseWheel { phase: TouchPhase::End | TouchPhase::Cancel, .. } => (sum, true, started),
            Event::MouseWheel { delta, .. } => (sum + delta.x, ended, started),
            _ => (sum, ended, started),
        })
    });

    if gesture_started {
        app.swipe_locked = false;
    }
    if app.swipe_locked {
        return;
    }

    if delta_sum != 0.0 {
        let delta_x = if app.scroll_inverted { -delta_sum } else { delta_sum };
        let sample_forward = wants_forward(delta_x, app.reading_mode);

        if !app.is_dragging() {
            // Whether there's nothing in flight, or a settle spring (a
            // commit/bounce-back from a previous drag) is still playing —
            // either way `start_page_drag` decides whether to grab it or
            // start fresh.
            app.start_page_drag(sample_forward);
        }

        if let Some(drag_forward) = app.dragging_forward() {
            let signed = if sample_forward == drag_forward { delta_x.abs() } else { -delta_x.abs() };
            let dt = ctx.input(|i| i.stable_dt);
            let full_distance = DRAG_FULL_DISTANCE / app.scroll_sensitivity.max(0.1);
            app.drag_page_by(signed / full_distance, dt);
        }
    }

    if gesture_ended && app.is_dragging() {
        app.end_page_drag();
    }
}

/// Whether a horizontal delta (positive = left-to-right, already adjusted
/// for `scroll_inverted`) means "advance" for the given reading mode. Single
/// Page mode follows the same convention as LTR.
fn wants_forward(delta_x: f32, reading_mode: ReadingMode) -> bool {
    let left_to_right = delta_x > 0.0;
    match reading_mode {
        ReadingMode::RTL => left_to_right,
        ReadingMode::LTR | ReadingMode::Single => !left_to_right,
    }
}
