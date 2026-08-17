# 01 — Tracer: a two-state machine animates an entity

**What to build:** An entity with an animation graph component visibly crossfades Idle → Walk when gameplay flips a parameter. This is the tracer bullet through every layer: a hand-authored `.animgraph` document (ENTRY node, two States referencing `.anim` clips, one Transition carrying blend duration and priority, a typed Parameter declaration), the ECS component that references the asset and owns the parameter blackboard (gameplay writes parameters, never states — ADR 0002 posture), and the engine-side evaluator (ADR 0001): entry state on first tick, transition on parameter change, crossfade weights over the stated duration, clip sampling into a Pose, skinning-palette write. Compiled plan cached per asset and invalidated on save, following the script-runner pattern. The transition rule for this slice may be always-true or a single hard-coded parameter check — the full rule-graph machinery is ticket 02. No editor work; the document is authored by hand.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] `.animgraph` document round-trips through serialization: states, transition data (duration, priority), parameter declarations
- [ ] Evaluator-seam test: document + parameter writes + frame ticks in → active state and blend weights out; entry state is active on the first tick
- [ ] Crossfade weight curve follows the stated duration; pose values verified on a synthetic skeleton, CPU only (no GPU, no asset files)
- [ ] An entity in a scene visibly switches Idle → Walk when a system writes the parameter
- [ ] Saving an edited document invalidates its cached plan; a stale plan never runs
- [ ] Entities without a graph still animate through the existing single-clip player with crossfade
