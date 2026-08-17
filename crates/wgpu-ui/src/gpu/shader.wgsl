// UI quad pipeline: pixel-space vertices, one atlas texture, alpha blending.
//
// Vertex colors arrive as straight sRGB (0..1) and are decoded to linear here,
// so blending happens in linear light and the sRGB render target re-encodes to
// exactly the authored color. The glyph atlas is a single-channel R8 coverage
// texture: when `mode == 1` the fragment takes white RGB from the vertex color
// and coverage from the atlas's red channel (so a solid — which samples the
// atlas's opaque-white texel, red = 1 — passes straight through). Host RGBA
// sprites use `mode == 0` and sample normally.

struct Uniforms {
    screen: vec2<f32>,
    _pad: vec2<f32>,
};

// Group 0 is set once per frame (uniforms + sampler); group 1 is the texture,
// swapped per batch so solids/glyphs (the atlas) and host sprites share one
// pipeline.
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var tex_sampler: sampler;
@group(1) @binding(0) var tex: texture_2d<f32>;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) mode: u32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) mode: u32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    // Pixel space (origin top-left, y down) -> normalized device coords.
    let ndc = vec2<f32>(
        in.pos.x / u.screen.x * 2.0 - 1.0,
        1.0 - in.pos.y / u.screen.y * 2.0,
    );
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    out.mode = in.mode;
    return out;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let cutoff = c <= vec3<f32>(0.04045);
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, cutoff);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(tex, tex_sampler, in.uv);
    let lin = srgb_to_linear(in.color.rgb);
    if (in.mode == 1u) {
        // Coverage atlas: white RGB from the vertex color, coverage from red.
        return vec4<f32>(lin, in.color.a * texel.r);
    }
    // Host RGBA sprite: modulate normally.
    return vec4<f32>(lin * texel.rgb, in.color.a * texel.a);
}
