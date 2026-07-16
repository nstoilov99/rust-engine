//! Central modal dialog stack for editor workflows.

use crate::engine::editor::command_palette::EditorAction;
use crusty_gui::id::Id;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogButtons {
    Ok,
    OkCancel,
    SaveDiscardCancel,
}

#[derive(Debug, Clone)]
pub struct DialogActions {
    pub primary: Option<EditorAction>,
    pub secondary: Option<EditorAction>,
    pub cancel: Option<EditorAction>,
}

impl DialogActions {
    pub fn none() -> Self {
        Self {
            primary: None,
            secondary: None,
            cancel: None,
        }
    }

    pub fn save_discard_cancel(save: EditorAction, discard: EditorAction) -> Self {
        Self {
            primary: Some(save),
            secondary: Some(discard),
            cancel: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Dialog {
    pub id: Id,
    pub title: String,
    pub message: String,
    pub buttons: DialogButtons,
    pub actions: DialogActions,
}

impl Dialog {
    pub fn new(
        id_source: impl std::hash::Hash,
        title: impl Into<String>,
        message: impl Into<String>,
        buttons: DialogButtons,
        actions: DialogActions,
    ) -> Self {
        Self {
            id: Id::new(id_source),
            title: title.into(),
            message: message.into(),
            buttons,
            actions,
        }
    }
}

#[derive(Debug, Default)]
pub struct DialogStack {
    dialogs: Vec<Dialog>,
}

impl DialogStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, dialog: Dialog) {
        self.dialogs.push(dialog);
    }

    pub fn info(&mut self, title: impl Into<String>, message: impl Into<String>) {
        self.push(Dialog::new(
            ("info", self.dialogs.len()),
            title,
            message,
            DialogButtons::Ok,
            DialogActions::none(),
        ));
    }

    pub fn save_discard_cancel(
        &mut self,
        id_source: impl std::hash::Hash,
        title: impl Into<String>,
        message: impl Into<String>,
        save_action: EditorAction,
        discard_action: EditorAction,
    ) {
        self.push(Dialog::new(
            id_source,
            title,
            message,
            DialogButtons::SaveDiscardCancel,
            DialogActions::save_discard_cancel(save_action, discard_action),
        ));
    }

    pub fn confirm(
        &mut self,
        id_source: impl std::hash::Hash,
        title: impl Into<String>,
        message: impl Into<String>,
        buttons: DialogButtons,
        actions: DialogActions,
    ) {
        self.push(Dialog::new(id_source, title, message, buttons, actions));
    }

    pub fn is_empty(&self) -> bool {
        self.dialogs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.dialogs.len()
    }

    pub fn clear(&mut self) {
        self.dialogs.clear();
    }

    /// The dialog currently on top of the stack (the one being shown).
    pub fn top(&self) -> Option<&Dialog> {
        self.dialogs.last()
    }

    pub fn pop_top(&mut self) {
        self.dialogs.pop();
    }
}
