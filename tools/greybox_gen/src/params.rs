//! Generator parameters. Baked into the tool: changing anything here (or the
//! sampling code) is a generator version bump — regenerate and re-check-in.

/// Bump when generated output changes for identical params.
pub const GENERATOR_VERSION: u32 = 1;

/// Fixed world seed. Not a CLI knob: the checked-in world is THE world.
pub const SEED: u32 = 0x00C0FFEE;

/// Cell edge length in meters (matches `game_shared::world_grid::CHUNK_SIZE`).
pub const CELL_SIZE: f32 = 64.0;

/// Terrain sample spacing in meters (integer global grid, 2 m).
pub const SPACING: f32 = 2.0;

/// Samples per cell edge (64 m / 2 m); vertices per edge is +1 (33).
pub const CELL_QUADS: i32 = 32;

/// Cell coordinate range: `(MIN_CELL..MAX_CELL)` on both axes, origin-centered.
pub const MIN_CELL: i32 = -4;
pub const MAX_CELL: i32 = 4;

/// Axis-aligned flat region: terrain height is forced to `height` inside the
/// rect (exactly flat), blending linearly back to sampled terrain over
/// `margin` meters OUTWARD from the rect edge. Skirt slope is roughly
/// `|height − terrain| / margin` on top of the terrain's own gradient — keep
/// heights near the terrain mean (8) and rects ≥ 2·margin apart so skirts
/// stay walkable (< 45°) and never overlap.
#[derive(Clone, Copy, Debug)]
pub struct Plateau {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub height: f32,
    pub margin: f32,
}

/// Plateaus (world Z-up meters, dyadic values). Rects plus margins must stay
/// disjoint.
pub const PLATEAUS: &[Plateau] = &[
    // Default spawn area, centered on the origin: covers the crossing of both
    // zone borders (x=0, y=0).
    Plateau {
        min: [-32.0, -32.0],
        max: [32.0, 32.0],
        height: 8.0,
        margin: 24.0,
    },
    // Traversal gym: spans cell border x=128, the zone border y=0, and cell
    // border y=64 (seam-route stations sit on those lines).
    Plateau {
        min: [80.0, -24.0],
        max: [208.0, 104.0],
        height: 9.0,
        margin: 24.0,
    },
    // M4/M5 route pad crossing the x=0 zone border in the north.
    Plateau {
        min: [-24.0, 120.0],
        max: [24.0, 184.0],
        height: 8.0,
        margin: 24.0,
    },
    // Route pad crossing the y=0 zone border in the west.
    Plateau {
        min: [-184.0, -24.0],
        max: [-120.0, 24.0],
        height: 7.0,
        margin: 24.0,
    },
];

/// Provisional character-controller constants the gym is built around
/// (written into the world manifest; M6 reads them instead of re-inventing).
pub const SLOPE_LIMIT_DEG: f32 = 45.0;
pub const STEP_HEIGHT: f32 = 0.4;
pub const JUMP_GAP: f32 = 3.0;
