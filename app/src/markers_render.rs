//! GPU half of the resource-marker overlay (see `markers.rs`): a small R8Uint
//! atlas of the RAW/FUEL/GOLD amount frames plus a quad pass sampling atlas →
//! working palette. Unlike the unit pass there is no team recolour (markers are
//! neutral) and no shadow — the simplest palette-LUT pass (`markers.wgsl`).
//!
//! Like the unit pass it owns its own 256×1 palette texture (updated alongside
//! the map renderer's) so the markers follow palette edits and colour cycling.

use wgpu::util::DeviceExt;

use crate::markers::MarkerLibrary;
use crate::units_render::SlotMeta;

/// Atlas geometry: 64px slots, 8 per row → a 512² atlas. 3 materials × 17
/// frames = 51 sprites (largest 62²), so 64 slots is ample.
const SLOT: u32 = 64;
const SLOTS_PER_ROW: u32 = 8;
const ATLAS: u32 = SLOT * SLOTS_PER_ROW;

/// Where each marker frame landed in the atlas, parallel to
/// [`MarkerLibrary::frames`] — `[material_row][frame]`.
pub struct MarkerSlots {
	frames: Vec<Vec<SlotMeta>>,
}

impl MarkerSlots {
	/// Atlas placement of material-row `row`'s frame `fi` (clamped to the strip,
	/// matching [`MarkerLibrary::frame_at`]).
	pub fn frame(&self, row: usize, fi: usize) -> Option<&SlotMeta> {
		let strip = self.frames.get(row)?;
		strip.get(fi.min(strip.len().checked_sub(1)?))
	}

	/// Build slots directly from atlas placements — for `markers.rs`'s quad tests
	/// (the real slots come from [`MarkersGpu::new`], which needs a GPU device).
	#[cfg(test)]
	pub(crate) fn from_meta(frames: Vec<Vec<SlotMeta>>) -> Self {
		Self { frames }
	}
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MarkerVertex {
	pos: [f32; 2],
	uv: [f32; 2],
	origin: [u32; 2],
}

pub struct MarkersGpu {
	pipeline: wgpu::RenderPipeline,
	bind_group: wgpu::BindGroup,
	palette_texture: wgpu::Texture,
	pub slots: MarkerSlots,
}

impl MarkersGpu {
	pub fn new(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		lib: &MarkerLibrary,
		format: wgpu::TextureFormat,
		palette_rgba: &[u8],
	) -> Self {
		// ---- atlas: pack every marker frame into the next free 64px slot ----
		let mut pixels = vec![0u8; (ATLAS * ATLAS) as usize];
		let mut next = 0u32;
		let mut place = |frame: &max_assets::image::IndexedFrame| -> SlotMeta {
			if frame.width > SLOT || frame.height > SLOT || next >= SLOTS_PER_ROW * SLOTS_PER_ROW {
				return SlotMeta::EMPTY;
			}
			let origin = ((next % SLOTS_PER_ROW) * SLOT, (next / SLOTS_PER_ROW) * SLOT);
			next += 1;
			for y in 0..frame.height {
				let src = (y * frame.width) as usize;
				let dst = ((origin.1 + y) * ATLAS + origin.0) as usize;
				pixels[dst..dst + frame.width as usize].copy_from_slice(&frame.pixels[src..src + frame.width as usize]);
			}
			SlotMeta { origin, size: (frame.width, frame.height) }
		};
		let slots =
			MarkerSlots { frames: lib.frames.iter().map(|strip| strip.iter().map(&mut place).collect()).collect() };

		let atlas_texture = device.create_texture_with_data(
			queue,
			&wgpu::TextureDescriptor {
				label: Some("markers.atlas"),
				size: wgpu::Extent3d { width: ATLAS, height: ATLAS, depth_or_array_layers: 1 },
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: wgpu::TextureFormat::R8Uint,
				usage: wgpu::TextureUsages::TEXTURE_BINDING,
				view_formats: &[],
			},
			wgpu::util::TextureDataOrder::LayerMajor,
			&pixels,
		);

		let palette_texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("markers.palette"),
			size: wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rgba8UnormSrgb,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});

		let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("markers.bg_layout"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Uint,
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 1,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Float { filterable: false },
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
					count: None,
				},
			],
		});
		let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("markers.bg"),
			layout: &bgl,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(
						&atlas_texture.create_view(&wgpu::TextureViewDescriptor::default()),
					),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::TextureView(
						&palette_texture.create_view(&wgpu::TextureViewDescriptor::default()),
					),
				},
			],
		});

		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("markers.shader"),
			source: wgpu::ShaderSource::Wgsl(include_str!("shaders/markers.wgsl").into()),
		});
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("markers.layout"),
			bind_group_layouts: &[Some(&bgl)],
			immediate_size: 0,
		});
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("markers.pipeline"),
			layout: Some(&layout),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs_main"),
				compilation_options: Default::default(),
				buffers: &[Some(wgpu::VertexBufferLayout {
					array_stride: std::mem::size_of::<MarkerVertex>() as u64,
					step_mode: wgpu::VertexStepMode::Vertex,
					attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32x2],
				})],
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
			multiview_mask: None,
			cache: None,
		});

		let gpu = Self { pipeline, bind_group, palette_texture, slots };
		gpu.update_palette(queue, palette_rgba);
		gpu
	}

	/// Re-upload the working palette (256 RGBA bytes) — call alongside the map
	/// renderer's palette update so cycling stays in sync. Marker art is authored
	/// against the *game* palette (the game overwrites the static slots at
	/// runtime), so apply the same statics here, exactly like the unit pass.
	pub fn update_palette(&self, queue: &wgpu::Queue, rgba: &[u8]) {
		let mut rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]]).collect();
		map_core::apply_game_statics(&mut rgb);
		let patched: Vec<u8> = rgb.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect();
		queue.write_texture(
			self.palette_texture.as_image_copy(),
			&patched,
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 4), rows_per_image: Some(1) },
			wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
		);
	}

	/// Draw the resource-marker quads over the map (native px, no scissor).
	pub fn draw(
		&self,
		device: &wgpu::Device,
		encoder: &mut wgpu::CommandEncoder,
		target: &wgpu::TextureView,
		quads: &[crate::markers::MarkerQuad],
		screen: (u32, u32),
	) {
		if quads.is_empty() {
			return;
		}
		let (w, h) = (screen.0 as f32, screen.1 as f32);
		let nx = |x: f32| x / w * 2.0 - 1.0;
		let ny = |y: f32| 1.0 - y / h * 2.0;
		let mut verts = Vec::with_capacity(quads.len() * 6);
		for q in quads {
			let (x0, y0, x1, y1) = (q.rect.x, q.rect.y, q.rect.x + q.rect.w, q.rect.y + q.rect.h);
			let (uw, uh) = (q.sprite.0 as f32, q.sprite.1 as f32);
			let v = |x: f32, y: f32, u: f32, vv: f32| MarkerVertex {
				pos: [nx(x), ny(y)],
				uv: [u, vv],
				origin: [q.origin.0, q.origin.1],
			};
			let (tl, tr, br, bl) = (v(x0, y0, 0.0, 0.0), v(x1, y0, uw, 0.0), v(x1, y1, uw, uh), v(x0, y1, 0.0, uh));
			verts.extend_from_slice(&[tl, bl, br, tl, br, tr]);
		}
		let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
			label: Some("markers.vertices"),
			contents: bytemuck::cast_slice(&verts),
			usage: wgpu::BufferUsages::VERTEX,
		});

		let mut pass = crate::render::load_pass(encoder, target, "markers.pass");
		pass.set_pipeline(&self.pipeline);
		pass.set_bind_group(0, &self.bind_group, &[]);
		pass.set_vertex_buffer(0, buffer.slice(..));
		pass.draw(0..verts.len() as u32, 0..1);
	}
}
