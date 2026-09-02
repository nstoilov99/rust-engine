//! The Anim Preview dock panel (per-document layouts ticket 03), and the
//! skinned preview pane it shares with the blend space tab.
//!
//! [`skinned_preview_pane`] is the one drawing of "a host render target,
//! an orbit camera over it, a chip top-left, play/pause and the clock along
//! the bottom, or a centred reason when nothing can draw" — the blend space
//! tab (ticket 08) and this panel both call it, so the two previews cannot
//! drift apart. [`anim_preview_panel`] is the panel body: the focused
//! graph's [`AnimGraphPreview`], already ticked by the host, painted into
//! the dock rect. Logic stays in `anim_graph_preview`; this file only draws.

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::math::{Color, Pos2, Rect, Vec2};
use crusty_gui::paint::{PaintCmd, TextureId};
use crusty_gui::text::FontFamily;
use crusty_gui::widgets::Button;

use super::anim_graph_preview::AnimGraphPreview;
use super::blend_space_preview::stem;
use super::mesh_editor::MeshPreviewState;
use super::mesh_editor_crusty::orbit_controls;

/// The preview pane's bottom control bar (play/pause, clock, hint).
pub const PREVIEW_BAR_H: f32 = 30.0;
/// The render target's own clear colour, so the pane reads as one surface
/// before the first frame lands and around a letterboxed target.
pub const PREVIEW_BG: Color = Color::rgb(0.16, 0.16, 0.18);

/// What the shared pane draws. Borrows are field-disjoint on purpose so a
/// caller can hand over pieces of one preview struct.
pub struct PreviewPaneCtx<'a> {
    /// The host's render target; its `size` is written back from the pane
    /// so the target follows the panel.
    pub gpu: Option<&'a mut MeshPreviewState>,
    /// The target's id in *this window's* crusty registry, once registered.
    pub texture: Option<TextureId>,
    /// Why the pane cannot draw — shown centred instead of the mesh.
    pub status: Option<&'a str>,
    /// Top-left chip text (mesh name, state or input readout).
    pub chip: Option<String>,
    /// A second chip line, e.g. which entity is being mirrored.
    pub chip_sub: Option<String>,
    /// The play/pause toggle; `None` hides the control (the pose is not
    /// this pane's to pause — a mirrored runtime).
    pub playing: Option<&'a mut bool>,
    /// Seconds into the clip, for the clock.
    pub time: f32,
}

/// The 3D preview: the host's render target painted edge to edge, an orbit
/// camera over it, a chip top-left and a control bar along the bottom.
/// Without a drawable target the pane explains why in its centre instead of
/// showing a broken pose. `id` salts the pane's widget ids per document.
pub fn skinned_preview_pane(ui: &mut Ui, rect: Rect, s: f32, id: Id, ctx: PreviewPaneCtx) {
    let PreviewPaneCtx { gpu, texture, status, chip, chip_sub, playing, time } = ctx;
    let st = ui.style();
    let pad = st.spacing.padding;
    ui.painter().rect_filled(rect, 0.0, PREVIEW_BG);
    let bar = Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - PREVIEW_BAR_H * s), rect.max);
    let view = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, bar.min.y));
    let mut gpu = gpu;
    if let Some(g) = gpu.as_deref_mut() {
        g.size = (rect.width().floor().max(1.0) as u32, rect.height().floor().max(1.0) as u32);
    }
    let ready = status.is_none() && gpu.as_deref().is_some_and(|g| !g.mesh_indices.is_empty());
    let centre = rect.center();
    let msg_w = (rect.width() - pad * 4.0).max(40.0);
    // Not drawable: the message alone, like the mesh editor's pane — no
    // chrome suggesting controls that have nothing to act on.
    let Some(tex) = texture.filter(|_| ready) else {
        let (msg, color) = match (status, ready) {
            (Some(why), _) => (why.to_string(), st.palette.text_secondary),
            (None, true) => ("Rendering preview\u{2026}".to_string(), st.palette.text_disabled),
            (None, false) => ("Loading preview\u{2026}".to_string(), st.palette.text_disabled),
        };
        centred_text(ui, centre, &msg, st.fonts.body, color, msg_w);
        return;
    };
    ui.painter().paint_mut().push(PaintCmd::Image {
        rect,
        uv_min: Pos2::new(0.0, 0.0),
        uv_max: Pos2::new(1.0, 1.0),
        tint: Color::WHITE,
        texture: tex,
    });
    if let Some(g) = gpu.as_deref_mut() {
        orbit_controls(ui, view, id.with("orbit"), g);
    }

    // Chip, top-left, bounded by the pane: one or two mono lines.
    if let Some(text) = chip {
        let mut p = ui.painter();
        let line_h = st.fonts.small * 1.5;
        let mut size = p.measure_text_family(&text, st.fonts.small, Some(msg_w), FontFamily::Mono);
        if let Some(sub) = &chip_sub {
            let sw = p.measure_text_family(sub, st.fonts.small, Some(msg_w), FontFamily::Mono);
            size.x = size.x.max(sw.x);
            size.y += line_h;
        }
        let size = size + Vec2::splat(pad * 1.2);
        let r = Rect::from_min_size(rect.min + Vec2::splat(pad), size);
        p.rect_filled(r, st.rounding.small, st.palette.elevated.with_alpha(0.85));
        let at = r.min + Vec2::splat(pad * 0.6);
        p.text_family(at, &text, st.fonts.small, st.palette.text, Some(msg_w), FontFamily::Mono);
        if let Some(sub) = &chip_sub {
            p.text_family(
                Pos2::new(at.x, at.y + line_h),
                sub,
                st.fonts.small,
                st.palette.accent_active,
                Some(msg_w),
                FontFamily::Mono,
            );
        }
    }

    // Control bar: play/pause, the clock, the camera hint.
    ui.painter().rect_filled(bar, 0.0, st.palette.panel.with_alpha(0.85));
    let bh = (bar.height() - 6.0 * s).min(st.metrics.control_height);
    let mut clock_x = bar.min.x + pad;
    let mut bar_used = 0.0;
    if let Some(playing) = playing {
        let brect = Rect::from_min_size(
            Pos2::new(bar.min.x + pad, bar.center().y - bh * 0.5),
            Vec2::new(bh * 1.4, bh),
        );
        let mut toggle = false;
        let glyph = if *playing { "\u{23F8}" } else { "\u{25B6}" };
        ui.run_at(
            brect,
            Direction::LeftToRight,
            id.with("play"),
            UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
            |ui| {
                toggle = Button::new(glyph).exact_size(brect.size()).ghost().show(ui).clicked;
            },
        );
        if toggle {
            *playing = !*playing;
        }
        clock_x = brect.max.x + pad;
        bar_used = brect.width();
    }
    let clock = format!("{time:.2} s");
    let mut p = ui.painter();
    p.text_family(
        Pos2::new(clock_x, bar.center().y - st.fonts.small * 0.62),
        &clock,
        st.fonts.small,
        st.palette.text_secondary,
        None,
        FontFamily::Mono,
    );
    let hint = "Left-drag orbit \u{00B7} Middle-drag pan \u{00B7} Wheel zoom";
    let hw = p.measure_text(hint, st.fonts.small, None).x;
    if bar.width() > bar_used + hw + pad * 6.0 + 80.0 * s {
        p.text(
            Pos2::new(bar.max.x - pad - hw, bar.center().y - st.fonts.small * 0.62),
            hint,
            st.fonts.small,
            st.palette.text_disabled,
            None,
        );
    }
}

/// Text centred on `centre`, wrapped to `max_w`.
pub fn centred_text(ui: &mut Ui, centre: Pos2, text: &str, font: f32, color: Color, max_w: f32) {
    let mut p = ui.painter();
    let size = p.measure_text(text, font, Some(max_w));
    p.text(Pos2::new(centre.x - size.x * 0.5, centre.y - size.y * 0.5), text, font, color, Some(max_w));
}

pub struct AnimPreviewPanelCtx<'a> {
    /// The focused graph's preview, ticked by the host this frame.
    pub preview: &'a mut AnimGraphPreview,
    /// The graph's content-relative key (widget id salt).
    pub key: &'a str,
    /// The preview target's id in this window's crusty registry.
    pub texture: Option<TextureId>,
}

/// The Anim Preview panel body: the pane over the focused graph's machine.
/// The chip names the mesh and the active state (`Idle → Walk` mid-fade);
/// a mirrored entity adds a `LIVE · name` line and hides play/pause, since
/// the pose is the entity's to drive.
pub fn anim_preview_panel(ui: &mut Ui, rect: Rect, ctx: AnimPreviewPanelCtx) {
    let AnimPreviewPanelCtx { preview: pv, key, texture } = ctx;
    pv.shown = true;
    let s = (ui.style().metrics.row_height / 22.0).max(0.1);
    let id = Id::new(("anim_preview_panel", key));
    let chip = pv.mesh.as_deref().map(|m| match pv.state_label() {
        Some(state) => format!("{}  \u{00B7}  {state}", stem(m)),
        None => stem(m),
    });
    let chip_sub = pv.mirror.as_ref().map(|m| {
        let who = if m.name.is_empty() { "(unnamed)" } else { m.name.as_str() };
        format!("LIVE \u{00B7} {who}")
    });
    let mirroring = pv.mirror.is_some();
    let time = pv.time();
    skinned_preview_pane(
        ui,
        rect,
        s,
        id,
        PreviewPaneCtx {
            gpu: pv.gpu.as_mut(),
            texture,
            status: pv.status.as_deref(),
            chip,
            chip_sub,
            playing: (!mirroring).then_some(&mut pv.playing),
            time,
        },
    );
}
