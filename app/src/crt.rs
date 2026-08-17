//! CRT post-process pass: render the whole frame to an offscreen scene
//! texture, then draw one fullscreen triangle that samples it through
//! `crt.wgsl` (scanlines + shadow mask + vignette) onto the real target.

/// An offscreen scene target the frame renders into, plus the bind group the
/// CRT pass samples it through. Recreated when the viewport size changes.
pub struct SceneTarget {
	pub size: (u32, u32),
	pub view: wgpu::TextureView,
	pub bind_group: wgpu::BindGroup,
}

pub struct CrtPass {
	pipeline: wgpu::RenderPipeline,
	bgl: wgpu::BindGroupLayout,
	sampler: wgpu::Sampler,
	format: wgpu::TextureFormat,
}

impl CrtPass {
	pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
		let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("crt.bg_layout"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Float { filterable: true },
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 1,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
					count: None,
				},
			],
		});
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			label: Some("crt.sampler"),
			mag_filter: wgpu::FilterMode::Nearest,
			min_filter: wgpu::FilterMode::Nearest,
			..Default::default()
		});
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("crt.shader"),
			source: wgpu::ShaderSource::Wgsl(include_str!("shaders/crt.wgsl").into()),
		});
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("crt.layout"),
			bind_group_layouts: &[Some(&bgl)],
			immediate_size: 0,
		});
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("crt.pipeline"),
			layout: Some(&layout),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs_main"),
				compilation_options: Default::default(),
				buffers: &[],
			},
			fragment: Some(wgpu::FragmentState {
				module: &shader,
				entry_point: Some("fs_crt"),
				compilation_options: Default::default(),
				targets: &[Some(format.into())],
			}),
			primitive: wgpu::PrimitiveState {
				topology: wgpu::PrimitiveTopology::TriangleList,
				cull_mode: None,
				..Default::default()
			},
			depth_stencil: None,
			multisample: wgpu::MultisampleState::default(),
			multiview_mask: None,
			cache: None,
		});
		Self { pipeline, bgl, sampler, format }
	}

	/// Create the offscreen scene target for a `size`-px viewport (+ its CRT
	/// sampling bind group). The `TextureView` keeps the texture alive.
	pub fn make_target(&self, device: &wgpu::Device, size: (u32, u32)) -> SceneTarget {
		let texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("crt.scene"),
			size: wgpu::Extent3d { width: size.0.max(1), height: size.1.max(1), depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: self.format,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
			view_formats: &[],
		});
		let view = texture.create_view(&Default::default());
		let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("crt.bg"),
			layout: &self.bgl,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
			],
		});
		SceneTarget { size, view, bind_group }
	}

	/// Post-process the scene (via `bind_group`) onto `target`.
	pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, bind_group: &wgpu::BindGroup) {
		let mut pass = crate::render::load_pass(encoder, target, "crt.pass");
		pass.set_pipeline(&self.pipeline);
		pass.set_bind_group(0, bind_group, &[]);
		pass.draw(0..3, 0..1);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::Path;

	/// Decode a PNG `capture` wrote back to tightly-packed RGBA8.
	fn read_png(path: &Path) -> (Vec<u8>, u32, u32) {
		let file = std::fs::File::open(path).expect("open png");
		let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().expect("png info");
		let mut buf = vec![0; reader.output_buffer_size().expect("png size")];
		let info = reader.next_frame(&mut buf).expect("png frame");
		buf.truncate(info.buffer_size());
		(buf, info.width, info.height)
	}

	/// One render pass that clears `view` to `color`.
	fn clear(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, color: wgpu::Color) {
		encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("test.clear"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view,
				resolve_target: None,
				depth_slice: None,
				ops: wgpu::Operations { load: wgpu::LoadOp::Clear(color), store: wgpu::StoreOp::Store },
			})],
			depth_stencil_attachment: None,
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
	}

	/// `make_target` builds the scene target at the requested size (a 0×0
	/// viewport clamps the texture to 1px instead of panicking), and `draw`
	/// post-processes it onto the real target: on a solid green scene, even
	/// physical rows are scanline-darkened and corners vignette-darkened
	/// relative to the centre, while every pixel stays green-dominant.
	#[test]
	fn crt_pass_applies_scanlines_and_vignette() {
		let (device, queue, _serial) = crate::visual_test::test_gpu();
		let crt = CrtPass::new(&device, crate::capture::FORMAT);

		let degenerate = crt.make_target(&device, (0, 0));
		assert_eq!(degenerate.size, (0, 0), "reports the requested size; the texture is clamped to 1px");

		let (w, h) = (64u32, 48u32);
		let scene = crt.make_target(&device, (w, h));
		assert_eq!(scene.size, (w, h));

		let dir = std::env::temp_dir().join(format!("max-map-editor-crt-{}", std::process::id()));
		let path = dir.join("crt.png");
		crate::capture::render_to_png(&device, &queue, w, h, &path, None, None, |encoder, view| {
			clear(encoder, &scene.view, wgpu::Color::GREEN);
			crt.draw(encoder, view, &scene.bind_group);
		});
		let (rgba, pw, ph) = read_png(&path);
		assert_eq!((pw, ph), (w, h));
		let green = |x: u32, y: u32| rgba[((y * w + x) * 4 + 1) as usize];
		assert!(
			rgba.chunks_exact(4).all(|p| p[1] >= p[0] && p[1] >= p[2] && p[3] == 255),
			"a green scene stays green-dominant and opaque"
		);
		// Scanlines: an even physical row is darker than the odd row below it
		// (same column, so the aperture mask cancels out).
		let (even, odd) = (green(31, 24), green(31, 25));
		assert!(even + 10 < odd, "scanline: even row {even} not darker than odd row {odd}");
		// Vignette: a corner is darker than the centre. (1,1) and (31,25) share
		// row parity and column mod 3, so scanline + mask factors match.
		let (corner, centre) = (green(1, 1), green(31, 25));
		assert!(corner + 10 < centre, "vignette: corner {corner} not darker than centre {centre}");
		let _ = std::fs::remove_dir_all(&dir);
	}
}
