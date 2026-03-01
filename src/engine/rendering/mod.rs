pub mod common;
pub mod render_target;

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
    pub mod mesh;
    pub mod mesh_manager;
    pub mod pipeline_3d;
    pub mod shadow;

    pub use deferred::*;
    pub use light::{AmbientLight, DirectionalLight, PointLight};
    pub use material::*;
    pub use mesh::{create_cube, create_plane};
    pub use mesh_manager::{GpuMesh, MeshManager};
    pub use pipeline_3d::*;
    pub use shadow::*;
}

pub use common::*;
pub use render_target::RenderTarget;
