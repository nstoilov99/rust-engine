//! Dev-only UI showcase window — gated behind `editor-debug` feature.
//!
//! Displays every custom widget in every state for visual verification
//! and manual testing of keyboard navigation and theme tokens.

use egui::{Context, Ui};

use crate::engine::editor::theme::StateKind;
use crate::engine::editor::widgets::{
    button::{themed_button, themed_button_variant, ButtonVariant},
    empty_state::{self, EmptyState},
    field_row::field_row,
    panel_header::panel_header,
    search_field::search_field,
    slider_with_input::slider_with_input,
    tab_bar::tab_bar,
    toggle_switch::toggle_switch,
    tree_row::{tree_row, TreeRowConfig},
    IconKind, UiExt,
};

/// Pages in the showcase window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShowcasePage {
    #[default]
    Buttons,
    Toggles,
    Sliders,
    FieldRows,
    PanelHeaders,
    TabBars,
    TreeRows,
    AssetSlots,
    SearchFields,
    EmptyStates,
    Density,
    ContrastVerification,
}

impl ShowcasePage {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Buttons => "Buttons",
            Self::Toggles => "Toggles",
            Self::Sliders => "Sliders",
            Self::FieldRows => "Field Rows",
            Self::PanelHeaders => "Panel Headers",
            Self::TabBars => "Tab Bars",
            Self::TreeRows => "Tree Rows",
            Self::AssetSlots => "Asset Slots",
            Self::SearchFields => "Search Fields",
            Self::EmptyStates => "Empty States",
            Self::Density => "Density",
            Self::ContrastVerification => "Contrast",
        }
    }

    pub const ALL: &'static [ShowcasePage] = &[
        Self::Buttons,
        Self::Toggles,
        Self::Sliders,
        Self::FieldRows,
        Self::PanelHeaders,
        Self::TabBars,
        Self::TreeRows,
        Self::AssetSlots,
        Self::SearchFields,
        Self::EmptyStates,
        Self::Density,
        Self::ContrastVerification,
    ];
}

/// Persistent state for the showcase window.
pub struct ShowcaseWindow {
    pub open: bool,
    active_page: ShowcasePage,
    // Widget demo state
    toggle_a: bool,
    toggle_b: bool,
    slider_val: f32,
    search_query: String,
    tab_idx: usize,
    header_open_a: bool,
    header_open_b: bool,
    field_float: f32,
    field_string: String,
}

impl Default for ShowcaseWindow {
    fn default() -> Self {
        Self {
            open: false,
            active_page: ShowcasePage::Buttons,
            toggle_a: false,
            toggle_b: true,
            slider_val: 0.5,
            search_query: String::new(),
            tab_idx: 0,
            header_open_a: true,
            header_open_b: false,
            field_float: 3.5,
            field_string: "Hello".to_string(),
        }
    }
}

impl ShowcaseWindow {
    pub fn show(&mut self, ctx: &Context) {
        if !self.open {
            return;
        }

        let mut open = self.open;
        egui::Window::new("Widget Showcase")
            .open(&mut open)
            .default_width(500.0)
            .default_height(600.0)
            .show(ctx, |ui| {
                // Page selector
                ui.horizontal(|ui| {
                    for page in ShowcasePage::ALL {
                        if ui
                            .selectable_label(self.active_page == *page, page.label())
                            .clicked()
                        {
                            self.active_page = *page;
                        }
                    }
                });
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| match self.active_page {
                    ShowcasePage::Buttons => self.show_buttons(ui),
                    ShowcasePage::Toggles => self.show_toggles(ui),
                    ShowcasePage::Sliders => self.show_sliders(ui),
                    ShowcasePage::FieldRows => self.show_field_rows(ui),
                    ShowcasePage::PanelHeaders => self.show_panel_headers(ui),
                    ShowcasePage::TabBars => self.show_tab_bars(ui),
                    ShowcasePage::TreeRows => self.show_tree_rows(ui),
                    ShowcasePage::AssetSlots => self.show_asset_slots(ui),
                    ShowcasePage::SearchFields => self.show_search_fields(ui),
                    ShowcasePage::EmptyStates => self.show_empty_states(ui),
                    ShowcasePage::Density => self.show_density(ui),
                    ShowcasePage::ContrastVerification => self.show_contrast(ui),
                });
            });
        self.open = open;
    }

    fn show_buttons(&mut self, ui: &mut Ui) {
        ui.heading("Buttons");
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            themed_button(ui, "Primary");
            themed_button_variant(ui, ButtonVariant::Secondary, "Secondary");
            themed_button_variant(ui, ButtonVariant::Danger, "Danger");
        });
        ui.add_space(4.0);

        ui.add_enabled_ui(false, |ui| {
            ui.horizontal(|ui| {
                themed_button(ui, "Disabled Primary");
                themed_button_variant(ui, ButtonVariant::Secondary, "Disabled Secondary");
            });
        });
    }

    fn show_toggles(&mut self, ui: &mut Ui) {
        ui.heading("Toggle Switches");
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label("Off: ");
            toggle_switch(ui, &mut self.toggle_a);
        });
        ui.horizontal(|ui| {
            ui.label("On: ");
            toggle_switch(ui, &mut self.toggle_b);
        });
    }

    fn show_sliders(&mut self, ui: &mut Ui) {
        ui.heading("Sliders");
        ui.add_space(8.0);
        slider_with_input(ui, &mut self.slider_val, 0.0..=1.0, "Value");
    }

    fn show_field_rows(&mut self, ui: &mut Ui) {
        ui.heading("Field Rows");
        ui.add_space(8.0);

        field_row(ui, "Float Value", |ui| {
            ui.add(egui::DragValue::new(&mut self.field_float).speed(0.01))
        });

        field_row(ui, "String Value", |ui| {
            ui.text_edit_singleline(&mut self.field_string)
        });
    }

    fn show_panel_headers(&mut self, ui: &mut Ui) {
        ui.heading("Panel Headers");
        ui.add_space(8.0);

        panel_header(
            ui,
            "Transform",
            Some(egui::Color32::from_rgb(66, 133, 244)),
            &mut self.header_open_a,
            |ui| {
                ui.label("Position: (0, 0, 0)");
                ui.label("Rotation: (0, 0, 0)");
                ui.label("Scale: (1, 1, 1)");
            },
        );

        panel_header(
            ui,
            "Physics",
            Some(egui::Color32::from_rgb(52, 168, 83)),
            &mut self.header_open_b,
            |ui| {
                ui.label("Mass: 1.0 kg");
                ui.label("Gravity: enabled");
            },
        );
    }

    fn show_tab_bars(&mut self, ui: &mut Ui) {
        ui.heading("Tab Bars");
        ui.add_space(8.0);
        tab_bar(ui, &["Scene", "Game", "Animation"], &mut self.tab_idx);
    }

    fn show_tree_rows(&mut self, ui: &mut Ui) {
        ui.heading("Tree Rows");
        ui.add_space(8.0);

        tree_row(
            ui,
            &TreeRowConfig {
                depth: 0,
                expanded: true,
                has_children: true,
                selected: false,
                icon: IconKind::Folder,
                label: "Root Entity".to_string(),
                draggable: true,
                override_dot: false,
            },
        );
        tree_row(
            ui,
            &TreeRowConfig {
                depth: 1,
                expanded: false,
                has_children: true,
                selected: true,
                icon: IconKind::Mesh,
                label: "Player Mesh".to_string(),
                draggable: true,
                override_dot: true,
            },
        );
        tree_row(
            ui,
            &TreeRowConfig {
                depth: 1,
                expanded: false,
                has_children: false,
                selected: false,
                icon: IconKind::Light,
                label: "Point Light".to_string(),
                draggable: true,
                override_dot: false,
            },
        );
        tree_row(
            ui,
            &TreeRowConfig {
                depth: 2,
                expanded: false,
                has_children: false,
                selected: false,
                icon: IconKind::Camera,
                label: "Main Camera".to_string(),
                draggable: false,
                override_dot: false,
            },
        );
    }

    fn show_asset_slots(&mut self, ui: &mut Ui) {
        ui.heading("Asset Slots");
        ui.add_space(8.0);

        crate::engine::editor::widgets::asset_slot::asset_slot(
            ui,
            "Material",
            Some("default.material.ron"),
        );
        ui.add_space(4.0);
        crate::engine::editor::widgets::asset_slot::asset_slot(ui, "Mesh", None);
    }

    fn show_search_fields(&mut self, ui: &mut Ui) {
        ui.heading("Search Fields");
        ui.add_space(8.0);
        search_field(ui, &mut self.search_query);
    }

    fn show_empty_states(&mut self, ui: &mut Ui) {
        ui.heading("Empty States");
        ui.add_space(8.0);

        empty_state::empty_state(
            ui,
            &EmptyState::new("No entities in scene")
                .with_icon(IconKind::Empty)
                .with_subtitle("Drag an asset here or create from File menu")
                .with_action("Create Entity"),
        );
    }

    fn show_density(&mut self, ui: &mut Ui) {
        ui.heading("Density Comparison");
        ui.add_space(8.0);
        ui.label("Current density mode is applied globally via EditorServices.");
        ui.label("Toggle between Compact and Comfortable in the View menu.");
    }

    fn show_contrast(&mut self, ui: &mut Ui) {
        ui.heading("Contrast Verification");
        ui.add_space(8.0);

        let theme = ui.theme();
        let issues = theme.verify_wcag_aa();

        if issues.is_empty() {
            ui.colored_label(
                theme.palette.semantic.success,
                "\u{2713} All text/background pairs pass WCAG AA",
            );
        } else {
            ui.colored_label(
                theme.palette.semantic.error,
                format!("\u{2717} {} failing pairs:", issues.len()),
            );
            for issue in &issues {
                ui.label(format!("  {issue}"));
            }
        }

        ui.add_space(16.0);
        ui.heading("State Colors");
        ui.add_space(4.0);

        for kind in &[
            StateKind::Error,
            StateKind::Warning,
            StateKind::Success,
            StateKind::Info,
            StateKind::Mixed,
            StateKind::Overridden,
            StateKind::Disabled,
        ] {
            let color = theme.state_color(*kind);
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 2, color);
                ui.label(format!("{kind:?}"));
            });
        }
    }
}
