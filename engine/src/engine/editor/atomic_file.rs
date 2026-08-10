//! Atomic config writes (Task 39.8 P6, ruling §5.6).
//!
//! The relaunch flow rewrites `project.ron`, `editor_prefs.ron` and
//! `editor_layout_crusty.ron` and then immediately exits the process. A
//! half-written config there is not a cosmetic bug — it is the file the
//! *next* process reads a moment later.
//!
//! ## The primitive
//!
//! Write a temp file **in the destination directory** (same volume, so the
//! replace cannot degrade into a copy), flush and `sync_all` it, then replace:
//!
//! - **Windows:** `ReplaceFileW`. `std::fs::rename` maps to `MoveFileExW`
//!   with `MOVEFILE_REPLACE_EXISTING`, which is *usually* fine but fails with
//!   a sharing violation if anything holds the destination open — and it does
//!   not carry the destination's attributes across. `ReplaceFileW` is the
//!   call Windows provides for exactly this "swap contents, keep identity"
//!   job. We fall back to `rename` if it fails, since a rename is still
//!   better than a truncating write.
//! - **Elsewhere:** `std::fs::rename`, which is atomic on POSIX.
//!
//! The temp file is cleaned up on every failure path.

use std::io;
use std::path::Path;

/// Write `contents` to `path`, replacing any existing file atomically.
pub fn atomic_write(path: &Path, contents: &str) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;

    // Same directory ⇒ same volume ⇒ the replace stays a metadata operation.
    let mut tmp = dir.join(file_name);
    tmp.as_mut_os_string()
        .push(format!(".{}.tmp", std::process::id()));

    write_and_sync(&tmp, contents).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;

    match replace_file(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn write_and_sync(tmp: &Path, contents: &str) -> io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(tmp)?;
    file.write_all(contents.as_bytes())?;
    file.flush()?;
    // Durability matters here specifically because the process is about to
    // exit: without this the bytes can still be in the page cache.
    file.sync_all()
}

#[cfg(windows)]
fn replace_file(tmp: &Path, dest: &Path) -> io::Result<()> {
    // A destination that does not exist yet has nothing to replace.
    if !dest.exists() {
        return std::fs::rename(tmp, dest);
    }

    #[cfg(feature = "editor")]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

        let wide = |p: &Path| -> Vec<u16> {
            p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
        };
        let (dest_w, tmp_w) = (wide(dest), wide(tmp));

        // SAFETY: both paths are NUL-terminated UTF-16 owned by this frame;
        // the remaining arguments are the documented "no backup, no flags"
        // form. `ReplaceFileW` returns 0 on failure and sets last-error.
        let ok = unsafe {
            ReplaceFileW(
                dest_w.as_ptr(),
                tmp_w.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if ok != 0 {
            return Ok(());
        }
        // Fall through to rename: still better than a truncating write, and
        // covers the cases ReplaceFileW refuses (e.g. different volumes).
    }

    std::fs::rename(tmp, dest)
}

#[cfg(not(windows))]
fn replace_file(tmp: &Path, dest: &Path) -> io::Result<()> {
    // POSIX rename(2) is atomic and replaces the destination.
    std::fs::rename(tmp, dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rust_engine_atomic_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn writes_a_new_file() {
        let dir = temp_dir("new");
        let path = dir.join("project.ron");
        atomic_write(&path, "(name: \"a\")").expect("write");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(name: \"a\")");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The case that matters: the destination already exists and must end up
    /// holding the new contents, with no temp file left behind.
    #[test]
    fn replaces_an_existing_file_and_leaves_no_temp_behind() {
        let dir = temp_dir("replace");
        let path = dir.join("project.ron");
        std::fs::write(&path, "old contents, longer than the new one").unwrap();

        atomic_write(&path, "new").expect("replace");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");

        let strays: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "project.ron")
            .collect();
        assert!(strays.is_empty(), "temp files left behind: {strays:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Repeated replaces must keep working — the relaunch path writes three
    /// files back to back, and a stale temp name would collide.
    #[test]
    fn repeated_replaces_succeed() {
        let dir = temp_dir("repeat");
        let path = dir.join("layout.ron");
        for i in 0..5 {
            atomic_write(&path, &format!("pass {i}")).expect("write");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("pass {i}"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_directory_is_an_error_not_a_panic() {
        let path = std::env::temp_dir()
            .join("rust_engine_atomic_nope")
            .join("deep")
            .join("x.ron");
        assert!(atomic_write(&path, "x").is_err());
    }
}
