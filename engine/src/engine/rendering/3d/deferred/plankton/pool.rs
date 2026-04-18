use std::sync::Arc;
use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer};
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PlanktonGpu {
    pub position: [f32; 3],
    pub lifetime: f32,
    pub velocity: [f32; 3],
    pub age: f32,
    pub color: [f32; 4],
    pub size: f32,
    pub seed: f32,
    pub _reserved: [f32; 2],
}

unsafe impl bytemuck::Pod for PlanktonGpu {}
unsafe impl bytemuck::Zeroable for PlanktonGpu {}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PoolCounters {
    pub alive_count: u32,
    pub dead_count: u32,
    pub emit_count: u32,
    pub _pad: u32,
}

unsafe impl bytemuck::Pod for PoolCounters {}
unsafe impl bytemuck::Zeroable for PoolCounters {}

pub struct PlanktonPool {
    pub capacity: u32,
    pub particle_buffer: Subbuffer<[PlanktonGpu]>,
    pub dead_list_buffer: Subbuffer<[u32]>,
    pub counters_buffer: Subbuffer<PoolCounters>,
    pub initialized: bool,
}

impl PlanktonPool {
    pub fn new(
        allocator: Arc<StandardMemoryAllocator>,
        capacity: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let particle_buffer = Buffer::new_slice::<PlanktonGpu>(
            allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::STORAGE_BUFFER | BufferUsage::TRANSFER_DST,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
                ..Default::default()
            },
            capacity as u64,
        )?;

        let dead_list_buffer = Buffer::new_slice::<u32>(
            allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::STORAGE_BUFFER | BufferUsage::TRANSFER_DST,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
                ..Default::default()
            },
            capacity as u64,
        )?;

        let counters_buffer = Buffer::from_data(
            allocator,
            BufferCreateInfo {
                usage: BufferUsage::STORAGE_BUFFER | BufferUsage::TRANSFER_DST,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            PoolCounters {
                alive_count: 0,
                dead_count: 0,
                emit_count: 0,
                _pad: 0,
            },
        )?;

        Ok(Self {
            capacity,
            particle_buffer,
            dead_list_buffer,
            counters_buffer,
            initialized: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn plankton_gpu_size() {
        assert_eq!(size_of::<PlanktonGpu>(), 64);
    }

    #[test]
    fn pool_counters_size() {
        assert_eq!(size_of::<PoolCounters>(), 16);
    }
}
