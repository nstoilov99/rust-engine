//! Generates `$OUT_DIR/collision_registry.rs`: `include_bytes!` references to
//! the cooked greybox collision chunks + manifest (M6 D1). The `.ccol` files
//! stay the single source of truth; only references are generated.
//!
//! Paths are emitted absolute with forward slashes — backslashes inside a
//! string literal would be parsed as escape sequences on Windows. No
//! `canonicalize()`: it yields `\\?\` UNC paths that `include_bytes!` rejects.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn literal_path(p: &Path) -> String {
    p.to_str()
        .expect("collision content path is not valid UTF-8")
        .replace('\\', "/")
}

fn git(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// M9 D4: soft build stamp — short git hash, `-dirty` when the tree has
/// uncommitted changes, `"unknown"` without git. Must match the format the
/// client stamps (`game_client/build.rs`) for the mismatch warning to work.
fn emit_build_id(workspace_root: &Path) {
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join(".git/HEAD").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join(".git/index").display()
    );
    // logs/HEAD changes on every commit — plain HEAD doesn't while the
    // branch stays the same.
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join(".git/logs/HEAD").display()
    );
    let build_id = match git(&["rev-parse", "--short", "HEAD"]).filter(|h| !h.is_empty()) {
        Some(hash) => {
            let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
            if dirty { format!("{hash}-dirty") } else { hash }
        }
        None => "unknown".to_string(),
    };
    println!("cargo:rustc-env=GIT_HASH={build_id}");
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    emit_build_id(&manifest_dir.join("../.."));
    let content = manifest_dir.join("../../content/collision/greybox");
    println!("cargo:rerun-if-changed={}", content.display());

    let manifest = content.join("manifest.ron");
    assert!(
        manifest.is_file(),
        "collision manifest missing: {} (run tools/greybox_gen)",
        manifest.display()
    );

    let mut chunks: Vec<PathBuf> = fs::read_dir(&content)
        .expect("read collision content dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "ccol"))
        .collect();
    chunks.sort();
    assert!(!chunks.is_empty(), "no .ccol chunks in {}", content.display());

    let mut out = String::new();
    let _ = writeln!(
        out,
        "pub static COLLISION_MANIFEST: &[u8] = include_bytes!(\"{}\");",
        literal_path(&manifest)
    );
    println!("cargo:rerun-if-changed={}", manifest.display());
    let _ = writeln!(out, "pub static COLLISION_CHUNKS: &[&[u8]] = &[");
    for chunk in &chunks {
        println!("cargo:rerun-if-changed={}", chunk.display());
        let _ = writeln!(out, "    include_bytes!(\"{}\"),", literal_path(chunk));
    }
    let _ = writeln!(out, "];");

    let dest = PathBuf::from(env::var("OUT_DIR").unwrap()).join("collision_registry.rs");
    fs::write(dest, out).expect("write collision_registry.rs");
}
