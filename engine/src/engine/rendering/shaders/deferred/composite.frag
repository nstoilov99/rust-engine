#version 460

layout(location = 0) in vec2 frag_uv;

layout(set = 0, binding = 0) uniform sampler2D hdr_input;
layout(set = 0, binding = 1) uniform sampler2D bloom_input;
layout(set = 0, binding = 2) uniform sampler2D luminance_input;

layout(push_constant) uniform CompositeData {
    float exposure;
    float bloom_intensity;
    float vignette_intensity;
    float tone_map_mode;
    float exposure_mode;
    float _pad0;
    float _pad1;
    float _pad2;
} params;

layout(location = 0) out vec4 out_color;

vec3 aces_film(vec3 x) {
    float a = 2.51;
    float b = 0.03;
    float c = 2.43;
    float d = 0.59;
    float e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), 0.0, 1.0);
}

void main() {
    vec3 color = texture(hdr_input, frag_uv).rgb;
    vec3 bloom = texture(bloom_input, frag_uv).rgb;
    color += bloom * params.bloom_intensity;

    float exposure = params.exposure;
    if (params.exposure_mode < 0.5) {
        float lum = texture(luminance_input, vec2(0.5)).r;
        float target_lum = 0.18;
        exposure = target_lum / max(lum, 0.001);
        exposure = clamp(exposure, 0.1, 10.0);
    }
    color *= exposure;

    if (params.tone_map_mode > 0.5) {
        color = aces_film(color);
    } else {
        color = color / (color + vec3(1.0));
    }

    vec2 uv_centered = frag_uv - 0.5;
    float dist = length(uv_centered);
    float vignette = smoothstep(0.7, 0.4, dist);
    color *= mix(1.0, vignette, params.vignette_intensity);

    out_color = vec4(color, 1.0);
}
