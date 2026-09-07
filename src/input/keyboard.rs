use crate::app::{ComicApp, RemappingAction};
use crate::input::keybindings::Action;

pub fn handle_keyboard(app: &mut ComicApp, ctx: &egui::Context) {
    // Mode remappage
    if app.remapping_action.is_some() {
        ctx.input(|input| {
            for event in &input.events {
                if let egui::Event::Key { key, pressed: true, .. } = event {
                    let key_str = format!("{:?}", key);
                    if let Some(RemappingAction::NextSpread) = app.remapping_action {
                        app.keybindings.add_key(Action::NextSpread, key_str);
                    } else if let Some(RemappingAction::PrevSpread) = app.remapping_action {
                        app.keybindings.add_key(Action::PrevSpread, key_str);
                    } else if let Some(RemappingAction::ShiftRight) = app.remapping_action {
                        app.keybindings.add_key(Action::ShiftRight, key_str);
                    } else if let Some(RemappingAction::ShiftLeft) = app.remapping_action {
                        app.keybindings.add_key(Action::ShiftLeft, key_str);
                    }
                }
            }
        });
        return;
    }

    // Mode normal
    ctx.input(|input| {
        for event in &input.events {
            if let egui::Event::Key { key, pressed: true, .. } = event {
                let key_str = format!("{:?}", key);
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

    // Plein écran
    if ctx.input(|i| i.key_pressed(egui::Key::F)) {
        app.fullscreen = !app.fullscreen;
        app.fullscreen_dirty = true;
    }
}