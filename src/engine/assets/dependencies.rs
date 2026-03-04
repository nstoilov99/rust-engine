use parking_lot::RwLock;
use std::collections::HashMap;

use super::handle::AssetId;

/// Tracks dependencies between assets
/// Example: Material depends on Texture, so when Texture reloads, Material should too
pub struct AssetDependencies {
    /// Maps asset ID -> list of assets that depend on it
    /// e.g., texture_id -> [material1_id, material2_id]
    dependents: RwLock<HashMap<AssetId, Vec<AssetId>>>,

    /// Maps asset ID -> list of assets it depends on
    /// e.g., material_id -> [texture1_id, texture2_id]
    dependencies: RwLock<HashMap<AssetId, Vec<AssetId>>>,
}

impl AssetDependencies {
    pub fn new() -> Self {
        Self {
            dependents: RwLock::new(HashMap::new()),
            dependencies: RwLock::new(HashMap::new()),
        }
    }

    /// Register that `asset` depends on `dependency`
    /// Example: add_dependency(material_id, texture_id) means material uses texture
    pub fn add_dependency(&self, asset: AssetId, dependency: AssetId) {
        // Add to dependents map (texture -> materials)
        {
            let mut dependents = self.dependents.write();
            dependents.entry(dependency).or_default().push(asset);
        }

        // Add to dependencies map (material -> textures)
        {
            let mut dependencies = self.dependencies.write();
            dependencies.entry(asset).or_default().push(dependency);
        }

        println!(
            "📎 Dependency added: {:?} depends on {:?}",
            asset, dependency
        );
    }

    /// Remove a dependency relationship
    pub fn remove_dependency(&self, asset: AssetId, dependency: AssetId) {
        // Remove from dependents
        {
            let mut dependents = self.dependents.write();
            if let Some(deps) = dependents.get_mut(&dependency) {
                deps.retain(|&id| id != asset);
            }
        }

        // Remove from dependencies
        {
            let mut dependencies = self.dependencies.write();
            if let Some(deps) = dependencies.get_mut(&asset) {
                deps.retain(|&id| id != dependency);
            }
        }
    }

    /// Get all assets that depend on this asset
    /// Example: get_dependents(texture_id) returns all materials using that texture
    pub fn get_dependents(&self, asset: AssetId) -> Vec<AssetId> {
        let dependents = self.dependents.read();
        dependents.get(&asset).cloned().unwrap_or_default()
    }

    /// Get all assets this asset depends on
    /// Example: get_dependencies(material_id) returns all textures it uses
    pub fn get_dependencies(&self, asset: AssetId) -> Vec<AssetId> {
        let dependencies = self.dependencies.read();
        dependencies.get(&asset).cloned().unwrap_or_default()
    }

    /// Get all dependents recursively (full dependency tree)
    pub fn get_all_dependents_recursive(&self, asset: AssetId) -> Vec<AssetId> {
        let mut result = Vec::new();
        let mut to_process = vec![asset];
        let mut visited = std::collections::HashSet::new();

        while let Some(current) = to_process.pop() {
            if !visited.insert(current) {
                continue; // Already processed
            }

            let deps = self.get_dependents(current);
            for dep in deps {
                if !result.contains(&dep) {
                    result.push(dep);
                }
                to_process.push(dep);
            }
        }

        result
    }

    /// Remove all dependencies for an asset (when it's unloaded)
    pub fn remove_asset(&self, asset: AssetId) {
        // Remove from dependents
        {
            let mut dependents = self.dependents.write();
            dependents.remove(&asset);

            // Also remove from other assets' dependent lists
            for deps in dependents.values_mut() {
                deps.retain(|&id| id != asset);
            }
        }

        // Remove from dependencies
        {
            let mut dependencies = self.dependencies.write();
            dependencies.remove(&asset);

            // Also remove from other assets' dependency lists
            for deps in dependencies.values_mut() {
                deps.retain(|&id| id != asset);
            }
        }
    }

    /// Clear all dependencies
    pub fn clear(&self) {
        self.dependents.write().clear();
        self.dependencies.write().clear();
    }

    /// Get statistics
    pub fn stats(&self) -> DependencyStats {
        let dependents = self.dependents.read();
        let dependencies = self.dependencies.read();

        DependencyStats {
            total_relationships: dependencies.values().map(|v| v.len()).sum(),
            assets_with_dependencies: dependencies.len(),
            assets_being_depended_on: dependents.len(),
        }
    }
}

impl Default for AssetDependencies {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DependencyStats {
    pub total_relationships: usize,
    pub assets_with_dependencies: usize,
    pub assets_being_depended_on: usize,
}

impl std::fmt::Display for DependencyStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Dependencies: {} relationships, {} assets depend on {} assets",
            self.total_relationships, self.assets_with_dependencies, self.assets_being_depended_on
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dependency_tracking() {
        let deps = AssetDependencies::new();

        let texture_id = AssetId::new(1);
        let material1_id = AssetId::new(2);
        let material2_id = AssetId::new(3);

        // Material 1 and 2 depend on texture
        deps.add_dependency(material1_id, texture_id);
        deps.add_dependency(material2_id, texture_id);

        // Check dependents
        let dependents = deps.get_dependents(texture_id);
        assert_eq!(dependents.len(), 2);
        assert!(dependents.contains(&material1_id));
        assert!(dependents.contains(&material2_id));

        // Check dependencies
        let dependencies = deps.get_dependencies(material1_id);
        assert_eq!(dependencies.len(), 1);
        assert_eq!(dependencies[0], texture_id);
    }
}
