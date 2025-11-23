/// Complete example demonstrating all asset management features:
/// - Hot-reload
/// - Asset dependencies
/// - Async loading
/// - Cache management

use rust_engine::Renderer;
use rust_engine::assets::{AssetManager, HotReloadWatcher, AssetDependencies, AsyncAssetLoader, LoadResult};
use std::sync::Arc;
use winit::event::{Event, VirtualKeyCode, WindowEvent, ElementState};
use winit::event_loop::{ControlFlow, EventLoop};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🎮 Asset Management Demo\n");

    let event_loop = EventLoop::new();
    let window = Arc::new(
        winit::window::WindowBuilder::new()
            .with_title("Asset Management Demo")
            .with_inner_size(winit::dpi::LogicalSize::new(800, 600))
            .build(&event_loop)?,
    );

    let mut renderer = Renderer::new(window.clone())?;

    // ========== STEP 1: Create Asset Manager ==========
    println!("\n📦 Step 1: Creating Asset Manager...");
    let asset_manager = Arc::new(AssetManager::new(
        renderer.device.clone(),
        renderer.queue.clone(),
        renderer.memory_allocator.clone(),
        renderer.command_buffer_allocator.clone(),
    ));
    println!("✅ Asset Manager created");

    // ========== STEP 2: Setup Hot-Reload Watcher ==========
    println!("\n🔄 Step 2: Setting up Hot-Reload...");
    let mut hot_reload = HotReloadWatcher::new(asset_manager.clone());

    // Watch the assets directory
    hot_reload.watch_directory("assets/")?;
    println!("✅ Hot-reload watching: assets/");

    // Track specific assets for reload
    hot_reload.track_asset("assets/models/Duck.glb");
    println!("✅ Tracking Duck.glb for changes");

    // ========== STEP 3: Setup Asset Dependencies ==========
    println!("\n📎 Step 3: Setting up Asset Dependencies...");
    let dependencies = Arc::new(AssetDependencies::new());

    // Example: Track that a material depends on textures
    // (In real usage, you'd do this when creating materials)
    use rust_engine::assets::AssetId;
    let duck_texture_id = AssetId::from_path("assets/models/Duck.glb");
    let duck_material_id = AssetId::from_path("duck_material");
    dependencies.add_dependency(duck_material_id, duck_texture_id);

    println!("✅ Dependencies configured");
    println!("   Stats: {}", dependencies.stats());

    // ========== STEP 4: Setup Async Asset Loader ==========
    println!("\n⚡ Step 4: Setting up Async Asset Loader...");
    let async_loader = AsyncAssetLoader::new(asset_manager.clone());
    println!("✅ Async loader ready");

    // Queue some assets to load in background
    println!("\n📥 Queueing async loads...");
    async_loader.load_model_async("assets/models/Duck.glb");

    // ========== STEP 5: Load Assets (Synchronous) ==========
    println!("\n💾 Step 5: Loading assets synchronously...");

    // Load Duck model (first time - loads from disk)
    let duck1 = asset_manager.models.load("assets/models/Duck.glb")?;
    println!("✅ Duck model loaded (ID: {:?})", duck1.id());

    // Load Duck model again (uses cache!)
    let duck2 = asset_manager.models.load("assets/models/Duck.glb")?;
    println!("✅ Duck model loaded again (cached, same ID: {:?})", duck2.id());

    // Verify they're the same
    assert_eq!(duck1.id(), duck2.id(), "IDs should match (cached)");

    // Check cache stats
    let stats = asset_manager.cache_stats();
    println!("\n📊 Cache Stats: {}", stats);

    // ========== STEP 6: Demo Hot-Reload Instructions ==========
    println!("\n\n🎯 HOT-RELOAD DEMO:");
    println!("   1. Open assets/models/Duck.glb in Blender");
    println!("   2. Make a change and save");
    println!("   3. Watch the console - it will reload automatically!");
    println!("   (File watcher is running in background)");

    // ========== STEP 7: Demo Async Loading ==========
    println!("\n\n⚡ ASYNC LOADING DEMO:");
    println!("   Assets are loading in background...");
    println!("   Checking for completed loads each frame");

    println!("\n\n🎮 Starting game loop...");
    println!("Controls:");
    println!("  R: Reload all assets");
    println!("  C: Show cache stats");
    println!("  D: Show dependency stats");
    println!("  L: Load asset async");
    println!("  ESC: Quit\n");

    let mut frame_count = 0;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;

        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => {
                    println!("👋 Closing...");
                    *control_flow = ControlFlow::Exit;
                }

                WindowEvent::KeyboardInput { input: keyboard_input, .. } => {
                    if keyboard_input.state == ElementState::Pressed {
                        match keyboard_input.virtual_keycode {
                            Some(VirtualKeyCode::Escape) => {
                                *control_flow = ControlFlow::Exit;
                            }
                            Some(VirtualKeyCode::R) => {
                                println!("\n🔄 Reloading all assets...");
                                match asset_manager.models.reload("assets/models/Duck.glb") {
                                    Ok(_) => println!("✅ Duck reloaded"),
                                    Err(e) => eprintln!("❌ Reload failed: {}", e),
                                }
                            }
                            Some(VirtualKeyCode::C) => {
                                let stats = asset_manager.cache_stats();
                                println!("\n📊 Cache Stats: {}", stats);
                            }
                            Some(VirtualKeyCode::D) => {
                                println!("\n📎 Dependency Stats: {}", dependencies.stats());
                                let deps = dependencies.get_dependencies(duck_material_id);
                                println!("   Duck material depends on {} assets", deps.len());
                            }
                            Some(VirtualKeyCode::L) => {
                                println!("\n⚡ Loading Duck.glb asynchronously...");
                                async_loader.load_model_async("assets/models/Duck.glb");
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }

            Event::MainEventsCleared => {
                // Poll async loading results
                let results = async_loader.poll_results();
                for result in results {
                    match result {
                        LoadResult::ModelLoaded { id, success, error } => {
                            if success {
                                println!("✅ Async load complete: Model {:?}", id);
                            } else {
                                eprintln!("❌ Async load failed: {:?}", error);
                            }
                        }
                        LoadResult::TextureLoaded { id, success, error } => {
                            if success {
                                println!("✅ Async load complete: Texture {:?}", id);
                            } else {
                                eprintln!("❌ Async load failed: {:?}", error);
                            }
                        }
                    }
                }

                frame_count += 1;

                // Print status every 300 frames (about 5 seconds at 60fps)
                if frame_count % 300 == 0 {
                    let stats = asset_manager.cache_stats();
                    println!("📊 Frame {}: {}", frame_count, stats);
                }

                // Request redraw
                window.request_redraw();
            }

            Event::RedrawRequested(_) => {
                // Render here (simplified - just clear screen)
                // In real usage, you'd render with the loaded assets
            }

            _ => {}
        }
    });
}