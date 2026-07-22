//! M9.6 P4: editor "Play As Listen Server" launcher. SpacetimeDB has no
//! true listen server — the sim always runs in the SpacetimeDB process —
//! so this ensures a local `spacetime start` is running, publishes the
//! module (wipe-free, via `server/publish.ps1`), then the editor joins as
//! a regular client. The spawned server deliberately outlives play-exit
//! (host_local.ps1's reuse-don't-stop rule).

use std::process::Command;
use std::sync::mpsc::{channel, Receiver};

pub fn is_local_host(host: &str) -> bool {
    let h = host
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let h = h.split(['/', ':']).next().unwrap_or("");
    matches!(h, "127.0.0.1" | "localhost")
}

/// Off-thread launcher; the receiver yields exactly one message. Dropping
/// the receiver abandons the wait but not the server (by design).
pub fn spawn_launcher(host: String, module: String) -> Receiver<Result<(), String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(launch(&host, &module));
    });
    rx
}

fn launch(host: &str, module: &str) -> Result<(), String> {
    if server_alive(host) {
        println!("listen server: reusing SpacetimeDB at {host}");
    } else {
        println!("listen server: starting `spacetime start`");
        Command::new("spacetime")
            .arg("start")
            .spawn()
            .map_err(|e| format!("failed to spawn `spacetime start`: {e}"))?;
    }
    // Readiness gate = a publish succeeding; retries cover a cold start.
    // Only sleep time counts toward the budget (host_local.ps1 rule), so a
    // slow module build inside publish can't eat the retries.
    let mut waited = 0;
    loop {
        match publish(module) {
            Ok(()) => return Ok(()),
            Err(e) if waited >= 30 => return Err(e),
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_secs(2));
                waited += 2;
            }
        }
    }
}

fn server_alive(host: &str) -> bool {
    Command::new("spacetime")
        .args(["server", "ping", host])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Wipe-free publish to the default (local) server target.
fn publish(module: &str) -> Result<(), String> {
    let status = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            "server/publish.ps1",
            "-Module",
            module,
        ])
        .status()
        .map_err(|e| format!("failed to run server/publish.ps1: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("publish failed ({status})"))
    }
}
