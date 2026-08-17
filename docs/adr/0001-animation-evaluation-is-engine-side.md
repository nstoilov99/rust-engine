# ADR 0001 — Animation graph evaluation is engine-side, not a portable crate

**Status:** accepted (2026-08-14, Task 41 design session)

## Context

The script-graph runtime (Task 45-A) lives in `crates/node_graph_exec` — a
wasm-clean crate with no engine dependencies, because server-side execution
inside the SpacetimeDB module is a real planned consumer (M6 established the
pattern with `game_shared`). Task 41's animation evaluator could have
followed the same discipline.

## Decision

Animation-graph evaluation lives in an engine module (like
`engine/scripting/` hosts the script runner), not a portable crate. It
samples engine clip assets (`.anim`) and writes the skinning palette
directly.

## Consequences

- No portability tax on pose evaluation: it may freely use engine types
  (SkeletonInstance, the skinning backend, asset caches).
- The server never evaluates animation. The wire protocol carries motion
  and combat state only; every client derives animation locally (see ADR
  0002).
- If server-side hit-frame timing is ever needed, the answer is a narrow
  clip-timing table shared via `game_shared` — not making the pose
  evaluator portable.

## Alternatives considered

A `crates/anim_exec` mirror of `node_graph_exec`. Rejected: animation is a
client-visual concern; the discipline would tax every design decision
(asset access, skeleton types, GPU palette) for a consumer that does not
exist and is not planned.
