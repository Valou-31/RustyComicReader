# 📖 Comic Reader - Desktop Edition

A modern, high-performance desktop comic reader built with **Rust** and **egui**, featuring dual-page display, customizable keybindings, and immersive reading experience.

![License](https://img.shields.io/badge/license-MIT-blue)
![Rust](https://img.shields.io/badge/rust-1.70%2B-orange)
![egui](https://img.shields.io/badge/egui-0.27-green)

---

## 🎯 Features

### 📖 Reading Experience
- **Dual-Page Display** - Books open naturally with left and right pages side-by-side
- **Customizable Reading Mode** 
  - LTR (Left-to-Right) - Western comics, Franco-Belgian BD
  - RTL (Right-to-Left) - Manga, Arabic comics
  - Instant mode switching with visual feedback
- **Smart Page Navigation** - Spreads advance by 2 pages for authentic book feel
- **Page Offset System** - Peek at next pages with `Q`/`E` keys without advancing progress

### 📦 Format Support
- **Archive Formats**: `.cbz`, `.cb7`, `.cbr`, `.zip`, `.7z`, `.rar`
- **Image Formats**: `.jpg`, `.jpeg`, `.png`, `.webp`, `.gif`
- **Smart Detection** - Auto-detects and sorts images alphanumerically

### ⌨️ Advanced Controls
- **Fully Remappable Keybindings**
  - Multiple keys per action (QWAD OR arrow keys)
  - Intuitive configuration panel
  - Persisted settings
  - Recommended presets included (left-hand, right-hand, dual-mode, numpad)

- **Default Keybindings**

Next Spread : Arrow Right → or D
Prev Spread : Arrow Left ← or A
Shift Right : E
Shift Left : Q
Fullscreen : F (non-remappable)


### 🖥️ Interface
- **Fullscreen Mode** - Maximum immersion with `F` key
- **Auto-Hide UI** - Interface disappears after 3 seconds of inactivity
  - Smooth fade animations
  - Configurable delay
  - Automatic reappear on mouse movement
- **Responsive Header** - Current spread info, filename, control buttons
- **Visual Indicators** - Page offset badges, real-time page numbers

### 🎨 Customization
- **Theme System** - Easy color/style modifications
- **Configurable Layout**
  - Adjust separator width and opacity
  - Modify spacing between pages
  - Customize transition speeds

### 💾 Persistence
- **Auto-Save Configuration**
  - Reading mode preference (LTR/RTL)
  - Custom keybindings
  - Layout preferences
- **100% Local & Private** - No server required

---

## 🚀 Installation

### Prerequisites
- Rust 1.70+ ([Install Rust](https://rustup.rs/))
- System libraries:
```bash
  # Ubuntu/Debian
  sudo apt-get install libssl-dev pkg-config

  # macOS
  brew install openssl pkg-config

  # Fedora
  sudo dnf install openssl-devel
```

### Build from Source

```bash
# Clone the repository
git clone https://github.com/yourusername/comic-reader.git
cd comic-reader

# Run (builds automatically)
cargo run

# Or build optimized release
cargo build --release
./target/release/comic-reader
```

---

## 📖 Usage

### Quick Start
1. **Launch the app**
```bash
   cargo run
```

2. **Load a Comic**
   - Click "Load File"
   - Select a `.cbz`, `.rar`, or `.7z` file
   - Images auto-extract and display

3. **Navigate**
   - Press `→` (or `D`) to read forward
   - Press `←` (or `A`) to go back
   - Use `E`/`Q` to peek at adjacent pages

### Keybinding Configuration

1. Open Settings (`⚙️` button)
2. Go to "Key Remapping"
3. Click "+ Add Key" for desired action
4. Press your preferred key
5. Repeat for additional keys per action
6. Press `Escape` when done

### Reading Modes

**Western Comics (LTR)**

┌─────┬─────┐
│ L │ R │ Press → to advance
└─────┴─────┘


**Manga (RTL)**

┌─────┬─────┐
│ R │ L │ Press → to advance
└─────┴─────┘


Switch via Settings → Reading Direction

---

## 🏗️ Architecture

src/
├── main.rs # Entry point & app initialization
├── app.rs # Core ComicApp state management
├── input/
│ ├── keybindings.rs # Keybinding action definitions
│ └── keyboard.rs # Keyboard event handling
├── ui/
│ ├── theme.rs # Color and style theming
│ ├── reader.rs # Double-page rendering
│ ├── header.rs # Top bar UI
│ ├── controls.rs # Navigation buttons
│ └── settings.rs # Settings panel
├── comic/
│ ├── archive.rs # ZIP/RAR/7Z extraction
│ └── loader.rs # Image management
├── storage/
│ └── config.rs # Config persistence
└── state/
├── navigation.rs # Spread navigation
└── spread.rs # Spread calculation


### Key Components

- **ComicApp** - Central state holder with navigation, pages, and UI state
- **KeyBindings** - Action-to-key mapping with HashMap support
- **ComicArchive** - Archive handling for ZIP, RAR, 7Z formats

---

## ⚙️ Dependencies

```toml
egui = "0.27"                   # UI Framework
eframe = "0.27"                 # Window management
image = "0.25"                  # Image processing
zip = "0.6"                     # ZIP extraction
sevenz-rust = "0.5"             # 7Z extraction
unrar = "0.5"                   # RAR extraction
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"              # JSON serialization
tokio = { version = "1", features = ["full"] }
dirs = "5.0"                    # Config directory
anyhow = "1.0"                  # Error handling
alphanumeric-sort = "1.0"       # Smart sorting
```

---

## 🛠️ Development

### Building

```bash
# Debug build (faster compile)
cargo run

# Release build (optimized)
cargo run --release

# Check without compiling
cargo check

# Format code
cargo fmt

# Lint
cargo clippy
```

### Adding New Features

**New Keybinding Action**
```rust
// src/input/keybindings.rs
pub enum Action {
    NextSpread,
    PrevSpread,
    ShiftRight,
    ShiftLeft,
    YourNewAction,  // Add here
}
```

**New Reading Mode**
```rust
// src/app.rs
pub enum ReadingMode {
    LTR,
    RTL,
    YourMode,  // Add here
}
```

---

## 🐛 Troubleshooting

### Won't Compile
```bash
cargo clean
cargo build
cargo update
```

### Missing Native Libraries
```bash
# Ubuntu/Debian
sudo apt-get install libssl-dev pkg-config

# macOS
brew install openssl pkg-config
```

### Compilation Time
- First build: 3-5 minutes
- Subsequent: 10-30 seconds (debug mode)
- Use `--release` only for distribution

---

## 🎮 Keyboard Shortcuts

| Action | Default Keys | Remappable |
|--------|-------------|-----------|
| Next Spread | `→` / `D` | ✅ Yes |
| Previous Spread | `←` / `A` | ✅ Yes |
| Shift Right | `E` | ✅ Yes |
| Shift Left | `Q` | ✅ Yes |
| Fullscreen | `F` | ❌ No |

---

## 📊 Performance

| Metric | Value |
|--------|-------|
| Archive Load Time | 100-500ms |
| Navigation Latency | <5ms |
| UI Transition | 300ms smooth fade |
| Memory | Minimal |

---

## 🎯 Roadmap

### v2.0 (Current)
- ✅ Dual-page display
- ✅ LTR/RTL modes
- ✅ Remappable keybindings
- ✅ Full archive support
- ✅ Auto-hide UI

### v2.1+
- 🔲 Custom zoom
- 🔲 Image rotation
- 🔲 Bookmarks
- 🔲 Reading history
- 🔲 Thumbnail gallery

---

## 📄 License

MIT License - see [LICENSE](LICENSE) for details.

---

## 🤝 Contributing

1. Fork the repository
2. Create feature branch (`git checkout -b feat/your-feature`)
3. Commit (`git commit -m "feat: description"`)
4. Push (`git push origin feat/your-feature`)
5. Open a Pull Request

---

## 🙏 Acknowledgments

- [egui](https://github.com/emilk/egui) - GUI Framework
- [image-rs](https://github.com/image-rs/image) - Image Processing
- [zip-rs](https://github.com/zip-rs/zip) - ZIP Handling
- [serde](https://serde.rs/) - Serialization

---

**Comic Reader v2.0 - Modern Desktop Comic Reading Experience** 📖✨
