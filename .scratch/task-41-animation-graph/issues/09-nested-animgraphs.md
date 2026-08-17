# 09 — Nested sub-state-machines (file-backed)

**What to build:** A State references a nested `.animgraph` asset instead of a clip, so a big machine (Locomotion) factors into its own file-backed sub-state-machine. The runtime evaluates the sub-machine as that state's Pose source. In the editor, double-click on such a state descends into the referenced file (double-click always means "descend" — into a file for states, into the embedded rule for transitions), with the breadcrumb distinguishing the two.

**Blocked by:** 01 — Tracer (runtime evaluation); 04 — Editor: author state machines (descend/breadcrumb).

**Status:** ready-for-agent

- [ ] A state referencing a nested `.animgraph` evaluates the sub-machine into its Pose, verified at the evaluator seam (entry, transitions, and crossfades inside the sub-machine work)
- [ ] Validation rejects a missing reference and a circular reference (a graph reaching itself through nesting)
- [ ] Editor double-click on the state opens the referenced file; breadcrumb shows the file chain and navigates back
- [ ] Plan invalidation on save covers documents that nest the saved graph (or the wholesale invalidation documented for the plan cache is confirmed to cover it)
