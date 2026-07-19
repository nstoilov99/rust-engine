//! Standalone in-game HUD, drawn with crusty-gui (M7 D7): connection chip,
//! unit frames, action bar with cooldown sweeps, cast bar, respawn overlay.

use crate::net::{HudState, HudTarget};
use game_client_net::OwnCombat;
use game_shared::combat::ABILITIES;
use rust_engine::engine::gui::crusty::{Color, Painter, Pos2, Rect, Ui, Vec2};

const MARGIN: f32 = 10.0;
const FONT_PX: f32 = 13.0;
const SMALL_PX: f32 = 11.0;
const SLOT: f32 = 48.0;
const GAP: f32 = 6.0;
const FRAME_W: f32 = 190.0;
const FRAME_H: f32 = 66.0;
const BAR_H: f32 = 14.0;

const PANEL_BG: Color = Color::rgba(0.0, 0.0, 0.0, 0.55);
const BAR_BG: Color = Color::rgba(0.12, 0.12, 0.14, 0.85);
const HP_COL: Color = Color::rgb(0.30, 0.72, 0.30);
const MANA_COL: Color = Color::rgb(0.25, 0.45, 0.90);
const CAST_COL: Color = Color::rgb(0.85, 0.62, 0.20);
const DIM_TEXT: Color = Color::rgba(1.0, 1.0, 1.0, 0.65);

pub fn draw(ui: &mut Ui, state: Option<&HudState>) {
    let Some(s) = state else { return };
    let screen = ui.available();
    let mut p = ui.painter();
    chip(&mut p, screen, &s.status);
    if let Some(own) = &s.own {
        own_frame(&mut p, own);
        action_bar(&mut p, screen, own, s.server_now_us);
        cast_bar(&mut p, screen, own, s.server_now_us);
        if !own.alive {
            respawn_overlay(&mut p, screen, own, s.server_now_us);
        }
    }
    if let Some(t) = &s.target {
        target_frame(&mut p, t, s.server_now_us);
    }
}

fn secs_until(now_us: u64, at_us: u64) -> f32 {
    at_us.saturating_sub(now_us) as f32 / 1_000_000.0
}

/// Top-right chip with the net status line.
fn chip(p: &mut Painter, screen: Rect, status: &str) {
    let pad = Vec2::new(8.0, 4.0);
    let text_size = p.measure_text(status, FONT_PX, None);
    let size = text_size + pad * 2.0;
    let min = Pos2::new(screen.max.x - MARGIN - size.x, screen.min.y + MARGIN);
    p.rect_filled(Rect::from_min_size(min, size), 4.0, PANEL_BG);
    p.text(min + pad, status, FONT_PX, Color::WHITE, None);
}

/// Translucent overlay on top of other widgets. Rrect fills take the glass
/// shader path, which writes opaque against the standalone's null backdrop
/// and would erase everything beneath — so overlays are plain alpha-blended
/// geometry, corners beveled to hug the slot rounding.
fn overlay(p: &mut Painter, rect: Rect, cut: f32, color: Color) {
    let (l, t, r, b) = (rect.min.x, rect.min.y, rect.max.x, rect.max.y);
    p.convex_polygon_filled(
        vec![
            Pos2::new(l + cut, t),
            Pos2::new(r - cut, t),
            Pos2::new(r, t + cut),
            Pos2::new(r, b - cut),
            Pos2::new(r - cut, b),
            Pos2::new(l + cut, b),
            Pos2::new(l, b - cut),
            Pos2::new(l, t + cut),
        ],
        color,
    );
}

/// Filled fraction bar with a centered label.
fn bar(p: &mut Painter, rect: Rect, frac: f32, fill: Color, label: &str) {
    p.rect_filled(rect, 2.0, BAR_BG);
    let w = rect.width() * frac.clamp(0.0, 1.0);
    if w > 0.5 {
        p.rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(w, rect.height())),
            2.0,
            fill,
        );
    }
    let size = p.measure_text(label, SMALL_PX, None);
    p.text(
        rect.center() - size * 0.5,
        label,
        SMALL_PX,
        Color::WHITE,
        None,
    );
}

fn unit_frame(p: &mut Painter, min: Pos2, name: &str, name_col: Color) -> Rect {
    let panel = Rect::from_min_size(min, Vec2::new(FRAME_W, FRAME_H));
    p.rect_filled(panel, 4.0, PANEL_BG);
    p.text(Pos2::new(min.x + 8.0, min.y + 5.0), name, FONT_PX, name_col, None);
    panel
}

fn frame_bar_rect(panel: Rect, row: usize) -> Rect {
    Rect::from_min_size(
        Pos2::new(panel.min.x + 8.0, panel.min.y + 24.0 + row as f32 * (BAR_H + 6.0)),
        Vec2::new(FRAME_W - 16.0, BAR_H),
    )
}

/// Top-left: local player hp + mana.
fn own_frame(p: &mut Painter, own: &OwnCombat) {
    let panel = unit_frame(p, Pos2::new(MARGIN, MARGIN), "You", Color::WHITE);
    bar(
        p,
        frame_bar_rect(panel, 0),
        own.hp / own.hp_max.max(1.0),
        HP_COL,
        &format!("{:.0} / {:.0}", own.hp, own.hp_max),
    );
    bar(
        p,
        frame_bar_rect(panel, 1),
        own.mana / own.mana_max.max(1.0),
        MANA_COL,
        &format!("{:.0} / {:.0}", own.mana, own.mana_max),
    );
}

/// Beside the own frame: target hp, plus the target's cast if any.
fn target_frame(p: &mut Painter, t: &HudTarget, now_us: u64) {
    let min = Pos2::new(MARGIN + FRAME_W + MARGIN, MARGIN);
    let name_col = if t.alive {
        Color::WHITE
    } else {
        Color::rgba(1.0, 0.45, 0.45, 1.0)
    };
    let panel = unit_frame(p, min, &t.label, name_col);
    let label = if t.alive {
        format!("{:.0} / {:.0}", t.hp, t.hp_max)
    } else {
        "Dead".to_string()
    };
    bar(p, frame_bar_rect(panel, 0), t.hp / t.hp_max.max(1.0), HP_COL, &label);
    if let Some(cast) = &t.cast {
        let dur = cast.finish_us.saturating_sub(cast.start_us);
        if dur > 0 {
            let frac = now_us.saturating_sub(cast.start_us) as f32 / dur as f32;
            bar(
                p,
                frame_bar_rect(panel, 1),
                frac,
                CAST_COL,
                ability_name(cast.ability_id),
            );
        }
    }
}

/// Bottom-center: one slot per roster ability (keys 1-4) with cooldown
/// sweep, GCD dimming and a blue tint when mana is short.
fn action_bar(p: &mut Painter, screen: Rect, own: &OwnCombat, now_us: u64) {
    let n = ABILITIES.len() as f32;
    let x0 = screen.center().x - (n * SLOT + (n - 1.0) * GAP) * 0.5;
    let y = screen.max.y - MARGIN - SLOT;
    for (i, def) in ABILITIES.iter().enumerate() {
        let rect = Rect::from_min_size(
            Pos2::new(x0 + i as f32 * (SLOT + GAP), y),
            Vec2::new(SLOT, SLOT),
        );
        p.rect_filled(rect, 4.0, Color::rgba(0.08, 0.08, 0.10, 0.85));
        if own.mana < def.mana_cost {
            overlay(p, rect, 4.0, Color::rgba(0.20, 0.30, 0.70, 0.35));
        }
        p.text(
            Pos2::new(rect.min.x + 4.0, rect.min.y + 3.0),
            &format!("{}", i + 1),
            SMALL_PX,
            DIM_TEXT,
            None,
        );
        let name_size = p.measure_text(def.name, SMALL_PX, None);
        p.text(
            Pos2::new(rect.center().x - name_size.x * 0.5, rect.max.y - name_size.y - 3.0),
            def.name,
            SMALL_PX,
            Color::WHITE,
            None,
        );

        let ready_at = own
            .cooldowns
            .iter()
            .find(|(id, _)| *id == def.id.0)
            .map_or(0, |(_, t)| *t);
        let remaining = secs_until(now_us, ready_at);
        if remaining > 0.0 && def.cooldown_secs > 0.0 {
            overlay(p, rect, 4.0, Color::rgba(0.0, 0.0, 0.0, 0.40));
            cooldown_pie(p, rect, (remaining / def.cooldown_secs).clamp(0.0, 1.0));
            let txt = format!("{:.0}", remaining.ceil());
            let size = p.measure_text(&txt, 16.0, None);
            p.text(rect.center() - size * 0.5, &txt, 16.0, Color::WHITE, None);
        } else if now_us < own.gcd_until_us {
            overlay(p, rect, 4.0, Color::rgba(0.0, 0.0, 0.0, 0.40));
        }
        p.rect_stroke(rect, 4.0, 1.0, Color::rgba(1.0, 1.0, 1.0, 0.25));
    }
}

/// Pie overlay for the remaining cooldown fraction, ending at 12 o'clock so
/// the dark edge unwinds clockwise as time passes. Drawn as a raw mesh —
/// `convex_polygon_filled` can't represent a >half-circle pie.
fn cooldown_pie(p: &mut Painter, rect: Rect, frac: f32) {
    use std::f32::consts::{FRAC_PI_2, TAU};
    let center = rect.center();
    let r = rect.width() * 0.42;
    let col = Color::rgba(0.0, 0.0, 0.0, 0.85);
    let steps = ((frac * 32.0).ceil() as u32).max(1);
    let mut vertices = vec![(center, col)];
    for i in 0..=steps {
        let a = -FRAC_PI_2 - frac * TAU * (1.0 - i as f32 / steps as f32);
        vertices.push((Pos2::new(center.x + a.cos() * r, center.y + a.sin() * r), col));
    }
    let indices = (1..=steps).flat_map(|i| [0, i, i + 1]).collect();
    p.mesh(vertices, indices);
}

/// Above the action bar (same width): own cast progress plus time left.
fn cast_bar(p: &mut Painter, screen: Rect, own: &OwnCombat, now_us: u64) {
    let Some(cast) = &own.active_cast else { return };
    let dur = cast.finish_us.saturating_sub(cast.start_us);
    if dur == 0 {
        return;
    }
    let frac = now_us.saturating_sub(cast.start_us) as f32 / dur as f32;
    let n = ABILITIES.len() as f32;
    let size = Vec2::new(n * SLOT + (n - 1.0) * GAP, 18.0);
    let min = Pos2::new(
        screen.center().x - size.x * 0.5,
        screen.max.y - MARGIN - SLOT - GAP - size.y,
    );
    let rect = Rect::from_min_size(min, size);
    bar(p, rect, frac, CAST_COL, ability_name(cast.ability_id));
    let left = format!("{:.1}s", secs_until(now_us, cast.finish_us));
    let ts = p.measure_text(&left, SMALL_PX, None);
    p.text(
        Pos2::new(rect.max.x - ts.x - 6.0, rect.center().y - ts.y * 0.5),
        &left,
        SMALL_PX,
        Color::WHITE,
        None,
    );
}

/// Centered notice with the respawn countdown while dead.
fn respawn_overlay(p: &mut Painter, screen: Rect, own: &OwnCombat, now_us: u64) {
    let txt = format!(
        "Respawn in {:.0}s",
        secs_until(now_us, own.respawn_at_us).ceil()
    );
    let size = p.measure_text(&txt, 18.0, None);
    let pad = Vec2::new(14.0, 8.0);
    let center = Pos2::new(screen.center().x, screen.min.y + screen.height() * 0.38);
    let panel = Rect::from_center_size(center, size + pad * 2.0);
    p.rect_filled(panel, 4.0, PANEL_BG);
    p.text(panel.min + pad, &txt, 18.0, Color::rgba(1.0, 0.45, 0.45, 1.0), None);
}

fn ability_name(ability_id: u16) -> &'static str {
    ABILITIES
        .iter()
        .find(|d| d.id.0 == ability_id)
        .map_or("Cast", |d| d.name)
}
