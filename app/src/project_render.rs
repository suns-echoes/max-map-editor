//! Layered project renderer: cell-stack texture + 2D-array
//! tile atlas + palette LUT, drawn by `project.wgsl`. The GPU path mirrors
//! `map_core`'s CPU `compose_cell` (verified against all original WRLs by
//! the equivalence test). Also owns the Tile Explorer grid pass (
//! `picker.wgsl`) — same atlas + palette, screen-space quads.

use map_core::{LAYER_GROUND, LAYER_WATER, Project};
use max_assets::wrl::TILE_DATA_SIZE;
use wgpu::util::DeviceExt;

use crate::picker::TileQuad;
use crate::render::Uniforms;
use crate::ui::Rect;

/// Tiles per atlas layer: 16×16 on a 1024×1024 layer. 65 536 tiles (the
/// engine cap) = 256 layers — comfortably inside GPU array limits.
const ATLAS_LAYER_TILES: u32 = 256;
const ATLAS_LAYER_PX: u32 = 1024;
const TILE_PX: u32 = 64;

pub struct ProjectRenderer {
	pipeline: wgpu::RenderPipeline,
	bind_group: wgpu::BindGroup,
	uniforms_buffer: wgpu::Buffer,
	palette_texture: wgpu::Texture,
	cells_texture: wgpu::Texture,
	/// Per-cell pass value (R8Uint) for the pass overlay.
	pass_texture: wgpu::Texture,
	/// Overlay enable flag (uniform), written per draw.
	overlay_buffer: wgpu::Buffer,
	/// Global atlas base index per pack (parallel to `project.packs`).
	pack_base: Vec<u32>,
	/// Tile Explorer grid pass — shares the atlas + palette.
	picker_pipeline: wgpu::RenderPipeline,
	picker_bind_group: wgpu::BindGroup,
}

/// One picker-grid vertex: clip-space position, 0..1 uv within the tile,
/// global atlas tile index, transform bits (0 = base art).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PickerVertex {
	pos: [f32; 2],
	uv: [f32; 2],
	index: u32,
	transform: u32,
	/// Whole-quad opacity — 1.0 for panels, <1 for the ghost-stamp preview.
	alpha: f32,
}

impl PickerVertex {
	fn layout() -> wgpu::VertexBufferLayout<'static> {
		const ATTRS: [wgpu::VertexAttribute; 5] =
			wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32, 3 => Uint32, 4 => Float32];
		wgpu::VertexBufferLayout {
			array_stride: std::mem::size_of::<PickerVertex>() as wgpu::BufferAddress,
			step_mode: wgpu::VertexStepMode::Vertex,
			attributes: &ATTRS,
		}
	}
}

impl ProjectRenderer {
	pub fn new(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		project: &Project,
		target_format: wgpu::TextureFormat,
	) -> Self {
		// --- Tile atlas: all packs' tiles, globally indexed -------------
		let total_tiles: u32 = project.packs.iter().map(|p| p.tile_count() as u32).sum();
		let layers = total_tiles.div_ceil(ATLAS_LAYER_TILES).max(1);
		let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("project.atlas"),
			size: wgpu::Extent3d { width: ATLAS_LAYER_PX, height: ATLAS_LAYER_PX, depth_or_array_layers: layers },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::R8Uint,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		let mut pack_base = Vec::with_capacity(project.packs.len());
		let mut global = 0u32;
		for pack in &project.packs {
			pack_base.push(global);
			for tile in 0..pack.tile_count() {
				let slot = global & (ATLAS_LAYER_TILES - 1);
				queue.write_texture(
					wgpu::TexelCopyTextureInfo {
						texture: &atlas_texture,
						mip_level: 0,
						origin: wgpu::Origin3d {
							x: (slot % 16) * TILE_PX,
							y: (slot / 16) * TILE_PX,
							z: global / ATLAS_LAYER_TILES,
						},
						aspect: wgpu::TextureAspect::All,
					},
					pack.tile_pixels(tile),
					wgpu::TexelCopyBufferLayout {
						offset: 0,
						bytes_per_row: Some(TILE_PX),
						rows_per_image: Some(TILE_PX),
					},
					wgpu::Extent3d { width: TILE_PX, height: TILE_PX, depth_or_array_layers: 1 },
				);
				let _ = TILE_DATA_SIZE; // (tile size is pinned by max-assets)
				global += 1;
			}
		}

		// --- Cell stacks: r/g water idx+1/transform, b/a ground --------
		let (w, h) = (project.width as u32, project.height as u32);
		let cell_data = build_cell_data(project, &pack_base);
		let cells_texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("project.cells"),
			size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rgba16Uint,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		queue.write_texture(
			cells_texture.as_image_copy(),
			bytemuck::cast_slice(&cell_data),
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 8), rows_per_image: Some(h) },
			wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
		);

		// --- Pass values per cell (R8Uint) for the pass overlay -
		let pass_texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("project.pass"),
			size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::R8Uint,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		queue.write_texture(
			pass_texture.as_image_copy(),
			&build_pass_data(project),
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
			wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
		);

		// --- Palette (cycled by the shared PaletteCycler) ---------------
		let mut palette_rgba = Vec::with_capacity(256 * 4);
		for rgb in project.palette.chunks_exact(3) {
			palette_rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
		}
		let palette_texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("project.palette"),
			size: wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rgba8UnormSrgb,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		queue.write_texture(
			palette_texture.as_image_copy(),
			&palette_rgba,
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 4), rows_per_image: Some(1) },
			wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
		);

		let uniforms_buffer = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("project.uniforms"),
			size: std::mem::size_of::<Uniforms>() as u64,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
		// Pass-overlay enable flag (16 bytes — uniform min size).
		let overlay_buffer = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("project.overlay"),
			size: 16,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});

		let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("project.bg_layout"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Buffer {
						ty: wgpu::BufferBindingType::Uniform,
						has_dynamic_offset: false,
						min_binding_size: None,
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 1,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Uint,
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 2,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Uint,
						view_dimension: wgpu::TextureViewDimension::D2Array,
						multisampled: false,
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 3,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Float { filterable: false },
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 4,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Uint,
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 5,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Buffer {
						ty: wgpu::BufferBindingType::Uniform,
						has_dynamic_offset: false,
						min_binding_size: None,
					},
					count: None,
				},
			],
		});
		let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("project.bg"),
			layout: &bind_group_layout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: uniforms_buffer.as_entire_binding() },
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::TextureView(&cells_texture.create_view(&Default::default())),
				},
				wgpu::BindGroupEntry {
					binding: 2,
					resource: wgpu::BindingResource::TextureView(&atlas_texture.create_view(
						&wgpu::TextureViewDescriptor {
							dimension: Some(wgpu::TextureViewDimension::D2Array),
							..Default::default()
						},
					)),
				},
				wgpu::BindGroupEntry {
					binding: 3,
					resource: wgpu::BindingResource::TextureView(&palette_texture.create_view(&Default::default())),
				},
				wgpu::BindGroupEntry {
					binding: 4,
					resource: wgpu::BindingResource::TextureView(&pass_texture.create_view(&Default::default())),
				},
				wgpu::BindGroupEntry { binding: 5, resource: overlay_buffer.as_entire_binding() },
			],
		});

		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("project.shader"),
			source: wgpu::ShaderSource::Wgsl(include_str!("shaders/project.wgsl").into()),
		});
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("project.layout"),
			bind_group_layouts: &[&bind_group_layout],
			push_constant_ranges: &[],
		});
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("project.pipeline"),
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
				targets: &[Some(target_format.into())],
			}),
			primitive: wgpu::PrimitiveState::default(),
			depth_stencil: None,
			multisample: wgpu::MultisampleState::default(),
			multiview: None,
			cache: None,
		});

		// --- Tile Explorer grid pass: atlas + palette only ---------
		let picker_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("picker.bg_layout"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Uint,
						view_dimension: wgpu::TextureViewDimension::D2Array,
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
		let picker_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("picker.bg"),
			layout: &picker_bgl,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(&atlas_texture.create_view(
						&wgpu::TextureViewDescriptor {
							dimension: Some(wgpu::TextureViewDimension::D2Array),
							..Default::default()
						},
					)),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::TextureView(&palette_texture.create_view(&Default::default())),
				},
			],
		});
		let picker_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("picker.shader"),
			source: wgpu::ShaderSource::Wgsl(include_str!("shaders/picker.wgsl").into()),
		});
		let picker_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("picker.layout"),
			bind_group_layouts: &[&picker_bgl],
			push_constant_ranges: &[],
		});
		let picker_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("picker.pipeline"),
			layout: Some(&picker_layout),
			vertex: wgpu::VertexState {
				module: &picker_shader,
				entry_point: Some("vs_main"),
				compilation_options: Default::default(),
				buffers: &[PickerVertex::layout()],
			},
			fragment: Some(wgpu::FragmentState {
				module: &picker_shader,
				entry_point: Some("fs_main"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState {
					format: target_format,
					blend: Some(wgpu::BlendState::ALPHA_BLENDING),
					write_mask: wgpu::ColorWrites::ALL,
				})],
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

		Self {
			pipeline,
			bind_group,
			uniforms_buffer,
			palette_texture,
			cells_texture,
			pass_texture,
			overlay_buffer,
			pack_base,
			picker_pipeline,
			picker_bind_group,
		}
	}

	/// Draw the Tile Explorer grid: one quad per visible tile, clipped to
	/// the panel body via the scissor rect.
	/// `alpha` scales every quad's opacity — 1.0 for panel content, lower for
	/// the ghost-stamp preview riding under the cursor.
	#[allow(clippy::too_many_arguments)]
	pub fn draw_picker(
		&self,
		device: &wgpu::Device,
		encoder: &mut wgpu::CommandEncoder,
		target: &wgpu::TextureView,
		tiles: &[TileQuad],
		scissor: Rect,
		screen: (u32, u32),
		alpha: f32,
	) {
		if tiles.is_empty() {
			return;
		}
		let (w, h) = (screen.0 as f32, screen.1 as f32);
		let sx = scissor.x.clamp(0.0, w) as u32;
		let sy = scissor.y.clamp(0.0, h) as u32;
		let sw = (scissor.x + scissor.w).clamp(0.0, w) as u32 - sx;
		let sh = (scissor.y + scissor.h).clamp(0.0, h) as u32 - sy;
		if sw == 0 || sh == 0 {
			return;
		}

		let nx = |x: f32| x / w * 2.0 - 1.0;
		let ny = |y: f32| 1.0 - y / h * 2.0;
		let mut verts = Vec::with_capacity(tiles.len() * 6);
		for t in tiles {
			let (x0, y0, x1, y1) = (t.rect.x, t.rect.y, t.rect.x + t.rect.w, t.rect.y + t.rect.h);
			let v = |x: f32, y: f32, u: f32, vv: f32| PickerVertex {
				pos: [nx(x), ny(y)],
				uv: [u, vv],
				index: t.index,
				transform: t.transform,
				alpha,
			};
			let (tl, tr, br, bl) = (v(x0, y0, 0.0, 0.0), v(x1, y0, 1.0, 0.0), v(x1, y1, 1.0, 1.0), v(x0, y1, 0.0, 1.0));
			verts.extend_from_slice(&[tl, bl, br, tl, br, tr]);
		}
		let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
			label: Some("picker.vertices"),
			contents: bytemuck::cast_slice(&verts),
			usage: wgpu::BufferUsages::VERTEX,
		});

		let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("picker.pass"),
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
		pass.set_pipeline(&self.picker_pipeline);
		pass.set_bind_group(0, &self.picker_bind_group, &[]);
		pass.set_vertex_buffer(0, buffer.slice(..));
		pass.draw(0..verts.len() as u32, 0..1);
	}

	/// Re-upload the cell-stack texture after edits (full upload — partial
	/// dirty-rect upload is a future dirty-rect optimization).
	pub fn update_cells(&self, queue: &wgpu::Queue, project: &Project) {
		let (w, h) = (project.width as u32, project.height as u32);
		queue.write_texture(
			self.cells_texture.as_image_copy(),
			bytemuck::cast_slice(&build_cell_data(project, &self.pack_base)),
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 8), rows_per_image: Some(h) },
			wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
		);
		// Pass values follow the cell edits (the pass overlay).
		queue.write_texture(
			self.pass_texture.as_image_copy(),
			&build_pass_data(project),
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
			wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
		);
	}

	pub fn update_palette(&self, queue: &wgpu::Queue, rgba: &[u8]) {
		queue.write_texture(
			self.palette_texture.as_image_copy(),
			rgba,
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 4), rows_per_image: Some(1) },
			wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
		);
	}

	pub fn draw(
		&self,
		queue: &wgpu::Queue,
		encoder: &mut wgpu::CommandEncoder,
		target: &wgpu::TextureView,
		uniforms: Uniforms,
		pass_overlay: bool,
	) {
		queue.write_buffer(&self.uniforms_buffer, 0, bytemuck::bytes_of(&uniforms));
		let overlay: [u32; 4] = [pass_overlay as u32, 0, 0, 0];
		queue.write_buffer(&self.overlay_buffer, 0, bytemuck::cast_slice(&overlay));
		let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("project.pass"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view: target,
				resolve_target: None,
				// Load (not Clear): the app-background steel is drawn first and the
				// shader discards out-of-map fragments, so it shows through.
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

/// Per-cell pass value (0 land / 1 water / 2 shore / 3 blocked) for the
/// overlay; a cell whose top tile has no pass data reads as land (0).
fn build_pass_data(project: &Project) -> Vec<u8> {
	let (w, h) = (project.width, project.height);
	let mut out = Vec::with_capacity(w as usize * h as usize);
	for y in 0..h {
		for x in 0..w {
			out.push(project.pass_at(x, y).unwrap_or(0));
		}
	}
	out
}

fn build_cell_data(project: &Project, pack_base: &[u32]) -> Vec<u16> {
	let mut cell_data = Vec::with_capacity(project.cells.len() * 4);
	for stack in &project.cells {
		for layer in [LAYER_WATER, LAYER_GROUND] {
			match stack[layer] {
				Some(t) => {
					let index = pack_base[t.pack as usize] + t.tile as u32;
					cell_data.push((index + 1) as u16);
					cell_data.push(t.transform.bits() as u16);
				}
				None => {
					cell_data.push(0);
					cell_data.push(0);
				}
			}
		}
	}
	cell_data
}
