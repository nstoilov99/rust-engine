# Rust Game Engine - Knowledge Base

This document contains conventions, patterns, common gotchas, and tribal knowledge for working with the engine.

## Coordinate System (Critical)

### The Golden Rule

**All game logic uses Z-up. Never mix coordinate systems.**

```
Game World (Z-up)              Vulkan Render (Y-up)
      Z (Up/Blue)                    Y (Up)
      |                              |
      |                              |
      +------ Y (Right/Green)        +------ X (Right)
     /                              /
    X (Forward/Red)               -Z (Forward)
```

### When to Use Each Matrix Function

| Function | Use Case |
|----------|----------|
| `local_matrix_zup()` | Hierarchy composition, physics, game logic |
| `model_matrix()` | Simple entities without parents (rendering) |
| `world_matrix_to_render()` | Convert final world matrix for rendering |

### Common Mistakes

```rust
// WRONG: Mixing coordinate systems
let world = parent.model_matrix() * child.local_matrix_zup();

// CORRECT: Compose in Z-up, convert at the end
let world_zup = parent.local_matrix_zup() * child.local_matrix_zup();
let render_matrix = world_matrix_to_render(&world_zup);
```

## ECS Patterns

### Components Are Data Only

```rust
// GOOD: Plain data struct
pub struct Health {
    pub current: f32,
    pub max: f32,
}

// BAD: Logic in component
impl Health {
    pub fn take_damage(&mut self, amount: f32) { ... }  // Don't do this
}
```

### Systems Are Stateless Functions

```rust
// GOOD: System function
pub fn damage_system(world: &mut World, delta: f32) {
    for (id, (health, damage)) in world.query::<(&mut Health, &Damage)>().iter() {
        health.current -= damage.amount * delta;
    }
}

// ACCEPTABLE: System struct for configuration
pub struct DamageSystem {
    pub damage_multiplier: f32,
}

impl System for DamageSystem {
    fn update(&mut self, world: &mut World, delta: f32) {
        // Use self.damage_multiplier
    }
}
```

### Querying Patterns

```rust
// Single component
for (id, transform) in world.query::<&Transform>().iter() { }

// Multiple components
for (id, (transform, mesh)) in world.query::<(&Transform, &MeshRenderer)>().iter() { }

// Optional component
for (id, (transform, mesh_opt)) in world.query::<(&Transform, Option<&MeshRenderer>)>().iter() {
    if let Some(mesh) = mesh_opt { }
}

// Mutable access
for (id, transform) in world.query_mut::<&mut Transform>() { }

// Exclude component
for (id, transform) in world.query::<&Transform>()
    .without::<Static>()
    .iter() { }
```

### Hierarchy Traversal

```rust
// Get world transform (handles parent chain)
let world_transform = hierarchy::get_world_transform(world, entity);

// Iterate children
if let Ok(children) = world.get::<&Children>(parent) {
    for child in &children.0 {
        // Process child
    }
}

// Set parent
hierarchy::set_parent(world, child, Some(parent));
```

## Rendering Patterns

### Mesh Management

```rust
// Load mesh once, reuse index
let mesh_index = mesh_manager.add_mesh(gpu_mesh);

// Reference in component
entity.insert(MeshRenderer {
    mesh_index,
    material_index: 0,
});
```

### Material Setup

```rust
// Materials are indexed, not stored in components
let material = PbrMaterial {
    albedo: [1.0, 0.0, 0.0, 1.0],  // Red
    metallic: 0.0,
    roughness: 0.5,
    ..Default::default()
};
let material_index = material_manager.add(material);
```

### Light Direction Convention

```rust
// Direction points FROM light TO scene (like sun rays)
let sun = DirectionalLight {
    direction: glm::vec3(0.5, -0.5, -1.0).normalize(),  // Z-up space
    color: glm::vec3(1.0, 0.98, 0.95),
    intensity: 2.0,
};
```

## Editor Patterns

### Panel State

```rust
pub struct MyPanel {
    // Persistent UI state
    selected_index: usize,
    scroll_offset: f32,

    // NOT scene data - that goes in ECS
}

impl MyPanel {
    pub fn show(&mut self, ui: &mut egui::Ui, world: &mut World) {
        // Read/write ECS, update UI state
    }
}
```

### Selection System

```rust
// Selection is stored in EditorState, not components
if let Some(selected) = editor_state.selected_entity {
    if let Ok(transform) = world.get::<&Transform>(selected) {
        // Show inspector for selected entity
    }
}
```

### Viewport Input Priority

1. Gizmo interaction (highest)
2. Camera controls
3. Entity selection
4. Panel interaction (lowest when cursor in viewport)

## Physics Patterns

### Body Types

```rust
// Dynamic: Affected by forces, collisions
RigidBodyType::Dynamic

// Kinematic: Moved by code, affects dynamic bodies
RigidBodyType::KinematicPositionBased

// Static: Never moves, infinite mass
RigidBodyType::Static
```

### Sync Timing

```rust
// Physics runs AFTER ECS systems, BEFORE rendering
loop {
    input.update();
    systems.update(world, delta);  // Game logic first
    physics.step(world, delta);    // Then physics
    render(world);                 // Then render
}
```

## Serialization Patterns

### Custom Serde for nalgebra-glm

```rust
// nalgebra types need custom serialization
#[derive(Serialize, Deserialize)]
pub struct Transform {
    #[serde(with = "vec3_serde")]
    pub position: glm::Vec3,

    #[serde(with = "quat_serde")]
    pub rotation: glm::Quat,
}
```

### Entity References in Saved Data

```rust
// DON'T save hecs::Entity directly (unstable IDs)
// DO save stable identifiers
pub struct EntityRef {
    pub name: String,  // Or UUID
}
```

## Performance Gotchas

### Profile Before Optimizing

```rust
crate::profile_function!();
crate::profile_scope!("expensive_operation");
```

### Avoid Per-Frame Allocations

```rust
// BAD: Allocates every frame
fn update(&mut self, world: &mut World) {
    let entities: Vec<Entity> = world.query::<&Transform>()
        .iter()
        .map(|(e, _)| e)
        .collect();
}

// GOOD: Reuse buffer
struct MySystem {
    entity_buffer: Vec<Entity>,
}

fn update(&mut self, world: &mut World) {
    self.entity_buffer.clear();
    self.entity_buffer.extend(
        world.query::<&Transform>().iter().map(|(e, _)| e)
    );
}
```

### Batch Rendering

```rust
// BAD: Draw call per entity
for entity in entities {
    draw(entity);
}

// GOOD: Sort by material, batch
entities.sort_by_key(|e| e.material_index);
for (material, group) in entities.group_by(|e| e.material_index) {
    bind_material(material);
    draw_batch(group);
}
```

## Common Errors and Fixes

### "Entity does not exist"

```rust
// Entity was despawned but reference kept
// FIX: Check existence before access
if world.contains(entity) {
    world.get::<&Transform>(entity)?;
}
```

### Transform Scale is Zero

```rust
// Scale components clamped to prevent matrix singularity
transform.scale.x = transform.scale.x.max(0.001);
```

### Gizmo in Wrong Position

```rust
// Probably using local transform instead of world transform
// FIX: Use hierarchy::get_world_transform()
let world_pos = hierarchy::get_world_transform(world, entity);
```

### Mesh Renders at Origin

```rust
// Model matrix not applied
// FIX: Check push constants include model matrix
push_constants.model = world_matrix_to_render(&world_transform);
```

### Physics Body Doesn't Move

```rust
// Check body type - Static bodies never move
// FIX: Use Dynamic for movable bodies
RigidBodyType::Dynamic
```

## Testing Patterns

### Unit Tests for Systems

```rust
#[test]
fn test_damage_system() {
    let mut world = World::new();
    let entity = world.spawn((Health { current: 100.0, max: 100.0 }, Damage { amount: 10.0 }));

    damage_system(&mut world, 1.0);

    let health = world.get::<&Health>(entity).unwrap();
    assert_eq!(health.current, 90.0);
}
```

### Integration Tests

```rust
// Test coordinate conversion round-trip
#[test]
fn test_coordinate_conversion() {
    let pos_zup = glm::vec3(1.0, 2.0, 3.0);
    let pos_yup = position_to_render(&pos_zup);
    // Verify mapping: X→-Z, Y→X, Z→Y
    assert_eq!(pos_yup, glm::vec3(2.0, 3.0, -1.0));
}
```

## Debugging Tips

### Visual Debugging

```rust
// Draw debug lines (add to debug render pass)
debug_draw.line(start, end, color);
debug_draw.sphere(center, radius, color);
debug_draw.aabb(min, max, color);
```

### Console Commands

```
stat fps          # Show FPS overlay
entity.count      # Count entities in world
help              # List all commands
```

### Profiler Shortcuts

- **puffin**: Built-in, shows flame graph
- **Tracy**: External, more detailed timeline

## Code Style

### Error Handling

```rust
// Use Result for recoverable errors
pub fn load_asset(path: &Path) -> Result<Asset, AssetError> {
    let file = std::fs::read(path)?;
    // ...
}

// Use expect() only for programmer errors
let value = map.get(&key).expect("Key should exist after insert");
```

### Naming Conventions

| Type | Convention | Example |
|------|------------|---------|
| Components | PascalCase noun | `Transform`, `MeshRenderer` |
| Systems | snake_case verb | `update_transforms`, `apply_damage` |
| Resources | PascalCase noun | `Time`, `EditorState` |
| Events | PascalCase past tense | `EntitySpawned`, `CollisionOccurred` |

### Module Organization

```rust
// mod.rs exports public API
pub mod components;
pub mod systems;

pub use components::*;
pub use systems::{TransformSystem, PhysicsSystem};
```

## Patched Dependencies

These crates are forked in `crates/` directory:

| Crate | Reason | Issue |
|-------|--------|-------|
| `emath` | DragValue crash fix | egui #7747 |
| `transform-gizmo` | Z-up coordinate system | Custom |
| `transform-gizmo-egui` | Z-up coordinate system | Custom |

When updating egui, check if patches are still needed.
