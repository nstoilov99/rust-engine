//! Asset registry for persistent metadata storage
//!
//! The registry maintains an index of all known assets with their metadata,
//! providing fast lookup, filtering, and folder tree generation.

use crate::engine::assets::{AssetId, AssetMetadata, AssetType};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Result of scanning a directory for assets
#[derive(Debug, Default)]
pub struct ScanResult {
    /// Number of new assets discovered
    pub added: usize,
    /// Number of assets that were updated (file changed)
    pub updated: usize,
    /// Number of assets that were removed (file deleted)
    pub removed: usize,
    /// Errors encountered during scanning
    pub errors: Vec<String>,
}

/// Filtering criteria for asset queries
#[derive(Debug, Clone, Default)]
pub struct AssetFilter {
    /// Text to search in name, filename, and tags
    pub search_text: Option<String>,
    /// Filter by asset types
    pub asset_types: Option<Vec<AssetType>>,
    /// Filter by tags (any match)
    pub tags: Option<Vec<String>>,
    /// Filter by folder path
    pub folder: Option<PathBuf>,
    /// Include assets from subfolders
    pub include_subfolders: bool,
    /// Sort criteria
    pub sort_by: SortCriteria,
    /// Sort direction
    pub sort_ascending: bool,
    /// Paths hidden from queries.
    pub excluded_paths: Vec<PathBuf>,
}

/// Sorting options for asset lists
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortCriteria {
    #[default]
    Name,
    Type,
    DateModified,
    Size,
}

/// Node in the folder tree structure
#[derive(Debug, Clone)]
pub struct FolderNode {
    /// Folder name (leaf component)
    pub name: String,
    /// Full path relative to assets root
    pub path: PathBuf,
    /// Child folders
    pub children: Vec<FolderNode>,
    /// Number of assets directly in this folder
    pub asset_count: usize,
    /// Number of assets including all subfolders
    pub total_asset_count: usize,
}

impl FolderNode {
    fn new(name: String, path: PathBuf) -> Self {
        Self {
            name,
            path,
            children: Vec::new(),
            asset_count: 0,
            total_asset_count: 0,
        }
    }
}

/// Persistent registry of all known assets
///
/// Maintains multiple indices for fast lookup and filtering.
#[derive(Debug, Serialize, Deserialize)]
pub struct AssetRegistry {
    /// All assets indexed by ID
    assets: HashMap<AssetId, AssetMetadata>,

    /// Path -> AssetId lookup
    #[serde(skip)]
    path_index: HashMap<PathBuf, AssetId>,

    /// Type -> Vec<AssetId> for filtering
    #[serde(skip)]
    type_index: HashMap<AssetType, Vec<AssetId>>,

    /// Tag -> Vec<AssetId> for tag-based search
    #[serde(skip)]
    tag_index: HashMap<String, Vec<AssetId>>,

    /// Root asset directory
    root_path: PathBuf,

    /// Dirty flag for persistence
    #[serde(skip)]
    dirty: bool,
}

impl AssetRegistry {
    /// Create a new registry for the given assets directory
    pub fn new(root_path: PathBuf) -> Self {
        Self {
            assets: HashMap::new(),
            path_index: HashMap::new(),
            type_index: HashMap::new(),
            tag_index: HashMap::new(),
            root_path,
            dirty: false,
        }
    }

    /// Get the root assets directory
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    /// Rebuild indices from the assets map
    ///
    /// Called after loading from disk or when indices become stale.
    pub fn rebuild_indices(&mut self) {
        self.path_index.clear();
        self.type_index.clear();
        self.tag_index.clear();

        for (id, metadata) in &self.assets {
            // Path index
            self.path_index.insert(metadata.path.clone(), *id);

            // Type index
            self.type_index
                .entry(metadata.asset_type)
                .or_default()
                .push(*id);

            // Tag index
            for tag in &metadata.tags {
                self.tag_index.entry(tag.clone()).or_default().push(*id);
            }
        }
    }

    /// Scan the assets directory and update the registry
    pub fn scan_directory(&mut self) -> ScanResult {
        let mut result = ScanResult::default();
        let mut found_paths: HashSet<PathBuf> = HashSet::new();

        // Walk the directory tree
        if let Err(e) = self.scan_directory_recursive(&self.root_path.clone(), &mut found_paths, &mut result) {
            result.errors.push(format!("Failed to scan directory: {}", e));
        }

        // Remove assets whose files no longer exist
        let to_remove: Vec<AssetId> = self
            .assets
            .iter()
            .filter(|(_, meta)| !found_paths.contains(&meta.path))
            .map(|(id, _)| *id)
            .collect();

        for id in to_remove {
            self.unregister(id);
            result.removed += 1;
        }

        if result.added > 0 || result.updated > 0 || result.removed > 0 {
            self.dirty = true;
        }

        result
    }

    fn scan_directory_recursive(
        &mut self,
        dir: &Path,
        found_paths: &mut HashSet<PathBuf>,
        result: &mut ScanResult,
    ) -> std::io::Result<()> {
        let entries = std::fs::read_dir(dir)?;

        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                // Recursively scan subdirectories
                self.scan_directory_recursive(&path, found_paths, result)?;
            } else if path.is_file() {
                // Get relative path
                let relative_path = path
                    .strip_prefix(&self.root_path)
                    .unwrap_or(&path)
                    .to_path_buf();

                // Check if this is a supported asset type
                let asset_type = AssetType::from_path(&relative_path);
                if asset_type == AssetType::Unknown {
                    continue;
                }

                found_paths.insert(relative_path.clone());

                // Check if asset already exists
                if let Some(&id) = self.path_index.get(&relative_path) {
                    // Check if file was modified
                    if let Some(metadata) = self.assets.get_mut(&id) {
                        if metadata.is_stale(&self.root_path) {
                            metadata.refresh(&self.root_path);
                            result.updated += 1;
                        }
                    }
                } else {
                    // New asset
                    if let Some(metadata) = AssetMetadata::from_path(relative_path, &self.root_path) {
                        self.register(metadata);
                        result.added += 1;
                    }
                }
            }
        }

        Ok(())
    }

    /// Register a new asset
    pub fn register(&mut self, metadata: AssetMetadata) {
        let id = metadata.id;
        let path = metadata.path.clone();
        let asset_type = metadata.asset_type;
        let tags = metadata.tags.clone();

        // Update path index
        self.path_index.insert(path, id);

        // Update type index
        self.type_index.entry(asset_type).or_default().push(id);

        // Update tag index
        for tag in tags {
            self.tag_index.entry(tag).or_default().push(id);
        }

        // Store metadata
        self.assets.insert(id, metadata);
        self.dirty = true;
    }

    /// Update an existing asset's metadata
    pub fn update(&mut self, id: AssetId, metadata: AssetMetadata) {
        if let Some(old) = self.assets.get(&id) {
            // Remove old tags from index
            for tag in &old.tags {
                if let Some(ids) = self.tag_index.get_mut(tag) {
                    ids.retain(|&i| i != id);
                }
            }
        }

        // Add new tags to index
        for tag in &metadata.tags {
            self.tag_index.entry(tag.clone()).or_default().push(id);
        }

        self.assets.insert(id, metadata);
        self.dirty = true;
    }

    /// Remove an asset from the registry
    pub fn unregister(&mut self, id: AssetId) {
        if let Some(metadata) = self.assets.remove(&id) {
            // Remove from path index
            self.path_index.remove(&metadata.path);

            // Remove from type index
            if let Some(ids) = self.type_index.get_mut(&metadata.asset_type) {
                ids.retain(|&i| i != id);
            }

            // Remove from tag index
            for tag in &metadata.tags {
                if let Some(ids) = self.tag_index.get_mut(tag) {
                    ids.retain(|&i| i != id);
                }
            }

            self.dirty = true;
        }
    }

    /// Get asset metadata by ID
    pub fn get(&self, id: AssetId) -> Option<&AssetMetadata> {
        self.assets.get(&id)
    }

    /// Get mutable asset metadata by ID
    pub fn get_mut(&mut self, id: AssetId) -> Option<&mut AssetMetadata> {
        self.dirty = true;
        self.assets.get_mut(&id)
    }

    /// Get asset by path
    pub fn get_by_path(&self, path: &Path) -> Option<&AssetMetadata> {
        self.path_index.get(path).and_then(|id| self.assets.get(id))
    }

    /// Query assets with filters
    pub fn query(&self, filter: &AssetFilter) -> Vec<&AssetMetadata> {
        let mut results: Vec<&AssetMetadata> = self
            .assets
            .values()
            .filter(|meta| self.matches_filter(meta, filter))
            .collect();

        // Sort results
        results.sort_by(|a, b| {
            let cmp = match filter.sort_by {
                SortCriteria::Name => a.display_name.cmp(&b.display_name),
                SortCriteria::Type => a.asset_type.display_name().cmp(b.asset_type.display_name()),
                SortCriteria::DateModified => a.last_modified.cmp(&b.last_modified),
                SortCriteria::Size => a.file_size.cmp(&b.file_size),
            };
            if filter.sort_ascending {
                cmp
            } else {
                cmp.reverse()
            }
        });

        results
    }

    fn matches_filter(&self, metadata: &AssetMetadata, filter: &AssetFilter) -> bool {
        if filter
            .excluded_paths
            .iter()
            .any(|path| path == &metadata.path)
        {
            return false;
        }

        // Check folder filter
        if let Some(folder) = &filter.folder {
            if let Some(parent) = metadata.path.parent() {
                if filter.include_subfolders {
                    if !parent.starts_with(folder) && parent != folder.as_path() {
                        return false;
                    }
                } else if parent != folder.as_path() {
                    return false;
                }
            } else {
                return false;
            }
        }

        // Check type filter
        if let Some(types) = &filter.asset_types {
            if !types.contains(&metadata.asset_type) {
                return false;
            }
        }

        // Check tag filter
        if let Some(tags) = &filter.tags {
            if !tags.iter().any(|t| metadata.tags.contains(t)) {
                return false;
            }
        }

        // Check search text
        if let Some(search) = &filter.search_text {
            if !search.is_empty() && !metadata.matches_search(search) {
                return false;
            }
        }

        true
    }

    /// Get all folders in the asset directory (including empty folders)
    pub fn get_folders(&self) -> Vec<PathBuf> {
        let mut folders: HashSet<PathBuf> = HashSet::new();

        // Walk the filesystem to find ALL folders (including empty ones)
        self.collect_folders_recursive(&self.root_path, &PathBuf::new(), &mut folders);

        let mut folders: Vec<PathBuf> = folders.into_iter().collect();
        folders.sort();
        folders
    }

    /// Recursively collect all folders from the filesystem
    fn collect_folders_recursive(
        &self,
        base_path: &Path,
        relative_path: &PathBuf,
        folders: &mut HashSet<PathBuf>,
    ) {
        let full_path = base_path.join(relative_path);

        if let Ok(entries) = std::fs::read_dir(&full_path) {
            for entry in entries.filter_map(|e| e.ok()) {
                let entry_path = entry.path();
                if entry_path.is_dir() {
                    // Calculate relative path from root
                    let folder_relative = if relative_path.as_os_str().is_empty() {
                        PathBuf::from(entry.file_name())
                    } else {
                        relative_path.join(entry.file_name())
                    };

                    // Add this folder
                    folders.insert(folder_relative.clone());

                    // Recurse into subdirectories
                    self.collect_folders_recursive(base_path, &folder_relative, folders);
                }
            }
        }
    }

    /// Generate a folder tree structure
    pub fn get_folder_tree(&self) -> FolderNode {
        let mut root = FolderNode::new("assets".to_string(), PathBuf::new());

        // Count assets per folder
        let mut folder_counts: HashMap<PathBuf, usize> = HashMap::new();
        for metadata in self.assets.values() {
            if let Some(parent) = metadata.path.parent() {
                *folder_counts.entry(parent.to_path_buf()).or_default() += 1;
            }
        }

        // Build tree structure
        let folders = self.get_folders();
        for folder in folders {
            self.insert_folder_node(&mut root, &folder, &folder_counts);
        }

        // Calculate total counts
        self.calculate_total_counts(&mut root);

        root
    }

    fn insert_folder_node(
        &self,
        root: &mut FolderNode,
        folder: &Path,
        counts: &HashMap<PathBuf, usize>,
    ) {
        let components: Vec<_> = folder.components().collect();
        let mut current = root;

        for (i, component) in components.iter().enumerate() {
            let name = component.as_os_str().to_string_lossy().to_string();
            let path: PathBuf = components[..=i].iter().collect();

            // Find or create child node
            let child_idx = current.children.iter().position(|c| c.name == name);

            if let Some(idx) = child_idx {
                current = &mut current.children[idx];
            } else {
                let mut node = FolderNode::new(name, path.clone());
                node.asset_count = counts.get(&path).copied().unwrap_or(0);
                current.children.push(node);
                current = current.children.last_mut().unwrap();
            }
        }
    }

    fn calculate_total_counts(&self, node: &mut FolderNode) {
        let mut total = node.asset_count;
        for child in &mut node.children {
            self.calculate_total_counts(child);
            total += child.total_asset_count;
        }
        node.total_asset_count = total;
    }

    /// Get all unique tags in the registry
    pub fn get_all_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self.tag_index.keys().cloned().collect();
        tags.sort();
        tags
    }

    /// Get number of assets with a specific tag
    pub fn tag_count(&self, tag: &str) -> usize {
        self.tag_index.get(tag).map(|ids| ids.len()).unwrap_or(0)
    }

    /// Get total number of registered assets
    pub fn len(&self) -> usize {
        self.assets.len()
    }

    /// Check if registry is empty
    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }

    /// Check if registry has unsaved changes
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Mark registry as clean (after saving)
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Save registry to a file (RON format)
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let contents = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Load registry from a file
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let contents = std::fs::read_to_string(path)?;
        let mut registry: Self = ron::from_str(&contents)?;
        registry.rebuild_indices();
        Ok(registry)
    }

    /// Load or create a new registry
    pub fn load_or_new(registry_path: &Path, assets_root: PathBuf) -> Self {
        match Self::load(registry_path) {
            Ok(mut registry) => {
                registry.root_path = assets_root;
                registry
            }
            Err(_) => Self::new(assets_root),
        }
    }
}

impl Default for AssetRegistry {
    fn default() -> Self {
        Self::new(PathBuf::from("assets"))
    }
}
