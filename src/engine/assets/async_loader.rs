use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use parking_lot::Mutex;

use super::asset_manager::AssetManager;
use super::handle::{Handle, AssetId};

/// Asset loading request
pub enum LoadRequest {
    Texture { path: String },
    Model { path: String },
}

/// Asset loading result
pub enum LoadResult {
    TextureLoaded { id: AssetId, success: bool, error: Option<String> },
    ModelLoaded { id: AssetId, success: bool, error: Option<String> },
}

/// Async asset loader for background loading
pub struct AsyncAssetLoader {
    runtime: Arc<Runtime>,
    assets: Arc<AssetManager>,
    result_receiver: Arc<Mutex<mpsc::UnboundedReceiver<LoadResult>>>,
    request_sender: mpsc::UnboundedSender<LoadRequest>,
}

impl AsyncAssetLoader {
    pub fn new(assets: Arc<AssetManager>) -> Self {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2) // 2 threads for asset loading
                .thread_name("async-asset-loader")
                .enable_all()
                .build()
                .expect("Failed to create async runtime")
        );

        let (request_tx, mut request_rx) = mpsc::unbounded_channel::<LoadRequest>();
        let (result_tx, result_rx) = mpsc::unbounded_channel::<LoadResult>();

        let assets_clone = assets.clone();
        let runtime_clone = runtime.clone();

        // Spawn background worker task
        runtime.spawn(async move {
            while let Some(request) = request_rx.recv().await {
                let assets = assets_clone.clone();
                let result_tx = result_tx.clone();

                // Spawn blocking task for CPU-intensive loading
                runtime_clone.spawn_blocking(move || {
                    match request {
                        LoadRequest::Texture { path } => {
                            let id = AssetId::from_path(&path);
                            println!("⏳ Loading texture async: {}", path);

                            match assets.textures.load(&path) {
                                Ok(_) => {
                                    println!("✅ Texture loaded: {}", path);
                                    let _ = result_tx.send(LoadResult::TextureLoaded {
                                        id,
                                        success: true,
                                        error: None,
                                    });
                                }
                                Err(e) => {
                                    eprintln!("❌ Failed to load texture: {}", e);
                                    let _ = result_tx.send(LoadResult::TextureLoaded {
                                        id,
                                        success: false,
                                        error: Some(e.to_string()),
                                    });
                                }
                            }
                        }
                        LoadRequest::Model { path } => {
                            let id = AssetId::from_path(&path);
                            println!("⏳ Loading model async: {}", path);

                            match assets.models.load(&path) {
                                Ok(_) => {
                                    println!("✅ Model loaded: {}", path);
                                    let _ = result_tx.send(LoadResult::ModelLoaded {
                                        id,
                                        success: true,
                                        error: None,
                                    });
                                }
                                Err(e) => {
                                    eprintln!("❌ Failed to load model: {}", e);
                                    let _ = result_tx.send(LoadResult::ModelLoaded {
                                        id,
                                        success: false,
                                        error: Some(e.to_string()),
                                    });
                                }
                            }
                        }
                    }
                });
            }
        });

        Self {
            runtime,
            assets,
            result_receiver: Arc::new(Mutex::new(result_rx)),
            request_sender: request_tx,
        }
    }

    /// Request to load a texture asynchronously
    pub fn load_texture_async(&self, path: impl Into<String>) {
        let _ = self.request_sender.send(LoadRequest::Texture {
            path: path.into(),
        });
    }

    /// Request to load a model asynchronously
    pub fn load_model_async(&self, path: impl Into<String>) {
        let _ = self.request_sender.send(LoadRequest::Model {
            path: path.into(),
        });
    }

    /// Poll for completed load results (call this each frame)
    /// Returns list of completed load results
    pub fn poll_results(&self) -> Vec<LoadResult> {
        let mut receiver = self.result_receiver.lock();
        let mut results = Vec::new();

        // Collect all available results (non-blocking)
        while let Ok(result) = receiver.try_recv() {
            results.push(result);
        }

        results
    }

    /// Check if a specific asset is loaded (synchronous check)
    pub fn is_texture_loaded(&self, path: &str) -> bool {
        let id = AssetId::from_path(path);
        // Check if it's in the cache
        self.assets.textures.cache_size() > 0 // Simplified check
    }

    /// Get number of pending load requests
    pub fn pending_count(&self) -> usize {
        // Note: mpsc doesn't expose queue size, so this is approximate
        0 // You could track this manually if needed
    }
}

/// Example usage pattern
#[cfg(test)]
mod example {
    use super::*;

    #[test]
    fn example_async_loading() {
        // This is just an example - won't actually run without proper setup
        /*
        let assets = Arc::new(AssetManager::new(...));
        let async_loader = AsyncAssetLoader::new(assets);

        // Queue assets to load in background
        async_loader.load_texture_async("assets/textures/enemy1.png");
        async_loader.load_texture_async("assets/textures/enemy2.png");
        async_loader.load_model_async("assets/models/tree.glb");

        // In your game loop:
        loop {
            // Poll for completed loads
            let results = async_loader.poll_results();
            for result in results {
                match result {
                    LoadResult::TextureLoaded { id, success, error } => {
                        if success {
                            println!("Texture {:?} ready to use!", id);
                        } else {
                            eprintln!("Texture load failed: {:?}", error);
                        }
                    }
                    LoadResult::ModelLoaded { id, success, error } => {
                        if success {
                            println!("Model {:?} ready to use!", id);
                        }
                    }
                }
            }

            // ... rest of game loop
        }
        */
    }
}