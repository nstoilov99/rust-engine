//! Menu bar rendered with crusty-gui.

use std::collections::HashMap;

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::math::{Color, Pos2, Rect, Vec2};
use crusty_gui::paint::{PaintCmd, TextureId};
use crusty_gui::text::FontFamily;
use crusty_gui::widgets::{
    show_tooltip_for, Button, ComboBox, Label, Popup, SelectableValue, TextEdit,
};

use super::build_dialog::{BuildDialog, BuildProfile, BuildState, BuildTarget};
use super::console::ConsoleLog;
use super::dock_crusty::CrustyDockLayout;
use super::menu_bar::MenuAction;
use super::play_settings::{NetPlayMode, PlaySettings};
use super::theme::EditorTheme;
use super::{CommandHistory, EditorTab};
use crate::engine::ecs::resources::PlayMode;

/// Overrides the Edit menu's history + clipboard state when a document editor
/// (e.g. a graph tab) owns the active edit target instead of the scene
/// (Task 40 P5). When `Some`, Undo/Redo labels and the Cut/Copy/Paste/Delete
/// enablement read from here; the actions still flow through `MenuAction` and
/// are routed to the focused editor by the host.
#[derive(Clone)]
pub struct EditMenuOverride {
    pub undo_label: String,
    pub can_undo: bool,
    pub redo_label: String,
    pub can_redo: bool,
    /// Node selection — gates Cut/Copy/Duplicate (clipboard is node-only).
    pub has_selection: bool,
    /// Anything deletable is selected (nodes *or* a comment/group) — gates
    /// Delete, which handles annotations too.
    pub has_deletable: bool,
    pub has_clipboard: bool,
}

pub struct MenuBarCtx<'a> {
    pub dock_state: &'a mut CrustyDockLayout,
    pub command_history: &'a CommandHistory,
    /// Active document-editor edit target (graph tab), when one has focus.
    pub edit_override: Option<EditMenuOverride>,
    pub play_mode: PlayMode,
    pub build_dialog: &'a mut BuildDialog,
    pub console_messages: &'a mut ConsoleLog,
    pub show_benchmark_tools: bool,
    pub icons: &'a HashMap<String, TextureId>,
    pub theme: &'a EditorTheme,
    pub has_selection: bool,
    pub has_clipboard: bool,
    pub play_settings: &'a mut PlaySettings,
    /// Active scene name for the centered "<scene> — Crusty" identity.
    pub scene_name: &'a str,
}

/// Brighten a status color for the hovered play-control state.
fn hover_tint(c: Color) -> Color {
    let t = 0.35;
    Color::rgba(
        c.r + (1.0 - c.r) * t,
        c.g + (1.0 - c.g) * t,
        c.b + (1.0 - c.b) * t,
        c.a,
    )
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
        edit_override,
        play_mode,
        build_dialog,
        show_benchmark_tools,
        icons,
        theme,
        has_selection,
        has_clipboard,
        play_settings,
        scene_name,
        ..
    } = ctx;

    // Opaque strip + centered "<scene> — Crusty" identity (mono, tertiary),
    // per the mockup menu-bar row.
    let strip_bg = ui.style().palette.window;
    ui.painter().rect_filled(rect, 0.0, strip_bg);
    {
        let text = format!("{scene_name} \u{2014} Crusty");
        let size = 11.0;
        let tw = ui
            .painter()
            .measure_text_family(&text, size, None, FontFamily::Mono)
            .x;
        let color = ui.style().palette.text_disabled;
        ui.painter().text_family(
            Pos2::new(
                rect.center().x - tw * 0.5,
                rect.center().y - size * 1.25 * 0.5,
            ),
            &text,
            size,
            color,
            None,
            FontFamily::Mono,
        );
    }

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
            // 16px logo mark ahead of the menus.
            if let Some(&tex) = icons.get("logo") {
                let slot = ui.allocate(Vec2::new(24.0, rect.height()));
                let tint = ui.style().palette.text_secondary;
                ui.ctx_mut().paint.push(PaintCmd::Image {
                    rect: Rect::from_center_size(slot.center(), Vec2::splat(16.0)),
                    uv_min: Pos2::new(0.0, 0.0),
                    uv_max: Pos2::new(1.0, 1.0),
                    tint,
                    texture: tex,
                });
            }
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
                    build_game_menu(ui, build_dialog, theme);
                });
                ui.separator();
                if ui.menu_item("Exit") {
                    action = MenuAction::Exit;
                }
            });

            let is_edit_mode = play_mode == PlayMode::Edit;
            ui.menu_button_width("Edit", 250.0, |ui| {
                // A focused document editor (graph tab) supplies its own
                // history + clipboard state; otherwise use the scene's.
                let (undo_desc, can_undo, redo_desc, can_redo, sel, del, clip) = match &edit_override
                {
                    Some(o) => (
                        Some(o.undo_label.clone()),
                        o.can_undo,
                        Some(o.redo_label.clone()),
                        o.can_redo,
                        o.has_selection,
                        o.has_deletable,
                        o.has_clipboard,
                    ),
                    None => (
                        command_history.undo_description().map(|d| format!("Undo {d}")),
                        is_edit_mode && command_history.can_undo(),
                        command_history.redo_description().map(|d| format!("Redo {d}")),
                        is_edit_mode && command_history.can_redo(),
                        is_edit_mode && has_selection,
                        is_edit_mode && has_selection,
                        is_edit_mode && has_clipboard,
                    ),
                };
                ui.menu_group_header("History");
                let undo_label = undo_desc.unwrap_or_else(|| "Undo".to_string());
                if ui.menu_item_shortcut(undo_label, "Ctrl+Z", can_undo) {
                    action = MenuAction::Undo;
                }
                let redo_label = redo_desc.unwrap_or_else(|| "Redo".to_string());
                if ui.menu_item_shortcut(redo_label, "Ctrl+Y", can_redo) {
                    action = MenuAction::Redo;
                }
                // Ships disabled until the undo-history browser lands (post-M10).
                let _ = ui.menu_item_enabled("Undo History\u{2026}", false);

                ui.separator();
                ui.menu_group_header("Edit");
                if ui.menu_item_shortcut("Cut", "Ctrl+X", sel) {
                    action = MenuAction::Cut;
                }
                if ui.menu_item_shortcut("Copy", "Ctrl+C", sel) {
                    action = MenuAction::Copy;
                }
                if ui.menu_item_shortcut("Paste", "Ctrl+V", clip) {
                    action = MenuAction::Paste;
                }
                if ui.menu_item_shortcut("Duplicate", "Ctrl+D", sel) {
                    action = MenuAction::Duplicate;
                }
                if ui.menu_item_shortcut("Delete", "Del", del) {
                    action = MenuAction::Delete;
                }

                ui.separator();
                ui.menu_group_header("Configuration");
                if ui.menu_item("Editor Preferences\u{2026}") {
                    action = MenuAction::OpenEditorPreferences;
                }
                let _ = ui.menu_item_enabled("Keyboard Shortcuts\u{2026}", false);
                if ui.menu_item("Project Settings\u{2026}") {
                    action = MenuAction::OpenProjectSettings;
                }
                let _ = ui.menu_item_enabled("Plugins", false);
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
                if ui.menu_item("Stream Around Camera") {
                    action = MenuAction::ToggleStreamAroundCamera;
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
                let dim = ui.style().palette.text_secondary;
                Label::new("Rust Game Engine").color(dim).show(ui);
            });

            render_play_controls(
                ui,
                rect,
                s,
                play_mode,
                icons,
                theme,
                play_settings,
                &mut action,
            );
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
    let dim = style.palette.text_secondary;
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
fn build_game_menu(ui: &mut Ui, build_dialog: &mut BuildDialog, theme: &EditorTheme) {
    let is_building = matches!(
        build_dialog.state,
        BuildState::Building | BuildState::CopyingContent
    );

    build_row(ui, "Platform:", |ui, _| {
        build_row_value(ui, build_dialog.settings.platform.label());
    });

    let target = &mut build_dialog.settings.target;
    build_row(ui, "Target:", |ui, w| {
        if is_building {
            build_row_value(ui, target.label());
        } else {
            ComboBox::new("build_target_menu")
                .selected_text(target.label())
                .width(w)
                .show_ui(ui, |ui| {
                    for t in [
                        BuildTarget::Standalone,
                        BuildTarget::MpClient,
                        BuildTarget::MpServer,
                    ] {
                        SelectableValue::new(target, t, t.label()).show(ui);
                    }
                });
        }
    });
    let target = build_dialog.settings.target;

    // MP Server publishes the WASM module — no exe profile or output dir.
    if target != BuildTarget::MpServer {
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
    }

    if target != BuildTarget::Standalone {
        let server_uri = &mut build_dialog.settings.server_uri;
        build_row(ui, "Server:", |ui, w| {
            if is_building {
                build_row_value(ui, server_uri);
            } else {
                TextEdit::new(server_uri).width(w).show(ui);
            }
        });

        let module = &mut build_dialog.settings.module;
        build_row(ui, "Module:", |ui, w| {
            if is_building {
                build_row_value(ui, module);
            } else {
                TextEdit::new(module).width(w).show(ui);
            }
        });
    }

    ui.separator();

    match &build_dialog.state {
        BuildState::Idle => {
            let label = if target == BuildTarget::MpServer { "Publish" } else { "Build" };
            if Button::new(label).show(ui).clicked {
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
            .color(theme.palette.status.success)
            .show(ui);
            if Button::new("Build Again").show(ui).clicked {
                build_dialog.start_build();
            }
        }
        BuildState::Failed { error } => {
            Label::new(format!("Failed: {}", error))
                .color(theme.palette.status.error)
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
    theme: &EditorTheme,
    settings: &mut PlaySettings,
    action: &mut MenuAction,
) {
    // Mockup transport: three 26×22 buttons + 20×22 settings caret.
    let _ = s;
    let (bw, bh, gap, caret_w) = (26.0, 22.0, 2.0, 20.0);
    let x0 = bar.max.x - 8.0 - (bw * 3.0 + gap * 3.0 + caret_w);
    let y0 = bar.min.y + (bar.height() - bh) * 0.5;
    let slot = |i: usize| {
        let w = if i == 3 { caret_w } else { bw };
        Rect::from_min_size(
            Pos2::new(x0 + i as f32 * (bw + gap), y0),
            Vec2::new(w, bh),
        )
    };

    let dis = ui.style().palette.text_disabled;
    let play = theme.palette.status.success;
    let pause = theme.palette.status.warning;
    let stop = theme.palette.status.error;
    let buttons: [PlayButton; 3] = match play_mode {
        PlayMode::Edit => [
            PlayButton {
                icon: "play-fill",
                fallback: "\u{25B6}",
                tint: play,
                hover: hover_tint(play),
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
                tint: pause,
                hover: hover_tint(pause),
                tooltip: "Pause (F6)",
                action: MenuAction::Pause,
            },
            PlayButton {
                icon: "stop-fill",
                fallback: "\u{23F9}",
                tint: stop,
                hover: hover_tint(stop),
                tooltip: "Stop (F5)",
                action: MenuAction::Stop,
            },
        ],
        PlayMode::Paused => [
            PlayButton {
                icon: "skip-forward-fill",
                fallback: "\u{23ED}",
                tint: play,
                hover: hover_tint(play),
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
                tint: stop,
                hover: hover_tint(stop),
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

    render_play_settings_dropdown(ui, slot(3), play_mode, icons, settings);
}

/// Chevron trigger + anchored popup with the net-play settings (M9.6).
/// Editable only in edit mode; while playing the values render read-only.
fn render_play_settings_dropdown(
    ui: &mut Ui,
    rect: Rect,
    play_mode: PlayMode,
    icons: &HashMap<String, TextureId>,
    settings: &mut PlaySettings,
) {
    let chevron = PlayButton {
        icon: "caret-down",
        fallback: "\u{25BE}",
        tint: ui.style().palette.text_secondary,
        hover: ui.style().palette.text,
        tooltip: "Play settings",
        action: MenuAction::None,
    };
    let popup_id = ui.alloc_id("play_settings_popup");
    if play_icon_button(ui, rect, icons.get(chevron.icon).copied(), &chevron) {
        Popup::toggle(ui, popup_id);
    }

    let locked = play_mode != PlayMode::Edit;
    Popup::new(popup_id, rect, 240.0).show(ui, |ui| {
        build_row(ui, "Mode:", |ui, w| {
            if locked {
                build_row_value(ui, settings.mode.label());
            } else {
                ComboBox::new("play_mode_combo")
                    .selected_text(settings.mode.label())
                    .width(w)
                    .show_ui(ui, |ui| {
                        for m in [
                            NetPlayMode::Standalone,
                            NetPlayMode::Client,
                            NetPlayMode::ListenServer,
                        ] {
                            SelectableValue::new(&mut settings.mode, m, m.label()).show(ui);
                        }
                    });
            }
        });

        if settings.mode != NetPlayMode::Standalone {
            build_row(ui, "Host:", |ui, w| {
                if locked {
                    build_row_value(ui, &settings.host);
                } else {
                    TextEdit::new(&mut settings.host).width(w).show(ui);
                }
            });
            build_row(ui, "Module:", |ui, w| {
                if locked {
                    build_row_value(ui, &settings.module);
                } else {
                    TextEdit::new(&mut settings.module).width(w).show(ui);
                }
            });
            build_row(ui, "Players:", |ui, w| {
                if locked {
                    build_row_value(ui, &settings.player_count.to_string());
                } else {
                    ComboBox::new("play_players_combo")
                        .selected_text(settings.player_count.to_string())
                        .width(w)
                        .show_ui(ui, |ui| {
                            for n in 1..=4u8 {
                                SelectableValue::new(
                                    &mut settings.player_count,
                                    n,
                                    n.to_string(),
                                )
                                .show(ui);
                            }
                        });
                }
            });
        }
    });
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
        // 11px glyph centered in the slot, per the mockup transport.
        let img = Rect::from_center_size(rect.center(), Vec2::splat(11.0));
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
