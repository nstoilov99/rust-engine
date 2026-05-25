//! Per-asset-type editor windows.
//!
//! Each sub-module defines an `*EditorState` struct (per-window state that
//! outlives the window) and a render function that draws the editor content
//! inside an egui `Ui`.

pub mod material;
pub mod material_instance;
pub mod texture;
pub mod audio;
pub mod animation_clip;
pub mod animation_graph;
pub mod material_graph;
pub mod prefab;
