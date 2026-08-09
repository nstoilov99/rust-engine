//! Typed error for the rendering module boundary.
//!
//! Replaces `Box<dyn Error>` on the paths owned by the deferred renderer and
//! render graph: pass resize/rebind, graph execution, and frame recording.
//! Deliberately pragmatic — a few variants carrying context strings, not an
//! exhaustive taxonomy. The render thread forwards these to the main thread
//! as `RenderEvent::RenderError` via their `Display` output.

use std::fmt;

#[derive(Debug)]
pub enum RenderError {
    /// A pass failed to recreate its size-dependent GPU targets.
    PassResize { pass: &'static str, message: String },
    /// A pass failed to rebind descriptor sets / framebuffers.
    PassRebind { pass: &'static str, message: String },
    /// Render-graph compile or execution failure (message carries the pass
    /// and resource context).
    Graph(String),
    /// Anything else on the render path: command recording, GPU object
    /// creation, missing prepared state.
    Render(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PassResize { pass, message } => {
                write!(f, "{pass} pass resize failed: {message}")
            }
            Self::PassRebind { pass, message } => {
                write!(f, "{pass} pass rebind failed: {message}")
            }
            Self::Graph(message) | Self::Render(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<String> for RenderError {
    fn from(message: String) -> Self {
        Self::Render(message)
    }
}

impl From<&str> for RenderError {
    fn from(message: &str) -> Self {
        Self::Render(message.to_string())
    }
}

/// Lets pass internals that still return `Box<dyn Error>` propagate with `?`.
impl From<Box<dyn std::error::Error>> for RenderError {
    fn from(err: Box<dyn std::error::Error>) -> Self {
        Self::Render(err.to_string())
    }
}

impl From<vulkano::Validated<vulkano::VulkanError>> for RenderError {
    fn from(err: vulkano::Validated<vulkano::VulkanError>) -> Self {
        Self::Render(err.to_string())
    }
}

impl From<crate::engine::rendering::graph::GraphError> for RenderError {
    fn from(err: crate::engine::rendering::graph::GraphError) -> Self {
        Self::Graph(err.to_string())
    }
}
