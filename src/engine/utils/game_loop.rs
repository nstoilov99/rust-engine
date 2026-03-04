use std::time::Instant;

/// Manages frame timing and delta time calculation
pub struct GameLoop {
    last_frame: Instant,
    delta_time: f32,
    accumulated_time: f32,
    frame_count: u32,
    fps: f32,
}

impl GameLoop {
    /// Creates a new game loop timer
    pub fn new() -> Self {
        Self {
            last_frame: Instant::now(),
            delta_time: 0.0,
            accumulated_time: 0.0,
            frame_count: 0,
            fps: 0.0,
        }
    }

    /// Call this once per frame to update delta time
    /// Returns delta time in seconds
    pub fn tick(&mut self) -> f32 {
        let now = Instant::now();
        self.delta_time = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;

        // Cap delta time to prevent "spiral of death"
        // If game freezes for 5 seconds, don't try to catch up
        self.delta_time = self.delta_time.min(0.1); // Max 100ms (10 FPS minimum)

        // Update FPS counter
        self.frame_count += 1;
        self.accumulated_time += self.delta_time;

        // Update FPS once per second
        if self.accumulated_time >= 1.0 {
            self.fps = self.frame_count as f32 / self.accumulated_time;
            self.frame_count = 0;
            self.accumulated_time = 0.0;
        }

        self.delta_time
    }

    /// Returns the delta time from the last frame (in seconds)
    pub fn delta(&self) -> f32 {
        self.delta_time
    }

    /// Returns the current FPS (updated once per second)
    pub fn fps(&self) -> f32 {
        self.fps
    }

    /// Returns time since last frame in milliseconds (for display)
    pub fn delta_ms(&self) -> f32 {
        self.delta_time * 1000.0
    }
}

impl Default for GameLoop {
    fn default() -> Self {
        Self::new()
    }
}
