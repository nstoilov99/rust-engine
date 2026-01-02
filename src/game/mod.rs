//! Game application module
//!
//! This module contains the game-specific code including:
//! - Application state management
//! - Input handling
//! - Scene setup
//! - Rendering orchestration

pub mod app;
pub mod game_setup;
pub mod input_handler;
pub mod render_loop;

pub use app::App;
