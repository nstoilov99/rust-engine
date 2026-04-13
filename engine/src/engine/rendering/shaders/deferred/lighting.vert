#version 460

// Fullscreen triangle (no vertex buffer needed)
vec2 positions[3] = vec2[](
    vec2(-1.0, -1.0),
    vec2(3.0, -1.0),
    vec2(-1.0, 3.0)
);

layout(location = 0) out vec2 frag_uv;

void main() {
    gl_Position = vec4(positions[gl_VertexIndex], 0.0, 1.0);
    frag_uv = positions[gl_VertexIndex] * 0.5 + 0.5;
}
