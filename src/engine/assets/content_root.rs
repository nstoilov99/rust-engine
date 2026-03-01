//! Content directory resolution for standalone and editor builds
//!
//! Resolution order:
//! 1. `<exe_dir>/content`
//! 2. `<cwd>/content`
//! 3. relative `"content"`

use std::path::PathBuf;

/// Resolve the content root directory.
///
/// Tries in order:
/// 1. Next to the executable
/// 2. In the current working directory
/// 3. Falls back to relative `"content"`
pub fn content_root() -> PathBuf {
    // 1. Next to the executable
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let candidate = exe_dir.join("content");
            if candidate.is_dir() {
                return candidate;
            }
        }
    }

    // 2. Current working directory
    if let Ok(cwd) = std::env::current_dir() {
        let candidate = cwd.join("content");
        if candidate.is_dir() {
            return candidate;
        }
    }

    // 3. Relative fallback
    PathBuf::from("content")
}

