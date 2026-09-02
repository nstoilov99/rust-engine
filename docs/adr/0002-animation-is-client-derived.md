# ADR 0002 — Character animation is client-derived; no animation state on the wire

**Status:** accepted (2026-08-14, Task 41 design session)

## Context

The co-op game replicates server-authoritative movement (position via
prediction, shared speed constants) and combat events (M6–M8). Remote
players' characters need to animate. The wire schema today carries zero
animation data.

## Decision

Every client evaluates animation graphs locally, for its own player and for
remote proxies, driven by the replicated movement/combat state it already
receives. Animation parameters, states, and poses are never replicated.

## Consequences

- Zero animation bandwidth; the protocol version is untouched by Task 41.
- Remote animation is an approximation derived from replicated state — a
  remote player's client may show a slightly different blend than the
  owner's. This is the standard trade in networked games and is accepted.
- The parameter bridge must be derivable from replicated state for remote
  proxies (Speed/IsGrounded from replicated motion; combat triggers from
  replicated combat events) — a constraint on parameter design, recorded
  here so future parameters stay derivable or explicitly local-only.

## Alternatives considered

Replicating the parameter blackboard per player. Rejected: new schema +
bandwidth per player for information clients can compute; revisit only if
gameplay-critical animation divergence is ever observed.
