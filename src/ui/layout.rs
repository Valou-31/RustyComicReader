use serde::{Deserialize, Serialize};
use std::time::Duration;

/// One customizable control in the header's toolbar — see
/// `LayoutConfig::toolbar_items`. `Separator` is itself an item, not a fixed
/// part of the layout, so it can be added, removed and moved exactly like
/// any other control — that's how a user divides their own toolbar into
/// groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolbarItem {
    LoadFile,
    NextInQueue,
    ReadingMode,
    Bookmark,
    ResetZoom,
    LockZoom,
    Settings,
    History,
    Bookmarks,
    UpdateStatus,
    BlueLightFilter,
    Separator,
}

impl ToolbarItem {
    pub const ALL: [ToolbarItem; 12] = [
        ToolbarItem::LoadFile,
        ToolbarItem::NextInQueue,
        ToolbarItem::ReadingMode,
        ToolbarItem::Bookmark,
        ToolbarItem::ResetZoom,
        ToolbarItem::LockZoom,
        ToolbarItem::Settings,
        ToolbarItem::History,
        ToolbarItem::Bookmarks,
        ToolbarItem::UpdateStatus,
        ToolbarItem::BlueLightFilter,
        ToolbarItem::Separator,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ToolbarItem::LoadFile => "Load File",
            ToolbarItem::NextInQueue => "Next in Queue",
            ToolbarItem::ReadingMode => "Reading Mode",
            ToolbarItem::Bookmark => "Bookmark Toggle",
            ToolbarItem::ResetZoom => "Reset Zoom",
            ToolbarItem::LockZoom => "Lock Zoom",
            ToolbarItem::Settings => "Settings",
            ToolbarItem::History => "History",
            ToolbarItem::Bookmarks => "Bookmarks",
            ToolbarItem::UpdateStatus => "Update Status",
            ToolbarItem::BlueLightFilter => "Blue Light Filter",
            ToolbarItem::Separator => "-- Separator --",
        }
    }
}

/// Which floating bar a toolbar item lives in — the header pinned to the
/// top of the window, or the footer pinned to the bottom. The footer only
/// renders at all once it holds at least one item (or while editing).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolbarArea {
    Header,
    Footer,
}

/// Horizontal placement of an item within its row — three independently
/// packed zones per row, laid out like a typical app toolbar (left cluster,
/// centered cluster, right-anchored cluster).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolbarAlign {
    Left,
    Center,
    Right,
}

/// One control's placement in the toolbar: which item, which bar, which
/// row within that bar (0-based, stacked top-to-bottom), and which zone of
/// that row. Order among items sharing the same (area, row, align) is the
/// order they appear in `LayoutConfig::toolbar_items` — drag-and-drop
/// editing (`ui::toolbar`) reorders by moving entries within that vec.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacedToolbarItem {
    pub item: ToolbarItem,
    pub area: ToolbarArea,
    pub row: usize,
    pub align: ToolbarAlign,
}

impl PlacedToolbarItem {
    fn header_left(item: ToolbarItem, row: usize) -> Self {
        Self { item, area: ToolbarArea::Header, row, align: ToolbarAlign::Left }
    }
}

/// Reads `toolbar_items` accepting either the current shape (a
/// `PlacedToolbarItem` object per entry) or the older one written by
/// pre-multi-row builds (a bare `ToolbarItem` string per entry, e.g.
/// `"Separator"`) — every legacy entry lands in the header's row 0, left
/// zone, in its original order, which is exactly the layout that flat list
/// used to mean. Without this, an existing config.json with the old shape
/// would fail to parse as a whole and silently reset *every* setting
/// (theme, keybindings, zoom lock, ...) back to default, not just the
/// toolbar.
fn deserialize_toolbar_items<'de, D>(deserializer: D) -> Result<Vec<PlacedToolbarItem>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Entry {
        Placed(PlacedToolbarItem),
        Legacy(ToolbarItem),
    }

    let entries = Vec::<Entry>::deserialize(deserializer)?;
    Ok(entries
        .into_iter()
        .map(|entry| match entry {
            Entry::Placed(placed) => placed,
            Entry::Legacy(item) => PlacedToolbarItem::header_left(item, 0),
        })
        .collect())
}

/// User-tunable layout knobs for the reader (README: "Configurable Layout —
/// separator width/opacity, spacing, transition speed").
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
    /// Width in points of the spine line painted between the two pages.
    /// `0.0` hides it entirely.
    pub spine_width: f32,
    /// Opacity of the spine line, `0.0`..=`1.0` — also the peak opacity of
    /// the spine shadow gradient, at its center.
    pub spine_opacity: f32,
    /// How far the spine's shadow gradient fades out on each side, in
    /// points — mimics the way a physical book's pages curve away into
    /// shadow near the binding. `0.0` disables it, leaving just the line.
    pub spine_shadow_width: f32,
    /// Color of the spine line and shadow, independent of the active theme.
    pub spine_color: [u8; 3],
    /// The last few distinct colors picked for `spine_color`, most recent
    /// first, for quick reuse — see `record_spine_color`.
    pub spine_color_history: Vec<[u8; 3]>,
    /// Gap in points between the two pages. `0.0` keeps them touching
    /// (the "sticked together" book-spread look).
    pub page_gap: f32,
    /// Duration in milliseconds of the header show/hide fade.
    pub fade_duration_ms: u64,
    /// Baseline speed, in milliseconds, of a page-turn settle spring
    /// (`ComicApp::step_transition`) — lower is faster. Since it's a spring
    /// rather than a fixed-length animation, this scales stiffness and
    /// damping together rather than literally bounding how long it runs.
    pub page_transition_ms: u64,
    /// Every control placed in the header or footer, and where — see
    /// `PlacedToolbarItem`. Edited via live drag-and-drop (`ui::toolbar`,
    /// entered from Settings); `ui::header`/`ui::footer` just render
    /// whatever's placed in their area.
    #[serde(deserialize_with = "deserialize_toolbar_items")]
    pub toolbar_items: Vec<PlacedToolbarItem>,
    /// If true, the header floats over the page like the footer already
    /// does, instead of reserving its own space above it — the page fills
    /// the full window and the header draws on top when shown, rather than
    /// the page shrinking to make room for it. See `ui::header::draw_header_overlay`.
    pub header_floats_over_reader: bool,
    /// Opacity, `0.0`..=`1.0`, of the header/footer's background panel when
    /// fully shown (before the idle-hide fade — see `fade_duration_ms` —
    /// scales it down further toward `0.0`). Unlike button opacity, this
    /// has no floor: `0.0` means no panel at all, just the buttons floating
    /// directly over the page.
    pub menu_bg_opacity: f32,
    /// Opacity, `0.0`..=`1.0`, of the header/footer's buttons/text when
    /// fully shown (same idle-hide interaction as `menu_bg_opacity`). Reads
    /// through `menu_button_opacity()` rather than directly, which enforces
    /// the floor below.
    pub menu_button_opacity: f32,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            spine_width: 1.0,
            spine_opacity: 0.24,
            spine_shadow_width: 28.0,
            spine_color: [20, 20, 20],
            spine_color_history: Vec::new(),
            page_gap: 0.0,
            fade_duration_ms: 300,
            page_transition_ms: 220,
            toolbar_items: Self::default_toolbar_items(),
            header_floats_over_reader: false,
            menu_bg_opacity: 0.9,
            menu_button_opacity: 1.0,
        }
    }
}

/// A hand-picked floor, not a technical minimum — below it the buttons
/// become hard to make out against the comic page behind them, especially
/// once the header can float over it (`header_floats_over_reader`).
pub const MIN_MENU_BUTTON_OPACITY: f32 = 0.2;

impl LayoutConfig {
    pub fn fade_duration(&self) -> Duration {
        Duration::from_millis(self.fade_duration_ms)
    }

    /// `menu_button_opacity`, clamped to `MIN_MENU_BUTTON_OPACITY` — always
    /// read through this rather than the field directly, since a hand-edited
    /// `config.json` could otherwise set it lower.
    pub fn menu_button_opacity(&self) -> f32 {
        self.menu_button_opacity.clamp(MIN_MENU_BUTTON_OPACITY, 1.0)
    }

    /// Fill color for the header/footer's background panel, given its theme
    /// base color and the current idle-hide fade (`0.0`..=`1.0`, from
    /// `Context::animate_bool_with_time` against `fade_duration`). Blends
    /// `menu_bg_opacity` into the alpha channel *only* — `Color32::gamma_multiply`
    /// (the previous approach, at a fixed near-1.0 factor) scales RGB too, which
    /// stayed unnoticeable right up near full opacity but would visibly darken
    /// the panel rather than just showing more of the page through it once a
    /// user can dial background opacity down toward `0.0`.
    pub fn menu_bg_fill(&self, base: egui::Color32, fade_alpha: f32) -> egui::Color32 {
        let a = (self.menu_bg_opacity.clamp(0.0, 1.0) * fade_alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        egui::Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), a)
    }

    /// Opacity to apply to the header/footer's buttons/text for the current
    /// idle-hide fade — `menu_button_opacity()` (already floored) scaled by
    /// the fade, so it still reaches `0.0` once fully hidden; the floor only
    /// bounds how low the *shown* state can go, not the hide animation.
    pub fn menu_content_opacity(&self, fade_alpha: f32) -> f32 {
        self.menu_button_opacity() * fade_alpha.clamp(0.0, 1.0)
    }

    /// The out-of-the-box toolbar layout — file/settings/history on the
    /// left of the header, reading mode centered, blue light filter and
    /// update status on the right; zoom lock/reset and both bookmark
    /// controls moved down to the footer. "Reset toolbar to default" in
    /// Settings restores exactly this. See `PlacedToolbarItem`.
    pub fn default_toolbar_items() -> Vec<PlacedToolbarItem> {
        use ToolbarAlign::{Center, Left, Right};
        use ToolbarArea::{Footer, Header};
        [
            (ToolbarItem::LoadFile, Header, 0, Left),
            (ToolbarItem::NextInQueue, Header, 0, Left),
            (ToolbarItem::Settings, Header, 0, Left),
            (ToolbarItem::Separator, Header, 0, Left),
            (ToolbarItem::History, Header, 0, Left),
            (ToolbarItem::ReadingMode, Header, 0, Center),
            (ToolbarItem::BlueLightFilter, Header, 0, Right),
            (ToolbarItem::UpdateStatus, Header, 0, Right),
            (ToolbarItem::LockZoom, Footer, 0, Left),
            (ToolbarItem::ResetZoom, Footer, 0, Left),
            (ToolbarItem::Bookmark, Footer, 0, Right),
            (ToolbarItem::Bookmarks, Footer, 0, Right),
        ]
        .into_iter()
        .map(|(item, area, row, align)| PlacedToolbarItem { item, area, row, align })
        .collect()
    }

    /// One past the highest occupied row index in `area` — i.e. how many
    /// rows of `area` currently hold at least one item. `0` means empty.
    pub fn toolbar_row_count(&self, area: ToolbarArea) -> usize {
        self.toolbar_items
            .iter()
            .filter(|placed| placed.area == area)
            .map(|placed| placed.row + 1)
            .max()
            .unwrap_or(0)
    }

    /// Whether `item` already appears anywhere in the toolbar — separators
    /// are exempt since any number of them make sense.
    pub fn is_toolbar_item_placed(&self, item: ToolbarItem) -> bool {
        item != ToolbarItem::Separator && self.toolbar_items.iter().any(|placed| placed.item == item)
    }

    /// Records `color` as the most recently used spine color, moving it to
    /// the front if it's already in the history, capped at 5 entries.
    pub fn record_spine_color(&mut self, color: [u8; 3]) {
        self.spine_color_history.retain(|&c| c != color);
        self.spine_color_history.insert(0, color);
        self.spine_color_history.truncate(5);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pre-multi-row `config.json`'s `layout.toolbar_items` — a bare
    /// list of `ToolbarItem` strings, no area/row/align. Loading a config
    /// this old must not error out the whole `Config` deserialize (which
    /// would silently reset every other setting to default too — see
    /// `deserialize_toolbar_items`).
    #[test]
    fn legacy_flat_toolbar_items_migrate_to_header_row_zero_left() {
        let json = r#"{
            "spine_width": 0.0, "spine_opacity": 0.58, "spine_shadow_width": 47.0,
            "spine_color": [0, 0, 0], "spine_color_history": [],
            "page_gap": 49.0, "fade_duration_ms": 300, "page_transition_ms": 600,
            "toolbar_items": ["Separator", "ResetZoom", "LockZoom", "Bookmark"]
        }"#;

        let layout: LayoutConfig = serde_json::from_str(json).expect("legacy shape should still parse");

        assert_eq!(
            layout.toolbar_items,
            vec![
                PlacedToolbarItem::header_left(ToolbarItem::Separator, 0),
                PlacedToolbarItem::header_left(ToolbarItem::ResetZoom, 0),
                PlacedToolbarItem::header_left(ToolbarItem::LockZoom, 0),
                PlacedToolbarItem::header_left(ToolbarItem::Bookmark, 0),
            ]
        );
    }

    #[test]
    fn current_shape_toolbar_items_round_trip() {
        let original = vec![
            PlacedToolbarItem { item: ToolbarItem::Settings, area: ToolbarArea::Footer, row: 1, align: ToolbarAlign::Right },
        ];
        let json = serde_json::to_string(&original).unwrap();
        let parsed: Vec<PlacedToolbarItem> =
            serde_json::from_str::<LayoutConfig>(&format!(
                r#"{{"toolbar_items": {json}}}"#
            ))
            .map(|l| l.toolbar_items)
            .unwrap();
        assert_eq!(parsed, original);
    }

    /// A hand-edited `config.json` (the settings slider itself can't go
    /// below `MIN_MENU_BUTTON_OPACITY`, but nothing stops someone editing
    /// the file directly) must not be able to make the buttons any harder
    /// to see than the floor allows.
    #[test]
    fn menu_button_opacity_is_floored_even_if_the_stored_value_is_lower() {
        let mut layout = LayoutConfig { menu_button_opacity: 0.0, ..LayoutConfig::default() };
        assert_eq!(layout.menu_button_opacity(), MIN_MENU_BUTTON_OPACITY);

        layout.menu_button_opacity = 0.5;
        assert_eq!(layout.menu_button_opacity(), 0.5);
    }

    /// `menu_bg_fill` must only ever touch the alpha channel — darkening
    /// the RGB itself (what `Color32::gamma_multiply` would do) means a low
    /// background-opacity setting visibly tints the panel instead of just
    /// letting more of the page show through it. `Color32` stores
    /// premultiplied-alpha internally, so `.r()/.g()/.b()` on a translucent
    /// color naturally scale with alpha (that's expected, not a bug) —
    /// `to_srgba_unmultiplied()` is what actually recovers the original,
    /// alpha-independent channel values to compare against.
    #[test]
    fn menu_bg_fill_only_changes_alpha_never_the_underlying_color() {
        let base = egui::Color32::from_rgb(200, 100, 50);
        let mut layout = LayoutConfig { menu_bg_opacity: 0.5, ..LayoutConfig::default() };

        let fully_faded_in = layout.menu_bg_fill(base, 1.0);
        let [r, g, b, a] = fully_faded_in.to_srgba_unmultiplied();
        // +/-1: the premultiply/unpremultiply round trip through 8-bit
        // storage is inherently lossy (e.g. 200 * 127/255, rounded down,
        // then scaled back up, lands on 199) — that's a property of
        // `Color32` itself, not something this function can avoid.
        for (actual, expected) in [(r, 200), (g, 100), (b, 50)] {
            assert!(actual.abs_diff(expected) <= 1, "{actual} not within 1 of {expected}");
        }
        assert_eq!(a, 128); // 0.5 * 255, rounded

        // The idle-hide fade multiplies in on top of the configured opacity.
        let half_faded = layout.menu_bg_fill(base, 0.5);
        assert_eq!(half_faded.to_srgba_unmultiplied()[3], 64); // 0.5 * 0.5 * 255, rounded

        layout.menu_bg_opacity = 0.0;
        assert_eq!(layout.menu_bg_fill(base, 1.0).a(), 0);
    }
}
