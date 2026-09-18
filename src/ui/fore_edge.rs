use egui::{Pos2, Rect, Ui, Vec2};

/// Width, in points, the fully-unread side's bar reaches right at the start
/// of the book — the read side grows to match it by the very end, so the
/// two bars together always read as one book's worth of pages shifting
/// weight from one hand to the other as you progress, the way a physical
/// book's page block does.
const MAX_BAR_WIDTH: f32 = 36.0;

/// Paints one page's fore-edge bar: a slice of the fore-edge composite
/// texture (`ComicApp::fore_edge_texture`, built column by column by
/// `ComicApp::update_fore_edge_column` — see `comic::fore_edge`), starting
/// flush at `edge_x` and extending *outward* — `extends_left` toward
/// smaller x, away from the page it belongs to, otherwise toward larger x —
/// into whatever margin `ui::reader::draw_fore_edge_slot` left there.
/// `read_on_left` says whether the screen's left side holds the pages
/// already read (true for LTR/Single, false for RTL); `read_fraction`,
/// `0.0`..=`1.0`, is how far into the book this bar's own page is. Together
/// these pick which half of the composite this particular bar shows (its
/// own side's share, read or unread) and how much of `MAX_BAR_WIDTH` it
/// gets — so as the reader progresses through the book, the read-side bars
/// widen and the unread-side ones narrow. Skipped once the resulting width
/// rounds away to nothing.
pub fn paint_bar(
    ui: &Ui,
    texture: &egui::TextureHandle,
    vertical: Rect,
    edge_x: f32,
    extends_left: bool,
    read_on_left: bool,
    read_fraction: f32,
) {
    let is_read_side = extends_left == read_on_left;
    let fraction = if is_read_side { read_fraction } else { 1.0 - read_fraction };
    let width = MAX_BAR_WIDTH * fraction;
    if width < 0.5 {
        return;
    }

    // The composite's columns run in page order regardless of reading
    // direction, so the "read" half is always its low-u end and the
    // "unread" half its high-u end.
    let uv = if is_read_side {
        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(read_fraction, 1.0))
    } else {
        Rect::from_min_max(Pos2::new(read_fraction, 0.0), Pos2::new(1.0, 1.0))
    };

    let rect = if extends_left {
        Rect::from_min_size(Pos2::new(edge_x - width, vertical.top()), Vec2::new(width, vertical.height()))
    } else {
        Rect::from_min_size(Pos2::new(edge_x, vertical.top()), Vec2::new(width, vertical.height()))
    };

    egui::Image::new(texture).uv(uv).paint_at(ui, rect);
}
