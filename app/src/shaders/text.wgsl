// Screen-space textured quads. Two fragment entry points share one vertex
// shader (and one bind-group layout: texture + sampler):
//   fs_cover - font/coverage atlases (R8): uv.x < 0 = solid fill, else the
//              sampled .r modulates alpha. Used for shapes + labels.
//   fs_steel - the brushed-steel sheet (RGBA, REPEAT-sampled): the texel rgb
//              is tinted by the vertex color, alpha = grain strength. Used for
//              chrome fills, so every panel/button is cut from one metal sheet.
// Positions arrive already in clip space (built on the CPU from pixels), so
// there is no uniform - just the texture + sampler.

@group(0) @binding(0) var atlas: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
) -> VsOut {
    var o: VsOut;
    o.clip = vec4<f32>(pos, 0.0, 1.0);
    o.uv = uv;
    o.color = color;
    return o;
}

@fragment
fn fs_cover(in: VsOut) -> @location(0) vec4<f32> {
    var cov = 1.0;
    if (in.uv.x >= 0.0) {
        cov = textureSample(atlas, samp, in.uv).r;
    }
    return vec4<f32>(in.color.rgb, in.color.a * cov);
}

@fragment
fn fs_steel(in: VsOut) -> @location(0) vec4<f32> {
    let steel = textureSample(atlas, samp, in.uv).rgb;
    return vec4<f32>(steel * in.color.rgb, in.color.a);
}
