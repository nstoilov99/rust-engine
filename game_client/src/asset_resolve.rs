//! Scene asset resolution shared by the editor and standalone builds.
//!
//! After a scene loads, `MeshRenderer` components only carry asset *paths*.
//! These helpers load the referenced meshes onto the GPU, sync material slots
//! from `.mesh.ron` sidecars, and build the material descriptor-set cache that
//! `render_loop::prepare_mesh_data` binds from.

use hecs::World;
use rust_engine::assets::AssetManager;
use rust_engine::engine::ecs::components::MeshRenderer;
use rust_engine::engine::rendering::rendering_3d::material::MaterialBaseId;
use rust_engine::engine::rendering::rendering_3d::{
    MaterialInstanceDef, MaterialInstanceId, MaterialManager,
};
use std::collections::HashMap;
use std::sync::Arc;
use vulkano::descriptor_set::DescriptorSet;

/// GPU handles needed to create material textures and descriptor sets.
pub struct MaterialGpu {
    pub allocator: Arc<vulkano::memory::allocator::StandardMemoryAllocator>,
    pub ds_allocator: Arc<vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator>,
    pub cmd_allocator: Arc<vulkano::command_buffer::allocator::StandardCommandBufferAllocator>,
    pub queue: Arc<vulkano::device::Queue>,
    pub device: Arc<vulkano::device::Device>,
    pub geom_layout: Arc<vulkano::pipeline::PipelineLayout>,
}

/// Material descriptor-set cache plus the instance bookkeeping behind it.
#[derive(Default)]
pub struct MaterialStore {
    /// Descriptor sets keyed by content-relative material path.
    pub cache: HashMap<String, Arc<DescriptorSet>>,
    pub manager: MaterialManager,
    pub base_ids: HashMap<String, MaterialBaseId>,
    pub matinst_ids: HashMap<String, MaterialInstanceId>,
}

/// Resolve `mesh_path` to `mesh_index` for all MeshRenderer components.
///
/// Loads meshes via the AssetManager if they aren't already uploaded.
/// Also auto-adapts `material_paths` from the `.mesh.ron` sidecar when a
/// mesh is first resolved.
pub fn resolve_mesh_paths(world: &mut World, asset_manager: &Arc<AssetManager>) {
    // Collect paths that need resolving
    let mut paths_to_load: Vec<String> = Vec::new();
    for (_entity, mr) in world.query_mut::<&MeshRenderer>() {
        if !mr.mesh_path.is_empty() {
            let meshes = asset_manager.meshes.read();
            if meshes.first_index_for_path(&mr.mesh_path).is_none() {
                paths_to_load.push(mr.mesh_path.clone());
            }
        }
    }

    // Load unique paths
    paths_to_load.sort();
    paths_to_load.dedup();
    for path in &paths_to_load {
        if let Err(e) = asset_manager.load_model_gpu(path) {
            log::warn!("Failed to load mesh '{}': {}", path, e);
        }
    }

    // Collect mesh paths that need material slot sync from sidecar.
    // This includes newly loaded paths AND any resolved meshes whose
    // material_paths are still empty (e.g. after re-import).
    let mut needs_sidecar: Vec<String> = paths_to_load.clone();
    for (_entity, mr) in world.query_mut::<&MeshRenderer>() {
        if !mr.mesh_path.is_empty()
            && !mr.mesh_path.starts_with("__primitive__/")
            && mr.material_paths.iter().all(|p| p.is_empty())
        {
            needs_sidecar.push(mr.mesh_path.clone());
        }
    }
    needs_sidecar.sort();
    needs_sidecar.dedup();

    // Load sidecar material slot info
    let mut sidecar_slots: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(content_root) = rust_engine::engine::assets::asset_source::content_root_path() {
        for path in &needs_sidecar {
            let mesh_fs_path = content_root.join(path);
            if let Ok(meta) =
                rust_engine::engine::assets::mesh_import::load_mesh_sidecar(&mesh_fs_path)
            {
                // Material paths in sidecar are relative to the mesh file's directory.
                // Convert them to content-relative paths.
                let mesh_dir = std::path::Path::new(path)
                    .parent()
                    .unwrap_or(std::path::Path::new(""));
                let mat_paths: Vec<String> = meta
                    .material_slots
                    .iter()
                    .map(|s| {
                        if s.material_path.is_empty() {
                            String::new()
                        } else {
                            mesh_dir
                                .join(&s.material_path)
                                .to_string_lossy()
                                .replace('\\', "/")
                        }
                    })
                    .collect();
                if !mat_paths.is_empty() {
                    sidecar_slots.insert(path.clone(), mat_paths);
                }
            }
        }
    }

    // Resolve indices and sync material slots
    let meshes = asset_manager.meshes.read();
    for (_entity, mr) in world.query_mut::<&mut MeshRenderer>() {
        if !mr.mesh_path.is_empty() {
            if let Some(idx) = meshes.first_index_for_path(&mr.mesh_path) {
                mr.mesh_index = idx;
            }
            // Auto-adapt material_paths from sidecar when all slots are empty
            if let Some(slots) = sidecar_slots.get(&mr.mesh_path) {
                let all_empty = mr.material_paths.iter().all(|p| p.is_empty());
                if all_empty || mr.material_paths.len() != slots.len() {
                    mr.material_paths = slots.clone();
                }
            }
        }
    }
}

/// Load `.material` / `.matinst` files referenced by MeshRenderers and cache their
/// GPU descriptor sets so `prepare_mesh_data` can bind them.
pub fn resolve_material_sets(
    world: &mut World,
    asset_manager: &Arc<AssetManager>,
    gpu: &MaterialGpu,
    store: &mut MaterialStore,
) {
    use rust_engine::engine::assets::mesh_import::load_material_ron;
    use rust_engine::engine::rendering::rendering_3d::material::{
        create_default_texture_with_format, PbrMaterial, DEFAULT_AO_RGBA,
        DEFAULT_METALLIC_ROUGHNESS_RGBA, DEFAULT_NORMAL_RGBA,
    };
    use vulkano::format::Format;
    use vulkano::image::sampler::{Filter, Sampler, SamplerAddressMode, SamplerCreateInfo};

    // Collect unique material paths that aren't cached yet
    let mut uncached: Vec<String> = Vec::new();
    for (_entity, mr) in world.query_mut::<&MeshRenderer>() {
        for mp in &mr.material_paths {
            if !mp.is_empty() && !store.cache.contains_key(mp) {
                uncached.push(mp.clone());
            }
        }
    }
    uncached.sort();
    uncached.dedup();

    if uncached.is_empty() {
        return;
    }

    let content_root = match rust_engine::engine::assets::asset_source::content_root_path() {
        Some(r) => r,
        None => return,
    };

    let sampler = match Sampler::new(
        gpu.device.clone(),
        SamplerCreateInfo {
            mag_filter: Filter::Linear,
            min_filter: Filter::Linear,
            address_mode: [SamplerAddressMode::Repeat; 3],
            ..Default::default()
        },
    ) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Failed to create material sampler: {}", e);
            return;
        }
    };

    for mat_path in &uncached {
        // `.matinst` assets resolve through the MaterialManager: the base
        // material supplies the textures, the instance supplies factors.
        let norm = mat_path.replace('\\', "/");
        if norm.ends_with(".material.ron") || norm.ends_with(".matinst.ron") {
            log::warn!(
                "Legacy '.ron'-suffixed material path: {} — rename via tools/migrate_asset_extensions",
                mat_path
            );
        }
        let inst_def = if norm.ends_with(".matinst.ron") || norm.ends_with(".matinst") {
            match MaterialInstanceDef::load(&content_root.join(mat_path)) {
                Ok(d) => Some(d),
                Err(e) => {
                    log::warn!("Failed to load material instance '{}': {}", mat_path, e);
                    continue;
                }
            }
        } else {
            None
        };

        // The file whose textures get loaded: the base material for instances.
        let base_rel = inst_def
            .as_ref()
            .map(|d| d.base_material.clone())
            .unwrap_or_else(|| mat_path.clone());

        // Instance whose base is already registered: skip texture loading.
        if let Some(inst) = &inst_def {
            if let Some(&base_id) = store.base_ids.get(&base_rel) {
                cache_material_instance(mat_path, base_id, inst, gpu, store);
                continue;
            }
        }

        let mat_ron_path = content_root.join(&base_rel);
        let mat_dir = mat_ron_path.parent().unwrap_or(&content_root);

        let def = match load_material_ron(&mat_ron_path) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("Failed to load material '{}': {}", base_rel, e);
                continue;
            }
        };

        // Helper: load a texture relative to the material file's directory,
        // or return a 1×1 fallback.
        let load_tex = |filename: &str,
                        fallback: [u8; 4],
                        format: Format|
         -> Result<Arc<vulkano::image::view::ImageView>, Box<dyn std::error::Error>> {
            if filename.is_empty() {
                return create_default_texture_with_format(
                    gpu.allocator.clone(),
                    gpu.cmd_allocator.clone(),
                    gpu.queue.clone(),
                    fallback,
                    format,
                );
            }
            // Try to find the texture relative to the material file
            let tex_path = mat_dir.join(filename);
            if tex_path.exists() {
                // Convert to content-relative for TextureManager
                let content_relative = tex_path
                    .strip_prefix(&content_root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| filename.to_string());
                match asset_manager.textures.load(&content_relative) {
                    Ok(handle) => Ok(handle.get_arc()),
                    Err(e) => {
                        log::warn!("Failed to load texture '{}': {}", content_relative, e);
                        create_default_texture_with_format(
                            gpu.allocator.clone(),
                            gpu.cmd_allocator.clone(),
                            gpu.queue.clone(),
                            fallback,
                            format,
                        )
                    }
                }
            } else {
                log::warn!("Material texture not found: {}", tex_path.display());
                create_default_texture_with_format(
                    gpu.allocator.clone(),
                    gpu.cmd_allocator.clone(),
                    gpu.queue.clone(),
                    fallback,
                    format,
                )
            }
        };

        let albedo = match load_tex(
            &def.albedo_texture,
            [255, 255, 255, 255],
            Format::R8G8B8A8_SRGB,
        ) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let normal = match load_tex(
            &def.normal_texture,
            DEFAULT_NORMAL_RGBA,
            Format::R8G8B8A8_UNORM,
        ) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let mr = match load_tex(
            &def.metallic_roughness_texture,
            DEFAULT_METALLIC_ROUGHNESS_RGBA,
            Format::R8G8B8A8_UNORM,
        ) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ao = match load_tex(&def.ao_texture, DEFAULT_AO_RGBA, Format::R8G8B8A8_UNORM) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let Some(inst) = &inst_def {
            let base_id =
                store
                    .manager
                    .register_base(albedo, normal, mr, ao, sampler.clone());
            store.base_ids.insert(base_rel.clone(), base_id);
            cache_material_instance(mat_path, base_id, inst, gpu, store);
            continue;
        }

        match PbrMaterial::new(
            albedo,
            normal,
            mr,
            ao,
            sampler.clone(),
            def.base_color_factor,
            def.metallic_factor,
            def.roughness_factor,
            def.emissive_factor,
            gpu.allocator.clone(),
            gpu.ds_allocator.clone(),
            gpu.geom_layout.clone(),
        ) {
            Ok(mat) => {
                println!("✅ Loaded material: {}", mat_path);
                store.cache.insert(mat_path.clone(), mat.descriptor_set);
            }
            Err(e) => {
                log::warn!("Failed to create PbrMaterial for '{}': {}", mat_path, e);
            }
        }
    }
}

/// Create a `MaterialInstance` from a registered base and cache its
/// descriptor set under the instance's asset path.
fn cache_material_instance(
    mat_path: &str,
    base_id: MaterialBaseId,
    def: &MaterialInstanceDef,
    gpu: &MaterialGpu,
    store: &mut MaterialStore,
) {
    match store.manager.create_instance(
        base_id,
        def.base_color_factor,
        def.metallic_factor,
        def.roughness_factor,
        def.emissive_factor,
        gpu.allocator.clone(),
        gpu.ds_allocator.clone(),
        gpu.geom_layout.clone(),
    ) {
        Ok(id) => {
            if let Some(inst) = store.manager.get_instance(id) {
                store.matinst_ids.insert(mat_path.to_string(), id);
                store
                    .cache
                    .insert(mat_path.to_string(), inst.descriptor_set.clone());
                println!("✅ Loaded material instance: {}", mat_path);
            }
        }
        Err(e) => {
            log::warn!("Failed to create material instance '{}': {}", mat_path, e);
        }
    }
}
