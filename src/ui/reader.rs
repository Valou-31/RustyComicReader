use crate::app::{
    ComicApp, PageZoom, ReadingMode, WEBTOON_WIDTH_PCT_MAX, WEBTOON_WIDTH_PCT_MIN, ZOOM_MAX, ZOOM_MIN, ZoomTarget,
};
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
    fore_edge: ForeEdgeCtx<'a>,
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

/// What `draw_fore_edge_slot` needs to paint a page's own fore-edge bar —
/// bundled separately from `DecodeCtx` (rather than just passing `app`)
/// because it's built from a disjoint borrow of `ComicApp::fore_edge_texture`
/// taken *before* `DecodeCtx` takes its own mutable borrows of `textures`/
/// `page_meta`; the two can then coexist for the rest of the frame.
struct ForeEdgeCtx<'a> {
    texture: Option<&'a TextureHandle>,
    total_pages: usize,
    /// Whether the pages already read pile up on the screen's left side —
    /// true for LTR/Single, false for RTL (manga), where the book opens
    /// from the right.
    read_on_left: bool,
}

impl ForeEdgeCtx<'_> {
    /// What fraction of the book `page_idx` is into, `0.0`..=`1.0` by
    /// 1-based page number — `None` if there's no page here or the book is
    /// too short (one page) for a read/unread split to mean anything.
    fn read_fraction(&self, page_idx: Option<usize>) -> Option<f32> {
        if self.total_pages <= 1 {
            return None;
        }
        let number = page_idx? + 1;
        Some((number as f32 / self.total_pages as f32).clamp(0.0, 1.0))
    }
}

/// Dispatches to the right reader implementation for `app.reading_mode` —
/// the continuous vertical strip (`draw_webtoon`) for `ReadingMode::Webtoon`,
/// the paginated spread view (`draw_double_page`) for everything else. The
/// two are different enough (continuous scroll position vs. discrete
/// spreads/transitions) that sharing one function would mean threading a
/// mode check through nearly every helper in this module instead of just
/// picking between two self-contained ones up front.
pub fn draw_reader(ui: &mut Ui, app: &mut ComicApp) {
    if app.reading_mode == ReadingMode::Webtoon {
        draw_webtoon(ui, app);
    } else {
        draw_double_page(ui, app);
    }
}

/// Reference width, in synthetic "document units", that `draw_webtoon` lays
/// its cumulative page offsets out in — deliberately independent of actual
/// screen pixels, so `ComicApp::webtoon_scroll` (also in these units) stays
/// meaningful across a window resize or a page-width% change instead of
/// needing to be rescaled every time either one does. The value itself is
/// arbitrary; only its ratio to page heights (via each page's aspect ratio)
/// matters.
const WEBTOON_DOC_WIDTH: f32 = 1000.0;

/// Fallback aspect ratio (width/height) reserved for a page's slot in the
/// strip before it's actually been decoded — most webtoon/manga pages run
/// taller than wide, so this keeps the not-yet-known layout close enough to
/// the real thing that a page finishing decode doesn't visibly jerk the
/// scroll position. Replaced by `PageMeta::aspect` the moment a page's
/// texture is ready.
const WEBTOON_DEFAULT_ASPECT: f32 = 0.7;

/// How far beyond the visible viewport (in document units, each direction)
/// pages are kept decoded and prefetched — see `ui::reader::draw_webtoon`.
/// Generous relative to a typical viewport height so a normal reading pace
/// never outruns the background decode worker; a page's decode has already
/// started well before it scrolls into view.
const WEBTOON_PRELOAD_MARGIN: f32 = 2000.0;

/// One page's height in document units, from its known aspect ratio (once
/// decoded, via `page_meta`) or `WEBTOON_DEFAULT_ASPECT` until then.
fn webtoon_page_height(idx: usize, page_meta: &HashMap<usize, PageMeta>) -> f32 {
    let aspect = page_meta.get(&idx).map_or(WEBTOON_DEFAULT_ASPECT, |m| m.aspect);
    WEBTOON_DOC_WIDTH / aspect.max(0.05)
}

/// Every page's own top offset (cumulative height of everything before it),
/// in document units, plus the book's total height — see
/// `webtoon_page_height`.
fn webtoon_offsets(total_pages: usize, page_meta: &HashMap<usize, PageMeta>) -> (Vec<f32>, f32) {
    let mut offsets = Vec::with_capacity(total_pages);
    let mut cursor = 0.0f32;
    for idx in 0..total_pages {
        offsets.push(cursor);
        cursor += webtoon_page_height(idx, page_meta);
    }
    (offsets, cursor)
}

/// Which contiguous page-index range has any part of its own slot within
/// `[keep_top, keep_bottom]` (see `WEBTOON_PRELOAD_MARGIN`) — `None` if
/// `offsets` is empty — and which single page counts as "current": the
/// first one whose bottom edge is below `scroll`, or the book's last page
/// if `scroll` is at (or past) the very end. Offsets are monotonically
/// increasing with page index (see `webtoon_offsets`), so the set of pages
/// intersecting any contiguous vertical range is itself always a
/// contiguous index range — hence returning bounds rather than a `Vec`.
fn webtoon_visible_range(
    offsets: &[f32],
    page_meta: &HashMap<usize, PageMeta>,
    scroll: f32,
    keep_top: f32,
    keep_bottom: f32,
) -> (Option<(usize, usize)>, usize) {
    let mut keep_range = None;
    let mut current_page = offsets.len().saturating_sub(1);
    let mut current_found = false;
    for (idx, &page_top) in offsets.iter().enumerate() {
        let page_bottom = page_top + webtoon_page_height(idx, page_meta);
        if page_bottom >= keep_top && page_top <= keep_bottom {
            keep_range = Some(match keep_range {
                Some((low, _)) => (low, idx),
                None => (idx, idx),
            });
        }
        if !current_found && page_bottom > scroll {
            current_page = idx;
            current_found = true;
        }
    }
    (keep_range, current_page)
}

/// Draws every page of the book as one continuous vertical strip —
/// `ReadingMode::Webtoon`. Each page is scaled to `app.webtoon_page_width_pct`
/// of the available width (preserving its own aspect ratio) and stacked
/// directly against its neighbors with no gap and no per-page chrome, so a
/// long vertical scroll reads as one unbroken image rather than a sequence
/// of individually "turned" pages. Scrolling itself comes from two sources:
/// `input::keyboard` (arrow keys, accumulated into `app.webtoon_scroll`
/// every frame that key is held) and, read directly below, two-finger
/// trackpad/mouse wheel scroll. This function turns that scroll position
/// into what's actually on screen, and is also the only place that knows
/// enough about the current layout to resolve `webtoon_scroll_target` (a
/// pending jump from opening a book, switching into this mode, or the
/// progress bar/bookmarks/"Go to Page") into an actual offset.
fn draw_webtoon(ui: &mut Ui, app: &mut ComicApp) {
    let base_rect = ui.available_rect_before_wrap();
    ui.set_clip_rect(base_rect);

    if app.total_pages == 0 || base_rect.width() <= 0.0 || base_rect.height() <= 0.0 {
        return;
    }

    let width_fraction =
        (app.webtoon_page_width_pct / 100.0).clamp(WEBTOON_WIDTH_PCT_MIN / 100.0, WEBTOON_WIDTH_PCT_MAX / 100.0);
    let displayed_width = base_rect.width() * width_fraction;
    let scale = displayed_width / WEBTOON_DOC_WIDTH;
    let viewport_doc_height = base_rect.height() / scale;

    let (offsets, total_doc_height) = webtoon_offsets(app.total_pages, &app.page_meta);

    if let Some(target) = app.webtoon_scroll_target.take() {
        app.webtoon_scroll = offsets.get(target).copied().unwrap_or(0.0);
    }

    // Two-finger trackpad (or mouse wheel) vertical scroll — natural 1:1
    // tracking with the content, same convention `egui::ScrollArea` itself
    // uses (`offset -= scroll_delta`), so it feels like every other
    // scrollable view in the app. `smooth_scroll_delta` is already in
    // screen points, so dividing by `scale` converts it to the same
    // document units `webtoon_scroll` is kept in (see `WEBTOON_DOC_WIDTH`);
    // it's also already zeroed out by egui itself while the zoom modifier
    // (Cmd/Ctrl) is held, so a pinch-zoom gesture doesn't also scroll.
    // Skipped while a modal panel is open, so a scroll meant for Settings/
    // History/Bookmarks underneath doesn't leak through — same guard
    // `handle_zoom_and_pan` uses for the paginated view's own pan/zoom.
    if !(app.show_settings || app.show_history || app.show_bookmarks) {
        let wheel_delta_y = ui.ctx().input(|i| i.smooth_scroll_delta().y);
        if wheel_delta_y != 0.0 {
            app.webtoon_scroll -= wheel_delta_y / scale;
            ui.ctx().request_repaint();
        }
    }

    let max_scroll = (total_doc_height - viewport_doc_height).max(0.0);
    app.webtoon_scroll = app.webtoon_scroll.clamp(0.0, max_scroll);
    let scroll = app.webtoon_scroll;

    // Which pages to keep decoded/resident and which to prefetch — anything
    // whose own slot falls within `WEBTOON_PRELOAD_MARGIN` doc units of the
    // visible viewport, ahead so approaching a page's end already has the
    // next one ready (per the brief: preload before the reader gets there),
    // behind so scrolling back up doesn't have to re-decode what was just
    // shown.
    let keep_top = scroll - WEBTOON_PRELOAD_MARGIN;
    let keep_bottom = scroll + viewport_doc_height + WEBTOON_PRELOAD_MARGIN;
    let (keep_range, current_page) =
        webtoon_visible_range(&offsets, &app.page_meta, scroll, keep_top, keep_bottom);
    app.update_webtoon_position(current_page);

    let Some((low, high)) = keep_range else { return };
    app.textures.retain(|&idx, _| (low..=high).contains(&idx));
    app.page_meta.retain(|&idx, _| (low..=high).contains(&idx));
    app.request_prefetch(low, high);

    let max_dimension = app.decode_max_dimension();
    let mut ctx = DecodeCtx {
        pages: &app.pages,
        textures: &mut app.textures,
        page_meta: &mut app.page_meta,
        max_dimension,
        // No fore-edge strip in Webtoon mode — the whole point is a
        // seamless strip with nothing painted alongside a page's own edge.
        fore_edge: ForeEdgeCtx { texture: None, total_pages: app.total_pages, read_on_left: true },
    };

    let x_min = base_rect.center().x - displayed_width / 2.0;
    for idx in low..=high {
        let page_top = offsets[idx];
        let height_doc = webtoon_page_height(idx, ctx.page_meta);
        let page_bottom = page_top + height_doc;
        // Kept resident (just decoded/prefetched) but currently scrolled
        // out of view — nothing to paint for it this frame.
        if page_bottom < scroll || page_top > scroll + viewport_doc_height {
            continue;
        }
        let y_top = base_rect.top() + (page_top - scroll) * scale;
        let column = Rect::from_min_size(Pos2::new(x_min, y_top), Vec2::new(displayed_width, height_doc * scale));
        draw_page_slot(ui, column, Align::Center, Some(idx), None, &mut ctx);
    }
}

fn draw_double_page(ui: &mut Ui, app: &mut ComicApp) {
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
    let mut ctx = DecodeCtx {
        pages: &app.pages,
        textures: &mut app.textures,
        page_meta: &mut app.page_meta,
        max_dimension,
        fore_edge: ForeEdgeCtx {
            texture: app.fore_edge_texture.as_ref(),
            total_pages: app.total_pages,
            read_on_left: app.reading_mode != ReadingMode::RTL,
        },
    };

    // Left page flush against the right edge of its column, right page
    // flush against the left edge of its — both meet at `mid_x` (or the
    // configured gap around it), so the spread reads as one continuous book
    // opening rather than two independently centered pages with a gap. A
    // lone double-page spread, an isolated page (`E`), or any page at all in
    // Single Page mode instead spans the full width, centered. Each spread's
    // seam travels with it, so both are drawn at their shifted `mid_x` while
    // sliding.
    let seams: [Option<SpreadSeam>; 2] = if let Some((progress, entry_sign, old_left, old_right, new_left, new_right)) = sliding {
        ui.ctx().request_repaint();
        let width = layout_rect.width();
        let new_shift = Vec2::new(entry_sign * (1.0 - progress) * width, 0.0);
        let old_shift = Vec2::new(-entry_sign * progress * width, 0.0);

        let old_full = old_right.is_none()
            && (single_page_mode || is_double_page(old_left, ctx.page_meta) || is_isolated(old_left, &app.isolated_pages));
        let new_full = new_right.is_none()
            && (single_page_mode || is_double_page(new_left, ctx.page_meta) || is_isolated(new_left, &app.isolated_pages));
        // Never zoomed while sliding (see `sliding` above), so nothing to
        // pass here.
        draw_spread(ui, &columns, old_shift, (old_left, old_right), old_full, SideZoom::default(), &mut ctx);
        draw_spread(ui, &columns, new_shift, (new_left, new_right), new_full, SideZoom::default(), &mut ctx);

        [
            Some(SpreadSeam { mid_x: mid_x + old_shift.x, left_idx: old_left, right_idx: old_right, full_spread: old_full }),
            Some(SpreadSeam { mid_x: mid_x + new_shift.x, left_idx: new_left, right_idx: new_right, full_spread: new_full }),
        ]
    } else {
        let full_spread = current_right.is_none()
            && (single_page_mode || is_double_page(current_left, ctx.page_meta) || is_isolated(current_left, &app.isolated_pages));
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

/// Paints `page_idx`'s own fore-edge bar(s) — see `ui::fore_edge::paint_bar`
/// — immediately outward from wherever it actually renders within `column`,
/// mirroring `draw_page_slot`'s own fit-to-height placement math so the bar
/// sits exactly where the page's own paper would continue. Called right
/// alongside `draw_page_slot` for the very same `column`/`align`/`page_idx`
/// (see `draw_spread`), so the bar is positioned, translated during a
/// page-turn slide, and clipped exactly like the page itself, with no
/// separate fallback needed for any of that. `Align::Max`/`Align::Min` (the
/// normal two-page-spread columns) each get one bar, on whichever side
/// faces away from the spine — the spine side is already handled by the
/// gap/shadow/line drawn in `draw_double_page`. `Align::Center` (a lone
/// double-page spread) gets one on each side, since both its edges face
/// outward. No-op if the page's texture hasn't decoded yet (nothing to
/// anchor the bar to) or the fore-edge composite itself isn't ready.
fn draw_fore_edge_slot(ui: &Ui, ctx: &DecodeCtx, column: Rect, align: Align, page_idx: Option<usize>) {
    let Some(texture) = ctx.fore_edge.texture else { return };
    let Some(read_fraction) = ctx.fore_edge.read_fraction(page_idx) else { return };
    let Some(size) = page_idx.and_then(|idx| ctx.textures.get(&idx)).map(|t| t.size_vec2()) else { return };

    let display_size = fit_to_height(size, column.height());
    let x_min = match align {
        Align::Min => column.left(),
        Align::Max => column.right() - display_size.x,
        Align::Center => column.center().x - display_size.x / 2.0,
    };
    let x_max = x_min + display_size.x;

    // `extends_left` (the bar grows further left, away from the page) also
    // says which screen side this bar is on, which is exactly what decides
    // whether it's the "read" or "unread" half against `read_on_left`.
    let read_on_left = ctx.fore_edge.read_on_left;
    match align {
        Align::Max => crate::ui::fore_edge::paint_bar(ui, texture, column, x_min, true, read_on_left, read_fraction),
        Align::Min => crate::ui::fore_edge::paint_bar(ui, texture, column, x_max, false, read_on_left, read_fraction),
        Align::Center => {
            crate::ui::fore_edge::paint_bar(ui, texture, column, x_min, true, read_on_left, read_fraction);
            crate::ui::fore_edge::paint_bar(ui, texture, column, x_max, false, read_on_left, read_fraction);
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

    if app.show_settings || app.show_history || app.show_bookmarks {
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
        let column = columns.full.translate(shift);
        draw_page_slot(ui, column, Align::Center, left_idx, zoom.left.or(zoom.right), ctx);
        draw_fore_edge_slot(ui, ctx, column, Align::Center, left_idx);
    } else {
        let left_column = columns.left.translate(shift);
        draw_page_slot(ui, left_column, Align::Max, left_idx, zoom.left, ctx);
        draw_fore_edge_slot(ui, ctx, left_column, Align::Max, left_idx);

        let right_column = columns.right.translate(shift);
        draw_page_slot(ui, right_column, Align::Min, right_idx, zoom.right, ctx);
        draw_fore_edge_slot(ui, ctx, right_column, Align::Min, right_idx);
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

/// Whether `idx` has been individually pinned to display alone via `E`
/// (`ComicApp::toggle_isolate_current_page`) — same centering treatment as
/// an actual double-page spread, since it's just as much "one page taking
/// up the whole opening" from a layout point of view.
fn is_isolated(idx: Option<usize>, isolated_pages: &std::collections::HashSet<usize>) -> bool {
    idx.is_some_and(|i| isolated_pages.contains(&i))
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

    fn page_meta_with_aspect(aspect: f32) -> PageMeta {
        use crate::comic::archive::EdgeColors;
        let placeholder = EdgeColors { left: Color32::WHITE, right: Color32::WHITE };
        PageMeta { edge: placeholder, is_spread: false, aspect }
    }

    #[test]
    fn webtoon_page_height_uses_the_known_aspect_ratio_once_decoded() {
        let mut page_meta = HashMap::new();
        // Square: height in doc units equals the doc width.
        page_meta.insert(0, page_meta_with_aspect(1.0));
        assert!((webtoon_page_height(0, &page_meta) - WEBTOON_DOC_WIDTH).abs() < EPSILON);
    }

    #[test]
    fn webtoon_page_height_falls_back_to_the_default_guess_when_not_yet_decoded() {
        let page_meta = HashMap::new();
        let expected = WEBTOON_DOC_WIDTH / WEBTOON_DEFAULT_ASPECT;
        assert!((webtoon_page_height(0, &page_meta) - expected).abs() < EPSILON);
    }

    #[test]
    fn webtoon_offsets_stacks_pages_directly_on_top_of_each_other() {
        let mut page_meta = HashMap::new();
        page_meta.insert(0, page_meta_with_aspect(1.0)); // height == WEBTOON_DOC_WIDTH
        page_meta.insert(1, page_meta_with_aspect(2.0)); // height == WEBTOON_DOC_WIDTH / 2

        let (offsets, total) = webtoon_offsets(3, &page_meta);

        assert_eq!(offsets.len(), 3);
        assert!((offsets[0] - 0.0).abs() < EPSILON);
        assert!((offsets[1] - WEBTOON_DOC_WIDTH).abs() < EPSILON);
        let page1_height = WEBTOON_DOC_WIDTH / 2.0;
        assert!((offsets[2] - (WEBTOON_DOC_WIDTH + page1_height)).abs() < EPSILON);
        // Page 2 hasn't been decoded — falls back to the default aspect.
        let page2_height = WEBTOON_DOC_WIDTH / WEBTOON_DEFAULT_ASPECT;
        assert!((total - (WEBTOON_DOC_WIDTH + page1_height + page2_height)).abs() < EPSILON);
    }

    #[test]
    fn webtoon_offsets_of_an_empty_book_is_empty_with_zero_height() {
        let (offsets, total) = webtoon_offsets(0, &HashMap::new());
        assert!(offsets.is_empty());
        assert_eq!(total, 0.0);
    }

    #[test]
    fn webtoon_visible_range_keeps_only_pages_within_the_margin() {
        // Five same-height (WEBTOON_DOC_WIDTH each) pages stacked at 0,
        // 1000, 2000, 3000, 4000 (so page `i` spans `[1000*i, 1000*(i+1))`).
        // A viewport scrolled to 2000 with a margin of 900 (keep window
        // [1100, 2900]) should keep only pages 1 and 2 — page 0 ends at
        // 1000 (short of 1100) and page 3 starts at 3000 (past 2900).
        let mut page_meta = HashMap::new();
        for i in 0..5 {
            page_meta.insert(i, page_meta_with_aspect(1.0));
        }
        let (offsets, _) = webtoon_offsets(5, &page_meta);

        let scroll = 2000.0;
        let (range, current) = webtoon_visible_range(&offsets, &page_meta, scroll, scroll - 900.0, scroll + 900.0);

        assert_eq!(range, Some((1, 2)));
        assert_eq!(current, 2); // page 2 spans [2000, 3000), its bottom is the first past `scroll`
    }

    #[test]
    fn webtoon_visible_range_current_page_falls_back_to_the_last_page_at_the_very_end() {
        let mut page_meta = HashMap::new();
        for i in 0..3 {
            page_meta.insert(i, page_meta_with_aspect(1.0));
        }
        let (offsets, total) = webtoon_offsets(3, &page_meta);

        // Scrolled exactly to the bottom: no page's bottom edge is strictly
        // past `scroll` anymore, so `current` must still land on a valid
        // page (the last one) rather than an out-of-range fallback.
        let (_, current) = webtoon_visible_range(&offsets, &page_meta, total, total - 100.0, total + 100.0);
        assert_eq!(current, 2);
    }

    #[test]
    fn webtoon_visible_range_of_an_empty_book_keeps_nothing() {
        let (range, current) = webtoon_visible_range(&[], &HashMap::new(), 0.0, 0.0, 0.0);
        assert_eq!(range, None);
        assert_eq!(current, 0);
    }
}
