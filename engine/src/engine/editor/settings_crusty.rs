//! Editor Preferences & Project Settings windows (M10 P7).
//!
//! One shell, two documents: a modal window with scope chip + search header,
//! a fixed category sidebar, 28px setting rows (modified dot, default hint,
//! per-row reset) and a status footer. Editor Preferences live-apply and
//! autosave (debounced) to `editor_prefs.ron`; Project Settings dirty the
//! project and save with Ctrl+S to `project.ron`.

use std::time::Instant;

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::math::{Pos2, Rect, Vec2};
use crusty_gui::text::FontFamily;
use crusty_gui::widgets::{
    Button, Checkbox, ComboBox, DragValue, ScrollArea, SelectableValue, Slider, TextEdit, Toggle,
    Window,
};

use super::build_dialog::BuildTarget;
use super::editor_prefs::{EditorPrefs, ThemePreset, PREFS_FILE};
use super::play_settings::NetPlayMode;
use super::project_config::{ProjectConfig, PROJECT_FILE, SERVER_WORLD_SCENE};
use crate::engine::utils::window_config::VSyncMode;

const SIDEBAR_W: f32 = 158.0;
const HEADER_H: f32 = 34.0;
const FOOTER_H: f32 = 28.0;
const CAT_ROW_H: f32 = 24.0;
const SECTION_H: f32 = 26.0;
const ROW_H: f32 = 28.0;
const GUTTER_W: f32 = 8.0;
const LABEL_W: f32 = 170.0;
const RESET_W: f32 = 22.0;
const FIELD_H: f32 = 18.0;

/// Categories: (sidebar label, row labels for search matching).
const PREFS_CATS: &[(&str, &[&str])] = &[
    ("Appearance", &["Theme preset", "Translucent popovers"]),
    (
        "Viewport",
        &[
            "Fly speed",
            "Speed multiplier",
            "Look sensitivity",
            "Invert Y",
            "Field of view",
            "Show grid",
            "Gizmo size",
        ],
    ),
    ("Snapping", &["Translate snap", "Rotate snap", "Scale snap"]),
    ("Editing", &["Undo history limit"]),
    ("Asset Browser", &["Thumbnail size"]),
    (
        "Console",
        &["Max lines", "Show info", "Show warnings", "Show errors"],
    ),
    (
        "Play",
        &["Play mode", "Server host", "Module name", "Player count"],
    ),
    ("Performance", &["VSync"]),
];

const PROJECT_CATS: &[(&str, &[&str])] = &[
    ("Project", &["Name", "Version"]),
    (
        "Maps & Modes",
        &[
            "Game default scene",
            "Editor startup scene",
            "Server world scene",
        ],
    ),
    ("Physics", &["Gravity Z", "Fixed timestep"]),
    ("Networking", &["Server host", "Module name", "Auto connect"]),
    (
        "Streaming",
        &["Load radius", "Unload radius", "Frame budget", "Max in flight"],
    ),
    ("Build", &["Default target", "Output directory"]),
];

/// UI + persistence state for both settings windows. Owned by the app;
/// `flush_prefs` must be called once per frame for the debounced autosave.
pub struct SettingsState {
    pub prefs: EditorPrefs,
    /// Snapshot the app last applied to live editor state (diffed per frame).
    pub prefs_applied: EditorPrefs,
    prefs_dirty_at: Option<Instant>,
    pub prefs_open: bool,
    prefs_search: String,
    prefs_cat: usize,

    pub project: ProjectConfig,
    pub project_saved: ProjectConfig,
    pub project_open: bool,
    project_search: String,
    project_cat: usize,

    /// Mirror of `WindowConfig.vsync`; the app persists changes + flags restart.
    pub vsync: VSyncMode,
    pub restart_pending: bool,
}

impl SettingsState {
    pub fn new(prefs: EditorPrefs, project: ProjectConfig, vsync: VSyncMode) -> Self {
        Self {
            prefs_applied: prefs.clone(),
            prefs,
            prefs_dirty_at: None,
            prefs_open: false,
            prefs_search: String::new(),
            prefs_cat: 0,
            project_saved: project.clone(),
            project,
            project_open: false,
            project_search: String::new(),
            project_cat: 0,
            vsync,
            restart_pending: false,
        }
    }

    pub fn project_dirty(&self) -> bool {
        self.project != self.project_saved
    }

    pub fn prefs_saving(&self) -> bool {
        self.prefs_dirty_at.is_some()
    }

    /// Restart the autosave debounce; called by the app when it detects a
    /// prefs change (window edits and menu play-settings edits alike).
    pub fn mark_prefs_dirty(&mut self) {
        self.prefs_dirty_at = Some(Instant::now());
    }

    /// Debounced autosave of `editor_prefs.ron` (~500ms after the last edit).
    pub fn flush_prefs(&mut self) {
        if let Some(t) = self.prefs_dirty_at {
            if t.elapsed().as_millis() >= 500 {
                let _ = self.prefs.save();
                self.prefs_dirty_at = None;
            }
        }
    }

    /// Save `project.ron` (+ exported net_config passthrough); Ctrl+S path.
    pub fn save_project(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.project.save()?;
        self.project_saved = self.project.clone();
        Ok(())
    }
}

// ── search filter ────────────────────────────────────────────────────────

struct Filter {
    q: String,
}

impl Filter {
    fn new(s: &str) -> Self {
        Self {
            q: s.trim().to_lowercase(),
        }
    }
    fn active(&self) -> bool {
        !self.q.is_empty()
    }
    fn matches(&self, label: &str) -> bool {
        self.q.is_empty() || label.to_lowercase().contains(&self.q)
    }
    fn cat_matches(&self, rows: &[&str]) -> bool {
        rows.iter().any(|r| self.matches(r))
    }
    /// Should this category's rows draw at all?
    fn cat_visible(&self, idx: usize, selected: usize, rows: &[&str]) -> bool {
        if self.active() {
            self.cat_matches(rows)
        } else {
            idx == selected
        }
    }
}

// ── shell chrome ─────────────────────────────────────────────────────────

/// Header (scope chip + search), sidebar, footer strip. Returns
/// (content rect, footer rect, filter).
fn shell_chrome(
    ui: &mut Ui,
    id: &str,
    scope_word: &str,
    file_word: &str,
    search: &mut String,
    cats: &[(&str, &[&str])],
    selected: &mut usize,
    cat_modified: &[bool],
) -> (Rect, Rect, Filter) {
    let style = ui.style();
    let pal = style.palette;
    let pad = style.spacing.padding;
    let avail = ui.available();
    let full = Rect::from_min_max(
        Pos2::new(avail.min.x - pad, avail.min.y - pad),
        Pos2::new(avail.max.x + pad, avail.max.y + pad),
    );

    // ── header strip: scope chip + file name left, search right
    let header = Rect::from_min_size(full.min, Vec2::new(full.width(), HEADER_H));
    {
        let mut p = ui.painter();
        p.rect_filled(header, 0.0, pal.header);
        p.line_segment(
            Pos2::new(header.min.x, header.max.y),
            Pos2::new(header.max.x, header.max.y),
            1.0,
            pal.stroke,
        );
        let chip_text_w = p
            .measure_text_family(scope_word, 9.0, None, FontFamily::Mono)
            .x;
        let chip = Rect::from_min_size(
            Pos2::new(header.min.x + 10.0, header.min.y + 9.0),
            Vec2::new(chip_text_w + 12.0, 16.0),
        );
        p.rect_filled(chip, 8.0, pal.active);
        p.text_family(
            Pos2::new(chip.min.x + 6.0, chip.min.y + 3.0),
            scope_word,
            9.0,
            pal.text_secondary,
            None,
            FontFamily::Mono,
        );
        p.text_family(
            Pos2::new(chip.max.x + 8.0, header.min.y + 10.0),
            file_word,
            10.0,
            pal.text_mono,
            None,
            FontFamily::Mono,
        );
    }
    ui.set_cursor(Pos2::new(header.max.x - 210.0, header.min.y + 6.0));
    TextEdit::new(search).width(200.0).hint("Search…").show(ui);

    let content_top = header.max.y + 1.0;
    let footer = Rect::from_min_max(
        Pos2::new(full.min.x, full.max.y - FOOTER_H),
        Pos2::new(full.max.x, full.max.y),
    );

    let filter = Filter::new(search);

    // ── sidebar
    let sidebar = Rect::from_min_max(
        Pos2::new(full.min.x, content_top),
        Pos2::new(full.min.x + SIDEBAR_W, footer.min.y),
    );
    ui.painter().rect_filled(sidebar, 0.0, pal.window);
    ui.painter().line_segment(
        Pos2::new(sidebar.max.x, sidebar.min.y),
        Pos2::new(sidebar.max.x, sidebar.max.y),
        1.0,
        pal.stroke,
    );
    let overridden = super::theme::Palette::invariant_status().overridden;
    for (i, (name, rows)) in cats.iter().enumerate() {
        let r = Rect::from_min_size(
            Pos2::new(sidebar.min.x, sidebar.min.y + 6.0 + i as f32 * CAT_ROW_H),
            Vec2::new(SIDEBAR_W, CAT_ROW_H),
        );
        let resp = ui.interact(Id::new(id).with("cat").with(i), r);
        if resp.clicked {
            *selected = i;
        }
        let is_sel = *selected == i && !filter.active();
        if is_sel {
            ui.painter().rect_filled(r, 0.0, pal.selection_fill);
        } else if resp.hovered {
            ui.painter().rect_filled(r, 0.0, pal.hover);
        }
        let dimmed = filter.active() && !filter.cat_matches(rows);
        let color = if dimmed {
            pal.text_disabled
        } else if is_sel {
            pal.selection_text
        } else {
            pal.text_secondary
        };
        ui.painter().text(
            Pos2::new(r.min.x + 12.0, r.min.y + 5.0),
            name,
            style.fonts.body,
            color,
            None,
        );
        if cat_modified.get(i).copied().unwrap_or(false) {
            ui.painter().rect_filled(
                Rect::from_center_size(Pos2::new(r.max.x - 12.0, r.center().y), Vec2::splat(5.0)),
                2.5,
                overridden,
            );
        }
    }

    // ── footer strip
    ui.painter().rect_filled(footer, 0.0, pal.window);
    ui.painter().line_segment(
        Pos2::new(footer.min.x, footer.min.y),
        Pos2::new(footer.max.x, footer.min.y),
        1.0,
        pal.stroke,
    );

    let content = Rect::from_min_max(
        Pos2::new(sidebar.max.x + 1.0, content_top),
        Pos2::new(full.max.x, footer.min.y),
    );
    (content, footer, filter)
}

/// 26px section header bar spanning the content width.
fn section_bar(ui: &mut Ui, label: &str) {
    let style = ui.style();
    let w = ui.available().width();
    let rect = ui.allocate(Vec2::new(w, SECTION_H));
    ui.painter().rect_filled(rect, 3.0, style.palette.header);
    ui.painter().text(
        Pos2::new(rect.min.x + 8.0, rect.min.y + 6.0),
        label,
        11.5,
        style.palette.text_secondary,
        None,
    );
    ui.add_space(3.0);
}

/// 28px setting row: modified dot gutter, 170px label column, control,
/// then right-aligned mono hint + R reset (only when modified) and an
/// optional RESTART chip. Returns true when reset was clicked.
fn setting_row(
    ui: &mut Ui,
    filter: &Filter,
    label: &str,
    modified: bool,
    hint: Option<&str>,
    restart: bool,
    control: impl FnOnce(&mut Ui),
) -> bool {
    if !filter.matches(label) {
        return false;
    }
    let style = ui.style();
    let pal = style.palette;
    let status = super::theme::Palette::invariant_status();
    let top = ui.cursor().y;
    let mut reset = false;
    ui.horizontal(|ui| {
        let start = ui.cursor();
        let right = ui.available().max.x;
        if modified {
            ui.painter().rect_filled(
                Rect::from_center_size(
                    Pos2::new(start.x + 2.5, start.y + ROW_H * 0.5),
                    Vec2::splat(5.0),
                ),
                2.5,
                status.overridden,
            );
        }
        let label_color = if modified { pal.text } else { pal.text_secondary };
        ui.painter().text(
            Pos2::new(start.x + GUTTER_W, start.y + 6.0),
            label,
            style.fonts.body,
            label_color,
            None,
        );
        ui.set_cursor(Pos2::new(start.x + GUTTER_W + LABEL_W, start.y + 3.0));
        control(ui);

        let mut right_edge = right;
        if restart {
            let text = "RESTART";
            let w = ui
                .painter()
                .measure_text_family(text, 9.0, None, FontFamily::Mono)
                .x;
            let chip = Rect::from_min_size(
                Pos2::new(right - w - 12.0, start.y + 6.0),
                Vec2::new(w + 12.0, 16.0),
            );
            ui.painter()
                .rect_filled(chip, 8.0, status.warning.with_alpha(0.12));
            ui.painter().text_family(
                Pos2::new(chip.min.x + 6.0, chip.min.y + 3.5),
                text,
                9.0,
                status.warning.with_alpha(0.9),
                None,
                FontFamily::Mono,
            );
            right_edge = chip.min.x - 8.0;
        }
        if modified {
            ui.set_cursor(Pos2::new(right_edge - RESET_W, start.y + 5.0));
            let b = Button::new("R")
                .exact_size(Vec2::new(RESET_W, FIELD_H))
                .show(ui);
            if b.hovered {
                ui.tooltip_for(b.rect, "Reset");
            }
            reset = b.clicked;
            if let Some(h) = hint {
                let w = ui
                    .painter()
                    .measure_text_family(h, 10.0, None, FontFamily::Mono)
                    .x;
                ui.painter().text_family(
                    Pos2::new(right_edge - RESET_W - 8.0 - w, start.y + 8.0),
                    h,
                    10.0,
                    pal.text_disabled,
                    None,
                    FontFamily::Mono,
                );
            }
        }
    });
    let advanced = ui.cursor().y - top;
    if advanced < ROW_H {
        ui.add_space(ROW_H - advanced);
    }
    reset
}

/// Compact float formatting for default hints ("1", "0.003", "45").
fn fmt_f(v: f32) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

fn slider_drag(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str) {
    let span = *range.end() - *range.start();
    let field_bg = ui.style().palette.input;
    let slider_w = 96.0;
    Slider::new(value, range.clone()).width(slider_w).show(ui);
    ui.add_space(6.0);
    DragValue::new(value)
        .speed(span / slider_w)
        .range(range)
        .suffix(suffix)
        .width(64.0)
        .height(FIELD_H)
        .fill(field_bg)
        .show(ui);
}

fn drag(ui: &mut Ui, value: &mut f32, speed: f64, range: std::ops::RangeInclusive<f32>, suffix: &str) {
    let field_bg = ui.style().palette.input;
    DragValue::new(value)
        .speed(speed)
        .range(range)
        .suffix(suffix)
        .width(64.0)
        .height(FIELD_H)
        .fill(field_bg)
        .show(ui);
}

// ── Editor Preferences ───────────────────────────────────────────────────

pub fn editor_prefs_window(ui: &mut Ui, state: &mut SettingsState) {
    if !state.prefs_open {
        return;
    }
    let mut open = true;

    Window::new("Editor Preferences")
        .modal(true)
        .resizable(false)
        .collapsible(false)
        .anchor_center(true)
        .default_size(Vec2::new(780.0, 560.0))
        .open(&mut open)
        .show(ui, |ui| {
            let d = EditorPrefs::default();
            let p = &state.prefs;
            let cat_mod = [
                p.theme_preset != d.theme_preset || p.popover_translucent != d.popover_translucent,
                p.camera_speed != d.camera_speed
                    || p.camera_speed_scalar != d.camera_speed_scalar
                    || p.mouse_sensitivity != d.mouse_sensitivity
                    || p.invert_y != d.invert_y
                    || p.fov_deg != d.fov_deg
                    || p.grid_visible != d.grid_visible
                    || p.gizmo_size != d.gizmo_size,
                p.grid_snap_enabled != d.grid_snap_enabled
                    || p.rotation_snap_enabled != d.rotation_snap_enabled
                    || p.scale_snap_enabled != d.scale_snap_enabled
                    || p.snap_translate != d.snap_translate
                    || p.snap_rotate != d.snap_rotate
                    || p.snap_scale != d.snap_scale,
                p.undo_limit != d.undo_limit,
                p.thumbnail_size != d.thumbnail_size,
                p.console_max_lines != d.console_max_lines
                    || p.console_show_info != d.console_show_info
                    || p.console_show_warning != d.console_show_warning
                    || p.console_show_error != d.console_show_error,
                p.play != super::play_settings::PlaySettings::default(),
                state.vsync != VSyncMode::default(),
            ];

            let (content, footer, filter) = shell_chrome(
                ui,
                "prefs",
                "user",
                PREFS_FILE,
                &mut state.prefs_search,
                PREFS_CATS,
                &mut state.prefs_cat,
                &cat_mod,
            );

            let opts = UiOptions {
                padding: Vec2::new(12.0, 8.0),
                spacing: 0.0,
            };
            let selected = state.prefs_cat;
            let prefs = &mut state.prefs;
            let vsync = &mut state.vsync;
            ui.run_at(content, Direction::TopDown, Id::new("prefs_content"), opts, |ui| {
                let h = ui.available_size().y;
                ScrollArea::new(h)
                    .auto_shrink(false)
                    .inset(0.0)
                    .spacing(0.0)
                    .show(ui, |ui| {
                        draw_prefs_rows(ui, &filter, selected, prefs, vsync);
                    });
            });

            let fopts = UiOptions {
                padding: Vec2::new(10.0, 4.0),
                spacing: 8.0,
            };
            ui.run_at(footer, Direction::LeftToRight, Id::new("prefs_footer"), fopts, |ui| {
                draw_prefs_footer(ui, footer, state);
            });
        });

    if !open {
        state.prefs_open = false;
    }
}

fn draw_prefs_rows(
    ui: &mut Ui,
    f: &Filter,
    selected: usize,
    p: &mut EditorPrefs,
    vsync: &mut VSyncMode,
) {
    let d = EditorPrefs::default();
    let vis = |i: usize| f.cat_visible(i, selected, PREFS_CATS[i].1);

    if vis(0) {
        section_bar(ui, "Appearance");
        let hint = format!("default {}", d.theme_preset.label());
        let m = p.theme_preset != d.theme_preset;
        if setting_row(ui, f, "Theme preset", m, Some(&hint), false, |ui| {
            ComboBox::new("prefs_theme")
                .selected_text(p.theme_preset.label())
                .width(140.0)
                .show_ui(ui, |ui| {
                    for t in ThemePreset::ALL {
                        SelectableValue::new(&mut p.theme_preset, t, t.label()).show(ui);
                    }
                });
        }) {
            p.theme_preset = d.theme_preset;
        }
        let m = p.popover_translucent != d.popover_translucent;
        if setting_row(ui, f, "Translucent popovers", m, Some("default on"), false, |ui| {
            Toggle::new(&mut p.popover_translucent, "").show(ui);
        }) {
            p.popover_translucent = d.popover_translucent;
        }
    }

    if vis(1) {
        section_bar(ui, "Viewport — Camera");
        let hint = format!("default {}", fmt_f(d.camera_speed));
        let m = p.camera_speed != d.camera_speed;
        if setting_row(ui, f, "Fly speed", m, Some(&hint), false, |ui| {
            slider_drag(ui, &mut p.camera_speed, 0.1..=10.0, "");
        }) {
            p.camera_speed = d.camera_speed;
        }
        let hint = format!("default {}", fmt_f(d.camera_speed_scalar));
        let m = p.camera_speed_scalar != d.camera_speed_scalar;
        if setting_row(ui, f, "Speed multiplier", m, Some(&hint), false, |ui| {
            drag(ui, &mut p.camera_speed_scalar, 0.02, 0.1..=10.0, "x");
        }) {
            p.camera_speed_scalar = d.camera_speed_scalar;
        }
        let hint = format!("default {}", fmt_f(d.mouse_sensitivity));
        let m = p.mouse_sensitivity != d.mouse_sensitivity;
        if setting_row(ui, f, "Look sensitivity", m, Some(&hint), false, |ui| {
            slider_drag(ui, &mut p.mouse_sensitivity, 0.0005..=0.01, "");
        }) {
            p.mouse_sensitivity = d.mouse_sensitivity;
        }
        let m = p.invert_y != d.invert_y;
        if setting_row(ui, f, "Invert Y", m, Some("default off"), false, |ui| {
            Checkbox::new(&mut p.invert_y, "").show(ui);
        }) {
            p.invert_y = d.invert_y;
        }
        let hint = format!("default {}°", fmt_f(d.fov_deg));
        let m = p.fov_deg != d.fov_deg;
        if setting_row(ui, f, "Field of view", m, Some(&hint), false, |ui| {
            slider_drag(ui, &mut p.fov_deg, 30.0..=110.0, "°");
        }) {
            p.fov_deg = d.fov_deg;
        }

        section_bar(ui, "Viewport — Grid & Gizmos");
        let m = p.grid_visible != d.grid_visible;
        if setting_row(ui, f, "Show grid", m, Some("default on"), false, |ui| {
            Checkbox::new(&mut p.grid_visible, "").show(ui);
        }) {
            p.grid_visible = d.grid_visible;
        }
        let hint = format!("default {}", fmt_f(d.gizmo_size));
        let m = p.gizmo_size != d.gizmo_size;
        if setting_row(ui, f, "Gizmo size", m, Some(&hint), false, |ui| {
            slider_drag(ui, &mut p.gizmo_size, 40.0..=120.0, "px");
        }) {
            p.gizmo_size = d.gizmo_size;
        }
    }

    if vis(2) {
        section_bar(ui, "Snapping");
        let hint = format!("default off · {}", fmt_f(d.snap_translate));
        let m = p.grid_snap_enabled != d.grid_snap_enabled || p.snap_translate != d.snap_translate;
        if setting_row(ui, f, "Translate snap", m, Some(&hint), false, |ui| {
            Checkbox::new(&mut p.grid_snap_enabled, "").show(ui);
            ui.add_space(8.0);
            drag(ui, &mut p.snap_translate, 0.05, 0.01..=100.0, " m");
        }) {
            p.grid_snap_enabled = d.grid_snap_enabled;
            p.snap_translate = d.snap_translate;
        }
        let hint = format!("default off · {}°", fmt_f(d.snap_rotate));
        let m =
            p.rotation_snap_enabled != d.rotation_snap_enabled || p.snap_rotate != d.snap_rotate;
        if setting_row(ui, f, "Rotate snap", m, Some(&hint), false, |ui| {
            Checkbox::new(&mut p.rotation_snap_enabled, "").show(ui);
            ui.add_space(8.0);
            drag(ui, &mut p.snap_rotate, 0.5, 1.0..=90.0, "°");
        }) {
            p.rotation_snap_enabled = d.rotation_snap_enabled;
            p.snap_rotate = d.snap_rotate;
        }
        let hint = format!("default off · {}", fmt_f(d.snap_scale));
        let m = p.scale_snap_enabled != d.scale_snap_enabled || p.snap_scale != d.snap_scale;
        if setting_row(ui, f, "Scale snap", m, Some(&hint), false, |ui| {
            Checkbox::new(&mut p.scale_snap_enabled, "").show(ui);
            ui.add_space(8.0);
            drag(ui, &mut p.snap_scale, 0.01, 0.01..=10.0, "");
        }) {
            p.scale_snap_enabled = d.scale_snap_enabled;
            p.snap_scale = d.snap_scale;
        }
    }

    if vis(3) {
        section_bar(ui, "Editing");
        let mut v = p.undo_limit as f32;
        let hint = format!("default {}", d.undo_limit);
        let m = p.undo_limit != d.undo_limit;
        if setting_row(ui, f, "Undo history limit", m, Some(&hint), false, |ui| {
            let bg = ui.style().palette.input;
            DragValue::new(&mut v)
                .speed(1.0)
                .range(10.0..=1000.0)
                .decimals(0)
                .width(64.0)
                .height(FIELD_H)
                .fill(bg)
                .show(ui);
        }) {
            v = d.undo_limit as f32;
        }
        p.undo_limit = v.round() as usize;
    }

    if vis(4) {
        section_bar(ui, "Asset Browser");
        let hint = format!("default {}", fmt_f(d.thumbnail_size));
        let m = p.thumbnail_size != d.thumbnail_size;
        if setting_row(ui, f, "Thumbnail size", m, Some(&hint), false, |ui| {
            slider_drag(ui, &mut p.thumbnail_size, 48.0..=192.0, "px");
        }) {
            p.thumbnail_size = d.thumbnail_size;
        }
    }

    if vis(5) {
        section_bar(ui, "Console");
        let mut v = p.console_max_lines as f32;
        let hint = format!("default {}", d.console_max_lines);
        let m = p.console_max_lines != d.console_max_lines;
        if setting_row(ui, f, "Max lines", m, Some(&hint), false, |ui| {
            let bg = ui.style().palette.input;
            DragValue::new(&mut v)
                .speed(10.0)
                .range(100.0..=20000.0)
                .decimals(0)
                .width(72.0)
                .height(FIELD_H)
                .fill(bg)
                .show(ui);
        }) {
            v = d.console_max_lines as f32;
        }
        p.console_max_lines = v.round() as usize;

        for (label, value, def) in [
            ("Show info", &mut p.console_show_info, d.console_show_info),
            (
                "Show warnings",
                &mut p.console_show_warning,
                d.console_show_warning,
            ),
            ("Show errors", &mut p.console_show_error, d.console_show_error),
        ] {
            let m = *value != def;
            if setting_row(ui, f, label, m, Some("default on"), false, |ui| {
                Checkbox::new(value, "").show(ui);
            }) {
                *value = def;
            }
        }
    }

    if vis(6) {
        section_bar(ui, "Play");
        let dp = super::play_settings::PlaySettings::default();
        let m = p.play.mode != dp.mode;
        let hint = format!("default {}", dp.mode.label());
        if setting_row(ui, f, "Play mode", m, Some(&hint), false, |ui| {
            ComboBox::new("prefs_play_mode")
                .selected_text(p.play.mode.label())
                .width(140.0)
                .show_ui(ui, |ui| {
                    for mode in [
                        NetPlayMode::Standalone,
                        NetPlayMode::Client,
                        NetPlayMode::ListenServer,
                    ] {
                        SelectableValue::new(&mut p.play.mode, mode, mode.label()).show(ui);
                    }
                });
        }) {
            p.play.mode = dp.mode;
        }
        let m = p.play.host != dp.host;
        let hint = format!("default {}", dp.host);
        if setting_row(ui, f, "Server host", m, Some(&hint), false, |ui| {
            TextEdit::new(&mut p.play.host).width(200.0).show(ui);
        }) {
            p.play.host = dp.host.clone();
        }
        let m = p.play.module != dp.module;
        let hint = format!("default {}", dp.module);
        if setting_row(ui, f, "Module name", m, Some(&hint), false, |ui| {
            TextEdit::new(&mut p.play.module).width(160.0).show(ui);
        }) {
            p.play.module = dp.module.clone();
        }
        let mut v = p.play.player_count as f32;
        let m = p.play.player_count != dp.player_count;
        let hint = format!("default {}", dp.player_count);
        if setting_row(ui, f, "Player count", m, Some(&hint), false, |ui| {
            let bg = ui.style().palette.input;
            DragValue::new(&mut v)
                .speed(0.05)
                .range(1.0..=4.0)
                .decimals(0)
                .width(48.0)
                .height(FIELD_H)
                .fill(bg)
                .show(ui);
        }) {
            v = dp.player_count as f32;
        }
        p.play.player_count = v.round() as u8;
    }

    if vis(7) {
        section_bar(ui, "Performance");
        let vsync_label = |v: VSyncMode| match v {
            VSyncMode::Off => "Off (uncapped)",
            VSyncMode::Mailbox => "Mailbox (triple buffer)",
            VSyncMode::On => "On (Fifo)",
        };
        let m = *vsync != VSyncMode::default();
        if setting_row(ui, f, "VSync", m, Some("default Off"), true, |ui| {
            ComboBox::new("prefs_vsync")
                .selected_text(vsync_label(*vsync))
                .width(180.0)
                .show_ui(ui, |ui| {
                    for v in [VSyncMode::Off, VSyncMode::Mailbox, VSyncMode::On] {
                        SelectableValue::new(vsync, v, vsync_label(v)).show(ui);
                    }
                });
        }) {
            *vsync = VSyncMode::default();
        }
    }
}

fn draw_prefs_footer(ui: &mut Ui, footer: Rect, state: &mut SettingsState) {
    let style = ui.style();
    let pal = style.palette;
    let status = super::theme::Palette::invariant_status();
    let cy = footer.center().y;

    ui.painter().text_family(
        Pos2::new(footer.min.x + 10.0, cy - 5.0),
        PREFS_FILE,
        10.0,
        pal.text_mono,
        None,
        FontFamily::Mono,
    );

    // Right side: reset-all button, then status text to its left.
    let btn_w = 74.0;
    ui.set_cursor(Pos2::new(footer.max.x - btn_w - 8.0, footer.min.y + 4.0));
    if Button::new("Reset All")
        .danger_outline()
        .min_size(Vec2::new(btn_w, 20.0))
        .show(ui)
        .clicked
    {
        state.prefs = EditorPrefs::default();
        state.vsync = VSyncMode::default();
    }
    let (text, color) = if state.restart_pending {
        ("Applied · restart for VSync", status.warning)
    } else if state.prefs_saving() {
        ("Applied · saving…", pal.text_secondary)
    } else {
        ("Applied · saved", status.success)
    };
    let w = ui.painter().measure_text(text, 11.0, None).x;
    ui.painter().text(
        Pos2::new(footer.max.x - btn_w - 20.0 - w, cy - 5.5),
        text,
        11.0,
        color,
        None,
    );
}

// ── Project Settings ─────────────────────────────────────────────────────

pub fn project_settings_window(ui: &mut Ui, state: &mut SettingsState) {
    if !state.project_open {
        return;
    }
    let mut open = true;

    Window::new("Project Settings")
        .modal(true)
        .resizable(false)
        .collapsible(false)
        .anchor_center(true)
        .default_size(Vec2::new(780.0, 520.0))
        .open(&mut open)
        .show(ui, |ui| {
            let c = &state.project;
            let s = &state.project_saved;
            let cat_mod = [
                c.name != s.name || c.version != s.version,
                c.default_scene != s.default_scene
                    || c.editor_startup_scene != s.editor_startup_scene,
                c.gravity_z != s.gravity_z || c.fixed_timestep_hz != s.fixed_timestep_hz,
                c.net_host != s.net_host
                    || c.net_module != s.net_module
                    || c.net_auto_connect != s.net_auto_connect,
                c.stream_r_load != s.stream_r_load
                    || c.stream_r_unload != s.stream_r_unload
                    || c.stream_budget_ms != s.stream_budget_ms
                    || c.stream_max_in_flight != s.stream_max_in_flight,
                c.build_target != s.build_target || c.build_output_dir != s.build_output_dir,
            ];

            let (content, footer, filter) = shell_chrome(
                ui,
                "project",
                "project",
                PROJECT_FILE,
                &mut state.project_search,
                PROJECT_CATS,
                &mut state.project_cat,
                &cat_mod,
            );

            let opts = UiOptions {
                padding: Vec2::new(12.0, 8.0),
                spacing: 0.0,
            };
            let selected = state.project_cat;
            let saved = state.project_saved.clone();
            let project = &mut state.project;
            ui.run_at(
                content,
                Direction::TopDown,
                Id::new("project_content"),
                opts,
                |ui| {
                    let h = ui.available_size().y;
                    ScrollArea::new(h)
                        .auto_shrink(false)
                        .inset(0.0)
                        .spacing(0.0)
                        .show(ui, |ui| {
                            draw_project_rows(ui, &filter, selected, project, &saved);
                        });
                },
            );

            let fopts = UiOptions {
                padding: Vec2::new(10.0, 4.0),
                spacing: 8.0,
            };
            ui.run_at(
                footer,
                Direction::LeftToRight,
                Id::new("project_footer"),
                fopts,
                |ui| {
                    draw_project_footer(ui, footer, state);
                },
            );
        });

    if !open {
        state.project_open = false;
    }
}

fn draw_project_rows(
    ui: &mut Ui,
    f: &Filter,
    selected: usize,
    c: &mut ProjectConfig,
    s: &ProjectConfig,
) {
    let vis = |i: usize| f.cat_visible(i, selected, PROJECT_CATS[i].1);

    if vis(0) {
        section_bar(ui, "Project");
        let m = c.name != s.name;
        let hint = format!("saved {}", s.name);
        if setting_row(ui, f, "Name", m, Some(&hint), false, |ui| {
            TextEdit::new(&mut c.name).width(180.0).show(ui);
        }) {
            c.name = s.name.clone();
        }
        let m = c.version != s.version;
        let hint = format!("saved {}", s.version);
        if setting_row(ui, f, "Version", m, Some(&hint), false, |ui| {
            TextEdit::new(&mut c.version).width(90.0).show(ui);
        }) {
            c.version = s.version.clone();
        }
    }

    if vis(1) {
        section_bar(ui, "Maps & Modes");
        let m = c.default_scene != s.default_scene;
        let hint = format!("saved {}", s.default_scene);
        if setting_row(ui, f, "Game default scene", m, Some(&hint), false, |ui| {
            TextEdit::new(&mut c.default_scene)
                .width(220.0)
                .hint("scenes/….scene")
                .show(ui);
        }) {
            c.default_scene = s.default_scene.clone();
        }
        let m = c.editor_startup_scene != s.editor_startup_scene;
        let hint = if s.editor_startup_scene.is_empty() {
            "saved (game default)".to_string()
        } else {
            format!("saved {}", s.editor_startup_scene)
        };
        if setting_row(ui, f, "Editor startup scene", m, Some(&hint), false, |ui| {
            TextEdit::new(&mut c.editor_startup_scene)
                .width(220.0)
                .hint("empty = game default")
                .show(ui);
        }) {
            c.editor_startup_scene = s.editor_startup_scene.clone();
        }
        setting_row(ui, f, "Server world scene", false, None, false, |ui| {
            let pal = ui.style().palette;
            let start = ui.cursor();
            let w = ui.painter().text_family(
                Pos2::new(start.x, start.y + 4.0),
                SERVER_WORLD_SCENE,
                11.0,
                pal.text_mono,
                None,
                FontFamily::Mono,
            );
            ui.painter().text(
                Pos2::new(start.x + w.x + 10.0, start.y + 4.0),
                "· compiled into module — republish to change",
                10.5,
                pal.text_disabled,
                None,
            );
        });
    }

    if vis(2) {
        section_bar(ui, "Physics");
        let m = c.gravity_z != s.gravity_z;
        let hint = format!("saved {}", fmt_f(s.gravity_z));
        if setting_row(ui, f, "Gravity Z", m, Some(&hint), false, |ui| {
            drag(ui, &mut c.gravity_z, 0.05, -50.0..=50.0, " m/s²");
        }) {
            c.gravity_z = s.gravity_z;
        }
        let m = c.fixed_timestep_hz != s.fixed_timestep_hz;
        let hint = format!("saved {}", fmt_f(s.fixed_timestep_hz));
        if setting_row(ui, f, "Fixed timestep", m, Some(&hint), false, |ui| {
            drag(ui, &mut c.fixed_timestep_hz, 0.5, 10.0..=240.0, " Hz");
        }) {
            c.fixed_timestep_hz = s.fixed_timestep_hz;
        }
    }

    if vis(3) {
        section_bar(ui, "Networking");
        let m = c.net_host != s.net_host;
        let hint = format!("saved {}", s.net_host);
        if setting_row(ui, f, "Server host", m, Some(&hint), false, |ui| {
            TextEdit::new(&mut c.net_host).width(220.0).show(ui);
        }) {
            c.net_host = s.net_host.clone();
        }
        let m = c.net_module != s.net_module;
        let hint = format!("saved {}", s.net_module);
        if setting_row(ui, f, "Module name", m, Some(&hint), false, |ui| {
            TextEdit::new(&mut c.net_module).width(160.0).show(ui);
        }) {
            c.net_module = s.net_module.clone();
        }
        let m = c.net_auto_connect != s.net_auto_connect;
        let hint = format!("saved {}", if s.net_auto_connect { "on" } else { "off" });
        if setting_row(ui, f, "Auto connect", m, Some(&hint), false, |ui| {
            Checkbox::new(&mut c.net_auto_connect, "").show(ui);
        }) {
            c.net_auto_connect = s.net_auto_connect;
        }
    }

    if vis(4) {
        section_bar(ui, "Streaming");
        let mut v = c.stream_r_load as f32;
        let m = c.stream_r_load != s.stream_r_load;
        let hint = format!("saved {}", s.stream_r_load);
        if setting_row(ui, f, "Load radius", m, Some(&hint), false, |ui| {
            drag(ui, &mut v, 0.05, 0.0..=8.0, " cells");
        }) {
            v = s.stream_r_load as f32;
        }
        c.stream_r_load = v.round() as i32;

        let mut v = c.stream_r_unload as f32;
        let m = c.stream_r_unload != s.stream_r_unload;
        let hint = format!("saved {}", s.stream_r_unload);
        if setting_row(ui, f, "Unload radius", m, Some(&hint), false, |ui| {
            drag(ui, &mut v, 0.05, 0.0..=10.0, " cells");
        }) {
            v = s.stream_r_unload as f32;
        }
        c.stream_r_unload = v.round() as i32;

        let m = c.stream_budget_ms != s.stream_budget_ms;
        let hint = format!("saved {}", fmt_f(s.stream_budget_ms));
        if setting_row(ui, f, "Frame budget", m, Some(&hint), false, |ui| {
            drag(ui, &mut c.stream_budget_ms, 0.02, 0.1..=8.0, " ms");
        }) {
            c.stream_budget_ms = s.stream_budget_ms;
        }

        let mut v = c.stream_max_in_flight as f32;
        let m = c.stream_max_in_flight != s.stream_max_in_flight;
        let hint = format!("saved {}", s.stream_max_in_flight);
        if setting_row(ui, f, "Max in flight", m, Some(&hint), false, |ui| {
            drag(ui, &mut v, 0.05, 1.0..=8.0, "");
        }) {
            v = s.stream_max_in_flight as f32;
        }
        c.stream_max_in_flight = v.round() as usize;
    }

    if vis(5) {
        section_bar(ui, "Build");
        let m = c.build_target != s.build_target;
        let hint = format!("saved {}", s.build_target.label());
        if setting_row(ui, f, "Default target", m, Some(&hint), false, |ui| {
            ComboBox::new("project_build_target")
                .selected_text(c.build_target.label())
                .width(140.0)
                .show_ui(ui, |ui| {
                    for t in [
                        BuildTarget::Standalone,
                        BuildTarget::MpClient,
                        BuildTarget::MpServer,
                    ] {
                        SelectableValue::new(&mut c.build_target, t, t.label()).show(ui);
                    }
                });
        }) {
            c.build_target = s.build_target;
        }
        let m = c.build_output_dir != s.build_output_dir;
        let hint = format!("saved {}", s.build_output_dir);
        if setting_row(ui, f, "Output directory", m, Some(&hint), false, |ui| {
            TextEdit::new(&mut c.build_output_dir).width(220.0).show(ui);
        }) {
            c.build_output_dir = s.build_output_dir.clone();
        }
    }
}

fn draw_project_footer(ui: &mut Ui, footer: Rect, state: &mut SettingsState) {
    let style = ui.style();
    let pal = style.palette;
    let status = super::theme::Palette::invariant_status();
    let cy = footer.center().y;

    ui.painter().text_family(
        Pos2::new(footer.min.x + 10.0, cy - 5.0),
        PROJECT_FILE,
        10.0,
        pal.text_mono,
        None,
        FontFamily::Mono,
    );

    if state.project_dirty() {
        let btn_w = 54.0;
        ui.set_cursor(Pos2::new(footer.max.x - btn_w - 8.0, footer.min.y + 4.0));
        if Button::new("Save")
            .primary()
            .min_size(Vec2::new(btn_w, 20.0))
            .show(ui)
            .clicked
        {
            let _ = state.save_project();
        }
        let text = "Modified — saves with Ctrl+S";
        let w = ui.painter().measure_text(text, 11.0, None).x;
        ui.painter().text(
            Pos2::new(footer.max.x - btn_w - 20.0 - w, cy - 5.5),
            text,
            11.0,
            status.warning,
            None,
        );
    } else {
        let text = "Saved";
        let w = ui.painter().measure_text(text, 11.0, None).x;
        ui.painter()
            .text(Pos2::new(footer.max.x - 8.0 - w, cy - 5.5), text, 11.0, status.success, None);
    }
}
