//! GUI integration surface.
//!
//! egui has been removed from the editor runtime; the seam that remains
//! is [`crusty`], the crusty-gui integration. The `crusty` module lives
//! on both the main thread (input translation + layout) and the render
//! thread (paint list recording).

#[cfg(feature = "editor")]
pub mod crusty;
