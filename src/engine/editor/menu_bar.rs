//! Editor Menu Bar
//!
//! Provides File, Edit, View, and Help menus for the editor.
//! The View menu allows restoring closed panels.

use super::{CommandHistory, EditorDockState, EditorTab};

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
}

/// Render the editor menu bar
///
/// Returns any action that should be handled by the caller.
pub fn render_menu_bar(
    ctx: &egui::Context,
    dock_state: &mut EditorDockState,
    command_history: &CommandHistory,
) -> MenuAction {
    let mut action = MenuAction::None;

    egui::TopBottomPanel::top("menu_bar")
        .exact_height(24.0)
        .show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                // File menu
                ui.menu_button("File", |ui| {
                    if ui.button("Save Scene (Ctrl+S)").clicked() {
                        action = MenuAction::SaveScene;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        action = MenuAction::Exit;
                        ui.close_menu();
                    }
                });

                // Edit menu
                ui.menu_button("Edit", |ui| {
                    let undo_text = if let Some(desc) = command_history.undo_description() {
                        format!("Undo: {} (Ctrl+Z)", desc)
                    } else {
                        "Undo (Ctrl+Z)".to_string()
                    };

                    if ui
                        .add_enabled(command_history.can_undo(), egui::Button::new(undo_text))
                        .clicked()
                    {
                        action = MenuAction::Undo;
                        ui.close_menu();
                    }

                    let redo_text = if let Some(desc) = command_history.redo_description() {
                        format!("Redo: {} (Ctrl+Y)", desc)
                    } else {
                        "Redo (Ctrl+Y)".to_string()
                    };

                    if ui
                        .add_enabled(command_history.can_redo(), egui::Button::new(redo_text))
                        .clicked()
                    {
                        action = MenuAction::Redo;
                        ui.close_menu();
                    }
                });

                // View menu - toggle panels
                ui.menu_button("View", |ui| {
                    ui.label(egui::RichText::new("Panels").strong());
                    ui.separator();

                    // List all panels with checkmarks for open ones
                    let panels = [
                        (EditorTab::Hierarchy, "Hierarchy"),
                        (EditorTab::Inspector, "Inspector"),
                        (EditorTab::AssetBrowser, "Assets"),
                        (EditorTab::Console, "Console"),
                        (EditorTab::Profiler, "Profiler"),
                    ];

                    for (tab, name) in panels {
                        let is_open = dock_state.is_tab_open(tab);
                        let label = if is_open {
                            format!("  {} {}", name, "")
                        } else {
                            format!("  {}", name)
                        };

                        if ui.button(label).clicked() {
                            if is_open {
                                // Already open - could close it, but for simplicity just focus
                                // Closing tabs is handled by the X button on tabs
                            } else {
                                dock_state.open_tab(tab);
                            }
                            ui.close_menu();
                        }
                    }

                    ui.separator();

                    if ui.button("Save Layout").clicked() {
                        action = MenuAction::SaveLayout;
                        ui.close_menu();
                    }

                    if ui.button("Reset Layout").clicked() {
                        action = MenuAction::ResetLayout;
                        ui.close_menu();
                    }
                });

                // Help menu
                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() {
                        // Could show an about dialog
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("Rust Game Engine").weak());
                    ui.label(egui::RichText::new("Tutorial 21: Inspector Panel").weak().small());
                });
            });
        });

    action
}
