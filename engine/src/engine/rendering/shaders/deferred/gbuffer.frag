#version 460

// Input from vertex shader
layout(location = 0) in vec3 frag_world_pos;
layout(location = 1) in vec3 frag_world_normal;
layout(location = 2) in vec2 frag_uv;
layout(location = 3) in vec4 frag_world_tangent;

// G-Buffer outputs (4 render targets)
layout(location = 0) out vec4 out_position;   // RT0: World position
layout(location = 1) out vec4 out_normal;     // RT1: World normal
layout(location = 2) out vec4 out_albedo;     // RT2: Albedo + roughness
layout(location = 3) out vec4 out_material;   // RT3: Metallic + AO

void main() {
    // Use default material values (no texture sampling for now)
    vec3 albedo = vec3(0.8, 0.8, 0.8); // Light gray default albedo
    float roughness = 0.5;
    float metallic = 0.0;

    // Use vertex normal directly (no normal mapping for now)
    vec3 normal = normalize(frag_world_normal);

    // Write to G-Buffer
    out_position = vec4(frag_world_pos, 1.0);
    out_normal = vec4(normal, 1.0);
    out_albedo = vec4(albedo, roughness); // RGB=albedo, A=roughness
    out_material = vec4(metallic, 1.0, 0.0, 1.0); // R=metallic, G=AO (1.0 default)
}
