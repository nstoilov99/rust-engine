//! Plugin Manager (Task 39.8 P6, D8) — a Project Settings page.
//!
//! Visual source of truth: `docs/mockup/Crusty Plugin Manager.dc.html`,
//! mapped onto theme tokens. The mockup was drawn tier-2-flavoured; D8's five
//! v1 deltas apply:
//!
//! 1. Manifest facts are module path + Cargo feature + kind + origin. Tier 1
//!    has no `plugin.ron` and no DLL entry point, so those rows would be
//!    fiction.
//! 2. There is no "built for engine X" blocked state — impossible when every
//!    plugin is compiled from this tree. Its slot is taken by
//!    **not in this build**, the manifest orphan the mockup lacks.
//! 3. Engine/Project group headers and a kind chip, per the engine survey.
//! 4. It lives in **Project Settings**: the plugin set is `project.ron`,
//!    VCS-checked-in, Ctrl+S dirty semantics — not user preferences.
//! 5. `Ctrl+F`, no Mac glyphs.
//!
//! The model ([`PluginManagerModel`]) is an owned snapshot built from
//! `PluginSet` + `ProjectConfig`. That keeps every state decision testable
//! without a UI, and sidesteps borrowing the plugin set while the settings
//! window holds its plugin *pages* mutably.

use crusty_gui::context::Ui;
use crusty_gui::id::Id;
use crusty_gui::math::{Pos2, Rect, Vec2};
use crusty_gui::text::FontFamily;
use crusty_gui::widgets::{ScrollArea, TextEdit};

use super::project_config::ProjectConfig;
use super::widgets::segmented_control;
use crate::engine::plugins::{
    PluginCounts, PluginEntry, PluginKind, PluginOrigin, PluginSet, RegistrationPhase,
};

const LIST_W: f32 = 406.0;
const ROW_H: f32 = 44.0;
const GROUP_H: f32 = 24.0;
const FILTER_H: f32 = 40.0;
const BANNER_H: f32 = 32.0;
const FOOTER_H: f32 = 24.0;

// ── Model ────────────────────────────────────────────────────────────────

/// Which rows the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ManagerFilter {
    #[default]
    All,
    Enabled,
    Disabled,
    Problems,
}

impl ManagerFilter {
    const ALL: [ManagerFilter; 4] = [
        ManagerFilter::All,
        ManagerFilter::Enabled,
        ManagerFilter::Disabled,
        ManagerFilter::Problems,
    ];
    fn label(self) -> &'static str {
        match self {
            ManagerFilter::All => "All",
            ManagerFilter::Enabled => "Enabled",
            ManagerFilter::Disabled => "Disabled",
            ManagerFilter::Problems => "Problems",
        }
    }
}

/// One row's condition. The D8 state contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowState {
    /// Built and running.
    Enabled,
    /// Built, but something collided — a duplicate node id, say.
    EnabledWithWarnings,
    /// Switched off in the manifest, and not running.
    Disabled,
    /// Manifest says on, runtime says off — takes effect on relaunch.
    PendingEnable,
    /// Manifest says off, still running until relaunch.
    PendingDisable,
    /// `build()` (or its commit) failed.
    Failed {
        phase: RegistrationPhase,
        message: String,
    },
    /// A dependency was absent, disabled, or itself failed.
    BlockedMissingDependency { missing: String },
    /// Named in `project.ron` but compiled into no build we have.
    NotInThisBuild,
}

impl RowState {
    /// Does this row belong under the Problems filter?
    pub fn is_problem(&self) -> bool {
        matches!(
            self,
            RowState::EnabledWithWarnings
                | RowState::Failed { .. }
                | RowState::BlockedMissingDependency { .. }
                | RowState::NotInThisBuild
        )
    }

    /// Is it running right now?
    pub fn is_live(&self) -> bool {
        matches!(
            self,
            RowState::Enabled | RowState::EnabledWithWarnings | RowState::PendingDisable
        )
    }

    /// What the manifest currently says (drives the toggle knob).
    pub fn manifest_on(&self) -> bool {
        matches!(
            self,
            RowState::Enabled
                | RowState::EnabledWithWarnings
                | RowState::PendingEnable
                | RowState::Failed { .. }
                | RowState::BlockedMissingDependency { .. }
        )
    }

    /// A blocked or absent plugin cannot be toggled into working.
    pub fn toggle_locked(&self) -> bool {
        matches!(
            self,
            RowState::BlockedMissingDependency { .. } | RowState::NotInThisBuild
        )
    }

    pub fn pending(&self) -> bool {
        matches!(self, RowState::PendingEnable | RowState::PendingDisable)
    }
}

/// A manager row: everything the list and the detail pane need.
#[derive(Debug, Clone)]
pub struct PluginRow {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub origin: PluginOrigin,
    pub kind: PluginKind,
    pub module_path: String,
    pub cargo_feature: Option<String>,
    pub depends_on: Vec<String>,
    pub state: RowState,
    pub counts: Option<PluginCounts>,
    pub warnings: Vec<String>,
}

/// An owned snapshot of the plugin world, ready to draw or assert on.
#[derive(Debug, Clone, Default)]
pub struct PluginManagerModel {
    pub rows: Vec<PluginRow>,
    /// Per-plugin dependency closure, precomputed at build time.
    ///
    /// Owned rather than looked up live so the page needs no borrow of the
    /// plugin set — the settings window is already holding its plugin *pages*
    /// mutably at that point.
    cascades: std::collections::BTreeMap<String, CascadePrompt>,
}

impl PluginManagerModel {
    /// Build from the live plugin set and the *edited* project config.
    ///
    /// Pending state is exactly the disagreement between those two: the
    /// manifest is what the user has typed, `PluginSet` is what is running.
    pub fn build(set: &PluginSet, project: &ProjectConfig) -> Self {
        let manifest_says = |id: &str| -> bool {
            project
                .plugins
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.enabled)
                .unwrap_or(true) // absent = enabled (batteries included)
        };

        let mut rows: Vec<PluginRow> = Vec::new();

        for manifest in set.manifests() {
            // The game's own module is never listed (D8): no engine presents
            // the project's code as a toggleable plugin.
            if manifest.internal {
                continue;
            }
            let record = set.records().iter().find(|r| r.manifest.id == manifest.id);
            let failure = set.failures().iter().find(|f| f.id == manifest.id);
            let wants_on = manifest_says(&manifest.id);
            let live = record.is_some();

            let state = match (failure, record) {
                (Some(f), _) => match &f.error {
                    crate::engine::plugins::PluginError::MissingDependency { missing, .. } => {
                        RowState::BlockedMissingDependency {
                            missing: missing.clone(),
                        }
                    }
                    crate::engine::plugins::PluginError::DependencyCycle { members } => {
                        RowState::Failed {
                            phase: f.phase,
                            message: format!("dependency cycle: {}", members.join(" -> ")),
                        }
                    }
                    other => RowState::Failed {
                        phase: f.phase,
                        message: other.to_string(),
                    },
                },
                (None, Some(rec)) if !rec.warnings.is_empty() && wants_on => {
                    RowState::EnabledWithWarnings
                }
                (None, Some(_)) if wants_on => RowState::Enabled,
                // Running, but the manifest has since been switched off.
                (None, Some(_)) => RowState::PendingDisable,
                // Not running, but the manifest asks for it.
                (None, None) if wants_on => RowState::PendingEnable,
                (None, None) => RowState::Disabled,
            };
            let _ = live;

            rows.push(PluginRow {
                id: manifest.id.clone(),
                name: manifest.name.clone(),
                version: manifest.version.clone(),
                author: manifest.author.clone(),
                description: manifest.description.clone(),
                origin: manifest.origin,
                kind: manifest.kind,
                module_path: manifest.module_path.clone(),
                cargo_feature: manifest.cargo_feature.clone(),
                depends_on: manifest.depends_on.clone(),
                state,
                counts: record.map(|r| r.counts),
                warnings: record.map(|r| r.warnings.clone()).unwrap_or_default(),
            });
        }

        // Manifest entries naming nothing in this build. Not an error — a
        // project moved between builds must not lose the user's intent.
        for id in set.orphans() {
            rows.push(PluginRow {
                id: id.clone(),
                name: id.clone(),
                version: String::new(),
                author: String::new(),
                description: "This plugin is named in project.ron but is not compiled into \
                              this build. Its setting is preserved."
                    .to_string(),
                origin: PluginOrigin::Project,
                kind: PluginKind::Runtime,
                module_path: String::new(),
                cargo_feature: None,
                depends_on: Vec::new(),
                state: RowState::NotInThisBuild,
                counts: None,
                warnings: Vec::new(),
            });
        }

        // Engine plugins first, then Project, each alphabetical — the group
        // order the manager renders.
        rows.sort_by(|a, b| {
            let key = |r: &PluginRow| match r.origin {
                PluginOrigin::Engine => 0,
                PluginOrigin::Project => 1,
            };
            key(a).cmp(&key(b)).then(a.name.cmp(&b.name))
        });

        let cascades = rows
            .iter()
            .map(|r| (r.id.clone(), dependents_of(set, &r.id)))
            .collect();

        Self { rows, cascades }
    }

    /// What disabling `id` would take down with it (§5.7).
    pub fn cascade_for(&self, id: &str) -> CascadePrompt {
        self.cascades.get(id).cloned().unwrap_or_else(|| CascadePrompt {
            id: id.to_string(),
            name: id.to_string(),
            dependents: Vec::new(),
            hits_internal: false,
        })
    }

    pub fn count(&self, filter: ManagerFilter) -> usize {
        self.rows.iter().filter(|r| self.passes(r, filter)).count()
    }

    fn passes(&self, row: &PluginRow, filter: ManagerFilter) -> bool {
        match filter {
            ManagerFilter::All => true,
            ManagerFilter::Enabled => row.state.manifest_on() && !row.state.toggle_locked(),
            ManagerFilter::Disabled => !row.state.manifest_on() || row.state.toggle_locked(),
            ManagerFilter::Problems => row.state.is_problem(),
        }
    }

    /// Rows matching the current filter and search text.
    pub fn visible(&self, filter: ManagerFilter, search: &str) -> Vec<&PluginRow> {
        let q = search.trim().to_lowercase();
        self.rows
            .iter()
            .filter(|r| self.passes(r, filter))
            .filter(|r| {
                q.is_empty()
                    || format!("{} {} {}", r.name, r.author, r.description)
                        .to_lowercase()
                        .contains(&q)
            })
            .collect()
    }

    pub fn pending_count(&self) -> usize {
        self.rows.iter().filter(|r| r.state.pending()).count()
    }

    pub fn live_count(&self) -> usize {
        self.rows.iter().filter(|r| r.state.is_live()).count()
    }

    pub fn get(&self, id: &str) -> Option<&PluginRow> {
        self.rows.iter().find(|r| r.id == id)
    }
}

/// The warning shown before disabling something other plugins need (§5.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CascadePrompt {
    pub id: String,
    pub name: String,
    /// Manager-visible plugins that would go down with it.
    pub dependents: Vec<String>,
    /// An internal dependent (the project's own module) is in the closure.
    /// It has no row to point at, so it gets named in prose instead.
    pub hits_internal: bool,
}

impl CascadePrompt {
    /// True when something would actually go down with it.
    pub fn is_empty(&self) -> bool {
        self.dependents.is_empty() && !self.hits_internal
    }

    pub fn message(&self) -> String {
        if self.is_empty() {
            return format!("Nothing else depends on {}.", self.name);
        }
        let mut parts: Vec<String> = Vec::new();
        if !self.dependents.is_empty() {
            parts.push(format!("will also disable {}", self.dependents.join(", ")));
        }
        if self.hits_internal {
            parts.push("will disable the project's gameplay systems".to_string());
        }
        format!("Disabling {} {}.", self.name, parts.join(", and "))
    }
}

/// Who depends on `id`, across the whole compiled-in set (internal included).
pub fn dependents_of(set: &PluginSet, id: &str) -> CascadePrompt {
    let mut dependents = Vec::new();
    let mut hits_internal = false;
    let manifests = set.manifests();
    for m in &manifests {
        if !m.depends_on.iter().any(|d| d == id) {
            continue;
        }
        if m.internal {
            hits_internal = true;
        } else {
            dependents.push(m.name.clone());
        }
    }
    let name = manifests
        .iter()
        .find(|m| m.id == id)
        .map(|m| m.name.clone())
        .unwrap_or_else(|| id.to_string());
    CascadePrompt {
        id: id.to_string(),
        name,
        dependents,
        hits_internal,
    }
}

/// Set `id`'s entry in the manifest, materializing it if absent. Orphan
/// entries elsewhere in the list are untouched.
pub fn set_manifest_enabled(project: &mut ProjectConfig, id: &str, enabled: bool) {
    if let Some(entry) = project.plugins.iter_mut().find(|e| e.id == id) {
        entry.enabled = enabled;
    } else {
        project.plugins.push(PluginEntry::new(id, enabled));
    }
}

// ── UI state ─────────────────────────────────────────────────────────────

/// Per-session manager UI state, owned by `SettingsState`.
#[derive(Debug, Default)]
pub struct PluginManagerState {
    pub search: String,
    pub filter: ManagerFilter,
    pub selected: Option<String>,
    /// Raised when the user clicks Relaunch; the app consumes it.
    pub relaunch_requested: bool,
    /// A disable waiting on cascade confirmation.
    pub pending_cascade: Option<CascadePrompt>,
}

// ── Drawing ──────────────────────────────────────────────────────────────

/// Glyph + color for a row/detail state, mirroring the mockup's ladder.
fn state_glyph(state: &RowState, ui: &Ui) -> (&'static str, crusty_gui::math::Color, f32) {
    let st = super::theme::Palette::invariant_status();
    let pal = ui.style().palette;
    match state {
        RowState::Failed { .. } => ("\u{2715}", st.error, 11.0),
        RowState::BlockedMissingDependency { .. } => ("\u{2298}", st.warning, 12.0),
        RowState::NotInThisBuild => ("\u{2298}", pal.text_disabled, 12.0),
        RowState::EnabledWithWarnings => ("\u{25B2}", st.warning, 9.0),
        RowState::PendingEnable | RowState::PendingDisable => ("\u{25CF}", st.warning, 8.0),
        RowState::Enabled => ("\u{25CF}", st.success, 8.0),
        RowState::Disabled => ("\u{25CB}", pal.text_disabled, 9.0),
    }
}

fn state_text(row: &PluginRow) -> String {
    match &row.state {
        RowState::Failed { phase, .. } => format!("Failed to load \u{2014} during {}", phase.label()),
        RowState::BlockedMissingDependency { missing } => {
            format!("Blocked \u{2014} requires {missing}, which is not available")
        }
        RowState::NotInThisBuild => "Not in this build \u{2014} setting preserved".to_string(),
        RowState::EnabledWithWarnings => format!(
            "Enabled \u{2014} loaded with {} warning{}",
            row.warnings.len(),
            if row.warnings.len() == 1 { "" } else { "s" }
        ),
        RowState::PendingEnable => "Enabled \u{2014} loads on relaunch".to_string(),
        RowState::PendingDisable => "Disabled \u{2014} unloads on relaunch".to_string(),
        RowState::Enabled => "Enabled \u{2014} loaded".to_string(),
        RowState::Disabled => "Disabled".to_string(),
    }
}

/// Chips shown after the name.
fn row_chips(row: &PluginRow) -> Vec<(&'static str, bool)> {
    let mut chips: Vec<(&'static str, bool)> = Vec::new();
    match &row.state {
        RowState::Failed { .. } => chips.push(("FAILED", true)),
        RowState::BlockedMissingDependency { .. } => chips.push(("BLOCKED", true)),
        RowState::NotInThisBuild => chips.push(("NOT IN BUILD", false)),
        _ => {}
    }
    if row.state.pending() {
        chips.push(("RESTART", true));
    }
    // Delta 3: the kind chip answers "what does this cost an export?" — but
    // an orphan is not in this build, so we do not actually know its kind and
    // must not assert one.
    if !matches!(row.state, RowState::NotInThisBuild) {
        chips.push(match row.kind {
            PluginKind::EditorOnly => ("EDITOR-ONLY", false),
            PluginKind::Runtime => ("SHIPS", false),
        });
    }
    chips
}

/// The Plugin Manager page. Draws into `content` and returns nothing —
/// mutations land in `state` and `project`.
pub fn plugin_manager_page(
    ui: &mut Ui,
    content: Rect,
    state: &mut PluginManagerState,
    model: &PluginManagerModel,
    project: &mut ProjectConfig,
) {
    let style = ui.style();
    let pal = style.palette;
    let st = super::theme::Palette::invariant_status();
    let font = style.fonts.body;

    // ── filter row
    let filter_rect = Rect::from_min_size(content.min, Vec2::new(content.width(), FILTER_H));
    ui.painter().line_segment(
        Pos2::new(filter_rect.min.x, filter_rect.max.y),
        Pos2::new(filter_rect.max.x, filter_rect.max.y),
        1.0,
        pal.stroke,
    );
    ui.set_cursor(Pos2::new(filter_rect.min.x + 10.0, filter_rect.min.y + 8.0));
    TextEdit::new(&mut state.search)
        .width(224.0)
        .hint("Filter plugins  (Ctrl+F)")
        .show(ui);

    let seg_labels: Vec<String> = ManagerFilter::ALL
        .iter()
        .map(|f| format!("{} {}", f.label(), model.count(*f)))
        .collect();
    let seg_refs: Vec<&str> = seg_labels.iter().map(|s| s.as_str()).collect();
    let seg_rect = Rect::from_min_size(
        Pos2::new(filter_rect.min.x + 244.0, filter_rect.min.y + 8.0),
        Vec2::new(300.0, 24.0),
    );
    let active = ManagerFilter::ALL
        .iter()
        .position(|f| *f == state.filter)
        .unwrap_or(0);
    if let Some(i) = segmented_control(ui, "plugin_filter", seg_rect, &seg_refs, active, true) {
        state.filter = ManagerFilter::ALL[i];
    }

    // ── pending-restart banner
    let pending = model.pending_count();
    let mut body_top = filter_rect.max.y + 1.0;
    if pending > 0 {
        let banner = Rect::from_min_size(
            Pos2::new(content.min.x, body_top),
            Vec2::new(content.width(), BANNER_H),
        );
        ui.painter().rect_filled(banner, 0.0, pal.elevated);
        ui.painter().circle_filled(
            Pos2::new(banner.min.x + 14.0, banner.center().y),
            3.5,
            st.warning,
        );
        ui.painter().text(
            Pos2::new(banner.min.x + 26.0, banner.center().y - font * 0.62),
            &format!(
                "{pending} change{} pending \u{2014} takes effect when the editor relaunches",
                if pending == 1 { "" } else { "s" }
            ),
            font,
            pal.text_secondary,
            None,
        );
        let btn = Rect::from_min_size(
            Pos2::new(banner.max.x - 108.0, banner.min.y + 6.0),
            Vec2::new(96.0, 20.0),
        );
        let resp = ui.interact(Id::new("plugin_relaunch"), btn);
        if resp.hovered {
            ui.painter().rect_filled(btn, style.rounding.small, pal.hover);
        }
        ui.painter()
            .rect_stroke(btn, style.rounding.small, 1.0, pal.stroke_strong);
        let label = "Relaunch now";
        let w = ui.painter().measure_text(label, font, None).x;
        ui.painter().text(
            Pos2::new(btn.center().x - w * 0.5, btn.center().y - font * 0.62),
            label,
            font,
            pal.text,
            None,
        );
        if resp.clicked {
            state.relaunch_requested = true;
        }
        body_top = banner.max.y + 1.0;
    }

    // ── cascade confirmation replaces the body while it is up
    if let Some(prompt) = state.pending_cascade.clone() {
        draw_cascade(ui, content, body_top, &prompt, state, project);
        return;
    }

    let footer = Rect::from_min_max(
        Pos2::new(content.min.x, content.max.y - FOOTER_H),
        content.max,
    );
    let body = Rect::from_min_max(
        Pos2::new(content.min.x, body_top),
        Pos2::new(content.max.x, footer.min.y),
    );

    let visible = model.visible(state.filter, &state.search);
    if model.rows.is_empty() {
        draw_empty_state(ui, body);
    } else {
        let list = Rect::from_min_max(
            body.min,
            Pos2::new((body.min.x + LIST_W).min(body.max.x), body.max.y),
        );
        let detail = Rect::from_min_max(Pos2::new(list.max.x + 1.0, body.min.y), body.max);
        ui.painter().line_segment(
            Pos2::new(list.max.x, list.min.y),
            Pos2::new(list.max.x, list.max.y),
            1.0,
            pal.stroke,
        );
        draw_list(ui, list, &visible, state, project, model);

        // Selection falls back to the first visible row so the pane is never
        // blank for a reason the user cannot see.
        let selected = state
            .selected
            .as_ref()
            .and_then(|id| model.get(id))
            .or_else(|| visible.first().copied());
        ui.painter().rect_filled(detail, 0.0, pal.panel);
        if let Some(row) = selected {
            draw_detail(ui, detail, row, model);
        }
    }

    // ── footer counts
    ui.painter().rect_filled(footer, 0.0, pal.window);
    ui.painter().line_segment(
        Pos2::new(footer.min.x, footer.min.y),
        Pos2::new(footer.max.x, footer.min.y),
        1.0,
        pal.stroke,
    );
    ui.painter().text_family(
        Pos2::new(footer.min.x + 12.0, footer.center().y - 5.0),
        &format!(
            "{} plugins \u{00B7} {} loaded",
            model.rows.len(),
            model.live_count()
        ),
        10.0,
        pal.text_secondary,
        None,
        FontFamily::Mono,
    );
}

fn draw_empty_state(ui: &mut Ui, body: Rect) {
    let style = ui.style();
    let pal = style.palette;
    let cx = body.center().x;
    let cy = body.center().y;
    // Delta: tier 1 has no plugins folder to point at or open — plugins are
    // compiled in — so the mockup's path line and "Open plugins folder"
    // button are replaced by an explanation of how discovery actually works.
    let lines: [(&str, f32, crusty_gui::math::Color); 2] = [
        ("No plugins in this build", 13.0, pal.text_secondary),
        (
            "Plugins are compiled into the editor and listed here at startup.",
            11.0,
            pal.text_disabled,
        ),
    ];
    for (i, (text, size, color)) in lines.iter().enumerate() {
        let w = ui.painter().measure_text(text, *size, None).x;
        ui.painter().text(
            Pos2::new(cx - w * 0.5, cy - 14.0 + i as f32 * 20.0),
            text,
            *size,
            *color,
            None,
        );
    }
}

fn draw_cascade(
    ui: &mut Ui,
    content: Rect,
    top: f32,
    prompt: &CascadePrompt,
    state: &mut PluginManagerState,
    project: &mut ProjectConfig,
) {
    let style = ui.style();
    let pal = style.palette;
    let st = super::theme::Palette::invariant_status();
    let font = style.fonts.body;
    let body = Rect::from_min_max(Pos2::new(content.min.x, top), content.max);
    ui.painter().rect_filled(body, 0.0, pal.panel);

    let x = body.min.x + 24.0;
    let mut y = body.min.y + 28.0;
    ui.painter().text(
        Pos2::new(x, y),
        "\u{25B2}  Other plugins depend on this one",
        13.0,
        st.warning,
        None,
    );
    y += 26.0;
    ui.painter().text(
        Pos2::new(x, y),
        &prompt.message(),
        font,
        pal.text_secondary,
        Some(body.width() - 48.0),
    );
    y += 44.0;

    let confirm = Rect::from_min_size(Pos2::new(x, y), Vec2::new(150.0, 24.0));
    let cancel = Rect::from_min_size(Pos2::new(x + 158.0, y), Vec2::new(90.0, 24.0));
    for (rect, label, danger) in [
        (confirm, "Disable anyway", true),
        (cancel, "Cancel", false),
    ] {
        let resp = ui.interact(Id::new("cascade").with(label), rect);
        if resp.hovered {
            ui.painter().rect_filled(rect, style.rounding.small, pal.hover);
        }
        ui.painter().rect_stroke(
            rect,
            style.rounding.small,
            1.0,
            if danger { st.warning } else { pal.stroke_strong },
        );
        let w = ui.painter().measure_text(label, font, None).x;
        ui.painter().text(
            Pos2::new(rect.center().x - w * 0.5, rect.center().y - font * 0.62),
            label,
            font,
            if danger { st.warning } else { pal.text },
            None,
        );
        if resp.clicked {
            if danger {
                set_manifest_enabled(project, &prompt.id, false);
            }
            state.pending_cascade = None;
        }
    }
}

fn draw_list(
    ui: &mut Ui,
    list: Rect,
    visible: &[&PluginRow],
    state: &mut PluginManagerState,
    project: &mut ProjectConfig,
    model: &PluginManagerModel,
) {
    let style = ui.style();
    let pal = style.palette;
    let st = super::theme::Palette::invariant_status();
    let font = style.fonts.body;

    if visible.is_empty() {
        ui.painter().text(
            Pos2::new(list.min.x + 14.0, list.min.y + 22.0),
            "No plugins match the current filter.",
            font,
            pal.text_disabled,
            None,
        );
        return;
    }

    let mut y = list.min.y;
    let mut last_origin: Option<PluginOrigin> = None;
    let mut toggle_request: Option<(String, bool)> = None;

    for row in visible {
        // ── group header (delta 3)
        if last_origin != Some(row.origin) {
            let header = Rect::from_min_size(Pos2::new(list.min.x, y), Vec2::new(list.width(), GROUP_H));
            ui.painter().rect_filled(header, 0.0, pal.window);
            // A rule above separates the group from the rows before it; the
            // header must read as a header, not as the previous row's
            // metadata.
            if last_origin.is_some() {
                ui.painter().line_segment(
                    Pos2::new(header.min.x, header.min.y),
                    Pos2::new(header.max.x, header.min.y),
                    1.0,
                    pal.stroke_strong,
                );
            }
            ui.painter().text_family(
                Pos2::new(header.min.x + 10.0, header.center().y - 5.0),
                match row.origin {
                    PluginOrigin::Engine => "ENGINE PLUGINS",
                    PluginOrigin::Project => "PROJECT PLUGINS",
                },
                10.0,
                pal.text_secondary,
                None,
                FontFamily::Mono,
            );
            y += GROUP_H;
            last_origin = Some(row.origin);
        }
        if y > list.max.y {
            break;
        }

        let rect = Rect::from_min_size(Pos2::new(list.min.x, y), Vec2::new(list.width(), ROW_H));
        let selected = state.selected.as_deref() == Some(row.id.as_str());
        let resp = ui.interact(Id::new("plugin_row").with(&row.id), rect);
        if selected {
            ui.painter().rect_filled(rect, 0.0, pal.selection_fill);
        } else if resp.hovered {
            ui.painter().rect_filled(rect, 0.0, pal.hover);
        }
        if resp.clicked {
            state.selected = Some(row.id.clone());
        }

        // glyph column
        let (glyph, gcolor, gsize) = state_glyph(&row.state, ui);
        if !glyph.is_empty() {
            let gw = ui.painter().measure_text(glyph, gsize, None).x;
            ui.painter().text(
                Pos2::new(rect.min.x + 8.0 + (14.0 - gw) * 0.5, rect.min.y + 15.0),
                glyph,
                gsize,
                gcolor,
                None,
            );
        }

        // name + version + chips
        let dim = !row.state.manifest_on() && !row.state.pending();
        let name_color = if selected {
            pal.selection_text
        } else if dim {
            pal.text_secondary
        } else {
            pal.text
        };
        let mut x = rect.min.x + 30.0;
        ui.painter()
            .text(Pos2::new(x, rect.min.y + 7.0), &row.name, 13.0, name_color, None);
        x += ui.painter().measure_text(&row.name, 13.0, None).x + 7.0;
        if !row.version.is_empty() {
            let v = format!("v{}", row.version);
            ui.painter().text_family(
                Pos2::new(x, rect.min.y + 10.0),
                &v,
                10.0,
                pal.text_disabled,
                None,
                FontFamily::Mono,
            );
            x += ui
                .painter()
                .measure_text_family(&v, 10.0, None, FontFamily::Mono)
                .x
                + 6.0;
        }
        for (text, loud) in row_chips(row) {
            let cw = ui
                .painter()
                .measure_text_family(text, 8.5, None, FontFamily::Mono)
                .x;
            let chip = Rect::from_min_size(
                Pos2::new(x, rect.min.y + 9.0),
                Vec2::new(cw + 10.0, 13.0),
            );
            if chip.max.x > rect.max.x - 52.0 {
                break;
            }
            let color = if loud { st.warning } else { pal.text_disabled };
            ui.painter()
                .rect_stroke(chip, style.rounding.small, 1.0, color);
            ui.painter().text_family(
                Pos2::new(chip.min.x + 5.0, chip.min.y + 2.0),
                text,
                8.5,
                color,
                None,
                FontFamily::Mono,
            );
            x = chip.max.x + 5.0;
        }

        // second line
        let (line2, l2color) = match &row.state {
            RowState::Failed { message, .. } => (message.clone(), st.error),
            RowState::BlockedMissingDependency { missing } => (
                format!("Blocked \u{2014} requires {missing}"),
                st.warning,
            ),
            RowState::NotInThisBuild => (
                "Not compiled into this build \u{2014} setting preserved".to_string(),
                pal.text_disabled,
            ),
            _ => {
                let text = if row.author.is_empty() {
                    row.description.clone()
                } else {
                    format!("{} \u{2014} {}", row.author, row.description)
                };
                (text, if dim { pal.text_disabled } else { pal.text_secondary })
            }
        };
        let line2 = truncate_to_width(ui, &line2, 10.5, rect.width() - 92.0);
        ui.painter().text(
            Pos2::new(rect.min.x + 30.0, rect.min.y + 26.0),
            &line2,
            10.5,
            l2color,
            None,
        );

        // toggle
        let locked = row.state.toggle_locked();
        let on = row.state.manifest_on() && !locked;
        let tg = Rect::from_min_size(
            Pos2::new(rect.max.x - 46.0, rect.center().y - 8.0),
            Vec2::new(30.0, 16.0),
        );
        let tg_resp = ui.interact(Id::new("plugin_toggle").with(&row.id), tg);
        let (fill, border) = if locked {
            (pal.input, pal.stroke)
        } else if on {
            (pal.accent_active, pal.accent_active)
        } else {
            (pal.input, pal.stroke_strong)
        };
        ui.painter().rect_filled(tg, 8.0, fill);
        ui.painter().rect_stroke(tg, 8.0, 1.0, border);
        ui.painter().circle_filled(
            Pos2::new(if on { tg.max.x - 7.0 } else { tg.min.x + 7.0 }, tg.center().y),
            5.0,
            if locked {
                pal.text_disabled
            } else if on {
                pal.accent_text
            } else {
                pal.text_secondary
            },
        );
        if tg_resp.clicked && !locked {
            toggle_request = Some((row.id.clone(), !on));
        }

        y = rect.max.y;
    }

    // Applied after the loop so the borrow of `visible` is done.
    if let Some((id, enable)) = toggle_request {
        state.selected = Some(id.clone());
        if enable {
            set_manifest_enabled(project, &id, true);
        } else {
            // §5.7: name the closure before taking it down.
            let prompt = model.cascade_for(&id);
            if prompt.is_empty() {
                set_manifest_enabled(project, &id, false);
            } else {
                state.pending_cascade = Some(prompt);
            }
        }
    }
}

fn draw_detail(ui: &mut Ui, detail: Rect, row: &PluginRow, model: &PluginManagerModel) {
    let style = ui.style();
    let pal = style.palette;
    let font = style.fonts.body;

    // ── header block
    let head_h = 62.0;
    let head = Rect::from_min_size(detail.min, Vec2::new(detail.width(), head_h));
    ui.painter().line_segment(
        Pos2::new(head.min.x, head.max.y),
        Pos2::new(head.max.x, head.max.y),
        1.0,
        pal.stroke,
    );
    let x = head.min.x + 16.0;
    ui.painter()
        .text(Pos2::new(x, head.min.y + 14.0), &row.name, 14.0, pal.text, None);
    let name_w = ui.painter().measure_text(&row.name, 14.0, None).x;
    if !row.version.is_empty() {
        ui.painter().text_family(
            Pos2::new(x + name_w + 8.0, head.min.y + 17.0),
            &format!("v{}", row.version),
            10.5,
            pal.text_disabled,
            None,
            FontFamily::Mono,
        );
    }
    if !row.author.is_empty() {
        let aw = ui.painter().measure_text(&row.author, 11.0, None).x;
        ui.painter().text(
            Pos2::new(head.max.x - 16.0 - aw, head.min.y + 16.0),
            &row.author,
            11.0,
            pal.text_secondary,
            None,
        );
    }
    let (glyph, gcolor, gsize) = state_glyph(&row.state, ui);
    ui.painter()
        .text(Pos2::new(x, head.min.y + 38.0), glyph, gsize, gcolor, None);
    ui.painter().text(
        Pos2::new(x + 16.0, head.min.y + 36.0),
        &state_text(row),
        font,
        gcolor,
        None,
    );

    // ── scrolling body
    let body = Rect::from_min_max(Pos2::new(detail.min.x, head.max.y + 1.0), detail.max);
    ui.set_cursor(Pos2::new(body.min.x + 16.0, body.min.y + 12.0));
    let avail_h = body.height() - 16.0;
    let wrap = body.width() - 32.0;
    let row = row.clone();
    let model = model.clone();
    ScrollArea::new(avail_h)
        .auto_shrink(false)
        .inset(0.0)
        .spacing(6.0)
        .show(ui, |ui| {
            let pal = ui.style().palette;
            let st = super::theme::Palette::invariant_status();
            let font = ui.style().fonts.body;

            label_wrapped(ui, &row.description, font, pal.text_secondary, wrap);
            ui.add_space(10.0);

            // Load error — with the phase progression lit up to the failure.
            if let RowState::Failed { phase, message } = &row.state {
                section_label(ui, "LOAD ERROR");
                draw_phases(ui, *phase);
                ui.add_space(4.0);
                label_wrapped(ui, message, font, st.error, wrap);
                ui.add_space(10.0);
            }

            if !row.warnings.is_empty() {
                section_label(ui, "WARNINGS");
                for w in &row.warnings {
                    label_wrapped(ui, &format!("\u{25B2}  {w}"), font, st.warning, wrap);
                }
                ui.add_space(10.0);
            }

            // Manifest facts — delta 1.
            section_label(ui, "MANIFEST");
            let kind = match row.kind {
                PluginKind::EditorOnly => "editor-only (never ships in the game)",
                PluginKind::Runtime => "runtime (ships with exported game)",
            };
            let origin = match row.origin {
                PluginOrigin::Engine => "engine",
                PluginOrigin::Project => "project",
            };
            let module = if row.module_path.is_empty() {
                "\u{2014}".to_string()
            } else {
                row.module_path.clone()
            };
            let feature = row
                .cargo_feature
                .clone()
                .unwrap_or_else(|| "(always compiled in)".to_string());
            for (k, v) in [
                ("Module", module.as_str()),
                ("Cargo feature", feature.as_str()),
                ("Kind", kind),
                ("Origin", origin),
            ] {
                fact_row(ui, k, v);
            }
            ui.add_space(10.0);

            section_label(ui, "DEPENDENCIES");
            if row.depends_on.is_empty() {
                label_wrapped(ui, "None", font, pal.text_disabled, wrap);
            } else {
                for dep in &row.depends_on {
                    let (note, color) = match model.get(dep) {
                        Some(d) if d.state.is_live() => ("loaded".to_string(), st.success),
                        Some(d) => (
                            match &d.state {
                                RowState::Failed { .. } => "failed".to_string(),
                                RowState::NotInThisBuild => "not in this build".to_string(),
                                _ => "disabled".to_string(),
                            },
                            st.warning,
                        ),
                        None => ("not present".to_string(), st.warning),
                    };
                    fact_row_colored(ui, dep, &note, color);
                }
            }
            ui.add_space(10.0);

            section_label(ui, "REGISTERED CONTENT");
            let reg = match (&row.state, row.counts) {
                (_, Some(c)) => registered_summary(&c),
                (RowState::Failed { .. }, None) => {
                    "\u{2014} load failed before registration".to_string()
                }
                (RowState::BlockedMissingDependency { .. }, None) => {
                    "\u{2014} never loaded".to_string()
                }
                (RowState::NotInThisBuild, None) => "\u{2014} not in this build".to_string(),
                (RowState::PendingEnable, None) => "\u{2014} loads on relaunch".to_string(),
                (_, None) => "\u{2014} not loaded".to_string(),
            };
            let reg_color = if row.counts.is_some() {
                pal.text_mono
            } else {
                pal.text_disabled
            };
            let mono = ui.style().fonts.body - 1.0;
            let pos = ui.cursor();
            ui.painter()
                .text_family(pos, &reg, mono, reg_color, None, FontFamily::Mono);
            ui.add_space(16.0);
        });
}

fn registered_summary(c: &PluginCounts) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut push = |n: usize, one: &str, many: &str| {
        if n > 0 {
            parts.push(format!("{n} {}", if n == 1 { one } else { many }));
        }
    };
    push(c.systems, "system", "systems");
    push(c.resources, "resource", "resources");
    push(c.node_types, "node type", "node types");
    push(c.domain_pins, "domain pin", "domain pins");
    push(c.migrations, "migration", "migrations");
    push(c.panels, "panel", "panels");
    push(c.settings_pages, "settings page", "settings pages");
    push(c.world_loaded_callbacks, "content hook", "content hooks");
    push(c.debug_draw_hooks, "debug overlay", "debug overlays");
    if parts.is_empty() {
        "\u{2014} nothing registered".to_string()
    } else {
        parts.join(" \u{00B7} ")
    }
}

/// The load pipeline, with everything after the failure struck out. Mirrors
/// the mockup's three-step progression over our real phases.
fn draw_phases(ui: &mut Ui, failed: RegistrationPhase) {
    let stages: [(&str, &[RegistrationPhase]); 3] = [
        ("dependencies", &[RegistrationPhase::Dependency]),
        ("build", &[RegistrationPhase::Build]),
        (
            "registration",
            &[
                RegistrationPhase::Systems,
                RegistrationPhase::Resources,
                RegistrationPhase::NodeRegistry,
                RegistrationPhase::Panels,
                RegistrationPhase::WorldLoaded,
            ],
        ),
    ];
    let st = super::theme::Palette::invariant_status();
    let pal = ui.style().palette;
    let pos = ui.cursor();
    let mut x = pos.x;
    let mut hit = false;
    for (label, phases) in stages {
        let is_fail = phases.contains(&failed);
        let (glyph, color) = if is_fail {
            hit = true;
            ("\u{2715}", st.error)
        } else if hit {
            ("\u{2014}", pal.text_disabled)
        } else {
            ("\u{2713}", st.success)
        };
        let text = format!("{glyph} {label}");
        ui.painter()
            .text_family(Pos2::new(x, pos.y), &text, 10.0, color, None, FontFamily::Mono);
        x += ui
            .painter()
            .measure_text_family(&text, 10.0, None, FontFamily::Mono)
            .x
            + 14.0;
    }
    ui.allocate(Vec2::new(ui.available().width(), 14.0));
}

fn section_label(ui: &mut Ui, text: &str) {
    let pal = ui.style().palette;
    let pos = ui.cursor();
    ui.painter().text_family(
        pos,
        text,
        9.5,
        pal.text_secondary,
        None,
        FontFamily::Mono,
    );
    ui.allocate(Vec2::new(ui.available().width(), 15.0));
}

fn fact_row(ui: &mut Ui, key: &str, value: &str) {
    let pal = ui.style().palette;
    fact_row_colored(ui, key, value, pal.text_mono);
}

fn fact_row_colored(ui: &mut Ui, key: &str, value: &str, color: crusty_gui::math::Color) {
    let pal = ui.style().palette;
    let font = ui.style().fonts.body;
    let pos = ui.cursor();
    let avail = ui.available().width();
    ui.painter()
        .text(pos, key, font, pal.text_disabled, None);
    // Values are paths and identifiers — clipping mid-word beats spilling
    // past the pane edge.
    let value = truncate_to_width(ui, value, font - 1.0, (avail - 108.0).max(40.0));
    ui.painter().text_family(
        Pos2::new(pos.x + 100.0, pos.y + 1.0),
        &value,
        font - 1.0,
        color,
        None,
        FontFamily::Mono,
    );
    ui.allocate(Vec2::new(avail, 17.0));
}

/// Clip `text` to `max_w` on one line, ending in an ellipsis.
///
/// Row second lines must stay single-line: the mockup's rows are a fixed
/// height, and a wrapped description would bleed into the row below it.
fn truncate_to_width(ui: &mut Ui, text: &str, size: f32, max_w: f32) -> String {
    if ui.painter().measure_text(text, size, None).x <= max_w {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let mut candidate = out.clone();
        candidate.push(ch);
        if ui
            .painter()
            .measure_text(&format!("{candidate}\u{2026}"), size, None)
            .x
            > max_w
        {
            break;
        }
        out = candidate;
    }
    let trimmed = out.trim_end();
    format!("{trimmed}\u{2026}")
}

fn label_wrapped(
    ui: &mut Ui,
    text: &str,
    size: f32,
    color: crusty_gui::math::Color,
    wrap: f32,
) {
    let pos = ui.cursor();
    let measured = ui.painter().measure_text(text, size, Some(wrap));
    ui.painter().text(pos, text, size, color, Some(wrap));
    ui.allocate(Vec2::new(ui.available().width(), measured.y.max(size * 1.4)));
}

#[cfg(test)]
mod tests;
