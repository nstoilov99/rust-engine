//! Transform gizmo integration using transform-gizmo-egui
//!
//! Uses a forked version of transform-gizmo that natively supports Z-up coordinates.
//! No coordinate conversion is needed - transforms are passed directly.

use glam::{Mat4, Vec3, Vec4};
use hecs::{Entity, World};
use nalgebra_glm as glm;
use transform_gizmo_egui::{
    mint::{Quaternion, RowMatrix4, Vector3, Vector4},
    EnumSet, Gizmo, GizmoConfig, GizmoExt, GizmoMode as TGMode, GizmoOrientation as TGOrientation,
};

use crate::engine::ecs::components::Transform;

use super::settings::{GizmoMode, GizmoOrientation, ToolMode};

/// Result from gizmo interaction
pub enum GizmoInteractionResult {
    /// No interaction
    None,
    /// Transform is being actively modified
    Transforming {
        entity: Entity,
        new_transform: Transform,
    },
    /// Drag operation just ended (create undo command)
    DragEnded {
        entity: Entity,
        start_transform: Transform,
        end_transform: Transform,
    },
}

/// Gizmo handler wrapping transform-gizmo-egui
pub struct GizmoHandler {
    gizmo: Gizmo,
    /// Current manipulation mode
    pub mode: GizmoMode,
    /// Local or world orientation
    pub orientation: GizmoOrientation,
    /// Whether snapping is enabled
    pub snapping_enabled: bool,
    /// Translation snap increment (units)
    pub snap_translate: f32,
    /// Rotation snap increment (degrees)
    pub snap_rotate: f32,
    /// Scale snap increment
    pub snap_scale: f32,
    /// Whether gizmo is currently being dragged
    is_dragging: bool,
    /// Transform at drag start (for undo)
    drag_start_transform: Option<Transform>,
    /// Entity being manipulated
    drag_entity: Option<Entity>,
}

impl GizmoHandler {
    /// Create a new gizmo handler
    pub fn new() -> Self {
        Self {
            gizmo: Gizmo::default(),
            mode: GizmoMode::Translate,
            orientation: GizmoOrientation::World,
            snapping_enabled: false,
            snap_translate: 1.0,
            snap_rotate: 15.0,
            snap_scale: 0.1,
            is_dragging: false,
            drag_start_transform: None,
            drag_entity: None,
        }
    }

    /// Returns true if the gizmo is currently being dragged
    pub fn is_dragging(&self) -> bool {
        self.is_dragging
    }

    /// Update and render the gizmo
    ///
    /// # Arguments
    /// * `ui` - egui UI context
    /// * `view_matrix` - Camera view matrix (Z-up)
    /// * `projection_matrix` - Camera projection matrix
    /// * `viewport_rect` - Viewport rectangle in screen space
    /// * `selected_entity` - Currently selected entity (if any)
    /// * `world` - ECS world for reading/writing transforms
    ///
    /// # Returns
    /// Result indicating if transform changed or drag ended
    pub fn update(
        &mut self,
        ui: &mut egui::Ui,
        view_matrix: Mat4,
        projection_matrix: Mat4,
        viewport_rect: egui::Rect,
        selected_entity: Option<Entity>,
        world: &World,
    ) -> GizmoInteractionResult {
        // No entity selected - nothing to do
        let Some(entity) = selected_entity else {
            self.reset_drag_state();
            return GizmoInteractionResult::None;
        };

        // Get transform component
        let Ok(transform_ref) = world.get::<&Transform>(entity) else {
            self.reset_drag_state();
            return GizmoInteractionResult::None;
        };

        // Clone the transform to avoid borrow issues
        let transform: Transform = *transform_ref;
        drop(transform_ref);

        // Adapt Y-up view matrix to work with Z-up world coordinates (for MVP projection)
        let view_adapted = adapt_view_matrix_for_zup(view_matrix);
        let view = mat4_to_row_matrix4(view_adapted);
        let proj = mat4_to_row_matrix4(projection_matrix);

        // Extract camera orientation vectors from Y-up view matrix and convert to Z-up world space
        // View matrix rows (from look_at_rh): row0=right, row1=up, row2=back
        let up_yup = Vec3::new(
            view_matrix.x_axis.y,
            view_matrix.y_axis.y,
            view_matrix.z_axis.y,
        );
        let back_yup = Vec3::new(
            view_matrix.x_axis.z,
            view_matrix.y_axis.z,
            view_matrix.z_axis.z,
        );

        // Convert Y-up direction vectors to Z-up: (x, y, z) → (-z, x, y)
        // This maps: Y-up X(right)→Z-up Y(right), Y-up Y(up)→Z-up Z(up), Y-up Z(back)→Z-up -X(back)
        let view_up = Vec3::new(-up_yup.z, up_yup.x, up_yup.y);
        let view_back = Vec3::new(-back_yup.z, back_yup.x, back_yup.y);
        let view_forward = -view_back; // Forward points into scene

        // Compute right from forward × up to ensure correct handedness in Z-up
        // (The direct conversion of right_yup gives wrong handedness)
        let view_right = view_forward.cross(view_up);

        // Configure gizmo with explicit view vector overrides for correct view-aligned elements
        let config = GizmoConfig {
            view_matrix: view,
            projection_matrix: proj,
            // Override view vectors so view-aligned elements (half circles, rotation rings) work correctly
            view_forward_override: Some(Vector3 {
                x: view_forward.x as f64,
                y: view_forward.y as f64,
                z: view_forward.z as f64,
            }),
            view_up_override: Some(Vector3 {
                x: view_up.x as f64,
                y: view_up.y as f64,
                z: view_up.z as f64,
            }),
            view_right_override: Some(Vector3 {
                x: view_right.x as f64,
                y: view_right.y as f64,
                z: view_right.z as f64,
            }),
            viewport: viewport_rect,
            modes: self.get_gizmo_modes(),
            orientation: self.get_gizmo_orientation(),
            snapping: self.snapping_enabled,
            snap_distance: self.snap_translate,
            snap_angle: self.snap_rotate.to_radians(),
            snap_scale: self.snap_scale,
            // Use default visuals from the forked library (already has correct Z-up colors)
            ..Default::default()
        };

        // Update gizmo config
        self.gizmo.update_config(config);

        // Convert ECS transform to gizmo transform - direct pass-through for Z-up
        let gizmo_transform = self.ecs_to_gizmo_transform(&transform);

        // Run gizmo interaction
        let interaction_result = self.gizmo.interact(ui, &[gizmo_transform]);

        // Handle result
        if let Some((_result, new_transforms)) = interaction_result {
            // Gizmo is being manipulated
            if !self.is_dragging {
                // Drag just started
                self.is_dragging = true;
                self.drag_start_transform = Some(transform);
                self.drag_entity = Some(entity);
            }

            // Convert new transform back to ECS space
            if let Some(new_gizmo_transform) = new_transforms.first() {
                let new_transform = self.gizmo_to_ecs_transform(new_gizmo_transform, &transform);
                return GizmoInteractionResult::Transforming {
                    entity,
                    new_transform,
                };
            }
        } else {
            // Check if drag just ended
            if self.is_dragging {
                let pointer_released = ui.input(|i| !i.pointer.any_down());
                if pointer_released {
                    if let (Some(start), Some(drag_entity)) =
                        (self.drag_start_transform.take(), self.drag_entity.take())
                    {
                        self.is_dragging = false;
                        return GizmoInteractionResult::DragEnded {
                            entity: drag_entity,
                            start_transform: start,
                            end_transform: transform,
                        };
                    }
                }
            }
        }

        GizmoInteractionResult::None
    }

    /// Reset drag state
    fn reset_drag_state(&mut self) {
        self.is_dragging = false;
        self.drag_start_transform = None;
        self.drag_entity = None;
    }

    /// Convert gizmo mode to library EnumSet
    fn get_gizmo_modes(&self) -> EnumSet<TGMode> {
        match self.mode {
            ToolMode::Select => {
                // No gizmo modes for selection tool
                EnumSet::empty()
            }
            ToolMode::Translate => {
                TGMode::TranslateX
                    | TGMode::TranslateY
                    | TGMode::TranslateZ
                    | TGMode::TranslateXY
                    | TGMode::TranslateXZ
                    | TGMode::TranslateYZ
                    | TGMode::TranslateView
            }
            ToolMode::Rotate => {
                TGMode::RotateX
                    | TGMode::RotateY
                    | TGMode::RotateZ
                    | TGMode::RotateView
                    | TGMode::Arcball
            }
            ToolMode::Scale => {
                TGMode::ScaleX | TGMode::ScaleY | TGMode::ScaleZ | TGMode::ScaleUniform
            }
        }
    }

    /// Check if gizmo should be shown (not in select mode)
    pub fn should_show_gizmo(&self) -> bool {
        self.mode != ToolMode::Select
    }

    /// Convert gizmo orientation to library type
    fn get_gizmo_orientation(&self) -> TGOrientation {
        match self.orientation {
            GizmoOrientation::Local => TGOrientation::Local,
            GizmoOrientation::World => TGOrientation::Global,
        }
    }

    /// Convert ECS Transform (Z-up) to gizmo transform
    /// Direct pass-through since the forked gizmo uses Z-up natively
    fn ecs_to_gizmo_transform(&self, t: &Transform) -> transform_gizmo_egui::math::Transform {
        transform_gizmo_egui::math::Transform {
            translation: Vector3 {
                x: t.position.x as f64,
                y: t.position.y as f64,
                z: t.position.z as f64,
            },
            rotation: Quaternion {
                v: Vector3 {
                    x: t.rotation.coords.x as f64,
                    y: t.rotation.coords.y as f64,
                    z: t.rotation.coords.z as f64,
                },
                s: t.rotation.coords.w as f64,
            },
            scale: Vector3 {
                x: t.scale.x as f64,
                y: t.scale.y as f64,
                z: t.scale.z as f64,
            },
        }
    }

    /// Convert gizmo transform back to ECS Transform
    /// Direct pass-through since the forked gizmo uses Z-up natively
    /// Only updates the component(s) that match the current gizmo mode.
    fn gizmo_to_ecs_transform(
        &mut self,
        gizmo_transform: &transform_gizmo_egui::math::Transform,
        original: &Transform,
    ) -> Transform {
        match self.mode {
            ToolMode::Select => {
                // Select mode doesn't modify transforms
                *original
            }
            ToolMode::Translate => {
                // Only update position, keep original rotation and scale
                Transform {
                    position: glm::vec3(
                        gizmo_transform.translation.x as f32,
                        gizmo_transform.translation.y as f32,
                        gizmo_transform.translation.z as f32,
                    ),
                    rotation: original.rotation,
                    scale: original.scale,
                }
            }
            ToolMode::Rotate => {
                // Only update rotation, keep original position and scale
                // Convert mint quaternion (v.xyz, s=w) to nalgebra_glm (x, y, z, w)
                // glm::quat takes parameters in order (x, y, z, w), NOT (w, x, y, z)!
                Transform {
                    position: original.position,
                    rotation: glm::quat(
                        gizmo_transform.rotation.v.x as f32,
                        gizmo_transform.rotation.v.y as f32,
                        gizmo_transform.rotation.v.z as f32,
                        gizmo_transform.rotation.s as f32,
                    ),
                    scale: original.scale,
                }
            }
            ToolMode::Scale => {
                // Only update scale, keep original position and rotation
                Transform {
                    position: original.position,
                    rotation: original.rotation,
                    scale: glm::vec3(
                        gizmo_transform.scale.x as f32,
                        gizmo_transform.scale.y as f32,
                        gizmo_transform.scale.z as f32,
                    ),
                }
            }
        }
    }
}

impl Default for GizmoHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Adapt a Y-up view matrix to work with Z-up world coordinates
///
/// The camera produces a Y-up view matrix (Vulkan convention).
/// The ECS uses Z-up coordinates. This function multiplies the
/// view matrix by a basis change matrix so that Z-up world
/// positions are correctly transformed to camera space.
///
/// Coordinate systems:
/// - Y-up (camera): X=right, Y=up, Z=back (towards camera)
/// - Z-up (world):  X=forward, Y=right, Z=up
///
/// To convert a Z-up world position to Y-up for the view matrix:
/// - Y-up X (right)  = Z-up Y (right)
/// - Y-up Y (up)     = Z-up Z (up)
/// - Y-up Z (back)   = -Z-up X (forward negated = back)
fn adapt_view_matrix_for_zup(view_yup: Mat4) -> Mat4 {
    // Basis change matrix: converts Z-up world coordinates to Y-up
    // Each column says where each Z-up axis goes in Y-up space
    // Column-major format: each Vec4 is a column
    let zup_to_yup = Mat4::from_cols(
        Vec4::new(0.0, 0.0, -1.0, 0.0), // Z-up X (forward) → Y-up -Z (back)
        Vec4::new(1.0, 0.0, 0.0, 0.0),  // Z-up Y (right)   → Y-up X (right)
        Vec4::new(0.0, 1.0, 0.0, 0.0),  // Z-up Z (up)      → Y-up Y (up)
        Vec4::new(0.0, 0.0, 0.0, 1.0),
    );

    view_yup * zup_to_yup
}

/// Convert glam Mat4 (column-major) to mint RowMatrix4<f64> (row-major)
fn mat4_to_row_matrix4(m: Mat4) -> RowMatrix4<f64> {
    RowMatrix4 {
        x: Vector4 {
            x: m.x_axis.x as f64,
            y: m.y_axis.x as f64,
            z: m.z_axis.x as f64,
            w: m.w_axis.x as f64,
        },
        y: Vector4 {
            x: m.x_axis.y as f64,
            y: m.y_axis.y as f64,
            z: m.z_axis.y as f64,
            w: m.w_axis.y as f64,
        },
        z: Vector4 {
            x: m.x_axis.z as f64,
            y: m.y_axis.z as f64,
            z: m.z_axis.z as f64,
            w: m.w_axis.z as f64,
        },
        w: Vector4 {
            x: m.x_axis.w as f64,
            y: m.y_axis.w as f64,
            z: m.z_axis.w as f64,
            w: m.w_axis.w as f64,
        },
    }
}
