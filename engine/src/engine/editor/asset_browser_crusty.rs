//! Asset Browser panel rendered with crusty-gui.
//!
//! Uses [`AssetBrowserPanel`] (registry, selection, events, rename/delete/
//! drag state) from `asset_browser/`. List-view headers actually sort,
//! Enter confirms the delete dialog, Escape cancels an in-flight drag.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::SystemTime;

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::input::{Key, Modifiers, Shortcut};
use crusty_gui::math::{Color, Pos2, Rect, Vec2};
use crusty_gui::paint::{PaintCmd, TextureId};
use crusty_gui::widgets::{Button, ComboBox, ScrollArea, Slider, Splitter, TextEdit, Window};

use super::asset_browser::{
    AssetBrowserEvent, AssetBrowserPanel, AssetDragPayload, DeleteConfirmation, DeleteTarget,
    DragPayload, FolderNode, RenameTarget, ViewMode,
};
use crate::engine::assets::{AssetId, AssetType};

/// dnd payload carried while an asset card / row is dragged.
pub(crate) struct DragAsset {
    pub(crate) id: AssetId,
    pub(crate) asset_type: AssetType,
    pub(crate) path: PathBuf,
}

/// dnd payload carried while a folder-tree row is dragged.
struct DragFolder {
    path: PathBuf,
    name: String,
}

/// List-view sort state, kept in crusty memory.
#[derive(Clone, Copy)]
struct ListSortState {
    col: u8, // 0 = Name, 1 = Type, 2 = Size, 3 = Modified
    asc: bool,
}

/// Which rename target the inline editor was last focused for, so focus is
/// requested exactly once per rename session.
#[derive(Clone, Copy)]
struct RenameFocusKey(u64);

/// Live text of the inline rename editor.
#[derive(Clone)]
struct RenameBuffer(String);

/// Borrowed asset-browser state.
pub struct AssetBrowserPanelCtx<'a> {
    pub panel: &'a mut AssetBrowserPanel,
    /// GPU-uploaded editor icon set keyed by file stem
    /// (`RenderEvent::RenderThreadReady::crusty_icons`).
    pub icons: &'a HashMap<String, TextureId>,
}

/// Owned per-frame snapshot of one visible asset so rendering can mutate
/// the panel freely.
struct AssetRow {
    id: AssetId,
    name: String,
    label: String,
    asset_type: AssetType,
    size: u64,
    size_str: String,
    modified: SystemTime,
    modified_str: String,
    path: PathBuf,
    thumb: Option<TextureId>,
}

fn hash_key<T: Hash>(t: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

/// Asset types map onto the design system's **assets** map (ramp indices,
/// bright tone) — tile type edges and typed icons. Same slot assignments the
/// browser always used, now resolved by key instead of by field.
pub(super) fn type_color(t: AssetType) -> Color {
    super::theme::asset_color(match t {
        AssetType::Texture | AssetType::Material => "materials",
        AssetType::Model | AssetType::Mesh | AssetType::Prefab => "geometry",
        AssetType::Animation => "animation",
        AssetType::Scene => "lights",
        AssetType::Audio => "audio",
        AssetType::Shader
        | AssetType::InputAction
        | AssetType::InputMappingContext
        | AssetType::Graph => "scripting",
        // A curve is animation data — it shares a slot with `.anim` because
        // Task 41 grows this exact asset into animation channels, and the two
        // reading alike is the point rather than an accident. An `.animgraph`
        // is the same family: the machine that plays those clips.
        AssetType::Curve | AssetType::AnimGraph | AssetType::BlendSpace => "animation",
        _ => "geometry",
    })
}

pub(super) fn type_icon_stem(t: AssetType) -> &'static str {
    match t {
        AssetType::Texture => "image-file",
        AssetType::Model | AssetType::Mesh => "file-mesh",
        AssetType::Shader => "code-file",
        _ => "file-document",
    }
}

/// Rough y-m-d from the unix epoch — cheap approximation used by the list
/// view; not calendar-accurate.
fn format_date(t: SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    let years = 1970 + days / 365;
    let rem = days % 365;
    let month = rem / 30 + 1;
    let day = rem % 30 + 1;
    format!("{years}-{month:02}-{day:02}")
}

fn truncate_to_width(ui: &mut Ui, text: &str, size: f32, max_w: f32) -> String {
    truncate_to_width_family(ui, text, size, max_w, crusty_gui::text::FontFamily::Ui)
}

fn truncate_to_width_family(
    ui: &mut Ui,
    text: &str,
    size: f32,
    max_w: f32,
    family: crusty_gui::text::FontFamily,
) -> String {
    let measure = |ui: &mut Ui, t: &str| ui.painter().measure_text_family(t, size, None, family).x;
    if measure(ui, text) <= max_w {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        out.push(ch);
        if measure(ui, &format!("{out}…")) > max_w {
            out.pop();
            break;
        }
    }
    out.push('…');
    out
}

/// Draw the asset browser into the dock tab's content rect (physical pixels).
pub fn asset_browser_panel(ui: &mut Ui, tab_rect: Rect, ctx: AssetBrowserPanelCtx) {
    let rect = tab_rect;
    let opts = UiOptions {
        padding: Vec2::new(0.0, 0.0),
        spacing: 0.0,
    };
    let panel_top = rect.min.y;
    ui.run_at(
        rect,
        Direction::TopDown,
        Id::new("engine_asset_browser_panel"),
        opts,
        |ui| {
            let AssetBrowserPanelCtx { panel, icons } = ctx;
            panel.process_rescan();

            // ── snapshot the visible assets ─────────────────────────────
            let sort_id = Id::new("ab_list_sort");
            let sort = ui
                .ctx()
                .memory
                .data_get::<ListSortState>(sort_id)
                .copied()
                .unwrap_or(ListSortState { col: 0, asc: true });
            ui.ctx_mut().memory.data_insert(sort_id, sort);

            let filter = panel.build_filter();
            let mut rows: Vec<AssetRow> = Vec::new();
            {
                let AssetBrowserPanel {
                    registry,
                    thumbnails,
                    ..
                } = &mut *panel;
                for meta in registry.query(&filter) {
                    let ext = meta.path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    let label = if ext.is_empty() {
                        meta.display_name.clone()
                    } else {
                        format!("{}.{ext}", meta.display_name)
                    };
                    rows.push(AssetRow {
                        id: meta.id,
                        name: meta.display_name.clone(),
                        label,
                        asset_type: meta.asset_type,
                        size: meta.file_size,
                        size_str: meta.formatted_size(),
                        modified: meta.last_modified,
                        modified_str: format_date(meta.last_modified),
                        path: meta.path.clone(),
                        thumb: thumbnails.crusty_texture_id(meta),
                    });
                }
            }
            if panel.view_mode == ViewMode::List {
                sort_rows(&mut rows, sort);
            }
            let visible_ids: Vec<AssetId> = rows.iter().map(|r| r.id).collect();
            let asset_count = rows.len();

            handle_keyboard(ui, panel, &visible_ids);

            // ── chrome ──────────────────────────────────────────────────
            ui.add_space((panel_top + 6.0 - ui.cursor().y).max(0.0));
            render_toolbar(ui, panel, icons, asset_count);
            ui.add_space((panel_top + 34.0 - ui.cursor().y).max(0.0));
            ui.separator();
            ui.add_space((panel_top + 44.0 - ui.cursor().y).max(0.0));
            render_breadcrumb(ui, panel);
            ui.add_space((panel_top + 70.0 - ui.cursor().y).max(0.0));
            ui.separator();
            ui.add_space((panel_top + 74.0 - ui.cursor().y).max(0.0));

            // paint the folder/grid area on the inner-panel fill
            // (surface[1]); only the toolbar/breadcrumb strips keep tab fill.
            let panel_fill = ui.style().palette.panel;
            ui.painter().rect_filled(
                Rect::from_min_max(Pos2::new(rect.min.x, panel_top + 74.0), rect.max),
                0.0,
                panel_fill,
            );

            // ── content: folder tree + asset views ──────────────────────
            // Splitter takes two closures; both need &mut panel, so pass it
            // through a RefCell (the closures run strictly one after the
            // other inside `show`).
            if panel.show_folders {
                let cell = std::cell::RefCell::new(&mut *panel);
                Splitter::horizontal("asset_browser_split")
                    // Folder rail default width 150.0 (design system).
                    .default_ratio((150.0 / rect.width().max(1.0)).clamp(0.1, 0.5))
                    .min_sizes(120.0, 200.0)
                    .show(
                        ui,
                        |ui| {
                            render_folder_pane(ui, &mut cell.borrow_mut(), icons);
                        },
                        |ui| {
                            render_content(ui, &mut cell.borrow_mut(), icons, &rows, &visible_ids);
                        },
                    );
            } else {
                render_content(ui, panel, icons, &rows, &visible_ids);
            }

            render_delete_confirmation(ui, panel);
            sync_drag_payload(ui, panel);
        },
    );
}

fn sort_rows(rows: &mut [AssetRow], sort: ListSortState) {
    rows.sort_by(|a, b| {
        let ord = match sort.col {
            1 => a.asset_type.display_name().cmp(b.asset_type.display_name()),
            2 => a.size.cmp(&b.size),
            3 => a.modified.cmp(&b.modified),
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };
        if sort.asc {
            ord
        } else {
            ord.reverse()
        }
    });
}

// ─── keyboard ───────────────────────────────────────────────────────────

fn handle_keyboard(ui: &mut Ui, panel: &mut AssetBrowserPanel, visible: &[AssetId]) {
    // The rename TextEdit handles Enter/Escape itself.
    if panel.renaming.is_some() {
        if ui.ctx().input.key_pressed(Key::Escape) {
            panel.renaming = None;
            panel.delete_confirmation = None;
        }
        return;
    }
    // Skip while a text field (search / console) has focus.
    if ui.ctx().memory.focused_widget.is_some() {
        return;
    }

    // Delete dialog owns the keyboard while open: Enter confirms
    // (improvement), Escape cancels.
    if panel.delete_confirmation.is_some() {
        if ui.ctx().input.key_pressed(Key::Enter) {
            confirm_delete(panel);
        } else if ui.ctx().input.key_pressed(Key::Escape) {
            panel.delete_confirmation = None;
        }
        return;
    }

    if ui.ctx().input.key_pressed(Key::Enter) {
        if let Some(id) = panel.selection.primary() {
            panel.events.push(AssetBrowserEvent::AssetOpened { id });
        }
    }

    if ui.ctx().input.key_pressed(Key::Delete) {
        if let Some(id) = panel.selection.primary() {
            if let Some(meta) = panel.registry.get(id) {
                let path = meta.path.clone();
                panel.delete_confirmation = Some(DeleteConfirmation {
                    target: DeleteTarget::Asset { id, path },
                    file_count: 1,
                });
            }
        } else if !panel.current_folder.as_os_str().is_empty() {
            let full_path = panel.registry.root_path().join(&panel.current_folder);
            if full_path.exists() {
                let file_count = std::fs::read_dir(&full_path)
                    .map(|entries| entries.count())
                    .unwrap_or(0);
                panel.delete_confirmation = Some(DeleteConfirmation {
                    target: DeleteTarget::Folder {
                        path: panel.current_folder.clone(),
                        is_empty: file_count == 0,
                    },
                    file_count,
                });
            }
        }
    }

    if ui.ctx().input.key_pressed(Key::F(2)) {
        if let Some(id) = panel.selection.primary() {
            if let Some(meta) = panel.registry.get(id) {
                panel.renaming = Some(RenameTarget::Asset {
                    id,
                    current_name: meta.display_name.clone(),
                });
            }
        } else if !panel.current_folder.as_os_str().is_empty() {
            let folder_name = panel
                .current_folder
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if !folder_name.is_empty() {
                panel.renaming = Some(RenameTarget::Folder {
                    path: panel.current_folder.clone(),
                    current_name: folder_name,
                });
            }
        }
    }

    // Escape priority chain: drag → search → selection (the dialog case
    // returned above).
    if ui.ctx().input.key_pressed(Key::Escape) {
        if ui.ctx().dnd.holds::<DragAsset>() || ui.ctx().dnd.holds::<DragFolder>() {
            ui.ctx_mut().dnd.clear();
            panel.drag_payload = None;
        } else if !panel.search_text.is_empty() {
            panel.search_text.clear();
        } else if !panel.selection.is_empty() {
            panel.selection.clear();
            panel.events.push(AssetBrowserEvent::SelectionCleared);
        }
    }

    if ui.ctx().input.key_pressed(Key::Backspace) && !panel.current_folder.as_os_str().is_empty() {
        if let Some(parent) = panel.current_folder.parent() {
            let parent_path = parent.to_path_buf();
            panel.current_folder = parent_path.clone();
            panel
                .events
                .push(AssetBrowserEvent::FolderChanged { path: parent_path });
        }
    }

    if ui.ctx().input.key_pressed(Key::F(5)) {
        panel.request_rescan();
    }

    if visible.is_empty() {
        return;
    }

    if ui.ctx().input.key_pressed(Key::ArrowDown) {
        if let Some(primary) = panel.selection.primary() {
            if let Some(idx) = visible.iter().position(|&id| id == primary) {
                if idx + 1 < visible.len() {
                    panel.selection.select(visible[idx + 1]);
                }
            }
        } else {
            panel.selection.select(visible[0]);
        }
    }
    if ui.ctx().input.key_pressed(Key::ArrowUp) {
        if let Some(primary) = panel.selection.primary() {
            if let Some(idx) = visible.iter().position(|&id| id == primary) {
                if idx > 0 {
                    panel.selection.select(visible[idx - 1]);
                }
            }
        } else {
            panel.selection.select(visible[visible.len() - 1]);
        }
    }
    if ui
        .ctx_mut()
        .input
        .consume_shortcut(Shortcut::ctrl(Key::Char('a')))
    {
        for &id in visible {
            panel.selection.add(id);
        }
    }
}

// ─── toolbar ────────────────────────────────────────────────────────────

fn icon_toggle(
    ui: &mut Ui,
    icons: &HashMap<String, TextureId>,
    stem: &str,
    fallback: &str,
    active: bool,
    tooltip: &str,
) -> bool {
    let style = ui.style();
    let resp = if let Some(&tex) = icons.get(stem) {
        // Idle: bare icon. Active: lit accent-soft toggle (accent border).
        let rect = ui.allocate(Vec2::splat(20.0));
        let resp = ui.interact(Id::new("ab_icon_toggle").with(stem), rect);
        if active {
            ui.painter()
                .rect_filled(rect, 3.0, style.palette.accent_soft);
            ui.painter().rect_stroke(
                rect,
                3.0,
                style.metrics.border,
                style.palette.accent_active,
            );
        } else if resp.hovered {
            ui.painter().rect_filled(rect, 3.0, style.palette.hover);
        }
        let tint = if active {
            style.palette.accent_active
        } else if resp.hovered {
            style.palette.text
        } else {
            style.palette.text_secondary
        };
        let c = rect.center();
        let image_rect = Rect::from_min_max(
            Pos2::new(c.x - 8.0, c.y - 8.0),
            Pos2::new(c.x + 8.0, c.y + 8.0),
        );
        ui.ctx_mut().paint.push(PaintCmd::Image {
            rect: image_rect,
            uv_min: Pos2::new(0.0, 0.0),
            uv_max: Pos2::new(1.0, 1.0),
            tint,
            texture: tex,
        });
        resp
    } else {
        Button::new(fallback)
            .exact_size(Vec2::new(22.0, 20.0))
            .show(ui)
    };
    if resp.hovered {
        ui.tooltip_for(resp.rect, tooltip);
    }
    resp.clicked
}

fn toolbar_vsep(ui: &mut Ui) {
    let stroke = ui.style().palette.stroke;
    let r = ui.allocate(Vec2::new(1.0, 20.0));
    let x = r.center().x;
    ui.painter().line_segment(
        Pos2::new(x, r.min.y + 2.0),
        Pos2::new(x, r.max.y - 2.0),
        1.0,
        stroke,
    );
}

fn render_toolbar(
    ui: &mut Ui,
    panel: &mut AssetBrowserPanel,
    icons: &HashMap<String, TextureId>,
    asset_count: usize,
) {
    let row = ui.available();
    let row_right = row.max.x - 8.0;
    let row_center_y = row.min.y + 11.0;

    ui.horizontal(|ui| {
        // Reference toolbar item_spacing.x is 8.0 (crusty default gap is tighter).
        ui.set_spacing(8.0);
        ui.add_space(6.0);
        if icon_toggle(
            ui,
            icons,
            "folder-browser",
            "F",
            panel.show_folders,
            "Toggle folders",
        ) {
            panel.show_folders = !panel.show_folders;
        }
        toolbar_vsep(ui);

        // Search: painted magnifier + text field + clear.
        let dim = ui.style().palette.text_secondary;
        let glass = ui.allocate(Vec2::new(16.0, 18.0));
        let c = Pos2::new(glass.center().x - 2.0, glass.center().y - 2.0);
        ui.painter().circle_stroke(c, 4.0, 1.5, dim);
        ui.painter().line_segment(
            Pos2::new(c.x + 3.0, c.y + 3.0),
            Pos2::new(c.x + 6.0, c.y + 6.0),
            1.5,
            dim,
        );
        let field_bg = ui.style().palette.input;
        let out = TextEdit::new(&mut panel.search_text)
            .width(150.0)
            .height(20.0)
            .fill(field_bg)
            .hint("Search...")
            .show_full(ui);
        if out.cancelled {
            panel.search_text.clear();
        }
        if !panel.search_text.is_empty() {
            ui.add_space(2.0);
            let resp = Button::new("x").exact_size(Vec2::new(20.0, 20.0)).show(ui);
            if resp.hovered {
                ui.tooltip_for(resp.rect, "Clear search");
            }
            if resp.clicked {
                panel.search_text.clear();
            }
        }
        toolbar_vsep(ui);

        let selected_text = match &panel.type_filter {
            Some(t) => t.display_name(),
            None => "All Types",
        };
        ComboBox::new("ab_type_filter")
            .selected_text(selected_text)
            .width(110.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut panel.type_filter, None, "All Types");
                ui.separator();
                for asset_type in AssetType::all() {
                    ui.selectable_value(
                        &mut panel.type_filter,
                        Some(*asset_type),
                        asset_type.display_name(),
                    );
                }
            });
        toolbar_vsep(ui);

        if icon_toggle(
            ui,
            icons,
            "grid-view",
            "G",
            panel.view_mode == ViewMode::Grid,
            "Grid view",
        ) {
            panel.view_mode = ViewMode::Grid;
        }
        if icon_toggle(
            ui,
            icons,
            "list-view",
            "L",
            panel.view_mode == ViewMode::List,
            "List view",
        ) {
            panel.view_mode = ViewMode::List;
        }

        if panel.view_mode == ViewMode::Grid {
            toolbar_vsep(ui);
            ui.label("Size:");
            Slider::new(&mut panel.grid_item_size, 48.0..=192.0)
                .width(100.0)
                .show(ui);
        }
    });

    // Right-aligned cluster: asset count + rescan.
    let refresh_rect =
        Rect::from_center_size(Pos2::new(row_right - 10.0, row_center_y), Vec2::splat(20.0));
    let resp = ui.interact(Id::new("ab_refresh"), refresh_rect);
    let color = if resp.hovered {
        ui.style().palette.text
    } else {
        ui.style().palette.text_secondary
    };
    let c = refresh_rect.center();
    ui.painter().circle_stroke(c, 6.0, 1.5, color);
    ui.painter().convex_polygon_filled(
        vec![
            Pos2::new(c.x + 3.0, c.y - 9.5),
            Pos2::new(c.x + 9.0, c.y - 6.0),
            Pos2::new(c.x + 3.0, c.y - 2.5),
        ],
        color,
    );
    if resp.hovered {
        ui.tooltip_for(refresh_rect, "Rescan assets");
    }
    if resp.clicked {
        panel.request_rescan();
    }

    let dim = ui.style().palette.text_secondary;
    let small = ui.style().fonts.small;
    let mono = crusty_gui::text::FontFamily::Mono;
    let count_text = format!("{asset_count} assets");
    let sz = ui
        .painter()
        .measure_text_family(&count_text, small, None, mono);
    ui.painter().text_family(
        Pos2::new(refresh_rect.min.x - 8.0 - sz.x, row_center_y - sz.y * 0.5),
        &count_text,
        small,
        dim,
        None,
        mono,
    );
}

// ─── breadcrumb ─────────────────────────────────────────────────────────

fn crumb(ui: &mut Ui, id: Id, text: &str, selected: bool) -> bool {
    let style = ui.style();
    let body = style.fonts.body;
    let sz = ui.painter().measure_text(text, body, None);
    let rect = ui.allocate(Vec2::new(sz.x + 12.0, 20.0));
    let resp = ui.interact(id, rect);
    if selected {
        ui.painter()
            .rect_filled(rect, 3.0, style.palette.accent_soft);
    } else if resp.hovered {
        ui.painter().rect_filled(rect, 3.0, style.palette.hover);
    }
    let color = if selected {
        style.palette.accent_text
    } else {
        style.palette.text_secondary
    };
    ui.painter().text(
        Pos2::new(rect.min.x + 6.0, rect.center().y - sz.y * 0.5),
        text,
        body,
        color,
        None,
    );
    resp.clicked
}

fn render_breadcrumb(ui: &mut Ui, panel: &mut AssetBrowserPanel) {
    let current = panel.current_folder.clone();
    let mut clicked_path: Option<PathBuf> = None;

    ui.horizontal(|ui| {
        ui.add_space(6.0);
        let root_selected = current.as_os_str().is_empty();
        if crumb(ui, Id::new("ab_crumb_root"), "Content", root_selected) {
            clicked_path = Some(PathBuf::new());
        }

        let mut accumulated = PathBuf::new();
        for (i, component) in current.components().enumerate() {
            let dim = ui.style().palette.text_secondary;
            let small = ui.style().fonts.small;
            let slash = ui.allocate(Vec2::new(8.0, 20.0));
            let ssz = ui.painter().measure_text("/", small, None);
            ui.painter().text(
                Pos2::new(
                    slash.center().x - ssz.x * 0.5,
                    slash.center().y - ssz.y * 0.5,
                ),
                "/",
                small,
                dim,
                None,
            );
            accumulated.push(component.as_os_str());
            let name = component.as_os_str().to_string_lossy().to_string();
            let is_last = accumulated == current;
            if crumb(ui, Id::new("ab_crumb").with(i), &name, is_last) && !is_last {
                clicked_path = Some(accumulated.clone());
            }
        }
    });

    if let Some(path) = clicked_path {
        panel.current_folder = path.clone();
        panel.events.push(AssetBrowserEvent::FolderChanged { path });
    }
}

// ─── inline rename editor ───────────────────────────────────────────────

enum RenameResult {
    Pending,
    Commit(String),
    Cancel,
}

fn rename_edit(ui: &mut Ui, rect: Rect, current_name: &str, key: u64, edit_id: Id) -> RenameResult {
    let fid = Id::new("ab_rename_focus");
    let bid = Id::new("ab_rename_buf");
    let prev = ui.ctx().memory.data_get::<RenameFocusKey>(fid);
    let request = prev.map(|k| k.0) != Some(key);
    ui.ctx_mut().memory.data_insert(fid, RenameFocusKey(key));

    let mut buf = if request {
        current_name.to_string()
    } else {
        ui.ctx()
            .memory
            .data_get::<RenameBuffer>(bid)
            .map(|b| b.0.clone())
            .unwrap_or_else(|| current_name.to_string())
    };

    let (out, _) = ui.run_at(
        rect,
        Direction::TopDown,
        edit_id,
        UiOptions {
            padding: Vec2::ZERO,
            spacing: 0.0,
        },
        |ui| {
            TextEdit::new(&mut buf)
                .width(rect.width())
                .height(rect.height())
                .request_focus(request)
                .show_full(ui)
        },
    );
    let result = if out.submitted {
        RenameResult::Commit(buf.clone())
    } else if out.cancelled {
        RenameResult::Cancel
    } else if !out.focused && !request {
        RenameResult::Commit(buf.clone())
    } else {
        RenameResult::Pending
    };
    ui.ctx_mut().memory.data_insert(bid, RenameBuffer(buf));
    result
}

fn commit_asset_rename(panel: &mut AssetBrowserPanel, id: AssetId, new_name: String) {
    let new_name = new_name.trim().to_string();
    if let Some(meta) = panel.registry.get(id) {
        let old_name = meta.display_name.clone();
        if !new_name.is_empty() && new_name != old_name {
            panel.events.push(AssetBrowserEvent::AssetRenamed {
                id,
                old_name,
                new_name,
            });
        }
    }
    panel.renaming = None;
}

fn commit_folder_rename(panel: &mut AssetBrowserPanel, old_path: PathBuf, new_name: String) {
    let new_name = new_name.trim().to_string();
    let current = old_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if !new_name.is_empty() && new_name != current {
        let new_path = match old_path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.join(&new_name),
            _ => PathBuf::from(&new_name),
        };
        panel
            .events
            .push(AssetBrowserEvent::FolderRenamed { old_path, new_path });
    }
    panel.renaming = None;
}

// ─── folder tree pane ───────────────────────────────────────────────────

fn render_folder_pane(
    ui: &mut Ui,
    panel: &mut AssetBrowserPanel,
    icons: &HashMap<String, TextureId>,
) {
    ui.add_space(8.0);
    let top = ui.cursor().y;
    let text_color = ui.style().palette.text_secondary;
    let left = ui.available().min.x;
    ui.painter().text_family(
        Pos2::new(left + 8.0, top + 4.0),
        "FOLDERS",
        10.0,
        text_color,
        None,
        crusty_gui::text::FontFamily::Mono,
    );
    ui.add_space(24.0);

    let tree = panel.registry.get_folder_tree();
    let avail_h = ui.available_size().y;
    ScrollArea::new(avail_h)
        .auto_shrink(false)
        .inset(0.0)
        .spacing(0.0)
        .show(ui, |ui| {
            render_folder_node(ui, panel, icons, &tree, 0);
        });
}

const FOLDER_ROW_H: f32 = 22.0;
const FOLDER_INDENT: f32 = 16.0;

fn render_folder_node(
    ui: &mut Ui,
    panel: &mut AssetBrowserPanel,
    icons: &HashMap<String, TextureId>,
    node: &FolderNode,
    depth: usize,
) {
    let is_root = node.path.as_os_str().is_empty();
    let has_children = !node.children.is_empty();
    let is_expanded = panel.folder_expanded.contains(&node.path);
    let is_selected = panel.current_folder == node.path;
    let is_renaming = matches!(
        &panel.renaming,
        Some(RenameTarget::Folder { path, .. }) if *path == node.path
    );

    let width = ui.available().width();
    let row_rect = ui.allocate(Vec2::new(width, FOLDER_ROW_H));
    let clip = ui.clip_rect();
    let visible = row_rect.max.y >= clip.min.y && row_rect.min.y <= clip.max.y;

    if visible {
        let indent = depth as f32 * FOLDER_INDENT;
        let left = row_rect.min.x + 4.0;
        let center_y = row_rect.center().y;
        let row_id = Id::new("ab_folder_row").with(hash_key(&node.path));

        // Interaction rect excludes the chevron column, like the historical tree.
        let body_rect = Rect::from_min_max(
            Pos2::new(left + indent + FOLDER_INDENT, row_rect.min.y),
            row_rect.max,
        );
        let resp = ui.interact(row_id, body_rect);

        // Highlights hug the row content (chevron → label end) like the
        // historical tree, not the full pane width.
        let body_font = ui.style().fonts.body;
        let name_w = ui.painter().measure_text(&node.name, body_font, None).x;
        let hl_rect = Rect::from_min_max(
            Pos2::new(left + indent, row_rect.min.y),
            Pos2::new(
                (left + indent + FOLDER_INDENT + 2.0 + 16.0 + 6.0 + name_w + 6.0)
                    .min(row_rect.max.x),
                row_rect.max.y,
            ),
        );

        let dragging_self = ui
            .ctx()
            .dnd
            .peek::<DragFolder>()
            .is_some_and(|d| d.path == node.path);

        // Drop-target highlight (assets always, folders when valid).
        let folder_drop_valid = ui
            .ctx()
            .dnd
            .peek::<DragFolder>()
            .is_some_and(|d| d.path != node.path && !node.path.starts_with(&d.path));
        let asset_hover = ui.dnd_hovering::<DragAsset>(row_rect);
        let folder_hover = folder_drop_valid && ui.dnd_hovering::<DragFolder>(row_rect);
        let style = ui.style();
        if asset_hover || folder_hover {
            // Drop-into: 1px accent border, never repaint the row fill.
            ui.painter().rect_stroke(
                hl_rect,
                2.0,
                style.metrics.border,
                style.palette.accent_active,
            );
        } else if is_selected {
            ui.painter()
                .rect_filled(hl_rect, 2.0, style.palette.selection_fill);
        } else if resp.hovered && !is_renaming {
            ui.painter().rect_filled(hl_rect, 2.0, style.palette.hover);
        }
        if dragging_self {
            ui.painter()
                .rect_filled(hl_rect, 2.0, style.palette.window.with_alpha(0.55));
        }

        // Chevron.
        if has_children {
            let ch_rect = Rect::from_min_size(
                Pos2::new(left + indent, center_y - FOLDER_INDENT * 0.5),
                Vec2::splat(FOLDER_INDENT),
            );
            let ch_resp = ui.interact(row_id.with("chevron"), ch_rect);
            let c = ch_rect.center();
            let s = 3.5;
            let color = if ch_resp.hovered {
                style.palette.text
            } else {
                style.palette.text_secondary
            };
            let (a, m, b) = if is_expanded {
                (
                    Pos2::new(c.x - s, c.y - s * 0.45),
                    Pos2::new(c.x, c.y + s * 0.55),
                    Pos2::new(c.x + s, c.y - s * 0.45),
                )
            } else {
                (
                    Pos2::new(c.x - s * 0.45, c.y - s),
                    Pos2::new(c.x + s * 0.55, c.y),
                    Pos2::new(c.x - s * 0.45, c.y + s),
                )
            };
            ui.painter().line_segment(a, m, 1.5, color);
            ui.painter().line_segment(m, b, 1.5, color);
            if ch_resp.clicked {
                if is_expanded {
                    panel.folder_expanded.remove(&node.path);
                } else {
                    panel.folder_expanded.insert(node.path.clone());
                }
            }
        }

        // Folder icon.
        let icon_stem = if is_expanded && has_children {
            "opened-folder"
        } else {
            "folder"
        };
        let icon_x = left + indent + FOLDER_INDENT + 2.0;
        let icon_rect = Rect::from_min_size(Pos2::new(icon_x, center_y - 8.0), Vec2::splat(16.0));
        // Open folder keeps its gold even when selected (design system).
        let tint = if is_expanded && has_children {
            super::theme::asset_color("lights")
        } else if is_selected {
            style.palette.selection_text
        } else {
            style.palette.text_secondary
        };
        if let Some(&tex) = icons.get(icon_stem) {
            ui.ctx_mut().paint.push(PaintCmd::Image {
                rect: icon_rect,
                uv_min: Pos2::new(0.0, 0.0),
                uv_max: Pos2::new(1.0, 1.0),
                tint,
                texture: tex,
            });
        }

        // Name / rename editor.
        let label_x = icon_rect.max.x + 6.0;
        if is_renaming {
            let edit_rect = Rect::from_min_max(
                Pos2::new(label_x, row_rect.min.y + 1.0),
                Pos2::new(
                    (label_x + 100.0).min(row_rect.max.x - 2.0),
                    row_rect.max.y - 1.0,
                ),
            );
            let current_name = node.name.clone();
            match rename_edit(
                ui,
                edit_rect,
                &current_name,
                hash_key(&("folder", &node.path)),
                row_id.with("rename"),
            ) {
                RenameResult::Commit(new_name) => {
                    commit_folder_rename(panel, node.path.clone(), new_name);
                }
                RenameResult::Cancel => panel.renaming = None,
                RenameResult::Pending => {}
            }
        } else {
            let body = style.fonts.body;
            let color = if is_selected {
                style.palette.selection_text
            } else {
                style.palette.text_secondary
            };
            let sz = ui.painter().measure_text(&node.name, body, None);
            ui.painter().text(
                Pos2::new(label_x, center_y - sz.y * 0.5),
                &node.name,
                body,
                color,
                None,
            );
        }

        // Drag source (non-root only).
        if !is_root && !is_renaming {
            let path = node.path.clone();
            let name = node.name.clone();
            ui.dnd_drag_source(row_id, body_rect, node.name.clone(), move || DragFolder {
                path,
                name,
            });
        }

        // Drops.
        if let Some(drag) = ui.dnd_drop_target::<DragAsset>(row_rect) {
            panel.events.push(AssetBrowserEvent::AssetMoved {
                id: drag.id,
                old_path: drag.path.clone(),
                new_path: node.path.join(drag.path.file_name().unwrap_or_default()),
            });
            panel.drag_payload = None;
        }
        if folder_drop_valid {
            if let Some(drag) = ui.dnd_drop_target::<DragFolder>(row_rect) {
                panel.events.push(AssetBrowserEvent::FolderMoved {
                    old_path: drag.path.clone(),
                    new_path: node.path.join(&drag.name),
                });
                panel.drag_payload = None;
            }
        }

        // Click navigation / double-click expand toggle.
        if !is_renaming && resp.clicked {
            panel.current_folder = node.path.clone();
            panel.events.push(AssetBrowserEvent::FolderChanged {
                path: node.path.clone(),
            });
        }
        if resp.double_clicked(ui) && has_children {
            if is_expanded {
                panel.folder_expanded.remove(&node.path);
            } else {
                panel.folder_expanded.insert(node.path.clone());
            }
        }

        if resp.hovered && !ui.ctx().dnd.is_dragging() {
            ui.tooltip_for(
                body_rect,
                &format!(
                    "{} assets ({} total)",
                    node.asset_count, node.total_asset_count
                ),
            );
        }

        // Context menu.
        let ctx_path = node.path.clone();
        let ctx_name = node.name.clone();
        let mut create = None;
        let mut rename = false;
        let mut reveal = false;
        let mut delete = false;
        ui.context_menu_for(("ab_folder_ctx", hash_key(&node.path)), body_rect, |ui| {
            create = create_submenu(ui);
            if !is_root {
                if ui.menu_item("Rename") {
                    rename = true;
                }
                ui.separator();
            } else {
                ui.separator();
            }
            if ui.menu_item("Reveal in Explorer") {
                reveal = true;
            }
            if !is_root {
                ui.separator();
                if ui.menu_item("Delete") {
                    delete = true;
                }
            }
        });
        if let Some(choice) = create {
            push_create_event(panel, choice, ctx_path.clone());
        }
        if rename {
            panel.renaming = Some(RenameTarget::Folder {
                path: ctx_path.clone(),
                current_name: ctx_name,
            });
        }
        if reveal {
            panel
                .events
                .push(AssetBrowserEvent::RevealFolderInExplorer {
                    path: ctx_path.clone(),
                });
        }
        if delete {
            let full_path = panel.registry.root_path().join(&ctx_path);
            let file_count = std::fs::read_dir(&full_path)
                .map(|entries| entries.count())
                .unwrap_or(0);
            panel.delete_confirmation = Some(DeleteConfirmation {
                target: DeleteTarget::Folder {
                    path: ctx_path,
                    is_empty: file_count == 0,
                },
                file_count,
            });
        }
    }

    if is_expanded {
        for child in &node.children {
            render_folder_node(ui, panel, icons, child, depth + 1);
        }
    }
}

// ─── content: shared click handling ─────────────────────────────────────

fn handle_asset_click(ui: &Ui, panel: &mut AssetBrowserPanel, id: AssetId, visible: &[AssetId]) {
    let mods = ui.ctx().input.modifiers;
    if mods.contains(Modifiers::CTRL) {
        panel.selection.toggle(id);
    } else if mods.contains(Modifiers::SHIFT) {
        panel.selection.select_range(visible, id);
    } else {
        panel.selection.select(id);
    }
    panel.events.push(AssetBrowserEvent::AssetSelected { id });
}

/// The `Create ▸` entries: label and the asset type (`None` is a folder).
const CREATE_ENTRIES: [(&str, Option<AssetType>); 7] = [
    ("Folder", None),
    ("Scene", Some(AssetType::Scene)),
    ("Material", Some(AssetType::Material)),
    ("Script Graph", Some(AssetType::Graph)),
    ("Animation Graph", Some(AssetType::AnimGraph)),
    ("Blend Space", Some(AssetType::BlendSpace)),
    ("Curve", Some(AssetType::Curve)),
];

/// `Create ▸` submenu shared by the folder-tree and grid-background menus.
/// Returns the picked entry (`Some(None)` = folder) on the click frame.
fn create_submenu(ui: &mut Ui) -> Option<Option<AssetType>> {
    let mut picked = None;
    ui.submenu("Create", |ui| {
        for (label, ty) in CREATE_ENTRIES {
            if ui.menu_item(label) {
                picked = Some(ty);
            }
        }
    });
    picked
}

fn push_create_event(panel: &mut AssetBrowserPanel, choice: Option<AssetType>, parent_path: PathBuf) {
    panel.events.push(match choice {
        None => AssetBrowserEvent::CreateFolder { parent_path },
        Some(asset_type) => AssetBrowserEvent::CreateAsset { asset_type, parent_path },
    });
}

/// Right-click on empty content space: create into the current folder or
/// reveal it. `rect` is empty while a row is under the pointer so the row's
/// own menu wins; an already-open menu keeps drawing regardless.
fn background_context_menu(ui: &mut Ui, panel: &mut AssetBrowserPanel, rect: Rect) {
    let mut create = None;
    let mut reveal = false;
    ui.context_menu_for("ab_bg_ctx", rect, |ui| {
        create = create_submenu(ui);
        ui.separator();
        if ui.menu_item("Reveal in Explorer") {
            reveal = true;
        }
    });
    let folder = panel.current_folder.clone();
    if let Some(choice) = create {
        push_create_event(panel, choice, folder.clone());
    }
    if reveal {
        panel
            .events
            .push(AssetBrowserEvent::RevealFolderInExplorer { path: folder });
    }
}

/// Context menu shared by grid cards and list rows. "Copy Path" is omitted —
/// crusty has no clipboard API yet.
fn asset_context_menu(ui: &mut Ui, panel: &mut AssetBrowserPanel, row: &AssetRow, rect: Rect) {
    let mut open = false;
    let mut rename = false;
    let mut reveal = false;
    let mut delete = false;
    ui.context_menu_for(("ab_asset_ctx", hash_key(&row.id)), rect, |ui| {
        if ui.menu_item("Open") {
            open = true;
        }
        if ui.menu_item("Rename") {
            rename = true;
        }
        ui.separator();
        if ui.menu_item("Reveal in Explorer") {
            reveal = true;
        }
        ui.separator();
        if ui.menu_item("Delete") {
            delete = true;
        }
    });
    if open {
        panel
            .events
            .push(AssetBrowserEvent::AssetOpened { id: row.id });
    }
    if rename {
        panel.renaming = Some(RenameTarget::Asset {
            id: row.id,
            current_name: row.name.clone(),
        });
    }
    if reveal {
        panel.events.push(AssetBrowserEvent::RevealInExplorer {
            path: row.path.clone(),
        });
    }
    if delete {
        panel.delete_confirmation = Some(DeleteConfirmation {
            target: DeleteTarget::Asset {
                id: row.id,
                path: row.path.clone(),
            },
            file_count: 1,
        });
    }
}

fn asset_tooltip(ui: &mut Ui, row: &AssetRow, rect: Rect) {
    ui.tooltip_for(
        rect,
        &format!(
            "{}\nType: {}\nSize: {}\nPath: {}",
            row.name,
            row.asset_type.display_name(),
            row.size_str,
            row.path.display()
        ),
    );
}

fn render_content(
    ui: &mut Ui,
    panel: &mut AssetBrowserPanel,
    icons: &HashMap<String, TextureId>,
    rows: &[AssetRow],
    visible_ids: &[AssetId],
) {
    ui.add_space(8.0);
    let content_rect = ui.available();
    if rows.is_empty() {
        let avail = content_rect;
        let dim = ui.style().palette.text_secondary;
        let msg = "No assets found";
        let sz = ui.painter().measure_text(msg, 16.0, None);
        let cx = avail.center().x;
        let y = avail.min.y + 50.0;
        ui.painter()
            .text(Pos2::new(cx - sz.x * 0.5, y), msg, 16.0, dim, None);
        let hint = if !panel.search_text.is_empty() || panel.type_filter.is_some() {
            Some("Try adjusting your filters")
        } else if !panel.current_folder.as_os_str().is_empty() {
            Some("This folder is empty")
        } else {
            None
        };
        if let Some(hint) = hint {
            let body = ui.style().fonts.body;
            let hsz = ui.painter().measure_text(hint, body, None);
            ui.painter().text(
                Pos2::new(cx - hsz.x * 0.5, y + sz.y + 6.0),
                hint,
                body,
                dim,
                None,
            );
        }
        background_context_menu(ui, panel, content_rect);
        return;
    }

    let over_row = match panel.view_mode {
        ViewMode::Grid => render_grid(ui, panel, icons, rows, visible_ids),
        ViewMode::List => render_list(ui, panel, icons, rows, visible_ids),
    };
    let bg = if over_row { Rect::ZERO } else { content_rect };
    background_context_menu(ui, panel, bg);
}

// ─── grid view ──────────────────────────────────────────────────────────

fn render_grid(
    ui: &mut Ui,
    panel: &mut AssetBrowserPanel,
    icons: &HashMap<String, TextureId>,
    rows: &[AssetRow],
    visible_ids: &[AssetId],
) -> bool {
    // Improved tile: thumbnail well + 2px type edge + framed two-line label.
    // 96px wide at the default slider value, scaling with it.
    let item = panel.grid_item_size;
    let card_w = item;
    // Square thumbnail well (mockup aspect-ratio:1) — square thumbnails
    // render undistorted.
    let thumb_h = card_w - 2.0;
    let card_h = thumb_h + 2.0 + 32.0;
    // Mockup grid rhythm: 14px outer padding, 10px tile gaps.
    let gap = 10.0;
    let inset = 14.0;

    let avail_h = ui.available_size().y;
    let mut over_card = false;
    ScrollArea::new(avail_h)
        .auto_shrink(false)
        .inset(0.0)
        .spacing(0.0)
        .show(ui, |ui| {
            let width = ui.available().width();
            let cols = (((width - inset * 2.0 + gap) / (card_w + gap)).floor() as usize).max(1);
            let grid_rows = rows.len().div_ceil(cols);
            let total_h = grid_rows as f32 * (card_h + gap) - gap + inset * 2.0;
            let origin = ui.cursor();
            ui.allocate(Vec2::new(width, total_h));
            let clip = ui.clip_rect();

            for (i, row) in rows.iter().enumerate() {
                let r = i / cols;
                let c = i % cols;
                let min = Pos2::new(
                    origin.x + inset + c as f32 * (card_w + gap),
                    origin.y + inset + r as f32 * (card_h + gap),
                );
                let card = Rect::from_min_size(min, Vec2::new(card_w, card_h));
                if card.max.y < clip.min.y || card.min.y > clip.max.y {
                    continue;
                }
                over_card |= ui.contains_pointer(card);
                render_grid_card(ui, panel, icons, row, card, thumb_h, visible_ids);
            }
        });
    over_card
}

#[allow(clippy::too_many_arguments)]
fn render_grid_card(
    ui: &mut Ui,
    panel: &mut AssetBrowserPanel,
    icons: &HashMap<String, TextureId>,
    row: &AssetRow,
    card: Rect,
    thumb_h: f32,
    visible_ids: &[AssetId],
) {
    let is_selected = panel.selection.is_selected(row.id);
    let is_renaming = matches!(
        &panel.renaming,
        Some(RenameTarget::Asset { id, .. }) if *id == row.id
    );
    let card_id = Id::new("ab_card").with(hash_key(&row.id));
    let resp = ui.interact(card_id, card);

    // Selection fills the card body + focus-ring border; the thumbnail well
    // keeps its own bg so content never tints.
    let style = ui.style();
    let (fill, border) = if is_selected {
        (style.palette.selection_fill, style.palette.focus_ring)
    } else if resp.hovered {
        (style.palette.elevated, style.palette.stroke_strong)
    } else {
        (style.palette.header, style.palette.stroke)
    };
    ui.painter().rect_filled(card, 4.0, fill);
    ui.painter()
        .rect_stroke(card, 4.0, style.metrics.border, border);

    // Thumbnail well.
    let thumb = Rect::from_min_max(
        Pos2::new(card.min.x + 1.0, card.min.y + 1.0),
        Pos2::new(card.max.x - 1.0, card.min.y + 1.0 + thumb_h),
    );
    ui.painter().rect_filled(thumb, 3.0, style.palette.window);
    if let Some(tex) = row.thumb {
        ui.ctx_mut().paint.push(PaintCmd::Image {
            rect: thumb,
            uv_min: Pos2::new(0.0, 0.0),
            uv_max: Pos2::new(1.0, 1.0),
            tint: Color::WHITE,
            texture: tex,
        });
    } else {
        let icon_rect = Rect::from_center_size(thumb.center(), Vec2::splat(30.0));
        if let Some(&tex) = icons.get(type_icon_stem(row.asset_type)) {
            ui.ctx_mut().paint.push(PaintCmd::Image {
                rect: icon_rect,
                uv_min: Pos2::new(0.0, 0.0),
                uv_max: Pos2::new(1.0, 1.0),
                tint: type_color(row.asset_type),
                texture: tex,
            });
        } else {
            let letter = row
                .name
                .chars()
                .next()
                .map(|c| c.to_ascii_uppercase().to_string())
                .unwrap_or_default();
            let sz = ui.painter().measure_text(&letter, 30.0, None);
            let c = thumb.center();
            ui.painter().text(
                Pos2::new(c.x - sz.x * 0.5, c.y - sz.y * 0.5),
                &letter,
                30.0,
                style.palette.text_disabled,
                None,
            );
        }
    }

    // 2px type edge under the thumbnail.
    let edge = Rect::from_min_size(
        Pos2::new(thumb.min.x, thumb.max.y),
        Vec2::new(thumb.width(), style.metrics.edge_accent),
    );
    ui.painter()
        .rect_filled(edge, 0.0, type_color(row.asset_type));

    // Label frame: name + mono type line (or the rename editor).
    let label_rect = Rect::from_min_max(
        Pos2::new(card.min.x + 6.0, edge.max.y + 4.0),
        Pos2::new(card.max.x - 6.0, card.max.y - 4.0),
    );
    if is_renaming {
        match rename_edit(
            ui,
            label_rect,
            &row.name,
            hash_key(&("asset", row.id)),
            card_id.with("rename"),
        ) {
            RenameResult::Commit(new_name) => commit_asset_rename(panel, row.id, new_name),
            RenameResult::Cancel => panel.renaming = None,
            RenameResult::Pending => {}
        }
    } else {
        let name_color = if is_selected {
            style.palette.selection_text
        } else {
            style.palette.text
        };
        let name_size = 10.5;
        // Extension dropped on a *tile* - the type row directly below states
        // it, and repeating it costs the characters that decide whether a
        // generated name still reads (DESIGN-panels, Asset tiles). The list
        // view keeps the full filename: a list of files is a list of files.
        let text = truncate_to_width(ui, &row.name, name_size, label_rect.width());
        ui.painter().text(
            Pos2::new(label_rect.min.x, label_rect.min.y),
            &text,
            name_size,
            name_color,
            None,
        );
        let type_text = row.asset_type.display_name().to_uppercase();
        let type_col = if is_selected || resp.hovered {
            type_color(row.asset_type)
        } else {
            style.palette.text_secondary
        };
        let type_text = truncate_to_width_family(
            ui,
            &type_text,
            9.0,
            label_rect.width(),
            crusty_gui::text::FontFamily::Mono,
        );
        ui.painter().text_family(
            Pos2::new(label_rect.min.x, label_rect.min.y + 14.0),
            &type_text,
            9.0,
            type_col,
            None,
            crusty_gui::text::FontFamily::Mono,
        );
    }

    // Interactions.
    if !is_renaming {
        if resp.clicked {
            handle_asset_click(ui, panel, row.id, visible_ids);
        }
        if resp.double_clicked(ui) {
            panel
                .events
                .push(AssetBrowserEvent::AssetOpened { id: row.id });
        }
        ui.dnd_drag_source(card_id, card, row.label.clone(), || DragAsset {
            id: row.id,
            asset_type: row.asset_type,
            path: row.path.clone(),
        });
        if resp.hovered && !ui.ctx().dnd.is_dragging() {
            asset_tooltip(ui, row, card);
        }
        asset_context_menu(ui, panel, row, card);
    }
}

// ─── list view ──────────────────────────────────────────────────────────

const LIST_ROW_H: f32 = 24.0;
const LIST_COLS: [(&str, f32, u8); 4] = [
    ("Name", 200.0, 0),
    ("Type", 80.0, 1),
    ("Size", 80.0, 2),
    ("Modified", 120.0, 3),
];

fn render_list(
    ui: &mut Ui,
    panel: &mut AssetBrowserPanel,
    icons: &HashMap<String, TextureId>,
    rows: &[AssetRow],
    visible_ids: &[AssetId],
) -> bool {
    let sort_id = Id::new("ab_list_sort");
    let mut sort = ui
        .ctx()
        .memory
        .data_get::<ListSortState>(sort_id)
        .copied()
        .unwrap_or(ListSortState { col: 0, asc: true });

    // Header row.
    let header = ui.allocate(Vec2::new(ui.available().width(), 22.0));
    let body = ui.style().fonts.body;
    let text_color = ui.style().palette.text;
    let dim = ui.style().palette.text_secondary;
    let mut x = header.min.x + 8.0;
    for (label, w, col) in LIST_COLS {
        let cell = Rect::from_min_max(Pos2::new(x, header.min.y), Pos2::new(x + w, header.max.y));
        let resp = ui.interact(Id::new("ab_list_header").with(col), cell);
        if resp.hovered {
            let hover = ui.style().palette.hover;
            ui.painter().rect_filled(cell, 2.0, hover);
        }
        let is_sorted = sort.col == col;
        let color = if is_sorted { text_color } else { dim };
        let sz = ui.painter().measure_text(label, body, None);
        ui.painter().text(
            Pos2::new(x + 2.0, cell.center().y - sz.y * 0.5),
            label,
            body,
            color,
            None,
        );
        if is_sorted {
            // Sort arrow (painted, not a glyph).
            let ax = x + 2.0 + sz.x + 8.0;
            let ay = cell.center().y;
            let s = 3.5;
            let pts = if sort.asc {
                vec![
                    Pos2::new(ax - s, ay + s * 0.6),
                    Pos2::new(ax + s, ay + s * 0.6),
                    Pos2::new(ax, ay - s * 0.8),
                ]
            } else {
                vec![
                    Pos2::new(ax - s, ay - s * 0.6),
                    Pos2::new(ax, ay + s * 0.8),
                    Pos2::new(ax + s, ay - s * 0.6),
                ]
            };
            ui.painter().convex_polygon_filled(pts, color);
        }
        if resp.clicked {
            if sort.col == col {
                sort.asc = !sort.asc;
            } else {
                sort = ListSortState { col, asc: true };
            }
            ui.ctx_mut().memory.data_insert(sort_id, sort);
        }
        x += w;
    }
    ui.separator();

    let avail_h = ui.available_size().y;
    let mut over_row = false;
    ScrollArea::new(avail_h)
        .auto_shrink(false)
        .inset(0.0)
        .spacing(0.0)
        .show(ui, |ui| {
            for row in rows {
                over_row |= render_list_row(ui, panel, icons, row, visible_ids);
            }
        });
    over_row
}

fn render_list_row(
    ui: &mut Ui,
    panel: &mut AssetBrowserPanel,
    icons: &HashMap<String, TextureId>,
    row: &AssetRow,
    visible_ids: &[AssetId],
) -> bool {
    let width = ui.available().width();
    let row_rect = ui.allocate(Vec2::new(width, LIST_ROW_H));
    let clip = ui.clip_rect();
    if row_rect.max.y < clip.min.y || row_rect.min.y > clip.max.y {
        return false;
    }
    let over_row = ui.contains_pointer(row_rect);

    let is_selected = panel.selection.is_selected(row.id);
    let is_renaming = matches!(
        &panel.renaming,
        Some(RenameTarget::Asset { id, .. }) if *id == row.id
    );
    let row_id = Id::new("ab_list_row").with(hash_key(&row.id));
    let resp = ui.interact(row_id, row_rect);

    let style = ui.style();
    if is_selected {
        ui.painter()
            .rect_filled(row_rect, 0.0, style.palette.selection_fill);
    } else if resp.hovered && !is_renaming {
        ui.painter().rect_filled(row_rect, 0.0, style.palette.hover);
    }

    let center_y = row_rect.center().y;
    let left = row_rect.min.x + 8.0;

    // Type icon.
    let icon_rect = Rect::from_min_size(Pos2::new(left, center_y - 8.0), Vec2::splat(16.0));
    if let Some(&tex) = icons.get(type_icon_stem(row.asset_type)) {
        ui.ctx_mut().paint.push(PaintCmd::Image {
            rect: icon_rect,
            uv_min: Pos2::new(0.0, 0.0),
            uv_max: Pos2::new(1.0, 1.0),
            tint: type_color(row.asset_type),
            texture: tex,
        });
    }

    let body = ui.style().fonts.body;
    let small = ui.style().fonts.small;
    let dim = ui.style().palette.text_secondary;

    // Name (or rename editor).
    let name_x = left + 20.0;
    if is_renaming {
        let edit_rect = Rect::from_min_max(
            Pos2::new(name_x, row_rect.min.y + 2.0),
            Pos2::new(left + 196.0, row_rect.max.y - 2.0),
        );
        match rename_edit(
            ui,
            edit_rect,
            &row.name,
            hash_key(&("asset", row.id)),
            row_id.with("rename"),
        ) {
            RenameResult::Commit(new_name) => commit_asset_rename(panel, row.id, new_name),
            RenameResult::Cancel => panel.renaming = None,
            RenameResult::Pending => {}
        }
    } else {
        let color = if is_selected {
            style.palette.selection_text
        } else {
            style.palette.text
        };
        let text = truncate_to_width(ui, &row.label, body, 200.0 - 20.0 - 4.0);
        let sz = ui.painter().measure_text(&text, body, None);
        ui.painter().text(
            Pos2::new(name_x, center_y - sz.y * 0.5),
            &text,
            body,
            color,
            None,
        );
    }

    // Type / Size / Modified columns.
    let mut cx = left + 200.0;
    let type_text = row.asset_type.display_name();
    let tsz = ui.painter().measure_text(type_text, small, None);
    ui.painter().text(
        Pos2::new(cx + 2.0, center_y - tsz.y * 0.5),
        type_text,
        small,
        type_color(row.asset_type),
        None,
    );
    cx += 80.0;
    let ssz = ui.painter().measure_text(&row.size_str, small, None);
    ui.painter().text(
        Pos2::new(cx + 2.0, center_y - ssz.y * 0.5),
        &row.size_str,
        small,
        dim,
        None,
    );
    cx += 80.0;
    let msz = ui.painter().measure_text(&row.modified_str, small, None);
    ui.painter().text(
        Pos2::new(cx + 2.0, center_y - msz.y * 0.5),
        &row.modified_str,
        small,
        dim,
        None,
    );

    // Interactions.
    if !is_renaming {
        if resp.clicked {
            handle_asset_click(ui, panel, row.id, visible_ids);
        }
        if resp.double_clicked(ui) {
            panel
                .events
                .push(AssetBrowserEvent::AssetOpened { id: row.id });
        }
        ui.dnd_drag_source(row_id, row_rect, row.label.clone(), || DragAsset {
            id: row.id,
            asset_type: row.asset_type,
            path: row.path.clone(),
        });
        if resp.hovered && !ui.ctx().dnd.is_dragging() {
            asset_tooltip(ui, row, row_rect);
        }
        asset_context_menu(ui, panel, row, row_rect);
    }
    over_row
}

// ─── delete confirmation ────────────────────────────────────────────────

fn confirm_delete(panel: &mut AssetBrowserPanel) {
    if let Some(confirmation) = panel.delete_confirmation.take() {
        match confirmation.target {
            DeleteTarget::Asset { id, path } => {
                panel
                    .events
                    .push(AssetBrowserEvent::AssetDeleted { id, path });
            }
            DeleteTarget::Folder { path, .. } => {
                panel.events.push(AssetBrowserEvent::FolderDeleted { path });
            }
        }
    }
}

fn render_delete_confirmation(ui: &mut Ui, panel: &mut AssetBrowserPanel) {
    let Some(confirmation) = panel.delete_confirmation.clone() else {
        return;
    };
    let (title, message) = match &confirmation.target {
        DeleteTarget::Asset { path, .. } => (
            "Delete Asset",
            format!(
                "Are you sure you want to delete '{}'?\n\nThis action cannot be undone.",
                path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string())
            ),
        ),
        DeleteTarget::Folder { path, is_empty } => {
            let folder_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string());
            if *is_empty {
                (
                    "Delete Folder",
                    format!("Are you sure you want to delete the folder '{folder_name}'?"),
                )
            } else {
                (
                    "Delete Non-Empty Folder",
                    format!(
                        "The folder '{}' contains {} items.\n\n\
                         Are you sure you want to delete this folder and ALL its contents?\n\n\
                         This action cannot be undone!",
                        folder_name, confirmation.file_count
                    ),
                )
            }
        }
    };

    let screen = ui.ctx().screen_rect();
    let size = Vec2::new(380.0, 150.0);
    let result = Window::new(title)
        .modal(true)
        .resizable(false)
        .collapsible(false)
        .default_pos(Pos2::new(
            screen.center().x - size.x * 0.5,
            screen.center().y - size.y * 0.5,
        ))
        .default_size(size)
        .show(ui, |ui| {
            ui.add_space(4.0);
            for line in message.split('\n') {
                if line.is_empty() {
                    ui.add_space(6.0);
                } else {
                    ui.label(line);
                }
            }
            ui.add_space(12.0);

            let mut cancel = false;
            let mut delete = false;
            let button_w = 80.0;
            let pad = ((ui.available_size().x - (button_w * 2.0 + 16.0)) * 0.5).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                if Button::new("Cancel")
                    .ghost()
                    .min_size(Vec2::new(button_w, 28.0))
                    .show(ui)
                    .clicked
                {
                    cancel = true;
                }
                ui.add_space(16.0);
                if Button::new("Delete")
                    .danger()
                    .min_size(Vec2::new(button_w, 28.0))
                    .show(ui)
                    .clicked
                {
                    delete = true;
                }
            });
            (cancel, delete)
        });

    if let Some(resp) = result {
        if let Some((cancel, delete)) = resp.inner {
            if delete {
                confirm_delete(panel);
            } else if cancel {
                panel.delete_confirmation = None;
            }
        }
    }
}

// ─── drag payload sync ──────────────────────────────────────────────────

/// Mirror the crusty dnd state into `panel.drag_payload` so external
/// consumers (viewport drop) see the same state.
fn sync_drag_payload(ui: &Ui, panel: &mut AssetBrowserPanel) {
    if let Some(d) = ui.ctx().dnd.peek::<DragAsset>() {
        panel.drag_payload = Some(DragPayload::Asset(AssetDragPayload {
            asset_id: d.id,
            asset_type: d.asset_type,
            path: d.path.clone(),
        }));
    } else if let Some(f) = ui.ctx().dnd.peek::<DragFolder>() {
        panel.drag_payload = Some(DragPayload::Folder {
            path: f.path.clone(),
            name: f.name.clone(),
        });
    } else if !ui.ctx().dnd.is_dragging() {
        panel.drag_payload = None;
    }
}
