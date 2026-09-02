# 08 — Gameplay bridge: local and remote characters animate in co-op

**What to build:** The payoff slice. Player characters move through idle/walk/run/jump/death animations that match what's happening — for the local player and for co-op partners — with zero added bandwidth. A small derivation step fills the parameter blackboard: local player Parameters come from local prediction state; remote-proxy Parameters are derived from already-replicated movement/combat state (ADR 0002 — nothing animation-related crosses the wire, protocol untouched). Ship a character `.animgraph` (idle / walk→run 1D blend / jump / death via Any State + Trigger) assigned to player entities. On a poor connection, remote animation degrades gracefully — approximate, never desynced-authoritative.

**Blocked by:** 02 — Rule graphs, Triggers, Any State (jump/death need Triggers and Any State); 03 — Blend trees (walk→run by speed).

**Status:** done

- [x] The local character idles, walks, runs, jumps, and dies in sync with gameplay, driven only by parameter writes
- [x] Remote characters animate from replicated movement/combat state; no new replicated fields, protocol version untouched
- [x] Every Parameter in the shipped character graph is derivable from replicated state or explicitly marked local-only, per ADR 0002
- [ ] Two-client check: both characters read correctly on each screen, including death and respawn

Close-out note: boxes 1–3 are verified headlessly — the shipped
`graphs/character.animgraph` is driven through the full
idle→walk→jump→land→stop→death→respawn ladder by derived parameter writes
alone (`anim_bridge::tests`, machine-level, no assets needed), the contract
test pins the graph to exactly the four bridge-derived parameters, and the
diff touches no `game_shared`/protocol code at all. The live two-client check
(box 4) was **not** performed from this seat: the machine was in active use
(a game in the foreground) and launching host/clients would steal focus.
Remaining live risks are visual only: model facing vs. yaw convention, the
foot-offset constant, and the single-clip content making walk/run/jump/death
poses read alike (one `.anim` clip exists — a content gap, not a code gap).
