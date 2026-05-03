//! Apply editor theme tokens to egui's `Style` and `Visuals`.

use super::EditorTheme;
use egui::{Context, CornerRadius, Margin, Stroke, Vec2, Visuals};

impl EditorTheme {
    /// Push this theme's design tokens into egui's style and visuals.
    ///
    /// Called at editor startup and whenever the theme changes (density toggle,
    /// palette reload). This sets egui's global style — all panels rendered
    /// after this call will reflect the new values.
    pub fn apply_to(&self, ctx: &Context) {
        let mut style = (*ctx.style()).clone();
        let p = &self.palette;

        // --- Visuals ---
        let mut visuals = Visuals::dark();

        // Window / panel backgrounds
        visuals.window_fill = p.surface[2];
        visuals.panel_fill = p.surface[1];
        visuals.faint_bg_color = p.surface[0];
        // extreme_bg_color is used for TextEdit and Slider track backgrounds —
        // use the darker field_bg to create a visible "inset" against the panel.
        visuals.extreme_bg_color = p.field_bg;

        // Selection
        visuals.selection.bg_fill = p.accent.gamma_multiply(0.35);
        visuals.selection.stroke = Stroke::new(1.0, p.accent);

        // Hyperlinks
        visuals.hyperlink_color = p.accent;

        // Corner radius
        let corner_radius = CornerRadius::same(3);
        visuals.window_corner_radius = CornerRadius::same(6);
        visuals.menu_corner_radius = CornerRadius::same(4);

        // Widget styles
        // Noninteractive (labels, panel backgrounds)
        visuals.widgets.noninteractive.bg_fill = p.surface[1];
        visuals.widgets.noninteractive.weak_bg_fill = p.surface[1];
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.stroke);
        visuals.widgets.noninteractive.corner_radius = corner_radius;
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text_secondary);

        // Inactive (clickable but not hovered) — buttons, combo boxes, drag values
        // Use field_bg to give inputs a clear inset look against the panel background.
        visuals.widgets.inactive.bg_fill = p.field_bg;
        visuals.widgets.inactive.weak_bg_fill = p.field_bg;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, p.stroke);
        visuals.widgets.inactive.corner_radius = corner_radius;
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text_primary);

        // Hovered — brighter fill + accent border
        visuals.widgets.hovered.bg_fill = p.surface[3];
        visuals.widgets.hovered.weak_bg_fill = p.surface[3];
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
        visuals.widgets.hovered.corner_radius = corner_radius;
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text_primary);

        // Active (being clicked / dragged) — accent outline
        visuals.widgets.active.bg_fill = p.surface[4];
        visuals.widgets.active.weak_bg_fill = p.surface[4];
        visuals.widgets.active.bg_stroke = Stroke::new(1.5, p.accent);
        visuals.widgets.active.corner_radius = corner_radius;
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, p.text_primary);

        // Open (expanded combo box, etc.)
        visuals.widgets.open.bg_fill = p.surface[3];
        visuals.widgets.open.weak_bg_fill = p.surface[3];
        visuals.widgets.open.bg_stroke = Stroke::new(1.0, p.accent);
        visuals.widgets.open.corner_radius = corner_radius;
        visuals.widgets.open.fg_stroke = Stroke::new(1.0, p.text_primary);

        // Window shadow (basic — Step 12 adds 9-slice)
        visuals.window_shadow = egui::Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 60),
        };

        visuals.popup_shadow = egui::Shadow {
            offset: [0, 1],
            blur: 6,
            spread: 0,
            color: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 50),
        };

        // Window stroke
        visuals.window_stroke = Stroke::new(1.0, p.stroke);

        // Slider: show trailing fill so the value amount is visible
        visuals.slider_trailing_fill = true;

        // Semantic colors
        visuals.warn_fg_color = p.semantic.warning;
        visuals.error_fg_color = p.semantic.error;

        style.visuals = visuals;

        // Slider rail height — slightly thicker for visibility
        style.spacing.slider_rail_height = 4.0;

        // --- Spacing ---
        let sp = &self.spacing;
        style.spacing.item_spacing = Vec2::new(sp.item_spacing_x, sp.item_spacing_y);
        style.spacing.button_padding = Vec2::new(sp.button_padding_x, sp.button_padding_y);
        style.spacing.interact_size = Vec2::new(sp.interact_size_y * 3.0, sp.interact_size_y);
        style.spacing.indent = sp.indent;
        style.spacing.window_margin = Margin::same(sp.window_margin as i8);

        // --- Text styles ---
        style.text_styles = self.typography.text_styles();

        // --- Fonts ---
        ctx.set_fonts(super::typography::Typography::font_definitions());

        ctx.set_style(style);
    }
}
