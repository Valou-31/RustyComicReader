use crate::app::{ComicApp, DOWNSCALE_MAX_DIMENSION, ReadingMode, ZoomTarget};
use crate::input::keybindings::{Action, Preset};
use crate::ui::layout::{LayoutConfig, MIN_MENU_BUTTON_OPACITY};
use crate::ui::theme::ThemePreset;
use egui::{Color32, Context};
use std::time::Duration;

/// How long the spine color picker must sit still before its value is
/// recorded into `LayoutConfig::spine_color_history` — long enough that
/// dragging around the hue/sat sliders doesn't flood the history with every
/// intermediate shade, short enough to feel immediate once you settle on one.
const COLOR_HISTORY_DEBOUNCE: Duration = Duration::from_millis(600);

/// Draws the settings window (reading direction, key remapping, presets)
/// when `app.show_settings` is set. No-op otherwise.
pub fn draw_settings(ctx: &Context, app: &mut ComicApp) {
    if !app.show_settings {
        return;
    }

    if let Some(since) = app.spine_color_pending_since {
        let elapsed = since.elapsed();
        if elapsed >= COLOR_HISTORY_DEBOUNCE {
            app.layout.record_spine_color(app.layout.spine_color);
            app.spine_color_pending_since = None;
            app.save_config();
        } else {
            ctx.request_repaint_after(COLOR_HISTORY_DEBOUNCE - elapsed);
        }
    }

    let mut open = true;
    egui::Window::new("⚙ Settings")
        .open(&mut open)
        .resizable(true)
        .collapsible(false)
        .default_height(520.0)
        .show(ctx, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.heading("Reading Direction");
            ui.horizontal(|ui| {
                if ui.selectable_label(app.reading_mode == ReadingMode::LTR, "➡ LTR (Western)").clicked() {
                    app.set_reading_mode(ReadingMode::LTR);
                }
                if ui.selectable_label(app.reading_mode == ReadingMode::RTL, "⬅ RTL (Manga)").clicked() {
                    app.set_reading_mode(ReadingMode::RTL);
                }
                if ui
                    .selectable_label(app.reading_mode == ReadingMode::Single, "📄 Single Page")
                    .on_hover_text("Shows one page at a time; each turn moves by a single page instead of two.")
                    .clicked()
                {
                    app.set_reading_mode(ReadingMode::Single);
                }
            });

            ui.separator();
            ui.heading("Key Remapping");

            if let Some(action) = app.remapping_action {
                ui.colored_label(
                    Color32::from_rgb(230, 180, 40),
                    format!("Press a key to bind to \"{}\"… (Esc to cancel)", action.label()),
                );
            }

            for action in Action::ALL {
                ui.horizontal(|ui| {
                    ui.label(action.label());

                    let keys = app.keybindings.bindings.get(&action).cloned().unwrap_or_default();
                    for key in keys {
                        if draw_key_chip(ui, &key) {
                            app.keybindings.remove_key(action, &key);
                            app.save_config();
                        }
                    }

                    let remapping_this = app.remapping_action == Some(action);
                    if ui
                        .add_enabled(!remapping_this, egui::Button::new("+ Add Key"))
                        .clicked()
                    {
                        app.remapping_action = Some(action);
                    }
                });
            }
            ui.label("Fullscreen (F) is not remappable.");

            ui.separator();
            ui.heading("Presets");
            ui.horizontal(|ui| {
                for preset in Preset::ALL {
                    if ui.button(preset.label()).clicked() {
                        app.keybindings = preset.bindings();
                        app.remapping_action = None;
                        app.save_config();
                    }
                }
            });

            ui.separator();
            ui.heading("Theme");
            ui.horizontal(|ui| {
                for preset in ThemePreset::ALL {
                    if ui.selectable_label(app.theme_preset == preset, preset.label()).clicked()
                        && app.theme_preset != preset
                    {
                        app.theme_preset = preset;
                        app.save_config();
                    }
                }
            });

            ui.separator();
            ui.heading("Performance");
            if ui
                .checkbox(&mut app.downscale_large_pages, "Downscale large pages to save memory")
                .on_hover_text(format!(
                    "Caps decoded pages at {DOWNSCALE_MAX_DIMENSION}px on the longest side. \
                     Lower RAM/VRAM use, slight loss of sharpness on very high-res scans."
                ))
                .changed()
            {
                app.textures.clear();
                app.save_config();
            }

            ui.separator();
            ui.heading("Trackpad");
            ui.horizontal(|ui| {
                ui.label("Scroll direction");
                if ui.selectable_label(!app.scroll_inverted, "Natural").clicked() && app.scroll_inverted {
                    app.scroll_inverted = false;
                    app.save_config();
                }
                if ui.selectable_label(app.scroll_inverted, "Inverted").clicked() && !app.scroll_inverted {
                    app.scroll_inverted = true;
                    app.save_config();
                }
            });
            ui.horizontal(|ui| {
                ui.label("Swipe sensitivity");
                if ui
                    .add(egui::Slider::new(&mut app.scroll_sensitivity, 0.5..=2.5))
                    .on_hover_text(
                        "How little trackpad movement a full page turn takes. Higher is more \
                         sensitive (less movement needed).",
                    )
                    .changed()
                {
                    app.save_config();
                }
            });
            if ui
                .checkbox(&mut app.one_turn_per_swipe, "One page turn per swipe")
                .on_hover_text(
                    "Cap a single trackpad swipe — including its momentum tail — to one page \
                     turn, so a big or fast swipe can't skip several spreads at once. Turn off \
                     to let a strong swipe chain through more than one.",
                )
                .changed()
            {
                app.save_config();
            }

            ui.separator();
            ui.heading("Progress Bar");
            let mut show_page_preview = app.show_page_preview;
            if ui
                .checkbox(&mut show_page_preview, "Show page preview on hover")
                .on_hover_text(
                    "Preview the page under the cursor when hovering the progress bar. Turning \
                     this off also unloads the whole-book preview cache, freeing the memory it \
                     was using.",
                )
                .changed()
            {
                app.set_show_page_preview(show_page_preview);
            }

            ui.separator();
            ui.heading("Zoom");
            ui.horizontal(|ui| {
                ui.label("Zoom applies to");
                for target in [ZoomTarget::Spread, ZoomTarget::SinglePage] {
                    if ui.selectable_label(app.zoom_target == target, target.label()).clicked()
                        && app.zoom_target != target
                    {
                        app.zoom_target = target;
                        app.save_config();
                    }
                }
            });

            ui.separator();
            ui.heading("Session");
            if ui
                .checkbox(&mut app.resume_last_session, "Reopen the last book on startup")
                .changed()
            {
                app.save_config();
            }
            #[cfg(windows)]
            if ui.button("Set as default comic reader").clicked() {
                if let Err(err) = crate::platform::windows::register_as_default() {
                    tracing::warn!("Failed to register as default app: {err}");
                }
                let _ = std::process::Command::new("cmd")
                    .args(["/C", "start", "ms-settings:defaultapps"])
                    .spawn();
            }

            ui.separator();
            ui.heading("Updates");
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            if ui.checkbox(&mut app.auto_check_updates, "Check for updates on startup").changed() {
                app.save_config();
            }
            ui.horizontal(|ui| {
                let checking = matches!(app.update_status, crate::app::UpdateStatus::Checking);
                if ui.add_enabled(!checking, egui::Button::new("Check now")).clicked() {
                    app.check_for_updates();
                }
                let status = match &app.update_status {
                    crate::app::UpdateStatus::Idle => "Up to date".to_string(),
                    crate::app::UpdateStatus::Checking => "Checking…".to_string(),
                    crate::app::UpdateStatus::Available(info) => format!("v{} available", info.version),
                    crate::app::UpdateStatus::Downloading => "Downloading…".to_string(),
                    crate::app::UpdateStatus::Ready => "Ready — restart to apply".to_string(),
                    crate::app::UpdateStatus::Failed(err) => format!("Failed: {err}"),
                };
                ui.label(status);
            });

            ui.separator();
            ui.heading("Layout");
            let mut layout_changed = false;
            ui.horizontal(|ui| {
                ui.label("Spine width");
                layout_changed |= ui.add(egui::Slider::new(&mut app.layout.spine_width, 0.0..=6.0)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("Spine opacity");
                layout_changed |= ui.add(egui::Slider::new(&mut app.layout.spine_opacity, 0.0..=1.0)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("Spine shadow width");
                layout_changed |=
                    ui.add(egui::Slider::new(&mut app.layout.spine_shadow_width, 0.0..=120.0)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("Spine color");
                if ui.color_edit_button_srgb(&mut app.layout.spine_color).changed() {
                    layout_changed = true;
                    app.spine_color_pending_since = Some(std::time::Instant::now());
                }
                if !app.layout.spine_color_history.is_empty() {
                    ui.label("Recent:");
                    for color in app.layout.spine_color_history.clone() {
                        let swatch = egui::Button::new("")
                            .fill(Color32::from_rgb(color[0], color[1], color[2]))
                            .min_size(egui::vec2(18.0, 18.0));
                        let hex = format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2]);
                        if ui.add(swatch).on_hover_text(hex).clicked() {
                            app.layout.spine_color = color;
                            app.layout.record_spine_color(color);
                            app.spine_color_pending_since = None;
                            app.save_config();
                        }
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Page gap");
                layout_changed |= ui.add(egui::Slider::new(&mut app.layout.page_gap, 0.0..=80.0)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("Fade speed (ms)");
                layout_changed |= ui.add(egui::Slider::new(&mut app.layout.fade_duration_ms, 50..=1000)).changed();
            });
            if ui
                .checkbox(&mut app.layout.header_floats_over_reader, "Float the top bar over the page")
                .on_hover_text(
                    "When shown, the top bar draws over the page instead of shrinking it to make room. \
                     The bottom bar already works this way.",
                )
                .changed()
            {
                layout_changed = true;
            }
            ui.horizontal(|ui| {
                ui.label("Menu background opacity");
                layout_changed |= ui.add(egui::Slider::new(&mut app.layout.menu_bg_opacity, 0.0..=1.0)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("Menu button opacity");
                layout_changed |= ui
                    .add(egui::Slider::new(&mut app.layout.menu_button_opacity, MIN_MENU_BUTTON_OPACITY..=1.0))
                    .on_hover_text("Capped at a minimum so buttons stay legible over the page.")
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label("Page transition speed (ms)");
                layout_changed |=
                    ui.add(egui::Slider::new(&mut app.layout.page_transition_ms, 0..=600)).changed();
            });
            if ui.button("Reset layout to defaults").clicked() {
                app.layout = LayoutConfig::default();
                layout_changed = true;
            }
            if layout_changed {
                app.save_config();
            }

            ui.separator();
            ui.heading("Toolbar");
            ui.label(
                "Customize the header and footer bars — add rows, group controls into left/center/\
                 right blocks, or move controls to a footer bar pinned to the bottom of the window.",
            );
            let book_open = !app.pages.is_empty();
            let edit_toolbar_clicked = ui
                .add_enabled(book_open, egui::Button::new("Edit Toolbar Layout"))
                .on_disabled_hover_text("Open a comic first — the toolbar only shows in the reading view.")
                .clicked();
            if edit_toolbar_clicked {
                app.toolbar_edit_mode = true;
                app.show_settings = false;
            }
            if ui.button("Reset toolbar to default").clicked() {
                app.layout.toolbar_items = LayoutConfig::default_toolbar_items();
                app.save_config();
            }

            ui.separator();
            if ui.button("Close").clicked() {
                app.show_settings = false;
            }
        });
        });

    if !open {
        app.show_settings = false;
        app.remapping_action = None;
    }
}

/// Draws a bound key as a single rounded chip — the key text with a small
/// "×" at the end, both inside one bordered rectangle instead of a separate
/// label + button. Returns `true` if the "×" was clicked (caller removes it).
fn draw_key_chip(ui: &mut egui::Ui, key: &str) -> bool {
    let mut remove = false;
    egui::Frame::NONE
        .fill(ui.visuals().widgets.inactive.weak_bg_fill)
        .stroke(ui.visuals().widgets.inactive.bg_stroke)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(key);
                if ui
                    .add(egui::Button::new(egui::RichText::new("×").size(13.0)).small().frame(false))
                    .clicked()
                {
                    remove = true;
                }
            });
        });
    remove
}
