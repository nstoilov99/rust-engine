# Task 41.6 — Locomotion Demo (Game Animation Sample–style)

**Status:** plan (2026-09-04)
**Depends on:** Task 41 (anim graph), 41.5 (IK + foot placement), M6 input layer.
**Branch:** `task-41.6-locomotion-demo`, branched from `task-41.5-animation-scale`.

## 1. Goal

A playable third-person character on an offline demo scene, as close to
Unreal's Game Animation Sample default mode as our content allows:

- WASD relative to the camera, Shift to run, Space to jump; mouse orbits
  the camera, scroll zooms; the character turns to face its velocity.
- Idle / Walk / Run from real Mixamo clips through the existing blend node,
  Jump and Death states unchanged.
- Foot IK with foot lock and pelvis drop on a ramp, stairs and uneven blocks.
- Runs in editor Play (F5 with the scene open) and in the standalone game
  (`game.exe --scene scenes/locomotion_demo.scene`).

### Non-goals

Motion matching, stride/orientation warping, turn-in-place, traversal,
strafe locomotion (needs strafe clips), root motion, networking. The net
player keeps `character.animgraph`; porting the real clips there is a
follow-up (the user's working tree has uncommitted edits to that file —
**do not touch it**).

## 2. What exists / gaps

| Need | Have | Gap |
|---|---|---|
| Clips | `Defeated.anim` only (death) | Idle/Walking/Running from Mixamo, same X Bot rig |
| Clip import | `import_model_to_mesh` (FBX → `.mesh` + `.anim`) | "Animation only" setting; **by-name bone remap** (`.anim` bone indices assume the sibling mesh's order) |
| Graph | `character.animgraph`: Idle, Locomotion (blend1d Walk/Run on `speed`), Jump, Death | New `locomotion_demo.animgraph` with real clips, `foot_ik` param, two IK Chain nodes |
| Input | Enhanced Input `gameplay` context: move/look/jump/sprint | Nothing consumes it offline |
| Controller | `PlayerInputSystem`, `CharacterMovementSystem` (force-based, first-person, **registered nowhere**) | Velocity-set rewrite, orient-to-movement, scene serialization |
| Camera | Renderer re-derives from the ECS `Camera` entity every frame in both hosts | No follow/orbit camera |
| Anim params | `anim_bridge::LocalDeriver` (net player only) | Offline bridge from `CharacterMovement` state |
| Foot IK ground query | `FootPlacementSystem` excludes the rig entity's own `RigidBody` | Rig is a **child**; the capsule is on the parent → must exclude the parent's body or rays hit the capsule from inside |
| Terrain | `main.scene` has one collider, on a dynamic body | Purpose-built scene with static colliders |

## 3. Design decisions

**D1 — Rapier dynamic capsule, velocity-set movement.** Not the M6 shared
`motion::step` controller: it collides against cooked `ChunkStore`
chunks, which would need the demo scene cooked and a second collision
world for the foot rays. One world (Rapier) serves movement, jump,
grounding, foot IK and the camera boom. `PhysicsWorld` gains
`linear_velocity(handle)` / `set_linear_velocity(handle, v)` (Z-up game
space, converted like `velocity_to_physics`). Body: Dynamic, all rotations
locked, capsule `half_height 0.5, radius 0.4` (matches `MotionConfig`,
feet at −0.9 from centre), damping 0, `can_sleep: false`.

**D2 — `CharacterMovementSystem` rewrite** (same file, same name; it is
dead code today). Per frame: desired horizontal velocity = camera-relative
input × (walk 1.6 m/s, run 4.5 m/s — tune to the clips, see D6); current
XY velocity accelerates toward it (accel 20 m/s², decel 30 m/s²); Z is left
to physics except jump (`vz = jump_speed 8`). Grounded =
`raycast_filtered(centre, −Z, 0.9 + 0.15, Some(self))`. Step assist: when
grounded, horizontal speed > 0.1 and a short forward ray at knee height
(centre − 0.6) hits within 0.5 m while the same ray at `centre − 0.9 +
step_height 0.35` is clear, add `vz = 3.0` for that frame (capsule radius
0.4 alone rolls over ~0.15 m; stairs use 0.15 m risers so this is a
safety net, not the primary mechanism). Orient-to-movement: when
horizontal speed > 0.2, yaw slerps toward the velocity heading at
720°/s; the yaw is written to `Transform.rotation` (Z axis). Fields on
`CharacterMovement`: replace `move_speed/sprint_multiplier/jump_impulse/
ground_check_dist/movement_mode` with `walk_speed, run_speed, accel,
decel, jump_speed, turn_rate_deg, step_height`; runtime-only
`desired_dir: [f32;2], run: bool, jump_requested, grounded, horizontal_speed`.
`MovementMode::Flying` and `LookController` are deleted (nothing uses
them).

**D3 — `OrbitCamera` component + system.** Lives on the Camera entity
(not parented — the boom is computed, hierarchy adds nothing). Serialized
fields: `target: Option<EntityGuid>` (the player; `None` = first
`CharacterMovement` entity), `distance 3.5`, `min/max_distance 1.5/8`,
`pivot_height 1.4`, `shoulder 0.4`, `sensitivity 0.003`, `pitch_min/max
−60°/+70°`; runtime `yaw, pitch`. `OrbitCameraSystem` (game_client): yaw/
pitch from the `look` action, distance from scroll; pivot = target
position + `(0,0,pivot_height)` + right × shoulder; camera = pivot −
forward(yaw, pitch) × distance, shortened by `raycast_filtered(pivot →
camera, exclude target body)` minus 0.2; writes the Camera entity's
`Transform` (position + rotation looking at the pivot) and marks it
`TransformDirty`. Runs in `Update`, `.after(PHYSICS_STEP)` and
`.before(TRANSFORM_PROPAGATION)` so the viewport follows this frame.
`PlayerInputSystem` reads the camera yaw through the `OrbitCamera` whose
target is the player (fallback: the first `OrbitCamera`).

**D4 — Mouse capture.** Editor Play already confines + hides the cursor
and enables raw mouse (F1 toggles). Standalone gets the same grab at
startup and Escape to release/recapture (winit `CursorGrabMode::Confined`,
fallback `Locked`, then `None`).

**D5 — Offline anim bridge.** `CharacterAnimBridgeSystem` (game_client
`anim_bridge.rs`, next to the net bridge): for each `(CharacterMovement,
RigidBody)` entity, the rig is the first child carrying `AnimGraphRunner`
(no marker component — the parent link is the contract); write
`LocalDeriver::step(vel, grounded, alive = true)` through
`anim_bridge::apply`. Same param slugs (`speed`, `grounded`, `alive`,
`died`). Runs PreUpdate `.after(CHARACTER_MOVEMENT)`,
`.before(FOOT_PLACEMENT)`.

**D6 — Clips and speeds.** The user exports from Mixamo for the **X Bot**
(the `Defeated` character): *Idle*, *Walking*, *Running*, FBX Binary, 30
fps, **In Place** ticked, With Skin (the importer needs the skeleton in the
file). Files go to `content/import/` (git-ignored source) and are imported
with the new `animation_only` setting into `content/anims/Idle.anim` etc.
Mixamo names every clip `mixamo.com`, so states pick by file, not
`clip_name`. Blend1d thresholds and the controller speeds must agree
(walk 1.6 / run 4.5 initial; tune by eye until feet do not slide, then
write the tuned numbers into the scene and the graph).

**D7 — By-name bone remap at arm time.** When a `ClipSet` is armed against
a skeleton whose bone-name table differs from the set's, build
`clip index → skeleton index` by name once per (clip set, skeleton) and
rewrite channel `bone_index` in the armed copy; channels naming bones the
skeleton lacks are dropped with one `eprintln!` per clip set. Identical
tables skip the work (the `Defeated` case).

**D8 — Foot placement excludes the parent's body.** `FootPlacementSystem`
resolves `exclude` as the rig entity's `RigidBody` handle, else its
`Parent`'s. Engine change, covered by a unit test on the closure input.

**D9 — Footfall events by tool, not by hand.** A `#[ignore]` engine test
`author_footfall_events` (run once with `--ignored`) samples each
locomotion clip's `LeftFoot`/`RightFoot` model-space height, finds local
minima, and writes `foot_l_down/foot_l_up` and `foot_r_down/foot_r_up`
markers (up = minimum + 40 % of the stride to the next minimum) through
`write_anim_binary`. Deterministic, re-runnable, and the Anim Events dialog
can still adjust them.

**D10 — New graph, new scene, nothing shared edited.**
`content/graphs/locomotion_demo.animgraph` starts as a copy of
`character.animgraph` with: clip properties → `anims/Idle.anim`,
`anims/Walking.anim`, `anims/Running.anim`; variable `foot_ik: Float =
1.0`; two IK Chain nodes on the machine canvas — `foot_l` (`mixamorig:
LeftUpLeg, mixamorig:LeftLeg, mixamorig:LeftFoot`) and `foot_r`, two-bone,
Weight `foot_ik`, Foot ticked, Pelvis `mixamorig:Hips`, Ankle Offset 0.1.
`content/scenes/locomotion_demo.scene`: 40×40 floor; ramps at 15° and 30°
(rotated cuboids); an 8-step stair (rise 0.15, run 0.3); a 3×3 field of
blocks with tops between 0.05 and 0.2; all `RigidBody(Static)` +
`Collider(Cuboid)`; Player (Transform, RigidBody, Collider capsule,
CharacterMovement, PlayerInput) with child `Character Rig` (Transform
`(0,0,−0.9)`, MeshRenderer `Defeated.mesh` + its two materials,
AnimGraphRunner `graphs/locomotion_demo.animgraph`, Parent by GUID); a
Camera entity with `OrbitCamera`; a DirectionalLight. Standalone gets
`--scene <content-relative path>` (default stays `scenes/main.scene`).

**D11 — Schedule.** PreUpdate: EnhancedInput → `PlayerInputSystem` →
`CharacterMovementSystem` (writes `PhysicsWorld`, `Transform`) →
`CharacterAnimBridgeSystem` → FootPlacement → AnimGraph → PhysicsStep.
Update: `OrbitCameraSystem` → TransformPropagation. New
`system_names::{PLAYER_INPUT, CHARACTER_MOVEMENT, CHARACTER_ANIM_BRIDGE,
ORBIT_CAMERA}`. Both hosts register the four systems; the schedule
validator runs at launch, so every read/write must be declared. Launch
both hosts once before calling any package done (the 41.5 lesson: only a
launch builds the runtime schedule).

## 4. Work packages (one commit each, serial)

| P | Scope | Files |
|---|---|---|
| P0 | `animation_only` import setting (+ dialog checkbox), by-name remap (D7), footfall tool (D9), import the three clips, author events | `assets/mesh_import.rs`, editor import dialog, `animation/graph/library.rs` (or wherever clips are armed), new test |
| P1 | `PhysicsWorld` velocity API; `CharacterMovement`/`PlayerInput` field changes + `OrbitCamera` component; delete `LookController`/`Flying`; scene serialization for all three; `CharacterMovementSystem` + `PlayerInputSystem` rewrite (D2) | `physics/world.rs`, `game_shared/components.rs`, `scene/scene_format.rs` + load/save glue, `game_client/systems/*` |
| P2 | `OrbitCameraSystem` (D3), standalone cursor grab (D4) | `game_client/systems/orbit_camera.rs`, `standalone.rs` |
| P3 | Offline anim bridge (D5), foot-placement parent exclude (D8) | `anim_bridge.rs`, `animation/foot_placement.rs` |
| P4 | Demo graph + scene (D10), `--scene`, register systems in both hosts (D11), speed/threshold tuning | `content/graphs`, `content/scenes`, `app.rs`, `standalone.rs` |
| P5 | Docs (ARCHITECTURE ▸ Offline character & orbit camera; KNOWLEDGE gotchas; ROADMAP close-out; CLAUDE.md run command), review pass, user live verification | docs |

P0 is blocked on the user's FBX export; P1–P3 do not need the clips and
can start immediately.

## 5. Acceptance

- Editor: open `locomotion_demo.scene`, F5: character stands in Idle,
  WASD walks, Shift runs, Space jumps, mouse orbits, scroll zooms, Escape/
  F1 releases the cursor. Character faces its movement direction.
- Standalone: `game.exe --scene scenes/locomotion_demo.scene` behaves
  identically.
- Feet plant on the 15° and 30° ramps, each stair tread and the block
  field with visible pelvis drop; setting `foot_ik` to 0 in the Variables
  panel shows the animated pose floating/clipping for comparison.
- Walk and run show no visible foot sliding on the flat.
- Both hosts launch with a clean schedule validation; engine + game_client
  tests green (except the known machine-only `test_render_thread_ready_
  handshake`).

## 6. Follow-up decided 2026-09-06 (Task 41.7, after this demo)

The animgraph root will become a **constrained pose graph** (Unreal
AnimGraph shape): State Machine nodes as pose sources, Layered Blend Per
Bone (the user wants upper-body layering over locomotion), Overlay, IK
Chain tail nodes, Output Pose; local-space nodes first, model-space IK
tail last, one Output. Implemented as a scope over the same flat document
(regions cannot nest), migrating today's pin-less slot/IK nodes into the
wired tail automatically. Reviewed with Codex (gpt-6-astra) and Opus,
notes in `.scratch/anim-flow/`. This demo authors IK with today's nodes
and is the migration fixture.

## 7. Risks

- **Capsule on stairs.** A dynamic capsule can snag on risers; the step
  assist (D2) covers it, and risers are 0.15 m. If it still snags, lower
  the risers to 0.12 m rather than adding a kinematic controller.
- **Foot sliding.** In-place clips carry no root speed; the walk/run
  numbers are guesses to be tuned against the clips (D6).
- **Mixamo bone order.** Handled by D7; if the remap drops channels the
  eprintln names them.
- **Editor cursor grab** is window-global in Play mode; dock interaction
  requires F1 release (existing behaviour, unchanged).
- **Blend1d Direction axis** stays unused (no strafe clips); the
  `.blendspace` asset is left as is.
