//! Player input system — Enhanced Input → `CharacterMovement` intent
//! (Task 41.6 D2).
//!
//! Movement is camera-relative: the yaw comes from the `OrbitCamera` whose
//! `target` is this player's GUID, else the first `OrbitCamera`, else world
//! +X. Runs in PreUpdate; `EnhancedInputSystem` is in `First`, so stage order
//! sequences them without an explicit edge.

use game_shared::components::{CharacterMovement, OrbitCamera, PlayerInput};
use rust_engine::engine::ecs::access::SystemDescriptor;
use rust_engine::engine::ecs::components::EntityGuid;
use rust_engine::engine::ecs::resources::Resources;
use rust_engine::engine::ecs::schedule::System;
use rust_engine::engine::ecs::system_names;
use rust_engine::engine::input::subsystem::InputSubsystem;
use uuid::Uuid;

pub struct PlayerInputSystem;

impl System for PlayerInputSystem {
    fn run(&mut self, world: &mut hecs::World, resources: &mut Resources) {
        let Some(input) = resources.get_mut::<InputSubsystem>() else {
            return;
        };

        // Activate each player's mapping context once (play-mode start).
        for (_, pi) in world.query_mut::<&mut PlayerInput>() {
            if !pi.context_active {
                if !input.has_context(&pi.mapping_context) {
                    input.add_context(&pi.mapping_context);
                }
                pi.context_active = true;
            }
        }

        let cams: Vec<(Option<Uuid>, f32)> = world
            .query_mut::<&OrbitCamera>()
            .into_iter()
            .map(|(_, c)| (c.target, c.yaw))
            .collect();

        for (_, (pi, cm, guid)) in
            world.query_mut::<(&PlayerInput, &mut CharacterMovement, Option<&EntityGuid>)>()
        {
            let yaw = camera_yaw(&cams, guid.map(|g| g.0));
            cm.desired_dir = camera_relative_dir(input.axis_2d(&pi.move_action), yaw);
            cm.run = input.digital(&pi.sprint_action);
            cm.jump_requested = input.just_pressed(&pi.jump_action);
        }
    }

    fn name(&self) -> &str {
        system_names::PLAYER_INPUT
    }
}

impl PlayerInputSystem {
    pub fn descriptor() -> SystemDescriptor {
        SystemDescriptor::new(system_names::PLAYER_INPUT)
            // `add_context` on first sight of a `PlayerInput`.
            .writes_resource::<InputSubsystem>()
            .writes::<PlayerInput>()
            .writes::<CharacterMovement>()
            .reads::<OrbitCamera>()
            .reads::<EntityGuid>()
    }
}

/// The yaw a player moves relative to, from `(target, yaw)` pairs of every
/// `OrbitCamera`: the one targeting `player`, else the first, else 0 (+X).
pub fn camera_yaw(cams: &[(Option<Uuid>, f32)], player: Option<Uuid>) -> f32 {
    cams.iter()
        .find(|(target, _)| target.is_some() && *target == player)
        .or(cams.first())
        .map_or(0.0, |(_, yaw)| *yaw)
}

/// Movement direction in the XY plane (Z-up: X forward, Y right) for an
/// Enhanced Input `move` axis (`x` right, `y` forward) seen from a camera
/// heading of `camera_yaw` radians. Magnitude is clamped to 1.
pub fn camera_relative_dir(move_xy: (f32, f32), camera_yaw: f32) -> [f32; 2] {
    let (mx, my) = move_xy;
    let (s, c) = camera_yaw.sin_cos();
    // forward = (c, s), right = (-s, c)
    let x = c * my - s * mx;
    let y = s * my + c * mx;
    let len = (x * x + y * y).sqrt();
    if len < 1e-3 {
        [0.0; 2]
    } else if len > 1.0 {
        [x / len, y / len]
    } else {
        [x, y]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5
    }

    #[test]
    fn forward_input_follows_camera_yaw() {
        assert!(close(camera_relative_dir((0.0, 1.0), 0.0), [1.0, 0.0]));
        assert!(close(camera_relative_dir((0.0, 1.0), FRAC_PI_2), [0.0, 1.0]));
        // Strafe right at yaw 0 is +Y; at yaw 90° it is -X.
        assert!(close(camera_relative_dir((1.0, 0.0), 0.0), [0.0, 1.0]));
        assert!(close(camera_relative_dir((1.0, 0.0), FRAC_PI_2), [-1.0, 0.0]));
    }

    #[test]
    fn diagonal_is_clamped_to_unit_and_deadzone_is_zero() {
        let d = camera_relative_dir((1.0, 1.0), 0.0);
        assert!((d[0] * d[0] + d[1] * d[1] - 1.0).abs() < 1e-5);
        assert!(camera_relative_dir((0.5, 0.0), 0.0)[1] == 0.5, "analog magnitude kept");
        assert_eq!(camera_relative_dir((0.0, 0.0), 1.0), [0.0; 2]);
    }

    #[test]
    fn camera_yaw_prefers_targeting_camera_then_first_then_world() {
        let me = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        let cams = [(Some(other), 0.5), (None, 0.7), (Some(me), 0.9)];
        assert_eq!(camera_yaw(&cams, Some(me)), 0.9);
        assert_eq!(camera_yaw(&cams, Some(Uuid::from_u128(3))), 0.5);
        assert_eq!(camera_yaw(&cams, None), 0.5, "untargeted camera never matches by None");
        assert_eq!(camera_yaw(&[], Some(me)), 0.0);
    }
}
