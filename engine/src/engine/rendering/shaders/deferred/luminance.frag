#version 460

layout(location = 0) in vec2 frag_uv;

layout(set = 0, binding = 0) uniform sampler2D src_texture;

layout(push_constant) uniform LumParams {
    float is_first_pass;
    float _pad0;
    float _pad1;
    float _pad2;
} params;

layout(location = 0) out float out_luminance;

void main() {
    vec2 texel_size = 1.0 / textureSize(src_texture, 0);

    vec4 s0 = texture(src_texture, frag_uv + texel_size * vec2(-0.5, -0.5));
    vec4 s1 = texture(src_texture, frag_uv + texel_size * vec2( 0.5, -0.5));
    vec4 s2 = texture(src_texture, frag_uv + texel_size * vec2(-0.5,  0.5));
    vec4 s3 = texture(src_texture, frag_uv + texel_size * vec2( 0.5,  0.5));

    if (params.is_first_pass > 0.5) {
        vec3 luma_weights = vec3(0.2126, 0.7152, 0.0722);
        float l0 = dot(s0.rgb, luma_weights);
        float l1 = dot(s1.rgb, luma_weights);
        float l2 = dot(s2.rgb, luma_weights);
        float l3 = dot(s3.rgb, luma_weights);
        out_luminance = (l0 + l1 + l2 + l3) * 0.25;
    } else {
        out_luminance = (s0.r + s1.r + s2.r + s3.r) * 0.25;
    }
}
