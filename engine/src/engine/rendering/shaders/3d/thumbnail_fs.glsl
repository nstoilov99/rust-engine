#version 450

layout(location = 0) in vec3 frag_normal;

layout(location = 0) out vec4 out_color;

void main() {
    vec3 normal = normalize(frag_normal);

    // Directional light from upper-right-front
    vec3 light_dir = normalize(vec3(0.5, 0.7, 0.5));
    float diffuse = max(dot(normal, light_dir), 0.0);

    // Fill light from opposite side (softer)
    vec3 fill_dir = normalize(vec3(-0.3, 0.2, -0.6));
    float fill = max(dot(normal, fill_dir), 0.0) * 0.3;

    // Neutral gray base color
    vec3 base_color = vec3(0.7, 0.7, 0.75);
    float ambient = 0.15;
    vec3 color = base_color * (ambient + diffuse * 0.75 + fill);

    out_color = vec4(color, 1.0);
}
