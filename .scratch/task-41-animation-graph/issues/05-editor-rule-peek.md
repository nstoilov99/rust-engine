# 05 — Editor: peek-overlay rule editing and navigation into embedded rules

**What to build:** Double-clicking a Transition descends into its embedded rule graph — like descending into a subgraph, except nothing new is created on disk. The rule opens as a peek overlay: an in-place editable canvas over the dimmed machine, with the source and target states still lit so the author keeps their bearings. Esc closes the peek; the ⤢ promote control escalates it to a full editor tab for large rules. The breadcrumb/tab stack gains a non-file scope target ("duck.animgraph ▸ rule: Idle → Locomotion"). Find, F8, and the palette index nodes inside embedded rule graphs, so renaming a parameter or hunting a node never silently skips rules. Editor copy/paste and undo treat a transition and its rule as one unit.

**Blocked by:** 04 — Editor: author state machines in the graph editor.

**Status:** done

- [x] Double-click on a transition opens the rule as a peek overlay: machine dimmed, source/target states lit, rule canvas fully editable in place
- [x] Esc closes the peek and returns to the machine; ⤢ promotes the rule to a full editor tab
- [x] Breadcrumb shows the non-file rule scope and navigates back out
- [x] F8, find, and the palette surface nodes inside embedded rules
- [x] Copy/paste and duplicate of a transition carry its rule in the editor; undo of either is one history
- [x] Visual pass per the repo's screenshot-review workflow (peek dim/lit treatment matches the mockup's option 3b)
