# 📖 Comic Reader

A desktop comic reader built with Rust and egui — dual-page spreads, remappable keybindings.

## Features

- Dual-page spreads with LTR (Western) and RTL (Manga) modes
- Webtoon mode: every page stacked into one continuous, gapless vertical strip at an adjustable width%, scrolled with the up/down arrow keys — pages preload ahead of where you're reading
- Isolate a page to display it alone instead of paired with its neighbor (`E`) — your reading mode (LTR/RTL/Single) is untouched; the mark just sticks to that page, even if you navigate away and back
- Fully remappable keybindings, with presets (dual-mode, left-hand, right-hand, numpad)
- Auto-hiding header/footer — shown while the cursor is near the top/bottom edge (or off the window), hidden 2s after it isn't; fullscreen mode
- Themes (Dark, Light, Midnight, Sepia) and adjustable layout (page spacing, spine width/opacity, fade speed)
- Reads `.cbz`/`.zip`, `.cb7`/`.7z`, `.cbr`/`.rar` — `.jpg`, `.png`, `.webp`, `.gif` pages
- Reads a `ComicInfo.xml` entry, if the archive has one, and shows its series/issue number (or title) in the header
- Jump straight to a page (`` ` ``) instead of paging through to it
- Drag and drop a comic archive onto the window to open it
- Settings persisted locally at `~/.config/comic-reader/config.json`

## macOS: "Apple could not verify..." warning

The release build is ad-hoc signed, not signed with a paid Apple Developer ID — so the first launch shows Gatekeeper's "Apple could not verify that this app is free of malware" warning. This is a one-time trust prompt, not a sign anything's wrong: right-click (or Control-click) `Comic Reader.app` and choose **Open**, then confirm **Open** again in the dialog. After that first approval, it launches normally like any other app.

(If macOS just refuses outright with no *Open* option, it's under **System Settings → Privacy & Security**, near the bottom of the page, as "Comic Reader was blocked" — click **Open Anyway** there instead.)

## Run

```bash
cargo run --release
```

## Default Keybindings

| Action | Keys | Remappable |
|---|---|---|
| Next spread | `→` / `D` | Yes |
| Previous spread | `←` / `A` | Yes |
| Isolate page (single view) | `E` | Yes |
| Fullscreen | `F` | No |
| Go to page | `` ` `` (backtick) | No |

All keybindings, the theme, and layout can be changed from the in-app Settings panel.
