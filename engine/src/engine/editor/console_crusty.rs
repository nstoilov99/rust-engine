//! Console panel rendered with crusty-gui (Phase 16 port).
//!
//! Reads/writes the exact same state as the egui version in
//! `tab_viewer::render_console`; with the `crusty` feature enabled the
//! egui tab only records its content rect and this panel draws there.

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::math::{Color, Pos2, Rect, Vec2};
use crusty_gui::widgets::{Button, Label, ScrollArea, TextEdit};
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

/// Filter toggle button matching the egui console header: `active_fill`
/// when the filter is on, dark gray otherwise; label tinted by log level.
fn filter_button(
    ui: &mut Ui,
    show: &mut bool,
    label: String,
    level: LogLevel,
    active_fill: Color,
) {
    let inactive_fill = Color::from_srgb_u8(45, 45, 45, 255);
    let inactive_text = Color::from_srgb_u8(160, 160, 160, 255);
    let (fill, text) = if *show {
        (active_fill, level_color(level))
    } else {
        (inactive_fill, inactive_text)
    };
    if Button::new(label).fill(fill).text_color(text).show(ui).clicked {
        *show = !*show;
    }
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
    // Tight padding to match the egui tab, which draws at its content rect
    // with only item spacing as inset.
    let opts = UiOptions {
        padding: Vec2::new(4.0 * ppp, 2.0 * ppp),
        spacing: style.spacing.item,
    };
    ui.run_at(rect, Direction::TopDown, Id::new("engine_console_panel"), opts, |ui| {
        let (info_n, warn_n, err_n) = ctx.messages.counts();

        // Header: heading + filter toggles with live counts, mirroring the
        // egui version's custom-styled buttons.
        ui.horizontal(|ui| {
            Label::new("Console").size(style.fonts.title).show(ui);
            ui.separator();
            filter_button(
                ui,
                &mut ctx.filter.show_error,
                format!("Errors ({err_n})"),
                LogLevel::Error,
                Color::from_srgb_u8(100, 50, 50, 180),
            );
            filter_button(
                ui,
                &mut ctx.filter.show_warning,
                format!("Warnings ({warn_n})"),
                LogLevel::Warning,
                Color::from_srgb_u8(100, 80, 40, 180),
            );
            filter_button(
                ui,
                &mut ctx.filter.show_info,
                format!("Info ({info_n})"),
                LogLevel::Info,
                Color::from_srgb_u8(60, 70, 90, 180),
            );
        });
        ui.separator();

        // Log area, pinned to the bottom while the user hasn't scrolled up.
        let input_h = style.fonts.body + 10.0;
        // Remaining height minus the separator (1px) and input row below.
        let log_h =
            (ui.available_size().y - input_h - 1.0 - style.spacing.item * 2.0).max(50.0);
        ScrollArea::new(log_h)
            .auto_shrink(false)
            .stick_to_bottom(true)
            .inset(0.0)
            .show(ui, |ui| {
                let mut shown = 0;
                for msg in ctx.messages.iter() {
                    if ctx.filter.should_show(msg) {
                        Label::new(format!("{} {}", msg.level.prefix(), msg.text))
                            .color(level_color(msg.level))
                            .show(ui);
                        shown += 1;
                    }
                }
                if shown == 0 {
                    Label::new("No messages")
                        .color(style.palette.text_dim)
                        .show(ui);
                }
            });
        ui.separator();

        // Command input: Enter submits (keeps focus), Up/Down = history,
        // Escape clears.
        let out = TextEdit::new(ctx.input)
            .width(ui.available().width())
            .hint("Enter command (type 'help' for available commands)")
            .keep_focus_on_submit(true)
            .fill(style.palette.surface)
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
