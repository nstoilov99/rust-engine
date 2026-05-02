//! Command palette — fuzzy-search popup over editor commands.
//!
//! `action.rs` defines `EditorAction` and `dispatch_action` (Step 2).
//! Step 7 adds `CommandRegistry`, `Command`, and the palette popup UI.

pub mod action;

pub use action::{dispatch_action, EditorAction};
