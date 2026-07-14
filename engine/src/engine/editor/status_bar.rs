//! Editor status bar state.
//!
//! The old `render_status_bar` fn was removed; the crusty analog lives in
//! `status_bar_crusty`.

#[derive(Debug, Clone)]
pub struct StatusBarState {
    pub left_text: String,
    pub right_text: String,
}

impl Default for StatusBarState {
    fn default() -> Self {
        Self {
            left_text: "Ready".to_string(),
            right_text: String::new(),
        }
    }
}

impl StatusBarState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_left(&mut self, text: impl Into<String>) {
        self.left_text = text.into();
    }

    pub fn set_right(&mut self, text: impl Into<String>) {
        self.right_text = text.into();
    }
}
