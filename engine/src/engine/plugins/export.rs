//! Export feature resolution (Task 39.8 D9).
//!
//! Cargo features are the *packaging* tool, not the user-facing toggle. A
//! shipped game has no `project.ron` and no runtime manifest — activation is
//! resolved here, at build time, and every plugin compiled into an export is
//! enabled.
//!
//! ## Why `--no-default-features`
//!
//! `game_client`'s `default` includes `hud`, which is not a plugin. Adding
//! `--features x` on top of the defaults could never *remove* a
//! default-enabled plugin feature, and `--no-default-features` alone would
//! strip the HUD. So exports pass
//! `--no-default-features --features <base + enabled runtime plugin features>`
//! where *base* is [`EXPORT_BASE_FEATURES`].
//!
//! ## The rules
//!
//! - [`PluginKind::EditorOnly`] never contributes a feature, **regardless of
//!   enabled state** — Unreal's Editor-module-type behaviour. `dev_nodes` is
//!   the proving case: fixture nodes must not reach a shipped game.
//! - `internal` plugins are the project's own code; they are not
//!   manifest-toggleable and carry no feature.
//! - A plugin with `cargo_feature: None` is always compiled in and
//!   contributes nothing. See the note on [`export_features`] about what that
//!   means for stripping.
//! - Everything else contributes its feature when the manifest enables it.
//!
//! Enabled-but-unused runtime plugins **do** ship, exactly like Unreal:
//! registration itself references the code, so no whole-program analysis can
//! drop it. The mitigation is visibility — the build dialog lists what is
//! going in — not magic.

use super::{PluginEntry, PluginKind, PluginManifest};

/// Non-plugin features that must survive `--no-default-features`.
///
/// Keep in step with `game_client`'s `[features] default` **minus anything a
/// plugin already carries as its `cargo_feature`**. `graph-scripting` is
/// default-on and in that default list, but it stays out of here on purpose:
/// it is the feature `graph_scripting`'s manifest names, so the export
/// resolver adds it exactly when that plugin is enabled. Listing it here too
/// would make it unconditional and quietly undo the gate.
pub const EXPORT_BASE_FEATURES: &[&str] = &["hud"];

/// A runtime plugin that will be compiled into an export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedPlugin {
    pub id: String,
    pub name: String,
    /// `None` = compiled in unconditionally (no feature gates it).
    pub cargo_feature: Option<String>,
}

/// The runtime plugins an export will contain, in manifest order.
///
/// Note the asymmetry this exposes: a plugin with no `cargo_feature` is
/// listed here even when the manifest disables it, because nothing can strip
/// it from the binary. Callers should present that honestly rather than imply
/// a toggle that does not reach exports.
pub fn exported_plugins(
    manifests: &[PluginManifest],
    project_plugins: &[PluginEntry],
) -> Vec<ExportedPlugin> {
    manifests
        .iter()
        .filter(|m| !m.internal)
        .filter(|m| m.kind == PluginKind::Runtime)
        .filter(|m| m.cargo_feature.is_none() || enabled_in(project_plugins, &m.id))
        .map(|m| ExportedPlugin {
            id: m.id.clone(),
            name: m.name.clone(),
            cargo_feature: m.cargo_feature.clone(),
        })
        .collect()
}

/// The full `--features` list for an export build: base plus every enabled
/// runtime plugin's feature, deduplicated and ordered for a stable command
/// line (and stable build logs).
pub fn export_features(
    manifests: &[PluginManifest],
    project_plugins: &[PluginEntry],
) -> Vec<String> {
    let mut out: Vec<String> = EXPORT_BASE_FEATURES
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    for plugin in exported_plugins(manifests, project_plugins) {
        if let Some(feature) = plugin.cargo_feature {
            out.push(feature);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Absent from the manifest = enabled (batteries included), matching
/// `PluginSet`'s activation rule.
fn enabled_in(entries: &[PluginEntry], id: &str) -> bool {
    entries
        .iter()
        .find(|e| e.id == id)
        .map(|e| e.enabled)
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::plugins::PluginOrigin;

    fn m(id: &str, kind: PluginKind, feature: Option<&str>) -> PluginManifest {
        let mut manifest = PluginManifest::new(id, id)
            .with_kind(kind)
            .with_origin(PluginOrigin::Engine);
        if let Some(f) = feature {
            manifest = manifest.with_cargo_feature(f);
        }
        manifest
    }

    #[test]
    fn base_features_survive_no_default_features() {
        let features = export_features(&[], &[]);
        assert_eq!(features, vec!["hud".to_string()]);
    }

    /// D9's headline rule, and the P3 note: an editor-only plugin never
    /// reaches an export even when it is enabled.
    #[test]
    fn editor_only_never_contributes_even_when_enabled() {
        let manifests = vec![m("dev_nodes", PluginKind::EditorOnly, Some("dev_nodes"))];
        assert_eq!(
            export_features(&manifests, &[PluginEntry::new("dev_nodes", true)]),
            vec!["hud".to_string()],
            "dev_nodes is fixture content; it must never ship"
        );
        assert!(exported_plugins(&manifests, &[]).is_empty());
    }

    #[test]
    fn enabled_runtime_plugin_contributes_its_feature() {
        let manifests = vec![m("steam", PluginKind::Runtime, Some("plugin-steam"))];
        assert_eq!(
            export_features(&manifests, &[]),
            vec!["hud".to_string(), "plugin-steam".to_string()]
        );
    }

    #[test]
    fn disabled_runtime_plugin_is_stripped() {
        let manifests = vec![m("steam", PluginKind::Runtime, Some("plugin-steam"))];
        assert_eq!(
            export_features(&manifests, &[PluginEntry::new("steam", false)]),
            vec!["hud".to_string()]
        );
        assert!(exported_plugins(&manifests, &[PluginEntry::new("steam", false)]).is_empty());
    }

    /// The `physics_rapier` shape: no feature gates it, so it is always in the
    /// export and the manifest toggle cannot reach it. It still gets listed,
    /// because the dialog must not silently omit what is shipping.
    #[test]
    fn ungated_runtime_plugin_always_ships_and_is_still_listed() {
        let manifests = vec![m("physics_rapier", PluginKind::Runtime, None)];
        assert_eq!(export_features(&manifests, &[]), vec!["hud".to_string()]);

        let listed = exported_plugins(&manifests, &[PluginEntry::new("physics_rapier", false)]);
        assert_eq!(listed.len(), 1, "it ships whether the manifest likes it or not");
        assert_eq!(listed[0].cargo_feature, None);
    }

    #[test]
    fn internal_plugins_carry_no_feature() {
        let manifests = vec![m("game_client", PluginKind::Runtime, Some("nope")).internal()];
        assert_eq!(export_features(&manifests, &[]), vec!["hud".to_string()]);
        assert!(exported_plugins(&manifests, &[]).is_empty());
    }

    #[test]
    fn features_are_sorted_and_deduplicated() {
        let manifests = vec![
            m("b", PluginKind::Runtime, Some("zeta")),
            m("a", PluginKind::Runtime, Some("alpha")),
            m("c", PluginKind::Runtime, Some("alpha")),
        ];
        assert_eq!(
            export_features(&manifests, &[]),
            vec!["alpha".to_string(), "hud".to_string(), "zeta".to_string()]
        );
    }
}
