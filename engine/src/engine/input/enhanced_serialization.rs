//! RON serialization for the enhanced input system.
//!
//! Loads and saves `InputActionSet` from/to `config/input_actions.ron`.
//! Also provides migration from legacy `ActionMap` format.

use super::action::{ActionType, InputSource};
use super::action_map::ActionMap;
use super::enhanced_action::*;
use super::trigger::InputTrigger;
use super::value::InputValueType;
use std::path::{Path, PathBuf};

/// Default path for the enhanced input config file.
pub fn default_action_set_path() -> PathBuf {
    PathBuf::from("config/input_actions.ron")
}

/// Load an InputActionSet from a RON file.
pub fn load_action_set(path: &Path) -> Option<InputActionSet> {
    let content = std::fs::read_to_string(path).ok()?;
    match ron::from_str::<InputActionSet>(&content) {
        Ok(set) => {
            log::info!("Loaded enhanced input config from {}", path.display());
            Some(set)
        }
        Err(e) => {
            log::warn!(
                "Failed to parse enhanced input config at {}: {e}",
                path.display()
            );
            None
        }
    }
}

/// Save an InputActionSet to a RON file.
pub fn save_action_set(
    set: &InputActionSet,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let ron_str = ron::ser::to_string_pretty(set, Default::default())?;
    std::fs::write(path, ron_str)?;
    log::info!("Saved enhanced input config to {}", path.display());
    Ok(())
}

// ── Per-file serialization (Unreal-style individual assets) ──

/// Load a single InputActionDefinition from a `.inputaction.ron` file.
pub fn load_input_action(path: &Path) -> Option<InputActionDefinition> {
    let content = std::fs::read_to_string(path).ok()?;
    ron::from_str::<InputActionDefinition>(&content).ok()
}

/// Save a single InputActionDefinition to a `.inputaction.ron` file.
pub fn save_input_action(
    action: &InputActionDefinition,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let ron_str = ron::ser::to_string_pretty(action, Default::default())?;
    std::fs::write(path, ron_str)?;
    Ok(())
}

/// Load a single MappingContext from a `.mappingcontext.ron` file.
pub fn load_mapping_context(path: &Path) -> Option<MappingContext> {
    let content = std::fs::read_to_string(path).ok()?;
    ron::from_str::<MappingContext>(&content).ok()
}

/// Save a single MappingContext to a `.mappingcontext.ron` file.
pub fn save_mapping_context(
    ctx: &MappingContext,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let ron_str = ron::ser::to_string_pretty(ctx, Default::default())?;
    std::fs::write(path, ron_str)?;
    Ok(())
}

/// Scan a directory tree for `.inputaction.ron` and `.mappingcontext.ron` files
/// and assemble them into a single `InputActionSet`.
pub fn scan_and_assemble(content_dir: &Path) -> Option<InputActionSet> {
    let mut set = InputActionSet::new();
    let mut found_any = false;

    for entry in std::fs::read_dir(content_dir).ok()?.flatten() {
        scan_dir_recursive(&entry.path(), &mut set, &mut found_any);
    }

    if found_any { Some(set) } else { None }
}

fn scan_dir_recursive(path: &Path, set: &mut InputActionSet, found: &mut bool) {
    if path.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                scan_dir_recursive(&entry.path(), set, found);
            }
        }
        return;
    }

    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if filename.ends_with(".inputaction.ron") {
        if let Some(action) = load_input_action(path) {
            log::info!("Loaded input action '{}' from {}", action.name, path.display());
            set.add_action(action);
            *found = true;
        }
    } else if filename.ends_with(".mappingcontext.ron") {
        if let Some(ctx) = load_mapping_context(path) {
            log::info!("Loaded mapping context '{}' from {}", ctx.name, path.display());
            set.add_context(ctx);
            *found = true;
        }
    }
}

/// Collect all action names from `.inputaction.ron` files in a directory tree.
pub fn scan_action_names(content_dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    scan_action_names_recursive(content_dir, &mut names);
    names.sort();
    names
}

fn scan_action_names_recursive(path: &Path, names: &mut Vec<String>) {
    if path.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                scan_action_names_recursive(&entry.path(), names);
            }
        }
        return;
    }

    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if filename.ends_with(".inputaction.ron") {
        if let Some(action) = load_input_action(path) {
            names.push(action.name);
        }
    }
}

/// Migrate a legacy `ActionMap` to the enhanced `InputActionSet`.
pub fn migrate_legacy_action_map(map: &ActionMap) -> InputActionSet {
    let mut set = InputActionSet::new();

    for (ctx_name, legacy_ctx) in &map.contexts {
        // Map legacy context names to priorities
        let priority = if ctx_name == "global" { 100 } else { 0 };

        let mut context = MappingContext::new(ctx_name.clone(), priority);

        for action_def in &legacy_ctx.actions {
            // Register the action definition if not already present
            let value_type = match action_def.action_type {
                ActionType::Digital => InputValueType::Digital,
                ActionType::Axis1D => InputValueType::Axis1D,
                ActionType::Axis2D => InputValueType::Axis2D,
            };

            // Choose appropriate triggers for the action type
            let trigger = if value_type == InputValueType::Digital {
                InputTrigger::Pressed
            } else {
                InputTrigger::Down
            };

            if !set.actions.contains_key(&action_def.name) {
                set.add_action(
                    InputActionDefinition::new(&action_def.name, value_type)
                        .with_trigger(trigger),
                );
            }

            // Convert bindings
            let mut entry = MappingContextEntry::new(&action_def.name);
            for binding in &action_def.bindings {
                entry.bindings.push(EnhancedBinding {
                    source: binding.source.clone(),
                    modifiers: Vec::new(),
                    triggers: Vec::new(),
                    axis_contribution: binding.axis_contribution,
                });
            }

            // Convert gamepad stick 2D binding to two axis bindings
            if let Some(ref stick) = action_def.gamepad_stick {
                entry.bindings.push(EnhancedBinding::axis_2d(
                    InputSource::GamepadAxis(stick.axis_x),
                    1.0,
                    0.0,
                ));
                entry.bindings.push(EnhancedBinding::axis_2d(
                    InputSource::GamepadAxis(stick.axis_y),
                    0.0,
                    1.0,
                ));
            }

            context.entries.push(entry);
        }

        set.add_context(context);
    }

    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::input::default_bindings::default_action_map;

    #[test]
    fn roundtrip_action_set() {
        let legacy = default_action_map();
        let set = migrate_legacy_action_map(&legacy);
        let ron_str = ron::ser::to_string_pretty(&set, Default::default()).unwrap();
        let loaded: InputActionSet = ron::from_str(&ron_str).unwrap();
        assert_eq!(set.actions.len(), loaded.actions.len());
        assert_eq!(set.contexts.len(), loaded.contexts.len());
    }

    #[test]
    fn migration_preserves_actions() {
        let legacy = default_action_map();
        let set = migrate_legacy_action_map(&legacy);

        assert!(set.actions.contains_key("jump"));
        assert!(set.actions.contains_key("move"));
        assert!(set.actions.contains_key("look"));
        assert!(set.actions.contains_key("pause"));

        assert_eq!(
            set.actions["jump"].value_type,
            InputValueType::Digital
        );
        assert_eq!(
            set.actions["move"].value_type,
            InputValueType::Axis2D
        );
    }

    #[test]
    fn migration_sets_priorities() {
        let legacy = default_action_map();
        let set = migrate_legacy_action_map(&legacy);

        let global = set.context("global").expect("global context");
        assert_eq!(global.priority, 100);

        let gameplay = set.context("gameplay").expect("gameplay context");
        assert_eq!(gameplay.priority, 0);
    }
}
