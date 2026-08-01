// Fixture for scripts/lint_design.sh rule 2. NOT compiled — see bad_hex.rs.
//
// Seeded violation: a 2px border outside the reserved active-tab / type-edge
// allow-list. Borders are 1px (`metrics.border`); 2px is the reserved edge.

pub struct Stroke {
    pub width: f32,
}

pub fn selected_border() -> Stroke {
    Stroke { width: 2.0 }
}
