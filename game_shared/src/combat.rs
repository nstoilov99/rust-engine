//! M7 shared ability model (plan D1).
//!
//! Const roster + pure validation, compiled into both the server module and
//! the client: the server enforces, the client (HUD, prediction of legality)
//! reads the same data without asking. No live tuning — changes republish.
//!
//! All times are server micros (`ctx.timestamp` lineage); helpers convert
//! from the roster's seconds.

/// Global cooldown started by every cast.
pub const GCD_SECS: f32 = 1.0;
pub const RESPAWN_SECS: f32 = 5.0;
pub const MANA_REGEN_PER_SEC: f32 = 5.0;
pub const PLAYER_HP_MAX: f32 = 100.0;
pub const PLAYER_MANA_MAX: f32 = 100.0;
pub const NPC_HP_MAX: f32 = 50.0;
/// Projectiles that hit nothing are deleted after this long.
pub const PROJECTILE_LIFETIME_SECS: f32 = 3.0;
/// Eye height above the capsule center for LoS rays (both endpoints).
pub const EYE_OFFSET_M: f32 = 0.6;

pub fn micros(secs: f32) -> u64 {
    (secs * 1_000_000.0) as u64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AbilityId(pub u16);

pub const STRIKE: AbilityId = AbilityId(1);
pub const FIREBOLT: AbilityId = AbilityId(2);
pub const NOVA: AbilityId = AbilityId(3);
pub const HEAL: AbilityId = AbilityId(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbilityKind {
    /// Instant melee hitscan on the target.
    Strike,
    /// Cast, then launch a simulated projectile at the target.
    Projectile,
    /// Instant AoE around the caster; `range_m` is the radius, no target.
    NovaAoe,
    /// Cast on self.
    Heal,
}

impl AbilityKind {
    /// Hostile kinds require a live, connected, non-self target.
    pub fn hostile_targeted(self) -> bool {
        matches!(self, AbilityKind::Strike | AbilityKind::Projectile)
    }
}

pub struct AbilityDef {
    pub id: AbilityId,
    pub name: &'static str,
    /// 0 = instant.
    pub cast_secs: f32,
    /// Per-ability cooldown; 0 = GCD-limited only.
    pub cooldown_secs: f32,
    pub mana_cost: f32,
    /// Max cast distance (3D, row-to-row); radius for `NovaAoe`.
    pub range_m: f32,
    /// Damage dealt, or hp restored for `Heal`.
    pub amount: f32,
    /// Launch speed; only meaningful for `Projectile`.
    pub projectile_speed_mps: f32,
    pub kind: AbilityKind,
}

pub const ABILITIES: &[AbilityDef] = &[
    AbilityDef {
        id: STRIKE,
        name: "Strike",
        cast_secs: 0.0,
        cooldown_secs: 0.0,
        mana_cost: 0.0,
        range_m: 3.0,
        amount: 15.0,
        projectile_speed_mps: 0.0,
        kind: AbilityKind::Strike,
    },
    AbilityDef {
        id: FIREBOLT,
        name: "Firebolt",
        cast_secs: 1.5,
        cooldown_secs: 0.0,
        mana_cost: 30.0,
        range_m: 25.0,
        amount: 25.0,
        // Under MotionConfig gravity (20 m/s²) the ballistic ceiling is
        // v²/g: 30 m/s reaches 45 m, comfortably past the 25 m cast range.
        projectile_speed_mps: 30.0,
        kind: AbilityKind::Projectile,
    },
    AbilityDef {
        id: NOVA,
        name: "Nova",
        cast_secs: 0.0,
        cooldown_secs: 8.0,
        mana_cost: 40.0,
        range_m: 8.0,
        amount: 15.0,
        projectile_speed_mps: 0.0,
        kind: AbilityKind::NovaAoe,
    },
    AbilityDef {
        id: HEAL,
        name: "Heal",
        cast_secs: 2.0,
        cooldown_secs: 5.0,
        mana_cost: 35.0,
        range_m: 0.0,
        amount: 30.0,
        projectile_speed_mps: 0.0,
        kind: AbilityKind::Heal,
    },
];

pub fn ability(id: AbilityId) -> Option<&'static AbilityDef> {
    ABILITIES.iter().find(|a| a.id == id)
}

/// Deterministic `AbilityCooldown` primary key — upsert-by-PK is race-free,
/// no auto_inc + scan. Allocator entity ids stay far below 2^48.
pub fn cooldown_key(entity_id: u64, ability_id: AbilityId) -> u64 {
    (entity_id << 16) | ability_id.0 as u64
}

/// Rejection reasons, ordered by the validation chain (plan D3). The server
/// maps these to its reducer error string; exploit tests assert on effects,
/// not on these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastError {
    Dead,
    UnknownAbility,
    GcdActive,
    OnCooldown,
    NotEnoughMana,
    NoTarget,
    TargetSelf,
    TargetNotFound,
    TargetDead,
    TargetOffline,
    OutOfRange,
    NoLineOfSight,
}

/// Caster-local gate: alive → GCD → cooldown → mana (cheap early-outs
/// first). `cooldown_ready_at_micros` is 0 when no cooldown row exists.
pub fn can_cast(
    def: &AbilityDef,
    now_micros: u64,
    gcd_until_micros: u64,
    cooldown_ready_at_micros: u64,
    mana: f32,
    alive: bool,
) -> Result<(), CastError> {
    if !alive {
        return Err(CastError::Dead);
    }
    if now_micros < gcd_until_micros {
        return Err(CastError::GcdActive);
    }
    if now_micros < cooldown_ready_at_micros {
        return Err(CastError::OnCooldown);
    }
    if mana < def.mana_cost {
        return Err(CastError::NotEnoughMana);
    }
    Ok(())
}

/// Resolved view of a cast target (the caller looks up the row).
/// `connected` is `true` for NPCs and for players with a live session —
/// offline player rows persist and must not be targetable.
#[derive(Debug, Clone, Copy)]
pub struct TargetView {
    pub entity_id: u64,
    pub alive: bool,
    pub connected: bool,
}

/// Target legality per kind (plan D3). `target` is `None` when the client
/// sent no target (id 0) or the row was not found — the two are
/// distinguished by the caller passing `None` only for "no target sent";
/// a sent-but-missing target should be reported as `TargetNotFound` before
/// calling this. Range and LoS are separate, geometry-owning checks.
pub fn target_legal(
    kind: AbilityKind,
    caster_id: u64,
    target: Option<&TargetView>,
) -> Result<(), CastError> {
    match kind {
        AbilityKind::NovaAoe => Ok(()), // untargeted
        AbilityKind::Heal => match target {
            // Self-only (ruled): no target means self; naming yourself is fine.
            None => Ok(()),
            Some(t) if t.entity_id == caster_id => Ok(()),
            Some(_) => Err(CastError::TargetSelf),
        },
        AbilityKind::Strike | AbilityKind::Projectile => {
            let t = target.ok_or(CastError::NoTarget)?;
            if t.entity_id == caster_id {
                return Err(CastError::TargetSelf);
            }
            if !t.connected {
                return Err(CastError::TargetOffline);
            }
            if !t.alive {
                return Err(CastError::TargetDead);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: AbilityId) -> &'static AbilityDef {
        ability(id).unwrap()
    }

    #[test]
    fn roster_ids_unique_and_resolvable() {
        for (i, a) in ABILITIES.iter().enumerate() {
            assert_eq!(ability(a.id).unwrap().name, a.name);
            for b in &ABILITIES[i + 1..] {
                assert_ne!(a.id, b.id);
            }
        }
        assert!(ability(AbilityId(999)).is_none());
    }

    #[test]
    fn cooldown_key_distinct_per_entity_and_ability() {
        assert_ne!(cooldown_key(1, STRIKE), cooldown_key(1, NOVA));
        assert_ne!(cooldown_key(1, STRIKE), cooldown_key(2, STRIKE));
        assert_eq!(cooldown_key(7, HEAL), cooldown_key(7, HEAL));
    }

    #[test]
    fn can_cast_validation_chain() {
        let nova = def(NOVA);
        let now = micros(100.0);
        assert_eq!(can_cast(nova, now, 0, 0, 50.0, true), Ok(()));
        assert_eq!(
            can_cast(nova, now, 0, 0, 50.0, false),
            Err(CastError::Dead)
        );
        assert_eq!(
            can_cast(nova, now, now + 1, 0, 50.0, true),
            Err(CastError::GcdActive)
        );
        assert_eq!(
            can_cast(nova, now, 0, now + 1, 50.0, true),
            Err(CastError::OnCooldown)
        );
        assert_eq!(
            can_cast(nova, now, 0, 0, 39.9, true),
            Err(CastError::NotEnoughMana)
        );
        // Boundary: exactly at the end times / exact mana is castable.
        assert_eq!(can_cast(nova, now, now, now, 40.0, true), Ok(()));
        // Chain order: dead wins over everything else.
        assert_eq!(
            can_cast(nova, now, now + 1, now + 1, 0.0, false),
            Err(CastError::Dead)
        );
    }

    #[test]
    fn strike_needs_live_connected_other() {
        let t = |alive, connected| TargetView {
            entity_id: 2,
            alive,
            connected,
        };
        for kind in [AbilityKind::Strike, AbilityKind::Projectile] {
            assert_eq!(target_legal(kind, 1, Some(&t(true, true))), Ok(()));
            assert_eq!(target_legal(kind, 1, None), Err(CastError::NoTarget));
            assert_eq!(
                target_legal(kind, 2, Some(&t(true, true))),
                Err(CastError::TargetSelf)
            );
            assert_eq!(
                target_legal(kind, 1, Some(&t(false, true))),
                Err(CastError::TargetDead)
            );
            assert_eq!(
                target_legal(kind, 1, Some(&t(true, false))),
                Err(CastError::TargetOffline)
            );
        }
    }

    #[test]
    fn heal_is_self_only_and_nova_untargeted() {
        let me = TargetView {
            entity_id: 1,
            alive: true,
            connected: true,
        };
        let other = TargetView {
            entity_id: 2,
            alive: true,
            connected: true,
        };
        assert_eq!(target_legal(AbilityKind::Heal, 1, None), Ok(()));
        assert_eq!(target_legal(AbilityKind::Heal, 1, Some(&me)), Ok(()));
        assert_eq!(
            target_legal(AbilityKind::Heal, 1, Some(&other)),
            Err(CastError::TargetSelf)
        );
        assert_eq!(target_legal(AbilityKind::NovaAoe, 1, None), Ok(()));
        assert_eq!(target_legal(AbilityKind::NovaAoe, 1, Some(&other)), Ok(()));
    }
}
