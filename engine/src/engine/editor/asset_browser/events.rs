//! Asset browser events
//!
//! Defines events for communication between the asset browser and other
//! editor panels (viewport, inspector, etc.).

use crate::engine::assets::{AssetId, AssetType};
use hecs::Entity;
use nalgebra_glm as glm;
use std::path::PathBuf;

/// Events emitted by the asset browser for other panels to handle
#[derive(Debug, Clone)]
pub enum AssetBrowserEvent {
    /// An asset was selected in the browser
    AssetSelected { id: AssetId },

    /// Asset selection was cleared
    SelectionCleared,

    /// An asset was dropped in the viewport
    AssetDroppedInViewport {
        id: AssetId,
        /// World position where the asset was dropped
        position: glm::Vec3,
        /// Entity created from the drop (if successful)
        created_entity: Option<Entity>,
    },

    /// An asset was imported or reimported
    AssetImported { id: AssetId, path: PathBuf },

    /// An asset was deleted
    AssetDeleted { id: AssetId, path: PathBuf },

    /// An asset was renamed
    AssetRenamed {
        id: AssetId,
        old_name: String,
        new_name: String,
    },

    /// An asset was moved to a different folder
    AssetMoved {
        id: AssetId,
        old_path: PathBuf,
        new_path: PathBuf,
    },

    /// Request to focus/reveal an asset in the browser
    ///
    /// Other panels can emit this to navigate to a specific asset.
    FocusAsset { id: AssetId },

    /// Request to reveal an asset in the system file explorer
    RevealInExplorer { path: PathBuf },

    /// Asset was double-clicked (open for editing)
    AssetOpened { id: AssetId },

    /// Asset tags were modified
    TagsModified { id: AssetId, tags: Vec<String> },

    /// Asset browser folder changed
    FolderChanged { path: PathBuf },

    // === Folder Operations ===
    /// A folder was renamed
    FolderRenamed {
        old_path: PathBuf,
        new_path: PathBuf,
    },

    /// A folder was moved to a different location
    FolderMoved {
        old_path: PathBuf,
        new_path: PathBuf,
    },

    /// A folder was deleted
    FolderDeleted { path: PathBuf },

    /// Request to create a new folder
    CreateFolder { parent_path: PathBuf },

    /// Request to create a new asset of the given type
    CreateAsset {
        asset_type: AssetType,
        parent_path: PathBuf,
    },

    /// Request to reveal a folder in the system file explorer
    RevealFolderInExplorer { path: PathBuf },
}

impl AssetBrowserEvent {
    /// Get the asset ID associated with this event, if any
    pub fn asset_id(&self) -> Option<AssetId> {
        match self {
            AssetBrowserEvent::AssetSelected { id } => Some(*id),
            AssetBrowserEvent::AssetDroppedInViewport { id, .. } => Some(*id),
            AssetBrowserEvent::AssetImported { id, .. } => Some(*id),
            AssetBrowserEvent::AssetDeleted { id, .. } => Some(*id),
            AssetBrowserEvent::AssetRenamed { id, .. } => Some(*id),
            AssetBrowserEvent::AssetMoved { id, .. } => Some(*id),
            AssetBrowserEvent::FocusAsset { id } => Some(*id),
            AssetBrowserEvent::AssetOpened { id } => Some(*id),
            AssetBrowserEvent::TagsModified { id, .. } => Some(*id),
            AssetBrowserEvent::SelectionCleared => None,
            AssetBrowserEvent::RevealInExplorer { .. } => None,
            AssetBrowserEvent::FolderChanged { .. } => None,
            AssetBrowserEvent::FolderRenamed { .. } => None,
            AssetBrowserEvent::FolderMoved { .. } => None,
            AssetBrowserEvent::FolderDeleted { .. } => None,
            AssetBrowserEvent::CreateFolder { .. } => None,
            AssetBrowserEvent::CreateAsset { .. } => None,
            AssetBrowserEvent::RevealFolderInExplorer { .. } => None,
        }
    }
}

/// Queue for collecting events during a frame
///
/// Events are processed at the end of the frame to avoid borrow issues.
#[derive(Debug, Default)]
pub struct AssetEventQueue {
    events: Vec<AssetBrowserEvent>,
}

impl AssetEventQueue {
    /// Create a new empty event queue
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an event to the queue
    pub fn push(&mut self, event: AssetBrowserEvent) {
        self.events.push(event);
    }

    /// Drain all events from the queue
    pub fn drain(&mut self) -> impl Iterator<Item = AssetBrowserEvent> + '_ {
        self.events.drain(..)
    }

    /// Check if the queue has any events
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Get the number of pending events
    pub fn len(&self) -> usize {
        self.events.len()
    }
}
