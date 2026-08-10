//! Editor relaunch (Task 39.8 P6, ruling §5.6).
//!
//! Toggling a plugin is restart-only by design (tier 1), so the manager needs
//! a "Relaunch now" that is safe rather than merely convenient.
//!
//! ## Ordering
//!
//! 1. The caller saves whatever is dirty through the normal paths — those now
//!    write via [`atomic_write`](super::atomic_file::atomic_write), so the
//!    config the next process reads is never half a file.
//! 2. [`spawn_replacement`] starts a copy of this executable with
//!    `--wait-parent <pid>`.
//! 3. The parent returns and exits its event loop normally (saving window
//!    config on the way out, like any other close).
//! 4. The child blocks in [`wait_for_parent`] until the parent process really
//!    is gone, then continues into normal startup and reads the config.
//!
//! ## Why wait on a handle
//!
//! A pid is reusable: between "parent exited" and "child looks", the number
//! can belong to something else entirely, and a polling loop can either miss
//! the exit or wait on a stranger. `OpenProcess(SYNCHRONIZE)` takes a handle
//! to *that* process object; the handle keeps the object alive even after the
//! process dies, so `WaitForSingleObject` answers about the process we meant.
//! If `OpenProcess` fails the parent has already exited and been reaped —
//! also a valid "go ahead".
//!
//! The M9.6 listen-server launcher is precedent for *spawning* only; it never
//! exits its parent, so no code is shared with it.

use std::path::PathBuf;

/// The flag the relaunched child is started with.
pub const WAIT_PARENT_FLAG: &str = "--wait-parent";

/// Parse `--wait-parent <pid>` out of a process argument list.
pub fn parse_wait_parent(args: &[String]) -> Option<u32> {
    let idx = args.iter().position(|a| a == WAIT_PARENT_FLAG)?;
    args.get(idx + 1)?.parse().ok()
}

/// Block until the process `pid` has exited.
///
/// Returns immediately if the process is already gone or cannot be opened —
/// the point is "the parent is not still running", not "we observed its
/// death". Never blocks forever on a pid that was never valid.
pub fn wait_for_parent(pid: u32) {
    #[cfg(all(windows, feature = "editor"))]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE,
        };

        // SAFETY: `OpenProcess` returns a null handle on failure, which we
        // check before use. `SYNCHRONIZE` is the least authority that permits
        // waiting.
        // windows-sys 0.52 models HANDLE as `isize`; 0 is the failure value.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if handle == 0 {
            // Already exited (or not ours to wait on) — nothing to wait for.
            return;
        }
        // SAFETY: `handle` is a live process handle we own until CloseHandle.
        let result = unsafe { WaitForSingleObject(handle, INFINITE) };
        // SAFETY: same handle, closed exactly once.
        unsafe { CloseHandle(handle) };
        if result != WAIT_OBJECT_0 {
            log::warn!("relaunch: unexpected wait result {result} for parent {pid}");
        }
    }
    #[cfg(not(all(windows, feature = "editor")))]
    {
        let _ = pid;
    }
}

/// Start a fresh copy of this executable, told to wait for us first.
///
/// Returns the child's pid. The caller is expected to exit promptly
/// afterwards; until it does, the child is parked in [`wait_for_parent`].
pub fn spawn_replacement() -> std::io::Result<u32> {
    let exe: PathBuf = std::env::current_exe()?;
    let cwd = std::env::current_dir()?;

    // Carry the original arguments across so a relaunch preserves how the
    // editor was launched (`--editor-benchmark-tools`, `--connect …`), minus
    // any wait-parent pair from a previous relaunch.
    let mut forwarded: Vec<String> = Vec::new();
    let original: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < original.len() {
        if original[i] == WAIT_PARENT_FLAG {
            i += 2; // skip flag + its pid
            continue;
        }
        forwarded.push(original[i].clone());
        i += 1;
    }

    let child = std::process::Command::new(exe)
        .current_dir(cwd)
        .args(&forwarded)
        .arg(WAIT_PARENT_FLAG)
        .arg(std::process::id().to_string())
        .spawn()?;

    Ok(child.id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_the_wait_parent_pid() {
        assert_eq!(
            parse_wait_parent(&args(["--wait-parent", "4321"].as_slice())),
            Some(4321)
        );
        assert_eq!(
            parse_wait_parent(&args(
                ["--editor-benchmark-tools", "--wait-parent", "7"].as_slice()
            )),
            Some(7)
        );
    }

    #[test]
    fn absent_or_malformed_wait_parent_is_none() {
        assert_eq!(parse_wait_parent(&args(&["--editor-benchmark-tools"])), None);
        assert_eq!(parse_wait_parent(&args(&["--wait-parent"])), None);
        assert_eq!(parse_wait_parent(&args(&["--wait-parent", "not-a-pid"])), None);
        assert_eq!(parse_wait_parent(&[]), None);
    }

    /// A pid that cannot be opened must not hang the child. This is the
    /// failure mode that would strand a user with no editor at all.
    #[test]
    fn waiting_on_a_dead_parent_returns_immediately() {
        let start = std::time::Instant::now();
        // A pid that is almost certainly not a live process we can open.
        wait_for_parent(0xFFFF_FFF0);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "wait_for_parent must not block on a pid it cannot open"
        );
    }

    /// The real handoff, with a dummy parent: spawn a short-lived process,
    /// wait on its handle, and confirm we were released only after it exited.
    #[cfg(windows)]
    #[test]
    fn waits_for_a_live_parent_until_it_exits() {
        let mut dummy = std::process::Command::new("cmd")
            .args(["/C", "timeout", "/T", "1", "/NOBREAK"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn dummy parent");
        let pid = dummy.id();

        let start = std::time::Instant::now();
        wait_for_parent(pid);
        let waited = start.elapsed();

        // The wait returned; the process must be finished by now.
        let exited = dummy
            .try_wait()
            .expect("try_wait")
            .is_some();
        assert!(
            exited,
            "wait_for_parent returned while the parent was still running"
        );
        assert!(
            waited < std::time::Duration::from_secs(30),
            "wait took implausibly long: {waited:?}"
        );
        let _ = dummy.kill();
    }
}
