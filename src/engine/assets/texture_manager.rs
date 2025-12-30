use std::sync::Arc;
use std::collections::HashMap;
use parking_lot::RwLock;
use vulkano::device::{Device, Queue};
use vulkano::image::view::ImageView;
use vulkano::memory::allocator::StandardMemoryAllocator;
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::format::Format;

use super::handle::{Handle, AssetId};

/// Manages texture loading and caching
pub struct TextureManager {
    device: Arc<Device>,
    queue: Arc<Queue>,
    allocator: Arc<StandardMemoryAllocator>,
    command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    cache: RwLock<HashMap<AssetId, Arc<ImageView>>>,
}

impl TextureManager {
    pub fn new(
        device: Arc<Device>,
        queue: Arc<Queue>,
        allocator: Arc<StandardMemoryAllocator>,
        command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    ) -> Self {
        Self {
            device,
            queue,
            allocator,
            command_buffer_allocator,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Load texture from file path (caches result)
    pub fn load(&self, path: &str) -> Result<Handle<ImageView>, Box<dyn std::error::Error>> {
        let id = AssetId::from_path(path);

        // Check cache first (read lock)
        {
            let cache = self.cache.read();
            if let Some(texture) = cache.get(&id) {
                return Ok(Handle::new(id, texture.clone()));
            }
        }

        // Not in cache - load it (write lock)
        let mut cache = self.cache.write();

        // Double-check (another thread might have loaded it)
        if let Some(texture) = cache.get(&id) {
            return Ok(Handle::new(id, texture.clone()));
        }

        // Load texture from disk
        println!("Loading texture: {}", path);
        let texture = self.load_texture_from_disk(path)?;

        // Insert into cache
        cache.insert(id, texture.clone());

        Ok(Handle::new(id, texture))
    }

    /// Reload texture from disk (updates cache)
    pub fn reload(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let id = AssetId::from_path(path);

        println!("Reloading texture: {}", path);
        let texture = self.load_texture_from_disk(path)?;

        let mut cache = self.cache.write();
        cache.insert(id, texture);

        Ok(())
    }

    /// Clear all cached textures
    pub fn clear_cache(&self) {
        let mut cache = self.cache.write();
        cache.clear();
    }

    /// Get number of cached textures
    pub fn cache_size(&self) -> usize {
        self.cache.read().len()
    }

    /// Internal: Load texture from disk
    fn load_texture_from_disk(&self, path: &str) -> Result<Arc<ImageView>, Box<dyn std::error::Error>> {
        use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
        use vulkano::memory::allocator::AllocationCreateInfo;
        use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage};
        use vulkano::memory::allocator::MemoryTypeFilter;
        use vulkano::command_buffer::{AutoCommandBufferBuilder, CommandBufferUsage, CopyBufferToImageInfo, PrimaryCommandBufferAbstract};
        use vulkano::sync::GpuFuture;

        // Load image file
        let img = image::open(path)?;
        let img_rgba = img.to_rgba8();
        let (width, height) = img_rgba.dimensions();

        // Create GPU image
        let image = Image::new(
            self.allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R8G8B8A8_SRGB,
                extent: [width, height, 1],
                usage: ImageUsage::TRANSFER_DST | ImageUsage::SAMPLED,
                ..Default::default()
            },
            AllocationCreateInfo::default(),
        )?;

        // Upload to GPU
        let buffer = Buffer::from_iter(
            self.allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::TRANSFER_SRC,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            img_rgba.into_raw(),
        )?;

        let mut builder = AutoCommandBufferBuilder::primary(
            self.command_buffer_allocator.clone(),
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;

        builder.copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(
            buffer,
            image.clone(),
        ))?;

        let command_buffer = builder.build()?;
        command_buffer.execute(self.queue.clone())?
            .then_signal_fence_and_flush()?
            .wait(None)?;

        let view = ImageView::new_default(image)?;
        Ok(view)
    }
}