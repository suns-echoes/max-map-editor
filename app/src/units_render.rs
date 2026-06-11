//! GPU half of the unit-preview feature (see `units.rs`): a single R8Uint
//! atlas holding every unit's body / turret / shadow frame in fixed 128px
//! slots, plus one quad pipeline that samples atlas → working palette with
//! the per-team color-slot remap done in the shader (`units.wgsl`).
//!
//! The pass owns its own 256×1 palette texture (updated from the cycler
//! alongside the map renderer's), so unit colors follow palette edits and
//! color cycling live — which is the whole point of the feature.

use wgpu::util::DeviceExt;

use crate::ui::Rect;
use crate::units::{UnitLibrary, UnitQuad};

/// Atlas geometry: 32×32 slots of 128px in a 4096² texture — room for ~340
/// units × 3 sprites each, far beyond the game's roster.
const SLOT: u32 = 128;
const SLOTS_PER_ROW: u32 = 32;
const ATLAS: u32 = SLOT * SLOTS_PER_ROW;

/// Where one sprite landed in the atlas.
pub struct SlotMeta {
	pub origin: (u32, u32),
	pub size: (u32, u32),
}

/// Per-unit atlas placements, parallel to `UnitLibrary::units`.
pub struct AtlasSlots {
	body: Vec<Option<SlotMeta>>,
	turret: Vec<Option<SlotMeta>>,
	shadow: Vec<Option<SlotMeta>>,
}

impl AtlasSlots {
	pub fn body(&self, unit: usize) -> Option<&SlotMeta> {
		self.body.get(unit)?.as_ref()
	}

	pub fn turret(&self, unit: usize) -> Option<&SlotMeta> {
		self.turret.get(unit)?.as_ref()
	}

	pub fn shadow(&self, unit: usize) -> Option<&SlotMeta> {
		self.shadow.get(unit)?.as_ref()
	}
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct UnitVertex {
	pos: [f32; 2],
	uv: [f32; 2],
	origin: [u32; 2],
	/// bits 0..3 = team, bit 3 = shadow.
	flags: u32,
}

pub struct UnitsGpu {
	pipeline: wgpu::RenderPipeline,
	bind_group: wgpu::BindGroup,
	palette_texture: wgpu::Texture,
	pub slots: AtlasSlots,
}

impl UnitsGpu {
	pub fn new(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		lib: &UnitLibrary,
		format: wgpu::TextureFormat,
		palette_rgba: &[u8],
	) -> Self {
		// ---- atlas: pack every frame into the next free 128px slot ----
		let mut pixels = vec![0u8; (ATLAS * ATLAS) as usize];
		let mut next = 0u32;
		let mut place = |frame: &max_assets::image::IndexedFrame| -> Option<SlotMeta> {
			if frame.width > SLOT || frame.height > SLOT || next >= SLOTS_PER_ROW * SLOTS_PER_ROW {
				return None;
			}
			let origin = ((next % SLOTS_PER_ROW) * SLOT, (next / SLOTS_PER_ROW) * SLOT);
			next += 1;
			for y in 0..frame.height {
				let src = (y * frame.width) as usize;
				let dst = ((origin.1 + y) * ATLAS + origin.0) as usize;
				pixels[dst..dst + frame.width as usize].copy_from_slice(&frame.pixels[src..src + frame.width as usize]);
			}
			Some(SlotMeta { origin, size: (frame.width, frame.height) })
		};

		let mut slots = AtlasSlots { body: Vec::new(), turret: Vec::new(), shadow: Vec::new() };
		for unit in &lib.units {
			slots.body.push(unit.body().and_then(&mut place));
			slots.turret.push(unit.turret().and_then(&mut place));
			slots.shadow.push(unit.shadow_frame().and_then(&mut place));
		}

		let atlas_texture = device.create_texture_with_data(
			queue,
			&wgpu::TextureDescriptor {
				label: Some("units.atlas"),
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
			label: Some("units.palette"),
			size: wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rgba8UnormSrgb,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});

		let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("units.bg_layout"),
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
			label: Some("units.bg"),
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
			label: Some("units.shader"),
			source: wgpu::ShaderSource::Wgsl(include_str!("shaders/units.wgsl").into()),
		});
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("units.layout"),
			bind_group_layouts: &[&bgl],
			push_constant_ranges: &[],
		});
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("units.pipeline"),
			layout: Some(&layout),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs_main"),
				compilation_options: Default::default(),
				buffers: &[wgpu::VertexBufferLayout {
					array_stride: std::mem::size_of::<UnitVertex>() as u64,
					step_mode: wgpu::VertexStepMode::Vertex,
					attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32x2, 3 => Uint32],
				}],
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

		let gpu = Self { pipeline, bind_group, palette_texture, slots };
		gpu.update_palette(queue, palette_rgba);
		gpu
	}

	/// Re-upload the working palette (256 RGBA bytes) — call alongside the
	/// map renderer's palette update so cycling stays in sync.
	///
	/// Unit art is authored against the *game* palette: the game overwrites
	/// every static slot (0-63, 160-255 — team ramps included) at runtime
	/// and only the dynamic slots (64-159) keep map/pack colors. Apply the
	/// same statics here, or units render in terrain colors.
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

	/// Draw unit quads (panel thumbnails or map placements). `scissor` clips
	/// panel content; pass `None` for the map overlay.
	pub fn draw(
		&self,
		device: &wgpu::Device,
		encoder: &mut wgpu::CommandEncoder,
		target: &wgpu::TextureView,
		quads: &[UnitQuad],
		scissor: Option<Rect>,
		screen: (u32, u32),
	) {
		if quads.is_empty() {
			return;
		}
		let (w, h) = (screen.0 as f32, screen.1 as f32);
		let (sx, sy, sw, sh) = match scissor {
			Some(s) => {
				let sx = s.x.clamp(0.0, w) as u32;
				let sy = s.y.clamp(0.0, h) as u32;
				let sw = (s.x + s.w).clamp(0.0, w) as u32 - sx;
				let sh = (s.y + s.h).clamp(0.0, h) as u32 - sy;
				(sx, sy, sw, sh)
			}
			None => (0, 0, screen.0, screen.1),
		};
		if sw == 0 || sh == 0 {
			return;
		}

		let nx = |x: f32| x / w * 2.0 - 1.0;
		let ny = |y: f32| 1.0 - y / h * 2.0;
		let mut verts = Vec::with_capacity(quads.len() * 6);
		for q in quads {
			let flags = (q.team as u32) | ((q.shadow as u32) << 3);
			let (x0, y0, x1, y1) = (q.rect.x, q.rect.y, q.rect.x + q.rect.w, q.rect.y + q.rect.h);
			let (uw, uh) = (q.sprite.0 as f32, q.sprite.1 as f32);
			let v = |x: f32, y: f32, u: f32, vv: f32| UnitVertex {
				pos: [nx(x), ny(y)],
				uv: [u, vv],
				origin: [q.origin.0, q.origin.1],
				flags,
			};
			let (tl, tr, br, bl) = (v(x0, y0, 0.0, 0.0), v(x1, y0, uw, 0.0), v(x1, y1, uw, uh), v(x0, y1, 0.0, uh));
			verts.extend_from_slice(&[tl, bl, br, tl, br, tr]);
		}
		let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
			label: Some("units.vertices"),
			contents: bytemuck::cast_slice(&verts),
			usage: wgpu::BufferUsages::VERTEX,
		});

		let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("units.pass"),
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
		pass.set_scissor_rect(sx, sy, sw, sh);
		pass.set_pipeline(&self.pipeline);
		pass.set_bind_group(0, &self.bind_group, &[]);
		pass.set_vertex_buffer(0, buffer.slice(..));
		pass.draw(0..verts.len() as u32, 0..1);
	}
}
