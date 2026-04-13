#version 450

// Vertex inputs
layout(location = 0) in vec2 position;
layout(location = 1) in vec2 uv;  // We have UVs but will override them

// Push constants (per-sprite data)
layout(push_constant) uniform PushConstants {
    mat4 view_projection;  // Camera matrix
    vec2 pos;              // Sprite position
    float rotation;        // Sprite rotation
    vec2 scale;            // Sprite scale
    vec4 uv_rect;          // UV coordinates (u_min, v_min, u_max, v_max) - NEW!
} constants;

// Output to fragment shader
layout(location = 0) out vec2 fragUV;

void main() {
    // Apply sprite transform (scale, rotate, translate)
    vec2 scaled = position * constants.scale;

    float c = cos(constants.rotation);
    float s = sin(constants.rotation);
    vec2 rotated = vec2(
        scaled.x * c - scaled.y * s,
        scaled.x * s + scaled.y * c
    );

    vec2 world_pos = rotated + constants.pos;

    // Apply camera view-projection
    gl_Position = constants.view_projection * vec4(world_pos, 0.0, 1.0);

    // Calculate UV from position and uv_rect
    // If uv_rect is (0,0,0,0), use default UVs from vertex
    // Otherwise, map position to sprite sheet UV rectangle
    if (constants.uv_rect == vec4(0.0, 0.0, 0.0, 0.0)) {
        // Default: use full texture (for non-animated sprites)
        fragUV = uv;
    } else {
        // Animation: map position to UV rectangle
        // position ranges from -0.5 to 0.5, convert to 0-1
        vec2 uv_local = position + 0.5;

        // Map to sprite sheet UV rectangle
        fragUV = mix(
            constants.uv_rect.xy,  // (u_min, v_min)
            constants.uv_rect.zw,  // (u_max, v_max)
            uv_local
        );
    }
}