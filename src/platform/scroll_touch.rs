//! Whether a trackpad's fingers are still down during a scroll-wheel
//! gesture, or the OS is instead coasting on momentum after they lifted —
//! `ui::reader::webtoon_edge_hold_step`'s edge-hold gesture needs this to
//! keep counting toward arming while it's genuinely still being pushed
//! (without letting a flick's momentum tail count the same way), and to
//! drop or resolve a hold the instant fingers actually lift rather than
//! however long `gesture_ended` takes to catch up. Plain `egui`/`winit`
//! touch-phase info (`Event::MouseWheel`'s `TouchPhase`) can't tell any of
//! this apart here — see that function's own docs for why — but the raw
//! `NSEvent` can: every scroll-wheel event carries a `momentumPhase`
//! distinct from its ordinary `phase`, `.none` while fingers are actually
//! on the pad and only ever set once they've lifted and it's coasting.
//!
//! Reads that off an `NSEvent` local monitor — same category of technique
//! `platform::macos` uses for the Open Documents event, just the public
//! monitor API instead of delegate-swizzling, and with no timing
//! constraint that early since nothing here needs to observe events from
//! before the app's own window exists. Exposes it as a plain poll,
//! `finger_down()`, rather than a queue — callers only ever want "is a
//! finger on the pad right now," not "everything since last frame."
//!
//! `finger_down()` returns `Option<bool>`, not `bool`: this is only ever a
//! real signal on macOS, so every other platform returns `None` ("no
//! opinion") rather than a guessed `false`, which callers can then fall
//! back to their old, signal-free behavior on instead of treating as
//! "definitely not touching."

#[cfg(target_os = "macos")]
mod imp {
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventPhase};
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Defaults to `true` (i.e. "trust real movement") until the first
    /// scroll-wheel event actually arrives, so a hold started before any
    /// event has reached the monitor isn't treated as momentum by default.
    static FINGER_DOWN: AtomicBool = AtomicBool::new(true);

    /// Installs the monitor described above. Call once, from `main.rs`'s
    /// `eframe` app-creation closure — by then `NSApp` is already up and
    /// pumping events (unlike `platform::macos::install_open_file_handler`,
    /// this doesn't need to run any earlier than that, since it isn't
    /// racing to catch a launch-time event). The returned monitor handle
    /// is deliberately leaked: it has to live for the process's entire
    /// lifetime, same as the app itself, and there's no matching
    /// `removeMonitor:` call site that would ever want to tear it down
    /// early.
    pub fn install() {
        // SAFETY: the handler only reads `event`'s phase fields and hands
        // the same pointer straight back, so winit's own scroll handling
        // still sees every event exactly as it would with no monitor
        // installed at all.
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                NSEventMask::ScrollWheel,
                &block2::RcBlock::new(|event: std::ptr::NonNull<NSEvent>| {
                    let down = event.as_ref().momentumPhase() == NSEventPhase::None;
                    FINGER_DOWN.store(down, Ordering::Relaxed);
                    event.as_ptr()
                }),
            )
        };
        std::mem::forget(monitor);
    }

    pub fn finger_down() -> Option<bool> {
        Some(FINGER_DOWN.load(Ordering::Relaxed))
    }
}

#[cfg(target_os = "macos")]
pub use imp::{finger_down, install};

#[cfg(not(target_os = "macos"))]
pub fn install() {}

#[cfg(not(target_os = "macos"))]
pub fn finger_down() -> Option<bool> {
    None
}
