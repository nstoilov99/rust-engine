# 01 — Alias node type, compile expansion, Any State upgrade

**What to build:** Per the spec, in `engine/src/engine/animation/graph/`: `ANIM_STATE_ALIAS_TYPE_ID`
+ `ALIAS_GLOBAL_PROP` / `ALIAS_STATES_PROP` in `plan.rs`; the `library.rs` descriptor ("State
Alias" — "Stands for the states it lists (or all of them); its transitions apply from each") replacing
the Any State one; `upgrade_any_state`; compile-time expansion with the refusals; remove
`TransitionFrom::AnyState` and the `machine.rs` arm (the comment about Any State interrupts goes with
it — an ordinary transition never fires while fading). Tests: expansion order/skip-target, global vs.
listed, both refusals, upgrade keeps ids/edges and is idempotent; update `acceptance.rs` (the Any
State case becomes a Global alias with the *ordinary* semantics — it no longer interrupts a fade) and
any plan/machine tests that construct `AnyState`. Keep `ANIM_ANY_STATE_TYPE_ID` only as the constant
the upgrade matches on (doc-comment it as legacy).

**Why:** the user's decision; the runtime special case is what they dislike.

**Blocked by:** —

**Status:** done

- [x] `cargo test -p rust_engine --features editor` green apart from the sanctioned failures; editor code compiles (it may still reference the old constants — fix minimally, ticket 02 does the UI)
- [x] `upgrade_any_state` runs inside `compile_anim_graph_with` (on a clone or before compile) and is exported for the editor

**Close-out (2026-09-01).** Runtime side done; no UI. Public API for ticket 02 (all in
`plan.rs`, re-exported from `animation::graph`): `ANIM_STATE_ALIAS_TYPE_ID = "anim_state_alias"`,
`ALIAS_GLOBAL_PROP = "global"`, `ALIAS_STATES_PROP = "states"`, `ANIM_ANY_STATE_TYPE_ID` (legacy,
kept), `pub fn upgrade_any_state(doc: &mut GraphDoc) -> usize` (count rewritten; keeps id/edges,
sets `global: true`, titles untitled nodes "Any State", idempotent). Refusals: `alias '<name>' has no
states`, `alias '<name>' references a missing state (node <id>)` — `<name>` is the title, default
`Alias`. `TransitionFrom` is now the single variant `State(usize)` (kept as an enum so call sites
don't churn). Compile upgrades on a `Cow` inside `compile_doc` (root and nested); `DiskAnimAssets`
and `ArmLoader` also upgrade nested documents after parsing (ruling). Editor: `is_flow_source` and
`transition_shortcut` accept the alias id; `AnimCardKind::Any`, `anim_node_tag`'s `ANY` arm,
`state_title`/`rule_state_name` fallbacks and `anchor_anim_refusal` (no `alias '` arm yet) are
left for ticket 02. Tests: 856 passed / 1 sanctioned failure (`test_render_thread_ready_handshake`).
