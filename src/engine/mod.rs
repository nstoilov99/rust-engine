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

// GUI (egui integration, editor only)
#[cfg(feature = "editor")]
pub mod gui;

// Physics system (Rapier 3D integration)
pub mod benchmark;
pub mod physics;

// Coordinate system adapters (Z-up ↔ Y-up conversion)
pub mod adapters;

// Editor systems and UI panels (editor only)
#[cfg(feature = "editor")]
pub mod editor;

// Math utilities (frustum culling, etc.)
pub mod math;

// Re-export commonly used types
pub use assets::{load_gltf, load_model, load_texture, LoadedMesh, Model};
pub use camera::*; // Camera2D, Camera3D, CameraPushConstants
pub use core::{
    create_logical_device, select_physical_device, LogicalDeviceContext, VulkanContext,
};
pub use input::InputManager;
pub use physics::{Collider, ColliderShape, PhysicsWorld, RigidBody, RigidBodyType, Velocity};
pub use rendering::common::*; // Renderer, framebuffer functions, etc.
pub use rendering::rendering_2d::{AnimatedSprite, SpriteBatch};
pub use rendering::rendering_3d::{
    create_cube, create_plane, AmbientLight, DirectionalLight, GpuMesh, MeshManager, PointLight,
};
pub use scene::*; // Scene, Entity, SpriteComponent, Transform2D, SpriteSheet, Animation, etc.
pub use utils::*; // GameLoop, coords functions

// Commonly used external types (optional, for convenience)
pub use glam::{Mat4, Vec2, Vec3};
