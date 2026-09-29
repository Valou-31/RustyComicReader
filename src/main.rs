#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod input;
mod ui;
mod comic;
mod storage;
mod state;
mod platform;
mod update;

use app::{ComicApp, UI_HIDE_DELAY};
use eframe::egui;
use std::path::PathBuf;

const LOGO_BYTES: &[u8] = include_bytes!("logo.ico");

/// Appends every panic (message, location, and a forced backtrace) to
/// `crash.log` next to `config.json`, in addition to letting the default
/// hook still print to stderr as usual. A panic in a GUI app launched
/// outside a terminal (double-click, `Finder`, a bundled `.app`) leaves no
/// trace anywhere else — the window just disappears — so this is the only
/// way to actually diagnose one after the fact.
fn install_crash_log_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        let Some(config_dir) = dirs::config_dir() else { return };
        let log_path = config_dir.join("comic-reader").join("crash.log");
        let Some(parent) = log_path.parent() else { return };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let backtrace = std::backtrace::Backtrace::force_capture();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = format!("\n--- crash at unix time {timestamp} ---\n{info}\n{backtrace}\n");
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
            let _ = file.write_all(entry.as_bytes());
        }
    }));
}

/// Decodes the embedded logo into raw RGBA bytes plus its dimensions, shared
/// by both the OS window/taskbar icon and the in-app splash texture.
fn load_logo_rgba() -> (Vec<u8>, u32, u32) {
    let image = image::load_from_memory(LOGO_BYTES)
        .expect("embedded logo.ico should decode")
        .into_rgba8();
    let (width, height) = (image.width(), image.height());
    (image.into_raw(), width, height)
}

/// Command-line arguments that look like a comic archive we can open —
/// how the OS hands us a file on double-click/"Open With" on Windows and
/// Linux (`%1`/`%f` in the registry/.desktop entry become plain argv).
fn opened_files() -> Vec<PathBuf> {
    std::env::args().skip(1).map(PathBuf::from).filter(|path| comic::loader::is_supported_path(path)).collect()
}

fn main() -> Result<(), eframe::Error> {
    install_crash_log_hook();
    // Must happen before `eframe::run_native` (and so before winit's
    // `EventLoop::new`) ever runs — see `platform::macos` for why.
    #[cfg(target_os = "macos")]
    platform::macos::install_open_file_handler();
    let (rgba, width, height) = load_logo_rgba();
    let options = eframe::NativeOptions {
        // `app_id` is what Wayland compositors (KWin, GNOME Shell...) use to
        // find a matching .desktop file and pull its `Icon=` — Wayland has no
        // per-window icon protocol, unlike X11's `_NET_WM_ICON` set below.
        viewport: egui::ViewportBuilder::default()
            .with_icon(egui::IconData { rgba, width, height })
            .with_app_id("comic-reader"),
        ..Default::default()
    };

    let files = opened_files();
    eframe::run_native(
        "Comic Reader",
        options,
        Box::new(move |_cc| {
            let app = if files.is_empty() { ComicApp::new() } else { ComicApp::new_with_files(files) };
            Ok(Box::new(app))
        }),
    )
}

impl eframe::App for ComicApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.theme_preset.theme().bg.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {  // ✅ Changé : &mut Ui au lieu de Context
        // Must run before the header/footer render any toolbar row this
        // frame — see `ComicApp::apply_pending_toolbar_edit`.
        self.apply_pending_toolbar_edit();
        self.theme_preset.theme().apply(ui.ctx());
        #[cfg(target_os = "macos")]
        self.poll_macos_open_files();
        self.poll_picking();
        self.poll_dropped_files(ui.ctx());
        self.poll_loading(ui.ctx());
        self.poll_decoded_pages(ui.ctx());
        self.poll_webtoon_decoded_pages(ui.ctx());
        self.poll_thumbnails(ui.ctx());
        self.poll_fore_edge(ui.ctx());
        self.poll_history_save();
        self.poll_update_check();
        self.poll_update_apply();
        if self.loading {
            ui.ctx().request_repaint();
        }
        if self.title_dirty {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(self.window_title.clone()));
            self.title_dirty = false;
        }
        if self.fullscreen_dirty {
            // A borderless maximized window, not `ViewportCommand::Fullscreen`
            // — on macOS that maps to the OS's own fullscreen (a dedicated
            // Space, entered/exited with the animated transition), whose
            // Metal/vsync frame scheduling has long-standing, widely
            // reported stutter independent of what's being drawn (e.g.
            // https://github.com/bevyengine/bevy/issues/16087,
            // https://developer.apple.com/forums/thread/733033). Hiding
            // decorations and maximizing gets the same fill-the-screen look
            // without leaving the normal windowed presentation path.
            //
            // On macOS specifically, that alone leaves the menu bar showing
            // (a maximized window only fills `NSScreen.visibleFrame`, which
            // excludes it) — `set_immersive_presentation` auto-hides the
            // menu bar/Dock first, which is what grows `visibleFrame` out to
            // the screen's full bounds for `Maximized` to then fill. Must
            // run *before* `Maximized`/*after* un-maximizing — see its own
            // doc comment for why the order matters.
            #[cfg(target_os = "macos")]
            if self.fullscreen {
                platform::macos::set_immersive_presentation(true);
            }
            let ctx = ui.ctx();
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(!self.fullscreen));
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(self.fullscreen));
            #[cfg(target_os = "macos")]
            if !self.fullscreen {
                platform::macos::set_immersive_presentation(false);
            }
            self.fullscreen_dirty = false;
        }

        input::keyboard::handle_keyboard(self, ui.ctx());
        input::scroll::handle_scroll(self, ui.ctx());
        ui::settings::draw_settings(ui.ctx(), self);
        ui::history::draw_history(ui.ctx(), self);
        ui::bookmarks::draw_bookmarks(ui.ctx(), self);
        ui::goto_page::draw_goto_page(ui.ctx(), self);

        if !self.pages.is_empty() {
            self.update_ui_activity(ui.ctx());

            // Keep repainting while either bar is due to hide or mid-fade;
            // once both are settled (fully shown or fully hidden) stop, and
            // let the cursor crossing into/out of a reveal zone wake the
            // loop back up. `request_repaint_after` coalesces to whichever
            // requested time is soonest, so scheduling both independently
            // here is enough — no need to compute the minimum by hand.
            let fade = self.layout.fade_duration();
            for idle_for in [self.header_idle_time(), self.footer_idle_time()] {
                if idle_for < UI_HIDE_DELAY {
                    ui.ctx().request_repaint_after(UI_HIDE_DELAY - idle_for);
                } else if idle_for < UI_HIDE_DELAY + fade {
                    ui.ctx().request_repaint();
                }
            }
        }

        if self.loading {
            if let Some(image) = self.load_preview.take() {
                self.preview_texture =
                    Some(ui.ctx().load_texture("loading_preview", image, egui::TextureOptions::LINEAR));
            }

            let available_height = ui.available_height();
            ui.vertical_centered(|ui| {
                ui.add_space((available_height * 0.5 - 90.0).max(0.0));

                if let Some(texture) = &self.preview_texture {
                    ui.add(
                        egui::Image::new(texture)
                            .max_size(egui::vec2(220.0, 300.0))
                            .shrink_to_fit(),
                    );
                } else {
                    ui.add(egui::Spinner::new().size(32.0));
                }
                ui.add_space(8.0);

                let (loaded, total) = self.load_progress;
                if total > 0 {
                    let fraction = loaded as f32 / total as f32;
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .desired_width(220.0)
                            .text(format!("{loaded} / {total}")),
                    );
                } else {
                    ui.label("Chargement...");
                }
            });
        } else if self.pages.is_empty() {
            if self.logo_texture.is_none() {
                let (rgba, width, height) = load_logo_rgba();
                let image = egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
                self.logo_texture = Some(ui.ctx().load_texture("logo", image, egui::TextureOptions::LINEAR));
            }

            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                if let Some(logo) = &self.logo_texture {
                    ui.add(egui::Image::new(logo).max_size(egui::vec2(96.0, 96.0)).shrink_to_fit());
                    ui.add_space(8.0);
                }
                ui.heading("Comic Reader");

                if let Some(error) = &self.load_error {
                    ui.colored_label(egui::Color32::from_rgb(220, 80, 80), error);
                }
                ui.label("Chargez un fichier pour commencer");
                if ui.button("📂 Load File").clicked() {
                    self.start_loading_file();
                }
                ui.add_space(8.0);
                if ui.button(self.reading_mode.label()).clicked() {
                    self.toggle_reading_mode();
                }
                if ui.button("⚙ Settings").clicked() {
                    self.show_settings = true;
                }
                if ui.button("🕘 History").clicked() {
                    self.show_history = true;
                }
                if ui.button("📑 Bookmarks").clicked() {
                    self.show_bookmarks = true;
                }
                ui::header::draw_update_indicator(ui, self);
            });
        } else if self.toolbar_edit_mode {
            // Replaces the header/reader/footer entirely rather than
            // squeezing into the header's usual compact space above the
            // page — editing isn't reading, so there's no reason to
            // preserve room for it (see `ui::toolbar::draw_toolbar_editor`).
            ui::toolbar::draw_toolbar_editor(ui, self);
        } else if self.layout.header_floats_over_reader {
            // The page fills the whole window and the header draws over it
            // afterward, in its own floating layer — instead of `draw_header`
            // reserving its own space above the page (shrinking it to fit).
            ui::reader::draw_reader(ui, self);
            ui::header::draw_header_overlay(ui.ctx(), self);
            ui::footer::draw_footer(ui.ctx(), self);
        } else {
            ui::header::draw_header(ui, self);
            ui::reader::draw_reader(ui, self);
            ui::footer::draw_footer(ui.ctx(), self);
        }

        // Warm-tint overlay for the blue light filter — painted last, above
        // everything else, on a dedicated foreground layer so it never
        // intercepts clicks meant for the header or the reader below it.
        // Skipped while editing: there's no page underneath to tint, just
        // the editor itself.
        if !self.toolbar_edit_mode && let Some(color) = self.blue_light_overlay_color() {
            let screen_rect = ui.ctx().input(|i| i.viewport_rect());
            ui.ctx()
                .layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("blue_light_filter")))
                .rect_filled(screen_rect, 0.0, color);
        }

        // A file is being dragged over the window — there's no other visual
        // sign that dropping it would do anything, so this fills the whole
        // window while it's happening. Drawn last (on top of the blue light
        // tint too) so it's never obscured. Shown regardless of what's
        // currently on screen — empty state, loading, or mid-read — since a
        // drop can land during any of those.
        if ui.ctx().input(|i| !i.raw.hovered_files.is_empty()) {
            let screen_rect = ui.ctx().input(|i| i.viewport_rect());
            let painter =
                ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop_hint")));
            painter.rect_filled(screen_rect, 0.0, egui::Color32::from_black_alpha(180));
            painter.text(
                screen_rect.center(),
                egui::Align2::CENTER_CENTER,
                "📂 Drop to open",
                egui::FontId::proportional(28.0),
                egui::Color32::WHITE,
            );
        }
    }

    /// Makes sure the last reading position is on disk even if the debounced
    /// periodic save (`poll_history_save`) hasn't fired yet.
    fn on_exit(&mut self) {
        self.flush_history();
    }
}
