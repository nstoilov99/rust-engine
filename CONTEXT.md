# Glossary

Domain language for the engine. Terms are canonical — code, docs, and
conversation use these words with exactly these meanings.

## Graph framework (Task 40 / 45-A)

- **Graph document**: the domain-agnostic node/edge/variable container
  (`GraphDoc`). Domains (scripting, animation) bring node libraries and
  runtimes; the document format is shared.
- **Script graph** (`.graph`): a graph executed by the interpreter — event
  entry points, exec flow, effects.
- **Subgraph** (`.subgraph`): a reusable graph with a declared interface,
  inlined into hosts at compile time.
- **Realm**: where a node/graph may execute (Editor / Client / Server /
  Shared). Authority violations are caught at edit time.
- **Effect**: the only way a running graph touches the world — an emitted,
  applied-by-the-runner action. Graphs never mutate directly.
- **Activation**: one execution thread of a graph instance, spawned by an
  event, with its own frames/locals; suspends across frames when latent.

## Animation (Task 41)

- **Animation graph** (`.animgraph`): a graph asset evaluated per frame to
  produce a Pose for a skeleton. No exec wires; distinct asset type sharing
  the graph framework.
- **Pose**: an array of local bone transforms (SQT per joint) for one
  skeleton — the value flowing through animation wires; finalized into the
  skinning palette.
- **Clip**: an imported animation (`.anim` asset) — per-bone keyframe
  channels with a duration. Source formats glTF/FBX.
- **State**: a discrete character mode (Idle, Locomotion, Jump) inside an
  animation graph's state machine; contains a blend tree.
- **Transition**: a node between two states carrying a blend duration and
  priority, whose condition is a boolean **rule graph** — pure logic nodes
  wired into the transition's Bool input (Unreal-expressive; on-canvas).
- **Blend tree**: the pose-producing node network inside a state (clip
  nodes, 1D/2D blends) evaluated recursively into one Pose.
- **Parameter**: a named, typed value (Float/Bool/Trigger) the animation
  graph reads and gameplay writes — the only bridge between gameplay and
  animation. Gameplay never touches states directly.
- **Trigger**: a one-shot parameter that stays set until a transition
  consumes it (consume-on-transition — buffered, never silently lost).
- **Any State**: a special source node whose outgoing transitions apply
  from whatever state is active (death/hit from anywhere); the only
  transitions allowed to interrupt a running transition in v1.
- **Anim event** (notify): a marker at a time in a clip that fires an event
  when playback crosses it (footsteps, hit frames).
- **Play-once slot**: the single override channel that plays a clip over
  the base result (attack over locomotion), then returns.
- **Sync group** (minimal form): normalized-phase matching between clips in
  one blend node so cyclic clips blend without stutter.
