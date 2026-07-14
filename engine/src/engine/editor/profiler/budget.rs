//! Frame-budget category tables and the `scope_total_ms` helper.
//!
//! The old `render` fn was removed; the crusty analog lives in
//! `profiler_crusty::budget_view_panel` (which reads `FRAME_BUDGET` +
//! `scope_total_ms` from here).

use super::data::ProfileFrame;

pub(crate) struct BudgetCategory {
    pub(crate) name: &'static str,
    pub(crate) budget_ms: f64,
    pub(crate) scopes: &'static [&'static str],
}

pub(crate) struct FrameBudget {
    pub(crate) total_budget_ms: f64,
    pub(crate) categories: &'static [BudgetCategory],
}

pub(crate) const FRAME_BUDGET: FrameBudget = FrameBudget {
    total_budget_ms: 16.67,
    categories: &[
        BudgetCategory {
            name: "ECS Systems",
            budget_ms: 2.0,
            scopes: &["ecs_systems"],
        },
        BudgetCategory {
            name: "Transforms",
            budget_ms: 1.5,
            scopes: &["transform_propagation"],
        },
        BudgetCategory {
            name: "Physics",
            budget_ms: 4.0,
            scopes: &["physics_step"],
        },
        BudgetCategory {
            name: "Geometry",
            budget_ms: 4.5,
            scopes: &["geometry_pass"],
        },
        BudgetCategory {
            name: "Lighting",
            budget_ms: 1.5,
            scopes: &["lighting_pass"],
        },
        BudgetCategory {
            name: "Grid",
            budget_ms: 0.5,
            scopes: &["grid_pass"],
        },
        BudgetCategory {
            name: "Present",
            budget_ms: 1.0,
            scopes: &["swapchain_present"],
        },
        BudgetCategory {
            name: "Profiler UI",
            budget_ms: 1.0,
            scopes: &["profiler_ui"],
        },
    ],
};
pub(crate) fn scope_total_ms(frame: &ProfileFrame, names: &[&str]) -> f64 {
    frame
        .threads
        .iter()
        .flat_map(|thread| thread.scopes.iter())
        .map(|scope| scope_total_ms_recursive(scope, names))
        .sum()
}

fn scope_total_ms_recursive(scope: &super::data::ProfileScope, names: &[&str]) -> f64 {
    let own_ms = if names.iter().any(|name| scope.name.as_ref() == *name) {
        scope.duration_ms()
    } else {
        0.0
    };

    own_ms
        + scope
            .children
            .iter()
            .map(|child| scope_total_ms_recursive(child, names))
            .sum::<f64>()
}
