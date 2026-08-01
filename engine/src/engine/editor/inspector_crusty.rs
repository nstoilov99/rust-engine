//! Inspector panel rendered with crusty-gui.
//!
//! Reads/writes [`InspectorPanel`] state (see `inspector_panel.rs`) and
//! world components. Intentional deviations from the historical reference
//! (user-approved):
//! - Sliders use crusty-gui's slider design.
//! - Color fields use crusty-gui's `ColorPicker` (Unreal/Unity/Godot style
//!   popup with cached swatches).
//! - Asset slots are searchable dropdowns (no drag-and-drop / thumbnails).

use crusty_gui::context::Ui;
use crusty_gui::id::Id;
use crusty_gui::math::{Color, Pos2, Rect, Vec2};
use crusty_gui::paint::TextureId;
use crusty_gui::widgets::{
    Button, Checkbox, CollapsingHeader, ColorPicker, ComboBox, DragValue, EyedropperState, Label,
    ScrollArea, SelectableValue, Slider, TextEdit,
};
use hecs::{Entity, World};
use nalgebra_glm as glm;

use super::asset_browser::{AssetBrowserPanel, AssetFilter};
use super::inspector_panel::{
    clean_display_degrees, euler_degrees_to_quaternion, quaternion_to_euler_degrees,
    quaternions_approximately_equal, ComponentPresence, InspectorPanel,
};
use super::Selection;
use crate::engine::animation::{AnimationPlayer, PlaybackState, SkeletonInstance};
use crate::engine::assets::asset_type::AssetType;
use crate::engine::audio::{AudioBus, AudioEmitter, AudioListener};
use crate::engine::ecs::resources::PlayMode;
use crate::engine::ecs::{
    Camera, CameraProjection, DirectionalLight, LightFalloff, MeshRenderer, Name, ParticleEffect,
    PointLight, SpawnShape, Transform, UpdateModule,
};
use crate::engine::physics::{Collider, ColliderShape, RigidBody, RigidBodyType, StaticCollision};

/// Section accent stripes: the 2px left bar is one of DESIGN.md's named
/// **deep**-tone jobs ("inspector section left bars"), resolved through the
/// assets map on the component's category slug (Transform = geometry,
/// Mesh Renderer = materials, …). Preset-invariant, index-based.
fn cat(kind: &str) -> Color {
    super::theme::asset_deep_color(kind)
}

const PROP_LABEL_W: f32 = 90.0;
const VEC3_LABEL_W: f32 = 52.0;
const VEC3_RESET_W: f32 = 20.0;
const VEC3_INPUT_W: f32 = 92.0;
// Numeric-field height and 26px row pitch.
const FIELD_H: f32 = 22.0;

/// Borrowed inspector state.
pub struct InspectorPanelCtx<'a> {
    pub panel: &'a mut InspectorPanel,
    pub world: &'a mut World,
    pub selection: &'a Selection,
    pub play_mode: PlayMode,
    pub asset_browser: &'a mut AssetBrowserPanel,
    /// GPU-uploaded icon set (same map as the hierarchy panel); the color
    /// pickers use the `color-picker` stem for the eyedropper button.
    pub icons: &'a std::collections::HashMap<String, TextureId>,
    /// Streamed-world object info, shown read-only when the world row is
    /// selected in the hierarchy.
    pub world_object: Option<&'a super::world_object::WorldObjectInfo>,
}

/// Shared color-picker context threaded to every color row: the saved
/// swatches, the desktop eyedropper bridge and the pipette icon.
struct Picker<'a> {
    swatches: &'a mut Vec<Color>,
    eye: &'a mut EyedropperState,
    icon: Option<TextureId>,
}

/// Deferred component removals.
enum ComponentAction {
    RemoveCamera,
    RemoveMeshRenderer,
    RemoveDirectionalLight,
    RemovePointLight,
    RemoveRigidBody,
    RemoveCollider,
    RemoveAudioEmitter,
    RemoveAudioListener,
    RemoveParticleEffect,
    RemoveStaticCollision,
}

/// Draw the inspector into the dock tab's content rect (physical pixels).
pub fn inspector_panel(ui: &mut Ui, tab_rect: Rect, ctx: InspectorPanelCtx) {
    let rect = tab_rect;
    let opts = crusty_gui::context::UiOptions {
        padding: Vec2::new(0.0, 4.0),
        spacing: 4.0,
    };
    ui.run_at(
        rect,
        crusty_gui::context::Direction::TopDown,
        Id::new("engine_inspector_panel"),
        opts,
        |ui| {
            let InspectorPanelCtx {
                panel,
                world,
                selection,
                play_mode,
                asset_browser,
                icons,
                world_object,
            } = ctx;
            let picker_icon = icons.get("color-picker").copied();
            let read_only = play_mode != PlayMode::Edit;

            render_header(ui, selection, read_only);
            ui.add_space(2.0);
            ui.separator();
            ui.add_space(4.0);

            if selection.world_selected() {
                match world_object {
                    Some(info) => render_world_info(ui, info),
                    None => render_empty_state(ui),
                }
                return;
            }

            let Some(entity) = selection.primary() else {
                render_empty_state(ui);
                return;
            };

            // Invalidate caches if entity changed.
            if panel.last_entity != Some(entity) {
                panel.euler_cache.clear();
                panel.last_entity = Some(entity);
                panel.cached_presence = ComponentPresence::probe(world, entity);
                panel.presence_entity = Some(entity);
            }
            if panel.presence_entity != Some(entity) {
                panel.cached_presence = ComponentPresence::probe(world, entity);
                panel.presence_entity = Some(entity);
            }

            render_filter(ui, panel);
            ui.separator();

            let avail_h = ui.available_size().y;
            ScrollArea::new(avail_h)
                .auto_shrink(false)
                .inset(2.0)
                .spacing(4.0)
                .show(ui, |ui| {
                    ui.add_space(6.0);
                    render_entity_info(ui, world, entity);
                    ui.separator();
                    if read_only {
                        let dim = ui.style().palette.text_secondary;
                        Label::new("Properties are locked during play mode")
                            .color(dim)
                            .show(ui);
                        return;
                    }
                    render_components(ui, panel, world, entity, asset_browser, picker_icon, icons);
                    ui.separator();
                    render_add_component(ui, panel, world, entity);
                });
        },
    );
}

fn render_header(ui: &mut Ui, selection: &Selection, read_only: bool) {
    let style = ui.style();
    let dim = style.palette.text_secondary;
    if read_only {
        Label::new("(Playing)")
            .size(style.fonts.small)
            .color(dim)
            .show(ui);
    } else if selection.count() > 1 {
        Label::new(format!("({} selected)", selection.count()))
            .color(dim)
            .show(ui);
    } else {
        ui.add_space(style.fonts.small + 2.0);
    }
}

fn render_empty_state(ui: &mut Ui) {
    let dim = ui.style().palette.text_secondary;
    let font = ui.style().fonts.body;
    ui.add_space(50.0);
    let avail = ui.available().width();
    for (text, gap) in [
        ("No entity selected", 10.0),
        ("Select an entity in the Hierarchy", 0.0),
    ] {
        let w = ui.painter().measure_text(text, font, None).x;
        let x = ui.cursor().x + ((avail - w) * 0.5).max(0.0);
        let y = ui.cursor().y;
        ui.painter().text(Pos2::new(x, y), text, font, dim, None);
        ui.add_space(font + 4.0 + gap);
    }
}

/// Read-only streamed-world details (UE Landscape "Information" style).
fn render_world_info(ui: &mut Ui, info: &super::world_object::WorldObjectInfo) {
    let style = ui.style();
    let text = style.palette.text;
    let dim = style.palette.text_secondary;
    let width = ui.available().width();

    ui.add_space(6.0);
    let row_top = ui.cursor();
    Label::new(info.name.clone()).size(16.0).show(ui);
    let id_text = "Streamed World";
    let id_w = ui
        .painter()
        .measure_text(id_text, style.fonts.small, None)
        .x;
    ui.painter().text(
        Pos2::new(row_top.x + width - id_w - 6.0, row_top.y + 4.0),
        id_text,
        style.fonts.small,
        dim,
        None,
    );
    ui.separator();
    ui.add_space(4.0);

    let bar = ui.allocate(Vec2::new(width, 22.0));
    ui.painter().rect_filled(bar, 2.0, style.palette.header);
    ui.painter().text(
        Pos2::new(bar.min.x + 8.0, bar.min.y + 4.0),
        "Information",
        style.fonts.body,
        text,
        None,
    );
    ui.add_space(4.0);

    let mode = if info.full_world {
        "Full World"
    } else {
        "Around Camera"
    };
    let zones = match info.current_zone {
        Some(z) => format!("{} (current: {z})", info.zone_count),
        None => info.zone_count.to_string(),
    };
    let rows = [
        ("Streaming Mode", mode.to_string()),
        ("Cell Size", format!("{:.0} m", info.cell_size)),
        (
            "Cells Resident",
            format!("{} / {}", info.cells_resident, info.cells_total),
        ),
        (
            "Collision Chunks",
            format!("{} / {}", info.chunks_resident, info.chunks_total),
        ),
        ("Zones", zones),
        ("IO In Flight", info.in_flight.to_string()),
        ("Ready Queue", info.ready.to_string()),
        ("Worst Frame", format!("{:.2} ms", info.worst_ms)),
    ];
    for (label, value) in rows {
        let row = ui.allocate(Vec2::new(width, 20.0));
        ui.painter().text(
            Pos2::new(row.min.x + 12.0, row.min.y + 3.0),
            label,
            style.fonts.body,
            dim,
            None,
        );
        ui.painter().text(
            Pos2::new(row.min.x + 150.0, row.min.y + 3.0),
            &value,
            style.fonts.body,
            text,
            None,
        );
    }
}

fn render_filter(ui: &mut Ui, panel: &mut InspectorPanel) {
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        let style = ui.style();
        let p = ui.cursor();
        ui.painter().text(
            Pos2::new(p.x, p.y + 3.0),
            "Filter:",
            style.fonts.body,
            style.palette.text,
            None,
        );
        let label_w = ui
            .painter()
            .measure_text("Filter:", style.fonts.body, None)
            .x;
        ui.add_space(label_w + 8.0);
        let w = (ui.available_size().x - 2.0).max(60.0);
        let out = TextEdit::new(&mut panel.search_filter)
            .width(w)
            .height(18.0)
            .fill(style.palette.input)
            .hint("Search components...")
            .show_full(ui);
        if out.cancelled {
            panel.search_filter.clear();
        }
    });
}

fn render_entity_info(ui: &mut Ui, world: &World, entity: Entity) {
    let name = world
        .get::<&Name>(entity)
        .map(|n| n.0.clone())
        .unwrap_or_else(|_| format!("Entity {:?}", entity.id()));
    let style = ui.style();
    let row_top = ui.cursor();
    let right = ui.available().max.x;
    Label::new(name).size(16.0).show(ui);
    let id_text = format!("ID: {}", entity.id());
    let id_w = ui
        .painter()
        .measure_text(&id_text, style.fonts.small, None)
        .x;
    ui.painter().text(
        Pos2::new(right - id_w - 6.0, row_top.y + 4.0),
        &id_text,
        style.fonts.small,
        style.palette.text_secondary,
        None,
    );
}

// ── Section / row helpers ────────────────────────────────────────────────

/// Component section: raised header bar with chevron + title, category
/// accent stripe down the left edge, collapsible body. Right-click on the
/// header opens a "Remove Component" menu when `removable`. Returns true
/// when removal was requested.
fn component_section(
    ui: &mut Ui,
    title: &str,
    accent: Color,
    removable: bool,
    body: impl FnOnce(&mut Ui),
) -> bool {
    let style = ui.style();
    let panel_rect = ui.available();
    let (left, right) = (panel_rect.min.x, panel_rect.max.x);
    let start_y = ui.cursor().y;

    let header_h = 26.0;
    let header_rect = ui.allocate(Vec2::new(panel_rect.width(), header_h));
    let sid = Id::new("inspector_section").with(title);
    // Before `interact`, which itself creates the memory entry.
    let first_time = ui.ctx().memory.get(sid).is_none();
    let resp = ui.interact(sid, header_rect);
    {
        let mem = ui.ctx_mut().memory.get_or_default(sid);
        if first_time {
            mem.bool_state = true;
        }
        mem.seen_this_frame = true;
        mem.last_rect = header_rect;
        if resp.clicked {
            mem.bool_state = !mem.bool_state;
        }
    }
    let open = ui
        .ctx()
        .memory
        .get(sid)
        .map(|m| m.bool_state)
        .unwrap_or(true);

    let bg = if resp.hovered {
        style.palette.hover
    } else {
        style.palette.header
    };
    let header_bg_rect = Rect::from_min_max(
        Pos2::new(left, header_rect.min.y),
        Pos2::new(right, header_rect.max.y),
    );
    ui.painter().rect_filled(header_bg_rect, 0.0, bg);
    ui.painter().line_segment(
        Pos2::new(left, header_rect.max.y),
        Pos2::new(right, header_rect.max.y),
        1.0,
        style.palette.stroke,
    );

    // Chevron (▼ open / ▶ closed), drawn like the hierarchy port.
    let center = Pos2::new(left + 13.0, (header_rect.min.y + header_rect.max.y) * 0.5);
    let s = 3.5;
    let chev = if resp.hovered {
        style.palette.accent_active
    } else {
        style.palette.text_secondary
    };
    let (a, m, b) = if open {
        (
            Pos2::new(center.x - s, center.y - s * 0.45),
            Pos2::new(center.x, center.y + s * 0.55),
            Pos2::new(center.x + s, center.y - s * 0.45),
        )
    } else {
        (
            Pos2::new(center.x - s * 0.45, center.y - s),
            Pos2::new(center.x + s * 0.55, center.y),
            Pos2::new(center.x - s * 0.45, center.y + s),
        )
    };
    ui.painter().line_segment(a, m, 1.5, chev);
    ui.painter().line_segment(m, b, 1.5, chev);

    ui.painter().text(
        Pos2::new(
            left + 24.0,
            header_rect.min.y + (header_h - style.fonts.body) * 0.5,
        ),
        title,
        style.fonts.body,
        style.palette.text,
        None,
    );

    let mut remove = false;
    if removable {
        ui.context_menu_for(("inspector_section_ctx", title), header_rect, |ui| {
            if ui.menu_item("Remove Component") {
                remove = true;
            }
        });
    }

    if open {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            ui.vertical(|ui| body(ui));
        });
        ui.add_space(6.0);
    }

    // Category stripe over the full section height + bottom separator
    // (2px — the reserved edge-accent width).
    let end_y = ui.cursor().y;
    ui.painter().rect_filled(
        Rect::from_min_max(
            Pos2::new(left, start_y),
            Pos2::new(left + style.metrics.edge_accent, end_y),
        ),
        0.0,
        accent,
    );
    ui.painter().line_segment(
        Pos2::new(left, end_y),
        Pos2::new(right, end_y),
        1.0,
        style.palette.stroke,
    );
    // Gap between sections, outside the accent stripe (~5px).
    ui.add_space(5.0);

    remove
}

/// Two-column property row: fixed-width dim label, then the value widgets.
fn property_row<R>(ui: &mut Ui, label: &str, value: impl FnOnce(&mut Ui) -> R) -> R {
    let style = ui.style();
    let dim = style.palette.text_secondary;
    let font = style.fonts.body;
    let top = ui.cursor().y;
    let r = ui.horizontal(|ui| {
        let start = ui.cursor();
        ui.painter()
            .text(Pos2::new(start.x, start.y + 3.0), label, font, dim, None);
        ui.set_cursor(Pos2::new(start.x + PROP_LABEL_W, start.y));
        value(ui)
    });
    // Short rows (checkbox, swatch) still advance a full field-height pitch.
    let advanced = ui.cursor().y - top;
    if advanced < FIELD_H {
        ui.add_space(FIELD_H - advanced);
    }
    r
}

/// Transform-style row: label, X/Y/Z drag fields with a 3px axis-colored
/// inset edge + tinted axis letter, then a 20×22 "R" reset at the row's
/// end. Returns true when reset was clicked.
fn vec3_row(
    ui: &mut Ui,
    label: &str,
    vals: &mut [f32; 3],
    speed: f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: Option<&str>,
) -> bool {
    let style = ui.style();
    let font = style.fonts.body;
    let dim = style.palette.text_secondary;
    let axis = super::theme::Palette::invariant_axis();
    ui.horizontal(|ui| {
        let start = ui.cursor();
        ui.painter()
            .text(Pos2::new(start.x, start.y + 3.0), label, font, dim, None);
        ui.set_cursor(Pos2::new(start.x + VEC3_LABEL_W, start.y));

        let remain = ui.available_size().x;
        let field_w =
            ((remain - (VEC3_RESET_W + 6.0) - 3.0 * 6.0) / 3.0).clamp(40.0, VEC3_INPUT_W);

        for (i, (axis_label, color)) in [("X", axis.x), ("Y", axis.y), ("Z", axis.z)]
            .into_iter()
            .enumerate()
        {
            let mut dv = DragValue::new(&mut vals[i])
                .speed(speed)
                .range(range.clone())
                .width(field_w)
                .height(FIELD_H)
                .axis(axis_label, color);
            if let Some(suffix) = suffix {
                dv = dv.suffix(suffix);
            }
            dv.show(ui);
            ui.add_space(6.0);
        }

        let reset = Button::new("R")
            .exact_size(Vec2::new(VEC3_RESET_W, FIELD_H))
            .show(ui);
        if reset.hovered {
            ui.tooltip_for(reset.rect, "Reset");
        }
        reset.clicked
    })
}

/// Property row with a crusty slider + linked drag field (user-approved
/// crusty-native slider design).
fn slider_row(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
) {
    let span = *range.end() - *range.start();
    property_row(ui, label, |ui| {
        let field_bg = ui.style().palette.input;
        let slider_w = 76.0;
        Slider::new(value, range.clone()).width(slider_w).show(ui);
        ui.add_space(6.0);
        DragValue::new(value)
            // Slider gradient: value change per dragged pixel.
            .speed(span / slider_w)
            .range(range)
            .suffix(suffix)
            .width(64.0)
            .height(FIELD_H)
            .fill(field_bg)
            .show(ui);
    });
}

/// Property row with a single drag field.
fn drag_row(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    speed: f64,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
) {
    property_row(ui, label, |ui| {
        let field_bg = ui.style().palette.input;
        DragValue::new(value)
            .speed(speed)
            .range(range)
            .suffix(suffix)
            .width(60.0)
            .height(FIELD_H)
            .fill(field_bg)
            .show(ui);
    });
}

/// Checkbox with the label on the property-row left column.
fn checkbox_row(ui: &mut Ui, label: &str, value: &mut bool) {
    property_row(ui, label, |ui| {
        Checkbox::new(value, "").show(ui);
    });
}

/// Color property row backed by crusty's `ColorPicker` (linear RGB [0,1]).
fn color_row(
    ui: &mut Ui,
    id_source: &str,
    label: &str,
    rgb: &mut [f32; 3],
    picker: &mut Picker,
) -> bool {
    property_row(ui, label, |ui| {
        let mut c = Color::rgb(rgb[0], rgb[1], rgb[2]);
        let mut cp = ColorPicker::new(id_source, &mut c)
            .alpha(false)
            .swatches(picker.swatches)
            .eyedropper(picker.eye);
        if let Some(icon) = picker.icon {
            cp = cp.eyedropper_icon(icon);
        }
        cp.show(ui);
        let changed = c.r != rgb[0] || c.g != rgb[1] || c.b != rgb[2];
        if changed {
            *rgb = [c.r, c.g, c.b];
        }
        changed
    })
}

// ── Asset reference field ────────────────────────────────────────────────

/// 52px framed thumbnail with a 2px bottom type edge.
const REF_THUMB: f32 = 52.0;
/// Session-wide Copy/Paste Reference clipboard (path string).
fn asset_ref_clipboard_id() -> Id {
    Id::new("asset_ref_clipboard")
}

/// One entry the picker can offer.
struct PickerEntry {
    path: String,
    name: String,
    thumb: Option<TextureId>,
}

/// Asset reference field — one layout for every asset-typed property:
/// framed live thumbnail with type edge, name dropdown opening the picker
/// popover, and an 18px utility strip (use browser selection / locate in
/// browser / reset). Missing refs read status-error on the name + a dot on
/// the thumb; the field itself stays neutral.
fn asset_ref_field(
    ui: &mut Ui,
    id_source: &str,
    label: &str,
    path: &mut String,
    allowed_types: &[AssetType],
    asset_browser: &mut AssetBrowserPanel,
    icons: &std::collections::HashMap<String, TextureId>,
) {
    use super::asset_browser_crusty::{type_color, type_icon_stem};
    use crate::engine::rendering::rendering_3d::mesh::PRIMITIVE_PATHS;
    use crusty_gui::paint::PaintCmd;
    use crusty_gui::text::FontFamily;
    use crusty_gui::widgets::Popup;

    let style = ui.style();
    let status = super::theme::Palette::invariant_status();
    let primary_type = allowed_types[0];
    let tcol = type_color(primary_type);
    let type_icon = icons.get(type_icon_stem(primary_type)).copied();

    // Resolve the current reference.
    let is_primitive = PRIMITIVE_PATHS.contains(&path.as_str());
    let meta = if path.is_empty() || is_primitive {
        None
    } else {
        asset_browser
            .registry
            .get_by_path(std::path::Path::new(path.as_str()))
            .cloned()
    };
    let missing = !path.is_empty() && !is_primitive && meta.is_none();
    let thumb_tex = meta
        .as_ref()
        .and_then(|m| asset_browser.thumbnails.crusty_texture_id(m));

    let avail_w = ui.available().width();
    let row = ui.allocate(Vec2::new(avail_w, REF_THUMB));
    ui.painter().text(
        Pos2::new(row.min.x, row.min.y + 3.0),
        label,
        style.fonts.body,
        style.palette.text_secondary,
        None,
    );

    // Thumb: bg-window frame, live preview or typed icon, 2px type edge.
    let thumb = Rect::from_min_size(
        Pos2::new(row.min.x + PROP_LABEL_W, row.min.y),
        Vec2::splat(REF_THUMB),
    );
    ui.painter().rect_filled(thumb, 3.0, style.palette.window);
    ui.painter().rect_stroke(thumb, 3.0, 1.0, style.palette.stroke);
    if let Some(tex) = thumb_tex {
        let inner = Rect::from_min_max(
            Pos2::new(thumb.min.x + 1.0, thumb.min.y + 1.0),
            Pos2::new(thumb.max.x - 1.0, thumb.max.y - 1.0),
        );
        ui.ctx_mut().paint.push(PaintCmd::Image {
            rect: inner,
            uv_min: Pos2::new(0.0, 0.0),
            uv_max: Pos2::new(1.0, 1.0),
            tint: Color::WHITE,
            texture: tex,
        });
    } else if let Some(tex) = type_icon {
        let tint = if missing { tcol.with_alpha(0.45) } else { tcol };
        ui.ctx_mut().paint.push(PaintCmd::Image {
            rect: Rect::from_center_size(thumb.center(), Vec2::splat(24.0)),
            uv_min: Pos2::new(0.0, 0.0),
            uv_max: Pos2::new(1.0, 1.0),
            tint,
            texture: tex,
        });
    }
    ui.painter().rect_filled(
        Rect::from_min_max(
            Pos2::new(thumb.min.x + 1.0, thumb.max.y - 3.0),
            Pos2::new(thumb.max.x - 1.0, thumb.max.y - 1.0),
        ),
        0.0,
        tcol,
    );
    if missing {
        ui.painter().circle_filled(
            Pos2::new(thumb.max.x - 7.0, thumb.min.y + 7.0),
            3.5,
            status.error,
        );
    }

    // Name dropdown — the picker trigger.
    let name_x = thumb.max.x + 6.0;
    let name_w = (row.max.x - name_x).max(80.0);
    let name_rect = Rect::from_min_size(Pos2::new(name_x, row.min.y), Vec2::new(name_w, FIELD_H));
    let pid = Id::new("asset_ref_picker").with(id_source);
    let open = Popup::is_open(ui, pid);
    let trigger = ui.interact(pid.with("trigger"), name_rect);
    ui.painter().rect_filled(name_rect, 3.0, style.palette.input);
    let border = if open {
        style.palette.focus_ring
    } else {
        style.palette.stroke
    };
    ui.painter().rect_stroke(name_rect, 3.0, 1.0, border);
    let (display, name_col) = if path.is_empty() {
        ("None".to_string(), style.palette.text_secondary)
    } else {
        let name = std::path::Path::new(path.as_str())
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        if missing {
            (name, status.error)
        } else {
            (name, style.palette.text)
        }
    };
    let ty = name_rect.min.y + (FIELD_H - style.fonts.body * 1.25) * 0.5;
    ui.painter().text(
        Pos2::new(name_rect.min.x + 8.0, ty),
        &display,
        style.fonts.body,
        name_col,
        Some(name_w - 30.0),
    );
    if missing {
        let nw = ui.painter().measure_text(&display, style.fonts.body, None).x;
        ui.painter().text_family(
            Pos2::new(name_rect.min.x + 8.0 + nw.min(name_w - 60.0) + 6.0, ty + 2.0),
            "missing",
            9.0,
            style.palette.text_disabled,
            None,
            FontFamily::Mono,
        );
    }
    let caret = if open { "\u{25B2}" } else { "\u{25BC}" };
    let csz = ui.painter().measure_text(caret, 8.0, None);
    ui.painter().text(
        Pos2::new(
            name_rect.max.x - 8.0 - csz.x,
            name_rect.min.y + (FIELD_H - csz.y) * 0.5,
        ),
        caret,
        8.0,
        style.palette.text_secondary,
        None,
    );
    if trigger.clicked {
        Popup::toggle(ui, pid);
        if !open {
            // Fresh open: clear search, focus it next frame.
            ui.ctx_mut().memory.data_insert(pid.with("search"), String::new());
            ui.ctx_mut().memory.data_insert(pid.with("focus"), true);
        }
    }

    // Utility strip: use-browser-selection / locate / reset.
    let strip_y = name_rect.max.y + 4.0;
    let browser_pick = asset_browser
        .selection
        .primary()
        .and_then(|id| asset_browser.registry.get(id))
        .filter(|m| allowed_types.contains(&m.asset_type))
        .map(|m| m.path.to_string_lossy().to_string());
    let mut bx = name_x;
    let mut strip_btn =
        |ui: &mut Ui, key: &str, icon: Option<TextureId>, mono: Option<&str>, enabled: bool| {
            let r = Rect::from_min_size(Pos2::new(bx, strip_y), Vec2::new(22.0, 18.0));
            bx += 26.0;
            let resp = ui.interact(pid.with(key), r);
            let style = ui.style();
            let fill = if resp.hovered && enabled {
                style.palette.hover
            } else {
                style.palette.elevated
            };
            ui.painter().rect_filled(r, 3.0, fill);
            ui.painter().rect_stroke(r, 3.0, 1.0, style.palette.stroke);
            let tint = if enabled {
                style.palette.text_secondary
            } else {
                style.palette.text_disabled
            };
            if let Some(tex) = icon {
                ui.ctx_mut().paint.push(PaintCmd::Image {
                    rect: Rect::from_center_size(r.center(), Vec2::splat(10.0)),
                    uv_min: Pos2::new(0.0, 0.0),
                    uv_max: Pos2::new(1.0, 1.0),
                    tint,
                    texture: tex,
                });
            } else if let Some(t) = mono {
                let sz = ui.painter().measure_text(t, 9.5, None);
                ui.painter().text_family(
                    Pos2::new(r.center().x - sz.x * 0.5, r.center().y - sz.y * 0.5),
                    t,
                    9.5,
                    tint,
                    None,
                    FontFamily::Mono,
                );
            }
            resp.clicked && enabled
        };
    if strip_btn(
        ui,
        "use_sel",
        icons.get("color-picker").copied(),
        Some("\u{2299}"),
        browser_pick.is_some(),
    ) {
        if let Some(p) = browser_pick {
            *path = p;
        }
    }
    if strip_btn(
        ui,
        "locate",
        icons.get("folder-browser").copied(),
        Some("\u{25A4}"),
        meta.is_some(),
    ) {
        if let Some(m) = &meta {
            asset_browser.selection.select(m.id);
            if let Some(parent) = m.path.parent() {
                asset_browser.current_folder = parent.to_path_buf();
            }
        }
    }
    if strip_btn(ui, "reset", None, Some("R"), !path.is_empty()) {
        path.clear();
    }

    // Picker popover (E3): search + type chip, list, actions, mono footer.
    if Popup::is_open(ui, pid) {
        asset_ref_picker(ui, pid, path, allowed_types, asset_browser, icons, name_rect);
    }
    ui.add_space(4.0);
}

/// The picker popover body. Search is auto-focused and pre-filtered to the
/// property's type; current value = selection-fill row; actions for the
/// reference sit under the list; footer = mono count + Esc.
fn asset_ref_picker(
    ui: &mut Ui,
    pid: Id,
    path: &mut String,
    allowed_types: &[AssetType],
    asset_browser: &mut AssetBrowserPanel,
    icons: &std::collections::HashMap<String, TextureId>,
    anchor: Rect,
) {
    use super::asset_browser_crusty::{type_color, type_icon_stem};
    use crate::engine::rendering::rendering_3d::mesh::PRIMITIVE_PATHS;
    use crusty_gui::paint::PaintCmd;
    use crusty_gui::text::FontFamily;
    use crusty_gui::widgets::Popup;

    let primary_type = allowed_types[0];
    let tcol = type_color(primary_type);
    let type_icon = icons.get(type_icon_stem(primary_type)).copied();
    let type_label = primary_type.display_name().to_uppercase();
    let pop_w = anchor.width().max(220.0);

    // Gather entries up front (borrow of asset_browser ends before the UI).
    let sid = pid.with("search");
    let search = ui
        .ctx()
        .memory
        .data_get::<String>(sid)
        .cloned()
        .unwrap_or_default();
    let search_lower = search.to_lowercase();

    let include_prims = allowed_types.contains(&AssetType::Mesh)
        || allowed_types.contains(&AssetType::Model);
    let mut entries: Vec<PickerEntry> = Vec::new();
    if include_prims {
        for prim in PRIMITIVE_PATHS {
            if search.is_empty() || prim.to_lowercase().contains(&search_lower) {
                entries.push(PickerEntry {
                    path: prim.to_string(),
                    name: prim.rsplit('/').next().unwrap_or(prim).to_string(),
                    thumb: None,
                });
            }
        }
    }
    let filter = AssetFilter {
        search_text: (!search.is_empty()).then(|| search.clone()),
        asset_types: Some(allowed_types.to_vec()),
        include_subfolders: true,
        ..Default::default()
    };
    let metas: Vec<_> = asset_browser
        .registry
        .query(&filter)
        .into_iter()
        .cloned()
        .collect();
    let prim_total = if include_prims { PRIMITIVE_PATHS.len() } else { 0 };
    let total = prim_total
        + if search.is_empty() {
            metas.len()
        } else {
            asset_browser
                .registry
                .query(&AssetFilter {
                    asset_types: Some(allowed_types.to_vec()),
                    include_subfolders: true,
                    ..Default::default()
                })
                .len()
        };
    for m in &metas {
        entries.push(PickerEntry {
            path: m.path.to_string_lossy().to_string(),
            name: m.display_name.clone(),
            thumb: asset_browser.thumbnails.crusty_texture_id(m),
        });
    }
    let shown = entries.len();

    let mut chosen: Option<String> = None;
    Popup::new(pid, anchor, pop_w).show(ui, |ui| {
        let inner_w = ui.available().width();

        // Search + locked type chip.
        let chip_w = ui
            .painter()
            .measure_text_family(&type_label, 9.5, None, FontFamily::Mono)
            .x
            + 14.0;
        let mut search_text = search.clone();
        let focus = ui
            .ctx()
            .memory
            .data_get::<bool>(pid.with("focus"))
            .copied()
            .unwrap_or(false);
        ui.horizontal(|ui| {
            TextEdit::new(&mut search_text)
                .width(inner_w - chip_w - 6.0)
                .height(FIELD_H)
                .hint("Search...")
                .request_focus(focus)
                .show_full(ui);
            let chip = ui.allocate(Vec2::new(chip_w, 20.0));
            ui.painter().rect_filled(chip, 3.0, tcol.with_alpha(0.12));
            ui.painter().rect_stroke(chip, 3.0, 1.0, tcol.with_alpha(0.45));
            let sz = ui
                .painter()
                .measure_text_family(&type_label, 9.5, None, FontFamily::Mono);
            ui.painter().text_family(
                Pos2::new(chip.center().x - sz.x * 0.5, chip.center().y - sz.y * 0.5),
                &type_label,
                9.5,
                tcol,
                None,
                FontFamily::Mono,
            );
        });
        if focus {
            ui.ctx_mut().memory.data_insert(pid.with("focus"), false);
        }
        ui.ctx_mut().memory.data_insert(sid, search_text);

        // Entry rows: None first, then primitives + registry assets.
        let row_h = 24.0;
        let entry_row = |ui: &mut Ui,
                             key: usize,
                             name: &str,
                             thumb: Option<TextureId>,
                             is_current: bool,
                             show_well: bool|
         -> bool {
            let r = ui.allocate(Vec2::new(inner_w, row_h));
            let resp = ui.interact(pid.with(("entry", key)), r);
            let style = ui.style();
            if is_current {
                ui.painter().rect_filled(r, 3.0, style.palette.selection_fill);
            } else if resp.hovered {
                ui.painter().rect_filled(r, 3.0, style.palette.hover);
            }
            let mut x = r.min.x + 6.0;
            if show_well {
                let well = Rect::from_center_size(
                    Pos2::new(x + 9.0, r.center().y),
                    Vec2::splat(18.0),
                );
                ui.painter().rect_filled(well, 2.0, style.palette.window);
                ui.painter().rect_stroke(well, 2.0, 1.0, style.palette.stroke);
                if let Some(tex) = thumb {
                    ui.ctx_mut().paint.push(PaintCmd::Image {
                        rect: well,
                        uv_min: Pos2::new(0.0, 0.0),
                        uv_max: Pos2::new(1.0, 1.0),
                        tint: Color::WHITE,
                        texture: tex,
                    });
                } else if let Some(tex) = type_icon {
                    ui.ctx_mut().paint.push(PaintCmd::Image {
                        rect: Rect::from_center_size(well.center(), Vec2::splat(10.0)),
                        uv_min: Pos2::new(0.0, 0.0),
                        uv_max: Pos2::new(1.0, 1.0),
                        tint: tcol,
                        texture: tex,
                    });
                }
                ui.painter().rect_filled(
                    Rect::from_min_max(
                        Pos2::new(well.min.x, well.max.y - 2.0),
                        Pos2::new(well.max.x, well.max.y),
                    ),
                    0.0,
                    tcol,
                );
                x = well.max.x + 7.0;
            }
            let col = if is_current {
                style.palette.selection_text
            } else {
                style.palette.text
            };
            let tsz = ui.painter().measure_text(name, style.fonts.body, None);
            ui.painter().text(
                Pos2::new(x, r.center().y - tsz.y * 0.5),
                name,
                style.fonts.body,
                col,
                Some(r.max.x - x - 50.0),
            );
            if is_current {
                let tag = "current";
                let sz = ui.painter().measure_text_family(tag, 9.0, None, FontFamily::Mono);
                ui.painter().text_family(
                    Pos2::new(r.max.x - 6.0 - sz.x, r.center().y - sz.y * 0.5),
                    tag,
                    9.0,
                    style.palette.selection_text.with_alpha(0.75),
                    None,
                    FontFamily::Mono,
                );
            }
            resp.clicked
        };

        let list = |ui: &mut Ui, chosen: &mut Option<String>| {
            if entry_row(ui, 0, "None", None, path.is_empty(), false) {
                *chosen = Some(String::new());
            }
            for (i, e) in entries.iter().enumerate() {
                if entry_row(ui, i + 1, &e.name, e.thumb, path.as_str() == e.path, true) {
                    *chosen = Some(e.path.clone());
                }
            }
            if entries.is_empty() {
                Label::new("No assets found")
                    .color(ui.style().palette.text_secondary)
                    .show(ui);
            }
        };
        if shown > 9 {
            ScrollArea::new(9.0 * row_h).show(ui, |ui| list(ui, &mut chosen));
        } else {
            list(ui, &mut chosen);
        }

        // Reference actions.
        ui.separator();
        let clip = ui
            .ctx()
            .memory
            .data_get::<String>(asset_ref_clipboard_id())
            .cloned();
        let action_row = |ui: &mut Ui, key: &str, label: &str, enabled: bool| -> bool {
            let r = ui.allocate(Vec2::new(inner_w, row_h));
            let resp = ui.interact(pid.with(key), r);
            let style = ui.style();
            if resp.hovered && enabled {
                ui.painter().rect_filled(r, 3.0, style.palette.hover);
            }
            let col = if enabled {
                style.palette.text
            } else {
                style.palette.text_disabled
            };
            let sz = ui.painter().measure_text(label, style.fonts.body, None);
            ui.painter().text(
                Pos2::new(r.min.x + 8.0, r.center().y - sz.y * 0.5),
                label,
                style.fonts.body,
                col,
                None,
            );
            resp.clicked && enabled
        };
        if action_row(ui, "copy_ref", "Copy Reference", !path.is_empty()) {
            let p = path.clone();
            ui.ctx_mut().memory.data_insert(asset_ref_clipboard_id(), p);
        }
        if action_row(ui, "paste_ref", "Paste Reference", clip.is_some()) {
            if let Some(c) = clip {
                chosen = Some(c);
            }
        }
        if action_row(ui, "clear_ref", "Clear", !path.is_empty()) {
            chosen = Some(String::new());
        }

        // Footer: mono count + keys.
        ui.separator();
        let f = ui.allocate(Vec2::new(inner_w, 18.0));
        let style = ui.style();
        let plural = match primary_type {
            AssetType::Mesh | AssetType::Model => "meshes",
            AssetType::Material => "materials",
            AssetType::Texture => "textures",
            AssetType::Audio => "audio clips",
            _ => "assets",
        };
        let count = format!("{shown} of {total} {plural}");
        ui.painter().text_family(
            Pos2::new(f.min.x + 6.0, f.min.y + 3.0),
            &count,
            9.5,
            style.palette.text_disabled,
            None,
            FontFamily::Mono,
        );
        let esc = "Esc";
        let sz = ui.painter().measure_text_family(esc, 9.5, None, FontFamily::Mono);
        ui.painter().text_family(
            Pos2::new(f.max.x - 6.0 - sz.x, f.min.y + 3.0),
            esc,
            9.5,
            style.palette.text_disabled,
            None,
            FontFamily::Mono,
        );
    });
    if let Some(c) = chosen {
        *path = c;
        ui.ctx_mut().memory.get_or_default(pid).bool_state = false;
    }
}

// ── Component editors ────────────────────────────────────────────────────

fn render_components(
    ui: &mut Ui,
    panel: &mut InspectorPanel,
    world: &mut World,
    entity: Entity,
    asset_browser: &mut AssetBrowserPanel,
    picker_icon: Option<TextureId>,
    icons: &std::collections::HashMap<String, TextureId>,
) {
    let mut action: Option<ComponentAction> = None;
    let p = panel.cached_presence;
    macro_rules! picker {
        () => {
            &mut Picker {
                swatches: &mut panel.crusty_swatches,
                eye: &mut panel.crusty_eyedropper,
                icon: picker_icon,
            }
        };
    }

    if panel.matches_filter("name") && p.has(ComponentPresence::NAME) {
        edit_name(ui, world, entity);
    }
    if (panel.matches_filter("transform")
        || panel.matches_filter("position")
        || panel.matches_filter("rotation")
        || panel.matches_filter("scale"))
        && p.has(ComponentPresence::TRANSFORM)
    {
        edit_transform(ui, panel, world, entity);
    }
    if (panel.matches_filter("camera") || panel.matches_filter("fov"))
        && p.has(ComponentPresence::CAMERA)
        && edit_camera(ui, world, entity, picker!())
    {
        action = Some(ComponentAction::RemoveCamera);
    }
    if (panel.matches_filter("mesh")
        || panel.matches_filter("renderer")
        || panel.matches_filter("material"))
        && p.has(ComponentPresence::MESH_RENDERER)
        && edit_mesh_renderer(ui, world, entity, asset_browser, icons)
    {
        action = Some(ComponentAction::RemoveMeshRenderer);
    }
    if (panel.matches_filter("directional")
        || panel.matches_filter("light")
        || panel.matches_filter("sun"))
        && p.has(ComponentPresence::DIR_LIGHT)
        && edit_directional_light(ui, world, entity, picker!())
    {
        action = Some(ComponentAction::RemoveDirectionalLight);
    }
    if (panel.matches_filter("point") || panel.matches_filter("light"))
        && p.has(ComponentPresence::POINT_LIGHT)
        && edit_point_light(ui, world, entity, picker!())
    {
        action = Some(ComponentAction::RemovePointLight);
    }
    if (panel.matches_filter("rigid")
        || panel.matches_filter("body")
        || panel.matches_filter("physics"))
        && p.has(ComponentPresence::RIGID_BODY)
        && edit_rigidbody(ui, world, entity)
    {
        action = Some(ComponentAction::RemoveRigidBody);
    }
    if (panel.matches_filter("collider")
        || panel.matches_filter("physics")
        || panel.matches_filter("collision"))
        && p.has(ComponentPresence::COLLIDER)
        && edit_collider(ui, world, entity)
    {
        action = Some(ComponentAction::RemoveCollider);
    }
    if (panel.matches_filter("skeleton")
        || panel.matches_filter("bone")
        || panel.matches_filter("animation"))
        && p.has(ComponentPresence::SKELETON)
    {
        edit_skeleton(ui, world, entity);
    }
    if (panel.matches_filter("animation")
        || panel.matches_filter("playback")
        || panel.matches_filter("clip"))
        && p.has(ComponentPresence::ANIM_PLAYER)
    {
        edit_animation_player(ui, world, entity);
    }
    if (panel.matches_filter("audio")
        || panel.matches_filter("emitter")
        || panel.matches_filter("sound")
        || panel.matches_filter("clip"))
        && p.has(ComponentPresence::AUDIO_EMITTER)
        && edit_audio_emitter(ui, world, entity, asset_browser, icons)
    {
        action = Some(ComponentAction::RemoveAudioEmitter);
    }
    if (panel.matches_filter("audio") || panel.matches_filter("listener"))
        && p.has(ComponentPresence::AUDIO_LISTENER)
        && edit_audio_listener(ui, world, entity)
    {
        action = Some(ComponentAction::RemoveAudioListener);
    }
    if (panel.matches_filter("plankton")
        || panel.matches_filter("vfx")
        || panel.matches_filter("particle"))
        && p.has(ComponentPresence::PARTICLE_EFFECT)
        && edit_particle_effect(ui, world, entity, picker!())
    {
        action = Some(ComponentAction::RemoveParticleEffect);
    }
    if (panel.matches_filter("static")
        || panel.matches_filter("collision")
        || panel.matches_filter("physics"))
        && p.has(ComponentPresence::STATIC_COLLISION)
        && edit_static_collision(ui, world, entity)
    {
        action = Some(ComponentAction::RemoveStaticCollision);
    }

    if let Some(action) = action {
        match action {
            ComponentAction::RemoveCamera => {
                let _ = world.remove_one::<Camera>(entity);
            }
            ComponentAction::RemoveMeshRenderer => {
                let _ = world.remove_one::<MeshRenderer>(entity);
            }
            ComponentAction::RemoveDirectionalLight => {
                let _ = world.remove_one::<DirectionalLight>(entity);
            }
            ComponentAction::RemovePointLight => {
                let _ = world.remove_one::<PointLight>(entity);
            }
            ComponentAction::RemoveRigidBody => {
                let _ = world.remove_one::<RigidBody>(entity);
            }
            ComponentAction::RemoveCollider => {
                let _ = world.remove_one::<Collider>(entity);
            }
            ComponentAction::RemoveAudioEmitter => {
                let _ = world.remove_one::<AudioEmitter>(entity);
            }
            ComponentAction::RemoveAudioListener => {
                let _ = world.remove_one::<AudioListener>(entity);
            }
            ComponentAction::RemoveParticleEffect => {
                let _ = world.remove_one::<ParticleEffect>(entity);
            }
            ComponentAction::RemoveStaticCollision => {
                let _ = world.remove_one::<StaticCollision>(entity);
            }
        }
        panel.cached_presence = ComponentPresence::probe(world, entity);
    }
}

fn edit_name(ui: &mut Ui, world: &mut World, entity: Entity) {
    if let Ok(mut name) = world.get::<&mut Name>(entity) {
        component_section(ui, "Name", cat("geometry"), false, |ui| {
            property_row(ui, "Name:", |ui| {
                let field_bg = ui.style().palette.input;
                // text_edit_width
                let w = (ui.available_size().x - 8.0).clamp(60.0, 280.0);
                TextEdit::new(&mut name.0)
                    .width(w)
                    .height(18.0)
                    .fill(field_bg)
                    .show_full(ui);
            });
        });
    }
}

fn edit_transform(ui: &mut Ui, panel: &mut InspectorPanel, world: &mut World, entity: Entity) {
    let snapshot = world.get::<&Transform>(entity).ok().map(|t| *t);

    if let Ok(mut transform) = world.get::<&mut Transform>(entity) {
        let euler_cache = &mut panel.euler_cache;
        component_section(ui, "Transform", cat("geometry"), false, |ui| {
            let mut position = [
                transform.position.x,
                transform.position.y,
                transform.position.z,
            ];
            for item in &mut position {
                if !item.is_finite() {
                    *item = 0.0;
                }
            }

            let entity_id = entity.id() as u64;
            let needs_recompute = match euler_cache.get(&entity_id) {
                Some((cached_quat, _)) => {
                    !quaternions_approximately_equal(cached_quat, &transform.rotation)
                }
                None => true,
            };
            if needs_recompute {
                let new_euler = quaternion_to_euler_degrees(&transform.rotation);
                euler_cache.insert(entity_id, (transform.rotation, new_euler));
            }
            let mut euler = euler_cache.get(&entity_id).unwrap().1;
            for item in &mut euler {
                if !item.is_finite() {
                    *item = 0.0;
                } else {
                    *item = clean_display_degrees(*item);
                }
            }
            let euler_before = euler;

            let mut scale = [transform.scale.x, transform.scale.y, transform.scale.z];
            for item in &mut scale {
                if !item.is_finite() {
                    *item = 1.0;
                }
            }

            ui.add_space(2.0);
            let reset_pos = vec3_row(
                ui,
                "Position",
                &mut position,
                0.1,
                f32::MIN..=f32::MAX,
                None,
            );
            let reset_rot = vec3_row(ui, "Rotation", &mut euler, 1.0, -180.0..=180.0, Some("°"));
            let reset_scale = vec3_row(ui, "Scale", &mut scale, 0.01, 0.001..=1000.0, None);
            ui.add_space(2.0);

            transform.position = glm::vec3(position[0], position[1], position[2]);
            transform.scale = glm::vec3(scale[0], scale[1], scale[2]);

            if euler != euler_before {
                let new_quat = euler_degrees_to_quaternion(&euler);
                transform.rotation = new_quat;
                euler_cache.insert(entity_id, (new_quat, euler));
            }
            if reset_pos {
                transform.position = glm::vec3(0.0, 0.0, 0.0);
            }
            if reset_rot {
                transform.rotation = glm::quat_identity();
                euler_cache.insert(entity_id, (glm::quat_identity(), [0.0, 0.0, 0.0]));
            }
            if reset_scale {
                transform.scale = glm::vec3(1.0, 1.0, 1.0);
            }
        });
    }

    if let Some(before) = snapshot {
        let changed = world.get::<&Transform>(entity).ok().is_some_and(|current| {
            current.position != before.position
                || current.rotation != before.rotation
                || current.scale != before.scale
        });
        if changed {
            crate::engine::ecs::hierarchy::mark_transform_dirty(world, entity);
        }
    }
}

fn edit_camera(ui: &mut Ui, world: &mut World, entity: Entity, picker: &mut Picker) -> bool {
    let Ok(mut camera) = world.get::<&mut Camera>(entity) else {
        return false;
    };
    component_section(ui, "Camera", cat("cameras"), true, |ui| {
        checkbox_row(ui, "Active", &mut camera.active);

        #[derive(PartialEq, Copy, Clone)]
        enum Proj {
            Perspective,
            Orthographic,
        }
        let mut kind = match camera.projection {
            CameraProjection::Perspective => Proj::Perspective,
            CameraProjection::Orthographic { .. } => Proj::Orthographic,
        };
        let before = kind;
        property_row(ui, "Projection", |ui| {
            let label = match kind {
                Proj::Perspective => "Perspective",
                Proj::Orthographic => "Orthographic",
            };
            ComboBox::new("cam_projection")
                .selected_text(label)
                .width(100.0)
                .show_ui(ui, |ui| {
                    SelectableValue::new(&mut kind, Proj::Perspective, "Perspective").show(ui);
                    SelectableValue::new(&mut kind, Proj::Orthographic, "Orthographic").show(ui);
                });
        });
        if kind != before {
            camera.projection = match kind {
                Proj::Perspective => CameraProjection::Perspective,
                Proj::Orthographic => CameraProjection::Orthographic { size: 10.0 },
            };
        }

        if !camera.fov.is_finite() {
            camera.fov = 60.0;
        }
        if !camera.near.is_finite() || camera.near <= 0.0 {
            camera.near = 0.1;
        }
        if !camera.far.is_finite() || camera.far <= camera.near {
            camera.far = 1000.0;
        }

        match &mut camera.projection {
            CameraProjection::Perspective => {
                slider_row(ui, "FOV", &mut camera.fov, 30.0..=120.0, "°");
            }
            CameraProjection::Orthographic { size } => {
                if !size.is_finite() {
                    *size = 10.0;
                }
                drag_row(ui, "Ortho Size", size, 0.1, 0.1..=1000.0, "");
            }
        }

        let near_max = (camera.far - 0.001).max(0.002);
        let far_min = (camera.near + 0.001).min(99999.0);
        drag_row(ui, "Near", &mut camera.near, 0.01, 0.001..=near_max, "");
        drag_row(ui, "Far", &mut camera.far, 1.0, far_min..=100000.0, "");

        color_row(
            ui,
            "cam_clear_color",
            "Clear Color",
            &mut camera.clear_color,
            picker,
        );

        let mut priority = camera.priority as f32;
        property_row(ui, "Priority", |ui| {
            let field_bg = ui.style().palette.input;
            DragValue::new(&mut priority)
                .speed(0.5)
                .range(-100.0..=100.0)
                .decimals(0)
                .width(60.0)
                .height(FIELD_H)
                .fill(field_bg)
                .show(ui);
        });
        camera.priority = priority.round() as i32;
    })
}

fn edit_mesh_renderer(
    ui: &mut Ui,
    world: &mut World,
    entity: Entity,
    asset_browser: &mut AssetBrowserPanel,
    icons: &std::collections::HashMap<String, TextureId>,
) -> bool {
    let Ok(mut renderer) = world.get::<&mut MeshRenderer>(entity) else {
        return false;
    };
    component_section(ui, "Mesh Renderer", cat("materials"), true, |ui| {
        checkbox_row(ui, "Visible", &mut renderer.visible);

        asset_ref_field(
            ui,
            "mesh_slot",
            "Mesh",
            &mut renderer.mesh_path,
            &[AssetType::Mesh, AssetType::Model],
            asset_browser,
            icons,
        );

        if renderer.material_paths.is_empty() {
            renderer.material_paths.push(String::new());
        }
        let slot_count = renderer.material_paths.len();
        for i in 0..slot_count {
            let label = if slot_count == 1 {
                "Material".to_string()
            } else {
                format!("Material {i}")
            };
            asset_ref_field(
                ui,
                &format!("material_slot_{i}"),
                &label,
                &mut renderer.material_paths[i],
                &[AssetType::Material],
                asset_browser,
                icons,
            );
        }

        ui.add_space(2.0);
        Checkbox::new(&mut renderer.cast_shadows, "Cast Shadows").show(ui);
        Checkbox::new(&mut renderer.receive_shadows, "Receive Shadows").show(ui);
    })
}

fn edit_directional_light(
    ui: &mut Ui,
    world: &mut World,
    entity: Entity,
    picker: &mut Picker,
) -> bool {
    let Ok(mut light) = world.get::<&mut DirectionalLight>(entity) else {
        return false;
    };
    component_section(ui, "Directional Light", cat("lights"), true, |ui| {
        if !light.direction.x.is_finite() {
            light.direction.x = 0.0;
        }
        if !light.direction.y.is_finite() {
            light.direction.y = -1.0;
        }
        if !light.direction.z.is_finite() {
            light.direction.z = 0.0;
        }
        let mut dir = [light.direction.x, light.direction.y, light.direction.z];
        property_row(ui, "Direction", |ui| {
            let field_bg = ui.style().palette.input;
            for (prefix, v) in ["X: ", "Y: ", "Z: "].into_iter().zip(dir.iter_mut()) {
                DragValue::new(v)
                    .prefix(prefix)
                    .speed(0.01)
                    .range(-1.0..=1.0)
                    .width(62.0)
                    .height(FIELD_H)
                    .fill(field_bg)
                    .show(ui);
                ui.add_space(4.0);
            }
        });
        light.direction = glm::vec3(dir[0], dir[1], dir[2]);
        let len = glm::length(&light.direction);
        if len > 0.001 {
            light.direction /= len;
        }

        let mut color = [light.color.x, light.color.y, light.color.z];
        if color_row(ui, "dir_light_color", "Color", &mut color, picker) {
            light.color = glm::vec3(color[0], color[1], color[2]);
        }

        slider_row(ui, "Intensity", &mut light.intensity, 0.0..=10.0, "");

        Checkbox::new(&mut light.shadow_enabled, "Cast Shadows").show(ui);
        if light.shadow_enabled {
            if !light.shadow_bias.is_finite() {
                light.shadow_bias = 0.005;
            }
            drag_row(
                ui,
                "Shadow Bias",
                &mut light.shadow_bias,
                0.001,
                0.0..=0.1,
                "",
            );
        }
    })
}

fn edit_point_light(ui: &mut Ui, world: &mut World, entity: Entity, picker: &mut Picker) -> bool {
    let Ok(mut light) = world.get::<&mut PointLight>(entity) else {
        return false;
    };
    component_section(ui, "Point Light", cat("lights"), true, |ui| {
        if !light.intensity.is_finite() {
            light.intensity = 1.0;
        }
        if !light.radius.is_finite() {
            light.radius = 10.0;
        }

        let mut color = [light.color.x, light.color.y, light.color.z];
        if color_row(ui, "point_light_color", "Color", &mut color, picker) {
            light.color = glm::vec3(color[0], color[1], color[2]);
        }

        slider_row(ui, "Intensity", &mut light.intensity, 0.0..=100.0, "");
        drag_row(ui, "Radius", &mut light.radius, 0.1, 0.1..=1000.0, "");

        property_row(ui, "Falloff", |ui| {
            ComboBox::new("pl_falloff")
                .selected_text(format!("{:?}", light.falloff))
                .width(140.0)
                .show_ui(ui, |ui| {
                    SelectableValue::new(&mut light.falloff, LightFalloff::Linear, "Linear")
                        .show(ui);
                    SelectableValue::new(&mut light.falloff, LightFalloff::Quadratic, "Quadratic")
                        .show(ui);
                    SelectableValue::new(
                        &mut light.falloff,
                        LightFalloff::InverseSquare,
                        "Inverse Square",
                    )
                    .show(ui);
                });
        });

        Checkbox::new(&mut light.shadow_enabled, "Cast Shadows").show(ui);
    })
}

fn edit_rigidbody(ui: &mut Ui, world: &mut World, entity: Entity) -> bool {
    let Ok(mut rb) = world.get::<&mut RigidBody>(entity) else {
        return false;
    };
    component_section(ui, "Rigid Body", cat("physics"), true, |ui| {
        if !rb.mass.is_finite() {
            rb.mass = 1.0;
        }
        if !rb.linear_damping.is_finite() {
            rb.linear_damping = 0.0;
        }
        if !rb.angular_damping.is_finite() {
            rb.angular_damping = 0.0;
        }

        property_row(ui, "Type", |ui| {
            ComboBox::new("rb_type")
                .selected_text(format!("{:?}", rb.body_type))
                .width(140.0)
                .show_ui(ui, |ui| {
                    SelectableValue::new(&mut rb.body_type, RigidBodyType::Dynamic, "Dynamic")
                        .show(ui);
                    SelectableValue::new(&mut rb.body_type, RigidBodyType::Kinematic, "Kinematic")
                        .show(ui);
                    SelectableValue::new(&mut rb.body_type, RigidBodyType::Static, "Static")
                        .show(ui);
                });
        });

        if rb.body_type == RigidBodyType::Dynamic {
            drag_row(ui, "Mass", &mut rb.mass, 0.1, 0.001..=10000.0, "");
        }

        slider_row(ui, "Linear Damp", &mut rb.linear_damping, 0.0..=10.0, "");
        slider_row(ui, "Angular Damp", &mut rb.angular_damping, 0.0..=10.0, "");

        Checkbox::new(&mut rb.can_sleep, "Can Sleep").show(ui);

        if rb.body_type == RigidBodyType::Dynamic {
            if !rb.gravity_scale.is_finite() {
                rb.gravity_scale = 1.0;
            }
            drag_row(
                ui,
                "Gravity Scale",
                &mut rb.gravity_scale,
                0.1,
                -10.0..=10.0,
                "",
            );
        }

        Checkbox::new(&mut rb.continuous_collision, "CCD (Continuous)").show(ui);

        let mut lock = rb.lock_rotation;
        property_row(ui, "Lock Rotation", |ui| {
            let style = ui.style();
            let font = style.fonts.body;
            let ax = crate::engine::editor::theme::Palette::invariant_axis();
            for ((axis, color), v) in [("X", ax.x), ("Y", ax.y), ("Z", ax.z)]
                .into_iter()
                .zip(lock.iter_mut())
            {
                let p = ui.cursor();
                ui.painter()
                    .text(Pos2::new(p.x, p.y + 1.0), axis, font, color, None);
                let axis_w = ui.painter().measure_text(axis, font, None).x;
                ui.add_space(axis_w + 3.0);
                Checkbox::new(v, "").show(ui);
                ui.add_space(8.0);
            }
        });
        rb.lock_rotation = lock;
    })
}

fn edit_collider(ui: &mut Ui, world: &mut World, entity: Entity) -> bool {
    let Ok(mut collider) = world.get::<&mut Collider>(entity) else {
        return false;
    };
    component_section(ui, "Collider", cat("physics"), true, |ui| {
        #[derive(PartialEq, Copy, Clone)]
        enum Shape {
            Cuboid,
            Ball,
            Capsule,
        }
        let mut kind = match &collider.shape {
            ColliderShape::Cuboid { .. } => Shape::Cuboid,
            ColliderShape::Ball { .. } => Shape::Ball,
            ColliderShape::Capsule { .. } => Shape::Capsule,
        };
        let before = kind;
        property_row(ui, "Shape", |ui| {
            let label = match kind {
                Shape::Cuboid => "Cuboid",
                Shape::Ball => "Ball",
                Shape::Capsule => "Capsule",
            };
            ComboBox::new("collider_shape")
                .selected_text(label)
                .width(140.0)
                .show_ui(ui, |ui| {
                    SelectableValue::new(&mut kind, Shape::Cuboid, "Cuboid").show(ui);
                    SelectableValue::new(&mut kind, Shape::Ball, "Ball").show(ui);
                    SelectableValue::new(&mut kind, Shape::Capsule, "Capsule").show(ui);
                });
        });
        if kind != before {
            collider.shape = match kind {
                Shape::Cuboid => ColliderShape::Cuboid {
                    half_extents: glm::vec3(0.5, 0.5, 0.5),
                },
                Shape::Ball => ColliderShape::Ball { radius: 0.5 },
                Shape::Capsule => ColliderShape::Capsule {
                    half_height: 0.5,
                    radius: 0.25,
                },
            };
        }

        match &mut collider.shape {
            ColliderShape::Cuboid { half_extents } => {
                if !half_extents.x.is_finite() {
                    half_extents.x = 0.5;
                }
                if !half_extents.y.is_finite() {
                    half_extents.y = 0.5;
                }
                if !half_extents.z.is_finite() {
                    half_extents.z = 0.5;
                }
                let mut he = [half_extents.x, half_extents.y, half_extents.z];
                vec3_row(ui, "Extents", &mut he, 0.01, 0.001..=1000.0, None);
                *half_extents = glm::vec3(he[0], he[1], he[2]);
            }
            ColliderShape::Ball { radius } => {
                if !radius.is_finite() {
                    *radius = 0.5;
                }
                drag_row(ui, "Radius", radius, 0.01, 0.001..=1000.0, "");
            }
            ColliderShape::Capsule {
                half_height,
                radius,
            } => {
                if !half_height.is_finite() {
                    *half_height = 0.5;
                }
                if !radius.is_finite() {
                    *radius = 0.25;
                }
                drag_row(ui, "Half Height", half_height, 0.01, 0.001..=1000.0, "");
                drag_row(ui, "Radius", radius, 0.01, 0.001..=1000.0, "");
            }
        }

        slider_row(ui, "Friction", &mut collider.friction, 0.0..=2.0, "");
        slider_row(ui, "Restitution", &mut collider.restitution, 0.0..=1.0, "");

        Checkbox::new(&mut collider.is_sensor, "Is Sensor (Trigger)").show(ui);
        Checkbox::new(&mut collider.debug_draw_visible, "Debug Draw").show(ui);
    })
}

fn edit_skeleton(ui: &mut Ui, world: &mut World, entity: Entity) {
    if let Ok(mut skeleton) = world.get::<&mut SkeletonInstance>(entity) {
        component_section(ui, "Skeleton", cat("animation"), false, |ui| {
            property_row(ui, "Bones", |ui| {
                Label::new(format!("{}", skeleton.bones.len())).show(ui);
            });
            Checkbox::new(&mut skeleton.debug_draw_visible, "Debug Draw").show(ui);

            if !skeleton.bones.is_empty() {
                CollapsingHeader::new("Bone List").show(ui, |ui| {
                    let style = ui.style();
                    let dim = style.palette.text_secondary;
                    for (i, bone) in skeleton.bones.iter().enumerate() {
                        let parent_str = bone
                            .parent_index
                            .map(|p| format!(" (parent: {p})"))
                            .unwrap_or_default();
                        Label::new(format!("[{i}] {}{parent_str}", bone.name))
                            .size(style.fonts.small)
                            .color(dim)
                            .show(ui);
                    }
                });
            }
        });
    }
}

fn edit_animation_player(ui: &mut Ui, world: &mut World, entity: Entity) {
    if let Ok(mut player) = world.get::<&mut AnimationPlayer>(entity) {
        component_section(ui, "Animation Player", cat("animation"), false, |ui| {
            property_row(ui, "Clip", |ui| {
                Label::new(player.clip.name.clone()).show(ui);
            });
            property_row(ui, "Duration", |ui| {
                Label::new(format!("{:.2}s", player.clip.duration_seconds)).show(ui);
            });

            ui.horizontal(|ui| {
                let is_playing = player.state == PlaybackState::Playing;
                if Button::new(if is_playing { "Stop" } else { "Play" })
                    .show(ui)
                    .clicked
                {
                    if is_playing {
                        player.stop();
                    } else {
                        player.play();
                    }
                }
                ui.add_space(4.0);
                if Button::new("Reset").show(ui).clicked {
                    player.reset();
                }
            });

            let duration = player.clip.duration_seconds.max(0.001);
            drag_row(ui, "Time", &mut player.time, 0.01, 0.0..=duration, "s");
            let w = ui.available_size().x - 8.0;
            Slider::new(&mut player.time, 0.0..=duration)
                .width(w.max(40.0))
                .show(ui);

            drag_row(ui, "Speed", &mut player.speed, 0.01, 0.0..=10.0, "");
            Checkbox::new(&mut player.looping, "Loop").show(ui);
        });
    }
}

fn edit_audio_emitter(
    ui: &mut Ui,
    world: &mut World,
    entity: Entity,
    asset_browser: &mut AssetBrowserPanel,
    icons: &std::collections::HashMap<String, TextureId>,
) -> bool {
    let Ok(mut emitter) = world.get::<&mut AudioEmitter>(entity) else {
        return false;
    };
    let mut remove_button = false;
    let remove_menu = component_section(ui, "Audio Emitter", cat("audio"), true, |ui| {
        asset_ref_field(
            ui,
            "audio_clip_slot",
            "Clip",
            &mut emitter.clip_path,
            &[AssetType::Audio],
            asset_browser,
            icons,
        );

        property_row(ui, "Bus", |ui| {
            ComboBox::new("audio_bus")
                .selected_text(emitter.bus.display_name())
                .width(140.0)
                .show_ui(ui, |ui| {
                    for &bus in AudioBus::ALL {
                        SelectableValue::new(&mut emitter.bus, bus, bus.display_name()).show(ui);
                    }
                });
        });

        drag_row(
            ui,
            "Volume (dB)",
            &mut emitter.volume_db,
            0.1,
            -80.0..=12.0,
            " dB",
        );
        drag_row(ui, "Pitch", &mut emitter.pitch, 0.01, 0.1..=4.0, "");

        ui.horizontal(|ui| {
            Checkbox::new(&mut emitter.looping, "Loop").show(ui);
            ui.add_space(12.0);
            Checkbox::new(&mut emitter.auto_play, "Auto-play").show(ui);
        });

        Checkbox::new(&mut emitter.spatial, "Spatial (3D)").show(ui);
        if emitter.spatial {
            drag_row(
                ui,
                "Max Distance",
                &mut emitter.max_distance,
                0.5,
                1.0..=1000.0,
                " m",
            );
            Checkbox::new(&mut emitter.hide_range_in_game, "Hidden in Game").show(ui);
        }

        ui.add_space(4.0);
        if Button::new("Remove")
            .text_color(ui.style().palette.danger)
            .show(ui)
            .clicked
        {
            remove_button = true;
        }
    });
    remove_menu || remove_button
}

fn edit_audio_listener(ui: &mut Ui, world: &mut World, entity: Entity) -> bool {
    let Ok(mut listener) = world.get::<&mut AudioListener>(entity) else {
        return false;
    };
    let mut remove_button = false;
    let remove_menu = component_section(ui, "Audio Listener", cat("audio"), true, |ui| {
        Checkbox::new(&mut listener.active, "Active").show(ui);
        ui.add_space(4.0);
        if Button::new("Remove")
            .text_color(ui.style().palette.danger)
            .show(ui)
            .clicked
        {
            remove_button = true;
        }
    });
    remove_menu || remove_button
}

fn edit_static_collision(ui: &mut Ui, world: &mut World, entity: Entity) -> bool {
    if world.get::<&StaticCollision>(entity).is_err() {
        return false;
    }
    let mut remove_button = false;
    let remove_menu = component_section(ui, "Static Collision", cat("physics"), true, |ui| {
        Label::new("Static collision source (cooked)").show(ui);
        ui.add_space(4.0);
        if Button::new("Remove")
            .text_color(ui.style().palette.danger)
            .show(ui)
            .clicked
        {
            remove_button = true;
        }
    });
    remove_menu || remove_button
}

fn edit_particle_effect(
    ui: &mut Ui,
    world: &mut World,
    entity: Entity,
    picker: &mut Picker,
) -> bool {
    let Ok(mut effect) = world.get::<&mut ParticleEffect>(entity) else {
        return false;
    };
    let mut remove_button = false;
    let remove_menu = component_section(ui, "Particle Effect", cat("vfx"), true, |ui| {
        property_row(ui, "Preset", |ui| {
            let mut chosen: Option<fn() -> ParticleEffect> = None;
            ComboBox::new("pe_preset")
                .selected_text("Select preset...")
                .width(140.0)
                .show_ui(ui, |ui| {
                    for (label, preset_fn) in [
                        ("Fire", ParticleEffect::fire as fn() -> ParticleEffect),
                        ("Smoke", ParticleEffect::smoke),
                        ("Sparks", ParticleEffect::sparks),
                        ("Dust", ParticleEffect::dust),
                    ] {
                        let mut dummy = false;
                        if SelectableValue::new(&mut dummy, true, label)
                            .show(ui)
                            .clicked
                        {
                            chosen = Some(preset_fn);
                        }
                    }
                });
            if let Some(preset_fn) = chosen {
                let preset = preset_fn();
                let capacity = effect.capacity;
                let show_gizmos = effect.show_gizmos;
                let texture_path = effect.texture_path.clone();
                *effect = preset;
                effect.capacity = capacity;
                effect.show_gizmos = show_gizmos;
                effect.texture_path = texture_path;
            }
        });

        CollapsingHeader::new("Lifecycle")
            .default_open(true)
            .show(ui, |ui| {
                Checkbox::new(&mut effect.enabled, "Enabled").show(ui);
                let mut capacity = effect.capacity as f32;
                slider_row(ui, "Capacity", &mut capacity, 256.0..=4096.0, "");
                effect.capacity = capacity.round() as u32;
            });

        CollapsingHeader::new("Emission")
            .default_open(true)
            .show(ui, |ui| {
                #[derive(PartialEq, Copy, Clone)]
                enum Shape {
                    Point,
                    Sphere,
                    Cone,
                    Box,
                }
                let mut kind = match effect.spawn_shape {
                    SpawnShape::Point => Shape::Point,
                    SpawnShape::Sphere { .. } => Shape::Sphere,
                    SpawnShape::Cone { .. } => Shape::Cone,
                    SpawnShape::Box { .. } => Shape::Box,
                };
                let before = kind;
                property_row(ui, "Shape", |ui| {
                    let label = match kind {
                        Shape::Point => "Point",
                        Shape::Sphere => "Sphere",
                        Shape::Cone => "Cone",
                        Shape::Box => "Box",
                    };
                    ComboBox::new("pe_shape")
                        .selected_text(label)
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            SelectableValue::new(&mut kind, Shape::Point, "Point").show(ui);
                            SelectableValue::new(&mut kind, Shape::Sphere, "Sphere").show(ui);
                            SelectableValue::new(&mut kind, Shape::Cone, "Cone").show(ui);
                            SelectableValue::new(&mut kind, Shape::Box, "Box").show(ui);
                        });
                });
                if kind != before {
                    effect.spawn_shape = match kind {
                        Shape::Point => SpawnShape::Point,
                        Shape::Sphere => SpawnShape::Sphere { radius: 1.0 },
                        Shape::Cone => SpawnShape::Cone {
                            angle_rad: 0.5,
                            radius: 0.5,
                        },
                        Shape::Box => SpawnShape::Box {
                            half_extents: [0.5, 0.5, 0.5],
                        },
                    };
                }

                match &mut effect.spawn_shape {
                    SpawnShape::Point => {}
                    SpawnShape::Sphere { radius } => {
                        drag_row(ui, "Radius", radius, 0.05, f32::MIN..=f32::MAX, "");
                    }
                    SpawnShape::Cone { angle_rad, radius } => {
                        drag_row(ui, "Angle", angle_rad, 0.01, f32::MIN..=f32::MAX, "");
                        drag_row(ui, "Radius", radius, 0.05, f32::MIN..=f32::MAX, "");
                    }
                    SpawnShape::Box { half_extents } => {
                        vec3_row(ui, "Extents", half_extents, 0.05, f32::MIN..=f32::MAX, None);
                    }
                }

                drag_row(ui, "Rate", &mut effect.emission_rate, 0.5, 0.0..=1000.0, "");
                let mut burst = effect.burst_count as f32;
                property_row(ui, "Burst Count", |ui| {
                    let field_bg = ui.style().palette.input;
                    DragValue::new(&mut burst)
                        .speed(1.0)
                        .range(0.0..=100000.0)
                        .decimals(0)
                        .width(60.0)
                        .height(FIELD_H)
                        .fill(field_bg)
                        .show(ui);
                });
                effect.burst_count = burst.round().max(0.0) as u32;
                drag_row(
                    ui,
                    "Burst Interval",
                    &mut effect.burst_interval,
                    0.01,
                    f32::MIN..=f32::MAX,
                    "",
                );
            });

        CollapsingHeader::new("Lifetime")
            .default_open(true)
            .show(ui, |ui| {
                drag_row(
                    ui,
                    "Min",
                    &mut effect.lifetime_min,
                    0.01,
                    0.01..=f32::MAX,
                    "",
                );
                drag_row(
                    ui,
                    "Max",
                    &mut effect.lifetime_max,
                    0.01,
                    0.01..=f32::MAX,
                    "",
                );
                if effect.lifetime_min > effect.lifetime_max {
                    effect.lifetime_max = effect.lifetime_min;
                }
            });

        CollapsingHeader::new("Velocity")
            .default_open(true)
            .show(ui, |ui| {
                vec3_row(
                    ui,
                    "Initial",
                    &mut effect.initial_velocity,
                    0.1,
                    f32::MIN..=f32::MAX,
                    None,
                );
                drag_row(
                    ui,
                    "Variance",
                    &mut effect.velocity_variance,
                    0.05,
                    0.0..=f32::MAX,
                    "",
                );
            });

        CollapsingHeader::new("Modules")
            .default_open(true)
            .show(ui, |ui| {
                let mut remove_idx: Option<usize> = None;
                for (idx, module) in effect.update_modules.iter_mut().enumerate() {
                    let title = format!("{} [{idx}]", module.display_name());
                    CollapsingHeader::new(title)
                        .default_open(true)
                        .show(ui, |ui| {
                            match module {
                                UpdateModule::Gravity(v) | UpdateModule::Wind(v) => {
                                    vec3_row(ui, "Vector", v, 0.1, f32::MIN..=f32::MAX, None);
                                }
                                UpdateModule::Drag(v) => {
                                    drag_row(ui, "Drag", v, 0.01, 0.0..=f32::MAX, "");
                                }
                                UpdateModule::CurlNoise {
                                    strength,
                                    scale,
                                    speed,
                                } => {
                                    drag_row(ui, "Strength", strength, 0.05, 0.0..=f32::MAX, "");
                                    drag_row(ui, "Scale", scale, 0.05, 0.01..=f32::MAX, "");
                                    drag_row(ui, "Speed", speed, 0.01, f32::MIN..=f32::MAX, "");
                                }
                                UpdateModule::ColorOverLife { start, end } => {
                                    // Stored floats are sRGB-encoded; round-trip u8.
                                    for (label, chan, salt) in
                                        [("Start", start, "col_start"), ("End", end, "col_end")]
                                    {
                                        property_row(ui, label, |ui| {
                                            let before = [
                                                (chan[0] * 255.0) as u8,
                                                (chan[1] * 255.0) as u8,
                                                (chan[2] * 255.0) as u8,
                                                (chan[3] * 255.0) as u8,
                                            ];
                                            let mut c = Color::from_srgb_u8(
                                                before[0], before[1], before[2], before[3],
                                            );
                                            let mut cp =
                                                ColorPicker::new(format!("{salt}_{idx}"), &mut c)
                                                    .swatches(picker.swatches)
                                                    .eyedropper(picker.eye);
                                            if let Some(icon) = picker.icon {
                                                cp = cp.eyedropper_icon(icon);
                                            }
                                            cp.show(ui);
                                            let after = c.to_srgb_u8();
                                            if after != before {
                                                *chan = [
                                                    after[0] as f32 / 255.0,
                                                    after[1] as f32 / 255.0,
                                                    after[2] as f32 / 255.0,
                                                    after[3] as f32 / 255.0,
                                                ];
                                            }
                                        });
                                    }
                                }
                                UpdateModule::SizeOverLife { start, end } => {
                                    drag_row(ui, "Start", start, 0.005, 0.0..=f32::MAX, "");
                                    drag_row(ui, "End", end, 0.005, 0.0..=f32::MAX, "");
                                }
                            }
                            if Button::new("Remove Module")
                                .text_color(ui.style().palette.danger)
                                .show(ui)
                                .clicked
                            {
                                remove_idx = Some(idx);
                            }
                        });
                }
                if let Some(idx) = remove_idx {
                    effect.update_modules.remove(idx);
                }

                ui.add_space(4.0);
                let mut add: Option<UpdateModule> = None;
                ComboBox::new("pe_add_module")
                    .selected_text("Add Module...")
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        let entries: [(&str, fn() -> UpdateModule); 6] = [
                            ("Gravity", || UpdateModule::Gravity([0.0, 0.0, -9.8])),
                            ("Drag", || UpdateModule::Drag(0.5)),
                            ("Wind", || UpdateModule::Wind([1.0, 0.0, 0.0])),
                            ("Curl Noise", || UpdateModule::CurlNoise {
                                strength: 1.0,
                                scale: 1.0,
                                speed: 0.5,
                            }),
                            ("Color Over Life", || UpdateModule::ColorOverLife {
                                start: [1.0, 1.0, 1.0, 1.0],
                                end: [1.0, 1.0, 1.0, 0.0],
                            }),
                            ("Size Over Life", || UpdateModule::SizeOverLife {
                                start: 0.1,
                                end: 0.0,
                            }),
                        ];
                        for (label, make) in entries {
                            let mut dummy = false;
                            if SelectableValue::new(&mut dummy, true, label)
                                .show(ui)
                                .clicked
                            {
                                add = Some(make());
                            }
                        }
                    });
                if let Some(module) = add {
                    effect.update_modules.push(module);
                }
            });

        CollapsingHeader::new("Rendering").show(ui, |ui| {
            property_row(ui, "Texture", |ui| {
                let field_bg = ui.style().palette.input;
                let w = (ui.available_size().x - 8.0).clamp(60.0, 280.0);
                TextEdit::new(&mut effect.texture_path)
                    .width(w)
                    .fill(field_bg)
                    .show_full(ui);
            });
            slider_row(
                ui,
                "Soft Fade",
                &mut effect.soft_fade_distance,
                0.0..=5.0,
                "",
            );
        });

        Checkbox::new(&mut effect.show_gizmos, "Show Gizmos").show(ui);

        ui.add_space(4.0);
        if Button::new("Remove")
            .text_color(ui.style().palette.danger)
            .show(ui)
            .clicked
        {
            remove_button = true;
        }
    });
    remove_menu || remove_button
}

// ── Add Component ────────────────────────────────────────────────────────

fn render_add_component(
    ui: &mut Ui,
    panel: &mut InspectorPanel,
    world: &mut World,
    entity: Entity,
) {
    let p = panel.cached_presence;
    let has_rigidbody = p.has(ComponentPresence::RIGID_BODY);
    let has_collider = p.has(ComponentPresence::COLLIDER);
    let has_camera = p.has(ComponentPresence::CAMERA);
    let has_dir_light = p.has(ComponentPresence::DIR_LIGHT);
    let has_point_light = p.has(ComponentPresence::POINT_LIGHT);

    let warning = super::theme::Palette::invariant_status().warning;
    if has_rigidbody && !has_collider {
        Label::new("Warning: RigidBody without Collider")
            .color(warning)
            .show(ui);
    } else if !has_rigidbody && has_collider {
        Label::new("Warning: Collider without RigidBody")
            .color(warning)
            .show(ui);
    }

    ui.add_space(8.0);

    let mut added = false;
    let dim = ui.style().palette.text_secondary;
    ComboBox::new("add_component")
        .selected_text("Add Component...")
        .width(150.0)
        .show_ui(ui, |ui| {
            let entry = |ui: &mut Ui, label: &str, conflict: bool| -> bool {
                if conflict {
                    Label::new(format!("{label} (conflicts)"))
                        .color(dim)
                        .show(ui);
                    return false;
                }
                let mut dummy = false;
                SelectableValue::new(&mut dummy, true, label)
                    .show(ui)
                    .clicked
            };

            if !has_camera && entry(ui, "Camera", has_dir_light || has_point_light) {
                let _ = world.insert_one(entity, Camera::default());
                added = true;
            }
            if !has_dir_light && entry(ui, "Directional Light", has_camera || has_point_light) {
                let _ = world.insert_one(entity, DirectionalLight::default());
                added = true;
            }
            if !has_point_light && entry(ui, "Point Light", has_camera || has_dir_light) {
                let _ = world.insert_one(entity, PointLight::default());
                added = true;
            }
            if !p.has(ComponentPresence::MESH_RENDERER) && entry(ui, "Mesh Renderer", false) {
                let _ = world.insert_one(entity, MeshRenderer::default());
                added = true;
            }
            if !has_rigidbody && entry(ui, "Rigid Body", false) {
                let _ = world.insert_one(entity, RigidBody::default());
                added = true;
            }
            if !has_collider && entry(ui, "Collider", false) {
                let _ = world.insert_one(entity, Collider::default());
                added = true;
            }
            if !p.has(ComponentPresence::AUDIO_EMITTER) && entry(ui, "Audio Emitter", false) {
                let _ = world.insert_one(entity, AudioEmitter::default());
                added = true;
            }
            if !p.has(ComponentPresence::AUDIO_LISTENER) && entry(ui, "Audio Listener", false) {
                let _ = world.insert_one(entity, AudioListener::default());
                added = true;
            }
            if !p.has(ComponentPresence::STATIC_COLLISION) && entry(ui, "Static Collision", false) {
                let _ = world.insert_one(entity, StaticCollision);
                added = true;
            }

            ui.separator();
            Label::new("VFX")
                .size(ui.style().fonts.small)
                .color(dim)
                .show(ui);
            if !p.has(ComponentPresence::PARTICLE_EFFECT) && entry(ui, "Particle Effect", false) {
                let _ = world.insert_one(entity, ParticleEffect::default());
                if world
                    .get::<&crate::engine::ecs::EntityGuid>(entity)
                    .is_err()
                {
                    let _ = world.insert_one(entity, crate::engine::ecs::EntityGuid::new());
                }
                added = true;
            }
        });

    if added {
        panel.cached_presence = ComponentPresence::probe(world, entity);
    }
}
