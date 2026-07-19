//! `.ccol` collision chunk format: writer + validating reader + manifest.
//!
//! Little-endian, explicitly serialized field-by-field (never struct memcpy),
//! 16-byte-aligned sections, every count/offset validated at load.
//!
//! Layout:
//! - Header: magic `RCOL`, format version (u32), cooker version hash (u32),
//!   flags (u32, reserved = 0), chunk coord (i32 x2), local AABB (f32 x6),
//!   section count (u32), section table entries (id, offset, size — u32 x3).
//! - Sections, each starting at a 16-byte-aligned file offset.
//!
//! Raw geometry is the durable format; parry BVHs are rebuilt at load.
//! Unknown section ids are skipped (forward compat); unknown format version
//! is rejected. Cooker hash is informational — chunk-set compatibility is
//! enforced via manifest content hashes.

use glam::IVec2;
use serde::{Deserialize, Serialize};

pub const MAGIC: [u8; 4] = *b"RCOL";
pub const FORMAT_VERSION: u32 = 1;

pub const SECTION_TRIMESH: u32 = 1;
/// Reserved for future sections (see plan): heightfield = 2, primitives = 3.
pub const SECTION_ALIGN: usize = 16;

/// Sane maximums — a chunk beyond these is corrupt or a cooker bug.
pub const MAX_VERTICES: u32 = 1 << 22;
pub const MAX_TRIANGLES: u32 = 1 << 22;
pub const MAX_SECTIONS: u32 = 64;

#[derive(Debug, Clone, PartialEq)]
pub struct ChunkData {
    pub coord: IVec2,
    /// Chunk-local AABB: (min xyz, max xyz).
    pub local_aabb: ([f32; 3], [f32; 3]),
    pub trimesh: TriMeshSection,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TriMeshSection {
    /// Chunk-local positions (Z-up).
    pub vertices: Vec<[f32; 3]>,
    pub indices: Vec<[u32; 3]>,
    /// Stable per-triangle id: hash of source-entity GUID + source triangle
    /// index. Same length as `indices`.
    pub triangle_ids: Vec<u64>,
    /// Material id in low 16 bits; rest reserved. Same length as `indices`.
    pub triangle_flags: Vec<u32>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FormatError {
    BadMagic,
    UnsupportedVersion(u32),
    Truncated,
    Misaligned { section: u32, offset: u32 },
    SectionOutOfBounds { section: u32 },
    TooLarge(&'static str),
    CountMismatch(&'static str),
    IndexOutOfRange { triangle: usize },
    NonFiniteFloat(&'static str),
    MissingTrimesh,
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for FormatError {}

/// FNV-1a 64 over raw bytes. Client and server hash their collision manifest
/// with this and compare at connect (M6 D1) — a mismatch means the two sides
/// would simulate against different geometry.
pub fn manifest_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

// ---------------------------------------------------------------- writing

pub fn write_chunk(chunk: &ChunkData, cooker_hash: u32) -> Vec<u8> {
    let sections: [(u32, Vec<u8>); 1] = [(SECTION_TRIMESH, encode_trimesh(&chunk.trimesh))];

    let header_len = 4 + 4 + 4 + 4 + 8 + 24 + 4 + sections.len() * 12;
    let mut out = Vec::with_capacity(header_len + sections[0].1.len() + SECTION_ALIGN);

    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&cooker_hash.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // flags (reserved)
    out.extend_from_slice(&chunk.coord.x.to_le_bytes());
    out.extend_from_slice(&chunk.coord.y.to_le_bytes());
    for v in chunk.local_aabb.0.iter().chain(chunk.local_aabb.1.iter()) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(sections.len() as u32).to_le_bytes());

    // Section table: offsets computed after 16-byte alignment.
    let mut offset = header_len;
    for (id, payload) in &sections {
        offset = align_up(offset, SECTION_ALIGN);
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        offset += payload.len();
    }
    for (_, payload) in &sections {
        while !out.len().is_multiple_of(SECTION_ALIGN) {
            out.push(0);
        }
        out.extend_from_slice(payload);
    }
    out
}

fn encode_trimesh(m: &TriMeshSection) -> Vec<u8> {
    assert_eq!(m.indices.len(), m.triangle_ids.len());
    assert_eq!(m.indices.len(), m.triangle_flags.len());
    let mut out = Vec::with_capacity(8 + m.vertices.len() * 12 + m.indices.len() * 24);
    out.extend_from_slice(&(m.vertices.len() as u32).to_le_bytes());
    out.extend_from_slice(&(m.indices.len() as u32).to_le_bytes());
    for v in &m.vertices {
        for c in v {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    for tri in &m.indices {
        for i in tri {
            out.extend_from_slice(&i.to_le_bytes());
        }
    }
    for id in &m.triangle_ids {
        out.extend_from_slice(&id.to_le_bytes());
    }
    for fl in &m.triangle_flags {
        out.extend_from_slice(&fl.to_le_bytes());
    }
    out
}

fn align_up(n: usize, align: usize) -> usize {
    n.div_ceil(align) * align
}

// ---------------------------------------------------------------- reading

pub fn read_chunk(bytes: &[u8]) -> Result<ChunkData, FormatError> {
    let mut r = Reader { bytes, pos: 0 };

    if r.take(4)? != MAGIC {
        return Err(FormatError::BadMagic);
    }
    let version = r.u32()?;
    if version != FORMAT_VERSION {
        return Err(FormatError::UnsupportedVersion(version));
    }
    let _cooker_hash = r.u32()?; // informational
    let _flags = r.u32()?;
    let coord = IVec2::new(r.i32()?, r.i32()?);
    let mut aabb = [0f32; 6];
    for v in &mut aabb {
        *v = r.f32()?;
        if !v.is_finite() {
            return Err(FormatError::NonFiniteFloat("aabb"));
        }
    }
    let section_count = r.u32()?;
    if section_count > MAX_SECTIONS {
        return Err(FormatError::TooLarge("section count"));
    }

    let mut trimesh = None;
    let mut table = Vec::with_capacity(section_count as usize);
    for _ in 0..section_count {
        table.push((r.u32()?, r.u32()?, r.u32()?)); // (id, offset, size)
    }
    for (id, offset, size) in table {
        if !(offset as usize).is_multiple_of(SECTION_ALIGN) {
            return Err(FormatError::Misaligned {
                section: id,
                offset,
            });
        }
        let end = (offset as usize)
            .checked_add(size as usize)
            .ok_or(FormatError::SectionOutOfBounds { section: id })?;
        if end > bytes.len() {
            return Err(FormatError::SectionOutOfBounds { section: id });
        }
        let payload = &bytes[offset as usize..end];
        if id == SECTION_TRIMESH {
            trimesh = Some(decode_trimesh(payload)?);
        } // other ids: skip (forward compat)
    }

    Ok(ChunkData {
        coord,
        local_aabb: ([aabb[0], aabb[1], aabb[2]], [aabb[3], aabb[4], aabb[5]]),
        trimesh: trimesh.ok_or(FormatError::MissingTrimesh)?,
    })
}

fn decode_trimesh(payload: &[u8]) -> Result<TriMeshSection, FormatError> {
    let mut r = Reader {
        bytes: payload,
        pos: 0,
    };
    let vertex_count = r.u32()?;
    let triangle_count = r.u32()?;
    if vertex_count > MAX_VERTICES {
        return Err(FormatError::TooLarge("vertex count"));
    }
    if triangle_count > MAX_TRIANGLES {
        return Err(FormatError::TooLarge("triangle count"));
    }
    let expected = 8 + vertex_count as usize * 12 + triangle_count as usize * (12 + 8 + 4);
    if payload.len() != expected {
        return Err(FormatError::CountMismatch("trimesh section size"));
    }

    let mut vertices = Vec::with_capacity(vertex_count as usize);
    for _ in 0..vertex_count {
        let v = [r.f32()?, r.f32()?, r.f32()?];
        if v.iter().any(|c| !c.is_finite()) {
            return Err(FormatError::NonFiniteFloat("vertex"));
        }
        vertices.push(v);
    }
    let mut indices = Vec::with_capacity(triangle_count as usize);
    for t in 0..triangle_count as usize {
        let tri = [r.u32()?, r.u32()?, r.u32()?];
        if tri.iter().any(|&i| i >= vertex_count) {
            return Err(FormatError::IndexOutOfRange { triangle: t });
        }
        indices.push(tri);
    }
    let mut triangle_ids = Vec::with_capacity(triangle_count as usize);
    for _ in 0..triangle_count {
        triangle_ids.push(r.u64()?);
    }
    let mut triangle_flags = Vec::with_capacity(triangle_count as usize);
    for _ in 0..triangle_count {
        triangle_flags.push(r.u32()?);
    }

    Ok(TriMeshSection {
        vertices,
        indices,
        triangle_ids,
        triangle_flags,
    })
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], FormatError> {
        let end = self.pos.checked_add(n).ok_or(FormatError::Truncated)?;
        if end > self.bytes.len() {
            return Err(FormatError::Truncated);
        }
        let s = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, FormatError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, FormatError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, FormatError> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, FormatError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

// ---------------------------------------------------------------- hashing

/// FNV-1a 64: deterministic content hash for manifest entries and stable
/// triangle ids. Not cryptographic — collision chunks are trusted content.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// ---------------------------------------------------------------- manifest

/// `content/collision/<scene>/manifest.ron`. Loader uses it for existence
/// checks and version validation; M4 streams from it. Chunk-set compatibility
/// between client and server is enforced by comparing content hashes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollisionManifest {
    pub scene: String,
    pub format_version: u32,
    pub cooker_hash: u32,
    /// `fnv1a_64` of the source `.scene` file bytes at cook time. Cook-side
    /// staleness check only (skip re-cook when unchanged) — not validated at
    /// load. Does not cover referenced mesh assets; `--force` re-cooks.
    pub scene_hash: u64,
    pub chunk_size: f32,
    pub chunks: Vec<ManifestChunk>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestChunk {
    pub coord: [i32; 2],
    /// `fnv1a_64` of the chunk's `.ccol` file bytes.
    pub content_hash: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_chunk() -> ChunkData {
        ChunkData {
            coord: IVec2::new(-2, 5),
            local_aabb: ([0.0, 0.0, -1.0], [64.0, 64.0, 10.0]),
            trimesh: TriMeshSection {
                vertices: vec![[0.0, 0.0, 0.0], [64.0, 0.0, 0.0], [0.0, 64.0, 2.5]],
                indices: vec![[0, 1, 2]],
                triangle_ids: vec![0xDEAD_BEEF_CAFE_F00D],
                triangle_flags: vec![7],
            },
        }
    }

    #[test]
    fn roundtrip() {
        let chunk = sample_chunk();
        let bytes = write_chunk(&chunk, 0x1234);
        assert_eq!(read_chunk(&bytes).unwrap(), chunk);
    }

    #[test]
    fn sections_are_aligned() {
        let bytes = write_chunk(&sample_chunk(), 0);
        // Section table starts at 52; single entry: id at 52, offset at 56.
        let offset = u32::from_le_bytes(bytes[56..60].try_into().unwrap());
        assert_eq!(offset as usize % SECTION_ALIGN, 0);
    }

    #[test]
    fn deterministic_output() {
        let a = write_chunk(&sample_chunk(), 9);
        let b = write_chunk(&sample_chunk(), 9);
        assert_eq!(a, b);
        assert_eq!(fnv1a_64(&a), fnv1a_64(&b));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = write_chunk(&sample_chunk(), 0);
        bytes[0] = b'X';
        assert_eq!(read_chunk(&bytes), Err(FormatError::BadMagic));
    }

    #[test]
    fn rejects_unknown_version() {
        let mut bytes = write_chunk(&sample_chunk(), 0);
        bytes[4..8].copy_from_slice(&99u32.to_le_bytes());
        assert_eq!(read_chunk(&bytes), Err(FormatError::UnsupportedVersion(99)));
    }

    #[test]
    fn rejects_truncation() {
        let bytes = write_chunk(&sample_chunk(), 0);
        for cut in [3, 20, 51, bytes.len() - 1] {
            assert!(read_chunk(&bytes[..cut]).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn rejects_out_of_range_index() {
        let mut chunk = sample_chunk();
        chunk.trimesh.indices[0][2] = 3; // only 3 vertices
        let bytes = write_chunk(&chunk, 0);
        assert_eq!(
            read_chunk(&bytes),
            Err(FormatError::IndexOutOfRange { triangle: 0 })
        );
    }

    #[test]
    fn rejects_non_finite_vertex() {
        let mut chunk = sample_chunk();
        chunk.trimesh.vertices[1][0] = f32::NAN;
        let bytes = write_chunk(&chunk, 0);
        assert_eq!(
            read_chunk(&bytes),
            Err(FormatError::NonFiniteFloat("vertex"))
        );
    }

    #[test]
    fn skips_unknown_section() {
        // Hand-build a file with an unknown section before the trimesh.
        let chunk = sample_chunk();
        let bytes = write_chunk(&chunk, 0);
        // Reuse the writer output but claim two sections: rewrite is simpler —
        // append an unknown-section entry pointing at aligned zero padding.
        // Easiest robust check: mutate the section id and expect MissingTrimesh.
        let mut mutated = bytes.clone();
        mutated[52..56].copy_from_slice(&999u32.to_le_bytes());
        assert_eq!(read_chunk(&mutated), Err(FormatError::MissingTrimesh));
    }

    #[test]
    fn manifest_ron_roundtrip() {
        let m = CollisionManifest {
            scene: "greybox".into(),
            format_version: FORMAT_VERSION,
            cooker_hash: 0xABCD,
            scene_hash: 0xFEED_F00D,
            chunk_size: crate::world_grid::CHUNK_SIZE,
            chunks: vec![ManifestChunk {
                coord: [-2, 5],
                content_hash: 42,
            }],
        };
        let text = ron::ser::to_string_pretty(&m, Default::default()).unwrap();
        assert_eq!(ron::from_str::<CollisionManifest>(&text).unwrap(), m);
    }

    #[test]
    fn manifest_hash_is_fnv1a64() {
        // Reference vectors — the value is a wire contract (client vs server
        // config row), so it must never drift.
        assert_eq!(manifest_hash(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(manifest_hash(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
