#version 450

// Vertex inputs (from Vertex3D struct)
layout(location = 0) in vec3 position;
layout(location = 1) in vec3 normal;
layout(location = 2) in vec2 uv;
layout(location = 3) in vec4 tangent;
layout(location = 4) in uvec4 joint_indices;
layout(location = 5) in vec4 joint_weights;

// Push constants (per-draw data)
layout(push_constant) uniform PushConstants {
    mat4 model;             // Model matrix (local -> world)
    mat4 view_projection;   // Combined view + projection
} constants;

// Outputs to fragment shader
layout(location = 0) out vec3 fragNormal;  // Normal in world space
layout(location = 1) out vec2 fragUV;
layout(location = 2) out vec3 fragWorldPos; // Position in world space

void main() {
    // Transform position to world space
    vec4 worldPos = constants.model * vec4(position, 1.0);
    fragWorldPos = worldPos.xyz;

    // Transform normal to world space
    fragNormal = mat3(constants.model) * normal;

    // Transform to clip space for GPU
    gl_Position = constants.view_projection * worldPos;

    // Pass through UV
    fragUV = uv;
}
