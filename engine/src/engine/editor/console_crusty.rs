//! Console panel rendered with crusty-gui.

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::math::{Color, Pos2, Rect, Vec2};
use crusty_gui::text::FontFamily;
use crusty_gui::widgets::{CountChip, Label, ScrollArea, TextEdit};
use hecs::World;

use super::console::{ConsoleLog, LogFilter, LogLevel, LogMessage};
use super::console_cmd::{CommandContext, ConsoleCommandSystem};
use super::theme::Palette;

/// Borrowed console + command state.
pub struct ConsolePanelCtx<'a> {
    pub messages: &'a mut ConsoleLog,
    pub filter: &'a mut LogFilter,
    pub command_system: &'a mut ConsoleCommandSystem,
    pub input: &'a mut String,
    pub world: &'a mut World,
    pub show_stat_fps: &'a mut bool,
}

/// Count-chip filter toggle: active = status-tinted pill, muted = neutral
/// outline. The count stays visible either way.
fn filter_chip(ui: &mut Ui, show: &mut bool, label: String, color: Color) {
    if CountChip::new(label, color).active(*show).show(ui).clicked {
        *show = !*show;
    }
}

/// Draw the console into the dock tab's content rect (physical pixels).
pub fn console_panel(ui: &mut Ui, tab_rect: Rect, ctx: ConsolePanelCtx) {
    let rect = tab_rect;
    let style = ui.style();
    // Tight padding — the tab draws at its content rect with only item
    // spacing as inset.
    let opts = UiOptions {
        padding: Vec2::new(4.0, 2.0),
        spacing: style.spacing.item,
    };
    ui.run_at(
        rect,
        Direction::TopDown,
        Id::new("engine_console_panel"),
        opts,
        |ui| {
            let (info_n, warn_n, err_n) = ctx.messages.counts();
            let status = Palette::invariant_status();

            // Header: count-chip filters with live counts.
            ui.horizontal(|ui| {
                ui.set_spacing(6.0);
                filter_chip(
                    ui,
                    &mut ctx.filter.show_error,
                    format!("Errors {err_n}"),
                    status.error,
                );
                filter_chip(
                    ui,
                    &mut ctx.filter.show_warning,
                    format!("Warnings {warn_n}"),
                    status.warning,
                );
                filter_chip(
                    ui,
                    &mut ctx.filter.show_info,
                    format!("Info {info_n}"),
                    status.info,
                );
            });
            ui.separator();

            // Log area on the window bg, pinned to the bottom while the user
            // hasn't scrolled up.
            let input_h = 28.0;
            // Remaining height minus the separator (1px) and input row below.
            let log_h =
                (ui.available_size().y - input_h - 1.0 - style.spacing.item * 2.0).max(50.0);
            let log_bg = Rect::from_min_size(
                Pos2::new(rect.min.x, ui.cursor().y),
                Vec2::new(rect.width(), log_h),
            );
            ui.painter().rect_filled(log_bg, 0.0, style.palette.window);
            let font = 11.5;
            let row_h = 18.0;
            ScrollArea::new(log_h)
                .auto_shrink(false)
                .stick_to_bottom(true)
                .inset(0.0)
                .spacing(0.0)
                .show(ui, |ui| {
                    let mut shown = 0;
                    for msg in ctx.messages.iter() {
                        if !ctx.filter.should_show(msg) {
                            continue;
                        }
                        shown += 1;
                        let (tag_color, text_color) = match msg.level {
                            LogLevel::Info => (status.info, style.palette.text_secondary),
                            LogLevel::Warning => (status.warning, status.warning),
                            LogLevel::Error => (status.error, status.error),
                        };
                        for (i, line) in msg.text.split('\n').enumerate() {
                            let width = ui.available().width();
                            let row = ui.allocate(Vec2::new(width, row_h));
                            let clip = ui.clip_rect();
                            if row.max.y < clip.min.y || row.min.y > clip.max.y {
                                continue;
                            }
                            // Error rows get a full-width 8% status tint.
                            if msg.level == LogLevel::Error {
                                ui.painter()
                                    .rect_filled(row, 0.0, status.error.with_alpha(0.08));
                            }
                            let mut x = row.min.x + 6.0;
                            let y = row.min.y + (row_h - font * 1.25) * 0.5;
                            if i == 0 {
                                let tag = msg.level.prefix();
                                let tsz = ui.painter().text_family(
                                    Pos2::new(x, y),
                                    tag,
                                    font,
                                    tag_color,
                                    None,
                                    FontFamily::Mono,
                                );
                                x += tsz.x + 6.0;
                            }
                            ui.painter().text_family(
                                Pos2::new(x, y),
                                line,
                                font,
                                text_color,
                                None,
                                FontFamily::Mono,
                            );
                        }
                    }
                    if shown == 0 {
                        Label::new("No messages")
                            .color(style.palette.text_secondary)
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
                .fill(style.palette.input)
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
                let prev = ctx
                    .command_system
                    .history
                    .previous(ctx.input)
                    .map(str::to_string);
                if let Some(p) = prev {
                    *ctx.input = p;
                }
            }
            if out.nav_down {
                let next = ctx
                    .command_system
                    .history
                    .navigate_next()
                    .map(str::to_string);
                if let Some(n) = next {
                    *ctx.input = n;
                }
            }
            if out.cancelled {
                ctx.input.clear();
            }
        },
    );
}
