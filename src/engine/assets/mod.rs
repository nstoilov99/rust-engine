pub mod asset_manager;
pub mod asset_source;
pub mod asset_type;
#[cfg(feature = "editor")]
pub mod async_loader;
pub mod content_root;
pub mod dependencies;
pub mod handle;
#[cfg(feature = "editor")]
pub mod hot_reload;
pub mod metadata;
pub mod model_loader;
pub mod model_manager;
pub mod pak;
pub mod texture;
pub mod texture_manager;

pub use asset_manager::{AssetManager, CacheStats};
pub use asset_type::AssetType;
#[cfg(feature = "editor")]
pub use async_loader::{AsyncAssetLoader, LoadRequest, LoadResult};
pub use content_root::content_root;
pub use dependencies::{AssetDependencies, DependencyStats};
pub use handle::{AssetId, Handle};
#[cfg(feature = "editor")]
pub use hot_reload::{HotReloadWatcher, ReloadEvent};
pub use metadata::AssetMetadata;
pub use model_loader::{
    load_gltf, load_gltf_from_bytes, load_model, load_model_from_bytes, LoadedMesh, Model,
};
pub use model_manager::ModelManager;
pub use texture::load_texture;
pub use texture_manager::TextureManager;
