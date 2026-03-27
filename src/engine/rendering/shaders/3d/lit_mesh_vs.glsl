#version 450

// Input vertex attributes
layout(location = 0) in vec3 position;
layout(location = 1) in vec3 normal;
layout(location = 2) in vec2 uv;
layout(location = 3) in vec4 tangent;
layout(location = 4) in uvec4 joint_indices;
layout(location = 5) in vec4 joint_weights;

// Output to fragment shader
layout(location = 0) out vec3 frag_position;  // World position
layout(location = 1) out vec3 frag_normal;    // World normal
layout(location = 2) out vec2 frag_uv;        // Texture coords

// Push constants (per-draw)
layout(push_constant) uniform PushConstants {
    mat4 model;
    mat4 view_projection;
} pc;

void main() {
    // Transform position to world space
    vec4 world_pos = pc.model * vec4(position, 1.0);
    frag_position = world_pos.xyz;

    // Transform normal to world space
    mat3 normal_matrix = mat3(pc.model);
    frag_normal = normalize(normal_matrix * normal);

    // Pass through UV coords
    frag_uv = uv;

    // Transform to clip space
    gl_Position = pc.view_projection * world_pos;
}
