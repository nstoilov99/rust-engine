//! Shader modules for deferred rendering

/// G-Buffer geometry pass vertex shader
pub mod gbuffer_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/deferred/gbuffer.vert",
    }
}

/// G-Buffer geometry pass fragment shader
pub mod gbuffer_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/deferred/gbuffer.frag",
    }
}

/// Lighting pass vertex shader (fullscreen triangle)
pub mod lighting_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/deferred/lightning_pass.vert",
    }
}

/// Lighting pass fragment shader (PBR lighting)
pub mod lighting_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/deferred/lightning_pass.frag",
    }
}
