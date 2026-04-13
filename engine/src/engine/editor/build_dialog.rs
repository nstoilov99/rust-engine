//! Build Game support for the editor
//!
//! Spawns `cargo build` as a child process, pipes stdout/stderr,
//! and shows progress in the File > Build Game submenu.

use super::console::LogMessage;
use std::path::PathBuf;
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
}

impl Default for BuildSettings {
    fn default() -> Self {
        Self {
            platform: BuildPlatform::Windows,
            profile: BuildProfile::Release,
            output_dir: "build/export".to_string(),
        }
    }
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

        self.state = BuildState::Building;
        self.start_time = Some(Instant::now());

        if let Ok(mut log) = self.build_log.lock() {
            log.push(format!(
                "Starting {} build for {}...",
                self.settings.profile.label(),
                self.settings.platform.label()
            ));
        }

        let profile = self.settings.profile;
        let build_log = self.build_log.clone();

        self.build_thread = Some(std::thread::spawn(move || {
            run_cargo_build(profile, build_log)
        }));
    }

    fn start_copy_thread(&mut self) {
        let output_dir = PathBuf::from(&self.settings.output_dir);
        let profile = self.settings.profile;
        let platform = self.settings.platform;
        let build_log = self.build_log.clone();

        self.copy_thread = Some(std::thread::spawn(move || {
            copy_build_output(profile, platform, &output_dir, build_log)
        }));
    }
}

fn run_cargo_build(
    profile: BuildProfile,
    build_log: Arc<Mutex<Vec<String>>>,
) -> Result<u64, String> {
    let args = profile.cargo_args();

    let mut child = Command::new("cargo")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn cargo: {}", e))?;

    // Drain stdout in a thread
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

    // Drain stderr in a thread
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
        .map_err(|e| format!("Failed to wait for cargo: {}", e))?;

    let _ = stdout_thread.join();
    let _ = stderr_thread.join();

    if !status.success() {
        return Err(format!("cargo exited with code {:?}", status.code()));
    }

    let exe_path = PathBuf::from(profile.output_dir()).join(format!("{}.exe", BIN_NAME));
    let binary_size = std::fs::metadata(&exe_path).map(|m| m.len()).unwrap_or(0);

    Ok(binary_size)
}

fn copy_build_output(
    profile: BuildProfile,
    platform: BuildPlatform,
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

    Ok(binary_size)
}
