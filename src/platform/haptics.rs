//! A haptic tap on the trackpad, for gestures — currently just
//! `ui::reader::webtoon_edge_hold_step`'s Webtoon edge-hold — that want a
//! physical cue for a state change the reader might not be looking at the
//! badge to notice. Compiled on every platform (so call sites never need
//! their own `#[cfg]`); only macOS actually does anything, via AppKit's
//! `NSHapticFeedbackManager` (already linked for `platform::macos`'s own
//! use, just with the extra `NSHapticFeedback` Cargo feature enabled) — a
//! silent no-op everywhere else, and on a Mac with no Force Touch trackpad
//! (`defaultPerformer` itself handles that; nothing here needs to know).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HapticTap {
    /// A push just crossed the minimum distance and started actually
    /// counting down toward arming.
    MinOverscrollCrossed,
    /// A hold just armed — releasing now opens the sibling volume.
    Armed,
}

#[cfg(target_os = "macos")]
pub fn perform(tap: HapticTap) {
    use objc2_app_kit::{NSHapticFeedbackManager, NSHapticFeedbackPattern, NSHapticFeedbackPerformanceTime, NSHapticFeedbackPerformer};

    // `NSHapticFeedbackManager`'s public API has no intensity/strength
    // parameter — only these three fixed patterns — so `LevelChange`
    // (AppKit's own strongest, used for e.g. snapping to a volume/
    // brightness limit) is already as strong as a single tap gets for
    // `Armed`. Firing it twice back to back reads as a firmer, more
    // emphatic double-pulse rather than a single light tap, which is as
    // close to "stronger" as this API allows.
    let performer = NSHapticFeedbackManager::defaultPerformer();
    match tap {
        HapticTap::MinOverscrollCrossed => {
            performer.performFeedbackPattern_performanceTime(NSHapticFeedbackPattern::Generic, NSHapticFeedbackPerformanceTime::Now);
        }
        HapticTap::Armed => {
            performer.performFeedbackPattern_performanceTime(NSHapticFeedbackPattern::LevelChange, NSHapticFeedbackPerformanceTime::Now);
            performer.performFeedbackPattern_performanceTime(NSHapticFeedbackPattern::LevelChange, NSHapticFeedbackPerformanceTime::Now);
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn perform(_tap: HapticTap) {}
