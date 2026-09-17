//! Shared rendering for one toolbar row (three zones — left, centered,
//! right-anchored) plus the live drag-and-drop editing built on top of it.
//! Used by both `ui::header` (the `ToolbarArea::Header` bar) and
//! `ui::footer` (the `ToolbarArea::Footer` bar) so the two bars behave
//! identically.
use crate::app::ComicApp;
use crate::ui::layout::{PlacedToolbarItem, ToolbarAlign, ToolbarArea, ToolbarItem};
use egui::{Align, Frame, Id, Layout, Ui};

/// What's carried while dragging a toolbar chip: the item type, plus where
/// it came from — `Some(index)` into `LayoutConfig::toolbar_items` for a
/// chip being moved from its current spot, `None` for a fresh instance
/// dragged in from the "Add" palette (`draw_palette`).
#[derive(Clone, Copy, Debug)]
struct DragPayload {
    item: ToolbarItem,
    source_index: Option<usize>,
}

/// A toolbar edit (drag-and-drop move, or a removal) discovered while
/// rendering a row and applied later, via `ComicApp::apply_pending_toolbar_edit`
/// — never mutate `app.layout.toolbar_items` directly mid-render. The header
/// and footer both render in the *same* frame, each drawing chips whose
/// `egui::Id` (`Id::new("toolbar_chip").with(idx)`) is just their index into
/// that shared vec. Mutating it synchronously mid-frame (as this used to do)
/// shifts every later index: a chip that already rendered once this frame
/// under id N, in the header's normal (background) layer, could render
/// *again* under that same id N later in the same frame in the footer's
/// floating (foreground) `Area` — same `Id`, two different layers, one
/// frame. egui's own consistency check catches exactly that mismatch and
/// panics (confirmed via the crash log this was added to diagnose:
/// `DEBUG ASSERT: Widget Id::new("toolbar_chip").with(11) changed layer_id
/// during the frame`). Queuing the edit and applying it once, before any
/// toolbar row renders for the frame, avoids the shift entirely.
pub enum PendingToolbarEdit {
    /// Move (or newly place, if `source_index` is `None`) `item` into
    /// `(area, row, align)`, immediately before whatever currently sits at
    /// `before_local_index` within that zone (`usize::MAX` appends at the
    /// end).
    Move { item: ToolbarItem, source_index: Option<usize>, area: ToolbarArea, row: usize, align: ToolbarAlign, before_local_index: usize },
    /// Remove the item currently at this index into `toolbar_items`.
    Remove { index: usize },
}

impl PendingToolbarEdit {
    /// Applies this edit to `app.layout.toolbar_items` and persists it.
    /// Must only be called before any toolbar row renders for the current
    /// frame — see the type-level docs.
    pub fn apply(self, app: &mut ComicApp) {
        app.layout.toolbar_items = self.resolve(&app.layout.toolbar_items);
        app.save_config();
    }

    /// The pure half of `apply` — computes the new `toolbar_items` list
    /// without touching `ComicApp` at all, so it can be exercised without
    /// the accompanying disk write (`apply` always calls `save_config`,
    /// which would overwrite the real `config.json` if invoked from a
    /// test). Split out purely for testability.
    fn resolve(self, current: &[PlacedToolbarItem]) -> Vec<PlacedToolbarItem> {
        let mut items = current.to_vec();
        match self {
            PendingToolbarEdit::Move { item, source_index, area, row, align, before_local_index } => {
                // If the drag started from an existing placement, that
                // entry is removed first — dropped a couple of positions
                // off from where it visually looked like it would land is
                // possible when the source and target are the same zone,
                // since removal shifts everything after it down by one;
                // good enough for a live drag-to-arrange control, not worth
                // chasing further.
                if let Some(src) = source_index
                    && src < items.len()
                {
                    items.remove(src);
                }

                let zone_indices: Vec<usize> = items
                    .iter()
                    .enumerate()
                    .filter(|(_, placed)| placed.area == area && placed.row == row && placed.align == align)
                    .map(|(i, _)| i)
                    .collect();
                let insert_at = if before_local_index >= zone_indices.len() {
                    zone_indices.last().map_or(items.len(), |&i| i + 1)
                } else {
                    zone_indices[before_local_index]
                };

                items.insert(insert_at, PlacedToolbarItem { item, area, row, align });
            }
            PendingToolbarEdit::Remove { index } => {
                if index < items.len() {
                    items.remove(index);
                }
            }
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placed(item: ToolbarItem, area: ToolbarArea, row: usize, align: ToolbarAlign) -> PlacedToolbarItem {
        PlacedToolbarItem { item, area, row, align }
    }

    /// The exact scenario from the crash report: `UpdateStatus` lives in a
    /// different row/zone than the chip it gets dropped onto
    /// (`BlueLightFilter`, in `Header`/row 0/Right). The crash itself was
    /// about *when* the mutation happens relative to rendering (see the
    /// `PendingToolbarEdit` docs) — this confirms the deferred logic still
    /// lands `UpdateStatus` in the right spot rather than, say, silently
    /// dropping it or duplicating `BlueLightFilter`.
    #[test]
    fn moving_an_item_into_a_different_zone_relocates_it_without_disturbing_the_target() {
        let current = vec![
            placed(ToolbarItem::BlueLightFilter, ToolbarArea::Header, 0, ToolbarAlign::Right),
            placed(ToolbarItem::UpdateStatus, ToolbarArea::Header, 1, ToolbarAlign::Center),
        ];
        let edit = PendingToolbarEdit::Move {
            item: ToolbarItem::UpdateStatus,
            source_index: Some(1),
            area: ToolbarArea::Header,
            row: 0,
            align: ToolbarAlign::Right,
            before_local_index: 0,
        };

        let result = edit.resolve(&current);

        assert_eq!(
            result,
            vec![
                placed(ToolbarItem::UpdateStatus, ToolbarArea::Header, 0, ToolbarAlign::Right),
                placed(ToolbarItem::BlueLightFilter, ToolbarArea::Header, 0, ToolbarAlign::Right),
            ]
        );
    }

    #[test]
    fn removing_by_index_drops_exactly_that_item() {
        let current = vec![
            placed(ToolbarItem::LoadFile, ToolbarArea::Header, 0, ToolbarAlign::Left),
            placed(ToolbarItem::BlueLightFilter, ToolbarArea::Header, 0, ToolbarAlign::Right),
        ];
        let result = PendingToolbarEdit::Remove { index: 0 }.resolve(&current);
        assert_eq!(result, vec![placed(ToolbarItem::BlueLightFilter, ToolbarArea::Header, 0, ToolbarAlign::Right)]);
    }

    #[test]
    fn an_out_of_range_source_or_removal_index_is_a_no_op_not_a_panic() {
        let current = vec![placed(ToolbarItem::LoadFile, ToolbarArea::Header, 0, ToolbarAlign::Left)];

        let moved = PendingToolbarEdit::Move {
            item: ToolbarItem::Settings,
            source_index: Some(99),
            area: ToolbarArea::Header,
            row: 0,
            align: ToolbarAlign::Left,
            before_local_index: usize::MAX,
        }
        .resolve(&current);
        assert_eq!(moved.len(), 2);

        let removed = PendingToolbarEdit::Remove { index: 99 }.resolve(&current);
        assert_eq!(removed, current);
    }
}

/// Renders one full toolbar row of `area` in its normal, compact,
/// non-editing form — one line, each zone dispatching its items through
/// `draw_item` (the caller's normal button/slider rendering). Never called
/// while `app.toolbar_edit_mode` is on — editing replaces the header,
/// reader and footer entirely with `draw_toolbar_editor`'s full-screen,
/// one-block-per-zone layout instead of trying to cram draggable chips into
/// this same compact line (that's what used to live here, and it's also
/// what made this function need the fragile width-splitting/estimation
/// dance below in the first place — see git history if resurrecting any of
/// that logic).
///
/// The three zones size to their own content rather than splitting the row
/// into equal thirds — the out-of-the-box toolbar lives entirely in the
/// Left zone (see `LayoutConfig::default_toolbar_items`), so an even split
/// would crush it into a third of the window's width while Center and Right
/// sat empty. Left renders first and takes whatever it needs; Right renders
/// last and gets whatever's genuinely left after Left, right-aligned within
/// it, so it always hugs the row's true right edge exactly. Center is
/// positioned independently of both, at the row's true horizontal midpoint
/// — via an explicit rect (`egui::UiBuilder::max_rect`) rather than
/// sequential layout, so it's centered on the row as a whole rather than in
/// whatever gap happens to be left between two *different-width* siblings.
/// Egui can't retroactively center arbitrary multi-widget content within a
/// single frame (it doesn't know the content's width until after painting
/// it), so the box's left edge is computed from *last frame's* measured
/// content width — one frame of lag, imperceptible in practice. Critically,
/// that estimate only sets the box's left edge, never its width: the box
/// always spans from there to the row's true right edge, generously, so
/// content is never capped to a too-small estimate and re-measured at that
/// same wrong size forever (an earlier version used the estimate for both,
/// and got stuck permanently clipping a button's text mid-word — capping
/// the render width and measuring the result from that same capped width is
/// a self-reinforcing loop with no way to correct itself). Since Center
/// doesn't participate in the row's left-to-right flow at all, it can never
/// starve Right of space either (worst case on a narrow window: Center
/// visually overlaps Left or Right, exactly like a real centered toolbar
/// title would). Each zone wraps its own content (`draw_zone`) instead of
/// letting it overflow past its boundary, so a crowded zone grows taller
/// rather than spilling into its neighbor or off the window.
pub fn draw_toolbar_row(
    ui: &mut Ui,
    app: &mut ComicApp,
    area: ToolbarArea,
    row: usize,
    mut draw_item: impl FnMut(&mut Ui, &mut ComicApp, ToolbarItem),
) {
    let full_row_rect = ui.available_rect_before_wrap();

    ui.horizontal(|ui| {
        // Left renders via `ui.scope` rather than an explicit width
        // allocation: `allocate_ui_with_layout`'s desired size is claimed
        // from the row in full — the outer cursor advances by what was
        // *granted*, not by what Left actually used — so giving Left "the
        // whole row" (needed so its own wrapping/scrolling has the right
        // boundary) silently left nothing for Right regardless of how
        // little of the row Left's real content occupied. `ui.scope`
        // advances the cursor by Left's *actual* rendered width instead.
        let left_spec = ZoneSpec { area, row, align: ToolbarAlign::Left, cross_align: Align::LEFT, label: "Left" };
        ui.scope(|ui| draw_zone(ui, app, left_spec, &mut draw_item));

        let center_spec = ZoneSpec { area, row, align: ToolbarAlign::Center, cross_align: Align::LEFT, label: "Center" };
        if !zone_items(app, area, row, ToolbarAlign::Center).is_empty() {
            let widths_id = Id::new("toolbar_row_center_width").with((area, row));
            let prev_center_w = ui.data(|d| d.get_temp::<f32>(widths_id)).unwrap_or(0.0);
            // Left edge only, from last frame's measured width — deliberately
            // *not* also used as the box's width. Capping the box at that
            // estimate too (as an earlier version of this did) meant a
            // still-too-small first estimate could never grow: the box
            // bounded the content, the content's rendered width was what got
            // re-measured, and a too-small estimate just reproduced itself
            // forever (confirmed live — a button's label was permanently cut
            // off mid-word). Spanning the box all the way to the row's right
            // edge instead means content always renders at its true natural
            // width regardless of how wrong the estimate is, so the
            // measurement — and therefore the centering — actually converges.
            let center_x = full_row_rect.left() + ((full_row_rect.width() - prev_center_w) / 2.0).max(0.0);
            // Height matches a single row of controls (egui's own standard
            // interactive-widget height), NOT `full_row_rect.height()` —
            // that rect is captured from `available_rect_before_wrap()`
            // before this row has been placed, so its height spans all the
            // way to the bottom of whatever contains the header (in practice
            // most of the window). Handing the *whole* remaining window
            // height to this box, even though nothing was ever going to
            // paint into most of it, made the header's own reported size
            // balloon to match it — visually, the header appeared to occupy
            // nearly the entire window, squeezing the actual page into a
            // sliver at the bottom. A row here is always one line of
            // buttons/sliders (this zone never wraps vertically — its
            // `ScrollArea` is horizontal-only), so one control's height is
            // exactly enough, with nothing left over to inflate the row.
            let center_rect = egui::Rect::from_min_size(
                egui::pos2(center_x, full_row_rect.top()),
                egui::vec2((full_row_rect.right() - center_x).max(0.0), ui.spacing().interact_size.y),
            );
            let center_resp =
                ui.scope_builder(egui::UiBuilder::new().max_rect(center_rect), |ui| draw_zone(ui, app, center_spec, &mut draw_item));
            ui.data_mut(|d| d.insert_temp(widths_id, center_resp.response.rect.width()));
        }

        // `.max(0.0)`: a very narrow window (or a single item too wide to
        // wrap on its own — `horizontal_wrapped` only wraps *between*
        // items) can leave nothing after Left; feeding a negative size into
        // `allocate_ui_with_layout` is exactly the kind of malformed `Rect`
        // egui warns against. Right is unaffected by Center's width, since
        // Center no longer participates in this row's sequential flow.
        let right_w_available = ui.available_width().max(0.0);
        ui.allocate_ui_with_layout(egui::vec2(right_w_available, ui.available_height()), Layout::top_down(Align::RIGHT), |ui| {
            let right_spec = ZoneSpec { area, row, align: ToolbarAlign::Right, cross_align: Align::RIGHT, label: "Right" };
            draw_zone(ui, app, right_spec, &mut draw_item);
        });
    });
}

fn zone_items(app: &ComicApp, area: ToolbarArea, row: usize, align: ToolbarAlign) -> Vec<(usize, ToolbarItem)> {
    app.layout
        .toolbar_items
        .iter()
        .enumerate()
        .filter(|(_, placed)| placed.area == area && placed.row == row && placed.align == align)
        .map(|(i, placed)| (i, placed.item))
        .collect()
}

/// One (area, row, align) target, plus how to lay it out and what to call
/// it — bundled so `draw_zone`/`draw_chip` take one argument for "where"
/// instead of four.
#[derive(Clone, Copy)]
struct ZoneSpec {
    area: ToolbarArea,
    row: usize,
    align: ToolbarAlign,
    cross_align: Align,
    label: &'static str,
}

/// A horizontally-scrollable, auto-hiding-scrollbar area for one zone's
/// items — used instead of `horizontal_wrapped` (which, nested this deep
/// inside `draw_toolbar_row`'s per-zone `allocate_ui_with_layout` calls,
/// wasn't reliably wrapping onto a second line: content just silently
/// extended past the window's edge instead of becoming visible some other
/// way). A scrollbar that only appears when a zone is genuinely overloaded
/// beats content disappearing off-window with no indication it's there.
fn zone_scroll_area(spec: ZoneSpec) -> egui::ScrollArea {
    egui::ScrollArea::horizontal()
        .id_salt((spec.area, spec.row, spec.align))
        // `auto_shrink([true, true])`, NOT `[false, ...]`: `false` on the
        // width axis means "never shrink narrower than what's available" —
        // every populated zone would then claim its *entire* granted width
        // regardless of how little content it actually held, leaving
        // nothing for whichever zone renders after it in the same row
        // (confirmed by instrumenting the actual widths at runtime: Left
        // alone was consuming 100% of the row even though its content
        // visibly used barely half of it). `true` lets the area shrink down
        // to its real content width, only capping at the granted width when
        // content genuinely overflows it.
        .auto_shrink([true, true])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
}

fn draw_zone(ui: &mut Ui, app: &mut ComicApp, spec: ZoneSpec, draw_item: &mut impl FnMut(&mut Ui, &mut ComicApp, ToolbarItem)) {
    ui.with_layout(Layout::top_down(spec.cross_align), |ui| {
        let items = zone_items(app, spec.area, spec.row, spec.align);
        if items.is_empty() {
            return;
        }
        zone_scroll_area(spec).show(ui, |ui| {
            ui.horizontal(|ui| {
                for (_, item) in items {
                    draw_item(ui, app, item);
                }
            });
        });
    });
}

/// One draggable chip. Also doubles as its own drop target: releasing a
/// drag over an *existing* chip inserts the dragged item immediately
/// before it, which is what gives edit mode precise reordering — dropping
/// on empty zone background (handled by `draw_zone`) just appends instead.
struct ChipTarget {
    spec: ZoneSpec,
    local_pos: usize,
}

fn draw_chip(ui: &mut Ui, app: &mut ComicApp, idx: usize, item: ToolbarItem, target: ChipTarget) {
    // The drop-target frame stays invisible at rest — `dnd_drop_zone`
    // paints it with the theme's active/inactive widget colors only while
    // something's actively being dragged over it. The chip's own button
    // chrome (same style already used by the "Drag to add" palette) is
    // what makes it visible the rest of the time.
    let chip_frame = Frame::default().inner_margin(2.0);
    let mut remove_clicked = false;
    let (_, dropped) = ui.dnd_drop_zone::<DragPayload, _>(chip_frame, |ui| {
        ui.horizontal(|ui| {
            let drag_id = Id::new("toolbar_chip").with(idx);
            ui.dnd_drag_source(drag_id, DragPayload { item, source_index: Some(idx) }, |ui| {
                ui.add(egui::Button::new(item.label()));
            });
            // A precise drag-to-trash (`draw_trash_zone`) is the only way to
            // remove a chip otherwise — fiddly for undoing a mistake, so
            // every chip also gets a one-click way to remove just itself.
            if ui.small_button("✕").on_hover_text("Remove from toolbar").clicked() {
                remove_clicked = true;
            }
        });
    });
    if remove_clicked {
        app.toolbar_pending_edit = Some(PendingToolbarEdit::Remove { index: idx });
        return;
    }
    if let Some(payload) = dropped {
        let spec = target.spec;
        app.toolbar_pending_edit = Some(PendingToolbarEdit::Move {
            item: payload.item,
            source_index: payload.source_index,
            area: spec.area,
            row: spec.row,
            align: spec.align,
            before_local_index: target.local_pos,
        });
    }
}

/// A drop target that deletes whatever's dropped on it. Only meaningful
/// for a chip already placed somewhere (`source_index: Some`); dropping a
/// fresh palette chip here is just a no-op cancel, not an error.
pub fn draw_trash_zone(ui: &mut Ui, app: &mut ComicApp) {
    let frame = Frame::default()
        .inner_margin(6.0)
        .corner_radius(4.0)
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(200, 90, 90)));
    let (_, dropped) = ui.dnd_drop_zone::<DragPayload, _>(frame, |ui| {
        ui.colored_label(egui::Color32::from_rgb(200, 90, 90), "Drop here to remove");
    });
    if let Some(payload) = dropped
        && let Some(src) = payload.source_index
    {
        app.toolbar_pending_edit = Some(PendingToolbarEdit::Remove { index: src });
    }
}

/// Draggable "add new" chips for every item not already placed anywhere in
/// the toolbar — separators are exempt since any number of them make
/// sense. Drag one onto a zone to place a fresh instance there.
pub fn draw_palette(ui: &mut Ui, app: &mut ComicApp) {
    ui.horizontal_wrapped(|ui| {
        for candidate in ToolbarItem::ALL {
            if app.layout.is_toolbar_item_placed(candidate) {
                continue;
            }
            let id = Id::new("toolbar_palette").with(candidate);
            ui.dnd_drag_source(id, DragPayload { item: candidate, source_index: None }, |ui| {
                ui.add(egui::Button::new(candidate.label()));
            });
        }
    });
}

/// The full-screen toolbar editor — the *entire* content of the window
/// while `app.toolbar_edit_mode` is on, replacing the header, reader and
/// footer outright (see `main.rs`) rather than squeezing into the header's
/// normal compact single-line space above the comic page. Editing isn't
/// reading, so there's no reason to preserve room for the page while doing
/// it — and giving the editor the whole window is what actually lets
/// Left/Center/Right become distinct, readable blocks (`draw_zone_block`)
/// instead of three cramped columns fighting over one line's worth of
/// width, which is what `draw_toolbar_row` needs for the compact look
/// everywhere else.
pub fn draw_toolbar_editor(ui: &mut Ui, app: &mut ComicApp) {
    ui.horizontal(|ui| {
        ui.heading("Editing Toolbar Layout");
        if ui.button("Done").clicked() {
            app.toolbar_edit_mode = false;
        }
    });
    ui.label("Drag icons to rearrange them within or between blocks, or click ✕ to remove one.");
    ui.separator();

    // Palette and trash render *first*, before the scrollable row list, so
    // they always get the room they need. A `ScrollArea` with its height
    // axis exempted from `auto_shrink` greedily claims the *entire*
    // remaining space regardless of how little its own content needs —
    // rendering it before these would starve them of room entirely (the
    // exact same "first thing renders, eats everything" trap the
    // Left/Center/Right width split hit in `draw_toolbar_row`, just on the
    // vertical axis this time). This is also the direct fix for not being
    // able to find a removed item to put back: it's now the first thing on
    // screen, not something that had to be scrolled to (or, before this,
    // was never actually reachable at all).
    ui.label("Not currently in the toolbar — drag one in to add it:");
    draw_palette(ui, app);
    ui.add_space(4.0);
    draw_trash_zone(ui, app);
    ui.separator();

    egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
        draw_area_section(ui, app, ToolbarArea::Header, "Header — top bar");
        ui.add_space(16.0);
        draw_area_section(ui, app, ToolbarArea::Footer, "Footer — bottom bar");
    });
}

/// One toolbar area's (`Header` or `Footer`) rows, each as three stacked
/// Left/Center/Right blocks — see `draw_zone_block`. Always shows one row
/// past whatever's already occupied, empty, so there's somewhere to drag a
/// chip to start a new row.
fn draw_area_section(ui: &mut Ui, app: &mut ComicApp, area: ToolbarArea, title: &str) {
    ui.strong(title);
    let row_count = (app.layout.toolbar_row_count(area) + 1).max(1);
    for row in 0..row_count {
        if row_count > 1 {
            ui.small(format!("Row {}", row + 1));
        }
        for (align, label) in
            [(ToolbarAlign::Left, "Left"), (ToolbarAlign::Center, "Center"), (ToolbarAlign::Right, "Right")]
        {
            let spec = ZoneSpec { area, row, align, cross_align: Align::LEFT, label };
            draw_zone_block(ui, app, spec);
        }
        ui.add_space(8.0);
    }
}

/// One (area, row, align) zone as its own full-width, labeled, bordered
/// block — the editor's replacement for squeezing Left/Center/Right onto
/// one shared line (`draw_toolbar_row`/`draw_zone`, used everywhere
/// *outside* editing). Because each block gets the entire row's width to
/// itself instead of splitting it three ways, its items can just
/// `horizontal_wrapped` — no need for `draw_toolbar_row`'s width
/// estimation/negotiation between zones, and nothing to silently starve.
fn draw_zone_block(ui: &mut Ui, app: &mut ComicApp, spec: ZoneSpec) {
    let items = zone_items(app, spec.area, spec.row, spec.align);
    let theme = app.theme_preset.theme();
    let frame = Frame::default()
        .fill(theme.panel.gamma_multiply(0.5))
        .stroke(egui::Stroke::new(1.0, theme.text_secondary.gamma_multiply(0.5)))
        .inner_margin(egui::Margin::same(6))
        .corner_radius(4.0);

    ui.set_width(ui.available_width());
    let (_, dropped) = ui.dnd_drop_zone::<DragPayload, _>(frame, |ui| {
        ui.vertical(|ui| {
            ui.colored_label(theme.text_secondary, spec.label);
            if items.is_empty() {
                ui.weak("— drop items here —");
            } else {
                ui.horizontal_wrapped(|ui| {
                    for (local_pos, (idx, item)) in items.into_iter().enumerate() {
                        draw_chip(ui, app, idx, item, ChipTarget { spec, local_pos });
                    }
                });
            }
        });
    });
    if let Some(payload) = dropped {
        app.toolbar_pending_edit = Some(PendingToolbarEdit::Move {
            item: payload.item,
            source_index: payload.source_index,
            area: spec.area,
            row: spec.row,
            align: spec.align,
            before_local_index: usize::MAX,
        });
    }
    ui.add_space(4.0);
}
