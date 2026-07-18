//! M5 Package 3: cache-diff replication of net entities into ECS proxies.
//!
//! The snapshot diff is the ONLY spawn/despawn authority (plan D3); row
//! callbacks upstream just mark dirty. Identity and lifecycle semantics are
//! normative in `docs/roadmap/M5-WORLD-IDENTITY-CONTRACT.md` (§2–3).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::interp::InterpBuffer;
use game_shared::components::NetProxy;
use game_shared::net::protocol::{ClientInput, EntityKind, EntityState, WorldSnapshot};
use game_shared::net::schema::{
    derive_proxy_guid, PLAYER_SPEED_MPS, REALM_ID, SPRINT_MULTIPLIER, TOMBSTONE_TTL_SECS,
};
use hecs::World;
use nalgebra_glm as glm;
use rust_engine::engine::ecs::components::{EntityGuid, MeshRenderer, Name, Transform};
use rust_engine::engine::rendering::rendering_3d::mesh::{PRIMITIVE_CUBE, PRIMITIVE_SPHERE};

/// Marker on the local player entity bound to the own player row
/// (contract §2.1: the own row is bound, never proxied).
pub struct NetLocalPlayer;

/// `(entity_id, generation) → hecs::Entity` for everything replication owns
/// (proxies plus the local-player binding).
#[derive(Default)]
pub struct NetIndex {
    map: HashMap<(u64, u32), hecs::Entity>,
}

impl NetIndex {
    pub fn get(&self, entity_id: u64, generation: u32) -> Option<hecs::Entity> {
        self.map.get(&(entity_id, generation)).copied()
    }

    pub fn keys(&self) -> impl Iterator<Item = (u64, u32)> + '_ {
        self.map.keys().copied()
    }
}

/// Client-side tombstone evidence (contract §3.2): fed by `TombstoneSeen`
/// events, pruned on the server GC horizon. Kept separately from the row
/// cache so destruction + GC arriving in one pump cannot erase evidence.
#[derive(Default)]
pub struct TombstoneEvidence {
    map: HashMap<u64, (u32, Instant)>,
}

impl TombstoneEvidence {
    pub fn record(&mut self, entity_id: u64, generation: u32, now: Instant) {
        let entry = self.map.entry(entity_id).or_insert((generation, now));
        // Repeated deaths keep the highest generation, latest sighting.
        entry.0 = entry.0.max(generation);
        entry.1 = now;
    }

    pub fn prune(&mut self, now: Instant) {
        let ttl = Duration::from_secs(TOMBSTONE_TTL_SECS);
        self.map.retain(|_, (_, seen)| now.duration_since(*seen) < ttl);
    }

    /// Contract §3.2: evidence with `generation >= g` marks incarnation `g`
    /// (and everything before it) as destroyed; older evidence does not
    /// classify a newer incarnation.
    pub fn destroyed(&self, entity_id: u64, generation: u32) -> bool {
        self.map.get(&entity_id).is_some_and(|(g, _)| *g >= generation)
    }
}

/// One decision produced by the snapshot diff. Pure data so the diff is
/// unit-testable without an ECS world.
#[derive(Debug, Clone, PartialEq)]
pub enum DiffOp {
    Spawn(EntityState),
    /// Raw transform write (interpolation lands in Package 4).
    Update(EntityState),
    Despawn {
        entity_id: u64,
        generation: u32,
        destroyed: bool,
    },
    /// Live row whose generation differs from the proxy's: despawn without
    /// effects + fresh spawn (contract §3.2).
    Replace {
        old_generation: u32,
        state: EntityState,
    },
}

/// Diff the snapshot against the live proxy set. `own_entity_id` rows and
/// index entries are excluded entirely — the local player is bound, not
/// proxied (contract §2.1), and handled by the caller.
pub fn diff_snapshot(
    live: &[(u64, u32)],
    snapshot: &WorldSnapshot,
    evidence: &TombstoneEvidence,
) -> Vec<DiffOp> {
    let own = snapshot.own_entity_id;
    let mut ops = Vec::new();

    let mut rows: HashMap<u64, &EntityState> = HashMap::new();
    for s in &snapshot.entities {
        if Some(s.entity_id) != own {
            rows.insert(s.entity_id, s);
        }
    }

    for &(entity_id, generation) in live {
        if Some(entity_id) == own {
            continue;
        }
        match rows.remove(&entity_id) {
            Some(s) if s.generation == generation => ops.push(DiffOp::Update(*s)),
            Some(s) => ops.push(DiffOp::Replace {
                old_generation: generation,
                state: *s,
            }),
            None => ops.push(DiffOp::Despawn {
                entity_id,
                generation,
                destroyed: evidence.destroyed(entity_id, generation),
            }),
        }
    }

    for (_, s) in rows {
        ops.push(DiffOp::Spawn(*s));
    }
    ops
}

/// Replication state: owns every proxy, its interpolation buffer, and the
/// local-player binding.
#[derive(Default)]
pub struct Replication {
    pub index: NetIndex,
    pub evidence: TombstoneEvidence,
    local_player: Option<hecs::Entity>,
    own_entity_id: Option<u64>,
    buffers: HashMap<(u64, u32), InterpBuffer>,
}

impl Replication {
    pub fn record_tombstone(&mut self, entity_id: u64, generation: u32) {
        self.evidence.record(entity_id, generation, Instant::now());
    }

    /// Queue an authoritative sample for interpolation (plan D6). Own-row
    /// samples are ignored (the local entity is client-driven in M5), as are
    /// samples for incarnations the diff hasn't spawned (snapshots are
    /// always emitted before samples).
    pub fn push_sample(&mut self, s: &EntityState) {
        if Some(s.entity_id) == self.own_entity_id {
            return;
        }
        if let Some(buf) = self.buffers.get_mut(&(s.entity_id, s.generation)) {
            buf.push(s.server_time_us, s.pos, s.yaw);
        }
    }

    /// Write interpolated transforms for every proxy. `render_time_us` is
    /// estimated server time minus the interpolation delay; `None` (clock
    /// not yet synced) snaps to the newest sample instead.
    pub fn interpolate(&mut self, world: &mut World, render_time_us: Option<u64>) {
        for ((id, generation), buf) in &mut self.buffers {
            let Some(entity) = self.index.get(*id, *generation) else {
                continue;
            };
            let evaluated = match render_time_us {
                Some(t) => buf.sample(t),
                None => buf.latest(),
            };
            if let Some((pos, yaw)) = evaluated {
                write_pose(world, entity, pos, yaw);
            }
        }
    }

    /// M5 trust-the-client local movement: integrate the desired direction
    /// directly on the bound entity and report the pose for `send_input`.
    /// Deliberately minimal (no physics, no camera); M6 replaces this with
    /// server-authoritative movement + reconciliation.
    pub fn drive_local_player(
        &mut self,
        world: &mut World,
        move_dir: [f32; 2],
        sprint: bool,
        dt: f32,
    ) -> Option<ClientInput> {
        let entity = self.local_player?;
        let (pos, yaw) = {
            let mut t = world.get::<&mut Transform>(entity).ok()?;
            let len = (move_dir[0] * move_dir[0] + move_dir[1] * move_dir[1]).sqrt();
            let yaw = if len > 1e-3 {
                let speed = PLAYER_SPEED_MPS * if sprint { SPRINT_MULTIPLIER } else { 1.0 };
                t.position.x += move_dir[0] / len * speed * dt;
                t.position.y += move_dir[1] / len * speed * dt;
                let yaw = move_dir[1].atan2(move_dir[0]);
                t.rotation = glm::quat_angle_axis(yaw, &glm::vec3(0.0, 0.0, 1.0));
                yaw
            } else {
                glm::quat_euler_angles(&t.rotation).z
            };
            ([t.position.x, t.position.y, t.position.z], yaw)
        };
        rust_engine::engine::ecs::hierarchy::mark_transform_dirty(world, entity);
        Some(ClientInput {
            epoch: 0, // stamped by the backend at send time
            seq: 0,
            pos,
            move_dir,
            yaw,
            sprint,
            jump: false,
        })
    }

    pub fn apply_snapshot(&mut self, world: &mut World, snapshot: &WorldSnapshot) {
        self.evidence.prune(Instant::now());
        self.bind_local_player(world, snapshot);

        let live: Vec<(u64, u32)> = self.index.keys().collect();
        for op in diff_snapshot(&live, snapshot, &self.evidence) {
            match op {
                DiffOp::Spawn(s) => self.spawn_proxy(world, &s),
                // Motion flows through the interpolation buffer; the buffer
                // dedupes states that also arrived as `StateUpdate` events.
                DiffOp::Update(s) => self.push_sample(&s),
                DiffOp::Despawn {
                    entity_id,
                    generation,
                    destroyed,
                } => {
                    self.buffers.remove(&(entity_id, generation));
                    if let Some(e) = self.index.map.remove(&(entity_id, generation)) {
                        let _ = world.despawn(e);
                        let kind = if destroyed { "destroyed" } else { "out of scope" };
                        println!("net: despawn {entity_id} gen {generation} ({kind})");
                    }
                }
                DiffOp::Replace {
                    old_generation,
                    state,
                } => {
                    self.buffers.remove(&(state.entity_id, old_generation));
                    if let Some(e) = self.index.map.remove(&(state.entity_id, old_generation)) {
                        let _ = world.despawn(e);
                    }
                    self.spawn_proxy(world, &state);
                }
            }
        }
    }

    /// Contract §2.1: the own row is bound to the local player entity —
    /// created on first sight here (the scene has none) — and registered in
    /// `NetIndex` so generation rules apply uniformly. Never a proxy.
    fn bind_local_player(&mut self, world: &mut World, snapshot: &WorldSnapshot) {
        let Some(own_id) = snapshot.own_entity_id else {
            return;
        };
        let Some(row) = snapshot.entities.iter().find(|s| s.entity_id == own_id) else {
            return;
        };
        self.own_entity_id = Some(own_id);

        match self.local_player {
            None => {
                let entity = world.spawn((
                    transform_from(row),
                    MeshRenderer {
                        mesh_path: PRIMITIVE_SPHERE.to_string(),
                        ..Default::default()
                    },
                    Name::new("Local Player (net)"),
                    EntityGuid(derive_proxy_guid(REALM_ID, row.entity_id, row.generation)),
                    NetLocalPlayer,
                ));
                self.index.map.insert((row.entity_id, row.generation), entity);
                self.local_player = Some(entity);
                println!("net: local player bound to entity {own_id}");
            }
            Some(entity) => {
                // The entity is client-driven (trust-the-client): routine
                // snapshots must NOT touch it. Only a generation bump
                // (respawn) re-keys the binding and snaps.
                if self.index.get(row.entity_id, row.generation) != Some(entity) {
                    self.index.map.retain(|k, v| !(k.0 == own_id && *v == entity));
                    self.index.map.insert((row.entity_id, row.generation), entity);
                    write_transform(world, entity, row);
                }
            }
        }
    }

    fn spawn_proxy(&mut self, world: &mut World, s: &EntityState) {
        let (mesh, label) = match s.kind {
            EntityKind::Player => (PRIMITIVE_SPHERE, "Net Player"),
            EntityKind::Npc => (PRIMITIVE_CUBE, "Net NPC"),
        };
        let entity = world.spawn((
            transform_from(s),
            MeshRenderer {
                mesh_path: mesh.to_string(),
                ..Default::default()
            },
            Name::new(format!("{label} {}", s.entity_id)),
            EntityGuid(derive_proxy_guid(REALM_ID, s.entity_id, s.generation)),
            NetProxy {
                realm_id: REALM_ID,
                entity_id: s.entity_id,
                generation: s.generation,
            },
        ));
        self.index.map.insert((s.entity_id, s.generation), entity);
        // Spawn pose is the first sample, so interpolation starts anchored.
        let mut buf = InterpBuffer::default();
        buf.push(s.server_time_us, s.pos, s.yaw);
        self.buffers.insert((s.entity_id, s.generation), buf);
        println!("net: spawn {label} {} gen {}", s.entity_id, s.generation);
    }
}

fn transform_from(s: &EntityState) -> Transform {
    Transform::new(glm::vec3(s.pos[0], s.pos[1], s.pos[2]))
        .with_rotation(glm::quat_angle_axis(s.yaw, &glm::vec3(0.0, 0.0, 1.0)))
}

fn write_transform(world: &mut World, entity: hecs::Entity, s: &EntityState) {
    write_pose(world, entity, s.pos, s.yaw);
}

fn write_pose(world: &mut World, entity: hecs::Entity, pos: [f32; 3], yaw: f32) {
    if let Ok(mut t) = world.get::<&mut Transform>(entity) {
        t.position = glm::vec3(pos[0], pos[1], pos[2]);
        t.rotation = glm::quat_angle_axis(yaw, &glm::vec3(0.0, 0.0, 1.0));
    } else {
        return;
    }
    // The render matrix is cached; incremental propagation only refreshes
    // entities tagged dirty.
    rust_engine::engine::ecs::hierarchy::mark_transform_dirty(world, entity);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(entity_id: u64, generation: u32) -> EntityState {
        EntityState {
            entity_id,
            generation,
            kind: EntityKind::Npc,
            pos: [1.0, 2.0, 3.0],
            vel: [0.0; 3],
            yaw: 0.5,
            zone_id: 0,
            server_time_us: 42,
        }
    }

    fn snap(entities: Vec<EntityState>, own: Option<u64>) -> WorldSnapshot {
        WorldSnapshot {
            entities,
            own_entity_id: own,
        }
    }

    #[test]
    fn spawns_new_rows_updates_live_ones() {
        let ev = TombstoneEvidence::default();
        let ops = diff_snapshot(&[(1, 1)], &snap(vec![state(1, 1), state(2, 1)], None), &ev);
        assert!(ops.contains(&DiffOp::Update(state(1, 1))));
        assert!(ops.contains(&DiffOp::Spawn(state(2, 1))));
        assert_eq!(ops.len(), 2);
    }

    #[test]
    fn identical_snapshot_is_idempotent() {
        let ev = TombstoneEvidence::default();
        let ops = diff_snapshot(&[(1, 1)], &snap(vec![state(1, 1)], None), &ev);
        assert_eq!(ops, vec![DiffOp::Update(state(1, 1))]);
    }

    #[test]
    fn vanished_row_without_evidence_is_out_of_scope() {
        let ev = TombstoneEvidence::default();
        let ops = diff_snapshot(&[(1, 1)], &snap(vec![], None), &ev);
        assert_eq!(
            ops,
            vec![DiffOp::Despawn {
                entity_id: 1,
                generation: 1,
                destroyed: false
            }]
        );
    }

    #[test]
    fn vanished_row_with_evidence_is_destroyed() {
        let mut ev = TombstoneEvidence::default();
        ev.record(1, 1, Instant::now());
        let ops = diff_snapshot(&[(1, 1)], &snap(vec![], None), &ev);
        assert_eq!(
            ops,
            vec![DiffOp::Despawn {
                entity_id: 1,
                generation: 1,
                destroyed: true
            }]
        );
    }

    #[test]
    fn older_generation_evidence_does_not_destroy_newer_incarnation() {
        let mut ev = TombstoneEvidence::default();
        ev.record(1, 1, Instant::now());
        assert!(ev.destroyed(1, 1));
        assert!(!ev.destroyed(1, 2));
    }

    #[test]
    fn generation_change_on_live_row_is_replace() {
        let ev = TombstoneEvidence::default();
        let ops = diff_snapshot(&[(1, 1)], &snap(vec![state(1, 2)], None), &ev);
        assert_eq!(
            ops,
            vec![DiffOp::Replace {
                old_generation: 1,
                state: state(1, 2)
            }]
        );
    }

    #[test]
    fn own_row_is_fully_excluded() {
        let ev = TombstoneEvidence::default();
        // Own row present in snapshot and in the index (binding entry):
        // neither side may produce an op (contract §2.1).
        let ops = diff_snapshot(&[(7, 1)], &snap(vec![state(7, 1)], Some(7)), &ev);
        assert!(ops.is_empty());
    }

    #[test]
    fn evidence_prunes_on_ttl() {
        let mut ev = TombstoneEvidence::default();
        let old = Instant::now() - Duration::from_secs(TOMBSTONE_TTL_SECS + 1);
        ev.record(1, 1, old);
        ev.prune(Instant::now());
        assert!(!ev.destroyed(1, 1));
    }
}
