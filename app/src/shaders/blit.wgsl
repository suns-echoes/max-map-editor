// Minimap blit: one textured quad, nearest sampling - the
// source texture is CPU-built per mode (overworld sample / pass colors /
// in-game minimap bytes through the palette).

struct VsIn {
	@location(0) pos: vec2<f32>, // clip space
	@location(1) uv: vec2<f32>,
};

struct VsOut {
	@builtin(position) pos: vec4<f32>,
	@location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@vertex
fn vs_main(in: VsIn) -> VsOut {
	var out: VsOut;
	out.pos = vec4<f32>(in.pos, 0.0, 1.0);
	out.uv = in.uv;
	return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
	return textureSample(source, samp, in.uv);
}
