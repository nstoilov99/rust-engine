//! Dedicated editor for individual `.mappingcontext.ron` files.
//!
//! Opens in a separate OS window when a user double-clicks a mapping context
//! asset in the asset browser — similar to Unreal Engine's InputMappingContext editor.
//! The egui `show_ui` rendering fn was removed; the crusty analog lives in
//! `input_editors_crusty::input_context_editor_panel`. The
//! `listen_start_modifiers: egui::Modifiers` field is preserved on the state
//! struct because the crusty path still uses it (type migration is deferred).

use crate::engine::input::action::InputSource;
use crate::engine::input::enhanced_action::MappingContext;
use crate::engine::input::enhanced_serialization;
use std::collections::HashMap;
use std::path::PathBuf;

/// State for one open mapping context editor window.
pub struct InputContextEditorState {
    pub context: MappingContext,
    pub dirty: bool,
    pub file_path: PathBuf,
    pub open: bool,
    pub status_message: Option<(String, f64)>,
    /// When set, the editor is listening for input on this (entry_idx, binding_idx).
    pub listening_binding: Option<(usize, usize)>,
    /// Modifier state captured when listening started, to detect new modifier-key presses.
    pub listen_start_modifiers: egui::Modifiers,
    /// Input detected from external sources (gamepad). Set by the main loop.
    pub pending_external_input: Option<InputSource>,
}

/// Manages all open mapping context editor windows.
#[derive(Default)]
pub struct InputContextEditor {
    pub open_contexts: HashMap<String, InputContextEditorState>,
    /// Action names discovered from .inputaction.ron files in content/.
    pub available_actions: Vec<String>,
}

impl InputContextEditor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Refresh the list of available action names (call after asset rescan).
    pub fn refresh_action_names(&mut self, content_dir: &std::path::Path) {
        self.available_actions = enhanced_serialization::scan_action_names(content_dir);
    }

    /// Open a mapping context for editing. Loads from file if not already open.
    /// Returns the editor key (file path string) for use with PendingWindowRequest.
    pub fn open(&mut self, file_path: PathBuf) -> String {
        let key = file_path.to_string_lossy().to_string();
        if !self.open_contexts.contains_key(&key) {
            let context = enhanced_serialization::load_mapping_context(&file_path)
                .unwrap_or_else(|| MappingContext::new("unnamed", 0));
            self.open_contexts.insert(
                key.clone(),
                InputContextEditorState {
                    context,
                    dirty: false,
                    file_path,
                    open: true,
                    status_message: None,
                    listening_binding: None,
                    listen_start_modifiers: egui::Modifiers::NONE,
                    pending_external_input: None,
                },
            );
        }
        key
    }

    pub fn save_state(
        state: &mut InputContextEditorState,
    ) -> Result<(), Box<dyn std::error::Error>> {
        enhanced_serialization::save_mapping_context(&state.context, &state.file_path)?;
        state.dirty = false;
        Ok(())
    }
}
