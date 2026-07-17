//! Headless static-collision cooker CLI (thin wrapper over
//! `rust_engine::engine::collision::cook`).
//!
//! Usage:
//!   collision_cooker <scene_relative> [content_root]
//!
//! `scene_relative` is content-relative (e.g. `scenes/main.scene`);
//! `content_root` defaults to `content`. Output replaces
//! `<content_root>/collision/<scene_stem>/`.

use std::path::PathBuf;

use rust_engine::engine::assets::asset_source;
use rust_engine::engine::collision::{cook, output};
use rust_engine::engine::ecs::World;
use rust_engine::engine::scene::scene_serializer;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: collision_cooker <scene_relative> [content_root]");
        eprintln!("  e.g. collision_cooker scenes/main.scene content");
        std::process::exit(1);
    }
    let scene_relative = &args[1];
    let content_root = PathBuf::from(args.get(2).map(String::as_str).unwrap_or("content"));
    if !content_root.is_dir() {
        eprintln!(
            "Error: content root '{}' is not a directory",
            content_root.display()
        );
        std::process::exit(1);
    }
    asset_source::init_filesystem(content_root);

    let mut world = World::new();
    if let Err(e) = scene_serializer::load_scene(&mut world, scene_relative) {
        eprintln!("Error loading scene '{scene_relative}': {e}");
        std::process::exit(1);
    }

    let stem = output::scene_stem(scene_relative);
    let mut loader = output::load_model_from_content;
    let cooked = cook::cook_scene(&world, &stem, &mut loader);
    for warning in &cooked.warnings {
        eprintln!("warning: {warning}");
    }

    let Some(dir) = output::collision_dir_for_scene(scene_relative) else {
        eprintln!("Error: no content root initialized");
        std::process::exit(1);
    };
    if let Err(e) = output::write_cooked_scene(&dir, &cooked) {
        eprintln!("Error writing cooked output to '{}': {e}", dir.display());
        std::process::exit(1);
    }

    let total: usize = cooked.chunks.iter().map(|(_, b)| b.len()).sum();
    println!(
        "Cooked '{stem}': {} chunk(s), {:.1} KiB -> {}",
        cooked.chunks.len(),
        total as f64 / 1024.0,
        dir.display()
    );
}
