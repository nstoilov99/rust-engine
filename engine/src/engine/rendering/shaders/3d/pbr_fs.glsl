#version 450

// Inputs from vertex shader
layout(location = 0) in vec3 frag_position;
layout(location = 1) in vec3 frag_normal;
layout(location = 2) in vec2 frag_uv;
layout(location = 3) in mat3 frag_TBN;  // Takes locations 3, 4, 5
layout(location = 6) in vec4 frag_pos_light_space;

// Output
layout(location = 0) out vec4 out_color;

// Texture maps
layout(set = 0, binding = 0) uniform sampler2D albedo_map;
layout(set = 0, binding = 1) uniform sampler2D normal_map;
layout(set = 0, binding = 2) uniform sampler2D metallic_roughness_map;  // R=unused, G=roughness, B=metallic
layout(set = 0, binding = 3) uniform sampler2D ao_map;  // Ambient occlusion

// Lighting data
layout(set = 1, binding = 0) uniform LightingData {
    vec3 camera_position;
    float _padding1;

    vec3 ambient_color;
    float ambient_intensity;

    vec3 directional_light_dir;
    float _padding2;

    vec3 directional_light_color;
    float directional_light_intensity;
} lighting;

layout(set = 2, binding = 0) uniform sampler2DShadow shadow_map;

const float PI = 3.14159265359;

// === PBR FUNCTIONS ===

// Normal Distribution Function (GGX/Trowbridge-Reitz)
float distribution_GGX(vec3 N, vec3 H, float roughness) {
    float a = roughness * roughness;
    float a2 = a * a;
    float NdotH = max(dot(N, H), 0.0);
    float NdotH2 = NdotH * NdotH;

    float num = a2;
    float denom = (NdotH2 * (a2 - 1.0) + 1.0);
    denom = PI * denom * denom;

    return num / denom;
}

// Geometry Function (Schlick-GGX)
float geometry_schlick_GGX(float NdotV, float roughness) {
    float r = (roughness + 1.0);
    float k = (r * r) / 8.0;

    float num = NdotV;
    float denom = NdotV * (1.0 - k) + k;

    return num / denom;
}

// Smith's method (combines view and light directions)
float geometry_smith(vec3 N, vec3 V, vec3 L, float roughness) {
    float NdotV = max(dot(N, V), 0.0);
    float NdotL = max(dot(N, L), 0.0);
    float ggx2 = geometry_schlick_GGX(NdotV, roughness);
    float ggx1 = geometry_schlick_GGX(NdotL, roughness);

    return ggx1 * ggx2;
}

// Fresnel-Schlick approximation
vec3 fresnel_schlick(float cos_theta, vec3 F0) {
    return F0 + (1.0 - F0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

// Calculates shadow factor: 0.0 = fully shadowed, 1.0 = fully lit
float calculate_shadow(vec4 frag_pos_light_space, vec3 normal, vec3 light_dir) {
    // Perspective divide
    vec3 proj_coords = frag_pos_light_space.xyz / frag_pos_light_space.w;

    // Transform to [0,1] range
    proj_coords = proj_coords * 0.5 + 0.5;

    // Outside shadow map = fully lit
    if (proj_coords.z > 1.0 || proj_coords.x < 0.0 || proj_coords.x > 1.0 ||
        proj_coords.y < 0.0 || proj_coords.y > 1.0) {
        return 1.0;
    }

    // Bias prevents shadow acne
    float bias = max(0.05 * (1.0 - dot(normal, light_dir)), 0.005);
    float current_depth = proj_coords.z - bias;

    // PCF (Percentage-Closer Filtering) for soft shadows
    // Sample 3x3 kernel around fragment
    float shadow = 0.0;
    vec2 texel_size = 1.0 / textureSize(shadow_map, 0);

    for(int x = -1; x <= 1; ++x) {
        for(int y = -1; y <= 1; ++y) {
            vec2 offset = vec2(x, y) * texel_size;
            shadow += texture(shadow_map, vec3(proj_coords.xy + offset, current_depth));
        }
    }
    shadow /= 9.0; // Average 9 samples

    return shadow;
}

void main() {
    // Sample material properties
    vec3 albedo = pow(texture(albedo_map, frag_uv).rgb, vec3(2.2)); // sRGB → linear
    vec3 normal_sample = texture(normal_map, frag_uv).rgb;
    vec2 metallic_roughness = texture(metallic_roughness_map, frag_uv).bg; // B=metallic, G=roughness
    float ao = texture(ao_map, frag_uv).r;

    float metallic = metallic_roughness.x;
    float roughness = metallic_roughness.y;

    // Transform normal from tangent space to world space
    vec3 normal = normal_sample * 2.0 - 1.0;  // [0,1] → [-1,1]
    normal = normalize(frag_TBN * normal);

    // Calculate vectors
    vec3 N = normal;
    vec3 V = normalize(lighting.camera_position - frag_position);

    // Base reflectivity (F0)
    // Non-metals: ~0.04 (4% reflection)
    // Metals: use albedo as F0
    vec3 F0 = vec3(0.04);
    F0 = mix(F0, albedo, metallic);

    // === LIGHTING CALCULATION ===

    vec3 Lo = vec3(0.0);  // Outgoing radiance

    // Directional light
    vec3 L = normalize(-lighting.directional_light_dir);
    vec3 H = normalize(V + L);
    vec3 radiance = lighting.directional_light_color * lighting.directional_light_intensity;

    // Cook-Torrance BRDF
    float NDF = distribution_GGX(N, H, roughness);
    float G = geometry_smith(N, V, L, roughness);
    vec3 F = fresnel_schlick(max(dot(H, V), 0.0), F0);

    // Specular
    vec3 numerator = NDF * G * F;
    float denominator = 4.0 * max(dot(N, V), 0.0) * max(dot(N, L), 0.0) + 0.0001; // Prevent divide by zero
    vec3 specular = numerator / denominator;

    // Diffuse (energy conservation)
    vec3 kS = F;  // Specular contribution
    vec3 kD = vec3(1.0) - kS;  // Diffuse contribution
    kD *= 1.0 - metallic;  // Metals have no diffuse

    float NdotL = max(dot(N, L), 0.0);
    Lo += (kD * albedo / PI + specular) * radiance * NdotL;

    // Ambient (simple approximation, IBL would be better)
    vec3 ambient = lighting.ambient_color * lighting.ambient_intensity * albedo * ao;

    // Calculate shadow
    float shadow = calculate_shadow(frag_pos_light_space, N, L);

    // Apply shadow to direct lighting (not ambient)
    vec3 color = ambient + shadow * Lo;

    // HDR tonemapping (Reinhard)
    color = color / (color + vec3(1.0));

    // Gamma correction (linear → sRGB)
    color = pow(color, vec3(1.0/2.2));

    out_color = vec4(color, 1.0);
}