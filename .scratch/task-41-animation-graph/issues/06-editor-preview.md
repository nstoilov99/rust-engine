# 06 — Preview: parameter strip and viewport preview

**What to build:** A graph author drives the machine by hand and watches it respond. A preview parameter strip lives in the editor's footer band (the variables-footer idiom): Float sliders, Bool checkboxes, and Trigger FIRE buttons for every declared Parameter. While previewing, the active state and firing transitions highlight live on the canvas. The graph's evaluated result plays on a selected entity in the viewport, so blends are judged on the actual character. If the strip fights the variables footer for space, flag it for the design pass rather than redesigning here (open item noted in the spec).

**Blocked by:** 04 — Editor: author state machines in the graph editor.

**Status:** ready-for-agent

- [ ] The footer strip lists all declared Parameters with the right control per type; edits drive the machine immediately
- [ ] Trigger FIRE respects buffering semantics — the trigger stays lit until a transition consumes it
- [ ] Active state and firing transitions highlight live during preview
- [ ] Selecting an entity previews the graph's Pose on it in the viewport
- [ ] Visual pass per the repo's screenshot-review workflow
