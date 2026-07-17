//! Menu bar rendered with crusty-gui.

use std::collections::HashMap;

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::math::{Color, Pos2, Rect, Vec2};
use crusty_gui::paint::{PaintCmd, TextureId};
use crusty_gui::widgets::{show_tooltip_for, Button, ComboBox, Label, SelectableValue, TextEdit};

use super::build_dialog::{BuildDialog, BuildProfile, BuildState};
use super::console::ConsoleLog;
use super::dock_crusty::CrustyDockLayout;
use super::menu_bar::MenuAction;
use super::{CommandHistory, EditorTab};
use crate::engine::ecs::resources::PlayMode;

pub struct MenuBarCtx<'a> {
    pub dock_state: &'a mut CrustyDockLayout,
    pub command_history: &'a CommandHistory,
    pub play_mode: PlayMode,
    pub build_dialog: &'a mut BuildDialog,
    pub console_messages: &'a mut ConsoleLog,
    pub show_benchmark_tools: bool,
    pub icons: &'a HashMap<String, TextureId>,
}

mod play_colors {
    use crusty_gui::math::Color;

    pub fn play() -> Color {
        Color::from_srgb_u8(80, 200, 80, 255)
    }
    pub fn play_hover() -> Color {
        Color::from_srgb_u8(120, 255, 120, 255)
    }
    pub fn pause() -> Color {
        Color::from_srgb_u8(220, 200, 60, 255)
    }
    pub fn pause_hover() -> Color {
        Color::from_srgb_u8(255, 240, 100, 255)
    }
    pub fn stop() -> Color {
        Color::from_srgb_u8(200, 80, 80, 255)
    }
    pub fn stop_hover() -> Color {
        Color::from_srgb_u8(255, 120, 120, 255)
    }
    pub fn disabled() -> Color {
        Color::from_srgb_u8(80, 80, 80, 255)
    }
}

pub fn menu_bar_panel(ui: &mut Ui, bar_rect: Rect, ctx: MenuBarCtx) -> MenuAction {
    let mut action = MenuAction::None;
    // Historical scale factor; 1.0 under the pure-crusty path (pixel-space
    // rects). Kept so the constants below stay in the same units.
    let s = 1.0_f32;

    // Per-frame build message poll.
    let build_msgs = ctx.build_dialog.poll();
    ctx.console_messages.extend(build_msgs);

    let rect = bar_rect;

    let MenuBarCtx {
        dock_state,
        command_history,
        play_mode,
        build_dialog,
        show_benchmark_tools,
        icons,
        ..
    } = ctx;

    let opts = UiOptions {
        padding: Vec2::new(4.0 * s, 0.0),
        spacing: 0.0,
    };
    ui.run_at(
        rect,
        Direction::LeftToRight,
        Id::new("crusty_menu_bar"),
        opts,
        |ui| {
            ui.menu_button("File", |ui| {
                if ui.menu_item("Save Scene (Ctrl+S)") {
                    action = MenuAction::SaveScene;
                }
                if ui.menu_item("Cook Collision") {
                    action = MenuAction::CookCollision;
                }
                if show_benchmark_tools {
                    ui.separator();
                    ui.submenu("Benchmark", |ui| {
                        if ui.menu_item("Load Benchmark Scene") {
                            action = MenuAction::LoadBenchmarkScene;
                        }
                        if ui.menu_item("Run CPU Benchmark") {
                            action = MenuAction::RunBenchmark;
                        }
                    });
                }
                ui.separator();
                ui.submenu_width("Build Game", 260.0 * s, |ui| {
                    build_game_menu(ui, build_dialog);
                });
                ui.separator();
                if ui.menu_item("Exit") {
                    action = MenuAction::Exit;
                }
            });

            let is_edit_mode = play_mode == PlayMode::Edit;
            ui.menu_button("Edit", |ui| {
                let undo_text = if let Some(desc) = command_history.undo_description() {
                    format!("Undo: {} (Ctrl+Z)", desc)
                } else {
                    "Undo (Ctrl+Z)".to_string()
                };
                if ui.menu_item_enabled(&undo_text, is_edit_mode && command_history.can_undo()) {
                    action = MenuAction::Undo;
                }

                let redo_text = if let Some(desc) = command_history.redo_description() {
                    format!("Redo: {} (Ctrl+Y)", desc)
                } else {
                    "Redo (Ctrl+Y)".to_string()
                };
                if ui.menu_item_enabled(&redo_text, is_edit_mode && command_history.can_redo()) {
                    action = MenuAction::Redo;
                }
            });

            ui.menu_button("View", |ui| {
                Label::new("Panels").show(ui);
                ui.separator();

                let panels = [
                    (EditorTab::Hierarchy, "Hierarchy"),
                    (EditorTab::Inspector, "Inspector"),
                    (EditorTab::AssetBrowser, "Assets"),
                    (EditorTab::Console, "Console"),
                    (EditorTab::Profiler, "Profiler"),
                ];
                for (tab, name) in &panels {
                    let is_open = dock_state.is_tab_open(tab);
                    if ui.menu_item(format!(" {}", name)) && !is_open {
                        dock_state.open_tab(tab.clone());
                    }
                }

                ui.separator();
                if ui.menu_item("Save Layout") {
                    action = MenuAction::SaveLayout;
                }
                if ui.menu_item("Reset Layout") {
                    action = MenuAction::ResetLayout;
                }
            });

            ui.menu_button("Debug", |ui| {
                if ui.menu_item("Rebuild All Shaders") {
                    action = MenuAction::RebuildShaders;
                }
                ui.separator();
                if ui.menu_item("Collision Chunk Wireframe") {
                    action = MenuAction::ToggleCollisionChunkDraw;
                }
                if ui.menu_item("Collision Chunk Grid") {
                    action = MenuAction::ToggleCollisionGridDraw;
                }
                #[cfg(feature = "editor-debug")]
                {
                    ui.separator();
                    if ui.menu_item("Icon Inspector") {
                        action = MenuAction::ToggleIconInspector;
                    }
                    if ui.menu_item("Widget Showcase") {
                        action = MenuAction::ToggleShowcase;
                    }
                }
            });

            ui.menu_button("Help", |ui| {
                let _ = ui.menu_item("About");
                ui.separator();
                Label::new("Rust Game Engine")
                    .color(Color::from_srgb_u8(140, 140, 140, 255))
                    .show(ui);
            });

            render_play_controls(ui, rect, s, play_mode, icons, &mut action);
        },
    );

    action
}

const BUILD_LABEL_W: f32 = 64.0;

/// Fixed-label-column row for the Build Game flyout: label painted
/// vertically centered against the field height, field gets the remaining
/// width so nothing overflows the flyout.
fn build_row(ui: &mut Ui, label: &str, field: impl FnOnce(&mut Ui, f32)) {
    let style = ui.style();
    let font = style.fonts.body;
    let dim = style.palette.text_dim;
    let row_h = font + 12.0; // combo trigger height; TextEdit is font + 10
    let field_w = ui.available().width() - BUILD_LABEL_W;
    let top = ui.cursor().y;
    ui.horizontal(|ui| {
        let start = ui.cursor();
        ui.painter().text(
            Pos2::new(start.x, start.y + (row_h - font * 1.25) * 0.5),
            label,
            font,
            dim,
            None,
        );
        ui.set_cursor(Pos2::new(start.x + BUILD_LABEL_W, start.y));
        field(ui, field_w);
    });
    let advanced = ui.cursor().y - top;
    if advanced < row_h {
        ui.add_space(row_h - advanced);
    }
}

/// Read-only value text aligned like the widgets in [`build_row`].
fn build_row_value(ui: &mut Ui, text: &str) {
    let style = ui.style();
    let font = style.fonts.body;
    let row_h = font + 12.0;
    let w = ui.text_mut().measure(text, font, None).x;
    let rect = ui.allocate(Vec2::new(w, row_h));
    ui.painter().text(
        Pos2::new(rect.min.x, rect.min.y + (row_h - font * 1.25) * 0.5),
        text,
        font,
        style.palette.text,
        None,
    );
}

/// Body of the `File > Build Game` flyout: settings while idle, progress /
/// result while building. While a build runs the settings render read-only.
fn build_game_menu(ui: &mut Ui, build_dialog: &mut BuildDialog) {
    let is_building = matches!(
        build_dialog.state,
        BuildState::Building | BuildState::CopyingContent
    );

    build_row(ui, "Platform:", |ui, _| {
        build_row_value(ui, build_dialog.settings.platform.label());
    });

    let profile = &mut build_dialog.settings.profile;
    build_row(ui, "Profile:", |ui, w| {
        if is_building {
            build_row_value(ui, profile.label());
        } else {
            ComboBox::new("build_profile_menu")
                .selected_text(profile.label())
                .width(w)
                .show_ui(ui, |ui| {
                    SelectableValue::new(profile, BuildProfile::Release, "Release").show(ui);
                    SelectableValue::new(profile, BuildProfile::Shipping, "Shipping").show(ui);
                });
        }
    });

    let output_dir = &mut build_dialog.settings.output_dir;
    build_row(ui, "Output:", |ui, w| {
        if is_building {
            build_row_value(ui, output_dir);
        } else {
            TextEdit::new(output_dir).width(w).show(ui);
        }
    });

    ui.separator();

    match &build_dialog.state {
        BuildState::Idle => {
            if Button::new("Build").show(ui).clicked {
                build_dialog.start_build();
            }
        }
        BuildState::Building | BuildState::CopyingContent => {
            let label = if build_dialog.state == BuildState::CopyingContent {
                "Packing assets..."
            } else {
                "Building..."
            };
            let text = if let Some(start) = build_dialog.start_time {
                format!("{} ({}s)", label, start.elapsed().as_secs())
            } else {
                label.to_string()
            };
            Label::new(text).show(ui);
            Label::new("See Console for details.").show(ui);
        }
        BuildState::Success { binary_size } => {
            Label::new(format!(
                "Done! {:.1} MB",
                *binary_size as f64 / (1024.0 * 1024.0)
            ))
            .color(Color::from_srgb_u8(80, 200, 80, 255))
            .show(ui);
            if Button::new("Build Again").show(ui).clicked {
                build_dialog.start_build();
            }
        }
        BuildState::Failed { error } => {
            Label::new(format!("Failed: {}", error))
                .color(Color::from_srgb_u8(200, 80, 80, 255))
                .show(ui);
            if Button::new("Retry").show(ui).clicked {
                build_dialog.start_build();
            }
        }
    }
}

/// Right-aligned Play | Pause | Stop cluster (Godot-style), drawn at fixed
/// positions inside the strip so it doesn't depend on the menu row cursor.
fn render_play_controls(
    ui: &mut Ui,
    bar: Rect,
    s: f32,
    play_mode: PlayMode,
    icons: &HashMap<String, TextureId>,
    action: &mut MenuAction,
) {
    let size = 18.0 * s;
    let gap = 4.0 * s;
    let x0 = bar.max.x - 8.0 * s - (size * 3.0 + gap * 2.0);
    let y0 = bar.min.y + (bar.height() - size) * 0.5;
    let slot = |i: usize| {
        Rect::from_min_size(
            Pos2::new(x0 + i as f32 * (size + gap), y0),
            Vec2::new(size, size),
        )
    };

    let dis = play_colors::disabled();
    let buttons: [PlayButton; 3] = match play_mode {
        PlayMode::Edit => [
            PlayButton {
                icon: "play-fill",
                fallback: "\u{25B6}",
                tint: play_colors::play(),
                hover: play_colors::play_hover(),
                tooltip: "Play (F5)",
                action: MenuAction::Play,
            },
            PlayButton {
                icon: "pause-fill",
                fallback: "\u{23F8}",
                tint: dis,
                hover: dis,
                tooltip: "Pause (F6) - not playing",
                action: MenuAction::None,
            },
            PlayButton {
                icon: "stop-fill",
                fallback: "\u{23F9}",
                tint: dis,
                hover: dis,
                tooltip: "Stop (F5) - not playing",
                action: MenuAction::None,
            },
        ],
        PlayMode::Playing => [
            PlayButton {
                icon: "play-fill",
                fallback: "\u{25B6}",
                tint: dis,
                hover: dis,
                tooltip: "Already playing",
                action: MenuAction::None,
            },
            PlayButton {
                icon: "pause-fill",
                fallback: "\u{23F8}",
                tint: play_colors::pause(),
                hover: play_colors::pause_hover(),
                tooltip: "Pause (F6)",
                action: MenuAction::Pause,
            },
            PlayButton {
                icon: "stop-fill",
                fallback: "\u{23F9}",
                tint: play_colors::stop(),
                hover: play_colors::stop_hover(),
                tooltip: "Stop (F5)",
                action: MenuAction::Stop,
            },
        ],
        PlayMode::Paused => [
            PlayButton {
                icon: "skip-forward-fill",
                fallback: "\u{23ED}",
                tint: play_colors::play(),
                hover: play_colors::play_hover(),
                tooltip: "Resume (F6)",
                action: MenuAction::Resume,
            },
            PlayButton {
                icon: "pause-fill",
                fallback: "\u{23F8}",
                tint: dis,
                hover: dis,
                tooltip: "Already paused",
                action: MenuAction::None,
            },
            PlayButton {
                icon: "stop-fill",
                fallback: "\u{23F9}",
                tint: play_colors::stop(),
                hover: play_colors::stop_hover(),
                tooltip: "Stop (F5)",
                action: MenuAction::Stop,
            },
        ],
    };

    for (i, b) in buttons.iter().enumerate() {
        let rect = slot(i);
        if play_icon_button(ui, rect, icons.get(b.icon).copied(), b) && b.action != MenuAction::None
        {
            *action = b.action.clone();
        }
    }
}

struct PlayButton<'a> {
    icon: &'a str,
    fallback: &'a str,
    tint: Color,
    hover: Color,
    tooltip: &'a str,
    action: MenuAction,
}

fn play_icon_button(ui: &mut Ui, rect: Rect, icon: Option<TextureId>, b: &PlayButton) -> bool {
    let id = ui.alloc_id(("menu_play_btn", b.tooltip));
    let resp = ui.interact(id, rect);
    let tint = if resp.hovered { b.hover } else { b.tint };

    if let Some(texture) = icon {
        let inset = 1.0;
        let img = Rect::from_min_max(
            Pos2::new(rect.min.x + inset, rect.min.y + inset),
            Pos2::new(rect.max.x - inset, rect.max.y - inset),
        );
        ui.painter().paint_mut().push(PaintCmd::Image {
            rect: img,
            uv_min: Pos2::new(0.0, 0.0),
            uv_max: Pos2::new(1.0, 1.0),
            tint,
            texture,
        });
    } else {
        let size = 12.0;
        let text_size = ui.text_mut().measure(b.fallback, size, None);
        ui.painter().text(
            Pos2::new(
                rect.center().x - text_size.x * 0.5,
                rect.center().y - text_size.y * 0.5,
            ),
            b.fallback,
            size,
            tint,
            None,
        );
    }

    if resp.hovered {
        show_tooltip_for(ui, rect, b.tooltip);
    }
    resp.clicked
}
