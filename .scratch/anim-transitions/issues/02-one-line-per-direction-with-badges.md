# 02 — One line per direction, circular arrow badges per transition

**What to build:** Per the spec: `anim_flow_layout` puts every same-direction transition on the
same straight line and returns a badge centre per transition (row centred at the midpoint, along
the line, offset right-of-travel by radius + gap); `build_geoms` gives an unselected transition a
D×D at-rest rect at that centre (D = `m.row_h`); the paint branch draws the badge as a vector reproduction of `engine/icons/left-arrow-circle.svg` — the icon is a disc with the arrow *cut out*: fill the disc in a mid grey (`stroke_strong`) and draw the arrow (shaft + two head strokes, same proportions as the SVG path) in the canvas background colour so it reads as a cut-out, matching Unreal — rotated so the arrow
points along the flow direction; the socket dot and text are gone from the canvas. Hover a badge →
tooltip with `TransitionChip::text()` (rule · duration · priority) plus "A → B"; select → the
`selection_outline` ring, and the standard card unfolds from the badge as today; errored →
`status.error` stroke. LOD: below the glyph threshold draw the plain disc. Update the
`graph_anim_edge` tests (`parallel_same_direction_transitions_stack_lanes` becomes
"same-direction transitions share one line and stack badges"; bidirectional lanes unchanged) and
add one for badge ordering/offset side. Record the ruling in `docs/mockup/AUDIT.md`'s transition
row (annotated edge → badge row; text moved to tooltip/card). Finish with the CLAUDE.md visual
review loop only if it can run without launching a window while the user is at the machine
(it cannot — do a gpt-5.6-Sol read-only code review against the Unreal reference screenshots in
`D:\T3Code\userdata\attachments\` instead and fix material issues); leave a note in the close-out.

**Why:** Unreal's look the user asked for; the old lanes/chips get unreadable past two transitions.

**Blocked by:** 01

**Status:** done

- [x] `character.animgraph` Idle⇄Locomotion: two parallel lines, one badge each; Jump→Idle's two transitions: one line, two badges side by side
- [x] Badge arrow points along the flow; hover shows the rule text; click selects and unfolds the card
- [x] Layout tests updated + new ordering test; audit ledger row updated
- [x] `cargo test -p rust_engine --features editor` (sanctioned failures only), `cargo check -p game_client --features editor`

**Close-out (2026-09-01).** Layout: `anim_flow_layout(doc, rects, BadgeMetrics { d, gap })` — one
line per (from, to), ±½·`LANE_GAP` only when the opposite direction exists (compressed to the smaller
endpoint's half-extent as before), badge row centred on the midpoint, ascending node id, offset
right-of-travel by `d/2 + gap`; both halves of every transition on a shared line are the same two
collinear segments meeting at the midpoint; new `chip_dir` gives the painter the arrow direction
(partial links point with the one wire they have). Geometry: unselected transition = `row_h`×`row_h`
rect; `ChipGeom { tip, dir }`. Paint: `draw_badge_arrow` — disc `stroke_strong`, ring `m.border`
in `stroke` (`status.error` when errored), arrow in the canvas colour at `m.ring_w`, SVG proportions
(tail −0.4r, tip +0.4r, wings at +0.1r ±0.3r, shaft stops where it meets the head's inner edges),
plain disc below `lod.glyphs()`. Hover → `TransitionChip::tooltip()` ("Idle → Jump" + chip line) in
the node-hover slot; live-highlight ring is circular for badges. Tests: `same_direction_transitions_
share_one_line_and_stack_badges`, `badges_order_by_node_id_on_the_right_of_travel`,
`tooltip_names_both_states`; the compress test now exercises the bidirectional split.
`docs/mockup/AUDIT.md` ▸ Wires gained the superseded-chip row.

*Review fallback.* The editor was not launched (user at the machine). gpt-5.6-Sol read-only review
of the diff against the spec + Unreal screenshot: no material issues; two minors fixed — shaft ran
through the head's notch (now ends at `0.4r − 1.2w`, the icon's x=8.4 at its own stroke), and an
empty explicit title bypassed the "State" fallback. Kept deliberately: an *unwired* endpoint reads
"?" (as the breadcrumb does), "State" is the fallback for a wired-but-untitled node.

*User must eyeball:* `character.animgraph` — Idle⇄Locomotion as two parallel lines with one badge on
each outer side; Jump→Idle's two transitions on one line with two badges side by side; badge arrows
pointing with the flow at every angle; hover text; click → card unfolds centred on the badge slot;
zoom out → plain discs; the live-preview ring around a firing badge.
