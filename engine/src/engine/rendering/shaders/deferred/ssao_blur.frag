#version 460

layout(location = 0) in vec2 frag_uv;

layout(set = 0, binding = 0) uniform sampler2D ssao_input;

layout(location = 0) out float out_occlusion;

void main() {
    vec2 texel_size = 1.0 / textureSize(ssao_input, 0);
    float result = 0.0;
    for (int x = -2; x <= 2; ++x) {
        for (int y = -2; y <= 2; ++y) {
            vec2 offset = vec2(float(x), float(y)) * texel_size;
            result += texture(ssao_input, frag_uv + offset).r;
        }
    }
    out_occlusion = result / 25.0;
}
