use crate::app::ComicApp;
use egui::Context;

/// Draws the "Go to Page" popup when `app.show_goto_page` is set — opened by
/// `Backtick` (see `input::keyboard`), a deliberately awkward key since this
/// is used far less often than plain page-turning. No-op otherwise. Mirrors
/// `ui::bookmarks::draw_bookmarks`'s window/close pattern.
pub fn draw_goto_page(ctx: &Context, app: &mut ComicApp) {
    if !app.show_goto_page {
        return;
    }

    let mut open = true;
    let mut submit = false;

    egui::Window::new("Go to Page")
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("Page (1-{}):", app.total_pages));
                let response = ui.add(egui::TextEdit::singleline(&mut app.goto_page_input).desired_width(60.0));
                if app.goto_page_focus_pending {
                    response.request_focus();
                    app.goto_page_focus_pending = false;
                }
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
                if ui.button("Go").clicked() {
                    submit = true;
                }
            });
        });

    if submit {
        app.submit_goto_page();
    } else if !open {
        app.show_goto_page = false;
    }
}
