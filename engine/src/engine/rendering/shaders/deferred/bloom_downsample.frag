#version 460

layout(location = 0) in vec2 frag_uv;

layout(set = 0, binding = 0) uniform sampler2D source_texture;

layout(push_constant) uniform DownsampleParams {
    vec2 texel_size;
    float is_first_pass;
    float _pad;
} params;

layout(location = 0) out vec4 out_color;

void main() {
    // 13-tap downsample (CoD:AW SIGGRAPH 2014)
    vec3 a = texture(source_texture, frag_uv + vec2(-2.0, -2.0) * params.texel_size).rgb;
    vec3 b = texture(source_texture, frag_uv + vec2( 0.0, -2.0) * params.texel_size).rgb;
    vec3 c = texture(source_texture, frag_uv + vec2( 2.0, -2.0) * params.texel_size).rgb;
    vec3 d = texture(source_texture, frag_uv + vec2(-2.0,  0.0) * params.texel_size).rgb;
    vec3 e = texture(source_texture, frag_uv).rgb;
    vec3 f = texture(source_texture, frag_uv + vec2( 2.0,  0.0) * params.texel_size).rgb;
    vec3 g = texture(source_texture, frag_uv + vec2(-2.0,  2.0) * params.texel_size).rgb;
    vec3 h = texture(source_texture, frag_uv + vec2( 0.0,  2.0) * params.texel_size).rgb;
    vec3 i = texture(source_texture, frag_uv + vec2( 2.0,  2.0) * params.texel_size).rgb;
    vec3 j = texture(source_texture, frag_uv + vec2(-1.0, -1.0) * params.texel_size).rgb;
    vec3 k = texture(source_texture, frag_uv + vec2( 1.0, -1.0) * params.texel_size).rgb;
    vec3 l = texture(source_texture, frag_uv + vec2(-1.0,  1.0) * params.texel_size).rgb;
    vec3 m = texture(source_texture, frag_uv + vec2( 1.0,  1.0) * params.texel_size).rgb;

    vec3 color;
    if (params.is_first_pass > 0.5) {
        // Karis average on first pass to reduce fireflies
        vec3 g0 = (a + b + d + e) * 0.25;
        vec3 g1 = (b + c + e + f) * 0.25;
        vec3 g2 = (d + e + g + h) * 0.25;
        vec3 g3 = (e + f + h + i) * 0.25;
        vec3 g4 = (j + k + l + m) * 0.25;

        float w0 = 1.0 / (1.0 + dot(g0, vec3(0.2126, 0.7152, 0.0722)));
        float w1 = 1.0 / (1.0 + dot(g1, vec3(0.2126, 0.7152, 0.0722)));
        float w2 = 1.0 / (1.0 + dot(g2, vec3(0.2126, 0.7152, 0.0722)));
        float w3 = 1.0 / (1.0 + dot(g3, vec3(0.2126, 0.7152, 0.0722)));
        float w4 = 1.0 / (1.0 + dot(g4, vec3(0.2126, 0.7152, 0.0722)));

        color = (g0 * w0 + g1 * w1 + g2 * w2 + g3 * w3 + g4 * w4) /
                (w0 + w1 + w2 + w3 + w4);
    } else {
        // Standard 13-tap box filter
        color = e * 0.125;
        color += (a + c + g + i) * 0.03125;
        color += (b + d + f + h) * 0.0625;
        color += (j + k + l + m) * 0.125;
    }

    out_color = vec4(color, 1.0);
}
