#version 460

layout(location = 0) in vec2 frag_uv;

layout(set = 0, binding = 0) uniform sampler2D hdr_input;

layout(push_constant) uniform BloomParams {
    float threshold;
    float _pad0;
    float _pad1;
    float _pad2;
} params;

layout(location = 0) out vec4 out_color;

void main() {
    vec3 color = texture(hdr_input, frag_uv).rgb;
    float brightness = dot(color, vec3(0.2126, 0.7152, 0.0722));

    float knee = params.threshold * 0.5;
    float soft = brightness - params.threshold + knee;
    soft = clamp(soft, 0.0, 2.0 * knee);
    soft = soft * soft / (4.0 * knee + 0.00001);
    float contribution = max(soft, brightness - params.threshold) / max(brightness, 0.00001);

    out_color = vec4(color * max(contribution, 0.0), 1.0);
}
