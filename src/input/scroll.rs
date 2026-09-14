use crate::app::{ComicApp, ReadingMode};
use std::time::{Duration, Instant};

/// Horizontal trackpad scroll accumulated (in points) before it triggers a
/// page turn — small individual scroll ticks shouldn't each flip a page.
const SCROLL_PAGE_THRESHOLD: f32 = 140.0;

/// After turning a page from a scroll gesture, ignore further scroll input
/// for this long — trackpad momentum keeps sending events after the fingers
/// lift, which would otherwise skip several pages from one swipe.
const SCROLL_COOLDOWN: Duration = Duration::from_millis(400);

/// Treat scroll input as a new gesture (reset the accumulator) once it's
/// paused for this long, instead of adding to a stale leftover accumulation.
const SCROLL_GESTURE_GAP: Duration = Duration::from_millis(200);

/// Two-finger horizontal trackpad scroll turns pages. The "natural"
/// (non-inverted) mapping follows the book's reading direction: in manga
/// (RTL) mode, scrolling left-to-right advances and right-to-left goes
/// back; in traditional (LTR) mode it's the opposite, since the next page
/// physically sits to the reader's left-to-right-following side. `scroll_
/// inverted` flips this globally, independent of reading mode — the same
/// "natural vs. inverted" choice macOS itself offers for trackpad scroll.
pub fn handle_scroll(app: &mut ComicApp, ctx: &egui::Context) {
    if app.pages.is_empty() || app.show_settings || app.show_history || app.remapping_action.is_some() {
        return;
    }

    let raw_delta_x = ctx.input(|i| i.smooth_scroll_delta.x);
    if raw_delta_x == 0.0 {
        return;
    }
    let delta_x = if app.scroll_inverted { -raw_delta_x } else { raw_delta_x };

    let now = Instant::now();
    if now < app.scroll_cooldown_until {
        return;
    }

    if now.duration_since(app.last_scroll_at) > SCROLL_GESTURE_GAP || (app.scroll_accum > 0.0) != (delta_x > 0.0) {
        app.scroll_accum = 0.0;
    }
    app.last_scroll_at = now;
    app.scroll_accum += delta_x;

    if app.scroll_accum.abs() >= SCROLL_PAGE_THRESHOLD {
        let left_to_right = app.scroll_accum > 0.0;
        let advance = match app.reading_mode {
            ReadingMode::RTL => left_to_right,
            ReadingMode::LTR => !left_to_right,
        };
        if advance {
            app.next_spread();
        } else {
            app.prev_spread();
        }
        app.scroll_accum = 0.0;
        app.scroll_cooldown_until = now + SCROLL_COOLDOWN;
    }
}
