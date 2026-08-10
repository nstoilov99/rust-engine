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
use super::keymap::{Action, ActionStatus, Chord, Context, Keymap, MouseProfile, Preset};
use crusty_gui::input::Key;
use super::graph_prefs::{CrossingStyle, ExecWirePrefs, TurnAnchor, TurnPriority, WireStyle};
use super::graph_wire_router::{self as router, RouteMeta};
use super::theme::{wire_color, Density, UI_SCALE_MAX, UI_SCALE_MIN};
use super::widgets::segmented_control;
use crate::engine::node_graph::PinType;
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
    (
        "Appearance",
        &["Theme preset", "Translucent popovers", "UI scale", "Density"],
    ),
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
    (
        "Keyboard Shortcuts",
        &["Preset", "Keyboard shortcuts", "Rebind", "Chord"],
    ),
    (
        "Graph",
        &[
            // Wires
            "Wire style",
            "Horizontal offset",
            "Turn anchor",
            "Corner radius",
            "Turn priority",
            "Disable pin offset",
            // Execution wires
            "Override style for exec wires",
            // Bundling
            "Bundle overlapping wires",
            "Bundle offset",
            "Merge offset",
            "Push outside",
            "Max wires per lane",
            // Crossings
            "Crossing style",
            "Crossing size",
            // Flow bubbles
            "Show flow bubbles",
            "Exec wires only",
            "Selected nodes only",
            "Size",
            "Speed",
            "Spacing",
            // Canvas
            "Min zoom",
            "Max zoom (Ctrl+wheel)",
        ],
    ),
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
    // 39.8 D8 delta 4: the Plugin Manager is a Project Settings page — the
    // plugin set is `project.ron`, VCS-checked-in, Ctrl+S dirty semantics.
    ("Plugins", &["Plugins", "Enable", "Disable", "Relaunch"]),
];

/// Index of the Plugins category — it replaces the rows pane wholesale
/// rather than drawing setting rows.
const PLUGINS_CAT: usize = PROJECT_CATS.len() - 1;

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
    /// Selected Project Settings category. Public so the app can deep-link
    /// (Edit ▸ Plugins opens straight to the manager).
    pub project_cat: usize,

    /// Mirror of `WindowConfig.vsync`; the app persists changes + flags restart.
    pub vsync: VSyncMode,
    pub restart_pending: bool,

    /// Plugin Manager page state (39.8 P6).
    pub plugins: super::plugin_manager::PluginManagerState,

    /// Action whose chord cell is currently capturing a keypress.
    keybind_capture: Option<Action>,
    /// A captured chord that collides with an existing binding, held pending
    /// the user's "Rebind anyway / Cancel". Deliberately not applied first and
    /// undone after — an accidental collision should never silently steal a
    /// key you rely on.
    keybind_pending: Option<(Action, Chord, String)>,
    /// Sections the user collapsed, by context label.
    keybind_collapsed: Vec<String>,
    /// Set when the keymap changed; the app debounces the `keymap.ron` write
    /// exactly as it does `editor_prefs.ron`.
    keymap_dirty_at: Option<Instant>,
    /// Reset All is behind a confirm — it discards every rebinding at once.
    keymap_reset_confirm: bool,
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
            keybind_capture: None,
            keybind_pending: None,
            keybind_collapsed: Vec::new(),
            keymap_dirty_at: None,
            keymap_reset_confirm: false,
            project_saved: project.clone(),
            project,
            project_open: false,
            project_search: String::new(),
            project_cat: 0,
            vsync,
            restart_pending: false,
            plugins: super::plugin_manager::PluginManagerState::default(),
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

    /// Restart the `keymap.ron` autosave debounce.
    pub fn mark_keymap_dirty(&mut self) {
        self.keymap_dirty_at = Some(Instant::now());
    }

    pub fn keymap_saving(&self) -> bool {
        self.keymap_dirty_at.is_some()
    }

    /// Debounced autosave of `keymap.ron`, on the same 500ms as prefs — a
    /// rebind is an edit like any other, with no OK button to press.
    pub fn flush_keymap(&mut self, keymap: &Keymap) {
        if let Some(t) = self.keymap_dirty_at {
            if t.elapsed().as_millis() >= 500 {
                let _ = keymap.save();
                self.keymap_dirty_at = None;
            }
        }
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

// ─────────────────────────────────────────────────────────────────────────────
// Keyboard Shortcuts
// ─────────────────────────────────────────────────────────────────────────────

/// The Keyboard Shortcuts page. Returns `true` when the keymap changed, so the
/// caller can start the autosave debounce.
///
/// Unlike the other categories this one is generated: every row comes from
/// `Keymap::rows()`, which is built from `Action::all()`. That is what makes
/// "every action is reachable here" a property of the data rather than a list
/// somebody has to remember to extend.
#[allow(clippy::too_many_arguments)]
fn draw_keymap_rows(
    ui: &mut Ui,
    f: &Filter,
    keymap: &mut Keymap,
    capture: &mut Option<Action>,
    pending: &mut Option<(Action, Chord, String)>,
    collapsed: &mut Vec<String>,
    reset_confirm: &mut bool,
) -> bool {
    let mut changed = false;
    section_bar(ui, "Keyboard Shortcuts");

    // ── Preset + Reset All
    let preset = keymap.preset;
    if setting_row(ui, f, "Preset", preset != Preset::default(), Some("default Crusty"), false, |ui| {
        ComboBox::new("keymap_preset")
            .selected_text(preset.label())
            .width(160.0)
            .show_ui(ui, |ui| {
                for p in Preset::ALL {
                    let mut sel = preset;
                    if SelectableValue::new(&mut sel, p, p.label()).show(ui).clicked {
                        // Rebasing drops every overlay entry, including ones
                        // that happened to match the old preset's default —
                        // switching preset means "give me that preset".
                        *keymap = Keymap::from_preset(p);
                    }
                }
            });
    }) {
        *keymap = Keymap::from_preset(Preset::default());
    }
    if *keymap != Keymap::from_preset(preset) || keymap.preset != preset {
        changed = true;
    }

    let profile = keymap.mouse_profile;
    setting_row(
        ui,
        f,
        "Mouse dialect",
        profile != MouseProfile::default(),
        Some("consumed by the input model"),
        false,
        |ui| {
            ComboBox::new("keymap_mouse")
                .selected_text(profile.label())
                .width(160.0)
                .show_ui(ui, |ui| {
                    for m in MouseProfile::ALL {
                        SelectableValue::new(&mut keymap.mouse_profile, m, m.label()).show(ui);
                    }
                });
        },
    );

    if f.matches("Reset all shortcuts") {
        ui.horizontal(|ui| {
            let start = ui.cursor();
            ui.set_cursor(Pos2::new(start.x + GUTTER_W, start.y + 3.0));
            if *reset_confirm {
                let body = ui.style().fonts.body;
                ui.painter().text(
                    Pos2::new(start.x + GUTTER_W, start.y + 6.0),
                    "Discard every rebinding?",
                    body,
                    super::theme::Palette::invariant_status().warning,
                    None,
                );
                ui.set_cursor(Pos2::new(start.x + GUTTER_W + LABEL_W, start.y + 3.0));
                if ui.button("Reset All").clicked {
                    *keymap = Keymap::from_preset(keymap.preset);
                    *reset_confirm = false;
                    changed = true;
                }
                if ui.button("Cancel").clicked {
                    *reset_confirm = false;
                }
            } else if ui.button("Reset All…").clicked {
                *reset_confirm = true;
            }
        });
        ui.add_space(ROW_H * 0.5);
    }

    // ── The generated rows, grouped by context then group.
    let rows = keymap.rows_matching(&f.q);
    let mut last_ctx: Option<Context> = None;
    let mut last_group = String::new();
    for row in rows {
        let ctx = row.action.context();
        if last_ctx != Some(ctx) {
            last_ctx = Some(ctx);
            last_group.clear();
            let label = ctx.label().to_string();
            let is_collapsed = collapsed.contains(&label);
            if keymap_section_header(ui, &label, is_collapsed) {
                if is_collapsed {
                    collapsed.retain(|c| *c != label);
                } else {
                    collapsed.push(label.clone());
                }
            }
        }
        if collapsed.iter().any(|c| c == ctx.label()) {
            continue;
        }
        if last_group != row.action.group() {
            last_group = row.action.group().to_string();
            keymap_group_header(ui, &last_group);
        }
        changed |= keymap_row(ui, keymap, row.action, capture, pending);
    }
    changed
}

/// Collapsing context header. Returns `true` when clicked.
fn keymap_section_header(ui: &mut Ui, label: &str, collapsed: bool) -> bool {
    let style = ui.style();
    let w = ui.available().width();
    let rect = ui.allocate(Vec2::new(w, SECTION_H));
    let resp = ui.interact(Id::new(("keymap_sec", label)), rect);
    let pal = style.palette;
    ui.painter().rect_filled(rect, 0.0, pal.elevated);
    let glyph = if collapsed { "\u{25B8}" } else { "\u{25BE}" };
    ui.painter().text(
        Pos2::new(rect.min.x + 8.0, rect.min.y + 5.0),
        &format!("{glyph}  {label}"),
        style.fonts.body,
        pal.text,
        None,
    );
    resp.clicked
}

fn keymap_group_header(ui: &mut Ui, label: &str) {
    let style = ui.style();
    let w = ui.available().width();
    let rect = ui.allocate(Vec2::new(w, style.fonts.small * 1.4 + 8.0));
    ui.painter().text_family(
        Pos2::new(rect.min.x + GUTTER_W, rect.min.y + 5.0),
        &label.to_uppercase(),
        style.fonts.small,
        style.palette.text_secondary,
        None,
        FontFamily::Mono,
    );
}

/// One action's row: name, "in Pass C" tag when it has no handler yet, its
/// chords in mono, a capture field while rebinding, and the inline conflict
/// prompt. Returns `true` if the keymap changed.
fn keymap_row(
    ui: &mut Ui,
    keymap: &mut Keymap,
    action: Action,
    capture: &mut Option<Action>,
    pending: &mut Option<(Action, Chord, String)>,
) -> bool {
    let style = ui.style();
    let pal = style.palette;
    let status = super::theme::Palette::invariant_status();
    let mut changed = false;

    let defaults = Keymap::from_preset(keymap.preset);
    let modified = keymap.chords_for(action) != defaults.chords_for(action);
    let capturing = *capture == Some(action);

    ui.horizontal(|ui| {
        let start = ui.cursor();
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
        // An action Pass C still owes reads as pending, not broken: dimmed
        // name plus an explicit tag, never hidden — a user who read the table
        // should find the key here and see why it does nothing.
        let live = action.status() == ActionStatus::Live;
        let name_color = if !live {
            pal.text_disabled
        } else if modified {
            pal.text
        } else {
            pal.text_secondary
        };
        ui.painter().text(
            Pos2::new(start.x + GUTTER_W, start.y + 6.0),
            action.name(),
            style.fonts.body,
            name_color,
            None,
        );
        if !live {
            let tag = if action.status() == ActionStatus::Fixed {
                "input model"
            } else {
                "in Pass C"
            };
            let x = start.x + GUTTER_W + LABEL_W - 84.0;
            ui.painter().text_family(
                Pos2::new(x, start.y + 7.0),
                tag,
                style.fonts.small,
                pal.text_disabled,
                None,
                FontFamily::Mono,
            );
        }

        ui.set_cursor(Pos2::new(start.x + GUTTER_W + LABEL_W, start.y + 3.0));
        if capturing {
            ui.painter().text(
                Pos2::new(start.x + GUTTER_W + LABEL_W, start.y + 6.0),
                "Press a key\u{2026}  (Esc cancels, Del unbinds)",
                style.fonts.body,
                status.overridden,
                None,
            );
        } else {
            let chords = keymap.chords_for(action);
            let mut x = start.x + GUTTER_W + LABEL_W;
            if chords.is_empty() {
                ui.painter().text_family(
                    Pos2::new(x, start.y + 6.0),
                    "\u{2014}",
                    style.fonts.body,
                    pal.text_disabled,
                    None,
                    FontFamily::Mono,
                );
            }
            for (i, c) in chords.iter().enumerate() {
                // The primary chord is the one menus and tooltips show, so it
                // reads at full strength; alternates sit back at 55%.
                let col = if i == 0 { pal.text } else { pal.text.with_alpha(0.55) };
                let label = c.label();
                let adv = ui.painter().text_family(
                    Pos2::new(x, start.y + 6.0),
                    &label,
                    style.fonts.body,
                    col,
                    None,
                    FontFamily::Mono,
                );
                x += adv.x + 10.0;
            }
            let hit = Rect::from_min_size(
                Pos2::new(start.x + GUTTER_W + LABEL_W - 4.0, start.y),
                Vec2::new(200.0, ROW_H),
            );
            if ui.interact(Id::new(("keymap_chord", action.id())), hit).clicked {
                *capture = Some(action);
            }
        }

        // Per-row reset, only where it would do something.
        if modified {
            let right = ui.available().max.x;
            ui.set_cursor(Pos2::new(right - 26.0, start.y + 3.0));
            if ui.button("R").clicked {
                keymap.reset(action);
                changed = true;
            }
        }
    });

    // ── Inline conflict prompt, directly under the row it belongs to.
    if let Some((a, chord, other)) = pending.clone() {
        if a == action {
            ui.horizontal(|ui| {
                let start = ui.cursor();
                ui.painter().text(
                    Pos2::new(start.x + GUTTER_W, start.y + 6.0),
                    &format!("{} is already {other}", chord.label()),
                    style.fonts.small,
                    status.warning,
                    None,
                );
                ui.set_cursor(Pos2::new(start.x + GUTTER_W + LABEL_W, start.y + 3.0));
                if ui.button("Rebind anyway").clicked {
                    keymap.set_chords(action, vec![chord]);
                    *pending = None;
                    changed = true;
                }
                if ui.button("Cancel").clicked {
                    *pending = None;
                }
            });
        }
    }

    // ── Capture. Runs after the row so the click that started it is spent.
    if capturing {
        let presses: Vec<_> = ui.ctx().input.key_presses.clone();
        for kp in presses {
            match kp.key {
                Key::Escape => {
                    *capture = None;
                }
                Key::Delete => {
                    keymap.set_chords(action, Vec::new());
                    *capture = None;
                    changed = true;
                }
                // A bare modifier is not a chord; keep waiting.
                _ => {
                    let chord = Chord::new(kp.key, kp.modifiers);
                    *capture = None;
                    // Ask before stealing: apply into a copy and see what the
                    // existing conflict check says, rather than committing and
                    // undoing.
                    let mut probe = keymap.clone();
                    probe.set_chords(action, vec![chord]);
                    match probe.conflicts().into_iter().find(|c| {
                        c.first == action || c.second == action
                    }) {
                        Some(c) => {
                            let other = if c.first == action { c.second } else { c.first };
                            *pending = Some((action, chord, other.name().to_string()));
                        }
                        None => {
                            *keymap = probe;
                            changed = true;
                        }
                    }
                }
            }
        }
    }
    changed
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

// ── Editor Preferences ▸ Graph helpers ───────────────────────────────────

/// Width of a segmented choice control in a settings row.
const SEG_W: f32 = 186.0;
/// Live preview strip, base units (scaled by the theme's ui_scale).
const PREVIEW_W: f32 = 300.0;
const PREVIEW_H: f32 = 120.0;

/// The strip's three demonstrations, as `(from, to, target pin row)` in
/// strip-local units. Pinned here (and asserted in tests) because each exists
/// to show a *specific* router behaviour, and a casual coordinate nudge could
/// silently turn it into an ordinary route:
///
/// - `[0]` a normal forward span (drawn exec, so both stroke weights show);
/// - `[1]`/`[2]` two wires into adjacent pins of one node from sources at
///   different distances — acceptance test 1, parallelism, made visible;
/// - `[3]` the residual stub (`|dx| < 24` and `|dy| < 20`), which draws
///   straight in every orthogonal mode. It leans slightly *backward* because
///   that is the only place the stub is reachable in Manhattan *and* Subway.
///   (The panels doc asked for "a span just under `min_dist`"; `min_dist` is
///   deleted, and this is the exception that actually exists.)
const PREVIEW_SAMPLES: [([f32; 2], [f32; 2], usize); 4] = [
    ([24.0, 14.0], [276.0, 40.0], 0),
    ([24.0, 50.0], [276.0, 74.0], 0),
    ([150.0, 60.0], [276.0, 96.0], 1),
    ([220.0, 110.0], [208.0, 114.0], 0),
];

/// A settings row that can be **disabled rather than hidden** (the Edit-menu
/// rule). A disabled row keeps its exact height and label column so the list
/// never reflows as the user changes wire style; it just greys, drops its
/// reset affordance, and swallows interaction.
#[allow(clippy::too_many_arguments)]
fn gated_row(
    ui: &mut Ui,
    filter: &Filter,
    label: &str,
    modified: bool,
    hint: Option<&str>,
    enabled: bool,
    control: impl FnOnce(&mut Ui, bool),
) -> bool {
    // A disabled row shows neither the modified dot nor the R reset: it is
    // not addressable right now, so offering to reset it would lie.
    let reset = setting_row(
        ui,
        filter,
        label,
        modified && enabled,
        hint.filter(|_| enabled),
        false,
        |ui| control(ui, enabled),
    );
    reset && enabled
}

/// The disabled stand-in for a numeric field — same 64x18 footprint as the
/// live `DragValue`, so nothing moves when a row greys out.
fn dead_value(ui: &mut Ui, text: &str) {
    let style = ui.style();
    let rect = ui.allocate(Vec2::new(64.0, FIELD_H));
    ui.painter()
        .rect_filled(rect, style.rounding.widget, style.palette.header);
    ui.painter().rect_stroke(
        rect,
        style.rounding.widget,
        style.metrics.border,
        style.palette.stroke.with_alpha(0.5),
    );
    ui.painter().text_family(
        Pos2::new(rect.min.x + 6.0, rect.center().y - 5.0),
        text,
        10.0,
        style.palette.text_disabled,
        None,
        FontFamily::Mono,
    );
}

/// Numeric row body: the live drag-value when enabled, the grey stand-in when
/// not. Mono value, `px`-style suffix — no new widgets, per the panels doc.
fn num_body(
    ui: &mut Ui,
    enabled: bool,
    value: &mut f32,
    speed: f64,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
) {
    if enabled {
        drag(ui, value, speed, range, suffix);
    } else {
        dead_value(ui, &format!("{}{suffix}", fmt_f(*value)));
    }
}

/// Segmented-choice row body over a small enum.
fn choice_body<T: PartialEq + Copy>(
    ui: &mut Ui,
    id: &str,
    enabled: bool,
    value: &mut T,
    options: &[T],
    labels: &[&str],
) {
    let rect = Rect::from_min_size(ui.cursor(), Vec2::new(SEG_W, FIELD_H + 2.0));
    let active = options.iter().position(|o| o == value).unwrap_or(0);
    if let Some(i) = segmented_control(ui, id, rect, labels, active, enabled) {
        *value = options[i];
    }
    ui.allocate(rect.size());
}

/// The live preview strip: three fixed sample wires drawn through the **real**
/// router, so it can never drift from what the canvas does.
///
/// (a) a normal forward span · (b) a pair from adjacent pins to targets at
/// different distances, which is acceptance test 1 (parallelism / anchoring)
/// made visible · (c) the residual stub case, `|dx| < 24` and `|dy| < 20`,
/// which is the documented straight-line exception made visible. (The panels
/// doc's third sample was "just under min_dist"; `min_dist` is deleted, and
/// the residual stub is the exception that actually exists.)
fn wire_preview(ui: &mut Ui, prefs: &super::graph_prefs::WirePrefs) {
    let style = ui.style();
    let s = (style.metrics.row_height / 22.0).max(0.1);
    let rect = ui.allocate(Vec2::new(PREVIEW_W * s, PREVIEW_H * s));
    ui.painter()
        .rect_filled(rect, style.rounding.widget, style.palette.input);
    ui.painter().rect_stroke(
        rect,
        style.rounding.widget,
        style.metrics.border,
        style.palette.stroke,
    );

    // Sample geometry in preview-local space, then offset into the strip.
    // Endpoints are pin positions; the rects are the notional nodes they sit
    // on, so the router sees the same shape of input the canvas gives it.
    let node = |x: f32, y: f32| {
        Rect::from_min_max(
            Pos2::new(x - 40.0 * s, y - 14.0 * s),
            Pos2::new(x + 40.0 * s, y + 14.0 * s),
        )
    };
    let p = |x: f32, y: f32| Pos2::new(rect.min.x + x * s, rect.min.y + y * s);

    let ty = [PinType::Exec, PinType::Float, PinType::Float, PinType::Float];
    let samples: Vec<(Pos2, Pos2, usize, PinType)> = PREVIEW_SAMPLES
        .iter()
        .zip(ty)
        .map(|((a, b, bi), t)| (p(a[0], a[1]), p(b[0], b[1]), *bi, t))
        .collect();
    let rects: Vec<Rect> = samples
        .iter()
        .flat_map(|(a, b, _, _)| [node(a.x, a.y), node(b.x, b.y)])
        .collect();

    let mut painter = ui.painter();
    for (a, b, bi, ty) in samples {
        let meta = RouteMeta {
            src_rect: Some(node(a.x, a.y)),
            dst_rect: Some(node(b.x, b.y)),
            target_pin_index: bi,
            node_rects: &rects,
        };
        let color = wire_color(None, &ty);
        let width = if ty == PinType::Exec { 2.4 } else { 1.9 };
        if prefs.style.is_orthogonal() {
            // The preview strip draws at 1:1, so its already-scaled radius
            // is the screen radius.
            let pts = router::round_corners(
                &router::route(a, b, prefs, &meta),
                prefs.corner_radius * s,
                1.0,
            );
            painter.polyline(&pts, width, color);
        } else {
            let (c1, c2) = router::spline_controls(a, b, prefs.curve);
            painter.bezier_cubic(a, c1, c2, b, width, color);
        }
        // Pin dots, so the strip reads as wires between pins.
        for (end, filled) in [(a, true), (b, false)] {
            if filled {
                painter.circle_filled(end, 3.0 * s, color);
            } else {
                painter.circle_stroke(end, 3.0 * s, 1.5, color);
            }
        }
    }
    ui.add_space(4.0);
}

// ── Editor Preferences ───────────────────────────────────────────────────

pub fn editor_prefs_window(ui: &mut Ui, state: &mut SettingsState, keymap: &mut Keymap) {
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
                p.theme_preset != d.theme_preset
                    || p.popover_translucent != d.popover_translucent
                    || p.ui_scale != d.ui_scale,
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
                p.graph != d.graph,
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
            let keys_cat = PREFS_CATS
                .iter()
                .position(|(n, _)| *n == "Keyboard Shortcuts")
                .unwrap_or(usize::MAX);
            // The shortcuts page owns ~50 generated rows rather than a fixed
            // handful, so it filters itself instead of going through
            // `setting_row`'s per-label match.
            let show_keys = filter.active() || selected == keys_cat;
            let mut keymap_changed = false;
            {
                let prefs = &mut state.prefs;
                let vsync = &mut state.vsync;
                let kb = &mut state.keybind_capture;
                let pending = &mut state.keybind_pending;
                let collapsed = &mut state.keybind_collapsed;
                let reset_confirm = &mut state.keymap_reset_confirm;
                ui.run_at(content, Direction::TopDown, Id::new("prefs_content"), opts, |ui| {
                    let h = ui.available_size().y;
                    ScrollArea::new(h)
                        .auto_shrink(false)
                        .inset(0.0)
                        .spacing(0.0)
                        .show(ui, |ui| {
                            draw_prefs_rows(ui, &filter, selected, prefs, vsync);
                            if show_keys {
                                keymap_changed |= draw_keymap_rows(
                                    ui,
                                    &filter,
                                    keymap,
                                    kb,
                                    pending,
                                    collapsed,
                                    reset_confirm,
                                );
                            }
                        });
                });
            }
            if keymap_changed {
                state.mark_keymap_dirty();
            }

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
                    // Rusty is brand-demo only — never offered here.
                    for t in ThemePreset::USER_SELECTABLE {
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
        // One master knob: every editor metric is a base value × ui_scale.
        // The density presets below are named values of this same field.
        let hint = format!("default {}", fmt_f(d.ui_scale));
        let m = p.ui_scale != d.ui_scale;
        if setting_row(ui, f, "UI scale", m, Some(&hint), false, |ui| {
            slider_drag(ui, &mut p.ui_scale, UI_SCALE_MIN..=UI_SCALE_MAX, "x");
        }) {
            p.ui_scale = d.ui_scale;
        }
        let m = Density::from_ui_scale(p.ui_scale) != Some(Density::default());
        if setting_row(ui, f, "Density", m, Some("default Comfortable"), false, |ui| {
            let picked = Density::from_ui_scale(p.ui_scale);
            let mut den = picked.unwrap_or_default();
            ComboBox::new("prefs_density")
                .selected_text(picked.map(|d| d.label()).unwrap_or("Custom"))
                .width(140.0)
                .show_ui(ui, |ui| {
                    for d2 in Density::ALL {
                        SelectableValue::new(&mut den, d2, d2.label()).show(ui);
                    }
                });
            if picked != Some(den) {
                p.ui_scale = den.ui_scale();
            }
        }) {
            p.ui_scale = Density::default().ui_scale();
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

    if vis(8) {
        draw_graph_rows(ui, f, p, &d);
    }
}

/// Editor Preferences > Graph. Five wire sections in the panels doc's order,
/// then the canvas prefs the doc calls their "natural later neighbours".
///
/// **Rows disable by style, never hide.** With Spline active, everything
/// below Wire style in Wires - plus the whole Execution wires and Bundling
/// sections - greys to the standard disabled state; Crossings and Flow
/// bubbles stay live, because a hop symbol is span-agnostic and reads on a
/// curve just as well as on a polyline. The list never reflows as the style
/// changes: a disabled row keeps its height, its label column and its
/// control footprint.
fn draw_graph_rows(ui: &mut Ui, f: &Filter, p: &mut EditorPrefs, d: &EditorPrefs) {
    let dw = d.graph.wires;
    // Orthogonal-only rows. Spline has no turns, no lanes and no bundles.
    let ortho = p.graph.wires.style.is_orthogonal();

    // -- Wires ------------------------------------------------------------
    section_bar(ui, "Wires");
    if f.matches("Wire style") {
        wire_preview(ui, &p.graph.wires);
    }

    let m = p.graph.wires.style != dw.style;
    let hint = format!("default {}", dw.style.label());
    if setting_row(ui, f, "Wire style", m, Some(&hint), false, |ui| {
        choice_body(
            ui,
            "gp_style",
            true,
            &mut p.graph.wires.style,
            &WireStyle::ALL,
            &["Spline", "Manhattan", "Subway"],
        );
    }) {
        p.graph.wires.style = dw.style;
    }

    let m = p.graph.wires.horizontal_offset != dw.horizontal_offset;
    let hint = format!("default {}px", fmt_f(dw.horizontal_offset));
    if gated_row(ui, f, "Horizontal offset", m, Some(&hint), ortho, |ui, on| {
        num_body(ui, on, &mut p.graph.wires.horizontal_offset, 0.25, 0.0..=64.0, "px");
    }) {
        p.graph.wires.horizontal_offset = dw.horizontal_offset;
    }

    let m = p.graph.wires.turn_anchor != dw.turn_anchor;
    let hint = format!("default {}", dw.turn_anchor.label());
    if gated_row(ui, f, "Turn anchor", m, Some(&hint), ortho, |ui, on| {
        choice_body(
            ui,
            "gp_anchor",
            on,
            &mut p.graph.wires.turn_anchor,
            &TurnAnchor::ALL,
            &["Target", "Source"],
        );
    }) {
        p.graph.wires.turn_anchor = dw.turn_anchor;
    }

    let m = p.graph.wires.corner_radius != dw.corner_radius;
    let hint = format!("default {}px", fmt_f(dw.corner_radius));
    if gated_row(ui, f, "Corner radius", m, Some(&hint), ortho, |ui, on| {
        num_body(ui, on, &mut p.graph.wires.corner_radius, 0.25, 0.0..=32.0, "px");
    }) {
        p.graph.wires.corner_radius = dw.corner_radius;
    }

    let m = p.graph.wires.priority != dw.priority;
    let hint = format!("default {}", dw.priority.label());
    if gated_row(ui, f, "Turn priority", m, Some(&hint), ortho, |ui, on| {
        choice_body(
            ui,
            "gp_priority",
            on,
            &mut p.graph.wires.priority,
            &TurnPriority::ALL,
            &["None", "Node", "Pin"],
        );
    }) {
        p.graph.wires.priority = dw.priority;
    }

    let m = p.graph.wires.disable_pin_offset != dw.disable_pin_offset;
    if gated_row(ui, f, "Disable pin offset", m, Some("default off"), ortho, |ui, on| {
        Checkbox::new(&mut p.graph.wires.disable_pin_offset, "")
            .enabled(on)
            .show(ui);
    }) {
        p.graph.wires.disable_pin_offset = dw.disable_pin_offset;
    }

    // -- Execution wires --------------------------------------------------
    section_bar(ui, "Execution wires");
    let m = p.graph.wires.exec_overwrite.is_some() != dw.exec_overwrite.is_some();
    let mut override_on = p.graph.wires.exec_overwrite.is_some();
    if gated_row(
        ui,
        f,
        "Override style for exec wires",
        m,
        Some("default off"),
        ortho,
        |ui, on| {
            if Checkbox::new(&mut override_on, "").enabled(on).show(ui).clicked && on {
                p.graph.wires.exec_overwrite = override_on.then(ExecWirePrefs::default);
            }
        },
    ) {
        p.graph.wires.exec_overwrite = dw.exec_overwrite;
    }
    // The rule nests: these disable while Override is off (and, above that,
    // while the base style is Spline).
    let exec_on = ortho && p.graph.wires.exec_overwrite.is_some();
    let mut exec = p.graph.wires.exec_overwrite.unwrap_or_default();
    let de = ExecWirePrefs::default();

    let m = exec.style != de.style;
    let hint = format!("default {}", de.style.label());
    if gated_row(ui, f, "Wire style", m, Some(&hint), exec_on, |ui, on| {
        choice_body(
            ui,
            "gp_exec_style",
            on,
            &mut exec.style,
            &WireStyle::ALL,
            &["Spline", "Manhattan", "Subway"],
        );
    }) {
        exec.style = de.style;
    }

    let m = exec.turn_anchor != de.turn_anchor;
    let hint = format!("default {}", de.turn_anchor.label());
    if gated_row(ui, f, "Turn anchor", m, Some(&hint), exec_on, |ui, on| {
        choice_body(
            ui,
            "gp_exec_anchor",
            on,
            &mut exec.turn_anchor,
            &TurnAnchor::ALL,
            &["Target", "Source"],
        );
    }) {
        exec.turn_anchor = de.turn_anchor;
    }

    let m = exec.priority != de.priority;
    let hint = format!("default {}", de.priority.label());
    if gated_row(ui, f, "Turn priority", m, Some(&hint), exec_on, |ui, on| {
        choice_body(
            ui,
            "gp_exec_priority",
            on,
            &mut exec.priority,
            &TurnPriority::ALL,
            &["None", "Node", "Pin"],
        );
    }) {
        exec.priority = de.priority;
    }
    // Only write back while the override is live, so editing the sub-rows can
    // never resurrect a disabled override.
    if exec_on && p.graph.wires.exec_overwrite != Some(exec) {
        p.graph.wires.exec_overwrite = Some(exec);
    }

    // -- Bundling ---------------------------------------------------------
    section_bar(ui, "Bundling");
    let m = p.graph.wires.bundle_enabled != dw.bundle_enabled;
    if gated_row(ui, f, "Bundle overlapping wires", m, Some("default on"), ortho, |ui, on| {
        Checkbox::new(&mut p.graph.wires.bundle_enabled, "")
            .enabled(on)
            .show(ui);
    }) {
        p.graph.wires.bundle_enabled = dw.bundle_enabled;
    }
    // Same nesting rule again: the parameters follow their own toggle.
    let bundle_on = ortho && p.graph.wires.bundle_enabled;

    let m = p.graph.wires.bundle_offset != dw.bundle_offset;
    let hint = format!("default {}px", fmt_f(dw.bundle_offset));
    if gated_row(ui, f, "Bundle offset", m, Some(&hint), bundle_on, |ui, on| {
        num_body(ui, on, &mut p.graph.wires.bundle_offset, 0.1, 0.0..=16.0, "px");
    }) {
        p.graph.wires.bundle_offset = dw.bundle_offset;
    }

    let m = p.graph.wires.bundle_merge_offset != dw.bundle_merge_offset;
    let hint = format!("default {}px", fmt_f(dw.bundle_merge_offset));
    if gated_row(ui, f, "Merge offset", m, Some(&hint), bundle_on, |ui, on| {
        num_body(ui, on, &mut p.graph.wires.bundle_merge_offset, 0.25, 0.0..=64.0, "px");
    }) {
        p.graph.wires.bundle_merge_offset = dw.bundle_merge_offset;
    }

    let m = p.graph.wires.bundle_push_outside != dw.bundle_push_outside;
    if gated_row(ui, f, "Push outside", m, Some("default off"), bundle_on, |ui, on| {
        Checkbox::new(&mut p.graph.wires.bundle_push_outside, "")
            .enabled(on)
            .show(ui);
    }) {
        p.graph.wires.bundle_push_outside = dw.bundle_push_outside;
    }

    let m = p.graph.wires.bundle_max != dw.bundle_max;
    let hint = format!("default {}", dw.bundle_max);
    if gated_row(ui, f, "Max wires per lane", m, Some(&hint), bundle_on, |ui, on| {
        let mut v = p.graph.wires.bundle_max as f32;
        num_body(ui, on, &mut v, 0.1, 1.0..=32.0, "");
        p.graph.wires.bundle_max = v.round().max(1.0) as u32;
    }) {
        p.graph.wires.bundle_max = dw.bundle_max;
    }

    // -- Crossings --------------------------------------------------------
    // Stays live on Spline: the hop symbol is span-agnostic and reads on a
    // curve just as well as on a polyline.
    section_bar(ui, "Crossings");
    let m = p.graph.wires.crossing != dw.crossing;
    let hint = format!("default {}", dw.crossing.label());
    if setting_row(ui, f, "Crossing style", m, Some(&hint), false, |ui| {
        choice_body(
            ui,
            "gp_crossing",
            true,
            &mut p.graph.wires.crossing,
            &CrossingStyle::ALL,
            &["None", "Gap", "Arc", "Circle"],
        );
    }) {
        p.graph.wires.crossing = dw.crossing;
    }

    let cross_on = p.graph.wires.crossing != CrossingStyle::None;
    let m = p.graph.wires.crossing_size != dw.crossing_size;
    let hint = format!("default {}px", fmt_f(dw.crossing_size));
    if gated_row(ui, f, "Crossing size", m, Some(&hint), cross_on, |ui, on| {
        num_body(ui, on, &mut p.graph.wires.crossing_size, 0.1, 1.0..=16.0, "px");
    }) {
        p.graph.wires.crossing_size = dw.crossing_size;
    }

    // -- Flow bubbles -----------------------------------------------------
    section_bar(ui, "Flow bubbles");
    let db = dw.bubbles;
    let m = p.graph.wires.bubbles.enabled != db.enabled;
    if setting_row(ui, f, "Show flow bubbles", m, Some("default on"), false, |ui| {
        Checkbox::new(&mut p.graph.wires.bubbles.enabled, "").show(ui);
    }) {
        p.graph.wires.bubbles.enabled = db.enabled;
    }
    let bub = p.graph.wires.bubbles.enabled;

    let m = p.graph.wires.bubbles.exec_only != db.exec_only;
    if gated_row(ui, f, "Exec wires only", m, Some("default on"), bub, |ui, on| {
        Checkbox::new(&mut p.graph.wires.bubbles.exec_only, "")
            .enabled(on)
            .show(ui);
    }) {
        p.graph.wires.bubbles.exec_only = db.exec_only;
    }

    let m = p.graph.wires.bubbles.selected_only != db.selected_only;
    if gated_row(ui, f, "Selected nodes only", m, Some("default off"), bub, |ui, on| {
        Checkbox::new(&mut p.graph.wires.bubbles.selected_only, "")
            .enabled(on)
            .show(ui);
    }) {
        p.graph.wires.bubbles.selected_only = db.selected_only;
    }

    let m = p.graph.wires.bubbles.size != db.size;
    let hint = format!("default {}px", fmt_f(db.size));
    if gated_row(ui, f, "Size", m, Some(&hint), bub, |ui, on| {
        num_body(ui, on, &mut p.graph.wires.bubbles.size, 0.1, 1.0..=12.0, "px");
    }) {
        p.graph.wires.bubbles.size = db.size;
    }

    let m = p.graph.wires.bubbles.speed != db.speed;
    let hint = format!("default {}", fmt_f(db.speed));
    if gated_row(ui, f, "Speed", m, Some(&hint), bub, |ui, on| {
        num_body(ui, on, &mut p.graph.wires.bubbles.speed, 1.0, 10.0..=600.0, "px/s");
    }) {
        p.graph.wires.bubbles.speed = db.speed;
    }

    let m = p.graph.wires.bubbles.spacing != db.spacing;
    let hint = format!("default {}px", fmt_f(db.spacing));
    if gated_row(ui, f, "Spacing", m, Some(&hint), bub, |ui, on| {
        num_body(ui, on, &mut p.graph.wires.bubbles.spacing, 0.5, 8.0..=200.0, "px");
    }) {
        p.graph.wires.bubbles.spacing = db.spacing;
    }

    // -- Canvas -----------------------------------------------------------
    // The wire sections come first by touch frequency; the canvas prefs are
    // the "natural later neighbours" the panels doc anticipates.
    section_bar(ui, "Canvas");
    let hint = format!("default {}", fmt_f(d.graph.zoom_min));
    let m = p.graph.zoom_min != d.graph.zoom_min;
    // The two rows do different jobs now: min floors both wheel paths, max is
    // Ctrl+wheel's ceiling (a plain wheel stops at the ladder's 220%).
    if setting_row(ui, f, "Min zoom", m, Some(&hint), false, |ui| {
        drag(ui, &mut p.graph.zoom_min, 0.01, 0.05..=1.0, "\u{00d7}");
    }) {
        p.graph.zoom_min = d.graph.zoom_min;
    }
    let hint = format!("default {}", fmt_f(d.graph.zoom_max));
    let m = p.graph.zoom_max != d.graph.zoom_max;
    if setting_row(ui, f, "Max zoom (Ctrl+wheel)", m, Some(&hint), false, |ui| {
        drag(ui, &mut p.graph.zoom_max, 0.1, 2.2..=8.0, "\u{00d7}");
    }) {
        p.graph.zoom_max = d.graph.zoom_max;
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

/// `plugin_pages` are the live plugin-contributed settings pages (39.8 D6).
/// They are appended after the built-in categories, so a page appears and
/// disappears exactly with its plugin's enablement — no separate bookkeeping.
pub fn project_settings_window(
    ui: &mut Ui,
    state: &mut SettingsState,
    plugin_pages: &mut [crate::engine::plugins::PluginSettingsEntry],
    plugin_model: &super::plugin_manager::PluginManagerModel,
) {
    if !state.project_open {
        return;
    }
    let mut open = true;

    Window::new("Project Settings")
        .modal(true)
        .resizable(false)
        .collapsible(false)
        .anchor_center(true)
        // Wide enough for the Plugin Manager's list + detail panes (the
        // mockup's shell is 1059px); the row-based pages just get roomier.
        .default_size(Vec2::new(1060.0, 620.0))
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

            // Built-in categories plus one per plugin page. `label_store`
            // backs the borrowed label slices for the length of the call.
            let label_store: Vec<[&str; 1]> =
                plugin_pages.iter().map(|p| [p.title.as_str()]).collect();
            let mut cats: Vec<(&str, &[&str])> = PROJECT_CATS.to_vec();
            for (page, labels) in plugin_pages.iter().zip(label_store.iter()) {
                cats.push((page.title.as_str(), labels.as_slice()));
            }
            let mut cat_mod = cat_mod.to_vec();
            cat_mod.resize(cats.len(), false);

            let (content, footer, filter) = shell_chrome(
                ui,
                "project",
                "project",
                PROJECT_FILE,
                &mut state.project_search,
                &cats,
                &mut state.project_cat,
                &cat_mod,
            );

            let opts = UiOptions {
                padding: Vec2::new(12.0, 8.0),
                spacing: 0.0,
            };
            let selected = state.project_cat;
            let saved = state.project_saved.clone();

            // The Plugin Manager owns the whole content pane: it is a
            // two-pane list/detail surface, not a stack of setting rows, so
            // it takes `content` directly instead of going through the
            // ScrollArea + rows path below.
            if selected == PLUGINS_CAT && !filter.active() {
                super::plugin_manager::plugin_manager_page(
                    ui,
                    content,
                    &mut state.plugins,
                    plugin_model,
                    &mut state.project,
                );
                let fopts = UiOptions {
                    padding: Vec2::new(10.0, 4.0),
                    spacing: 8.0,
                };
                ui.run_at(footer, Direction::LeftToRight, Id::new("project_footer"), fopts, |ui| {
                    draw_project_footer(ui, footer, state);
                });
                return;
            }

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
                            // A plugin page owns everything below the built-in
                            // categories; it draws only when its own row is the
                            // selected one (or search is showing everything).
                            for (i, page) in plugin_pages.iter_mut().enumerate() {
                                let idx = PROJECT_CATS.len() + i;
                                if !filter.cat_visible(idx, selected, &[page.title.as_str()]) {
                                    continue;
                                }
                                section_bar(ui, &page.title.clone());
                                page.page
                                    .draw(ui, &mut crate::engine::plugins::PluginSettingsCtx {
                                        project,
                                    });
                            }
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
#[cfg(test)]
mod tests {
    use super::*;
    use crusty_gui::math::Pos2;
    use super::super::graph_prefs::WirePrefs;

    fn route_sample(i: usize, style: WireStyle) -> Vec<Pos2> {
        let prefs = WirePrefs { style, ..Default::default() };
        let (a, b, bi) = PREVIEW_SAMPLES[i];
        let rects = [];
        router::route(
            Pos2::new(a[0], a[1]),
            Pos2::new(b[0], b[1]),
            &prefs,
            &RouteMeta {
                src_rect: None,
                dst_rect: None,
                target_pin_index: bi,
                node_rects: &rects,
            },
        )
    }

    /// The preview strip must demonstrate what it claims to. Each sample is
    /// checked against the *real* router, so a coordinate nudge that turned a
    /// sample into an ordinary route fails here rather than quietly making
    /// the strip lie about the router.
    #[test]
    fn preview_samples_demonstrate_their_documented_cases() {
        for style in [WireStyle::Manhattan, WireStyle::Subway] {
            // [0..2] are real routes with turns, not the near-horizontal
            // shortcut (which would collapse them to a 2-point line).
            for i in 0..3 {
                let pts = route_sample(i, style);
                assert!(
                    pts.len() > 2,
                    "{style:?}: preview sample {i} collapsed to a straight line"
                );
            }
            // [3] is the residual stub: straight, in both orthogonal modes.
            assert_eq!(
                route_sample(3, style).len(),
                2,
                "{style:?}: preview sample 3 is no longer the residual stub"
            );
        }

        let turn = |p: &[Pos2]| p[p.len() - 2].x;
        // [1] and [2] arrive at adjacent pins of one node from sources at
        // different distances. Subway anchors both turns at the same x...
        let s1 = route_sample(1, WireStyle::Subway);
        let s2 = route_sample(2, WireStyle::Subway);
        assert!(
            (turn(&s1) - turn(&s2)).abs() < 1e-3,
            "the parallelism sample stopped being parallel"
        );
        // ...and Manhattan staggers them by a bundle_offset, which is the
        // other half of what the pair is there to show.
        let m1 = route_sample(1, WireStyle::Manhattan);
        let m2 = route_sample(2, WireStyle::Manhattan);
        let d = WirePrefs::default();
        assert!(
            (turn(&m1) - turn(&m2)).abs() >= d.bundle_offset - 1e-3,
            "the Manhattan stagger is no longer visible in the preview"
        );
    }

    /// Every sample stays inside the strip, so nothing draws outside the box.
    #[test]
    fn preview_samples_fit_the_strip() {
        for (i, (a, b, _)) in PREVIEW_SAMPLES.iter().enumerate() {
            for q in [a, b] {
                assert!(
                    q[0] >= 0.0 && q[0] <= PREVIEW_W && q[1] >= 0.0 && q[1] <= PREVIEW_H,
                    "preview sample {i} endpoint {q:?} escapes the strip"
                );
            }
        }
    }

    /// Every row the Graph category draws is reachable by search, and the
    /// deleted min-dist rows have not crept back in.
    #[test]
    fn graph_category_search_index_is_complete() {
        let rows = PREFS_CATS
            .iter()
            .find(|(name, _)| *name == "Graph")
            .expect("Graph category")
            .1;
        for label in [
            "Wire style",
            "Horizontal offset",
            "Turn anchor",
            "Corner radius",
            "Turn priority",
            "Disable pin offset",
            "Override style for exec wires",
            "Bundle overlapping wires",
            "Bundle offset",
            "Merge offset",
            "Push outside",
            "Max wires per lane",
            "Crossing style",
            "Crossing size",
            "Show flow bubbles",
            "Exec wires only",
            "Selected nodes only",
            "Size",
            "Speed",
            "Spacing",
            "Min zoom",
            "Max zoom (Ctrl+wheel)",
        ] {
            assert!(rows.contains(&label), "'{label}' is missing from the search index");
        }
        assert!(!rows.iter().any(|r| r.to_lowercase().contains("minimum distance")));
        assert!(!rows.iter().any(|r| r.to_lowercase().contains("below minimum")));
    }
}
