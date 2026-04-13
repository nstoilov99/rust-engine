//! Asset type classification
//!
//! Defines the different categories of assets supported by the engine.
//! Used for filtering, icon display, and import pipeline routing.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Classification of asset types supported by the engine
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssetType {
    /// Image textures (PNG, JPG, JPEG, TGA, BMP)
    Texture,
    /// 3D models — source formats (GLTF, GLB, OBJ, FBX)
    Model,
    /// Native preprocessed mesh (.mesh)
    Mesh,
    /// Animation clips (.anim)
    Animation,
    /// Scene files (*.scene.ron)
    Scene,
    /// Material definitions (*.material.ron)
    Material,
    /// Audio files (WAV, OGG, MP3)
    Audio,
    /// Shader files (GLSL, VERT, FRAG, COMP)
    Shader,
    /// Prefab entity templates (*.prefab.ron)
    Prefab,
    /// Input action definitions (*.inputaction.ron)
    InputAction,
    /// Input mapping context definitions (*.mappingcontext.ron)
    InputMappingContext,
    /// Unknown or unsupported file type
    #[default]
    Unknown,
}

impl AssetType {
    /// Determine asset type from file extension
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            // Textures
            "png" | "jpg" | "jpeg" | "tga" | "bmp" | "dds" => AssetType::Texture,
            // 3D Models (source)
            "gltf" | "glb" | "obj" | "fbx" => AssetType::Model,
            // Native mesh (preprocessed)
            "mesh" => AssetType::Mesh,
            // Animation clips
            "anim" => AssetType::Animation,
            // Audio
            "wav" | "ogg" | "mp3" | "flac" => AssetType::Audio,
            // Shaders
            "glsl" | "vert" | "frag" | "comp" | "spv" => AssetType::Shader,
            // RON files - need content inspection for specific type
            "ron" => AssetType::Unknown, // Will be refined by filename pattern
            _ => AssetType::Unknown,
        }
    }

    /// Determine asset type from full path, including filename patterns
    pub fn from_path(path: &Path) -> Self {
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Check for RON file patterns first
        if filename.ends_with(".scene.ron") {
            return AssetType::Scene;
        }
        if filename.ends_with(".material.ron") {
            return AssetType::Material;
        }
        if filename.ends_with(".prefab.ron") {
            return AssetType::Prefab;
        }
        if filename.ends_with(".inputaction.ron") {
            return AssetType::InputAction;
        }
        if filename.ends_with(".mappingcontext.ron") {
            return AssetType::InputMappingContext;
        }
        // Hide .mesh.ron sidecars from the browser (they're metadata, not assets)
        if filename.ends_with(".mesh.ron") {
            return AssetType::Unknown;
        }

        // Fall back to extension-based detection
        path.extension()
            .and_then(|e| e.to_str())
            .map(Self::from_extension)
            .unwrap_or(AssetType::Unknown)
    }

    /// Get a display name for this asset type
    pub fn display_name(&self) -> &'static str {
        match self {
            AssetType::Texture => "Texture",
            AssetType::Model => "Model",
            AssetType::Mesh => "Mesh",
            AssetType::Animation => "Animation",
            AssetType::Scene => "Scene",
            AssetType::Material => "Material",
            AssetType::Audio => "Audio",
            AssetType::Shader => "Shader",
            AssetType::Prefab => "Prefab",
            AssetType::InputAction => "Input Action",
            AssetType::InputMappingContext => "Input Mapping Context",
            AssetType::Unknown => "Unknown",
        }
    }

    /// Get file extensions associated with this asset type
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            AssetType::Texture => &["png", "jpg", "jpeg", "tga", "bmp", "dds"],
            AssetType::Model => &["gltf", "glb", "obj", "fbx"],
            AssetType::Mesh => &["mesh"],
            AssetType::Animation => &["anim"],
            AssetType::Scene => &["scene.ron"],
            AssetType::Material => &["material.ron"],
            AssetType::Audio => &["wav", "ogg", "mp3", "flac"],
            AssetType::Shader => &["glsl", "vert", "frag", "comp", "spv"],
            AssetType::Prefab => &["prefab.ron"],
            AssetType::InputAction => &["inputaction.ron"],
            AssetType::InputMappingContext => &["mappingcontext.ron"],
            AssetType::Unknown => &[],
        }
    }

    /// Get all defined asset types (excluding Unknown)
    pub fn all() -> &'static [AssetType] {
        &[
            AssetType::Texture,
            AssetType::Model,
            AssetType::Mesh,
            AssetType::Animation,
            AssetType::Scene,
            AssetType::Material,
            AssetType::Audio,
            AssetType::Shader,
            AssetType::Prefab,
            AssetType::InputAction,
            AssetType::InputMappingContext,
        ]
    }

    /// Check if this asset type can be dragged into the viewport
    pub fn is_viewport_droppable(&self) -> bool {
        matches!(self, AssetType::Model | AssetType::Mesh | AssetType::Prefab)
    }

    /// Check if this asset type has a thumbnail preview
    pub fn has_thumbnail(&self) -> bool {
        matches!(
            self,
            AssetType::Texture
                | AssetType::Model
                | AssetType::Mesh
                | AssetType::Animation
                | AssetType::Material
        )
    }
}

impl std::fmt::Display for AssetType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_extension() {
        assert_eq!(AssetType::from_extension("png"), AssetType::Texture);
        assert_eq!(AssetType::from_extension("PNG"), AssetType::Texture);
        assert_eq!(AssetType::from_extension("gltf"), AssetType::Model);
        assert_eq!(AssetType::from_extension("glb"), AssetType::Model);
        assert_eq!(AssetType::from_extension("mesh"), AssetType::Mesh);
        assert_eq!(AssetType::from_extension("wav"), AssetType::Audio);
        assert_eq!(AssetType::from_extension("xyz"), AssetType::Unknown);
    }

    #[test]
    fn test_from_path() {
        assert_eq!(
            AssetType::from_path(Path::new("assets/textures/diffuse.png")),
            AssetType::Texture
        );
        assert_eq!(
            AssetType::from_path(Path::new("assets/scenes/main.scene.ron")),
            AssetType::Scene
        );
        assert_eq!(
            AssetType::from_path(Path::new("assets/materials/metal.material.ron")),
            AssetType::Material
        );
    }
}
