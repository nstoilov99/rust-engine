//! Crusty editor design tokens — the single source of color and metrics.
//!
//! Landed from `docs/mockup/theme.rs` (v1.2). Semantic tokens only;
//! components must never hard-code a color that exists here — CI enforces it
//! (`scripts/lint_design.sh`, which excludes exactly this file).
//!
//! Presets swap SURFACES + ACCENTS only. Selection, status, danger, axis and
//! the 12-hue RAMP (assets / pins / categories) are invariant across presets.
//! One declared carve-out (core rule 3): a preset with an achromatic role may
//! override `selection.outline` — today only Graphite (#B8BCC0).
//!
//! v1.1: `type_colors` split into three maps over one shared ramp. The maps
//! store RAMP INDICES, not colors — recoloring the ramp is a one-line change
//! that stays coherent; recoloring one map cannot leak into another.
//!
//! v1.2: `pin_color()` / `wire_color()` resolvers added (the pins map had no
//! resolver — every call site would have re-derived it by hand); the ninth
//! `domain_default` slot removed (it collided with Color/violet — an
//! unregistered Domain now provably renders `neutral`).
//!
//! Engine deltas from the mockup file, all mechanical:
//! - `PinType::Enum` carries no payload in `node_graph::doc`, so the match
//!   arm is `PinType::Enum` (the mockup guessed `Enum(_)`).
//! - `domain_registration` is a [`NodeRegistry`] method, not a free function,
//!   so the domain/pin/wire resolvers take `Option<&NodeRegistry>`. `None`
//!   (registry not reachable from the call site) and "not registered" both
//!   resolve to `neutral` — the same answer either way.
//! - `rgba()` sets the float alpha directly via `Color::with_alpha` rather
//!   than quantizing through a u8 constructor; RGB channels are byte-
//!   identical, alpha is exactly the documented fraction (0.15 not 38/255).
//! - `Accents::on_accent` is an engine-only extra with no mockup counterpart
//!   (text on filled accent buttons); kept, harmless.

use crusty_gui::math::Color;

use crate::engine::node_graph::doc::PinType;
use crate::engine::node_graph::registry::NodeRegistry;

/// sRGB 8-bit RGB → linear crusty [`Color`].
#[inline]
fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_srgb_u8(r, g, b, 255)
}

/// sRGB 8-bit RGB with an explicit float alpha.
#[inline]
fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    rgb(r, g, b).with_alpha(a)
}

/// The one hash used by every ramp fallback: `h = h * 31 + byte`, wrapping,
/// then `% 12`. Matches the design doc and the HTML prototype.
#[inline]
fn hash_index(s: &str) -> u8 {
    let h = s
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    (h % 12) as u8
}

// ---------------------------------------------------------------------------
// Ramp — one ramp, two tones, three maps.
// ---------------------------------------------------------------------------

/// One ramp hue in two tones.
/// `bright` — small marks: pin dots, typed icons, asset tile edges.
/// `deep`   — large bands: node category 2px top edge, GroupBox tint + border,
///            inspector section left bars.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hue {
    pub bright: Color,
    pub deep: Color,
}

/// Built in OKLCH: natural luminance curve (yellows lighter, blues darker),
/// at least 24 deg hue separation, positioned to clear `status`, `mixed` and
/// the preset accents. No gray (reserved for unknown), no pure red (reserved for
/// status.error / status.danger). Preset-invariant.
pub fn ramp() -> [Hue; 12] {
    [
        Hue { bright: rgb(0xFE, 0x87, 0x8D), deep: rgb(0xC6, 0x4D, 0x58) }, // 0  ember   18
        Hue { bright: rgb(0xFF, 0xA1, 0x65), deep: rgb(0xCA, 0x64, 0x0A) }, // 1  amber   52
        Hue { bright: rgb(0xF1, 0xB4, 0x40), deep: rgb(0xB1, 0x7E, 0x05) }, // 2  gold    80
        Hue { bright: rgb(0xCA, 0xC7, 0x49), deep: rgb(0x91, 0x8E, 0x02) }, // 3  lime    108
        Hue { bright: rgb(0x8B, 0xD1, 0x72), deep: rgb(0x50, 0x99, 0x33) }, // 4  green   138
        Hue { bright: rgb(0x2B, 0xCE, 0x9F), deep: rgb(0x00, 0x92, 0x6F) }, // 5  teal    168
        Hue { bright: rgb(0x02, 0xBD, 0xC2), deep: rgb(0x09, 0x84, 0x87) }, // 6  cyan    198
        Hue { bright: rgb(0x00, 0xAB, 0xDD), deep: rgb(0x01, 0x76, 0x9A) }, // 7  azure   228
        Hue { bright: rgb(0x63, 0x94, 0xEE), deep: rgb(0x31, 0x61, 0xBE) }, // 8  blue    262
        Hue { bright: rgb(0x8F, 0x86, 0xEA), deep: rgb(0x62, 0x53, 0xB9) }, // 9  violet  286
        Hue { bright: rgb(0xC3, 0x7C, 0xD3), deep: rgb(0x91, 0x47, 0xA2) }, // 10 magenta 320
        Hue { bright: rgb(0xE5, 0x7C, 0xB4), deep: rgb(0xB0, 0x45, 0x82) }, // 11 rose    348
    ]
}

/// OUTSIDE the ramp on purpose: unambiguously "unknown / unregistered".
/// The hash fallback never produces it.
pub fn neutral() -> Hue {
    Hue { bright: rgb(0x8D, 0x93, 0x99), deep: rgb(0x5E, 0x64, 0x6A) }
}

/// The three maps. Indices into [`ramp()`] — never colors.
pub struct Palettes {
    /// Asset kind -> ramp index (tile edges, typed icons; bright tone).
    pub assets: &'static [(&'static str, u8)],
    /// PinType -> ramp index (pin dots + wires; bright tone). Order:
    /// float, vec, color, bool, enum, texture, mesh, entity, int, string,
    /// array.
    /// Exec is not in the ramp (white @ 92%) and Domain resolves through
    /// [`domain_ramp_index`] — neither belongs in this array. (v1.2: the old
    /// ninth `domain_default` slot was 9/violet, pixel-identical to Color.)
    ///
    /// v1.3 (Task 45-A): the last three slots are appended, never inserted —
    /// the existing indices are the published mapping.
    pub pins: [u8; 11],
    /// Reserved category name -> ramp index (2px node edge, GroupBox tint;
    /// deep tone). Unreserved names: hash % 12. Dev = neutral, by assignment.
    pub categories: &'static [(&'static str, u8)],
}

pub const PALETTES: Palettes = Palettes {
    assets: &[
        ("ui", 0),        // ember   — was #F08C7E; now 34 deg clear of vfx
        ("vfx", 1),       // amber
        ("lights", 2),    // gold    — ~10 deg from status.warning: accepted,
        //           separated by lightness + role
        ("physics", 3),   // lime
        ("audio", 4),     // green
        ("scripting", 5), // teal
        ("materials", 6), // cyan    — moved off violet (see DESIGN.md, palette
        //           architecture: sign-off record)
        ("cameras", 7),    // azure
        ("geometry", 10),  // magenta — moved off gray; gray = unknown, only
        ("animation", 11), // rose
    ],
    //      float vec color bool enum texture mesh entity int string array
    //   Int/3 lime sits one ramp step from Float/4 green: numbers read as a
    //   family, the same "adjacent = related" logic that puts Vec/8 blue next
    //   to Color/9 violet. String/5 teal and Array/6 cyan take the two
    //   remaining cool slots, clear of every scalar-numeric hue so a
    //   collection never reads as a number. Gold/2 stays free.
    pins: [4, 8, 9, 0, 11, 1, 10, 7, 3, 5, 6],
    categories: &[
        ("Event", 0),
        ("Flow", 2),
        ("Math", 3),
        ("Logic", 4),
        ("Data", 5),
        ("Audio", 6),
        ("Physics", 7),
        ("Spatial", 8),
        ("Render", 9),
        ("Gameplay", 10),
        ("Interface", 11),
        // "Dev" -> neutral(), special-cased in category_color(): fixture
        // nodes visually announce themselves as not-a-real-category.
    ],
};

#[inline]
fn asset_hue(kind: &str) -> Hue {
    PALETTES
        .assets
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, i)| ramp()[*i as usize])
        .unwrap_or_else(neutral)
}

/// Mesh pin and geometry asset share magenta ON PURPOSE — same concept; they
/// move together or not at all. Texture/amber and vfx/amber likewise.
/// Float/green vs audio/green is coincidental reuse across independent maps:
/// they never co-occur, and being separate tokens, moving one can't move the
/// other.
///
/// Bright tone: tile edges, typed icons, small marks and label text.
pub fn asset_color(kind: &str) -> Color {
    asset_hue(kind).bright
}

/// Deep tone of the same asset slot — for the *large bands* the design system
/// reserves it for: inspector section left bars, tile wash. Unknown kinds
/// resolve to `neutral.deep`.
pub fn asset_deep_color(kind: &str) -> Color {
    asset_hue(kind).deep
}

#[inline]
fn category_hue(name: &str) -> Hue {
    if name == "Dev" || name == "Debug" {
        return neutral();
    }
    PALETTES
        .categories
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, i)| ramp()[*i as usize])
        .unwrap_or_else(|| ramp()[hash_index(name) as usize])
}

/// Node category 2px top edge / GroupBox tint — the deep tone.
pub fn category_color(name: &str) -> Color {
    category_hue(name).deep
}

/// Category tag text (the 9px mono tag) uses the BRIGHT tone of the same slot
/// so it stays legible at 9px where the deep tone would sink into `elevated`.
pub fn category_tag_color(name: &str) -> Color {
    category_hue(name).bright
}

// ---------------------------------------------------------------------------
// Pin + wire resolvers (v1.2). Call sites never index the maps by hand.
// ---------------------------------------------------------------------------

/// Three cases; prose and code agree on which one is neutral:
///   registered with an explicit key  -> Some(that index)
///   registered without a key         -> Some(hash(slug) % 12)
///   NOT registered                   -> None  (=> neutral: error state)
///
/// `registry: None` means the call site has no registry in reach; it answers
/// the same as "not registered", because that is the only honest answer.
pub fn domain_ramp_index(registry: Option<&NodeRegistry>, slug: &str) -> Option<u8> {
    match registry.and_then(|r| r.domain_registration(slug)) {
        Some(Some(idx)) => Some(idx % 12),
        Some(None) => Some(hash_index(slug)),
        None => None,
    }
}

/// Exec is not a ramp hue: white @ 92%. An unregistered `Domain` resolves to
/// `neutral` — never a guessed color (see DESIGN-nodegraph.md, Pins).
pub fn pin_color(registry: Option<&NodeRegistry>, ty: &PinType) -> Color {
    let b = |i: usize| ramp()[PALETTES.pins[i] as usize].bright;
    match ty {
        PinType::Exec => rgba(0xFF, 0xFF, 0xFF, 0.92),
        PinType::Float => b(0),
        PinType::Vec2 | PinType::Vec3 | PinType::Vec4 => b(1),
        PinType::Color => b(2),
        PinType::Bool => b(3),
        PinType::Enum => b(4),
        PinType::Texture => b(5),
        PinType::Mesh => b(6),
        PinType::Entity => b(7),
        PinType::Int => b(8),
        PinType::String => b(9),
        // An array is its own hue, not a shade of its element type: at pin-dot
        // size a "green-ish" dot next to a green dot is not a distinction, and
        // shape already carries the element type's family in the label.
        PinType::Array(_) => b(10),
        PinType::Domain(k) => match domain_ramp_index(registry, k) {
            Some(i) => ramp()[i as usize].bright,
            None => neutral().bright,
        },
    }
}

/// Wire = its pin color at 80%; exec wires pass through at white @ 92%.
pub fn wire_color(registry: Option<&NodeRegistry>, ty: &PinType) -> Color {
    match ty {
        PinType::Exec => rgba(0xFF, 0xFF, 0xFF, 0.92),
        _ => pin_color(registry, ty).with_alpha(0.80),
    }
}

// ---------------------------------------------------------------------------
// Graph canvas grid (DESIGN-nodegraph ▸ Canvas)
// ---------------------------------------------------------------------------

/// Minor grid pitch, world units.
pub const GRID_MINOR_STEP: f32 = 16.0;
/// Major grid pitch, world units.
pub const GRID_MAJOR_STEP: f32 = 128.0;
/// The minor grid stops being drawn below this zoom (it turns to mush).
pub const GRID_MINOR_MIN_ZOOM: f32 = 0.40;

/// Grid lines are white at a fixed alpha — not a surface token, because the
/// grid must read identically on every preset's canvas. Named here so widget
/// code never spells the value (and never spells a hex).
pub fn grid_minor() -> Color {
    Color::WHITE.with_alpha(0.05)
}

/// Major grid line: same white, 9%.
pub fn grid_major() -> Color {
    Color::WHITE.with_alpha(0.09)
}

// ---------------------------------------------------------------------------
// Surfaces / accents / invariants
// ---------------------------------------------------------------------------

/// Opaque surface stack. Depth = lighter fill + 1px border, never shadows.
#[derive(Clone, Debug)]
pub struct Surfaces {
    /// Editable fields (TextEdit, DragValue, spinbox) — darkest, reads inset.
    pub input: Color,
    /// Main window background, tab strips, status bar.
    pub window: Color,
    /// Panel bodies.
    pub panel: Color,
    /// Section headers, spinbox caps.
    pub header: Color,
    /// Popovers, dialogs, toasts, dropdown lists (E3/E4).
    pub elevated: Color,
    /// Pointer-over fill for rows, menu items, ghost buttons.
    pub hover: Color,
    /// Pressed/active neutral fill (also "on" filter chips).
    pub active: Color,
    /// 1px default border.
    pub stroke: Color,
    /// 1px border for hovered controls + popover/dialog outlines.
    pub stroke_strong: Color,
}

/// Accent tokens — split by JOB. One job per token, never shared.
#[derive(Clone, Debug)]
pub struct Accents {
    /// Active tab underline, toggled tool buttons, live profiler controls,
    /// checked checkbox/toggle, slider + progress fill.
    pub accent_active: Color,
    /// Focused inputs & keyboard nav. 1px border swap, no glow.
    pub focus_ring: Color,
    /// `accent_active` at 14-16% alpha: fill behind toggled tool buttons,
    /// active filter chips, segmented controls.
    pub accent_soft: Color,
    /// Text/icons on top of `accent_active` fills (primary buttons).
    /// Engine-only token; no mockup counterpart.
    pub on_accent: Color,
}

/// Preset-invariant selection tokens (UE-outliner-style calm neutral).
#[derive(Clone, Debug)]
pub struct Selection {
    /// Hierarchy rows, asset tiles, dropdown current value, palette row.
    /// Opaque desaturated blue-gray — identical in every preset.
    pub fill: Color, // #3C4654
    /// Text/icons on top of `fill`.
    pub text: Color, // #FFFFFF
    /// SELECTED-node border on the graph canvas (nodes never take `fill`).
    /// Last-clicked at 100%, rest of the set at 55%, 2px outline offset.
    /// Its own token so selection and focus_ring stay separate JOBS.
    /// Overridable ONLY by a preset declaring an achromatic role (core
    /// rule 3 carve-out): Graphite -> #B8BCC0. fill/text stay pinned.
    ///
    /// **User override 2026-08-02; the mockup value was #A9C0D8.** The calm
    /// blue-gray read as "another shade of UI" against a node's own stroke;
    /// selection has to be unmistakable at a glance. #FFD44D measures 13.6:1
    /// against the canvas and 11.6:1 against a node header. #F0C64A was
    /// considered and rejected: it lands within ΔRGB (10, 6, 5) of
    /// `status.warning` #E6C04F, so a selected node would have read as a
    /// warning. #FFD44D is 25% brighter in luminance and stays distinct.
    pub outline: Color, // #FFD44D
}

/// Preset-invariant status + danger colors.
#[derive(Clone, Debug)]
pub struct Status {
    pub error: Color,   // messages, chips, log tags
    pub warning: Color, //
    pub success: Color, //
    pub info: Color,    //
    /// Destructive ACTIONS (Remove/Delete buttons). Not for messages.
    pub danger: Color,
    pub mixed: Color,      // mixed multi-selection values
    pub overridden: Color, // prefab/asset override marker
}

/// Preset-invariant axis colors for transform fields + gizmos.
#[derive(Clone, Debug)]
pub struct Axis {
    pub x: Color,
    pub y: Color,
    pub z: Color,
}

#[derive(Clone, Debug)]
pub struct TextColors {
    pub primary: Color,
    pub secondary: Color,
    /// ~3.6-3.8:1 on header/elevated: RECORDED decision, disabled text is
    /// allowed below AA (see DESIGN.md contrast table); never for essential
    /// information, never legal on `hover`/`active`.
    pub disabled: Color,
    /// Numeric values, paths, logs.
    pub mono: Color,
}

/// Motion tokens. Transitions animate fill, border and opacity ONLY —
/// never layout. Pressed state snaps (0ms).
#[derive(Clone, Copy, Debug)]
pub struct Motion {
    pub duration_fast_ms: f32, // 80  — hover fill, border swaps, checkbox
    pub duration_base_ms: f32, // 140 — popover open, tab switch, panel collapse
    /// Standard ease-out: cubic-bezier(0, 0, 0.2, 1).
    pub easing: [f32; 4],
}

impl Motion {
    pub fn tokens() -> Self {
        Self {
            duration_fast_ms: 80.0,
            duration_base_ms: 140.0,
            easing: [0.0, 0.0, 0.2, 1.0],
        }
    }
}

impl Default for Motion {
    fn default() -> Self {
        Self::tokens()
    }
}

/// Non-color tokens. Every metric is a BASE value; resolve as
/// `base * ui_scale` — never ship fixed pixels to a widget.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    /// Global scale factor. 1.0 = Comfortable. Density presets:
    /// Compact 0.85 / Comfortable 1.0 / Spacious 1.15. Windows display
    /// scaling (125% / 150%) multiplies on top.
    pub ui_scale: f32,
    pub radius_small: f32,  // 2.0 — checkboxes, chips
    pub radius_widget: f32, // 3.0 — buttons, inputs, rows
    pub radius_panel: f32,  // 6.0 — panels, popovers, dialogs
    pub border: f32,        // 1.0 — the only border width
    /// 2px is reserved: active-tab top edge + asset-tile/node type edge.
    pub edge_accent: f32, // 2.0
    pub row_height: f32,     // 22.0
    pub control_height: f32, // 24.0
    pub spacing: [f32; 7],   // 2 4 6 8 12 16 24
    pub indent: f32,         // 18.0
    /// Alpha for transient surfaces (menus, dropdowns, tooltips) ONLY.
    /// 1.0 disables translucency globally. Simple alpha — no blur.
    pub popover_alpha: f32, // 0.96
    pub scrim_alpha: f32,    // 0.45 — behind modals
}

impl Metrics {
    pub fn tokens() -> Self {
        Self {
            ui_scale: 1.0,
            radius_small: 2.0,
            radius_widget: 3.0,
            radius_panel: 6.0,
            border: 1.0,
            edge_accent: 2.0,
            row_height: 22.0,
            control_height: 24.0,
            spacing: [2.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0],
            indent: 18.0,
            popover_alpha: 0.96,
            scrim_alpha: 0.45,
        }
    }

    /// Resolve a base metric at the current scale.
    #[inline]
    pub fn scaled(&self, base: f32) -> f32 {
        base * self.ui_scale
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::tokens()
    }
}

/// Full color palette: preset surfaces/accents + the invariant groups.
/// (Motion and Metrics hang off `EditorTheme`, matching the mockup's `Theme`.)
#[derive(Clone, Debug)]
pub struct Palette {
    pub surfaces: Surfaces,
    pub accents: Accents,
    pub selection: Selection,
    pub status: Status,
    pub axis: Axis,
    pub text: TextColors,
}

impl Palette {
    /// Steel — default preset. Cool neutral surfaces, steel-blue accent.
    pub fn steel() -> Self {
        Self {
            surfaces: Surfaces {
                input: rgb(14, 14, 17),
                window: rgb(18, 18, 21),
                panel: rgb(24, 24, 28),
                header: rgb(30, 31, 35),
                elevated: rgb(34, 35, 39),
                hover: rgb(48, 49, 54),
                active: rgb(60, 62, 68),
                stroke: rgb(55, 57, 63),
                stroke_strong: rgb(74, 77, 85),
            },
            accents: Accents {
                accent_active: rgb(79, 163, 232),
                focus_ring: rgb(122, 187, 240),
                accent_soft: rgba(79, 163, 232, 0.15),
                on_accent: rgb(14, 21, 32),
            },
            ..Self::invariants()
        }
    }

    /// Tidepool — teal accent, surfaces pulled slightly green.
    pub fn tidepool() -> Self {
        Self {
            surfaces: Surfaces {
                input: rgb(13, 16, 16),
                window: rgb(17, 20, 20),
                panel: rgb(23, 27, 26),
                header: rgb(29, 34, 32),
                elevated: rgb(33, 39, 38),
                hover: rgb(45, 53, 51),
                active: rgb(57, 67, 65),
                stroke: rgb(54, 62, 60),
                stroke_strong: rgb(73, 84, 81),
            },
            accents: Accents {
                accent_active: rgb(63, 193, 176),
                focus_ring: rgb(106, 212, 197),
                accent_soft: rgba(63, 193, 176, 0.15),
                on_accent: rgb(14, 21, 32),
            },
            ..Self::invariants()
        }
    }

    /// Graphite — for COLOR-CRITICAL work (grading, texture review): fully
    /// achromatic chrome so nothing casts onto content. That is its job; it
    /// is Steel-without-hue, not Steel-minus. Because the accent is a bright
    /// gray, accent state must never rely on hue: accent-marked elements
    /// always pair the fill with `accent_soft` or the 2px edge, so position
    /// and brightness carry the state. v1.2: overrides `selection.outline`
    /// to an achromatic #B8BCC0 — the invariant #A9C0D8 (OKLCH C 0.042)
    /// would otherwise be the most saturated element on this preset's
    /// canvas, with selected/focused/accent-marked reading as three shades
    /// of one pale blue-gray.
    pub fn graphite() -> Self {
        Self {
            surfaces: Surfaces {
                input: rgb(15, 15, 15),
                window: rgb(19, 19, 19),
                panel: rgb(25, 25, 25),
                header: rgb(31, 31, 32),
                elevated: rgb(35, 35, 36),
                hover: rgb(47, 48, 50),
                active: rgb(59, 60, 62),
                stroke: rgb(60, 60, 63),
                stroke_strong: rgb(78, 78, 82),
            },
            accents: Accents {
                accent_active: rgb(196, 203, 212),
                focus_ring: rgb(154, 163, 174),
                accent_soft: rgba(196, 203, 212, 0.13),
                on_accent: rgb(14, 21, 32),
            },
            selection: Selection {
                // Achromatic override (rule 3 carve-out): C~=0.005, 8.2:1 on
                // elevated. fill/text stay the invariant values.
                outline: rgb(0xB8, 0xBC, 0xC0),
                ..Self::invariants().selection
            },
            ..Self::invariants()
        }
    }

    /// Rusty — brand DEMO preset: splash, about, marketing shots ONLY.
    /// Not offered in the user-facing preset picker (orange survives in
    /// exactly one place — the brand mark; a shipping orange preset would
    /// repaint every screenshot and bug report). Kept here so brand
    /// surfaces can construct it explicitly.
    pub fn rusty() -> Self {
        Self {
            surfaces: Surfaces {
                input: rgb(16, 14, 12),
                window: rgb(21, 18, 16),
                panel: rgb(27, 24, 21),
                header: rgb(34, 30, 26),
                elevated: rgb(38, 34, 29),
                hover: rgb(53, 48, 42),
                active: rgb(66, 59, 51),
                stroke: rgb(62, 56, 49),
                stroke_strong: rgb(87, 78, 68),
            },
            accents: Accents {
                accent_active: rgb(204, 107, 51),
                focus_ring: rgb(224, 150, 104),
                accent_soft: rgba(204, 107, 51, 0.15),
                on_accent: rgb(14, 21, 32),
            },
            ..Self::invariants()
        }
    }

    /// Preset-invariant token groups, usable from panel code without a live
    /// theme reference (the design system locks these across presets).
    pub fn invariant_axis() -> Axis {
        Self::invariants().axis
    }
    pub fn invariant_status() -> Status {
        Self::invariants().status
    }
    pub fn invariant_selection() -> Selection {
        Self::invariants().selection
    }

    /// Tokens shared by every preset. (Surfaces/accents here are placeholders
    /// immediately overridden by the preset constructors.)
    fn invariants() -> Self {
        Self {
            surfaces: Surfaces {
                input: Color::BLACK,
                window: Color::BLACK,
                panel: Color::BLACK,
                header: Color::BLACK,
                elevated: Color::BLACK,
                hover: Color::BLACK,
                active: Color::BLACK,
                stroke: Color::BLACK,
                stroke_strong: Color::BLACK,
            },
            accents: Accents {
                accent_active: Color::WHITE,
                focus_ring: Color::WHITE,
                accent_soft: Color::WHITE,
                on_accent: Color::BLACK,
            },
            selection: Selection {
                fill: rgb(60, 70, 84),
                text: Color::WHITE,
                outline: rgb(0xFF, 0xD4, 0x4D),
            },
            status: Status {
                error: rgb(226, 91, 84),
                warning: rgb(230, 192, 79),
                success: rgb(95, 200, 117),
                info: rgb(110, 168, 232),
                danger: rgb(179, 56, 47),
                mixed: rgb(171, 130, 255),
                overridden: rgb(255, 167, 38),
            },
            axis: Axis {
                x: rgb(224, 82, 82),
                y: rgb(113, 194, 78),
                z: rgb(74, 158, 232),
            },
            text: TextColors {
                primary: rgb(232, 234, 237),
                secondary: rgb(170, 176, 182),
                disabled: rgb(116, 122, 128),
                mono: rgb(198, 203, 211),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Contrast
// ---------------------------------------------------------------------------

/// Contrast ratio between two colors (WCAG 2.x relative luminance formula).
/// Returns a value >= 1.0; higher = more contrast.
pub fn contrast_ratio(fg: Color, bg: Color) -> f32 {
    let lum_fg = relative_luminance(fg);
    let lum_bg = relative_luminance(bg);
    let (lighter, darker) = if lum_fg > lum_bg {
        (lum_fg, lum_bg)
    } else {
        (lum_bg, lum_fg)
    };
    (lighter + 0.05) / (darker + 0.05)
}

/// Relative luminance from a linear-space [`Color`] (no additional sRGB
/// conversion — `Color` is already linear, which is exactly the stage WCAG's
/// formula operates on).
fn relative_luminance(c: Color) -> f32 {
    0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
}

/// A failing WCAG contrast pair.
#[derive(Debug)]
pub struct ContrastIssue {
    pub fg_name: &'static str,
    pub bg_name: &'static str,
    pub ratio: f32,
    pub required: f32,
}

impl std::fmt::Display for ContrastIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} on {} = {:.1}:1 (need {:.1}:1)",
            self.fg_name, self.bg_name, self.ratio, self.required
        )
    }
}

impl Palette {
    /// Verify WCAG AA compliance for all text/background pairings.
    /// Returns an empty vec if everything passes.
    ///
    /// Rules:
    /// - Body text (text.primary, text.secondary): >= 4.5:1 against
    ///   window/panel/elevated/input surfaces
    /// - UI text (text.disabled): >= 3.0:1 against the same surfaces
    pub fn verify_wcag_aa(&self) -> Vec<ContrastIssue> {
        let mut issues = Vec::new();

        let bg_surfaces: [(&str, Color); 4] = [
            ("window", self.surfaces.window),
            ("panel", self.surfaces.panel),
            ("elevated", self.surfaces.elevated),
            ("input", self.surfaces.input),
        ];

        // Body text pairs — need 4.5:1
        let body_text: [(&str, Color); 2] = [
            ("text.primary", self.text.primary),
            ("text.secondary", self.text.secondary),
        ];
        for (fg_name, fg) in &body_text {
            for (bg_name, bg) in &bg_surfaces {
                let ratio = contrast_ratio(*fg, *bg);
                if ratio < 4.5 {
                    issues.push(ContrastIssue { fg_name, bg_name, ratio, required: 4.5 });
                }
            }
        }

        // UI / large text — need 3.0:1
        let ui_text: [(&str, Color); 1] = [("text.disabled", self.text.disabled)];
        for (fg_name, fg) in &ui_text {
            for (bg_name, bg) in &bg_surfaces {
                let ratio = contrast_ratio(*fg, *bg);
                if ratio < 3.0 {
                    issues.push(ContrastIssue { fg_name, bg_name, ratio, required: 3.0 });
                }
            }
        }

        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::node_graph::registry::NodeRegistry;

    #[test]
    fn all_presets_pass_wcag_aa() {
        for (name, palette) in [
            ("steel", Palette::steel()),
            ("tidepool", Palette::tidepool()),
            ("graphite", Palette::graphite()),
            ("rusty", Palette::rusty()),
        ] {
            let issues = palette.verify_wcag_aa();
            assert!(
                issues.is_empty(),
                "WCAG AA failures in {name}:\n{}",
                issues.iter().map(|i| format!("  {i}")).collect::<Vec<_>>().join("\n")
            );
        }
    }

    #[test]
    fn contrast_ratio_white_on_black() {
        let ratio = contrast_ratio(Color::WHITE, Color::BLACK);
        assert!((ratio - 21.0).abs() < 0.5, "expected ~21:1, got {ratio}");
    }

    /// DESIGN.md ▸ Contrast: the published 28-cell table (4 text tokens × 7
    /// Steel surfaces) plus `selection.text` on `selection.fill`.
    ///
    /// The doc rounds to one decimal; we assert to ±0.05, i.e. "the doc's
    /// printed value is the correct rounding of what the tokens actually
    /// produce". A cell that disagrees by more is a token/doc conflict and
    /// must be resolved in the doc, never by nudging a token.
    #[test]
    fn published_contrast_table_matches_tokens() {
        let p = Palette::steel();
        let surfaces: [(&str, Color); 7] = [
            ("input", p.surfaces.input),
            ("window", p.surfaces.window),
            ("panel", p.surfaces.panel),
            ("header", p.surfaces.header),
            ("elevated", p.surfaces.elevated),
            ("hover", p.surfaces.hover),
            ("active", p.surfaces.active),
        ];
        // Row order matches DESIGN.md; columns match `surfaces` above.
        let rows: [(&str, Color, [f32; 7]); 4] = [
            ("primary", p.text.primary, [16.0, 15.5, 14.7, 13.7, 13.0, 10.8, 8.9]),
            ("mono", p.text.mono, [11.8, 11.5, 10.9, 10.1, 9.6, 8.0, 6.6]),
            ("secondary", p.text.secondary, [8.8, 8.5, 8.1, 7.5, 7.2, 5.9, 4.9]),
            ("disabled", p.text.disabled, [4.4, 4.3, 4.1, 3.8, 3.6, 3.0, 2.5]),
        ];

        let mut bad = Vec::new();
        for (tname, fg, expected) in rows {
            for (i, (sname, bg)) in surfaces.iter().enumerate() {
                let got = contrast_ratio(fg, *bg);
                if (got - expected[i]).abs() > 0.05 {
                    bad.push(format!(
                        "  {tname} on {sname}: doc {:.1}, tokens {got:.3}",
                        expected[i]
                    ));
                }
            }
        }
        let sel = contrast_ratio(p.selection.text, p.selection.fill);
        if (sel - 9.6).abs() > 0.05 {
            bad.push(format!("  selection.text on selection.fill: doc 9.6, tokens {sel:.3}"));
        }
        assert!(bad.is_empty(), "DESIGN.md contrast table disagrees:\n{}", bad.join("\n"));
    }

    /// Ramp bright tones double as text (category tags, NOTE-bar labels);
    /// DESIGN.md records violet-on-elevated 5.06 as the worst case, AA.
    #[test]
    fn ramp_bright_clears_aa_on_elevated() {
        let elevated = Palette::steel().surfaces.elevated;
        for (i, h) in ramp().iter().enumerate() {
            let r = contrast_ratio(h.bright, elevated);
            assert!(r >= 4.5, "ramp[{i}].bright on elevated = {r:.2} (< AA 4.5)");
        }
    }

    #[test]
    fn hash_fallback_never_neutral_and_is_stable() {
        // The hash lands inside the ramp by construction; assert it also
        // never coincides with the neutral hue for a broad name sample.
        for n in 0..2000u32 {
            let name = format!("Category{n}");
            assert_ne!(category_color(&name), neutral().deep);
            assert_ne!(category_tag_color(&name), neutral().bright);
        }
        // h = h*31 + byte: "A" = 65 -> 65 % 12 = 5.
        assert_eq!(hash_index("A"), 5);
        // "AB" = 65*31 + 66 = 2081 -> 2081 % 12 = 5.
        assert_eq!(hash_index("AB"), 5);
    }

    #[test]
    fn reserved_categories_and_dev_neutral() {
        assert_eq!(category_color("Event"), ramp()[0].deep);
        assert_eq!(category_color("Interface"), ramp()[11].deep);
        assert_eq!(category_color("Dev"), neutral().deep);
        assert_eq!(category_color("Debug"), neutral().deep);
        assert_eq!(category_tag_color("Dev"), neutral().bright);
    }

    #[test]
    fn asset_map_and_unknown_kind() {
        assert_eq!(asset_color("geometry"), ramp()[10].bright);
        assert_eq!(asset_deep_color("geometry"), ramp()[10].deep);
        // Mesh pin and geometry asset share magenta, on purpose.
        assert_eq!(asset_color("geometry"), pin_color(None, &PinType::Mesh));
        assert_eq!(asset_color("not-a-kind"), neutral().bright);
    }

    #[test]
    fn domain_resolver_three_cases() {
        let mut reg = NodeRegistry::new();
        reg.register_domain_pin("shader"); // registered, unkeyed
        reg.register_domain_pin_keyed("curve", 7); // registered, keyed

        assert_eq!(domain_ramp_index(Some(&reg), "curve"), Some(7));
        assert_eq!(domain_ramp_index(Some(&reg), "shader"), Some(hash_index("shader")));
        assert_eq!(domain_ramp_index(Some(&reg), "nope"), None);
        // No registry in reach answers the same as "not registered".
        assert_eq!(domain_ramp_index(None, "shader"), None);

        let unreg = PinType::Domain("nope".into());
        assert_eq!(pin_color(Some(&reg), &unreg), neutral().bright);
        assert_eq!(pin_color(Some(&reg), &PinType::Domain("curve".into())), ramp()[7].bright);
    }

    /// The Task 45-A pin types resolve to their own deliberate ramp slots —
    /// appended, so the published mapping of the eight Task 40 types is
    /// untouched — and every one of them is a *distinct* hue.
    #[test]
    fn new_pin_types_take_deliberate_ramp_slots() {
        assert_eq!(PALETTES.pins[..8], [4, 8, 9, 0, 11, 1, 10, 7]);

        // Int sits one ramp step from Float: numbers read as a family.
        assert_eq!(pin_color(None, &PinType::Int), ramp()[3].bright);
        assert_eq!(pin_color(None, &PinType::Float), ramp()[4].bright);
        assert_eq!(pin_color(None, &PinType::String), ramp()[5].bright);
        assert_eq!(pin_color(None, &PinType::Array(Box::new(PinType::Float))), ramp()[6].bright);
        // An array's hue is its own, not its element type's — at pin-dot size
        // a shade of green next to green is not a distinction.
        assert_ne!(
            pin_color(None, &PinType::Array(Box::new(PinType::Float))),
            pin_color(None, &PinType::Float)
        );
        assert_eq!(
            pin_color(None, &PinType::Array(Box::new(PinType::Float))),
            pin_color(None, &PinType::Array(Box::new(PinType::Entity))),
            "nesting does not change the slot"
        );

        // No two pin types collide, and none of them lands on neutral (which
        // means "unregistered", the only unknown-color case).
        let all = [
            PinType::Float,
            PinType::Int,
            PinType::Vec3,
            PinType::Color,
            PinType::Bool,
            PinType::Enum,
            PinType::String,
            PinType::Array(Box::new(PinType::Int)),
            PinType::Texture,
            PinType::Mesh,
            PinType::Entity,
        ];
        for (i, a) in all.iter().enumerate() {
            assert_ne!(pin_color(None, a), neutral().bright, "{a:?} must not read as unknown");
            for b in all.iter().skip(i + 1) {
                assert_ne!(pin_color(None, a), pin_color(None, b), "{a:?} vs {b:?}");
            }
        }

        // Wires follow their pins at 80%, new types included.
        for ty in [PinType::Int, PinType::String, PinType::Array(Box::new(PinType::Int))] {
            assert_eq!(wire_color(None, &ty), pin_color(None, &ty).with_alpha(0.80));
        }
    }

    #[test]
    fn exec_pin_and_wire_alphas() {
        let exec = pin_color(None, &PinType::Exec);
        assert!((exec.a - 0.92).abs() < 1e-6);
        assert!((wire_color(None, &PinType::Exec).a - 0.92).abs() < 1e-6);
        assert!((wire_color(None, &PinType::Float).a - 0.80).abs() < 1e-6);
    }

    #[test]
    fn graphite_overrides_only_selection_outline() {
        let g = Palette::graphite();
        let inv = Palette::invariant_selection();
        assert_eq!(g.selection.fill, inv.fill);
        assert_eq!(g.selection.text, inv.text);
        assert_ne!(g.selection.outline, inv.outline);
        assert_eq!(g.selection.outline, rgb(0xB8, 0xBC, 0xC0));
        // Every other preset keeps the invariant outline.
        for p in [Palette::steel(), Palette::tidepool(), Palette::rusty()] {
            assert_eq!(p.selection.outline, inv.outline);
        }
    }

    #[test]
    fn metric_tokens_are_the_documented_base_set() {
        let m = Metrics::tokens();
        assert_eq!(m.ui_scale, 1.0);
        assert_eq!(
            [m.radius_small, m.radius_widget, m.radius_panel],
            [2.0, 3.0, 6.0]
        );
        assert_eq!([m.border, m.edge_accent], [1.0, 2.0]);
        assert_eq!([m.row_height, m.control_height], [22.0, 24.0]);
        assert_eq!(m.spacing, [2.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0]);
        assert_eq!(m.indent, 18.0);
        assert_eq!([m.popover_alpha, m.scrim_alpha], [0.96, 0.45]);
        assert_eq!(Metrics { ui_scale: 1.5, ..m }.scaled(22.0), 33.0);
    }
}
