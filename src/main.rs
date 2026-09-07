mod app;
mod input;
mod ui;
mod comic;
mod storage;
mod state;

use app::{ComicApp, UI_HIDE_DELAY};
use eframe::egui;

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions::default();

    eframe::run_native(
        "Comic Reader",
        options,
        Box::new(|_cc| Ok(Box::new(ComicApp::new()))),
    )
}

impl eframe::App for ComicApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.theme_preset.theme().bg.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {  // ✅ Changé : &mut Ui au lieu de Context
        self.theme_preset.theme().apply(ui.ctx());
        self.poll_loading();
        if self.loading {
            ui.ctx().request_repaint();
        }
        if self.title_dirty {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(self.window_title.clone()));
            self.title_dirty = false;
        }
        if self.fullscreen_dirty {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            self.fullscreen_dirty = false;
        }

        input::keyboard::handle_keyboard(self, ui.ctx());
        ui::settings::draw_settings(ui.ctx(), self);

        if !self.pages.is_empty() {
            let moved = ui.ctx().input(|i| i.pointer.delta() != egui::Vec2::ZERO);
            if moved {
                self.last_mouse_move = std::time::Instant::now();
            }

            // Keep repainting while the header is due to hide or mid-fade;
            // once it's settled (fully shown or fully hidden) stop, and let
            // the next real mouse-move event wake the loop back up.
            let idle_for = self.idle_time();
            if idle_for < UI_HIDE_DELAY {
                ui.ctx().request_repaint_after(UI_HIDE_DELAY - idle_for);
            } else if idle_for < UI_HIDE_DELAY + self.layout.fade_duration() {
                ui.ctx().request_repaint();
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
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.heading("🎯 Comic Reader v2.0");

                if let Some(error) = &self.load_error {
                    ui.colored_label(egui::Color32::from_rgb(220, 80, 80), error);
                }
                ui.label("Chargez un fichier pour commencer");
                if ui.button("Load File").clicked() {
                    self.start_loading_file();
                }
                ui.add_space(8.0);
                if ui.button(self.reading_mode.label()).clicked() {
                    self.toggle_reading_mode();
                }
                if ui.button("⚙ Settings").clicked() {
                    self.show_settings = true;
                }
            });
        } else {
            ui::header::draw_header(ui, self);
            ui::reader::draw_double_page(ui, self);
        }
    }
}