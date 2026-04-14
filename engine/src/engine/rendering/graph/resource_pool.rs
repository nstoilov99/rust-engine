use super::resource::ResourceDesc;
use std::sync::Arc;
use vulkano::image::view::ImageView;

pub type AllocateFn<'a> =
    &'a mut dyn FnMut(&ResourceDesc) -> Result<Arc<ImageView>, Box<dyn std::error::Error>>;

#[derive(Default)]
pub struct TransientResourcePool {
    pool: Vec<(ResourceDesc, Arc<ImageView>, bool)>,
}

impl TransientResourcePool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allocate(
        &mut self,
        desc: &ResourceDesc,
        create_fn: AllocateFn<'_>,
    ) -> Result<Arc<ImageView>, Box<dyn std::error::Error>> {
        for entry in &mut self.pool {
            if !entry.2 && entry.0 == *desc {
                entry.2 = true;
                return Ok(entry.1.clone());
            }
        }

        let image = create_fn(desc)?;
        self.pool.push((desc.clone(), image.clone(), true));
        Ok(image)
    }

    pub fn reset(&mut self) {
        for entry in &mut self.pool {
            entry.2 = false;
        }
    }

    pub fn pool_size(&self) -> usize {
        self.pool.len()
    }
}
