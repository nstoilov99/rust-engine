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
}
