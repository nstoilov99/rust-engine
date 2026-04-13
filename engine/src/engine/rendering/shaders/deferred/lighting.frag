#version 460

// Input from vertex shader
layout(location = 0) in vec2 frag_uv;

// G-Buffer inputs (sample from textures)
layout(set = 0, binding = 0) uniform sampler2D g_position;
layout(set = 0, binding = 1) uniform sampler2D g_normal;
layout(set = 0, binding = 2) uniform sampler2D g_albedo;
layout(set = 0, binding = 3) uniform sampler2D g_material;

// Light data via push constants (simpler than uniform buffer)
layout(push_constant) uniform LightData {
    vec3 camera_position;
    float _pad0;
    vec3 directional_light_dir;
    float _pad1;
    vec3 directional_light_color;
    float directional_light_intensity;
    vec3 ambient_color;
    float ambient_intensity;
} lights;

// Output color
layout(location = 0) out vec4 out_color;

void main() {
    // Sample G-Buffer
    vec3 world_pos = texture(g_position, frag_uv).rgb;
    vec3 normal = texture(g_normal, frag_uv).rgb;
    vec4 albedo_rough = texture(g_albedo, frag_uv);
    vec3 albedo = albedo_rough.rgb;
    float roughness = albedo_rough.a;
    vec4 material = texture(g_material, frag_uv);
    float metallic = material.r;
    float ao = material.g;

    // Use actual light data from push constants
    vec3 light_dir = normalize(-lights.directional_light_dir);

    // Simple diffuse lighting
    float ndotl = max(dot(normal, light_dir), 0.0);
    vec3 diffuse = albedo * ndotl * lights.directional_light_color * lights.directional_light_intensity;

    // Ambient
    vec3 ambient = albedo * lights.ambient_color * lights.ambient_intensity * ao;

    vec3 final_color = diffuse + ambient;

    out_color = vec4(final_color, 1.0);
}
