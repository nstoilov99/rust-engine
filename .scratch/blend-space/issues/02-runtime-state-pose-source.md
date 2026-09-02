# 02 — A state plays a blend space

**What to build:** A state whose `space` property names a `.blendspace` produces the blended pose at runtime. `PlanTree::Space` (axis parameter names, compiled blend space, one `PlanClip` + rate scale per sample). Compiler precedence: blend-tree region > `graph` > `space` > `clip`; the blend space document is obtained through the same loader-closure pattern used for nested graphs so tests inject it in memory. Refusals anchored to the state: file not found, no samples, sample clip not found, axis parameter not declared as Float. All samples form one sync group under the existing blend-node phase rule; input smoothing (when nonzero) exponentially follows the axis inputs with per-state memory reset on entry. Steady-state evaluation allocates nothing. A small blend-space cache keyed by content path serves compiled spaces; `.blendspace` saves/reloads invalidate the plan cache wholesale (existing path). Gameplay drives it purely through Float parameters (ADR 0002).

**Blocked by:** 01

**Status:** done

- [x] Evaluator-seam test: in-memory animgraph + in-memory blendspace + synthetic clips → pose on a synthetic skeleton follows the input point (exact on a sample, blended between)
- [x] Two cyclic samples stay phase-matched while the input moves
- [x] With input smoothing the pose converges toward the target input over ticks; with 0 it snaps
- [x] Precedence: a state with both `space` and `clip` plays the blend space; with `graph` set the graph wins
- [x] Missing file / empty samples / missing clip / undeclared axis parameter each refuse, anchored with the state's node id, and the message names the state
- [x] Saving a `.blendspace` in the editor invalidates the cached plan so the next tick recompiles
- [x] The demo `locomotion.blendspace` loads and plays on the Human entity when referenced from a state
