#version 450

// Vertex inputs
layout(location = 0) in vec2 position;
layout(location = 1) in vec2 uv;

// Push constants (transform data)
layout(push_constant) uniform PushConstants {
    vec2 pos;       // Position
    float rotation; // Rotation (radians)
    vec2 scale;     // Scale
} transform;

// Output to fragment shader
layout(location = 0) out vec2 fragUV;

void main() {
    // Apply scale
    vec2 scaled = position * transform.scale;

    // Apply rotation
    float c = cos(transform.rotation);
    float s = sin(transform.rotation);
    vec2 rotated = vec2(
        scaled.x * c - scaled.y * s,
        scaled.x * s + scaled.y * c
    );

    // Apply position
    vec2 final_pos = rotated + transform.pos;

    gl_Position = vec4(final_pos, 0.0, 1.0);
    fragUV = uv;
}