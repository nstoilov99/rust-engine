//! Editor asset preview registry.
//!
//! The registry owns stable preview identifiers and optional egui texture IDs.
//! GPU producers can attach or update textures when available; editor panels can
//! still register previews and invalidate them before the renderer side exists.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PreviewId(u64);

impl PreviewId {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreviewKind {
    Material,
    MaterialInstance,
    Texture,
}

#[derive(Debug, Clone)]
pub struct AssetPreview {
    pub id: PreviewId,
    pub kind: PreviewKind,
    pub key: String,
    pub source_path: PathBuf,
    pub texture_id: Option<egui::TextureId>,
    pub dirty: bool,
}

#[derive(Debug)]
pub struct AssetPreviewRegistry {
    next_id: u64,
    previews: HashMap<PreviewId, AssetPreview>,
    by_key: HashMap<(PreviewKind, String), PreviewId>,
}

impl Default for AssetPreviewRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetPreviewRegistry {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            previews: HashMap::new(),
            by_key: HashMap::new(),
        }
    }

    pub fn material_preview<P: AsRef<Path>>(&mut self, path: P) -> PreviewId {
        self.preview_for(PreviewKind::Material, path)
    }

    pub fn material_instance_preview<P: AsRef<Path>>(&mut self, path: P) -> PreviewId {
        self.preview_for(PreviewKind::MaterialInstance, path)
    }

    pub fn texture_preview<P: AsRef<Path>>(&mut self, path: P) -> PreviewId {
        self.preview_for(PreviewKind::Texture, path)
    }

    pub fn texture_for(&self, id: PreviewId) -> Option<egui::TextureId> {
        self.previews
            .get(&id)
            .and_then(|preview| preview.texture_id)
    }

    pub fn set_texture(&mut self, id: PreviewId, texture_id: egui::TextureId) {
        if let Some(preview) = self.previews.get_mut(&id) {
            preview.texture_id = Some(texture_id);
            preview.dirty = false;
        }
    }

    pub fn invalidate(&mut self, id: PreviewId) {
        if let Some(preview) = self.previews.get_mut(&id) {
            preview.dirty = true;
        }
    }

    pub fn is_dirty(&self, id: PreviewId) -> bool {
        self.previews
            .get(&id)
            .map(|preview| preview.dirty)
            .unwrap_or(false)
    }

    pub fn get(&self, id: PreviewId) -> Option<&AssetPreview> {
        self.previews.get(&id)
    }

    pub fn remove(&mut self, id: PreviewId) -> Option<AssetPreview> {
        let preview = self.previews.remove(&id)?;
        self.by_key.remove(&(preview.kind, preview.key.clone()));
        Some(preview)
    }

    fn preview_for<P: AsRef<Path>>(&mut self, kind: PreviewKind, path: P) -> PreviewId {
        let source_path = path.as_ref().to_path_buf();
        let key = normalize_preview_key(&source_path);
        if let Some(id) = self.by_key.get(&(kind, key.clone())) {
            return *id;
        }

        let id = PreviewId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.previews.insert(
            id,
            AssetPreview {
                id,
                kind,
                key: key.clone(),
                source_path,
                texture_id: None,
                dirty: true,
            },
        );
        self.by_key.insert((kind, key), id);
        id
    }
}

fn normalize_preview_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}
