pub mod common;
pub mod counters;
pub mod frame_packet;
pub mod graph;
pub mod pipeline_registry;
pub mod render_target;
pub mod render_thread;
#[cfg(feature = "editor")]
pub mod shader_compiler;

#[path = "2d"]
pub mod rendering_2d {
    pub mod pipeline_2d;
    pub mod sprite_batch;

    pub use pipeline_2d::*;
    pub use sprite_batch::{AnimatedSprite, SpriteBatch};
}

#[path = "3d"]
pub mod rendering_3d {
    pub mod deferred;
    pub mod light;
    pub mod material;
    pub mod material_manager;
    pub mod mesh;
    pub mod mesh_manager;
    pub mod pipeline_3d;
    pub mod shadow;
    pub mod skinning;

    pub use deferred::*;
    pub use light::{AmbientLight, DirectionalLight, PointLight};
    pub use material::*;
    pub use material_manager::{MaterialInstanceDef, MaterialInstanceId, MaterialManager};
    pub use mesh::{create_cube, create_plane};
    pub use mesh_manager::{GpuMesh, MeshManager};
    pub use pipeline_3d::*;
    pub use shadow::*;
    pub use skinning::SkinningBackend;
}

pub use common::*;
pub use counters::*;
pub use render_target::RenderTarget;
