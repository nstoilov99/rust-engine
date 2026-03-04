# Task 28: Automated Testing & CI Pipeline

## Objective

Establish a comprehensive testing infrastructure and continuous integration
pipeline for the Rust engine, covering ECS, physics, serialization, and
editor play-mode features. All tests are CPU-only and deterministic (no
GPU/Vulkan required).

## Deliverables

### 1. Shared Test Helpers

| File | Scope |
|------|-------|
| `src/engine/ecs/test_helpers.rs` | `#[cfg(test)]` helpers for inline unit tests: `assert_approx_eq`, `assert_vec3_approx_eq`, `assert_quat_approx_eq`, `spawn_named_at`, `spawn_child_at`, `test_resources`, `test_resources_playing` |
| `tests/common/mod.rs` | Shared helpers for integration tests: `assert_approx_eq`, `spawn_named_entity`, `spawn_child_entity` |

### 2. Unit Tests (inline `#[cfg(test)]` modules)

| Module | Tests | Coverage |
|--------|-------|----------|
| `commands.rs` | 8 | Spawn, despawn, insert, remove, custom, ordering, empty apply, clear |
| `events.rs` | 11 | Double-buffering, batch send, auto-register, multiple types, clearing |
| `schedule.rs` | 8 | Stage ordering, insertion order, run criteria, enable/disable |
| `components.rs` | 12 | Transform builders/matrix/orthogonality, EntityGuid uniqueness/roundtrip, serde |
| `physics/world.rs` | 10 | Register dynamic/static/kinematic, skip duplicates, gravity, collider shapes |

### 3. Extended Test Modules

| Module | New Tests |
|--------|-----------|
| `render_adapter.rs` | 3 round-trip tests (position, rotation, non-uniform scale) |
| `physics_adapter.rs` | 4 round-trip tests (velocity, rotation 90deg, gravity, position zero) |
| `coords.rs` | 3 round-trip tests (arbitrary position, scale, multiple rotations) |

### 4. Integration Tests

| File | Tests | Description |
|------|-------|-------------|
| `tests/serialization_roundtrip.rs` | 12 | RON serialize/deserialize for all component types, hierarchy, GUIDs, error cases |
| `tests/play_mode.rs` | 7 | Editor snapshot/restore: entity count, transforms, hierarchy, physics rebuild, GUIDs, selection, root order (gated behind `--features editor`) |
| `tests/scene_smoke.rs` | 1 | 51+ entity scene with diverse components, full roundtrip |

### 5. CI Pipeline (`.github/workflows/ci.yml`)

- **Windows** (primary): build, test, clippy, format (both default and editor features)
- **Linux** (cross-platform): build, test, clippy, format (default features)
- Cargo registry + build caching
- Clippy runs with `-D warnings` on `--lib --bins --tests`

### 6. Clippy/Format Cleanup

All pre-existing clippy warnings resolved across the codebase:
- `clone_on_copy`, `type_complexity`, `collapsible_if`, `useless_conversion`
- `get_first`, `unwrap_or_default`, `unnecessary_map_or`, `for_kv_map`
- `ptr_arg`, `should_implement_trait`, `new_without_default`
- `derivable_impls`, `needless_borrow`, `needless_return`, `too_many_arguments`
- `unused_enumerate_index`, `map_clone`, `module_inception`
- All formatting standardized via `cargo fmt`

## Test Counts

| Category | Count |
|----------|-------|
| Library unit tests (default) | 119 |
| Library unit tests (editor) | 132 |
| Integration: serialization | 12 |
| Integration: play_mode | 7 |
| Integration: scene_smoke | 1 |
| **Total (default)** | **132** |
| **Total (editor)** | **152** |

## Constraints

- No GPU/Vulkan initialization required
- No new crate dependencies added
- All tests deterministic and reproducible
- `expect()` preferred over `unwrap()` in test assertions
- Both `cargo test` and `cargo test --features editor` pass

## Known Pre-existing Issues

- `test_scope_colors` in `engine::editor::profiler::scope_colors` fails
  (color assertion unrelated to this task)
- Example files have compilation errors (excluded from clippy via
  `--lib --bins --tests`)

---

## How to Run Tests

```bash
# Run all library unit tests (default features)
cargo test --lib

# Run all library unit tests (with editor features)
cargo test --lib --features editor

# Run all integration tests
cargo test --test serialization_roundtrip
cargo test --test scene_smoke
cargo test --test play_mode --features editor

# Run everything at once (default features)
cargo test

# Run everything at once (editor features)
cargo test --features editor

# Clippy lint check (matching CI)
cargo clippy --lib --bins --tests -- -D warnings
cargo clippy --lib --bins --tests --features editor -- -D warnings

# Format check
cargo fmt --all -- --check

# Run a specific test by name
cargo test transform_roundtrip
cargo test --lib -- schedule::tests

# Run tests with output shown
cargo test -- --nocapture
```
