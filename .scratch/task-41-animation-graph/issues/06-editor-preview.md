# 06 — Preview: parameter strip and viewport preview

**What to build:** A graph author drives the machine by hand and watches it respond. A preview parameter strip lives in the editor's footer band (the variables-footer idiom): Float sliders, Bool checkboxes, and Trigger FIRE buttons for every declared Parameter. While previewing, the active state and firing transitions highlight live on the canvas. The graph's evaluated result plays on a selected entity in the viewport, so blends are judged on the actual character. If the strip fights the variables footer for space, flag it for the design pass rather than redesigning here (open item noted in the spec).

**Blocked by:** 04 — Editor: author state machines in the graph editor.

**Status:** done

- [x] The footer strip lists all declared Parameters with the right control per type; edits drive the machine immediately
- [x] Trigger FIRE respects buffering semantics — the trigger stays lit until a transition consumes it
- [x] Active state and firing transitions highlight live during preview
- [x] Selecting an entity previews the graph's Pose on it in the viewport
- [x] Visual pass per the repo's screenshot-review workflow

Notes (implementation): the strip is the footer band (mockup 2g) — anatomy
`PREVIEW · entity` chip + Float slider / Bool checkbox / Trigger FIRE per
declared Parameter; binding follows the LIVE chip's ladder (explicit pick
from the chip's upward picker, else the selected entity running this graph).
Preview targets are entities already running the graph via `AnimGraphRunner`
— which `AnimGraphSystem` already poses in the editor viewport — and
net-play rigs whose parameters `anim_bridge` owns are excluded outright
(driving them would fight gameplay every frame). Float slider ranges derive
from what the graph reads (1D blend thresholds + rule compare constants, 25%
headroom, `(0,1)` fallback). Strip edits are runtime-only writes the host
applies after the UI — never document state, never undo entries. The band
did not end up fighting the variables footer (the vars column simply stops
above it), so the spec's open design item stays closed.
