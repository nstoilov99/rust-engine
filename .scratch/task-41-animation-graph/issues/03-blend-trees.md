# 03 — Blend trees: 1D/2D blends with sync groups

**What to build:** A State produces a smoothly blended Pose, not just one clip. Inside a state: clip nodes, a 1D blend node (walk → run driven by a Float parameter), and a 2D directional blend node (8-way movement). Cyclic clips feeding one blend node phase-match (minimal sync group) so a walk→run blend doesn't stutter as weights shift. Blend evaluation is recursive tree evaluation producing a weighted Pose — slerp for rotations, lerp for positions — through the existing keyframe sampling functions.

**Blocked by:** 01 — Tracer: a two-state machine animates an entity.

**Status:** done

- [x] 1D blend weights across the full parameter range verified at the evaluator seam (endpoints play pure clips, midpoints blend proportionally)
- [x] 2D directional blend produces the expected weighted poses for cardinal and diagonal inputs
- [x] Sync group keeps cyclic clips phase-aligned as the blend weight changes over time
- [x] A blend-tree state crossfades against a plain clip state correctly (weights account for the transition, poses stay sane on a synthetic skeleton)
