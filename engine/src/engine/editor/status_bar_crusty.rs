//! Status bar rendered with crusty-gui (Crusty Design System).
//!
//! 28px strip: panel toggle chips + command field on the left, mono status
//! clusters (unsaved count, FPS, ready state) separated by hairlines on the
//! right.

use std::collections::HashMap;

use crusty_gui::context::Ui;
use crusty_gui::math::{Pos2, Rect, Vec2};
use crusty_gui::paint::{PaintCmd, TextureId};
use crusty_gui::text::FontFamily;

use super::dock_crusty::CrustyDockLayout;
use super::theme::EditorTheme;
use super::EditorTab;

pub struct StatusBarCtx<'a> {
    pub dock_state: &'a mut CrustyDockLayout,
    pub theme: &'a EditorTheme,
    /// Console error count — shown as a pill on the Console chip when > 0.
    pub error_count: usize,
    /// Unsaved scenes/assets, warning-tinted when > 0.
    pub unsaved_count: usize,
    pub icons: &'a HashMap<String, TextureId>,
}

const CHIP_H: f32 = 22.0;
const MONO_SIZE: f32 = 11.0;

/// Returns true when the command field was clicked (host opens the palette).
pub fn status_bar_panel(ui: &mut Ui, bar_rect: Rect, mut ctx: StatusBarCtx) -> bool {
    let style = ui.style();
    let rect = bar_rect;

    ui.painter().rect_filled(rect, 0.0, style.palette.window);
    ui.painter().line_segment(
        rect.min,
        Pos2::new(rect.max.x, rect.min.y),
        1.0,
        style.palette.stroke,
    );

    let chip_y = rect.min.y + (rect.height() - CHIP_H) * 0.5;
    let mut x = rect.min.x + 8.0;

    // ── panel toggle chips (11px icon + label, per the mockup)
    let chips = [
        (EditorTab::Console, "Console", "list-view"),
        (EditorTab::AssetBrowser, "Assets", "grid-view"),
        (EditorTab::Profiler, "Profiler", "camera-speed"),
    ];
    for (tab, label, icon) in chips {
        let errors = if tab == EditorTab::Console {
            ctx.error_count
        } else {
            0
        };
        x = panel_chip(ui, &mut ctx, x, chip_y, &tab, label, icon, errors);
    }

    // ── command field: `> cmd…`, 170×22, opens the command palette
    x += 8.0;
    let field = Rect::from_min_size(Pos2::new(x, chip_y), Vec2::new(170.0, CHIP_H));
    let field_id = ui.alloc_id("status_cmd_field");
    let field_resp = ui.interact(field_id, field);
    let field_stroke = if field_resp.hovered {
        style.palette.stroke_strong
    } else {
        style.palette.stroke
    };
    ui.painter()
        .rect_filled(field, style.rounding.small, style.palette.input);
    ui.painter()
        .rect_stroke(field, style.rounding.small, 1.0, field_stroke);
    mono_text(
        ui,
        Pos2::new(field.min.x + 8.0, field.center().y - MONO_SIZE * 1.25 * 0.5),
        "> cmd\u{2026}",
        style.palette.text_secondary,
    );

    // ── right clusters (right-to-left): Ready · FPS · unsaved
    let cy = rect.center().y;
    let mut rx = rect.max.x - 10.0;

    let ready_text = "Ready";
    let ready_col = ctx.theme.palette.status.success;
    let tw = mono_width(ui, ready_text);
    rx -= tw;
    mono_text(ui, Pos2::new(rx, cy - MONO_SIZE * 1.25 * 0.5), ready_text, ready_col);
    let dot_r = 3.0;
    rx -= 6.0 + dot_r * 2.0;
    ui.painter().rect_filled(
        Rect::from_center_size(Pos2::new(rx + dot_r, cy), Vec2::splat(dot_r * 2.0)),
        dot_r,
        ready_col,
    );

    // No FPS/frame-time cluster — perf lives in the Profiler panel (mockup).
    rx = hairline(ui, rx, cy);

    let unsaved_text = format!("{} unsaved", ctx.unsaved_count);
    let unsaved_col = if ctx.unsaved_count > 0 {
        ctx.theme.palette.status.warning
    } else {
        style.palette.text_secondary
    };
    let tw = mono_width(ui, &unsaved_text);
    rx -= tw;
    mono_text(
        ui,
        Pos2::new(rx, cy - MONO_SIZE * 1.25 * 0.5),
        &unsaved_text,
        unsaved_col,
    );

    field_resp.clicked
}

/// One toggle chip; returns the next cursor x.
#[allow(clippy::too_many_arguments)]
fn panel_chip(
    ui: &mut Ui,
    ctx: &mut StatusBarCtx,
    x: f32,
    y: f32,
    tab: &EditorTab,
    label: &str,
    icon: &str,
    errors: usize,
) -> f32 {
    let style = ui.style();
    let font = style.fonts.body;
    let icon_tex = ctx.icons.get(icon).copied();
    let icon_w = if icon_tex.is_some() { 17.0 } else { 0.0 };
    let label_w = ui.text_mut().measure(label, font, None).x;

    // Error pill (mono count on a translucent error fill).
    let pill = (errors > 0).then(|| {
        let t = errors.to_string();
        let w = ui
            .text_mut()
            .measure_family(&t, style.fonts.small, None, FontFamily::Mono)
            .x;
        (t, w + 10.0)
    });
    let pill_w = pill.as_ref().map_or(0.0, |(_, w)| w + 6.0);

    let chip = Rect::from_min_size(
        Pos2::new(x, y),
        Vec2::new(icon_w + label_w + 18.0 + pill_w, CHIP_H),
    );
    let id = ui.alloc_id(("status_chip", label));
    let resp = ui.interact(id, chip);

    let is_open = ctx.dock_state.is_tab_open(tab);
    if is_open || resp.hovered {
        ui.painter()
            .rect_filled(chip, style.rounding.small, style.palette.hover);
    }
    let text_col = if is_open {
        style.palette.text
    } else {
        style.palette.text_secondary
    };
    if let Some(tex) = icon_tex {
        ui.ctx_mut().paint.push(PaintCmd::Image {
            rect: Rect::from_center_size(
                Pos2::new(chip.min.x + 9.0 + 5.5, chip.center().y),
                Vec2::splat(11.0),
            ),
            uv_min: Pos2::new(0.0, 0.0),
            uv_max: Pos2::new(1.0, 1.0),
            tint: text_col,
            texture: tex,
        });
    }
    ui.painter().text(
        Pos2::new(
            chip.min.x + 9.0 + icon_w,
            chip.center().y - font * 1.25 * 0.5,
        ),
        label,
        font,
        text_col,
        None,
    );

    if let Some((count, w)) = pill {
        let err = ctx.theme.palette.status.error;
        let pr = Rect::from_min_size(
            Pos2::new(
                chip.min.x + 9.0 + icon_w + label_w + 6.0,
                chip.center().y - 7.0,
            ),
            Vec2::new(w, 14.0),
        );
        ui.painter().rect_filled(pr, 7.0, err.with_alpha(0.15));
        let tint = crusty_gui::math::Color::rgba(
            err.r + (1.0 - err.r) * 0.4,
            err.g + (1.0 - err.g) * 0.4,
            err.b + (1.0 - err.b) * 0.4,
            1.0,
        );
        let small = style.fonts.small;
        let tw = ui
            .painter()
            .measure_text_family(&count, small, None, FontFamily::Mono)
            .x;
        ui.painter().text_family(
            Pos2::new(pr.center().x - tw * 0.5, pr.center().y - small * 1.25 * 0.5),
            &count,
            small,
            tint,
            None,
            FontFamily::Mono,
        );
    }

    if resp.clicked {
        if is_open {
            ctx.dock_state.remove_tab(tab);
        } else {
            ctx.dock_state.open_tab(tab.clone());
        }
    }

    chip.max.x + 4.0
}

/// 1×14px vertical separator; returns the next right-edge x.
fn hairline(ui: &mut Ui, rx: f32, cy: f32) -> f32 {
    let stroke = ui.style().palette.stroke;
    let x = rx - 10.0;
    ui.painter()
        .line_segment(Pos2::new(x, cy - 7.0), Pos2::new(x, cy + 7.0), 1.0, stroke);
    x - 10.0
}

fn mono_text(ui: &mut Ui, pos: Pos2, s: &str, color: crusty_gui::math::Color) {
    ui.painter()
        .text_family(pos, s, MONO_SIZE, color, None, FontFamily::Mono);
}

fn mono_width(ui: &mut Ui, s: &str) -> f32 {
    ui.painter()
        .measure_text_family(s, MONO_SIZE, None, FontFamily::Mono)
        .x
}
