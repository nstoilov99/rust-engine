//! The gameplay → animation parameter bridge (Task 41 ticket 08, ADR 0002).
//!
//! Every client derives animation locally from state it already has: the
//! local player's parameters come from prediction (exact controller state),
//! remote proxies' from the replicated motion/combat state — position deltas
//! of the interpolated transform and the latest-value combat row. Nothing
//! animation-related crosses the wire; the protocol is untouched.
//!
//! The derivation itself is pure ([`LocalDeriver`] / [`RemoteDeriver`] — unit
//! tested headlessly); [`AnimBridge::update`] is the thin ECS walk that feeds
//! each character rig's parameter blackboard. Writes land before the machine
//! ticks in `Stage::PreUpdate`, per the runner contract.
//!
//! Offline play (Task 41.6 D5) has no net session: [`CharacterAnimBridgeSystem`]
//! feeds the same [`LocalDeriver`] from `CharacterMovement` instead, for the
//! rig child of every scene-authored character.

use std::collections::HashMap;

use rust_engine::engine::animation::graph::{AnimGraphRunner, AnimGraphRuntime, AnimParams};
use rust_engine::engine::ecs::access::SystemDescriptor;
use rust_engine::engine::ecs::components::Transform;
use rust_engine::engine::ecs::hierarchy::{Children, Parent};
use rust_engine::engine::ecs::resources::Resources;
use rust_engine::engine::ecs::schedule::System;
use rust_engine::engine::ecs::system_names;

use crate::prediction::Prediction;
use crate::replication::{NetLocalPlayer, Replication};
use game_shared::components::{CharacterMovement, NetProxy};

/// Parameter slugs of `graphs/character.animgraph` — the contract between
/// this bridge and the shipped graph. Every entry is derivable from
/// replicated state (ADR 0002); none is local-only.
pub const SPEED_PARAM: &str = "speed";
pub const GROUNDED_PARAM: &str = "grounded";
pub const ALIVE_PARAM: &str = "alive";
pub const DIED_TRIGGER: &str = "died";

/// Marker on the character-mesh child replication spawns under net player
/// entities. The bridge derives parameters for exactly these rigs; scene-
/// authored graph entities (no rig marker) are left to their own writers.
pub struct CharacterRig;

/// One frame's derived parameter values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamWrites {
    pub speed: f32,
    pub grounded: bool,
    pub alive: bool,
    /// Fire the `died` Trigger this frame (alive edge — exactly once per
    /// death; the machine buffers it until the Death transition consumes it).
    pub fire_died: bool,
}

/// Write one frame's values onto a blackboard. Graphs that declare fewer
/// parameters refuse the extra writes harmlessly (the `AnimParams` contract).
pub fn apply(w: &ParamWrites, params: &mut AnimParams) {
    params.set_float(SPEED_PARAM, w.speed);
    params.set_bool(GROUNDED_PARAM, w.grounded);
    params.set_bool(ALIVE_PARAM, w.alive);
    if w.fire_died {
        params.fire_trigger(DIED_TRIGGER);
    }
}

/// Death-edge memory: `true → false` fires once, staying dead does not.
#[derive(Debug, Clone, Copy)]
struct AliveEdge {
    was_alive: bool,
}

impl Default for AliveEdge {
    fn default() -> Self {
        Self { was_alive: true }
    }
}

impl AliveEdge {
    fn step(&mut self, alive: bool) -> bool {
        let fired = self.was_alive && !alive;
        self.was_alive = alive;
        fired
    }
}

/// Local player: exact controller state from prediction — the same values
/// the server simulates with, so local animation is never an approximation.
#[derive(Default)]
pub struct LocalDeriver {
    edge: AliveEdge,
}

impl LocalDeriver {
    pub fn step(&mut self, vel: [f32; 3], grounded: bool, alive: bool) -> ParamWrites {
        ParamWrites {
            speed: (vel[0] * vel[0] + vel[1] * vel[1]).sqrt(),
            grounded,
            alive,
            fire_died: self.edge.step(alive),
        }
    }
}

/// Smoothing time constant for the remote speed estimate — long enough to
/// damp interpolation jitter, short enough that walk→run reads promptly.
const SPEED_SMOOTH_TAU: f32 = 0.1;
/// Nothing legitimate moves faster; caps spikes from irregular frame times.
const MAX_SPEED: f32 = 8.0;
/// A one-frame jump beyond this is a warp (spawn snap, big reconcile), not
/// motion: hold pose instead of flashing a sprint.
const TELEPORT_DIST: f32 = 2.0;
/// |vertical velocity| above this reads as airborne. Between jump-arc speeds
/// (up to `jump_speed` 8 m/s) and flat/gentle-slope walking; steep-ramp
/// sprints may briefly misread — the ADR 0002 approximation trade, accepted.
const AIRBORNE_VZ: f32 = 2.0;

/// Remote proxy: parameters derived from the interpolated transform the
/// replication layer already writes (velocity is not kept client-side, so
/// position deltas are the honest source) plus the latest combat row.
#[derive(Default)]
pub struct RemoteDeriver {
    last_pos: Option<[f32; 3]>,
    speed: f32,
    grounded: Option<bool>,
    edge: AliveEdge,
}

impl RemoteDeriver {
    pub fn step(&mut self, pos: [f32; 3], alive: bool, dt: f32) -> ParamWrites {
        let fire_died = self.edge.step(alive);
        if dt > 1e-4 {
            if let Some(last) = self.last_pos {
                let (dx, dy, dz) = (pos[0] - last[0], pos[1] - last[1], pos[2] - last[2]);
                let horizontal = (dx * dx + dy * dy).sqrt();
                if horizontal > TELEPORT_DIST {
                    self.speed = 0.0;
                    self.grounded = Some(true);
                } else {
                    let raw = (horizontal / dt).min(MAX_SPEED);
                    let alpha = (dt / SPEED_SMOOTH_TAU).min(1.0);
                    self.speed += (raw - self.speed) * alpha;
                    self.grounded = Some((dz / dt).abs() < AIRBORNE_VZ);
                }
            }
            self.last_pos = Some(pos);
        }
        ParamWrites {
            speed: self.speed,
            grounded: self.grounded.unwrap_or(true),
            alive,
            fire_died,
        }
    }
}

/// Where a rig's parameters come from — resolved per frame in a read-only
/// pass so the write pass can borrow runtimes mutably.
enum Source {
    Local,
    Remote { pos: [f32; 3], alive: bool },
}

/// Per-frame parameter derivation for every [`CharacterRig`] in the world.
/// Owned by the net session; ticks right after proxy interpolation so remote
/// deltas measure this frame's rendered motion.
#[derive(Default)]
pub struct AnimBridge {
    local: LocalDeriver,
    remote: HashMap<hecs::Entity, RemoteDeriver>,
}

impl AnimBridge {
    pub fn update(
        &mut self,
        world: &mut hecs::World,
        dt: f32,
        prediction: &Prediction,
        replication: &Replication,
    ) {
        // Read pass: plain data out, no long-lived borrows.
        let mut rigs: Vec<(hecs::Entity, Source)> = Vec::new();
        for (child, (_rig, parent)) in world.query::<(&CharacterRig, &Parent)>().iter() {
            let owner = parent.0;
            if world.get::<&NetLocalPlayer>(owner).is_ok() {
                rigs.push((child, Source::Local));
            } else if let Ok(proxy) = world.get::<&NetProxy>(owner) {
                let Ok(t) = world.get::<&Transform>(owner) else {
                    continue;
                };
                // Coarse (far-tier) proxies carry no combat row: alive.
                let alive = replication
                    .combat_state(proxy.entity_id, proxy.generation)
                    .map_or(true, |c| c.alive);
                rigs.push((
                    child,
                    Source::Remote {
                        pos: [t.position.x, t.position.y, t.position.z],
                        alive,
                    },
                ));
            }
        }

        // Write pass. A rig whose machine has not armed yet has no runtime:
        // the deriver must not step either, so one-shot edges (a proxy that
        // spawns already dead) still fire on the first frame that can hear
        // them, not into the void.
        for (child, source) in rigs {
            let Ok(mut rt) = world.get::<&mut AnimGraphRuntime>(child) else {
                continue;
            };
            let writes = match source {
                Source::Local => match prediction.anim_motion() {
                    Some((vel, grounded)) => self.local.step(vel, grounded, prediction.alive()),
                    None => continue, // not in world yet
                },
                Source::Remote { pos, alive } => {
                    self.remote.entry(child).or_default().step(pos, alive, dt)
                }
            };
            apply(&writes, &mut rt.params);
        }
        self.remote.retain(|e, _| world.contains(*e));
    }
}

/// The offline character's rig: the first child carrying an
/// [`AnimGraphRunner`] and *not* the net [`CharacterRig`] marker (net rigs
/// belong to [`AnimBridge`]). The parent link is the whole contract — no
/// marker component to author.
pub fn offline_rig(world: &hecs::World, children: &Children) -> Option<hecs::Entity> {
    children.iter().copied().find(|&c| {
        world.get::<&AnimGraphRunner>(c).is_ok() && world.get::<&CharacterRig>(c).is_err()
    })
}

/// Task 41.6 D5: per-frame parameter derivation for scene-authored
/// characters (`CharacterMovement` + `Children`), driven by the controller's
/// own velocity/grounded state. `PreUpdate`, after the controller wrote this
/// frame's state and before the anim stack reads the blackboard.
#[derive(Default)]
pub struct CharacterAnimBridgeSystem {
    /// One death-edge memory per rig entity, pruned with the entity.
    derivers: HashMap<hecs::Entity, LocalDeriver>,
    /// Read-pass output (rig, velocity, grounded) — reused across frames.
    scratch: Vec<(hecs::Entity, [f32; 3], bool)>,
}

impl CharacterAnimBridgeSystem {
    pub fn descriptor() -> SystemDescriptor {
        SystemDescriptor::new(system_names::CHARACTER_ANIM_BRIDGE)
            .reads::<CharacterMovement>()
            .reads::<Children>()
            .reads::<AnimGraphRunner>()
            .reads::<CharacterRig>()
            .writes::<AnimGraphRuntime>()
            // Both write `CharacterMovement`; this frame's state must be in.
            .after(system_names::PLAYER_INPUT)
            .after(system_names::CHARACTER_MOVEMENT)
            // Both write `AnimGraphRuntime`; the writes must land first.
            .before(system_names::FOOT_PLACEMENT)
            .before(system_names::ANIM_GRAPH)
    }
}

impl System for CharacterAnimBridgeSystem {
    fn run(&mut self, world: &mut hecs::World, _resources: &mut Resources) {
        self.scratch.clear();
        for (_, (cm, children)) in world.query::<(&CharacterMovement, &Children)>().iter() {
            if let Some(rig) = offline_rig(world, children) {
                self.scratch.push((rig, cm.velocity, cm.grounded));
            }
        }
        // A rig whose machine has not armed yet has no runtime: skip the
        // deriver too, so its edges fire on the first frame that can hear them.
        for &(rig, vel, grounded) in &self.scratch {
            let Ok(mut rt) = world.get::<&mut AnimGraphRuntime>(rig) else {
                continue;
            };
            let writes = self.derivers.entry(rig).or_default().step(vel, grounded, true);
            apply(&writes, &mut rt.params);
        }
        self.derivers.retain(|e, _| world.contains(*e));
    }

    fn name(&self) -> &str {
        system_names::CHARACTER_ANIM_BRIDGE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_engine::engine::animation::graph::{AnimParamType, ParamDecl, ParamValue};

    /// The character graph's blackboard, as the compiled plan declares it.
    fn character_params() -> AnimParams {
        AnimParams::from_decls(&[
            ParamDecl {
                slug: SPEED_PARAM.into(),
                ty: AnimParamType::Float,
                default: ParamValue::Float(0.0),
            },
            ParamDecl {
                slug: GROUNDED_PARAM.into(),
                ty: AnimParamType::Bool,
                default: ParamValue::Bool(true),
            },
            ParamDecl {
                slug: ALIVE_PARAM.into(),
                ty: AnimParamType::Bool,
                default: ParamValue::Bool(true),
            },
            ParamDecl {
                slug: DIED_TRIGGER.into(),
                ty: AnimParamType::Trigger,
                default: ParamValue::Trigger(false),
            },
        ])
    }

    #[test]
    fn shipped_character_graph_compiles_and_declares_the_bridge_contract() {
        use rust_engine::engine::animation::graph::{
            compile_anim_graph, AnimAssetLoader, DiskAnimAssets,
        };
        let loader = DiskAnimAssets {
            content_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../content"),
        };
        let doc = loader
            .load_graph("graphs/character.animgraph")
            .expect("shipped graph loads");
        let plan = compile_anim_graph(&doc).expect("shipped graph compiles");

        let ty_of = |slug: &str| plan.parameters.iter().find(|p| p.slug == slug).map(|p| p.ty);
        assert_eq!(ty_of(SPEED_PARAM), Some(AnimParamType::Float));
        assert_eq!(ty_of(GROUNDED_PARAM), Some(AnimParamType::Bool));
        assert_eq!(ty_of(ALIVE_PARAM), Some(AnimParamType::Bool));
        assert_eq!(ty_of(DIED_TRIGGER), Some(AnimParamType::Trigger));
        assert_eq!(
            plan.parameters.len(),
            4,
            "every parameter is bridge-derived (ADR 0002) — new ones need a derivation"
        );

        let names: Vec<&str> = plan.states.iter().map(|s| s.name.as_str()).collect();
        for state in ["Idle", "Locomotion", "Jump", "Death"] {
            assert!(names.contains(&state), "missing state {state}");
        }
    }

    #[test]
    fn shipped_graph_walks_the_full_ladder_from_derived_writes_only() {
        use rust_engine::engine::animation::graph::{
            compile_anim_graph, AnimAssetLoader, AnimMachine, DiskAnimAssets,
        };
        let loader = DiskAnimAssets {
            content_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../content"),
        };
        let doc = loader.load_graph("graphs/character.animgraph").expect("loads");
        let plan = compile_anim_graph(&doc).expect("compiles");
        let mut machine = AnimMachine::new(&plan);
        let mut params = AnimParams::from_decls(&plan.parameters);
        let mut local = LocalDeriver::default();

        // One second of frames with this controller state (the machine ticks
        // need no clips — state selection is pure).
        let mut drive = |machine: &mut AnimMachine,
                         params: &mut AnimParams,
                         vel: [f32; 3],
                         grounded: bool,
                         alive: bool| {
            for _ in 0..60 {
                apply(&local.step(vel, grounded, alive), params);
                machine.tick(&plan, params, 1.0 / 60.0);
            }
        };
        let name = |m: &AnimMachine| plan.states[m.current_state()].name.as_str();

        machine.tick(&plan, &mut params, 0.0);
        assert_eq!(name(&machine), "Idle", "entry");
        drive(&mut machine, &mut params, [4.0, 0.0, 0.0], true, true);
        assert_eq!(name(&machine), "Locomotion", "walking");
        drive(&mut machine, &mut params, [5.0, 0.0, 3.0], false, true);
        assert_eq!(name(&machine), "Jump", "airborne");
        drive(&mut machine, &mut params, [4.0, 0.0, 0.0], true, true);
        assert_eq!(name(&machine), "Locomotion", "landed moving");
        drive(&mut machine, &mut params, [0.0, 0.0, 0.0], true, true);
        assert_eq!(name(&machine), "Idle", "stopped");
        drive(&mut machine, &mut params, [0.0, 0.0, 0.0], true, false);
        assert_eq!(name(&machine), "Death", "died from anywhere (Any State + Trigger)");
        drive(&mut machine, &mut params, [0.0, 0.0, 0.0], true, false);
        assert_eq!(name(&machine), "Death", "stays dead — the Trigger fired once");
        drive(&mut machine, &mut params, [0.0, 0.0, 0.0], true, true);
        assert_eq!(name(&machine), "Idle", "respawn returns to Idle");
    }

    #[test]
    fn offline_rig_is_the_unmarked_graph_child() {
        use rust_engine::engine::ecs::hierarchy::set_parent;
        let mut world = hecs::World::new();
        let player = world.spawn((CharacterMovement::default(),));
        let net_rig = world.spawn((AnimGraphRunner::new("graphs/x.animgraph"), CharacterRig));
        let plain = world.spawn((Transform::default(),));
        let rig = world.spawn((AnimGraphRunner::new("graphs/x.animgraph"),));
        for c in [net_rig, plain, rig] {
            assert!(set_parent(&mut world, c, player));
        }
        {
            let children = world.get::<&Children>(player).unwrap();
            assert_eq!(
                offline_rig(&world, &children),
                Some(rig),
                "skips the net rig and the child without a runner"
            );
        }

        let lonely = world.spawn((CharacterMovement::default(), Children::new()));
        let none = world.get::<&Children>(lonely).unwrap();
        assert_eq!(offline_rig(&world, &none), None);
    }

    #[test]
    fn local_speed_is_horizontal_velocity() {
        let mut d = LocalDeriver::default();
        let w = d.step([3.0, 4.0, -9.0], true, true);
        assert_eq!(w.speed, 5.0, "vertical velocity is not locomotion speed");
        assert!(w.grounded && w.alive && !w.fire_died);
    }

    #[test]
    fn death_fires_once_and_respawn_rearms() {
        let mut d = LocalDeriver::default();
        assert!(!d.step([0.0; 3], true, true).fire_died);
        assert!(d.step([0.0; 3], true, false).fire_died, "alive edge fires");
        assert!(!d.step([0.0; 3], true, false).fire_died, "staying dead does not");
        assert!(!d.step([0.0; 3], true, true).fire_died, "respawn does not");
        assert!(d.step([0.0; 3], true, false).fire_died, "second death fires again");
    }

    #[test]
    fn remote_speed_converges_on_steady_motion() {
        let mut d = RemoteDeriver::default();
        let dt = 1.0 / 60.0;
        // 4 m/s along +X on flat ground.
        let mut w = d.step([0.0, 0.0, 1.0], true, dt);
        assert_eq!(w.speed, 0.0, "first sample has no delta yet");
        for i in 1..=60 {
            w = d.step([4.0 * dt * i as f32, 0.0, 1.0], true, dt);
        }
        assert!((w.speed - 4.0).abs() < 0.2, "smoothed speed ≈ 4, got {}", w.speed);
        assert!(w.grounded);
    }

    #[test]
    fn remote_vertical_motion_reads_airborne() {
        let mut d = RemoteDeriver::default();
        let dt = 1.0 / 60.0;
        d.step([0.0, 0.0, 0.0], true, dt);
        let w = d.step([0.0, 0.0, 6.0 * dt], true, dt); // rising 6 m/s
        assert!(!w.grounded, "jump arc reads airborne");
        let w = d.step([0.0, 0.0, 6.0 * dt + 0.001], true, dt); // apex-ish
        assert!(w.grounded, "near-zero vertical speed reads grounded");
    }

    #[test]
    fn remote_teleport_holds_pose_instead_of_sprinting() {
        let mut d = RemoteDeriver::default();
        let dt = 1.0 / 60.0;
        d.step([0.0, 0.0, 0.0], true, dt);
        for i in 1..=30 {
            d.step([6.0 * dt * i as f32, 0.0, 0.0], true, dt);
        }
        let w = d.step([500.0, 0.0, 0.0], true, dt); // spawn snap / warp
        assert_eq!(w.speed, 0.0, "warp resets speed");
        assert!(w.grounded);
    }

    #[test]
    fn remote_zero_dt_changes_nothing() {
        let mut d = RemoteDeriver::default();
        let dt = 1.0 / 60.0;
        d.step([0.0; 3], true, dt);
        let before = d.step([1.0 * dt, 0.0, 0.0], true, dt);
        let w = d.step([99.0, 99.0, 99.0], true, 0.0);
        assert_eq!(w.speed, before.speed, "no time, no new estimate");
        assert!(w.speed.is_finite());
    }

    #[test]
    fn remote_death_edge_fires_died() {
        let mut d = RemoteDeriver::default();
        let dt = 1.0 / 60.0;
        assert!(!d.step([0.0; 3], true, dt).fire_died);
        assert!(d.step([0.0; 3], false, dt).fire_died);
        assert!(!d.step([0.0; 3], false, dt).fire_died);
    }

    #[test]
    fn apply_writes_the_declared_blackboard() {
        let mut params = character_params();
        apply(
            &ParamWrites {
                speed: 5.5,
                grounded: false,
                alive: false,
                fire_died: true,
            },
            &mut params,
        );
        assert_eq!(params.get_float(SPEED_PARAM), Some(5.5));
        assert_eq!(params.get_bool(GROUNDED_PARAM), Some(false));
        assert_eq!(params.get_bool(ALIVE_PARAM), Some(false));
        assert_eq!(params.trigger_set(DIED_TRIGGER), Some(true));
    }

    #[test]
    fn apply_is_refused_harmlessly_by_a_foreign_graph() {
        // A graph that declares none of the character parameters (e.g. the
        // scene demo machine) must not gain entries from bridge writes.
        let mut params = AnimParams::from_decls(&[ParamDecl {
            slug: "walk".into(),
            ty: AnimParamType::Bool,
            default: ParamValue::Bool(false),
        }]);
        apply(
            &ParamWrites {
                speed: 5.5,
                grounded: false,
                alive: false,
                fire_died: true,
            },
            &mut params,
        );
        assert_eq!(params.get_float(SPEED_PARAM), None);
        assert_eq!(params.get_bool("walk"), Some(false), "untouched");
    }
}
