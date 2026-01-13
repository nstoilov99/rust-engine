#version 460

// Unreal-style infinite grid using a camera-centered ground plane quad
// Hardware depth testing handles occlusion (no manual depth sampling needed)

layout(push_constant) uniform PushConstants {
    mat4 view_proj;      // View-projection matrix
    vec4 camera_pos;     // xyz = camera position, w = grid_extent
    vec4 grid_params;    // x = grid_size1, y = grid_size2, z = fade_start, w = fade_end
} pc;

layout(location = 0) out vec3 world_pos;

// 4 vertices for a quad (triangle strip order)
const vec2 verts[4] = vec2[](
    vec2(-1.0, -1.0),
    vec2( 1.0, -1.0),
    vec2(-1.0,  1.0),
    vec2( 1.0,  1.0)
);

void main() {
    vec2 v = verts[gl_VertexIndex];
    float grid_extent = pc.camera_pos.w;

    // Large ground plane centered on camera (Y=0 in render space = Z=0 in game space)
    world_pos = vec3(
        pc.camera_pos.x + v.x * grid_extent,
        0.0,
        pc.camera_pos.z + v.y * grid_extent
    );

    gl_Position = pc.view_proj * vec4(world_pos, 1.0);
}
