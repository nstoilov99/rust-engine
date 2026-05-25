//! Typography and font registration for the editor theme.
//!
//! Defines the type scale and registers fonts with egui.
//! Uses egui's built-in font families as the base, with the option to
//! load custom fonts (Inter, JetBrains Mono) when bundled under
//! `engine/assets/fonts/`.

use egui::{FontDefinitions, FontFamily, FontId, TextStyle};
use std::collections::BTreeMap;

/// Typography configuration — font sizes for each semantic text role.
#[derive(Clone, Debug)]
pub struct Typography {
    pub heading_large: f32,
    pub heading: f32,
    pub body: f32,
    pub caption: f32,
    pub mono: f32,
}

impl Typography {
    /// Comfortable density baseline.
    pub fn comfortable() -> Self {
        Self {
            heading_large: 18.0,
            heading: 14.0,
            body: 12.0,
            caption: 10.5,
            mono: 12.0,
        }
    }

    /// Returns a scaled copy by the given factor.
    pub fn scaled(&self, factor: f32) -> Self {
        Self {
            heading_large: self.heading_large * factor,
            heading: self.heading * factor,
            body: self.body * factor,
            caption: self.caption * factor,
            mono: self.mono * factor,
        }
    }

    /// Build the egui `TextStyle` -> `FontId` mapping for this typography.
    pub fn text_styles(&self) -> BTreeMap<TextStyle, FontId> {
        let mut map = BTreeMap::new();
        map.insert(
            TextStyle::Heading,
            FontId::new(self.heading_large, FontFamily::Proportional),
        );
        map.insert(
            TextStyle::Body,
            FontId::new(self.body, FontFamily::Proportional),
        );
        map.insert(
            TextStyle::Small,
            FontId::new(self.caption, FontFamily::Proportional),
        );
        map.insert(
            TextStyle::Button,
            FontId::new(self.body, FontFamily::Proportional),
        );
        map.insert(
            TextStyle::Monospace,
            FontId::new(self.mono, FontFamily::Monospace),
        );
        // Custom named styles
        map.insert(
            TextStyle::Name("heading".into()),
            FontId::new(self.heading, FontFamily::Proportional),
        );
        map.insert(
            TextStyle::Name("heading_large".into()),
            FontId::new(self.heading_large, FontFamily::Proportional),
        );
        map.insert(
            TextStyle::Name("caption".into()),
            FontId::new(self.caption, FontFamily::Proportional),
        );
        map
    }

    /// Build font definitions, optionally loading custom fonts from disk.
    ///
    /// If custom fonts are not found, falls back to egui's built-in fonts.
    pub fn font_definitions() -> FontDefinitions {
        let mut fonts = FontDefinitions::default();

        // Try loading Inter Regular
        if let Ok(data) = std::fs::read("engine/assets/fonts/Inter-Regular.ttf") {
            fonts.font_data.insert(
                "Inter-Regular".to_owned(),
                egui::FontData::from_owned(data).into(),
            );
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, "Inter-Regular".to_owned());
        }

        // Try loading Inter Bold
        if let Ok(data) = std::fs::read("engine/assets/fonts/Inter-Bold.ttf") {
            fonts.font_data.insert(
                "Inter-Bold".to_owned(),
                egui::FontData::from_owned(data).into(),
            );
            // Register as a named family for bold usage
            fonts
                .families
                .entry(FontFamily::Name("Bold".into()))
                .or_default()
                .push("Inter-Bold".to_owned());
        }

        // Try loading JetBrains Mono
        if let Ok(data) = std::fs::read("engine/assets/fonts/JetBrainsMono-Regular.ttf") {
            fonts.font_data.insert(
                "JetBrainsMono".to_owned(),
                egui::FontData::from_owned(data).into(),
            );
            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .insert(0, "JetBrainsMono".to_owned());
        }

        fonts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comfortable_baseline_sizes() {
        let typo = Typography::comfortable();
        assert_eq!(typo.heading_large, 18.0);
        assert_eq!(typo.body, 12.0);
        assert_eq!(typo.caption, 10.5);
    }

    #[test]
    fn scaling_applies_factor() {
        let base = Typography::comfortable();
        let scaled = base.scaled(0.93);
        assert!((scaled.body - 12.0 * 0.93).abs() < 0.01);
    }

    #[test]
    fn text_styles_contain_all_defaults() {
        let typo = Typography::comfortable();
        let styles = typo.text_styles();
        assert!(styles.contains_key(&TextStyle::Heading));
        assert!(styles.contains_key(&TextStyle::Body));
        assert!(styles.contains_key(&TextStyle::Small));
        assert!(styles.contains_key(&TextStyle::Button));
        assert!(styles.contains_key(&TextStyle::Monospace));
    }
}
