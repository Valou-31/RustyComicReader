use crate::comic::prefetch::{DecodeQueue, DecodeRequest};
use crate::input::keybindings::{Action, KeyBindings};
use crate::storage::config::Config;
use crate::storage::history::History;
use crate::ui::layout::LayoutConfig;
use crate::ui::theme::ThemePreset;
use crate::update::{ApplyEvent, CheckEvent, UpdateInfo};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long the header stays visible after the last mouse movement.
pub const UI_HIDE_DELAY: Duration = Duration::from_secs(3);

/// Minimum time between debounced history saves triggered by page
/// navigation — an exit or a freshly-loaded book flushes immediately
/// regardless of this.
pub const HISTORY_SAVE_DEBOUNCE: Duration = Duration::from_secs(5);

/// When downscaling is enabled, no decoded page dimension exceeds this —
/// comic scans are routinely 3000-6000px on a side, far beyond what any
/// display can show, so this trades invisible sharpness for a large RAM/VRAM
/// saving per page.
pub const DOWNSCALE_MAX_DIMENSION: u32 = 2400;

/// A page-turn slide in progress, either a live trackpad drag (`dragging:
/// true`, `progress` set directly by `drag_page_by` each frame) or a
/// physical damped spring pulling `progress` toward `target`, stepped once
/// per frame by `ComicApp::step_transition` — for a keyboard-triggered turn
/// (`target` is `1.0` from the start) or a drag that just committed/cancelled
/// at gesture end (`target` flips to `1.0`/`0.0`, carrying over the drag's
/// last velocity so the settle continues the gesture's motion instead of
/// starting from rest). Cleared automatically once the spring comes to
/// rest — there's no fixed duration to wait out.
///
/// `old_left`/`old_right` are the spread being left; `new_left`/`new_right`
/// the spread being entered — captured once up front since, during a live
/// drag, `current_page`/`page_offset` haven't moved yet, so `left_page`/
/// `right_page` alone can't tell you what's being dragged towards.
/// `entry_sign` says which side the new spread slides in from (`+1.0`
/// right, `-1.0` left); `forward` says whether committing means
/// `next_spread` (`true`) or `prev_spread` (`false`).
pub struct PageTransition {
    pub entry_sign: f32,
    pub forward: bool,
    pub old_left: Option<usize>,
    pub old_right: Option<usize>,
    pub new_left: Option<usize>,
    pub new_right: Option<usize>,
    pub progress: f32,
    pub velocity: f32,
    pub target: f32,
    pub dragging: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadingMode {
    LTR, // Left-to-Right
    RTL, // Right-to-Left
    /// One page on screen at a time — `pages_for_position` never pairs a
    /// second page alongside it, so every turn moves by exactly one page
    /// instead of two.
    Single,
}

impl ReadingMode {
    pub fn label(&self) -> &'static str {
        match self {
            ReadingMode::LTR => "➡ LTR (Western)",
            ReadingMode::RTL => "⬅ RTL (Manga)",
            ReadingMode::Single => "📄 Single Page",
        }
    }
}

/// Where the background update check/download currently stands — drives
/// both the header's small update button and the Settings "Updates" panel.
#[derive(Clone, Debug, Default)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    Available(UpdateInfo),
    Downloading,
    /// Downloaded and swapped into place — takes effect on next launch.
    Ready,
    Failed(String),
}

pub struct ComicApp {
    /// Each page's original compressed bytes (JPEG/PNG/etc.), not decoded
    /// pixels — decoding happens on demand in the reader UI, only for pages
    /// near the current spread, so memory doesn't scale with book length.
    pub pages: Vec<Vec<u8>>,
    /// Page index of the leading page of the currently displayed spread —
    /// its own second page (if any) follows from `pages_for_position`, which
    /// accounts for double-page spreads and reading direction. Not simply
    /// "spread number × 2": a double-page spread occupies one index by
    /// itself, so spreads before it can shift this off the even/odd pattern.
    pub current_page: usize,
    pub total_pages: usize,
    pub page_offset: usize,
    pub reading_mode: ReadingMode,
    pub filename: String,
    pub show_settings: bool,
    pub last_mouse_move: std::time::Instant,
    pub fullscreen: bool,
    pub fullscreen_dirty: bool,
    pub keybindings: KeyBindings,
    pub theme_preset: ThemePreset,
    pub layout: LayoutConfig,
    /// Warm-tint overlay strength, `0.0` (off) to `1.0` (strongest). Exposed
    /// as a slider on the auto-hiding header rather than in Settings, since
    /// it's meant to be nudged in the moment while reading.
    pub blue_light_filter: f32,
    /// Flips the direction two-finger trackpad scroll maps to next/prev —
    /// the base mapping already depends on `reading_mode`, this is purely a
    /// per-user trackpad preference (like macOS's own "natural scrolling").
    pub scroll_inverted: bool,
    /// Multiplier on how much a two-finger trackpad swipe counts toward a
    /// full page turn — `1.0` is the default feel, higher means less
    /// physical movement is needed for the same amount of turn.
    pub scroll_sensitivity: f32,
    /// When on, a trackpad swipe that has already committed or bounced back
    /// won't start another page-turn until a fresh gesture begins (see
    /// `swipe_locked`) — caps one big or fast swipe, momentum tail included,
    /// to a single page turn instead of chaining through several.
    pub one_turn_per_swipe: bool,
    /// Set by `end_page_drag` right after a swipe resolves, while
    /// `one_turn_per_swipe` is on; cleared by `input::scroll` the moment it
    /// sees a fresh `TouchPhase::Start`. While `true`, `input::scroll`
    /// ignores incoming wheel deltas outright — this is what stops a
    /// strong swipe's momentum (which keeps sending `Move`-phase deltas
    /// with no new `Start` in between) from being mistaken for a deliberate
    /// second swipe. Not persisted: purely a within-session, frame-to-frame
    /// signal, reset fresh on every launch.
    pub swipe_locked: bool,
    /// Whether `ui::progress_bar` decodes and shows page previews at all.
    /// On by default; with it off, cost is zero — `warm_thumbnail_cache`
    /// never runs and `thumbnail_textures`/`thumbnail_hires_textures` stay
    /// empty — and toggling it off mid-session (`set_show_page_preview`)
    /// unloads whatever was already cached. The progress bar itself (fill,
    /// hover marker, click-to-jump) is unaffected either way.
    pub show_page_preview: bool,
    /// The action currently waiting for its next key press to be bound to it.
    pub remapping_action: Option<Action>,
    pub textures: HashMap<usize, egui::TextureHandle>,
    /// Each decoded page's edge colors and double-page-spread flag,
    /// alongside its texture in `textures` — see `comic::archive::PageMeta`.
    pub page_meta: HashMap<usize, crate::comic::archive::PageMeta>,
    /// Cheap low-res previews for `ui::progress_bar`, one per page for the
    /// whole book — see `warm_thumbnail_cache`. Small enough to keep for the
    /// whole session (cleared only when a different book opens) without it
    /// costing much, and exists so there's always *something* instant to
    /// show while the sharp version in `thumbnail_hires_textures` is still
    /// decoding. Kept separate from `textures` since scrubbing can touch
    /// pages far outside the current spread's prefetch/eviction window.
    pub thumbnail_textures: HashMap<usize, egui::TextureHandle>,
    /// Sharp on-demand previews for whichever page(s) the progress bar was
    /// actually hovered over recently — see `request_thumbnail`. Far more
    /// expensive per page than `thumbnail_textures`, so this is capped
    /// (`THUMBNAIL_HIRES_CACHE_CAP`) rather than kept for the whole book.
    pub thumbnail_hires_textures: HashMap<usize, egui::TextureHandle>,
    pub loading: bool,
    pub load_error: Option<String>,
    /// Full path of the archive currently loaded, if any — `None` on the
    /// empty-state screen. Used to key reading-history updates.
    pub current_path: Option<PathBuf>,
    /// Files picked (or passed on the command line) alongside the one
    /// currently loading/loaded, waiting their turn.
    pub file_queue: VecDeque<PathBuf>,
    pub history: History,
    pub show_history: bool,
    /// Set true by navigation, cleared once `flush_history` runs.
    history_dirty: bool,
    history_last_saved: std::time::Instant,
    /// Whether to reopen the last-read book (at its last page) on startup.
    /// Persisted; toggled from the settings panel.
    pub resume_last_session: bool,
    /// Page to jump to once the in-flight load finishes — set when that
    /// load is a session resume or a history reopen, `None` for a fresh
    /// file that should just start at page 0.
    resume_to_page: Option<usize>,
    pending_pick: Option<std::sync::mpsc::Receiver<crate::comic::loader::PickEvent>>,
    /// (pages decoded so far, total pages in the archive), while `loading` is true.
    pub load_progress: (usize, usize),
    /// The first decoded page, handed off once so the UI can upload it to a
    /// texture; `None` once picked up or when there is nothing new to show.
    pub load_preview: Option<egui::ColorImage>,
    pub preview_texture: Option<egui::TextureHandle>,
    pub window_title: String,
    pub title_dirty: bool,
    /// Lazily uploaded the first time the empty-state screen is drawn.
    pub logo_texture: Option<egui::TextureHandle>,
    pending_load: Option<std::sync::mpsc::Receiver<crate::comic::loader::LoadEvent>>,
    /// Downscale decoded pages to `DOWNSCALE_MAX_DIMENSION` to save RAM/VRAM,
    /// at some cost to sharpness on very high-res scans. User-toggleable.
    pub downscale_large_pages: bool,
    /// Identifies the currently loaded archive, bumped every time a new file
    /// finishes loading — lets `poll_decoded_pages` discard a background
    /// decode that was still in flight for the *previous* file.
    load_generation: u64,
    /// The background decode worker's input queue. `request_prefetch`
    /// reconciles it to exactly the pages currently wanted every frame, so a
    /// page the user has scrolled past gets dropped from it instead of
    /// piling up behind a worker that can't keep up.
    decode_queue: DecodeQueue,
    decode_result_rx: std::sync::mpsc::Receiver<crate::comic::prefetch::DecodedPage>,
    /// The background thumbnail worker's request queue and result channel —
    /// see `comic::thumbnail`. Decodes hover-preview thumbnails off the UI
    /// thread so showing one never blocks a frame.
    thumbnail_queue: crate::comic::thumbnail::ThumbnailQueue,
    thumbnail_result_rx: std::sync::mpsc::Receiver<crate::comic::thumbnail::DecodedThumbnail>,
    /// The page a hi-res thumbnail request is currently in flight for, if
    /// any — `request_thumbnail` uses this to avoid re-queuing the same
    /// page every single frame the cursor holds still over it.
    thumbnail_pending: Option<usize>,
    /// Set by `next_spread`/`prev_spread` or an in-progress trackpad drag,
    /// consumed and cleared once fully settled.
    pub page_transition: Option<PageTransition>,
    /// When the spine color picker was last changed — the settings panel
    /// waits for this to go quiet before recording the color into
    /// `LayoutConfig::spine_color_history`, so dragging around the picker
    /// doesn't flood the history with every intermediate shade.
    pub spine_color_pending_since: Option<Instant>,
    /// Whether to check GitHub for a newer release on startup. Persisted;
    /// toggled from the settings panel.
    pub auto_check_updates: bool,
    pub update_status: UpdateStatus,
    pending_update_check: Option<std::sync::mpsc::Receiver<CheckEvent>>,
    pending_update_apply: Option<std::sync::mpsc::Receiver<ApplyEvent>>,
}

impl Default for ComicApp {
    fn default() -> Self {
        let (decode_queue, decode_result_rx) = crate::comic::prefetch::spawn_decode_worker();
        let (thumbnail_queue, thumbnail_result_rx) = crate::comic::thumbnail::spawn_thumbnail_worker();
        Self {
            pages: Vec::new(),
            current_page: 0,
            total_pages: 0,
            page_offset: 0,
            reading_mode: ReadingMode::LTR,
            filename: "Aucun fichier".to_string(),
            show_settings: false,
            last_mouse_move: std::time::Instant::now(),
            fullscreen: false,
            fullscreen_dirty: false,
            keybindings: KeyBindings::default(),
            theme_preset: ThemePreset::default(),
            layout: LayoutConfig::default(),
            blue_light_filter: 0.0,
            scroll_inverted: false,
            scroll_sensitivity: 1.0,
            one_turn_per_swipe: true,
            swipe_locked: false,
            show_page_preview: true,
            remapping_action: None,
            textures: HashMap::new(),
            page_meta: HashMap::new(),
            thumbnail_textures: HashMap::new(),
            thumbnail_hires_textures: HashMap::new(),
            loading: false,
            load_error: None,
            current_path: None,
            file_queue: VecDeque::new(),
            history: History::default(),
            show_history: false,
            history_dirty: false,
            history_last_saved: std::time::Instant::now(),
            resume_last_session: true,
            resume_to_page: None,
            pending_pick: None,
            load_progress: (0, 0),
            load_preview: None,
            preview_texture: None,
            window_title: "Comic Reader".to_string(),
            title_dirty: false,
            logo_texture: None,
            pending_load: None,
            downscale_large_pages: true,
            load_generation: 0,
            decode_queue,
            decode_result_rx,
            thumbnail_queue,
            thumbnail_result_rx,
            thumbnail_pending: None,
            page_transition: None,
            spine_color_pending_since: None,
            auto_check_updates: true,
            update_status: UpdateStatus::default(),
            pending_update_check: None,
            pending_update_apply: None,
        }
    }
}

impl ComicApp {
    /// Loads whatever was last saved to disk (reading mode, keybindings,
    /// theme, layout, reading history) into `self`, falling back to
    /// whatever's already there if a file is missing or unreadable.
    fn load_persisted(&mut self) {
        if let Ok(config) = Config::load() {
            self.reading_mode = config.reading_mode;
            self.keybindings = config.keybindings;
            self.theme_preset = config.theme;
            self.layout = config.layout;
            self.blue_light_filter = config.blue_light_filter;
            self.scroll_inverted = config.scroll_inverted;
            self.scroll_sensitivity = config.scroll_sensitivity;
            self.one_turn_per_swipe = config.one_turn_per_swipe;
            self.show_page_preview = config.show_page_preview;
            self.downscale_large_pages = config.downscale_large_pages;
            self.resume_last_session = config.resume_last_session;
            self.auto_check_updates = config.auto_check_updates;
        }
        if let Ok(history) = History::load() {
            self.history = history;
        }
    }

    /// Like `default()`, but starts from whatever was last saved to disk,
    /// and — if `resume_last_session` is on and the file still exists —
    /// starts loading the most recently read book back to its last page.
    pub fn new() -> Self {
        let mut app = Self::default();
        app.load_persisted();
        if app.resume_last_session
            && let Some(entry) = app.history.entries.first()
            && entry.path.exists()
        {
            let path = entry.path.clone();
            app.resume_to_page = Some(entry.last_page);
            app.start_loading_path(path);
        }
        if app.auto_check_updates {
            app.check_for_updates();
        }
        app
    }

    /// Like `new()`, but opens the given files instead of resuming the last
    /// session — used when the app was launched with file arguments (double-
    /// click / "Open With" / a comic archive passed on the command line).
    /// The first file loads immediately; the rest join the queue.
    pub fn new_with_files(paths: Vec<PathBuf>) -> Self {
        let mut app = Self::default();
        app.load_persisted();
        let mut paths = paths.into_iter();
        if let Some(first) = paths.next() {
            app.file_queue.extend(paths);
            app.start_loading_path(first);
        }
        if app.auto_check_updates {
            app.check_for_updates();
        }
        app
    }

    /// Persists the current reading mode, keybindings, theme and layout.
    /// Failures are logged, not surfaced — losing a settings save shouldn't
    /// interrupt reading.
    pub fn save_config(&self) {
        let config = Config {
            reading_mode: self.reading_mode,
            keybindings: self.keybindings.clone(),
            theme: self.theme_preset,
            layout: self.layout.clone(),
            blue_light_filter: self.blue_light_filter,
            scroll_inverted: self.scroll_inverted,
            scroll_sensitivity: self.scroll_sensitivity,
            one_turn_per_swipe: self.one_turn_per_swipe,
            show_page_preview: self.show_page_preview,
            downscale_large_pages: self.downscale_large_pages,
            resume_last_session: self.resume_last_session,
            auto_check_updates: self.auto_check_updates,
        };
        if let Err(err) = config.save() {
            tracing::warn!("Failed to save config: {err}");
        }
    }

    /// Advances from wherever the peek offset currently has us looking (not
    /// from the un-offset spread boundary, so a peek — `shift_right`/
    /// `shift_left` — carries forward instead of being discarded) to the
    /// next spread — one page for a double-page spread, two for a normal
    /// pair. Plays a full slide animation immediately (unlike a trackpad
    /// drag, there's no gesture to follow first).
    pub fn next_spread(&mut self) {
        let old = (self.left_page(), self.right_page());
        if self.advance_forward() {
            self.begin_transition(self.forward_entry_sign(), true, old);
        }
    }

    pub fn prev_spread(&mut self) {
        let old = (self.left_page(), self.right_page());
        if self.advance_backward() {
            self.begin_transition(-self.forward_entry_sign(), false, old);
        }
    }

    /// Jumps straight to the spread starting at `page_idx` — used by
    /// `ui::progress_bar` when the progress bar is clicked. Clamped to a
    /// valid index; drops any in-flight page-turn transition and clears the
    /// peek offset, since an arbitrary jump has no adjacent spread to slide
    /// in from. No pairing realignment: `page_idx` becomes the new spread's
    /// own leading page, same as a history/session resume.
    pub fn jump_to_page(&mut self, page_idx: usize) {
        if self.total_pages == 0 {
            return;
        }
        self.current_page = page_idx.min(self.total_pages - 1);
        self.page_offset = 0;
        self.page_transition = None;
        self.mark_history_dirty();
    }

    /// Moves to the next spread without starting any transition — the
    /// mutation half of `next_spread`, reused by a trackpad drag committing
    /// at gesture end (which settles the spring from wherever the drag left
    /// off, rather than a fresh `0.0`).
    fn advance_forward(&mut self) -> bool {
        let position = self.effective_position();
        let next = position + self.spread_width(position);
        let can_advance = next < self.total_pages;
        if can_advance {
            self.current_page = next;
            self.page_offset = 0;
            self.mark_history_dirty();
        }
        can_advance
    }

    fn advance_backward(&mut self) -> bool {
        let Some(prev) = self.prev_spread_start(self.effective_position()) else { return false };
        self.current_page = prev;
        self.page_offset = 0;
        self.mark_history_dirty();
        true
    }

    /// Which side the *next* spread visually enters from: `+1.0` (right) in
    /// traditional (LTR) mode, `-1.0` (left) in manga (RTL) mode — matches
    /// the trackpad scroll direction convention in `input::scroll`. Single
    /// Page mode slides the same way as LTR. `prev_spread` uses the
    /// opposite sign.
    fn forward_entry_sign(&self) -> f32 {
        match self.reading_mode {
            ReadingMode::LTR | ReadingMode::Single => 1.0,
            ReadingMode::RTL => -1.0,
        }
    }

    /// What `left_page()`/`right_page()` would be after moving `forward`
    /// (`next_spread`) or backward (`prev_spread`), without mutating
    /// anything — `None` at a boundary where that move isn't possible.
    fn peek_adjacent_pages(&self, forward: bool) -> Option<(Option<usize>, Option<usize>)> {
        let position = self.effective_position();
        let target = if forward {
            let next = position + self.spread_width(position);
            (next < self.total_pages).then_some(next)
        } else {
            self.prev_spread_start(position)
        }?;
        Some(self.pages_for_position(target))
    }

    /// The page index `left_page()`/`right_page()` are currently based on:
    /// the current spread's leading page, shifted by any active peek.
    fn effective_position(&self) -> usize {
        self.current_page + self.page_offset
    }

    /// `left_page()`/`right_page()` (in that order) for the spread starting
    /// at `position`. In Single Page mode `position` is always shown alone.
    /// Otherwise, a double-page spread (its own image spanning a whole
    /// opening — see `PageMeta::is_spread`) occupies `position` alone, with
    /// `None` on the other side; so does a page whose would-be partner is
    /// one, since it can't be paired into a normal two-page spread either.
    /// Otherwise pairs `position` with `position + 1`, swapped for RTL.
    fn pages_for_position(&self, position: usize) -> (Option<usize>, Option<usize>) {
        if position >= self.total_pages {
            return (None, None);
        }
        if self.reading_mode == ReadingMode::Single {
            return (Some(position), None);
        }
        let partner = position + 1;
        if self.is_double_page(position) || partner >= self.total_pages || self.is_double_page(partner) {
            return (Some(position), None);
        }
        if self.reading_mode == ReadingMode::RTL {
            (Some(partner), Some(position))
        } else {
            (Some(position), Some(partner))
        }
    }

    /// How many page indices the spread starting at `position` occupies —
    /// `1` for a double-page spread (or the trailing single page of an
    /// odd-length book), `2` for a normal pair.
    fn spread_width(&self, position: usize) -> usize {
        if self.pages_for_position(position).1.is_some() { 2 } else { 1 }
    }

    /// The leading page index of the spread immediately before the one
    /// starting at `position`, or `None` if `position` is already the
    /// book's first page. In Single Page mode this is always `position - 1`.
    /// Otherwise it mirrors `pages_for_position`'s forward pairing rule
    /// applied backwards: a page pairs with the one before it only if
    /// neither is a double-page spread.
    fn prev_spread_start(&self, position: usize) -> Option<usize> {
        let prev = position.checked_sub(1)?;
        if self.reading_mode == ReadingMode::Single || prev == 0 || self.is_double_page(prev) {
            return Some(prev);
        }
        let prev2 = prev - 1;
        Some(if self.is_double_page(prev2) { prev } else { prev2 })
    }

    /// Whether `page_idx`'s image is a double-page spread — `false` until
    /// it's actually been decoded (see `comic::archive::PageMeta`), so a
    /// page approached from far enough away that prefetch hasn't reached it
    /// yet is briefly assumed normal and self-corrects the frame after its
    /// decode lands.
    fn is_double_page(&self, page_idx: usize) -> bool {
        self.page_meta.get(&page_idx).is_some_and(|m| m.is_spread)
    }

    fn begin_transition(&mut self, entry_sign: f32, forward: bool, old: (Option<usize>, Option<usize>)) {
        let new = (self.left_page(), self.right_page());
        self.page_transition = Some(PageTransition {
            entry_sign,
            forward,
            old_left: old.0,
            old_right: old.1,
            new_left: new.0,
            new_right: new.1,
            progress: 0.0,
            velocity: 0.0,
            target: 1.0,
            dragging: false,
        });
    }

    /// True while a trackpad drag is live (its outcome not decided yet) —
    /// as opposed to settled/settling toward one.
    pub fn is_dragging(&self) -> bool {
        matches!(&self.page_transition, Some(t) if t.dragging)
    }

    /// Whether an in-progress drag would commit to `next_spread` (`true`) or
    /// `prev_spread` (`false`) if released now, or `None` if there's no live
    /// drag.
    pub fn dragging_forward(&self) -> Option<bool> {
        self.page_transition.as_ref().filter(|t| t.dragging).map(|t| t.forward)
    }

    /// Starts a finger-driven drag toward the adjacent spread in `forward`'s
    /// direction.
    ///
    /// If a bounce-back is still mid-flight (a previous drag that fell short
    /// and is settling back to the current spread, `target == 0.0`) and
    /// happens to be heading the same direction, grabs it instead of waiting
    /// it out — resumes live control from wherever its progress/velocity
    /// currently are, so retrying a swipe that didn't quite make it
    /// interrupts the bounce-back rather than queuing up behind it. Its
    /// `old`/`new` pair is still exactly right to resume, since nothing has
    /// actually moved yet.
    ///
    /// Any other settle — one that already *committed* (`target == 1.0`,
    /// from a keyboard turn or a drag that made it past halfway) — is left
    /// to finish on its own and replaced outright by a fresh drag peeked
    /// from wherever navigation now stands, same/forward direction or not:
    /// `current_page` already moved when it committed, so grabbing its
    /// stale `old`/`new` pair instead would replay the transition that
    /// already happened rather than advancing further, and the eventual
    /// commit would then jump an extra spread past what the animation
    /// showed. An already-live drag is simply left alone.
    ///
    /// No-op if there's no adjacent spread that way (start of book going
    /// back, end going forward).
    pub fn start_page_drag(&mut self, forward: bool) {
        if let Some(t) = self.page_transition.as_mut() {
            if t.dragging {
                return;
            }
            if t.forward == forward && t.target == 0.0 {
                t.dragging = true;
                return;
            }
        }
        let Some(new) = self.peek_adjacent_pages(forward) else { return };
        let entry_sign = if forward { self.forward_entry_sign() } else { -self.forward_entry_sign() };
        self.page_transition = Some(PageTransition {
            entry_sign,
            forward,
            old_left: self.left_page(),
            old_right: self.right_page(),
            new_left: new.0,
            new_right: new.1,
            progress: 0.0,
            velocity: 0.0,
            target: 0.0,
            dragging: true,
        });
    }

    /// Moves a live drag's progress by `delta` (positive = further toward
    /// the new spread), clamped to `0.0..=1.0`. Also updates the drag's
    /// current velocity from `delta / dt`, exponentially smoothed so one
    /// noisy frame (trackpad wheel deltas arrive in uneven bursts) can't
    /// swing it wildly — used both for `end_page_drag`'s fling detection and,
    /// carried over into the settle spring at gesture end, so a fast flick
    /// keeps its momentum instead of the settle starting dead still. No-op
    /// once the drag has settled (`dragging` is `false`).
    pub fn drag_page_by(&mut self, delta: f32, dt: f32) {
        const VELOCITY_SMOOTHING: f32 = 0.3;

        if let Some(t) = self.page_transition.as_mut().filter(|t| t.dragging) {
            t.progress = (t.progress + delta).clamp(0.0, 1.0);
            if dt > 0.0 {
                let sample = delta / dt;
                t.velocity += (sample - t.velocity) * VELOCITY_SMOOTHING;
            }
        }
    }

    /// Ends a live drag: commits to the new spread either because it passed
    /// the halfway point (a slow, deliberate drag) or because it was
    /// released while moving fast toward it (a quick flick, regardless of
    /// how little distance it covered) — like a real page, it responds to
    /// either a long slow push or a short sharp one. Otherwise settles back
    /// to the current spread. Either way hands off to the settle spring
    /// (`step_transition`) for the remaining distance rather than snapping.
    /// If `one_turn_per_swipe` is on, also sets `swipe_locked` so this
    /// swipe's resolution (commit or cancel) is final — a momentum tail
    /// still trickling in afterward won't chain into another one. No-op if
    /// there's no live drag.
    pub fn end_page_drag(&mut self) {
        const COMMIT_THRESHOLD: f32 = 0.5;
        /// Progress-units-per-second (`progress` is `0.0..=1.0` across a
        /// full drag, see `input::scroll::DRAG_FULL_DISTANCE`) fast enough
        /// to count as a flick — comfortably above a slow full-width drag
        /// (~1.0/s) and below an actual quick flick (~5+/s).
        const FLING_VELOCITY: f32 = 3.0;

        let Some((progress, velocity, forward)) =
            self.page_transition.as_ref().filter(|t| t.dragging).map(|t| (t.progress, t.velocity, t.forward))
        else {
            return;
        };

        let commit =
            if velocity.abs() >= FLING_VELOCITY { velocity > 0.0 } else { progress >= COMMIT_THRESHOLD };
        if commit {
            if forward { self.advance_forward() } else { self.advance_backward() };
        }

        if let Some(t) = self.page_transition.as_mut() {
            t.dragging = false;
            t.target = if commit { 1.0 } else { 0.0 };
        }

        if self.one_turn_per_swipe {
            self.swipe_locked = true;
        }
    }

    /// Advances an in-flight page-turn slide by one frame (`dt` seconds) —
    /// call unconditionally, once per frame; a no-op while nothing is
    /// turning or while a trackpad drag is still live (its `progress` is
    /// driven directly by `drag_page_by` instead). The motion is a plain
    /// damped spring (`progress` toward `target`), tuned close to critically
    /// damped so it settles briskly with only the faintest overshoot rather
    /// than a linear or eased slide. `LayoutConfig::page_transition_ms`
    /// scales both stiffness and damping together, so raising or lowering it
    /// changes speed without changing how bouncy it feels. Clears
    /// `page_transition` once the spring is close enough to at rest that
    /// continuing to animate it wouldn't read as motion.
    pub fn step_transition(&mut self, dt: f32) {
        const BASE_STIFFNESS: f32 = 280.0;
        const BASE_DAMPING: f32 = 26.0;
        const BASELINE_MS: f32 = 220.0;
        const REST_VELOCITY: f32 = 0.02;
        const REST_DISTANCE: f32 = 0.003;

        let speed = (BASELINE_MS / self.layout.page_transition_ms.max(1) as f32).clamp(0.3, 3.0);
        let stiffness = BASE_STIFFNESS * speed * speed;
        let damping = BASE_DAMPING * speed;

        let Some(t) = self.page_transition.as_mut().filter(|t| !t.dragging) else { return };
        let force = -stiffness * (t.progress - t.target) - damping * t.velocity;
        t.velocity += force * dt;
        t.progress += t.velocity * dt;

        if t.velocity.abs() < REST_VELOCITY && (t.progress - t.target).abs() < REST_DISTANCE {
            self.page_transition = None;
        }
    }

    /// How long since the mouse last moved — drives the header auto-hide fade.
    pub fn idle_time(&self) -> Duration {
        self.last_mouse_move.elapsed()
    }

    /// Cycles LTR → RTL → Single Page → LTR — used by the single reading-
    /// mode button in the header/empty-state screen, which shows the
    /// current mode's own label and advances to the next one on each click.
    pub fn toggle_reading_mode(&mut self) {
        self.set_reading_mode(Self::next_reading_mode(self.reading_mode));
    }

    fn next_reading_mode(mode: ReadingMode) -> ReadingMode {
        match mode {
            ReadingMode::LTR => ReadingMode::RTL,
            ReadingMode::RTL => ReadingMode::Single,
            ReadingMode::Single => ReadingMode::LTR,
        }
    }

    /// Switches directly to `mode` — used by Settings' three explicit
    /// reading-direction buttons. No-op (skips the offset reset and save)
    /// if already in `mode`, so clicking the already-selected option is
    /// harmless.
    pub fn set_reading_mode(&mut self, mode: ReadingMode) {
        if self.apply_reading_mode(mode) {
            self.save_config();
        }
    }

    /// The mutating half of `set_reading_mode`, split out so it can be
    /// exercised without the accompanying disk write: switches to `mode`
    /// and resets any active peek offset. Returns whether anything actually
    /// changed (`false` if already in `mode`).
    fn apply_reading_mode(&mut self, mode: ReadingMode) -> bool {
        if self.reading_mode == mode {
            return false;
        }
        self.reading_mode = mode;
        self.page_offset = 0;
        true
    }

    /// Largest offset for which the spread's left page still stays in bounds.
    fn max_page_offset(&self) -> usize {
        self.total_pages.saturating_sub(self.current_page + 1)
    }

    pub fn shift_right(&mut self) {
        self.page_offset = (self.page_offset + 1).min(self.max_page_offset());
        self.mark_history_dirty();
    }

    pub fn shift_left(&mut self) {
        self.page_offset = self.page_offset.saturating_sub(1);
        self.mark_history_dirty();
    }

    fn mark_history_dirty(&mut self) {
        if self.current_path.is_some() {
            self.history_dirty = true;
        }
    }

    pub fn left_page(&self) -> Option<usize> {
        self.pages_for_position(self.effective_position()).0
    }

    pub fn right_page(&self) -> Option<usize> {
        self.pages_for_position(self.effective_position()).1
    }

    /// Opens the native file picker (multi-select); once it returns,
    /// `poll_picking` starts loading the first pick and queues the rest.
    pub fn start_loading_file(&mut self) {
        if self.loading {
            return;
        }
        self.pending_pick = Some(crate::comic::loader::spawn_file_picker());
    }

    /// Starts decoding an already-known path in the background — used by the
    /// file picker, reading history, the multi-file queue, session resume,
    /// and files passed on the command line.
    pub fn start_loading_path(&mut self, path: PathBuf) {
        self.loading = true;
        self.load_error = None;
        self.load_progress = (0, 0);
        self.load_preview = None;
        self.preview_texture = None;
        self.pending_load = Some(crate::comic::loader::spawn_file_load(path));
    }

    /// Pops the next queued file (if any) and starts loading it.
    pub fn open_next_in_queue(&mut self) {
        if let Some(path) = self.file_queue.pop_front() {
            self.start_loading_path(path);
        }
    }

    /// Call once per frame: picks up the file picker's result, if any.
    pub fn poll_picking(&mut self) {
        use crate::comic::loader::PickEvent;
        use std::sync::mpsc::TryRecvError;

        let Some(receiver) = &self.pending_pick else {
            return;
        };

        match receiver.try_recv() {
            Ok(PickEvent::Picked(paths)) => {
                self.pending_pick = None;
                let mut paths = paths.into_iter();
                if let Some(first) = paths.next() {
                    self.file_queue.extend(paths);
                    self.start_loading_path(first);
                }
            }
            Ok(PickEvent::Cancelled) | Err(TryRecvError::Disconnected) => {
                self.pending_pick = None;
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    /// Call once per frame: drains every background-load event queued since
    /// the last poll (progress ticks can arrive faster than frames).
    pub fn poll_loading(&mut self) {
        use crate::comic::loader::LoadEvent;
        use std::sync::mpsc::TryRecvError;

        let Some(receiver) = &self.pending_load else {
            return;
        };

        loop {
            match receiver.try_recv() {
                Ok(LoadEvent::Progress { loaded, total, first_page }) => {
                    self.load_progress = (loaded, total);
                    if first_page.is_some() {
                        self.load_preview = first_page;
                    }
                }
                Ok(LoadEvent::Finished(result)) => {
                    self.textures.clear();
                    self.page_meta.clear();
                    self.thumbnail_textures.clear();
                    self.thumbnail_hires_textures.clear();
                    self.thumbnail_pending = None;
                    self.thumbnail_queue.clear();
                    self.total_pages = result.pages.len();
                    self.pages = result.pages;
                    self.filename = result.filename;
                    self.current_path = Some(result.path.clone());

                    self.current_page = self.resume_to_page.take().unwrap_or(0);
                    self.page_offset = 0;

                    self.loading = false;
                    // Any decode still in flight for the previous archive is
                    // now moot; `load_generation` lets `poll_decoded_pages`
                    // tell such a late result apart from one for this book.
                    self.load_generation += 1;
                    self.decode_queue.clear();
                    self.load_preview = None;
                    self.preview_texture = None;
                    self.pending_load = None;
                    self.window_title = format!("Comic Reader — {}", self.filename);
                    self.title_dirty = true;
                    if self.show_page_preview {
                        self.warm_thumbnail_cache();
                    }

                    let last_page = self.effective_position();
                    self.history.touch(&result.path, &self.filename, last_page, self.total_pages);
                    self.history_dirty = false;
                    self.history_last_saved = std::time::Instant::now();
                    if let Err(err) = self.history.save() {
                        tracing::warn!("Failed to save history: {err}");
                    }
                    break;
                }
                Ok(LoadEvent::Failed(err)) => {
                    self.load_error = Some(err);
                    self.loading = false;
                    self.load_preview = None;
                    self.preview_texture = None;
                    self.pending_load = None;
                    self.resume_to_page = None;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // Dialog was cancelled or the worker thread panicked.
                    self.loading = false;
                    self.load_preview = None;
                    self.preview_texture = None;
                    self.pending_load = None;
                    self.resume_to_page = None;
                    break;
                }
            }
        }
    }

    /// Call once per frame: flushes the reading position to disk if it's
    /// changed and enough time has passed since the last save (debounced —
    /// navigating rapidly shouldn't hit disk every frame). `flush_history`
    /// bypasses the debounce for exit and fresh-load saves.
    pub fn poll_history_save(&mut self) {
        if self.history_dirty && self.history_last_saved.elapsed() >= HISTORY_SAVE_DEBOUNCE {
            self.flush_history();
        }
    }

    /// Records the current page against `current_path` (if any) and writes
    /// history to disk immediately. Safe to call with nothing loaded or
    /// nothing dirty — becomes a cheap no-op / redundant save.
    pub fn flush_history(&mut self) {
        if let Some(path) = &self.current_path {
            let last_page = self.effective_position();
            self.history.update_page(path, last_page);
        }
        if let Err(err) = self.history.save() {
            tracing::warn!("Failed to save history: {err}");
        }
        self.history_dirty = false;
        self.history_last_saved = std::time::Instant::now();
    }

    /// The warm-tint overlay color to paint over the whole window for the
    /// current filter strength, or `None` when the filter is off.
    pub fn blue_light_overlay_color(&self) -> Option<egui::Color32> {
        if self.blue_light_filter <= 0.0 {
            return None;
        }
        let alpha = (self.blue_light_filter.clamp(0.0, 1.0) * 150.0) as u8;
        Some(egui::Color32::from_rgba_unmultiplied(255, 147, 30, alpha))
    }

    /// The dimension cap to decode pages at, or `None` for full source
    /// resolution — depends on the user's downscale setting.
    pub fn decode_max_dimension(&self) -> Option<u32> {
        self.downscale_large_pages.then_some(DOWNSCALE_MAX_DIMENSION)
    }

    /// Makes the background decode queue want exactly the pages in
    /// `[low, high]` that don't have a texture yet — pages just ahead of (or
    /// behind) the current spread, so their textures are ready by the time
    /// the user turns to them instead of decoding synchronously on that
    /// frame. A page that leaves this range before the worker gets to it is
    /// dropped from the queue rather than decoded anyway — see `DecodeQueue`.
    pub fn request_prefetch(&self, low: usize, high: usize) {
        if self.pages.is_empty() {
            return;
        }
        let max_dimension = self.decode_max_dimension();
        let high = high.min(self.pages.len() - 1);
        let generation = self.load_generation;
        let pages = &self.pages;

        let wanted: HashSet<usize> = (low..=high).filter(|idx| !self.textures.contains_key(idx)).collect();
        self.decode_queue.reconcile(&wanted, |page_idx| DecodeRequest {
            generation,
            data: pages[page_idx].clone(),
            max_dimension,
        });
    }

    /// Call once per frame: turns every background-decoded page that's
    /// finished since the last poll into a GPU texture. Cheap no-op when
    /// nothing has arrived.
    pub fn poll_decoded_pages(&mut self, ctx: &egui::Context) {
        while let Ok(decoded) = self.decode_result_rx.try_recv() {
            if decoded.generation != self.load_generation {
                continue; // stale — belonged to a since-replaced archive
            }
            self.page_meta.entry(decoded.page_idx).or_insert(decoded.meta);
            self.textures.entry(decoded.page_idx).or_insert_with(|| {
                ctx.load_texture(format!("page_{}", decoded.page_idx), decoded.image, egui::TextureOptions::LINEAR)
            });
        }
    }

    /// Asks the background thumbnail worker (`comic::thumbnail`) to decode
    /// `page_idx`'s hover preview right away, ahead of any whole-book
    /// preload in progress, unless it's already cached or already the most
    /// recently requested page — called every frame `ui::progress_bar`
    /// shows a preview for `page_idx`, so holding the cursor still over one
    /// spot doesn't keep re-queuing (and re-cloning that page's bytes for)
    /// the same request.
    /// Above this many cached hi-res hover previews, the whole cache is
    /// dropped rather than evicted piecemeal — simplest way to bound its
    /// (much higher per-page) memory cost without tracking per-entry
    /// recency. Never hit in normal use (you'd need to scrub across this
    /// many distinct pages in one sitting), just a backstop.
    const THUMBNAIL_HIRES_CACHE_CAP: usize = 30;

    /// Asks the background thumbnail worker (`comic::thumbnail`) to decode
    /// `page_idx`'s *sharp* hover preview right away, ahead of anything
    /// else queued, unless it's already cached or already the most recently
    /// requested page — called every frame `ui::progress_bar` shows a
    /// preview for `page_idx`, so holding the cursor still over one spot
    /// doesn't keep re-queuing (and re-cloning that page's bytes for) the
    /// same request. The cheap low-res preview from `warm_thumbnail_cache`
    /// is what actually makes the popup feel instant; this just upgrades it
    /// to something sharp a few milliseconds later.
    pub fn request_thumbnail(&mut self, page_idx: usize) {
        if self.thumbnail_hires_textures.contains_key(&page_idx) || self.thumbnail_pending == Some(page_idx) {
            return;
        }
        let Some(data) = self.pages.get(page_idx) else { return };
        self.thumbnail_pending = Some(page_idx);
        self.thumbnail_queue.prioritize(crate::comic::thumbnail::ThumbnailRequest {
            generation: self.load_generation,
            page_idx,
            tier: crate::comic::thumbnail::ThumbnailTier::High,
            max_dimension: crate::comic::thumbnail::HIGH_RES_MAX_DIMENSION,
            data: data.clone(),
        });
    }

    /// Call once per frame: turns every background-decoded thumbnail that's
    /// finished since the last poll into a GPU texture, routed to the
    /// low-res or hi-res cache per its tier (capping the latter). Cheap
    /// no-op when nothing has arrived.
    pub fn poll_thumbnails(&mut self, ctx: &egui::Context) {
        use crate::comic::thumbnail::ThumbnailTier;

        while let Ok(decoded) = self.thumbnail_result_rx.try_recv() {
            if decoded.tier == ThumbnailTier::High && self.thumbnail_pending == Some(decoded.page_idx) {
                self.thumbnail_pending = None;
            }
            if decoded.generation != self.load_generation {
                continue; // stale — belonged to a since-replaced archive
            }
            let texture = ctx.load_texture(
                format!("thumb_{}_{:?}", decoded.page_idx, decoded.tier),
                decoded.image,
                egui::TextureOptions::LINEAR,
            );
            match decoded.tier {
                ThumbnailTier::Low => {
                    self.thumbnail_textures.insert(decoded.page_idx, texture);
                }
                ThumbnailTier::High => {
                    if self.thumbnail_hires_textures.len() >= Self::THUMBNAIL_HIRES_CACHE_CAP {
                        self.thumbnail_hires_textures.clear();
                    }
                    self.thumbnail_hires_textures.insert(decoded.page_idx, texture);
                }
            }
            // The whole point of decoding off-thread is to show the preview
            // the instant it's ready — don't wait for the next mouse move.
            ctx.request_repaint();
        }
    }

    /// Turns the whole page-preview feature on or off, from Settings.
    /// Turning it on immediately starts the whole-book low-res preload (if
    /// a book is open); turning it off drops everything it had cached —
    /// both tiers, plus anything still queued and any in-flight hover
    /// request — so the memory is actually freed, not just left unused.
    /// No-op (skips the save) if already in that state.
    pub fn set_show_page_preview(&mut self, enabled: bool) {
        if self.apply_show_page_preview(enabled) {
            self.save_config();
        }
    }

    /// The mutating half of `set_show_page_preview`, split out so it can be
    /// exercised without the accompanying disk write. Returns whether
    /// anything actually changed (`false` if already in that state).
    fn apply_show_page_preview(&mut self, enabled: bool) -> bool {
        if self.show_page_preview == enabled {
            return false;
        }
        self.show_page_preview = enabled;
        if enabled {
            if !self.pages.is_empty() {
                self.warm_thumbnail_cache();
            }
        } else {
            self.thumbnail_textures.clear();
            self.thumbnail_hires_textures.clear();
            self.thumbnail_queue.clear();
            self.thumbnail_pending = None;
        }
        true
    }

    /// Queues a background decode, at low priority and low resolution, for
    /// every page that doesn't already have a cached low-res thumbnail —
    /// called once when a book finishes loading, so the whole book
    /// gradually becomes ready to hover instantly well before the reader
    /// would ever check the progress bar, rather than only starting once
    /// they go looking for it. Kept for the entire session (cleared only
    /// when a different book is opened, same as the full-page texture
    /// cache) instead of being unloaded after a period of inactivity — at
    /// this resolution the memory cost is modest even for a very long book,
    /// and unloading was actively counterproductive: normal reading goes
    /// idle for longer than any reasonable timeout, so the cache would tend
    /// to be cold again right when it'd actually get used. `request_thumbnail`
    /// layers a sharper on-demand preview on top for whichever page is
    /// actually under the cursor.
    fn warm_thumbnail_cache(&mut self) {
        let generation = self.load_generation;
        let current = self.effective_position();
        let cached: HashSet<usize> = self.thumbnail_textures.keys().copied().collect();
        let wanted = Self::thumbnail_preload_order(self.total_pages, current, &cached);

        let pages = &self.pages;
        let requests = wanted
            .into_iter()
            .filter_map(|page_idx| {
                pages.get(page_idx).map(|data| crate::comic::thumbnail::ThumbnailRequest {
                    generation,
                    page_idx,
                    tier: crate::comic::thumbnail::ThumbnailTier::Low,
                    max_dimension: crate::comic::thumbnail::LOW_RES_MAX_DIMENSION,
                    data: data.clone(),
                })
            })
            .collect();
        self.thumbnail_queue.extend_low_priority(requests);
    }

    /// Every page index in `0..total_pages` not already in `cached`,
    /// ordered by distance from `current` (closest first) — split out from
    /// `warm_thumbnail_cache` as a pure function purely so the ordering
    /// itself is unit-testable without needing real texture handles or a
    /// running decode worker.
    fn thumbnail_preload_order(total_pages: usize, current: usize, cached: &HashSet<usize>) -> Vec<usize> {
        let mut wanted: Vec<usize> = (0..total_pages).filter(|idx| !cached.contains(idx)).collect();
        wanted.sort_by_key(|&idx| (idx as isize - current as isize).abs());
        wanted
    }

    /// Starts a background check against GitHub's latest release. No-op if
    /// one's already in flight (a check or a download).
    pub fn check_for_updates(&mut self) {
        if matches!(self.update_status, UpdateStatus::Checking | UpdateStatus::Downloading) {
            return;
        }
        self.update_status = UpdateStatus::Checking;
        self.pending_update_check = Some(crate::update::spawn_check());
    }

    /// Starts downloading and applying the update found by the last check.
    /// No-op if `update_status` isn't currently `Available`.
    pub fn start_update_download(&mut self) {
        let UpdateStatus::Available(info) = self.update_status.clone() else { return };
        self.update_status = UpdateStatus::Downloading;
        self.pending_update_apply = Some(crate::update::spawn_apply(info));
    }

    /// Relaunches the app (picking up the just-applied update) and exits
    /// this process. Only meaningful once `update_status` is `Ready`.
    pub fn restart_to_apply_update(&mut self) {
        self.flush_history();
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).spawn();
        }
        std::process::exit(0);
    }

    /// Call once per frame: picks up the result of a background update
    /// check, if any.
    pub fn poll_update_check(&mut self) {
        use std::sync::mpsc::TryRecvError;

        let Some(receiver) = &self.pending_update_check else { return };
        match receiver.try_recv() {
            Ok(CheckEvent::Available(info)) => {
                self.update_status = UpdateStatus::Available(info);
                self.pending_update_check = None;
            }
            Ok(CheckEvent::UpToDate) => {
                self.update_status = UpdateStatus::Idle;
                self.pending_update_check = None;
            }
            Ok(CheckEvent::Failed(err)) => {
                tracing::warn!("Update check failed: {err}");
                self.update_status = UpdateStatus::Idle;
                self.pending_update_check = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.pending_update_check = None,
        }
    }

    /// Call once per frame: picks up the result of a background update
    /// download/apply, if any.
    pub fn poll_update_apply(&mut self) {
        use std::sync::mpsc::TryRecvError;

        let Some(receiver) = &self.pending_update_apply else { return };
        match receiver.try_recv() {
            Ok(ApplyEvent::Done) => {
                self.update_status = UpdateStatus::Ready;
                self.pending_update_apply = None;
            }
            Ok(ApplyEvent::Failed(err)) => {
                tracing::warn!("Failed to apply update: {err}");
                self.update_status = UpdateStatus::Failed(err);
                self.pending_update_apply = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.pending_update_apply = None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comic::archive::{EdgeColors, PageMeta};

    /// A `ComicApp` with `total_pages` pages and no real archive bytes —
    /// enough to exercise the pure navigation math, which only touches
    /// `current_page`/`page_offset`/`total_pages`/`page_meta`. Indices in
    /// `doubles` are marked as double-page spreads.
    fn app_with(total_pages: usize, doubles: &[usize]) -> ComicApp {
        let mut app = ComicApp::default();
        app.total_pages = total_pages;
        let placeholder = EdgeColors { left: egui::Color32::WHITE, right: egui::Color32::WHITE };
        for &i in doubles {
            app.page_meta.insert(i, PageMeta { edge: placeholder, is_spread: true });
        }
        app
    }

    #[test]
    fn pairs_pages_normally_with_no_double_pages() {
        let app = app_with(4, &[]);
        assert_eq!((app.left_page(), app.right_page()), (Some(0), Some(1)));
    }

    #[test]
    fn double_page_shown_alone_and_realigns_pairing_around_it() {
        // Pages: 0(single) 1(single) 2(DOUBLE) 3(single) 4(single).
        let mut app = app_with(5, &[2]);
        assert_eq!((app.left_page(), app.right_page()), (Some(0), Some(1)));

        app.next_spread();
        assert_eq!((app.left_page(), app.right_page()), (Some(2), None));

        app.next_spread();
        assert_eq!((app.left_page(), app.right_page()), (Some(3), Some(4)));

        app.prev_spread();
        assert_eq!((app.left_page(), app.right_page()), (Some(2), None));

        app.prev_spread();
        assert_eq!((app.left_page(), app.right_page()), (Some(0), Some(1)));
    }

    #[test]
    fn page_before_a_double_page_is_also_shown_alone() {
        // Pages: 0(single) 1(DOUBLE) 2(single) 3(single) — 0 can't pair with
        // 1, so it's orphaned solo even though 0 itself isn't a spread.
        let mut app = app_with(4, &[1]);
        assert_eq!((app.left_page(), app.right_page()), (Some(0), None));

        app.next_spread();
        assert_eq!((app.left_page(), app.right_page()), (Some(1), None));

        app.next_spread();
        assert_eq!((app.left_page(), app.right_page()), (Some(2), Some(3)));
    }

    #[test]
    fn rtl_swaps_which_side_each_page_of_a_pair_is_on() {
        let mut app = app_with(4, &[]);
        app.reading_mode = ReadingMode::RTL;
        assert_eq!((app.left_page(), app.right_page()), (Some(1), Some(0)));
    }

    #[test]
    fn single_page_mode_shows_one_page_and_turns_one_at_a_time() {
        let mut app = app_with(5, &[]);
        app.reading_mode = ReadingMode::Single;
        assert_eq!((app.left_page(), app.right_page()), (Some(0), None));

        app.next_spread();
        assert_eq!((app.left_page(), app.right_page()), (Some(1), None));

        app.next_spread();
        assert_eq!((app.left_page(), app.right_page()), (Some(2), None));

        app.prev_spread();
        assert_eq!((app.left_page(), app.right_page()), (Some(1), None));

        app.prev_spread();
        assert_eq!((app.left_page(), app.right_page()), (Some(0), None));

        // Already at the first page: nothing further back to step to.
        app.prev_spread();
        assert_eq!((app.left_page(), app.right_page()), (Some(0), None));
    }

    #[test]
    fn single_page_mode_ignores_double_page_pairing_rules() {
        // Pages: 0(single) 1(DOUBLE) 2(single) 3(single) — in LTR/RTL mode
        // page 0 would be orphaned solo because it can't pair with the
        // double page at 1, but Single Page mode never pairs anything, so
        // stepping through is uniform regardless of what's a double page.
        let mut app = app_with(4, &[1]);
        app.reading_mode = ReadingMode::Single;

        app.next_spread();
        assert_eq!((app.left_page(), app.right_page()), (Some(1), None));
        app.next_spread();
        assert_eq!((app.left_page(), app.right_page()), (Some(2), None));
        app.next_spread();
        assert_eq!((app.left_page(), app.right_page()), (Some(3), None));
    }

    #[test]
    fn reading_mode_cycles_ltr_rtl_single_ltr() {
        // Exercises the pure cycling logic directly rather than through
        // `toggle_reading_mode`, which also calls `save_config` (writes the
        // user's real config file) — not something a unit test should do.
        assert_eq!(ComicApp::next_reading_mode(ReadingMode::LTR), ReadingMode::RTL);
        assert_eq!(ComicApp::next_reading_mode(ReadingMode::RTL), ReadingMode::Single);
        assert_eq!(ComicApp::next_reading_mode(ReadingMode::Single), ReadingMode::LTR);
    }

    #[test]
    fn apply_reading_mode_resets_peek_offset_and_reports_change() {
        // Exercises `set_reading_mode`'s pure mutation half directly, since
        // the public method also calls `save_config` (writes the user's
        // real config file) — not something a unit test should do.
        let mut app = app_with(4, &[]);
        app.shift_right();
        assert_eq!(app.page_offset, 1);

        assert!(app.apply_reading_mode(ReadingMode::Single));
        assert_eq!(app.page_offset, 0);
        assert_eq!(app.reading_mode, ReadingMode::Single);

        // Re-applying the same mode is a no-op and reports no change.
        assert!(!app.apply_reading_mode(ReadingMode::Single));
    }

    /// Steps `app`'s transition at a fixed 60fps timestep until it's fully
    /// settled (`page_transition` cleared) or `max_steps` is hit — used to
    /// check the settle spring actually converges instead of oscillating or
    /// drifting forever, which a mistuned stiffness/damping pair easily can.
    fn run_transition_to_rest(app: &mut ComicApp, max_steps: usize) -> usize {
        const DT: f32 = 1.0 / 60.0;
        for step in 0..max_steps {
            if app.page_transition.is_none() {
                return step;
            }
            app.step_transition(DT);
        }
        max_steps
    }

    #[test]
    fn settle_spring_converges_from_rest() {
        let mut app = app_with(4, &[]);
        app.page_transition = Some(PageTransition {
            entry_sign: 1.0,
            forward: true,
            old_left: Some(0),
            old_right: Some(1),
            new_left: Some(2),
            new_right: Some(3),
            progress: 0.0,
            velocity: 0.0,
            target: 1.0,
            dragging: false,
        });

        let steps = run_transition_to_rest(&mut app, 600); // 10s at 60fps
        assert!(steps < 600, "settle spring never came to rest");
        assert!(app.page_transition.is_none());
    }

    #[test]
    fn next_and_prev_spread_animate_and_settle() {
        let mut app = app_with(6, &[]);
        app.next_spread();
        assert!(app.page_transition.is_some());
        let steps = run_transition_to_rest(&mut app, 600);
        assert!(steps < 600, "page-turn spring never came to rest");
        assert_eq!((app.left_page(), app.right_page()), (Some(2), Some(3)));
    }

    /// Applies `total_delta` progress to `app`'s live drag spread evenly
    /// over `frames` calls at a fixed 60fps timestep — simulates a gesture
    /// unfolding at a steady, realistic pace (as opposed to one giant
    /// instantaneous delta, which `drag_page_by`'s velocity smoothing would
    /// read as an implausibly fast flick regardless of `total_delta`).
    fn drag_over_frames(app: &mut ComicApp, total_delta: f32, frames: u32) {
        const DT: f32 = 1.0 / 60.0;
        for _ in 0..frames {
            app.drag_page_by(total_delta / frames as f32, DT);
        }
    }

    #[test]
    fn slow_drag_past_halfway_then_released_commits_to_next_spread() {
        let mut app = app_with(6, &[]);
        app.start_page_drag(true);
        assert!(app.is_dragging());
        drag_over_frames(&mut app, 0.6, 60); // ~0.6 progress over 1s: well under fling speed
        // Still mid-gesture: nothing committed yet.
        assert_eq!((app.left_page(), app.right_page()), (Some(0), Some(1)));

        app.end_page_drag();
        assert!(!app.is_dragging());
        assert_eq!((app.left_page(), app.right_page()), (Some(2), Some(3)));

        let steps = run_transition_to_rest(&mut app, 600);
        assert!(steps < 600, "post-commit settle spring never came to rest");
        assert_eq!((app.left_page(), app.right_page()), (Some(2), Some(3)));
    }

    #[test]
    fn slow_drag_short_of_halfway_then_released_snaps_back() {
        let mut app = app_with(6, &[]);
        app.start_page_drag(true);
        drag_over_frames(&mut app, 0.3, 60); // ~0.3 progress over 1s: well under fling speed

        app.end_page_drag();
        assert!(!app.is_dragging());
        // Cancelled: current spread hasn't moved.
        assert_eq!((app.left_page(), app.right_page()), (Some(0), Some(1)));

        let steps = run_transition_to_rest(&mut app, 600);
        assert!(steps < 600, "post-cancel settle spring never came to rest");
        assert_eq!((app.left_page(), app.right_page()), (Some(0), Some(1)));
    }

    #[test]
    fn quick_flick_commits_despite_covering_little_distance() {
        let mut app = app_with(6, &[]);
        app.start_page_drag(true);
        // A short, sharp swipe: well short of halfway, but fast enough to
        // read as a flick — should commit anyway, like a real page would
        // respond to a quick flick as readily as a long slow push.
        app.drag_page_by(0.25, 1.0 / 60.0);
        assert!(app.page_transition.as_ref().unwrap().progress < 0.5);

        app.end_page_drag();
        assert!(!app.is_dragging());
        assert_eq!((app.left_page(), app.right_page()), (Some(2), Some(3)));
    }

    #[test]
    fn drag_at_start_of_book_cannot_go_backward() {
        let mut app = app_with(6, &[]);
        app.start_page_drag(false);
        assert!(!app.is_dragging());
        assert!(app.page_transition.is_none());
    }

    #[test]
    fn jump_to_page_moves_directly_there() {
        let mut app = app_with(10, &[]);
        app.jump_to_page(6);
        assert_eq!((app.left_page(), app.right_page()), (Some(6), Some(7)));
    }

    #[test]
    fn jump_to_page_clamps_past_the_end() {
        let mut app = app_with(10, &[]);
        app.jump_to_page(999);
        assert_eq!(app.current_page, 9);
    }

    #[test]
    fn jump_to_page_clears_peek_offset_and_any_in_flight_transition() {
        let mut app = app_with(10, &[]);
        app.shift_right();
        app.next_spread(); // leaves an in-flight page_transition
        assert!(app.page_transition.is_some());

        app.jump_to_page(4);
        assert_eq!(app.page_offset, 0);
        assert!(app.page_transition.is_none());
        assert_eq!((app.left_page(), app.right_page()), (Some(4), Some(5)));
    }

    #[test]
    fn request_thumbnail_marks_pending_and_avoids_requeuing_the_same_page() {
        let mut app = app_with(4, &[]);
        app.pages = vec![vec![0u8]; 4]; // dummy bytes — only pending-tracking matters here

        assert!(app.thumbnail_pending.is_none());
        app.request_thumbnail(2);
        assert_eq!(app.thumbnail_pending, Some(2));

        // Still hovering the same page: no-op, stays marked pending for it.
        app.request_thumbnail(2);
        assert_eq!(app.thumbnail_pending, Some(2));
    }

    #[test]
    fn request_thumbnail_skips_a_page_already_cached_at_hires() {
        let mut app = app_with(4, &[]);
        app.pages = vec![vec![0u8]; 4];

        let ctx = egui::Context::default();
        let image = egui::ColorImage::filled([2, 2], egui::Color32::WHITE);
        let texture = ctx.load_texture("test_thumb", image, egui::TextureOptions::LINEAR);
        // A low-res cache hit alone shouldn't skip the request: it's the
        // hi-res tier `request_thumbnail` is responsible for.
        app.thumbnail_textures.insert(1, texture.clone());
        app.request_thumbnail(1);
        assert_eq!(app.thumbnail_pending, Some(1));

        app.thumbnail_pending = None;
        app.thumbnail_hires_textures.insert(1, texture);
        app.request_thumbnail(1);
        assert!(app.thumbnail_pending.is_none()); // never queued: already cached at hi-res
    }

    #[test]
    fn thumbnail_preload_order_starts_from_the_current_page_outward() {
        let cached = HashSet::new();
        let order = ComicApp::thumbnail_preload_order(6, 2, &cached);
        assert_eq!(order, vec![2, 1, 3, 0, 4, 5]);
    }

    #[test]
    fn thumbnail_preload_order_skips_already_cached_pages() {
        let cached: HashSet<usize> = [2, 3].into_iter().collect();
        let order = ComicApp::thumbnail_preload_order(6, 2, &cached);
        assert_eq!(order, vec![1, 0, 4, 5]);
    }

    #[test]
    fn disabling_page_preview_unloads_both_caches_and_pending_state() {
        let mut app = app_with(4, &[]);
        app.pages = vec![vec![0u8]; 4];

        let ctx = egui::Context::default();
        let image = egui::ColorImage::filled([2, 2], egui::Color32::WHITE);
        app.thumbnail_textures.insert(0, ctx.load_texture("a", image.clone(), egui::TextureOptions::LINEAR));
        app.thumbnail_hires_textures.insert(0, ctx.load_texture("b", image, egui::TextureOptions::LINEAR));
        app.thumbnail_pending = Some(0);

        assert!(app.apply_show_page_preview(false));
        assert!(app.thumbnail_textures.is_empty());
        assert!(app.thumbnail_hires_textures.is_empty());
        assert!(app.thumbnail_pending.is_none());
        assert!(!app.show_page_preview);
    }

    #[test]
    fn toggling_page_preview_to_the_same_state_is_a_no_op() {
        let mut app = app_with(4, &[]);
        assert!(app.show_page_preview); // on by default
        assert!(!app.apply_show_page_preview(true));
    }

    #[test]
    fn resolving_a_swipe_locks_out_further_turns_when_one_turn_per_swipe_is_on() {
        let mut app = app_with(6, &[]);
        assert!(app.one_turn_per_swipe); // on by default
        app.start_page_drag(true);
        drag_over_frames(&mut app, 0.6, 60);
        app.end_page_drag();
        assert!(app.swipe_locked);

        // Locked out: a fresh swipe attempt (e.g. momentum still trickling
        // in after the fingers lifted) is ignored by `input::scroll` before
        // it ever reaches `start_page_drag` — but even calling it directly
        // here, the page shouldn't move any further than the one commit.
        assert_eq!((app.left_page(), app.right_page()), (Some(2), Some(3)));
    }

    #[test]
    fn resolving_a_swipe_does_not_lock_when_one_turn_per_swipe_is_off() {
        let mut app = app_with(6, &[]);
        app.one_turn_per_swipe = false;
        app.start_page_drag(true);
        drag_over_frames(&mut app, 0.6, 60);
        app.end_page_drag();
        assert!(!app.swipe_locked);
    }

    #[test]
    fn resolving_a_cancelled_swipe_also_locks_when_one_turn_per_swipe_is_on() {
        let mut app = app_with(6, &[]);
        app.start_page_drag(true);
        drag_over_frames(&mut app, 0.2, 60); // short of halfway: cancels
        app.end_page_drag();
        assert!(app.swipe_locked);
        assert_eq!((app.left_page(), app.right_page()), (Some(0), Some(1)));
    }

    #[test]
    fn retrying_same_direction_mid_bounce_grabs_the_settle_instead_of_queuing() {
        let mut app = app_with(6, &[]);
        app.start_page_drag(true);
        drag_over_frames(&mut app, 0.3, 60); // slow: stays under fling speed
        app.end_page_drag();
        // Cancelled: now settling back toward the current spread, without
        // having stepped the spring at all yet.
        assert!(!app.is_dragging());
        let progress_at_bounce_start = app.page_transition.as_ref().unwrap().progress;
        assert!(progress_at_bounce_start > 0.0);

        // Swiping forward again immediately should grab that settle from
        // wherever it currently is, not get dropped until it finishes.
        app.start_page_drag(true);
        assert!(app.is_dragging());
        assert_eq!(app.page_transition.as_ref().unwrap().progress, progress_at_bounce_start);

        drag_over_frames(&mut app, 0.3, 60); // push the rest of the way, slowly
        app.end_page_drag();
        assert_eq!((app.left_page(), app.right_page()), (Some(2), Some(3)));
    }

    #[test]
    fn retrying_opposite_direction_mid_bounce_replaces_it() {
        let mut app = app_with(6, &[]);
        app.next_spread();
        assert!(run_transition_to_rest(&mut app, 600) < 600);
        assert_eq!((app.left_page(), app.right_page()), (Some(2), Some(3)));

        app.start_page_drag(true);
        drag_over_frames(&mut app, 0.2, 60); // slow: stays under fling speed
        app.end_page_drag();
        assert!(!app.is_dragging());
        assert!(app.page_transition.as_ref().unwrap().forward);

        // Swiping backward instead discards that still-settling forward
        // bounce-back and starts a fresh backward drag from scratch.
        app.start_page_drag(false);
        assert!(app.is_dragging());
        let t = app.page_transition.as_ref().unwrap();
        assert!(!t.forward);
        assert_eq!(t.progress, 0.0);

        drag_over_frames(&mut app, 0.6, 60);
        app.end_page_drag();
        assert_eq!((app.left_page(), app.right_page()), (Some(0), Some(1)));
    }

    #[test]
    fn retrying_same_direction_during_a_commit_settle_advances_further_instead_of_replaying_it() {
        let mut app = app_with(8, &[]);
        app.next_spread(); // commits immediately: (0,1) -> (2,3), settling
        assert_eq!((app.left_page(), app.right_page()), (Some(2), Some(3)));
        assert_eq!(app.page_transition.as_ref().unwrap().target, 1.0);

        // Swiping forward again before that settle finishes must NOT grab
        // its stale (0,1)->(2,3) pair — it already happened — but instead
        // peek from wherever navigation now stands, toward (4,5).
        app.start_page_drag(true);
        assert!(app.is_dragging());
        let t = app.page_transition.as_ref().unwrap();
        assert_eq!((t.old_left, t.old_right), (Some(2), Some(3)));
        assert_eq!((t.new_left, t.new_right), (Some(4), Some(5)));

        drag_over_frames(&mut app, 0.6, 60);
        app.end_page_drag();
        assert_eq!((app.left_page(), app.right_page()), (Some(4), Some(5)));
    }
}