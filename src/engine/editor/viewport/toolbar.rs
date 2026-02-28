//! Unreal Engine 5 style viewport toolbar
//!
//! Provides controls for tool selection, coordinate space, snapping, and camera speed.
//! Uses PNG icons when available, falls back to text labels.

use egui::{Color32, Ui, Vec2};

use super::camera_controller::CameraControlMode;
use super::settings::{
    GizmoOrientation, ToolMode, ViewportSettings,
    GRID_SNAP_VALUES, ROTATION_SNAP_VALUES, SCALE_SNAP_VALUES,
};
use crate::engine::editor::icons::{IconManager, ToolbarIcon};

/// Colors for the toolbar UI
mod colors {
    use egui::Color32;

    pub const BUTTON_ACTIVE: Color32 = Color32::from_rgb(70, 90, 120);
    pub const BUTTON_INACTIVE: Color32 = Color32::from_rgba_premultiplied(50, 50, 50, 200);
    pub const TOOLBAR_BG: Color32 = Color32::from_rgba_premultiplied(30, 30, 30, 230);
    pub const SEPARATOR: Color32 = Color32::from_gray(60);
}

/// Button sizes
const BUTTON_SIZE: Vec2 = Vec2::new(26.0, 24.0);
const SNAP_BUTTON_HEIGHT: f32 = 24.0;

/// Helper to render a button with icon (if available) or text fallback
/// Returns true if clicked
fn icon_tool_button(
    ui: &mut Ui,
    icon: ToolbarIcon,
    fallback_label: &str,
    selected: bool,
    tooltip: &str,
    icon_manager: Option<&IconManager>,
) -> bool {
    let fill = if selected {
        colors::BUTTON_ACTIVE
    } else {
        colors::BUTTON_INACTIVE
    };

    let corner_radius = BUTTON_SIZE.y / 2.0; // Pill shape

    // Allocate space
    let (rect, _response) = ui.allocate_exact_size(BUTTON_SIZE, egui::Sense::hover());

    // Check for click using raw input
    let pointer_pos = ui.input(|i| i.pointer.latest_pos());
    let in_rect = pointer_pos.map(|p| rect.contains(p)).unwrap_or(false);
    let primary_released = ui.input(|i| i.pointer.primary_released());
    let clicked = primary_released && in_rect;

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();

        // Draw pill-shaped background
        painter.rect_filled(rect, corner_radius, fill);

        // Try to draw icon if available
        let has_icon = icon_manager
            .and_then(|mgr| mgr.get(icon))
            .map(|texture| {
                let image_rect = rect.shrink(2.0); // Less shrink for larger icon
                let tint = if selected {
                    Color32::WHITE
                } else if in_rect {
                    Color32::from_gray(240) // Brighter on hover
                } else {
                    Color32::from_gray(210) // Brighter default for better visibility
                };
                painter.image(
                    texture.id(),
                    image_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    tint,
                );
                true
            })
            .unwrap_or(false);

        // Fallback to text if no icon
        if !has_icon {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                fallback_label,
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
        }

        // Hover effect
        if in_rect {
            painter.rect_stroke(
                rect,
                corner_radius,
                egui::Stroke::new(1.0, Color32::from_gray(100)),
                egui::StrokeKind::Outside,
            );
            // Tooltip
            egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), egui::Id::new(tooltip), |ui| {
                ui.label(tooltip);
            });
        }
    }

    clicked
}

/// Helper to render a text button with optional selection state (pill-shaped)
/// Returns true if clicked
fn tool_button(
    ui: &mut Ui,
    label: &str,
    selected: bool,
    tooltip: &str,
) -> bool {
    let fill = if selected {
        colors::BUTTON_ACTIVE
    } else {
        colors::BUTTON_INACTIVE
    };

    let corner_radius = BUTTON_SIZE.y / 2.0; // Pill shape

    // Allocate space
    let (rect, _response) = ui.allocate_exact_size(BUTTON_SIZE, egui::Sense::hover());

    // Check for click using raw input
    let pointer_pos = ui.input(|i| i.pointer.latest_pos());
    let in_rect = pointer_pos.map(|p| rect.contains(p)).unwrap_or(false);
    let primary_released = ui.input(|i| i.pointer.primary_released());
    let clicked = primary_released && in_rect;

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();

        // Draw pill-shaped background
        painter.rect_filled(rect, corner_radius, fill);

        // Draw text centered
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );

        // Hover effect
        if in_rect {
            painter.rect_stroke(
                rect,
                corner_radius,
                egui::Stroke::new(1.0, Color32::from_gray(100)),
                egui::StrokeKind::Outside,
            );
            // Tooltip
            egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), egui::Id::new(tooltip), |ui| {
                ui.label(tooltip);
            });
        }
    }

    clicked
}

/// Pill-shaped snap button with toggle on left and dropdown menu on right
/// Returns true if toggle was clicked (to toggle snap on/off)
fn snap_button_with_menu(
    ui: &mut Ui,
    icon: ToolbarIcon,
    fallback_label: &str,
    enabled: bool,
    value_text: String,
    icon_tooltip: &str,
    menu_id: &str,
    values: &[f32],
    current_value: &mut f32,
    icon_manager: Option<&IconManager>,
) -> bool {
    let icon_width = 24.0;
    let dropdown_width = 36.0;
    let total_width = icon_width + dropdown_width;
    let height = SNAP_BUTTON_HEIGHT;
    let corner_radius = height / 2.0;

    // Allocate space for the entire button group
    let (total_rect, _) = ui.allocate_exact_size(Vec2::new(total_width, height), egui::Sense::hover());

    let icon_rect = egui::Rect::from_min_size(total_rect.min, Vec2::new(icon_width, height));
    let dropdown_rect = egui::Rect::from_min_size(
        egui::pos2(total_rect.min.x + icon_width, total_rect.min.y),
        Vec2::new(dropdown_width, height),
    );

    let painter = ui.painter();

    // Draw pill-shaped background
    painter.rect_filled(total_rect, corner_radius, colors::BUTTON_INACTIVE);

    // Highlight icon side if enabled
    if enabled {
        painter.rect_filled(
            icon_rect.shrink(1.0),
            egui::CornerRadius { nw: 11, sw: 11, ne: 0, se: 0 },
            colors::BUTTON_ACTIVE,
        );
    }

    // Vertical separator
    painter.line_segment(
        [
            egui::pos2(icon_rect.right(), icon_rect.top() + 4.0),
            egui::pos2(icon_rect.right(), icon_rect.bottom() - 4.0),
        ],
        egui::Stroke::new(1.0, Color32::from_gray(60)),
    );

    // Draw icon or fallback label
    let has_icon = icon_manager
        .and_then(|mgr| mgr.get(icon))
        .map(|texture| {
            let image_rect = icon_rect.shrink(2.0); // Less shrink for larger icon
            let tint = if enabled {
                Color32::WHITE
            } else {
                Color32::from_gray(210) // Brighter for better visibility
            };
            painter.image(
                texture.id(),
                image_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                tint,
            );
            true
        })
        .unwrap_or(false);

    if !has_icon {
        painter.text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            fallback_label,
            egui::FontId::proportional(10.0),
            Color32::WHITE,
        );
    }

    // Draw value text (centered in dropdown area)
    let text_center = dropdown_rect.center() - egui::vec2(4.0, 0.0);
    painter.text(
        text_center,
        egui::Align2::CENTER_CENTER,
        &value_text,
        egui::FontId::proportional(11.0),
        Color32::WHITE,
    );

    // Draw dropdown caret
    let caret_center = egui::pos2(dropdown_rect.right() - 6.0, dropdown_rect.center().y);
    let caret_size = 3.0;
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(caret_center.x - caret_size, caret_center.y - caret_size / 2.0),
            egui::pos2(caret_center.x + caret_size, caret_center.y - caret_size / 2.0),
            egui::pos2(caret_center.x, caret_center.y + caret_size / 2.0),
        ],
        Color32::from_gray(180),
        egui::Stroke::NONE,
    ));

    // Check for clicks using raw input
    let pointer_pos = ui.input(|i| i.pointer.latest_pos());
    let in_icon = pointer_pos.map(|p| icon_rect.contains(p)).unwrap_or(false);
    let in_dropdown = pointer_pos.map(|p| dropdown_rect.contains(p)).unwrap_or(false);
    let primary_released = ui.input(|i| i.pointer.primary_released());

    let icon_clicked = primary_released && in_icon;

    // Hover effects
    if in_icon {
        painter.rect_stroke(
            icon_rect,
            egui::CornerRadius { nw: 12, sw: 12, ne: 0, se: 0 },
            egui::Stroke::new(1.0, Color32::from_gray(100)),
            egui::StrokeKind::Outside,
        );
        egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), egui::Id::new(icon_tooltip), |ui| {
            ui.label(icon_tooltip);
        });
    }
    if in_dropdown {
        painter.rect_stroke(
            dropdown_rect,
            egui::CornerRadius { nw: 0, sw: 0, ne: 12, se: 12 },
            egui::Stroke::new(1.0, Color32::from_gray(100)),
            egui::StrokeKind::Outside,
        );
    }

    // Dropdown menu using egui's popup system
    let popup_id = ui.make_persistent_id(menu_id);

    // Open popup on dropdown click
    if primary_released && in_dropdown {
        ui.memory_mut(|mem| mem.toggle_popup(popup_id));
    }

    // Show popup menu below the dropdown area
    egui::popup_below_widget(ui, popup_id, &ui.interact(dropdown_rect, popup_id.with("interact"), egui::Sense::hover()), egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(100.0);
        ui.style_mut().spacing.item_spacing.y = 1.0;

        for &value in values {
            let label = format_snap_value(value);
            let selected = (*current_value - value).abs() < 0.0001;
            if ui.selectable_label(selected, egui::RichText::new(&label).size(13.0)).clicked() {
                *current_value = value;
                ui.memory_mut(|mem| mem.close_popup(popup_id));
            }
        }
    });

    icon_clicked
}

/// Format snap value for display (removes trailing zeros, shows integers cleanly)
fn format_snap_value(value: f32) -> String {
    if value >= 1.0 && value == value.floor() {
        format!("{}", value as i32)
    } else if value >= 0.1 {
        format!("{:.2}", value).trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        format!("{}", value)
    }
}

/// Camera speed button with Unreal Engine style dropdown (slider + scalar input)
fn camera_speed_button_with_menu(
    ui: &mut Ui,
    value_text: String,
    camera_speed: &mut f32,
    camera_speed_scalar: &mut f32,
    icon_manager: Option<&IconManager>,
) {
    let icon_width = 24.0;
    let dropdown_width = 36.0;
    let total_width = icon_width + dropdown_width;
    let height = SNAP_BUTTON_HEIGHT;
    let corner_radius = height / 2.0;

    // Allocate space
    let (total_rect, _) = ui.allocate_exact_size(Vec2::new(total_width, height), egui::Sense::hover());

    let icon_rect = egui::Rect::from_min_size(total_rect.min, Vec2::new(icon_width, height));
    let dropdown_rect = egui::Rect::from_min_size(
        egui::pos2(total_rect.min.x + icon_width, total_rect.min.y),
        Vec2::new(dropdown_width, height),
    );

    let painter = ui.painter();

    // Draw pill-shaped background
    painter.rect_filled(total_rect, corner_radius, colors::BUTTON_INACTIVE);

    // Vertical separator
    painter.line_segment(
        [
            egui::pos2(icon_rect.right(), icon_rect.top() + 4.0),
            egui::pos2(icon_rect.right(), icon_rect.bottom() - 4.0),
        ],
        egui::Stroke::new(1.0, Color32::from_gray(60)),
    );

    // Draw icon or fallback label
    let has_icon = icon_manager
        .and_then(|mgr| mgr.get(ToolbarIcon::CameraSpeed))
        .map(|texture| {
            let image_rect = icon_rect.shrink(2.0); // Less shrink for larger icon
            painter.image(
                texture.id(),
                image_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::from_gray(210), // Brighter for better visibility
            );
            true
        })
        .unwrap_or(false);

    if !has_icon {
        painter.text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            "📷",
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );
    }

    // Draw value text (centered)
    let text_center = dropdown_rect.center() - egui::vec2(4.0, 0.0);
    painter.text(
        text_center,
        egui::Align2::CENTER_CENTER,
        &value_text,
        egui::FontId::proportional(11.0),
        Color32::WHITE,
    );

    // Draw dropdown caret
    let caret_center = egui::pos2(dropdown_rect.right() - 6.0, dropdown_rect.center().y);
    let caret_size = 3.0;
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(caret_center.x - caret_size, caret_center.y - caret_size / 2.0),
            egui::pos2(caret_center.x + caret_size, caret_center.y - caret_size / 2.0),
            egui::pos2(caret_center.x, caret_center.y + caret_size / 2.0),
        ],
        Color32::from_gray(180),
        egui::Stroke::NONE,
    ));

    // Check for clicks
    let pointer_pos = ui.input(|i| i.pointer.latest_pos());
    let in_dropdown = pointer_pos.map(|p| dropdown_rect.contains(p)).unwrap_or(false);
    let in_icon = pointer_pos.map(|p| icon_rect.contains(p)).unwrap_or(false);
    let primary_released = ui.input(|i| i.pointer.primary_released());

    // Hover effects
    if in_icon {
        painter.rect_stroke(
            icon_rect,
            egui::CornerRadius { nw: 12, sw: 12, ne: 0, se: 0 },
            egui::Stroke::new(1.0, Color32::from_gray(100)),
            egui::StrokeKind::Outside,
        );
        egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), egui::Id::new("camera_speed_tooltip"), |ui| {
            ui.label("Camera speed");
        });
    }
    if in_dropdown {
        painter.rect_stroke(
            dropdown_rect,
            egui::CornerRadius { nw: 0, sw: 0, ne: 12, se: 12 },
            egui::Stroke::new(1.0, Color32::from_gray(100)),
            egui::StrokeKind::Outside,
        );
    }

    // Dropdown menu - Unreal Engine style with slider and scalar
    let popup_id = ui.make_persistent_id("camera_speed_menu");

    if primary_released && in_dropdown {
        ui.memory_mut(|mem| mem.toggle_popup(popup_id));
    }

    egui::popup_below_widget(ui, popup_id, &ui.interact(dropdown_rect, popup_id.with("interact"), egui::Sense::hover()), egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(180.0);
        ui.style_mut().spacing.item_spacing.y = 6.0;

        // Header
        ui.label(egui::RichText::new("CAMERA SPEED").size(10.0).color(Color32::GRAY));
        ui.separator();

        // Camera Speed slider row with label
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Camera Speed").size(12.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::DragValue::new(camera_speed)
                    .range(0.03..=8.0)
                    .speed(0.01)
                    .fixed_decimals(2));
            });
        });

        // Custom slider with logarithmic feel
        let slider_height = 16.0;
        let (slider_rect, slider_response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), slider_height),
            egui::Sense::click_and_drag()
        );

        if ui.is_rect_visible(slider_rect) {
            let painter = ui.painter();

            // Draw track background
            let track_rect = egui::Rect::from_center_size(
                slider_rect.center(),
                egui::vec2(slider_rect.width() - 8.0, 4.0)
            );
            painter.rect_filled(track_rect, 2.0, Color32::from_gray(60));

            // Calculate handle position from value using logarithmic mapping
            let min_val = 0.03_f32;
            let max_val = 8.0_f32;
            let log_min = min_val.ln();
            let log_max = max_val.ln();
            let t = ((camera_speed.ln() - log_min) / (log_max - log_min)).clamp(0.0, 1.0);
            let handle_x = track_rect.left() + t * track_rect.width();

            // Draw handle
            let handle_radius = 6.0;
            let handle_color = if slider_response.dragged() {
                Color32::from_rgb(100, 150, 255)
            } else if slider_response.hovered() {
                Color32::from_rgb(180, 180, 180)
            } else {
                Color32::from_rgb(150, 150, 150)
            };
            painter.circle_filled(egui::pos2(handle_x, slider_rect.center().y), handle_radius, handle_color);
        }

        // Handle dragging with logarithmic mapping
        if slider_response.dragged() || slider_response.clicked() {
            if let Some(pointer_pos) = ui.input(|i| i.pointer.interact_pos()) {
                let track_left = slider_rect.left() + 4.0;
                let track_right = slider_rect.right() - 4.0;
                let track_width = track_right - track_left;

                // Calculate t from mouse x position, clamped to [0, 1]
                let t = ((pointer_pos.x - track_left) / track_width).clamp(0.0, 1.0);

                // Convert t to value using logarithmic mapping
                let min_val = 0.03_f32;
                let max_val = 8.0_f32;
                let log_min = min_val.ln();
                let log_max = max_val.ln();
                *camera_speed = (log_min + t * (log_max - log_min)).exp();
            }
        }

        ui.add_space(4.0);

        // Speed Scalar row
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Speed Scalar").size(12.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::DragValue::new(camera_speed_scalar)
                    .range(0.1..=10.0)
                    .speed(0.1)
                    .fixed_decimals(1));
            });
        });
    });
}

/// Rotation snap button with single-column dropdown
fn rotation_snap_button_with_menu(
    ui: &mut Ui,
    enabled: bool,
    value_text: String,
    current_value: &mut f32,
    icon_manager: Option<&IconManager>,
) -> bool {
    let icon_width = 24.0;
    let dropdown_width = 36.0;
    let total_width = icon_width + dropdown_width;
    let height = SNAP_BUTTON_HEIGHT;
    let corner_radius = height / 2.0;

    // Allocate space
    let (total_rect, _) = ui.allocate_exact_size(Vec2::new(total_width, height), egui::Sense::hover());

    let icon_rect = egui::Rect::from_min_size(total_rect.min, Vec2::new(icon_width, height));
    let dropdown_rect = egui::Rect::from_min_size(
        egui::pos2(total_rect.min.x + icon_width, total_rect.min.y),
        Vec2::new(dropdown_width, height),
    );

    let painter = ui.painter();

    // Draw pill-shaped background
    painter.rect_filled(total_rect, corner_radius, colors::BUTTON_INACTIVE);

    // Highlight icon side if enabled
    if enabled {
        painter.rect_filled(
            icon_rect.shrink(1.0),
            egui::CornerRadius { nw: 11, sw: 11, ne: 0, se: 0 },
            colors::BUTTON_ACTIVE,
        );
    }

    // Vertical separator
    painter.line_segment(
        [
            egui::pos2(icon_rect.right(), icon_rect.top() + 4.0),
            egui::pos2(icon_rect.right(), icon_rect.bottom() - 4.0),
        ],
        egui::Stroke::new(1.0, Color32::from_gray(60)),
    );

    // Draw icon or fallback label
    let has_icon = icon_manager
        .and_then(|mgr| mgr.get(ToolbarIcon::RotationSnap))
        .map(|texture| {
            let image_rect = icon_rect.shrink(2.0); // Less shrink for larger icon
            let tint = if enabled {
                Color32::WHITE
            } else {
                Color32::from_gray(210) // Brighter for better visibility
            };
            painter.image(
                texture.id(),
                image_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                tint,
            );
            true
        })
        .unwrap_or(false);

    if !has_icon {
        painter.text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            "∠",
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );
    }

    // Draw value text (centered)
    let text_center = dropdown_rect.center() - egui::vec2(4.0, 0.0);
    painter.text(
        text_center,
        egui::Align2::CENTER_CENTER,
        &value_text,
        egui::FontId::proportional(11.0),
        Color32::WHITE,
    );

    // Draw dropdown caret
    let caret_center = egui::pos2(dropdown_rect.right() - 6.0, dropdown_rect.center().y);
    let caret_size = 3.0;
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(caret_center.x - caret_size, caret_center.y - caret_size / 2.0),
            egui::pos2(caret_center.x + caret_size, caret_center.y - caret_size / 2.0),
            egui::pos2(caret_center.x, caret_center.y + caret_size / 2.0),
        ],
        Color32::from_gray(180),
        egui::Stroke::NONE,
    ));

    // Check for clicks
    let pointer_pos = ui.input(|i| i.pointer.latest_pos());
    let in_icon = pointer_pos.map(|p| icon_rect.contains(p)).unwrap_or(false);
    let in_dropdown = pointer_pos.map(|p| dropdown_rect.contains(p)).unwrap_or(false);
    let primary_released = ui.input(|i| i.pointer.primary_released());

    let icon_clicked = primary_released && in_icon;

    // Hover effects
    if in_icon {
        painter.rect_stroke(
            icon_rect,
            egui::CornerRadius { nw: 12, sw: 12, ne: 0, se: 0 },
            egui::Stroke::new(1.0, Color32::from_gray(100)),
            egui::StrokeKind::Outside,
        );
        egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), egui::Id::new("rotation_snap_tooltip"), |ui| {
            ui.label("Rotation snapping");
        });
    }
    if in_dropdown {
        painter.rect_stroke(
            dropdown_rect,
            egui::CornerRadius { nw: 0, sw: 0, ne: 12, se: 12 },
            egui::Stroke::new(1.0, Color32::from_gray(100)),
            egui::StrokeKind::Outside,
        );
    }

    // Dropdown menu - simple single column
    let popup_id = ui.make_persistent_id("rotation_snap_menu");

    if primary_released && in_dropdown {
        ui.memory_mut(|mem| mem.toggle_popup(popup_id));
    }

    egui::popup_below_widget(ui, popup_id, &ui.interact(dropdown_rect, popup_id.with("interact"), egui::Sense::hover()), egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(80.0);
        ui.style_mut().spacing.item_spacing.y = 1.0;

        // All rotation values in a single list
        for &value in ROTATION_SNAP_VALUES {
            let label = format!("{}°", value as i32);
            let selected = (*current_value - value).abs() < 0.0001;
            if ui.selectable_label(selected, egui::RichText::new(&label).size(13.0)).clicked() {
                *current_value = value;
                ui.memory_mut(|mem| mem.close_popup(popup_id));
            }
        }
    });

    icon_clicked
}

/// Render a small vertical separator
fn toolbar_separator(ui: &mut Ui) {
    let rect = ui.available_rect_before_wrap();
    let painter = ui.painter();
    let x = rect.left() + 4.0;
    painter.line_segment(
        [egui::pos2(x, rect.top() + 4.0), egui::pos2(x, rect.bottom() - 4.0)],
        egui::Stroke::new(1.0, colors::SEPARATOR),
    );
    ui.add_space(9.0);
}

/// Render the viewport toolbar as a floating overlay (Unreal Engine 5 style)
///
/// Single row: transform tools, snapping, camera speed, camera mode indicator.
pub fn render_viewport_toolbar_overlay(
    ctx: &egui::Context,
    viewport_rect: egui::Rect,
    settings: &mut ViewportSettings,
    camera_mode: CameraControlMode,
    icon_manager: Option<&IconManager>,
) {
    let toolbar_width = viewport_rect.width();

    egui::Area::new(egui::Id::new("viewport_toolbar_overlay"))
        .fixed_pos(egui::pos2(viewport_rect.left(), viewport_rect.top()))
        .order(egui::Order::Foreground)
        .interactable(true)
        .sense(egui::Sense::click_and_drag())
        .show(ctx, |ui| {
            ui.set_width(toolbar_width);
            egui::Frame::new()
                .fill(colors::TOOLBAR_BG)
                .corner_radius(0.0)
                .inner_margin(egui::Margin::symmetric(6, 3))
                .show(ui, |ui| {
                    ui.set_width(toolbar_width - 12.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;

                        let tools = [
                            (ToolMode::Select, ToolbarIcon::Select, "Q", "Select (Q)"),
                            (ToolMode::Translate, ToolbarIcon::Translate, "W", "Translate (W)"),
                            (ToolMode::Rotate, ToolbarIcon::Rotate, "E", "Rotate (E)"),
                            (ToolMode::Scale, ToolbarIcon::Scale, "R", "Scale (R)"),
                        ];

                        for (mode, icon, fallback, tooltip) in tools {
                            let selected = settings.tool_mode == mode;
                            if icon_tool_button(ui, icon, fallback, selected, tooltip, icon_manager) {
                                settings.tool_mode = mode;
                            }
                        }

                        toolbar_separator(ui);

                        let is_world = settings.gizmo_orientation == GizmoOrientation::World;
                        let (icon, space_label, space_tooltip) = if is_world {
                            (ToolbarIcon::World, "G", "World space (click for Local)")
                        } else {
                            (ToolbarIcon::Local, "L", "Local space (click for World)")
                        };
                        if icon_tool_button(ui, icon, space_label, false, space_tooltip, icon_manager) {
                            settings.gizmo_orientation = if is_world {
                                GizmoOrientation::Local
                            } else {
                                GizmoOrientation::World
                            };
                        }

                        toolbar_separator(ui);

                        if snap_button_with_menu(
                            ui, ToolbarIcon::GridSnap, "▦",
                            settings.grid_snap_enabled,
                            format!("{}", settings.snap_translate),
                            "Grid snapping", "grid_snap_menu",
                            GRID_SNAP_VALUES, &mut settings.snap_translate, icon_manager,
                        ) {
                            settings.grid_snap_enabled = !settings.grid_snap_enabled;
                        }

                        if rotation_snap_button_with_menu(
                            ui, settings.rotation_snap_enabled,
                            format!("{}°", settings.snap_rotate as i32),
                            &mut settings.snap_rotate, icon_manager,
                        ) {
                            settings.rotation_snap_enabled = !settings.rotation_snap_enabled;
                        }

                        if snap_button_with_menu(
                            ui, ToolbarIcon::ScaleSnap, "⊞",
                            settings.scale_snap_enabled,
                            format!("{}", settings.snap_scale),
                            "Scale snapping", "scale_snap_menu",
                            SCALE_SNAP_VALUES, &mut settings.snap_scale, icon_manager,
                        ) {
                            settings.scale_snap_enabled = !settings.scale_snap_enabled;
                        }

                        toolbar_separator(ui);

                        camera_speed_button_with_menu(
                            ui, format!("{:.2}", settings.camera_speed),
                            &mut settings.camera_speed, &mut settings.camera_speed_scalar, icon_manager,
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (text, color) = match camera_mode {
                                CameraControlMode::Fly => {
                                    (format!("Fly {:.1}x", settings.camera_speed), Color32::from_rgb(100, 200, 100))
                                }
                                CameraControlMode::Orbit => ("Orbit".to_string(), Color32::from_rgb(100, 150, 200)),
                                CameraControlMode::Pan => ("Pan".to_string(), Color32::from_rgb(200, 150, 100)),
                                CameraControlMode::LookDrag => ("Look".to_string(), Color32::from_rgb(150, 100, 200)),
                                CameraControlMode::None => return,
                            };
                            ui.label(egui::RichText::new(text).color(color).small());
                        });
                    });
                });
        });
}

/// Render a small orientation indicator cube in a corner
pub fn render_orientation_indicator(ui: &mut Ui, rect: egui::Rect, forward: [f32; 3], right: [f32; 3], up: [f32; 3]) {
    let painter = ui.painter();
    let size = 40.0;
    let padding = 8.0;

    // Position in bottom-right corner
    let center = egui::pos2(
        rect.right() - size / 2.0 - padding,
        rect.bottom() - size / 2.0 - padding,
    );

    // Background circle
    painter.circle_filled(center, size / 2.0 + 2.0, Color32::from_rgba_unmultiplied(0, 0, 0, 150));

    // Draw axes (simplified 2D projection)
    let axis_length = size / 2.0 - 4.0;

    let project = |dir: [f32; 3]| -> egui::Vec2 {
        egui::vec2(dir[0] * axis_length, -dir[1] * axis_length)
    };

    // X axis (Red)
    let x_end = center + project([forward[0], forward[1], forward[2]]);
    painter.line_segment(
        [center, x_end],
        egui::Stroke::new(2.0, Color32::from_rgb(255, 80, 80)),
    );
    painter.text(
        x_end,
        egui::Align2::CENTER_CENTER,
        "X",
        egui::FontId::proportional(10.0),
        Color32::from_rgb(255, 80, 80),
    );

    // Y axis (Green)
    let y_end = center + project([right[0], right[1], right[2]]);
    painter.line_segment(
        [center, y_end],
        egui::Stroke::new(2.0, Color32::from_rgb(80, 255, 80)),
    );
    painter.text(
        y_end,
        egui::Align2::CENTER_CENTER,
        "Y",
        egui::FontId::proportional(10.0),
        Color32::from_rgb(80, 255, 80),
    );

    // Z axis (Blue)
    let z_end = center + project([up[0], up[1], up[2]]);
    painter.line_segment(
        [center, z_end],
        egui::Stroke::new(2.0, Color32::from_rgb(80, 80, 255)),
    );
    painter.text(
        z_end,
        egui::Align2::CENTER_CENTER,
        "Z",
        egui::FontId::proportional(10.0),
        Color32::from_rgb(80, 80, 255),
    );
}
