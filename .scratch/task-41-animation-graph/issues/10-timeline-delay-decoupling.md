# 10 — Adjacent chore: Timeline/Delay latent decoupling and curve-editor growth

**What to build:** The roadmap's deferred-ledger item owned by Task 41 (see ROADMAP ▸ Task 45-A deferred ledger). In script graphs, Timeline and Delay are both latents on one activation, so a Timeline cannot run *while* a Delay on the same activation waits. Decouple them Blueprint-style: per-node timelines driven independently of exec flow. Grow the deliberately-basic `.curve` editor only as far as the decoupling work needs; broader curve-editor features should be triaged as their own item. Per the Task 41 spec, this is an adjacent chore, not a gate on any animation-graph ticket.

**Blocked by:** None — can start immediately (independent of tickets 01–09).

**Status:** ready-for-agent

- [ ] A Timeline advances while a Delay on the same activation is waiting; both complete with correct timing
- [ ] Existing latent semantics (resume-edge pulse, suspension rules) hold for the decoupled form; existing script-runner tests stay green
- [ ] Any `.curve` editor changes made are limited to what the decoupling needs, with further growth filed separately
