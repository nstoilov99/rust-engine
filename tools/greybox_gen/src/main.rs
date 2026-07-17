//! Greybox world generator CLI (M3).
//!
//! Usage:
//!   greybox_gen [content_root] [--check]
//!
//! Default: write all generated content (terrain cell meshes, scene, world
//! manifest) into the content root. `--check`: regenerate in memory and
//! byte-compare against the checked-in files — non-zero exit on any drift
//! (CI + export precondition; generated content is checked in, never rebuilt
//! by packaging). Remember to run the collision cook after regenerating.

use std::path::PathBuf;

use greybox_gen::outputs::generate_all;
use greybox_gen::params::{GENERATOR_VERSION, SEED};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let check = args.iter().any(|a| a == "--check");
    args.retain(|a| a != "--check");
    let content_root = PathBuf::from(args.first().map(String::as_str).unwrap_or("content"));
    if !content_root.is_dir() {
        eprintln!(
            "Error: content root '{}' is not a directory",
            content_root.display()
        );
        std::process::exit(1);
    }

    let outputs = generate_all();
    println!(
        "greybox_gen v{GENERATOR_VERSION} (seed {SEED:#010x}): {} output file(s)",
        outputs.len()
    );

    if check {
        let mut drifted = 0usize;
        for (rel, bytes) in &outputs {
            let path = content_root.join(rel);
            match std::fs::read(&path) {
                Ok(on_disk) if &on_disk == bytes => {}
                Ok(_) => {
                    eprintln!("DRIFT: '{rel}' differs from generated output");
                    drifted += 1;
                }
                Err(e) => {
                    eprintln!("DRIFT: '{rel}' unreadable: {e}");
                    drifted += 1;
                }
            }
        }
        if drifted > 0 {
            eprintln!("{drifted} file(s) drifted — rerun greybox_gen and re-cook collision");
            std::process::exit(1);
        }
        println!("check OK: all generated content matches");
        return;
    }

    let mut written = 0usize;
    for (rel, bytes) in &outputs {
        let path = content_root.join(rel);
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("Error creating '{}': {e}", parent.display());
                std::process::exit(1);
            }
        }
        if let Err(e) = std::fs::write(&path, bytes) {
            eprintln!("Error writing '{}': {e}", path.display());
            std::process::exit(1);
        }
        written += 1;
    }
    println!(
        "Wrote {written} file(s) -> {} (now cook: collision_cooker scenes/greybox.scene)",
        content_root.display()
    );
}
