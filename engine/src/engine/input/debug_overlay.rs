//! Input debug overlay for visualizing the enhanced input system state.
//!
//! Renders an egui window showing active contexts, action states,
//! and consumed sources. Toggled via the "debug_overlay" action (F3).

use super::subsystem::InputSubsystem;
use super::trigger::ActionPhase;
use super::value::InputValue;

/// Persistent state for the input debug overlay.
#[derive(Default)]
pub struct InputDebugOverlay {
    pub enabled: bool,
}

impl InputDebugOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle the overlay on/off.
    pub fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    /// Render the debug overlay. Call this during the editor/game UI pass.
    #[cfg(feature = "editor")]
    pub fn show(&self, ctx: &egui::Context, subsystem: &InputSubsystem) {
        if !self.enabled {
            return;
        }

        egui::Window::new("Input Debug")
            .default_pos([10.0, 10.0])
            .default_size([320.0, 400.0])
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
                // Active contexts
                ui.heading("Active Contexts");
                let contexts = subsystem.active_contexts();
                if contexts.is_empty() {
                    ui.label(egui::RichText::new("(none)").weak());
                } else {
                    for name in contexts {
                        let priority = subsystem
                            .action_set
                            .context(name)
                            .map(|c| c.priority)
                            .unwrap_or(0);
                        ui.label(format!("  {} (pri: {})", name, priority));
                    }
                }

                ui.separator();

                // Action states
                ui.heading("Actions");
                let mut phases = subsystem.action_phases();
                phases.sort_by(|a, b| a.0.cmp(&b.0));

                egui::Grid::new("input_debug_grid")
                    .striped(true)
                    .min_col_width(80.0)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Action").strong());
                        ui.label(egui::RichText::new("Phase").strong());
                        ui.label(egui::RichText::new("Value").strong());
                        ui.end_row();

                        for (name, phase, value) in &phases {
                            ui.label(name);
                            let phase_color = phase_color(*phase);
                            ui.label(
                                egui::RichText::new(phase.label())
                                    .color(phase_color)
                                    .monospace(),
                            );
                            ui.label(
                                egui::RichText::new(format_value(value)).monospace(),
                            );
                            ui.end_row();
                        }
                    });

                ui.separator();

                // Consumed sources
                let consumed = subsystem.consumed_sources();
                ui.label(format!("Consumed sources: {}", consumed.len()));
            });
    }
}

fn phase_color(phase: ActionPhase) -> egui::Color32 {
    match phase {
        ActionPhase::None => egui::Color32::GRAY,
        ActionPhase::Started => egui::Color32::from_rgb(100, 200, 255),
        ActionPhase::Triggered => egui::Color32::from_rgb(50, 255, 50),
        ActionPhase::Ongoing => egui::Color32::from_rgb(255, 200, 50),
        ActionPhase::Completed => egui::Color32::from_rgb(150, 150, 255),
        ActionPhase::Canceled => egui::Color32::from_rgb(255, 100, 100),
    }
}

fn format_value(value: &InputValue) -> String {
    match value {
        InputValue::Digital(b) => {
            if *b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        InputValue::Axis1D(v) => format!("{:.2}", v),
        InputValue::Axis2D(v) => format!("({:.2}, {:.2})", v.x, v.y),
        InputValue::Axis3D(v) => format!("({:.2}, {:.2}, {:.2})", v.x, v.y, v.z),
    }
}
