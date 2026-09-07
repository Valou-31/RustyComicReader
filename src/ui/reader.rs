use crate::app::ComicApp;
use egui::{TextureHandle, TextureOptions, Ui};
use std::collections::HashMap;

pub fn draw_double_page(ui: &mut Ui, app: &mut ComicApp) {
    let available_size = ui.available_size();

    let left_page = app.left_page();
    let right_page = app.right_page();

    let pages = &app.pages;
    let textures = &mut app.textures;

    ui.horizontal(|ui| {
        ui.set_width(available_size.x);
        ui.set_height(available_size.y);

        if let Some(page_idx) = left_page {
            if let Some(image) = pages.get(page_idx) {
                draw_single_page(ui, image, textures, page_idx, available_size.x * 0.5);
            }
        }

        ui.separator();

        if let Some(page_idx) = right_page {
            if let Some(image) = pages.get(page_idx) {
                draw_single_page(ui, image, textures, page_idx, available_size.x * 0.5);
            }
        }
    });
}

fn draw_single_page(
    ui: &mut Ui,
    image: &egui::ColorImage,
    textures: &mut HashMap<usize, TextureHandle>,
    page_idx: usize,
    max_width: f32,
) {
    let texture = textures.entry(page_idx).or_insert_with(|| {
        ui.ctx()
            .load_texture(format!("page_{page_idx}"), image.clone(), TextureOptions::LINEAR)
    });

    let max_height = ui.available_height();
    ui.add(
        egui::Image::new(&*texture)
            .max_size(egui::vec2(max_width, max_height))
            .shrink_to_fit(),
    );
}
