pub mod vulkan_context;
pub mod physical_device;
pub mod logical_device;
pub mod swapchain;
pub mod renderer;
pub mod render_pass;
pub mod framebuffer;
pub mod pipeline;
pub mod texture;
pub mod components;
pub mod sprite_batch; 
pub mod camera;
pub mod input;
pub mod scene;
pub mod game_loop;
pub mod coords;
pub mod mesh;
pub mod depth_buffer;
pub mod model_loader;
pub mod mesh_manager;


pub use vulkan_context::VulkanContext;
pub use physical_device::select_physical_device;
pub use logical_device::{create_logical_device, LogicalDeviceContext};
//pub use swapchain::create_swapchain;
pub use renderer::Renderer;
pub use texture::load_texture;
pub use components::{Transform2D, SpriteSheet, Animation, AnimationController, AnimationStateMachine, AnimationTransition, TransitionCondition};
pub use sprite_batch::{SpriteBatch, AnimatedSprite};
pub use camera::*;
pub use input::InputManager;
pub use scene::{Scene, Entity, EntityId, SpriteComponent};
pub use game_loop::GameLoop;
pub use coords::{
    GameplayTransform, CoordinateSystem,
    convert_position_zup_to_yup, convert_position_yup_to_zup,
    convert_rotation_zup_to_yup, convert_transform_zup_to_yup,
    zup, yup,
};
pub use mesh::{create_cube, create_plane};
pub use depth_buffer::create_depth_buffer;
pub use model_loader::{load_gltf, print_gltf_info, load_model, Model, LoadedMesh};
pub use mesh_manager::{MeshManager, GpuMesh};