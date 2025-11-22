#version 450

// Input vertex attributes
layout(location = 0) in vec3 position;
layout(location = 1) in vec3 normal;
layout(location = 2) in vec2 uv;
layout(location = 3) in vec4 tangent;

// Push constants
layout(push_constant) uniform PushConstants {
    mat4 model;           // Object's model matrix
    mat4 light_vp;        // Light's view-projection matrix
} pc;

void main() {
    // Transform vertex to light space
    vec4 world_pos = pc.model * vec4(position, 1.0);
    gl_Position = pc.light_vp * world_pos;
}