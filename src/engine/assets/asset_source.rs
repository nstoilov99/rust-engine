//! Abstracts where game content is loaded from.
//!
//! - **Filesystem**: used by the editor and development builds; reads loose files.
//! - **Pak**: used by shipping/release builds; reads from a single `.pak` archive.
//!
//! A global `AssetSource` is set once at startup via [`init`] and queried via
//! [`read_bytes`] / [`read_string`] / [`exists`].

use super::pak::PakReader;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static ASSET_SOURCE: OnceLock<AssetSource> = OnceLock::new();

enum AssetSource {
    Filesystem { content_root: PathBuf },
    Pak(PakReader),
}

/// Initialize the global asset source.
///
/// Call exactly once at startup before any assets are loaded.
/// In the editor or dev builds, pass the content root directory.
/// In shipping builds, pass the path to `game.pak`.
///
/// Panics if called more than once.
pub fn init_filesystem(content_root: PathBuf) {
    ASSET_SOURCE
        .set(AssetSource::Filesystem { content_root })
        .unwrap_or_else(|_| panic!("AssetSource already initialized"));
}

pub fn init_pak(pak_path: &Path) {
    let reader = PakReader::open(pak_path)
        .unwrap_or_else(|e| panic!("Failed to open pak file {}: {}", pak_path.display(), e));
    println!(
        "Loaded pak: {} ({} files)",
        pak_path.display(),
        reader.entry_count()
    );
    ASSET_SOURCE
        .set(AssetSource::Pak(reader))
        .unwrap_or_else(|_| panic!("AssetSource already initialized"));
}

fn source() -> &'static AssetSource {
    ASSET_SOURCE
        .get()
        .expect("AssetSource not initialized — call asset_source::init_* at startup")
}

/// Read a content-relative file as raw bytes.
///
/// `relative` uses forward slashes and does NOT start with `content/`.
/// Example: `"models/Duck.glb"`, `"scenes/main.scene.ron"`.
pub fn read_bytes(relative: &str) -> Result<Vec<u8>, std::io::Error> {
    match source() {
        AssetSource::Filesystem { content_root } => {
            let path = content_root.join(relative);
            std::fs::read(&path)
        }
        AssetSource::Pak(reader) => reader
            .read(relative)
            .map(|b| b.to_vec())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("not found in pak: {}", relative),
                )
            }),
    }
}

/// Read a content-relative file as a UTF-8 string.
pub fn read_string(relative: &str) -> Result<String, std::io::Error> {
    match source() {
        AssetSource::Filesystem { content_root } => {
            let path = content_root.join(relative);
            std::fs::read_to_string(&path)
        }
        AssetSource::Pak(reader) => reader.read_string(relative).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("not found in pak: {}", relative),
            )
        }),
    }
}

/// Check if a content-relative file exists.
pub fn exists(relative: &str) -> bool {
    match source() {
        AssetSource::Filesystem { content_root } => content_root.join(relative).exists(),
        AssetSource::Pak(reader) => reader.contains(relative),
    }
}

/// Returns the filesystem content root, if using filesystem source.
/// Used by editor-only code that needs to write files (save scene, hot-reload).
pub fn content_root_path() -> Option<PathBuf> {
    match source() {
        AssetSource::Filesystem { content_root } => Some(content_root.clone()),
        AssetSource::Pak(_) => None,
    }
}

/// Returns true if running from a pak file.
pub fn is_pak() -> bool {
    matches!(source(), AssetSource::Pak(_))
}

/// Resolve a content-relative path to an absolute filesystem path.
///
/// Only meaningful in filesystem mode; in pak mode this still returns a path
/// based on a fallback `"content"` prefix (useful for logging / error messages).
pub fn resolve(relative: &str) -> PathBuf {
    match source() {
        AssetSource::Filesystem { content_root } => content_root.join(relative),
        AssetSource::Pak(_) => PathBuf::from("content").join(relative),
    }
}

/// Extract the content-relative portion from a path.
///
/// Handles absolute paths (`C:/foo/content/models/Duck.glb` -> `models/Duck.glb`),
/// relative paths (`content/models/Duck.glb` -> `models/Duck.glb`),
/// and already-relative paths (`models/Duck.glb` -> `models/Duck.glb`).
pub fn to_content_relative(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    if let Some(idx) = normalized.find("/content/") {
        normalized[idx + "/content/".len()..].to_string()
    } else if let Some(stripped) = normalized.strip_prefix("content/") {
        stripped.to_string()
    } else {
        normalized
    }
}
