//! Tree row widget for hierarchy and asset browser tree views.

use egui::{Response, Sense, Ui};

use super::UiExt;
use super::IconKind;

/// Configuration for a single tree row.
pub struct TreeRowConfig {
    /// Indentation depth (0 = root)
    pub depth: usize,
    /// Whether this node is expanded
    pub expanded: bool,
    /// Whether this node has children
    pub has_children: bool,
    /// Whether this row is currently selected
    pub selected: bool,
    /// Icon to display
    pub icon: IconKind,
    /// Display name
    pub label: String,
    /// Whether to show the drag handle on hover
    pub draggable: bool,
    /// Optional prefab override indicator dot
    pub override_dot: bool,
}

/// Result of rendering a tree row.
pub struct TreeRowResult {
    /// The row response (for click/hover/drag detection)
    pub response: Response,
    /// True if the expand/collapse chevron was toggled
    pub toggled: bool,
}

/// Render a tree row with indent guides, chevron, icon, and label.
///
/// Keyboard contract:
/// - Arrow keys: navigate between rows (caller handles)
/// - Enter: expand/collapse
/// - F2: begin rename (caller handles)
pub fn tree_row(ui: &mut Ui, config: &TreeRowConfig) -> TreeRowResult {
    let theme = ui.theme();
    let indent_px = theme.spacing.indent;
    let row_height = theme.spacing.interact_size_y;

    let indent = config.depth as f32 * indent_px;
    let available_width = ui.available_width();

    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(available_width, row_height),
        Sense::click_and_drag(),
    );

    let mut toggled = false;

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();

        // Selection/hover background
        if config.selected {
            painter.rect_filled(rect, 0.0, theme.palette.accent.gamma_multiply(0.2));
        } else if response.hovered() {
            painter.rect_filled(rect, 0.0, theme.palette.surface[3]);
        }

        let mut x = rect.left() + indent + 4.0;

        // Indent guides (vertical strokes)
        for d in 1..=config.depth {
            let guide_x = rect.left() + d as f32 * indent_px - indent_px / 2.0;
            painter.line_segment(
                [
                    egui::pos2(guide_x, rect.top()),
                    egui::pos2(guide_x, rect.bottom()),
                ],
                egui::Stroke::new(1.0, theme.palette.stroke),
            );
        }

        // Chevron (expand/collapse)
        if config.has_children {
            let chevron_center = egui::pos2(x + 6.0, rect.center().y);
            let chevron_text = if config.expanded { "\u{25BE}" } else { "\u{25B8}" };
            painter.text(
                chevron_center,
                egui::Align2::CENTER_CENTER,
                chevron_text,
                egui::FontId::proportional(10.0),
                theme.palette.text_secondary,
            );

            // Toggle on click in chevron area
            let chevron_rect = egui::Rect::from_center_size(chevron_center, egui::vec2(14.0, row_height));
            if response.clicked() && chevron_rect.contains(response.interact_pointer_pos().unwrap_or_default()) {
                toggled = true;
            }

            x += 14.0;
        } else {
            x += 14.0;
        }

        // Icon
        let icon_text = super::IconRegistry::fallback_text(config.icon);
        painter.text(
            egui::pos2(x + 8.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            icon_text,
            egui::FontId::proportional(11.0),
            theme.palette.text_secondary,
        );
        x += 18.0;

        // Override dot
        if config.override_dot {
            painter.circle_filled(
                egui::pos2(x, rect.center().y),
                3.0,
                theme.palette.semantic.overridden,
            );
            x += 8.0;
        }

        // Label
        painter.text(
            egui::pos2(x + 2.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            &config.label,
            egui::FontId::proportional(12.0),
            theme.palette.text_primary,
        );

        // Drag handle (visible on hover)
        if config.draggable && response.hovered() {
            let handle_x = rect.right() - 16.0;
            painter.text(
                egui::pos2(handle_x, rect.center().y),
                egui::Align2::CENTER_CENTER,
                "\u{2261}", // ≡
                egui::FontId::proportional(14.0),
                theme.palette.text_disabled,
            );
        }

        // Focus ring
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                0,
                egui::Stroke::new(1.5, theme.palette.focus_ring),
                egui::StrokeKind::Inside,
            );
        }
    }

    TreeRowResult { response, toggled }
}
