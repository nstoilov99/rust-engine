#version 450

// Vertex inputs (Vertex3D layout)
layout(location = 0) in vec3 position;
layout(location = 1) in vec3 normal;
layout(location = 2) in vec2 uv;
layout(location = 3) in vec4 tangent;
layout(location = 4) in uvec4 joint_indices;
layout(location = 5) in vec4 joint_weights;

// Bone palette SSBO (preview paths bind a fresh one-off buffer per draw;
// element 0 is the identity when the mesh is static).
layout(set = 0, binding = 0) readonly buffer BonePalette {
    mat4 bones[];
} palette;

// Per-pass constants (view-projection for this preview render)
layout(set = 0, binding = 1) uniform PassData {
    mat4 view_projection;
} pass_data;

// Push constants: per-draw model matrix + palette base index
layout(push_constant) uniform PushConstants {
    mat4 model;
    uint palette_base;
} pc;

// Output to fragment shader
layout(location = 0) out vec3 frag_normal;

void main() {
    // Skinning: compute blended bone matrix
    mat4 skin_matrix =
        joint_weights.x * palette.bones[pc.palette_base + joint_indices.x] +
        joint_weights.y * palette.bones[pc.palette_base + joint_indices.y] +
        joint_weights.z * palette.bones[pc.palette_base + joint_indices.z] +
        joint_weights.w * palette.bones[pc.palette_base + joint_indices.w];

    vec4 skinned_pos = skin_matrix * vec4(position, 1.0);
    vec3 skinned_normal = mat3(skin_matrix) * normal;

    vec4 world_pos = pc.model * skinned_pos;
    frag_normal = normalize(mat3(pc.model) * skinned_normal);
    gl_Position = pass_data.view_projection * world_pos;
}
