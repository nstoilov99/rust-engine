// Core Vulkan systems
pub mod core;

// Rendering systems
pub mod rendering;

// Scene management (includes components)
pub mod scene;

// Camera systems
pub mod camera;

// Input handling
pub mod input;

// Asset loading
pub mod assets;

// Utilities
pub mod utils;

// Entity Component System
pub mod ecs; 

// Re-export commonly used types
pub use core::{VulkanContext, select_physical_device, create_logical_device, LogicalDeviceContext};
pub use rendering::common::*;  // Renderer, framebuffer functions, etc.
pub use rendering::rendering_2d::{SpriteBatch, AnimatedSprite};
pub use rendering::rendering_3d::{GpuMesh, MeshManager, DirectionalLight, PointLight, AmbientLight, create_cube, create_plane};
pub use scene::*;  // Scene, Entity, SpriteComponent, Transform2D, SpriteSheet, Animation, etc.
pub use camera::*;  // Camera2D, Camera3D, CameraPushConstants
pub use input::InputManager;
pub use assets::{load_texture, load_gltf, load_model, Model, LoadedMesh};
pub use utils::*;  // GameLoop, coords functions

// Commonly used external types (optional, for convenience)
pub use glam::{Vec2, Vec3, Mat4};