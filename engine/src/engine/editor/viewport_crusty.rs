//! Viewport panel rendered with crusty-gui.
//!
//! One user-approved deviation from the historical reference: the toolbar
//! is a dedicated strip *above* the scene image instead of a floating
//! overlay, so nothing covers the render.

use std::collections::HashMap;

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::input::{Key, Modifiers};
use crusty_gui::math::{Color, Pos2, Rect, Rounding, Vec2};
use crusty_gui::paint::{PaintCmd, TextureId};
use crusty_gui::widgets::{show_tooltip_for, DragValue, Label, Popup, SelectableValue};

use super::viewport::{
    CameraControlMode, EditorCamera, GizmoHandler, GizmoInteractionResult, GizmoOrientation,
    ToolMode, ViewportSettings, GRID_SNAP_VALUES, ROTATION_SNAP_VALUES, SCALE_SNAP_VALUES,
};
use super::{CommandHistory, TransformChangeCommand};
use crate::engine::ecs::components::Transform;
use crate::engine::ecs::resources::PlayMode;
use hecs::{Entity, World};

const TOOLBAR_H: f32 = 36.0;
const BUTTON_W: f32 = 26.0;
const BUTTON_H: f32 = 24.0;
const ICON_PX: f32 = 12.0; // uniform icon size, centered with padding

/// Everything the viewport panel reads/writes.
pub struct ViewportPanelCtx<'a> {
    /// Crusty texture id of the viewport render target (None until ready).
    pub texture: Option<TextureId>,
    /// Output: desired render size in physical pixels (drives texture resize).
    pub viewport_size: &'a mut (u32, u32),
    /// Output: image rect in screen-space pixels (camera input gating).
    pub viewport_rect: &'a mut Rect,
    /// Output: pointer is over the scene image.
    pub viewport_hovered: &'a mut bool,
    pub settings: &'a mut ViewportSettings,
    pub gizmo: &'a mut GizmoHandler,
    pub grid_visible: &'a mut bool,
    pub camera: &'a EditorCamera,
    pub selected: Option<Entity>,
    pub world: &'a mut World,
    pub command_history: &'a mut CommandHistory,
    pub play_mode: PlayMode,
    /// SVG file stem → crusty texture id.
    pub icons: &'a HashMap<String, TextureId>,
    /// The host leaf's tab strip is hidden — the corner triangle occupies
    /// the toolbar's top-left, so its left padding steps 6 → 16px (mockup).
    pub strip_hidden: bool,
    pub show_stat_fps: bool,
    pub fps: f32,
    pub delta_ms: f32,
    /// World-streaming stats; `None` when the scene has no world manifest.
    pub streaming: Option<StreamingOverlay>,
    /// Net connection status line (M5); `None` when no net session exists.
    pub net_status: Option<String>,
}

/// Snapshot of world-streamer state for the stat overlay.
#[derive(Clone, Copy)]
pub struct StreamingOverlay {
    pub cells: usize,
    pub chunks: usize,
    pub in_flight: usize,
    pub ready: usize,
    pub worst_ms: f32,
}

/// Render the viewport panel into `tab_rect` (physical pixels).
pub fn viewport_panel(ui: &mut Ui, tab_rect: Rect, ctx: ViewportPanelCtx) {
    let rect = tab_rect;
    let opts = UiOptions {
        padding: Vec2::ZERO,
        spacing: 0.0,
    };
    ui.run_at(
        rect,
        Direction::TopDown,
        Id::new("engine_viewport_panel"),
        opts,
        |ui| {
            panel_body(ui, rect, 1.0, ctx);
        },
    );
}

fn panel_body(ui: &mut Ui, rect: Rect, s: f32, ctx: ViewportPanelCtx) {
    let toolbar_rect = Rect::from_min_size(rect.min, Vec2::new(rect.width(), TOOLBAR_H * s));
    toolbar(
        ui,
        toolbar_rect,
        s,
        ctx.settings,
        ctx.camera.current_mode(),
        ctx.icons,
        ctx.strip_hidden,
    );

    // ── scene image (raw paint, NOT a widget — camera input must pass through)
    let image_area = Rect::from_min_max(Pos2::new(rect.min.x, toolbar_rect.max.y), rect.max);
    let w = image_area.width().floor().max(1.0);
    let h = image_area.height().floor().max(1.0);
    *ctx.viewport_size = (w as u32, h as u32);
    let image_rect = Rect::from_min_size(
        Pos2::new(image_area.min.x.floor(), image_area.min.y.floor()),
        Vec2::new(w, h),
    );

    if let Some(texture) = ctx.texture {
        ui.painter().paint_mut().push(PaintCmd::Image {
            rect: image_rect,
            uv_min: Pos2::new(0.0, 0.0),
            uv_max: Pos2::new(1.0, 1.0),
            tint: Color::WHITE,
            texture,
        });
    } else {
        placeholder(ui, image_rect, s);
    }

    // Camera input gating reads this rect in physical pixels.
    *ctx.viewport_rect = image_rect;
    let hovered = ui.contains_pointer(image_rect);
    *ctx.viewport_hovered = hovered;

    // ── keyboard shortcuts (blocked while a camera drag is active: WASD flies)
    let camera_dragging = ctx.camera.current_mode() != CameraControlMode::None;
    if hovered && !camera_dragging {
        if key_pressed(ui, 'q') {
            ctx.settings.tool_mode = ToolMode::Select;
        }
        if key_pressed(ui, 'w') {
            ctx.settings.tool_mode = ToolMode::Translate;
        }
        if key_pressed(ui, 'e') {
            ctx.settings.tool_mode = ToolMode::Rotate;
        }
        if key_pressed(ui, 'r') {
            ctx.settings.tool_mode = ToolMode::Scale;
        }
        if key_pressed(ui, 'g') {
            ctx.settings.grid_visible = !ctx.settings.grid_visible;
        }
        if ui.ctx().input.modifiers.contains(Modifiers::CTRL) {
            match ctx.settings.tool_mode {
                ToolMode::Select | ToolMode::Translate => ctx.settings.grid_snap_enabled = true,
                ToolMode::Rotate => ctx.settings.rotation_snap_enabled = true,
                ToolMode::Scale => ctx.settings.scale_snap_enabled = true,
            }
        }
    }

    // ── gizmo (sync + result handling)
    ctx.gizmo.mode = ctx.settings.tool_mode;
    ctx.gizmo.orientation = ctx.settings.gizmo_orientation;
    ctx.gizmo.snap_translate = ctx.settings.snap_translate;
    ctx.gizmo.snap_rotate = ctx.settings.snap_rotate;
    ctx.gizmo.snap_scale = ctx.settings.snap_scale;
    ctx.gizmo.snapping_enabled = ctx.settings.current_snap_enabled();
    *ctx.grid_visible = ctx.settings.grid_visible;

    if ctx.play_mode == PlayMode::Edit && ctx.gizmo.should_show_gizmo() {
        if let Some(entity) = ctx.selected {
            let view = ctx.camera.view_matrix();
            let proj = ctx.camera.projection_matrix_for_gizmo();
            let result = ctx.gizmo.update_crusty(
                ui,
                hovered,
                view,
                proj,
                image_rect,
                Some(entity),
                ctx.world,
            );
            match result {
                GizmoInteractionResult::Transforming {
                    entity,
                    new_transform,
                } => {
                    if let Ok(mut t) = ctx.world.get::<&mut Transform>(entity) {
                        *t = new_transform;
                    }
                    crate::engine::ecs::hierarchy::mark_transform_dirty(ctx.world, entity);
                }
                GizmoInteractionResult::DragEnded {
                    entity,
                    start_transform,
                    end_transform,
                } => {
                    let cmd = TransformChangeCommand::new(entity, &start_transform, &end_transform);
                    ctx.command_history.execute(Box::new(cmd), ctx.world);
                }
                GizmoInteractionResult::None => {}
            }
        }
    }

    // ── play-mode indicator bar over the image top edge
    let status = super::theme::Palette::invariant_status();
    let bar_color = match ctx.play_mode {
        PlayMode::Playing => Some(status.success),
        PlayMode::Paused => Some(status.warning),
        PlayMode::Edit => None,
    };
    if let Some(color) = bar_color {
        let bar = Rect::from_min_size(image_rect.min, Vec2::new(image_rect.width(), 3.0 * s));
        ui.painter().rect_filled(bar, 0.0, color);
    }

    // ── stat fps overlay (Unreal style)
    if ctx.show_stat_fps {
        let padding = 8.0 * s;
        let line_height = 18.0 * s;
        let extra_lines = if ctx.streaming.is_some() { 2.0 } else { 0.0 }
            + if ctx.net_status.is_some() { 1.0 } else { 0.0 };
        let wide = ctx.streaming.is_some() || ctx.net_status.is_some();
        let pos = Pos2::new(image_rect.min.x + padding, image_rect.min.y + padding);
        let bg = Rect::from_min_size(
            pos,
            Vec2::new(
                if wide { 190.0 } else { 120.0 } * s,
                line_height * (2.0 + extra_lines) + padding,
            ),
        );
        let style = ui.style();
        let stat_status = super::theme::Palette::invariant_status();
        ui.painter()
            .rect_filled(bg, 4.0 * s, style.palette.window.with_alpha(0.85));
        let fps_color = if ctx.fps >= 60.0 {
            stat_status.success
        } else if ctx.fps >= 30.0 {
            stat_status.warning
        } else {
            stat_status.error
        };
        ui.painter().text(
            Pos2::new(pos.x + padding * 0.5, pos.y + padding * 0.5),
            &format!("FPS: {:.1}", ctx.fps),
            13.0 * s,
            fps_color,
            None,
        );
        ui.painter().text(
            Pos2::new(pos.x + padding * 0.5, pos.y + padding * 0.5 + line_height),
            &format!("Frame: {:.2} ms", ctx.delta_ms),
            13.0 * s,
            style.palette.text,
            None,
        );
        if let Some(st) = &ctx.streaming {
            ui.painter().text(
                Pos2::new(
                    pos.x + padding * 0.5,
                    pos.y + padding * 0.5 + line_height * 2.0,
                ),
                &format!("Stream: {} cells, {} chunks", st.cells, st.chunks),
                13.0 * s,
                style.palette.text,
                None,
            );
            ui.painter().text(
                Pos2::new(
                    pos.x + padding * 0.5,
                    pos.y + padding * 0.5 + line_height * 3.0,
                ),
                &format!(
                    "IO: {} in-flight, {} ready, {:.2} ms worst",
                    st.in_flight, st.ready, st.worst_ms
                ),
                13.0 * s,
                style.palette.text,
                None,
            );
        }
        if let Some(net) = &ctx.net_status {
            let line = if ctx.streaming.is_some() { 4.0 } else { 2.0 };
            ui.painter().text(
                Pos2::new(
                    pos.x + padding * 0.5,
                    pos.y + padding * 0.5 + line_height * line,
                ),
                net,
                13.0 * s,
                style.palette.text,
                None,
            );
        }
    }
}

/// `Key::Char` may arrive upper- or lowercase depending on shift state.
fn key_pressed(ui: &Ui, c: char) -> bool {
    let input = &ui.ctx().input;
    input.key_pressed(Key::Char(c)) || input.key_pressed(Key::Char(c.to_ascii_uppercase()))
}

fn placeholder(ui: &mut Ui, rect: Rect, s: f32) {
    let style = ui.style();
    let center = rect.center();
    let heading = "3D Viewport";
    let hw = ui.painter().measure_text(heading, 20.0 * s, None).x;
    ui.painter().text(
        Pos2::new(center.x - hw * 0.5, center.y - 40.0 * s),
        heading,
        20.0 * s,
        style.palette.text,
        None,
    );
    let size_text = format!("Size: {:.0} x {:.0}", rect.width(), rect.height());
    let sw = ui.painter().measure_text(&size_text, 14.0 * s, None).x;
    ui.painter().text(
        Pos2::new(center.x - sw * 0.5, center.y - 12.0 * s),
        &size_text,
        14.0 * s,
        style.palette.text_secondary,
        None,
    );
    let init = "Initializing viewport texture...";
    let iw = ui.painter().measure_text(init, 14.0 * s, None).x;
    ui.painter().text(
        Pos2::new(center.x - iw * 0.5, center.y + 14.0 * s),
        init,
        14.0 * s,
        style.palette.text_disabled,
        None,
    );
}

// ── toolbar ─────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn toolbar(
    ui: &mut Ui,
    rect: Rect,
    s: f32,
    settings: &mut ViewportSettings,
    camera_mode: CameraControlMode,
    icons: &HashMap<String, TextureId>,
    strip_hidden: bool,
) {
    let panel_fill = ui.style().palette.panel;
    ui.painter().rect_filled(rect, 0.0, panel_fill);

    // Step past the show-tabs corner triangle when the strip is hidden.
    let left_pad = if strip_hidden { 16.0 } else { 6.0 };
    let mut x = rect.min.x + left_pad * s;
    let y = rect.min.y + (rect.height() - BUTTON_H * s) * 0.5;

    // Transform tools
    let tools = [
        (ToolMode::Select, "select", "Q", "Select (Q)"),
        (ToolMode::Translate, "translate", "W", "Translate (W)"),
        (ToolMode::Rotate, "rotate", "E", "Rotate (E)"),
        (ToolMode::Scale, "scale", "R", "Scale (R)"),
    ];
    for (mode, stem, fallback, tooltip) in tools {
        let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(BUTTON_W * s, BUTTON_H * s));
        let selected = settings.tool_mode == mode;
        if pill_button(
            ui,
            r,
            s,
            icons.get(stem).copied(),
            fallback,
            selected,
            tooltip,
        ) {
            settings.tool_mode = mode;
        }
        x = r.max.x + 2.0 * s;
    }

    x = separator(ui, rect, x, s);

    // World/local orientation toggle
    let is_world = settings.gizmo_orientation == GizmoOrientation::World;
    let (stem, fallback, tooltip) = if is_world {
        ("world", "G", "World space (click for Local)")
    } else {
        ("local", "L", "Local space (click for World)")
    };
    let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(BUTTON_W * s, BUTTON_H * s));
    if pill_button(ui, r, s, icons.get(stem).copied(), fallback, false, tooltip) {
        settings.gizmo_orientation = if is_world {
            GizmoOrientation::Local
        } else {
            GizmoOrientation::World
        };
    }
    x = r.max.x + 2.0 * s;

    x = separator(ui, rect, x, s);

    // Snap split-buttons
    let (clicked, next_x) = snap_split_button(
        ui,
        Pos2::new(x, y),
        s,
        icons.get("grid-snap").copied(),
        "▦",
        settings.grid_snap_enabled,
        &format_snap_value(settings.snap_translate),
        "Grid snapping",
        Id::new("vp_grid_snap"),
        100.0 * s,
        GRID_SNAP_VALUES,
        &mut settings.snap_translate,
        format_snap_value,
    );
    if clicked {
        settings.grid_snap_enabled = !settings.grid_snap_enabled;
    }
    x = next_x;

    let (clicked, next_x) = snap_split_button(
        ui,
        Pos2::new(x, y),
        s,
        icons.get("rotation-snap").copied(),
        "∠",
        settings.rotation_snap_enabled,
        &format!("{}°", settings.snap_rotate as i32),
        "Rotation snapping",
        Id::new("vp_rotation_snap"),
        80.0 * s,
        ROTATION_SNAP_VALUES,
        &mut settings.snap_rotate,
        |v| format!("{}°", v as i32),
    );
    if clicked {
        settings.rotation_snap_enabled = !settings.rotation_snap_enabled;
    }
    x = next_x;

    let (clicked, next_x) = snap_split_button(
        ui,
        Pos2::new(x, y),
        s,
        icons.get("scale-snap").copied(),
        "⊞",
        settings.scale_snap_enabled,
        &format_snap_value(settings.snap_scale),
        "Scale snapping",
        Id::new("vp_scale_snap"),
        100.0 * s,
        SCALE_SNAP_VALUES,
        &mut settings.snap_scale,
        format_snap_value,
    );
    if clicked {
        settings.scale_snap_enabled = !settings.scale_snap_enabled;
    }
    let _ = next_x;

    // Right cluster (mockup): camera speed lives at the right edge, with the
    // camera-mode indicator to its left.
    let speed_w = 60.0 * s;
    let speed_x = rect.max.x - 8.0 * s - speed_w;
    camera_speed_button(
        ui,
        Pos2::new(speed_x, y),
        s,
        icons.get("camera-speed").copied(),
        settings,
    );

    // Camera mode indicator, left of the speed cluster
    let cam_status = super::theme::Palette::invariant_status();
    let cam_types = super::theme::Palette::invariant_type_colors();
    let (text, color) = match camera_mode {
        CameraControlMode::Fly => (
            format!("Fly {:.1}x", settings.camera_speed),
            cam_status.success,
        ),
        CameraControlMode::Orbit => ("Orbit".to_string(), cam_types.cameras),
        CameraControlMode::Pan => ("Pan".to_string(), cam_types.vfx),
        CameraControlMode::LookDrag => ("Look".to_string(), cam_types.materials),
        CameraControlMode::None => return,
    };
    let font = 11.0 * s;
    let tw = ui.painter().measure_text(&text, font, None).x;
    ui.painter().text(
        Pos2::new(
            speed_x - 8.0 * s - tw,
            rect.center().y - font * 1.25 * 0.5,
        ),
        &text,
        font,
        color,
        None,
    );
}

/// Vertical separator line; returns the next cursor x.
fn separator(ui: &mut Ui, toolbar_rect: Rect, x: f32, s: f32) -> f32 {
    let stroke = ui.style().palette.stroke;
    let line_x = x + 4.0 * s;
    ui.painter().line_segment(
        Pos2::new(line_x, toolbar_rect.min.y + 10.0 * s),
        Pos2::new(line_x, toolbar_rect.max.y - 10.0 * s),
        1.0,
        stroke,
    );
    x + 13.0 * s
}

/// Toolbar icon button (26×24, 3px radius). Toggled-on state uses the
/// accent-soft fill with an accent border and accent icon per the design
/// system; at rest the button is flat on the panel.
fn pill_button(
    ui: &mut Ui,
    rect: Rect,
    s: f32,
    icon: Option<TextureId>,
    fallback: &str,
    selected: bool,
    tooltip: &str,
) -> bool {
    let style = ui.style();
    let id = ui.alloc_id(("vp_toolbar_btn", tooltip));
    let resp = ui.interact(id, rect);
    let rounding = style.rounding.small;
    if selected {
        ui.painter()
            .rect_filled(rect, rounding, style.palette.accent_soft);
        ui.painter()
            .rect_stroke(rect, rounding, 1.0, style.palette.accent_active);
    } else if resp.hovered {
        ui.painter().rect_filled(rect, rounding, style.palette.hover);
    }

    let tint = if selected {
        style.palette.accent_active
    } else if resp.hovered {
        style.palette.text
    } else {
        style.palette.text_secondary
    };
    paint_icon_centered(ui, rect, s, icon, fallback, tint);

    if resp.hovered {
        show_tooltip_for(ui, rect, tooltip);
    }
    resp.clicked
}

fn paint_icon_centered(
    ui: &mut Ui,
    rect: Rect,
    s: f32,
    icon: Option<TextureId>,
    fallback: &str,
    tint: Color,
) {
    if let Some(texture) = icon {
        let size = ICON_PX * s;
        let c = rect.center();
        let irect = Rect::from_min_size(
            Pos2::new((c.x - size * 0.5).round(), (c.y - size * 0.5).round()),
            Vec2::splat(size),
        );
        ui.painter().paint_mut().push(PaintCmd::Image {
            rect: irect,
            uv_min: Pos2::new(0.0, 0.0),
            uv_max: Pos2::new(1.0, 1.0),
            tint,
            texture,
        });
    } else {
        let font = 12.0 * s;
        let size = ui.painter().measure_text(fallback, font, None);
        let c = rect.center();
        ui.painter().text(
            Pos2::new(c.x - size.x * 0.5, c.y - font * 1.25 * 0.5),
            fallback,
            font,
            tint,
            None,
        );
    }
}

/// Snap split-button: icon zone toggles the snap, dropdown zone opens a value
/// menu. Returns (icon_clicked, next cursor x).
#[allow(clippy::too_many_arguments)]
fn snap_split_button(
    ui: &mut Ui,
    pos: Pos2,
    s: f32,
    icon: Option<TextureId>,
    fallback: &str,
    enabled: bool,
    value_text: &str,
    tooltip: &str,
    id: Id,
    popup_width: f32,
    values: &[f32],
    current: &mut f32,
    fmt: impl Fn(f32) -> String,
) -> (bool, f32) {
    let (total_rect, icon_rect, dropdown_rect, icon_clicked, dd_clicked) =
        split_button_chrome(ui, pos, s, icon, fallback, enabled, value_text, tooltip, id);

    popup(ui, id, total_rect, popup_width, dd_clicked, |ui| {
        for &value in values {
            SelectableValue::new(current, value, fmt(value)).show(ui);
        }
    });

    let _ = (icon_rect, dropdown_rect);
    (icon_clicked, total_rect.max.x + 2.0 * s)
}

/// Shared chrome for split buttons: pill background, enabled highlight,
/// separator, centered icon, value text, caret, hover strokes + tooltip.
/// Returns (total, icon zone, dropdown zone, icon clicked, dropdown clicked).
#[allow(clippy::too_many_arguments)]
fn split_button_chrome(
    ui: &mut Ui,
    pos: Pos2,
    s: f32,
    icon: Option<TextureId>,
    fallback: &str,
    enabled: bool,
    value_text: &str,
    tooltip: &str,
    id: Id,
) -> (Rect, Rect, Rect, bool, bool) {
    let icon_w = 24.0 * s;
    let dropdown_w = 36.0 * s;
    let h = BUTTON_H * s;
    let total_rect = Rect::from_min_size(pos, Vec2::new(icon_w + dropdown_w, h));
    let icon_rect = Rect::from_min_size(pos, Vec2::new(icon_w, h));
    let dropdown_rect =
        Rect::from_min_size(Pos2::new(pos.x + icon_w, pos.y), Vec2::new(dropdown_w, h));

    let icon_resp = ui.interact(id.with("icon"), icon_rect);
    let dd_resp = ui.interact(id.with("dd"), dropdown_rect);

    let style = ui.style();
    let r = style.rounding.small.nw;
    let rounding = Rounding::same(r);
    ui.painter()
        .rect_filled(total_rect, rounding, style.palette.header);
    if enabled {
        ui.painter().rect_filled(
            icon_rect,
            Rounding {
                nw: r,
                sw: r,
                ne: 0.0,
                se: 0.0,
            },
            style.palette.accent_soft,
        );
    }
    ui.painter().line_segment(
        Pos2::new(icon_rect.max.x, icon_rect.min.y + 4.0 * s),
        Pos2::new(icon_rect.max.x, icon_rect.max.y - 4.0 * s),
        1.0,
        style.palette.stroke,
    );

    let tint = if enabled {
        style.palette.accent_active
    } else {
        style.palette.text_secondary
    };
    paint_icon_centered(ui, icon_rect, s, icon, fallback, tint);

    // Value text centered in the dropdown zone (nudged left of the caret).
    let font = 11.0 * s;
    let tw = ui.painter().measure_text(value_text, font, None).x;
    let tc = dropdown_rect.center();
    ui.painter().text(
        Pos2::new(tc.x - 4.0 * s - tw * 0.5, tc.y - font * 1.25 * 0.5),
        value_text,
        font,
        style.palette.text,
        None,
    );

    // Caret
    let cc = Pos2::new(dropdown_rect.max.x - 6.0 * s, dropdown_rect.center().y);
    let cs = 3.0 * s;
    ui.painter().triangle(
        Pos2::new(cc.x - cs, cc.y - cs * 0.5),
        Pos2::new(cc.x + cs, cc.y - cs * 0.5),
        Pos2::new(cc.x, cc.y + cs * 0.5),
        style.palette.text_secondary,
    );

    if icon_resp.hovered {
        ui.painter().rect_stroke(
            icon_rect,
            Rounding {
                nw: r,
                sw: r,
                ne: 0.0,
                se: 0.0,
            },
            1.0,
            style.palette.stroke_strong,
        );
        show_tooltip_for(ui, icon_rect, tooltip);
    }
    if dd_resp.hovered {
        ui.painter().rect_stroke(
            dropdown_rect,
            Rounding {
                nw: 0.0,
                sw: 0.0,
                ne: r,
                se: r,
            },
            1.0,
            style.palette.stroke_strong,
        );
    }

    (
        total_rect,
        icon_rect,
        dropdown_rect,
        icon_resp.clicked,
        dd_resp.clicked,
    )
}

/// Camera speed split-button with an Unreal-style popup (log slider + scalar).
fn camera_speed_button(
    ui: &mut Ui,
    pos: Pos2,
    s: f32,
    icon: Option<TextureId>,
    settings: &mut ViewportSettings,
) {
    let id = Id::new("vp_camera_speed");
    let value_text = format!("{:.2}", settings.camera_speed);
    let (total_rect, _, _, _, dd_clicked) = split_button_chrome(
        ui,
        pos,
        s,
        icon,
        "📷",
        false,
        &value_text,
        "Camera speed",
        id,
    );

    let camera_speed = &mut settings.camera_speed;
    let camera_speed_scalar = &mut settings.camera_speed_scalar;
    popup(ui, id, total_rect, 180.0 * s, dd_clicked, |ui| {
        let style = ui.style();
        Label::new("CAMERA SPEED")
            .size(10.0 * s)
            .color(style.palette.text_disabled)
            .show(ui);
        let sep = ui.allocate(Vec2::new(ui.available().width(), 5.0 * s));
        ui.painter().line_segment(
            Pos2::new(sep.min.x, sep.center().y),
            Pos2::new(sep.max.x, sep.center().y),
            1.0,
            style.palette.stroke,
        );

        ui.horizontal(|ui| {
            Label::new("Camera Speed").size(12.0 * s).show(ui);
            DragValue::new(camera_speed)
                .range(0.03..=8.0)
                .speed(0.01)
                .decimals(2)
                .width(52.0 * s)
                .show(ui);
        });

        // Logarithmic slider.
        let slider_rect = ui.allocate(Vec2::new(ui.available().width(), 16.0 * s));
        let resp = ui.interact(Id::new("vp_camera_speed_slider"), slider_rect);
        let track = Rect::from_min_max(
            Pos2::new(
                slider_rect.min.x + 4.0 * s,
                slider_rect.center().y - 2.0 * s,
            ),
            Pos2::new(
                slider_rect.max.x - 4.0 * s,
                slider_rect.center().y + 2.0 * s,
            ),
        );
        let (log_min, log_max) = (0.03_f32.ln(), 8.0_f32.ln());
        if resp.pressed {
            if let Some(p) = ui.ctx().input.pointer_pos {
                let t = ((p.x - track.min.x) / track.width()).clamp(0.0, 1.0);
                *camera_speed = (log_min + t * (log_max - log_min)).exp();
            }
        }
        ui.painter().rect_filled(track, 2.0 * s, style.palette.header);
        let t = ((camera_speed.ln() - log_min) / (log_max - log_min)).clamp(0.0, 1.0);
        let handle_color = if resp.pressed {
            style.palette.accent_active
        } else if resp.hovered {
            style.palette.text_secondary
        } else {
            style.palette.text_disabled
        };
        ui.painter().circle_filled(
            Pos2::new(track.min.x + t * track.width(), slider_rect.center().y),
            6.0 * s,
            handle_color,
        );

        ui.horizontal(|ui| {
            Label::new("Speed Scalar").size(12.0 * s).show(ui);
            DragValue::new(camera_speed_scalar)
                .range(0.1..=10.0)
                .speed(0.1)
                .decimals(1)
                .width(52.0 * s)
                .show(ui);
        });
    });
}

/// Anchored dropdown popup: toggle on trigger click, then render if open.
fn popup(ui: &mut Ui, id: Id, anchor: Rect, width: f32, toggle: bool, body: impl FnOnce(&mut Ui)) {
    if toggle {
        Popup::toggle(ui, id);
    }
    Popup::new(id, anchor, width).show(ui, body);
}

/// Format snap value for display (integers cleanly, trims trailing zeros).
fn format_snap_value(value: f32) -> String {
    if value >= 1.0 && value == value.floor() {
        format!("{}", value as i32)
    } else if value >= 0.1 {
        format!("{:.2}", value)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    } else {
        format!("{}", value)
    }
}
