//! Central material manager — owns all `MaterialBase` and `MaterialInstance` objects.
//!
//! Provides creation, lookup, and update methods. The geometry pass resolves
//! material descriptor sets through this manager at frame-packet build time.

use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::image::sampler::Sampler;
use vulkano::image::view::ImageView;
use vulkano::memory::allocator::StandardMemoryAllocator;
use vulkano::pipeline::PipelineLayout;

use super::material::{
    MaterialBase, MaterialBaseId, MaterialInstance, MaterialParamsGpu,
};

/// Unique identifier for a material instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct MaterialInstanceId(pub Uuid);

impl MaterialInstanceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Central registry for material bases and instances.
pub struct MaterialManager {
    bases: HashMap<MaterialBaseId, MaterialBase>,
    instances: HashMap<MaterialInstanceId, MaterialInstance>,
}

impl MaterialManager {
    pub fn new() -> Self {
        Self {
            bases: HashMap::new(),
            instances: HashMap::new(),
        }
    }

    /// Register a new material base (shared texture set).
    pub fn register_base(
        &mut self,
        albedo: Arc<ImageView>,
        normal: Arc<ImageView>,
        metallic_roughness: Arc<ImageView>,
        ao: Arc<ImageView>,
        sampler: Arc<Sampler>,
    ) -> MaterialBaseId {
        let id = MaterialBaseId::new();
        self.bases.insert(
            id,
            MaterialBase {
                id,
                albedo,
                normal,
                metallic_roughness,
                ao,
                sampler,
            },
        );
        id
    }

    /// Register a base with a pre-determined ID (for deserialization).
    pub fn register_base_with_id(
        &mut self,
        id: MaterialBaseId,
        albedo: Arc<ImageView>,
        normal: Arc<ImageView>,
        metallic_roughness: Arc<ImageView>,
        ao: Arc<ImageView>,
        sampler: Arc<Sampler>,
    ) {
        self.bases.insert(
            id,
            MaterialBase {
                id,
                albedo,
                normal,
                metallic_roughness,
                ao,
                sampler,
            },
        );
    }

    /// Look up a material base by ID.
    pub fn get_base(&self, id: MaterialBaseId) -> Option<&MaterialBase> {
        self.bases.get(&id)
    }

    /// Create a new material instance from a base and factor overrides.
    pub fn create_instance(
        &mut self,
        base_id: MaterialBaseId,
        base_color_factor: [f32; 4],
        metallic_factor: f32,
        roughness_factor: f32,
        emissive_factor: [f32; 3],
        allocator: Arc<StandardMemoryAllocator>,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        geom_pipeline_layout: Arc<PipelineLayout>,
    ) -> Result<MaterialInstanceId, Box<dyn std::error::Error>> {
        let base = self
            .bases
            .get(&base_id)
            .ok_or("MaterialBase not found")?;

        let instance = MaterialInstance::new(
            base,
            base_color_factor,
            metallic_factor,
            roughness_factor,
            emissive_factor,
            allocator,
            descriptor_set_allocator,
            geom_pipeline_layout,
        )?;

        let id = MaterialInstanceId::new();
        self.instances.insert(id, instance);
        Ok(id)
    }

    /// Create an instance with a pre-determined ID (for deserialization).
    pub fn create_instance_with_id(
        &mut self,
        id: MaterialInstanceId,
        base_id: MaterialBaseId,
        base_color_factor: [f32; 4],
        metallic_factor: f32,
        roughness_factor: f32,
        emissive_factor: [f32; 3],
        allocator: Arc<StandardMemoryAllocator>,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        geom_pipeline_layout: Arc<PipelineLayout>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let base = self
            .bases
            .get(&base_id)
            .ok_or("MaterialBase not found")?;

        let instance = MaterialInstance::new(
            base,
            base_color_factor,
            metallic_factor,
            roughness_factor,
            emissive_factor,
            allocator,
            descriptor_set_allocator,
            geom_pipeline_layout,
        )?;

        self.instances.insert(id, instance);
        Ok(())
    }

    /// Look up a material instance by ID.
    pub fn get_instance(&self, id: MaterialInstanceId) -> Option<&MaterialInstance> {
        self.instances.get(&id)
    }

    /// Update the per-instance factors and rebuild the UBO + descriptor set.
    pub fn update_instance(
        &mut self,
        id: MaterialInstanceId,
        base_color_factor: [f32; 4],
        metallic_factor: f32,
        roughness_factor: f32,
        emissive_factor: [f32; 3],
        allocator: Arc<StandardMemoryAllocator>,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        geom_pipeline_layout: Arc<PipelineLayout>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let instance = self
            .instances
            .get(&id)
            .ok_or("MaterialInstance not found")?;
        let base_id = instance.base_id;
        let base = self
            .bases
            .get(&base_id)
            .ok_or("MaterialBase not found")?;

        let new_instance = MaterialInstance::new(
            base,
            base_color_factor,
            metallic_factor,
            roughness_factor,
            emissive_factor,
            allocator,
            descriptor_set_allocator,
            geom_pipeline_layout,
        )?;

        self.instances.insert(id, new_instance);
        Ok(())
    }

    /// Get the GPU parameters for an instance.
    pub fn instance_params(&self, id: MaterialInstanceId) -> Option<&MaterialParamsGpu> {
        self.instances.get(&id).map(|i| &i.params)
    }

    pub fn base_count(&self) -> usize {
        self.bases.len()
    }

    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }
}

impl Default for MaterialManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// .matinst.ron serialization format
// ---------------------------------------------------------------------------

/// On-disk representation of a material instance (`.matinst.ron`).
///
/// Example:
/// ```ron
/// MaterialInstanceDef(
///     base_material: "materials/metal.material.ron",
///     base_color_factor: (1.0, 1.0, 1.0, 1.0),
///     metallic_factor: 1.0,
///     roughness_factor: 0.5,
///     emissive_factor: (0.0, 0.0, 0.0),
/// )
/// ```
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MaterialInstanceDef {
    /// Content-relative path to the base material asset.
    pub base_material: String,
    #[serde(default = "default_base_color")]
    pub base_color_factor: [f32; 4],
    #[serde(default = "default_one")]
    pub metallic_factor: f32,
    #[serde(default = "default_half")]
    pub roughness_factor: f32,
    #[serde(default)]
    pub emissive_factor: [f32; 3],
}

fn default_base_color() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}
fn default_one() -> f32 {
    1.0
}
fn default_half() -> f32 {
    0.5
}

impl MaterialInstanceDef {
    /// Load from a `.matinst.ron` file.
    pub fn load(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let contents = std::fs::read_to_string(path)?;
        let def: Self = ron::from_str(&contents)?;
        Ok(def)
    }

    /// Save to a `.matinst.ron` file.
    pub fn save(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        let pretty = ron::ser::PrettyConfig::new()
            .depth_limit(3)
            .struct_names(true);
        let contents = ron::ser::to_string_pretty(self, pretty)?;
        std::fs::write(path, contents)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matinst_ron_roundtrip() {
        let def = MaterialInstanceDef {
            base_material: "materials/metal.material.ron".to_string(),
            base_color_factor: [0.8, 0.2, 0.1, 1.0],
            metallic_factor: 0.9,
            roughness_factor: 0.3,
            emissive_factor: [1.0, 0.5, 0.0],
        };

        let temp = std::env::temp_dir().join("test_matinst_roundtrip.matinst.ron");
        def.save(&temp).expect("save");

        let loaded = MaterialInstanceDef::load(&temp).expect("load");
        assert_eq!(loaded.base_material, def.base_material);
        assert_eq!(loaded.base_color_factor, def.base_color_factor);
        assert!((loaded.metallic_factor - def.metallic_factor).abs() < 1e-6);
        assert!((loaded.roughness_factor - def.roughness_factor).abs() < 1e-6);
        assert_eq!(loaded.emissive_factor, def.emissive_factor);

        let _ = std::fs::remove_file(&temp);
    }

    #[test]
    fn matinst_ron_defaults() {
        let ron_str = r#"MaterialInstanceDef(base_material: "materials/test.material.ron")"#;
        let def: MaterialInstanceDef = ron::from_str(ron_str).expect("parse with defaults");
        assert_eq!(def.base_color_factor, [1.0, 1.0, 1.0, 1.0]);
        assert!((def.metallic_factor - 1.0).abs() < 1e-6);
        assert!((def.roughness_factor - 0.5).abs() < 1e-6);
        assert_eq!(def.emissive_factor, [0.0, 0.0, 0.0]);
    }
}
