//! Viewport systems for editor
//!
//! This module contains:
//! - EditorCamera: Unreal-style camera controls (fly, orbit, pan)
//! - GizmoHandler: Transform gizmo integration
//! - Toolbar: Viewport toolbar UI
//! - Settings: Persisted viewport settings

mod camera_controller;
mod gizmo_handler;
mod settings;
mod toolbar;

pub use camera_controller::{CameraControlMode, EditorCamera};
pub use gizmo_handler::{GizmoHandler, GizmoInteractionResult};
pub use settings::{
    GizmoMode, GizmoOrientation, ToolMode, ViewportSettings, CAMERA_SPEED_VALUES, GRID_SNAP_VALUES,
    ROTATION_SNAP_VALUES, SCALE_SNAP_VALUES,
};
pub use toolbar::{render_orientation_indicator, render_viewport_toolbar_overlay};
