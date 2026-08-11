//! Graph scripting runtime binding (Task 45-A P5).
//!
//! **The component is unconditional; the runner is not.** This mirrors the
//! Rapier split exactly, and for the same reason: a scene that names a graph
//! must load, save and display losslessly whether or not the plugin that
//! *runs* graphs is compiled in or enabled. Scene serialization, the prefab
//! path and the inspector therefore all see [`GraphRunner`] with no `cfg` in
//! sight; only [`runner`] — which pulls in the interpreter crate — is gated
//! behind the `graph-scripting` feature.
//!
//! With the plugin off, a `GraphRunner` is inert: it round-trips through the
//! scene, the inspector says so, and nothing ticks.

use serde::{Deserialize, Serialize};

#[cfg(feature = "graph-scripting")]
pub mod runner;

/// 45-A P7's execution recorder. Editor builds only — see the module docs for
/// why a shipped game contains none of it.
#[cfg(all(feature = "graph-scripting", feature = "editor"))]
pub mod trace;

/// P5 acceptance evidence: the real plugin, the real schedule, a real world.
#[cfg(all(test, feature = "graph-scripting"))]
mod acceptance;

#[cfg(feature = "graph-scripting")]
pub use runner::{CurveCache, GraphPlanCache, GraphRuntime, GraphScriptRunnerSystem};

#[cfg(all(feature = "graph-scripting", feature = "editor"))]
pub use trace::GraphTrace;

/// Attaches a `.graph` asset to an entity. Serialized config only — the
/// running state lives in a separate, never-serialized component created by
/// the runner on the first playing tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphRunner {
    /// Content-relative path with forward slashes, e.g.
    /// `graphs/runner_demo.graph` — the same key form the graph editor's
    /// tabs, the resolver and hot-reload matching all use.
    pub graph: String,
    /// Off means "attached but not running". Disabling is not the same as
    /// removing: the reference survives, which is what makes it a debugging
    /// switch rather than a destructive edit.
    pub enabled: bool,
}

impl Default for GraphRunner {
    fn default() -> Self {
        Self {
            graph: String::new(),
            enabled: true,
        }
    }
}

impl GraphRunner {
    pub fn new(graph: &str) -> Self {
        Self {
            graph: normalize_graph_path(graph),
            enabled: true,
        }
    }

    /// Is this runner worth creating an instance for? An empty path is a
    /// component someone added and has not pointed anywhere yet — a normal
    /// intermediate state in the inspector, not an error.
    pub fn is_runnable(&self) -> bool {
        self.enabled && !self.graph.trim().is_empty()
    }
}

/// Content-relative, forward slashes. Windows hands back `\` from every path
/// API, and a scene authored on Windows must load on Linux.
pub fn normalize_graph_path(path: &str) -> String {
    path.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_normalize_and_empty_runners_are_not_runnable() {
        assert_eq!(
            GraphRunner::new("graphs\\lib\\demo.graph").graph,
            "graphs/lib/demo.graph"
        );

        let mut r = GraphRunner::new("graphs/demo.graph");
        assert!(r.is_runnable());
        r.enabled = false;
        assert!(!r.is_runnable(), "disabled keeps the reference but does not run");

        // A component added in the inspector and not yet pointed anywhere is
        // a normal intermediate state, not an error.
        assert!(!GraphRunner::default().is_runnable());
        assert!(!GraphRunner::new("   ").is_runnable());
    }
}
