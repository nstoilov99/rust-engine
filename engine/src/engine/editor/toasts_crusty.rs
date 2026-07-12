//! Toast notifications rendered with crusty-gui (Phase 16 port).
//!
//! Reads the same `ToastStack` as the egui version in `toasts.rs` (the caller
//! prunes expired toasts via `ToastStack::prune` and hands us the slice).
//! Anchored top-right like the egui `Area`, latest five, newest on top.

use crusty_gui::context::Ui;
use crusty_gui::math::{Color, Pos2, Rect, Vec2};

use super::toasts::{Toast, ToastKind};

const MAX_WIDTH: f32 = 360.0;
const PAD: f32 = 6.0;
const GAP: f32 = 6.0;

fn kind_style(kind: ToastKind) -> (&'static str, Color) {
    match kind {
        ToastKind::Info => ("Info", Color::from_srgb_u8(90, 150, 220, 255)),
        ToastKind::Success => ("Saved", Color::from_srgb_u8(80, 180, 110, 255)),
        ToastKind::Warning => ("Warning", Color::from_srgb_u8(220, 170, 70, 255)),
        ToastKind::Error => ("Error", Color::from_srgb_u8(220, 90, 90, 255)),
    }
}

pub fn toasts_panel(ui: &mut Ui, screen_rect: egui::Rect, ppp: f32, toasts: &[Toast]) {
    if toasts.is_empty() {
        return;
    }
    let font = ui.style().fonts.body;
    let text_color = ui.style().palette.text;
    let fill = Color::from_srgb_u8(28, 30, 34, 235);
    let s = ppp;

    let right = screen_rect.max.x * s - 16.0 * s;
    let mut y = screen_rect.min.y * s + 40.0 * s;

    for toast in toasts.iter().rev().take(5) {
        let (label, color) = kind_style(toast.kind);
        let label_size = ui.text_mut().measure(label, font, None);
        let msg_max = MAX_WIDTH * s - PAD * s * 2.0 - label_size.x - 4.0 * s;
        let msg_size = ui
            .text_mut()
            .measure(&toast.message, font, Some(msg_max.max(40.0)));

        let inner_w = label_size.x + 4.0 * s + msg_size.x;
        let inner_h = msg_size.y.max(label_size.y);
        let panel = Rect::from_min_size(
            Pos2::new(
                right - (inner_w + PAD * s * 2.0),
                y,
            ),
            Vec2::new(inner_w + PAD * s * 2.0, inner_h + PAD * s * 2.0),
        );

        ui.painter().rect_filled(panel, 4.0 * s, fill);
        ui.painter().rect_stroke(panel, 4.0 * s, 1.0, color);

        let tx = panel.min.x + PAD * s;
        let ty = panel.min.y + PAD * s;
        ui.painter().text(Pos2::new(tx, ty), label, font, color, None);
        ui.painter().text(
            Pos2::new(tx + label_size.x + 4.0 * s, ty),
            &toast.message,
            font,
            text_color,
            Some(msg_max.max(40.0)),
        );

        y = panel.max.y + GAP * s;
    }
}
