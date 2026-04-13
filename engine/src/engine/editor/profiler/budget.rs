use super::data::{ProfileFrame, ProfilerState};
use egui::{Color32, RichText, Ui};

struct BudgetCategory {
    name: &'static str,
    budget_ms: f64,
    scopes: &'static [&'static str],
}

struct FrameBudget {
    total_budget_ms: f64,
    categories: &'static [BudgetCategory],
}

const FRAME_BUDGET: FrameBudget = FrameBudget {
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

pub fn render(ui: &mut Ui, state: &mut ProfilerState) {
    let Some(frame) = state.selected_frame() else {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new("Select a captured frame to inspect budgets").weak());
        });
        return;
    };

    let frame_duration_ms = frame.duration_ms();
    let frame_color = if frame_duration_ms <= FRAME_BUDGET.total_budget_ms {
        Color32::from_rgb(80, 200, 80)
    } else if frame_duration_ms <= 33.33 {
        Color32::from_rgb(220, 180, 60)
    } else {
        Color32::from_rgb(220, 80, 80)
    };

    ui.horizontal(|ui| {
        ui.label(RichText::new("Frame Budget").strong());
        ui.label(
            RichText::new(format!(
                "{:.2} / {:.2} ms",
                frame_duration_ms, FRAME_BUDGET.total_budget_ms
            ))
            .color(frame_color),
        );
    });
    ui.add(
        egui::ProgressBar::new((frame_duration_ms / FRAME_BUDGET.total_budget_ms).min(2.0) as f32)
            .desired_width(f32::INFINITY)
            .fill(frame_color)
            .text(format!("{:.2} ms", frame_duration_ms)),
    );

    ui.add_space(8.0);
    ui.label(RichText::new("Category Budgets").strong());
    ui.add_space(4.0);

    for category in FRAME_BUDGET.categories {
        let actual_ms = scope_total_ms(frame, category.scopes);
        let color = if actual_ms <= category.budget_ms {
            Color32::from_rgb(80, 200, 80)
        } else if actual_ms <= category.budget_ms * 1.5 {
            Color32::from_rgb(220, 180, 60)
        } else {
            Color32::from_rgb(220, 80, 80)
        };

        ui.horizontal(|ui| {
            ui.add_sized([100.0, 18.0], egui::Label::new(category.name));
            ui.add(
                egui::ProgressBar::new((actual_ms / category.budget_ms).min(2.0) as f32)
                    .desired_width((ui.available_width() - 170.0).max(120.0))
                    .fill(color)
                    .text(format!("{:.2} / {:.2} ms", actual_ms, category.budget_ms)),
            );
        });
    }

    ui.separator();
    ui.label(RichText::new("Runtime Counters").strong());

    egui::Grid::new("profiler_budget_counters")
        .num_columns(2)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            ui.label("Draw Calls");
            ui.label(state.latest_render_counters.draw_calls.to_string());
            ui.end_row();

            ui.label("Triangles");
            ui.label(state.latest_render_counters.triangles.to_string());
            ui.end_row();

            ui.label("Material Changes");
            ui.label(state.latest_render_counters.material_changes.to_string());
            ui.end_row();

            ui.label("Visible Entities");
            ui.label(state.latest_render_counters.visible_entities.to_string());
            ui.end_row();

            ui.label("Entities");
            ui.label(state.latest_resource_counters.entity_count.to_string());
            ui.end_row();

            ui.label("Meshes");
            ui.label(state.latest_resource_counters.mesh_count.to_string());
            ui.end_row();

            ui.label("Textures");
            ui.label(state.latest_resource_counters.texture_count.to_string());
            ui.end_row();

            ui.label("Rigid Bodies");
            ui.label(state.latest_resource_counters.rigid_body_count.to_string());
            ui.end_row();
        });
}

fn scope_total_ms(frame: &ProfileFrame, names: &[&str]) -> f64 {
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
