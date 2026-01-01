pub mod engine;

pub use engine::*;

// ============================================================================
// Unified Profiling Macros
// ============================================================================
// These macros instrument both puffin (in-app) and Tracy (external GUI) profilers.
// Tracy is only active when built with `--features tracy`.

/// Profile a scope with both puffin and Tracy (when enabled)
///
/// Usage: `profile_scope!("scope_name");`
#[macro_export]
macro_rules! profile_scope {
    ($name:expr) => {
        puffin::profile_scope!($name);
        #[cfg(feature = "tracy")]
        let _tracy_span = tracy_client::span!($name);
    };
}

/// Profile a function with both puffin and Tracy (when enabled)
///
/// Usage: `profile_function!();` at the start of a function
#[macro_export]
macro_rules! profile_function {
    () => {
        puffin::profile_function!();
        #[cfg(feature = "tracy")]
        let _tracy_span = tracy_client::span!();
    };
}