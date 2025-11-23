use std::sync::Arc;
use std::hash::{Hash, Hasher};
use std::fmt;

/// Type-safe handle to an asset
/// Generic over asset type (Texture, Model, etc.)
#[derive(Clone)]
pub struct Handle<T> {
    inner: Arc<HandleInner<T>>,
}

struct HandleInner<T> {
    id: AssetId,
    asset: Arc<T>,
}

/// Unique identifier for an asset
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AssetId(u64);

impl AssetId {
    pub fn from_path(path: &str) -> Self {
        use std::collections::hash_map::DefaultHasher;
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        AssetId(hasher.finish())
    }

    pub fn new(id: u64) -> Self {
        AssetId(id)
    }
}

impl<T> Handle<T> {
    pub fn new(id: AssetId, asset: Arc<T>) -> Self {
        Self {
            inner: Arc::new(HandleInner { id, asset }),
        }
    }

    pub fn id(&self) -> AssetId {
        self.inner.id
    }

    pub fn get(&self) -> &T {
        &self.inner.asset
    }

    pub fn clone_asset(&self) -> Arc<T> {
        self.inner.asset.clone()
    }

    pub fn get_arc(&self) -> Arc<T> {
        self.inner.asset.clone()
    }
}

// Implement equality based on ID
impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner.id == other.inner.id
    }
}

impl<T> Eq for Handle<T> {}

impl<T> Hash for Handle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.id.hash(state);
    }
}

impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handle")
            .field("id", &self.inner.id)
            .finish()
    }
}