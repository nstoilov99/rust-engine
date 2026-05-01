//! Core ECS components
use nalgebra_glm as glm;
use serde::{Deserialize, Deserializer, Serialize};

/// 3D transform component
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Transform {
    #[serde(with = "vec3_serde")]
    pub position: glm::Vec3,
    #[serde(with = "quat_serde")]
    pub rotation: glm::Quat,
    #[serde(with = "vec3_serde")]
    pub scale: glm::Vec3,
}

impl Transform {
    pub fn new(position: glm::Vec3) -> Self {
        Self {
            position,
            rotation: glm::quat_identity(),
            scale: glm::vec3(1.0, 1.0, 1.0),
        }
    }

    pub fn with_rotation(mut self, rotation: glm::Quat) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_scale(mut self, scale: glm::Vec3) -> Self {
        self.scale = scale;
        self
    }

    /// Build the local transform matrix in Z-up space (no coordinate conversion).
    ///
    /// This composes Translation * Rotation * Scale in the game's native Z-up
    /// coordinate system. Use this for:
    /// - Hierarchy composition (parent * child matrices)
    /// - Physics calculations
    /// - Any game logic that needs world-space matrices
    ///
    /// For rendering, convert the result using `render_adapter::world_matrix_to_render()`.
    pub fn local_matrix_zup(&self) -> glm::Mat4 {
        let translation = glm::translation(&self.position);
        let rotation = glm::quat_to_mat4(&self.rotation);
        let scale = glm::scaling(&self.scale);
        translation * rotation * scale
    }

    /// Calculate the model matrix for this transform, converted to render space (Y-up).
    ///
    /// **Note**: This returns only the LOCAL transform. For entities with parents,
    /// use `hierarchy::get_world_transform()` followed by `render_adapter::world_matrix_to_render()`.
    ///
    /// ECS uses Z-up coordinates. Vulkan uses Y-up for rendering.
    /// Delegates to render_adapter for centralized coordinate conversion.
    pub fn model_matrix(&self) -> glm::Mat4 {
        crate::engine::adapters::render_adapter::transform_to_model_matrix(self)
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: glm::vec3(0.0, 0.0, 0.0),
            rotation: glm::quat_identity(),
            scale: glm::vec3(1.0, 1.0, 1.0),
        }
    }
}

/// Mesh renderer component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshRenderer {
    /// Content-relative path to a `.mesh` asset (e.g. "Defeated.mesh")
    #[serde(default)]
    pub mesh_path: String,
    /// Per-submesh material paths (content-relative `.material.ron` paths).
    /// Index 0 = first submesh, etc. Empty vec or missing entries = default material.
    #[serde(default)]
    pub material_paths: Vec<String>,
    /// Backward-compat: old single material_path (migrated to material_paths[0] at load).
    #[serde(default, skip_serializing)]
    pub material_path: String,
    /// Runtime-resolved GPU mesh index (not serialized)
    #[serde(skip)]
    pub mesh_index: usize,
    /// Runtime-resolved material sort key (not serialized)
    #[serde(skip)]
    pub material_index: usize,
    /// Whether this mesh is rendered
    #[serde(default = "default_true")]
    pub visible: bool,
    /// Whether this mesh casts shadows (when shadow mapping is active)
    #[serde(default = "default_true")]
    pub cast_shadows: bool,
    /// Whether this mesh receives shadows from other objects
    #[serde(default = "default_true")]
    pub receive_shadows: bool,
    /// Per-instance base color factor override (RGBA).
    #[serde(default = "default_base_color_factor")]
    pub base_color_factor: [f32; 4],
    /// Per-instance metallic factor override.
    #[serde(default = "default_one")]
    pub metallic_factor: f32,
    /// Per-instance roughness factor override.
    #[serde(default = "default_half")]
    pub roughness_factor: f32,
    /// Per-instance emissive factor override (RGB).
    #[serde(default)]
    pub emissive_factor: [f32; 3],
}

impl MeshRenderer {
    /// Migrate legacy `material_path` to `material_paths` if needed.
    pub fn migrate_legacy_material_path(&mut self) {
        if !self.material_path.is_empty() && self.material_paths.is_empty() {
            self.material_paths.push(std::mem::take(&mut self.material_path));
        }
    }
}

impl Default for MeshRenderer {
    fn default() -> Self {
        Self {
            mesh_path: String::new(),
            material_paths: Vec::new(),
            material_path: String::new(),
            mesh_index: 0,
            material_index: 0,
            visible: true,
            cast_shadows: true,
            receive_shadows: true,
            base_color_factor: [1.0, 1.0, 1.0, 1.0],
            metallic_factor: 1.0,
            roughness_factor: 0.5,
            emissive_factor: [0.0, 0.0, 0.0],
        }
    }
}

/// Camera projection type
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CameraProjection {
    Perspective,
    Orthographic { size: f32 },
}

impl Default for CameraProjection {
    fn default() -> Self {
        Self::Perspective
    }
}

/// Camera component for 3D rendering
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Camera {
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    pub active: bool,
    /// Projection type (perspective or orthographic)
    #[serde(default)]
    pub projection: CameraProjection,
    /// Background clear color (RGB)
    #[serde(default = "default_clear_color")]
    pub clear_color: [f32; 3],
    /// Render priority (higher renders on top)
    #[serde(default)]
    pub priority: i32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            fov: 60.0,
            near: 0.1,
            far: 1000.0,
            active: true,
            projection: CameraProjection::default(),
            clear_color: default_clear_color(),
            priority: 0,
        }
    }
}

/// Directional light component
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DirectionalLight {
    #[serde(with = "vec3_serde")]
    pub direction: glm::Vec3,
    #[serde(with = "vec3_serde")]
    pub color: glm::Vec3,
    pub intensity: f32,
    /// Enable shadow mapping for this light
    #[serde(default)]
    pub shadow_enabled: bool,
    /// Shadow bias to prevent shadow acne artifacts
    #[serde(default = "default_shadow_bias")]
    pub shadow_bias: f32,
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self {
            direction: glm::vec3(0.0, -1.0, 0.0),
            color: glm::vec3(1.0, 1.0, 1.0),
            intensity: 1.0,
            shadow_enabled: false,
            shadow_bias: default_shadow_bias(),
        }
    }
}

/// Light attenuation falloff model
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LightFalloff {
    /// Light fades linearly with distance
    Linear,
    /// Physically-based quadratic falloff
    #[default]
    Quadratic,
    /// Realistic inverse-square law
    InverseSquare,
}

/// Point light component
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PointLight {
    #[serde(with = "vec3_serde")]
    pub color: glm::Vec3,
    pub intensity: f32,
    pub radius: f32,
    /// Enable shadow mapping for this light
    #[serde(default)]
    pub shadow_enabled: bool,
    /// Attenuation falloff model
    #[serde(default)]
    pub falloff: LightFalloff,
}

impl Default for PointLight {
    fn default() -> Self {
        Self {
            color: glm::vec3(1.0, 1.0, 1.0),
            intensity: 1.0,
            radius: 10.0,
            shadow_enabled: false,
            falloff: LightFalloff::default(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_base_color_factor() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}

fn default_one() -> f32 {
    1.0
}

fn default_half() -> f32 {
    0.5
}

fn default_clear_color() -> [f32; 3] {
    [0.1, 0.1, 0.15]
}

fn default_shadow_bias() -> f32 {
    0.005
}

/// Spawn shape for particle emitters (Z-up game space)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SpawnShape {
    Point,
    Sphere { radius: f32 },
    Cone { angle_rad: f32, radius: f32 },
    Box { half_extents: [f32; 3] },
}

impl Default for SpawnShape {
    fn default() -> Self {
        Self::Point
    }
}

/// Composable particle simulation module.
///
/// Each variant represents a single behavior applied during the simulate stage.
/// Modules are evaluated in order, so a `Gravity` followed by `Drag` applies
/// drag to the gravity-accelerated velocity.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum UpdateModule {
    Gravity([f32; 3]),
    Drag(f32),
    Wind([f32; 3]),
    CurlNoise { strength: f32, scale: f32, speed: f32 },
    ColorOverLife { start: [f32; 4], end: [f32; 4] },
    SizeOverLife { start: f32, end: f32 },
}

impl UpdateModule {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Gravity(_) => "Gravity",
            Self::Drag(_) => "Drag",
            Self::Wind(_) => "Wind",
            Self::CurlNoise { .. } => "Curl Noise",
            Self::ColorOverLife { .. } => "Color Over Life",
            Self::SizeOverLife { .. } => "Size Over Life",
        }
    }
}

/// Particle render mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderMode {
    Billboard,
    // Ribbon, Mesh, Light — future
}

impl Default for RenderMode {
    fn default() -> Self {
        Self::Billboard
    }
}

/// Deserialize capacity with clamping to 256..=4096
fn deserialize_clamped_capacity<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let val = u32::deserialize(deserializer)?;
    let clamped = val.clamp(256, 4096);
    if clamped != val {
        log::warn!(
            "ParticleEffect capacity {} clamped to {}",
            val,
            clamped
        );
    }
    Ok(clamped)
}

/// Composable GPU particle effect component (codename: Plankton).
///
/// Core emitter properties (spawn shape, rate, lifetime, velocity) live as
/// direct fields. Simulation behaviors are composed via an ordered list of
/// `UpdateModule` entries. The GPU pipeline flattens modules into per-frame
/// push constants — the module list itself is an authoring-side concept.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleEffect {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_plankton_capacity", deserialize_with = "deserialize_clamped_capacity")]
    pub capacity: u32,
    #[serde(default = "default_plankton_emission_rate")]
    pub emission_rate: f32,
    #[serde(default)]
    pub burst_count: u32,
    #[serde(default)]
    pub burst_interval: f32,
    #[serde(default)]
    pub spawn_shape: SpawnShape,
    #[serde(default = "default_plankton_lifetime_min")]
    pub lifetime_min: f32,
    #[serde(default = "default_plankton_lifetime_max")]
    pub lifetime_max: f32,
    #[serde(default = "default_plankton_initial_velocity")]
    pub initial_velocity: [f32; 3],
    #[serde(default)]
    pub velocity_variance: f32,
    #[serde(default)]
    pub update_modules: Vec<UpdateModule>,
    #[serde(default)]
    pub render_mode: RenderMode,
    #[serde(default)]
    pub texture_path: String,
    #[serde(default)]
    pub soft_fade_distance: f32,
    #[serde(default)]
    pub show_gizmos: bool,
}

fn default_plankton_capacity() -> u32 { 2048 }
fn default_plankton_emission_rate() -> f32 { 20.0 }
fn default_plankton_lifetime_min() -> f32 { 1.0 }
fn default_plankton_lifetime_max() -> f32 { 2.0 }
fn default_plankton_initial_velocity() -> [f32; 3] { [0.0, 0.0, 2.0] } // upward in Z-up

impl Default for ParticleEffect {
    fn default() -> Self {
        Self {
            enabled: true,
            capacity: default_plankton_capacity(),
            emission_rate: default_plankton_emission_rate(),
            burst_count: 0,
            burst_interval: 0.0,
            spawn_shape: SpawnShape::default(),
            lifetime_min: default_plankton_lifetime_min(),
            lifetime_max: default_plankton_lifetime_max(),
            initial_velocity: default_plankton_initial_velocity(),
            velocity_variance: 0.5,
            update_modules: vec![
                UpdateModule::ColorOverLife {
                    start: [1.0, 1.0, 1.0, 1.0],
                    end: [1.0, 1.0, 1.0, 0.0],
                },
                UpdateModule::SizeOverLife {
                    start: 0.1,
                    end: 0.0,
                },
            ],
            render_mode: RenderMode::default(),
            texture_path: String::new(),
            soft_fade_distance: 0.0,
            show_gizmos: false,
        }
    }
}

impl ParticleEffect {
    // === Module query helpers ===

    pub fn gravity(&self) -> Option<[f32; 3]> {
        self.update_modules.iter().find_map(|m| match m {
            UpdateModule::Gravity(v) => Some(*v),
            _ => None,
        })
    }

    pub fn drag(&self) -> Option<f32> {
        self.update_modules.iter().find_map(|m| match m {
            UpdateModule::Drag(v) => Some(*v),
            _ => None,
        })
    }

    pub fn wind(&self) -> Option<[f32; 3]> {
        self.update_modules.iter().find_map(|m| match m {
            UpdateModule::Wind(v) => Some(*v),
            _ => None,
        })
    }

    pub fn curl_noise(&self) -> Option<(f32, f32, f32)> {
        self.update_modules.iter().find_map(|m| match m {
            UpdateModule::CurlNoise { strength, scale, speed } => Some((*strength, *scale, *speed)),
            _ => None,
        })
    }

    pub fn color_over_life(&self) -> Option<([f32; 4], [f32; 4])> {
        self.update_modules.iter().find_map(|m| match m {
            UpdateModule::ColorOverLife { start, end } => Some((*start, *end)),
            _ => None,
        })
    }

    pub fn size_over_life(&self) -> Option<(f32, f32)> {
        self.update_modules.iter().find_map(|m| match m {
            UpdateModule::SizeOverLife { start, end } => Some((*start, *end)),
            _ => None,
        })
    }

    // === Presets ===

    /// Fire preset: upward velocity, orange->red, short life
    pub fn fire() -> Self {
        Self {
            emission_rate: 60.0,
            lifetime_min: 0.3,
            lifetime_max: 0.8,
            initial_velocity: [0.0, 0.0, 3.0],
            velocity_variance: 1.0,
            update_modules: vec![
                UpdateModule::Gravity([0.0, 0.0, 1.0]),
                UpdateModule::ColorOverLife {
                    start: [1.0, 0.6, 0.1, 1.0],
                    end: [1.0, 0.1, 0.0, 0.0],
                },
                UpdateModule::SizeOverLife {
                    start: 0.15,
                    end: 0.02,
                },
            ],
            ..Self::default()
        }
    }

    /// Smoke preset: slow rise, gray, long fade
    pub fn smoke() -> Self {
        Self {
            emission_rate: 15.0,
            lifetime_min: 2.0,
            lifetime_max: 4.0,
            initial_velocity: [0.0, 0.0, 0.8],
            velocity_variance: 0.3,
            update_modules: vec![
                UpdateModule::Drag(0.5),
                UpdateModule::CurlNoise { strength: 1.0, scale: 0.5, speed: 0.3 },
                UpdateModule::ColorOverLife {
                    start: [0.5, 0.5, 0.5, 0.6],
                    end: [0.3, 0.3, 0.3, 0.0],
                },
                UpdateModule::SizeOverLife {
                    start: 0.1,
                    end: 0.4,
                },
            ],
            ..Self::default()
        }
    }

    /// Sparks preset: random dirs, bright yellow, gravity, short life
    pub fn sparks() -> Self {
        Self {
            emission_rate: 40.0,
            lifetime_min: 0.5,
            lifetime_max: 1.5,
            initial_velocity: [0.0, 0.0, 5.0],
            velocity_variance: 3.0,
            update_modules: vec![
                UpdateModule::Gravity([0.0, 0.0, -9.8]),
                UpdateModule::ColorOverLife {
                    start: [1.0, 0.9, 0.3, 1.0],
                    end: [1.0, 0.4, 0.0, 0.0],
                },
                UpdateModule::SizeOverLife {
                    start: 0.04,
                    end: 0.01,
                },
            ],
            ..Self::default()
        }
    }

    /// Dust preset: slow drift, gray, long life
    pub fn dust() -> Self {
        Self {
            emission_rate: 10.0,
            lifetime_min: 3.0,
            lifetime_max: 6.0,
            initial_velocity: [0.0, 0.0, 0.2],
            velocity_variance: 0.5,
            update_modules: vec![
                UpdateModule::Drag(1.0),
                UpdateModule::CurlNoise { strength: 0.5, scale: 2.0, speed: 0.1 },
                UpdateModule::ColorOverLife {
                    start: [0.6, 0.55, 0.5, 0.4],
                    end: [0.5, 0.45, 0.4, 0.0],
                },
                UpdateModule::SizeOverLife {
                    start: 0.03,
                    end: 0.05,
                },
            ],
            ..Self::default()
        }
    }
}

/// Marker component indicating that an entity's transform has changed
/// and its world matrix (and descendants' matrices) need to be recomputed.
///
/// Added by mutation sites (inspector, gizmo, physics sync, undo/redo).
/// Cleared by `TransformCache::propagate_incremental()` after re-propagation.
#[derive(Debug, Clone, Copy)]
pub struct TransformDirty;

/// Tag component for player-controlled entities
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Player;

/// Tag component for naming entities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Name(pub String);

impl Name {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

/// Globally unique identifier for entities.
/// Survives serialization round-trips and is used for snapshot restore.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityGuid(pub uuid::Uuid);

impl EntityGuid {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    pub fn from_string(s: &str) -> Option<Self> {
        uuid::Uuid::parse_str(s).ok().map(Self)
    }
}

impl Default for EntityGuid {
    fn default() -> Self {
        Self::new()
    }
}

// ========== Custom Serde for nalgebra-glm types ==========

mod vec3_serde {
    use nalgebra_glm as glm;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Vec3Surrogate {
        x: f32,
        y: f32,
        z: f32,
    }

    pub fn serialize<S>(vec: &glm::Vec3, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let surrogate = Vec3Surrogate {
            x: vec.x,
            y: vec.y,
            z: vec.z,
        };
        surrogate.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<glm::Vec3, D::Error>
    where
        D: Deserializer<'de>,
    {
        let surrogate = Vec3Surrogate::deserialize(deserializer)?;
        Ok(glm::vec3(surrogate.x, surrogate.y, surrogate.z))
    }
}

mod quat_serde {
    use nalgebra_glm as glm;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct QuatSurrogate {
        x: f32,
        y: f32,
        z: f32,
        w: f32,
    }

    pub fn serialize<S>(quat: &glm::Quat, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let surrogate = QuatSurrogate {
            x: quat.coords.x,
            y: quat.coords.y,
            z: quat.coords.z,
            w: quat.coords.w,
        };
        surrogate.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<glm::Quat, D::Error>
    where
        D: Deserializer<'de>,
    {
        let surrogate = QuatSurrogate::deserialize(deserializer)?;
        // glm::quat takes parameters in order (x, y, z, w), NOT (w, x, y, z)!
        Ok(glm::quat(
            surrogate.x,
            surrogate.y,
            surrogate.z,
            surrogate.w,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_default_is_identity() {
        let t = Transform::default();
        assert_eq!(t.position, glm::vec3(0.0, 0.0, 0.0));
        assert!((t.rotation.coords.w - 1.0).abs() < 1e-6);
        assert!(t.rotation.coords.x.abs() < 1e-6);
        assert!(t.rotation.coords.y.abs() < 1e-6);
        assert!(t.rotation.coords.z.abs() < 1e-6);
        assert_eq!(t.scale, glm::vec3(1.0, 1.0, 1.0));
    }

    #[test]
    fn transform_new_sets_position() {
        let t = Transform::new(glm::vec3(1.0, 2.0, 3.0));
        assert_eq!(t.position, glm::vec3(1.0, 2.0, 3.0));
        // Default rotation and scale
        assert!((t.rotation.coords.w - 1.0).abs() < 1e-6);
        assert_eq!(t.scale, glm::vec3(1.0, 1.0, 1.0));
    }

    #[test]
    fn transform_builder_with_scale() {
        let t = Transform::new(glm::vec3(0.0, 0.0, 0.0)).with_scale(glm::vec3(2.0, 3.0, 4.0));
        assert_eq!(t.scale, glm::vec3(2.0, 3.0, 4.0));
    }

    #[test]
    fn transform_builder_with_rotation() {
        let rot = glm::quat_angle_axis(std::f32::consts::FRAC_PI_2, &glm::vec3(0.0, 0.0, 1.0));
        let t = Transform::new(glm::vec3(0.0, 0.0, 0.0)).with_rotation(rot);

        let diff = (t.rotation.coords - rot.coords).norm();
        assert!(diff < 1e-6, "rotation should match");
    }

    #[test]
    fn transform_local_matrix_identity() {
        let t = Transform::default();
        let mat = t.local_matrix_zup();
        let expected = glm::identity::<f32, 4>();

        for i in 0..4 {
            for j in 0..4 {
                assert!(
                    (mat[(i, j)] - expected[(i, j)]).abs() < 1e-5,
                    "matrix[{},{}] should be identity",
                    i,
                    j
                );
            }
        }
    }

    #[test]
    fn transform_local_matrix_translation() {
        let t = Transform::new(glm::vec3(5.0, 10.0, 15.0));
        let mat = t.local_matrix_zup();

        // Translation is in the last column
        assert!((mat[(0, 3)] - 5.0).abs() < 1e-5);
        assert!((mat[(1, 3)] - 10.0).abs() < 1e-5);
        assert!((mat[(2, 3)] - 15.0).abs() < 1e-5);
    }

    #[test]
    fn transform_local_matrix_scale() {
        let t = Transform::new(glm::vec3(0.0, 0.0, 0.0)).with_scale(glm::vec3(2.0, 3.0, 4.0));
        let mat = t.local_matrix_zup();
        let det = glm::determinant(&mat);

        assert!(
            (det - 24.0).abs() < 1e-3,
            "det should be 2*3*4=24, got {}",
            det
        );
    }

    #[test]
    fn transform_rotation_preserves_orthogonality() {
        let rot = glm::quat_angle_axis(0.7, &glm::vec3(1.0, 1.0, 0.0));
        let rot = glm::quat_normalize(&rot);
        let t = Transform::new(glm::vec3(0.0, 0.0, 0.0)).with_rotation(rot);
        let mat = t.local_matrix_zup();

        // Extract 3x3 rotation part and check orthogonality
        let col0 = glm::vec3(mat[(0, 0)], mat[(1, 0)], mat[(2, 0)]);
        let col1 = glm::vec3(mat[(0, 1)], mat[(1, 1)], mat[(2, 1)]);
        let col2 = glm::vec3(mat[(0, 2)], mat[(1, 2)], mat[(2, 2)]);

        assert!(
            glm::dot(&col0, &col1).abs() < 1e-5,
            "columns should be orthogonal"
        );
        assert!(
            glm::dot(&col0, &col2).abs() < 1e-5,
            "columns should be orthogonal"
        );
        assert!(
            glm::dot(&col1, &col2).abs() < 1e-5,
            "columns should be orthogonal"
        );
    }

    #[test]
    fn entity_guid_uniqueness() {
        let a = EntityGuid::new();
        let b = EntityGuid::new();
        assert_ne!(a, b, "two new GUIDs should be different");
    }

    #[test]
    fn entity_guid_from_string_roundtrip() {
        let original = EntityGuid::new();
        let string = original.0.to_string();
        let parsed = EntityGuid::from_string(&string).expect("should parse");
        assert_eq!(original, parsed);
    }

    #[test]
    fn entity_guid_invalid_string() {
        assert!(EntityGuid::from_string("not-a-uuid").is_none());
    }

    #[test]
    fn transform_serde_roundtrip() {
        let t = Transform::new(glm::vec3(1.0, 2.0, 3.0))
            .with_scale(glm::vec3(0.5, 1.5, 2.5))
            .with_rotation(glm::quat_angle_axis(1.0, &glm::vec3(0.0, 0.0, 1.0)));

        let serialized = ron::to_string(&t).expect("serialize");
        let deserialized: Transform = ron::from_str(&serialized).expect("deserialize");

        assert!((deserialized.position.x - t.position.x).abs() < 1e-5);
        assert!((deserialized.position.y - t.position.y).abs() < 1e-5);
        assert!((deserialized.position.z - t.position.z).abs() < 1e-5);
        assert!((deserialized.scale.x - t.scale.x).abs() < 1e-5);
        assert!((deserialized.rotation.coords.w - t.rotation.coords.w).abs() < 1e-5);
    }
}
