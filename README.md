# 📖 Comic Reader

A desktop comic reader built with Rust and egui — dual-page spreads, remappable keybindings.

## Features

- Dual-page spreads with LTR (Western) and RTL (Manga) modes
- Isolate a page to display it alone instead of paired with its neighbor (`E`) — your reading mode (LTR/RTL/Single) is untouched; the mark just sticks to that page, even if you navigate away and back
- Fully remappable keybindings, with presets (dual-mode, left-hand, right-hand, numpad)
- Auto-hiding header, fullscreen mode
- Themes (Dark, Light, Midnight, Sepia) and adjustable layout (page spacing, spine width/opacity, fade speed)
- Reads `.cbz`/`.zip`, `.cb7`/`.7z`, `.cbr`/`.rar` — `.jpg`, `.png`, `.webp`, `.gif` pages
- Settings persisted locally at `~/.config/comic-reader/config.json`

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

All keybindings, the theme, and layout can be changed from the in-app Settings panel.
