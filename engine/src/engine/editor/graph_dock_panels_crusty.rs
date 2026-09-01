//! The graph document's dock panels — Details and Variables (per-document
//! layouts ticket 02).
//!
//! Both are thin views over the *focused graph document*'s `GraphEditorState`.
//! The host resolves which document that is (`active_graph_key`) and draws
//! the "No graph focused" line itself when there is none, so a panel here
//! always has a state to show. Nothing in this file has edit logic of its
//! own: Details renders the selection's config rows through the same
//! `config_rows` / `inline_widget` pair the canvas band uses — one
//! `SetProperty` path, so an edit here undoes, dirties and saves exactly as
//! the same edit on the node would — and Variables is the tab's strip
//! (`variables_panel`) drawn docked. The strip and these panels share the
//! document state, so they cannot disagree.

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::math::{Pos2, Rect, Vec2};
use crusty_gui::style::Style;
use crusty_gui::text::FontFamily;
use crusty_gui::widgets::{ScrollArea, TextEdit};

use super::graph_editor::{
    alias_name, pin_type_label, prop_display, DetailsRename, GraphDomain, GraphEditorState,
};
use super::graph_editor_crusty::{
    anim_state_name, config_rows, inline_widget, variables_panel, DocResolvers, InlineKind,
};
use super::theme::Palette;
use crate::engine::animation::graph::plan::{
    ANIM_STATE_ALIAS_TYPE_ID, ANIM_STATE_TYPE_ID, ANIM_TRANSITION_TYPE_ID,
};
use crate::engine::node_graph::{CurveResolver, GraphResolver, NodeInst, NodeRegistry};

/// Label column of a Details row, at UI scale 1.
const DETAILS_LABEL_W: f32 = 84.0;
/// The row pitch the label column is scaled against.
const BASE_ROW_H: f32 = 22.0;

pub struct GraphDetailsPanelCtx<'a> {
    pub state: &'a mut GraphEditorState,
    pub registry: &'a NodeRegistry,
    /// Same resolvers the tab binds, so a state's rows see the same document
    /// descriptors the canvas does.
    pub resolver: &'a dyn GraphResolver,
    pub curves: &'a dyn CurveResolver,
}

pub struct GraphVariablesPanelCtx<'a> {
    pub state: &'a mut GraphEditorState,
    pub registry: &'a NodeRegistry,
}

/// The Variables panel: the tab's strip, docked. A locate raised here has no
/// canvas to frame on, so it is parked on the state for the tab's next draw.
pub fn graph_variables_panel(ui: &mut Ui, rect: Rect, ctx: GraphVariablesPanelCtx) {
    let GraphVariablesPanelCtx { state, registry } = ctx;
    // Same beat as the tab: a default drag closes when the pointer does,
    // whichever surface it was made on.
    if !ui.ctx().input.pointer_down {
        state.flush_var_default_edit(registry);
    }
    let mut locate = None;
    variables_panel(ui, rect, state, registry, Id::new("graph_vars_dock"), true, &mut locate);
    if locate.is_some() {
        state.locate_request = locate;
    }
}

/// The Details panel: the selection's properties, or the document's summary.
pub fn graph_details_panel(ui: &mut Ui, rect: Rect, ctx: GraphDetailsPanelCtx) {
    let GraphDetailsPanelCtx { state, registry, resolver, curves } = ctx;
    if !ui.ctx().input.pointer_down {
        state.flush_prop_edit(registry);
    }
    let st = ui.style();
    let pad = st.spacing.padding;
    // Salted by document: a field focused here must not carry its focus to
    // the same-numbered node of the next graph that takes the document focus.
    let root = Id::new(("graph_details_dock", state.path.as_str()));
    ui.run_at(
        rect,
        Direction::TopDown,
        root,
        UiOptions { padding: Vec2::splat(pad), spacing: pad * 0.5 },
        |ui| {
            let h = ui.available().height();
            ScrollArea::new(h).inset(0.0).show(ui, |ui| {
                let selected: Vec<u64> = state.selection.iter().copied().collect();
                match selected.as_slice() {
                    // No node selected: the strip's selected variable is the
                    // next most specific thing the user has pointed at.
                    [] => match state.vars.selected.clone() {
                        Some(slug) if state.doc.variable(&slug).is_some() => {
                            variable_body(ui, state, &slug, &st)
                        }
                        _ => summary_body(ui, state, &st),
                    },
                    [id] => match state.doc.node(*id).cloned() {
                        Some(n) => {
                            node_body(ui, state, registry, resolver, curves, &n, root, &st)
                        }
                        // A selected id the document no longer has (an edge
                        // selection, a stale id): the document is still the
                        // honest thing to show.
                        None => summary_body(ui, state, &st),
                    },
                    many => {
                        header(ui, &format!("{} nodes selected", many.len()), None, &st);
                        caption(ui, "Select one node to edit its properties.", &st);
                    }
                }
            });
        },
    );
}

/// Nothing selected: what the document is, in a glance.
fn summary_body(ui: &mut Ui, state: &GraphEditorState, st: &Style) {
    let name = state
        .path
        .rsplit('/')
        .next()
        .unwrap_or(state.path.as_str())
        .to_string();
    header(ui, &name, Some(domain_label(state.domain)), st);
    read_row(ui, "File", &state.path, st, st.palette.text_mono);
    read_row(ui, "Nodes", &state.doc.nodes.len().to_string(), st, st.palette.text);
    read_row(ui, "Variables", &state.doc.variables.len().to_string(), st, st.palette.text);
    let errors = state.errors.len() + state.domain_errors.len();
    let err_col = if errors > 0 {
        Palette::invariant_status().error
    } else {
        st.palette.text
    };
    read_row(ui, "Errors", &errors.to_string(), st, err_col);
    caption(ui, "Select a node to edit its properties.", st);
}

/// The strip's selected variable, read back as a declaration. Read-only on
/// purpose: its editor is the Variables panel's footer, and a third rename
/// surface would be one more thing to keep from fighting.
fn variable_body(ui: &mut Ui, state: &GraphEditorState, slug: &str, st: &Style) {
    let Some(decl) = state.doc.variable(slug) else {
        return;
    };
    header(ui, &decl.label, Some("Variable"), st);
    read_row(ui, "Slug", &decl.slug, st, st.palette.text_mono);
    read_row(ui, "Type", &pin_type_label(&decl.ty), st, st.palette.text_mono);
    let default = decl
        .default
        .as_ref()
        .map(prop_display)
        .unwrap_or_else(|| "\u{2014}".to_string());
    read_row(ui, "Default", &default, st, st.palette.text_mono);
    if let Some(group) = &decl.group {
        read_row(ui, "Group", group, st, st.palette.text_mono);
    }
    let uses = state.variable_usage_count(slug);
    read_row(ui, "Uses", &uses.to_string(), st, st.palette.text);
    caption(ui, "Edit the declaration in the Variables panel.", st);
}

/// One node: its identity, then its config rows — the same rows, in the
/// same order, with the same widgets, as the band on the node itself.
#[allow(clippy::too_many_arguments)]
fn node_body(
    ui: &mut Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    resolver: &dyn GraphResolver,
    curves: &dyn CurveResolver,
    n: &NodeInst,
    root: Id,
    st: &Style,
) {
    let (title, tag) = node_identity(state, registry, n);
    header(ui, &title, Some(&tag), st);
    let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
    let label_w = DETAILS_LABEL_W * s;
    // A state's or alias's Name: the only title the animation library
    // spells back to the user, so it gets a row (spec: Name / Clip / …).
    if matches!(
        n.type_id.as_str(),
        ANIM_STATE_TYPE_ID | ANIM_STATE_ALIAS_TYPE_ID
    ) {
        let row = ui.allocate(Vec2::new(ui.available().width(), st.metrics.control_height));
        row_label(ui, row, "Name", st);
        let cell = Rect::from_min_max(Pos2::new(row.min.x + label_w, row.min.y), row.max);
        name_row(ui, state, registry, n, cell, root);
    }
    if n.type_id == ANIM_TRANSITION_TYPE_ID {
        let rule = state
            .doc
            .regions
            .get(&n.id)
            .map(|r| r.nodes.len())
            .unwrap_or(0);
        let text = match rule {
            0 => "empty \u{2014} always taken".to_string(),
            1 => "1 node".to_string(),
            k => format!("{k} nodes"),
        };
        read_row(ui, "Rule", &text, st, st.palette.text_mono);
    }
    // Rows are owned copies: the descriptor binding borrows the document,
    // and the widgets below need it mutably.
    let rows: Vec<(String, String, InlineKind)> = {
        let docd = DocResolvers { graphs: resolver, curves }.bind(&state.doc, registry);
        config_rows(n, &docd)
    };
    if rows.is_empty() && n.type_id != ANIM_TRANSITION_TYPE_ID {
        caption(ui, "No properties on this node.", st);
        return;
    }
    for (key, label, kind) in rows {
        let row = ui.allocate(Vec2::new(ui.available().width(), st.metrics.control_height));
        row_label(ui, row, &label, st);
        let cell = Rect::from_min_max(Pos2::new(row.min.x + label_w, row.min.y), row.max);
        match &kind {
            InlineKind::Float(_)
            | InlineKind::Int(_)
            | InlineKind::Bool(_)
            | InlineKind::Str(_)
            | InlineKind::Enum { .. }
            | InlineKind::Choice { .. } => {
                let id = root.with(("graph_details_inline", n.id, key.as_str()));
                inline_widget(ui, state, registry, n.id, &key, cell, &kind, 1.0, id);
            }
            InlineKind::Chip(text) => mono_value(ui, cell, text, st, st.palette.text_secondary),
            InlineKind::Raw(text) => {
                mono_value(ui, cell, text, st, Palette::invariant_status().warning)
            }
            InlineKind::Color(c) => {
                let hex = format!("{:02X}{:02X}{:02X}", chan(c[0]), chan(c[1]), chan(c[2]));
                mono_value(ui, cell, &hex, st, st.palette.text_mono);
            }
        }
    }
}

/// The Name entry: one text field over a draft that lives on the state, so
/// typing survives frames; committed on Enter or on losing a focus it had,
/// as one `SetNodeTitle` (never per keystroke). Until edited, the field
/// shows the name as the canvas spells it.
fn name_row(
    ui: &mut Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    n: &NodeInst,
    cell: Rect,
    root: Id,
) {
    let shown = shown_name(n);
    let mut draft = None;
    if let Some(d) = state.details_rename.take() {
        if d.node == n.id {
            draft = Some(d);
        } else if d.seen_focus {
            // Selecting another node while typing is a blur: the text goes
            // to the node it was typed for, exactly as clicking away would.
            commit_title(state, registry, d.node, &d.buf);
        }
    }
    let seen_focus = draft.as_ref().is_some_and(|d| d.seen_focus);
    let mut buf = draft.map(|d| d.buf).unwrap_or_else(|| shown.clone());
    let (mut submitted, mut cancelled, mut focused) = (false, false, false);
    let field_bg = ui.style().palette.input;
    ui.run_at(
        cell,
        Direction::TopDown,
        root.with(("graph_details_name", n.id)),
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| {
            let out = TextEdit::new(&mut buf)
                .width(cell.width())
                .fill(field_bg)
                .show_full(ui);
            submitted = out.submitted;
            cancelled = out.cancelled;
            focused = out.focused;
        },
    );
    if cancelled {
        return;
    }
    if submitted || (seen_focus && !focused) {
        commit_title(state, registry, n.id, &buf);
        return;
    }
    if focused {
        state.details_rename = Some(DetailsRename {
            node: n.id,
            buf,
            seen_focus: true,
        });
    }
}

/// Apply a typed name to `node` as one undo entry — unless it is blank or
/// already what the canvas shows for that node.
fn commit_title(state: &mut GraphEditorState, registry: &NodeRegistry, node: u64, buf: &str) {
    let text = buf.trim();
    let Some(current) = state.doc.node(node).map(shown_name) else {
        return;
    };
    if !text.is_empty() && text != current {
        state.set_node_title(node, Some(text.to_string()), registry);
    }
}

/// The name the canvas spells for a state or alias — the Name row's baseline.
fn shown_name(n: &NodeInst) -> String {
    if n.type_id == ANIM_STATE_ALIAS_TYPE_ID {
        alias_name(n)
    } else {
        anim_state_name(n)
    }
}

/// Title + type tag for the header, by the canvas's own naming rules.
fn node_identity(state: &GraphEditorState, registry: &NodeRegistry, n: &NodeInst) -> (String, String) {
    let type_name = registry
        .get(&n.type_id)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| n.type_id.clone());
    if state.domain.is_animation() {
        if n.type_id == ANIM_STATE_TYPE_ID {
            return (shown_name(n), "State".to_string());
        }
        if n.type_id == ANIM_STATE_ALIAS_TYPE_ID {
            return (shown_name(n), "State Alias".to_string());
        }
        if n.type_id == ANIM_TRANSITION_TYPE_ID {
            let from = state.doc.edges.iter().find(|e| e.to_node == n.id).map(|e| e.from_node);
            let to = state.doc.edges.iter().find(|e| e.from_node == n.id).map(|e| e.to_node);
            let end = |id: Option<u64>| {
                id.and_then(|id| state.doc.node(id))
                    .map(|s| {
                        if s.type_id == ANIM_STATE_ALIAS_TYPE_ID {
                            alias_name(s)
                        } else {
                            anim_state_name(s)
                        }
                    })
                    .unwrap_or_else(|| "?".to_string())
            };
            return (format!("{} \u{2192} {}", end(from), end(to)), "Transition".to_string());
        }
    }
    let title = n
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| type_name.clone());
    (title, type_name)
}

fn domain_label(domain: GraphDomain) -> &'static str {
    match domain {
        GraphDomain::Script => "Script graph",
        GraphDomain::Animation => "Animation graph",
        GraphDomain::AnimationRule { .. } => "Transition rule",
    }
}

fn header(ui: &mut Ui, title: &str, tag: Option<&str>, st: &Style) {
    let row = ui.allocate(Vec2::new(ui.available().width(), st.metrics.control_height));
    let mut p = ui.painter();
    let body = st.fonts.body;
    let tw = p
        .text(
            Pos2::new(row.min.x, row.center().y - body * 0.62),
            title,
            body,
            st.palette.text,
            Some(row.width()),
        )
        .x;
    if let Some(tag) = tag {
        let small = st.fonts.small;
        let x = row.min.x + tw + st.spacing.item;
        p.text_family(
            Pos2::new(x, row.center().y - small * 0.62),
            tag,
            small,
            st.palette.text_secondary,
            Some((row.max.x - x).max(0.0)),
            FontFamily::Mono,
        );
    }
}

fn caption(ui: &mut Ui, text: &str, st: &Style) {
    let row = ui.allocate(Vec2::new(ui.available().width(), st.metrics.row_height));
    ui.painter().text(
        Pos2::new(row.min.x, row.center().y - st.fonts.small * 0.62),
        text,
        st.fonts.small,
        st.palette.text_disabled,
        Some(row.width()),
    );
}

fn row_label(ui: &mut Ui, row: Rect, label: &str, st: &Style) {
    let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
    ui.painter().text(
        Pos2::new(row.min.x, row.center().y - st.fonts.body * 0.62),
        label,
        st.fonts.body,
        st.palette.text_secondary,
        Some(DETAILS_LABEL_W * s - st.spacing.item),
    );
}

/// A read-only row: label, then a mono value.
fn read_row(ui: &mut Ui, label: &str, value: &str, st: &Style, color: crusty_gui::math::Color) {
    let row = ui.allocate(Vec2::new(ui.available().width(), st.metrics.control_height));
    row_label(ui, row, label, st);
    let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
    let cell = Rect::from_min_max(Pos2::new(row.min.x + DETAILS_LABEL_W * s, row.min.y), row.max);
    mono_value(ui, cell, value, st, color);
}

fn mono_value(ui: &mut Ui, cell: Rect, text: &str, st: &Style, color: crusty_gui::math::Color) {
    ui.painter().text_family(
        Pos2::new(cell.min.x, cell.center().y - st.fonts.small * 0.62),
        text,
        st.fonts.small,
        color,
        Some(cell.width()),
        FontFamily::Mono,
    );
}

fn chan(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}
