pub mod common;

#[path = "2d"]
pub mod rendering_2d {
    pub mod sprite_batch;
    pub mod pipeline_2d;

    pub use sprite_batch::{SpriteBatch, AnimatedSprite};
    pub use pipeline_2d::*;
}

#[path = "3d"]
pub mod rendering_3d {
    pub mod mesh;
    pub mod mesh_manager;
    pub mod light;
    pub mod pipeline_3d;
    pub mod material;
    pub mod shadow;

    pub use mesh::{create_cube, create_plane};
    pub use mesh_manager::{GpuMesh, MeshManager};
    pub use light::{DirectionalLight, PointLight, AmbientLight};
    pub use pipeline_3d::*;
    pub use material::*;
    pub use shadow::*;
}

pub use common::*;