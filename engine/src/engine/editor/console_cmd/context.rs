//! Command execution context
//!
//! Provides access to engine state that commands may need to read or modify.

use crate::engine::editor::console::LogMessage;
use hecs::World;

/// Context passed to commands during execution
///
/// Provides access to engine state that commands may need to read or modify.
/// Not all fields are always available - commands should check for None.
pub struct CommandContext<'a> {
    /// ECS World (entities, components)
    pub world: Option<&'a mut World>,

    /// Toggle for stat fps overlay display
    pub show_stat_fps: Option<&'a mut bool>,

    /// Output log messages (commands append here)
    pub output: Vec<LogMessage>,
}

impl<'a> CommandContext<'a> {
    /// Create a minimal context (for testing or headless execution)
    pub fn minimal() -> Self {
        Self {
            world: None,
            show_stat_fps: None,
            output: Vec::new(),
        }
    }

    /// Create a context with world access
    pub fn with_world(world: &'a mut World) -> Self {
        Self {
            world: Some(world),
            show_stat_fps: None,
            output: Vec::new(),
        }
    }

    /// Create a full context with all state
    pub fn new(world: &'a mut World, show_stat_fps: &'a mut bool) -> Self {
        Self {
            world: Some(world),
            show_stat_fps: Some(show_stat_fps),
            output: Vec::new(),
        }
    }

    /// Log an info message to command output
    pub fn log_info(&mut self, msg: impl Into<String>) {
        self.output.push(LogMessage::info(msg));
    }

    /// Log a warning message
    pub fn log_warning(&mut self, msg: impl Into<String>) {
        self.output.push(LogMessage::warning(msg));
    }

    /// Log an error message
    pub fn log_error(&mut self, msg: impl Into<String>) {
        self.output.push(LogMessage::error(msg));
    }
}
