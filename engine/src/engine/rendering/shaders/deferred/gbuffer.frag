#version 460

layout(location = 0) in vec3 frag_world_pos;
layout(location = 1) in vec3 frag_world_normal;
layout(location = 2) in vec2 frag_uv;
layout(location = 3) in vec4 frag_world_tangent;

layout(set = 1, binding = 0) uniform sampler2D albedo_tex;
layout(set = 1, binding = 1) uniform sampler2D normal_tex;
layout(set = 1, binding = 2) uniform sampler2D metallic_roughness_tex;
layout(set = 1, binding = 3) uniform sampler2D ao_tex;
layout(set = 1, binding = 4) uniform MaterialParams {
    vec4  base_color_factor;
    float metallic_factor;
    float roughness_factor;
    vec2  _pad0;
    vec3  emissive_factor;
    float _pad1;
} material;

layout(location = 0) out vec4 out_position;
layout(location = 1) out vec4 out_normal;
layout(location = 2) out vec4 out_albedo;
layout(location = 3) out vec4 out_material;
layout(location = 4) out vec4 out_emissive;

void main() {
    vec4 albedo_s = texture(albedo_tex, frag_uv) * material.base_color_factor;
    vec3 mr_s    = texture(metallic_roughness_tex, frag_uv).rgb;
    float rough  = mr_s.g * material.roughness_factor;
    float metal  = mr_s.b * material.metallic_factor;
    float ao     = texture(ao_tex, frag_uv).r;

    // Build TBN from interpolated tangent/bitangent/normal, sample tangent-space normal.
    vec3 N  = normalize(frag_world_normal);
    vec3 T  = normalize(frag_world_tangent.xyz);
    vec3 B  = cross(N, T) * frag_world_tangent.w;
    mat3 TBN = mat3(T, B, N);
    vec3 n_tan  = texture(normal_tex, frag_uv).rgb * 2.0 - 1.0;
    vec3 world_n = normalize(TBN * n_tan);

    out_position = vec4(frag_world_pos, 1.0);
    out_normal   = vec4(world_n, 1.0);
    out_albedo   = vec4(albedo_s.rgb, rough);
    out_material = vec4(metal, ao, 0.0, 1.0);
    out_emissive = vec4(material.emissive_factor, 0.0);
}
