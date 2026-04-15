#version 460

layout(location = 0) in vec2 frag_uv;

layout(set = 0, binding = 0) uniform sampler2D source_texture;

layout(push_constant) uniform UpsampleParams {
    vec2 texel_size;
    float _pad0;
    float _pad1;
} params;

layout(location = 0) out vec4 out_color;

void main() {
    // 9-tap tent filter (3x3 bilinear taps)
    vec3 color = vec3(0.0);
    color += texture(source_texture, frag_uv + vec2(-1.0, -1.0) * params.texel_size).rgb * (1.0 / 16.0);
    color += texture(source_texture, frag_uv + vec2( 0.0, -1.0) * params.texel_size).rgb * (2.0 / 16.0);
    color += texture(source_texture, frag_uv + vec2( 1.0, -1.0) * params.texel_size).rgb * (1.0 / 16.0);
    color += texture(source_texture, frag_uv + vec2(-1.0,  0.0) * params.texel_size).rgb * (2.0 / 16.0);
    color += texture(source_texture, frag_uv).rgb                                      * (4.0 / 16.0);
    color += texture(source_texture, frag_uv + vec2( 1.0,  0.0) * params.texel_size).rgb * (2.0 / 16.0);
    color += texture(source_texture, frag_uv + vec2(-1.0,  1.0) * params.texel_size).rgb * (1.0 / 16.0);
    color += texture(source_texture, frag_uv + vec2( 0.0,  1.0) * params.texel_size).rgb * (2.0 / 16.0);
    color += texture(source_texture, frag_uv + vec2( 1.0,  1.0) * params.texel_size).rgb * (1.0 / 16.0);

    out_color = vec4(color, 1.0);
}
