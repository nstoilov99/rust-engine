#version 460

// Input vertex attributes
layout(location = 0) in vec3 position;
layout(location = 1) in vec3 normal;
layout(location = 2) in vec2 uv;
layout(location = 3) in vec4 tangent;
layout(location = 4) in uvec4 joint_indices;
layout(location = 5) in vec4 joint_weights;

// Bone palette SSBO ring region (LargeSsbo skinning backend). Flat-indexed
// by pc.palette_base; element 0 is the identity (static meshes use base 0).
layout(set = 0, binding = 0) readonly buffer BonePalette {
    mat4 bones[];
} palette;

// Per-pass constants (one UBO per fence-ring slot, written per frame)
layout(set = 0, binding = 1) uniform PassData {
    mat4 view_projection;
} pass_data;

// Push constants: per-draw model matrix + palette base index
layout(push_constant) uniform PushConstants {
    mat4 model;
    uint palette_base;
} pc;

// Output to fragment shader
layout(location = 0) out vec3 frag_world_pos;
layout(location = 1) out vec3 frag_world_normal;
layout(location = 2) out vec2 frag_uv;
layout(location = 3) out vec4 frag_world_tangent;

void main() {
    // Skinning: compute blended bone matrix
    mat4 skin_matrix =
        joint_weights.x * palette.bones[pc.palette_base + joint_indices.x] +
        joint_weights.y * palette.bones[pc.palette_base + joint_indices.y] +
        joint_weights.z * palette.bones[pc.palette_base + joint_indices.z] +
        joint_weights.w * palette.bones[pc.palette_base + joint_indices.w];

    vec4 skinned_pos = skin_matrix * vec4(position, 1.0);
    vec3 skinned_normal = mat3(skin_matrix) * normal;
    vec3 skinned_tangent = mat3(skin_matrix) * tangent.xyz;

    // Transform position to world space
    vec4 world_pos = pc.model * skinned_pos;
    frag_world_pos = world_pos.xyz;

    // Transform normal to world space (use normal matrix for non-uniform scaling)
    mat3 normal_matrix = transpose(inverse(mat3(pc.model)));
    frag_world_normal = normalize(normal_matrix * skinned_normal);
    frag_world_tangent = vec4(normalize(normal_matrix * skinned_tangent), tangent.w);

    // Pass through UV
    frag_uv = uv;

    // Final clip-space position
    gl_Position = pass_data.view_projection * world_pos;
}
