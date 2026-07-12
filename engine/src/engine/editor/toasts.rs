//! Lightweight editor toast notifications.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub kind: ToastKind,
    pub message: String,
    pub created_at: Instant,
    pub duration: Duration,
}

impl Toast {
    fn new(kind: ToastKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            created_at: Instant::now(),
            duration: Duration::from_secs(4),
        }
    }

    fn expired(&self, now: Instant) -> bool {
        now.duration_since(self.created_at) >= self.duration
    }
}

#[derive(Debug, Default)]
pub struct ToastStack {
    toasts: Vec<Toast>,
}

impl ToastStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, kind: ToastKind, message: impl Into<String>) {
        self.toasts.push(Toast::new(kind, message));
    }

    pub fn info(&mut self, message: impl Into<String>) {
        self.push(ToastKind::Info, message);
    }

    pub fn success(&mut self, message: impl Into<String>) {
        self.push(ToastKind::Success, message);
    }

    pub fn warning(&mut self, message: impl Into<String>) {
        self.push(ToastKind::Warning, message);
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.push(ToastKind::Error, message);
    }

    pub fn len(&self) -> usize {
        self.toasts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }

    pub fn clear(&mut self) {
        self.toasts.clear();
    }

    /// Drop expired toasts and return the survivors (oldest first).
    pub fn prune(&mut self) -> &[Toast] {
        let now = Instant::now();
        self.toasts.retain(|toast| !toast.expired(now));
        &self.toasts
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.toasts.retain(|toast| !toast.expired(now));
        if self.toasts.is_empty() {
            return;
        }

        ctx.request_repaint_after(Duration::from_millis(250));

        egui::Area::new(egui::Id::new("editor_toast_stack"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-16.0, 40.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    ui.set_max_width(360.0);
                    for toast in self.toasts.iter().rev().take(5) {
                        let (label, color) = match toast.kind {
                            ToastKind::Info => ("Info", egui::Color32::from_rgb(90, 150, 220)),
                            ToastKind::Success => ("Saved", egui::Color32::from_rgb(80, 180, 110)),
                            ToastKind::Warning => {
                                ("Warning", egui::Color32::from_rgb(220, 170, 70))
                            }
                            ToastKind::Error => ("Error", egui::Color32::from_rgb(220, 90, 90)),
                        };

                        egui::Frame::popup(ui.style())
                            .fill(egui::Color32::from_rgba_unmultiplied(28, 30, 34, 235))
                            .stroke(egui::Stroke::new(1.0, color))
                            .corner_radius(4.0)
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.colored_label(color, label);
                                    ui.label(toast.message.as_str());
                                });
                            });
                        ui.add_space(6.0);
                    }
                });
            });
    }
}
