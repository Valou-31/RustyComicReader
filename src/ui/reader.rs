use crate::app::ComicApp;
use egui::{Align, Color32, Pos2, Rect, Stroke, TextureHandle, TextureOptions, Ui, Vec2};
use std::collections::HashMap;

pub fn draw_double_page(ui: &mut Ui, app: &mut ComicApp) {
    let available_rect = ui.available_rect_before_wrap();
    let mid_x = available_rect.center().x;
    let half_gap = (app.layout.page_gap / 2.0).max(0.0);

    let left_column =
        Rect::from_min_max(available_rect.left_top(), Pos2::new(mid_x - half_gap, available_rect.bottom()));
    let right_column =
        Rect::from_min_max(Pos2::new(mid_x + half_gap, available_rect.top()), available_rect.right_bottom());

    let left_page = app.left_page();
    let right_page = app.right_page();

    let pages = &app.pages;
    let textures = &mut app.textures;

    // Left page flush against the right edge of its column, right page
    // flush against the left edge of its — both meet at `mid_x` (or the
    // configured gap around it), so the spread reads as one continuous book
    // opening rather than two independently centered pages with a gap.
    draw_page_slot(ui, left_column, Align::Max, left_page.and_then(|idx| pages.get(idx).map(|img| (idx, img))), textures);
    draw_page_slot(ui, right_column, Align::Min, right_page.and_then(|idx| pages.get(idx).map(|img| (idx, img))), textures);

    if app.layout.spine_width > 0.0 {
        let separator = app.theme_preset.theme().separator;
        let alpha = (app.layout.spine_opacity.clamp(0.0, 1.0) * 255.0) as u8;
        let spine_color = Color32::from_rgba_unmultiplied(separator.r(), separator.g(), separator.b(), alpha);
        ui.painter().line_segment(
            [Pos2::new(mid_x, available_rect.top()), Pos2::new(mid_x, available_rect.bottom())],
            Stroke::new(app.layout.spine_width, spine_color),
        );
    }

    ui.allocate_rect(available_rect, egui::Sense::hover());
}

/// Paints the page's image inside `column`, always filling it top-to-bottom
/// and horizontally anchored to `align` (`Min` = flush left, `Max` = flush
/// right) — an empty column (no page at this spot, e.g. the last page of an
/// odd-length book) still reserves its space so the other page doesn't jump
/// to fill it.
fn draw_page_slot(
    ui: &Ui,
    column: Rect,
    align: Align,
    page: Option<(usize, &egui::ColorImage)>,
    textures: &mut HashMap<usize, TextureHandle>,
) {
    let Some((page_idx, image)) = page else {
        return;
    };

    let texture = textures
        .entry(page_idx)
        .or_insert_with(|| ui.ctx().load_texture(format!("page_{page_idx}"), image.clone(), TextureOptions::LINEAR));

    let display_size = fit_to_height(texture.size_vec2(), column.height());
    let x_min = match align {
        Align::Min => column.left(),
        Align::Max => column.right() - display_size.x,
        Align::Center => column.center().x - display_size.x / 2.0,
    };
    let image_rect = Rect::from_min_size(Pos2::new(x_min, column.top()), display_size);
    egui::Image::new(&*texture).paint_at(ui, image_rect);
}

/// Scales `size` so its height exactly matches `height`, preserving aspect
/// ratio — the page always fills the full screen height; width follows from
/// that.
fn fit_to_height(size: Vec2, height: f32) -> Vec2 {
    if size.y <= 0.0 {
        return Vec2::ZERO;
    }
    size * (height / size.y)
}
