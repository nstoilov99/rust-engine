//! Build Game support for the editor
//!
//! Spawns `cargo build` as a child process, pipes stdout/stderr,
//! and shows progress in the File > Build Game submenu.

use super::console::LogMessage;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Binary name produced by the build
pub const BIN_NAME: &str = "game";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildPlatform {
    Windows,
}

impl BuildPlatform {
    pub fn label(&self) -> &'static str {
        match self {
            BuildPlatform::Windows => "Windows",
        }
    }

    pub fn exe_name(&self) -> String {
        format!("{}.exe", BIN_NAME)
    }
}

/// UE-style build target (M9 D6). Same binary either way for client
/// targets — the target is configuration; MP Server is a module publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BuildTarget {
    /// Exe + pak, no net config (deletes a stale one).
    Standalone,
    /// Exe + pak + `net_config.ron` (auto_connect) next to the exe.
    MpClient,
    /// No exe at all: runs `server/publish.ps1` (WASM module publish).
    MpServer,
}

impl BuildTarget {
    pub fn label(&self) -> &'static str {
        match self {
            BuildTarget::Standalone => "Standalone",
            BuildTarget::MpClient => "MP Client",
            BuildTarget::MpServer => "MP Server",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProfile {
    Release,
    Shipping,
}

impl BuildProfile {
    pub fn label(&self) -> &'static str {
        match self {
            BuildProfile::Release => "Release",
            BuildProfile::Shipping => "Shipping",
        }
    }

    pub fn cargo_args(&self) -> Vec<&'static str> {
        match self {
            BuildProfile::Release => vec!["build", "--release", "--bin", BIN_NAME],
            BuildProfile::Shipping => vec!["build", "--profile", "shipping", "--bin", BIN_NAME],
        }
    }

    pub fn output_dir(&self) -> &'static str {
        match self {
            BuildProfile::Release => "target/release",
            BuildProfile::Shipping => "target/shipping",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildState {
    Idle,
    Building,
    CopyingContent,
    Success { binary_size: u64 },
    Failed { error: String },
}

pub struct BuildSettings {
    pub platform: BuildPlatform,
    pub profile: BuildProfile,
    pub output_dir: String,
    pub target: BuildTarget,
    /// MP client bundle destination / MP server publish destination.
    /// Defaults must match `game_client/src/net.rs`.
    pub server_uri: String,
    pub module: String,
}

impl Default for BuildSettings {
    fn default() -> Self {
        Self {
            platform: BuildPlatform::Windows,
            profile: BuildProfile::Release,
            output_dir: "build/export".to_string(),
            target: BuildTarget::Standalone,
            server_uri: "http://127.0.0.1:3000".to_string(),
            module: "rust-engine-dev".to_string(),
        }
    }
}

/// Same rules as the export/publish scripts (D2/D3).
fn validate_mp_settings(server_uri: &str, module: &str) -> Result<(), String> {
    let valid_module = !module.is_empty()
        && !module.starts_with('-')
        && !module.ends_with('-')
        && !module.contains("--")
        && module
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid_module {
        return Err(format!(
            "invalid module name '{module}' (lowercase alphanumeric, single '-' separators)"
        ));
    }
    if !server_uri.starts_with("http://") && !server_uri.starts_with("https://") {
        return Err(format!("invalid server URI '{server_uri}' (must be http(s)://...)"));
    }
    Ok(())
}

pub struct BuildDialog {
    pub settings: BuildSettings,
    pub state: BuildState,
    pub build_log: Arc<Mutex<Vec<String>>>,
    pub start_time: Option<Instant>,
    build_thread: Option<std::thread::JoinHandle<Result<u64, String>>>,
    copy_thread: Option<std::thread::JoinHandle<Result<u64, String>>>,
}

impl Default for BuildDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildDialog {
    pub fn new() -> Self {
        Self {
            settings: BuildSettings::default(),
            state: BuildState::Idle,
            build_log: Arc::new(Mutex::new(Vec::new())),
            start_time: None,
            build_thread: None,
            copy_thread: None,
        }
    }

    /// Drain new log lines from background threads into `LogMessage`s and
    /// check if a background build/copy has completed.
    pub fn poll(&mut self) -> Vec<LogMessage> {
        let mut messages = Vec::new();

        if let Ok(mut log) = self.build_log.lock() {
            for line in log.drain(..) {
                messages.push(LogMessage::info(format!("[build] {}", line)));
            }
        }

        // Check build thread
        if let Some(handle) = &self.build_thread {
            if handle.is_finished() {
                let handle = self.build_thread.take().expect("checked is_finished");
                match handle.join() {
                    Ok(Ok(binary_size)) if self.settings.target == BuildTarget::MpServer => {
                        messages.push(LogMessage::info("[build] Module publish complete."));
                        self.state = BuildState::Success { binary_size };
                    }
                    Ok(Ok(_binary_size)) => {
                        messages.push(LogMessage::info(
                            "[build] Build succeeded. Copying files...",
                        ));
                        self.state = BuildState::CopyingContent;
                        self.start_copy_thread();
                    }
                    Ok(Err(e)) => {
                        messages.push(LogMessage::error(format!("[build] BUILD FAILED: {}", e)));
                        self.state = BuildState::Failed { error: e };
                    }
                    Err(_) => {
                        messages.push(LogMessage::error("[build] Build thread panicked"));
                        self.state = BuildState::Failed {
                            error: "Build thread panicked".to_string(),
                        };
                    }
                }
            }
        }

        // Check copy thread
        if let Some(handle) = &self.copy_thread {
            if handle.is_finished() {
                let handle = self.copy_thread.take().expect("checked is_finished");
                match handle.join() {
                    Ok(Ok(binary_size)) => {
                        messages.push(LogMessage::info(format!(
                            "[build] Export complete! Binary size: {:.1} MB",
                            binary_size as f64 / (1024.0 * 1024.0)
                        )));
                        self.state = BuildState::Success { binary_size };
                    }
                    Ok(Err(e)) => {
                        messages.push(LogMessage::error(format!("[build] Copy failed: {}", e)));
                        self.state = BuildState::Failed { error: e };
                    }
                    Err(_) => {
                        messages.push(LogMessage::error("[build] Copy thread panicked"));
                        self.state = BuildState::Failed {
                            error: "Copy thread panicked".to_string(),
                        };
                    }
                }
            }
        }

        messages
    }

    pub fn start_build(&mut self) {
        if self.state == BuildState::Building || self.state == BuildState::CopyingContent {
            return;
        }

        let target = self.settings.target;
        if target != BuildTarget::Standalone {
            if let Err(e) = validate_mp_settings(&self.settings.server_uri, &self.settings.module)
            {
                if let Ok(mut log) = self.build_log.lock() {
                    log.push(format!("ERROR: {e}"));
                }
                self.state = BuildState::Failed { error: e };
                return;
            }
        }

        self.state = BuildState::Building;
        self.start_time = Some(Instant::now());
        let build_log = self.build_log.clone();

        if target == BuildTarget::MpServer {
            if let Ok(mut log) = self.build_log.lock() {
                log.push(format!(
                    "Publishing module '{}' to {}...",
                    self.settings.module, self.settings.server_uri
                ));
            }
            let server_uri = self.settings.server_uri.clone();
            let module = self.settings.module.clone();
            self.build_thread = Some(std::thread::spawn(move || {
                run_publish(server_uri, module, build_log)
            }));
            return;
        }

        if let Ok(mut log) = self.build_log.lock() {
            log.push(format!(
                "Starting {} build for {} ({})...",
                self.settings.profile.label(),
                self.settings.platform.label(),
                target.label()
            ));
        }

        let profile = self.settings.profile;
        self.build_thread = Some(std::thread::spawn(move || {
            run_cargo_build(profile, build_log)
        }));
    }

    fn start_copy_thread(&mut self) {
        let output_dir = PathBuf::from(&self.settings.output_dir);
        let profile = self.settings.profile;
        let platform = self.settings.platform;
        let target = self.settings.target;
        let server_uri = self.settings.server_uri.clone();
        let module = self.settings.module.clone();
        let build_log = self.build_log.clone();

        self.copy_thread = Some(std::thread::spawn(move || {
            copy_build_output(
                profile, platform, target, &server_uri, &module, &output_dir, build_log,
            )
        }));
    }
}

/// Stream a child's stdout/stderr into the build log and wait for exit.
fn stream_child(
    mut child: std::process::Child,
    build_log: &Arc<Mutex<Vec<String>>>,
    what: &str,
) -> Result<std::process::ExitStatus, String> {
    let stdout = child.stdout.take();
    let log_stdout = build_log.clone();
    let stdout_thread = std::thread::spawn(move || {
        if let Some(stdout) = stdout {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if let Ok(mut log) = log_stdout.lock() {
                    log.push(line);
                }
            }
        }
    });

    let stderr = child.stderr.take();
    let log_stderr = build_log.clone();
    let stderr_thread = std::thread::spawn(move || {
        if let Some(stderr) = stderr {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                if let Ok(mut log) = log_stderr.lock() {
                    log.push(line);
                }
            }
        }
    });

    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for {what}: {e}"))?;
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();
    Ok(status)
}

fn run_cargo_build(
    profile: BuildProfile,
    build_log: Arc<Mutex<Vec<String>>>,
) -> Result<u64, String> {
    let args = profile.cargo_args();

    let child = Command::new("cargo")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn cargo: {}", e))?;

    let status = stream_child(child, &build_log, "cargo")?;
    if !status.success() {
        return Err(format!("cargo exited with code {:?}", status.code()));
    }

    let exe_path = PathBuf::from(profile.output_dir()).join(format!("{}.exe", BIN_NAME));
    let binary_size = std::fs::metadata(&exe_path).map(|m| m.len()).unwrap_or(0);

    Ok(binary_size)
}

/// MP Server target (M9 D6): no exe build — publish the WASM module via
/// `server/publish.ps1` (which also stamps the build id, D4). The scripts
/// stay the authority; this just streams them into the dialog. A missing
/// `spacetime` CLI surfaces as a normal build-log error.
fn run_publish(
    server_uri: String,
    module: String,
    build_log: Arc<Mutex<Vec<String>>>,
) -> Result<u64, String> {
    let child = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            "server/publish.ps1",
            "-Server",
            &server_uri,
            "-Module",
            &module,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn powershell for publish.ps1: {e}"))?;

    let status = stream_child(child, &build_log, "publish.ps1")?;
    if !status.success() {
        return Err(format!(
            "publish failed (exit {:?}) — is the spacetime CLI on PATH and the server reachable?",
            status.code()
        ));
    }

    // Report the module size as the "binary" (newest wasm artifact).
    let wasm_dir = PathBuf::from("server/game_module/target/wasm32-unknown-unknown/release");
    let wasm_size = std::fs::read_dir(&wasm_dir)
        .ok()
        .and_then(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "wasm"))
                .filter_map(|e| e.metadata().ok())
                .filter_map(|m| m.modified().ok().map(|t| (t, m.len())))
                .max_by_key(|(t, _)| *t)
        })
        .map(|(_, len)| len)
        .unwrap_or(0);
    Ok(wasm_size)
}

fn copy_build_output(
    profile: BuildProfile,
    platform: BuildPlatform,
    target: BuildTarget,
    server_uri: &str,
    module: &str,
    output_dir: &PathBuf,
    build_log: Arc<Mutex<Vec<String>>>,
) -> Result<u64, String> {
    let build_dir = PathBuf::from(profile.output_dir());

    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create output dir: {}", e))?;

    // Copy executable
    let exe_name = platform.exe_name();
    let src_exe = build_dir.join(&exe_name);
    let dst_exe = output_dir.join(&exe_name);

    if !src_exe.exists() {
        return Err(format!("Binary not found: {}", src_exe.display()));
    }

    std::fs::copy(&src_exe, &dst_exe).map_err(|e| format!("Failed to copy binary: {}", e))?;

    let binary_size = std::fs::metadata(&dst_exe).map(|m| m.len()).unwrap_or(0);

    if let Ok(mut log) = build_log.lock() {
        log.push(format!(
            "Copied {} ({:.1} MB)",
            exe_name,
            binary_size as f64 / (1024.0 * 1024.0)
        ));
    }

    // Copy DLLs
    if let Ok(entries) = std::fs::read_dir(&build_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "dll").unwrap_or(false) {
                let dst = output_dir.join(path.file_name().expect("has filename"));
                if let Err(e) = std::fs::copy(&path, &dst) {
                    if let Ok(mut log) = build_log.lock() {
                        log.push(format!("Warning: failed to copy {}: {}", path.display(), e));
                    }
                } else if let Ok(mut log) = build_log.lock() {
                    log.push(format!(
                        "Copied {}",
                        path.file_name().expect("has filename").to_string_lossy()
                    ));
                }
            }
        }
    }

    // Pack content into game.pak
    let content_src = PathBuf::from("content");
    let pak_dst = output_dir.join("game.pak");

    if content_src.is_dir() {
        cook_stale_collision(&content_src, &build_log)?;

        if let Ok(mut log) = build_log.lock() {
            log.push("Packing content/ into game.pak...".to_string());
        }

        match crate::engine::assets::pak::pack_directory(&content_src, &pak_dst) {
            Ok(pak_size) => {
                if let Ok(mut log) = build_log.lock() {
                    log.push(format!(
                        "Created game.pak ({:.1} MB)",
                        pak_size as f64 / (1024.0 * 1024.0)
                    ));
                }
            }
            Err(e) => {
                return Err(format!("Failed to pack content: {}", e));
            }
        }
    } else if let Ok(mut log) = build_log.lock() {
        log.push("Warning: content/ directory not found".to_string());
    }

    // Net config (M9 D2/D6): targets own their marker files — standalone
    // deletes a stale config so re-exporting over an mp-client bundle can't
    // auto-connect.
    let net_cfg = output_dir.join("net_config.ron");
    match target {
        BuildTarget::MpClient => {
            let contents = format!(
                "NetConfig(\n    host: \"{server_uri}\",\n    module: \"{module}\",\n    auto_connect: true,\n)\n"
            );
            std::fs::write(&net_cfg, contents)
                .map_err(|e| format!("Failed to write net_config.ron: {e}"))?;
            if let Ok(mut log) = build_log.lock() {
                log.push(format!("Wrote net_config.ron -> {server_uri} / {module}"));
            }
        }
        BuildTarget::Standalone => {
            if net_cfg.exists() {
                let _ = std::fs::remove_file(&net_cfg);
                if let Ok(mut log) = build_log.lock() {
                    log.push("Removed stale net_config.ron (standalone target)".to_string());
                }
            }
        }
        // MP Server never reaches the copy phase.
        BuildTarget::MpServer => {}
    }

    Ok(binary_size)
}

/// Re-cook collision for every scene whose cook is stale before packing, so
/// the pak never ships outdated chunks. Cook failures fail the build.
fn cook_stale_collision(
    content_src: &Path,
    build_log: &Arc<Mutex<Vec<String>>>,
) -> Result<(), String> {
    use crate::engine::collision::output;

    let scenes_dir = content_src.join("scenes");
    let Ok(entries) = std::fs::read_dir(&scenes_dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e == "scene") != Some(true) {
            continue;
        }
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        let scene_relative = format!("scenes/{name}");
        if output::cook_is_current(&scene_relative) {
            continue;
        }
        let (chunks, warnings) = output::cook_scene_headless(&scene_relative)
            .map_err(|e| format!("Collision cook failed for '{scene_relative}': {e}"))?;
        if let Ok(mut log) = build_log.lock() {
            log.push(format!(
                "Cooked collision for '{scene_relative}': {chunks} chunk(s)"
            ));
            for w in warnings {
                log.push(format!("  cook warning: {w}"));
            }
        }
    }
    Ok(())
}
