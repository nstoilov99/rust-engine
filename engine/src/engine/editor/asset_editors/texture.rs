//! Texture editor — stub showing image metadata and import settings.
//! Save is disabled; apply-on-save lands in a future task.

use std::path::PathBuf;

/// Per-window editor state for a texture asset.
pub struct TextureEditorState {
    pub path: PathBuf,
    pub dimensions: Option<[u32; 2]>,
    pub settings: TextureImportSettings,
}

/// Editor-side import settings (not persisted in v1).
#[derive(Default, Clone)]
pub struct TextureImportSettings {
    pub srgb: bool,
    pub mipmaps: bool,
    pub compression: TextureCompression,
    pub wrap_mode: WrapMode,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum TextureCompression {
    #[default]
    None,
    BC1,
    BC3,
    BC5,
    BC7,
}

impl TextureCompression {
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::BC1 => "BC1",
            Self::BC3 => "BC3",
            Self::BC5 => "BC5",
            Self::BC7 => "BC7",
        }
    }

    pub const ALL: &'static [Self] = &[Self::None, Self::BC1, Self::BC3, Self::BC5, Self::BC7];
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum WrapMode {
    #[default]
    Repeat,
    Clamp,
    Mirror,
}

impl WrapMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Repeat => "Repeat",
            Self::Clamp => "Clamp",
            Self::Mirror => "Mirror",
        }
    }

    pub const ALL: &'static [Self] = &[Self::Repeat, Self::Clamp, Self::Mirror];
}

impl TextureEditorState {
    /// Create editor state by probing image dimensions from disk.
    pub fn open(path: PathBuf) -> Self {
        let dimensions = probe_image_dimensions(&path);
        Self {
            path,
            dimensions,
            settings: TextureImportSettings::default(),
        }
    }
}

fn probe_image_dimensions(path: &std::path::Path) -> Option<[u32; 2]> {
    // Try reading just the header for dimensions
    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file);
    let img_reader = image::ImageReader::new(reader)
        .with_guessed_format()
        .ok()?;
    let (w, h) = img_reader.into_dimensions().ok()?;
    Some([w, h])
}

/// Render the texture editor UI.
pub fn show_texture_editor(ui: &mut egui::Ui, state: &mut TextureEditorState) {
    ui.heading("Texture Editor");
    ui.label(
        egui::RichText::new(state.path.display().to_string())
            .weak()
            .small(),
    );
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // --- Preview placeholder ---
            let preview_rect = ui.allocate_space(egui::vec2(256.0, 256.0));
            ui.painter().rect_filled(
                egui::Rect::from_min_size(preview_rect.1.min, egui::vec2(256.0, 256.0)),
                4.0,
                egui::Color32::from_gray(35),
            );
            ui.painter().text(
                preview_rect.1.min + egui::vec2(128.0, 128.0),
                egui::Align2::CENTER_CENTER,
                "Preview lands when\nAssetPreviewRegistry ships\nin Task 39.4 Step 14",
                egui::FontId::proportional(11.0),
                egui::Color32::from_gray(120),
            );
            ui.add_space(8.0);

            // --- Metadata ---
            ui.label(egui::RichText::new("Metadata").strong());
            ui.separator();
            if let Some([w, h]) = state.dimensions {
                ui.label(format!("Dimensions: {} \u{00D7} {}", w, h));
            } else {
                ui.label("Dimensions: (could not read)");
            }
            ui.add_space(8.0);

            // --- Import Settings ---
            ui.label(egui::RichText::new("Import Settings").strong());
            ui.separator();
            ui.checkbox(&mut state.settings.srgb, "sRGB");
            ui.checkbox(&mut state.settings.mipmaps, "Generate Mipmaps");

            // Compression dropdown
            egui::ComboBox::from_label("Compression")
                .selected_text(state.settings.compression.label())
                .show_ui(ui, |ui| {
                    for &c in TextureCompression::ALL {
                        ui.selectable_value(&mut state.settings.compression, c, c.label());
                    }
                });

            // Wrap mode dropdown
            egui::ComboBox::from_label("Wrap Mode")
                .selected_text(state.settings.wrap_mode.label())
                .show_ui(ui, |ui| {
                    for &w in WrapMode::ALL {
                        ui.selectable_value(&mut state.settings.wrap_mode, w, w.label());
                    }
                });
        });

    ui.separator();

    // --- Footer: Save disabled ---
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Texture").weak().small());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let save_btn = ui.add_enabled(false, egui::Button::new("Save"));
            save_btn.on_disabled_hover_text("Apply-on-save lands in a future task");
        });
    });
}
