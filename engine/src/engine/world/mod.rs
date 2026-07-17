//! World streaming (M4): zone & chunk lifecycle driven by the M3 world
//! manifest. See `docs/roadmap/VULKANO-M4-ZONE-CHUNK-LIFECYCLE.md`.

pub mod streamer;

pub use streamer::{
    diff_ops, load_world, LoadedWorld, StreamOps, StreamingConfig, WorldLoadReport, WorldStreamer,
    ZoneChanged,
};
