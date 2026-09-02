# 02 — Rule graphs, Trigger consumption, and Any State

**What to build:** Transitions become Unreal-expressive. Each Transition carries an embedded rule graph — a pure boolean condition network stored inline in the same document, keyed under the transition's id (virtual subgraph: no file on disk, one undo history, versioned with the parent). The rule node set: parameter reads, comparisons, math, boolean logic, and exactly one Bool RESULT sink. Validation enforces rule purity (no exec, effect, event-emitting, or latent nodes inside a rule; single RESULT; parameter types match) and rejects Server-realm nodes in animation libraries outright. An unwired transition Bool input means always-true. Trigger parameters buffer: a set Trigger stays set across frames until a transition whose rule reads it actually fires, which consumes it. When multiple rules pass, priority resolves deterministically. An Any State node's outgoing transitions apply from whatever state is active, and only Any State transitions may interrupt a running crossfade (interruption rule v1).

**Blocked by:** 01 — Tracer: a two-state machine animates an entity.

**Status:** done

- [x] Rule graphs serialize inline with the parent document; duplicating or copy-pasting a transition carries its rule; deleting a transition never orphans one
- [x] Validation rejects effect/exec/event/latent nodes inside a rule, enforces a single RESULT, rejects parameter type mismatches, and rejects Server-realm nodes
- [x] A transition with an unwired Bool input evaluates as always-true
- [x] A Trigger stays set across frames until consumed by a firing transition, and is consumed exactly once
- [x] Multiple passing rules on one state resolve by priority, deterministically, verified at the evaluator seam
- [x] Any State transitions fire regardless of active state and can interrupt a running transition; ordinary transitions wait
