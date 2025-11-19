use glam::Vec3;

/// Directional light (sun, moon)
/// Light rays are parallel (source infinitely far away)
#[derive(Clone, Copy, Debug)]
pub struct DirectionalLight {
    /// Direction light is traveling (not where it's pointing FROM)
    /// Example: Vec3::new(0.0, -1.0, 0.0) = light coming from above
    pub direction: Vec3,

    /// Light color and intensity
    /// Example: Vec3::new(1.0, 0.95, 0.8) = warm sunlight
    pub color: Vec3,

    /// Brightness multiplier
    /// Example: 1.0 = normal, 2.0 = twice as bright
    pub intensity: f32,
}

impl DirectionalLight {
    /// Creates a new directional light
    pub fn new(direction: Vec3, color: Vec3, intensity: f32) -> Self {
        Self {
            direction: direction.normalize(), // Always normalize!
            color,
            intensity,
        }
    }

    /// Standard sunlight (from above, slightly angled)
    pub fn sun() -> Self {
        Self::new(
            Vec3::new(0.3, -1.0, 0.2),  // Coming from upper-right
            Vec3::new(1.0, 0.95, 0.8),  // Warm white
            1.0,
        )
    }

    /// Moonlight (cooler, dimmer)
    pub fn moon() -> Self {
        Self::new(
            Vec3::new(-0.2, -1.0, -0.3),
            Vec3::new(0.7, 0.8, 1.0),  // Cool blue
            0.3,
        )
    }
}

/// Point light (light bulb, torch, fire)
/// Light radiates in all directions, attenuates with distance
#[derive(Clone, Copy, Debug)]
pub struct PointLight {
    /// Position in world space
    pub position: Vec3,

    /// Light color
    pub color: Vec3,

    /// Light intensity at source
    pub intensity: f32,

    /// How far light reaches before fading to zero
    /// Example: 10.0 = light visible 10 units away
    pub range: f32,
}

impl PointLight {
    /// Creates a new point light
    pub fn new(position: Vec3, color: Vec3, intensity: f32, range: f32) -> Self {
        Self {
            position,
            color,
            intensity,
            range,
        }
    }

    /// Calculates light attenuation (fade) at a given distance
    /// Returns 0.0 (no light) to 1.0 (full light)
    pub fn attenuation(&self, distance: f32) -> f32 {
        // Inverse square falloff (physically accurate)
        // But we clamp to range for performance
        if distance > self.range {
            return 0.0;
        }

        // Smooth falloff: 1.0 at distance=0, 0.0 at distance=range
        let normalized = distance / self.range;
        let falloff = 1.0 - normalized * normalized;
        falloff.max(0.0)
    }
}

/// Ambient light (global illumination approximation)
#[derive(Clone, Copy, Debug)]
pub struct AmbientLight {
    /// Color of ambient light
    pub color: Vec3,

    /// Intensity (usually low, like 0.1-0.3)
    pub intensity: f32,
}

impl AmbientLight {
    pub fn new(color: Vec3, intensity: f32) -> Self {
        Self { color, intensity }
    }

    /// Neutral white ambient
    pub fn default() -> Self {
        Self::new(Vec3::ONE, 0.2)
    }
}