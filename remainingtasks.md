# Remaining Tasks

Feature checklist derived from `README.md`, checked against the actual code in `src/` (not the README's own claims). `[x]` = actually wired up and reachable from the running app. `[ ]` = missing, or present in code but never connected to any UI/input path (dead code the compiler itself flags as unused).

## 📖 Reading Experience

- [x] Dual-Page Display — `ui/reader.rs::draw_double_page`, renders left/right pages as real GPU textures
- [x] Customizable Reading Mode (LTR/RTL) — `ComicApp::toggle_reading_mode` + a button (shown both before and after loading a comic) that flips `reading_mode` and resets `page_offset`
  - [x] Instant mode switching with visual feedback — egui re-renders every frame, so the page swap and the button's own label (`➡ LTR (Western)` / `⬅ RTL (Manga)`) update in the same frame the button is clicked
- [x] Smart Page Navigation (spreads advance by 2 pages) — `next_spread`/`prev_spread`
- [x] Page Offset System (peek with `Q`/`E`) — works, but only peeks *forward*; `page_offset` is `usize` and clamped at 0, so `Q` can't peek before the current spread the way the README's "peek at adjacent pages" implies

## 📦 Format Support

- [x] Archive Formats: `.cbz`, `.cb7`, `.cbr`, `.zip`, `.7z`, `.rar` — all three decoders implemented in `comic/archive.rs`, verified against real 192-page archives in all three formats
- [x] Image Formats: `.jpg`, `.jpeg`, `.png`, `.webp`, `.gif` — decoded via the `image` crate (webp decoder confirmed present in `Cargo.lock`); only `.jpg` exercised in testing so far
- [x] Smart Detection — `is_image_file` filter + `alphanumeric_sort::compare_str` ordering

## ⌨️ Advanced Controls

- [x] Fully Remappable Keybindings — `ui/settings.rs` (new) is a real settings window: "+ Add Key" sets `remapping_action`, the next physical key press (captured in `keyboard.rs`) binds it and exits remap mode automatically, `Escape` cancels a pending remap without binding anything. Removed the redundant `RemappingAction` enum (`Option<Action>` directly) and fixed `KeyBindings::add_key` to dedupe case-insensitively (pressing the same key twice for one action no longer creates duplicate chips in the UI).
  - [x] Multiple keys per action — `HashMap<Action, Vec<String>>`, each shown as a removable chip with a "✕" button
  - [x] Configuration panel — `ui/settings.rs::draw_settings`, opened via a "⚙ Settings" button in the header and on the empty-state screen
  - [x] Persisted settings — see Persistence section below
  - [x] Recommended presets (left-hand, right-hand, dual-mode, numpad) — `input/keybindings.rs::Preset`, one-click buttons in the settings panel; see that file for the exact key choices (numpad presets use `Num4/6/7/9` since egui doesn't distinguish numpad digits from the top row)
- [x] Default Keybindings (arrows/WASD, `E`/`Q`) — match the README's table exactly
- [x] Fullscreen key non-remappable — `F` is hardcoded in `keyboard.rs`, separate from the remap system (see Interface section for whether it actually does anything)

## 🖥️ Interface

- [x] Fullscreen Mode — `F` toggles `app.fullscreen` and sets `fullscreen_dirty`; `main.rs` sends `ViewportCommand::Fullscreen` when that flag is set, same dirty-flag pattern already used for the window title
- [x] Auto-Hide UI — `ComicApp::idle_time()` (derived from `last_mouse_move`, no more redundant `ui_hidden` bool) drives `Context::animate_bool_with_time` in the new `ui/header.rs` for a real fade; header is skipped entirely (not just transparent) once fully hidden so it stops intercepting clicks; reappears the instant `pointer.delta() != 0`; repaints are scheduled only around the hide/fade window (`request_repaint_after`), not continuously, to avoid burning CPU while idle. Delay (3s) and fade duration (300ms) are named constants (`UI_HIDE_DELAY`/`UI_FADE_DURATION`) rather than a settings-panel-exposed value, since the settings panel itself still doesn't exist (see Advanced Controls).
- [x] Responsive Header — now a real component, `ui/header.rs::draw_header`, showing filename, live page numbers, and controls, wrapped in the fade above
- [x] Visual Indicators — header now shows real page numbers (e.g. "Page 5-6 / 192", verified correct for both LTR/RTL and the last-odd-page case) plus a "👁 peek +N" badge whenever `page_offset > 0`

## 🎨 Customization

- [x] Theme System — `ui/theme.rs::Theme` + `ThemePreset` (Dark/Light/Midnight/Sepia), applied every frame via `Theme::apply()` in `main.rs:50` and `clear_color()`; picker lives in the settings panel. Visually verified by launching the app and pixel-sampling screenshots across all four presets — window/panel fill colors match the theme definitions exactly.
- [x] Configurable Layout (separator width/opacity, spacing, transition speed) — settings panel exposes spine width/opacity, page gap, and fade speed sliders wired into `reader.rs`, with a "Reset layout to defaults" button

## 💾 Persistence

- [x] Auto-Save Configuration (reading mode / keybindings) — `Config` now uses real typed fields (`ReadingMode`, `KeyBindings`) instead of `String`/`serde_json::Value`; `ComicApp::new()` loads it at startup (used from `main()` instead of `default()`) and `save_config()` writes it back on every reading-mode toggle and every keybinding change, verified with an actual save→reload round trip against `~/.config/comic-reader/config.json`. "Layout preferences" specifically aren't covered — that config doesn't exist yet (see Customization: Configurable Layout, still not done).
- [x] 100% Local & Private — trivially true, no network code exists in the app
- [x] Reading History — `storage/history.rs::History`, persisted to `history.json` alongside `config.json`; a "🕘 History" window (`ui/history.rs`) lists every book with its saved page, reopens on click, and lets an entry be removed. Updated on load, debounced (5s) during navigation, and flushed unconditionally on `on_exit`. Verified end-to-end: opened two real archives, navigated, quit, and confirmed the on-disk JSON matched.
- [x] Resume Previous Session — `ComicApp::new()` reopens the most-recently-read book (if `resume_last_session` is on, toggle in Settings → Session, default on) at its saved page. Verified by quitting mid-book and relaunching with no arguments: reopened the same archive at the exact saved spread.
- [x] Multi-File Queue — the file picker is multi-select (`rfd::pick_files`); every pick after the first goes into `ComicApp::file_queue`, and a "▶ Next (N)" button appears in the header to advance to it. Verified by launching with two archive paths and clicking through.

## 🔗 System Integration

- [x] Default app for `.cbz`/`.cb7`/`.cbr` — deliberately **not** `.zip`/`.7z`/`.rar`, which stay pointed at whatever the user already has for generic archives.
  - CLI-arg opening (`main.rs::opened_files`) — verified: `./rusty_comic_reader some.cbz` opens directly into that book.
  - Linux: `.desktop` `MimeType=`, a shared-mime-info package (`packaging/linux/comic-reader-mime.xml`) since these aren't standard system MIME types, and `install.sh` now runs `xdg-mime default` — not runnable on this dev machine, so unverified in practice.
  - Windows: `platform::windows::register_as_default` (behind a Settings button) writes the `HKCU\Software\RegisteredApplications` capability pattern Microsoft requires since Win8 blocked silent `UserChoice` writes, then opens `ms-settings:defaultapps`. Code checked against the real `winreg` 0.56 API but not runnable on this dev machine — unverified in practice.
  - macOS: **known gap.** `Info.plist` now declares the UTIs so Finder's "Open With" lists Comic Reader at all, but double-click/"Open With" launches via an Apple Event (`kAEOpenDocuments`) that this winit-based app doesn't handle — confirmed empirically by building the real `.app` bundle and running `open -a "Comic Reader.app" file.cbz`: the app launched to the empty-state screen, no file received. `open` from a Terminal already inside the bundle (`./Comic\ Reader.app/Contents/MacOS/rusty_comic_reader file.cbz`) works fine, same as any CLI launch. Closing this gap needs an `objc2`/`objc2-app-kit` Apple Event handler — not implemented.

## Implemented but not in the README

These landed during this session and aren't reflected in the README's feature list yet:

- [x] Native "Load File" picker (`rfd`), decoding on a background thread so the UI never blocks
- [x] Loading progress bar (`loaded / total` pages) with a live preview of the first decoded page
- [x] Window title updates to the loaded filename

## Also worth knowing (not features, but affects anyone acting on this file)

- README's Installation/Dependencies sections are stale — they list `egui = "0.27"`, `zip = "0.6"`, `dirs = "5.0"`, etc.; `Cargo.toml` actually pins `egui 0.36.1`, `zip 8.6.0`, `dirs 7.0.0`, and adds `rfd`/`pollster` that the README doesn't mention at all.
- README's Architecture tree lists `state/navigation.rs` and `state/spread.rs` as implemented; both files are still empty — navigation logic actually lives in `app.rs`.
- No `LICENSE` file exists despite the README linking to one.

## Summary

| Section | Done | Remaining |
|---|---|---|
| Reading Experience | 5 / 5 | 0 |
| Format Support | 3 / 3 | 0 |
| Advanced Controls | 6 / 6 | 0 |
| Interface | 4 / 4 | 0 |
| Customization | 2 / 2 | 0 |
| Persistence | 2 / 2 | 0 |
