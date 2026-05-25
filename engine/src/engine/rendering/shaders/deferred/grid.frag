#version 460

// Calm Godot-style infinite grid:
//   - logarithmic LOD blending (cells stay roughly constant on screen)
//   - low-contrast lines, muted axis tints
//   - soft, wide horizon fade — no hard ring
//   - camera-height aware outer fade keeps the visible disc consistent
// Hardware depth testing handles occlusion against scene geometry.

layout(location = 0) in vec3 world_pos;
layout(location = 0) out vec4 out_color;

layout(push_constant) uniform PushConstants {
    mat4 view_proj;
    vec4 camera_pos;     // xyz = camera position (render space), w = grid_extent
    vec4 grid_params;    // x = base spacing, y = unused, z = fade_start, w = fade_end
} pc;

// Anti-aliased grid line coverage for a given world-space spacing.
float grid_lines(vec2 coord, float spacing) {
    vec2 g = coord / spacing;
    vec2 d = fwidth(g);
    vec2 a = abs(fract(g - 0.5) - 0.5) / max(d, vec2(1e-6));
    return 1.0 - min(min(a.x, a.y), 1.0);
}

// Anti-aliased single-line coverage at value = 0 along one axis.
float axis_line(float coord_v, float width_px) {
    float d = fwidth(coord_v);
    return 1.0 - smoothstep(0.0, d * width_px, abs(coord_v));
}

void main() {
    vec2 coord = world_pos.xz;

    // Camera-height aware outer fade.
    float cam_height = max(abs(pc.camera_pos.y), 1.0);
    float fade_start = pc.grid_params.z + cam_height * 0.5;
    float fade_end   = pc.grid_params.w + cam_height * 2.0;
    float dist = length(pc.camera_pos.xz - coord);
    float dist_fade = 1.0 - smoothstep(fade_start, fade_end, dist);
    // Soften the falloff curve so the outer edge dissolves rather than
    // forming a hard ring.
    dist_fade = dist_fade * dist_fade;

    // Wide horizon fade — gradual transition, no visible band.
    vec3 view_dir = normalize(world_pos - pc.camera_pos.xyz);
    float horizon_fade = smoothstep(0.0, 0.35, abs(view_dir.y));

    // Logarithmic LOD selection.
    float base = max(pc.grid_params.x, 1e-4);
    float deriv = max(max(fwidth(coord).x, fwidth(coord).y), 1e-6);
    const float TARGET_PX = 10.0;
    float lod = max(0.0, log(deriv * TARGET_PX / base) / log(10.0));
    float lod_fract = fract(lod);
    float lod_floor = floor(lod);

    float s0 = base * pow(10.0, lod_floor);
    float s1 = s0 * 10.0;

    float l0 = grid_lines(coord, s0);
    float l1 = grid_lines(coord, s1);
    // Godot-style two-tier weighting: minor lines stay quiet, major lines
    // (every 10 minor cells) read clearly so the eye gets a coarse rhythm.
    float minor_alpha = l0 * (1.0 - lod_fract) * 0.18;
    float major_alpha = l1 * 0.50;
    float grid_alpha = max(minor_alpha, major_alpha);

    // Axis lines, ~1.4 px wide regardless of zoom — drawn at full opacity.
    float x_axis = axis_line(coord.y, 1.4); // line where coord.y == 0
    float y_axis = axis_line(coord.x, 1.4); // line where coord.x == 0

    vec3 grid_color = vec3(0.55);
    vec3 x_color = vec3(1.0, 0, 0);
    vec3 y_color = vec3(0, 1.0, 0);

    // Grid lines are faded by distance and horizon; axis lines are NOT —
    // they stay fully opaque everywhere on the visible plane.
    float faded_grid_alpha = grid_alpha * dist_fade * horizon_fade;
    float axis_alpha = max(x_axis, y_axis);

    // Blend grid colour first, then overlay the axis colour on top so the
    // axis pixels keep their saturated red/green regardless of grid state.
    vec3 final_color = grid_color;
    if (x_axis > 0.0) {
        final_color = mix(final_color, x_color, x_axis);
    }
    if (y_axis > 0.0) {
        final_color = mix(final_color, y_color, y_axis);
    }

    float final_alpha = max(faded_grid_alpha, axis_alpha);

    if (final_alpha < 0.001) {
        discard;
    }

    out_color = vec4(final_color, final_alpha);
}
