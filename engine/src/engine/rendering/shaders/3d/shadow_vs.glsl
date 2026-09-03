#version 450

// Input vertex attributes
layout(location = 0) in vec3 position;
layout(location = 1) in vec3 normal;
layout(location = 2) in vec2 uv;
layout(location = 3) in vec4 tangent;
layout(location = 4) in uvec4 joint_indices;
layout(location = 5) in vec4 joint_weights;

// Bone palette SSBO ring region (LargeSsbo skinning backend). Flat-indexed
// by the instance's palette_base; element 0 is the identity (static meshes
// use base 0).
layout(set = 0, binding = 0) readonly buffer BonePalette {
    mat4 bones[];
} palette;

// Per-pass constants: the light's view-projection matrix
layout(set = 0, binding = 1) uniform PassData {
    mat4 view_projection;
} pass_data;

// Per-instance metadata SSBO ring region (instanced draws, Task 41.5 P7).
// 80 B std430 stride — must match `skinning::InstanceData` and gbuffer.vert.
struct InstanceData {
    mat4 model;
    uint palette_base;
    uint _pad0;
    uint _pad1;
    uint _pad2;
};
layout(set = 0, binding = 2) readonly buffer Instances {
    InstanceData data[];
} instances;

void main() {
    mat4 model = instances.data[gl_InstanceIndex].model;
    uint palette_base = instances.data[gl_InstanceIndex].palette_base;

    // Skinning: compute blended bone matrix
    mat4 skin_matrix =
        joint_weights.x * palette.bones[palette_base + joint_indices.x] +
        joint_weights.y * palette.bones[palette_base + joint_indices.y] +
        joint_weights.z * palette.bones[palette_base + joint_indices.z] +
        joint_weights.w * palette.bones[palette_base + joint_indices.w];

    vec4 skinned_pos = skin_matrix * vec4(position, 1.0);

    // Transform vertex to light space
    vec4 world_pos = model * skinned_pos;
    gl_Position = pass_data.view_projection * world_pos;
}
