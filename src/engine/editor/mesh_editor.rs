//! Mesh editor panel — 3D preview viewport + per-submesh material slot details.

use std::sync::Arc;

use glam::{Mat4, Vec3};
use vulkano::buffer::Subbuffer;
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::command_buffer::{
    AutoCommandBufferBuilder, CommandBufferUsage, PrimaryAutoCommandBuffer,
    RenderPassBeginInfo, SubpassBeginInfo, SubpassContents, SubpassEndInfo,
};
use vulkano::device::{Device, DeviceOwned, Queue};
use vulkano::format::Format;
use vulkano::image::view::ImageView;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::DescriptorSet;
use vulkano::memory::allocator::{AllocationCreateInfo, StandardMemoryAllocator};
use vulkano::pipeline::graphics::color_blend::{ColorBlendAttachmentState, ColorBlendState};
use vulkano::pipeline::graphics::depth_stencil::{DepthState, DepthStencilState};
use vulkano::pipeline::graphics::input_assembly::InputAssemblyState;
use vulkano::pipeline::graphics::multisample::MultisampleState;
use vulkano::pipeline::graphics::rasterization::RasterizationState;
use vulkano::pipeline::graphics::vertex_input::{Vertex as VertexTrait, VertexDefinition};
use vulkano::pipeline::graphics::viewport::{Viewport, ViewportState as VkViewportState};
use vulkano::pipeline::graphics::GraphicsPipelineCreateInfo;
use vulkano::pipeline::layout::PipelineDescriptorSetLayoutCreateInfo;
use vulkano::pipeline::{
    DynamicState, GraphicsPipeline, Pipeline, PipelineLayout, PipelineShaderStageCreateInfo,
};
use vulkano::render_pass::{Framebuffer, FramebufferCreateInfo, RenderPass};

use super::asset_browser::AssetBrowserPanel;
use super::viewport_texture::ViewportTexture;
use crate::engine::assets::asset_type::AssetType;
use crate::engine::assets::handle::AssetId;
use crate::engine::assets::mesh_import::{MaterialSlot, MeshImportMeta};
use crate::engine::rendering::rendering_3d::mesh_manager::MeshManager;
use crate::engine::rendering::rendering_3d::pipeline_3d::Vertex3D;
use crate::engine::rendering::rendering_3d::SkinningBackend;
use vulkano::pipeline::PipelineBindPoint;
use egui::{CollapsingHeader, Color32, RichText, ScrollArea, Stroke, Ui};

/// GPU mesh data for preview rendering: (vertex_buffer, index_buffer, index_count).
pub type GpuMeshBuffers = (Subbuffer<[Vertex3D]>, Subbuffer<[u32]>, u32);

// ---------------------------------------------------------------------------
// Shader modules (reuse the same GLSL files as the thumbnail renderer)
// ---------------------------------------------------------------------------

mod preview_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/3d/thumbnail_vs.glsl",
    }
}

mod preview_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/3d/thumbnail_fs.glsl",
    }
}

// ---------------------------------------------------------------------------
// MeshPreviewRenderer — forward pipeline (created once, shared across editors)
// ---------------------------------------------------------------------------

pub struct MeshPreviewRenderer {
    queue: Arc<Queue>,
    memory_allocator: Arc<StandardMemoryAllocator>,
    command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    render_pass: Arc<RenderPass>,
    pipeline: Arc<GraphicsPipeline>,
    identity_palette_set: Arc<DescriptorSet>,
}

impl MeshPreviewRenderer {
    pub fn new(
        device: Arc<Device>,
        queue: Arc<Queue>,
        memory_allocator: Arc<StandardMemoryAllocator>,
        command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Use the single_pass_renderpass! macro which defaults final_layout
        // to ColorAttachmentOptimal.  The preview CB and egui CB are chained
        // in the same submission (secondary_window.rs), so vulkano's
        // AutoCommandBufferBuilder detects the layout mismatch
        // (ColorAttachmentOptimal → ShaderReadOnlyOptimal) and inserts a
        // pipeline barrier with COLOR_ATTACHMENT_WRITE → SHADER_READ access
        // flags — guaranteeing memory visibility for the egui fragment shader.
        let render_pass = vulkano::single_pass_renderpass!(
            device.clone(),
            attachments: {
                color: {
                    format: Format::B8G8R8A8_SRGB,
                    samples: 1,
                    load_op: Clear,
                    store_op: Store,
                },
                depth_stencil: {
                    format: Format::D32_SFLOAT,
                    samples: 1,
                    load_op: Clear,
                    store_op: DontCare,
                },
            },
            pass: {
                color: [color],
                depth_stencil: {depth_stencil},
            },
        )?;

        let vs = preview_vs::load(device.clone())?
            .entry_point("main")
            .ok_or("Missing vertex shader entry point")?;
        let fs = preview_fs::load(device.clone())?
            .entry_point("main")
            .ok_or("Missing fragment shader entry point")?;

        let vertex_input_state = Vertex3D::per_vertex().definition(&vs)?;

        let stages = [
            PipelineShaderStageCreateInfo::new(vs),
            PipelineShaderStageCreateInfo::new(fs),
        ];

        let layout = PipelineLayout::new(
            device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
                .into_pipeline_layout_create_info(device.clone())?,
        )?;

        let pipeline = GraphicsPipeline::new(
            device,
            None,
            GraphicsPipelineCreateInfo {
                stages: stages.into_iter().collect(),
                vertex_input_state: Some(vertex_input_state),
                input_assembly_state: Some(InputAssemblyState::default()),
                viewport_state: Some(VkViewportState::default()),
                rasterization_state: Some(RasterizationState::default()),
                multisample_state: Some(MultisampleState::default()),
                depth_stencil_state: Some(DepthStencilState {
                    depth: Some(DepthState::simple()),
                    ..Default::default()
                }),
                color_blend_state: Some(ColorBlendState::with_attachment_states(
                    1,
                    ColorBlendAttachmentState::default(),
                )),
                dynamic_state: [DynamicState::Viewport].into_iter().collect(),
                subpass: Some(render_pass.clone().first_subpass().into()),
                ..GraphicsPipelineCreateInfo::layout(layout)
            },
        )?;

        // Identity bone palette for static mesh preview
        let identity_palette_set = SkinningBackend::create_identity_set_for_layout(
            &memory_allocator,
            &descriptor_set_allocator,
            pipeline.layout().set_layouts()[0].clone(),
        )
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?;

        Ok(Self {
            queue,
            memory_allocator,
            command_buffer_allocator,
            render_pass,
            pipeline,
            identity_palette_set,
        })
    }

    pub fn render_pass(&self) -> &Arc<RenderPass> {
        &self.render_pass
    }

    pub fn memory_allocator(&self) -> &Arc<StandardMemoryAllocator> {
        &self.memory_allocator
    }

    /// Render mesh submeshes into the given framebuffer.
    /// Returns a command buffer ready for submission (not executed).
    pub fn render(
        &self,
        framebuffer: &Arc<Framebuffer>,
        width: u32,
        height: u32,
        gpu_meshes: &[GpuMeshBuffers],
        view_projection: Mat4,
    ) -> Result<Arc<PrimaryAutoCommandBuffer>, Box<dyn std::error::Error>> {
        let mut builder = AutoCommandBufferBuilder::primary(
            self.command_buffer_allocator.clone(),
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;

        builder.begin_render_pass(
            RenderPassBeginInfo {
                clear_values: vec![
                    Some([0.16, 0.16, 0.18, 1.0].into()), // dark gray background
                    Some(1.0f32.into()),                    // depth
                ],
                ..RenderPassBeginInfo::framebuffer(framebuffer.clone())
            },
            SubpassBeginInfo {
                contents: SubpassContents::Inline,
                ..Default::default()
            },
        )?;

        builder.set_viewport(
            0,
            [Viewport {
                offset: [0.0, 0.0],
                extent: [width as f32, height as f32],
                depth_range: 0.0..=1.0,
            }]
            .into_iter()
            .collect(),
        )?;

        builder.bind_pipeline_graphics(self.pipeline.clone())?;

        // Bind identity bone palette (set 0) — preview renders static meshes
        builder.bind_descriptor_sets(
            PipelineBindPoint::Graphics,
            self.pipeline.layout().clone(),
            0,
            self.identity_palette_set.clone(),
        )?;

        let model_matrix = Mat4::IDENTITY;
        let push_data = preview_vs::PushConstants {
            model: model_matrix.to_cols_array_2d(),
            view_projection: view_projection.to_cols_array_2d(),
        };
        builder.push_constants(self.pipeline.layout().clone(), 0, push_data)?;

        for (vb, ib, index_count) in gpu_meshes {
            builder.bind_vertex_buffers(0, vb.clone())?;
            builder.bind_index_buffer(ib.clone())?;
            // SAFETY: vertex/index buffers match the pipeline's vertex input state.
            unsafe { builder.draw_indexed(*index_count, 1, 0, 0, 0)? };
        }

        builder.end_render_pass(SubpassEndInfo::default())?;

        let cb = builder.build()?;
        Ok(cb)
    }
}

// ---------------------------------------------------------------------------
// MeshPreviewState — per-editor preview (texture, framebuffer, orbit camera)
// ---------------------------------------------------------------------------

pub struct MeshPreviewState {
    pub texture: ViewportTexture,
    pub texture_id: Option<egui::TextureId>,
    pub depth_image: Arc<Image>,
    pub depth_view: Arc<ImageView>,
    pub framebuffer: Arc<Framebuffer>,
    pub size: (u32, u32),
    // Orbit camera
    pub orbit_yaw: f32,
    pub orbit_pitch: f32,
    pub orbit_distance: f32,
    pub orbit_target: Vec3,
    // Mesh references
    pub mesh_indices: Vec<usize>,
    pub mesh_center: Vec3,
    pub mesh_radius: f32,
}

impl MeshPreviewState {
    pub fn new(
        renderer: &MeshPreviewRenderer,
        mesh_manager: &MeshManager,
        mesh_path: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let indices = mesh_manager
            .indices_for_path(mesh_path)
            .map(|s| s.to_vec())
            .unwrap_or_default();

        // Compute combined bounding sphere
        let (center, radius) = if indices.is_empty() {
            (Vec3::ZERO, 1.0)
        } else {
            let mut centers = Vec::new();
            let mut radii = Vec::new();
            for &idx in &indices {
                if let Some(gm) = mesh_manager.get(idx) {
                    centers.push(gm.center);
                    radii.push(gm.radius);
                }
            }
            if centers.is_empty() {
                (Vec3::ZERO, 1.0)
            } else if centers.len() == 1 {
                (centers[0], radii[0])
            } else {
                let avg: Vec3 = centers.iter().copied().sum::<Vec3>() / centers.len() as f32;
                let r = centers
                    .iter()
                    .zip(radii.iter())
                    .map(|(c, r)| (*c - avg).length() + r)
                    .fold(0.0f32, f32::max);
                (avg, r)
            }
        };

        let radius = if radius < 0.001 { 1.0 } else { radius };

        // Orbit camera defaults
        let fov = 45.0_f32.to_radians();
        let distance = radius / (fov / 2.0).sin() * 1.3;

        // Use a small placeholder; the real size comes from the GUI panel on
        // the next frame and triggers a resize + first render at full resolution.
        let init_w: u32 = 4;
        let init_h: u32 = 4;

        let device = renderer.memory_allocator.device().clone();
        let texture =
            ViewportTexture::new(device.clone(), renderer.memory_allocator.clone(), init_w, init_h)?;

        let (depth_image, depth_view) =
            Self::create_depth(renderer.memory_allocator(), init_w, init_h)?;

        let framebuffer = Self::create_framebuffer(
            renderer.render_pass(),
            &texture.image_view(),
            &depth_view,
        )?;

        Ok(Self {
            texture,
            texture_id: None,
            depth_image,
            depth_view,
            framebuffer,
            // Start at 0x0 so the first render waits for GUI to set the real size.
            size: (0, 0),
            orbit_yaw: 45.0_f32.to_radians(),
            orbit_pitch: 30.0_f32.to_radians(),
            orbit_distance: distance,
            orbit_target: center,
            mesh_indices: indices,
            mesh_center: center,
            mesh_radius: radius,
        })
    }

    pub fn compute_view_projection(&self, aspect: f32) -> Mat4 {
        let fov = 45.0_f32.to_radians();
        let x = self.orbit_distance * self.orbit_pitch.cos() * self.orbit_yaw.sin();
        let y = self.orbit_distance * self.orbit_pitch.sin();
        let z = self.orbit_distance * self.orbit_pitch.cos() * self.orbit_yaw.cos();
        let camera_pos = self.orbit_target + Vec3::new(x, y, z);
        let near = (self.orbit_distance * 0.01).max(0.01);
        let far = self.orbit_distance * 10.0;
        let view = Mat4::look_at_rh(camera_pos, self.orbit_target, Vec3::Y);
        let mut proj = Mat4::perspective_rh(fov, aspect, near, far);
        // Vulkan NDC Y-flip (clip-space Y is inverted vs OpenGL)
        proj.y_axis.y *= -1.0;
        proj * view
    }

    pub fn resize(
        &mut self,
        renderer: &MeshPreviewRenderer,
        new_w: u32,
        new_h: u32,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        // Compare against actual GPU texture dimensions, NOT self.size.
        // self.size is set by the GUI each frame (show_preview_panel), so it
        // may already reflect the desired size before resize() is called.
        if new_w == self.texture.width() && new_h == self.texture.height() {
            return Ok(false);
        }
        if new_w == 0 || new_h == 0 {
            return Ok(false);
        }

        self.texture.resize(new_w, new_h)?;
        let (depth_image, depth_view) =
            Self::create_depth(renderer.memory_allocator(), new_w, new_h)?;
        self.depth_image = depth_image;
        self.depth_view = depth_view.clone();
        self.framebuffer = Self::create_framebuffer(
            renderer.render_pass(),
            &self.texture.image_view(),
            &depth_view,
        )?;
        self.size = (new_w, new_h);
        Ok(true)
    }

    fn create_depth(
        allocator: &Arc<StandardMemoryAllocator>,
        w: u32,
        h: u32,
    ) -> Result<(Arc<Image>, Arc<ImageView>), Box<dyn std::error::Error>> {
        let image = Image::new(
            allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::D32_SFLOAT,
                extent: [w, h, 1],
                usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT,
                ..Default::default()
            },
            AllocationCreateInfo::default(),
        )?;
        let view = ImageView::new_default(image.clone())?;
        Ok((image, view))
    }

    fn create_framebuffer(
        render_pass: &Arc<RenderPass>,
        color_view: &Arc<ImageView>,
        depth_view: &Arc<ImageView>,
    ) -> Result<Arc<Framebuffer>, Box<dyn std::error::Error>> {
        Ok(Framebuffer::new(
            render_pass.clone(),
            FramebufferCreateInfo {
                attachments: vec![color_view.clone(), depth_view.clone()],
                ..Default::default()
            },
        )?)
    }
}

// ---------------------------------------------------------------------------
// MeshEditorData / MeshEditorPanel
// ---------------------------------------------------------------------------

/// Per-mesh editor state, loaded from the `.mesh.ron` sidecar.
pub struct MeshEditorData {
    /// Content-relative path (e.g. "Defeated.mesh").
    pub mesh_path: String,
    /// Sidecar metadata (editable material_slots live here).
    pub meta: MeshImportMeta,
    /// Unsaved changes flag.
    pub dirty: bool,
    /// 3D preview state (lazily initialized).
    pub preview: Option<MeshPreviewState>,
    /// Whether the editor window is open (false = user closed it).
    pub open: bool,
    /// Whether the orbit camera changed and the preview needs re-rendering.
    pub preview_dirty: bool,
}

pub struct MeshEditorPanel;

impl MeshEditorPanel {
    /// Render the mesh editor tab with toolbar, 3D preview (left), and details (right).
    pub fn show(ui: &mut Ui, data: &mut MeshEditorData, asset_browser: &mut AssetBrowserPanel) {
        let editor_id = ui.id().with(&data.mesh_path);

        // Top toolbar
        egui::TopBottomPanel::top(editor_id.with("toolbar"))
            .exact_height(28.0)
            .show_inside(ui, |ui| {
                Self::show_toolbar(ui, data);
            });

        // Right side-panel: details
        egui::SidePanel::right(editor_id.with("details"))
            .default_width(300.0)
            .min_width(200.0)
            .resizable(true)
            .show_inside(ui, |ui| {
                Self::show_details_panel(ui, data, asset_browser);
            });

        // Remaining space: 3D preview
        Self::show_preview_panel(ui, data);
    }

    /// Details panel (right side): header, info, material slots, save.
    fn show_details_panel(
        ui: &mut Ui,
        data: &mut MeshEditorData,
        asset_browser: &mut AssetBrowserPanel,
    ) {
        let filename = std::path::Path::new(&data.mesh_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| data.mesh_path.clone());

        ui.horizontal(|ui| {
            ui.heading(RichText::new(&filename).strong());
            if data.dirty {
                ui.label(RichText::new("*").strong().color(Color32::from_rgb(220, 180, 50)));
            }
        });
        ui.label(RichText::new(&data.mesh_path).weak().small());
        ui.separator();

        ui.label(format!(
            "Material Slots: {}",
            data.meta.material_slots.len()
        ));
        if !data.meta.source.is_empty() {
            ui.label(
                RichText::new(format!("Source: {}", data.meta.source))
                    .weak()
                    .small(),
            );
        }
        ui.separator();

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if data.meta.material_slots.is_empty() {
                    ui.label(
                        RichText::new("No material slots. Re-import with materials enabled.")
                            .weak()
                            .italics(),
                    );
                } else {
                    let mut changed = false;
                    for (i, slot) in data.meta.material_slots.iter_mut().enumerate() {
                        changed |= Self::render_material_slot(ui, i, slot, asset_browser);
                    }
                    if changed {
                        data.dirty = true;
                    }
                }

                ui.add_space(12.0);

                let save_btn = ui.add_enabled(
                    data.dirty,
                    egui::Button::new("Save").min_size(egui::vec2(80.0, 28.0)),
                );
                if save_btn.clicked() {
                    match Self::save_sidecar(data) {
                        Ok(()) => data.dirty = false,
                        Err(e) => log::error!("Failed to save mesh sidecar: {}", e),
                    }
                }
            });
    }

    /// Toolbar at the top of the mesh editor.
    fn show_toolbar(ui: &mut Ui, data: &mut MeshEditorData) {
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new("Mesh Editor").strong());
            ui.separator();
            let filename = std::path::Path::new(&data.mesh_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            ui.label(&filename);
            if data.dirty {
                ui.label(
                    RichText::new("*")
                        .strong()
                        .color(Color32::from_rgb(220, 180, 50)),
                );
            }
            // Right side reserved for future toolbar actions
        });
    }

    /// 3D preview panel (left side, fills remaining space).
    fn show_preview_panel(ui: &mut Ui, data: &mut MeshEditorData) {
        let available = ui.available_size();

        if let Some(ref mut preview) = data.preview {
            // Track desired size for resize
            let new_w = (available.x.max(1.0)) as u32;
            let new_h = (available.y.max(1.0)) as u32;
            preview.size = (new_w, new_h);

            if preview.mesh_indices.is_empty() {
                // Mesh not loaded on GPU — can't render preview
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(available.y / 2.0 - 20.0);
                        ui.label(RichText::new("3D Preview").weak());
                        ui.label(
                            RichText::new("Mesh not loaded on GPU")
                                .weak()
                                .small(),
                        );
                    });
                });
            } else if let Some(tex_id) = preview.texture_id {
                let response = ui.add(
                    egui::Image::new(egui::load::SizedTexture::new(
                        tex_id,
                        egui::vec2(available.x, available.y),
                    ))
                    .sense(egui::Sense::click_and_drag()),
                );

                // Orbit camera: left-drag rotates
                if response.dragged_by(egui::PointerButton::Primary) {
                    let delta = response.drag_delta();
                    let sensitivity = 0.005;
                    preview.orbit_yaw += delta.x * sensitivity;
                    preview.orbit_pitch -= delta.y * sensitivity;
                    preview.orbit_pitch = preview
                        .orbit_pitch
                        .clamp(-89.0_f32.to_radians(), 89.0_f32.to_radians());
                    data.preview_dirty = true;
                }

                // Middle-drag pans
                if response.dragged_by(egui::PointerButton::Middle) {
                    let delta = response.drag_delta();
                    let pan_speed = preview.orbit_distance * 0.002;

                    let forward = (preview.orbit_target - Self::camera_position(preview)).normalize();
                    let right = forward.cross(Vec3::Y).normalize();
                    let up = right.cross(forward).normalize();

                    preview.orbit_target -= right * delta.x * pan_speed;
                    preview.orbit_target += up * delta.y * pan_speed;
                    data.preview_dirty = true;
                }

                // Scroll zooms
                if response.contains_pointer() {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                    if scroll.abs() > 0.0 {
                        let zoom_factor = 1.0 - scroll * 0.002;
                        preview.orbit_distance *= zoom_factor;
                        let min_dist = preview.mesh_radius * 0.1;
                        let max_dist = preview.mesh_radius * 50.0;
                        preview.orbit_distance = preview.orbit_distance.clamp(min_dist, max_dist);
                        data.preview_dirty = true;
                    }
                }
            } else {
                // Waiting for first render to complete
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("Rendering preview...").weak());
                });
            }
        } else {
            // Preview not yet initialized
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Loading preview...").weak());
            });
        }
    }

    fn camera_position(preview: &MeshPreviewState) -> Vec3 {
        let x = preview.orbit_distance * preview.orbit_pitch.cos() * preview.orbit_yaw.sin();
        let y = preview.orbit_distance * preview.orbit_pitch.sin();
        let z = preview.orbit_distance * preview.orbit_pitch.cos() * preview.orbit_yaw.cos();
        preview.orbit_target + Vec3::new(x, y, z)
    }

    /// Render a single material slot with DnD support.
    fn render_material_slot(
        ui: &mut Ui,
        index: usize,
        slot: &mut MaterialSlot,
        asset_browser: &mut AssetBrowserPanel,
    ) -> bool {
        let mut changed = false;

        CollapsingHeader::new(
            RichText::new(format!("[{}] {}", index, slot.name)).strong(),
        )
        .default_open(true)
        .id_salt(format!("mat_slot_{}", index))
        .show(ui, |ui| {
            let thumb_size = 64.0;
            let slot_height = thumb_size + 8.0;
            let available_width = ui.available_width();

            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(available_width, slot_height),
                egui::Sense::hover(),
            );

            // DnD hover
            let mut is_valid_hover = false;
            if let Some(hovered_id) = response.dnd_hover_payload::<AssetId>() {
                if let Some(meta) = asset_browser.registry.get(*hovered_id) {
                    if meta.asset_type == AssetType::Material {
                        is_valid_hover = true;
                    }
                }
            }

            // DnD drop
            if let Some(dropped_id) = response.dnd_release_payload::<AssetId>() {
                if let Some(meta) = asset_browser.registry.get(*dropped_id) {
                    if meta.asset_type == AssetType::Material {
                        slot.material_path = meta.path.to_string_lossy().to_string();
                        changed = true;
                    }
                }
            }

            let painter = ui.painter_at(rect);

            // Background
            let bg_color = if is_valid_hover {
                Color32::from_rgba_premultiplied(40, 60, 90, 255)
            } else {
                Color32::from_gray(35)
            };
            painter.rect_filled(rect, 4.0, bg_color);

            // Border
            let border_color = if is_valid_hover {
                Color32::from_rgb(100, 180, 255)
            } else {
                Color32::from_gray(60)
            };
            painter.rect_stroke(
                rect,
                4.0,
                Stroke::new(1.0, border_color),
                egui::epaint::StrokeKind::Inside,
            );

            if slot.material_path.is_empty() {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Drop .material.ron here",
                    egui::FontId::proportional(12.0),
                    Color32::from_gray(120),
                );
            } else {
                // Thumbnail
                let thumb_rect = egui::Rect::from_min_size(
                    rect.min + egui::vec2(4.0, 4.0),
                    egui::vec2(thumb_size, thumb_size),
                );
                let asset_id = AssetId::from_path(&slot.material_path);
                if let Some(meta) = asset_browser.registry.get(asset_id) {
                    if let Some(tex_id) =
                        asset_browser.thumbnails.get_texture_id(ui.ctx(), meta)
                    {
                        painter.image(
                            tex_id,
                            thumb_rect,
                            egui::Rect::from_min_max(
                                egui::Pos2::ZERO,
                                egui::pos2(1.0, 1.0),
                            ),
                            Color32::WHITE,
                        );
                    } else {
                        painter.rect_filled(thumb_rect, 2.0, Color32::from_gray(50));
                    }
                } else {
                    painter.rect_filled(thumb_rect, 2.0, Color32::from_gray(50));
                }

                // Filename
                let filename = std::path::Path::new(&slot.material_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| slot.material_path.clone());
                let text_left = thumb_rect.right() + 8.0;
                painter.text(
                    egui::pos2(text_left, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &filename,
                    egui::FontId::proportional(12.0),
                    Color32::from_gray(200),
                );

                // Clear button
                let clear_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.max.x - 22.0, rect.center().y - 8.0),
                    egui::vec2(16.0, 16.0),
                );
                let clear_response = ui.interact(
                    clear_rect,
                    ui.id().with(format!("mat_slot_{}_clear", index)),
                    egui::Sense::click(),
                );
                let clear_color = if clear_response.hovered() {
                    Color32::from_rgb(220, 80, 80)
                } else {
                    Color32::from_gray(120)
                };
                painter.text(
                    clear_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "x",
                    egui::FontId::proportional(12.0),
                    clear_color,
                );
                if clear_response.clicked() {
                    slot.material_path = String::new();
                    changed = true;
                }
            }
        });

        changed
    }

    /// Write updated sidecar back to disk.
    fn save_sidecar(data: &MeshEditorData) -> Result<(), Box<dyn std::error::Error>> {
        use crate::engine::assets::mesh_import::write_mesh_sidecar;
        let mesh_path = std::path::Path::new("content").join(&data.mesh_path);
        write_mesh_sidecar(&mesh_path, &data.meta)?;
        log::info!("Saved mesh sidecar: {}", data.mesh_path);
        Ok(())
    }
}
