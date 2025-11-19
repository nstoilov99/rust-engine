/// Defines a single animation (e.g., "walk", "run", "idle")
#[derive(Debug, Clone)]
pub struct Animation {
    pub name: String,
    pub start_frame: u32,
    pub end_frame: u32,
    pub frame_duration: f32,  // Seconds per frame
    pub looping: bool,
}

impl Animation {
    pub fn new(name: &str, start_frame: u32, end_frame: u32, fps: f32, looping: bool) -> Self {
        Self {
            name: name.to_string(),
            start_frame,
            end_frame,
            frame_duration: 1.0 / fps,
            looping,
        }
    }

    /// Total number of frames in this animation
    pub fn frame_count(&self) -> u32 {
        self.end_frame - self.start_frame + 1
    }

    /// Total duration of animation in seconds
    pub fn duration(&self) -> f32 {
        self.frame_count() as f32 * self.frame_duration
    }
}

/// Manages animation state and playback
#[derive(Debug, Clone)]
pub struct AnimationController {
    pub current_animation: String,
    pub current_frame: u32,
    pub time_accumulator: f32,
    pub playing: bool,
    animations: Vec<Animation>,
}

impl AnimationController {
    pub fn new() -> Self {
        Self {
            current_animation: String::new(),
            current_frame: 0,
            time_accumulator: 0.0,
            playing: false,
            animations: Vec::new(),
        }
    }

    /// Add an animation to the controller
    pub fn add_animation(&mut self, animation: Animation) {
        self.animations.push(animation);
    }

    /// Play a specific animation by name
    pub fn play(&mut self, animation_name: &str) {
        if let Some(anim) = self.animations.iter().find(|a| a.name == animation_name) {
            // Only reset if switching to different animation
            if self.current_animation != animation_name {
                self.current_animation = animation_name.to_string();
                self.current_frame = anim.start_frame;
                self.time_accumulator = 0.0;
            }
            self.playing = true;
        }
    }

    /// Stop the current animation
    pub fn stop(&mut self) {
        self.playing = false;
    }

    /// Update animation (call every frame)
    pub fn update(&mut self, delta_time: f32) {
        if !self.playing {
            return;
        }

        let Some(anim) = self.animations.iter().find(|a| a.name == self.current_animation) else {
            return;
        };

        self.time_accumulator += delta_time;

        // Check if we should advance to next frame
        if self.time_accumulator >= anim.frame_duration {
            self.time_accumulator -= anim.frame_duration;
            self.current_frame += 1;

            // Handle end of animation
            if self.current_frame > anim.end_frame {
                if anim.looping {
                    self.current_frame = anim.start_frame;
                } else {
                    self.current_frame = anim.end_frame;
                    self.playing = false;
                }
            }
        }
    }

    /// Get the current frame index
    pub fn get_current_frame(&self) -> u32 {
        self.current_frame
    }
}