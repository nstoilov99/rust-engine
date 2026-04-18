//! 3D curl-noise texture generation for plankton turbulence.

use std::sync::Arc;
use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage};
use vulkano::command_buffer::{
    allocator::StandardCommandBufferAllocator, AutoCommandBufferBuilder, CommandBufferUsage,
    CopyBufferToImageInfo,
};
use vulkano::device::Queue;
use vulkano::format::Format;
use vulkano::image::view::ImageView;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use vulkano::sync::GpuFuture;

const NOISE_SIZE: u32 = 64;

/// Generate a 64³ R16G16B16A16_SFLOAT 3D curl-noise texture.
pub fn generate_curl_noise_texture(
    allocator: Arc<StandardMemoryAllocator>,
    command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    queue: Arc<Queue>,
) -> Result<Arc<ImageView>, Box<dyn std::error::Error>> {
    let size = NOISE_SIZE as usize;
    let total = size * size * size;
    let mut data: Vec<[F16Placeholder; 4]> = Vec::with_capacity(total);

    for z in 0..size {
        for y in 0..size {
            for x in 0..size {
                let fx = x as f32 / size as f32;
                let fy = y as f32 / size as f32;
                let fz = z as f32 / size as f32;

                // Compute curl of a 3D noise field
                // curl(F) = (dFz/dy - dFy/dz, dFx/dz - dFz/dx, dFy/dx - dFx/dy)
                let eps = 1.0 / size as f32;

                let curl_x = (value_noise_3d(fx, fy + eps, fz) - value_noise_3d(fx, fy - eps, fz))
                    - (value_noise_3d(fx, fy, fz + eps) - value_noise_3d(fx, fy, fz - eps));
                let curl_y = (value_noise_3d(fx, fy, fz + eps) - value_noise_3d(fx, fy, fz - eps))
                    - (value_noise_3d(fx + eps, fy, fz) - value_noise_3d(fx - eps, fy, fz));
                let curl_z = (value_noise_3d(fx + eps, fy, fz) - value_noise_3d(fx - eps, fy, fz))
                    - (value_noise_3d(fx, fy + eps, fz) - value_noise_3d(fx, fy - eps, fz));

                // Normalize and pack into [0, 1] range (shader unpacks with * 2 - 1)
                let len = (curl_x * curl_x + curl_y * curl_y + curl_z * curl_z).sqrt().max(0.001);
                let nx = (curl_x / len) * 0.5 + 0.5;
                let ny = (curl_y / len) * 0.5 + 0.5;
                let nz = (curl_z / len) * 0.5 + 0.5;

                data.push([
                    f32_to_f16(nx),
                    f32_to_f16(ny),
                    f32_to_f16(nz),
                    f32_to_f16(0.0),
                ]);
            }
        }
    }

    // Create the 3D image
    let image = Image::new(
        allocator.clone(),
        ImageCreateInfo {
            image_type: ImageType::Dim3d,
            format: Format::R16G16B16A16_SFLOAT,
            extent: [NOISE_SIZE, NOISE_SIZE, NOISE_SIZE],
            usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
            ..Default::default()
        },
    )?;

    // Upload via staging buffer
    let byte_data: &[u8] = bytemuck::cast_slice(&data);
    let staging_buffer = Buffer::from_iter(
        allocator,
        BufferCreateInfo {
            usage: BufferUsage::TRANSFER_SRC,
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::PREFER_HOST
                | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
            ..Default::default()
        },
        byte_data.iter().copied(),
    )?;

    let mut builder = AutoCommandBufferBuilder::primary(
        command_buffer_allocator,
        queue.queue_family_index(),
        CommandBufferUsage::OneTimeSubmit,
    )?;

    builder.copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(
        staging_buffer,
        image.clone(),
    ))?;

    let command_buffer = builder.build()?;
    let future = vulkano::sync::now(queue.device().clone())
        .then_execute(queue, command_buffer)?
        .then_signal_fence_and_flush()?;
    future.wait(None)?;

    ImageView::new_default(image).map_err(|e| e.into())
}

// ---- Minimal value noise implementation ----

/// Simple 3D value noise using hash-based lattice interpolation.
fn value_noise_3d(x: f32, y: f32, z: f32) -> f32 {
    // Scale for multi-octave look
    let x = x * 4.0;
    let y = y * 4.0;
    let z = z * 4.0;

    let ix = x.floor() as i32;
    let iy = y.floor() as i32;
    let iz = z.floor() as i32;

    let fx = x - x.floor();
    let fy = y - y.floor();
    let fz = z - z.floor();

    // Smoothstep
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let sz = fz * fz * (3.0 - 2.0 * fz);

    // Trilinear interpolation of hashed corners
    let n000 = hash_float(ix, iy, iz);
    let n100 = hash_float(ix + 1, iy, iz);
    let n010 = hash_float(ix, iy + 1, iz);
    let n110 = hash_float(ix + 1, iy + 1, iz);
    let n001 = hash_float(ix, iy, iz + 1);
    let n101 = hash_float(ix + 1, iy, iz + 1);
    let n011 = hash_float(ix, iy + 1, iz + 1);
    let n111 = hash_float(ix + 1, iy + 1, iz + 1);

    let nx00 = lerp(n000, n100, sx);
    let nx10 = lerp(n010, n110, sx);
    let nx01 = lerp(n001, n101, sx);
    let nx11 = lerp(n011, n111, sx);

    let nxy0 = lerp(nx00, nx10, sy);
    let nxy1 = lerp(nx01, nx11, sy);

    lerp(nxy0, nxy1, sz)
}

fn hash_float(x: i32, y: i32, z: i32) -> f32 {
    let h = hash_int(x.wrapping_mul(374761393)
        .wrapping_add(y.wrapping_mul(668265263))
        .wrapping_add(z.wrapping_mul(1274126177)));
    (h as u32 as f32) / (u32::MAX as f32)
}

fn hash_int(x: i32) -> i32 {
    let x = x as u32;
    let x = ((x >> 16) ^ x).wrapping_mul(0x45d9f3b);
    let x = ((x >> 16) ^ x).wrapping_mul(0x45d9f3b);
    let x = (x >> 16) ^ x;
    x as i32
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// ---- f16 conversion (IEEE 754 half-precision) ----

type F16Placeholder = u16;

fn f32_to_f16(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = (bits >> 16) & 0x8000;
    let exponent = ((bits >> 23) & 0xFF) as i32;
    let mantissa = bits & 0x007FFFFF;

    if exponent == 255 {
        // Inf/NaN
        return (sign | 0x7C00 | if mantissa != 0 { 1 } else { 0 }) as u16;
    }

    let exp_adj = exponent - 127 + 15;

    if exp_adj >= 31 {
        return (sign | 0x7C00) as u16; // overflow → inf
    }

    if exp_adj <= 0 {
        if exp_adj < -10 {
            return sign as u16; // too small → zero
        }
        let mantissa = (mantissa | 0x00800000) >> (1 - exp_adj);
        return (sign | (mantissa >> 13)) as u16;
    }

    (sign | ((exp_adj as u32) << 10) | (mantissa >> 13)) as u16
}
