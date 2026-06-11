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
			bind_group_layouts: &[&bgl],
			push_constant_ranges: &[],
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
			multiview: None,
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
		let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("crt.pass"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view: target,
				resolve_target: None,
				ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
				depth_slice: None,
			})],
			depth_stencil_attachment: None,
			timestamp_writes: None,
			occlusion_query_set: None,
		});
		pass.set_pipeline(&self.pipeline);
		pass.set_bind_group(0, bind_group, &[]);
		pass.draw(0..3, 0..1);
	}
}
