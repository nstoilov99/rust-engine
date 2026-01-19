//! Asset selection management
//!
//! Handles single and multi-selection of assets in the asset browser,
//! mirroring the entity Selection system.

use crate::engine::assets::AssetId;
use std::collections::HashSet;

/// Manages the selection state for assets in the asset browser
///
/// Supports single selection, multi-selection with Ctrl+click,
/// and range selection with Shift+click.
#[derive(Debug, Default)]
pub struct AssetSelection {
    /// Set of all selected asset IDs
    selected: HashSet<AssetId>,
    /// The primary (most recently) selected asset
    primary: Option<AssetId>,
    /// The anchor point for shift-click range selection
    anchor: Option<AssetId>,
}

impl AssetSelection {
    /// Create a new empty selection
    pub fn new() -> Self {
        Self::default()
    }

    /// Select a single asset, clearing previous selection
    pub fn select(&mut self, id: AssetId) {
        self.selected.clear();
        self.selected.insert(id);
        self.primary = Some(id);
        self.anchor = Some(id);
    }

    /// Add an asset to the selection (Ctrl+click behavior)
    pub fn add(&mut self, id: AssetId) {
        self.selected.insert(id);
        self.primary = Some(id);
        // Don't update anchor for add operations
    }

    /// Remove an asset from the selection
    pub fn remove(&mut self, id: AssetId) {
        self.selected.remove(&id);
        if self.primary == Some(id) {
            self.primary = self.selected.iter().next().copied();
        }
        if self.anchor == Some(id) {
            self.anchor = self.primary;
        }
    }

    /// Toggle an asset's selection state (Ctrl+click)
    pub fn toggle(&mut self, id: AssetId) {
        if self.selected.contains(&id) {
            self.remove(id);
        } else {
            self.add(id);
        }
    }

    /// Clear all selection
    pub fn clear(&mut self) {
        self.selected.clear();
        self.primary = None;
        self.anchor = None;
    }

    /// Check if an asset is selected
    pub fn is_selected(&self, id: AssetId) -> bool {
        self.selected.contains(&id)
    }

    /// Get the primary (most recently) selected asset
    pub fn primary(&self) -> Option<AssetId> {
        self.primary
    }

    /// Get the selection anchor (for range selection)
    pub fn anchor(&self) -> Option<AssetId> {
        self.anchor
    }

    /// Set the selection anchor
    pub fn set_anchor(&mut self, id: AssetId) {
        self.anchor = Some(id);
    }

    /// Get all selected assets
    pub fn all(&self) -> impl Iterator<Item = AssetId> + '_ {
        self.selected.iter().copied()
    }

    /// Get all selected assets as a slice (for batch operations)
    pub fn as_vec(&self) -> Vec<AssetId> {
        self.selected.iter().copied().collect()
    }

    /// Get the number of selected assets
    pub fn count(&self) -> usize {
        self.selected.len()
    }

    /// Check if any assets are selected
    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    /// Select multiple assets at once
    pub fn select_multiple(&mut self, ids: impl IntoIterator<Item = AssetId>) {
        self.selected.clear();
        for id in ids {
            self.selected.insert(id);
            if self.primary.is_none() {
                self.primary = Some(id);
                self.anchor = Some(id);
            }
        }
    }

    /// Add multiple assets to selection
    pub fn add_multiple(&mut self, ids: impl IntoIterator<Item = AssetId>) {
        for id in ids {
            self.selected.insert(id);
        }
        // Update primary to last added
        if let Some(id) = self.selected.iter().last().copied() {
            self.primary = Some(id);
        }
    }

    /// Select a range of assets (Shift+click behavior)
    ///
    /// Takes an ordered list of all visible assets and selects from anchor to target.
    pub fn select_range(&mut self, visible_assets: &[AssetId], target: AssetId) {
        let anchor = self.anchor.unwrap_or(target);

        // Find positions of anchor and target
        let anchor_pos = visible_assets.iter().position(|&id| id == anchor);
        let target_pos = visible_assets.iter().position(|&id| id == target);

        if let (Some(start), Some(end)) = (anchor_pos, target_pos) {
            let (start, end) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };

            // Select all assets in range
            self.selected.clear();
            for &id in &visible_assets[start..=end] {
                self.selected.insert(id);
            }
            self.primary = Some(target);
            // Keep the original anchor
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_id(n: u64) -> AssetId {
        AssetId::new(n)
    }

    #[test]
    fn test_single_select() {
        let mut selection = AssetSelection::new();
        selection.select(make_id(1));

        assert!(selection.is_selected(make_id(1)));
        assert_eq!(selection.count(), 1);
        assert_eq!(selection.primary(), Some(make_id(1)));
    }

    #[test]
    fn test_multi_select() {
        let mut selection = AssetSelection::new();
        selection.select(make_id(1));
        selection.add(make_id(2));
        selection.add(make_id(3));

        assert!(selection.is_selected(make_id(1)));
        assert!(selection.is_selected(make_id(2)));
        assert!(selection.is_selected(make_id(3)));
        assert_eq!(selection.count(), 3);
    }

    #[test]
    fn test_toggle() {
        let mut selection = AssetSelection::new();
        selection.select(make_id(1));
        selection.toggle(make_id(2));
        selection.toggle(make_id(1));

        assert!(!selection.is_selected(make_id(1)));
        assert!(selection.is_selected(make_id(2)));
        assert_eq!(selection.count(), 1);
    }

    #[test]
    fn test_range_select() {
        let mut selection = AssetSelection::new();
        let assets: Vec<AssetId> = (1..=10).map(make_id).collect();

        selection.select(make_id(3));
        selection.select_range(&assets, make_id(7));

        assert_eq!(selection.count(), 5); // 3, 4, 5, 6, 7
        assert!(selection.is_selected(make_id(3)));
        assert!(selection.is_selected(make_id(5)));
        assert!(selection.is_selected(make_id(7)));
        assert!(!selection.is_selected(make_id(2)));
        assert!(!selection.is_selected(make_id(8)));
    }
}
