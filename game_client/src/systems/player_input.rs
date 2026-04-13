//! Player input system — reads Enhanced Input and writes to movement components.

use game_shared::components::{CharacterMovement, LookController, MovementMode, PlayerInput};
use nalgebra_glm as glm;
use rust_engine::engine::ecs::access::SystemDescriptor;
use rust_engine::engine::ecs::components::{Player, Transform};
use rust_engine::engine::ecs::resources::Resources;
use rust_engine::engine::ecs::schedule::System;
use rust_engine::engine::ecs::system_names;
use rust_engine::engine::input::subsystem::InputSubsystem;

pub struct PlayerInputSystem;

impl System for PlayerInputSystem {
    fn run(&mut self, world: &mut hecs::World, resources: &mut Resources) {
        {
            let mut contexts_to_add: Vec<String> = Vec::new();
            for (_entity, pi) in world.query_mut::<&mut PlayerInput>() {
                if !pi.context_active {
                    contexts_to_add.push(pi.mapping_context.clone());
                    pi.context_active = true;
                }
            }
            if !contexts_to_add.is_empty() {
                if let Some(subsystem) = resources.get_mut::<InputSubsystem>() {
                    for ctx in &contexts_to_add {
                        if !subsystem.has_context(ctx) {
                            subsystem.add_context(ctx);
                        }
                    }
                }
            }
        }

        struct InputData {
            move_xy: (f32, f32),
            look_xy: (f32, f32),
            jump: bool,
            sprint: bool,
        }

        let input_data = {
            let Some(subsystem) = resources.get::<InputSubsystem>() else {
                return;
            };

            let mut data_map: Vec<(hecs::Entity, InputData)> = Vec::new();
            for (entity, pi) in world.query::<&PlayerInput>().iter() {
                data_map.push((
                    entity,
                    InputData {
                        move_xy: subsystem.axis_2d(&pi.move_action),
                        look_xy: subsystem.axis_2d(&pi.look_action),
                        jump: subsystem.just_pressed(&pi.jump_action),
                        sprint: subsystem.digital(&pi.sprint_action),
                    },
                ));
            }
            data_map
        };

        for (entity, input) in &input_data {
            let Ok(mut cm) = world.get::<&mut CharacterMovement>(*entity) else {
                continue;
            };
            let Ok(mut look) = world.get::<&mut LookController>(*entity) else {
                continue;
            };

            let (move_x, move_y) = input.move_xy;
            let (look_x, look_y) = input.look_xy;

            look.yaw -= look_x * look.mouse_sensitivity;
            look.pitch -= look_y * look.mouse_sensitivity;
            look.pitch = look.pitch.clamp(look.pitch_min, look.pitch_max);

            let forward = glm::vec3(look.yaw.cos(), look.yaw.sin(), 0.0);
            let right = glm::vec3(-look.yaw.sin(), look.yaw.cos(), 0.0);

            let has_input = move_x.abs() > 0.01 || move_y.abs() > 0.01;
            if has_input {
                let desired = forward * move_y + right * move_x;
                cm.desired_velocity = [desired.x, desired.y, desired.z];
            } else {
                cm.desired_velocity = [0.0; 3];
            }

            if cm.movement_mode == MovementMode::Flying && has_input {
                let look_forward = glm::vec3(
                    look.yaw.cos() * look.pitch.cos(),
                    look.yaw.sin() * look.pitch.cos(),
                    look.pitch.sin(),
                );
                let desired = look_forward * move_y + right * move_x;
                cm.desired_velocity = [desired.x, desired.y, desired.z];
            }

            cm.jump_requested = input.jump;
            cm.is_sprinting = input.sprint;
        }
    }

    fn name(&self) -> &str {
        "PlayerInputSystem"
    }
}

impl PlayerInputSystem {
    pub fn descriptor() -> SystemDescriptor {
        SystemDescriptor::new("PlayerInputSystem")
            .reads_resource::<InputSubsystem>()
            .writes_resource::<InputSubsystem>()
            .writes::<PlayerInput>()
            .writes::<CharacterMovement>()
            .writes::<LookController>()
            .reads::<Transform>()
            .reads::<Player>()
            .after(system_names::PHYSICS_STEP)
    }
}
