//! Editor Menu Bar
//!
//! Provides File, Edit, View, and Help menus for the editor.
//! The View menu allows restoring closed panels.
//! Play mode icon helpers are defined here and used by the viewport tab bar overlay.

use super::build_dialog::{BuildDialog, BuildProfile, BuildState};
use super::console::ConsoleLog;
use super::icons::{IconManager, ToolbarIcon};
use super::{CommandHistory, EditorDockState, EditorTab};
use crate::engine::ecs::resources::PlayMode;
use egui::{Color32, Vec2};

/// Actions that can be triggered from the menu bar
#[derive(Debug, Clone, PartialEq)]
pub enum MenuAction {
    /// No action
    None,
    /// Save the current scene
    SaveScene,
    /// Exit the application
    Exit,
    /// Undo the last action
    Undo,
    /// Redo the last undone action
    Redo,
    /// Save the current layout
    SaveLayout,
    /// Reset dock layout to default
    ResetLayout,
    /// Replace the current world with the deterministic benchmark scene.
    LoadBenchmarkScene,
    /// Launch the standalone benchmark runner.
    RunBenchmark,
    /// Enter play mode (Edit -> Playing)
    Play,
    /// Pause play mode (Playing -> Paused)
    Pause,
    /// Resume play mode (Paused -> Playing)
    Resume,
    /// Stop play mode and restore snapshot (Playing|Paused -> Edit)
    Stop,
}

pub(super) mod play_colors {
    use egui::Color32;

    pub const PLAY: Color32 = Color32::from_rgb(80, 200, 80);
    pub const PLAY_HOVER: Color32 = Color32::from_rgb(120, 255, 120);
    pub const PAUSE: Color32 = Color32::from_rgb(220, 200, 60);
    pub const PAUSE_HOVER: Color32 = Color32::from_rgb(255, 240, 100);
    pub const STOP: Color32 = Color32::from_rgb(200, 80, 80);
    pub const STOP_HOVER: Color32 = Color32::from_rgb(255, 120, 120);
    pub const RESUME: Color32 = Color32::from_rgb(80, 200, 80);
    pub const RESUME_HOVER: Color32 = Color32::from_rgb(120, 255, 120);
    pub const DISABLED: Color32 = Color32::from_gray(80);
}

/// Render a tinted play-mode icon button.
/// Returns true if clicked.
pub(super) fn play_icon_button(
    ui: &mut egui::Ui,
    icon: ToolbarIcon,
    fallback: &str,
    tint: Color32,
    hover_tint: Color32,
    tooltip: &str,
    icon_manager: Option<&IconManager>,
) -> bool {
    let size = Vec2::new(18.0, 18.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());

    let pointer_pos = ui.input(|i| i.pointer.latest_pos());
    let in_rect = pointer_pos.map(|p| rect.contains(p)).unwrap_or(false);
    let primary_released = ui.input(|i| i.pointer.primary_released());
    let clicked = primary_released && in_rect;

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let current_tint = if in_rect { hover_tint } else { tint };

        let has_icon = icon_manager
            .and_then(|mgr| mgr.get(icon))
            .map(|texture| {
                let image_rect = rect.shrink(1.0);
                painter.image(
                    texture.id(),
                    image_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    current_tint,
                );
                true
            })
            .unwrap_or(false);

        if !has_icon {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                fallback,
                egui::FontId::proportional(12.0),
                current_tint,
            );
        }

        if in_rect {
            egui::containers::Tooltip::always_open(
                ui.ctx().clone(),
                ui.layer_id(),
                egui::Id::new(tooltip),
                egui::containers::PopupAnchor::Pointer,
            )
            .show(|ui| {
                ui.label(tooltip);
            });
        }
    }

    clicked
}

/// Render the play/pause/stop icon cluster
pub(super) fn render_play_controls(
    ui: &mut egui::Ui,
    play_mode: PlayMode,
    icon_manager: Option<&IconManager>,
    action: &mut MenuAction,
) {
    match play_mode {
        PlayMode::Edit => {
            if play_icon_button(
                ui,
                ToolbarIcon::Play,
                "▶",
                play_colors::PLAY,
                play_colors::PLAY_HOVER,
                "Play (F5)",
                icon_manager,
            ) {
                *action = MenuAction::Play;
            }
            play_icon_button(
                ui,
                ToolbarIcon::Pause,
                "⏸",
                play_colors::DISABLED,
                play_colors::DISABLED,
                "Pause (F6) - not playing",
                icon_manager,
            );
            play_icon_button(
                ui,
                ToolbarIcon::Stop,
                "⏹",
                play_colors::DISABLED,
                play_colors::DISABLED,
                "Stop (F5) - not playing",
                icon_manager,
            );
        }
        PlayMode::Playing => {
            play_icon_button(
                ui,
                ToolbarIcon::Play,
                "▶",
                play_colors::DISABLED,
                play_colors::DISABLED,
                "Already playing",
                icon_manager,
            );
            if play_icon_button(
                ui,
                ToolbarIcon::Pause,
                "⏸",
                play_colors::PAUSE,
                play_colors::PAUSE_HOVER,
                "Pause (F6)",
                icon_manager,
            ) {
                *action = MenuAction::Pause;
            }
            if play_icon_button(
                ui,
                ToolbarIcon::Stop,
                "⏹",
                play_colors::STOP,
                play_colors::STOP_HOVER,
                "Stop (F5)",
                icon_manager,
            ) {
                *action = MenuAction::Stop;
            }
        }
        PlayMode::Paused => {
            if play_icon_button(
                ui,
                ToolbarIcon::SkipForward,
                "⏭",
                play_colors::RESUME,
                play_colors::RESUME_HOVER,
                "Resume (F6)",
                icon_manager,
            ) {
                *action = MenuAction::Resume;
            }
            play_icon_button(
                ui,
                ToolbarIcon::Pause,
                "⏸",
                play_colors::DISABLED,
                play_colors::DISABLED,
                "Already paused",
                icon_manager,
            );
            if play_icon_button(
                ui,
                ToolbarIcon::Stop,
                "⏹",
                play_colors::STOP,
                play_colors::STOP_HOVER,
                "Stop (F5)",
                icon_manager,
            ) {
                *action = MenuAction::Stop;
            }
        }
    }
}

/// Render the editor menu bar
///
/// Returns any action that should be handled by the caller.
pub fn render_menu_bar(
    ctx: &egui::Context,
    dock_state: &mut EditorDockState,
    command_history: &CommandHistory,
    play_mode: PlayMode,
    build_dialog: &mut BuildDialog,
    console_messages: &mut ConsoleLog,
    show_benchmark_tools: bool,
) -> MenuAction {
    let mut action = MenuAction::None;

    let build_msgs = build_dialog.poll();
    let is_build_active = matches!(
        build_dialog.state,
        BuildState::Building | BuildState::CopyingContent
    );
    console_messages.extend(build_msgs);
    if is_build_active {
        ctx.request_repaint();
    }

    egui::TopBottomPanel::top("menu_bar")
        .exact_height(24.0)
        .show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                // File menu
                ui.menu_button("File", |ui| {
                    if ui.button("Save Scene (Ctrl+S)").clicked() {
                        action = MenuAction::SaveScene;
                        ui.close();
                    }
                    if show_benchmark_tools {
                        ui.separator();

                        ui.menu_button("Benchmark", |ui| {
                            if ui.button("Load Benchmark Scene").clicked() {
                                action = MenuAction::LoadBenchmarkScene;
                                ui.close();
                            }
                            if ui.button("Run CPU Benchmark").clicked() {
                                action = MenuAction::RunBenchmark;
                                ui.close();
                            }
                        });
                    }

                    ui.separator();

                    ui.menu_button("Build Game", |ui| {
                        ui.set_min_width(260.0);

                        let is_building = matches!(
                            build_dialog.state,
                            BuildState::Building | BuildState::CopyingContent
                        );

                        ui.add_enabled_ui(!is_building, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Platform:");
                                ui.label(build_dialog.settings.platform.label());
                            });

                            ui.horizontal(|ui| {
                                ui.label("Profile:");
                                egui::ComboBox::from_id_salt("build_profile_menu")
                                    .selected_text(build_dialog.settings.profile.label())
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut build_dialog.settings.profile,
                                            BuildProfile::Release,
                                            "Release",
                                        );
                                        ui.selectable_value(
                                            &mut build_dialog.settings.profile,
                                            BuildProfile::Shipping,
                                            "Shipping",
                                        );
                                    });
                            });

                            ui.horizontal(|ui| {
                                ui.label("Output:");
                                ui.text_edit_singleline(&mut build_dialog.settings.output_dir);
                            });
                        });

                        ui.separator();

                        match &build_dialog.state {
                            BuildState::Idle => {
                                if ui.button("Build").clicked() {
                                    build_dialog.start_build();
                                }
                            }
                            BuildState::Building | BuildState::CopyingContent => {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    let label = if build_dialog.state == BuildState::CopyingContent
                                    {
                                        "Packing assets..."
                                    } else {
                                        "Building..."
                                    };
                                    if let Some(start) = build_dialog.start_time {
                                        let elapsed = start.elapsed().as_secs();
                                        ui.label(format!("{} ({}s)", label, elapsed));
                                    } else {
                                        ui.label(label);
                                    }
                                });
                                ui.label("See Console for details.");
                            }
                            BuildState::Success { binary_size } => {
                                ui.colored_label(
                                    egui::Color32::from_rgb(80, 200, 80),
                                    format!(
                                        "Done! {:.1} MB",
                                        *binary_size as f64 / (1024.0 * 1024.0)
                                    ),
                                );
                                if ui.button("Build Again").clicked() {
                                    build_dialog.start_build();
                                }
                            }
                            BuildState::Failed { error } => {
                                ui.colored_label(
                                    egui::Color32::from_rgb(200, 80, 80),
                                    format!("Failed: {}", error),
                                );
                                if ui.button("Retry").clicked() {
                                    build_dialog.start_build();
                                }
                            }
                        }
                    });

                    ui.separator();
                    if ui.button("Exit").clicked() {
                        action = MenuAction::Exit;
                        ui.close();
                    }
                });

                // Edit menu
                let is_edit_mode = play_mode == PlayMode::Edit;
                ui.menu_button("Edit", |ui| {
                    let undo_text = if let Some(desc) = command_history.undo_description() {
                        format!("Undo: {} (Ctrl+Z)", desc)
                    } else {
                        "Undo (Ctrl+Z)".to_string()
                    };

                    if ui
                        .add_enabled(
                            is_edit_mode && command_history.can_undo(),
                            egui::Button::new(undo_text),
                        )
                        .clicked()
                    {
                        action = MenuAction::Undo;
                        ui.close();
                    }

                    let redo_text = if let Some(desc) = command_history.redo_description() {
                        format!("Redo: {} (Ctrl+Y)", desc)
                    } else {
                        "Redo (Ctrl+Y)".to_string()
                    };

                    if ui
                        .add_enabled(
                            is_edit_mode && command_history.can_redo(),
                            egui::Button::new(redo_text),
                        )
                        .clicked()
                    {
                        action = MenuAction::Redo;
                        ui.close();
                    }
                });

                // View menu - toggle panels
                ui.menu_button("View", |ui| {
                    ui.label(egui::RichText::new("Panels").strong());
                    ui.separator();

                    let panels = [
                        (EditorTab::Viewport, "Viewport"),
                        (EditorTab::Hierarchy, "Hierarchy"),
                        (EditorTab::Inspector, "Inspector"),
                        (EditorTab::AssetBrowser, "Assets"),
                        (EditorTab::Console, "Console"),
                        (EditorTab::Profiler, "Profiler"),
                    ];

                    for (tab, name) in &panels {
                        let is_open = dock_state.is_tab_open(tab);
                        let label = if is_open {
                            format!("  {} {}", name, "")
                        } else {
                            format!("  {}", name)
                        };

                        if ui.button(label).clicked() {
                            if !is_open {
                                dock_state.open_tab(tab.clone());
                            }
                            ui.close();
                        }
                    }

                    ui.separator();

                    if ui.button("Save Layout").clicked() {
                        action = MenuAction::SaveLayout;
                        ui.close();
                    }

                    if ui.button("Reset Layout").clicked() {
                        action = MenuAction::ResetLayout;
                        ui.close();
                    }
                });

                // Help menu
                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() {
                        ui.close();
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("Rust Game Engine").weak());
                    ui.label(
                        egui::RichText::new("Tutorial 21: Inspector Panel")
                            .weak()
                            .small(),
                    );
                });
            });
        });

    action
}
