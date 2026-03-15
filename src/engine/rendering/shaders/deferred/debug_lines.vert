#version 460

layout(push_constant) uniform PushConstants {
    mat4 view_proj;
} pc;

layout(location = 0) in vec3 position;
layout(location = 1) in vec4 color;

layout(location = 0) out vec4 v_color;

void main() {
    v_color = color;
    gl_Position = pc.view_proj * vec4(position, 1.0);
}
