use egui::Ui;
use crate::app::ComicApp;

pub fn draw_double_page(ui: &mut Ui, app: &mut ComicApp) {
    let available_size = ui.available_size();
    
    let left_page = app.left_page();
    let right_page = app.right_page();

    ui.horizontal(|ui| {
        ui.set_width(available_size.x);
        ui.set_height(available_size.y);

        // Page de gauche
        if let Some(page_idx) = left_page {
            if page_idx < app.pages.len() {
                draw_single_page(ui, &app.pages[page_idx], available_size.x * 0.5);
            }
        }

        // Séparateur
        ui.separator();

        // Page de droite
        if let Some(page_idx) = right_page {
            if page_idx < app.pages.len() {
                draw_single_page(ui, &app.pages[page_idx], available_size.x * 0.5);
            }
        }
    });
}

fn draw_single_page(ui: &mut Ui, _image: &egui::ColorImage, _max_width: f32) {
    ui.label("Page placeholder");
}