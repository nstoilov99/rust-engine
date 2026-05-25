#version 460

layout(set = 0, binding = 1) uniform sampler2D particle_texture;
layout(set = 1, binding = 0) uniform sampler2D gbuffer_depth;

layout(push_constant) uniform RenderParams {
    mat4 view_projection;
    vec4 camera_right;
    vec4 camera_up;             // .w = soft_fade_distance
    vec4 camera_near_far_pad;   // .x = near, .y = far, .zw = 0
} params;

layout(location = 0) in vec4 frag_color;
layout(location = 1) in vec2 frag_uv;

layout(location = 0) out vec4 out_color;

float linearize_depth(float ndc_z, float near, float far) {
    // Vulkan forward-Z [0, 1] NDC -> view-space depth.
    // ndc_z = 0 -> near; ndc_z = 1 -> far.
    return (near * far) / (far - (far - near) * ndc_z);
}

void main() {
    vec4 tex = texture(particle_texture, frag_uv);
    vec4 color = tex * frag_color;

    vec2 screen_uv = gl_FragCoord.xy / vec2(textureSize(gbuffer_depth, 0));
    float scene_ndc_z = texture(gbuffer_depth, screen_uv).r;
    float particle_ndc_z = gl_FragCoord.z;

    // The plankton pass composites into HDR without a hardware depth attachment,
    // so manually reject particles hidden behind opaque G-buffer geometry.
    const float depth_epsilon = 0.00001;
    if (scene_ndc_z + depth_epsilon < particle_ndc_z) {
        discard;
    }

    float soft_fade_distance = params.camera_up.w;
    if (soft_fade_distance > 0.0) {
        float near = params.camera_near_far_pad.x;
        float far  = params.camera_near_far_pad.y;
        float linear_scene    = linearize_depth(scene_ndc_z, near, far);
        float linear_particle = linearize_depth(particle_ndc_z, near, far);
        float fade = smoothstep(0.0, soft_fade_distance, linear_scene - linear_particle);
        color.a *= fade;
    }

    // Additive blending uses src * ONE, so alpha must be applied before output.
    color.rgb *= color.a;
    if (color.a <= 0.001) {
        discard;
    }

    out_color = color;
}
