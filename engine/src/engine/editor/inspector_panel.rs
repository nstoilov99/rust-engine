//! Inspector Panel - Property editor for selected entities
//!
//! Displays and allows editing of component properties for the selected entity.

use super::asset_browser::AssetBrowserPanel;
use super::Selection;
use crate::engine::assets::asset_type::AssetType;
use crate::engine::assets::handle::AssetId;
use crate::engine::ecs::resources::PlayMode;
use crate::engine::ecs::{
    Camera, CameraProjection, DirectionalLight, LightFalloff, MeshRenderer, Name, PointLight,
    Transform, ParticleEffect, SpawnShape, UpdateModule,
};
use crate::engine::animation::{AnimationPlayer, PlaybackState, SkeletonInstance};
use crate::engine::audio::{AudioBus, AudioEmitter, AudioListener};
use crate::engine::physics::{Collider, ColliderShape, RigidBody, RigidBodyType};
use egui::{CollapsingHeader, Color32, DragValue, RichText, ScrollArea, Stroke, Ui};
use hecs::{Entity, World};
use nalgebra_glm as glm;
use std::collections::{HashMap, HashSet};

/// Axis colors (industry standard: X=red, Y=green, Z=blue)
const AXIS_COLOR_X: Color32 = Color32::from_rgb(220, 80, 80); // Red
const AXIS_COLOR_Y: Color32 = Color32::from_rgb(80, 180, 80); // Green
const AXIS_COLOR_Z: Color32 = Color32::from_rgb(80, 120, 220); // Blue

/// Component categories for visual grouping
#[derive(Clone, Copy)]
enum ComponentCategory {
    Core,      // Transform, Name
    Rendering, // Camera, MeshRenderer, Lights
    Physics,   // RigidBody, Collider
    Animation, // SkeletonInstance, AnimationPlayer
    Audio,     // AudioEmitter, AudioListener
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
    RemoveAudioEmitter,
    RemoveAudioListener,
    RemoveParticleEffect,
}

/// Bitflags for which inspectable components an entity has.
/// Computed once per selection change / component mutation, not per frame.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct ComponentPresence {
    bits: u16,
}

impl ComponentPresence {
    const NAME: u16 = 1 << 0;
    const TRANSFORM: u16 = 1 << 1;
    const CAMERA: u16 = 1 << 2;
    const MESH_RENDERER: u16 = 1 << 3;
    const DIR_LIGHT: u16 = 1 << 4;
    const POINT_LIGHT: u16 = 1 << 5;
    const RIGID_BODY: u16 = 1 << 6;
    const COLLIDER: u16 = 1 << 7;
    const SKELETON: u16 = 1 << 8;
    const ANIM_PLAYER: u16 = 1 << 9;
    const AUDIO_EMITTER: u16 = 1 << 10;
    const AUDIO_LISTENER: u16 = 1 << 11;
    const PARTICLE_EFFECT: u16 = 1 << 12;

    fn probe(world: &World, entity: Entity) -> Self {
        let mut bits = 0u16;
        if world.get::<&Name>(entity).is_ok() {
            bits |= Self::NAME;
        }
        if world.get::<&Transform>(entity).is_ok() {
            bits |= Self::TRANSFORM;
        }
        if world.get::<&Camera>(entity).is_ok() {
            bits |= Self::CAMERA;
        }
        if world.get::<&MeshRenderer>(entity).is_ok() {
            bits |= Self::MESH_RENDERER;
        }
        if world.get::<&DirectionalLight>(entity).is_ok() {
            bits |= Self::DIR_LIGHT;
        }
        if world.get::<&PointLight>(entity).is_ok() {
            bits |= Self::POINT_LIGHT;
        }
        if world.get::<&RigidBody>(entity).is_ok() {
            bits |= Self::RIGID_BODY;
        }
        if world.get::<&Collider>(entity).is_ok() {
            bits |= Self::COLLIDER;
        }
        if world.get::<&SkeletonInstance>(entity).is_ok() {
            bits |= Self::SKELETON;
        }
        if world.get::<&AnimationPlayer>(entity).is_ok() {
            bits |= Self::ANIM_PLAYER;
        }
        if world.get::<&AudioEmitter>(entity).is_ok() {
            bits |= Self::AUDIO_EMITTER;
        }
        if world.get::<&AudioListener>(entity).is_ok() {
            bits |= Self::AUDIO_LISTENER;
        }
        if world.get::<&ParticleEffect>(entity).is_ok() {
            bits |= Self::PARTICLE_EFFECT;
        }
        Self { bits }
    }

    fn has(self, flag: u16) -> bool {
        self.bits & flag != 0
    }
}

/// Inspector Panel state
pub struct InspectorPanel {
    /// Cache for euler angles (quaternion -> euler conversion).
    euler_cache: HashMap<u64, (glm::Quat, [f32; 3])>,
    /// Last known entity for euler cache invalidation
    last_entity: Option<Entity>,
    _collapsed_components: HashSet<String>,
    /// Search/filter text for components
    search_filter: String,
    /// Cached component presence mask (recomputed on selection change or mutation).
    cached_presence: ComponentPresence,
    /// The entity for which `cached_presence` was computed.
    presence_entity: Option<Entity>,
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
            _collapsed_components: HashSet::new(),
            search_filter: String::new(),
            cached_presence: ComponentPresence::default(),
            presence_entity: None,
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
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        world: &mut World,
        selection: &Selection,
        play_mode: PlayMode,
        asset_browser: &mut AssetBrowserPanel,
    ) {
        egui::SidePanel::right("inspector_panel")
            .resizable(true)
            .default_width(300.0)
            .min_width(200.0)
            .show(ctx, |ui| {
                self.show_contents(ui, world, selection, play_mode, asset_browser);
            });
    }

    /// Render just the contents (for use inside dock tabs)
    pub fn show_contents(
        &mut self,
        ui: &mut Ui,
        world: &mut World,
        selection: &Selection,
        play_mode: PlayMode,
        asset_browser: &mut AssetBrowserPanel,
    ) {
        let read_only = play_mode != PlayMode::Edit;

        self.render_header(ui, selection, read_only);
        ui.separator();

        if let Some(entity) = selection.primary() {
            // Invalidate caches if entity changed
            if self.last_entity != Some(entity) {
                self.euler_cache.clear();
                self.last_entity = Some(entity);
                self.cached_presence = ComponentPresence::probe(world, entity);
                self.presence_entity = Some(entity);
            }
            // Re-probe when the cached entity matches but presence may have changed
            // (component added/removed). This is still cheaper than probing per-component
            // per-frame because it's a single pass of lightweight has-component checks.
            if self.presence_entity != Some(entity) {
                self.cached_presence = ComponentPresence::probe(world, entity);
                self.presence_entity = Some(entity);
            }

            // Search/filter box
            ui.horizontal(|ui| {
                ui.label("Filter:");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.search_filter)
                        .hint_text("Search components...")
                        .desired_width(ui.available_width()),
                );
                // Clear filter on Escape
                if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.search_filter.clear();
                }
            });
            ui.separator();

            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.render_entity_info(ui, world, entity);
                    ui.separator();
                    ui.add_enabled_ui(!read_only, |ui| {
                        self.render_components(ui, world, entity, asset_browser);
                        ui.separator();
                        self.render_add_component(ui, world, entity);
                    });
                });
        } else {
            self.render_empty_state(ui);
        }
    }

    /// Render panel header
    fn render_header(&self, ui: &mut Ui, selection: &Selection, read_only: bool) {
        ui.horizontal(|ui| {
            ui.heading("Inspector");
            if read_only {
                ui.label(RichText::new("(Playing)").weak().italics().small());
            } else if selection.count() > 1 {
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
                ui.label(RichText::new(format!("ID: {}", entity.id())).weak().small());
            });
        });
    }

    /// Component category colors
    fn category_color(category: ComponentCategory) -> Color32 {
        match category {
            ComponentCategory::Core => Color32::from_rgb(100, 160, 220), // Blue
            ComponentCategory::Rendering => Color32::from_rgb(220, 180, 80), // Yellow/Gold
            ComponentCategory::Physics => Color32::from_rgb(100, 180, 120), // Green
            ComponentCategory::Animation => Color32::from_rgb(200, 120, 200), // Purple
            ComponentCategory::Audio => Color32::from_rgb(220, 140, 80),     // Orange
        }
    }

    /// Draw a visual divider between component sections
    fn draw_component_divider(ui: &mut Ui) {
        ui.add_space(10.0);
        let rect = ui.available_rect_before_wrap();
        let y = rect.top();
        ui.painter()
            .hline(rect.x_range(), y, Stroke::new(1.0, Color32::from_gray(70)));
        ui.add_space(8.0);
    }

    /// Render all component editors using the cached presence snapshot.
    fn render_components(
        &mut self,
        ui: &mut Ui,
        world: &mut World,
        entity: Entity,
        asset_browser: &mut AssetBrowserPanel,
    ) {
        let mut component_count = 0;
        let mut pending_action = ComponentAction::None;
        let p = self.cached_presence;

        // === CORE COMPONENTS ===
        if self.matches_filter("name") && p.has(ComponentPresence::NAME) {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            self.edit_name(ui, world, entity, ComponentCategory::Core);
            component_count += 1;
        }
        if (self.matches_filter("transform")
            || self.matches_filter("position")
            || self.matches_filter("rotation")
            || self.matches_filter("scale"))
            && p.has(ComponentPresence::TRANSFORM)
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            self.edit_transform(ui, world, entity, ComponentCategory::Core);
            component_count += 1;
        }

        // === RENDERING COMPONENTS ===
        if (self.matches_filter("camera") || self.matches_filter("fov"))
            && p.has(ComponentPresence::CAMERA)
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            if let Some(action) = self.edit_camera(ui, world, entity, ComponentCategory::Rendering)
            {
                pending_action = action;
            }
            component_count += 1;
        }
        if (self.matches_filter("mesh")
            || self.matches_filter("renderer")
            || self.matches_filter("material"))
            && p.has(ComponentPresence::MESH_RENDERER)
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            if let Some(action) =
                self.edit_mesh_renderer(ui, world, entity, ComponentCategory::Rendering, asset_browser)
            {
                pending_action = action;
            }
            component_count += 1;
        }
        if (self.matches_filter("directional")
            || self.matches_filter("light")
            || self.matches_filter("sun"))
            && p.has(ComponentPresence::DIR_LIGHT)
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            if let Some(action) =
                self.edit_directional_light(ui, world, entity, ComponentCategory::Rendering)
            {
                pending_action = action;
            }
            component_count += 1;
        }
        if (self.matches_filter("point") || self.matches_filter("light"))
            && p.has(ComponentPresence::POINT_LIGHT)
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            if let Some(action) =
                self.edit_point_light(ui, world, entity, ComponentCategory::Rendering)
            {
                pending_action = action;
            }
            component_count += 1;
        }

        // === PHYSICS COMPONENTS ===
        if (self.matches_filter("rigid")
            || self.matches_filter("body")
            || self.matches_filter("physics"))
            && p.has(ComponentPresence::RIGID_BODY)
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            if let Some(action) = self.edit_rigidbody(ui, world, entity, ComponentCategory::Physics)
            {
                pending_action = action;
            }
            component_count += 1;
        }
        if (self.matches_filter("collider")
            || self.matches_filter("physics")
            || self.matches_filter("collision"))
            && p.has(ComponentPresence::COLLIDER)
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            if let Some(action) = self.edit_collider(ui, world, entity, ComponentCategory::Physics)
            {
                pending_action = action;
            }
        }

        // === ANIMATION COMPONENTS ===
        if (self.matches_filter("skeleton")
            || self.matches_filter("bone")
            || self.matches_filter("animation"))
            && p.has(ComponentPresence::SKELETON)
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            self.edit_skeleton(ui, world, entity, ComponentCategory::Animation);
            component_count += 1;
        }
        if (self.matches_filter("animation")
            || self.matches_filter("playback")
            || self.matches_filter("clip"))
            && p.has(ComponentPresence::ANIM_PLAYER)
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            self.edit_animation_player(ui, world, entity, ComponentCategory::Animation);
        }

        // === AUDIO COMPONENTS ===
        if (self.matches_filter("audio")
            || self.matches_filter("emitter")
            || self.matches_filter("sound")
            || self.matches_filter("clip"))
            && p.has(ComponentPresence::AUDIO_EMITTER)
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            if let Some(action) =
                self.edit_audio_emitter(ui, world, entity, ComponentCategory::Audio, asset_browser)
            {
                pending_action = action;
            }
            component_count += 1;
        }
        if (self.matches_filter("audio") || self.matches_filter("listener"))
            && p.has(ComponentPresence::AUDIO_LISTENER)
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            if let Some(action) =
                self.edit_audio_listener(ui, world, entity, ComponentCategory::Audio)
            {
                pending_action = action;
            }
        }

        if (self.matches_filter("plankton") || self.matches_filter("vfx") || self.matches_filter("particle"))
            && p.has(ComponentPresence::PARTICLE_EFFECT)
        {
            if component_count > 0 {
                Self::draw_component_divider(ui);
            }
            if let Some(action) =
                self.edit_particle_effect(ui, world, entity, ComponentCategory::Rendering)
            {
                pending_action = action;
            }
            component_count += 1;
        }
        let _ = component_count; // suppress unused warning

        // Execute pending action and invalidate snapshot
        let mutated = !matches!(pending_action, ComponentAction::None);
        match pending_action {
            ComponentAction::None => {}
            ComponentAction::RemoveCamera => {
                let _ = world.remove_one::<Camera>(entity);
            }
            ComponentAction::RemoveMeshRenderer => {
                let _ = world.remove_one::<MeshRenderer>(entity);
            }
            ComponentAction::RemoveDirectionalLight => {
                let _ = world.remove_one::<DirectionalLight>(entity);
            }
            ComponentAction::RemovePointLight => {
                let _ = world.remove_one::<PointLight>(entity);
            }
            ComponentAction::RemoveRigidBody => {
                let _ = world.remove_one::<RigidBody>(entity);
            }
            ComponentAction::RemoveCollider => {
                let _ = world.remove_one::<Collider>(entity);
            }
            ComponentAction::RemoveAudioEmitter => {
                let _ = world.remove_one::<AudioEmitter>(entity);
            }
            ComponentAction::RemoveAudioListener => {
                let _ = world.remove_one::<AudioListener>(entity);
            }
            ComponentAction::RemoveParticleEffect => {
                let _ = world.remove_one::<ParticleEffect>(entity);
            }
        }
        if mutated {
            self.cached_presence = ComponentPresence::probe(world, entity);
        }
    }

    /// Edit Name component
    fn edit_name(
        &self,
        ui: &mut Ui,
        world: &mut World,
        entity: Entity,
        category: ComponentCategory,
    ) {
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
    fn edit_transform(
        &mut self,
        ui: &mut Ui,
        world: &mut World,
        entity: Entity,
        category: ComponentCategory,
    ) {
        // Snapshot before editing so we can detect changes and mark dirty.
        let snapshot = world.get::<&Transform>(entity).ok().map(|t| *t);

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
                    if !transform.position.x.is_finite() {
                        transform.position.x = 0.0;
                    }
                    if !transform.position.y.is_finite() {
                        transform.position.y = 0.0;
                    }
                    if !transform.position.z.is_finite() {
                        transform.position.z = 0.0;
                    }
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
                            self.euler_cache.insert(
                                entity.id() as u64,
                                (glm::quat_identity(), [0.0, 0.0, 0.0]),
                            );
                        }
                    });

                    // Get or calculate euler angles.
                    // The cache stores (quaternion, euler) pairs so we can detect when
                    // the quaternion has been modified externally (e.g., by the gizmo).
                    let entity_id = entity.id() as u64;
                    let needs_recompute = match self.euler_cache.get(&entity_id) {
                        Some((cached_quat, _)) => {
                            !quaternions_approximately_equal(cached_quat, &transform.rotation)
                        }
                        None => true,
                    };
                    if needs_recompute {
                        let new_euler = quaternion_to_euler_degrees(&transform.rotation);
                        self.euler_cache
                            .insert(entity_id, (transform.rotation, new_euler));
                    }

                    // Get a copy of euler to work with (avoids borrow issues with closure)
                    let mut euler = self.euler_cache.get(&entity_id).unwrap().1;

                    // Sanitize cached euler values to prevent DragValue crash
                    for item in &mut euler {
                        if !item.is_finite() {
                            *item = 0.0;
                        }
                    }

                    let mut euler_changed = false;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("X").color(AXIS_COLOR_X));
                        let response_x = ui.add(
                            DragValue::new(&mut euler[0])
                                .speed(1.0)
                                .suffix("°")
                                .range(-180.0..=180.0),
                        );
                        ui.label(RichText::new("Y").color(AXIS_COLOR_Y));
                        let response_y = ui.add(
                            DragValue::new(&mut euler[1])
                                .speed(1.0)
                                .suffix("°")
                                .range(-180.0..=180.0),
                        );
                        ui.label(RichText::new("Z").color(AXIS_COLOR_Z));
                        let response_z = ui.add(
                            DragValue::new(&mut euler[2])
                                .speed(1.0)
                                .suffix("°")
                                .range(-180.0..=180.0),
                        );

                        euler_changed =
                            response_x.changed() || response_y.changed() || response_z.changed();
                    });

                    if euler_changed {
                        let new_quat = euler_degrees_to_quaternion(&euler);
                        transform.rotation = new_quat;
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
                    if !transform.scale.x.is_finite() {
                        transform.scale.x = 1.0;
                    }
                    if !transform.scale.y.is_finite() {
                        transform.scale.y = 1.0;
                    }
                    if !transform.scale.z.is_finite() {
                        transform.scale.z = 1.0;
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("X").color(AXIS_COLOR_X));
                        ui.add(
                            DragValue::new(&mut transform.scale.x)
                                .speed(0.01)
                                .range(0.001..=1000.0),
                        );
                        ui.label(RichText::new("Y").color(AXIS_COLOR_Y));
                        ui.add(
                            DragValue::new(&mut transform.scale.y)
                                .speed(0.01)
                                .range(0.001..=1000.0),
                        );
                        ui.label(RichText::new("Z").color(AXIS_COLOR_Z));
                        ui.add(
                            DragValue::new(&mut transform.scale.z)
                                .speed(0.01)
                                .range(0.001..=1000.0),
                        );
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

        // After the mutable borrow is released, check if transform changed
        // and mark dirty for incremental propagation.
        if let Some(before) = snapshot {
            let changed = world.get::<&Transform>(entity).ok().is_some_and(|current| {
                current.position != before.position
                    || current.rotation != before.rotation
                    || current.scale != before.scale
            });
            if changed {
                crate::engine::ecs::hierarchy::mark_transform_dirty(world, entity);
            }
        }
    }

    /// Edit Camera component
    fn edit_camera(
        &self,
        ui: &mut Ui,
        world: &mut World,
        entity: Entity,
        category: ComponentCategory,
    ) -> Option<ComponentAction> {
        let mut action = None;
        if let Ok(mut camera) = world.get::<&mut Camera>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            let header = CollapsingHeader::new(RichText::new("Camera").strong())
                .default_open(true)
                .show(ui, |ui| {
                    ui.checkbox(&mut camera.active, "Active")
                        .on_hover_text("Whether this camera is currently rendering");

                    // Projection type
                    ui.horizontal(|ui| {
                        ui.label("Projection:");
                        let current_label = match camera.projection {
                            CameraProjection::Perspective => "Perspective",
                            CameraProjection::Orthographic { .. } => "Orthographic",
                        };
                        egui::ComboBox::from_id_salt("cam_projection")
                            .selected_text(current_label)
                            .show_ui(ui, |ui| {
                                let is_perspective =
                                    matches!(camera.projection, CameraProjection::Perspective);
                                if ui.selectable_label(is_perspective, "Perspective").clicked() {
                                    camera.projection = CameraProjection::Perspective;
                                }
                                let is_ortho = matches!(
                                    camera.projection,
                                    CameraProjection::Orthographic { .. }
                                );
                                if ui.selectable_label(is_ortho, "Orthographic").clicked() {
                                    camera.projection =
                                        CameraProjection::Orthographic { size: 10.0 };
                                }
                            });
                    });

                    // Sanitize camera values to prevent DragValue crash
                    if !camera.fov.is_finite() {
                        camera.fov = 60.0;
                    }
                    if !camera.near.is_finite() || camera.near <= 0.0 {
                        camera.near = 0.1;
                    }
                    if !camera.far.is_finite() || camera.far <= camera.near {
                        camera.far = 1000.0;
                    }

                    // FOV or Ortho Size depending on projection
                    match &mut camera.projection {
                        CameraProjection::Perspective => {
                            ui.horizontal(|ui| {
                                ui.label("FOV:");
                                ui.add(
                                    egui::Slider::new(&mut camera.fov, 30.0..=120.0)
                                        .suffix("°")
                                        .clamping(egui::SliderClamping::Always),
                                )
                                .on_hover_text("Field of view angle. Wider = more visible area");
                            });
                        }
                        CameraProjection::Orthographic { size } => {
                            if !size.is_finite() {
                                *size = 10.0;
                            }
                            ui.horizontal(|ui| {
                                ui.label("Ortho Size:");
                                ui.add(DragValue::new(size).speed(0.1).range(0.1..=1000.0))
                                    .on_hover_text("Orthographic camera view size");
                            });
                        }
                    }

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
                        .on_hover_text(
                            "Near clipping plane. Objects closer than this won't render",
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Far:");
                        ui.add(
                            DragValue::new(&mut camera.far)
                                .speed(1.0)
                                .range(far_min..=100000.0),
                        )
                        .on_hover_text(
                            "Far clipping plane. Objects farther than this won't render",
                        );
                    });

                    // Clear color
                    ui.horizontal(|ui| {
                        ui.label("Clear Color:");
                        ui.color_edit_button_rgb(&mut camera.clear_color)
                            .on_hover_text("Background color when nothing is rendered");
                    });

                    // Priority
                    ui.horizontal(|ui| {
                        ui.label("Priority:");
                        ui.add(
                            DragValue::new(&mut camera.priority)
                                .speed(1)
                                .range(-100..=100),
                        )
                        .on_hover_text("Camera render order. Higher priority renders on top");
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
    fn edit_mesh_renderer(
        &self,
        ui: &mut Ui,
        world: &mut World,
        entity: Entity,
        category: ComponentCategory,
        asset_browser: &mut AssetBrowserPanel,
    ) -> Option<ComponentAction> {
        let mut action = None;
        if let Ok(mut renderer) = world.get::<&mut MeshRenderer>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            let header = CollapsingHeader::new(RichText::new("Mesh Renderer").strong())
                .default_open(true)
                .show(ui, |ui| {
                    ui.checkbox(&mut renderer.visible, "Visible")
                        .on_hover_text("Whether this mesh is rendered");

                    // Mesh asset slot
                    let mesh_idx = renderer.mesh_index;
                    ui.label("Mesh:");
                    Self::asset_slot(
                        ui,
                        "mesh_slot",
                        &mut renderer.mesh_path,
                        &[AssetType::Mesh, AssetType::Model],
                        asset_browser,
                        mesh_idx,
                    );

                    ui.add_space(4.0);

                    // Material slots — one slot per submesh material
                    if renderer.material_paths.is_empty() {
                        renderer.material_paths.push(String::new());
                    }
                    let slot_count = renderer.material_paths.len();
                    for i in 0..slot_count {
                        let label = if slot_count == 1 {
                            "Material:".to_string()
                        } else {
                            format!("Material [{}]:", i)
                        };
                        ui.label(&label);
                        Self::asset_slot(
                            ui,
                            &format!("material_slot_{}", i),
                            &mut renderer.material_paths[i],
                            &[AssetType::Material],
                            asset_browser,
                            0,
                        );
                        ui.add_space(2.0);
                    }

                    ui.add_space(4.0);

                    ui.checkbox(&mut renderer.cast_shadows, "Cast Shadows")
                        .on_hover_text("Whether this mesh casts shadows");
                    ui.checkbox(&mut renderer.receive_shadows, "Receive Shadows")
                        .on_hover_text("Whether this mesh receives shadows from other objects");

                    // --- Material Instance Overrides ---
                    ui.add_space(6.0);
                    ui.separator();
                    ui.label(RichText::new("Material Overrides").strong());

                    // Base Color
                    ui.horizontal(|ui| {
                        ui.label("Base Color:");
                        let mut color3 = [
                            renderer.base_color_factor[0],
                            renderer.base_color_factor[1],
                            renderer.base_color_factor[2],
                        ];
                        if ui.color_edit_button_rgb(&mut color3).changed() {
                            renderer.base_color_factor[0] = color3[0];
                            renderer.base_color_factor[1] = color3[1];
                            renderer.base_color_factor[2] = color3[2];
                        }
                    });
                    ui.add(
                        egui::Slider::new(&mut renderer.base_color_factor[3], 0.0..=1.0)
                            .text("Alpha"),
                    );

                    ui.add(
                        egui::Slider::new(&mut renderer.metallic_factor, 0.0..=1.0)
                            .text("Metallic"),
                    );
                    ui.add(
                        egui::Slider::new(&mut renderer.roughness_factor, 0.0..=1.0)
                            .text("Roughness"),
                    );

                    // Emissive
                    ui.horizontal(|ui| {
                        ui.label("Emissive:");
                        let mut em = renderer.emissive_factor;
                        let mut changed = false;
                        changed |= ui
                            .add(egui::DragValue::new(&mut em[0]).speed(0.01).range(0.0..=10.0).prefix("R: "))
                            .changed();
                        changed |= ui
                            .add(egui::DragValue::new(&mut em[1]).speed(0.01).range(0.0..=10.0).prefix("G: "))
                            .changed();
                        changed |= ui
                            .add(egui::DragValue::new(&mut em[2]).speed(0.01).range(0.0..=10.0).prefix("B: "))
                            .changed();
                        if changed {
                            renderer.emissive_factor = em;
                        }
                    });
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

    /// Draw an asset slot widget that accepts drag-and-drop from the asset browser.
    ///
    /// Shows a square thumbnail + filename when an asset is assigned, or a drop hint
    /// when empty. If `legacy_index > 0` and path is empty, shows the legacy index.
    /// Accepts drops of the specified `allowed_types` from the asset browser.
    fn asset_slot(
        ui: &mut Ui,
        id_salt: &str,
        path: &mut String,
        allowed_types: &[AssetType],
        asset_browser: &mut AssetBrowserPanel,
        _legacy_index: usize,
    ) {
        let slot_size: f32 = 90.0;
        let thumb_size: f32 = slot_size - 8.0;
        let slot_width = slot_size.max(ui.available_width().min(slot_size));

        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(slot_width, slot_size),
            egui::Sense::click(),
        );

        // Check for DnD hover
        let mut is_valid_hover = false;
        if let Some(hovered_id) = response.dnd_hover_payload::<AssetId>() {
            if let Some(meta) = asset_browser.registry.get(*hovered_id) {
                if allowed_types.contains(&meta.asset_type) {
                    is_valid_hover = true;
                }
            }
        }

        // Check for DnD drop
        if let Some(dropped_id) = response.dnd_release_payload::<AssetId>() {
            if let Some(meta) = asset_browser.registry.get(*dropped_id) {
                if allowed_types.contains(&meta.asset_type) {
                    *path = meta.path.to_string_lossy().to_string();
                }
            }
        }

        // Popup ID for asset picker
        let popup_id = ui.id().with(id_salt).with("asset_picker_popup");

        // Use a child UI constrained to the slot rect for proper clipping and layout
        let mut slot_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));

        let painter = slot_ui.painter();

        // Background
        let bg_color = if is_valid_hover {
            Color32::from_rgba_premultiplied(40, 60, 90, 255)
        } else if response.hovered() {
            Color32::from_gray(45)
        } else {
            Color32::from_gray(35)
        };
        painter.rect_filled(rect, 4.0, bg_color);

        // Border
        let border_color = if is_valid_hover {
            Color32::from_rgb(100, 180, 255)
        } else if response.hovered() {
            Color32::from_gray(100)
        } else {
            Color32::from_gray(60)
        };
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, border_color),
            egui::epaint::StrokeKind::Inside,
        );

        if path.is_empty() {
            // Empty slot — click prompt
            let type_names: Vec<&str> =
                allowed_types.iter().map(|t| t.display_name()).collect();
            let hint = format!("Click to select\n{}", type_names.join("/"));
            let text_color = Color32::from_gray(120);
            let inner = rect.shrink(6.0);
            slot_ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space((inner.height() - 28.0).max(0.0) / 2.0);
                    ui.label(RichText::new(hint).font(egui::FontId::proportional(11.0)).color(text_color));
                });
            });
        } else {
            // Has asset — show square thumbnail with filename overlay
            let thumb_rect = egui::Rect::from_min_size(
                rect.min + egui::vec2(4.0, 4.0),
                egui::vec2(thumb_size, thumb_size),
            );

            // Try to get thumbnail
            let asset_id = AssetId::from_path(path);
            if let Some(meta) = asset_browser.registry.get(asset_id) {
                if let Some(tex_id) = asset_browser.thumbnails.get_texture_id(ui.ctx(), meta) {
                    painter.image(
                        tex_id,
                        thumb_rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    painter.rect_filled(thumb_rect, 2.0, Color32::from_gray(50));
                }
            } else {
                // For primitives or unregistered assets, show type icon placeholder
                painter.rect_filled(thumb_rect, 2.0, Color32::from_gray(50));
                if path.starts_with("__primitive__/") {
                    painter.text(
                        thumb_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "\u{25A6}",
                        egui::FontId::proportional(24.0),
                        Color32::from_gray(140),
                    );
                }
            }

            // Filename text at bottom of thumbnail
            let filename = std::path::Path::new(path.as_str())
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone());
            // Dark overlay behind text for readability
            let text_bg = egui::Rect::from_min_max(
                egui::pos2(thumb_rect.min.x, thumb_rect.max.y - 18.0),
                thumb_rect.max,
            );
            painter.rect_filled(text_bg, 0.0, Color32::from_black_alpha(160));
            let clipped = painter.with_clip_rect(thumb_rect);
            clipped.text(
                egui::pos2(thumb_rect.center().x, thumb_rect.max.y - 4.0),
                egui::Align2::CENTER_BOTTOM,
                &filename,
                egui::FontId::proportional(10.0),
                Color32::from_gray(220),
            );

            // Clear button (x) top-right corner
            let clear_rect = egui::Rect::from_min_size(
                egui::pos2(thumb_rect.max.x - 16.0, thumb_rect.min.y),
                egui::vec2(16.0, 16.0),
            );
            let clear_response = ui.interact(
                clear_rect,
                ui.id().with(id_salt).with("clear"),
                egui::Sense::click(),
            );
            let clear_color = if clear_response.hovered() {
                Color32::from_rgb(220, 80, 80)
            } else {
                Color32::from_gray(160)
            };
            painter.rect_filled(clear_rect, 2.0, Color32::from_black_alpha(120));
            painter.text(
                clear_rect.center(),
                egui::Align2::CENTER_CENTER,
                "x",
                egui::FontId::proportional(11.0),
                clear_color,
            );
            if clear_response.clicked() {
                *path = String::new();
            }
        }

        // Asset picker popup — uses egui's built-in toggle + close-on-click-outside
        egui::Popup::from_toggle_button_response(&response)
            .id(popup_id)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(300.0)
            .show(|ui| {
                ui.set_max_height(350.0);

                // Search bar
                let search_id = popup_id.with("search_text");
                let mut search_text: String = ui.data_mut(|d| {
                    d.get_temp_mut_or_default::<String>(search_id).clone()
                });
                ui.horizontal(|ui| {
                    ui.label("Search:");
                    let te = ui.text_edit_singleline(&mut search_text);
                    if te.changed() {
                        ui.data_mut(|d| {
                            *d.get_temp_mut_or_default::<String>(search_id) = search_text.clone();
                        });
                    }
                    // Auto-focus search on open
                    if !te.has_focus() {
                        te.request_focus();
                    }
                });

                ui.separator();

                // Collect matching assets
                let search_lower = search_text.to_lowercase();
                let include_meshes = allowed_types.contains(&AssetType::Mesh)
                    || allowed_types.contains(&AssetType::Model);

                ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    let item_w = 60.0;
                    let label_h = 16.0;
                    let item_h = item_w + label_h;
                    let cols = ((ui.available_width()) / (item_w + 4.0)).floor().max(1.0) as usize;

                    // Primitives section (only for mesh slots)
                    if include_meshes {
                        use crate::engine::rendering::rendering_3d::mesh::PRIMITIVE_PATHS;
                        let matching_prims: Vec<&str> = PRIMITIVE_PATHS
                            .iter()
                            .filter(|p| {
                                search_text.is_empty()
                                    || p.to_lowercase().contains(&search_lower)
                            })
                            .copied()
                            .collect();

                        if !matching_prims.is_empty() {
                            ui.label(RichText::new("Primitives").small().color(Color32::from_gray(140)));
                            ui.add_space(2.0);
                            egui::Grid::new(popup_id.with("prims_grid"))
                                .spacing(egui::vec2(4.0, 4.0))
                                .show(ui, |ui| {
                                    for (i, &prim_path) in matching_prims.iter().enumerate() {
                                        let label = prim_path.rsplit('/').next().unwrap_or(prim_path);
                                        let is_selected = path.as_str() == prim_path;

                                        let (item_rect, item_resp) = ui.allocate_exact_size(
                                            egui::vec2(item_w, item_h),
                                            egui::Sense::click(),
                                        );

                                        let bg = if is_selected {
                                            Color32::from_rgb(40, 60, 100)
                                        } else if item_resp.hovered() {
                                            Color32::from_gray(55)
                                        } else {
                                            Color32::from_gray(40)
                                        };
                                        ui.painter().rect_filled(item_rect, 3.0, bg);
                                        // Icon (centered in the square thumb area)
                                        let thumb_bottom = item_rect.max.y - label_h;
                                        ui.painter().text(
                                            egui::pos2(item_rect.center().x, (item_rect.min.y + thumb_bottom) / 2.0),
                                            egui::Align2::CENTER_CENTER,
                                            "\u{25A6}",
                                            egui::FontId::proportional(20.0),
                                            Color32::from_gray(180),
                                        );
                                        // Label (clipped to item rect)
                                        let clipped = ui.painter().with_clip_rect(item_rect);
                                        clipped.text(
                                            egui::pos2(item_rect.center().x, item_rect.max.y - 3.0),
                                            egui::Align2::CENTER_BOTTOM,
                                            label,
                                            egui::FontId::proportional(10.0),
                                            Color32::from_gray(200),
                                        );

                                        if item_resp.clicked() {
                                            *path = prim_path.to_string();
                                            ui.close();
                                        }

                                        if (i + 1) % cols == 0 {
                                            ui.end_row();
                                        }
                                    }
                                });
                            ui.add_space(4.0);
                            ui.separator();
                            ui.add_space(2.0);
                        }
                    }

                    // Registry assets section
                    let filter = super::asset_browser::AssetFilter {
                        search_text: if search_text.is_empty() {
                            None
                        } else {
                            Some(search_text.clone())
                        },
                        asset_types: Some(allowed_types.to_vec()),
                        include_subfolders: true,
                        ..Default::default()
                    };
                    let results = asset_browser.registry.query(&filter);

                    if results.is_empty() && !include_meshes {
                        ui.label(
                            RichText::new("No assets found")
                                .color(Color32::from_gray(100)),
                        );
                    } else if !results.is_empty() {
                        ui.label(RichText::new("Assets").small().color(Color32::from_gray(140)));
                        ui.add_space(2.0);
                        egui::Grid::new(popup_id.with("assets_grid"))
                            .spacing(egui::vec2(4.0, 4.0))
                            .show(ui, |ui| {
                                for (i, meta) in results.iter().enumerate() {
                                    let asset_path =
                                        meta.path.to_string_lossy().to_string();
                                    let display = &meta.display_name;
                                    let is_selected = path.as_str() == asset_path;

                                    let (item_rect, item_resp) = ui.allocate_exact_size(
                                        egui::vec2(item_w, item_h),
                                        egui::Sense::click(),
                                    );

                                    let bg = if is_selected {
                                        Color32::from_rgb(40, 60, 100)
                                    } else if item_resp.hovered() {
                                        Color32::from_gray(55)
                                    } else {
                                        Color32::from_gray(40)
                                    };
                                    ui.painter().rect_filled(item_rect, 3.0, bg);

                                    // Square thumbnail area above the label
                                    let thumb_r = egui::Rect::from_min_size(
                                        item_rect.min + egui::vec2(4.0, 4.0),
                                        egui::vec2(item_w - 8.0, item_w - 8.0),
                                    );
                                    if let Some(tex_id) = asset_browser
                                        .thumbnails
                                        .get_texture_id(ui.ctx(), meta)
                                    {
                                        ui.painter().image(
                                            tex_id,
                                            thumb_r,
                                            egui::Rect::from_min_max(
                                                egui::Pos2::ZERO,
                                                egui::pos2(1.0, 1.0),
                                            ),
                                            Color32::WHITE,
                                        );
                                    } else {
                                        ui.painter().rect_filled(
                                            thumb_r,
                                            2.0,
                                            Color32::from_gray(50),
                                        );
                                    }

                                    // Label (clipped to item rect)
                                    let clipped = ui.painter().with_clip_rect(item_rect);
                                    clipped.text(
                                        egui::pos2(
                                            item_rect.center().x,
                                            item_rect.max.y - 3.0,
                                        ),
                                        egui::Align2::CENTER_BOTTOM,
                                        display,
                                        egui::FontId::proportional(9.0),
                                        Color32::from_gray(200),
                                    );

                                    if item_resp.clicked() {
                                        *path = asset_path;
                                        ui.close();
                                    }

                                    if (i + 1) % cols == 0 {
                                        ui.end_row();
                                    }
                                }
                            });
                    }
                });
            });
    }

    /// Edit DirectionalLight component
    fn edit_directional_light(
        &self,
        ui: &mut Ui,
        world: &mut World,
        entity: Entity,
        category: ComponentCategory,
    ) -> Option<ComponentAction> {
        let mut action = None;
        if let Ok(mut light) = world.get::<&mut DirectionalLight>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            let header = CollapsingHeader::new(RichText::new("Directional Light").strong())
                .default_open(true)
                .show(ui, |ui| {
                    // Direction
                    ui.label("Direction:")
                        .on_hover_text("Direction the light is pointing (normalized)");
                    // Sanitize direction to prevent DragValue crash
                    if !light.direction.x.is_finite() {
                        light.direction.x = 0.0;
                    }
                    if !light.direction.y.is_finite() {
                        light.direction.y = -1.0;
                    }
                    if !light.direction.z.is_finite() {
                        light.direction.z = 0.0;
                    }
                    ui.horizontal(|ui| {
                        ui.add(
                            DragValue::new(&mut light.direction.x)
                                .prefix("X: ")
                                .speed(0.01)
                                .range(-1.0..=1.0),
                        );
                        ui.add(
                            DragValue::new(&mut light.direction.y)
                                .prefix("Y: ")
                                .speed(0.01)
                                .range(-1.0..=1.0),
                        );
                        ui.add(
                            DragValue::new(&mut light.direction.z)
                                .prefix("Z: ")
                                .speed(0.01)
                                .range(-1.0..=1.0),
                        );
                    });

                    // Normalize direction (safe - values are now bounded)
                    let len = glm::length(&light.direction);
                    if len > 0.001 {
                        light.direction /= len;
                    }

                    // Color (RGB)
                    ui.horizontal(|ui| {
                        ui.label("Color:");
                        let mut color = [light.color.x, light.color.y, light.color.z];
                        if ui
                            .color_edit_button_rgb(&mut color)
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

                    // Shadows
                    ui.checkbox(&mut light.shadow_enabled, "Cast Shadows")
                        .on_hover_text("Enable shadow casting from this light");
                    if light.shadow_enabled {
                        if !light.shadow_bias.is_finite() {
                            light.shadow_bias = 0.005;
                        }
                        ui.horizontal(|ui| {
                            ui.label("Shadow Bias:");
                            ui.add(
                                DragValue::new(&mut light.shadow_bias)
                                    .speed(0.001)
                                    .range(0.0..=0.1),
                            )
                            .on_hover_text("Bias to prevent shadow acne artifacts");
                        });
                    }
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
    fn edit_point_light(
        &self,
        ui: &mut Ui,
        world: &mut World,
        entity: Entity,
        category: ComponentCategory,
    ) -> Option<ComponentAction> {
        let mut action = None;
        if let Ok(mut light) = world.get::<&mut PointLight>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            let header = CollapsingHeader::new(RichText::new("Point Light").strong())
                .default_open(true)
                .show(ui, |ui| {
                    // Sanitize light values to prevent DragValue crash
                    if !light.intensity.is_finite() {
                        light.intensity = 1.0;
                    }
                    if !light.radius.is_finite() {
                        light.radius = 10.0;
                    }

                    // Color
                    ui.horizontal(|ui| {
                        ui.label("Color:");
                        let mut color = [light.color.x, light.color.y, light.color.z];
                        if ui
                            .color_edit_button_rgb(&mut color)
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

                    // Falloff
                    ui.horizontal(|ui| {
                        ui.label("Falloff:");
                        egui::ComboBox::from_id_salt("pl_falloff")
                            .selected_text(format!("{:?}", light.falloff))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut light.falloff,
                                    LightFalloff::Linear,
                                    "Linear",
                                )
                                .on_hover_text("Light decreases linearly with distance");
                                ui.selectable_value(
                                    &mut light.falloff,
                                    LightFalloff::Quadratic,
                                    "Quadratic",
                                )
                                .on_hover_text("Realistic falloff (default)");
                                ui.selectable_value(
                                    &mut light.falloff,
                                    LightFalloff::InverseSquare,
                                    "Inverse Square",
                                )
                                .on_hover_text("Physically accurate inverse-square law");
                            });
                    });

                    // Shadows
                    ui.checkbox(&mut light.shadow_enabled, "Cast Shadows")
                        .on_hover_text("Enable shadow casting from this light");
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
    fn edit_rigidbody(
        &self,
        ui: &mut Ui,
        world: &mut World,
        entity: Entity,
        category: ComponentCategory,
    ) -> Option<ComponentAction> {
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

                    // Gravity scale (dynamic only)
                    if rb.body_type == RigidBodyType::Dynamic {
                        if !rb.gravity_scale.is_finite() { rb.gravity_scale = 1.0; }
                        ui.horizontal(|ui| {
                            ui.label("Gravity Scale:");
                            ui.add(
                                DragValue::new(&mut rb.gravity_scale)
                                    .speed(0.1)
                                    .range(-10.0..=10.0),
                            )
                            .on_hover_text("Gravity multiplier. 0 = no gravity, negative = anti-gravity");
                        });
                    }

                    // CCD
                    ui.checkbox(&mut rb.continuous_collision, "CCD (Continuous)")
                        .on_hover_text("Continuous collision detection. Prevents fast objects from tunneling through thin walls");

                    // Lock rotation axes
                    ui.label("Lock Rotation:");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut rb.lock_rotation[0], RichText::new("X").color(AXIS_COLOR_X));
                        ui.checkbox(&mut rb.lock_rotation[1], RichText::new("Y").color(AXIS_COLOR_Y));
                        ui.checkbox(&mut rb.lock_rotation[2], RichText::new("Z").color(AXIS_COLOR_Z));
                    });
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
    fn edit_collider(
        &self,
        ui: &mut Ui,
        world: &mut World,
        entity: Entity,
        category: ComponentCategory,
    ) -> Option<ComponentAction> {
        let mut action = None;
        if let Ok(mut collider) = world.get::<&mut Collider>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            let header = CollapsingHeader::new(RichText::new("Collider").strong())
                .default_open(true)
                .show(ui, |ui| {
                    // Shape type dropdown
                    let current_shape_name = match &collider.shape {
                        ColliderShape::Cuboid { .. } => "Cuboid",
                        ColliderShape::Ball { .. } => "Ball",
                        ColliderShape::Capsule { .. } => "Capsule",
                    };
                    ui.horizontal(|ui| {
                        ui.label("Shape:").on_hover_text("Collision shape geometry");
                        egui::ComboBox::from_id_salt("collider_shape")
                            .selected_text(current_shape_name)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(
                                        matches!(&collider.shape, ColliderShape::Cuboid { .. }),
                                        "Cuboid",
                                    )
                                    .clicked()
                                    && !matches!(&collider.shape, ColliderShape::Cuboid { .. })
                                {
                                    collider.shape = ColliderShape::Cuboid {
                                        half_extents: glm::vec3(0.5, 0.5, 0.5),
                                    };
                                }
                                if ui
                                    .selectable_label(
                                        matches!(&collider.shape, ColliderShape::Ball { .. }),
                                        "Ball",
                                    )
                                    .clicked()
                                    && !matches!(&collider.shape, ColliderShape::Ball { .. })
                                {
                                    collider.shape = ColliderShape::Ball { radius: 0.5 };
                                }
                                if ui
                                    .selectable_label(
                                        matches!(&collider.shape, ColliderShape::Capsule { .. }),
                                        "Capsule",
                                    )
                                    .clicked()
                                    && !matches!(&collider.shape, ColliderShape::Capsule { .. })
                                {
                                    collider.shape = ColliderShape::Capsule {
                                        half_height: 0.5,
                                        radius: 0.25,
                                    };
                                }
                            });
                    });

                    // Shape-specific parameters
                    match &mut collider.shape {
                        ColliderShape::Cuboid { half_extents } => {
                            if !half_extents.x.is_finite() {
                                half_extents.x = 0.5;
                            }
                            if !half_extents.y.is_finite() {
                                half_extents.y = 0.5;
                            }
                            if !half_extents.z.is_finite() {
                                half_extents.z = 0.5;
                            }
                            ui.label("Half Extents:");
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("X").color(AXIS_COLOR_X));
                                ui.add(
                                    DragValue::new(&mut half_extents.x)
                                        .speed(0.01)
                                        .range(0.001..=1000.0),
                                );
                                ui.label(RichText::new("Y").color(AXIS_COLOR_Y));
                                ui.add(
                                    DragValue::new(&mut half_extents.y)
                                        .speed(0.01)
                                        .range(0.001..=1000.0),
                                );
                                ui.label(RichText::new("Z").color(AXIS_COLOR_Z));
                                ui.add(
                                    DragValue::new(&mut half_extents.z)
                                        .speed(0.01)
                                        .range(0.001..=1000.0),
                                );
                            });
                        }
                        ColliderShape::Ball { radius } => {
                            if !radius.is_finite() {
                                *radius = 0.5;
                            }
                            ui.horizontal(|ui| {
                                ui.label("Radius:");
                                ui.add(DragValue::new(radius).speed(0.01).range(0.001..=1000.0));
                            });
                        }
                        ColliderShape::Capsule {
                            half_height,
                            radius,
                        } => {
                            if !half_height.is_finite() {
                                *half_height = 0.5;
                            }
                            if !radius.is_finite() {
                                *radius = 0.25;
                            }
                            ui.horizontal(|ui| {
                                ui.label("Half Height:");
                                ui.add(
                                    DragValue::new(half_height)
                                        .speed(0.01)
                                        .range(0.001..=1000.0),
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.label("Radius:");
                                ui.add(DragValue::new(radius).speed(0.01).range(0.001..=1000.0));
                            });
                        }
                    }

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

                    ui.checkbox(&mut collider.debug_draw_visible, "Debug Draw")
                        .on_hover_text("Show collider wireframe in viewport");
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

    /// Display skeleton instance info with debug draw toggle
    fn edit_skeleton(
        &self,
        ui: &mut Ui,
        world: &mut World,
        entity: Entity,
        category: ComponentCategory,
    ) {
        if let Ok(mut skeleton) = world.get::<&mut SkeletonInstance>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            CollapsingHeader::new(RichText::new("Skeleton").strong())
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Bones:");
                        ui.label(format!("{}", skeleton.bones.len()));
                    });

                    ui.checkbox(&mut skeleton.debug_draw_visible, "Debug Draw")
                        .on_hover_text("Show bone hierarchy wireframe in viewport");

                    if !skeleton.bones.is_empty() {
                        CollapsingHeader::new("Bone List")
                            .default_open(false)
                            .show(ui, |ui| {
                                for (i, bone) in skeleton.bones.iter().enumerate() {
                                    let parent_str = bone
                                        .parent_index
                                        .map(|p| format!(" (parent: {})", p))
                                        .unwrap_or_default();
                                    ui.label(
                                        RichText::new(format!(
                                            "[{}] {}{}",
                                            i, bone.name, parent_str
                                        ))
                                        .monospace()
                                        .size(11.0),
                                    );
                                }
                            });
                    }
                });

            let end_y = ui.cursor().top();
            let accent_rect = egui::Rect::from_min_max(
                egui::pos2(ui.min_rect().left(), start_y),
                egui::pos2(ui.min_rect().left() + 4.0, end_y),
            );
            ui.painter().rect_filled(accent_rect, 1.0, color);
        }
    }

    /// Display and control animation player
    fn edit_animation_player(
        &self,
        ui: &mut Ui,
        world: &mut World,
        entity: Entity,
        category: ComponentCategory,
    ) {
        if let Ok(mut player) = world.get::<&mut AnimationPlayer>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            CollapsingHeader::new(RichText::new("Animation Player").strong())
                .default_open(true)
                .show(ui, |ui| {
                    // Clip info
                    ui.horizontal(|ui| {
                        ui.label("Clip:");
                        ui.label(&player.clip.name);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Duration:");
                        ui.label(format!("{:.2}s", player.clip.duration_seconds));
                    });

                    // Playback controls
                    ui.horizontal(|ui| {
                        let is_playing = player.state == PlaybackState::Playing;
                        if ui
                            .button(if is_playing { "Stop" } else { "Play" })
                            .clicked()
                        {
                            if is_playing {
                                player.stop();
                            } else {
                                player.play();
                            }
                        }
                        if ui.button("Reset").clicked() {
                            player.reset();
                        }
                    });

                    // Time scrubber
                    let duration = player.clip.duration_seconds.max(0.001);
                    ui.horizontal(|ui| {
                        ui.label("Time:");
                        ui.add(
                            DragValue::new(&mut player.time)
                                .speed(0.01)
                                .range(0.0..=duration)
                                .suffix("s"),
                        );
                    });
                    ui.add(
                        egui::Slider::new(&mut player.time, 0.0..=duration)
                            .show_value(false)
                            .clamping(egui::SliderClamping::Always),
                    );

                    // Speed
                    ui.horizontal(|ui| {
                        ui.label("Speed:");
                        ui.add(
                            DragValue::new(&mut player.speed)
                                .speed(0.01)
                                .range(0.0..=10.0),
                        );
                    });

                    // Looping
                    ui.checkbox(&mut player.looping, "Loop");
                });

            let end_y = ui.cursor().top();
            let accent_rect = egui::Rect::from_min_max(
                egui::pos2(ui.min_rect().left(), start_y),
                egui::pos2(ui.min_rect().left() + 4.0, end_y),
            );
            ui.painter().rect_filled(accent_rect, 1.0, color);
        }
    }

    /// Edit AudioEmitter component
    fn edit_audio_emitter(
        &self,
        ui: &mut Ui,
        world: &mut World,
        entity: Entity,
        category: ComponentCategory,
        asset_browser: &mut AssetBrowserPanel,
    ) -> Option<ComponentAction> {
        let mut action = None;
        if let Ok(mut emitter) = world.get::<&mut AudioEmitter>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            CollapsingHeader::new(RichText::new("Audio Emitter").strong())
                .default_open(true)
                .show(ui, |ui| {
                    // Clip path (drag-drop from asset browser)
                    ui.label("Clip:");
                    Self::asset_slot(
                        ui,
                        "audio_clip_slot",
                        &mut emitter.clip_path,
                        &[AssetType::Audio],
                        asset_browser,
                        0,
                    );

                    // Bus dropdown
                    ui.horizontal(|ui| {
                        ui.label("Bus:");
                        egui::ComboBox::from_id_salt("audio_bus")
                            .selected_text(emitter.bus.display_name())
                            .show_ui(ui, |ui| {
                                for &bus in AudioBus::ALL {
                                    ui.selectable_value(&mut emitter.bus, bus, bus.display_name());
                                }
                            });
                    });

                    // Volume (dB)
                    ui.horizontal(|ui| {
                        ui.label("Volume (dB):");
                        ui.add(
                            DragValue::new(&mut emitter.volume_db)
                                .speed(0.1)
                                .range(-80.0..=12.0)
                                .suffix(" dB"),
                        );
                    });

                    // Pitch
                    ui.horizontal(|ui| {
                        ui.label("Pitch:");
                        ui.add(
                            DragValue::new(&mut emitter.pitch)
                                .speed(0.01)
                                .range(0.1..=4.0),
                        );
                    });

                    // Looping & Auto-play
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut emitter.looping, "Loop");
                        ui.checkbox(&mut emitter.auto_play, "Auto-play");
                    });

                    // Spatial
                    ui.checkbox(&mut emitter.spatial, "Spatial (3D)");
                    if emitter.spatial {
                        ui.horizontal(|ui| {
                            ui.label("Max Distance:");
                            ui.add(
                                DragValue::new(&mut emitter.max_distance)
                                    .speed(0.5)
                                    .range(1.0..=1000.0)
                                    .suffix(" m"),
                            );
                        });
                        ui.checkbox(&mut emitter.hide_range_in_game, "Hidden in Game");
                    }

                    ui.add_space(4.0);
                    if ui
                        .button(RichText::new("Remove").color(Color32::from_rgb(220, 80, 80)))
                        .clicked()
                    {
                        action = Some(ComponentAction::RemoveAudioEmitter);
                    }
                });

            let end_y = ui.cursor().top();
            let accent_rect = egui::Rect::from_min_max(
                egui::pos2(ui.min_rect().left(), start_y),
                egui::pos2(ui.min_rect().left() + 4.0, end_y),
            );
            ui.painter().rect_filled(accent_rect, 1.0, color);
        }
        action
    }

    /// Edit AudioListener component
    fn edit_audio_listener(
        &self,
        ui: &mut Ui,
        world: &mut World,
        entity: Entity,
        category: ComponentCategory,
    ) -> Option<ComponentAction> {
        let mut action = None;
        if let Ok(mut listener) = world.get::<&mut AudioListener>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            CollapsingHeader::new(RichText::new("Audio Listener").strong())
                .default_open(true)
                .show(ui, |ui| {
                    ui.checkbox(&mut listener.active, "Active");

                    ui.add_space(4.0);
                    if ui
                        .button(RichText::new("Remove").color(Color32::from_rgb(220, 80, 80)))
                        .clicked()
                    {
                        action = Some(ComponentAction::RemoveAudioListener);
                    }
                });

            let end_y = ui.cursor().top();
            let accent_rect = egui::Rect::from_min_max(
                egui::pos2(ui.min_rect().left(), start_y),
                egui::pos2(ui.min_rect().left() + 4.0, end_y),
            );
            ui.painter().rect_filled(accent_rect, 1.0, color);
        }
        action
    }

    /// Edit ParticleEffect component (module-stack architecture)
    fn edit_particle_effect(
        &self,
        ui: &mut Ui,
        world: &mut World,
        entity: Entity,
        category: ComponentCategory,
    ) -> Option<ComponentAction> {
        let mut action = None;
        if let Ok(mut effect) = world.get::<&mut ParticleEffect>(entity) {
            let color = Self::category_color(category);
            let start_y = ui.cursor().top();

            CollapsingHeader::new(RichText::new("Particle Effect").strong())
                .default_open(true)
                .show(ui, |ui| {
                    // Preset dropdown
                    egui::ComboBox::from_label("Preset")
                        .selected_text("Select preset...")
                        .show_ui(ui, |ui| {
                            for (label, preset_fn) in [
                                ("Fire", ParticleEffect::fire as fn() -> ParticleEffect),
                                ("Smoke", ParticleEffect::smoke),
                                ("Sparks", ParticleEffect::sparks),
                                ("Dust", ParticleEffect::dust),
                            ] {
                                if ui.selectable_label(false, label).clicked() {
                                    let preset = preset_fn();
                                    let capacity = effect.capacity;
                                    let show_gizmos = effect.show_gizmos;
                                    let texture_path = effect.texture_path.clone();
                                    *effect = preset;
                                    effect.capacity = capacity;
                                    effect.show_gizmos = show_gizmos;
                                    effect.texture_path = texture_path;
                                }
                            }
                        });

                    ui.add_space(4.0);

                    // Lifecycle
                    CollapsingHeader::new("Lifecycle")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.checkbox(&mut effect.enabled, "Enabled");
                            ui.add(
                                egui::Slider::new(&mut effect.capacity, 256..=4096)
                                    .text("Capacity"),
                            );
                        });

                    // Emission
                    CollapsingHeader::new("Emission")
                        .default_open(true)
                        .show(ui, |ui| {
                            let shape_label = match effect.spawn_shape {
                                SpawnShape::Point => "Point",
                                SpawnShape::Sphere { .. } => "Sphere",
                                SpawnShape::Cone { .. } => "Cone",
                                SpawnShape::Box { .. } => "Box",
                            };
                            egui::ComboBox::from_label("Shape")
                                .selected_text(shape_label)
                                .show_ui(ui, |ui| {
                                    if ui.selectable_label(matches!(effect.spawn_shape, SpawnShape::Point), "Point").clicked() {
                                        effect.spawn_shape = SpawnShape::Point;
                                    }
                                    if ui.selectable_label(matches!(effect.spawn_shape, SpawnShape::Sphere { .. }), "Sphere").clicked() {
                                        effect.spawn_shape = SpawnShape::Sphere { radius: 1.0 };
                                    }
                                    if ui.selectable_label(matches!(effect.spawn_shape, SpawnShape::Cone { .. }), "Cone").clicked() {
                                        effect.spawn_shape = SpawnShape::Cone { angle_rad: 0.5, radius: 0.5 };
                                    }
                                    if ui.selectable_label(matches!(effect.spawn_shape, SpawnShape::Box { .. }), "Box").clicked() {
                                        effect.spawn_shape = SpawnShape::Box { half_extents: [0.5, 0.5, 0.5] };
                                    }
                                });

                            match &mut effect.spawn_shape {
                                SpawnShape::Point => {}
                                SpawnShape::Sphere { radius } => {
                                    ui.add(DragValue::new(radius).speed(0.05).prefix("Radius: "));
                                }
                                SpawnShape::Cone { angle_rad, radius } => {
                                    ui.add(DragValue::new(angle_rad).speed(0.01).prefix("Angle: "));
                                    ui.add(DragValue::new(radius).speed(0.05).prefix("Radius: "));
                                }
                                SpawnShape::Box { half_extents } => {
                                    ui.horizontal(|ui| {
                                        ui.label("Half Extents:");
                                        ui.add(DragValue::new(&mut half_extents[0]).speed(0.05).prefix("X: "));
                                        ui.add(DragValue::new(&mut half_extents[1]).speed(0.05).prefix("Y: "));
                                        ui.add(DragValue::new(&mut half_extents[2]).speed(0.05).prefix("Z: "));
                                    });
                                }
                            }

                            ui.add(DragValue::new(&mut effect.emission_rate).speed(0.5).range(0.0..=1000.0).prefix("Rate: "));
                            ui.add(DragValue::new(&mut effect.burst_count).speed(1.0).prefix("Burst Count: "));
                            ui.add(DragValue::new(&mut effect.burst_interval).speed(0.01).prefix("Burst Interval: "));
                        });

                    // Lifetime
                    CollapsingHeader::new("Lifetime")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.add(DragValue::new(&mut effect.lifetime_min).speed(0.01).range(0.01..=f32::MAX).prefix("Min: "));
                            ui.add(DragValue::new(&mut effect.lifetime_max).speed(0.01).range(0.01..=f32::MAX).prefix("Max: "));
                            if effect.lifetime_min > effect.lifetime_max {
                                effect.lifetime_max = effect.lifetime_min;
                            }
                        });

                    // Velocity
                    CollapsingHeader::new("Velocity")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Initial:");
                                ui.add(DragValue::new(&mut effect.initial_velocity[0]).speed(0.1).prefix("X: "));
                                ui.add(DragValue::new(&mut effect.initial_velocity[1]).speed(0.1).prefix("Y: "));
                                ui.add(DragValue::new(&mut effect.initial_velocity[2]).speed(0.1).prefix("Z: "));
                            });
                            ui.add(DragValue::new(&mut effect.velocity_variance).speed(0.05).range(0.0..=f32::MAX).prefix("Variance: "));
                        });

                    // Modules (composable update stack)
                    CollapsingHeader::new("Modules")
                        .default_open(true)
                        .show(ui, |ui| {
                            let mut remove_idx: Option<usize> = None;

                            for (idx, module) in effect.update_modules.iter_mut().enumerate() {
                                let id = ui.make_persistent_id(format!("module_{}", idx));
                                egui::collapsing_header::CollapsingState::load_with_default_open(
                                    ui.ctx(), id, true,
                                ).show_header(ui, |ui| {
                                    ui.label(RichText::new(module.display_name()).strong());
                                    if ui.small_button("X").clicked() {
                                        remove_idx = Some(idx);
                                    }
                                }).body(|ui| {
                                    match module {
                                        UpdateModule::Gravity(v) => {
                                            ui.horizontal(|ui| {
                                                ui.add(DragValue::new(&mut v[0]).speed(0.1).prefix("X: "));
                                                ui.add(DragValue::new(&mut v[1]).speed(0.1).prefix("Y: "));
                                                ui.add(DragValue::new(&mut v[2]).speed(0.1).prefix("Z: "));
                                            });
                                        }
                                        UpdateModule::Drag(v) => {
                                            ui.add(DragValue::new(v).speed(0.01).range(0.0..=f32::MAX).prefix("Drag: "));
                                        }
                                        UpdateModule::Wind(v) => {
                                            ui.horizontal(|ui| {
                                                ui.add(DragValue::new(&mut v[0]).speed(0.1).prefix("X: "));
                                                ui.add(DragValue::new(&mut v[1]).speed(0.1).prefix("Y: "));
                                                ui.add(DragValue::new(&mut v[2]).speed(0.1).prefix("Z: "));
                                            });
                                        }
                                        UpdateModule::CurlNoise { strength, scale, speed } => {
                                            ui.add(DragValue::new(strength).speed(0.05).range(0.0..=f32::MAX).prefix("Strength: "));
                                            ui.add(DragValue::new(scale).speed(0.05).range(0.01..=f32::MAX).prefix("Scale: "));
                                            ui.add(DragValue::new(speed).speed(0.01).prefix("Speed: "));
                                        }
                                        UpdateModule::ColorOverLife { start, end } => {
                                            ui.horizontal(|ui| {
                                                ui.label("Start:");
                                                let mut c = [
                                                    (start[0] * 255.0) as u8,
                                                    (start[1] * 255.0) as u8,
                                                    (start[2] * 255.0) as u8,
                                                    (start[3] * 255.0) as u8,
                                                ];
                                                if ui.color_edit_button_srgba_unmultiplied(&mut c).changed() {
                                                    *start = [
                                                        c[0] as f32 / 255.0,
                                                        c[1] as f32 / 255.0,
                                                        c[2] as f32 / 255.0,
                                                        c[3] as f32 / 255.0,
                                                    ];
                                                }
                                            });
                                            ui.horizontal(|ui| {
                                                ui.label("End:");
                                                let mut c = [
                                                    (end[0] * 255.0) as u8,
                                                    (end[1] * 255.0) as u8,
                                                    (end[2] * 255.0) as u8,
                                                    (end[3] * 255.0) as u8,
                                                ];
                                                if ui.color_edit_button_srgba_unmultiplied(&mut c).changed() {
                                                    *end = [
                                                        c[0] as f32 / 255.0,
                                                        c[1] as f32 / 255.0,
                                                        c[2] as f32 / 255.0,
                                                        c[3] as f32 / 255.0,
                                                    ];
                                                }
                                            });
                                        }
                                        UpdateModule::SizeOverLife { start, end } => {
                                            ui.add(DragValue::new(start).speed(0.005).range(0.0..=f32::MAX).prefix("Start: "));
                                            ui.add(DragValue::new(end).speed(0.005).range(0.0..=f32::MAX).prefix("End: "));
                                        }
                                    }
                                });
                            }

                            if let Some(idx) = remove_idx {
                                effect.update_modules.remove(idx);
                            }

                            // Add Module dropdown
                            ui.add_space(4.0);
                            egui::ComboBox::from_id_salt("add_module")
                                .selected_text("Add Module...")
                                .show_ui(ui, |ui| {
                                    if ui.selectable_label(false, "Gravity").clicked() {
                                        effect.update_modules.push(UpdateModule::Gravity([0.0, 0.0, -9.8]));
                                    }
                                    if ui.selectable_label(false, "Drag").clicked() {
                                        effect.update_modules.push(UpdateModule::Drag(0.5));
                                    }
                                    if ui.selectable_label(false, "Wind").clicked() {
                                        effect.update_modules.push(UpdateModule::Wind([1.0, 0.0, 0.0]));
                                    }
                                    if ui.selectable_label(false, "Curl Noise").clicked() {
                                        effect.update_modules.push(UpdateModule::CurlNoise {
                                            strength: 1.0, scale: 1.0, speed: 0.5,
                                        });
                                    }
                                    if ui.selectable_label(false, "Color Over Life").clicked() {
                                        effect.update_modules.push(UpdateModule::ColorOverLife {
                                            start: [1.0, 1.0, 1.0, 1.0],
                                            end: [1.0, 1.0, 1.0, 0.0],
                                        });
                                    }
                                    if ui.selectable_label(false, "Size Over Life").clicked() {
                                        effect.update_modules.push(UpdateModule::SizeOverLife {
                                            start: 0.1, end: 0.0,
                                        });
                                    }
                                });
                        });

                    // Texture & Soft Fade
                    CollapsingHeader::new("Rendering")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Texture:");
                                ui.text_edit_singleline(&mut effect.texture_path);
                            });
                            ui.add(
                                egui::Slider::new(&mut effect.soft_fade_distance, 0.0..=5.0)
                                    .text("Soft Fade"),
                            );
                        });

                    // Gizmos
                    ui.checkbox(&mut effect.show_gizmos, "Show Gizmos");

                    ui.add_space(4.0);
                    if ui
                        .button(RichText::new("Remove").color(Color32::from_rgb(220, 80, 80)))
                        .clicked()
                    {
                        action = Some(ComponentAction::RemoveParticleEffect);
                    }
                });

            let end_y = ui.cursor().top();
            let accent_rect = egui::Rect::from_min_max(
                egui::pos2(ui.min_rect().left(), start_y),
                egui::pos2(ui.min_rect().left() + 4.0, end_y),
            );
            ui.painter().rect_filled(accent_rect, 1.0, color);
        }
        action
    }

    /// Render "Add Component" UI with compatibility validation.
    /// Uses `self.cached_presence` to avoid per-frame world probing.
    fn render_add_component(&mut self, ui: &mut Ui, world: &mut World, entity: Entity) {
        let p = self.cached_presence;
        let has_rigidbody = p.has(ComponentPresence::RIGID_BODY);
        let has_collider = p.has(ComponentPresence::COLLIDER);
        let has_camera = p.has(ComponentPresence::CAMERA);
        let has_dir_light = p.has(ComponentPresence::DIR_LIGHT);
        let has_point_light = p.has(ComponentPresence::POINT_LIGHT);

        if has_rigidbody && !has_collider {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Warning: RigidBody without Collider")
                        .color(Color32::from_rgb(220, 180, 50)),
                );
            });
        } else if !has_rigidbody && has_collider {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Warning: Collider without RigidBody")
                        .color(Color32::from_rgb(220, 180, 50)),
                );
            });
        }

        ui.add_space(10.0);

        let mut added = false;
        egui::ComboBox::from_label("")
            .selected_text("Add Component...")
            .show_ui(ui, |ui| {
                if !has_camera {
                    let conflicts = has_dir_light || has_point_light;
                    if conflicts {
                        ui.add_enabled(false, egui::Button::selectable(false, "Camera"))
                            .on_disabled_hover_text("Conflicts with existing light component");
                    } else if ui.selectable_label(false, "Camera").clicked() {
                        let _ = world.insert_one(entity, Camera::default());
                        added = true;
                    }
                }

                if !has_dir_light {
                    let conflicts = has_camera || has_point_light;
                    if conflicts {
                        ui.add_enabled(false, egui::Button::selectable(false, "Directional Light"))
                            .on_disabled_hover_text(
                                "Conflicts with existing Camera or Point Light",
                            );
                    } else if ui.selectable_label(false, "Directional Light").clicked() {
                        let _ = world.insert_one(entity, DirectionalLight::default());
                        added = true;
                    }
                }

                if !has_point_light {
                    let conflicts = has_camera || has_dir_light;
                    if conflicts {
                        ui.add_enabled(false, egui::Button::selectable(false, "Point Light"))
                            .on_disabled_hover_text(
                                "Conflicts with existing Camera or Directional Light",
                            );
                    } else if ui.selectable_label(false, "Point Light").clicked() {
                        let _ = world.insert_one(entity, PointLight::default());
                        added = true;
                    }
                }

                if !p.has(ComponentPresence::MESH_RENDERER)
                    && ui.selectable_label(false, "Mesh Renderer").clicked()
                {
                    let _ = world.insert_one(entity, MeshRenderer::default());
                    added = true;
                }

                if !has_rigidbody && ui.selectable_label(false, "Rigid Body").clicked() {
                    let _ = world.insert_one(entity, RigidBody::default());
                    added = true;
                }

                if !has_collider && ui.selectable_label(false, "Collider").clicked() {
                    let _ = world.insert_one(entity, Collider::default());
                    added = true;
                }

                if !p.has(ComponentPresence::AUDIO_EMITTER)
                    && ui.selectable_label(false, "Audio Emitter").clicked()
                {
                    let _ = world.insert_one(entity, AudioEmitter::default());
                    added = true;
                }

                if !p.has(ComponentPresence::AUDIO_LISTENER)
                    && ui.selectable_label(false, "Audio Listener").clicked()
                {
                    let _ = world.insert_one(entity, AudioListener::default());
                    added = true;
                }

                ui.separator();
                ui.label(RichText::new("VFX").strong().size(11.0));

                if !p.has(ComponentPresence::PARTICLE_EFFECT)
                    && ui.selectable_label(false, "Particle Effect").clicked()
                {
                    let _ = world.insert_one(entity, ParticleEffect::default());
                    // Ensure the entity has an EntityGuid (required for plankton tracking)
                    if world.get::<&crate::engine::ecs::EntityGuid>(entity).is_err() {
                        let _ = world.insert_one(entity, crate::engine::ecs::EntityGuid::new());
                    }
                    added = true;
                }
            });

        if added {
            self.cached_presence = ComponentPresence::probe(world, entity);
        }
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
        if euler.x.is_finite() {
            euler.x.to_degrees()
        } else {
            0.0
        },
        if euler.y.is_finite() {
            euler.y.to_degrees()
        } else {
            0.0
        },
        if euler.z.is_finite() {
            euler.z.to_degrees()
        } else {
            0.0
        },
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
