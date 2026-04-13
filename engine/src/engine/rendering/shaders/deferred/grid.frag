#version 460

// Unreal-style infinite grid fragment shader
// Uses world position from vertex shader (no ray-plane intersection needed)
// Hardware depth testing handles occlusion automatically

layout(location = 0) in vec3 world_pos;
layout(location = 0) out vec4 out_color;

layout(push_constant) uniform PushConstants {
    mat4 view_proj;      // View-projection matrix
    vec4 camera_pos;     // xyz = camera position, w = grid_extent
    vec4 grid_params;    // x = grid_size1, y = grid_size2, z = fade_start, w = fade_end
} pc;

// Calculate grid line intensity with anti-aliasing
float gridLine(vec2 pos, float size) {
    vec2 grid = abs(fract(pos / size - 0.5) - 0.5) / fwidth(pos / size);
    return 1.0 - min(min(grid.x, grid.y), 1.0);
}

void main() {
    // Grid coordinates on XZ plane (render space)
    // This corresponds to XY plane in Z-up game space
    vec2 coord = world_pos.xz;
    float dist = length(pc.camera_pos.xz - coord);

    // Grid parameters
    float grid_size1 = pc.grid_params.x;   // Fine grid (e.g., 1.0)
    float grid_size2 = pc.grid_params.y;   // Coarse grid (e.g., 10.0)
    float fade_start = pc.grid_params.z;
    float fade_end = pc.grid_params.w;

    // Distance-based fade
    float fade = 1.0 - smoothstep(fade_start, fade_end, dist);

    // Two grid levels
    float fineGrid = gridLine(coord, grid_size1);
    float coarseGrid = gridLine(coord, grid_size2);

    // Fade fine grid faster to avoid noise at distance
    float fineFade = 1.0 - smoothstep(fade_start * 0.3, fade_end * 0.3, dist);
    float grid = max(fineGrid * fineFade * 0.3, coarseGrid * 0.5);

    // Axis colors (Z-up convention)
    // In render space (Y-up): X runs along render X, Y runs along render Z
    // In game space (Z-up): X=forward (red), Y=right (green)
    float lineWidth = 0.05;
    vec3 axisColor = vec3(0.0);
    float axisMask = 0.0;

    // Y axis (green) - runs along render X direction at z=0
    // This is the Y axis in Z-up game space (right direction)
    if (abs(world_pos.z) < lineWidth) {
        axisColor = vec3(0.2, 0.9, 0.2);
        axisMask = 1.0;
    }

    // X axis (red) - runs along render Z direction at x=0
    // This is the X axis in Z-up game space (forward direction, negated)
    if (abs(world_pos.x) < lineWidth) {
        axisColor = vec3(0.9, 0.2, 0.2);
        axisMask = 1.0;
    }

    // Combine grid and axis colors
    vec3 gridColor = vec3(0.35);
    vec3 finalColor = mix(gridColor, axisColor, axisMask);
    float alpha = grid * fade + axisMask * fade;

    // Discard fully transparent pixels
    if (alpha < 0.01) {
        discard;
    }

    out_color = vec4(finalColor, alpha);
}
