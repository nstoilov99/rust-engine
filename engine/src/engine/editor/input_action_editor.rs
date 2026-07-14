//! Dedicated editor for individual `.inputaction.ron` files.
//!
//! Opens in a separate OS window when a user double-clicks an input action
//! asset in the asset browser — similar to Unreal Engine's InputAction editor.
//! The old `show_ui` rendering fn was removed; the crusty analog lives in
//! `input_editors_crusty::input_action_editor_panel`.

use crate::engine::input::enhanced_action::InputActionDefinition;
use crate::engine::input::enhanced_serialization;
use crate::engine::input::value::InputValueType;
use std::collections::HashMap;
use std::path::PathBuf;

/// State for one open input action editor window.
pub struct InputActionEditorState {
    pub definition: InputActionDefinition,
    pub dirty: bool,
    pub file_path: PathBuf,
    pub open: bool,
    pub status_message: Option<(String, f64)>,
}

/// Manages all open input action editor windows.
#[derive(Default)]
pub struct InputActionEditor {
    pub open_actions: HashMap<String, InputActionEditorState>,
}

impl InputActionEditor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open an input action for editing. Loads from file if not already open.
    /// Returns the editor key (file path string) for use with PendingWindowRequest.
    pub fn open(&mut self, file_path: PathBuf) -> String {
        let key = file_path.to_string_lossy().to_string();
        if !self.open_actions.contains_key(&key) {
            let definition = enhanced_serialization::load_input_action(&file_path)
                .unwrap_or_else(|| InputActionDefinition::new("unnamed", InputValueType::Digital));
            self.open_actions.insert(
                key.clone(),
                InputActionEditorState {
                    definition,
                    dirty: false,
                    file_path,
                    open: true,
                    status_message: None,
                },
            );
        }
        key
    }

    pub fn save_state(
        state: &mut InputActionEditorState,
    ) -> Result<(), Box<dyn std::error::Error>> {
        enhanced_serialization::save_input_action(&state.definition, &state.file_path)?;
        state.dirty = false;
        Ok(())
    }
}
