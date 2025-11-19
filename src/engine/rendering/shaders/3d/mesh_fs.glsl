#version 450

// Inputs from vertex shader
layout(location = 0) in vec3 fragNormal;
layout(location = 1) in vec2 fragUV;
layout(location = 2) in vec3 fragWorldPos;

// Texture sampler
layout(set = 0, binding = 0) uniform sampler2D texSampler;

// Output color
layout(location = 0) out vec4 outColor;

void main() {
    // Simple textured output (no lighting yet)
    vec4 texColor = texture(texSampler, fragUV);

    // Debug: Use normal as color (remove after testing)
    // vec3 normalColor = normalize(fragNormal) * 0.5 + 0.5;
    // outColor = vec4(normalColor, 1.0);

    outColor = texColor;
}