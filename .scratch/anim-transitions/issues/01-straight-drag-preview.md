# 01 — The transition drag preview is a straight arrow

**What to build:** In the animation-machine level of `graph_editor_crusty.rs`, the in-flight
connection wire (`state.connect_drag` from a state's flow pin / border anchor to the pointer) is
drawn as a straight line from the source state's border to the pointer with the machine arrowhead,
in the machine edge colour, regardless of `wire_prefs.style` — the same rule `stroke_wire` applies
to `direct` wires. Nested regions (blend trees etc.) keep the routed preview. Also hide the
Spline/Manhattan/Subway segmented control in the toolbar while the machine level is shown (it has
no effect there); it reappears after descending into a region.

**Why:** the user dragged a transition and got a Subway elbow in a graph of straight arrows.

**Blocked by:** —

**Status:** done

- [x] Dragging from a state shows a straight arrow to the pointer in all three wire styles
- [x] The toolbar hides the wire-style control at the machine level and shows it inside a region
- [x] `cargo test -p rust_engine --features editor` (sanctioned failures only), `cargo check -p game_client --features editor`

**Close-out (2026-09-01).** Not previously implemented: HEAD `d5cb31d` had the ghost on
`wire_anchor` + the pref route (`direct: false`) and the style switch unconditional (a mid-task
review that reported it "already at HEAD" had read this working copy). `graph_editor_crusty.rs`:
the connect-drag ghost at the machine level (`state.domain.is_animation()`; the rule child runs as
`AnimationRule`) starts at the source state's border facing the pointer
(`graph_anim_edge::border_exit`, new pub helper), is a two-point `direct` + `arrow` `WireGeom` in
the machine edge colour (`wire_color` of the flow pin; the success/error tint over a target pin is
kept) with `draw_arrow_head` at the pointer; nothing is drawn while the pointer is still inside the
source card. `graph_toolbar` skips the Spline/Manhattan/Subway control when
`GraphEditorState::at_machine_level()` (animation document, no rule open); a peek or promoted rule
shows it again. Regression tests: `border_exit_leaves_the_card_toward_the_pointer`
(graph_anim_edge) and `machine_level_is_an_animation_document_with_no_rule_open` (graph_editor).
The two-point-ghost-per-WireStyle check itself is not unit-testable — it lives inside the UI frame
pass — but the branch never consults `wire_prefs`. Tests: lib 849 passed / 1 failed (sanctioned
handshake test), integration suites green; `cargo check -p game_client --features editor` clean.
