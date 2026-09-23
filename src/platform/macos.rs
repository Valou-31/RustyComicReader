//! Handles macOS's "Open Documents" Apple Event — what Finder sends a
//! launching or already-running app when the user double-clicks or "Open
//! With"s a `.cbz`/`.cb7`/`.cbr` file, or drags one onto the Dock icon.
//!
//! `Info.plist` (`packaging/macos/Info.plist`) already declares the UTIs so
//! Finder lists Comic Reader as an option at all; without this module the
//! event never reached the app, so double-click/"Open With" launched to the
//! empty-state screen with nothing loaded (CLI-arg opening always worked
//! fine, since that's a plain argv the OS never needs an Apple Event for).
//!
//! The obvious fix — install our own `NSApplicationDelegate` implementing
//! `application:openURLs:` — doesn't work here: `eframe`'s winit backend
//! (0.30.x) already claims `NSApplication`'s one delegate slot for its own
//! `WinitApplicationDelegate` inside `EventLoop::new` (see
//! `winit::platform_impl::macos::app_state`), and calling `setDelegate`
//! again ourselves would silently detach it — breaking the
//! `applicationDidFinishLaunching:`/`applicationWillTerminate:` handling
//! winit relies on to actually start pumping events and exit cleanly.
//!
//! So instead of replacing that delegate, this patches it in place —
//! `class_addMethod` adds an `application:openURLs:` implementation
//! directly onto whatever class gets set as `NSApplication`'s delegate,
//! exactly what an Objective-C category compiles down to, just issued from
//! Rust, leaving every method winit already installed untouched. Verified
//! empirically (build the real `.app`, `open -a` it with a file) that this
//! mechanism works correctly.
//!
//! Getting the *timing* right took a second attempt: patching the delegate
//! class from inside `eframe`'s app-creation closure — the obvious place,
//! since that's the first point at which the delegate is guaranteed to
//! exist — installs too late. That closure runs as part of winit
//! dispatching its init events, which happens only once its `run_app`'s
//! internal `[NSApp run]` is already pumping — by which point AppKit has,
//! empirically, already dispatched (and dropped, finding no handler) the
//! launch-time Open Documents event. A *second* `open -a` sent to the same,
//! already-running process worked immediately, confirming the patch itself
//! is correct and it's purely a launch-time race.
//!
//! The fix: swizzle `-[NSApplication setDelegate:]` itself, installed at
//! the very top of `main`, before `eframe::run_native` (and so before
//! winit's `EventLoop::new`) ever runs. Winit's own `EventLoop::new` calls
//! `app.setDelegate(...)` synchronously, long before it calls `run_app`
//! (i.e. before `[NSApp run]` starts pumping events at all — Apple Events
//! can't be dispatched before then), so patching *at the moment the
//! delegate is set* rather than *some point after* closes the race
//! entirely, without needing to know winit's delegate class by name (this
//! patches whatever class the very next `setDelegate:` call receives).
use objc2::ffi::{class_addMethod, method_setImplementation, object_getClass};
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::sel;
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSArray, NSObject, NSURL};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Files handed to us by `application_open_urls` since the last
/// `take_opened_files` poll.
fn opened_files() -> &'static Mutex<Vec<PathBuf>> {
    static OPENED_FILES: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
    OPENED_FILES.get_or_init(|| Mutex::new(Vec::new()))
}

/// Drains every file macOS has handed us via Apple Event since the last
/// call — polled once a frame by `ComicApp::poll_macos_open_files`. Covers
/// both a fresh launch (the very first Apple Event, arriving after `main`
/// has already handed `ComicApp::new`/`new_with_files` the CLI-arg files —
/// harmless, since on a real double-click/"Open With" launch there are no
/// CLI args, so this queue is the only source) and a file opened while
/// already running.
pub fn take_opened_files() -> Vec<PathBuf> {
    std::mem::take(&mut *opened_files().lock().unwrap())
}

/// The original `-[NSApplication setDelegate:]`, so the swizzle below can
/// still forward to it — everything winit relies on for its own lifecycle
/// handling still runs exactly as before, unpatched.
type SetDelegateFn = unsafe extern "C-unwind" fn(&NSApplication, Sel, *mut AnyObject);
static ORIGINAL_SET_DELEGATE: OnceLock<Imp> = OnceLock::new();

/// Installs the `setDelegate:` swizzle described above. Call once, at the
/// very top of `main`, before touching `eframe`/winit at all. Safe to call
/// even if something later never actually sets a delegate — it just never
/// fires, and macOS file-open stays unsupported instead of crashing.
pub fn install_open_file_handler() {
    let Some(class) = AnyClass::get(c"NSApplication") else {
        return;
    };
    let sel = sel!(setDelegate:);
    // SAFETY: `class_getInstanceMethod` (via `object_getClass`'s sibling
    // API) with a valid, live class and selector is safe; the null check
    // below covers the (practically impossible) case where `NSApplication`
    // has no `setDelegate:` at all.
    let method = unsafe { objc2::ffi::class_getInstanceMethod(class, sel) };
    if method.is_null() {
        return;
    }
    // SAFETY: `new_imp`'s signature (`&NSApplication, Sel, *mut AnyObject`)
    // matches `-[NSApplication setDelegate:]`'s real one (`self, _cmd,
    // id<NSApplicationDelegate>`) exactly, and `method_setImplementation`
    // is the runtime primitive method swizzling compiles down to — used
    // here on a system class specifically to observe every future
    // `setDelegate:` call, which is exactly what it's for. The returned
    // original implementation is stored so the swizzle can still forward to
    // it, preserving winit's own delegate-setting behavior unchanged.
    let new_imp = unsafe { std::mem::transmute::<SetDelegateFn, Imp>(swizzled_set_delegate) };
    let Some(original) = (unsafe { method_setImplementation(method, new_imp) }) else {
        return;
    };
    let _ = ORIGINAL_SET_DELEGATE.set(original);
}

/// Replaces `-[NSApplication setDelegate:]`: patches `application:openURLs:`
/// onto whatever class `delegate` is (winit's `WinitApplicationDelegate`,
/// in practice — the only thing that ever calls this in this app), then
/// forwards to the original implementation so `NSApp.delegate` still ends
/// up set exactly as before.
unsafe extern "C-unwind" fn swizzled_set_delegate(this: &NSApplication, cmd: Sel, delegate: *mut AnyObject) {
    if let Some(delegate_ref) = unsafe { delegate.as_ref() } {
        patch_open_urls_onto(delegate_ref);
    }
    if let Some(&original) = ORIGINAL_SET_DELEGATE.get() {
        let original: SetDelegateFn = unsafe { std::mem::transmute(original) };
        unsafe { original(this, cmd, delegate) };
    }
}

/// Adds `application:openURLs:` to `delegate`'s own class — a no-op (fails
/// harmlessly) if that class already implements the selector itself, and
/// otherwise leaves every method it already had untouched.
fn patch_open_urls_onto(delegate: &AnyObject) {
    let class = unsafe { object_getClass(delegate as *const AnyObject) };
    if class.is_null() {
        return;
    }
    // SAFETY: `class_addMethod` is the runtime primitive an Objective-C
    // category compiles down to. `types` (`"v@:@@"`: void return; self,
    // _cmd, then two object arguments) matches `application_open_urls`'s
    // signature below exactly.
    unsafe {
        class_addMethod(
            class.cast_mut(),
            sel!(application:openURLs:),
            std::mem::transmute::<
                unsafe extern "C-unwind" fn(&NSObject, Sel, &NSApplication, &NSArray<NSURL>),
                Imp,
            >(application_open_urls),
            c"v@:@@".as_ptr(),
        );
    }
}

/// The `application:openURLs:` implementation patched onto NSApp's
/// delegate. Extracts each URL's filesystem path and queues it for
/// `take_opened_files` — actually loading a file means touching `ComicApp`,
/// which this delegate (being winit's, not ours) has no access to, hence
/// the queue instead of a direct call.
unsafe extern "C-unwind" fn application_open_urls(
    _this: &NSObject,
    _cmd: Sel,
    _application: &NSApplication,
    urls: &NSArray<NSURL>,
) {
    let paths: Vec<PathBuf> =
        urls.iter().filter_map(|url| url.path()).map(|path| PathBuf::from(path.to_string())).collect();
    if paths.is_empty() {
        return;
    }
    opened_files().lock().unwrap().extend(paths);
}
