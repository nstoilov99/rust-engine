//! Dev-only Icon Inspector window — gated behind `editor-debug` feature.
//!
//! Opens as a secondary OS window. Lists every editor icon, lets you tweak
//! palette colors live, and verifies tinting / category-color decisions.
//!
//! **No silent persistence.** Closing or restarting reverts to `IconPalette::default_dark()`.
//! Use the **Export Palette** button to copy a Rust snippet to the clipboard.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Color32, Context, RichText, TextureHandle, Ui};

use crate::engine::editor::icon_classes::{ChromeState, Severity, TintMode, TypeCategory};
use crate::engine::editor::widgets::{load_icon_textures, IconKind, IconRegistry};

/// What the inspector is currently focused on. The detail panel uses this to
/// decide which editor to render — `IconKind` overrides for the typed/
/// chrome/severity classes, or panel-icon overrides for an SVG inside one of
/// the auto-discovered subfolders of `engine/icons/`.
#[derive(Clone)]
enum InspectorSelection {
    /// One of the enum-keyed icons the rest of the editor renders through
    /// `IconRegistry`.
    Kind(IconKind),
    /// An SVG inside `engine/icons/<panel>/<stem>.svg`.
    Panel { panel: String, stem: String },
}

impl InspectorSelection {
    fn is_kind(&self, kind: IconKind) -> bool {
        matches!(self, InspectorSelection::Kind(k) if *k == kind)
    }

    fn is_panel(&self, panel: &str, stem: &str) -> bool {
        matches!(
            self,
            InspectorSelection::Panel { panel: p, stem: s } if p == panel && s == stem
        )
    }
}

/// Persistent state for the Icon Inspector window.
pub struct IconInspectorWindow {
    pub open: bool,
    /// Display size for icons in the grid (px).
    icon_size: f32,
    /// Current chrome state for chrome icon preview.
    chrome_state: ChromeState,
    /// Whatever's selected for the detail panel — IconKind or panel SVG.
    selection: Option<InspectorSelection>,
    /// Icon textures loaded into the secondary window's egui context.
    /// `IconRegistry`'s textures are bound to the main window's context,
    /// so the secondary renderer cannot sample them — we keep a local copy
    /// bound to whichever context we render into.
    local_textures: HashMap<IconKind, TextureHandle>,
    /// Per-panel icon sets auto-discovered from `engine/icons/<panel>/*.svg`.
    /// Each subdirectory becomes its own collapsible section in the inspector,
    /// keyed by file stem. New panels appear by simply creating a subfolder
    /// with SVGs in it — no code change needed here.
    panel_sets: Vec<PanelIconSet>,
    /// Free-text filter (matches IconKind name and panel-icon stems).
    search: String,
    /// Marker identifying which egui context `local_textures` were loaded for.
    /// When this differs from the current context's marker, textures are reloaded
    /// (e.g. the inspector was closed and a fresh secondary window was created).
    last_ctx_marker: Option<u64>,
    /// Transient feedback for the most recent Save action.
    save_status: Option<SaveStatus>,
}

/// A panel-scoped icon set: every SVG inside a single subfolder of
/// `engine/icons/`, keyed by file stem.
struct PanelIconSet {
    /// Display name (the folder name, title-cased).
    name: String,
    /// Source folder, kept around for diagnostics in the toolbar.
    folder: PathBuf,
    /// Stem-keyed icon textures — same shape as `HierarchyIcons`.
    textures: HashMap<String, TextureHandle>,
}

impl PanelIconSet {
    /// Stable key used for palette overrides (raw folder name in lowercase).
    /// Diverges from `name` (which is title-cased for display) so the on-disk
    /// override key matches what the panel code looks up.
    fn folder_key(&self) -> String {
        self.folder
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_default()
    }
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
            icon_size: 28.0,
            chrome_state: ChromeState::Default,
            selection: None,
            local_textures: HashMap::new(),
            panel_sets: Vec::new(),
            search: String::new(),
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
                self.show_search_row(ui);
                ui.separator();

                // Two-column layout: icon browser (left) + detail panel (right)
                let detail_width = if self.selection.is_some() { 260.0 } else { 0.0 };

                ui.horizontal_top(|ui| {
                    // Left: scrollable icon browser
                    let browser_width = (ui.available_width() - detail_width).max(220.0);
                    ui.vertical(|ui| {
                        ui.set_width(browser_width);
                        egui::ScrollArea::vertical()
                            .id_salt("icon_browser_scroll")
                            .show(ui, |ui| {
                                // Panel-folder sections come FIRST so icons
                                // tied to a specific editor surface (Hierarchy,
                                // future Asset Browser, …) are easy to find.
                                self.show_panel_sections(ui, registry);
                                if !self.panel_sets.is_empty() {
                                    ui.add_space(6.0);
                                }
                                self.show_chrome_section(ui, registry);
                                ui.add_space(6.0);
                                self.show_typed_section(ui, registry);
                                ui.add_space(6.0);
                                self.show_severity_section(ui, registry);
                            });
                    });

                    // Right: detail panel for selected icon
                    if let Some(selected) = self.selection.clone() {
                        ui.separator();
                        ui.vertical(|ui| {
                            ui.set_width(detail_width);
                            egui::ScrollArea::vertical()
                                .id_salt("icon_detail_scroll")
                                .show(ui, |ui| {
                                    match selected {
                                        InspectorSelection::Kind(kind) => {
                                            self.show_detail_panel(ui, kind, registry);
                                        }
                                        InspectorSelection::Panel { panel, stem } => {
                                            self.show_panel_detail_panel(
                                                ui, &panel, &stem, registry,
                                            );
                                        }
                                    }
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
            self.panel_sets = discover_panel_sets(ctx);
            self.last_ctx_marker = Some(ctx_marker);
            // Request another frame so any textures the renderer is still
            // uploading get a chance to show without forcing the user to
            // click around — addresses the "icons only appear after I click"
            // first-frame artefact.
            ctx.request_repaint();
        }
    }

    fn show_toolbar(&mut self, ui: &mut Ui, registry: &mut IconRegistry) {
        ui.horizontal(|ui| {
            // Save persists the current palette to `editor_icon_palette.ron`
            // (workspace root). It's auto-loaded on next launch.
            if ui
                .button("\u{1F4BE} Save")
                .on_hover_text("Save palette to editor_icon_palette.ron")
                .clicked()
            {
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

            if ui
                .button("Reset")
                .on_hover_text("Discard local edits and revert to default_dark")
                .clicked()
            {
                registry.reset_palette();
                self.selection = None;
                self.save_status = None;
            }

            if ui
                .button("Export")
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

            // Save status pill — pinned to the right so it doesn't push the
            // size slider around as it appears / disappears.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(status) = self.save_status {
                    let (text, color) = match status {
                        SaveStatus::Saved => ("Saved \u{2713}", Color32::from_rgb(0x6E, 0xCB, 0x7C)),
                        SaveStatus::Failed => ("Save failed", Color32::from_rgb(0xE5, 0x6B, 0x6B)),
                    };
                    ui.colored_label(color, text);
                }
            });
        });
    }

    /// Second toolbar row: search field + size slider. Splitting these out of
    /// the main button bar keeps the file-action buttons readable.
    fn show_search_row(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Search:");
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .desired_width(220.0)
                    .hint_text("Filter by name…"),
            );
            if !self.search.is_empty() && ui.small_button("\u{2715}").clicked() {
                self.search.clear();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::Slider::new(&mut self.icon_size, 16.0..=64.0)
                        .step_by(4.0)
                        .suffix(" px"),
                );
                ui.label("Size:");
            });
        });
    }

    /// Render one collapsible section per discovered `engine/icons/<panel>/`
    /// subfolder. Each section lays out its SVGs in a wrapping grid with the
    /// stem name as a tooltip — adding new icons is a drag-drop into the
    /// folder, no code change required here.
    fn show_panel_sections(&mut self, ui: &mut Ui, registry: &IconRegistry) {
        let panel_count = self.panel_sets.len();
        for set_idx in 0..panel_count {
            // `name` is cheap to clone and lets us avoid an aliasing borrow
            // of `self.panel_sets` while we call self-methods inside the body.
            let name = self.panel_sets[set_idx].name.clone();
            let count = self.panel_sets[set_idx].textures.len();
            let header = format!("{name} ({count})");

            egui::CollapsingHeader::new(RichText::new(header).strong())
                .default_open(true)
                .id_salt(format!("panel_section_{name}"))
                .show(ui, |ui| {
                    if count == 0 {
                        ui.label(
                            RichText::new(format!(
                                "No SVG files yet — drop them into {}",
                                self.panel_sets[set_idx].folder.display()
                            ))
                            .weak()
                            .small()
                            .italics(),
                        );
                        return;
                    }
                    self.draw_named_icon_grid(ui, set_idx, registry);
                });
        }
    }

    /// Wrapping grid of stem-keyed icons drawn at the inspector's icon_size.
    /// Search filter dims the section to nothing if no matches.
    fn draw_named_icon_grid(&mut self, ui: &mut Ui, set_idx: usize, registry: &IconRegistry) {
        let panel = self.panel_sets[set_idx].folder_key();
        let mut stems: Vec<String> = self.panel_sets[set_idx]
            .textures
            .keys()
            .cloned()
            .collect();
        stems.sort();

        let needle = self.search.trim().to_lowercase();
        let cell_size = self.icon_size + 6.0;
        let spacing = 4.0;

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(spacing, spacing);
            for stem in &stems {
                if !needle.is_empty() && !stem.to_lowercase().contains(&needle) {
                    continue;
                }

                let is_selected = self
                    .selection
                    .as_ref()
                    .map_or(false, |s| s.is_panel(&panel, stem));
                let has_override = registry.panel_icon_has_any_override(&panel, stem);

                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(cell_size, cell_size),
                    egui::Sense::click(),
                );

                if response.clicked() {
                    self.selection = if is_selected {
                        None
                    } else {
                        Some(InspectorSelection::Panel {
                            panel: panel.clone(),
                            stem: stem.clone(),
                        })
                    };
                }

                let painter = ui.painter();

                if is_selected {
                    painter.rect_filled(rect, 4.0, ui.visuals().selection.bg_fill);
                } else if response.hovered() {
                    painter.rect_filled(rect, 4.0, ui.visuals().widgets.hovered.bg_fill);
                }

                let icon_rect = egui::Rect::from_center_size(
                    rect.center(),
                    egui::vec2(self.icon_size, self.icon_size),
                );

                // Tint resolved through the registry so the inspector reflects
                // saved overrides immediately.
                let tint = registry.panel_icon_tint(
                    &panel,
                    stem,
                    self.chrome_state,
                    Color32::WHITE,
                );

                if let Some(tex) = self.panel_sets[set_idx].textures.get(stem) {
                    painter.image(
                        tex.id(),
                        icon_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        tint,
                    );
                }

                if has_override {
                    let r = 2.5;
                    let c = egui::pos2(rect.right() - r - 2.0, rect.top() + r + 2.0);
                    painter.circle_filled(c, r, Color32::from_rgb(0xFF, 0xC8, 0x57));
                }

                let mut tooltip = stem.clone();
                if has_override {
                    tooltip.push_str(" — overridden");
                }
                response.on_hover_text(tooltip);
            }
        });
    }

    /// Detail panel for a selected panel-folder SVG: large preview at the
    /// current chrome state, classification info, and the same per-state
    /// override grid as `IconKind` icons.
    fn show_panel_detail_panel(
        &mut self,
        ui: &mut Ui,
        panel: &str,
        stem: &str,
        registry: &mut IconRegistry,
    ) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new(stem).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("\u{2715}").clicked() {
                    self.selection = None;
                }
            });
        });
        ui.label(
            RichText::new(format!("Panel: {panel}"))
                .weak()
                .small(),
        );

        ui.separator();

        // Large preview at the current chrome state.
        let preview_size = 64.0;
        let (preview_rect, _) = ui.allocate_exact_size(
            egui::vec2(preview_size, preview_size),
            egui::Sense::hover(),
        );
        let painter = ui.painter();
        painter.rect_filled(preview_rect, 4.0, Color32::from_gray(30));

        let preview_tint = registry.panel_icon_tint(panel, stem, self.chrome_state, Color32::WHITE);
        // Find the texture for this (panel, stem) in our local panel_sets.
        let texture = self
            .panel_sets
            .iter()
            .find(|s| s.folder_key() == panel)
            .and_then(|s| s.textures.get(stem));

        if let Some(tex) = texture {
            painter.image(
                tex.id(),
                preview_rect.shrink(4.0),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                preview_tint,
            );
        } else {
            painter.text(
                preview_rect.center(),
                egui::Align2::CENTER_CENTER,
                "?",
                egui::FontId::proportional(32.0),
                Color32::from_gray(120),
            );
        }

        ui.add_space(8.0);

        // Per-state overrides — same model as IconKind, but stored against
        // (panel, stem, state) so adding new SVGs needs no enum work.
        ui.label(RichText::new("Per-State Overrides").strong());
        ui.label(
            RichText::new(
                "Set any state explicitly. Unset states fall back to the \
                 Default override, then to the icon's authored colours.",
            )
            .weak()
            .small(),
        );
        ui.add_space(4.0);

        egui::Grid::new(format!("panel_overrides_{panel}_{stem}"))
            .num_columns(3)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                for &state in ChromeState::ALL {
                    let has_override = registry.panel_icon_override(panel, stem, state).is_some();
                    let effective = registry.panel_icon_tint(
                        panel,
                        stem,
                        state,
                        Color32::WHITE,
                    );
                    let mut display_color =
                        registry.panel_icon_override(panel, stem, state).unwrap_or(effective);

                    ui.label(state.display_name());

                    let resp = ui.color_edit_button_srgba(&mut display_color);
                    if resp.changed() {
                        registry.set_panel_icon_override(panel, stem, state, display_color);
                    }

                    if has_override {
                        if ui
                            .small_button("\u{2715}")
                            .on_hover_text("Clear this state's override")
                            .clicked()
                        {
                            registry.clear_panel_icon_override(panel, stem, state);
                        }
                    } else {
                        ui.label(RichText::new("inherits").weak().small().italics());
                    }
                    ui.end_row();
                }
            });

        ui.add_space(6.0);
        if registry.panel_icon_has_any_override(panel, stem)
            && ui.button("Clear All Overrides").clicked()
        {
            registry.clear_all_panel_icon_overrides(panel, stem);
        }
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

    /// Render a wrapping grid of `IconKind` cells.
    ///
    /// Cells are square (just the icon — no per-cell label below) so the
    /// grid stays dense; the icon's display name appears on hover and a small
    /// dot in the corner marks per-state overrides. Click selects the icon
    /// for the detail panel on the right.
    fn icon_grid(
        &mut self,
        ui: &mut Ui,
        registry: &IconRegistry,
        icons: Vec<IconKind>,
        state: ChromeState,
    ) {
        let cell_size = self.icon_size + 6.0;
        let spacing = 4.0;
        let needle = self.search.trim().to_lowercase();

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(spacing, spacing);

            for &kind in &icons {
                if !needle.is_empty()
                    && !kind.display_name().to_lowercase().contains(&needle)
                {
                    continue;
                }

                let is_selected = self
                    .selection
                    .as_ref()
                    .map_or(false, |s| s.is_kind(kind));
                let has_override = registry.icon_has_any_override(kind);
                let is_authored = registry.tint_mode(kind) == TintMode::Authored;
                let tint = registry.tint(kind, state);

                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(cell_size, cell_size),
                    egui::Sense::click(),
                );

                if response.clicked() {
                    self.selection = if is_selected {
                        None
                    } else {
                        Some(InspectorSelection::Kind(kind))
                    };
                }

                let painter = ui.painter();

                // Selection / hover background.
                if is_selected {
                    painter.rect_filled(rect, 4.0, ui.visuals().selection.bg_fill);
                } else if response.hovered() {
                    painter.rect_filled(rect, 4.0, ui.visuals().widgets.hovered.bg_fill);
                }

                let icon_rect = egui::Rect::from_center_size(
                    rect.center(),
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
                    // Fallback glyph for IconKinds with no PNG mapping.
                    let text = IconRegistry::fallback_text(kind);
                    let font_size = (self.icon_size * 0.6).max(10.0);
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        text,
                        egui::FontId::proportional(font_size),
                        draw_tint,
                    );
                }

                // Corner badges:
                //   • dot   = at least one per-state override
                //   • "A"   = authored mode (drawn as-is, no tint)
                if has_override {
                    let r = 2.5;
                    let c = egui::pos2(rect.right() - r - 2.0, rect.top() + r + 2.0);
                    painter.circle_filled(c, r, Color32::from_rgb(0xFF, 0xC8, 0x57));
                }
                if is_authored {
                    painter.text(
                        egui::pos2(rect.left() + 2.0, rect.top() + 1.0),
                        egui::Align2::LEFT_TOP,
                        "A",
                        egui::FontId::proportional(8.0),
                        Color32::from_gray(160),
                    );
                }

                // Tooltip carries the full display name.
                let mut tooltip = kind.display_name().to_string();
                if has_override {
                    tooltip.push_str(" — overridden");
                }
                if is_authored {
                    tooltip.push_str(" — authored");
                }
                response.on_hover_text(tooltip);
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
                    self.selection = None;
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
/// Walk every subdirectory of `engine/icons/` and load each one as a panel
/// icon set. Subdirectory name → section title (title-cased), `*.svg` files
/// inside → keyed-by-stem texture entries.
///
/// Adding a new panel is therefore a no-code task: create the folder, drop
/// SVGs in. Errors / missing folder are logged once and yield no sections.
fn discover_panel_sets(ctx: &Context) -> Vec<PanelIconSet> {
    use crate::engine::editor::hierarchy_icons::load_svg_texture;

    let root = Path::new("engine/icons");
    let mut sets = Vec::new();

    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("Icons root unreadable ({}): {}", root.display(), e);
            return sets;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let raw_name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        // SVG list inside the folder.
        let mut textures = HashMap::new();
        if let Ok(inner) = std::fs::read_dir(&path) {
            for f in inner.flatten() {
                let fp = f.path();
                if fp.extension().and_then(|e| e.to_str()) != Some("svg") {
                    continue;
                }
                let stem = match fp.file_stem().and_then(|s| s.to_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                match load_svg_texture(ctx, &fp, &format!("inspector_{raw_name}_{stem}")) {
                    Ok(tex) => {
                        textures.insert(stem, tex);
                    }
                    Err(e) => {
                        log::warn!("Inspector SVG load failed ({}): {}", fp.display(), e);
                    }
                }
            }
        }

        sets.push(PanelIconSet {
            name: title_case_folder(&raw_name),
            folder: path,
            textures,
        });
    }

    sets.sort_by(|a, b| a.name.cmp(&b.name));
    sets
}

/// Convert a folder name like `asset_browser` to `Asset Browser` for display.
fn title_case_folder(name: &str) -> String {
    name.split(|c: char| c == '_' || c == '-')
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

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
