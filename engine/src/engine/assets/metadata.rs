//! Asset metadata storage
//!
//! Provides persistent metadata about assets including type classification,
//! tags, file information, and dependency tracking.

use super::asset_type::AssetType;
use super::handle::AssetId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

/// Metadata about a single asset
///
/// This struct contains all the information needed to display, filter,
/// and manage an asset in the asset browser without loading the actual
/// asset data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetMetadata {
    /// Unique identifier for this asset
    pub id: AssetId,

    /// Path relative to the assets root directory
    pub path: PathBuf,

    /// Classification of the asset type
    pub asset_type: AssetType,

    /// User-facing display name (defaults to filename without extension)
    pub display_name: String,

    /// User-defined tags for organization and filtering
    pub tags: Vec<String>,

    /// File size in bytes
    pub file_size: u64,

    /// Last modification time of the source file
    pub last_modified: SystemTime,

    /// Hash of the thumbnail for cache invalidation
    /// None if no thumbnail has been generated
    pub thumbnail_hash: Option<u64>,

    /// Assets that this asset depends on (e.g., textures used by a model)
    pub dependencies: Vec<AssetId>,

    /// Time when this asset was first registered
    pub created_at: SystemTime,
}

impl AssetMetadata {
    /// Create new metadata from a file path
    ///
    /// Automatically determines asset type and extracts file information.
    /// Returns None if the file doesn't exist or can't be read.
    pub fn from_path(path: PathBuf, root: &std::path::Path) -> Option<Self> {
        let full_path = root.join(&path);
        let metadata = std::fs::metadata(&full_path).ok()?;

        let display_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string();

        let asset_type = AssetType::from_path(&path);
        let id = AssetId::from_path(path.to_str().unwrap_or(""));

        Some(Self {
            id,
            path,
            asset_type,
            display_name,
            tags: Vec::new(),
            file_size: metadata.len(),
            last_modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            thumbnail_hash: None,
            dependencies: Vec::new(),
            created_at: SystemTime::now(),
        })
    }

    /// Create metadata with explicit values (for testing or manual creation)
    pub fn new(path: PathBuf, asset_type: AssetType, display_name: String, file_size: u64) -> Self {
        let id = AssetId::from_path(path.to_str().unwrap_or(""));
        Self {
            id,
            path,
            asset_type,
            display_name,
            tags: Vec::new(),
            file_size,
            last_modified: SystemTime::now(),
            thumbnail_hash: None,
            dependencies: Vec::new(),
            created_at: SystemTime::now(),
        }
    }

    /// Get the filename without directory path
    pub fn filename(&self) -> &str {
        self.path.file_name().and_then(|s| s.to_str()).unwrap_or("")
    }

    /// Get the parent folder path
    pub fn folder(&self) -> Option<&std::path::Path> {
        self.path.parent()
    }

    /// Check if the asset matches a search query
    ///
    /// Searches in display name, filename, and tags (case-insensitive)
    pub fn matches_search(&self, query: &str) -> bool {
        let query_lower = query.to_lowercase();

        // Check display name
        if self.display_name.to_lowercase().contains(&query_lower) {
            return true;
        }

        // Check filename
        if self.filename().to_lowercase().contains(&query_lower) {
            return true;
        }

        // Check tags
        for tag in &self.tags {
            if tag.to_lowercase().contains(&query_lower) {
                return true;
            }
        }

        false
    }

    /// Add a tag to this asset
    pub fn add_tag(&mut self, tag: String) {
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
        }
    }

    /// Remove a tag from this asset
    pub fn remove_tag(&mut self, tag: &str) {
        self.tags.retain(|t| t != tag);
    }

    /// Check if the asset has a specific tag
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// Format file size for display (e.g., "2.4 MB")
    pub fn formatted_size(&self) -> String {
        const KB: u64 = 1024;
        const MB: u64 = KB * 1024;
        const GB: u64 = MB * 1024;

        if self.file_size >= GB {
            format!("{:.1} GB", self.file_size as f64 / GB as f64)
        } else if self.file_size >= MB {
            format!("{:.1} MB", self.file_size as f64 / MB as f64)
        } else if self.file_size >= KB {
            format!("{:.1} KB", self.file_size as f64 / KB as f64)
        } else {
            format!("{} B", self.file_size)
        }
    }

    /// Check if the source file has been modified since metadata was created
    pub fn is_stale(&self, root: &std::path::Path) -> bool {
        let full_path = root.join(&self.path);
        if let Ok(metadata) = std::fs::metadata(&full_path) {
            if let Ok(modified) = metadata.modified() {
                return modified > self.last_modified;
            }
        }
        // If we can't check, assume stale
        true
    }

    /// Refresh metadata from disk
    pub fn refresh(&mut self, root: &std::path::Path) -> bool {
        let full_path = root.join(&self.path);
        if let Ok(metadata) = std::fs::metadata(&full_path) {
            self.file_size = metadata.len();
            if let Ok(modified) = metadata.modified() {
                self.last_modified = modified;
            }
            // Invalidate thumbnail since file changed
            self.thumbnail_hash = None;
            true
        } else {
            false
        }
    }
}

impl PartialEq for AssetMetadata {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for AssetMetadata {}

impl std::hash::Hash for AssetMetadata {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_formatted_size() {
        let mut meta = AssetMetadata::new(
            PathBuf::from("test.png"),
            AssetType::Texture,
            "test".to_string(),
            0,
        );

        meta.file_size = 512;
        assert_eq!(meta.formatted_size(), "512 B");

        meta.file_size = 2048;
        assert_eq!(meta.formatted_size(), "2.0 KB");

        meta.file_size = 2_500_000;
        assert_eq!(meta.formatted_size(), "2.4 MB");
    }

    #[test]
    fn test_matches_search() {
        let mut meta = AssetMetadata::new(
            PathBuf::from("textures/diffuse_map.png"),
            AssetType::Texture,
            "Diffuse Map".to_string(),
            1000,
        );
        meta.tags = vec!["character".to_string(), "pbr".to_string()];

        assert!(meta.matches_search("diffuse"));
        assert!(meta.matches_search("DIFFUSE"));
        assert!(meta.matches_search("character"));
        assert!(meta.matches_search("map"));
        assert!(!meta.matches_search("normal"));
    }
}
