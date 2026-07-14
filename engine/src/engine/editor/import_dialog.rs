//! Model import dialog state — settings/preview/action model.
//!
//! Shown when model files (FBX, OBJ, glTF) are dropped onto the editor.
//! Follows the same `Option<DialogState>` pattern as `DeleteConfirmation`.
//! The old rendering fn was removed; the crusty analog lives in
//! `dialogs_crusty::import_dialog_panel`.

use crate::engine::assets::mesh_import::MeshImportSettings;
use std::path::PathBuf;

/// Preview information extracted from the source model.
#[derive(Debug, Clone)]
pub struct ImportPreview {
    pub mesh_count: usize,
    pub total_vertices: u32,
    pub total_indices: u32,
    pub material_count: usize,
    pub bone_count: usize,
    pub animation_count: usize,
}

/// State for the model import dialog.
#[derive(Debug, Clone)]
pub struct ImportDialogState {
    /// Source files being imported.
    pub source_files: Vec<PathBuf>,
    /// Index of the file currently being configured.
    pub current_file_index: usize,
    /// Import settings (shared for all files in batch, user can adjust).
    pub settings: MeshImportSettings,
    /// Content-relative target folder for output .mesh files.
    pub target_folder: PathBuf,
    /// Preview info (populated lazily on first render).
    pub preview: Option<ImportPreview>,
    /// Whether preview loading has been attempted.
    pub preview_attempted: bool,
}

impl ImportDialogState {
    /// Create a new import dialog for the given source files.
    pub fn new(source_files: Vec<PathBuf>, target_folder: PathBuf) -> Self {
        Self {
            source_files,
            current_file_index: 0,
            settings: MeshImportSettings::default(),
            target_folder,
            preview: None,
            preview_attempted: false,
        }
    }

    /// Get the current source file being configured.
    pub fn current_file(&self) -> Option<&PathBuf> {
        self.source_files.get(self.current_file_index)
    }

    /// Get the display name of the current file.
    pub fn current_file_name(&self) -> String {
        self.current_file()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown".to_string())
    }

    /// Get the format name based on extension.
    pub fn format_name(&self) -> &'static str {
        self.current_file()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(|ext| match ext.to_ascii_lowercase().as_str() {
                "gltf" => "glTF Text",
                "glb" => "glTF Binary",
                "obj" => "Wavefront OBJ",
                "fbx" => "Autodesk FBX",
                _ => "Unknown",
            })
            .unwrap_or("Unknown")
    }
}

/// Result from rendering the import dialog.
pub enum ImportDialogAction {
    /// No action taken (dialog still open).
    None,
    /// User cancelled the import.
    Cancel,
    /// User confirmed import for the current file.
    Import,
}
