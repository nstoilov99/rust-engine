//! Viewport settings with persistence

use serde::{Deserialize, Serialize};

/// Tool mode for viewport interaction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ToolMode {
    /// Selection tool - click to select objects
    #[default]
    Select,
    /// Translation gizmo
    Translate,
    /// Rotation gizmo
    Rotate,
    /// Scale gizmo
    Scale,
}

/// Gizmo manipulation mode (alias for backward compatibility)
pub type GizmoMode = ToolMode;

/// Gizmo orientation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GizmoOrientation {
    /// Transform relative to object's local axes
    Local,
    /// Transform relative to world axes
    #[default]
    World,
}

/// Predefined snap values for grid/translation (Unreal Engine style - Snap Sizes)
pub const GRID_SNAP_VALUES: &[f32] = &[1.0, 5.0, 10.0, 50.0, 100.0, 500.0, 1000.0, 5000.0, 10000.0];

/// Predefined snap values for rotation - increments (degrees)
pub const ROTATION_SNAP_VALUES: &[f32] = &[5.0, 10.0, 15.0, 30.0, 45.0, 60.0, 90.0, 120.0];

/// Predefined snap values for rotation - divisions of 360 (degrees)
pub const ROTATION_DIVISIONS_360: &[f32] = &[2.8125, 5.625, 11.25, 22.5];

/// Predefined snap values for scale (Unreal Engine style - Snap Sizes)
pub const SCALE_SNAP_VALUES: &[f32] = &[0.03125, 0.0625, 0.1, 0.125, 0.25, 0.5, 1.0, 10.0];

/// Predefined camera speed multipliers
pub const CAMERA_SPEED_VALUES: &[f32] = &[0.1, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0];

/// Persisted viewport settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewportSettings {
    /// Current tool mode
    pub tool_mode: ToolMode,
    /// Current gizmo orientation
    pub gizmo_orientation: GizmoOrientation,
    /// Grid visibility
    pub grid_visible: bool,

    /// Grid/translation snapping enabled
    pub grid_snap_enabled: bool,
    /// Rotation snapping enabled
    pub rotation_snap_enabled: bool,
    /// Scale snapping enabled
    pub scale_snap_enabled: bool,

    /// Translation snap increment (units)
    pub snap_translate: f32,
    /// Rotation snap increment (degrees)
    pub snap_rotate: f32,
    /// Scale snap increment
    pub snap_scale: f32,

    /// Camera fly speed (0.0 to 8.0, controlled by slider)
    pub camera_speed: f32,
    /// Camera speed scalar multiplier (1.0 default)
    pub camera_speed_scalar: f32,
    /// Mouse sensitivity for camera rotation
    pub mouse_sensitivity: f32,
}

impl Default for ViewportSettings {
    fn default() -> Self {
        Self {
            tool_mode: ToolMode::Select,
            gizmo_orientation: GizmoOrientation::World,
            grid_visible: true,
            grid_snap_enabled: false,
            rotation_snap_enabled: false,
            scale_snap_enabled: false,
            snap_translate: 1.0,
            snap_rotate: 15.0,
            snap_scale: 0.1,
            camera_speed: 1.0,
            camera_speed_scalar: 1.0,
            mouse_sensitivity: 0.003,
        }
    }
}

impl ViewportSettings {
    /// Get the effective camera speed (speed * scalar)
    pub fn effective_camera_speed(&self) -> f32 {
        self.camera_speed * self.camera_speed_scalar
    }
}

impl ViewportSettings {
    /// Check if any snapping is enabled
    pub fn any_snap_enabled(&self) -> bool {
        self.grid_snap_enabled || self.rotation_snap_enabled || self.scale_snap_enabled
    }

    /// Get the appropriate snap value for the current tool mode
    pub fn current_snap_value(&self) -> f32 {
        match self.tool_mode {
            ToolMode::Select | ToolMode::Translate => self.snap_translate,
            ToolMode::Rotate => self.snap_rotate,
            ToolMode::Scale => self.snap_scale,
        }
    }

    /// Check if snapping is enabled for the current tool mode
    pub fn current_snap_enabled(&self) -> bool {
        match self.tool_mode {
            ToolMode::Select | ToolMode::Translate => self.grid_snap_enabled,
            ToolMode::Rotate => self.rotation_snap_enabled,
            ToolMode::Scale => self.scale_snap_enabled,
        }
    }
}
