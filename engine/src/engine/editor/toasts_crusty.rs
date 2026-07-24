//! Toast notifications (Crusty Design System).
//!
//! Bottom-right stack above the status bar, newest nearest the corner, max
//! 3 visible. Each toast: E3 surface, 3px status rail, wrapped message,
//! close button. Errors are sticky (see `ToastStack::prune`).

use crusty_gui::context::Ui;
use crusty_gui::layer::Order;
use crusty_gui::math::{Pos2, Rect, Rounding, Vec2};

use super::theme::EditorTheme;
use super::toasts::{ToastKind, ToastStack};

const WIDTH: f32 = 300.0;
const RAIL_W: f32 = 3.0;
const PAD: f32 = 8.0;
const GAP: f32 = 8.0;
const MARGIN: f32 = 12.0;
/// Clears the 28px status bar strip.
const BOTTOM_MARGIN: f32 = 28.0 + MARGIN;
const MAX_VISIBLE: usize = 3;

pub fn toasts_panel(ui: &mut Ui, screen_rect: Rect, stack: &mut ToastStack, theme: &EditorTheme) {
    stack.prune();
    if stack.is_empty() {
        return;
    }

    let style = ui.style();
    let font = style.fonts.body;
    let total = stack.len();
    let first = total.saturating_sub(MAX_VISIBLE);
    let msg_max = WIDTH - RAIL_W - PAD * 2.0 - 16.0;

    // Layout newest at the bottom, stacking upward.
    let right = screen_rect.max.x - MARGIN;
    let mut bottom = screen_rect.max.y - BOTTOM_MARGIN;
    let mut rects: Vec<(usize, Rect)> = Vec::new();
    for idx in (first..total).rev() {
        let msg = &stack.as_slice()[idx].message;
        let msg_h = ui.text_mut().measure(msg, font, Some(msg_max)).y;
        let h = (msg_h + PAD * 2.0).max(30.0);
        let rect = Rect::from_min_size(Pos2::new(right - WIDTH, bottom - h), Vec2::new(WIDTH, h));
        rects.push((idx, rect));
        bottom = rect.min.y - GAP;
    }

    // Top-order area so close buttons win pointer priority over the dock.
    let area_id = ui.alloc_id("toasts_area");
    let union = rects.iter().fold(rects[0].1, |acc, (_, r)| acc.union(*r));
    let area = ui
        .ctx_mut()
        .memory
        .area_entry(area_id, union.min, Order::Tooltip);
    area.pos = union.min;
    area.size = union.size();
    let prev_area = ui.area();
    ui.set_area(Some(area_id));

    let mut remove = None;
    for (idx, rect) in &rects {
        let toast = &stack.as_slice()[*idx];
        let rail_color = match toast.kind {
            ToastKind::Info => theme.palette.status.info,
            ToastKind::Success => theme.palette.status.success,
            ToastKind::Warning => theme.palette.status.warning,
            ToastKind::Error => theme.palette.status.error,
        };
        let message = toast.message.clone();

        let close = Rect::from_center_size(
            Pos2::new(rect.max.x - 12.0, rect.min.y + 12.0),
            Vec2::splat(16.0),
        );
        let close_id = ui.alloc_id(("toast_close", *idx));
        let resp = ui.interact(close_id, close);
        if resp.clicked {
            remove = Some(*idx);
        }
        let close_col = if resp.hovered {
            style.palette.text
        } else {
            style.palette.text_secondary
        };

        let r = style.rounding.widget;
        let rail_rounding = Rounding {
            nw: r.nw,
            ne: 0.0,
            sw: r.sw,
            se: 0.0,
        };
        let mut p = ui.overlay_painter();
        p.rect_filled(
            *rect,
            r,
            style.palette.elevated.with_alpha(style.palette.popover_alpha),
        );
        p.rect_stroke(*rect, r, style.metrics.border, style.palette.stroke_strong);
        p.rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(RAIL_W, rect.height())),
            rail_rounding,
            rail_color,
        );
        p.text(
            Pos2::new(rect.min.x + RAIL_W + PAD, rect.min.y + PAD),
            &message,
            font,
            style.palette.text,
            Some(msg_max),
        );
        let x_str = "\u{00D7}";
        let x_size = p.measure_text(x_str, font, None);
        p.text(
            Pos2::new(
                close.center().x - x_size.x * 0.5,
                close.center().y - x_size.y * 0.5,
            ),
            x_str,
            font,
            close_col,
            None,
        );
    }
    ui.set_area(prev_area);

    if let Some(idx) = remove {
        stack.remove(idx);
    }
}
