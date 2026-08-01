// Near-miss fixture for scripts/lint_design.sh: every line here is LEGAL and
// must not trip either rule. NOT compiled — see bad_hex.rs.
//
// Rule 1 near-misses:
//   - issue refs in comments (4 digits, and comment-only lines are skipped)
//   - attribute syntax, which has no hex digits after `#`
//   - HTML entities, where `&` is an excluded left boundary
// Rule 2 near-misses:
//   - widths that merely start with the digit 2 (2.4, 20.0, 24.0)
//   - the reserved edge read from a token instead of typed

// fixes #4021 — a bug number, not a color.
// see also #40213 and #f0 (2 digits) and #ABCDE (5 digits): none are 3/6/8.

#[derive(Clone, Copy, Debug)]
pub struct Widths {
    pub hair: f32,
    pub arrow: f32,
    pub gutter: f32,
}

pub fn widths(edge_accent: f32) -> Widths {
    Widths {
        // The reserved 2px comes from the token, never a literal.
        hair: edge_accent,
        // Legal non-2 widths that the published regex used to flag.
        arrow: 2.4,
        gutter: 20.0,
    }
}

pub fn entity_escape() -> &'static str {
    "it&#x27;s fine"
}

#[cfg(test)]
mod tests {
    #[test]
    fn nothing_to_lint() {
        let stroke_width = 24.0;
        assert_eq!(stroke_width, 24.0);
    }
}
