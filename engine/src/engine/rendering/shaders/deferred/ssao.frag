#version 460

layout(location = 0) in vec2 frag_uv;

layout(set = 0, binding = 0) uniform sampler2D g_position;
layout(set = 0, binding = 1) uniform sampler2D g_normal;
layout(set = 0, binding = 2) uniform sampler2D noise_texture;

layout(set = 1, binding = 0) uniform SsaoKernel {
    vec4 samples[64];
} kernel;

layout(push_constant) uniform SsaoParams {
    mat4 view_projection;
    vec2 screen_size;
    float radius;
    float bias;
} params;

layout(location = 0) out float out_occlusion;

void main() {
    vec3 world_pos = texture(g_position, frag_uv).rgb;
    vec3 world_normal = normalize(texture(g_normal, frag_uv).rgb);

    vec2 noise_scale = params.screen_size / 4.0;
    vec3 random_vec = vec3(texture(noise_texture, frag_uv * noise_scale).rg, 0.0);

    // Construct TBN from world normal + noise rotation
    vec3 tangent = normalize(random_vec - world_normal * dot(random_vec, world_normal));
    vec3 bitangent = cross(world_normal, tangent);
    mat3 TBN = mat3(tangent, bitangent, world_normal);

    float occlusion = 0.0;
    for (int i = 0; i < 64; ++i) {
        vec3 sample_offset = TBN * kernel.samples[i].xyz;
        vec3 sample_pos = world_pos + sample_offset * params.radius;

        // Project to screen space
        vec4 offset = params.view_projection * vec4(sample_pos, 1.0);
        offset.xyz /= offset.w;
        offset.xy = offset.xy * 0.5 + 0.5;

        // Read world position at sample screen coordinate
        vec3 sampled_pos = texture(g_position, offset.xy).rgb;

        // Compare depths in world space (distance from camera implied by projection)
        vec4 sample_clip = params.view_projection * vec4(sampled_pos, 1.0);
        float sample_depth = sample_clip.z / sample_clip.w;
        float frag_depth = offset.z;

        float range_check = smoothstep(0.0, 1.0, params.radius / max(abs(frag_depth - sample_depth), 0.001));
        occlusion += (sample_depth < frag_depth - params.bias ? 1.0 : 0.0) * range_check;
    }

    occlusion = 1.0 - (occlusion / 64.0);
    out_occlusion = occlusion;
}
