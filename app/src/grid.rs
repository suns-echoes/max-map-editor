//! Grid overlay pass: one fullscreen, alpha-blended pass
//! that draws a cell-bevel grid (light top/left, dark bottom/right inner
//! edges, >= 1 screen px so it can't vanish at any zoom) on top of whatever
//! map the active renderer drew. Path-agnostic — the same pass serves the
//! project and flat-WRL renderers (it reads only pan/zoom/map-size).

use crate::render::Uniforms;

/// Master opacity of the bevel (the shader derives both tones from it).
pub const GRID_STRENGTH: f32 = 0.45;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GridUniforms {
	screen_size: [f32; 2],
	pan: [f32; 2],
	map_size: [f32; 2],
	zoom: f32,
	strength: f32,
}

pub struct GridPass {
	pipeline: wgpu::RenderPipeline,
	bind_group: wgpu::BindGroup,
	uniforms_buffer: wgpu::Buffer,
}

impl GridPass {
	pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
		let uniforms_buffer = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("grid.uniforms"),
			size: std::mem::size_of::<GridUniforms>() as u64,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
		let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("grid.bg_layout"),
			entries: &[wgpu::BindGroupLayoutEntry {
				binding: 0,
				visibility: wgpu::ShaderStages::FRAGMENT,
				ty: wgpu::BindingType::Buffer {
					ty: wgpu::BufferBindingType::Uniform,
					has_dynamic_offset: false,
					min_binding_size: None,
				},
				count: None,
			}],
		});
		let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("grid.bg"),
			layout: &bgl,
			entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms_buffer.as_entire_binding() }],
		});
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("grid.shader"),
			source: wgpu::ShaderSource::Wgsl(include_str!("shaders/grid.wgsl").into()),
		});
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("grid.layout"),
			bind_group_layouts: &[&bgl],
			push_constant_ranges: &[],
		});
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("grid.pipeline"),
			layout: Some(&layout),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs_main"),
				compilation_options: Default::default(),
				buffers: &[],
			},
			fragment: Some(wgpu::FragmentState {
				module: &shader,
				entry_point: Some("fs_main"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState {
					format,
					blend: Some(wgpu::BlendState::ALPHA_BLENDING),
					write_mask: wgpu::ColorWrites::ALL,
				})],
			}),
			primitive: wgpu::PrimitiveState::default(),
			depth_stencil: None,
			multisample: wgpu::MultisampleState::default(),
			multiview: None,
			cache: None,
		});
		Self { pipeline, bind_group, uniforms_buffer }
	}

	/// Draw the grid on top of `target` (load, don't clear).
	pub fn draw(
		&self,
		queue: &wgpu::Queue,
		encoder: &mut wgpu::CommandEncoder,
		target: &wgpu::TextureView,
		u: Uniforms,
		strength: f32,
	) {
		let gu = GridUniforms { screen_size: u.screen_size, pan: u.pan, map_size: u.map_size, zoom: u.zoom, strength };
		queue.write_buffer(&self.uniforms_buffer, 0, bytemuck::bytes_of(&gu));
		let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("grid.pass"),
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
		pass.set_bind_group(0, &self.bind_group, &[]);
		pass.draw(0..3, 0..1);
	}
}
