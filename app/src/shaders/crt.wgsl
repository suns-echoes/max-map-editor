// CRT post-process (#10): samples the rendered scene and applies a tasteful
// shadow-mask/scanline/vignette over the whole app. One fullscreen triangle,
// no vertex buffer (positions are generated from the vertex index).

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // A triangle that covers the viewport (clip-space corners well outside it).
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    let xy = corners[vi];
    var out: VsOut;
    out.clip = vec4<f32>(xy, 0.0, 1.0);
    out.uv = vec2<f32>((xy.x + 1.0) * 0.5, (1.0 - xy.y) * 0.5);
    return out;
}

@fragment
fn fs_crt(in: VsOut) -> @location(0) vec4<f32> {
    var col = textureSample(scene, samp, in.uv).rgb;

    // Scanlines: darken every other physical pixel row.
    let py = i32(in.clip.y);
    if ((py & 1) == 0) {
        col *= 0.72;
    }

    // Aperture (shadow) mask: emphasize R / G / B across runs of three columns.
    let px = i32(in.clip.x) % 3;
    var mask = vec3<f32>(0.92, 0.92, 0.92);
    if (px == 0) { mask.r = 1.08; }
    else if (px == 1) { mask.g = 1.08; }
    else { mask.b = 1.08; }
    col *= mask;

    // Vignette: gently darken toward the edges/corners.
    let d = in.uv - vec2<f32>(0.5, 0.5);
    col *= clamp(1.0 - dot(d, d) * 0.7, 0.0, 1.0);

    return vec4<f32>(col, 1.0);
}
