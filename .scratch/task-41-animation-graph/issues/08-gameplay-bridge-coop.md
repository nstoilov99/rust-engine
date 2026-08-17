# 08 — Gameplay bridge: local and remote characters animate in co-op

**What to build:** The payoff slice. Player characters move through idle/walk/run/jump/death animations that match what's happening — for the local player and for co-op partners — with zero added bandwidth. A small derivation step fills the parameter blackboard: local player Parameters come from local prediction state; remote-proxy Parameters are derived from already-replicated movement/combat state (ADR 0002 — nothing animation-related crosses the wire, protocol untouched). Ship a character `.animgraph` (idle / walk→run 1D blend / jump / death via Any State + Trigger) assigned to player entities. On a poor connection, remote animation degrades gracefully — approximate, never desynced-authoritative.

**Blocked by:** 02 — Rule graphs, Triggers, Any State (jump/death need Triggers and Any State); 03 — Blend trees (walk→run by speed).

**Status:** ready-for-agent

- [ ] The local character idles, walks, runs, jumps, and dies in sync with gameplay, driven only by parameter writes
- [ ] Remote characters animate from replicated movement/combat state; no new replicated fields, protocol version untouched
- [ ] Every Parameter in the shipped character graph is derivable from replicated state or explicitly marked local-only, per ADR 0002
- [ ] Two-client check: both characters read correctly on each screen, including death and respawn
