//! Asset pak file format for shipping builds.
//!
//! Packs all content files into a single binary archive so assets
//! aren't exposed as loose files in the distribution.
//!
//! Format (little-endian):
//!   Header:
//!     magic: [u8; 4]  = b"RPAK"
//!     version: u32     = 1
//!     entry_count: u32
//!     toc_offset: u64  (byte offset to the TOC in the file)
//!
//!   File data:
//!     For each entry: raw bytes (no per-file compression for simplicity)
//!
//!   Table of Contents (at toc_offset):
//!     For each entry:
//!       path_len: u32
//!       path: [u8; path_len]  (UTF-8, forward-slash normalized)
//!       offset: u64           (byte offset of file data from start)
//!       size: u64             (byte length of file data)

use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const PAK_MAGIC: &[u8; 4] = b"RPAK";
const PAK_VERSION: u32 = 1;
const HEADER_SIZE: u64 = 4 + 4 + 4 + 8; // magic + version + entry_count + toc_offset

struct TocEntry {
    path: String,
    offset: u64,
    size: u64,
}

/// Packs a directory into a `.pak` file.
pub fn pack_directory(content_dir: &Path, output_path: &Path) -> io::Result<u64> {
    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    collect_files(content_dir, content_dir, &mut entries)?;
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut file = std::fs::File::create(output_path)?;

    // Write placeholder header (we'll fill toc_offset later)
    file.write_all(PAK_MAGIC)?;
    file.write_all(&PAK_VERSION.to_le_bytes())?;
    file.write_all(&(entries.len() as u32).to_le_bytes())?;
    file.write_all(&0u64.to_le_bytes())?; // toc_offset placeholder

    // Write file data, record offsets
    let mut toc: Vec<TocEntry> = Vec::with_capacity(entries.len());
    for (rel_path, abs_path) in &entries {
        let offset = file.stream_position()?;
        let data = std::fs::read(abs_path)?;
        let size = data.len() as u64;
        file.write_all(&data)?;
        toc.push(TocEntry {
            path: rel_path.clone(),
            offset,
            size,
        });
    }

    // Write TOC
    let toc_offset = file.stream_position()?;
    for entry in &toc {
        let path_bytes = entry.path.as_bytes();
        file.write_all(&(path_bytes.len() as u32).to_le_bytes())?;
        file.write_all(path_bytes)?;
        file.write_all(&entry.offset.to_le_bytes())?;
        file.write_all(&entry.size.to_le_bytes())?;
    }

    // Patch toc_offset in header
    file.seek(SeekFrom::Start(4 + 4 + 4))?; // after magic + version + entry_count
    file.write_all(&toc_offset.to_le_bytes())?;

    let total_size = file.seek(SeekFrom::End(0))?;
    Ok(total_size)
}

fn collect_files(
    base: &Path,
    dir: &Path,
    out: &mut Vec<(String, PathBuf)>,
) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(base, &path, out)?;
        } else {
            let rel = path.strip_prefix(base).unwrap();
            let normalized = rel
                .to_string_lossy()
                .replace('\\', "/");
            out.push((normalized, path));
        }
    }
    Ok(())
}

/// Reads assets from a loaded `.pak` file.
pub struct PakReader {
    data: Vec<u8>,
    toc: HashMap<String, (u64, u64)>, // path -> (offset, size)
}

impl PakReader {
    /// Open and parse a `.pak` file.
    pub fn open(path: &Path) -> io::Result<Self> {
        let data = std::fs::read(path)?;
        Self::from_bytes(data)
    }

    fn from_bytes(data: Vec<u8>) -> io::Result<Self> {
        if data.len() < HEADER_SIZE as usize {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "pak file too small"));
        }

        let mut cursor = io::Cursor::new(&data);

        let mut magic = [0u8; 4];
        cursor.read_exact(&mut magic)?;
        if &magic != PAK_MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid pak magic"));
        }

        let mut buf4 = [0u8; 4];
        let mut buf8 = [0u8; 8];

        cursor.read_exact(&mut buf4)?;
        let version = u32::from_le_bytes(buf4);
        if version != PAK_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported pak version {}", version),
            ));
        }

        cursor.read_exact(&mut buf4)?;
        let entry_count = u32::from_le_bytes(buf4) as usize;

        cursor.read_exact(&mut buf8)?;
        let toc_offset = u64::from_le_bytes(buf8);

        cursor.seek(SeekFrom::Start(toc_offset))?;

        let mut toc = HashMap::with_capacity(entry_count);
        for _ in 0..entry_count {
            cursor.read_exact(&mut buf4)?;
            let path_len = u32::from_le_bytes(buf4) as usize;

            let mut path_bytes = vec![0u8; path_len];
            cursor.read_exact(&mut path_bytes)?;
            let path = String::from_utf8(path_bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

            cursor.read_exact(&mut buf8)?;
            let offset = u64::from_le_bytes(buf8);

            cursor.read_exact(&mut buf8)?;
            let size = u64::from_le_bytes(buf8);

            toc.insert(path, (offset, size));
        }

        Ok(Self { data, toc })
    }

    /// Read a file from the pak by its content-relative path (forward slashes).
    pub fn read(&self, path: &str) -> Option<&[u8]> {
        let normalized = path.replace('\\', "/");
        let (offset, size) = self.toc.get(&normalized)?;
        let start = *offset as usize;
        let end = start + *size as usize;
        if end > self.data.len() {
            return None;
        }
        Some(&self.data[start..end])
    }

    /// Read a file as a UTF-8 string.
    pub fn read_string(&self, path: &str) -> Option<String> {
        let bytes = self.read(path)?;
        String::from_utf8(bytes.to_vec()).ok()
    }

    /// Check if a path exists in the pak.
    pub fn contains(&self, path: &str) -> bool {
        let normalized = path.replace('\\', "/");
        self.toc.contains_key(&normalized)
    }

    /// Returns the number of entries in the pak.
    pub fn entry_count(&self) -> usize {
        self.toc.len()
    }

    /// Lists all paths in the pak.
    pub fn list_files(&self) -> Vec<&str> {
        self.toc.keys().map(|s| s.as_str()).collect()
    }
}
