use crate::app::ComicApp;
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

    // Normal reading navigation.
    ctx.input(|input| {
        for event in &input.events {
            if let egui::Event::Key { key, pressed: true, .. } = event {
                let key_str = format!("{key:?}");
                if let Some(action) = app.keybindings.action_for_key(&key_str) {
                    match action {
                        Action::NextSpread => app.next_spread(),
                        Action::PrevSpread => app.prev_spread(),
                        Action::ShiftRight => app.shift_right(),
                        Action::ShiftLeft => app.shift_left(),
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
}
