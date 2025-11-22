#version 450

// Input vertex attributes
layout(location = 0) in vec3 position;
layout(location = 1) in vec3 normal;
layout(location = 2) in vec2 uv;
layout(location = 3) in vec4 tangent;  // NEW: xyz=tangent, w=handedness

// Output to fragment shader
layout(location = 0) out vec3 frag_position;
layout(location = 1) out vec3 frag_normal;
layout(location = 2) out vec2 frag_uv;
layout(location = 3) out mat3 frag_TBN;  // Tangent-Bitangent-Normal matrix (takes locations 3, 4, 5)
layout(location = 6) out vec4 frag_pos_light_space;

// Push constants
layout(push_constant) uniform PushConstants {
    mat4 model;
    mat4 view_projection;
    mat4 light_vp;
} pc;

void main() {
    // Transform to world space
    vec4 world_pos = pc.model * vec4(position, 1.0);
    frag_position = world_pos.xyz;
    frag_pos_light_space = pc.light_vp * world_pos;

    // Transform normal and tangent to world space
    mat3 normal_matrix = mat3(pc.model);
    vec3 N = normalize(normal_matrix * normal);
    vec3 T = normalize(normal_matrix * tangent.xyz);

    // Re-orthogonalize tangent
    T = normalize(T - dot(T, N) * N);

    // Calculate bitangent
    vec3 B = cross(N, T) * tangent.w;

    // Build TBN matrix (tangent space → world space)
    frag_TBN = mat3(T, B, N);

    // Pass through
    frag_normal = N;
    frag_uv = uv;

    // Clip space
    gl_Position = pc.view_projection * world_pos;
}