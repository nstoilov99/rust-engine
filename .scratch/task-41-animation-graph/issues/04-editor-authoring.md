# 04 — Editor: author state machines in the graph editor

**What to build:** A graph author creates a `.animgraph` asset from the editor and builds a state machine in the existing crusty graph editor — same canvas, design tokens, wire router, annotations, anchored errors, palette, auto-layout. The animation node library registers ENTRY, State, Any State, and Transition as placeable, wireable nodes; transition data (blend duration, priority) is editable; Pose wires and animation node categories get their own token colors per the design-system spec. At rest, each transition renders as an edge chip summarizing its rule, duration, and priority ("Speed > 3.0 · 0.20s"), with a filled/hollow socket dot for wired/always-true and "⋯ n" elision at three-plus conditions. Anchored validation errors and the organization tools work in animation graphs, so the editor feels like one product.

**Blocked by:** 01 — Tracer (document shape the editor saves must be what the runtime accepts); 02 — Rule graphs (chips summarize embedded rules).

**Status:** ready-for-agent

- [ ] A new `.animgraph` is creatable from the editor, opens in the graph editor with the animation node library, and saves a document the runtime evaluates
- [ ] ENTRY, State, Any State, and Transition nodes are placeable and wireable; duration and priority editable on a transition
- [ ] At-rest chips show rule summary · duration and priority, with filled/hollow always-true dot and "⋯ n" elision; chip summarization/elision text has unit tests
- [ ] Validation errors anchor to the offending nodes; annotations, auto-layout, and alignment work in animation graphs
- [ ] Visual pass per the repo's screenshot-review workflow (Pose wire and category colors match the design-system spec)
