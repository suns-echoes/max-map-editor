//! Shared screen-space texture blit (one quad, nearest sampling) — the
//! minimap sources and the New Map modal's pack previews both draw
//! CPU-built RGBA textures through this pass (`blit.wgsl`).

use wgpu::util::DeviceExt;

use crate::ui::Rect;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlitVertex {
	pos: [f32; 2],
	uv: [f32; 2],
}

pub struct BlitPass {
	pipeline: wgpu::RenderPipeline,
	bgl: wgpu::BindGroupLayout,
	sampler: wgpu::Sampler,
}

impl BlitPass {
	pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
		let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("blit.bg_layout"),
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
			label: Some("blit.sampler"),
			mag_filter: wgpu::FilterMode::Nearest,
			min_filter: wgpu::FilterMode::Nearest,
			..Default::default()
		});
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("blit.shader"),
			source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blit.wgsl").into()),
		});
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("blit.layout"),
			bind_group_layouts: &[&bgl],
			push_constant_ranges: &[],
		});
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("blit.pipeline"),
			layout: Some(&layout),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs_main"),
				compilation_options: Default::default(),
				buffers: &[wgpu::VertexBufferLayout {
					array_stride: std::mem::size_of::<BlitVertex>() as wgpu::BufferAddress,
					step_mode: wgpu::VertexStepMode::Vertex,
					attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
				}],
			},
			fragment: Some(wgpu::FragmentState {
				module: &shader,
				entry_point: Some("fs_main"),
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
		Self { pipeline, bgl, sampler }
	}

	/// Upload an RGBA texture and bind it for this pass.
	pub fn upload(&self, device: &wgpu::Device, queue: &wgpu::Queue, rgba: &[u8], size: (u32, u32)) -> wgpu::BindGroup {
		let texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("blit.source"),
			size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rgba8UnormSrgb,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		queue.write_texture(
			texture.as_image_copy(),
			rgba,
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size.0 * 4), rows_per_image: Some(size.1) },
			wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
		);
		device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("blit.bg"),
			layout: &self.bgl,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(&texture.create_view(&Default::default())),
				},
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
			],
		})
	}

	/// Draw the bound texture's `uv` window (`[u0, v0, u1, v1]`) into `dst`,
	/// clipped to `scissor`.
	#[allow(clippy::too_many_arguments)]
	pub fn draw(
		&self,
		device: &wgpu::Device,
		encoder: &mut wgpu::CommandEncoder,
		target: &wgpu::TextureView,
		bind_group: &wgpu::BindGroup,
		dst: Rect,
		uv: [f32; 4],
		scissor: Rect,
		screen: (u32, u32),
	) {
		let (w, h) = (screen.0 as f32, screen.1 as f32);
		let sx = (scissor.x.max(0.0) as u32).min(screen.0);
		let sy = (scissor.y.max(0.0) as u32).min(screen.1);
		let sx1 = ((scissor.x + scissor.w).max(0.0) as u32).min(screen.0);
		let sy1 = ((scissor.y + scissor.h).max(0.0) as u32).min(screen.1);
		if sx1 <= sx || sy1 <= sy {
			return;
		}

		let nx = |x: f32| x / w * 2.0 - 1.0;
		let ny = |y: f32| 1.0 - y / h * 2.0;
		let (x0, y0, x1, y1) = (dst.x, dst.y, dst.x + dst.w, dst.y + dst.h);
		let v = |x: f32, y: f32, u: f32, vv: f32| BlitVertex { pos: [nx(x), ny(y)], uv: [u, vv] };
		let verts = [
			v(x0, y0, uv[0], uv[1]),
			v(x0, y1, uv[0], uv[3]),
			v(x1, y1, uv[2], uv[3]),
			v(x0, y0, uv[0], uv[1]),
			v(x1, y1, uv[2], uv[3]),
			v(x1, y0, uv[2], uv[1]),
		];
		let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
			label: Some("blit.vertices"),
			contents: bytemuck::cast_slice(&verts),
			usage: wgpu::BufferUsages::VERTEX,
		});
		let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("blit.pass"),
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
		pass.set_scissor_rect(sx, sy, sx1 - sx, sy1 - sy);
		pass.set_pipeline(&self.pipeline);
		pass.set_bind_group(0, bind_group, &[]);
		pass.set_vertex_buffer(0, buffer.slice(..));
		pass.draw(0..verts.len() as u32, 0..1);
	}
}
