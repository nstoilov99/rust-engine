# 04 — Editor: author state machines in the graph editor

**What to build:** A graph author creates a `.animgraph` asset from the editor and builds a state machine in the existing crusty graph editor — same canvas, design tokens, wire router, annotations, anchored errors, palette, auto-layout. The animation node library registers ENTRY, State, Any State, and Transition as placeable, wireable nodes; transition data (blend duration, priority) is editable; Pose wires and animation node categories get their own token colors per the design-system spec. At rest, each transition renders as an edge chip summarizing its rule, duration, and priority ("Speed > 3.0 · 0.20s"), with a filled/hollow socket dot for wired/always-true and "⋯ n" elision at three-plus conditions. Anchored validation errors and the organization tools work in animation graphs, so the editor feels like one product.

**Blocked by:** 01 — Tracer (document shape the editor saves must be what the runtime accepts); 02 — Rule graphs (chips summarize embedded rules).

**Status:** done

- [x] A new `.animgraph` is creatable from the editor (asset browser folder menu ▸ New Animation Graph, seeded ENTRY→Idle at Client realm), opens in the graph editor with the animation node library (its own `NodeRegistry`, selected by extension), and saves a document the runtime evaluates (save → `AnimGraphPlanCache` invalidation; hot-reload reloads clean open tabs)
- [x] ENTRY, State, Any State, and Transition nodes are placeable and wireable (`anim_flow` registered as a flow-like domain: fan-in + cycles legal; a state→state wire drop auto-inserts a Transition); duration and priority editable on a selected transition's config rows
- [x] At-rest chips show rule summary · duration and priority tag, with filled/hollow always-true dot and "⋯ n" elision; `graph_anim_chip` summarizer/elision has 9 unit tests
- [x] Compiler refusals anchor to the node they name (`DomainError` through the shared `ErrorIndex` — badge, count chip, F8); annotations, auto-layout, and alignment run on the shared, domain-free code paths
- [x] Visual pass per the screenshot workflow (`.scratch/t04_demo_tab.png` + crops): Animation category rose (paired with the animation asset slot), `anim_flow` gold, `anim_pose` rose, `anim_trigger` ember; chips verified filled/hollow/elided on screen
