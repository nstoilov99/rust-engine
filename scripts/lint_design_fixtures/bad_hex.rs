// Fixture for scripts/lint_design.sh rule 1. NOT compiled — this directory
// is outside every crate's src/ and outside the lint's scan scope; it exists
// only so `--selftest` can prove the rule fires.
//
// Seeded violation: a raw hex in widget code. The fix is always a token.

pub fn panel_background() -> &'static str {
    "#1E1F23"
}
