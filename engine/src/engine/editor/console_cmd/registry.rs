//! Command registry for registration and lookup
//!
//! Stores all available commands with O(1) lookup by name or alias.

use std::collections::HashMap;

use super::command::ConsoleCommand;

/// Registry storing all available commands
///
/// Supports:
/// - Primary name lookup
/// - Alias lookup
/// - Namespace enumeration (e.g., all "render.*" commands)
pub struct CommandRegistry {
    /// Commands indexed by primary name
    commands: HashMap<String, Box<dyn ConsoleCommand>>,
    /// Alias -> primary name mapping
    aliases: HashMap<String, String>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
            aliases: HashMap::new(),
        }
    }

    /// Register a command
    ///
    /// Returns Err if name or any alias conflicts with existing registration.
    pub fn register(&mut self, command: Box<dyn ConsoleCommand>) -> Result<(), String> {
        let meta = command.meta();
        let name = meta.name.to_string();

        // Check for conflicts
        if self.commands.contains_key(&name) {
            return Err(format!("Command '{}' already registered", name));
        }
        for alias in meta.aliases {
            if self.aliases.contains_key(*alias) || self.commands.contains_key(*alias) {
                return Err(format!(
                    "Alias '{}' conflicts with existing command/alias",
                    alias
                ));
            }
        }

        // Register aliases
        for alias in meta.aliases {
            self.aliases.insert(alias.to_string(), name.clone());
        }

        // Register command
        self.commands.insert(name, command);
        Ok(())
    }

    /// Look up a command by name or alias
    pub fn get(&self, name_or_alias: &str) -> Option<&dyn ConsoleCommand> {
        // Direct lookup
        if let Some(cmd) = self.commands.get(name_or_alias) {
            return Some(cmd.as_ref());
        }
        // Alias lookup
        if let Some(primary) = self.aliases.get(name_or_alias) {
            return self.commands.get(primary).map(|c| c.as_ref());
        }
        None
    }

    /// Get all commands (for help listing)
    pub fn all_commands(&self) -> impl Iterator<Item = &dyn ConsoleCommand> {
        self.commands.values().map(|c| c.as_ref())
    }

    /// Get commands matching a namespace prefix (e.g., "render.")
    pub fn commands_in_namespace(&self, prefix: &str) -> Vec<&dyn ConsoleCommand> {
        self.commands
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(_, cmd)| cmd.as_ref())
            .collect()
    }

    /// Get all command names for autocomplete
    pub fn all_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.commands.keys().map(|s| s.as_str()).collect();
        names.extend(self.aliases.keys().map(|s| s.as_str()));
        names.sort();
        names
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}
