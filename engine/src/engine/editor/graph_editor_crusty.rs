//! Graph editor panel (Task 40, P4) — placeholder body only.
//!
//! Draws the document path, its node/edge counts, and any validation errors.
//! The pan/zoom canvas, nodes, wires, and create menu land in P5. Colors come
//! from the theme only (no raw color literals — M10 grep gate); error text
//! uses the theme's invariant status color, matching `console_crusty` /
//! `inspector_crusty`.

use crusty_gui::context::Ui;
use crusty_gui::widgets::Label;

use super::graph_editor::GraphEditorState;
use super::theme::Palette;

/// Placeholder panel body for an open graph document.
pub fn graph_editor_panel(ui: &mut Ui, state: &mut GraphEditorState) {
    let title_font = ui.style().fonts.title;
    Label::new(format!("Graph \u{2014} {}", state.path))
        .size(title_font)
        .show(ui);

    ui.add_space(6.0);

    let dim = ui.style().palette.text_secondary;
    Label::new(format!(
        "{} nodes \u{00b7} {} edges",
        state.doc.nodes.len(),
        state.doc.edges.len()
    ))
    .color(dim)
    .show(ui);

    if !state.errors.is_empty() {
        ui.add_space(6.0);
        let err_col = Palette::invariant_status().error;
        Label::new(format!(
            "{} validation error{}:",
            state.errors.len(),
            if state.errors.len() == 1 { "" } else { "s" }
        ))
        .color(err_col)
        .show(ui);
        for e in &state.errors {
            Label::new(format!("  {e}")).color(err_col).show(ui);
        }
    }
}
