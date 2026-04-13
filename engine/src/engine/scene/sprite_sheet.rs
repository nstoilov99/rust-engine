use glam::Vec2;

/// Defines a sprite sheet with frame layout
#[derive(Debug, Clone)]
pub struct SpriteSheet {
    pub texture_size: Vec2,  // Full texture size (e.g., 512x512)
    pub frame_size: Vec2,    // Size of one frame (e.g., 64x64)
    pub frames_per_row: u32, // Frames in each row
    pub total_frames: u32,   // Total number of frames
}

impl SpriteSheet {
    /// Creates a new sprite sheet definition
    pub fn new(
        texture_width: f32,
        texture_height: f32,
        frame_width: f32,
        frame_height: f32,
    ) -> Self {
        // Validate dimensions
        assert!(
            texture_width > 0.0,
            "SpriteSheet: texture_width must be > 0"
        );
        assert!(
            texture_height > 0.0,
            "SpriteSheet: texture_height must be > 0"
        );
        assert!(frame_width > 0.0, "SpriteSheet: frame_width must be > 0");
        assert!(frame_height > 0.0, "SpriteSheet: frame_height must be > 0");
        assert!(
            frame_width <= texture_width,
            "SpriteSheet: frame_width ({}) cannot be larger than texture_width ({})",
            frame_width,
            texture_width
        );
        assert!(
            frame_height <= texture_height,
            "SpriteSheet: frame_height ({}) cannot be larger than texture_height ({})",
            frame_height,
            texture_height
        );

        let frames_per_row = (texture_width / frame_width) as u32;
        let frames_per_col = (texture_height / frame_height) as u32;

        let total_frames = frames_per_row * frames_per_col;

        if total_frames == 0 {
            eprintln!("Warning: SpriteSheet has 0 frames! Check your dimensions:");
            eprintln!("    Texture: {}x{}", texture_width, texture_height);
            eprintln!("    Frame: {}x{}", frame_width, frame_height);
            eprintln!(
                "    Frames: {} per row, {} per col",
                frames_per_row, frames_per_col
            );
        }

        Self {
            texture_size: Vec2::new(texture_width, texture_height),
            frame_size: Vec2::new(frame_width, frame_height),
            frames_per_row,
            total_frames,
        }
    }

    /// Calculate UV coordinates for a specific frame
    pub fn get_frame_uvs(&self, frame_index: u32) -> [Vec2; 4] {
        // Handle edge case: no frames
        if self.total_frames == 0 {
            // Return full texture as fallback
            return [
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(1.0, 1.0),
                Vec2::new(0.0, 1.0),
            ];
        }

        let frame_index = frame_index.min(self.total_frames - 1);

        let col = frame_index % self.frames_per_row;
        let row = frame_index / self.frames_per_row;

        // Calculate UV coordinates (0.0 to 1.0)
        let u_start = (col as f32 * self.frame_size.x) / self.texture_size.x;
        let v_start = (row as f32 * self.frame_size.y) / self.texture_size.y;
        let u_end = u_start + (self.frame_size.x / self.texture_size.x);
        let v_end = v_start + (self.frame_size.y / self.texture_size.y);

        // Return UV coords for quad vertices [TL, TR, BL, BR]
        [
            Vec2::new(u_start, v_start), // Top-left
            Vec2::new(u_end, v_start),   // Top-right
            Vec2::new(u_start, v_end),   // Bottom-left
            Vec2::new(u_end, v_end),     // Bottom-right
        ]
    }

    /// Get frame index from row and column
    pub fn frame_at(&self, row: u32, col: u32) -> u32 {
        (row * self.frames_per_row + col).min(self.total_frames - 1)
    }
}
