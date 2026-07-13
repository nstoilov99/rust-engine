//! Crusty-gui port of the command palette (Phase 16). Mirrors
//! `CommandPalette::show` in `command_palette/mod.rs`.

use super::command_palette::{CommandPalette, CommandRegistry, EditorAction};
use crusty_gui::context::Ui;
use crusty_gui::math::{Color, Pos2, Vec2};
use crusty_gui::widgets::{ScrollArea, TextEdit, Window};

/// Top-center search popup over the command registry. Returns the action to
/// dispatch when a command is chosen (Enter or click); closes on Escape.
/// The window is movable and resizable; top-center is only the initial spot.
pub fn command_palette_panel(
    ui: &mut Ui,
    palette: &mut CommandPalette,
    registry: &CommandRegistry,
) -> Option<EditorAction> {
    if !palette.open {
        return None;
    }

    let mut selected_action = None;
    let mut should_close = false;

    const WIDTH: f32 = 360.0;
    let screen = ui.ctx().screen_rect();
    let top_center = Pos2::new(screen.center().x - WIDTH * 0.5, 80.0);

    Window::new("Command Palette")
        .collapsible(false)
        .default_pos(top_center)
        .default_size(Vec2::new(WIDTH, 420.0))
        .show(ui, |ui| {
            let out = TextEdit::new(&mut palette.query)
                .hint("Search commands")
                .width(ui.available().width())
                .request_focus(std::mem::take(&mut palette.just_opened))
                .keep_focus_on_submit(true)
                .show_full(ui);

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

            ui.separator();
            let list_height = ui.available().height();
            ScrollArea::new(list_height).auto_shrink(false).show(ui, |ui| {
                for (index, command) in matches.iter().enumerate() {
                    if command_row(ui, index == palette.selected, command.label.as_str()) {
                        palette.selected = index;
                        selected_action = Some(command.action.clone());
                        should_close = true;
                    }
                }
            });
        });

    if should_close {
        palette.close();
    }
    selected_action
}

/// One palette row. Selected rows use the engine's muted highlight (dark
/// amber fill, accent text) instead of crusty's bright accent fill so the
/// label stays readable. Returns true when clicked.
fn command_row(ui: &mut Ui, selected: bool, label: &str) -> bool {
    let style = ui.style();
    let id = ui.alloc_id(&label);

    let label_size = ui.text_mut().measure(label, style.fonts.body, None);
    let height = label_size.y + 6.0;
    let rect = ui.allocate(Vec2::new(ui.available().width(), height));

    let hovered = ui.contains_pointer(rect);
    let pointer_pressed = ui.ctx().input.pointer_pressed;
    let pointer_released = ui.ctx().input.pointer_released;
    if pointer_pressed && hovered {
        ui.ctx_mut().memory.active_widget = Some(id);
    }
    let is_active = ui.ctx().memory.active_widget == Some(id) || (pointer_pressed && hovered);
    let clicked = pointer_released && is_active && hovered;

    let fill = if selected {
        Color::from_srgb_u8(98, 59, 35, 255)
    } else if hovered {
        style.palette.surface_hover
    } else {
        Color::TRANSPARENT
    };
    let text_color = if selected {
        style.palette.accent
    } else {
        style.palette.text
    };
    let text_pos = Pos2::new(
        rect.min.x + 8.0,
        rect.min.y + (rect.height() - label_size.y) * 0.5,
    );
    let mut painter = ui.painter();
    if fill.a > 0.0 {
        painter.rect_filled(rect, style.rounding.small, fill);
    }
    painter.text(text_pos, label, style.fonts.body, text_color, None);

    clicked
}
