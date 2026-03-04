//! Console Command System
//!
//! A modular, extensible console command system for the engine.
//! Commands can be registered from any subsystem and executed via the console UI.
//!
//! # Architecture
//!
//! - `ConsoleCommand` trait: Interface all commands implement
//! - `CommandRegistry`: Stores and looks up commands by name/alias
//! - `CommandContext`: Execution context with access to engine state
//! - `ConsoleCommandSystem`: Main system orchestrating execution
//!
//! # Example
//!
//! ```ignore
//! // Create system (registers built-in commands)
//! let mut system = ConsoleCommandSystem::new();
//!
//! // Execute a command
//! let mut ctx = CommandContext::minimal();
//! let output = system.execute("echo hello", &mut ctx);
//! ```

mod builtin;
mod command;
mod context;
mod history;
mod parser;
mod registry;

pub use command::{ArgSpec, CommandMeta, CommandResult, ConsoleCommand};
pub use context::CommandContext;
pub use history::InputHistory;
pub use parser::{args, parse_input, ParsedInput};
pub use registry::CommandRegistry;

use crate::engine::editor::console::LogMessage;

/// Main console command system
///
/// Owns the registry and history. Created once in App::new().
pub struct ConsoleCommandSystem {
    /// Command registry
    pub registry: CommandRegistry,
    /// Input history for recall
    pub history: InputHistory,
}

impl ConsoleCommandSystem {
    /// Create a new console command system with built-in commands
    pub fn new() -> Self {
        let mut system = Self {
            registry: CommandRegistry::new(),
            history: InputHistory::default(),
        };

        // Register built-in commands
        builtin::register_builtins(&mut system.registry);

        system
    }

    /// Execute a command from raw input string
    ///
    /// Returns output messages to append to console log.
    pub fn execute(&mut self, input: &str, ctx: &mut CommandContext) -> Vec<LogMessage> {
        // Add to history
        self.history.push(input.to_string());

        // Parse input
        let Some(parsed) = parse_input(input) else {
            return vec![];
        };

        // Look up command
        let Some(command) = self.registry.get(&parsed.command) else {
            return vec![LogMessage::error(format!(
                "Unknown command: '{}'. Type 'help' for available commands.",
                parsed.command
            ))];
        };

        // Convert args for execution
        let arg_refs: Vec<&str> = parsed.args.iter().map(|s| s.as_str()).collect();

        // Execute
        let result = command.execute(ctx, &arg_refs);

        // Collect output
        let mut output = std::mem::take(&mut ctx.output);

        match result {
            CommandResult::Success(Some(msg)) => {
                // Handle special commands
                if msg == "__CLEAR__" {
                    // Return empty - caller checks for this
                    return vec![LogMessage::info("__CLEAR__")];
                } else if let Some(cmd_name) = msg.strip_prefix("__HELP__:") {
                    // Detailed help for a specific command
                    if let Some(cmd) = self.registry.get(cmd_name) {
                        let meta = cmd.meta();
                        output.push(LogMessage::info(format!("Command: {}", meta.name)));
                        if !meta.aliases.is_empty() {
                            output.push(LogMessage::info(format!(
                                "Aliases: {}",
                                meta.aliases.join(", ")
                            )));
                        }
                        output.push(LogMessage::info(format!("Category: {}", meta.category)));
                        output.push(LogMessage::info(""));
                        output.push(LogMessage::info(meta.help));
                    } else {
                        output.push(LogMessage::error(format!(
                            "Unknown command: '{}'",
                            cmd_name
                        )));
                    }
                } else if !msg.is_empty() {
                    output.push(LogMessage::info(msg));
                }
            }
            CommandResult::Success(None) => {}
            CommandResult::Error(msg) => {
                output.push(LogMessage::error(msg));
            }
            CommandResult::Output(lines) => {
                for line in lines {
                    output.push(LogMessage::info(line));
                }
            }
        }

        output
    }

    /// Get all command names for autocomplete
    pub fn command_names(&self) -> Vec<&str> {
        self.registry.all_names()
    }
}

impl Default for ConsoleCommandSystem {
    fn default() -> Self {
        Self::new()
    }
}
