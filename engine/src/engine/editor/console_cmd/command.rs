//! Console command trait and result types
//!
//! Defines the interface that all console commands must implement.

use super::context::CommandContext;

/// Result of command execution
pub enum CommandResult {
    /// Command executed successfully with optional output message
    Success(Option<String>),
    /// Command failed with error message
    Error(String),
    /// Command produced multiple output lines
    Output(Vec<String>),
}

/// Argument specification for help text and future validation/autocomplete
#[derive(Clone, Debug)]
pub struct ArgSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub required: bool,
}

/// Command metadata for registration and help
pub struct CommandMeta {
    /// Primary command name (e.g., "render.reload")
    pub name: &'static str,
    /// Optional aliases (e.g., ["rr"])
    pub aliases: &'static [&'static str],
    /// Short description for help listing
    pub description: &'static str,
    /// Detailed usage/help text
    pub help: &'static str,
    /// Argument specifications
    pub args: &'static [ArgSpec],
    /// Category for grouping in help (e.g., "Rendering", "Debug")
    pub category: &'static str,
}

/// Trait for console commands
///
/// Commands are stateless by default - they receive context and args,
/// perform work, and return a result.
pub trait ConsoleCommand: Send + Sync {
    /// Get command metadata
    fn meta(&self) -> CommandMeta;

    /// Execute the command with parsed arguments
    ///
    /// # Arguments
    /// * `ctx` - Execution context with access to engine state
    /// * `args` - Parsed arguments as strings (command handles type conversion)
    fn execute(&self, ctx: &mut CommandContext, args: &[&str]) -> CommandResult;
}
