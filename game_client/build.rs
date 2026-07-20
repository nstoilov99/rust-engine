use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Workspace-root .git — package-relative paths would never exist here.
    // logs/HEAD changes on every commit — plain HEAD doesn't while the
    // branch stays the same.
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");
    println!("cargo:rerun-if-changed=../.git/logs/HEAD");

    // M9 D4: same format as the server module stamps (short hash + `-dirty`),
    // so the connect-time build-id comparison is meaningful.
    let git_hash = match git(&["rev-parse", "--short", "HEAD"]).filter(|h| !h.is_empty()) {
        Some(hash) => {
            let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
            if dirty { format!("{hash}-dirty") } else { hash }
        }
        None => "unknown".to_string(),
    };

    println!("cargo:rustc-env=GIT_HASH={git_hash}");
    println!(
        "cargo:rustc-env=BUILD_PROFILE={}",
        std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string())
    );
}
