# 10 — Adjacent chore: Timeline/Delay latent decoupling and curve-editor growth

**What to build:** The roadmap's deferred-ledger item owned by Task 41 (see ROADMAP ▸ Task 45-A deferred ledger). In script graphs, Timeline and Delay are both latents on one activation, so a Timeline cannot run *while* a Delay on the same activation waits. Decouple them Blueprint-style: per-node timelines driven independently of exec flow. Grow the deliberately-basic `.curve` editor only as far as the decoupling work needs; broader curve-editor features should be triaged as their own item. Per the Task 41 spec, this is an adjacent chore, not a gate on any animation-graph ticket.

**Blocked by:** None — can start immediately (independent of tickets 01–09).

**Status:** done

- [x] A Timeline advances while a Delay on the same activation is waiting; both complete with correct timing — `a_timeline_advances_while_a_delay_in_its_update_chain_waits` (Delay in the Update chain parks only that tick's drive activation) and `play_frees_its_caller_and_a_delay_after_it_waits_alongside` (Play is fire-and-forget; a Delay after it waits alongside the run), both asserting exact interleaved tick-by-tick sequences
- [x] Existing latent semantics (resume-edge pulse, suspension rules) hold for the decoupled form; existing script-runner tests stay green — Timeline no longer touches the latent machinery at all (per-node ticker: `GraphInstance::tickers` + interpreter-spawned drive activations, entered through a synthetic `~ticker` pin); every pre-existing timeline/latent/trace/breakpoint expectation passes unchanged, including the tick-exact Update/Finished/Stop sequences
- [x] Any `.curve` editor changes made are limited to what the decoupling needs, with further growth filed separately — the decoupling needed **zero** curve-editor changes; broader curve-editor growth (the 45-A "deliberately basic" ledger note) remains open and should be triaged as its own item at Task 41 close-out
