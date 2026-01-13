//! Inspector Panel - Property editor for selected entities
//!
//! Displays and allows editing of component properties for the selected entity.

use super::Selection;
use crate::engine::ecs::{
    Camera, DirectionalLight, MeshRenderer, Name, PointLight, Transform,
};
use crate::engine::physics::{Collider, ColliderShape, RigidBody, RigidBodyType};
use egui::{Color32, CollapsingHeader, DragValue, RichText, ScrollArea, Stroke, Ui};
use hecs::{Entity, World};
use nalgebra_glm as glm;
use std::collections::{HashMap, HashSet};

/// Axis colors (industry standard: X=red, Y=green, Z=blue)
const AXIS_COLOR_X: Color32 = Color32::from_rgb(220, 80, 80);   // Red
const AXIS_COLOR_Y: Color32 = Color32::from_rgb(80, 180, 80);   // Green
const AXIS_COLOR_Z: Color32 = Color32::from_rgb(80, 120, 220);  // Blue

/// Component categories for visual grouping
#[derive(Clone, Copy)]
enum ComponentCategory {
    Core,      // Transform, Name
    Rendering, // Camera, MeshRenderer, Lights
    Physics,   // RigidBody, Collider
}

/// Actions to perform after component editing (deferred to avoid borrow issues)
enum ComponentAction {
    None,
    RemoveCamera,
    RemoveMeshRenderer,
    RemoveDirectionalLight,
    RemovePointLight,
    RemoveRigidBody,
    RemoveCollider,
}

/// Inspector Panel state
pub struct InspectorPanel {
    /// Cache for euler angles (quaternion -> euler conversion).
    /// Stores (quaternion, euler_angles) so we can detect when the quaternion
    /// has been modified externally (e.g., by the gizmo) and recompute euler.
    euler_cache: HashMap<u64, (glm::Quat, [f32; 3])>,
    /// Last known entity for euler cache invalidation
    last_entity: Option<Entity>,
    /// Track which components are collapsed
    collapsed_components: HashSet<String>,
    /// Search/filter text for components
    search_filter: String,
}

impl Default for InspectorPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl InspectorPanel {
    pub fn new() -> Self {
        Self {
            euler_cache: HashMap::new(),
            last_entity: None,
            collapsed_components: HashSet::new(),
            search_filter: String::new(),
        }
    }

    /// Check if a component matches the search filter
    fn matches_filter(&self, component_name: &str) -> bool {
        self.search_filter.is_empty()
            || component_name
                .to_lowercase()
                .contains(&self.search_filter.to_lowercase())
    }

    /// Render the inspector panel as a side panel
    pub fn show(&mut self, ctx: &egui::Context, world: &mut World, selection: &Selection) {
        egui::SidePanel::right("inspector_panel")
            .resizable(true)
            .default_width(300.0)
            .min_width(200.0)
            .show(ctx, |ui| {
                self.show_contents(ui, world, selection);
            });
    }

    /// Render just the contents (for use inside dock tabs)
    pub fn show_contents(&mut self, ui: &mut Ui, world: &mut World, selection: &Selection) {
        self.render_header(ui, selection);
        ui.separator();

        if let Some(entity) = selection.primary() {
            // Invalidate euler cache if entity changed
            if self.last_entity != Some(entity) {
                self.euler_cache.clear();
                self.last_entity = Some(entity);
            }

            // Search/filter box
            ui.horizontal(|ui| {
                ui.label("Filter:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.search_filter)
                        .hint_text("Search components...")
                        .desired_width(ui.available_width()),
                );
            });
            ui.separator();

            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.render_entity_info(ui, world, entity);
                    ui.separator();
                    self.render_components(ui, world, entity);
                    ui.separator();
                    self.render_add_component(ui, world, entity);
                });
        } else {
            self.render_empty_state(ui);
        }
    }

    /// Render panel header
    fn render_header(&self, ui: &mut Ui, selection: &Selection) {
        ui.horizontal(|ui| {
            ui.heading("Inspector");
            if selection.count() > 1 {
                ui.label(format!("({} selected)", selection.count()));
            }
        });
    }

    /// Render empty state when nothing is selected
    fn render_empty_state(&self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);
            ui.label(RichText::new("No entity selected").weak());
            ui.add_space(10.0);
            ui.label(RichText::new("Select an entity in the Hierarchy").weak());
        });
    }

    /// Render entity info header
    fn render_entity_info(&self, ui: &mut Ui, world: &World, entity: Entity) {
        let name = world
            .get::<&Name>(entity)
            .map(|n| n.0.clone())
            .unwrap_or_else(|_| format!("Entity {:?}", entity.id()));

        ui.horizontal(|ui| {
            ui.label(RichText::new(&name).strong().size(16.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("ID: {}", entity.id()))
                        .weak()
                        .small(),
                );
            });
        });
    }

    /// Component category colors
    fn category_color(category: ComponentCategory) -> Color32 {
        match category {
            ComponentCategory::Core => Color32::from_rgb(100, 160, 220),      // Blue
            ComponentCategory::Rendering => Color32::from_rgb(220, 180, 80),  // Yellow/Gold
            ComponentCategory::Physics => Color32::from_rgb(100, 180, 120),   // Green
        }
    }

    /// Draw a visual divider between component sections
    fn draw_component_divider(ui: &mut Ui) {
        ui.add_space(10.0);
        let rect = ui.available_rect_before_wrap();
        let y = rect.top();
        ui.painter().hline(
            rect.x_range(),
            y,
            Stroke::new(1.0, Color32::from_gray(70)),
        );
        ui.add_space(8.0);
    }

    /// Render all component editors
    fn render_components(&mut self, ui: &mut Ui, world: &mut World, entity: Entity) {
        let mut component_count = 0;
        let mut pending_action = ComponentAction::None;

        // === CORE COMPONENTS ===
        if self.matches_filter("name") && world.get::<&Name>(entity).is_ok() {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            self.edit_name(ui, world, entity, ComponentCategory::Core);
            component_count += 1;
        }
        if (self.matches_filter("transform") || self.matches_filter("position")
            || self.matches_filter("rotation") || self.matches_filter("scale"))
            && world.get::<&Transform>(entity).is_ok()
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            self.edit_transform(ui, world, entity, ComponentCategory::Core);
            component_count += 1;
        }

        // === RENDERING COMPONENTS ===
        if (self.matches_filter("camera") || self.matches_filter("fov"))
            && world.get::<&Camera>(entity).is_ok()
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            if let Some(action) = self.edit_camera(ui, world, entity, ComponentCategory::Rendering) {
                pending_action = action;
            }
            component_count += 1;
        }
        if (self.matches_filter("mesh") || self.matches_filter("renderer") || self.matches_filter("material"))
            && world.get::<&MeshRenderer>(entity).is_ok()
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            if let Some(action) = self.edit_mesh_renderer(ui, world, entity, ComponentCategory::Rendering) {
                pending_action = action;
            }
            component_count += 1;
        }
        if (self.matches_filter("directional") || self.matches_filter("light") || self.matches_filter("sun"))
            && world.get::<&DirectionalLight>(entity).is_ok()
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            if let Some(action) = self.edit_directional_light(ui, world, entity, ComponentCategory::Rendering) {
                pending_action = action;
            }
            component_count += 1;
        }
        if (self.matches_filter("point") || self.matches_filter("light"))
            && world.get::<&PointLight>(entity).is_ok()
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            if let Some(action) = self.edit_point_light(ui, world, entity, ComponentCategory::Rendering) {
                pending_action = action;
            }
            component_count += 1;
        }

        // === PHYSICS COMPONENTS ===
        if (self.matches_filter("rigid") || self.matches_filter("body") || self.matches_filter("physics"))
            && world.get::<&RigidBody>(entity).is_ok()
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            if let Some(action) = self.edit_rigidbody(ui, world, entity, ComponentCategory::Physics) {
                pending_action = action;
            }
            component_count += 1;
        }
        if (self.matches_filter("collider") || self.matches_filter("physics") || self.matches_filter("collision"))
            && world.get::<&Collider>(entity).is_ok()
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            if let Some(action) = self.edit_collider(ui, world, entity, ComponentCategory::Physics) {
                pending_action = action;
            }
        }

        // Execute pending action (deferred to avoid borrow conflicts)
        match pending_action {
            ComponentAction::None => {}
            ComponentAction::RemoveCamera => { let _ = world.remove_one::<Camera>(entity); }
            ComponentAction::RemoveMeshRenderer => { let _ = world.remove_one::<MeshRenderer>(entity); }
            ComponentAction::RemoveDirectionalLight => { let _ = world.remove_one::<DirectionalLight>(entity); }
            ComponentAction::RemovePointLight => { let _ = world.remove_one::<PointLight>(entity); }
            ComponentAction::RemoveRigidBody => { let _ = world.remove_one::<RigidBody>(entity); }
            ComponentAction::RemoveCollider => { let _ = world.remove_one::<Collider>(entity); }
        }
    }

    /// Edit Name component
    fn edit_name(&self, ui: &mut Ui, world: &mut World, entity: Entity, category: ComponentCategory) {
        if let Ok(mut name) = world.get::<&mut Name>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            CollapsingHeader::new(RichText::new("Name").strong())
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        let mut name_str = name.0.clone();
                        if ui.text_edit_singleline(&mut name_str).changed() {
                            name.0 = name_str;
                        }
                    });
                });

            // Draw colored accent bar on the left side of the component
            let end_y = ui.cursor().top();
            let accent_rect = egui::Rect::from_min_max(
                egui::pos2(ui.min_rect().left(), start_y),
                egui::pos2(ui.min_rect().left() + 4.0, end_y),
            );
            ui.painter().rect_filled(accent_rect, 1.0, color);
        }
    }

    /// Edit Transform component
    fn edit_transform(&mut self, ui: &mut Ui, world: &mut World, entity: Entity, category: ComponentCategory) {
        if let Ok(mut transform) = world.get::<&mut Transform>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            CollapsingHeader::new(RichText::new("Transform").strong())
                .default_open(true)
                .show(ui, |ui| {
                    // Position
                    ui.horizontal(|ui| {
                        ui.label("Position");
                        if ui.small_button("R").on_hover_text("Reset").clicked() {
                            transform.position = glm::vec3(0.0, 0.0, 0.0);
                        }
                    });
                    // Sanitize position values to prevent NaN/Infinity issues
                    if !transform.position.x.is_finite() { transform.position.x = 0.0; }
                    if !transform.position.y.is_finite() { transform.position.y = 0.0; }
                    if !transform.position.z.is_finite() { transform.position.z = 0.0; }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("X").color(AXIS_COLOR_X));
                        ui.add(DragValue::new(&mut transform.position.x).speed(0.1));
                        ui.label(RichText::new("Y").color(AXIS_COLOR_Y));
                        ui.add(DragValue::new(&mut transform.position.y).speed(0.1));
                        ui.label(RichText::new("Z").color(AXIS_COLOR_Z));
                        ui.add(DragValue::new(&mut transform.position.z).speed(0.1));
                    });

                    // Rotation (as Euler angles in degrees)
                    ui.horizontal(|ui| {
                        ui.label("Rotation");
                        if ui.small_button("R").on_hover_text("Reset").clicked() {
                            transform.rotation = glm::quat_identity();
                            self.euler_cache.insert(entity.id() as u64, (glm::quat_identity(), [0.0, 0.0, 0.0]));
                        }
                    });

                    // Get or calculate euler angles.
                    // The cache stores (quaternion, euler) pairs so we can detect when
                    // the quaternion has been modified externally (e.g., by the gizmo).
                    let entity_id = entity.id() as u64;
                    let needs_recompute = match self.euler_cache.get(&entity_id) {
                        Some((cached_quat, _)) => !quaternions_approximately_equal(cached_quat, &transform.rotation),
                        None => true,
                    };
                    if needs_recompute {
                        let new_euler = quaternion_to_euler_degrees(&transform.rotation);
                        self.euler_cache.insert(entity_id, (transform.rotation.clone(), new_euler));
                    }

                    // Get a copy of euler to work with (avoids borrow issues with closure)
                    let mut euler = self.euler_cache.get(&entity_id).unwrap().1;

                    // Sanitize cached euler values to prevent DragValue crash
                    for i in 0..3 {
                        if !euler[i].is_finite() {
                            euler[i] = 0.0;
                        }
                    }

                    let mut euler_changed = false;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("X").color(AXIS_COLOR_X));
                        let response_x = ui.add(DragValue::new(&mut euler[0]).speed(1.0).suffix("°").range(-180.0..=180.0));
                        ui.label(RichText::new("Y").color(AXIS_COLOR_Y));
                        let response_y = ui.add(DragValue::new(&mut euler[1]).speed(1.0).suffix("°").range(-180.0..=180.0));
                        ui.label(RichText::new("Z").color(AXIS_COLOR_Z));
                        let response_z = ui.add(DragValue::new(&mut euler[2]).speed(1.0).suffix("°").range(-180.0..=180.0));

                        euler_changed = response_x.changed() || response_y.changed() || response_z.changed();
                    });

                    if euler_changed {
                        let new_quat = euler_degrees_to_quaternion(&euler);
                        transform.rotation = new_quat.clone();
                        // Update cache with new quaternion so we don't recompute euler next frame
                        self.euler_cache.insert(entity_id, (new_quat, euler));
                    }

                    // Scale
                    ui.horizontal(|ui| {
                        ui.label("Scale");
                        if ui.small_button("R").on_hover_text("Reset").clicked() {
                            transform.scale = glm::vec3(1.0, 1.0, 1.0);
                        }
                    });
                    // Sanitize scale values to prevent DragValue crash
                    if !transform.scale.x.is_finite() { transform.scale.x = 1.0; }
                    if !transform.scale.y.is_finite() { transform.scale.y = 1.0; }
                    if !transform.scale.z.is_finite() { transform.scale.z = 1.0; }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("X").color(AXIS_COLOR_X));
                        ui.add(DragValue::new(&mut transform.scale.x).speed(0.01).range(0.001..=1000.0));
                        ui.label(RichText::new("Y").color(AXIS_COLOR_Y));
                        ui.add(DragValue::new(&mut transform.scale.y).speed(0.01).range(0.001..=1000.0));
                        ui.label(RichText::new("Z").color(AXIS_COLOR_Z));
                        ui.add(DragValue::new(&mut transform.scale.z).speed(0.01).range(0.001..=1000.0));
                    });
                });

            // Draw colored accent bar
            let end_y = ui.cursor().top();
            let accent_rect = egui::Rect::from_min_max(
                egui::pos2(ui.min_rect().left(), start_y),
                egui::pos2(ui.min_rect().left() + 4.0, end_y),
            );
            ui.painter().rect_filled(accent_rect, 1.0, color);
        }
    }

    /// Edit Camera component
    fn edit_camera(&self, ui: &mut Ui, world: &mut World, entity: Entity, category: ComponentCategory) -> Option<ComponentAction> {
        let mut action = None;
        if let Ok(mut camera) = world.get::<&mut Camera>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            let header = CollapsingHeader::new(RichText::new("Camera").strong())
                .default_open(true)
                .show(ui, |ui| {
                    ui.checkbox(&mut camera.active, "Active")
                        .on_hover_text("Whether this camera is currently rendering");

                    // Sanitize camera values to prevent DragValue crash
                    if !camera.fov.is_finite() { camera.fov = 60.0; }
                    if !camera.near.is_finite() || camera.near <= 0.0 { camera.near = 0.1; }
                    if !camera.far.is_finite() || camera.far <= camera.near { camera.far = 1000.0; }

                    // FOV slider (30-120 degrees)
                    ui.horizontal(|ui| {
                        ui.label("FOV:");
                        ui.add(
                            egui::Slider::new(&mut camera.fov, 30.0..=120.0)
                                .suffix("°")
                                .clamping(egui::SliderClamping::Always),
                        )
                        .on_hover_text("Field of view angle. Wider = more visible area");
                    });

                    // Near/Far planes - calculate safe dynamic ranges
                    let near_max = (camera.far - 0.001).max(0.002);
                    let far_min = (camera.near + 0.001).min(99999.0);

                    ui.horizontal(|ui| {
                        ui.label("Near:");
                        ui.add(
                            DragValue::new(&mut camera.near)
                                .speed(0.01)
                                .range(0.001..=near_max),
                        )
                        .on_hover_text("Near clipping plane. Objects closer than this won't render");
                    });
                    ui.horizontal(|ui| {
                        ui.label("Far:");
                        ui.add(
                            DragValue::new(&mut camera.far)
                                .speed(1.0)
                                .range(far_min..=100000.0),
                        )
                        .on_hover_text("Far clipping plane. Objects farther than this won't render");
                    });
                });

            // Context menu for component removal
            header.header_response.context_menu(|ui| {
                if ui.button("Remove Component").clicked() {
                    action = Some(ComponentAction::RemoveCamera);
                    ui.close();
                }
            });

            // Draw colored accent bar
            let end_y = ui.cursor().top();
            let accent_rect = egui::Rect::from_min_max(
                egui::pos2(ui.min_rect().left(), start_y),
                egui::pos2(ui.min_rect().left() + 4.0, end_y),
            );
            ui.painter().rect_filled(accent_rect, 1.0, color);
        }
        action
    }

    /// Edit MeshRenderer component
    fn edit_mesh_renderer(&self, ui: &mut Ui, world: &mut World, entity: Entity, category: ComponentCategory) -> Option<ComponentAction> {
        let mut action = None;
        if let Ok(mut renderer) = world.get::<&mut MeshRenderer>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            let header = CollapsingHeader::new(RichText::new("Mesh Renderer").strong())
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Mesh Index:");
                        ui.add(DragValue::new(&mut renderer.mesh_index).range(0..=1000));
                    });

                    ui.horizontal(|ui| {
                        ui.label("Material Index:");
                        ui.add(DragValue::new(&mut renderer.material_index).range(0..=1000));
                    });

                    ui.label(
                        RichText::new("Tip: Use Asset Browser for mesh/material selection")
                            .color(Color32::from_gray(160)),
                    );
                });

            // Context menu for component removal
            header.header_response.context_menu(|ui| {
                if ui.button("Remove Component").clicked() {
                    action = Some(ComponentAction::RemoveMeshRenderer);
                    ui.close();
                }
            });

            // Draw colored accent bar
            let end_y = ui.cursor().top();
            let accent_rect = egui::Rect::from_min_max(
                egui::pos2(ui.min_rect().left(), start_y),
                egui::pos2(ui.min_rect().left() + 4.0, end_y),
            );
            ui.painter().rect_filled(accent_rect, 1.0, color);
        }
        action
    }

    /// Edit DirectionalLight component
    fn edit_directional_light(&self, ui: &mut Ui, world: &mut World, entity: Entity, category: ComponentCategory) -> Option<ComponentAction> {
        let mut action = None;
        if let Ok(mut light) = world.get::<&mut DirectionalLight>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            let header = CollapsingHeader::new(RichText::new("Directional Light").strong())
                .default_open(true)
                .show(ui, |ui| {
                    // Direction
                    ui.label("Direction:").on_hover_text("Direction the light is pointing (normalized)");
                    // Sanitize direction to prevent DragValue crash
                    if !light.direction.x.is_finite() { light.direction.x = 0.0; }
                    if !light.direction.y.is_finite() { light.direction.y = -1.0; }
                    if !light.direction.z.is_finite() { light.direction.z = 0.0; }
                    ui.horizontal(|ui| {
                        ui.add(DragValue::new(&mut light.direction.x).prefix("X: ").speed(0.01).range(-1.0..=1.0));
                        ui.add(DragValue::new(&mut light.direction.y).prefix("Y: ").speed(0.01).range(-1.0..=1.0));
                        ui.add(DragValue::new(&mut light.direction.z).prefix("Z: ").speed(0.01).range(-1.0..=1.0));
                    });

                    // Normalize direction (safe - values are now bounded)
                    let len = glm::length(&light.direction);
                    if len > 0.001 {
                        light.direction = light.direction / len;
                    }

                    // Color (RGB)
                    ui.horizontal(|ui| {
                        ui.label("Color:");
                        let mut color = [light.color.x, light.color.y, light.color.z];
                        if ui.color_edit_button_rgb(&mut color)
                            .on_hover_text("Light color")
                            .changed()
                        {
                            light.color = glm::vec3(color[0], color[1], color[2]);
                        }
                    });

                    // Intensity
                    ui.horizontal(|ui| {
                        ui.label("Intensity:");
                        ui.add(
                            egui::Slider::new(&mut light.intensity, 0.0..=10.0)
                                .clamping(egui::SliderClamping::Always),
                        )
                        .on_hover_text("Light brightness multiplier");
                    });
                });

            // Context menu for component removal
            header.header_response.context_menu(|ui| {
                if ui.button("Remove Component").clicked() {
                    action = Some(ComponentAction::RemoveDirectionalLight);
                    ui.close();
                }
            });

            // Draw colored accent bar
            let end_y = ui.cursor().top();
            let accent_rect = egui::Rect::from_min_max(
                egui::pos2(ui.min_rect().left(), start_y),
                egui::pos2(ui.min_rect().left() + 4.0, end_y),
            );
            ui.painter().rect_filled(accent_rect, 1.0, color);
        }
        action
    }

    /// Edit PointLight component
    fn edit_point_light(&self, ui: &mut Ui, world: &mut World, entity: Entity, category: ComponentCategory) -> Option<ComponentAction> {
        let mut action = None;
        if let Ok(mut light) = world.get::<&mut PointLight>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            let header = CollapsingHeader::new(RichText::new("Point Light").strong())
                .default_open(true)
                .show(ui, |ui| {
                    // Sanitize light values to prevent DragValue crash
                    if !light.intensity.is_finite() { light.intensity = 1.0; }
                    if !light.radius.is_finite() { light.radius = 10.0; }

                    // Color
                    ui.horizontal(|ui| {
                        ui.label("Color:");
                        let mut color = [light.color.x, light.color.y, light.color.z];
                        if ui.color_edit_button_rgb(&mut color)
                            .on_hover_text("Light color")
                            .changed()
                        {
                            light.color = glm::vec3(color[0], color[1], color[2]);
                        }
                    });

                    // Intensity
                    ui.horizontal(|ui| {
                        ui.label("Intensity:");
                        ui.add(
                            egui::Slider::new(&mut light.intensity, 0.0..=100.0)
                                .clamping(egui::SliderClamping::Always),
                        )
                        .on_hover_text("Light brightness multiplier");
                    });

                    // Radius
                    ui.horizontal(|ui| {
                        ui.label("Radius:");
                        ui.add(
                            DragValue::new(&mut light.radius)
                                .speed(0.1)
                                .range(0.1..=1000.0),
                        )
                        .on_hover_text("Maximum distance the light reaches");
                    });
                });

            // Context menu for component removal
            header.header_response.context_menu(|ui| {
                if ui.button("Remove Component").clicked() {
                    action = Some(ComponentAction::RemovePointLight);
                    ui.close();
                }
            });

            // Draw colored accent bar
            let end_y = ui.cursor().top();
            let accent_rect = egui::Rect::from_min_max(
                egui::pos2(ui.min_rect().left(), start_y),
                egui::pos2(ui.min_rect().left() + 4.0, end_y),
            );
            ui.painter().rect_filled(accent_rect, 1.0, color);
        }
        action
    }

    /// Edit RigidBody component
    fn edit_rigidbody(&self, ui: &mut Ui, world: &mut World, entity: Entity, category: ComponentCategory) -> Option<ComponentAction> {
        let mut action = None;
        if let Ok(mut rb) = world.get::<&mut RigidBody>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            let header = CollapsingHeader::new(RichText::new("Rigid Body").strong())
                .default_open(true)
                .show(ui, |ui| {
                    // Sanitize rigidbody values to prevent DragValue crash
                    if !rb.mass.is_finite() { rb.mass = 1.0; }
                    if !rb.linear_damping.is_finite() { rb.linear_damping = 0.0; }
                    if !rb.angular_damping.is_finite() { rb.angular_damping = 0.0; }

                    // Body type dropdown
                    ui.horizontal(|ui| {
                        ui.label("Type:").on_hover_text(
                            "Dynamic: Affected by forces\nKinematic: Moved by code only\nStatic: Never moves",
                        );
                        egui::ComboBox::from_id_salt("rb_type")
                            .selected_text(format!("{:?}", rb.body_type))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut rb.body_type,
                                    RigidBodyType::Dynamic,
                                    "Dynamic",
                                );
                                ui.selectable_value(
                                    &mut rb.body_type,
                                    RigidBodyType::Kinematic,
                                    "Kinematic",
                                );
                                ui.selectable_value(
                                    &mut rb.body_type,
                                    RigidBodyType::Static,
                                    "Static",
                                );
                            });
                    });

                    // Mass (only for dynamic bodies)
                    if rb.body_type == RigidBodyType::Dynamic {
                        ui.horizontal(|ui| {
                            ui.label("Mass:");
                            ui.add(
                                DragValue::new(&mut rb.mass)
                                    .speed(0.1)
                                    .range(0.001..=10000.0),
                            )
                            .on_hover_text("Object weight. Affects momentum and collision response");
                        });
                    }

                    // Damping
                    ui.horizontal(|ui| {
                        ui.label("Linear Damping:");
                        ui.add(
                            egui::Slider::new(&mut rb.linear_damping, 0.0..=10.0)
                                .clamping(egui::SliderClamping::Always),
                        )
                        .on_hover_text("Air resistance. Higher values slow movement faster");
                    });

                    ui.horizontal(|ui| {
                        ui.label("Angular Damping:");
                        ui.add(
                            egui::Slider::new(&mut rb.angular_damping, 0.0..=10.0)
                                .clamping(egui::SliderClamping::Always),
                        )
                        .on_hover_text("Rotational resistance. Higher values stop spinning faster");
                    });

                    ui.checkbox(&mut rb.can_sleep, "Can Sleep")
                        .on_hover_text("Allow physics to deactivate this body when at rest");
                });

            // Context menu for component removal
            header.header_response.context_menu(|ui| {
                if ui.button("Remove Component").clicked() {
                    action = Some(ComponentAction::RemoveRigidBody);
                    ui.close();
                }
            });

            // Draw colored accent bar
            let end_y = ui.cursor().top();
            let accent_rect = egui::Rect::from_min_max(
                egui::pos2(ui.min_rect().left(), start_y),
                egui::pos2(ui.min_rect().left() + 4.0, end_y),
            );
            ui.painter().rect_filled(accent_rect, 1.0, color);
        }
        action
    }

    /// Edit Collider component
    fn edit_collider(&self, ui: &mut Ui, world: &mut World, entity: Entity, category: ComponentCategory) -> Option<ComponentAction> {
        let mut action = None;
        if let Ok(mut collider) = world.get::<&mut Collider>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            let header = CollapsingHeader::new(RichText::new("Collider").strong())
                .default_open(true)
                .show(ui, |ui| {
                    // Shape type (read-only display)
                    ui.horizontal(|ui| {
                        ui.label("Shape:").on_hover_text("Collision shape geometry");
                        match &collider.shape {
                            ColliderShape::Cuboid { half_extents } => {
                                ui.label(format!(
                                    "Cuboid ({:.2}, {:.2}, {:.2})",
                                    half_extents.x, half_extents.y, half_extents.z
                                ));
                            }
                            ColliderShape::Ball { radius } => {
                                ui.label(format!("Ball (r={:.2})", radius));
                            }
                            ColliderShape::Capsule { half_height, radius } => {
                                ui.label(format!("Capsule (h={:.2}, r={:.2})", half_height, radius));
                            }
                        }
                    });

                    // Friction
                    ui.horizontal(|ui| {
                        ui.label("Friction:");
                        ui.add(
                            egui::Slider::new(&mut collider.friction, 0.0..=2.0)
                                .clamping(egui::SliderClamping::Always),
                        )
                        .on_hover_text("Surface grip. 0 = ice, 1 = rubber, 2 = very sticky");
                    });

                    // Restitution (bounciness)
                    ui.horizontal(|ui| {
                        ui.label("Restitution:");
                        ui.add(
                            egui::Slider::new(&mut collider.restitution, 0.0..=1.0)
                                .clamping(egui::SliderClamping::Always),
                        )
                        .on_hover_text("Bounciness. 0 = no bounce, 1 = perfect bounce");
                    });

                    ui.checkbox(&mut collider.is_sensor, "Is Sensor (Trigger)")
                        .on_hover_text("Detects overlaps without physical collision");
                });

            // Context menu for component removal
            header.header_response.context_menu(|ui| {
                if ui.button("Remove Component").clicked() {
                    action = Some(ComponentAction::RemoveCollider);
                    ui.close();
                }
            });

            // Draw colored accent bar
            let end_y = ui.cursor().top();
            let accent_rect = egui::Rect::from_min_max(
                egui::pos2(ui.min_rect().left(), start_y),
                egui::pos2(ui.min_rect().left() + 4.0, end_y),
            );
            ui.painter().rect_filled(accent_rect, 1.0, color);
        }
        action
    }

    /// Render "Add Component" UI
    fn render_add_component(&self, ui: &mut Ui, world: &mut World, entity: Entity) {
        ui.add_space(10.0);

        egui::ComboBox::from_label("")
            .selected_text("Add Component...")
            .show_ui(ui, |ui| {
                // Only show components the entity doesn't have
                if world.get::<&Camera>(entity).is_err() {
                    if ui.selectable_label(false, "Camera").clicked() {
                        let _ = world.insert_one(entity, Camera::default());
                    }
                }

                if world.get::<&DirectionalLight>(entity).is_err() {
                    if ui.selectable_label(false, "Directional Light").clicked() {
                        let _ = world.insert_one(entity, DirectionalLight::default());
                    }
                }

                if world.get::<&PointLight>(entity).is_err() {
                    if ui.selectable_label(false, "Point Light").clicked() {
                        let _ = world.insert_one(entity, PointLight::default());
                    }
                }

                if world.get::<&MeshRenderer>(entity).is_err() {
                    if ui.selectable_label(false, "Mesh Renderer").clicked() {
                        let _ = world.insert_one(
                            entity,
                            MeshRenderer {
                                mesh_index: 0,
                                material_index: 0,
                            },
                        );
                    }
                }

                if world.get::<&RigidBody>(entity).is_err() {
                    if ui.selectable_label(false, "Rigid Body").clicked() {
                        let _ = world.insert_one(entity, RigidBody::default());
                    }
                }

                if world.get::<&Collider>(entity).is_err() {
                    if ui.selectable_label(false, "Collider").clicked() {
                        let _ = world.insert_one(entity, Collider::default());
                    }
                }
            });
    }
}

/// Convert quaternion to Euler angles (degrees)
///
/// Guards against NaN values that can occur from numerical edge cases
/// in the quaternion-to-euler conversion (e.g., gimbal lock regions).
/// Normalizes the quaternion first to prevent denormalization drift.
fn quaternion_to_euler_degrees(q: &glm::Quat) -> [f32; 3] {
    // Normalize quaternion to prevent NaN from denormalized quats
    let q_norm = glm::quat_normalize(q);
    let euler = glm::quat_euler_angles(&q_norm);
    [
        if euler.x.is_finite() { euler.x.to_degrees() } else { 0.0 },
        if euler.y.is_finite() { euler.y.to_degrees() } else { 0.0 },
        if euler.z.is_finite() { euler.z.to_degrees() } else { 0.0 },
    ]
}

/// Convert Euler angles (degrees) to quaternion
fn euler_degrees_to_quaternion(euler: &[f32; 3]) -> glm::Quat {
    let rad_x = euler[0].to_radians();
    let rad_y = euler[1].to_radians();
    let rad_z = euler[2].to_radians();

    // Build quaternion from euler angles (XYZ order)
    let qx = glm::quat_angle_axis(rad_x, &glm::vec3(1.0, 0.0, 0.0));
    let qy = glm::quat_angle_axis(rad_y, &glm::vec3(0.0, 1.0, 0.0));
    let qz = glm::quat_angle_axis(rad_z, &glm::vec3(0.0, 0.0, 1.0));

    // Normalize to prevent drift accumulation
    glm::quat_normalize(&(qz * qy * qx))
}

/// Check if two quaternions represent approximately the same rotation.
/// Accounts for q and -q representing the same rotation.
fn quaternions_approximately_equal(a: &glm::Quat, b: &glm::Quat) -> bool {
    let dot = (a.coords.x * b.coords.x
        + a.coords.y * b.coords.y
        + a.coords.z * b.coords.z
        + a.coords.w * b.coords.w)
        .abs();
    dot > 0.9999
}
