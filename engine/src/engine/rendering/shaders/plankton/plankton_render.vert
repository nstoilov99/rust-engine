#version 460

struct Plankton {
    vec3 position; float lifetime;
    vec3 velocity; float age;
    vec4 color;
    float size; float seed; vec2 _reserved;
};

layout(std430, set = 0, binding = 0) readonly buffer Particles {
    Plankton particles[];
};

layout(push_constant) uniform RenderParams {
    mat4 view_projection;
    vec4 camera_right;          // .xyz = camera right vector, .w unused
    vec4 camera_up;             // .xyz = camera up vector, .w = soft_fade_distance
    vec4 camera_near_far_pad;   // .x = near, .y = far, .zw = 0
} params;

layout(location = 0) out vec4 frag_color;
layout(location = 1) out vec2 frag_uv;

void main() {
    uint particle_id = gl_InstanceIndex;
    uint vertex_id = gl_VertexIndex;

    Plankton p = particles[particle_id];

    // Dead-slot discard: push off-screen deterministically
    if (p.lifetime <= 0.0) {
        gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
        frag_color = vec4(0.0);
        frag_uv = vec2(0.0);
        return;
    }

    // Billboard quad corners (triangle strip: 0=BL, 1=BR, 2=TL, 3=TR)
    vec2 offsets[4] = vec2[4](
        vec2(-0.5, -0.5),
        vec2( 0.5, -0.5),
        vec2(-0.5,  0.5),
        vec2( 0.5,  0.5)
    );

    vec2 uvs[4] = vec2[4](
        vec2(0.0, 1.0),
        vec2(1.0, 1.0),
        vec2(0.0, 0.0),
        vec2(1.0, 0.0)
    );

    vec2 offset = offsets[vertex_id];
    vec3 world_pos = p.position
                   + params.camera_right.xyz * offset.x * p.size
                   + params.camera_up.xyz    * offset.y * p.size;

    gl_Position = params.view_projection * vec4(world_pos, 1.0);
    frag_color = p.color;
    frag_uv = uvs[vertex_id];
}
