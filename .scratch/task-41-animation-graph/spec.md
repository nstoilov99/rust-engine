# Task 41 — Animation Graph: state machine, blend trees, and in-transition rule graphs on the node-graph framework

Status: ready-for-agent

## Problem Statement

Characters in the game don't animate beyond single-clip playback with a basic crossfade. There is no way to author "Idle until moving, blend walk→run by speed, jump when airborne, die from anywhere" without writing engine code. The gameplay side (movement, combat — all shipped through M6–M8) produces plenty of state, but nothing turns that state into poses. In a co-op game this shows immediately: your own character slides around in a T-pose grade single loop, and remote players look worse.

## Solution

An **animation graph** asset (`.animgraph`) — the second consumer of the node-graph framework after script graphs. A graph author builds a state machine (states containing blend trees, transitions carrying boolean rule graphs) in the same graph editor they already know, drives it through a small typed **parameter** blackboard (Float/Bool/Trigger), and assigns it to an entity. Every frame the engine evaluates the graph into a **Pose** and feeds the skinning palette. Gameplay only ever writes parameters; it never touches states.

Transitions are first-class and Unreal-expressive: each transition node carries a **rule graph** — a pure boolean condition network — that lives *inside* the transition, in the same document, with no file on disk. Double-clicking the transition descends into it, exactly like descending into a subgraph, except nothing new is created on disk. At rest the rule collapses to a readable chip on the edge ("Speed > 3.0 · 0.20s").

Animation is entirely client-side: evaluation is an engine module (ADR 0001), and every client derives animation locally from replicated movement/combat state — nothing animation-related crosses the wire (ADR 0002).

## User Stories

1. As a graph author, I want to create a `.animgraph` asset and open it in the graph editor, so that I can author character animation without writing code.
2. As a graph author, I want to place **State** nodes that reference a `.anim` clip, so that a state plays a specific imported animation.
3. As a graph author, I want a state to instead reference a nested `.animgraph`, so that I can factor a big machine (Locomotion) into its own file-backed sub-state-machine.
4. As a graph author, I want an **ENTRY** node that is a real node on the canvas, so that the machine's starting state is visible and rewireable like everything else.
5. As a graph author, I want to draw a **Transition** between two states with a blend duration and a priority, so that the character crossfades between modes deterministically.
6. As a graph author, I want to double-click a transition and get inside it, so that I can edit its boolean rule as a graph — like a subgraph, but stored inside the transition with no separate asset created.
7. As a graph author, I want the rule to open as a peek overlay with the state machine still visible (dimmed, source and target states lit), so that I keep my bearings while editing a condition.
8. As a graph author, I want a promote control (⤢) that escalates the peek overlay into a full editor tab, so that big composite rules get a full canvas.
9. As a graph author, I want Esc to close the peek and return to the machine, so that quick rule edits cost nothing.
10. As a graph author, I want to build rules from parameter reads, comparisons, and logic nodes wired into a single Bool **RESULT** node, so that conditions compose like Unreal transition rules.
11. As a graph author, I want a transition with an unwired Bool input to read as "always true" (hollow socket dot vs. filled), so that duration-only transitions are trivial and honest.
12. As a graph author, I want each transition to display an at-rest **chip** summarizing its rule, duration, and priority ("Speed > 3.0 · 0.20s", "Died ∧ HP ≤ 0 · 0.10s"), with n-condition elision ("⋯ 3 · 0.15s"), so that the topology stays readable without descending.
13. As a graph author, I want an **Any State** node whose outgoing transitions apply from whatever state is active, so that death/hit reactions don't need an edge from every state.
14. As a graph author, I want to declare typed **Parameters** (Float/Bool/Trigger) on the graph, so that gameplay has a stable, typed surface to drive animation.
15. As a graph author, I want a **Trigger** parameter to stay set until a transition consumes it, so that one-shot events (Jump, Died) are never silently lost between frames.
16. As a graph author, I want to build a **blend tree** inside a state — clip nodes, 1D blend (walk→run by Speed), 2D blend (directional movement) — so that a state produces a smoothly blended pose, not just one clip.
17. As a graph author, I want cyclic clips inside one blend node to phase-match (minimal **sync group**), so that walk→run blends don't stutter.
18. As a graph author, I want a **play-once slot** that plays a clip over the base result and then returns, so that attacks and hit reactions overlay locomotion without a dedicated state.
19. As a graph author, I want to place **anim event** markers (notifies) on clip timelines and have them fire engine events when playback crosses them, so that footsteps and hit frames line up with the animation.
20. As a graph author, I want validation to stop me from putting exec, effect, or event-emitting nodes inside a rule graph, so that rules stay pure boolean expressions.
21. As a graph author, I want save/undo/copy-paste to treat a transition and its embedded rule as one unit, so that duplicating or moving a transition never orphans or loses its rule.
22. As a graph author, I want find/navigation (F8, palette, search) to index nodes inside embedded rule graphs, so that renaming a parameter or hunting a node doesn't silently skip rules.
23. As a graph author, I want a preview parameter strip in the editor (Float slider, Bool checkbox, Trigger FIRE button) so that I can drive the machine by hand and watch states/transitions light up.
24. As a graph author, I want to preview the graph's result on a selected entity in the viewport, so that I judge blends on the actual character.
25. As a graph author, I want anchored validation errors and the graph editor's existing organization tools (annotations, auto-layout, alignment) to work in animation graphs, so that the editor feels like one product, not two.
26. As a gameplay programmer, I want an ECS component that references a `.animgraph` and exposes the parameter blackboard, so that systems write Speed/IsGrounded/Jump and nothing else.
27. As a gameplay programmer, I want parameters for remote players to be derived from already-replicated movement/combat state, so that remote characters animate with zero added bandwidth.
28. As a gameplay programmer, I want the graph re-validated and its compiled plan invalidated on save, so that a stale plan never runs against an edited document.
29. As a gameplay programmer, I want simple single-clip playback (existing player component + crossfade) to keep working without a graph, so that trivial cases don't pay the graph tax.
30. As a player, I want my character and my co-op partner's character to move through idle/walk/run/jump/death animations that match what's happening, so that the world reads as alive.
31. As a player on a poor connection, I want remote characters' animation derived from replicated state to degrade gracefully (approximate, never desynced-authoritative), so that animation never fights the server.

## Implementation Decisions

**Asset & document model**

- `.animgraph` is a new asset type built on the shared graph document container from the node-graph framework (Task 40). It brings its own node library and validation profile; no exec wires exist in this domain — the flowing value is **Pose**.
- The state machine is graph-native: ENTRY, State, Any State, and **Transition are all nodes**; transitions carry blend duration and priority as node data.
- A transition's rule graph is an **embedded nested graph region inside the same document**, keyed under the transition's id — a "virtual subgraph": no file on disk, serialized inline, versioned/migrated with the parent document. Copy/paste and duplicate of a transition carry the rule; undo is one history because it is one document.
- States reference either a `.anim` clip (leaf) or a nested `.animgraph` asset (file-backed sub-machine). Double-click always means "descend" — into a file for states, into the embedded rule for transitions — distinguished by breadcrumb ("duck.animgraph ▸ rule: Idle → Locomotion").

**Rule graphs (Unreal-style conditions)**

- Rule scope is restricted by validation to pure nodes: parameter reads, comparisons, math, boolean logic, and exactly one Bool RESULT sink. No effects, no events-as-nodes, no latent nodes.
- An unwired transition Bool input means always-true (rendered as a hollow socket dot; wired = filled).
- Event-like conditions are expressed through **Trigger parameters** with consume-on-transition semantics: a set trigger stays set (buffered) until a transition whose rule reads it actually fires, which consumes it. This is the only stateful element in rule evaluation, and it is owned by the machine, not the rule.

**Runtime & evaluation (per ADR 0001)**

- Evaluation lives in the engine's animation module — not a portable crate. It samples `.anim` clip assets through the existing keyframe sampling functions and writes the skinning palette through the existing skeleton-instance path.
- Per-frame order: gameplay systems write parameters → machine update (evaluate active-state + Any State transition rules in priority order, start/advance crossfades, consume triggers) → blend tree evaluation of the active state(s) into a Pose → play-once slot overlay → palette write.
- Interruption rule v1: a running transition can only be interrupted by an **Any State** transition; ordinary transitions wait.
- Compiled-plan caching follows the script-runner pattern: document compiles to a plan once, cached per asset, invalidated on save. The animation node library gets its own derive macro (the domain-macro pattern established for the framework).
- Anim events fire as engine-level events when clip playback crosses a marker, subject to the active blend weight (no firing from fully blended-out clips).

**Gameplay bridge (per ADR 0002)**

- One ECS component references the `.animgraph` and owns the parameter blackboard. Gameplay writes parameters; it can never set states directly.
- Nothing animation-related is replicated. Local player parameters come from local prediction state; remote-proxy parameters are derived from replicated movement/combat state by a small derivation step. Every new parameter must be derivable from replicated state or explicitly marked local-only — this is a design-time constraint recorded in ADR 0002.
- The existing simple animation player component (single clip + crossfade) remains supported for entities without a graph.

**Editor**

- The animation graph editor is the existing crusty graph editor with the animation node library: same canvas, design tokens, wire router, annotations, anchored errors, palette/find/auto-layout. Pose wires and animation node categories get their own token colors per the design-system spec.
- Rule entry opens as a **peek overlay** (mockup option 3b): in-place editable canvas over the dimmed machine, source/target states stay lit, Esc closes, ⤢ promotes to a full tab (3a) for large rules.
- At-rest transitions render as edge chips (rule summary · duration, priority tag, filled/hollow dot); three-plus conditions elide to "⋯ n".
- The preview parameter strip lives in the editor's footer band (the variables-footer idiom already established): Float sliders, Bool checkboxes, Trigger FIRE buttons; active state and firing transitions highlight live.
- Editor navigation gains a non-file scope target (the embedded rule) on its breadcrumb/tab stack; find and F8 index into embedded rules.

## Testing Decisions

Good tests here exercise **external behavior at the highest existing seam**: feed a document + parameter writes + frame ticks, assert what a player-visible observer could see (active state, blend weights over time, pose values on a synthetic skeleton, events fired, triggers consumed). No asserting on plan internals or node evaluation order.

- **Machine/evaluator seam (the one new seam):** document in → Pose out. Tests build small in-memory documents and synthetic skeletons/clips (a few bones, hand-authored keys) and tick the evaluator on the CPU — no GPU, no asset files. This mirrors the script runtime's acceptance-test style (runner-level tests in the scripting module). Covers: entry state on first tick, rule-driven transitions, crossfade weight curves over the stated duration, priority resolution when multiple rules pass, Any State interruption (and that ordinary transitions cannot interrupt), trigger buffering + consume-on-transition, always-true (unwired) transitions, play-once slot overlay and return, 1D/2D blend weights, sync-group phase matching, anim-event firing at crossings and suppression when blended out.
- **Validation seam (existing):** descriptor/validation tests in the node-graph framework's existing test style — rule-scope purity rejections (effect/exec/event nodes inside rules), single-RESULT enforcement, parameter type mismatches.
- **Document round-trip (existing seam):** serialize/deserialize `.animgraph` with embedded rules; duplicate-transition carries its rule; migration hooks versioned like other graph documents.
- **Clip sampling (existing, already covered):** the keyframe lerp/slerp functions are reused as-is; no new tests beyond what blend-tree tests exercise through them.
- Editor interaction (peek overlay, chips, breadcrumb) is verified visually per the repo's screenshot-review workflow, not unit tests — matching how the rest of the graph editor shipped; extractable pure logic (chip summarization/elision text) gets small unit tests.

## Out of Scope

- **Full layered blending** (independent state machines composited with bone masks — upper-body attack over lower-body run). The play-once slot is v1's only override channel; layers are a follow-up.
- **Root motion**, IK / foot placement, retargeting between skeletons, morph targets.
- **General transition interruption rules** beyond the Any State exception.
- **Blend spaces beyond 1D/2D** and blend-tree exotica (additive nodes may slip in only if the play-once slot needs them; not a goal).
- **Any server or wire involvement**: no replicated parameters, no server-side clip-timing table (ADR 0001 names the escape hatch if hit-frame authority is ever needed — not now).
- **Clip authoring**: timeline/curve editing of `.anim` content, event-marker *editing UI* beyond a minimal list on the clip asset; import stays as-is.
- Editor workflow presets, animation-graph-specific dock layouts.

## Further Notes

- Design sources: the animgraph wireframes mockup (turn 3 is authoritative for transition entry: virtual subgraph, double-click, peek-with-promote), the two ADRs from the 2026-08-14 design session (0001 engine-side evaluation, 0002 client-derived animation), and the repo glossary (CONTEXT.md), whose Task 41 terms — Pose, Clip, State, Transition, Rule graph, Parameter, Trigger, Any State, Anim event, Play-once slot, Sync group — are canonical and used verbatim above.
- One open visual-design item was settled here rather than in the mockup: the preview parameter strip goes in the footer band (variables-footer idiom). Revisit during the design pass if it fights the variables footer for space.
- The roadmap's deferred ledger assigns Task 41 the curve-editor decoupling / growth item from Task 45-A; treat it as an adjacent chore, not a gate on this spec.
- Realm note: animation graphs are Client-realm by definition; the framework's realm validation should reject Server-realm nodes in animation libraries outright.
