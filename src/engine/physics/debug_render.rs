//! Debug visualization for physics

use crate::engine::physics::PhysicsSystem;
use rapier3d::prelude::*;

pub struct PhysicsDebugRenderer {
    pub enabled: bool,
    pub draw_colliders: bool,
    pub draw_rigid_bodies: bool,
    pub collider_color: [f32; 4],
    pub sleeping_color: [f32; 4],
}

impl Default for PhysicsDebugRenderer {
    fn default() -> Self {
        Self {
            enabled: false,
            draw_colliders: true,
            draw_rigid_bodies: true,
            collider_color: [0.0, 1.0, 0.0, 0.5], // Green wireframe
            sleeping_color: [0.5, 0.5, 0.5, 0.3], // Gray (sleeping)
        }
    }
}

impl PhysicsDebugRenderer {
    /// Generate debug line primitives for rendering
    pub fn generate_debug_lines(&self, physics: &PhysicsSystem) -> Vec<DebugLine> {
        if !self.enabled {
            return Vec::new();
        }

        let mut lines = Vec::new();

        if self.draw_colliders {
            for (handle, collider) in physics.collider_set.iter() {
                let rb_handle = collider.parent().unwrap();
                let rb = &physics.rigid_body_set[rb_handle];

                let color = if rb.is_sleeping() {
                    self.sleeping_color
                } else {
                    self.collider_color
                };

                // Draw collider shape
                self.draw_collider_shape(collider, rb.position(), color, &mut lines);
            }
        }

        lines
    }

    fn draw_collider_shape(
        &self,
        collider: &Collider,
        position: &Isometry<Real>,
        color: [f32; 4],
        lines: &mut Vec<DebugLine>,
    ) {
        match collider.shape().shape_type() {
            ShapeType::Cuboid => {
                // Draw box wireframe (12 edges)
                if let Some(cuboid) = collider.shape().as_cuboid() {
                    let half = cuboid.half_extents;

                    // 8 vertices of the box
                    let vertices = [
                        nalgebra::Vector3::new(-half.x, -half.y, -half.z),
                        nalgebra::Vector3::new( half.x, -half.y, -half.z),
                        nalgebra::Vector3::new( half.x,  half.y, -half.z),
                        nalgebra::Vector3::new(-half.x,  half.y, -half.z),
                        nalgebra::Vector3::new(-half.x, -half.y,  half.z),
                        nalgebra::Vector3::new( half.x, -half.y,  half.z),
                        nalgebra::Vector3::new( half.x,  half.y,  half.z),
                        nalgebra::Vector3::new(-half.x,  half.y,  half.z),
                    ];

                    // Transform vertices by position
                    let transformed: Vec<_> = vertices
                        .iter()
                        .map(|v| position * nalgebra::Point3::from(*v))
                        .collect();

                    // 12 edges (bottom face, top face, vertical edges)
                    let edges = [
                        (0, 1), (1, 2), (2, 3), (3, 0), // Bottom
                        (4, 5), (5, 6), (6, 7), (7, 4), // Top
                        (0, 4), (1, 5), (2, 6), (3, 7), // Vertical
                    ];

                    for (i, j) in edges {
                        lines.push(DebugLine {
                            start: transformed[i].coords,
                            end: transformed[j].coords,
                            color,
                        });
                    }
                }
            }
            ShapeType::Ball => {
                // Draw sphere wireframe (3 circles: XY, XZ, YZ planes)
                if let Some(ball) = collider.shape().as_ball() {
                    let radius = ball.radius;
                    let segments = 16;

                    // XY circle
                    for i in 0..segments {
                        let angle1 = (i as f32 / segments as f32) * std::f32::consts::TAU;
                        let angle2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;

                        let p1 = nalgebra::Point3::new(
                            radius * angle1.cos(),
                            radius * angle1.sin(),
                            0.0,
                        );
                        let p2 = nalgebra::Point3::new(
                            radius * angle2.cos(),
                            radius * angle2.sin(),
                            0.0,
                        );

                        lines.push(DebugLine {
                            start: (position * p1).coords,
                            end: (position * p2).coords,
                            color,
                        });
                    }

                    // XZ circle
                    for i in 0..segments {
                        let angle1 = (i as f32 / segments as f32) * std::f32::consts::TAU;
                        let angle2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;

                        let p1 = nalgebra::Point3::new(
                            radius * angle1.cos(),
                            0.0,
                            radius * angle1.sin(),
                        );
                        let p2 = nalgebra::Point3::new(
                            radius * angle2.cos(),
                            0.0,
                            radius * angle2.sin(),
                        );

                        lines.push(DebugLine {
                            start: (position * p1).coords,
                            end: (position * p2).coords,
                            color,
                        });
                    }

                    // YZ circle
                    for i in 0..segments {
                        let angle1 = (i as f32 / segments as f32) * std::f32::consts::TAU;
                        let angle2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;

                        let p1 = nalgebra::Point3::new(
                            0.0,
                            radius * angle1.cos(),
                            radius * angle1.sin(),
                        );
                        let p2 = nalgebra::Point3::new(
                            0.0,
                            radius * angle2.cos(),
                            radius * angle2.sin(),
                        );

                        lines.push(DebugLine {
                            start: (position * p1).coords,
                            end: (position * p2).coords,
                            color,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

pub struct DebugLine {
    pub start: nalgebra::Vector3<f32>,
    pub end: nalgebra::Vector3<f32>,
    pub color: [f32; 4],
}
