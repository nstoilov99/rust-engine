//! Console panel rendered with crusty-gui (Phase 16 port).
//!
//! Reads/writes the exact same state as the egui version in
//! `tab_viewer::render_console`; with the `crusty` feature enabled the
//! egui tab only records its content rect and this panel draws there.

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::math::{Color, Pos2, Rect, Vec2};
use crusty_gui::widgets::{Label, ScrollArea, TextEdit};
use hecs::World;

use super::console::{ConsoleLog, LogFilter, LogLevel, LogMessage};
use super::console_cmd::{CommandContext, ConsoleCommandSystem};

/// Borrowed console + command state — the same fields the egui tab viewer
/// uses, so both UIs stay in lockstep.
pub struct ConsolePanelCtx<'a> {
    pub messages: &'a mut ConsoleLog,
    pub filter: &'a mut LogFilter,
    pub command_system: &'a mut ConsoleCommandSystem,
    pub input: &'a mut String,
    pub world: &'a mut World,
    pub show_stat_fps: &'a mut bool,
}

fn level_color(level: LogLevel) -> Color {
    let c = level.color();
    Color::from_srgb_u8(c.r(), c.g(), c.b(), c.a())
}

/// Draw the console into the dock tab's content rect. `tab_rect` is in
/// egui points; `ppp` (egui pixels_per_point) maps it into crusty's
/// physical-pixel space.
pub fn console_panel(ui: &mut Ui, tab_rect: egui::Rect, ppp: f32, ctx: ConsolePanelCtx) {
    let rect = Rect::from_min_max(
        Pos2::new(tab_rect.min.x * ppp, tab_rect.min.y * ppp),
        Pos2::new(tab_rect.max.x * ppp, tab_rect.max.y * ppp),
    );
    let style = ui.style();
    let opts = UiOptions {
        padding: Vec2::new(style.spacing.padding, style.spacing.padding * 0.5),
        spacing: style.spacing.item,
    };
    ui.run_at(rect, Direction::TopDown, Id::new("engine_console_panel"), opts, |ui| {
        let (info_n, warn_n, err_n) = ctx.messages.counts();

        // Header: filter toggles with live counts.
        ui.horizontal(|ui| {
            ui.checkbox(&mut ctx.filter.show_error, format!("Errors ({err_n})"));
            ui.checkbox(&mut ctx.filter.show_warning, format!("Warnings ({warn_n})"));
            ui.checkbox(&mut ctx.filter.show_info, format!("Info ({info_n})"));
        });

        // Log area, pinned to the bottom while the user hasn't scrolled up.
        let input_h = style.fonts.body + 10.0;
        let log_h = (ui.available().height() - input_h - style.spacing.item * 2.0).max(50.0);
        ScrollArea::new(log_h)
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                let mut shown = 0;
                for msg in ctx.messages.iter() {
                    if ctx.filter.should_show(msg) {
                        Label::new(format!("{} {}", msg.level.prefix(), msg.text))
                            .size(style.fonts.small)
                            .color(level_color(msg.level))
                            .show(ui);
                        shown += 1;
                    }
                }
                if shown == 0 {
                    Label::new("No messages")
                        .size(style.fonts.small)
                        .color(style.palette.text_dim)
                        .show(ui);
                }
            });
        ui.add_space(style.spacing.item);

        // Command input: Enter submits (keeps focus), Up/Down = history,
        // Escape clears.
        let out = TextEdit::new(ctx.input)
            .width(ui.available().width())
            .hint("Enter command (type 'help' for available commands)")
            .keep_focus_on_submit(true)
            .show_full(ui);

        if out.submitted {
            let cmd = std::mem::take(ctx.input);
            if !cmd.is_empty() {
                ctx.messages.push(LogMessage::info(format!("> {cmd}")));
                let mut cmd_ctx = CommandContext::new(ctx.world, ctx.show_stat_fps);
                let output = ctx.command_system.execute(&cmd, &mut cmd_ctx);
                if output.len() == 1 && output[0].text == "__CLEAR__" {
                    ctx.messages.clear();
                } else {
                    ctx.messages.extend(output);
                }
            }
        }
        if out.nav_up {
            let prev = ctx.command_system.history.previous(ctx.input).map(str::to_string);
            if let Some(p) = prev {
                *ctx.input = p;
            }
        }
        if out.nav_down {
            let next = ctx.command_system.history.navigate_next().map(str::to_string);
            if let Some(n) = next {
                *ctx.input = n;
            }
        }
        if out.cancelled {
            ctx.input.clear();
        }
    });
}
