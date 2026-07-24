//! Crusty-gui command palette popover (Crusty Design System).
//!
//! Fixed 420px popover at 18% viewport height: search field, 26px result
//! rows (selection_fill/selection_text), footer hint. Renders into
//! `overlay_paint` on an E3 surface so it floats above every panel.

use super::command_palette::{CommandPalette, CommandRegistry, EditorAction};
use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::input::Key;
use crusty_gui::layer::Order;
use crusty_gui::math::{Color, Pos2, Rect, Vec2};
use crusty_gui::paint::{PaintCmd, RectShape};
use crusty_gui::widgets::{ScrollArea, TextEdit};

const WIDTH: f32 = 420.0;
const ROW_H: f32 = 26.0;
const MAX_LIST_H: f32 = ROW_H * 9.0;

/// Fixed search popover over the command registry. Returns the action to
/// dispatch when a command is chosen (Enter or click); closes on Escape or
/// an outside click.
pub fn command_palette_panel(
    ui: &mut Ui,
    palette: &mut CommandPalette,
    registry: &CommandRegistry,
) -> Option<EditorAction> {
    if !palette.open {
        return None;
    }

    let style = ui.style();
    let screen = ui.ctx().screen_rect();
    let origin = Pos2::new(screen.center().x - WIDTH * 0.5, screen.height() * 0.18);

    let mut selected_action = None;
    let mut should_close = false;

    let palette_id = ui.alloc_id("command_palette");
    let area_id = palette_id.with("area");
    let scratch = Rect::from_min_size(origin, Vec2::new(WIDTH, 600.0));

    // Body paints to the main list first, then gets sandwiched into
    // overlay_paint above the popover background (ComboBox pattern).
    let body_start = ui.ctx().paint.len();

    let opts = UiOptions {
        padding: Vec2::new(8.0, 8.0),
        spacing: 6.0,
    };
    let (_, used) = ui.run_at(
        scratch,
        Direction::TopDown,
        palette_id.with("root"),
        opts,
        |inner| {
            inner.set_area(Some(area_id));

            let out = TextEdit::new(&mut palette.query)
                .hint("Search commands\u{2026}")
                .width(inner.available().width())
                .request_focus(std::mem::take(&mut palette.just_opened))
                .keep_focus_on_submit(true)
                .show_full(inner);

            let matches = registry.search(&palette.query);
            if palette.selected >= matches.len() {
                palette.selected = matches.len().saturating_sub(1);
            }

            if out.cancelled {
                should_close = true;
            }
            if out.nav_down {
                palette.selected = (palette.selected + 1).min(matches.len().saturating_sub(1));
            }
            if out.nav_up {
                palette.selected = palette.selected.saturating_sub(1);
            }
            if out.submitted {
                if let Some(command) = matches.get(palette.selected) {
                    selected_action = Some(command.action.clone());
                    should_close = true;
                }
            }

            inner.separator();
            let list_h = ((matches.len() as f32) * ROW_H).clamp(ROW_H, MAX_LIST_H);
            ScrollArea::new(list_h)
                .auto_shrink(false)
                .inset(0.0)
                .spacing(0.0)
                .show(inner, |ui| {
                    if matches.is_empty() {
                        let dim = ui.style().palette.text_secondary;
                        crusty_gui::widgets::Label::new("No matching commands")
                            .color(dim)
                            .show(ui);
                    }
                    for (index, command) in matches.iter().enumerate() {
                        if command_row(ui, index == palette.selected, command.label.as_str()) {
                            palette.selected = index;
                            selected_action = Some(command.action.clone());
                            should_close = true;
                        }
                    }
                });

            inner.separator();
            let hint = "\u{2191}\u{2193} navigate \u{00B7} Enter run \u{00B7} Esc close";
            crusty_gui::widgets::Label::new(hint)
                .size(style.fonts.small)
                .color(style.palette.text_secondary)
                .show(inner);
        },
    );
    let used_h = used.height();

    let drained: Vec<PaintCmd> = ui.ctx_mut().paint.drain(body_start..).collect();
    let popover_rect = Rect::from_min_size(origin, Vec2::new(WIDTH, used_h + 16.0));

    ui.ctx_mut().overlay_paint.push(PaintCmd::Rrect(
        RectShape::fill(
            popover_rect,
            style.rounding.panel,
            style.palette.elevated.with_alpha(style.palette.popover_alpha),
        )
        .with_stroke(style.metrics.border, style.palette.stroke_strong)
        .with_shadow(24.0, Color::BLACK.with_alpha(0.45)),
    ));
    ui.ctx_mut().overlay_paint.extend(drained);

    // Top-order area so palette rows win pointer priority over panels below.
    let area = ui
        .ctx_mut()
        .memory
        .area_entry(area_id, popover_rect.min, Order::Tooltip);
    area.pos = popover_rect.min;
    area.size = popover_rect.size();

    if ui.ctx().input.key_pressed(Key::Escape) {
        should_close = true;
    }
    if ui.ctx().input.pointer_pressed {
        if let Some(p) = ui.ctx().input.pointer_pos {
            if !popover_rect.contains(p) {
                should_close = true;
            }
        }
    }

    if should_close {
        palette.close();
    }
    selected_action
}

/// One 26px palette row; selected rows use the selection tokens.
fn command_row(ui: &mut Ui, selected: bool, label: &str) -> bool {
    let style = ui.style();
    let id = ui.alloc_id(&label);
    let rect = ui.allocate(Vec2::new(ui.available().width(), ROW_H));

    let hovered = ui.contains_pointer(rect);
    let pointer_pressed = ui.ctx().input.pointer_pressed;
    let pointer_released = ui.ctx().input.pointer_released;
    if pointer_pressed && hovered {
        ui.ctx_mut().memory.active_widget = Some(id);
    }
    let is_active = ui.ctx().memory.active_widget == Some(id) || (pointer_pressed && hovered);
    let clicked = pointer_released && is_active && hovered;

    let fill = if selected {
        style.palette.selection_fill
    } else if hovered {
        style.palette.hover
    } else {
        Color::TRANSPARENT
    };
    let text_color = if selected {
        style.palette.selection_text
    } else {
        style.palette.text
    };
    let text_pos = Pos2::new(
        rect.min.x + 8.0,
        rect.min.y + (ROW_H - style.fonts.body * 1.25) * 0.5,
    );
    let mut painter = ui.painter();
    if fill.a > 0.0 {
        painter.rect_filled(rect, style.rounding.small, fill);
    }
    painter.text(text_pos, label, style.fonts.body, text_color, None);

    clicked
}
