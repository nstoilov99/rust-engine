#version 450

// Input vertex attributes
layout(location = 0) in vec3 position;
layout(location = 1) in vec3 normal;
layout(location = 2) in vec2 uv;
layout(location = 3) in vec4 tangent;
layout(location = 4) in uvec4 joint_indices;
layout(location = 5) in vec4 joint_weights;

// Bone palette UBO (FixedUbo skinning backend)
layout(set = 0, binding = 0) uniform BonePalette {
    mat4 bones[256];
    uint bone_count;
} palette;

// Push constants
layout(push_constant) uniform PushConstants {
    mat4 model;           // Object's model matrix
    mat4 light_vp;        // Light's view-projection matrix
} pc;

void main() {
    // Skinning: compute blended bone matrix
    mat4 skin_matrix =
        joint_weights.x * palette.bones[joint_indices.x] +
        joint_weights.y * palette.bones[joint_indices.y] +
        joint_weights.z * palette.bones[joint_indices.z] +
        joint_weights.w * palette.bones[joint_indices.w];

    vec4 skinned_pos = skin_matrix * vec4(position, 1.0);

    // Transform vertex to light space
    vec4 world_pos = pc.model * skinned_pos;
    gl_Position = pc.light_vp * world_pos;
}
