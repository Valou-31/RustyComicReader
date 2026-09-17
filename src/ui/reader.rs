use crate::app::{ComicApp, PageZoom, ReadingMode, ZOOM_MAX, ZOOM_MIN, ZoomTarget};
use crate::comic::archive::{ComicArchive, PageMeta};
use egui::{Align, Color32, Pos2, Rect, Stroke, TextureHandle, TextureOptions, Ui, Vec2};
use std::collections::HashMap;

/// How many pages beyond the currently displayed spread keep their decoded
/// texture resident (in each direction) — also how far ahead/behind pages
/// get prefetched. Close enough to flip back and forth a page or two
/// instantly; anything further out is evicted (see `spread_window`), so
/// memory stays bounded regardless of how long the book is.
const TEXTURE_KEEP_RADIUS: usize = 4;

/// Where a page's compressed bytes come from, and the shared caches its
/// resulting texture and metadata are read from / written into — bundled so
/// `draw_page_slot`/`draw_spread` don't each need half a dozen parameters.
struct DecodeCtx<'a> {
    pages: &'a [Vec<u8>],
    textures: &'a mut HashMap<usize, TextureHandle>,
    page_meta: &'a mut HashMap<usize, PageMeta>,
    max_dimension: Option<u32>,
}

/// The three rects a spread can render into: split `left`/`right` columns
/// for a normal two-page spread, or the `full` rect (spanning both) for a
/// lone double-page image — all pre-split from one `available_rect` so a
/// sliding spread only needs to `translate` them, not recompute the split.
struct Columns {
    full: Rect,
    left: Rect,
    right: Rect,
}

/// One rendered spread's seam: where its own two pages meet (which moves
/// during a slide, see `draw_double_page`), which page is on each side, and
/// whether it's actually a lone double-page image rather than a genuine
/// pair — needed to draw its spine/gap fill, or skip them entirely.
struct SpreadSeam {
    mid_x: f32,
    left_idx: Option<usize>,
    right_idx: Option<usize>,
    full_spread: bool,
}

/// Which local zoom (magnification, pan), if any, applies to each side of a
/// spread — see `draw_page_slot`. Both `None` outside of an actively-zoomed
/// `ZoomTarget::SinglePage` (`Spread`-mode zoom instead scales the whole
/// layout rect the columns are split from, so it needs no per-side info).
#[derive(Clone, Copy, Default)]
struct SideZoom {
    left: Option<(f32, Vec2)>,
    right: Option<(f32, Vec2)>,
}

pub fn draw_double_page(ui: &mut Ui, app: &mut ComicApp) {
    let base_rect = ui.available_rect_before_wrap();

    // Reads pinch/ctrl(-or-Cmd)-scroll zoom and click-drag panning for this
    // frame, updating `app.zoom_spread`/`zoom_left`/`zoom_right` in place —
    // called before any layout below reads them, so a drag or zoom gesture
    // is reflected the same frame it happens rather than one frame late.
    handle_zoom_and_pan(ui, app, base_rect);

    // In `Spread` mode, zoom scales the whole two-page layout together, so
    // it's applied to the rect the columns are split from. In `SinglePage`
    // mode the overall spread stays laid out normally — zoom instead
    // applies locally to each page's own slot, below — so the split always
    // comes from the plain `base_rect`.
    let layout_rect = if app.zoom_spread.is_zoomed() {
        Rect::from_center_size(base_rect.center() + app.zoom_spread.pan, base_rect.size() * app.zoom_spread.scale)
    } else {
        base_rect
    };
    let mid_x = layout_rect.center().x;
    let half_gap = (app.layout.page_gap / 2.0).max(0.0);

    let columns = Columns {
        full: layout_rect,
        left: Rect::from_min_max(layout_rect.left_top(), Pos2::new(mid_x - half_gap, layout_rect.bottom())),
        right: Rect::from_min_max(Pos2::new(mid_x + half_gap, layout_rect.top()), layout_rect.right_bottom()),
    };

    // Crop the zoomed content to the actual reading area — without this,
    // zooming in (in `Spread` mode) would paint past `base_rect`'s edges,
    // over the header or the window's own bounds.
    ui.set_clip_rect(base_rect);

    // Steps any in-flight page-turn spring by this frame's delta time — a
    // no-op while idle. Clears `page_transition` itself once the spring's at
    // rest, so there's no separate "is it finished" check to do here after.
    app.step_transition(ui.ctx().input(|i| i.stable_dt));

    let current_left = app.left_page();
    let current_right = app.right_page();
    let single_page_mode = app.reading_mode == ReadingMode::Single;

    // Each side's local zoom+pan, if any — `zoom_left`/`zoom_right` are
    // independent, so both pages can be zoomed to different levels at once
    // (only meaningful in `SinglePage` mode; `Spread`-mode zoom instead
    // scales `layout_rect` above, so these stay unzoomed either way).
    let left_zoom = app.zoom_left.is_zoomed().then_some((app.zoom_left.scale, app.zoom_left.pan));
    let right_zoom = app.zoom_right.is_zoomed().then_some((app.zoom_right.scale, app.zoom_right.pan));

    // This frame's transition state, if any: visual progress, which side
    // the new spread enters from, and the two spreads involved. Suppressed
    // while zoomed in — sliding a zoomed, panned view during a page turn
    // reads as visual noise rather than a page-turn animation, so a turn
    // that happens while zoomed just cuts straight to the new page instead.
    let sliding = (!app.is_zoomed())
        .then(|| app.page_transition.as_ref().map(|t| (t.progress, t.entry_sign, t.old_left, t.old_right, t.new_left, t.new_right)))
        .flatten();

    let keep_alive = [current_left, current_right].into_iter().flatten().chain(sliding.into_iter().flat_map(
        |(_, _, old_left, old_right, new_left, new_right)| {
            [old_left, old_right, new_left, new_right].into_iter().flatten()
        },
    ));
    if let Some((low, high)) = spread_window(keep_alive) {
        app.textures.retain(|&page_idx, _| (low..=high).contains(&page_idx));
        app.page_meta.retain(|&page_idx, _| (low..=high).contains(&page_idx));
        app.request_prefetch(low, high);
    }

    let max_dimension = app.decode_max_dimension();
    let mut ctx =
        DecodeCtx { pages: &app.pages, textures: &mut app.textures, page_meta: &mut app.page_meta, max_dimension };

    // Left page flush against the right edge of its column, right page
    // flush against the left edge of its — both meet at `mid_x` (or the
    // configured gap around it), so the spread reads as one continuous book
    // opening rather than two independently centered pages with a gap. A
    // lone double-page spread — or any page at all in Single Page mode —
    // instead spans the full width, centered. Each spread's seam travels
    // with it, so both are drawn at their shifted `mid_x` while sliding.
    let seams: [Option<SpreadSeam>; 2] = if let Some((progress, entry_sign, old_left, old_right, new_left, new_right)) = sliding {
        ui.ctx().request_repaint();
        let width = layout_rect.width();
        let new_shift = Vec2::new(entry_sign * (1.0 - progress) * width, 0.0);
        let old_shift = Vec2::new(-entry_sign * progress * width, 0.0);

        let old_full = old_right.is_none() && (single_page_mode || is_double_page(old_left, ctx.page_meta));
        let new_full = new_right.is_none() && (single_page_mode || is_double_page(new_left, ctx.page_meta));
        // Never zoomed while sliding (see `sliding` above), so nothing to
        // pass here.
        draw_spread(ui, &columns, old_shift, (old_left, old_right), old_full, SideZoom::default(), &mut ctx);
        draw_spread(ui, &columns, new_shift, (new_left, new_right), new_full, SideZoom::default(), &mut ctx);

        [
            Some(SpreadSeam { mid_x: mid_x + old_shift.x, left_idx: old_left, right_idx: old_right, full_spread: old_full }),
            Some(SpreadSeam { mid_x: mid_x + new_shift.x, left_idx: new_left, right_idx: new_right, full_spread: new_full }),
        ]
    } else {
        let full_spread = current_right.is_none() && (single_page_mode || is_double_page(current_left, ctx.page_meta));
        draw_spread(ui, &columns, Vec2::ZERO, (current_left, current_right), full_spread, SideZoom { left: left_zoom, right: right_zoom }, &mut ctx);

        [Some(SpreadSeam { mid_x, left_idx: current_left, right_idx: current_right, full_spread }), None]
    };

    let spine_color = {
        let [r, g, b] = app.layout.spine_color;
        let alpha = (app.layout.spine_opacity.clamp(0.0, 1.0) * 255.0) as u8;
        Color32::from_rgba_unmultiplied(r, g, b, alpha)
    };

    for seam in seams.into_iter().flatten() {
        // A lone double-page image has no seam of its own to mark — drawing
        // one would fake a page break in the middle of a single picture.
        if seam.full_spread {
            continue;
        }
        if half_gap > 0.0 {
            let gap_rect = Rect::from_min_max(
                Pos2::new(seam.mid_x - half_gap, layout_rect.top()),
                Pos2::new(seam.mid_x + half_gap, layout_rect.bottom()),
            );
            draw_gap_fill(ui.painter(), ctx.page_meta, gap_rect, seam.left_idx, seam.right_idx);
        }
        if app.layout.spine_shadow_width > 0.0 {
            draw_spine_shadow(
                ui.painter(),
                seam.mid_x,
                layout_rect.top(),
                layout_rect.bottom(),
                app.layout.spine_shadow_width,
                spine_color,
            );
        }
        if app.layout.spine_width > 0.0 {
            ui.painter().line_segment(
                [Pos2::new(seam.mid_x, layout_rect.top()), Pos2::new(seam.mid_x, layout_rect.bottom())],
                Stroke::new(app.layout.spine_width, spine_color),
            );
        }
    }
}

/// Reads pinch/ctrl(-or-Cmd)-scroll zoom and click-drag panning for this
/// frame and updates `app.zoom`/`zoom_pan`/`zoom_single_page` in place.
/// `egui` already merges a trackpad pinch gesture and a modifier-held
/// scroll into one `zoom_delta`, so both gestures are handled here by the
/// same few lines — `input::scroll` separately makes sure a modifier-held
/// scroll isn't *also* read as a page-turn swipe.
///
/// No-op while a modal (Settings/History) is open, so a pinch or drag meant
/// for it doesn't leak through to the reader underneath.
fn handle_zoom_and_pan(ui: &Ui, app: &mut ComicApp, base_rect: Rect) {
    // Claims the whole reading area so drags starting on it are ours to
    // read, regardless of zoom level — allocated unconditionally so the
    // area still counts as claimed for layout even when zoomed out.
    let response = ui.interact(base_rect, ui.id().with("reader_pan_zoom"), egui::Sense::click_and_drag());

    if app.show_settings || app.show_history {
        return;
    }

    let single_page_target = app.zoom_target == ZoomTarget::SinglePage;

    // Panning: applied to whichever zoom a drag is actually over — the
    // whole spread in `Spread` mode, or whichever page's own box the drag
    // is over in `SinglePage` mode (`zoom_left`/`zoom_right` are
    // independent, so dragging one page never touches the other's pan).
    // `interact_pointer_pos`, not `hover_pos`: it keeps reporting the
    // pointer for a drag that started on this area even once the cursor
    // strays outside it.
    if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let target = if single_page_target {
            if pos.x < base_rect.center().x { &mut app.zoom_left } else { &mut app.zoom_right }
        } else {
            &mut app.zoom_spread
        };
        if target.is_zoomed() {
            target.pan += response.drag_delta();
        }
    }

    let zoom_delta = ui.ctx().input(|i| i.zoom_delta());
    if zoom_delta != 1.0 && response.hovered() {
        if let Some(cursor) = ui.ctx().input(|i| i.pointer.hover_pos()) {
            if single_page_target {
                // Always applies to whichever half of the spread the
                // cursor is *currently* over — so zooming in on the left
                // page, then moving to the right one and zooming that
                // in too, leaves both independently zoomed rather than
                // the second gesture fighting over one shared value.
                let target = if cursor.x < base_rect.center().x { &mut app.zoom_left } else { &mut app.zoom_right };
                target.scale = (target.scale * zoom_delta).clamp(ZOOM_MIN, ZOOM_MAX);
                if target.scale <= ZOOM_MIN {
                    *target = PageZoom::NONE;
                }
            } else {
                let old_zoom = app.zoom_spread.scale;
                app.zoom_spread.scale = (old_zoom * zoom_delta).clamp(ZOOM_MIN, ZOOM_MAX);
                // Zoom toward the cursor, not always from dead-center, so
                // whatever's under it stays put as the page scales — what
                // every pinch/scroll-zoom gesture is expected to do.
                app.zoom_spread.pan =
                    zoom_toward_point(app.zoom_spread.pan, cursor - base_rect.center(), old_zoom, app.zoom_spread.scale);
                if app.zoom_spread.scale <= ZOOM_MIN {
                    app.zoom_spread = PageZoom::NONE;
                }
            }
        }

        // Mirrors the header's blue-light-filter slider, which also saves
        // on every change rather than debouncing — a small, fast local
        // write, not worth complicating this over.
        if app.zoom_locked {
            app.save_config();
        }
    }

    // In `SinglePage` mode each side's pan is applied within its own
    // (roughly half-width) box rather than the whole reading area, so
    // that — not `base_rect` — is the basis for how far it can travel.
    clamp_pan(&mut app.zoom_spread, base_rect.size());
    if single_page_target {
        let half_width = Vec2::new(base_rect.width() / 2.0, base_rect.height());
        clamp_pan(&mut app.zoom_left, half_width);
        clamp_pan(&mut app.zoom_right, half_width);
    }
}

/// Clamps `zoom.pan` so its content can't be dragged entirely out of a box
/// of size `bounds` — the overflow (how much bigger than `bounds` the
/// zoomed content is) is the most either axis can travel in either
/// direction before the content's edge would leave the box's.
fn clamp_pan(zoom: &mut PageZoom, bounds: Vec2) {
    let overflow = ((bounds * (zoom.scale - 1.0)) / 2.0).max(Vec2::ZERO);
    zoom.pan.x = zoom.pan.x.clamp(-overflow.x, overflow.x);
    zoom.pan.y = zoom.pan.y.clamp(-overflow.y, overflow.y);
}

/// The pan offset that keeps `cursor_offset` (a point relative to the
/// viewport's center) visually fixed on screen while zoom changes from
/// `old_zoom` to `new_zoom`.
fn zoom_toward_point(old_pan: Vec2, cursor_offset: Vec2, old_zoom: f32, new_zoom: f32) -> Vec2 {
    if old_zoom <= 0.0 {
        return old_pan;
    }
    let scale = new_zoom / old_zoom;
    cursor_offset * (1.0 - scale) + old_pan * scale
}

/// Draws one spread — a lone double-page image centered across the whole
/// `columns.full` rect if `full_spread`, otherwise `pages` in their own
/// columns as usual — shifted by `shift` (used while sliding). `zoom`, if
/// either side is set, locally magnifies just that side's slot — see
/// `draw_page_slot`.
fn draw_spread(
    ui: &mut Ui,
    columns: &Columns,
    shift: Vec2,
    pages: (Option<usize>, Option<usize>),
    full_spread: bool,
    zoom: SideZoom,
    ctx: &mut DecodeCtx,
) {
    let (left_idx, right_idx) = pages;
    if full_spread {
        draw_page_slot(ui, columns.full.translate(shift), Align::Center, left_idx, zoom.left.or(zoom.right), ctx);
    } else {
        draw_page_slot(ui, columns.left.translate(shift), Align::Max, left_idx, zoom.left, ctx);
        draw_page_slot(ui, columns.right.translate(shift), Align::Min, right_idx, zoom.right, ctx);
    }
}

/// Paints a page's image inside `column`, always filling it top-to-bottom
/// and horizontally anchored to `align` (`Min` = flush left, `Max` = flush
/// right, `Center` = centered — used for a lone double-page spread) — an
/// empty column (no page at this spot, e.g. the last page of an odd-length
/// book) still reserves its space so the other page doesn't jump to fill it.
///
/// `zoom`, if set, is `(magnification, pan)` for `ZoomTarget::SinglePage`:
/// the image is scaled up from its own (unzoomed) center — not `column`'s —
/// so it grows in place rather than jumping the moment zoom engages, offset
/// by `pan`, and clipped to `column` so the magnified page stays inside its
/// own box instead of spilling into the facing page's.
fn draw_page_slot(ui: &mut Ui, column: Rect, align: Align, page_idx: Option<usize>, zoom: Option<(f32, Vec2)>, ctx: &mut DecodeCtx) {
    let Some(page_idx) = page_idx else {
        return;
    };
    let Some(data) = ctx.pages.get(page_idx) else {
        return;
    };

    let texture = match ctx.textures.entry(page_idx) {
        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
        std::collections::hash_map::Entry::Vacant(entry) => {
            // Only reached when the background prefetch (see
            // `ComicApp::request_prefetch`) hasn't produced this page's
            // texture yet — e.g. right after opening a file, or a jump to a
            // spread outside the prefetch window. Decoding here blocks this
            // frame, but it's the exception rather than the rule.
            let image = match ComicArchive::decode_image(data, ctx.max_dimension) {
                Ok(image) => image,
                Err(err) => {
                    tracing::warn!("Failed to decode page {page_idx}: {err}");
                    return;
                }
            };
            ctx.page_meta.entry(page_idx).or_insert_with(|| PageMeta::sample(&image));
            entry.insert(ui.ctx().load_texture(format!("page_{page_idx}"), image, TextureOptions::LINEAR))
        }
    };

    let display_size = fit_to_height(texture.size_vec2(), column.height());
    let x_min = match align {
        Align::Min => column.left(),
        Align::Max => column.right() - display_size.x,
        Align::Center => column.center().x - display_size.x / 2.0,
    };
    let base_image_rect = Rect::from_min_size(Pos2::new(x_min, column.top()), display_size);

    match zoom {
        None => egui::Image::new(&*texture).paint_at(ui, base_image_rect),
        Some((magnification, pan)) => {
            let zoomed_rect =
                Rect::from_center_size(base_image_rect.center() + pan, base_image_rect.size() * magnification);
            let previous_clip = ui.clip_rect();
            ui.set_clip_rect(column.intersect(previous_clip));
            egui::Image::new(&*texture).paint_at(ui, zoomed_rect);
            ui.set_clip_rect(previous_clip);
        }
    }
}

/// Fills the gap between two facing pages (from `x_left` to `x_right`) with
/// a two-stop gradient from `left_idx`'s own right-edge color to
/// `right_idx`'s own left-edge color — an all-white or all-black spread
/// keeps a solid white/black gap, two different pages blend between them,
/// so the gap reads as part of the page rather than flat app background.
/// Falls back to whichever single side's color is known if only one page
/// exists (e.g. the last page of an odd-length book), and draws nothing if
/// neither's edge color has been sampled yet.
fn draw_gap_fill(
    painter: &egui::Painter,
    page_meta: &HashMap<usize, PageMeta>,
    rect: Rect,
    left_idx: Option<usize>,
    right_idx: Option<usize>,
) {
    if rect.width() <= 0.0 {
        return;
    }
    let left_color = left_idx.and_then(|i| page_meta.get(&i)).map(|m| m.edge.right);
    let right_color = right_idx.and_then(|i| page_meta.get(&i)).map(|m| m.edge.left);
    let (left_color, right_color) = match (left_color, right_color) {
        (Some(l), Some(r)) => (l, r),
        (Some(l), None) => (l, l),
        (None, Some(r)) => (r, r),
        (None, None) => return,
    };

    let mut mesh = egui::Mesh::default();
    mesh.vertices.extend([
        egui::epaint::Vertex { pos: rect.left_top(), uv: egui::epaint::WHITE_UV, color: left_color },
        egui::epaint::Vertex { pos: rect.left_bottom(), uv: egui::epaint::WHITE_UV, color: left_color },
        egui::epaint::Vertex { pos: rect.right_top(), uv: egui::epaint::WHITE_UV, color: right_color },
        egui::epaint::Vertex { pos: rect.right_bottom(), uv: egui::epaint::WHITE_UV, color: right_color },
    ]);
    mesh.indices.extend([0, 1, 3, 0, 3, 2]);
    painter.add(egui::Shape::mesh(mesh));
}

/// Paints a soft shadow straddling `x` from `top` to `bottom`, fading from
/// `peak_color` at the center to fully transparent `half_width` points out
/// on *both* sides — a symmetric gradient meant to read as the way a
/// physical book's pages curve away into shadow near the binding, rather
/// than a flat printed line.
fn draw_spine_shadow(painter: &egui::Painter, x: f32, top: f32, bottom: f32, half_width: f32, peak_color: Color32) {
    let column = |px: f32, color: Color32| {
        [egui::epaint::Vertex { pos: Pos2::new(px, top), uv: egui::epaint::WHITE_UV, color }, egui::epaint::Vertex {
            pos: Pos2::new(px, bottom),
            uv: egui::epaint::WHITE_UV,
            color,
        }]
    };
    let transparent = Color32::TRANSPARENT;

    let mut mesh = egui::Mesh::default();
    mesh.vertices.extend(column(x - half_width, transparent));
    mesh.vertices.extend(column(x, peak_color));
    mesh.vertices.extend(column(x + half_width, transparent));
    // Two quads (each two triangles): edge-to-center on the left, center-to-edge on the right.
    mesh.indices.extend([0, 1, 3, 0, 3, 2, 2, 3, 5, 2, 5, 4]);

    painter.add(egui::Shape::mesh(mesh));
}

/// The page-index window (inclusive) within `TEXTURE_KEEP_RADIUS` of the
/// currently displayed pages — used both to decide which textures to evict
/// and which pages to prefetch, so the two stay in sync by construction.
/// `None` when there's nothing currently displayed.
fn spread_window(current_pages: impl Iterator<Item = usize>) -> Option<(usize, usize)> {
    let (min_page, max_page) = current_pages.fold((usize::MAX, 0usize), |(min, max), page| {
        (min.min(page), max.max(page))
    });
    if min_page > max_page {
        return None;
    }
    Some((min_page.saturating_sub(TEXTURE_KEEP_RADIUS), max_page + TEXTURE_KEEP_RADIUS))
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

/// Whether `idx`'s own image is a double-page spread (as opposed to merely
/// being shown alone because it's the trailing page of an odd-length book,
/// or because its would-be partner is the double-page instead) — decides
/// whether a lone page renders centered across the full width or flush in
/// its normal column. `false` if `idx` is `None` or hasn't been decoded yet.
fn is_double_page(idx: Option<usize>, page_meta: &HashMap<usize, PageMeta>) -> bool {
    idx.is_some_and(|i| page_meta.get(&i).is_some_and(|m| m.is_spread))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 0.001;

    fn assert_vec2_approx(a: Vec2, b: Vec2) {
        assert!((a - b).length() < EPSILON, "{a:?} != {b:?}");
    }

    #[test]
    fn zoom_toward_point_keeps_the_cursor_offset_fixed_from_rest() {
        // Zooming in 2x centered on a point 40 points right of screen
        // center, starting unpanned, should pull the pan left so that same
        // point stays under the cursor.
        let pan = zoom_toward_point(Vec2::ZERO, Vec2::new(40.0, 0.0), 1.0, 2.0);
        assert_vec2_approx(pan, Vec2::new(-40.0, 0.0));
    }

    #[test]
    fn zoom_toward_point_is_a_no_op_when_unpanned_and_centered() {
        // Nothing offset from center and no existing pan: zooming further
        // has nothing to correct for.
        let pan = zoom_toward_point(Vec2::ZERO, Vec2::ZERO, 1.0, 3.0);
        assert_vec2_approx(pan, Vec2::ZERO);
    }

    #[test]
    fn zoom_toward_point_scales_an_existing_pan_even_when_cursor_is_centered() {
        // The cursor being at dead center doesn't mean nothing moves: an
        // already-panned view keeps scaling around the screen's center, not
        // around wherever the pan happens to be.
        let pan = zoom_toward_point(Vec2::new(5.0, -5.0), Vec2::ZERO, 1.0, 3.0);
        assert_vec2_approx(pan, Vec2::new(15.0, -15.0));
    }

    #[test]
    fn zoom_toward_point_scales_existing_pan_when_zoom_changes_further() {
        // Zooming from 2x to 4x (another 2x on top of an already-applied
        // pan) should double that existing pan alongside the new
        // cursor-offset correction.
        let pan = zoom_toward_point(Vec2::new(-40.0, 0.0), Vec2::new(40.0, 0.0), 2.0, 4.0);
        assert_vec2_approx(pan, Vec2::new(-120.0, 0.0));
    }
}
