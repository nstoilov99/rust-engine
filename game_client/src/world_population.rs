//! The single world-population seam (Task 39.8 P2, ruling §5.5).
//!
//! Five moments populate the ECS world — editor startup, editor scene open,
//! standalone `load_world`, benchmark load, and play-mode stop — and every one
//! of them must afterwards re-register physics bodies and run each plugin's
//! `on_world_loaded` callback. Before P2 those steps were copy-pasted (and, at
//! scene open, simply missing). They now live here, once.
//!
//! P5 moves the physics half into `RapierPhysicsPlugin`'s own
//! `on_world_loaded`; because there is exactly one call, that is a deletion
//! rather than a hunt.
//!
//! Not a population moment: play-mode *enter*. Nothing is loaded there — it
//! resyncs Rapier with transforms edited in edit mode via
//! `play_mode::rebuild_physics`.

use rust_engine::engine::ecs::game_world::GameWorld;
use rust_engine::engine::plugins::{PluginFailure, PluginSet};

/// Run the post-population hooks. Call immediately after anything that
/// populates, clears, or wholesale replaces the contents of `game_world`.
///
/// Returns the plugins whose callback failed; the caller applies the failure
/// policy ([`report_failures`] in the editor, [`abort_on_failures`] in a
/// shipped game).
#[must_use]
pub fn after_world_populated(
    game_world: &mut GameWorld,
    plugin_set: &mut PluginSet,
) -> Vec<PluginFailure> {
    let (world, resources) = game_world.world_and_resources_mut();

    // Since P5 this is *only* the callback run. Physics body registration is
    // `RapierPhysicsPlugin`'s `on_world_loaded`, so with that plugin disabled
    // no handles are created here at all — which is the whole point of D7.
    // The rebuild it performs clears first, so a loader that already
    // registered (the benchmark scene does) cannot end up with doubled bodies.
    plugin_set.run_world_loaded(world, resources)
}

/// One line per failure, for a console or a log.
pub fn describe(failure: &PluginFailure) -> String {
    format!(
        "plugin '{}' failed during {}: {}",
        failure.id,
        failure.phase.label(),
        failure.error
    )
}

/// Editor policy (ruling §5.9): surface and keep going — a broken plugin
/// callback must not cost the user their editor.
#[cfg_attr(not(feature = "editor"), allow(dead_code))]
pub fn report_failures(failures: &[PluginFailure]) {
    for failure in failures {
        eprintln!("{}", describe(failure));
    }
}

/// Shipped-game policy (ruling §5.9): fatal. A game whose plugin could not
/// set up the world it just loaded should not limp on.
#[cfg_attr(feature = "editor", allow(dead_code))]
pub fn abort_on_failures(failures: &[PluginFailure]) {
    if failures.is_empty() {
        return;
    }
    for failure in failures {
        eprintln!("fatal: {}", describe(failure));
    }
    std::process::exit(1);
}
