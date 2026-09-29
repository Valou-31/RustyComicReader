use crate::app::{
    ComicApp, PageZoom, ReadingMode, SiblingPreviewState, WEBTOON_WIDTH_PCT_MAX, WEBTOON_WIDTH_PCT_MIN, WebtoonEdgeDrag,
    ZOOM_MAX, ZOOM_MIN, ZoomTarget,
};
use crate::comic::archive::{ComicArchive, PageMeta};
use egui::{Align, Align2, Color32, Event, Pos2, Rect, Stroke, TextureHandle, TextureOptions, TouchPhase, Ui, Vec2};
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

/// Raw (physical-points, unscaled by zoom/`webtoon_wheel_sensitivity`) drag
/// distance past the edge of the strip that maps to a full `0.0..=1.0`
/// `WebtoonEdgeDrag::progress` — the vertical counterpart of
/// `input::scroll::DRAG_FULL_DISTANCE` (`220.0`). Somewhat larger than that
/// horizontal page-turn distance — this changes which *book* is open, not
/// just the page, so it should still take a clearly deliberate push, not a
/// hair-trigger flick — but nowhere near the `1200.0`-plus-decay the
/// previous accumulator design needed, since that design had to size the
/// threshold to also outrun trackpad momentum on its own (no `TouchPhase`
/// gating); `webtoon_edge_drag_step` now does that gating structurally
/// instead (see `WebtoonEdgeDrag`'s docs), so this constant is free to be
/// just "how far is a deliberate push," the same job `DRAG_FULL_DISTANCE`
/// does for a page turn.
const WEBTOON_EDGE_DRAG_FULL_DISTANCE: f32 = 400.0;

/// `WebtoonEdgeDrag::progress` fraction that commits at release if the
/// gesture wasn't fast enough to count as a fling (see
/// `WEBTOON_EDGE_DRAG_FLING_VELOCITY`) — same role and value as
/// `ComicApp::end_page_drag`'s own `COMMIT_THRESHOLD`.
const WEBTOON_EDGE_DRAG_COMMIT_THRESHOLD: f32 = 0.5;

/// `WebtoonEdgeDrag::velocity` (progress-units/second) fast enough at
/// release to commit regardless of how little of `WEBTOON_EDGE_DRAG_FULL_DISTANCE`
/// was actually covered — a quick, sharp push reads as just as deliberate
/// as a slow, full one. Same value and reasoning as `ComicApp::end_page_drag`'s
/// `FLING_VELOCITY`: comfortably above a slow full push (~1.0/s), below an
/// actual quick flick (~5+/s).
const WEBTOON_EDGE_DRAG_FLING_VELOCITY: f32 = 3.0;

/// How fast (in progress-units/second) a released-but-not-committed
/// `WebtoonEdgeDrag` settles its `progress` back to `0.0` — purely a
/// visual "let go and it springs back" cue; unlike the old
/// accumulator-based decay, this never runs while the gesture is still
/// live (see `WebtoonEdgeDrag`'s docs), so it can never eat into progress
/// the reader is actively building. Fast enough that letting go reads as an
/// immediate cancel rather than a lingering, ambiguous fade.
const WEBTOON_EDGE_DRAG_RELEASE_DECAY_PER_SECOND: f32 = 3.5;

/// One page's height in document units, from its known aspect ratio (once
/// decoded, via `ComicApp::webtoon_aspect`) or `WEBTOON_DEFAULT_ASPECT`
/// until then.
///
/// The floor here (`0.001`, just guarding the division against a
/// degenerate/corrupt `0.0` aspect) must match `webtoon_slice_display_heights`'s
/// own floor exactly — this function decides how much doc-space (and so
/// screen space, via `draw_webtoon`'s column) a page reserves, while that
/// one decides how tall its texture actually renders; a *higher* floor
/// here (this used `0.05` until real webtoon-chapter content exposed the
/// bug) silently reserves less space than a page taller than 20x its own
/// width actually needs once decoded — and some merged-chapter webtoon
/// strips genuinely are (down to ~0.03 aspect, measured) — so its real
/// content overflows past the space `draw_webtoon` allocated it, drawing
/// over whatever comes next instead of being contained.
fn webtoon_page_height(idx: usize, aspect_map: &HashMap<usize, f32>) -> f32 {
    let aspect = aspect_map.get(&idx).copied().unwrap_or(WEBTOON_DEFAULT_ASPECT);
    WEBTOON_DOC_WIDTH / aspect.max(0.001)
}

/// Every page's own top offset (cumulative height of everything before it,
/// plus `gap` — `ComicApp::webtoon_page_gap` — between each pair), in
/// document units, plus the book's total height — see `webtoon_page_height`.
/// `gap` is only inserted *between* pages, never trailing after the last
/// one, so it doesn't inflate the scrollable height for no reason.
fn webtoon_offsets(total_pages: usize, aspect_map: &HashMap<usize, f32>, gap: f32) -> (Vec<f32>, f32) {
    let mut offsets = Vec::with_capacity(total_pages);
    let mut cursor = 0.0f32;
    for idx in 0..total_pages {
        if idx > 0 {
            cursor += gap;
        }
        offsets.push(cursor);
        cursor += webtoon_page_height(idx, aspect_map);
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
    aspect_map: &HashMap<usize, f32>,
    scroll: f32,
    keep_top: f32,
    keep_bottom: f32,
) -> (Option<(usize, usize)>, usize) {
    let mut keep_range = None;
    let mut current_page = offsets.len().saturating_sub(1);
    let mut current_found = false;
    for (idx, &page_top) in offsets.iter().enumerate() {
        let page_bottom = page_top + webtoon_page_height(idx, aspect_map);
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

/// One frame's worth of `draw_webtoon`'s wheel-scroll handling against the
/// Webtoon strip's own top/bottom edges, pulled out into a pure function so
/// it's testable without an `egui::Context` — the vertical, edge-gated
/// counterpart of `ComicApp::drag_page_by`/`end_page_drag`. Reconciles the
/// current scroll position and any live `WebtoonEdgeDrag` against a new
/// wheel delta and this frame's gesture-phase signals, and returns the
/// updated `(scroll, edge_drag, commit)` — `commit` is `Some(direction)` on
/// the exact frame a drag resolves to opening the sibling volume that way;
/// the caller still has to actually do that (a pure function can't touch
/// the filesystem).
///
/// `raw_delta` and `doc_delta` carry the same delta in two different units
/// — `raw_delta` in physical points (what `WebtoonEdgeDrag::progress` is
/// measured against, so the drag means the same physical gesture regardless
/// of zoom/window size, same reasoning `input::scroll::DRAG_FULL_DISTANCE`
/// uses) and `doc_delta` in the document units `scroll` itself is kept in
/// (see `WEBTOON_DOC_WIDTH`) — with the same sign. `scroll` is returned
/// *not yet* clamped to `[0, max_scroll]`; the caller still does that.
///
/// `gesture_started`/`gesture_ended` come from this frame's raw
/// `Event::MouseWheel` phases (`TouchPhase::Start`/`End`/`Cancel`), same
/// signal `input::scroll::handle_scroll` uses for the paginated swipe. A
/// drag can only ever *start* on a frame where `gesture_started` is true
/// and the strip is already sitting at that edge — never from the tail end
/// of whatever gesture carried it there (still `Move`-phase events, no new
/// `Start`) — see `WebtoonEdgeDrag`'s docs for why that matters. Once
/// started, it holds `progress` in place while merely held still (no
/// decay), and only resolves — commit or release-and-settle — on the frame
/// `gesture_ended` fires.
#[allow(clippy::too_many_arguments)] // each parameter is independently meaningful frame state; see the docs above
fn webtoon_edge_drag_step(
    scroll: f32,
    max_scroll: f32,
    raw_delta: f32,
    doc_delta: f32,
    dt: f32,
    gesture_started: bool,
    gesture_ended: bool,
    edge_drag: Option<WebtoonEdgeDrag>,
) -> (f32, Option<WebtoonEdgeDrag>, Option<i32>) {
    const VELOCITY_SMOOTHING: f32 = 0.3;

    if let Some(mut drag) = edge_drag {
        if !drag.dragging {
            // Released without committing: ignore any further input and
            // just settle back to rest, same as a bounced-back page-turn
            // drag settling toward `target: 0.0`.
            drag.progress = (drag.progress - WEBTOON_EDGE_DRAG_RELEASE_DECAY_PER_SECOND * dt).max(0.0);
            return if drag.progress <= 0.0 { (scroll + doc_delta, None, None) } else { (scroll, Some(drag), None) };
        }

        if gesture_ended {
            // `drag.velocity` is already in progress-space (see `signed`
            // below), where positive always means "still pushing further
            // past this edge" regardless of which edge `direction` is —
            // no second multiply by `direction` needed (or correct: doing
            // so would flip the sign right back for the top edge's `-1`).
            let commit = if drag.velocity.abs() >= WEBTOON_EDGE_DRAG_FLING_VELOCITY {
                drag.velocity > 0.0
            } else {
                drag.progress >= WEBTOON_EDGE_DRAG_COMMIT_THRESHOLD
            };
            if commit {
                return (scroll, None, Some(drag.direction));
            }
            drag.dragging = false;
            return (scroll, Some(drag), None);
        }

        // Still live: `signed` is this frame's push further past the edge
        // (positive) or back toward it (negative), along the drag's own
        // direction — symmetric with `ComicApp::drag_page_by`, which lets a
        // partial page-turn drag reduce its own progress the same way.
        let signed = raw_delta * drag.direction as f32;
        let delta_progress = signed / WEBTOON_EDGE_DRAG_FULL_DISTANCE;
        if dt > 0.0 {
            let sample = delta_progress / dt;
            drag.velocity += (sample - drag.velocity) * VELOCITY_SMOOTHING;
        }
        let new_progress = drag.progress + delta_progress;
        if delta_progress < 0.0 && new_progress <= 0.0 {
            // Pulled all the way back out of the edge before ever
            // releasing — the gesture is still live, so just hand control
            // back to normal scrolling instead of treating this like a
            // release. Gated on `delta_progress < 0.0` (actually pulling
            // back this frame), not just `new_progress <= 0.0` on its own —
            // a drag that starts at `0.0` and is simply held still for a
            // frame (zero delta) must not be mistaken for having already
            // been pulled back out.
            return (scroll + doc_delta, None, None);
        }
        drag.progress = new_progress.clamp(0.0, 1.0);
        return (scroll, Some(drag), None);
    }

    let at_bottom = scroll >= max_scroll;
    let at_top = scroll <= 0.0;
    if gesture_started && at_bottom && raw_delta > 0.0 {
        return (scroll, Some(WebtoonEdgeDrag { direction: 1, progress: 0.0, velocity: 0.0, dragging: true }), None);
    }
    if gesture_started && at_top && raw_delta < 0.0 {
        return (scroll, Some(WebtoonEdgeDrag { direction: -1, progress: 0.0, velocity: 0.0, dragging: true }), None);
    }
    (scroll + doc_delta, None, None)
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

    let (offsets, total_doc_height) = webtoon_offsets(app.total_pages, &app.webtoon_aspect, app.webtoon_page_gap);

    // A page's height is only a guess (`WEBTOON_DEFAULT_ASPECT`) until it's
    // actually decoded. If a page above the last known anchor finishes
    // decoding between frames, its corrected height shifts every offset
    // below it — including the anchor's own — even though `webtoon_scroll`
    // didn't move. Carry the scroll by the same delta so the page on screen
    // stays put instead of jumping as pages above it settle into their real
    // size.
    if let Some(anchor) = app.webtoon_anchor_page
        && let Some(&new_offset) = offsets.get(anchor)
    {
        app.webtoon_scroll += new_offset - app.webtoon_anchor_offset;
    }

    if let Some(target) = app.webtoon_scroll_target.take() {
        app.webtoon_scroll = offsets.get(target).copied().unwrap_or(0.0);
    }

    let max_scroll = (total_doc_height - viewport_doc_height).max(0.0);

    // Two-finger trackpad (or mouse wheel) vertical scroll — natural 1:1
    // tracking with the content by default, same convention
    // `egui::ScrollArea` itself uses (`offset -= scroll_delta`), so it feels
    // like every other scrollable view in the app; `webtoon_wheel_sensitivity`
    // scales that tracking up or down (independent of `webtoon_scroll_speed`,
    // which only affects holding a scroll key). `smooth_scroll_delta` is
    // already in screen points, so dividing by `scale` converts it to the
    // same document units `webtoon_scroll` is kept in (see
    // `WEBTOON_DOC_WIDTH`); it's also already zeroed out by egui itself
    // while the zoom modifier (Cmd/Ctrl) is held, so a pinch-zoom gesture
    // doesn't also scroll. Skipped while a modal panel is open, so a scroll
    // meant for Settings/History/Bookmarks underneath doesn't leak through —
    // same guard `handle_zoom_and_pan` uses for the paginated view's own
    // pan/zoom.
    //
    // Once already at either edge, a scroll that keeps pushing past it
    // doesn't move `webtoon_scroll` at all — it drives a live
    // `WebtoonEdgeDrag` instead (see `webtoon_edge_drag_step`), which
    // resolves to `ComicApp::open_sibling_volume` when the gesture ends
    // past its commit threshold, exactly like a manga page turn.
    if !(app.show_settings || app.show_history || app.show_bookmarks) {
        let wheel_delta_y = ui.ctx().input(|i| i.smooth_scroll_delta().y);
        let (gesture_started, gesture_ended) = ui.ctx().input(|i| {
            i.events.iter().fold((false, false), |(started, ended), event| match event {
                // A scroll held with the zoom modifier is ctrl/Cmd-scroll-
                // to-zoom, not a scroll gesture — same filter
                // `input::scroll::handle_scroll` uses for the paginated
                // view's own swipe.
                Event::MouseWheel { modifiers, .. } if modifiers.command => (started, ended),
                Event::MouseWheel { phase: TouchPhase::Start, .. } => (true, ended),
                Event::MouseWheel { phase: TouchPhase::End | TouchPhase::Cancel, .. } => (started, true),
                _ => (started, ended),
            })
        });

        if wheel_delta_y != 0.0 || gesture_started || gesture_ended || app.webtoon_edge_drag.is_some() {
            let raw_delta = if app.webtoon_scroll_inverted { wheel_delta_y } else { -wheel_delta_y };
            let doc_delta = raw_delta * app.webtoon_wheel_sensitivity / scale;
            let dt = ui.ctx().input(|i| i.stable_dt);
            let (new_scroll, new_drag, commit) = webtoon_edge_drag_step(
                app.webtoon_scroll,
                max_scroll,
                raw_delta,
                doc_delta,
                dt,
                gesture_started,
                gesture_ended,
                app.webtoon_edge_drag.take(),
            );
            app.webtoon_scroll = new_scroll;
            app.webtoon_edge_drag = new_drag;
            if let Some(direction) = commit {
                app.webtoon_sibling_preview = None;
                app.open_sibling_volume(direction);
            }
            ui.ctx().request_repaint();
        }
    }

    app.webtoon_scroll = app.webtoon_scroll.clamp(0.0, max_scroll);
    let scroll = app.webtoon_scroll;

    // How much of the viewport the incoming sibling volume's preview
    // should cover this frame — `0.0..=1.0`, signed by direction (positive
    // pushing up from the bottom, i.e. "next"; negative pushing down from
    // the top, i.e. "previous").
    let reveal_fraction = app.webtoon_edge_drag.as_ref().map(|d| d.progress * d.direction as f32).unwrap_or(0.0);

    app.sync_webtoon_sibling_preview(ui.ctx());

    // Existing content slides away from whichever edge is being pushed
    // against — up off the top as the strip overscrolls down past the
    // bottom (revealing the next volume sliding up to take its place),
    // or down off the bottom pulling up past the top — exactly the
    // paginated view's own page-turn slide (`draw_double_page`'s `sliding`
    // branch), just rotated 90°. `content_shift` is that same motion
    // applied uniformly to every page's `y_top` below, in screen points.
    let content_shift = reveal_fraction * base_rect.height();
    let x_min = base_rect.center().x - displayed_width / 2.0;
    if reveal_fraction != 0.0 {
        draw_sibling_preview_slide(ui, base_rect, x_min, displayed_width, reveal_fraction, app.webtoon_sibling_preview.as_ref());
    }

    // Which pages to keep decoded/resident and which to prefetch — anything
    // whose own slot falls within `WEBTOON_PRELOAD_MARGIN` doc units of the
    // visible viewport, ahead so approaching a page's end already has the
    // next one ready (per the brief: preload before the reader gets there),
    // behind so scrolling back up doesn't have to re-decode what was just
    // shown.
    let keep_top = scroll - WEBTOON_PRELOAD_MARGIN;
    let keep_bottom = scroll + viewport_doc_height + WEBTOON_PRELOAD_MARGIN;
    let (keep_range, current_page) =
        webtoon_visible_range(&offsets, &app.webtoon_aspect, scroll, keep_top, keep_bottom);
    app.update_webtoon_position(current_page);
    app.webtoon_anchor_page = Some(current_page);
    app.webtoon_anchor_offset = offsets.get(current_page).copied().unwrap_or(0.0);

    let Some((low, high)) = keep_range else { return };
    app.webtoon_textures.retain(|&idx, _| (low..=high).contains(&idx));
    // `webtoon_aspect` is deliberately *not* evicted the same way — unlike
    // `webtoon_textures` (real pixel data, genuinely worth bounding), an
    // aspect ratio is a few bytes per page. `ComicApp::page_dimensions`
    // already populates this for the *whole* book up front specifically so
    // `webtoon_offsets`'s `total_doc_height` (and so `max_scroll`) stays
    // accurate for pages outside the current window too — evicting it here
    // would silently undo that the moment a page scrolls out of range,
    // right back to `WEBTOON_DEFAULT_ASPECT`'s guess for it.
    app.request_webtoon_prefetch(low, high);

    let max_dimension = app.decode_max_dimension();
    let mut ctx =
        WebtoonSliceCtx { pages: &app.pages, textures: &mut app.webtoon_textures, aspect: &mut app.webtoon_aspect, max_dimension };

    for idx in low..=high {
        let page_top = offsets[idx];
        let height_doc = webtoon_page_height(idx, ctx.aspect);
        let page_bottom = page_top + height_doc;
        // Kept resident (just decoded/prefetched) but currently scrolled
        // out of view — nothing to paint for it this frame.
        if page_bottom < scroll || page_top > scroll + viewport_doc_height {
            continue;
        }
        let y_top = base_rect.top() + (page_top - scroll) * scale - content_shift;
        let column = Rect::from_min_size(Pos2::new(x_min, y_top), Vec2::new(displayed_width, height_doc * scale));
        draw_webtoon_page_slot(ui, column, idx, &mut ctx);
    }
}

/// Draws the incoming sibling volume's opening page sliding into the
/// screen — the "new page arriving" half of `draw_webtoon`'s
/// overscroll-past-the-edge transition; `content_shift` (computed
/// alongside this call) is the other half, the same motion applied to the
/// *existing* page content, together reading as one continuous slide, the
/// vertical counterpart of the paginated view's own page-turn
/// (`draw_double_page`'s `sliding` branch).
///
/// `reveal_fraction` is signed: positive reveals from the bottom (pushing
/// down past the end — the next volume), negative from the top (pulling up
/// past the start — the previous one); its magnitude (`0.0..=1.0`) is how
/// much of the viewport height is revealed so far.
///
/// Before the preview image has actually finished decoding (see
/// `ComicApp::sync_webtoon_sibling_preview`) — normally brief, but not
/// instant — falls back to a small fading label in the same revealed
/// region, so the gesture gives *some* feedback immediately rather than
/// looking like scrolling just stopped working until the preview happens
/// to land.
fn draw_sibling_preview_slide(
    ui: &Ui,
    base_rect: Rect,
    x_min: f32,
    displayed_width: f32,
    reveal_fraction: f32,
    preview: Option<&SiblingPreviewState>,
) {
    let at_top = reveal_fraction < 0.0;
    let reveal = reveal_fraction.abs() * base_rect.height();

    let texture = match preview {
        Some(SiblingPreviewState::Ready { texture, .. }) => Some(texture),
        _ => None,
    };

    let Some(texture) = texture else {
        let label = if at_top { "▲ Previous chapter" } else { "▼ Next chapter" };
        let alpha = (reveal_fraction.abs() * 255.0) as u8;
        let y = if at_top { base_rect.top() + reveal.min(24.0) } else { base_rect.bottom() - reveal.min(24.0) };
        ui.painter().text(
            Pos2::new(base_rect.center().x, y),
            Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(14.0),
            Color32::from_white_alpha(alpha),
        );
        return;
    };

    let size = texture.size_vec2();
    let natural_height = displayed_width * (size.y / size.x.max(1.0));
    let top = if at_top { base_rect.top() + reveal - natural_height } else { base_rect.bottom() - reveal };
    let rect = Rect::from_min_size(Pos2::new(x_min, top), Vec2::new(displayed_width, natural_height));
    egui::Image::new(texture).paint_at(ui, rect);
}

/// Where a Webtoon page's compressed bytes come from, and the shared caches
/// its resulting slice textures and aspect ratio are read from / written
/// into — the Webtoon-mode counterpart of `DecodeCtx`, kept separate
/// because a page here can be more than one texture (see
/// `ComicApp::webtoon_textures`).
struct WebtoonSliceCtx<'a> {
    pages: &'a [Vec<u8>],
    textures: &'a mut HashMap<usize, Vec<TextureHandle>>,
    aspect: &'a mut HashMap<usize, f32>,
    max_dimension: Option<u32>,
}

/// Paints a Webtoon page's slices stacked top-to-bottom inside `column`,
/// together filling it exactly the way a single `draw_page_slot` texture
/// would — each slice sized to whatever fraction of the page's total pixel
/// height it actually covers (read off the textures' own sizes, so this
/// stays correct however many slices `decode_page_slices` produced, without
/// needing to track slice heights separately). Draws nothing for a page
/// whose slices haven't arrived yet — `request_webtoon_prefetch`'s worker
/// generally has them ready well before they'd scroll into view (see
/// `WEBTOON_PRELOAD_MARGIN`), so this is the rare case, not decoding
/// synchronously on the UI thread the way `draw_page_slot` does for the
/// paginated view — a page-sized decode here is a much larger stall to risk
/// on the UI thread than a normal page's.
fn draw_webtoon_page_slot(ui: &mut Ui, column: Rect, page_idx: usize, ctx: &mut WebtoonSliceCtx) {
    if !ctx.textures.contains_key(&page_idx) {
        let Some(data) = ctx.pages.get(page_idx) else { return };
        let Ok((slices, aspect)) = ComicArchive::decode_page_slices(data, ctx.max_dimension) else {
            return;
        };
        let textures: Vec<TextureHandle> = slices
            .into_iter()
            .enumerate()
            .map(|(slice_idx, image)| {
                ui.ctx().load_texture(format!("webtoon_{page_idx}_{slice_idx}"), image, TextureOptions::LINEAR)
            })
            .collect();
        ctx.aspect.entry(page_idx).or_insert(aspect);
        ctx.textures.insert(page_idx, textures);
    }

    let Some(textures) = ctx.textures.get(&page_idx) else { return };
    let total_height_px: f32 = textures.iter().map(|t| t.size_vec2().y).sum::<f32>().max(1.0);

    let aspect = ctx.aspect.get(&page_idx).copied().unwrap_or(WEBTOON_DEFAULT_ASPECT);
    let slice_pixel_heights: Vec<f32> = textures.iter().map(|t| t.size_vec2().y).collect();
    let slice_heights = webtoon_slice_display_heights(column.width(), aspect, &slice_pixel_heights, total_height_px);

    let mut y = column.top();
    for (texture, slice_height) in textures.iter().zip(slice_heights) {
        let rect = Rect::from_min_size(Pos2::new(column.left(), y), Vec2::new(column.width(), slice_height));
        egui::Image::new(texture).paint_at(ui, rect);
        y += slice_height;
    }
}

/// Each slice's on-screen height, given the *page's* own aspect ratio and
/// `column_width` — not `column.height()` directly. The caller
/// (`draw_webtoon`) already fixed `column`'s height before a page's texture
/// (and so its real aspect) was necessarily known, from whatever
/// `ComicApp::webtoon_aspect` held at the top of the frame — still
/// `WEBTOON_DEFAULT_ASPECT`'s guess for a page decoded for the first time
/// just now. Stretching each slice to fill `column.height()` in that case
/// would visibly distort it — e.g. a normal page's worth of height forced
/// onto a webtoon chapter's 20+:1 strip. Deriving height from `aspect` and
/// `column_width` instead means a page always renders undistorted; if that
/// doesn't match `column.height()` because of a stale guess, it briefly
/// over/undershoots its reserved slot by a few pixels instead of
/// stretching — self-corrects the very next frame once `draw_webtoon`
/// recomputes offsets from `webtoon_aspect`, which now has this page's real
/// value.
fn webtoon_slice_display_heights(column_width: f32, aspect: f32, slice_pixel_heights: &[f32], total_pixel_height: f32) -> Vec<f32> {
    let natural_height = column_width / aspect.max(0.001);
    slice_pixel_heights.iter().map(|&h| natural_height * (h / total_pixel_height.max(1.0))).collect()
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
            let image = match ComicArchive::decode_page_image(data, ctx.max_dimension) {
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
    fn webtoon_slice_display_heights_uses_the_pages_own_aspect_not_column_height() {
        // Reproduces the stretch bug: a page freshly decoded to an extreme
        // aspect ratio (a tall webtoon strip, 0.05) must render at its own
        // proportions from `column_width` alone — the height this returns
        // should have no relationship to whatever `column.height()`
        // happened to be (here, a stand-in for a stale
        // `WEBTOON_DEFAULT_ASPECT`-guessed column, deliberately very
        // different from what aspect 0.05 implies).
        let column_width = 400.0;
        let aspect = 0.05; // width/height — a narrow, very tall strip
        let heights = webtoon_slice_display_heights(column_width, aspect, &[8000.0], 8000.0);

        assert_eq!(heights.len(), 1);
        // Undistorted: height = width / aspect, regardless of any
        // `column.height()` a caller might have passed in from a stale
        // guess.
        assert!((heights[0] - column_width / aspect).abs() < EPSILON);
    }

    #[test]
    fn webtoon_slice_display_heights_splits_proportionally_to_each_slices_own_pixels() {
        let column_width = 300.0;
        let aspect = 0.5;
        // Three slices: 5000, 3000, 2000 native px tall (10000 total) — the
        // on-screen heights should split the page's total natural height in
        // those same proportions (50%, 30%, 20%).
        let heights = webtoon_slice_display_heights(column_width, aspect, &[5000.0, 3000.0, 2000.0], 10000.0);

        let natural_total = column_width / aspect;
        assert_eq!(heights.len(), 3);
        assert!((heights[0] - natural_total * 0.5).abs() < EPSILON);
        assert!((heights[1] - natural_total * 0.3).abs() < EPSILON);
        assert!((heights[2] - natural_total * 0.2).abs() < EPSILON);
        let sum: f32 = heights.iter().sum();
        assert!((sum - natural_total).abs() < EPSILON);
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

    #[test]
    fn webtoon_page_height_uses_the_known_aspect_ratio_once_decoded() {
        let mut aspect_map = HashMap::new();
        // Square: height in doc units equals the doc width.
        aspect_map.insert(0, 1.0);
        assert!((webtoon_page_height(0, &aspect_map) - WEBTOON_DOC_WIDTH).abs() < EPSILON);
    }

    #[test]
    fn webtoon_page_height_falls_back_to_the_default_guess_when_not_yet_decoded() {
        let aspect_map = HashMap::new();
        let expected = WEBTOON_DOC_WIDTH / WEBTOON_DEFAULT_ASPECT;
        assert!((webtoon_page_height(0, &aspect_map) - expected).abs() < EPSILON);
    }

    #[test]
    fn webtoon_page_height_agrees_with_slice_display_height_for_an_extreme_real_world_aspect() {
        // Regression test for a real bug: a merged-chapter webtoon page can
        // legitimately have an aspect ratio well under the `0.05` this
        // function used to floor at (e.g. a real 1654x37045 page measures
        // ~0.04465) — `draw_webtoon` reserved column space from *this*
        // function's (clamped, too-short) height while `draw_webtoon_page_slot`
        // rendered the page at its real (unclamped, taller) height, so the
        // page's own later content overflowed past its reserved slot and
        // drew over whatever came after it — visible as missing/misplaced
        // content. The two must agree, in document units converted to the
        // same units `webtoon_slice_display_heights` returns (multiply by
        // `column_width / WEBTOON_DOC_WIDTH` to convert one to the other).
        let real_aspect = 1654.0 / 37045.0;
        let mut aspect_map = HashMap::new();
        aspect_map.insert(0, real_aspect);

        let column_width = 304.0; // an arbitrary, real displayed_width
        let layout_height_px = webtoon_page_height(0, &aspect_map) * (column_width / WEBTOON_DOC_WIDTH);

        // A single "slice" spanning the whole page — same math
        // `draw_webtoon_page_slot` uses for a page's actual rendered height.
        let rendered_height_px = webtoon_slice_display_heights(column_width, real_aspect, &[37045.0], 37045.0)[0];

        assert!(
            (layout_height_px - rendered_height_px).abs() < 1.0,
            "layout reserved {layout_height_px}px but rendering draws {rendered_height_px}px — a page's own content would overflow its slot"
        );
    }

    #[test]
    fn webtoon_offsets_stacks_pages_directly_on_top_of_each_other() {
        let mut aspect_map = HashMap::new();
        aspect_map.insert(0, 1.0); // height == WEBTOON_DOC_WIDTH
        aspect_map.insert(1, 2.0); // height == WEBTOON_DOC_WIDTH / 2

        let (offsets, total) = webtoon_offsets(3, &aspect_map, 0.0);

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
    fn webtoon_offsets_inserts_the_gap_between_pages_but_not_after_the_last_one() {
        let mut aspect_map = HashMap::new();
        aspect_map.insert(0, 1.0); // height == WEBTOON_DOC_WIDTH
        aspect_map.insert(1, 1.0); // height == WEBTOON_DOC_WIDTH

        let (offsets, total) = webtoon_offsets(2, &aspect_map, 40.0);

        assert!((offsets[0] - 0.0).abs() < EPSILON);
        assert!((offsets[1] - (WEBTOON_DOC_WIDTH + 40.0)).abs() < EPSILON);
        // No trailing gap after the last page.
        assert!((total - (2.0 * WEBTOON_DOC_WIDTH + 40.0)).abs() < EPSILON);
    }

    #[test]
    fn webtoon_offsets_shift_when_a_page_above_finishes_decoding() {
        // Reproduces the reflow `draw_webtoon`'s anchor correction guards
        // against: page 0 is still using the default-aspect guess when the
        // reader is sitting on page 1, then page 0 finishes decoding to a
        // taller-than-guessed real aspect. Page 1's offset — and thus where
        // its top actually is — moves even though nothing scrolled.
        let mut aspect_map = HashMap::new();
        // Page 1 already decoded; page 0 hasn't yet, so it uses the guess.
        aspect_map.insert(1, 1.0);
        let (offsets_before, _) = webtoon_offsets(2, &aspect_map, 0.0);
        let anchor_offset = offsets_before[1];

        // Page 0 finishes decoding: much taller than the default guess.
        aspect_map.insert(0, 0.1);
        let (offsets_after, _) = webtoon_offsets(2, &aspect_map, 0.0);

        assert!(
            (offsets_after[1] - anchor_offset).abs() > EPSILON,
            "the scenario should actually reflow, or this test proves nothing"
        );
        // The correction `draw_webtoon` applies is exactly this delta added
        // to `webtoon_scroll`, which keeps page 1's top at the same screen
        // position it was at before page 0 resolved its real height.
        let correction = offsets_after[1] - anchor_offset;
        let scroll_before = anchor_offset; // reader sitting right at page 1's top
        let scroll_after = scroll_before + correction;
        assert!((scroll_after - offsets_after[1]).abs() < EPSILON);
    }

    #[test]
    fn webtoon_offsets_of_an_empty_book_is_empty_with_zero_height() {
        let (offsets, total) = webtoon_offsets(0, &HashMap::new(), 0.0);
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
        let mut aspect_map = HashMap::new();
        for i in 0..5 {
            aspect_map.insert(i, 1.0);
        }
        let (offsets, _) = webtoon_offsets(5, &aspect_map, 0.0);

        let scroll = 2000.0;
        let (range, current) = webtoon_visible_range(&offsets, &aspect_map, scroll, scroll - 900.0, scroll + 900.0);

        assert_eq!(range, Some((1, 2)));
        assert_eq!(current, 2); // page 2 spans [2000, 3000), its bottom is the first past `scroll`
    }

    #[test]
    fn webtoon_visible_range_current_page_falls_back_to_the_last_page_at_the_very_end() {
        let mut aspect_map = HashMap::new();
        for i in 0..3 {
            aspect_map.insert(i, 1.0);
        }
        let (offsets, total) = webtoon_offsets(3, &aspect_map, 0.0);

        // Scrolled exactly to the bottom: no page's bottom edge is strictly
        // past `scroll` anymore, so `current` must still land on a valid
        // page (the last one) rather than an out-of-range fallback.
        let (_, current) = webtoon_visible_range(&offsets, &aspect_map, total, total - 100.0, total + 100.0);
        assert_eq!(current, 2);
    }

    #[test]
    fn webtoon_visible_range_of_an_empty_book_keeps_nothing() {
        let (range, current) = webtoon_visible_range(&[], &HashMap::new(), 0.0, 0.0, 0.0);
        assert_eq!(range, None);
        assert_eq!(current, 0);
    }

    /// Shorthand for a live (`dragging: true`) drag at some `progress`, `0.0`
    /// velocity unless a test overrides it after construction.
    fn live_drag(direction: i32, progress: f32) -> WebtoonEdgeDrag {
        WebtoonEdgeDrag { direction, progress, velocity: 0.0, dragging: true }
    }

    #[test]
    fn webtoon_edge_drag_step_scrolls_normally_away_from_either_edge() {
        // Mid-strip: a downward push just scrolls, no drag starts, even on
        // a fresh gesture.
        let (scroll, drag, commit) = webtoon_edge_drag_step(500.0, 1000.0, 50.0, 5.0, 1.0 / 60.0, true, false, None);
        assert!((scroll - 505.0).abs() < EPSILON);
        assert!(drag.is_none());
        assert_eq!(commit, None);
    }

    #[test]
    fn webtoon_edge_drag_step_never_starts_from_a_gestures_own_momentum_tail() {
        // Already at max_scroll and still being pushed down — but
        // `gesture_started` is false, so this is the tail end of whatever
        // gesture arrived here, not a fresh push. No drag should start;
        // `scroll` just stays clamped at the edge for the caller to clamp.
        let (_, drag, commit) = webtoon_edge_drag_step(1000.0, 1000.0, 50.0, 5.0, 1.0 / 60.0, false, false, None);
        assert!(drag.is_none());
        assert_eq!(commit, None);
    }

    #[test]
    fn webtoon_edge_drag_step_starts_only_on_a_fresh_gesture_already_at_the_bottom() {
        let (scroll, drag, commit) = webtoon_edge_drag_step(1000.0, 1000.0, 50.0, 5.0, 1.0 / 60.0, true, false, None);
        assert!((scroll - 1000.0).abs() < EPSILON); // doesn't move past the edge
        let drag = drag.expect("a fresh push at the edge should start a drag");
        assert_eq!(drag.direction, 1);
        assert!(drag.dragging);
        assert_eq!(commit, None);
    }

    #[test]
    fn webtoon_edge_drag_step_starts_symmetrically_at_the_top() {
        let (scroll, drag, _) = webtoon_edge_drag_step(0.0, 1000.0, -50.0, -5.0, 1.0 / 60.0, true, false, None);
        assert!((scroll - 0.0).abs() < EPSILON);
        assert_eq!(drag.expect("should start").direction, -1);
    }

    #[test]
    fn webtoon_edge_drag_step_progress_matches_full_distance() {
        let drag = live_drag(1, 0.0);
        let push = WEBTOON_EDGE_DRAG_FULL_DISTANCE / 2.0;
        let (_, drag, _) = webtoon_edge_drag_step(1000.0, 1000.0, push, push * 0.1, 1.0 / 60.0, false, false, Some(drag));
        assert!((drag.unwrap().progress - 0.5).abs() < EPSILON);
    }

    #[test]
    fn webtoon_edge_drag_step_holds_progress_steady_while_held_still() {
        // The whole point of the redesign: a live drag with zero delta this
        // frame (fingers down but not moving) must not lose any progress —
        // unlike the old accumulator, there's no per-frame decay while
        // `dragging` is still true.
        let drag = live_drag(1, 0.4);
        let (_, drag, _) = webtoon_edge_drag_step(1000.0, 1000.0, 0.0, 0.0, 1.0, false, false, Some(drag));
        assert!((drag.unwrap().progress - 0.4).abs() < EPSILON);
    }

    #[test]
    fn webtoon_edge_drag_step_holding_still_right_after_starting_does_not_cancel_it() {
        // A drag that just started sits at `progress: 0.0` — a zero-delta
        // hold on the very next frame must still count as "holding", not
        // "already pulled back past zero" (an easy off-by-one to get wrong
        // since both look like `progress <= 0.0` from the outside).
        let drag = live_drag(1, 0.0);
        let (_, drag, commit) = webtoon_edge_drag_step(1000.0, 1000.0, 0.0, 0.0, 1.0 / 60.0, false, false, Some(drag));
        assert_eq!(commit, None);
        let drag = drag.expect("holding still at progress 0.0 must not drop the drag");
        assert!(drag.dragging);
        assert!((drag.progress - 0.0).abs() < EPSILON);
    }

    #[test]
    fn webtoon_edge_drag_step_pulling_back_reduces_progress() {
        let drag = live_drag(1, 0.5);
        let pull = WEBTOON_EDGE_DRAG_FULL_DISTANCE * 0.2;
        let (_, drag, _) = webtoon_edge_drag_step(1000.0, 1000.0, -pull, -pull * 0.1, 1.0 / 60.0, false, false, Some(drag));
        assert!((drag.unwrap().progress - 0.3).abs() < EPSILON);
    }

    #[test]
    fn webtoon_edge_drag_step_pulling_all_the_way_back_drops_the_drag_and_resumes_scrolling() {
        let drag = live_drag(1, 0.1);
        let pull = WEBTOON_EDGE_DRAG_FULL_DISTANCE * 0.2; // more than enough to zero out 0.1 progress
        let (scroll, drag, commit) = webtoon_edge_drag_step(1000.0, 1000.0, -pull, -3.0, 1.0 / 60.0, false, false, Some(drag));
        assert!(drag.is_none());
        assert_eq!(commit, None);
        assert!((scroll - 997.0).abs() < EPSILON); // normal scroll applied instead
    }

    #[test]
    fn webtoon_edge_drag_step_progress_never_exceeds_one() {
        let drag = live_drag(1, 0.9);
        let push = WEBTOON_EDGE_DRAG_FULL_DISTANCE; // would overshoot to 1.9 uncapped
        let (_, drag, _) = webtoon_edge_drag_step(1000.0, 1000.0, push, push * 0.1, 1.0 / 60.0, false, false, Some(drag));
        assert!((drag.unwrap().progress - 1.0).abs() < EPSILON);
    }

    #[test]
    fn webtoon_edge_drag_step_commits_at_release_past_the_threshold() {
        let drag = live_drag(1, WEBTOON_EDGE_DRAG_COMMIT_THRESHOLD + 0.01);
        let (_, drag, commit) = webtoon_edge_drag_step(1000.0, 1000.0, 0.0, 0.0, 1.0 / 60.0, false, true, Some(drag));
        assert!(drag.is_none());
        assert_eq!(commit, Some(1));
    }

    #[test]
    fn webtoon_edge_drag_step_releasing_below_threshold_settles_back_instead_of_committing() {
        let drag = live_drag(1, WEBTOON_EDGE_DRAG_COMMIT_THRESHOLD - 0.01);
        let (_, drag, commit) = webtoon_edge_drag_step(1000.0, 1000.0, 0.0, 0.0, 1.0 / 60.0, false, true, Some(drag));
        assert_eq!(commit, None);
        let drag = drag.expect("should still exist, settling back to rest");
        assert!(!drag.dragging);
    }

    #[test]
    fn webtoon_edge_drag_step_a_fast_flick_commits_even_with_little_progress() {
        // Checked for both directions — `velocity` is already sign-
        // normalized to progress-space ("positive = further past this
        // edge") regardless of which edge, so a naive re-multiply by
        // `direction` at the commit check would silently flip this for the
        // top edge (`-1`) alone while `direction: 1` still passed.
        for direction in [1, -1] {
            let mut drag = live_drag(direction, 0.1);
            drag.velocity = WEBTOON_EDGE_DRAG_FLING_VELOCITY + 1.0;
            let (_, _, commit) = webtoon_edge_drag_step(1000.0, 1000.0, 0.0, 0.0, 1.0 / 60.0, false, true, Some(drag));
            assert_eq!(commit, Some(direction));
        }
    }

    #[test]
    fn webtoon_edge_drag_step_a_fast_flick_the_wrong_way_does_not_commit() {
        // Fast velocity in itself isn't enough — it has to point the same
        // way as the drag (a sharp pull back out counts as a cancel, not a
        // flung commit). Checked for both directions, see the sibling test.
        for direction in [1, -1] {
            let mut drag = live_drag(direction, 0.1);
            drag.velocity = -(WEBTOON_EDGE_DRAG_FLING_VELOCITY + 1.0);
            let (_, drag, commit) = webtoon_edge_drag_step(1000.0, 1000.0, 0.0, 0.0, 1.0 / 60.0, false, true, Some(drag));
            assert_eq!(commit, None);
            assert!(!drag.unwrap().dragging);
        }
    }

    #[test]
    fn webtoon_edge_drag_step_settling_drag_decays_and_then_clears() {
        let drag = WebtoonEdgeDrag { direction: 1, progress: 0.5, velocity: 0.0, dragging: false };
        let dt = 0.5 / WEBTOON_EDGE_DRAG_RELEASE_DECAY_PER_SECOND; // exactly enough to drain 0.5
        let (scroll, drag, commit) = webtoon_edge_drag_step(1000.0, 1000.0, 0.0, 0.0, dt, false, false, Some(drag));
        assert!(drag.is_none());
        assert_eq!(commit, None);
        assert!((scroll - 1000.0).abs() < EPSILON);
    }

    #[test]
    fn webtoon_edge_drag_step_settling_drag_ignores_further_input() {
        // Once released without committing, new wheel input (e.g. the same
        // momentum tail continuing) shouldn't revive the drag or move
        // `scroll` — it just keeps decaying toward `0.0`.
        let drag = WebtoonEdgeDrag { direction: 1, progress: 0.5, velocity: 0.0, dragging: false };
        let (scroll, drag, commit) = webtoon_edge_drag_step(1000.0, 1000.0, 50.0, 5.0, 1.0 / 60.0, false, false, Some(drag));
        assert!((scroll - 1000.0).abs() < EPSILON);
        assert!(drag.is_some());
        assert_eq!(commit, None);
    }
}
