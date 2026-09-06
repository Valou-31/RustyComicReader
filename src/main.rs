mod app;
mod input;
mod ui;
mod comic;
mod storage;
mod state;

use eframe::egui;
use app::ComicApp;

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions::default();
    
    eframe::run_native(
        "Comic Reader",
        options,
        Box::new(|_cc| Ok(Box::<ComicApp>::default())),
    )
}

impl eframe::App for ComicApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {  // ✅ Changé : &mut Ui au lieu de Context
        ui.heading("🎯 Comic Reader v2.0");
        ui.label("Chargez un fichier pour commencer");
    }
}