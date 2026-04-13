//! Tracy profiler integration
//!
//! Provides optional Tracy profiler support. When built with `--features tracy`,
//! enables connection to the Tracy GUI for deep profiling analysis.

#[cfg(feature = "tracy")]
mod enabled {
    use tracy_client::Client;

    /// Tracy profiler state when the feature is enabled
    ///
    /// The Tracy client is started immediately on creation so that
    /// profile spans work from the very start of the application.
    pub struct TracyState {
        _client: Client, // Always running - required for span!() to work
    }

    impl TracyState {
        pub fn new() -> Self {
            // Start Tracy client immediately so spans work from the start
            Self {
                _client: Client::start(),
            }
        }

        /// Enable Tracy profiling - no-op since client is always running
        pub fn enable(&mut self) {
            // Client is always running when tracy feature is enabled
        }

        /// Disable Tracy profiling - no-op since client must stay running
        pub fn disable(&mut self) {
            // Cannot disable - client must stay running for spans to work
        }

        /// Check if Tracy is currently enabled (always true when feature enabled)
        pub fn is_enabled(&self) -> bool {
            true
        }

        /// Check if Tracy client is running (for connection status display)
        pub fn is_connected(&self) -> bool {
            Client::is_running()
        }
    }

    impl Default for TracyState {
        fn default() -> Self {
            Self::new()
        }
    }

    impl std::fmt::Debug for TracyState {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("TracyState")
                .field("enabled", &self.is_enabled())
                .field("connected", &self.is_connected())
                .finish()
        }
    }
}

#[cfg(not(feature = "tracy"))]
mod disabled {
    /// Stub Tracy state when the feature is disabled
    #[derive(Debug, Default)]
    pub struct TracyState;

    impl TracyState {
        pub fn new() -> Self {
            Self
        }

        pub fn enable(&mut self) {
            // No-op when Tracy is not compiled in
        }

        pub fn disable(&mut self) {
            // No-op when Tracy is not compiled in
        }

        pub fn is_enabled(&self) -> bool {
            false
        }

        pub fn is_connected(&self) -> bool {
            false
        }
    }
}

#[cfg(feature = "tracy")]
pub use enabled::*;

#[cfg(not(feature = "tracy"))]
pub use disabled::*;

/// Check if Tracy support is compiled in
pub const fn is_tracy_available() -> bool {
    cfg!(feature = "tracy")
}
