//! Cooked-collision load integration: the checked-in greybox cook must load
//! cleanly. Run with `--release -- --nocapture` to read the M4 streaming
//! baseline (full-world load time).

use rust_engine::engine::assets::asset_source;
use rust_engine::engine::collision::CollisionWorld;

#[test]
fn greybox_cooked_collision_loads_cleanly() {
    asset_source::init_filesystem("../content".into());
    let mut world = CollisionWorld::new();
    let report = world.load_for_scene("scenes/greybox.scene");
    assert_eq!(report.disabled, None);
    assert_eq!(report.skipped, 0, "warnings: {:?}", report.warnings);
    assert_eq!(report.loaded, 81);
    println!(
        "greybox baseline: {} chunk(s) in {:.1} ms",
        report.loaded, report.elapsed_ms
    );
}
