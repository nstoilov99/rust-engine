//! Audio editor — stub showing audio metadata, play/stop controls.
//! Save is disabled; apply-on-save lands in a future task.

use std::path::PathBuf;

/// Per-window editor state for an audio asset.
pub struct AudioEditorState {
    pub path: PathBuf,
    pub metadata: AudioMetadata,
    pub settings: AudioImportSettings,
}

/// Probed audio metadata.
pub struct AudioMetadata {
    pub duration_secs: f32,
    pub channels: u16,
    pub sample_rate: u32,
}

impl Default for AudioMetadata {
    fn default() -> Self {
        Self {
            duration_secs: 0.0,
            channels: 0,
            sample_rate: 0,
        }
    }
}

/// Editor-side import settings (not persisted in v1).
#[derive(Default, Clone)]
pub struct AudioImportSettings {
    pub loop_audio: bool,
    pub gain_db: f32,
    pub spatial: bool,
    pub bus: AudioBus,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum AudioBus {
    #[default]
    SFX,
    Music,
    Voice,
    UI,
}

impl AudioBus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SFX => "SFX",
            Self::Music => "Music",
            Self::Voice => "Voice",
            Self::UI => "UI",
        }
    }
    pub const ALL: &'static [Self] = &[Self::SFX, Self::Music, Self::Voice, Self::UI];
}

impl AudioEditorState {
    /// Create editor state by reading audio file metadata.
    pub fn open(path: PathBuf) -> Self {
        let metadata = probe_audio_metadata(&path);
        Self {
            path,
            metadata,
            settings: AudioImportSettings::default(),
        }
    }
}

fn probe_audio_metadata(path: &std::path::Path) -> AudioMetadata {
    // Try to read basic audio metadata using kira's StaticSoundData
    // or a lightweight probe. For v1, we'll show what we can determine.
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Use file size as a rough indicator if we can't probe properly
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    // Estimate based on common formats (very rough for v1)
    let (channels, sample_rate) = match ext.as_str() {
        "wav" => (2, 44100),
        "ogg" => (2, 44100),
        "mp3" => (2, 44100),
        "flac" => (2, 44100),
        _ => (2, 44100),
    };

    // Rough duration estimate: file_size / (sample_rate * channels * bytes_per_sample)
    let bytes_per_sample = 2u64; // 16-bit
    let estimated_duration = if file_size > 0 && sample_rate > 0 {
        file_size as f32 / (sample_rate as f32 * channels as f32 * bytes_per_sample as f32)
    } else {
        0.0
    };

    AudioMetadata {
        duration_secs: estimated_duration,
        channels,
        sample_rate,
    }
}

/// Render the audio editor UI.
///
/// Note: Play/Stop functionality requires access to `AudioEngine` which lives
/// in the game world. For v1, the editor shows the controls but playback is
/// wired through the secondary window render loop in main.rs.
pub fn show_audio_editor(ui: &mut egui::Ui, state: &mut AudioEditorState, is_playing: bool) -> AudioEditorAction {
    let mut action = AudioEditorAction::None;

    ui.heading("Audio Editor");
    ui.label(
        egui::RichText::new(state.path.display().to_string())
            .weak()
            .small(),
    );
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // --- Waveform placeholder ---
            let waveform_rect = ui.allocate_space(egui::vec2(ui.available_width().min(800.0), 80.0));
            ui.painter().rect_filled(
                egui::Rect::from_min_size(waveform_rect.1.min, egui::vec2(waveform_rect.1.size().x, 80.0)),
                4.0,
                egui::Color32::from_gray(30),
            );
            // Draw a simple center line for the waveform placeholder
            let center_y = waveform_rect.1.min.y + 40.0;
            ui.painter().line_segment(
                [
                    egui::pos2(waveform_rect.1.min.x, center_y),
                    egui::pos2(waveform_rect.1.min.x + waveform_rect.1.size().x, center_y),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
            );
            ui.painter().text(
                waveform_rect.1.min + egui::vec2(waveform_rect.1.size().x / 2.0, 40.0),
                egui::Align2::CENTER_CENTER,
                "Waveform preview",
                egui::FontId::proportional(11.0),
                egui::Color32::from_gray(90),
            );
            ui.add_space(4.0);

            // --- Controls ---
            ui.horizontal(|ui| {
                if is_playing {
                    if ui.button("\u{25A0} Stop").clicked() {
                        action = AudioEditorAction::Stop;
                    }
                } else if ui.button("\u{25B6} Play").clicked() {
                    action = AudioEditorAction::Play;
                }
            });
            ui.add_space(8.0);

            // --- Metadata ---
            ui.label(egui::RichText::new("Metadata").strong());
            ui.separator();
            ui.label(format!("Duration: {:.1}s", state.metadata.duration_secs));
            ui.label(format!(
                "Channels: {} ({})",
                state.metadata.channels,
                if state.metadata.channels >= 2 {
                    "Stereo"
                } else {
                    "Mono"
                }
            ));
            ui.label(format!("Sample Rate: {} Hz", state.metadata.sample_rate));
            ui.add_space(8.0);

            // --- Import Settings ---
            ui.label(egui::RichText::new("Import Settings").strong());
            ui.separator();
            ui.checkbox(&mut state.settings.loop_audio, "Loop");
            ui.horizontal(|ui| {
                ui.label("Gain (dB):");
                ui.add(egui::Slider::new(
                    &mut state.settings.gain_db,
                    -60.0..=12.0,
                ));
            });
            ui.checkbox(&mut state.settings.spatial, "Spatial (3D)");
            egui::ComboBox::from_label("Bus")
                .selected_text(state.settings.bus.label())
                .show_ui(ui, |ui| {
                    for &b in AudioBus::ALL {
                        ui.selectable_value(&mut state.settings.bus, b, b.label());
                    }
                });
        });

    ui.separator();

    // --- Footer: Save disabled ---
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Audio").weak().small());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let save_btn = ui.add_enabled(false, egui::Button::new("Save"));
            save_btn.on_disabled_hover_text("Apply-on-save lands in a future task");
        });
    });

    action
}

/// Actions emitted by the audio editor UI.
pub enum AudioEditorAction {
    None,
    Play,
    Stop,
}
