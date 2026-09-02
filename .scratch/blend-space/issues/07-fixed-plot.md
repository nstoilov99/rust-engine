# 07 — The plot is a fixed frame that fills the panel (no pan/zoom)

**What to build:** Replace the free pan/zoom `Canvas` world box with a fixed plot: the axis ranges map onto the canvas rect minus margins reserved for the axis labels/arrows, stretching to whatever space the panel has (Unreal's blend space). No `CanvasView`, no fit-to-view, no wheel zoom, no Space/middle-drag pan — the plot can never be scrolled away. 1D stays a horizontal strip at a fixed band height centred in the frame. All ticket 05 interactions keep working (select, drag with snap, right-click menu, Ctrl preview point, Delete, Esc) through a rect-based `doc_to_screen` / `screen_to_doc` mapping. The "Hold Ctrl to set the preview point" hint stays.

**Why:** the user reported the grid could be scrolled off into empty space; a blend space is a bounded plot, not a canvas.

**Blocked by:** 05

**Status:** done

- [x] The plot fills the canvas area at every panel size; resizing the tab rescales it; nothing pans or zooms
- [x] Axis labels, arrows and min/max sit in the reserved margins and never overlap the plot
- [x] Drag, snap, add-sample-here, Ctrl preview point and weights still work (existing tests updated to the new mapping)
