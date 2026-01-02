//! Built-in console commands
//!
//! Core commands that are always available: help, clear, echo, entity.count, stat fps

use super::command::{ArgSpec, CommandMeta, CommandResult, ConsoleCommand};
use super::context::CommandContext;
use super::registry::CommandRegistry;

/// Help command - lists all commands or shows help for a specific command
pub struct HelpCommand;

impl ConsoleCommand for HelpCommand {
    fn meta(&self) -> CommandMeta {
        CommandMeta {
            name: "help",
            aliases: &["?", "h"],
            description: "Show available commands or help for a specific command",
            help: "Usage: help [command]\n\nWithout arguments, lists all commands.\nWith a command name, shows detailed help.",
            args: &[ArgSpec {
                name: "command",
                description: "Command to get help for",
                required: false,
            }],
            category: "General",
        }
    }

    fn execute(&self, ctx: &mut CommandContext, args: &[&str]) -> CommandResult {
        // Note: For specific command help, the caller needs to provide registry access
        // via the output. For now we show a basic help message.
        if args.is_empty() {
            ctx.log_info("Available commands:");
            ctx.log_info("  help [cmd]     - Show help (aliases: ?, h)");
            ctx.log_info("  clear          - Clear console (alias: cls)");
            ctx.log_info("  echo <text>    - Print text to console");
            ctx.log_info("  entity.count   - Show entity count (alias: ec)");
            ctx.log_info("  stat <type>    - Display statistics (e.g., stat fps)");
            ctx.log_info("");
            ctx.log_info("Type 'help <command>' for detailed help.");
            CommandResult::Success(None)
        } else {
            let cmd_name = args[0];
            // Return a marker that the system should look up detailed help
            CommandResult::Success(Some(format!("__HELP__:{}", cmd_name)))
        }
    }
}

/// Clear command - clears console output
pub struct ClearCommand;

impl ConsoleCommand for ClearCommand {
    fn meta(&self) -> CommandMeta {
        CommandMeta {
            name: "clear",
            aliases: &["cls"],
            description: "Clear console output",
            help: "Usage: clear\n\nClears all messages from the console.",
            args: &[],
            category: "General",
        }
    }

    fn execute(&self, _ctx: &mut CommandContext, _args: &[&str]) -> CommandResult {
        // Clearing is handled by the UI layer checking for this special result
        CommandResult::Success(Some("__CLEAR__".to_string()))
    }
}

/// Echo command - prints arguments back
pub struct EchoCommand;

impl ConsoleCommand for EchoCommand {
    fn meta(&self) -> CommandMeta {
        CommandMeta {
            name: "echo",
            aliases: &[],
            description: "Print text to console",
            help: "Usage: echo <text...>\n\nPrints the given text to the console.",
            args: &[ArgSpec {
                name: "text",
                description: "Text to print",
                required: true,
            }],
            category: "General",
        }
    }

    fn execute(&self, _ctx: &mut CommandContext, args: &[&str]) -> CommandResult {
        if args.is_empty() {
            CommandResult::Success(Some(String::new()))
        } else {
            CommandResult::Success(Some(args.join(" ")))
        }
    }
}

/// Entity count command - shows number of entities in the world
pub struct EntityCountCommand;

impl ConsoleCommand for EntityCountCommand {
    fn meta(&self) -> CommandMeta {
        CommandMeta {
            name: "entity.count",
            aliases: &["ec"],
            description: "Show number of entities in the world",
            help: "Usage: entity.count\n\nDisplays the current entity count in the ECS world.",
            args: &[],
            category: "Debug",
        }
    }

    fn execute(&self, ctx: &mut CommandContext, _args: &[&str]) -> CommandResult {
        match &ctx.world {
            Some(world) => {
                let count = world.len();
                CommandResult::Success(Some(format!("Entities: {}", count)))
            }
            None => CommandResult::Error("World not available".to_string()),
        }
    }
}

/// Stat command - displays various statistics (Unreal Engine style)
pub struct StatCommand;

impl ConsoleCommand for StatCommand {
    fn meta(&self) -> CommandMeta {
        CommandMeta {
            name: "stat",
            aliases: &[],
            description: "Display various statistics",
            help: "Usage: stat <type>\n\nTypes:\n  fps - Toggle FPS/frame time overlay in viewport",
            args: &[ArgSpec {
                name: "type",
                description: "Statistic type (fps)",
                required: true,
            }],
            category: "Debug",
        }
    }

    fn execute(&self, ctx: &mut CommandContext, args: &[&str]) -> CommandResult {
        match args.first().copied() {
            Some("fps") => match &mut ctx.show_stat_fps {
                Some(show) => {
                    **show = !**show;
                    if **show {
                        CommandResult::Success(Some("stat fps enabled".to_string()))
                    } else {
                        CommandResult::Success(Some("stat fps disabled".to_string()))
                    }
                }
                None => CommandResult::Error("stat fps not available in this context".to_string()),
            },
            Some(other) => {
                CommandResult::Error(format!("Unknown stat type: '{}'. Available: fps", other))
            }
            None => CommandResult::Error("Usage: stat <type> (e.g., 'stat fps')".to_string()),
        }
    }
}

/// Register all built-in commands
pub fn register_builtins(registry: &mut CommandRegistry) {
    // Use expect() since these are static definitions that should never conflict
    registry
        .register(Box::new(HelpCommand))
        .expect("Failed to register HelpCommand");
    registry
        .register(Box::new(ClearCommand))
        .expect("Failed to register ClearCommand");
    registry
        .register(Box::new(EchoCommand))
        .expect("Failed to register EchoCommand");
    registry
        .register(Box::new(EntityCountCommand))
        .expect("Failed to register EntityCountCommand");
    registry
        .register(Box::new(StatCommand))
        .expect("Failed to register StatCommand");
}
