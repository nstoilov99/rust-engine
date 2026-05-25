//! Centralized asset-open routing.
//!
//! `window_kind_for_extension` is the single source of truth for mapping an
//! asset path to a `SecondaryWindowKind` + editor key. All previous ad-hoc
//! extension branches in the `AssetBrowserEvent::AssetOpened` handler are
//! folded into this module.

use std::path::Path;

use super::secondary_window::{PendingWindowRequest, SecondaryWindowKind};

/// Map an asset path to (SecondaryWindowKind, content-relative key).
/// Returns None when no editor is registered for the extension.
///
/// `.scene.ron` is intentionally NOT handled here — scene loading goes
/// through its own viewport path, not a secondary window.
pub fn window_kind_for_extension(path: &Path) -> Option<(SecondaryWindowKind, String)> {
    use SecondaryWindowKind::*;
    let key = path.to_string_lossy().to_string();
    let lower = key.to_lowercase();

    // Double-extension RON assets (order matters: more specific first)
    if lower.ends_with(".material.ron") {
        return Some((Material, key));
    }
    if lower.ends_with(".matinst.ron") {
        return Some((MaterialInstance, key));
    }
    if lower.ends_with(".mappingcontext.ron") {
        return Some((InputContext, key));
    }
    if lower.ends_with(".inputaction.ron") {
        return Some((InputAction, key));
    }
    if lower.ends_with(".mesh.ron") {
        return Some((Mesh, key));
    }
    if lower.ends_with(".animgraph.ron") {
        return Some((AnimationGraph, key));
    }
    if lower.ends_with(".matgraph.ron") {
        return Some((MaterialGraph, key));
    }
    if lower.ends_with(".prefab.ron") {
        return Some((Prefab, key));
    }

    // Single-extension assets matched by file extension
    match ext_lower(&lower).as_deref() {
        Some("png" | "jpg" | "jpeg" | "ktx2" | "tga" | "exr") => Some((Texture, key)),
        Some("wav" | "ogg" | "mp3" | "flac") => Some((Audio, key)),
        _ => None,
    }
}

/// Build a `PendingWindowRequest` for the given asset path and push it onto
/// the pending-request queue. Does nothing if no editor is registered.
pub fn route_asset_open(
    asset_path: &Path,
    pending_requests: &mut Vec<PendingWindowRequest>,
) {
    let Some((kind, key)) = window_kind_for_extension(asset_path) else {
        log::info!("No editor registered for {}", asset_path.display());
        return;
    };
    let (w, h) = kind.default_size();
    pending_requests.push(PendingWindowRequest {
        editor_key: key.clone(),
        kind,
        title: kind.window_title(&key),
        width: w,
        height: h,
        restored_position: None,
        restored_size: None,
        start_maximized: false,
        focus_existing_if_open: true,
    });
}

fn ext_lower(lower_path: &str) -> Option<String> {
    Path::new(lower_path)
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
}
