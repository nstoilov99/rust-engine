#version 460

// Input vertex attributes
layout(location = 0) in vec3 position;
layout(location = 1) in vec3 normal;
layout(location = 2) in vec2 uv;
layout(location = 3) in vec4 tangent;

// Push constants (model + view-projection matrices)
layout(push_constant) uniform PushConstants {
    mat4 model;
    mat4 view_projection;
} pc;

// Output to fragment shader
layout(location = 0) out vec3 frag_world_pos;
layout(location = 1) out vec3 frag_world_normal;
layout(location = 2) out vec2 frag_uv;
layout(location = 3) out vec4 frag_world_tangent;

void main() {
    // Transform position to world space
    vec4 world_pos = pc.model * vec4(position, 1.0);
    frag_world_pos = world_pos.xyz;

    // Transform normal to world space (use normal matrix for non-uniform scaling)
    mat3 normal_matrix = transpose(inverse(mat3(pc.model)));
    frag_world_normal = normalize(normal_matrix * normal);
    frag_world_tangent = vec4(normalize(normal_matrix * tangent.xyz), tangent.w);

    // Pass through UV
    frag_uv = uv;

    // Final clip-space position
    gl_Position = pc.view_projection * world_pos;
}
