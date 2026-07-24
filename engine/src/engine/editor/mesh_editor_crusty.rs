//! Mesh editor panel rendered with crusty-gui.
//!
//! Reads/writes [`MeshEditorData`] (see `mesh_editor.rs`). Material slots
//! follow the Unreal layout: each slot
//! shows a material thumbnail next to a searchable dropdown picker,
//! accepts drag-and-drop from the asset browser, and has undo (clear to
//! "None") / trash (remove added slot) actions. Slots can be added with
//! the add-circle button but never removed below the mesh's submesh
//! count. Save lives in a bar pinned to the panel's bottom-right, with
//! save-check / save-changes icon states.
//!
//! Float windows have their own texture registries, so material
//! thumbnails arrive via `MeshEditorPanelCtx::float_thumbs` (uploaded
//! per-window from the shared cache's retained pixels).

use std::collections::HashMap;

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::input::MouseButton;
use crusty_gui::math::{Color, Pos2, Rect, Vec2};
use crusty_gui::paint::{PaintCmd, TextureId};
use crusty_gui::widgets::{
    show_tooltip_for, CollapsingHeader, ComboBox, Label, ScrollArea, TextEdit,
};
use glam::Vec3;

use super::asset_browser::{AssetBrowserPanel, AssetFilter};
use super::asset_browser_crusty::DragAsset;
use super::mesh_editor::{MeshEditorData, MeshEditorPanel, MeshPreviewState};
use crate::engine::assets::asset_type::AssetType;
use crate::engine::assets::mesh_import::MaterialSlot;
use crate::engine::assets::AssetId;

const TOOLBAR_H: f32 = 28.0;
const DETAILS_W: f32 = 300.0;
/// Material thumbnail edge in the slot card.
const THUMB: f32 = 64.0;
/// Bottom save-bar height.
const SAVE_BAR_H: f32 = 40.0;
/// srgb(220,180,50) — the dirty-star color.
const DIRTY_STAR: Color = Color::rgba(0.715694, 0.456411, 0.031896, 1.0);
/// srgb(30,31,35) — engine theme `header_bg` (same constant as other ports).
const HEADER_BG: Color = Color::rgba(0.012983, 0.013702, 0.016807, 1.0);

fn gray(v: u8) -> Color {
    Color::from_srgb_u8(v, v, v, 255)
}

/// Weak text: gamma-blended halfway toward the panel fill (`text_dim` alone
/// is the *normal* label color, too bright here).
fn weak(ui: &Ui) -> Color {
    let t = ui.style().palette.text_secondary.to_srgb_u8();
    let p = ui.style().palette.panel.to_srgb_u8();
    Color::from_srgb_u8(
        (t[0] >> 1) + (p[0] >> 1),
        (t[1] >> 1) + (p[1] >> 1),
        (t[2] >> 1) + (p[2] >> 1),
        255,
    )
}

/// Borrowed mesh-editor state.
pub struct MeshEditorPanelCtx<'a> {
    pub data: &'a mut MeshEditorData,
    /// Preview texture id in *this window's* crusty registry (None until
    /// the render target is registered).
    pub texture: Option<TextureId>,
    pub asset_browser: &'a mut AssetBrowserPanel,
    /// GPU-uploaded editor icon set keyed by file stem.
    pub icons: &'a HashMap<String, TextureId>,
    /// Material-thumbnail ids in *this window's* registry. `Some` for float
    /// windows (main-registry ids don't apply there), `None` for the main
    /// window, which reads the shared `ThumbnailCache` directly.
    pub float_thumbs: Option<&'a HashMap<AssetId, TextureId>>,
}

/// Render the mesh editor into `tab_rect` (physical pixels).
pub fn mesh_editor_panel(ui: &mut Ui, tab_rect: Rect, ctx: MeshEditorPanelCtx) {
    let rect = tab_rect;
    let opts = UiOptions {
        padding: Vec2::ZERO,
        spacing: 0.0,
    };
    let panel_id = Id::new("engine_mesh_editor").with(ctx.data.mesh_path.as_str());
    ui.run_at(rect, Direction::TopDown, panel_id, opts, |ui| {
        let s = 1.0_f32;
        let MeshEditorPanelCtx {
            data,
            texture,
            asset_browser,
            icons,
            float_thumbs,
        } = ctx;

        let toolbar_rect = Rect::from_min_size(rect.min, Vec2::new(rect.width(), TOOLBAR_H * s));
        toolbar(ui, toolbar_rect, s, data);

        let details_w = (DETAILS_W * s).min(rect.width() * 0.5);
        let details_rect = Rect::from_min_max(
            Pos2::new(rect.max.x - details_w, toolbar_rect.max.y),
            rect.max,
        );
        details(
            ui,
            details_rect,
            s,
            panel_id,
            data,
            asset_browser,
            icons,
            float_thumbs,
        );

        let preview_rect = Rect::from_min_max(
            Pos2::new(rect.min.x, toolbar_rect.max.y),
            Pos2::new(details_rect.min.x, rect.max.y),
        );
        preview(ui, preview_rect, panel_id, data, texture);
    });
}

// ── toolbar ─────────────────────────────────────────────────────────────────

fn toolbar(ui: &mut Ui, rect: Rect, s: f32, data: &MeshEditorData) {
    let stroke = ui.style().palette.stroke;
    let font = ui.style().fonts.body;
    ui.painter().rect_filled(rect, 0.0, HEADER_BG);
    ui.painter().line_segment(
        Pos2::new(rect.min.x, rect.max.y),
        Pos2::new(rect.max.x, rect.max.y),
        1.0,
        stroke,
    );

    let text_y = rect.center().y - font * 1.25 * 0.5;
    let mut x = rect.min.x + 8.0 * s;

    let w = ui
        .painter()
        .text(Pos2::new(x, text_y), "Mesh Editor", font, gray(230), None)
        .x;
    x += w + 10.0 * s;

    ui.painter().line_segment(
        Pos2::new(x, rect.min.y + 7.0 * s),
        Pos2::new(x, rect.max.y - 7.0 * s),
        1.0,
        gray(60),
    );
    x += 10.0 * s;

    let filename = filename_of(&data.mesh_path);
    let w = ui
        .painter()
        .text(Pos2::new(x, text_y), &filename, font, gray(200), None)
        .x;
    x += w + 6.0 * s;

    if data.dirty {
        ui.painter()
            .text(Pos2::new(x, text_y), "*", font, DIRTY_STAR, None);
    }
}

// ── details panel ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn details(
    ui: &mut Ui,
    rect: Rect,
    s: f32,
    panel_id: Id,
    data: &mut MeshEditorData,
    asset_browser: &mut AssetBrowserPanel,
    icons: &HashMap<String, TextureId>,
    float_thumbs: Option<&HashMap<AssetId, TextureId>>,
) {
    // Left edge separator against the preview.
    let stroke = ui.style().palette.stroke;
    ui.painter().line_segment(
        Pos2::new(rect.min.x, rect.min.y),
        Pos2::new(rect.min.x, rect.max.y),
        1.0,
        stroke,
    );

    let save_bar = Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - SAVE_BAR_H * s), rect.max);
    let body_rect = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, save_bar.min.y));

    let opts = UiOptions {
        padding: Vec2::new(8.0 * s, 6.0 * s),
        spacing: 4.0 * s,
    };
    ui.run_at(
        body_rect,
        Direction::TopDown,
        panel_id.with("details"),
        opts,
        |ui| {
            let style = ui.style();
            let heading = style.fonts.title;
            let small = style.fonts.small;
            let dim = weak(ui);

            // Header: filename + dirty star.
            let filename = filename_of(&data.mesh_path);
            ui.horizontal(|ui| {
                Label::new(filename).size(heading).show(ui);
                if data.dirty {
                    Label::new("*").size(heading).color(DIRTY_STAR).show(ui);
                }
            });
            Label::new(data.mesh_path.clone())
                .size(small)
                .color(dim)
                .show(ui);
            ui.separator();

            // "Material Slots: N" + add-circle button, right-aligned.
            let font = style.fonts.body;
            let row_h = 20.0 * s;
            let row = ui.allocate(Vec2::new(ui.available_size().x, row_h));
            ui.painter().text(
                Pos2::new(row.min.x, row.center().y - font * 1.25 * 0.5),
                &format!("Material Slots: {}", data.meta.material_slots.len()),
                font,
                style.palette.text_secondary,
                None,
            );
            let add_rect =
                Rect::from_min_size(Pos2::new(row.max.x - row_h, row.min.y), Vec2::splat(row_h));
            if icon_button(
                ui,
                panel_id.with("add_slot"),
                add_rect,
                s,
                icons.get("add-circle").copied(),
                "+",
                true,
                "Add material slot",
            ) {
                let idx = data.meta.material_slots.len();
                data.meta.material_slots.push(MaterialSlot {
                    name: format!("Slot {}", idx),
                    material_path: String::new(),
                });
                data.dirty = true;
            }

            if !data.meta.source.is_empty() {
                Label::new(format!("Source: {}", data.meta.source))
                    .size(small)
                    .color(dim)
                    .show(ui);
            }
            ui.separator();

            // Slots the mesh was imported with can't be removed; extras can.
            let floor = data
                .preview
                .as_ref()
                .map(|p| p.mesh_indices.len())
                .filter(|n| *n > 0);

            let avail_h = ui.available_size().y;
            ScrollArea::new(avail_h)
                .auto_shrink(false)
                .inset(0.0)
                .spacing(4.0 * s)
                .show(ui, |ui| {
                    if data.meta.material_slots.is_empty() {
                        Label::new("No material slots. Re-import with materials enabled.")
                            .color(dim)
                            .show(ui);
                    } else {
                        let mut changed = false;
                        let mut remove: Option<usize> = None;
                        for (i, slot) in data.meta.material_slots.iter_mut().enumerate() {
                            let title = format!("[{}] {}", i, slot.name);
                            let removable = floor.is_some_and(|n| i >= n);
                            CollapsingHeader::new(title)
                                .default_open(true)
                                .show(ui, |ui| {
                                    let act = slot_card(
                                        ui,
                                        panel_id.with(("slot", i)),
                                        s,
                                        slot,
                                        asset_browser,
                                        icons,
                                        float_thumbs,
                                        removable,
                                    );
                                    changed |= act.changed;
                                    if act.remove {
                                        remove = Some(i);
                                    }
                                });
                        }
                        if let Some(i) = remove {
                            data.meta.material_slots.remove(i);
                            changed = true;
                        }
                        if changed {
                            data.dirty = true;
                        }
                    }
                });
        },
    );

    save_bar_ui(ui, save_bar, s, panel_id, data, icons);
}

/// Bottom bar with the Save button pinned to the right ("like other
/// engines"). Icon reflects state: save-changes (dirty) / save-check.
fn save_bar_ui(
    ui: &mut Ui,
    rect: Rect,
    s: f32,
    panel_id: Id,
    data: &mut MeshEditorData,
    icons: &HashMap<String, TextureId>,
) {
    let style = ui.style();
    ui.painter().rect_filled(rect, 0.0, HEADER_BG);
    ui.painter().line_segment(
        Pos2::new(rect.min.x, rect.min.y),
        Pos2::new(rect.max.x, rect.min.y),
        1.0,
        style.palette.stroke,
    );

    let font = style.fonts.body;
    let btn_h = 26.0 * s;
    let icon_px = 16.0 * s;
    let label = "Save";
    let text_w = ui.painter().measure_text(label, font, None).x;
    let pad = 10.0 * s;
    let gap = 6.0 * s;
    let btn_w = pad + icon_px + gap + text_w + pad;
    let btn = Rect::from_min_size(
        Pos2::new(rect.max.x - btn_w - 8.0 * s, rect.center().y - btn_h * 0.5),
        Vec2::new(btn_w, btn_h),
    );

    let enabled = data.dirty;
    let id = panel_id.with("save_btn");
    let resp = ui.interact(id, btn);
    let hovered = resp.hovered && enabled;
    let fill = if hovered {
        style.palette.hover
    } else {
        style.palette.input
    };
    ui.painter().rect_filled(btn, 4.0, fill);
    let stroke = if enabled {
        style.palette.accent_active
    } else {
        style.palette.stroke
    };
    ui.painter().rect_stroke(btn, 4.0, 1.0, stroke);

    let icon_key = if data.dirty {
        "save-changes"
    } else {
        "save-check"
    };
    let tint = if enabled { gray(230) } else { gray(120) };
    let icon_rect = Rect::from_min_size(
        Pos2::new(btn.min.x + pad, (btn.center().y - icon_px * 0.5).round()),
        Vec2::splat(icon_px),
    );
    if let Some(tex) = icons.get(icon_key).copied() {
        ui.painter().paint_mut().push(PaintCmd::Image {
            rect: icon_rect,
            uv_min: Pos2::new(0.0, 0.0),
            uv_max: Pos2::new(1.0, 1.0),
            tint,
            texture: tex,
        });
    }
    ui.painter().text(
        Pos2::new(icon_rect.max.x + gap, btn.center().y - font * 1.25 * 0.5),
        label,
        font,
        tint,
        None,
    );
    if resp.hovered {
        let tip = if data.dirty {
            "Save changes"
        } else {
            "All changes saved"
        };
        show_tooltip_for(ui, btn, tip);
    }

    if enabled && resp.clicked {
        match MeshEditorPanel::save_sidecar(data) {
            Ok(()) => data.dirty = false,
            Err(e) => log::error!("Failed to save mesh sidecar: {}", e),
        }
    }
}

// ── material slot card ──────────────────────────────────────────────────────

#[derive(Default)]
struct SlotAction {
    changed: bool,
    remove: bool,
}

/// One material slot, Unreal-style: thumbnail + searchable dropdown, with
/// undo (clear to "None") and trash (remove added slot) below the dropdown.
/// Accepts material drag-and-drop from the asset browser onto the card.
#[allow(clippy::too_many_arguments)]
fn slot_card(
    ui: &mut Ui,
    id: Id,
    s: f32,
    slot: &mut MaterialSlot,
    asset_browser: &mut AssetBrowserPanel,
    icons: &HashMap<String, TextureId>,
    float_thumbs: Option<&HashMap<AssetId, TextureId>>,
    removable: bool,
) -> SlotAction {
    let mut act = SlotAction::default();
    let card_h = (THUMB + 12.0) * s;
    let rect = ui.allocate(Vec2::new(ui.available_size().x, card_h));

    // ── DnD from the asset browser (same window).
    let mut hover_valid = false;
    if ui.dnd_hovering::<DragAsset>(rect) {
        if let Some(p) = ui.ctx().dnd.peek::<DragAsset>() {
            hover_valid = p.asset_type == AssetType::Material;
        }
    }
    if let Some(drop) = ui.dnd_drop_target::<DragAsset>(rect) {
        if drop.asset_type == AssetType::Material {
            let path = drop.path.to_string_lossy().to_string();
            if slot.material_path != path {
                slot.material_path = path;
                act.changed = true;
            }
        }
    }

    // ── Card background (blue highlight while a material hovers).
    let (bg, border) = if hover_valid {
        (
            Color::from_srgb_u8(40, 60, 90, 255),
            Color::from_srgb_u8(100, 180, 255, 255),
        )
    } else {
        (gray(35), gray(60))
    };
    ui.painter().rect_filled(rect, 4.0, bg);
    ui.painter().rect_stroke(rect, 4.0, 1.0, border);

    // ── Thumbnail.
    let thumb_rect = Rect::from_min_size(
        Pos2::new(rect.min.x + 6.0 * s, rect.min.y + 6.0 * s),
        Vec2::splat(THUMB * s),
    );
    draw_material_thumb(
        ui,
        thumb_rect,
        &slot.material_path,
        asset_browser,
        float_thumbs,
    );

    // ── Right column: dropdown on top, undo/trash icons below.
    let col = Rect::from_min_max(
        Pos2::new(thumb_rect.max.x + 8.0 * s, rect.min.y + 6.0 * s),
        Pos2::new(rect.max.x - 6.0 * s, rect.max.y - 6.0 * s),
    );
    let opts = UiOptions {
        padding: Vec2::ZERO,
        spacing: 8.0 * s,
    };
    ui.run_at(col, Direction::TopDown, id.with("col"), opts, |ui| {
        act.changed |= material_picker(
            ui,
            id,
            col.width(),
            &mut slot.material_path,
            asset_browser,
            icons,
            float_thumbs,
        );

        ui.horizontal(|ui| {
            let sz = 22.0 * s;
            let undo_rect = ui.allocate(Vec2::splat(sz));
            if icon_button(
                ui,
                id.with("undo"),
                undo_rect,
                s,
                icons.get("undo").copied(),
                "\u{21A9}",
                !slot.material_path.is_empty(),
                "Clear material (None)",
            ) {
                slot.material_path.clear();
                act.changed = true;
            }
            let trash_rect = ui.allocate(Vec2::splat(sz));
            if icon_button(
                ui,
                id.with("trash"),
                trash_rect,
                s,
                icons.get("trash").copied(),
                "\u{2716}",
                removable,
                if removable {
                    "Remove slot"
                } else {
                    "Imported slots can't be removed"
                },
            ) {
                act.remove = true;
            }
        });
    });

    act
}

/// Paint the material thumbnail (or a placeholder for "None"/pending).
fn draw_material_thumb(
    ui: &mut Ui,
    rect: Rect,
    material_path: &str,
    asset_browser: &mut AssetBrowserPanel,
    float_thumbs: Option<&HashMap<AssetId, TextureId>>,
) {
    let tex = if material_path.is_empty() {
        None
    } else {
        material_thumb(asset_browser, float_thumbs, material_path)
    };
    match tex {
        Some(tex) => {
            ui.painter().paint_mut().push(PaintCmd::Image {
                rect,
                uv_min: Pos2::new(0.0, 0.0),
                uv_max: Pos2::new(1.0, 1.0),
                tint: Color::WHITE,
                texture: tex,
            });
            ui.painter().rect_stroke(rect, 2.0, 1.0, gray(70));
        }
        None => {
            ui.painter().rect_filled(rect, 2.0, gray(50));
            ui.painter().rect_stroke(rect, 2.0, 1.0, gray(70));
            if material_path.is_empty() {
                let font = ui.style().fonts.small;
                let w = ui.painter().measure_text("None", font, None).x;
                ui.painter().text(
                    Pos2::new(
                        rect.center().x - w * 0.5,
                        rect.center().y - font * 1.25 * 0.5,
                    ),
                    "None",
                    font,
                    gray(120),
                    None,
                );
            }
        }
    }
}

/// This-window texture id for a material's thumbnail, requesting generation
/// through the shared cache when missing.
fn material_thumb(
    asset_browser: &mut AssetBrowserPanel,
    float_thumbs: Option<&HashMap<AssetId, TextureId>>,
    material_path: &str,
) -> Option<TextureId> {
    let asset_id = AssetId::from_path(material_path);
    let meta = asset_browser.registry.get(asset_id)?;
    // Always drives generation + main-registry upload (and rgba retention
    // for float windows).
    let main_id = asset_browser.thumbnails.crusty_texture_id(meta);
    match float_thumbs {
        Some(map) => map.get(&asset_id).copied(),
        None => main_id,
    }
}

// ── material picker dropdown ────────────────────────────────────────────────

/// Searchable material dropdown with thumbnails and an Unreal-style
/// folder-tree toggle. Returns true when the assignment changed.
fn material_picker(
    ui: &mut Ui,
    id: Id,
    width: f32,
    material_path: &mut String,
    asset_browser: &mut AssetBrowserPanel,
    icons: &HashMap<String, TextureId>,
    float_thumbs: Option<&HashMap<AssetId, TextureId>>,
) -> bool {
    let mut changed = false;
    let display = if material_path.is_empty() {
        "None".to_string()
    } else {
        filename_of(material_path)
    };
    let s = 1.0; // popup content is in points like the rest of the panel
    ComboBox::new(format!("mesh_mat_slot_{:?}", id))
        .selected_text(display)
        .width(width)
        .popup_width(width.max(240.0))
        .max_popup_height(260.0)
        .show_ui(ui, |ui| {
            let dim = weak(ui);
            let small = ui.style().fonts.small;

            // Persistent search + tree-mode state for this slot.
            let sid = id.with("search");
            let mut search = ui
                .ctx()
                .memory
                .data_get::<String>(sid)
                .cloned()
                .unwrap_or_default();
            let tid = id.with("tree_mode");
            let mut tree_mode = ui
                .ctx()
                .memory
                .data_get::<bool>(tid)
                .copied()
                .unwrap_or(false);

            ui.horizontal(|ui| {
                let btn = 20.0;
                let w = (ui.available().width() - btn - 6.0).max(60.0);
                TextEdit::new(&mut search)
                    .width(w)
                    .height(18.0)
                    .hint("Search...")
                    .show_full(ui);
                let tree_rect = ui.allocate(Vec2::splat(btn));
                if icon_toggle(
                    ui,
                    id.with("tree_btn"),
                    tree_rect,
                    s,
                    icons.get("tree-view").copied(),
                    "\u{2263}",
                    tree_mode,
                    "Folder tree view",
                ) {
                    tree_mode = !tree_mode;
                }
            });
            ui.ctx_mut().memory.data_insert(sid, search.clone());
            ui.ctx_mut().memory.data_insert(tid, tree_mode);

            let mut chosen: Option<String> = None;

            // "None" entry.
            if picker_row(
                ui,
                id.with("none"),
                None,
                "None",
                material_path.is_empty(),
                0.0,
            ) {
                chosen = Some(String::new());
            }

            let filter = AssetFilter {
                search_text: if search.is_empty() {
                    None
                } else {
                    Some(search)
                },
                asset_types: Some(vec![AssetType::Material]),
                include_subfolders: true,
                ..Default::default()
            };
            let mut results: Vec<(AssetId, String, String, String)> = asset_browser
                .registry
                .query(&filter)
                .into_iter()
                .map(|m| {
                    let path = m.path.to_string_lossy().to_string();
                    let folder = std::path::Path::new(&path)
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default();
                    (m.id, m.display_name.clone(), path, folder)
                })
                .collect();
            if results.is_empty() {
                Label::new("No materials found")
                    .size(small)
                    .color(dim)
                    .show(ui);
            }

            let indent = if tree_mode { 14.0 } else { 0.0 };
            if tree_mode {
                results.sort_by(|a, b| a.3.cmp(&b.3).then_with(|| a.1.cmp(&b.1)));
            }
            let mut last_folder: Option<String> = None;
            for (asset_id, name, path, folder) in results {
                if tree_mode && last_folder.as_deref() != Some(folder.as_str()) {
                    let label = if folder.is_empty() {
                        "/"
                    } else {
                        folder.as_str()
                    };
                    Label::new(label.to_string())
                        .size(small)
                        .color(dim)
                        .show(ui);
                    last_folder = Some(folder.clone());
                }
                let thumb = {
                    let meta = asset_browser.registry.get(asset_id);
                    meta.and_then(|m| {
                        let main_id = asset_browser.thumbnails.crusty_texture_id(m);
                        match float_thumbs {
                            Some(map) => map.get(&asset_id).copied(),
                            None => main_id,
                        }
                    })
                };
                let selected = material_path.as_str() == path;
                if picker_row(ui, id.with(("mat", &path)), thumb, &name, selected, indent) {
                    chosen = Some(path);
                }
            }

            if let Some(c) = chosen {
                if *material_path != c {
                    *material_path = c;
                    changed = true;
                }
            }
        });
    changed
}

/// Truncate `text` with a trailing ellipsis so it fits in `max_w`.
fn elide_to(ui: &mut Ui, text: &str, font: f32, max_w: f32) -> String {
    if ui.text_mut().measure(text, font, None).x <= max_w {
        return text.to_string();
    }
    let mut s = text.to_string();
    while !s.is_empty() {
        s.pop();
        let candidate = format!("{s}\u{2026}");
        if ui.text_mut().measure(&candidate, font, None).x <= max_w {
            return candidate;
        }
    }
    "\u{2026}".into()
}

/// One dropdown row: mini thumbnail + name. Mirrors `SelectableValue`'s
/// styling (accent fill when selected) and closes the popup on click.
fn picker_row(
    ui: &mut Ui,
    id: Id,
    thumb: Option<TextureId>,
    label: &str,
    selected: bool,
    indent: f32,
) -> bool {
    let style = ui.style();
    let font = style.fonts.body;
    let h = 22.0;
    let full = ui.allocate(Vec2::new(ui.available().width(), h));
    let rect = Rect::from_min_max(Pos2::new(full.min.x + indent, full.min.y), full.max);
    let resp = ui.interact(id, rect);

    let fill = if selected {
        let mut accent = style.palette.accent_active;
        accent.a = 0.85;
        accent
    } else if resp.hovered {
        style.palette.hover
    } else {
        Color::TRANSPARENT
    };
    if fill.a > 0.0 {
        ui.painter().rect_filled(rect, 3.0, fill);
    }

    let thumb_sz = 16.0;
    let thumb_rect = Rect::from_min_size(
        Pos2::new(rect.min.x + 4.0, (rect.center().y - thumb_sz * 0.5).round()),
        Vec2::splat(thumb_sz),
    );
    match thumb {
        Some(tex) => ui.painter().paint_mut().push(PaintCmd::Image {
            rect: thumb_rect,
            uv_min: Pos2::new(0.0, 0.0),
            uv_max: Pos2::new(1.0, 1.0),
            tint: Color::WHITE,
            texture: tex,
        }),
        None => ui.painter().rect_filled(thumb_rect, 2.0, gray(50)),
    }

    let text_color = if selected {
        Color::rgba(0.10, 0.10, 0.12, 0.95)
    } else {
        style.palette.text
    };
    let text_x = thumb_rect.max.x + 6.0;
    let label = elide_to(ui, label, font, rect.max.x - 4.0 - text_x);
    ui.painter().text(
        Pos2::new(text_x, rect.center().y - font * 1.25 * 0.5),
        &label,
        font,
        text_color,
        None,
    );

    if resp.clicked {
        ui.ctx_mut().memory.popup_close_requested = true;
        return true;
    }
    false
}

// ── icon buttons ────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn icon_button(
    ui: &mut Ui,
    id: Id,
    rect: Rect,
    s: f32,
    icon: Option<TextureId>,
    fallback: &str,
    enabled: bool,
    tooltip: &str,
) -> bool {
    icon_button_impl(ui, id, rect, s, icon, fallback, enabled, false, tooltip)
}

/// Icon button with a persistent "on" state (accent stroke).
#[allow(clippy::too_many_arguments)]
fn icon_toggle(
    ui: &mut Ui,
    id: Id,
    rect: Rect,
    s: f32,
    icon: Option<TextureId>,
    fallback: &str,
    on: bool,
    tooltip: &str,
) -> bool {
    icon_button_impl(ui, id, rect, s, icon, fallback, true, on, tooltip)
}

#[allow(clippy::too_many_arguments)]
fn icon_button_impl(
    ui: &mut Ui,
    id: Id,
    rect: Rect,
    s: f32,
    icon: Option<TextureId>,
    fallback: &str,
    enabled: bool,
    on: bool,
    tooltip: &str,
) -> bool {
    let style = ui.style();
    let resp = ui.interact(id, rect);
    let hovered = resp.hovered && enabled;
    let fill = if hovered {
        style.palette.hover
    } else {
        style.palette.input
    };
    ui.painter().rect_filled(rect, 4.0, fill);
    if on {
        ui.painter()
            .rect_stroke(rect, 4.0, 1.0, style.palette.accent_active);
    } else if hovered {
        ui.painter().rect_stroke(rect, 4.0, 1.0, gray(100));
    }

    let tint = if !enabled {
        gray(90)
    } else if hovered || on {
        gray(240)
    } else {
        gray(200)
    };
    match icon {
        Some(texture) => {
            let sz = (16.0 * s).min(rect.height() - 4.0);
            let c = rect.center();
            let irect = Rect::from_min_size(
                Pos2::new((c.x - sz * 0.5).round(), (c.y - sz * 0.5).round()),
                Vec2::splat(sz),
            );
            ui.painter().paint_mut().push(PaintCmd::Image {
                rect: irect,
                uv_min: Pos2::new(0.0, 0.0),
                uv_max: Pos2::new(1.0, 1.0),
                tint,
                texture,
            });
        }
        None => {
            let font = style.fonts.body;
            let w = ui.painter().measure_text(fallback, font, None).x;
            ui.painter().text(
                Pos2::new(
                    rect.center().x - w * 0.5,
                    rect.center().y - font * 1.25 * 0.5,
                ),
                fallback,
                font,
                tint,
                None,
            );
        }
    }

    if resp.hovered {
        show_tooltip_for(ui, rect, tooltip);
    }
    enabled && resp.clicked
}

// ── 3D preview ──────────────────────────────────────────────────────────────

fn preview(
    ui: &mut Ui,
    rect: Rect,
    panel_id: Id,
    data: &mut MeshEditorData,
    texture: Option<TextureId>,
) {
    let dim = weak(ui);
    let Some(preview) = data.preview.as_mut() else {
        centered_text(ui, rect, "Loading preview...", dim);
        return;
    };

    // Track desired render size (drives texture resize on the host side).
    let w = rect.width().floor().max(1.0) as u32;
    let h = rect.height().floor().max(1.0) as u32;
    preview.size = (w, h);

    if preview.mesh_indices.is_empty() {
        let c = rect.center();
        centered_text_at(ui, Pos2::new(c.x, c.y - 12.0), "3D Preview", 14.0, dim);
        centered_text_at(
            ui,
            Pos2::new(c.x, c.y + 8.0),
            "Mesh not loaded on GPU",
            11.0,
            dim,
        );
        return;
    }
    let Some(tex) = texture else {
        centered_text(ui, rect, "Rendering preview...", dim);
        return;
    };

    ui.painter().paint_mut().push(PaintCmd::Image {
        rect,
        uv_min: Pos2::new(0.0, 0.0),
        uv_max: Pos2::new(1.0, 1.0),
        tint: Color::WHITE,
        texture: tex,
    });

    // ── orbit controls
    let id = panel_id.with("preview");
    let resp = ui.interact(id, rect);
    let input = &ui.ctx().input;
    let pointer = input.pointer_pos;
    let scroll_y = input.scroll_delta.y;
    let middle_down = input.middle_down;
    let middle_pressed = input.button_presses.contains(&MouseButton::Middle);

    // Middle-drag latch: starts over the preview, ends on release anywhere.
    let mid_id = id.with("mid_drag");
    let mut mid_drag = ui
        .ctx()
        .memory
        .data_get::<bool>(mid_id)
        .copied()
        .unwrap_or(false);
    if middle_pressed && resp.hovered {
        mid_drag = true;
    }
    if !middle_down {
        mid_drag = false;
    }

    // Pointer delta from the last frame's position.
    let last_id = id.with("last_pos");
    let last = ui.ctx().memory.data_get::<Pos2>(last_id).copied();
    let delta = match (pointer, last) {
        (Some(p), Some(l)) => Vec2::new(p.x - l.x, p.y - l.y),
        _ => Vec2::ZERO,
    };
    {
        let mem = &mut ui.ctx_mut().memory;
        mem.data_insert(mid_id, mid_drag);
        if let Some(p) = pointer {
            mem.data_insert(last_id, p);
        }
    }

    // Left-drag rotates.
    if resp.pressed && (delta.x != 0.0 || delta.y != 0.0) {
        let sensitivity = 0.005;
        preview.orbit_yaw += delta.x * sensitivity;
        preview.orbit_pitch -= delta.y * sensitivity;
        preview.orbit_pitch = preview
            .orbit_pitch
            .clamp(-89.0_f32.to_radians(), 89.0_f32.to_radians());
        data.preview_dirty = true;
    }

    // Middle-drag pans.
    if mid_drag && (delta.x != 0.0 || delta.y != 0.0) {
        let pan_speed = preview.orbit_distance * 0.002;
        let forward = (preview.orbit_target - camera_position(preview)).normalize();
        let right = forward.cross(Vec3::Y).normalize();
        let up = right.cross(forward).normalize();
        preview.orbit_target -= right * delta.x * pan_speed;
        preview.orbit_target += up * delta.y * pan_speed;
        data.preview_dirty = true;
    }

    // Scroll zooms.
    if resp.hovered && scroll_y.abs() > 0.0 {
        let zoom_factor = 1.0 - scroll_y * 0.002;
        preview.orbit_distance *= zoom_factor;
        let min_dist = preview.mesh_radius * 0.1;
        let max_dist = preview.mesh_radius * 50.0;
        preview.orbit_distance = preview.orbit_distance.clamp(min_dist, max_dist);
        data.preview_dirty = true;
    }
}

fn camera_position(preview: &MeshPreviewState) -> Vec3 {
    let x = preview.orbit_distance * preview.orbit_pitch.cos() * preview.orbit_yaw.sin();
    let y = preview.orbit_distance * preview.orbit_pitch.sin();
    let z = preview.orbit_distance * preview.orbit_pitch.cos() * preview.orbit_yaw.cos();
    preview.orbit_target + Vec3::new(x, y, z)
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn filename_of(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

fn centered_text(ui: &mut Ui, rect: Rect, text: &str, color: Color) {
    let c = rect.center();
    centered_text_at(ui, c, text, 13.0, color);
}

fn centered_text_at(ui: &mut Ui, center: Pos2, text: &str, font: f32, color: Color) {
    let w = ui.painter().measure_text(text, font, None).x;
    ui.painter().text(
        Pos2::new(center.x - w * 0.5, center.y - font * 1.25 * 0.5),
        text,
        font,
        color,
        None,
    );
}
