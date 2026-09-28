use crate::app::{ComicApp, ReadingMode, WEBTOON_SCROLL_SPEED_DOC};
use crate::input::keybindings::Action;

pub fn handle_keyboard(app: &mut ComicApp, ctx: &egui::Context) {
    // Waiting for the next key press to bind to `action`.
    if let Some(action) = app.remapping_action {
        ctx.input(|input| {
            for event in &input.events {
                if let egui::Event::Key { key, pressed: true, .. } = event {
                    if *key == egui::Key::Escape {
                        app.remapping_action = None;
                    } else {
                        app.keybindings.add_key(action, format!("{key:?}"));
                        app.remapping_action = None;
                        app.save_config();
                    }
                    break;
                }
            }
        });
        return;
    }

    // The settings window is open but not actively capturing a key — only
    // let Escape through (to close it); block page-turning underneath it.
    if app.show_settings {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            app.show_settings = false;
        }
        return;
    }

    // The toolbar editor replaces the reader entirely (see `main.rs`), so
    // there's no page underneath for arrow keys etc. to turn — same
    // Escape-only pattern as the settings window above.
    if app.toolbar_edit_mode {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            app.toolbar_edit_mode = false;
        }
        return;
    }

    // The "Go to Page" popup is open — block page-turn shortcuts entirely
    // (typing a page number shouldn't also turn pages: the Numpad preset,
    // for one, binds Num4/Num6 to Prev/NextSpread, which are exactly the
    // digits being typed). Escape cancels it; Enter/submit is handled by
    // the popup's own text field in `ui::goto_page`, since egui's raw key
    // events here don't know which widget has focus.
    if app.show_goto_page {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            app.show_goto_page = false;
        }
        return;
    }

    // In Webtoon mode the up/down arrows scroll the continuous strip
    // instead of turning a page — held-down (`key_down`, polled every
    // frame) rather than a per-press step, so holding the key reads as a
    // smooth scroll instead of a staircase. Actual clamping against the
    // book's content height happens in `ui::reader::draw_webtoon`, which is
    // the only place that knows the current layout; this just accumulates
    // the raw delta.
    if app.reading_mode == ReadingMode::Webtoon {
        let (up, down) = ctx.input(|i| (i.key_down(egui::Key::ArrowUp), i.key_down(egui::Key::ArrowDown)));
        if up != down {
            let dt = ctx.input(|i| i.stable_dt);
            let direction = if down { 1.0 } else { -1.0 };
            app.webtoon_scroll += WEBTOON_SCROLL_SPEED_DOC * dt * direction;
            ctx.request_repaint();
        }
    }

    // Normal reading navigation.
    ctx.input(|input| {
        for event in &input.events {
            if let egui::Event::Key { key, pressed: true, .. } = event {
                let key_str = format!("{key:?}");
                if let Some(action) = app.keybindings.action_for_key(&key_str) {
                    match action {
                        Action::NextSpread => app.next_spread(),
                        Action::PrevSpread => app.prev_spread(),
                        Action::IsolatePage => app.toggle_isolate_current_page(),
                    }
                }
            }
        }
    });

    // Plein écran — non-remappable.
    if ctx.input(|i| i.key_pressed(egui::Key::F)) {
        app.fullscreen = !app.fullscreen;
        app.fullscreen_dirty = true;
    }

    // Aller à la page — non-remappable, and deliberately on an awkward key
    // (the backtick, top-left corner of the keyboard): unlike page-turning,
    // this is reached for rarely enough that ergonomics don't matter, and
    // an easy-to-reach key is better spent on something used constantly.
    if ctx.input(|i| i.key_pressed(egui::Key::Backtick)) {
        app.open_goto_page();
    }
}
