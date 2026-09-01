# 04 — Default profiles wired + review

**What to build:** Make the AnimGraph / ScriptGraph default trees use the real panels from 02–03,
tune proportions against the Unreal reference (attachments in `D:\T3Code\userdata\attachments\`),
update `docs/KNOWLEDGE.md` (layout profiles gotchas) and `docs/ARCHITECTURE.md` (editor section),
and run the CLAUDE.md visual review loop if it can run without launching a window while the user is
at the machine; otherwise a gpt-5.6-Sol read-only review against the reference and a note.

**Blocked by:** 02, 03

**Status:** done

- [ ] Fresh layout file: opening Graph — character gives Variables | graph | Preview over Details, Assets + Console below — **user to eyeball** (no editor launch from the agent; tree shape pinned by `graph_defaults_pin_the_spec_trees`)
- [x] Docs updated; review recorded

## Close-out (2026-09-01)

**Defaults (`dock_crusty::default_tree`).** The trees from ticket 01 already used the real panel ids
(`graph_variables` / `anim_preview` / `graph_details`); this ticket only retuned the AnimGraph right
column: inner split 0.72 → 0.70, so Variables 18% | documents 57% | Preview over Details 25%
(was 23%). ScriptGraph stays 18% | 62% | 20% — Details takes the Scene Inspector's width. The
bottom strip (Assets | Console under the documents leaf) uses the Scene profile's 0.75 vertical
ratio in every profile. Against the Unreal ABP editor reference (`…-8cfba8b6-….png`): Unreal puts
the preview *top-left* over My Blueprint with Details right; the spec chose preview top-right over
Details (decided in the spec, kept), and Unreal's columns are ~25% / 55% / 20% — ours 18 / 57 / 25
since our Variables list is narrower than Unreal's My Blueprint and our preview needs the width.
New test `graph_defaults_pin_the_spec_trees` asserts shape, tab order and every ratio for all six
profiles. Note: a saved `editor_layout_crusty.ron` never re-reads `default_tree` — only View ▸
Reset Layout / Reset All Layouts or a fresh file shows the new ratio.

**Docs.** `docs/KNOWLEDGE.md` ▸ Editor Patterns ▸ "Layout profiles (per-document layouts)" (five
gotchas: documents marker, gather rule, `focused_document` vs dock focus, layout file v1 → v2, one
preview target at a time). `docs/ARCHITECTURE.md` ▸ Editor Architecture: "Panel System" rewritten
around the dock tree + profiles (Scene / AnimGraph diagrams), new "Graph side panels" subsection
(Details, Variables, Anim Preview, the shared `skinned_preview_pane`). `CLAUDE.md` ▸ Current
Development Focus: one bullet for this round (transition badges, state aliases replacing Any
State, layout profiles).

**Review.** No editor launch and no OS input (user at the machine) — fallback per the ticket: a
gpt-5.6-Sol read-only review of the defaults, the pin test and the three doc edits, cross-checked
against `app.rs` / `dock_crusty.rs`. Findings and what changed: see "Review findings" below.

**What the user must eyeball live (nothing here was screenshotted).**
1. Move `editor_layout_crusty.ron` aside (or View ▸ Reset All Layouts). Open *Graph — character*:
   Variables (left, narrow) | graph strip | Preview (top-right) over Details (bottom-right), Assets |
   Console under the graph. The right column should be about a quarter of the window, the preview
   roughly square-ish; Variables should fit `Speed Float 0×` rows without clipping.
2. Click *Main Scene*: Hierarchy | viewport | Inspector, Console | Profiler below, exactly the old
   default. Click back to the graph: the AnimGraph layout returns unchanged.
3. Open *Graph — runner_demo* (a script graph): Variables | graph | Details, no Preview, Assets |
   Console below.
4. Open *Blend Space — locomotion* and *Curve — duck_hop*: documents over Assets | Console only
   (their tabs embed their own details/preview).
5. Drag a splitter in the AnimGraph layout, restart, confirm it stuck and the Scene layout didn't
   change; then View ▸ Reset Layout resets only the layout you are in.
6. With the graph focused, click Assets and Console: Details / Variables / Preview keep the graph;
   click Hierarchy in the Scene layout after switching — Delete still targets the entity.
7. Preview panel: mesh in the entry state, orbit / Play-Pause / clock work; toggle `Died` in the
   strip and watch the fade chip. Enter play with a character selected: panel reads `LIVE · <name>`.

**Not done / deferred.**
- No screenshot-based visual comparison (rule: no editor launch while the user is at the machine).
- Unreal's preview-left arrangement and My Blueprint-style graph/function lists — not in the spec.
- Extracting the blend space / curve embedded panels into profile panels (spec: out of scope).

## Review findings

Codex (gpt-5.6-Sol) was out of quota (usage limit until 20:16), so the fallback was an Opus
read-only subagent reviewing the defaults, the pin test and the doc edits against `app.rs` /
`dock_crusty.rs`. Findings and what changed:

1. **Factual error, fixed.** KNOWLEDGE and the pin test's doc comment claimed a saved layout never
   re-reads `default_tree`. False: `swap_profile` builds from `default_tree(to)` whenever `profiles`
   has no entry (every non-Scene profile after a v1 migration, all after Reset All, a stored tree
   that lost its marker). Both now say "a *stored* profile never re-reads it; one without a stored
   tree does on first activation".
2. **Overclaim, fixed.** ARCHITECTURE said ScriptGraph is "AnimGraph without the Preview"; the
   centre split differs (0.75 vs 0.70 - Details 20% vs 25%). Now spelled out as 18 / 62 / 20.
3. **Stale module doc, fixed.** `graph_dock_panels_crusty.rs` said the host resolves the document
   via `active_graph_key`; the call sites pass `focused_graph_key` (the exact distinction KNOWLEDGE
   draws). One-word fix in that file.
4. **Wording, fixed.** "same `GraphEdit` path" became "same `GraphEdit::SetProperty` path" to
   match the panel module's own doc.
5. **Missing gotchas, added to KNOWLEDGE.** (a) `build_anim_preview_cbs` records at most one
   command buffer per frame under the fixed `ANIM_PREVIEW_TAB` key and `break`s - a second Anim
   Preview surface needs per-graph target keys; (b) `profile_of` gates on `is_animation_family`
   (machine + rule graphs) while `anim_preview_body` needs `is_animation`, so a focused rule graph
   shows the Preview panel with "Not an animation graph"; (c) the document strip is the
   most-documents leaf heuristic (`documents_leaf_index`), not a stored id.
6. **Defaults vs spec: no mismatch.** Effective columns AnimGraph 18 / 57.4 / 24.6, ScriptGraph
   18 / 61.5 / 20.5, Scene 20 / 60 / 20; the pin test asserts the same literals.

Not changed on purpose: (5b) is behaviour, not docs - a rule graph cannot be previewed standalone,
so the placeholder is right; the profile still fits (Variables/Details apply to rule graphs).
