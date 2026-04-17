#version 460

layout(location = 0) in vec2 frag_uv;

layout(set = 0, binding = 0) uniform sampler2D g_position;
layout(set = 0, binding = 1) uniform sampler2D g_normal;
layout(set = 0, binding = 2) uniform sampler2D g_albedo;
layout(set = 0, binding = 3) uniform sampler2D g_material;

layout(set = 1, binding = 0) uniform sampler2DShadow shadow_map;

layout(set = 2, binding = 0) uniform sampler2D ssao_map;

layout(push_constant) uniform LightData {
    vec3 camera_position;
    float shadow_bias;
    vec3 directional_light_dir;
    float shadow_enabled;
    vec3 directional_light_color;
    float directional_light_intensity;
    vec3 ambient_color;
    float ambient_intensity;
    mat4 light_vp;
} lights;

layout(location = 0) out vec4 out_color;

const float PI = 3.14159265359;

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

float geometry_schlick_GGX(float NdotV, float roughness) {
    float r = (roughness + 1.0);
    float k = (r * r) / 8.0;

    float num = NdotV;
    float denom = NdotV * (1.0 - k) + k;

    return num / denom;
}

float geometry_smith(vec3 N, vec3 V, vec3 L, float roughness) {
    float NdotV = max(dot(N, V), 0.0);
    float NdotL = max(dot(N, L), 0.0);
    float ggx2 = geometry_schlick_GGX(NdotV, roughness);
    float ggx1 = geometry_schlick_GGX(NdotL, roughness);

    return ggx1 * ggx2;
}

vec3 fresnel_schlick(float cos_theta, vec3 F0) {
    return F0 + (1.0 - F0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

float calculate_shadow(vec4 frag_pos_light_space, vec3 normal, vec3 light_dir) {
    vec3 proj_coords = frag_pos_light_space.xyz / frag_pos_light_space.w;
    // glam's orthographic_rh produces NDC.z in [0, 1] (Vulkan convention),
    // so only XY need the [-1,1] -> [0,1] remap. Z is already a depth value.
    proj_coords.xy = proj_coords.xy * 0.5 + 0.5;

    if (proj_coords.z > 1.0 || proj_coords.z < 0.0 ||
        proj_coords.x < 0.0 || proj_coords.x > 1.0 ||
        proj_coords.y < 0.0 || proj_coords.y > 1.0) {
        return 1.0;
    }

    float bias = max(lights.shadow_bias * (1.0 - dot(normal, light_dir)), lights.shadow_bias * 0.1);
    float current_depth = proj_coords.z - bias;

    // 5x5 PCF with a 1.5-texel stride. The hardware shadow sampler already
    // does a 2x2 bilinear compare per tap, so effective filter footprint is
    // ~8 texels — soft enough to read as a real shadow without looking blurry.
    float shadow = 0.0;
    vec2 texel_size = 1.0 / textureSize(shadow_map, 0);
    for(int x = -2; x <= 2; ++x) {
        for(int y = -2; y <= 2; ++y) {
            vec2 offset = vec2(x, y) * texel_size * 1.5;
            shadow += texture(shadow_map, vec3(proj_coords.xy + offset, current_depth));
        }
    }
    shadow /= 25.0;
    return shadow;
}

void main() {
    vec3 world_pos = texture(g_position, frag_uv).rgb;
    vec3 normal = texture(g_normal, frag_uv).rgb;
    vec4 albedo_rough = texture(g_albedo, frag_uv);
    vec3 albedo = albedo_rough.rgb;
    float roughness = albedo_rough.a;
    vec4 material = texture(g_material, frag_uv);
    float metallic = material.r;
    float ao = material.g;

    vec3 N = normalize(normal);
    vec3 V = normalize(lights.camera_position - world_pos);
    vec3 L = normalize(-lights.directional_light_dir);
    vec3 H = normalize(V + L);

    vec3 F0 = mix(vec3(0.04), albedo, metallic);
    vec3 radiance = lights.directional_light_color * lights.directional_light_intensity;

    float NDF = distribution_GGX(N, H, roughness);
    float G = geometry_smith(N, V, L, roughness);
    vec3 F = fresnel_schlick(max(dot(H, V), 0.0), F0);

    vec3 numerator = NDF * G * F;
    float NdotV = max(dot(N, V), 0.0);
    float NdotL = max(dot(N, L), 0.0);
    float denominator = 4.0 * NdotV * NdotL + 0.0001;
    vec3 specular = numerator / denominator;

    vec3 kS = F;
    vec3 kD = (vec3(1.0) - kS) * (1.0 - metallic);

    vec3 Lo = (kD * albedo / PI + specular) * radiance * NdotL;

    vec4 frag_pos_light_space = lights.light_vp * vec4(world_pos, 1.0);
    float shadow = (lights.shadow_enabled > 0.5)
        ? calculate_shadow(frag_pos_light_space, N, L)
        : 1.0;

    float ssao = texture(ssao_map, frag_uv).r;
    vec3 ambient = lights.ambient_color * lights.ambient_intensity * albedo * ao * ssao;

    vec3 color = ambient + shadow * Lo;

    out_color = vec4(color, 1.0);
}
