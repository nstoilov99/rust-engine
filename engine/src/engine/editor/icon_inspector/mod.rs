//! Dev-only Icon Inspector window — gated behind `editor-debug` feature.
//!
//! Opens as a secondary OS window. Lists every editor icon, lets you tweak
//! palette colors live, and verifies tinting / category-color decisions.
//!
//! **No silent persistence.** Closing or restarting reverts to `IconPalette::default_dark()`.
//! Use the **Export Palette** button to copy a Rust snippet to the clipboard.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, Context, RichText, TextureHandle, Ui};

use crate::engine::editor::icon_classes::{ChromeState, Severity, TintMode, TypeCategory};
use crate::engine::editor::widgets::{load_icon_textures, IconKind, IconRegistry};

/// Persistent state for the Icon Inspector window.
pub struct IconInspectorWindow {
    pub open: bool,
    /// Display size for icons in the grid (px).
    icon_size: f32,
    /// Current chrome state for chrome icon preview.
    chrome_state: ChromeState,
    /// Currently selected icon for the detail panel.
    selected_icon: Option<IconKind>,
    /// Icon textures loaded into the secondary window's egui context.
    /// `IconRegistry`'s textures are bound to the main window's context,
    /// so the secondary renderer cannot sample them — we keep a local copy
    /// bound to whichever context we render into.
    local_textures: HashMap<IconKind, TextureHandle>,
    /// Marker identifying which egui context `local_textures` were loaded for.
    /// When this differs from the current context's marker, textures are reloaded
    /// (e.g. the inspector was closed and a fresh secondary window was created).
    last_ctx_marker: Option<u64>,
    /// Transient feedback for the most recent Save action.
    save_status: Option<SaveStatus>,
}

#[derive(Clone, Copy)]
enum SaveStatus {
    Saved,
    Failed,
}

impl Default for IconInspectorWindow {
    fn default() -> Self {
        Self {
            open: false,
            icon_size: 24.0,
            chrome_state: ChromeState::Default,
            selected_icon: None,
            local_textures: HashMap::new(),
            last_ctx_marker: None,
            save_status: None,
        }
    }
}

impl IconInspectorWindow {
    /// Render the Icon Inspector UI.
    ///
    /// Called from the secondary OS window render path. Renders directly into
    /// a `CentralPanel` — the secondary window provides the OS chrome.
    ///
    /// Accepts `&mut Arc<IconRegistry>` and uses `Arc::get_mut` internally.
    /// If exclusive access cannot be obtained, shows a read-only fallback.
    pub fn show(&mut self, ctx: &Context, icons_arc: &mut Arc<IconRegistry>) {
        if !self.open {
            return;
        }

        // Ensure icon textures live in the current egui context. Each secondary
        // window has its own context with its own renderer texture cache; the
        // shared IconRegistry's handles are valid only in the main window.
        self.ensure_local_textures(ctx);

        // Try to get exclusive mutable access to the registry.
        if let Some(registry) = Arc::get_mut(icons_arc) {
            egui::CentralPanel::default().show(ctx, |ui| {
                self.show_toolbar(ui, registry);
                ui.separator();

                // Two-column layout: icon browser (left) + detail panel (right)
                let detail_width = if self.selected_icon.is_some() {
                    240.0
                } else {
                    0.0
                };

                ui.horizontal_top(|ui| {
                    // Left: scrollable icon browser
                    let browser_width = (ui.available_width() - detail_width).max(200.0);
                    ui.vertical(|ui| {
                        ui.set_width(browser_width);
                        egui::ScrollArea::vertical()
                            .id_salt("icon_browser_scroll")
                            .show(ui, |ui| {
                                self.show_chrome_section(ui, registry);
                                ui.add_space(8.0);
                                self.show_typed_section(ui, registry);
                                ui.add_space(8.0);
                                self.show_severity_section(ui, registry);
                            });
                    });

                    // Right: detail panel for selected icon
                    if let Some(selected) = self.selected_icon {
                        ui.separator();
                        ui.vertical(|ui| {
                            ui.set_width(detail_width);
                            egui::ScrollArea::vertical()
                                .id_salt("icon_detail_scroll")
                                .show(ui, |ui| {
                                    self.show_detail_panel(ui, selected, registry);
                                });
                        });
                    }
                });
            });
        } else {
            // Could not get exclusive access — show a message.
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label("Icon Inspector: waiting for exclusive registry access...");
                });
            });
        }
    }

    /// Tag the current egui context with a unique marker (or read its existing
    /// marker), and reload local textures whenever the marker changes. This
    /// detects the "secondary window closed and recreated" case, where a fresh
    /// context replaces the previous one and our cached texture handles become
    /// stale.
    fn ensure_local_textures(&mut self, ctx: &Context) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);

        let marker_id = egui::Id::new("__icon_inspector_ctx_marker");
        let ctx_marker = ctx.data_mut(|d| {
            if let Some(existing) = d.get_temp::<u64>(marker_id) {
                existing
            } else {
                let new_id = COUNTER.fetch_add(1, Ordering::Relaxed);
                d.insert_temp(marker_id, new_id);
                new_id
            }
        });

        if self.last_ctx_marker != Some(ctx_marker) {
            self.local_textures = load_icon_textures(ctx);
            self.last_ctx_marker = Some(ctx_marker);
        }
    }

    fn show_toolbar(&mut self, ui: &mut Ui, registry: &mut IconRegistry) {
        ui.horizontal(|ui| {
            // Save persists the current palette to `editor_icon_palette.ron`
            // (workspace root). It's auto-loaded on next launch.
            let save_resp = ui
                .button("Save Palette")
                .on_hover_text("Persist current colors to editor_icon_palette.ron");
            if save_resp.clicked() {
                match registry.palette().save_to_default() {
                    Ok(()) => {
                        let path = crate::engine::editor::icon_classes::IconPalette::default_path();
                        log::info!("Saved icon palette to {}", path.display());
                        self.save_status = Some(SaveStatus::Saved);
                    }
                    Err(e) => {
                        log::error!("Failed to save icon palette: {e}");
                        self.save_status = Some(SaveStatus::Failed);
                    }
                }
            }

            if ui.button("Reset Palette").clicked() {
                registry.reset_palette();
                self.selected_icon = None;
                self.save_status = None;
            }

            if ui
                .button("Export Snippet")
                .on_hover_text("Copy a Rust snippet to clipboard for IconPalette::default_dark()")
                .clicked()
            {
                let snippet = export_palette_snippet(registry);
                match arboard::Clipboard::new() {
                    Ok(mut clipboard) => {
                        if let Err(e) = clipboard.set_text(&snippet) {
                            log::error!("Failed to copy to clipboard: {e}");
                        } else {
                            log::info!("Palette snippet copied to clipboard");
                        }
                    }
                    Err(e) => {
                        log::error!("Clipboard unavailable: {e}");
                        log::info!("--- Export Palette (copy from log) ---\n{snippet}");
                    }
                }
            }

            // Inline status pill so the user gets feedback without opening logs.
            if let Some(status) = self.save_status {
                ui.separator();
                let (text, color) = match status {
                    SaveStatus::Saved => ("Saved \u{2713}", Color32::from_rgb(0x6E, 0xCB, 0x7C)),
                    SaveStatus::Failed => ("Save failed", Color32::from_rgb(0xE5, 0x6B, 0x6B)),
                };
                ui.colored_label(color, text);
            }

            ui.separator();
            ui.label("Size:");
            ui.add(
                egui::Slider::new(&mut self.icon_size, 12.0..=64.0)
                    .step_by(4.0)
                    .suffix("px"),
            );
        });
    }

    fn show_chrome_section(&mut self, ui: &mut Ui, registry: &mut IconRegistry) {
        egui::CollapsingHeader::new(
            RichText::new(format!("Chrome ({})", IconKind::chrome_icons().count())).strong(),
        )
        .default_open(true)
        .show(ui, |ui| {
            // Chrome state selector
            ui.horizontal(|ui| {
                ui.label("State:");
                for &state in ChromeState::ALL {
                    if ui
                        .selectable_label(self.chrome_state == state, state.display_name())
                        .clicked()
                    {
                        self.chrome_state = state;
                    }
                }
            });

            // Color pickers for each chrome state
            ui.add_space(4.0);
            egui::Grid::new("chrome_colors")
                .num_columns(2)
                .spacing([8.0, 2.0])
                .show(ui, |ui| {
                    for &state in ChromeState::ALL {
                        let mut color = registry
                            .palette()
                            .chrome
                            .get(&state)
                            .copied()
                            .unwrap_or(Color32::WHITE);
                        ui.label(format!("{}:", state.display_name()));
                        if ui.color_edit_button_srgba(&mut color).changed() {
                            registry.set_chrome_color(state, color);
                        }
                        ui.end_row();
                    }
                });

            // Icon grid
            ui.add_space(8.0);
            self.icon_grid(ui, registry, IconKind::chrome_icons().collect(), self.chrome_state);
        });
    }

    fn show_typed_section(&mut self, ui: &mut Ui, registry: &mut IconRegistry) {
        egui::CollapsingHeader::new(
            RichText::new(format!("Typed ({})", IconKind::typed_icons().count())).strong(),
        )
        .default_open(true)
        .show(ui, |ui| {
            // Category color pickers in a grid
            ui.label(RichText::new("Category Palette:").strong());
            egui::Grid::new("category_colors")
                .num_columns(2)
                .spacing([8.0, 2.0])
                .show(ui, |ui| {
                    for &cat in TypeCategory::ALL {
                        let mut color = registry
                            .palette()
                            .category
                            .get(&cat)
                            .copied()
                            .unwrap_or(Color32::WHITE);
                        ui.label(format!("{}:", cat.display_name()));
                        if ui.color_edit_button_srgba(&mut color).changed() {
                            registry.set_category_color(cat, color);
                        }
                        ui.end_row();
                    }
                });

            // Icons grouped by category
            ui.add_space(8.0);
            for &cat in TypeCategory::ALL {
                let icons: Vec<_> = IconKind::icons_in_category(cat).collect();
                if icons.is_empty() {
                    continue;
                }
                ui.horizontal(|ui| {
                    let cat_color = registry
                        .palette()
                        .category
                        .get(&cat)
                        .copied()
                        .unwrap_or(Color32::WHITE);
                    let (swatch_rect, _) =
                        ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    ui.painter().rect_filled(swatch_rect, 2.0, cat_color);
                    ui.label(RichText::new(cat.display_name()).strong());
                });
                self.icon_grid(ui, registry, icons, ChromeState::Default);
                ui.add_space(4.0);
            }
        });
    }

    fn show_severity_section(&mut self, ui: &mut Ui, registry: &mut IconRegistry) {
        egui::CollapsingHeader::new(
            RichText::new(format!("Severity ({})", IconKind::severity_icons().count())).strong(),
        )
        .default_open(true)
        .show(ui, |ui| {
            // Severity color pickers in a grid
            egui::Grid::new("severity_colors")
                .num_columns(2)
                .spacing([8.0, 2.0])
                .show(ui, |ui| {
                    for &sev in Severity::ALL {
                        let mut color = registry
                            .palette()
                            .severity
                            .get(&sev)
                            .copied()
                            .unwrap_or(Color32::WHITE);
                        ui.label(format!("{}:", sev.display_name()));
                        if ui.color_edit_button_srgba(&mut color).changed() {
                            registry.set_severity_color(sev, color);
                        }
                        ui.end_row();
                    }
                });

            // Icon grid
            ui.add_space(8.0);
            self.icon_grid(
                ui,
                registry,
                IconKind::severity_icons().collect(),
                ChromeState::Default,
            );
        });
    }

    /// Render a wrapping grid of icons. Clicking selects for the detail panel.
    fn icon_grid(
        &mut self,
        ui: &mut Ui,
        registry: &IconRegistry,
        icons: Vec<IconKind>,
        state: ChromeState,
    ) {
        let cell_size = self.icon_size + 8.0; // icon + padding
        let spacing = 4.0;

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(spacing, spacing);

            for &kind in &icons {
                let is_selected = self.selected_icon == Some(kind);
                let has_override = registry.icon_has_any_override(kind);
                let is_authored = registry.tint_mode(kind) == TintMode::Authored;
                let tint = registry.tint(kind, state);

                // Each icon is a clickable cell
                let total_height = cell_size + 14.0; // icon + label below
                let cell_width = cell_size.max(48.0);
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(cell_width, total_height), egui::Sense::click());

                if response.clicked() {
                    self.selected_icon = if is_selected { None } else { Some(kind) };
                }

                if ui.is_rect_visible(rect) {
                    let painter = ui.painter();

                    // Selection highlight
                    if is_selected {
                        painter.rect_filled(
                            rect.expand(1.0),
                            4.0,
                            ui.visuals().selection.bg_fill,
                        );
                    } else if response.hovered() {
                        painter.rect_filled(
                            rect.expand(1.0),
                            4.0,
                            ui.visuals().widgets.hovered.bg_fill,
                        );
                    }

                    // Icon centered in upper portion
                    let icon_center = egui::pos2(
                        rect.center().x,
                        rect.top() + 4.0 + self.icon_size / 2.0,
                    );
                    let icon_rect = egui::Rect::from_center_size(
                        icon_center,
                        egui::vec2(self.icon_size, self.icon_size),
                    );

                    let draw_tint = match registry.tint_mode(kind) {
                        TintMode::Tinted => tint,
                        TintMode::Authored => Color32::WHITE,
                    };

                    if let Some(texture) = self.local_textures.get(&kind) {
                        painter.image(
                            texture.id(),
                            icon_rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            draw_tint,
                        );
                    } else {
                        let text = IconRegistry::fallback_text(kind);
                        let font_size = (self.icon_size * 0.6).max(8.0);
                        painter.text(
                            icon_center,
                            egui::Align2::CENTER_CENTER,
                            text,
                            egui::FontId::proportional(font_size),
                            draw_tint,
                        );
                    }

                    // Label below icon
                    let label_pos = egui::pos2(rect.center().x, rect.bottom() - 10.0);
                    let mut label = kind.display_name().to_string();
                    if has_override {
                        label.push_str(" \u{25CF}"); // dot
                    }
                    if is_authored {
                        label.push_str(" A");
                    }

                    let label_color = if is_selected {
                        ui.visuals().selection.stroke.color
                    } else {
                        ui.visuals().text_color()
                    };
                    painter.text(
                        label_pos,
                        egui::Align2::CENTER_CENTER,
                        &label,
                        egui::FontId::proportional(9.0),
                        label_color,
                    );
                }

                // Tooltip on hover
                if response.hovered() {
                    response.on_hover_text(kind.display_name());
                }
            }
        });
    }

    /// Show the per-icon detail panel for the selected icon (right column).
    fn show_detail_panel(&mut self, ui: &mut Ui, kind: IconKind, registry: &mut IconRegistry) {
        // Header with close button
        ui.horizontal(|ui| {
            ui.heading(RichText::new(kind.display_name()).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("\u{2715}").clicked() {
                    self.selected_icon = None;
                }
            });
        });

        ui.separator();

        // Large preview
        let preview_size = 64.0;
        let tint = registry.tint(kind, self.chrome_state);
        let (preview_rect, _) =
            ui.allocate_exact_size(egui::vec2(preview_size, preview_size), egui::Sense::hover());
        if ui.is_rect_visible(preview_rect) {
            let draw_tint = match registry.tint_mode(kind) {
                TintMode::Tinted => tint,
                TintMode::Authored => Color32::WHITE,
            };
            // Background for preview
            ui.painter().rect_filled(
                preview_rect,
                4.0,
                Color32::from_gray(30),
            );
            if let Some(texture) = self.local_textures.get(&kind) {
                ui.painter().image(
                    texture.id(),
                    preview_rect.shrink(4.0),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    draw_tint,
                );
            } else {
                let text = IconRegistry::fallback_text(kind);
                ui.painter().text(
                    preview_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    egui::FontId::proportional(32.0),
                    draw_tint,
                );
            }
        }

        ui.add_space(8.0);

        // Classification info
        ui.label(format!("Class: {:?}", kind.class()));
        if let Some(cat) = kind.category() {
            ui.label(format!("Category: {}", cat.display_name()));
        }
        if let Some(sev) = kind.severity() {
            ui.label(format!("Severity: {}", sev.display_name()));
        }

        ui.add_space(8.0);
        ui.separator();

        // Tint mode control
        ui.label(RichText::new("Tint Mode").strong());
        let current_mode = registry.tint_mode(kind);
        let has_mode_override = registry.palette().tint_modes.contains_key(&kind);

        let default_label = format!("Default ({:?})", kind.default_tint_mode());
        if ui
            .selectable_label(!has_mode_override, &default_label)
            .clicked()
        {
            registry.clear_icon_tint_mode(kind);
        }
        if ui
            .selectable_label(
                has_mode_override && current_mode == TintMode::Tinted,
                "Tinted",
            )
            .clicked()
        {
            registry.set_icon_tint_mode(kind, TintMode::Tinted);
        }
        if ui
            .selectable_label(
                has_mode_override && current_mode == TintMode::Authored,
                "Authored",
            )
            .clicked()
        {
            registry.set_icon_tint_mode(kind, TintMode::Authored);
        }

        ui.add_space(8.0);
        ui.separator();

        // Per-state tint overrides
        if registry.tint_mode(kind) == TintMode::Tinted {
            ui.label(RichText::new("Per-State Overrides").strong());
            ui.label(
                RichText::new(
                    "Set any state explicitly. Unset states fall back to the \
                     Default override, then to the class color.",
                )
                .weak()
                .small(),
            );

            ui.add_space(4.0);

            egui::Grid::new("per_state_overrides")
                .num_columns(3)
                .spacing([6.0, 4.0])
                .show(ui, |ui| {
                    for &state in ChromeState::ALL {
                        let has_override = registry.icon_override(kind, state).is_some();
                        let effective = registry.tint(kind, state);
                        let mut display_color = registry
                            .icon_override(kind, state)
                            .unwrap_or(effective);

                        ui.label(state.display_name());

                        // Editing the swatch upserts the override for this state.
                        let resp = ui.color_edit_button_srgba(&mut display_color);
                        if resp.changed() {
                            registry.set_icon_override(kind, state, display_color);
                        }

                        // Status / clear column.
                        if has_override {
                            if ui
                                .small_button("\u{2715}")
                                .on_hover_text("Clear this state's override")
                                .clicked()
                            {
                                registry.clear_icon_override(kind, state);
                            }
                        } else {
                            ui.label(
                                RichText::new("inherits")
                                    .weak()
                                    .small()
                                    .italics(),
                            );
                        }

                        ui.end_row();
                    }
                });

            ui.add_space(6.0);
            if registry.icon_has_any_override(kind) && ui.button("Clear All Overrides").clicked() {
                registry.clear_all_icon_overrides(kind);
            }
        } else {
            ui.label(
                RichText::new("Drawn as authored \u{2014} no tint applied.")
                    .weak()
                    .italics(),
            );
        }
    }
}

/// Generate a Rust snippet for `IconPalette::default_dark()` body from the current palette.
fn export_palette_snippet(registry: &IconRegistry) -> String {
    let palette = registry.palette();
    let mut s = String::with_capacity(2048);

    s.push_str("pub fn default_dark() -> Self {\n");

    // Categories
    s.push_str("    let mut category = HashMap::new();\n");
    for &cat in TypeCategory::ALL {
        if let Some(&color) = palette.category.get(&cat) {
            s.push_str(&format!(
                "    category.insert(TypeCategory::{cat:?}, Color32::from_rgb(0x{:02X}, 0x{:02X}, 0x{:02X}));\n",
                color.r(), color.g(), color.b()
            ));
        }
    }

    // Severities
    s.push_str("\n    let mut severity = HashMap::new();\n");
    for &sev in Severity::ALL {
        if let Some(&color) = palette.severity.get(&sev) {
            s.push_str(&format!(
                "    severity.insert(Severity::{sev:?}, Color32::from_rgb(0x{:02X}, 0x{:02X}, 0x{:02X}));\n",
                color.r(), color.g(), color.b()
            ));
        }
    }

    // Chrome states
    s.push_str("\n    let mut chrome = HashMap::new();\n");
    for &state in ChromeState::ALL {
        if let Some(&color) = palette.chrome.get(&state) {
            if color == Color32::WHITE {
                s.push_str(&format!(
                    "    chrome.insert(ChromeState::{state:?}, Color32::WHITE);\n"
                ));
            } else {
                s.push_str(&format!(
                    "    chrome.insert(ChromeState::{state:?}, Color32::from_rgb(0x{:02X}, 0x{:02X}, 0x{:02X}));\n",
                    color.r(), color.g(), color.b()
                ));
            }
        }
    }

    // Per-icon, per-state overrides (only non-empty entries emitted)
    s.push_str("\n    let mut overrides = HashMap::new();\n");
    for kind in IconKind::ALL {
        for &state in ChromeState::ALL {
            if let Some(&color) = palette.overrides.get(&(*kind, state)) {
                s.push_str(&format!(
                    "    overrides.insert((IconKind::{kind:?}, ChromeState::{state:?}), Color32::from_rgb(0x{:02X}, 0x{:02X}, 0x{:02X}));\n",
                    color.r(), color.g(), color.b()
                ));
            }
        }
    }

    // Per-icon tint mode overrides (only non-empty)
    s.push_str("\n    let mut tint_modes = HashMap::new();\n");
    for kind in IconKind::ALL {
        if let Some(&mode) = palette.tint_modes.get(kind) {
            s.push_str(&format!(
                "    tint_modes.insert(IconKind::{kind:?}, TintMode::{mode:?});\n"
            ));
        }
    }

    s.push_str("\n    Self { category, severity, chrome, overrides, tint_modes }\n");
    s.push_str("}\n");

    s
}
