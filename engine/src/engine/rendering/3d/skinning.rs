//! Skinning backend: bone-palette SSBO ring (Task 41.5 P1, the `LargeSsbo`
//! backend the old FixedUbo comments anticipated).
//!
//! One large storage buffer holds every skeleton's palette back-to-back.
//! It is split into [`PALETTE_RING_REGIONS`] independently-written regions.
//! Region reuse is gated on the renderer's 3-slot fence ring: the main thread
//! writes frame N's palettes into region `N % 4` after the render thread has
//! reclaimed the fence of frame N-4 — published through [`PaletteRingSync`]
//! (marked when a fence-ring slot is reclaimed, or immediately when a frame
//! is consumed without GPU work). [`SkinningBackend::begin_frame`] blocks on
//! that marker.
//!
//! Why 4 regions against a 3-slot fence ring (P1 ruling): the renderer
//! reclaims a fence-ring slot *lazily* — frame N-3's fence is taken at the
//! start of processing frame N's packet. With only 3 regions the main thread
//! would have to wait for that reclaim before it could build (and send)
//! frame N — a producer/consumer deadlock. One region of slack matches the
//! actual reclaim point: frame N needs frame N-4 done, which the renderer
//! publishes while processing frame N-1 (already sent). Every index is
//! derived from the packet's `frame_number` (regions `% 4`, fence slots
//! `% 3`) — there is no second ring counter to drift.
//!
//! Draws address palettes with a flat `palette_base` index into the frame's
//! region — no dynamic offsets, no per-entity descriptor sets. Element 0 of
//! every region is the identity matrix, so static meshes use
//! `palette_base = 0`.
//!
//! A second ring buffer with the same region/sync discipline holds per-draw
//! [`InstanceData`] (Task 41.5 P7): batched draws address it by
//! `gl_InstanceIndex`, which carries `model` + `palette_base` per instance —
//! push constants dropped entirely. Frames are done atomically, so the one
//! [`PaletteRingSync`] handshake gates both buffers; the instance buffer is a
//! parallel cursor in this backend rather than a generic ring allocator
//! because the palette side carries residency (upload-gate) state the
//! instance side has no use for — the shared parts are two small helpers.
//!
//! Growth: if a frame needs more elements than a region holds, a new (larger)
//! buffer is allocated on the spot and the current frame's writes are copied
//! over. In-flight frames keep the old buffer alive through their descriptor
//! sets (Arc), so growth needs no fence wait of its own — but the ring wait
//! keeps running afterwards: with two ring buffers gated by one handshake, a
//! "fresh regions" wait-skip would only be sound if *both* buffers were just
//! replaced, and the steady-state wait is free (frame N-4 is long reclaimed).

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use glam::Mat4;
use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer};
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::layout::DescriptorSetLayout;
use vulkano::descriptor_set::{DescriptorSet, WriteDescriptorSet};
use vulkano::device::DeviceOwned;
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use vulkano::DeviceSize;

/// Fence-ring slots (the render thread's ring) — release markers are keyed
/// by `frame_number % PALETTE_RING_SLOTS`.
pub const PALETTE_RING_SLOTS: usize = 3;

/// Palette ring regions — one more than the fence ring because fence slots
/// are reclaimed lazily (see module docs). Regions are indexed by
/// `frame_number % PALETTE_RING_REGIONS`.
pub const PALETTE_RING_REGIONS: usize = 4;

/// Bytes per palette matrix (column-major mat4).
const MAT_BYTES: DeviceSize = 64;

/// Initial region capacity in matrices (before alignment rounding).
const INITIAL_REGION_MATS: DeviceSize = 256;

/// Initial instance-region capacity in instances (before alignment rounding).
const INITIAL_REGION_INSTANCES: DeviceSize = 256;

const IDENTITY_MAT: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// Per-draw-instance metadata for instanced skinned draws (Task 41.5 P7),
/// addressed by `gl_InstanceIndex` in `gbuffer.vert` / `shadow_vs.glsl`.
///
/// std430 layout — must match the shader-side `InstanceData` struct exactly:
/// `mat4 model` (64 B, offset 0) + `uint palette_base` (4 B, offset 64) +
/// 12 B explicit padding = **80 B stride** (std430 rounds the struct to the
/// mat4's 16-byte alignment).
#[derive(Clone, Copy, vulkano::buffer::BufferContents)]
#[repr(C)]
pub struct InstanceData {
    /// Model (world) matrix, column-major.
    pub model: [[f32; 4]; 4],
    /// Flat index of this instance's palette in the frame's palette region
    /// (0 = identity, static meshes).
    pub palette_base: u32,
    /// Explicit std430 tail padding.
    pub _pad: [u32; 3],
}

/// Cross-thread release markers for the palette ring.
///
/// `state[slot]` holds `seq + 1` of the newest frame on that slot whose GPU
/// work is provably finished (0 = none yet). The render thread publishes via
/// [`mark_done`](Self::mark_done); the main thread blocks in
/// [`wait_done`](Self::wait_done) before rewriting a region.
pub struct PaletteRingSync {
    state: Mutex<[u64; PALETTE_RING_SLOTS]>,
    cv: Condvar,
}

impl Default for PaletteRingSync {
    fn default() -> Self {
        Self::new()
    }
}

impl PaletteRingSync {
    pub fn new() -> Self {
        Self {
            state: Mutex::new([0; PALETTE_RING_SLOTS]),
            cv: Condvar::new(),
        }
    }

    /// Frame `seq` (which used region `slot`) is finished on the GPU — its
    /// region may be rewritten.
    pub fn mark_done(&self, slot: usize, seq: u64) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state[slot] < seq + 1 {
            state[slot] = seq + 1;
            self.cv.notify_all();
        }
    }

    /// Block until frame `seq` on `slot` has been marked done. Returns
    /// `false` on timeout (render thread stalled or gone).
    pub fn wait_done(&self, slot: usize, seq: u64, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while state[slot] < seq + 1 {
            let now = std::time::Instant::now();
            if now >= deadline {
                return false;
            }
            let (guard, _) = self
                .cv
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = guard;
        }
        true
    }
}

/// One frame's view of the skinning rings, handed to the renderer: the
/// palette region slice (bound whole at set 0 / binding 0), the instance
/// metadata region slice (set 0 / binding 2) and the region index the frame
/// occupies (`frame_number % PALETTE_RING_REGIONS`) — also the renderer's
/// index into its per-slot VP UBOs / set cache.
#[derive(Clone)]
pub struct SkinnedPaletteFrame {
    pub region: Subbuffer<[[f32; 16]]>,
    pub instances: Subbuffer<[InstanceData]>,
    pub slot: usize,
}

/// What one ring region currently holds, in frame write order: one entry per
/// `write_palette` call — `(entity key, matrix count, palette revision)`.
///
/// The Task 41.5 P4 upload gate: a write may skip its memcpy when this
/// region already holds the same palette revision at the same base. Bases
/// are made stable the cheap way — the cursor **always** advances (space is
/// reserved whether or not the copy happens), so an entity's base repeats
/// exactly when the frame's write sequence prefix repeats. Each visit
/// compares its writes against `entries` in order; the first divergence
/// (entity appeared/vanished/reordered, bone count changed) flips
/// `prefix_ok` and every later write this visit copies and rewrites its
/// entry. Data behind a still-matching prefix is untouched since this
/// region's last visit (writes are the only mutation, spans are disjoint and
/// sequential), so a revision match there means the bytes are already right.
#[derive(Default)]
struct RegionResidency {
    entries: Vec<(u64, u32, u64)>,
    /// Writes so far this visit (index into `entries`).
    cursor: usize,
    /// This visit's write sequence has matched `entries` so far.
    prefix_ok: bool,
}

/// Main-thread palette writer over the SSBO ring.
pub struct SkinningBackend {
    allocator: Arc<StandardMemoryAllocator>,
    sync: Arc<PaletteRingSync>,
    buffer: Subbuffer<[[f32; 16]]>,
    /// Region capacity in matrices; region byte size is a multiple of
    /// `minStorageBufferOffsetAlignment` so every region offset is aligned.
    region_capacity: DeviceSize,
    /// Alignment expressed in matrices (>= 1).
    align_mats: DeviceSize,
    /// Instance-metadata ring (P7): same 4 regions, same sync markers, own
    /// cursor/capacity/growth. No residency — instance data (models) changes
    /// every frame, so every write copies.
    inst_buffer: Subbuffer<[InstanceData]>,
    /// Instance-region capacity in instances; region byte offsets are kept
    /// multiples of `minStorageBufferOffsetAlignment`.
    inst_capacity: DeviceSize,
    /// Instance-capacity granularity (>= 1) that keeps region offsets aligned.
    inst_align: DeviceSize,
    /// Instances written into the current region.
    inst_cursor: DeviceSize,
    /// First frame sequence of the process — the first 4 frames skip the
    /// ring wait (no prior occupant). Not reset on growth: with two ring
    /// buffers behind one handshake the skip would only be sound if both
    /// were just replaced (see module docs).
    epoch_start_seq: u64,
    cur_seq: u64,
    cur_slot: usize,
    /// Matrices written into the current region (element 0 is the identity).
    cursor: DeviceSize,
    /// Per-region contents for the upload gate, index-aligned with the ring
    /// regions (see [`RegionResidency`]).
    residency: [RegionResidency; PALETTE_RING_REGIONS],
    warned_wait: bool,
}

impl SkinningBackend {
    pub fn new(
        allocator: Arc<StandardMemoryAllocator>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let align: DeviceSize = allocator
            .device()
            .physical_device()
            .properties()
            .min_storage_buffer_offset_alignment
            .as_devicesize();
        let align_mats = align.div_ceil(MAT_BYTES).max(1);
        let region_capacity = round_up(INITIAL_REGION_MATS, align_mats);
        let buffer = Self::alloc_buffer(&allocator, region_capacity)?;

        // Instance stride is 80 B (not a divisor of the alignment), so the
        // capacity granularity that keeps region byte offsets aligned is
        // `align / gcd(stride, align)` instances.
        let inst_stride = std::mem::size_of::<InstanceData>() as DeviceSize;
        let inst_align = (align / gcd(inst_stride, align)).max(1);
        let inst_capacity = round_up(INITIAL_REGION_INSTANCES, inst_align);
        let inst_buffer = Self::alloc_inst_buffer(&allocator, inst_capacity)?;

        Ok(Self {
            allocator,
            sync: Arc::new(PaletteRingSync::new()),
            buffer,
            region_capacity,
            align_mats,
            inst_buffer,
            inst_capacity,
            inst_align,
            inst_cursor: 0,
            epoch_start_seq: 0,
            cur_seq: 0,
            cur_slot: 0,
            cursor: 1,
            residency: Default::default(),
            warned_wait: false,
        })
    }

    /// The release-marker handshake shared with the render thread
    /// (pass a clone into `RenderThreadConfig`).
    pub fn sync(&self) -> &Arc<PaletteRingSync> {
        &self.sync
    }

    /// Allocate a ring buffer of `4 * capacity` matrices and write the
    /// identity matrix into element 0 of each region.
    fn alloc_buffer(
        allocator: &Arc<StandardMemoryAllocator>,
        capacity: DeviceSize,
    ) -> Result<Subbuffer<[[f32; 16]]>, Box<dyn std::error::Error>> {
        let buffer = Buffer::new_slice::<[f32; 16]>(
            allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::STORAGE_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            capacity * PALETTE_RING_REGIONS as DeviceSize,
        )?;
        for slot in 0..PALETTE_RING_REGIONS as DeviceSize {
            let ident = buffer.clone().slice(slot * capacity..slot * capacity + 1);
            let mut guard = ident.write()?;
            guard[0] = IDENTITY_MAT;
        }
        Ok(buffer)
    }

    /// Allocate an instance-metadata ring buffer of `4 * capacity` instances.
    /// No identity element — every draw's instances are written each frame.
    fn alloc_inst_buffer(
        allocator: &Arc<StandardMemoryAllocator>,
        capacity: DeviceSize,
    ) -> Result<Subbuffer<[InstanceData]>, Box<dyn std::error::Error>> {
        Ok(Buffer::new_slice::<InstanceData>(
            allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::STORAGE_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            capacity * PALETTE_RING_REGIONS as DeviceSize,
        )?)
    }

    /// Open frame `seq` (the packet's `frame_number`): claims region
    /// `seq % 4` of both rings (palette + instance metadata), blocking until
    /// frame `seq - 4` — the region's previous occupant — has been marked
    /// done (its fence reclaimed, or it never submitted GPU work).
    pub fn begin_frame(&mut self, seq: u64) {
        let slot = (seq % PALETTE_RING_REGIONS as u64) as usize;
        if seq >= self.epoch_start_seq + PALETTE_RING_REGIONS as u64 {
            let gate = seq - PALETTE_RING_REGIONS as u64;
            let gate_fence_slot = (gate % PALETTE_RING_SLOTS as u64) as usize;
            if !self
                .sync
                .wait_done(gate_fence_slot, gate, Duration::from_secs(2))
            {
                // Anti-deadlock escape hatch: writing the region anyway can
                // race a merely-stalled (not dead) render thread's GPU reads.
                if !self.warned_wait {
                    eprintln!(
                        "skinning: palette ring wait timed out (render thread stalled?) — proceeding"
                    );
                    self.warned_wait = true;
                }
            } else {
                // Warn once per stall episode, not once per process.
                self.warned_wait = false;
            }
        }
        self.cur_seq = seq;
        self.cur_slot = slot;
        self.cursor = 1;
        self.inst_cursor = 0;
        let res = &mut self.residency[slot];
        res.cursor = 0;
        res.prefix_ok = true;
    }

    /// Write one skeleton's palette into the current frame's region and
    /// return its flat `palette_base` index (region-relative). Call once per
    /// skeleton per frame, between `begin_frame` and `end_frame`.
    ///
    /// `key` identifies the skeleton across frames (entity id bits) and
    /// `revision` its palette revision ([`SkeletonInstance::revision`]): when
    /// this region already holds exactly this revision at this base (see
    /// [`RegionResidency`]) the memcpy is skipped — the P4 upload gate for
    /// update-rate-throttled skeletons. The palette is still *present* in
    /// the region either way (R2: the ring rotates, every visible skeleton
    /// occupies its span in every frame's region).
    pub fn write_palette(
        &mut self,
        key: u64,
        revision: u64,
        palette: &[Mat4],
    ) -> Result<u32, Box<dyn std::error::Error>> {
        let n = palette.len() as DeviceSize;
        if n == 0 {
            return Ok(0);
        }
        if self.cursor + n > self.region_capacity {
            self.grow(self.cursor + n)?;
        }
        let base = self.cursor as u32;
        let res = &mut self.residency[self.cur_slot];
        let i = res.cursor;
        if res.prefix_ok {
            match res.entries.get(i) {
                Some(&(k, len, rev)) if k == key && len == n as u32 => {
                    if rev == revision {
                        // Region already holds this palette at this base.
                        res.cursor = i + 1;
                        self.cursor += n;
                        return Ok(base);
                    }
                    // Same span, stale contents: copy below, record the new
                    // revision (prefix stays intact).
                }
                _ => res.prefix_ok = false,
            }
        }
        if !res.prefix_ok {
            res.entries.truncate(i);
        }
        let start = self.cur_slot as DeviceSize * self.region_capacity + self.cursor;
        let sub = self.buffer.clone().slice(start..start + n);
        {
            let mut guard = sub.write()?;
            for (dst, mat) in guard.iter_mut().zip(palette) {
                *dst = mat.to_cols_array();
            }
        }
        let res = &mut self.residency[self.cur_slot];
        if res.entries.len() == i {
            res.entries.push((key, n as u32, revision));
        } else {
            res.entries[i] = (key, n as u32, revision);
        }
        res.cursor = i + 1;
        self.cursor += n;
        Ok(base)
    }

    /// Append one draw instance's metadata to the current frame's instance
    /// region and return its flat instance index (region-relative — the value
    /// `gl_InstanceIndex` takes when the draw's `first_instance` points at
    /// the run's first entry). Call in draw order between `begin_frame` and
    /// `end_frame`; consecutive calls are contiguous, which is what makes a
    /// batch's `first_instance..+instance_count` range valid.
    pub fn write_instance(
        &mut self,
        model: [[f32; 4]; 4],
        palette_base: u32,
    ) -> Result<u32, Box<dyn std::error::Error>> {
        if self.inst_cursor + 1 > self.inst_capacity {
            self.grow_instances(self.inst_cursor + 1)?;
        }
        let index = self.inst_cursor as u32;
        let start = self.cur_slot as DeviceSize * self.inst_capacity + self.inst_cursor;
        let sub = self.inst_buffer.clone().slice(start..start + 1);
        {
            let mut guard = sub.write()?;
            guard[0] = InstanceData {
                model,
                palette_base,
                _pad: [0; 3],
            };
        }
        self.inst_cursor += 1;
        Ok(index)
    }

    /// Close the frame: the region slices + slot for the `FramePacket`.
    pub fn end_frame(&self) -> SkinnedPaletteFrame {
        let start = self.cur_slot as DeviceSize * self.region_capacity;
        let inst_start = self.cur_slot as DeviceSize * self.inst_capacity;
        SkinnedPaletteFrame {
            region: self
                .buffer
                .clone()
                .slice(start..start + self.region_capacity),
            instances: self
                .inst_buffer
                .clone()
                .slice(inst_start..inst_start + self.inst_capacity),
            slot: self.cur_slot,
        }
    }

    /// Reallocate with room for at least `needed` matrices per region and
    /// carry the current frame's writes over. Old buffer stays alive via the
    /// Arcs held by in-flight frames' descriptor sets, so no fence wait is
    /// needed here. The ring wait keeps running for later frames (the epoch
    /// is not reset — see the field doc / module docs).
    fn grow(&mut self, needed: DeviceSize) -> Result<(), Box<dyn std::error::Error>> {
        let new_capacity = round_up(needed.max(self.region_capacity * 2), self.align_mats);
        let new_buffer = Self::alloc_buffer(&self.allocator, new_capacity)?;
        if self.cursor > 1 {
            let old_start = self.cur_slot as DeviceSize * self.region_capacity;
            let new_start = self.cur_slot as DeviceSize * new_capacity;
            let src = self
                .buffer
                .clone()
                .slice(old_start + 1..old_start + self.cursor);
            let dst = new_buffer
                .clone()
                .slice(new_start + 1..new_start + self.cursor);
            let src_guard = src.read()?;
            let mut dst_guard = dst.write()?;
            dst_guard.copy_from_slice(&src_guard);
        }
        self.buffer = new_buffer;
        self.region_capacity = new_capacity;
        // Residency: the current region's already-written span was carried
        // over (entries up to this visit's cursor stay valid); everything
        // else refers to the old buffer and must be forgotten.
        for (i, res) in self.residency.iter_mut().enumerate() {
            if i == self.cur_slot {
                let keep = res.cursor;
                res.entries.truncate(keep);
            } else {
                res.entries.clear();
                res.cursor = 0;
                res.prefix_ok = true;
            }
        }
        Ok(())
    }

    /// Reallocate the instance ring with room for at least `needed` instances
    /// per region and carry the current frame's writes over (same discipline
    /// as [`grow`](Self::grow); no residency to fix up).
    fn grow_instances(&mut self, needed: DeviceSize) -> Result<(), Box<dyn std::error::Error>> {
        let new_capacity = round_up(needed.max(self.inst_capacity * 2), self.inst_align);
        let new_buffer = Self::alloc_inst_buffer(&self.allocator, new_capacity)?;
        if self.inst_cursor > 0 {
            let old_start = self.cur_slot as DeviceSize * self.inst_capacity;
            let new_start = self.cur_slot as DeviceSize * new_capacity;
            let src = self
                .inst_buffer
                .clone()
                .slice(old_start..old_start + self.inst_cursor);
            let dst = new_buffer
                .clone()
                .slice(new_start..new_start + self.inst_cursor);
            let src_guard = src.read()?;
            let mut dst_guard = dst.write()?;
            dst_guard.copy_from_slice(&src_guard);
        }
        self.inst_buffer = new_buffer;
        self.inst_capacity = new_capacity;
        Ok(())
    }

    /// One-off palette + view-projection descriptor set for the editor
    /// preview pipelines (thumbnails, mesh/anim/blend-space previews). Their
    /// set 0 is `{ binding 0: palette SSBO, binding 1: view-projection UBO }`;
    /// an empty `palette` binds a single identity matrix (`palette_base = 0`).
    ///
    /// These paths record a fresh buffer per call (no reuse), so no ring
    /// discipline applies.
    pub fn create_preview_set(
        allocator: &Arc<StandardMemoryAllocator>,
        descriptor_set_allocator: &Arc<StandardDescriptorSetAllocator>,
        set_layout: Arc<DescriptorSetLayout>,
        palette: &[Mat4],
        view_projection: Mat4,
    ) -> Result<Arc<DescriptorSet>, Box<dyn std::error::Error>> {
        let mats: Vec<[f32; 16]> = if palette.is_empty() {
            vec![IDENTITY_MAT]
        } else {
            palette.iter().map(|m| m.to_cols_array()).collect()
        };
        let palette_buffer = Buffer::from_iter(
            allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::STORAGE_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            mats,
        )?;
        let vp_buffer = Buffer::from_data(
            allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::UNIFORM_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            view_projection.to_cols_array_2d(),
        )?;
        let set = DescriptorSet::new(
            descriptor_set_allocator.clone(),
            set_layout,
            [
                WriteDescriptorSet::buffer(0, palette_buffer),
                WriteDescriptorSet::buffer(1, vp_buffer),
            ],
            [],
        )?;
        Ok(set)
    }
}

fn round_up(value: DeviceSize, multiple: DeviceSize) -> DeviceSize {
    value.div_ceil(multiple) * multiple
}

fn gcd(mut a: DeviceSize, mut b: DeviceSize) -> DeviceSize {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_sync_marks_and_waits() {
        let sync = PaletteRingSync::new();
        assert!(!sync.wait_done(0, 0, Duration::from_millis(10)));
        sync.mark_done(0, 0);
        assert!(sync.wait_done(0, 0, Duration::from_millis(10)));
        // Monotone: an older seq can't regress the marker.
        sync.mark_done(0, 5);
        sync.mark_done(0, 2);
        assert!(sync.wait_done(0, 5, Duration::from_millis(10)));
        assert!(!sync.wait_done(0, 6, Duration::from_millis(10)));
    }

    #[test]
    fn round_up_multiples() {
        assert_eq!(round_up(256, 4), 256);
        assert_eq!(round_up(257, 4), 260);
        assert_eq!(round_up(1, 1), 1);
    }

    /// The shader-side std430 `InstanceData` stride is 80 B; the Rust struct
    /// must match exactly (explicit tail padding, no compiler surprises).
    #[test]
    fn instance_data_layout() {
        assert_eq!(std::mem::size_of::<InstanceData>(), 80);
        assert_eq!(std::mem::offset_of!(InstanceData, model), 0);
        assert_eq!(std::mem::offset_of!(InstanceData, palette_base), 64);
    }

    /// Region byte offsets stay aligned for every power-of-two SSBO offset
    /// alignment when capacities are rounded to `align / gcd(stride, align)`.
    #[test]
    fn instance_region_alignment_granularity() {
        let stride = std::mem::size_of::<InstanceData>() as DeviceSize;
        for align in [1u64, 4, 16, 64, 256] {
            let granule = (align / gcd(stride, align)).max(1);
            let capacity = round_up(INITIAL_REGION_INSTANCES, granule);
            assert_eq!((capacity * stride) % align, 0, "align {align}");
        }
    }
}
