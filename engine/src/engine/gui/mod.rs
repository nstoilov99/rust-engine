//! GUI integration surface.
//!
//! The seam is [`crusty`], the crusty-gui integration. The `crusty` module
//! lives on both the main thread (input translation + layout) and the render
//! thread (paint list recording).

pub mod crusty;
