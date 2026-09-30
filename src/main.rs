#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), eframe::Error> {
    native::main()
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use rusty_comic_reader_lib::app::ComicApp;
    use rusty_comic_reader_lib::{load_logo_rgba, platform};
    use eframe::egui;
    use std::path::PathBuf;

    /// Appends every panic (message, location, and a forced backtrace) to
    /// `crash.log` next to `config.json`, in addition to letting the default
    /// hook still print to stderr as usual. A panic in a GUI app launched
    /// outside a terminal (double-click, `Finder`, a bundled `.app`) leaves no
    /// trace anywhere else — the window just disappears — so this is the only
    /// way to actually diagnose one after the fact.
    fn install_crash_log_hook() {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            default_hook(info);
            let Some(config_dir) = dirs::config_dir() else { return };
            let log_path = config_dir.join("comic-reader").join("crash.log");
            let Some(parent) = log_path.parent() else { return };
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
            let backtrace = std::backtrace::Backtrace::force_capture();
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let entry = format!("\n--- crash at unix time {timestamp} ---\n{info}\n{backtrace}\n");
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
                let _ = file.write_all(entry.as_bytes());
            }
        }));
    }

    /// Command-line arguments that look like a comic archive we can open —
    /// how the OS hands us a file on double-click/"Open With" on Windows and
    /// Linux (`%1`/`%f` in the registry/.desktop entry become plain argv).
    fn opened_files() -> Vec<PathBuf> {
        std::env::args().skip(1).map(PathBuf::from).filter(|path| rusty_comic_reader_lib::comic::loader::is_supported_path(path)).collect()
    }

    pub fn main() -> Result<(), eframe::Error> {
        install_crash_log_hook();
        // Must happen before `eframe::run_native` (and so before winit's
        // `EventLoop::new`) ever runs — see `platform::macos` for why.
        #[cfg(target_os = "macos")]
        platform::macos::install_open_file_handler();
        let (rgba, width, height) = load_logo_rgba();
        let options = eframe::NativeOptions {
            // `app_id` is what Wayland compositors (KWin, GNOME Shell...) use to
            // find a matching .desktop file and pull its `Icon=` — Wayland has no
            // per-window icon protocol, unlike X11's `_NET_WM_ICON` set below.
            viewport: egui::ViewportBuilder::default()
                .with_icon(egui::IconData { rgba, width, height })
                .with_app_id("comic-reader"),
            ..Default::default()
        };

        let files = opened_files();
        eframe::run_native(
            "Comic Reader",
            options,
            Box::new(move |_cc| {
                #[cfg(target_os = "macos")]
                platform::scroll_touch::install();
                let app = if files.is_empty() { ComicApp::new() } else { ComicApp::new_with_files(files) };
                Ok(Box::new(app))
            }),
        )
    }
}
