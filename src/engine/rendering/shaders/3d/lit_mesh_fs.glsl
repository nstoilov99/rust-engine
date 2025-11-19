#version 450

// Input from vertex shader
layout(location = 0) in vec3 frag_position;
layout(location = 1) in vec3 frag_normal;
layout(location = 2) in vec2 frag_uv;

// Output color
layout(location = 0) out vec4 out_color;

// Textures
layout(set = 0, binding = 0) uniform sampler2D albedo_texture;
// NOTE: Normal mapping requires tangent vectors - covered in Tutorial 12 PBR 

// Lighting uniforms (passed via descriptor set)
layout(set = 1, binding = 0) uniform LightingData {
    vec3 camera_position;           // For specular calculation
    float _padding1;

    vec3 ambient_color;             // Ambient light
    float ambient_intensity;

    vec3 directional_light_dir;     // Sun/moon direction
    float _padding2;

    vec3 directional_light_color;   // Sun/moon color
    float directional_light_intensity;

    // Material properties
    float metallic;
    float roughness;
    float _padding3;
    float _padding4;
} lighting;

// Calculate diffuse lighting (Lambertian)
float calculate_diffuse(vec3 normal, vec3 light_dir) {
    // Dot product of normal and light direction
    // 1.0 = surface facing light directly
    // 0.0 = surface perpendicular to light
    // Negative = surface facing away (clamp to 0)
    return max(dot(normal, light_dir), 0.0);
}

// Calculate specular lighting (Blinn-Phong)
float calculate_specular(vec3 normal, vec3 light_dir, vec3 view_dir, float shininess) {
    // Halfway vector between light and view
    vec3 halfway = normalize(light_dir + view_dir);

    // Specular intensity
    float spec = pow(max(dot(normal, halfway), 0.0), shininess);
    return spec;
}

void main() {
    // Sample albedo (base color) from texture
    vec4 albedo = texture(albedo_texture, frag_uv);

    // Use geometry normal (normal mapping with tangent space in Tutorial 12)
    vec3 normal = normalize(frag_normal);

    // View direction (from surface to camera)
    vec3 view_dir = normalize(lighting.camera_position - frag_position);

    // === AMBIENT LIGHT ===
    vec3 ambient = lighting.ambient_color * lighting.ambient_intensity;

    // === DIRECTIONAL LIGHT (Sun) ===
    vec3 light_dir = normalize(-lighting.directional_light_dir); // Negate because we want direction TO light

    // Diffuse
    float diffuse_strength = calculate_diffuse(normal, light_dir);
    vec3 diffuse = lighting.directional_light_color * diffuse_strength * lighting.directional_light_intensity;

    // Specular (shininess based on roughness)
    float shininess = mix(256.0, 8.0, lighting.roughness); // Smooth = 256, rough = 8
    float specular_strength = calculate_specular(normal, light_dir, view_dir, shininess);
    vec3 specular = lighting.directional_light_color * specular_strength * lighting.directional_light_intensity;

    // Metals have colored specular, non-metals have white specular
    specular *= mix(vec3(1.0), albedo.rgb, lighting.metallic);

    // === COMBINE LIGHTING ===
    vec3 final_color = (ambient + diffuse) * albedo.rgb + specular;

    // Output with alpha
    out_color = vec4(final_color, albedo.a);
}