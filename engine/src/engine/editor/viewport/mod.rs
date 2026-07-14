//! Viewport systems for editor
//!
//! This module contains:
//! - EditorCamera: Unreal-style camera controls (fly, orbit, pan)
//! - GizmoHandler: Transform gizmo integration
//! - Settings: Persisted viewport settings
//!
//! The egui toolbar (`toolbar.rs`) was removed; `viewport_crusty` has its own.

mod camera_controller;
mod gizmo_handler;
mod settings;

pub use camera_controller::{CameraControlMode, EditorCamera};
pub use gizmo_handler::{GizmoHandler, GizmoInteractionResult};
pub use settings::{
    GizmoMode, GizmoOrientation, ToolMode, ViewportSettings, CAMERA_SPEED_VALUES, GRID_SNAP_VALUES,
    ROTATION_SNAP_VALUES, SCALE_SNAP_VALUES,
};
