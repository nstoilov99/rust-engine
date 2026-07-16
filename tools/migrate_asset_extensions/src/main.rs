//! Renames legacy `<name>.<type>.ron` assets to `<name>.<type>` and rewrites
//! references inside asset files. `.mesh.ron` sidecars are NOT renamed (they
//! would collide with the binary `.mesh` file) but references inside them are
//! rewritten.
//!
//! Usage: migrate_asset_extensions [--apply] [dir ...]   (default dirs: content assets)
//! Dry-run by default: prints the plan without touching disk.

use std::fs;
use std::path::{Path, PathBuf};

/// Legacy suffixes to strip the trailing `.ron` from, both as filenames and
/// inside file contents.
const LEGACY_SUFFIXES: &[&str] = &[
    ".scene.ron",
    ".material.ron",
    ".matinst.ron",
    ".prefab.ron",
    ".inputaction.ron",
    ".mappingcontext.ron",
];

/// Files whose text contents may reference other assets.
const TEXT_EXTENSIONS: &[&str] = &[
    "ron",
    "scene",
    "material",
    "matinst",
    "prefab",
    "inputaction",
    "mappingcontext",
];

fn main() {
    let mut apply = false;
    let mut roots: Vec<PathBuf> = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--apply" => apply = true,
            "--help" | "-h" => {
                println!("Usage: migrate_asset_extensions [--apply] [dir ...]");
                println!("Default dirs: content assets. Dry-run unless --apply.");
                return;
            }
            other => roots.push(PathBuf::from(other)),
        }
    }
    if roots.is_empty() {
        roots = vec![PathBuf::from("content"), PathBuf::from("assets")];
    }

    let mut files = Vec::new();
    for root in &roots {
        if root.is_dir() {
            collect_files(root, &mut files);
        } else {
            eprintln!("warning: '{}' is not a directory, skipping", root.display());
        }
    }
    files.sort();

    let renames = plan_renames(&files);
    let mut rewrites = Vec::new();
    for path in files.iter().filter(|p| is_text_asset(p)) {
        if let Ok(contents) = fs::read_to_string(path) {
            let rewritten = rewrite_content(&contents);
            if rewritten != contents {
                rewrites.push((path.clone(), rewritten));
            }
        }
    }

    if renames.is_empty() && rewrites.is_empty() {
        println!("Nothing to migrate.");
        return;
    }

    for (from, to) in &renames {
        println!("rename: {} -> {}", from.display(), to.display());
    }
    for (path, _) in &rewrites {
        println!("rewrite refs: {}", path.display());
    }

    if !apply {
        println!("\nDry run — pass --apply to perform the migration.");
        return;
    }

    // Rewrite contents first (paths are unchanged by content rewrites),
    // then rename files.
    for (path, contents) in &rewrites {
        if let Err(e) = fs::write(path, contents) {
            eprintln!("error: failed to rewrite {}: {}", path.display(), e);
            std::process::exit(1);
        }
    }
    for (from, to) in &renames {
        if let Err(e) = fs::rename(from, to) {
            eprintln!(
                "error: failed to rename {} -> {}: {}",
                from.display(),
                to.display(),
                e
            );
            std::process::exit(1);
        }
    }
    println!(
        "\nDone: {} file(s) renamed, {} file(s) rewritten.",
        renames.len(),
        rewrites.len()
    );
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn is_text_asset(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| TEXT_EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

fn legacy_target(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    if LEGACY_SUFFIXES.iter().any(|s| name.ends_with(s)) {
        Some(path.with_file_name(name.strip_suffix(".ron").unwrap()))
    } else {
        None
    }
}

fn plan_renames(files: &[PathBuf]) -> Vec<(PathBuf, PathBuf)> {
    files
        .iter()
        .filter_map(|f| {
            let target = legacy_target(f)?;
            if target.exists() {
                eprintln!(
                    "warning: skipping {} — target {} already exists",
                    f.display(),
                    target.display()
                );
                None
            } else {
                Some((f.clone(), target))
            }
        })
        .collect()
}

/// Strip the trailing `.ron` from legacy asset references in file contents.
/// `.mesh.ron` references are left untouched.
fn rewrite_content(contents: &str) -> String {
    let mut out = contents.to_string();
    for suffix in LEGACY_SUFFIXES {
        let replacement = suffix.strip_suffix(".ron").unwrap();
        out = out.replace(suffix, replacement);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_all_legacy_suffixes() {
        let input = r#"(
            scene: "scenes/main.scene.ron",
            material_paths: ["metal.material.ron", "red.matinst.ron"],
            prefab: "props/crate.prefab.ron",
            action: "input/Shoot.inputaction.ron",
            ctx: "input/Default.mappingcontext.ron",
        )"#;
        let out = rewrite_content(input);
        assert!(out.contains(r#""scenes/main.scene""#));
        assert!(out.contains(r#""metal.material""#));
        assert!(out.contains(r#""red.matinst""#));
        assert!(out.contains(r#""props/crate.prefab""#));
        assert!(out.contains(r#""input/Shoot.inputaction""#));
        assert!(out.contains(r#""input/Default.mappingcontext""#));
        assert!(!out.contains(".ron"));
    }

    #[test]
    fn leaves_mesh_sidecar_references_alone() {
        let input = r#"(source: "Duck.glb", sidecar: "models/Duck.mesh.ron")"#;
        assert_eq!(rewrite_content(input), input);
    }

    #[test]
    fn leaves_new_scheme_alone() {
        let input = r#"(material_paths: ["metal.material", "red.matinst"])"#;
        assert_eq!(rewrite_content(input), input);
    }

    #[test]
    fn legacy_target_strips_ron_only_for_known_types() {
        assert_eq!(
            legacy_target(Path::new("scenes/main.scene.ron")),
            Some(PathBuf::from("scenes/main.scene"))
        );
        assert_eq!(legacy_target(Path::new("models/Duck.mesh.ron")), None);
        assert_eq!(legacy_target(Path::new("Shoot.ron")), None);
        assert_eq!(legacy_target(Path::new("scenes/main.scene")), None);
    }

    #[test]
    fn plan_and_apply_roundtrip_in_tempdir() {
        let dir = std::env::temp_dir().join(format!("migrate_ext_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();

        let legacy = dir.join("metal.material.ron");
        let sidecar = dir.join("Duck.mesh.ron");
        let scene = dir.join("main.scene.ron");
        fs::write(&legacy, "MaterialDefinition(name: \"metal\")").unwrap();
        fs::write(&sidecar, "(material_path: \"metal.material.ron\")").unwrap();
        fs::write(&scene, "(material_paths: [\"metal.material.ron\"])").unwrap();

        let mut files = Vec::new();
        collect_files(&dir, &mut files);
        files.sort();

        let renames = plan_renames(&files);
        assert_eq!(renames.len(), 2); // material + scene, not the sidecar

        for path in files.iter().filter(|p| is_text_asset(p)) {
            let contents = fs::read_to_string(path).unwrap();
            let rewritten = rewrite_content(&contents);
            if rewritten != contents {
                fs::write(path, rewritten).unwrap();
            }
        }
        for (from, to) in &renames {
            fs::rename(from, to).unwrap();
        }

        assert!(dir.join("metal.material").exists());
        assert!(dir.join("main.scene").exists());
        assert!(sidecar.exists());
        assert_eq!(
            fs::read_to_string(dir.join("main.scene")).unwrap(),
            "(material_paths: [\"metal.material\"])"
        );
        assert_eq!(
            fs::read_to_string(&sidecar).unwrap(),
            "(material_path: \"metal.material\")"
        );

        fs::remove_dir_all(&dir).unwrap();
    }
}
