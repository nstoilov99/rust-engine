//! M5 Package 3: cache-diff replication of net entities into ECS proxies.
//!
//! The snapshot diff is the ONLY spawn/despawn authority (plan D3); row
//! callbacks upstream just mark dirty. Identity and lifecycle semantics are
//! normative in `docs/roadmap/M5-WORLD-IDENTITY-CONTRACT.md` (§2–3).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::anim_bridge::CharacterRig;
use crate::interp::InterpBuffer;
use game_shared::components::NetProxy;
use game_shared::motion::MotionConfig;
use game_shared::net::protocol::{CoarseState, EntityKind, EntityState, WorldSnapshot};
use game_shared::net::schema::{derive_proxy_guid, REALM_ID, TOMBSTONE_TTL_SECS};
use hecs::World;
use nalgebra_glm as glm;
use rust_engine::engine::animation::graph::AnimGraphRunner;
use rust_engine::engine::ecs::components::{EntityGuid, MeshRenderer, Name, Transform};
use rust_engine::engine::ecs::hierarchy::{despawn_recursive, set_parent};
use rust_engine::engine::rendering::rendering_3d::mesh::PRIMITIVE_CUBE;

/// Marker on the local player entity bound to the own player row
/// (contract §2.1: the own row is bound, never proxied).
pub struct NetLocalPlayer;

/// The character graph every net player runs — local and proxies alike, on
/// every client (ADR 0002: animation is derived, never replicated).
const CHARACTER_GRAPH: &str = "graphs/character.animgraph";

/// Attach the character visual to a net player entity: a mesh child carrying
/// the animation graph, offset so the model's feet sit under the replicated
/// capsule-center position. A child, not components on the entity itself,
/// because the per-frame net pose write owns the parent transform.
fn spawn_character_rig(world: &mut World, owner: hecs::Entity) {
    let cfg = MotionConfig::default();
    let foot_offset = cfg.capsule_half_seg + cfg.capsule_radius + cfg.skin;
    let rig = world.spawn((
        Transform::new(glm::vec3(0.0, 0.0, -foot_offset)),
        MeshRenderer {
            mesh_path: "Defeated.mesh".to_string(),
            material_paths: vec![
                "Defeated_Beta_HighLimbsGeoSG3.material".to_string(),
                "Defeated_Beta_Joints_MAT1.material".to_string(),
            ],
            ..Default::default()
        },
        Name::new("Character Rig"),
        AnimGraphRunner::new(CHARACTER_GRAPH),
        CharacterRig,
    ));
    set_parent(world, rig, owner);
    rust_engine::engine::ecs::hierarchy::mark_transform_dirty(world, rig);
}

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
    /// M8 D3: far-tier light proxy (no interp buffer, no combat, no HUD).
    SpawnCoarse(CoarseState),
    /// Same-incarnation coarse row: feed the coarse track — or, if the proxy
    /// is currently full-tier, demote it (near→far hand-off, GUID kept).
    UpdateCoarse(CoarseState),
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
    ReplaceCoarse {
        old_generation: u32,
        state: CoarseState,
    },
}

/// Merged per-entity view of the snapshot (M8 tier precedence): a full-tier
/// row always wins; a coarse row matters only when no full row is present.
#[derive(Clone, Copy)]
enum Tier<'a> {
    Full(&'a EntityState),
    Coarse(&'a CoarseState),
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

    let mut rows: HashMap<u64, Tier> = HashMap::new();
    for c in &snapshot.coarse {
        if Some(c.entity_id) != own {
            rows.insert(c.entity_id, Tier::Coarse(c));
        }
    }
    for s in &snapshot.entities {
        if Some(s.entity_id) != own {
            rows.insert(s.entity_id, Tier::Full(s)); // full tier wins
        }
    }

    for &(entity_id, generation) in live {
        if Some(entity_id) == own {
            continue;
        }
        match rows.remove(&entity_id) {
            Some(Tier::Full(s)) if s.generation == generation => ops.push(DiffOp::Update(*s)),
            Some(Tier::Full(s)) => ops.push(DiffOp::Replace {
                old_generation: generation,
                state: *s,
            }),
            Some(Tier::Coarse(c)) if c.generation == generation => {
                ops.push(DiffOp::UpdateCoarse(*c))
            }
            Some(Tier::Coarse(c)) => ops.push(DiffOp::ReplaceCoarse {
                old_generation: generation,
                state: *c,
            }),
            None => ops.push(DiffOp::Despawn {
                entity_id,
                generation,
                destroyed: evidence.destroyed(entity_id, generation),
            }),
        }
    }

    for (_, row) in rows {
        ops.push(match row {
            Tier::Full(s) => DiffOp::Spawn(*s),
            Tier::Coarse(c) => DiffOp::SpawnCoarse(*c),
        });
    }
    ops
}

/// Latest-value combat state of a proxy (M7 D2): never interpolated, guarded
/// by sample time so a reordered stale sample can't resurrect hp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CombatState {
    pub hp: f32,
    pub hp_max: f32,
    pub alive: bool,
    server_time_us: u64,
}

/// Far-tier motion (M8 D3): coarse samples arrive ~1 Hz, so no
/// `InterpBuffer` — lerp from the rendered position toward the newest sample
/// over the two samples' timestamp gap, on the local wall clock (reads as
/// slow drift at distance; no server-clock coupling).
struct CoarseTrack {
    from: [f32; 3],
    to: [f32; 3],
    duration: Duration,
    since: Instant,
    last_time_us: u64,
    yaw: f32,
}

impl CoarseTrack {
    fn new(pos: [f32; 3], time_us: u64) -> Self {
        Self {
            from: pos,
            to: pos,
            duration: Duration::ZERO,
            since: Instant::now(),
            last_time_us: time_us,
            yaw: 0.0,
        }
    }

    fn push(&mut self, pos: [f32; 3], time_us: u64) {
        if time_us <= self.last_time_us {
            return; // stale/duplicate sample
        }
        let now = Instant::now();
        let cur = self.eval(now).0;
        let gap_us = (time_us - self.last_time_us).clamp(100_000, 2_000_000);
        let (dx, dy) = (pos[0] - cur[0], pos[1] - cur[1]);
        if dx * dx + dy * dy > 1e-4 {
            self.yaw = dy.atan2(dx);
        }
        self.from = cur;
        self.to = pos;
        self.duration = Duration::from_micros(gap_us);
        self.since = now;
        self.last_time_us = time_us;
    }

    fn eval(&self, now: Instant) -> ([f32; 3], f32) {
        if self.duration.is_zero() {
            return (self.to, self.yaw);
        }
        let f = (now.duration_since(self.since).as_secs_f32() / self.duration.as_secs_f32())
            .min(1.0);
        let lerp = |a: f32, b: f32| a + (b - a) * f;
        (
            [
                lerp(self.from[0], self.to[0]),
                lerp(self.from[1], self.to[1]),
                lerp(self.from[2], self.to[2]),
            ],
            self.yaw,
        )
    }
}

/// Replication state: owns every proxy, its interpolation buffer, and the
/// local-player binding.
#[derive(Default)]
pub struct Replication {
    pub index: NetIndex,
    pub evidence: TombstoneEvidence,
    local_player: Option<hecs::Entity>,
    own_entity_id: Option<u64>,
    buffers: HashMap<(u64, u32), (EntityKind, InterpBuffer)>,
    combat: HashMap<(u64, u32), CombatState>,
    /// Far-tier proxies (M8 D3): disjoint from `buffers`, so coarse proxies
    /// are excluded from targeting and HUD for free.
    coarse: HashMap<(u64, u32), CoarseTrack>,
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
        if let Some((_, buf)) = self.buffers.get_mut(&(s.entity_id, s.generation)) {
            buf.push(s.server_time_us, s.pos, s.yaw);
            self.push_combat(s);
        }
    }

    /// Latest-value overwrite; older samples never regress it.
    fn push_combat(&mut self, s: &EntityState) {
        let next = CombatState {
            hp: s.hp,
            hp_max: s.hp_max,
            alive: s.alive,
            server_time_us: s.server_time_us,
        };
        let slot = self.combat.entry((s.entity_id, s.generation)).or_insert(next);
        if s.server_time_us >= slot.server_time_us {
            *slot = next;
        }
    }

    /// Latest known combat state of a live proxy incarnation (M7 HUD target
    /// frame; the anim bridge derives remote death from it).
    pub fn combat_state(&self, entity_id: u64, generation: u32) -> Option<&CombatState> {
        self.combat.get(&(entity_id, generation))
    }

    /// Kind + latest combat state of the live incarnation of `entity_id`
    /// (M7 HUD target frame). At most one incarnation is ever live.
    #[cfg(all(not(feature = "editor"), feature = "hud"))]
    pub fn target_info(&self, entity_id: u64) -> Option<(EntityKind, Option<CombatState>)> {
        self.buffers
            .iter()
            .find(|((id, _), _)| *id == entity_id)
            .map(|((id, generation), (kind, _))| {
                (*kind, self.combat.get(&(*id, *generation)).copied())
            })
    }

    /// Alive proxies with a known position (M7 D6 Tab targeting). The own
    /// row is never in `buffers`, so the local player is excluded for free.
    pub fn targetables(&self) -> Vec<(u64, [f32; 3])> {
        self.buffers
            .iter()
            .filter_map(|((id, generation), (_, buf))| {
                let alive = self
                    .combat
                    .get(&(*id, *generation))
                    .map_or(true, |c| c.alive);
                let (pos, _) = buf.latest()?;
                (alive).then_some((*id, pos))
            })
            .collect()
    }

    /// Write interpolated transforms for every proxy, each evaluated at
    /// estimated server time minus its kind's interpolation delay.
    /// `None` (clock not yet synced) snaps to the newest sample instead.
    pub fn interpolate(&mut self, world: &mut World, server_now_us: Option<u64>) {
        for ((id, generation), (kind, buf)) in &mut self.buffers {
            let Some(entity) = self.index.get(*id, *generation) else {
                continue;
            };
            let evaluated = match server_now_us {
                Some(t) => buf.sample(t.saturating_sub(crate::interp::interp_delay_us(*kind))),
                None => buf.latest(),
            };
            if let Some((pos, yaw)) = evaluated {
                write_pose(world, entity, pos, yaw);
            }
        }
        // Far-tier drift (M8 D3): wall-clock lerp, no server time base.
        let now = Instant::now();
        for ((id, generation), track) in &self.coarse {
            if let Some(entity) = self.index.get(*id, *generation) {
                let (pos, yaw) = track.eval(now);
                write_pose(world, entity, pos, yaw);
            }
        }
    }

    /// Write the predicted pose onto the bound local player (M6 D4: the
    /// local entity is prediction-driven; snapshots never touch it).
    pub fn set_local_pose(&self, world: &mut World, pos: [f32; 3], yaw: f32) {
        if let Some(entity) = self.local_player {
            write_pose(world, entity, pos, yaw);
        }
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
                // A coarse-tier proxy seeing a full row is the far→near
                // hand-off: promote in place (same entity, same GUID).
                DiffOp::Update(s) => {
                    let key = (s.entity_id, s.generation);
                    if self.coarse.remove(&key).is_some() {
                        let mut buf = InterpBuffer::default();
                        buf.push(s.server_time_us, s.pos, s.yaw);
                        self.buffers.insert(key, (s.kind, buf));
                        self.push_combat(&s);
                    } else {
                        self.push_sample(&s);
                    }
                }
                DiffOp::SpawnCoarse(c) => self.spawn_coarse_proxy(world, &c),
                DiffOp::UpdateCoarse(c) => {
                    let key = (c.entity_id, c.generation);
                    // Near→far hand-off: drop full-tier state, keep the
                    // entity; the track starts from the last rendered pose.
                    if let Some((_, buf)) = self.buffers.remove(&key) {
                        self.combat.remove(&key);
                        let pos = buf.latest().map_or(c.pos, |(p, _)| p);
                        let mut track = CoarseTrack::new(pos, 0);
                        track.push(c.pos, c.server_time_us.max(1));
                        self.coarse.insert(key, track);
                    } else if let Some(track) = self.coarse.get_mut(&key) {
                        track.push(c.pos, c.server_time_us);
                    }
                }
                DiffOp::Despawn {
                    entity_id,
                    generation,
                    destroyed,
                } => {
                    self.buffers.remove(&(entity_id, generation));
                    self.combat.remove(&(entity_id, generation));
                    self.coarse.remove(&(entity_id, generation));
                    if let Some(e) = self.index.map.remove(&(entity_id, generation)) {
                        despawn_recursive(world, e);
                        let kind = if destroyed { "destroyed" } else { "out of scope" };
                        println!("net: despawn {entity_id} gen {generation} ({kind})");
                    }
                }
                DiffOp::Replace {
                    old_generation,
                    state,
                } => {
                    self.remove_incarnation(world, state.entity_id, old_generation);
                    self.spawn_proxy(world, &state);
                }
                DiffOp::ReplaceCoarse {
                    old_generation,
                    state,
                } => {
                    self.remove_incarnation(world, state.entity_id, old_generation);
                    self.spawn_coarse_proxy(world, &state);
                }
            }
        }
    }

    fn remove_incarnation(&mut self, world: &mut World, entity_id: u64, generation: u32) {
        self.buffers.remove(&(entity_id, generation));
        self.combat.remove(&(entity_id, generation));
        self.coarse.remove(&(entity_id, generation));
        if let Some(e) = self.index.map.remove(&(entity_id, generation)) {
            despawn_recursive(world, e);
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
                    Name::new("Local Player (net)"),
                    EntityGuid(derive_proxy_guid(REALM_ID, row.entity_id, row.generation)),
                    NetLocalPlayer,
                ));
                spawn_character_rig(world, entity);
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
        let label = proxy_label(s.kind);
        let entity = world.spawn((
            transform_from(s),
            Name::new(format!("{label} {}", s.entity_id)),
            EntityGuid(derive_proxy_guid(REALM_ID, s.entity_id, s.generation)),
            NetProxy {
                realm_id: REALM_ID,
                entity_id: s.entity_id,
                generation: s.generation,
            },
        ));
        attach_proxy_visual(world, entity, s.kind);
        self.index.map.insert((s.entity_id, s.generation), entity);
        // Spawn pose is the first sample, so interpolation starts anchored.
        let mut buf = InterpBuffer::default();
        buf.push(s.server_time_us, s.pos, s.yaw);
        self.buffers.insert((s.entity_id, s.generation), (s.kind, buf));
        self.push_combat(s);
        println!("net: spawn {label} {} gen {}", s.entity_id, s.generation);
    }

    /// M8 D3: far-tier light proxy — same visual and GUID scheme as a full
    /// proxy (tier hand-offs keep the entity), but no interp buffer and no
    /// combat state, so it is untargetable and HUD-invisible by construction.
    fn spawn_coarse_proxy(&mut self, world: &mut World, c: &CoarseState) {
        let label = proxy_label(c.kind);
        let entity = world.spawn((
            Transform::new(glm::vec3(c.pos[0], c.pos[1], c.pos[2])),
            Name::new(format!("{label} {} (far)", c.entity_id)),
            EntityGuid(derive_proxy_guid(REALM_ID, c.entity_id, c.generation)),
            NetProxy {
                realm_id: REALM_ID,
                entity_id: c.entity_id,
                generation: c.generation,
            },
        ));
        attach_proxy_visual(world, entity, c.kind);
        self.index.map.insert((c.entity_id, c.generation), entity);
        self.coarse.insert(
            (c.entity_id, c.generation),
            CoarseTrack::new(c.pos, c.server_time_us),
        );
        println!("net: spawn {label} {} gen {} (far)", c.entity_id, c.generation);
    }
}

fn proxy_label(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Player => "Net Player",
        EntityKind::Npc => "Net NPC",
    }
}

/// A player proxy gets the animated character rig (both tiers — hand-offs
/// keep the entity, so the visual must not depend on tier); an NPC stays the
/// M5 cube.
fn attach_proxy_visual(world: &mut World, entity: hecs::Entity, kind: EntityKind) {
    match kind {
        EntityKind::Player => spawn_character_rig(world, entity),
        EntityKind::Npc => {
            let _ = world.insert_one(
                entity,
                MeshRenderer {
                    mesh_path: PRIMITIVE_CUBE.to_string(),
                    ..Default::default()
                },
            );
        }
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
            cell_id: 0,
            hp: 50.0,
            hp_max: 50.0,
            alive: true,
            server_time_us: 42,
        }
    }

    fn coarse(entity_id: u64, generation: u32) -> CoarseState {
        CoarseState {
            entity_id,
            generation,
            kind: EntityKind::Npc,
            pos: [4.0, 5.0, 6.0],
            server_time_us: 42,
        }
    }

    fn snap(entities: Vec<EntityState>, own: Option<u64>) -> WorldSnapshot {
        WorldSnapshot {
            entities,
            coarse: vec![],
            own_entity_id: own,
        }
    }

    fn snap_tiers(
        entities: Vec<EntityState>,
        coarse: Vec<CoarseState>,
        own: Option<u64>,
    ) -> WorldSnapshot {
        WorldSnapshot {
            entities,
            coarse,
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
    fn stale_generation_sample_does_not_attach() {
        let mut r = Replication::default();
        let mut world = World::new();
        r.apply_snapshot(&mut world, &snap(vec![state(1, 1)], None));
        // A respawned incarnation replaces the proxy...
        let mut newer = state(1, 2);
        newer.pos = [9.0, 9.0, 9.0];
        newer.server_time_us = 100;
        r.apply_snapshot(&mut world, &snap(vec![newer], None));
        assert!(!r.buffers.contains_key(&(1, 1)));
        // ...and a late sample from the old incarnation must not attach.
        let mut stale = state(1, 1);
        stale.pos = [50.0, 0.0, 0.0];
        stale.server_time_us = 200;
        r.push_sample(&stale);
        let (_, buf) = &r.buffers[&(1, 2)];
        assert_eq!(buf.latest().unwrap().0, [9.0, 9.0, 9.0]);
    }

    #[test]
    fn stale_combat_sample_never_resurrects() {
        let mut r = Replication::default();
        let mut world = World::new();
        r.apply_snapshot(&mut world, &snap(vec![state(1, 1)], None));
        let mut dead = state(1, 1);
        (dead.hp, dead.alive, dead.server_time_us) = (0.0, false, 100);
        r.push_sample(&dead);
        r.push_sample(&state(1, 1)); // reordered stale sample (t=42, hp 50)
        let c = *r.combat_state(1, 1).unwrap();
        assert_eq!((c.hp, c.alive), (0.0, false));
    }

    #[test]
    fn full_row_wins_over_coarse_row() {
        // M8 tier precedence: dual membership during a swap must not
        // produce a second op for the same entity.
        let ev = TombstoneEvidence::default();
        let ops = diff_snapshot(
            &[(1, 1)],
            &snap_tiers(vec![state(1, 1)], vec![coarse(1, 1)], None),
            &ev,
        );
        assert_eq!(ops, vec![DiffOp::Update(state(1, 1))]);
    }

    #[test]
    fn coarse_only_row_spawns_and_updates_light() {
        let ev = TombstoneEvidence::default();
        let ops = diff_snapshot(&[], &snap_tiers(vec![], vec![coarse(2, 1)], None), &ev);
        assert_eq!(ops, vec![DiffOp::SpawnCoarse(coarse(2, 1))]);
        let ops = diff_snapshot(&[(2, 1)], &snap_tiers(vec![], vec![coarse(2, 1)], None), &ev);
        assert_eq!(ops, vec![DiffOp::UpdateCoarse(coarse(2, 1))]);
    }

    #[test]
    fn coarse_generation_bump_is_replace() {
        let ev = TombstoneEvidence::default();
        let ops = diff_snapshot(&[(2, 1)], &snap_tiers(vec![], vec![coarse(2, 2)], None), &ev);
        assert_eq!(
            ops,
            vec![DiffOp::ReplaceCoarse {
                old_generation: 1,
                state: coarse(2, 2)
            }]
        );
    }

    #[test]
    fn tier_handoff_keeps_entity_and_toggles_targetability() {
        let mut r = Replication::default();
        let mut world = World::new();
        // Spawn far, promote to near, demote back — one entity throughout.
        r.apply_snapshot(&mut world, &snap_tiers(vec![], vec![coarse(1, 1)], None));
        let far = r.index.get(1, 1).expect("far proxy spawned");
        assert!(r.targetables().is_empty(), "coarse proxy must be untargetable");

        r.apply_snapshot(&mut world, &snap(vec![state(1, 1)], None));
        assert_eq!(r.index.get(1, 1), Some(far), "promotion replaced the entity");
        assert!(r.buffers.contains_key(&(1, 1)) && !r.coarse.contains_key(&(1, 1)));
        assert_eq!(r.targetables().len(), 1);

        r.apply_snapshot(&mut world, &snap_tiers(vec![], vec![coarse(1, 1)], None));
        assert_eq!(r.index.get(1, 1), Some(far), "demotion replaced the entity");
        assert!(!r.buffers.contains_key(&(1, 1)) && r.coarse.contains_key(&(1, 1)));
        assert!(r.targetables().is_empty());
        assert_eq!(world.len(), 1);
    }

    #[test]
    fn coarse_track_ignores_stale_and_drifts_toward_newest() {
        let mut t = CoarseTrack::new([0.0; 3], 100);
        t.push([10.0, 0.0, 0.0], 50); // stale: ignored
        assert_eq!(t.eval(Instant::now()).0, [0.0; 3]);
        t.push([10.0, 0.0, 0.0], 1_100_000);
        let (pos, yaw) = t.eval(Instant::now() + Duration::from_secs(10));
        assert_eq!(pos, [10.0, 0.0, 0.0]); // clamped at the newest sample
        assert!(yaw.abs() < 1e-6); // facing +X
    }

    #[test]
    fn player_proxy_gets_a_character_rig_and_despawns_with_it() {
        use rust_engine::engine::ecs::hierarchy::Parent;

        let mut r = Replication::default();
        let mut world = World::new();
        let mut s = state(1, 1);
        s.kind = EntityKind::Player;
        r.apply_snapshot(&mut world, &snap(vec![s], None));

        assert_eq!(world.len(), 2, "proxy + rig child");
        let (rig, owner) = world
            .query::<(&CharacterRig, &Parent)>()
            .iter()
            .map(|(e, (_, p))| (e, p.0))
            .next()
            .expect("player proxy carries a rig");
        assert_eq!(owner, r.index.get(1, 1).unwrap());
        assert_eq!(
            world.get::<&AnimGraphRunner>(rig).unwrap().graph,
            CHARACTER_GRAPH,
            "the rig runs the shipped character graph"
        );

        // Out of scope: the rig must not outlive its proxy.
        r.apply_snapshot(&mut world, &snap(vec![], None));
        assert_eq!(world.len(), 0);
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
