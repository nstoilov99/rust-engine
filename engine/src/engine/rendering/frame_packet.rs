//! Frame packet types for cross-thread render data transfer.
//!
//! `FramePacket` is the sole data interface between the main thread
//! (game logic, ECS, egui layout) and the render thread (command
//! recording, GPU submission). All fields are owned/cloned — no
//! shared mutable state crosses the boundary.

use crate::engine::debug_draw::DebugDrawData;
use crate::engine::rendering::rendering_3d::{LightUniformData, MeshRenderData};
use glam::{Mat4, Vec3};
#[cfg(feature = "editor")]
use std::sync::Arc;
#[cfg(feature = "editor")]
use vulkano::image::view::ImageView;

/// The render mode determines which code path the render thread uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    Standalone,
    #[cfg(feature = "editor")]
    Editor,
}

/// A command to bind or update an egui texture slot on the render thread.
#[cfg(feature = "editor")]
pub struct TextureBindCommand {
    pub texture_id: egui::TextureId,
    pub image_view: Arc<ImageView>,
}

/// All data needed by the render thread to produce one frame.
pub struct FramePacket {
    // Scene data
    pub mesh_data: Vec<MeshRenderData>,
    pub light_data: LightUniformData,
    pub view_proj: Mat4,
    pub camera_pos: Vec3,
    pub grid_visible: bool,
    pub debug_draw: DebugDrawData,

    // egui data (None until Step 7 wires up the egui split)
    #[cfg(feature = "editor")]
    pub egui_primitives: Option<Vec<egui::ClippedPrimitive>>,
    #[cfg(feature = "editor")]
    pub egui_texture_deltas: Option<egui::TexturesDelta>,

    // Render config
    pub render_mode: RenderMode,
    pub window_dimensions: [u32; 2],
    #[cfg(feature = "editor")]
    pub viewport_dimensions: Option<[u32; 2]>,

    // Frame metadata
    pub frame_number: u64,

    // Texture bind commands (for egui texture slot protocol)
    #[cfg(feature = "editor")]
    pub texture_binds: Vec<TextureBindCommand>,

    /// The egui TextureId assigned to the viewport texture, so the render
    /// thread can update its EguiRenderer cache after a viewport resize.
    #[cfg(feature = "editor")]
    pub viewport_texture_id: Option<egui::TextureId>,
}

/// Events sent from the render thread back to the main thread.
pub enum RenderEvent {
    RenderThreadReady {
        #[cfg(feature = "editor")]
        viewport_texture: Option<Arc<ImageView>>,
    },
    SwapchainRecreated {
        dimensions: [u32; 2],
    },
    #[cfg(feature = "editor")]
    ViewportTextureChanged {
        texture_id: egui::TextureId,
        image_view: Arc<ImageView>,
    },
    RenderError {
        message: String,
    },
}

impl FramePacket {
    /// Build a standalone-mode frame packet from prepared render data.
    pub fn build_standalone(
        mesh_data: Vec<MeshRenderData>,
        light_data: LightUniformData,
        view_proj: Mat4,
        camera_pos: Vec3,
        grid_visible: bool,
        debug_draw: DebugDrawData,
        window_dimensions: [u32; 2],
        frame_number: u64,
    ) -> Self {
        Self {
            mesh_data,
            light_data,
            view_proj,
            camera_pos,
            grid_visible,
            debug_draw,
            #[cfg(feature = "editor")]
            egui_primitives: None,
            #[cfg(feature = "editor")]
            egui_texture_deltas: None,
            render_mode: RenderMode::Standalone,
            window_dimensions,
            #[cfg(feature = "editor")]
            viewport_dimensions: None,
            frame_number,
            #[cfg(feature = "editor")]
            texture_binds: Vec::new(),
            #[cfg(feature = "editor")]
            viewport_texture_id: None,
        }
    }

    /// Build an editor-mode frame packet from prepared render data.
    #[cfg(feature = "editor")]
    pub fn build_editor(
        mesh_data: Vec<MeshRenderData>,
        light_data: LightUniformData,
        view_proj: Mat4,
        camera_pos: Vec3,
        grid_visible: bool,
        debug_draw: DebugDrawData,
        window_dimensions: [u32; 2],
        viewport_dimensions: Option<[u32; 2]>,
        frame_number: u64,
    ) -> Self {
        Self {
            mesh_data,
            light_data,
            view_proj,
            camera_pos,
            grid_visible,
            debug_draw,
            egui_primitives: None,
            egui_texture_deltas: None,
            render_mode: RenderMode::Editor,
            window_dimensions,
            viewport_dimensions,
            frame_number,
            texture_binds: Vec::new(),
            viewport_texture_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn test_frame_packet_is_send() {
        assert_send::<FramePacket>();
    }

    #[test]
    fn test_render_event_is_send() {
        assert_send::<RenderEvent>();
    }
}
