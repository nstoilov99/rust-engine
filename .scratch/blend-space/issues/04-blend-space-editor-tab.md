# 04 — Blend space editor tab: document, details column, chrome

**What to build:** `EditorTab::BlendSpace(key)` (`blendspace:<key>` id) mirroring the curve editor end to end: per-path editor state (document, undo/redo stack with saved cursor, dirty flag, save = atomic write + plan-cache invalidation), open from the asset browser and from the animation graph's state descend, Ctrl+S in dock and floating, dirty dot/title, close veto with save/discard/cancel. Panel layout: a details column with **Axes** (1D/2D toggle; per-axis Name, Parameter, Min, Max, Grid divisions), **Samples** (list; per sample a Clip dropdown fed by the asset registry's `.anim` rows, a Clip-name dropdown when the container has several clips, X / Y (Y hidden for 1D), Rate scale, delete; an Add Sample button), **Smoothing** (input smoothing seconds); the canvas area is reserved and draws axes + grid + samples as static dots (interaction is ticket 05). Every edit goes through the stack as one undo step with a verb/object label. A sample whose clip does not resolve shows an inline warning.

**Blocked by:** 01, 03

**Status:** done

- [x] Double-clicking a `.blendspace` opens the tab; the tab reopens from the persisted layout after restart
- [x] Editing any field marks the tab dirty; undo/redo restore the document exactly; Ctrl+S saves and clears dirty; closing dirty prompts save/discard/cancel
- [x] Switching 1D ↔ 2D keeps samples (y preserved but hidden in 1D)
- [x] Clip dropdown lists the project's `.anim` files; choosing one writes the content-relative path; clip-name dropdown appears only for multi-clip containers
- [x] Missing-clip samples show a warning in the list
- [x] Saving invalidates the plan cache so a previewed entity picks up the change
