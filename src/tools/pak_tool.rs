//! CLI tool for packing/inspecting .pak asset archives.
//!
//! Usage:
//!   pak_tool pack <content_dir> <output.pak>
//!   pak_tool list <file.pak>

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    match args[1].as_str() {
        "pack" => {
            if args.len() < 4 {
                eprintln!("Usage: pak_tool pack <content_dir> <output.pak>");
                std::process::exit(1);
            }
            let content_dir = Path::new(&args[2]);
            let output_path = Path::new(&args[3]);

            if !content_dir.is_dir() {
                eprintln!("Error: '{}' is not a directory", content_dir.display());
                std::process::exit(1);
            }

            if let Some(parent) = output_path.parent() {
                if !parent.exists() {
                    std::fs::create_dir_all(parent).unwrap();
                }
            }

            match rust_engine::engine::assets::pak::pack_directory(content_dir, output_path) {
                Ok(size) => {
                    println!(
                        "Packed '{}' -> '{}' ({:.1} MB)",
                        content_dir.display(),
                        output_path.display(),
                        size as f64 / (1024.0 * 1024.0)
                    );
                }
                Err(e) => {
                    eprintln!("Error packing: {}", e);
                    std::process::exit(1);
                }
            }
        }
        "list" => {
            if args.len() < 3 {
                eprintln!("Usage: pak_tool list <file.pak>");
                std::process::exit(1);
            }
            let pak_path = Path::new(&args[2]);
            match rust_engine::engine::assets::pak::PakReader::open(pak_path) {
                Ok(reader) => {
                    let mut files = reader.list_files();
                    files.sort();
                    println!("{} files in '{}':", files.len(), pak_path.display());
                    for f in &files {
                        println!("  {}", f);
                    }
                }
                Err(e) => {
                    eprintln!("Error opening pak: {}", e);
                    std::process::exit(1);
                }
            }
        }
        _ => {
            print_usage();
            std::process::exit(1);
        }
    }
}

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  pak_tool pack <content_dir> <output.pak>");
    eprintln!("  pak_tool list <file.pak>");
}
