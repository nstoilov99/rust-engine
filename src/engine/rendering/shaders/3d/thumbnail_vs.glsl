#version 450

// Vertex inputs (Vertex3D layout)
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
layout(location = 0) out vec3 frag_normal;

void main() {
    vec4 world_pos = pc.model * vec4(position, 1.0);
    frag_normal = normalize(mat3(pc.model) * normal);
    gl_Position = pc.view_projection * world_pos;
}
