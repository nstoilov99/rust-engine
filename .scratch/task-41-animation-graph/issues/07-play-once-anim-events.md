# 07 — Play-once slot and anim events

**What to build:** Two overlay behaviours on top of the base machine result. The play-once slot plays a clip over the base result and then returns — attacks and hit reactions overlay locomotion without a dedicated state (v1's only override channel; full layered blending with bone masks is out of scope). Anim event markers (notifies) placed on clip timelines fire engine-level events when playback crosses them — footsteps and hit frames line up with the animation — subject to the active blend weight, so fully blended-out clips never fire. Marker authoring is a minimal list on the clip asset; timeline editing UI is out of scope.

**Blocked by:** 01 — Tracer: a two-state machine animates an entity.

**Status:** done

- [x] A play-once request overlays the base Pose and returns to the base result when the clip finishes, verified at the evaluator seam
- [x] Anim events fire exactly once per crossing; a looping clip refires on each cycle's crossing
- [x] No events fire from a clip whose blend weight is fully blended out
- [x] Markers are viewable and editable as a minimal list on the clip asset
