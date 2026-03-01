pub mod texture;
pub mod model_loader;
pub mod handle;
pub mod texture_manager;
pub mod asset_manager;
pub mod model_manager;
pub mod content_root;
pub mod pak;
pub mod asset_source;
#[cfg(feature = "editor")]
pub mod hot_reload;
pub mod dependencies;
#[cfg(feature = "editor")]
pub mod async_loader;
pub mod asset_type;
pub mod metadata;

pub use handle::{Handle, AssetId};
pub use texture::{load_texture};
pub use texture_manager::TextureManager;
pub use model_manager::ModelManager;
pub use asset_manager::{AssetManager, CacheStats};
#[cfg(feature = "editor")]
pub use hot_reload::{HotReloadWatcher, ReloadEvent};
pub use dependencies::{AssetDependencies, DependencyStats};
#[cfg(feature = "editor")]
pub use async_loader::{AsyncAssetLoader, LoadRequest, LoadResult};
pub use model_loader::{Model, LoadedMesh, load_model, load_model_from_bytes, load_gltf, load_gltf_from_bytes};
pub use asset_type::AssetType;
pub use metadata::AssetMetadata;
pub use content_root::content_root;