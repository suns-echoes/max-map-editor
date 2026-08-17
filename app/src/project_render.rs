//! Layered project renderer: cell-stack texture + 2D-array
//! tile atlas + palette LUT, drawn by `project.wgsl`. The GPU path mirrors
//! `map_core`'s CPU `compose_cell` (verified against all original WRLs by
//! the equivalence test). Also owns the Tile Explorer grid pass (
//! `picker.wgsl`) - same atlas + palette, screen-space quads.

use map_core::{LAYER_GROUND, LAYER_WATER, Project, RenderDirty};
use max_assets::wrl::TILE_DATA_SIZE;
use wgpu::util::DeviceExt;

use crate::picker::TileQuad;
use crate::render::Uniforms;
use crate::ui::Rect;

use crate::render::TILE_PX;

/// How the tile atlas packs tiles into a 2D-array texture, chosen from the
/// device limits: `cols`×`cols` tiles per `layer_px`-square layer. The layout is
/// as compact as the historical 16×16/1024² default for small tile counts, and
/// grows the layer (doubling `cols`) only when 16 cols would need more array
/// layers than the hardware allows — so it fits the GPU's real
/// `max_texture_dimension_2d` / `max_texture_array_layers`.
#[derive(Clone, Copy)]
struct AtlasLayout {
	/// Tiles per row within a layer (a power of two, ≥ 16 where the hw allows).
	cols: u32,
	/// `cols * cols` — tiles per array layer.
	tiles_per_layer: u32,
	/// `cols * TILE_PX` — the square layer's side in px.
	layer_px: u32,
	/// Array layers needed to hold `total` tiles.
	layers: u32,
}

/// Pick an atlas layout for `total` tiles within `limits`.
fn atlas_layout(total: u32, limits: &wgpu::Limits) -> AtlasLayout {
	let max_cols = (limits.max_texture_dimension_2d / TILE_PX).max(1);
	let max_layers = limits.max_texture_array_layers.max(1);
	let total = total.max(1);
	// Start at the historical 16 cols (1024px / 256 tiles per layer); double the
	// layer only when that packing would exceed the array-layer limit.
	let mut cols = 16u32.min(max_cols);
	while total.div_ceil(cols * cols) > max_layers && cols < max_cols {
		cols = (cols * 2).min(max_cols);
	}
	let tiles_per_layer = cols * cols;
	AtlasLayout { cols, tiles_per_layer, layer_px: cols * TILE_PX, layers: total.div_ceil(tiles_per_layer).max(1) }
}

/// The device-lifetime half of the project/picker render path: the two bind
/// group layouts and the two pipelines (with their shaders), which are
/// identical for every project — the shaders read the per-project atlas
/// packing from uniforms (F7), so nothing here depends on the document. Built
/// once per (device, target format) and shared by every [`ProjectRenderer`]
/// through an `Rc`: rebuilding them per tab switch was the measured ~16 ms
/// fixed cost of `make_renderer` (OPTIMIZATION-BACKLOG.md, tab-switch note).
pub struct RenderCore {
	bgl: wgpu::BindGroupLayout,
	pipeline: wgpu::RenderPipeline,
	picker_bgl: wgpu::BindGroupLayout,
	picker_pipeline: wgpu::RenderPipeline,
}

impl RenderCore {
	pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> std::rc::Rc<RenderCore> {
		let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
				wgpu::BindGroupLayoutEntry {
					binding: 6,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Uint,
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
					count: None,
				},
			],
		});
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("project.shader"),
			source: wgpu::ShaderSource::Wgsl(include_str!("shaders/project.wgsl").into()),
		});
		let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("project.layout"),
			bind_group_layouts: &[Some(&bgl)],
			immediate_size: 0,
		});
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("project.pipeline"),
			layout: Some(&pipeline_layout),
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
			multiview_mask: None,
			cache: None,
		});

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
					binding: 3,
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
						sample_type: wgpu::TextureSampleType::Float { filterable: false },
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
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
					count: None,
				},
			],
		});
		let picker_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("picker.shader"),
			source: wgpu::ShaderSource::Wgsl(include_str!("shaders/picker.wgsl").into()),
		});
		let picker_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("picker.layout"),
			bind_group_layouts: &[Some(&picker_bgl)],
			immediate_size: 0,
		});
		let picker_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("picker.pipeline"),
			layout: Some(&picker_layout),
			vertex: wgpu::VertexState {
				module: &picker_shader,
				entry_point: Some("vs_main"),
				compilation_options: Default::default(),
				buffers: &[Some(PickerVertex::layout())],
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
			multiview_mask: None,
			cache: None,
		});

		std::rc::Rc::new(RenderCore { bgl, pipeline, picker_bgl, picker_pipeline })
	}
}

pub struct ProjectRenderer {
	core: std::rc::Rc<RenderCore>,
	bind_group: wgpu::BindGroup,
	uniforms_buffer: wgpu::Buffer,
	palette_texture: wgpu::Texture,
	cells_texture: wgpu::Texture,
	/// Per-cell pass value (R8Uint) for the pass overlay.
	pass_texture: wgpu::Texture,
	/// Overlay enable flag (uniform), written per draw.
	overlay_buffer: wgpu::Buffer,
	/// The hardware-chosen atlas packing (both shaders read it to address tiles).
	layout: AtlasLayout,
	/// Global atlas base index per pack (parallel to `project.packs`).
	pack_base: Vec<u32>,
	/// Tile Explorer grid pass - shares the atlas + palette.
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
	/// Whole-quad opacity - 1.0 for panels, <1 for the ghost-stamp preview.
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
	pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, project: &Project, core: &std::rc::Rc<RenderCore>) -> Self {
		// --- Tile atlas: all packs' tiles, globally indexed -------------
		let total_tiles: u32 = project.packs.iter().map(|p| p.tile_count() as u32).sum();
		let layout = atlas_layout(total_tiles, &device.limits());
		let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("project.atlas"),
			size: wgpu::Extent3d {
				width: layout.layer_px,
				height: layout.layer_px,
				depth_or_array_layers: layout.layers,
			},
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
				let slot = global % layout.tiles_per_layer;
				queue.write_texture(
					wgpu::TexelCopyTextureInfo {
						texture: &atlas_texture,
						mip_level: 0,
						origin: wgpu::Origin3d {
							x: (slot % layout.cols) * TILE_PX,
							y: (slot / layout.cols) * TILE_PX,
							z: global / layout.tiles_per_layer,
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

		// --- Per-tile transparency mask (R16Uint, atlas-indexed) -------
		// `0` = opaque family; else the family's mask color + 1. The shader
		// reads it for the ground tile to decide which pixels fall through. A
		// 256-wide table mirrors the atlas slot/layer split (index -> x=idx&255,
		// y=idx>>8), so it's never updated per-edit (masks are pack data).
		let mask_data = build_tile_mask_data(project);
		let mask_h = (mask_data.len() / MASK_W) as u32;
		let tile_mask_texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("project.tile_mask"),
			size: wgpu::Extent3d { width: MASK_W as u32, height: mask_h, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::R16Uint,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		queue.write_texture(
			tile_mask_texture.as_image_copy(),
			bytemuck::cast_slice(&mask_data),
			wgpu::TexelCopyBufferLayout {
				offset: 0,
				bytes_per_row: Some(MASK_W as u32 * 2),
				rows_per_image: Some(mask_h),
			},
			wgpu::Extent3d { width: MASK_W as u32, height: mask_h, depth_or_array_layers: 1 },
		);

		let uniforms_buffer = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("project.uniforms"),
			size: std::mem::size_of::<Uniforms>() as u64,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
		// Pass-overlay enable flag (16 bytes - uniform min size).
		let overlay_buffer = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("project.overlay"),
			size: 16,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});

		let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("project.bg"),
			layout: &core.bgl,
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
				wgpu::BindGroupEntry {
					binding: 6,
					resource: wgpu::BindingResource::TextureView(&tile_mask_texture.create_view(&Default::default())),
				},
			],
		});

		// --- Tile Explorer grid pass: atlas + palette + atlas layout ---------
		// The atlas packing (cols, tiles/layer) - constant for this renderer, so
		// written once. The project pass reads the same values from `overlay`.
		let atlas_meta_buffer = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("picker.atlas_meta"),
			size: 16,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
		let atlas_meta: [u32; 4] = [layout.cols, layout.tiles_per_layer, 0, 0];
		queue.write_buffer(&atlas_meta_buffer, 0, bytemuck::cast_slice(&atlas_meta));
		let picker_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("picker.bg"),
			layout: &core.picker_bgl,
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
				wgpu::BindGroupEntry {
					binding: 2,
					resource: wgpu::BindingResource::TextureView(&tile_mask_texture.create_view(&Default::default())),
				},
				wgpu::BindGroupEntry { binding: 3, resource: atlas_meta_buffer.as_entire_binding() },
			],
		});
		Self {
			core: core.clone(),
			bind_group,
			uniforms_buffer,
			palette_texture,
			cells_texture,
			pass_texture,
			overlay_buffer,
			layout,
			pack_base,
			picker_bind_group,
		}
	}

	/// Draw the Tile Explorer grid: one quad per visible tile, clipped to
	/// the panel body via the scissor rect.
	/// `alpha` scales every quad's opacity - 1.0 for panel content, lower for
	/// the ghost-stamp preview riding under the cursor.
	pub fn draw_picker(
		&self,
		device: &wgpu::Device,
		encoder: &mut wgpu::CommandEncoder,
		target: &wgpu::TextureView,
		tiles: &[TileQuad],
		scissor: Rect,
		screen: (u32, u32),
		scale: f32,
		alpha: f32,
	) {
		if tiles.is_empty() {
			return;
		}
		// Panel tiles are laid out in **logical** px; project from the logical size
		// (physical / scale) and convert the scissor to **physical** px for the GPU.
		// Map-space callers (the ghost stamp) pass `scale = 1.0` → the original path.
		let (w, h) = (screen.0 as f32, screen.1 as f32);
		let (lw, lh) = (w / scale, h / scale);
		let sx = (scissor.x * scale).clamp(0.0, w) as u32;
		let sy = (scissor.y * scale).clamp(0.0, h) as u32;
		let sw = ((scissor.x + scissor.w) * scale).clamp(0.0, w) as u32 - sx;
		let sh = ((scissor.y + scissor.h) * scale).clamp(0.0, h) as u32 - sy;
		if sw == 0 || sh == 0 {
			return;
		}

		let nx = |x: f32| x / lw * 2.0 - 1.0;
		let ny = |y: f32| 1.0 - y / lh * 2.0;
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

		let mut pass = crate::render::load_pass(encoder, target, "picker.pass");
		pass.set_scissor_rect(sx, sy, sw, sh);
		pass.set_pipeline(&self.core.picker_pipeline);
		pass.set_bind_group(0, &self.picker_bind_group, &[]);
		pass.set_vertex_buffer(0, buffer.slice(..));
		pass.draw(0..verts.len() as u32, 0..1);
	}

	/// Re-upload only the sub-rectangles an edit touched (drained from
	/// [`Project::take_render_dirty`]): the cell-stack texture where tile stacks
	/// changed and the pass texture where the overlay changed. A paint/stamp/pass
	/// stroke touches a handful of cells, so this replaces the former
	/// whole-map rebuild + full re-upload that ran on every frame of a drag.
	pub fn update_cells(&self, queue: &wgpu::Queue, project: &Project, dirty: &RenderDirty) {
		let (w, h) = (project.width, project.height);
		if let Some((x0, y0, x1, y1)) = clamp_rect(dirty.cells, w, h) {
			let (rw, rh) = ((x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32);
			queue.write_texture(
				wgpu::TexelCopyTextureInfo {
					texture: &self.cells_texture,
					mip_level: 0,
					origin: wgpu::Origin3d { x: x0 as u32, y: y0 as u32, z: 0 },
					aspect: wgpu::TextureAspect::All,
				},
				bytemuck::cast_slice(&build_cell_data_rect(project, &self.pack_base, x0, y0, x1, y1)),
				wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(rw * 8), rows_per_image: Some(rh) },
				wgpu::Extent3d { width: rw, height: rh, depth_or_array_layers: 1 },
			);
		}
		if let Some((x0, y0, x1, y1)) = clamp_rect(dirty.pass, w, h) {
			let (rw, rh) = ((x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32);
			queue.write_texture(
				wgpu::TexelCopyTextureInfo {
					texture: &self.pass_texture,
					mip_level: 0,
					origin: wgpu::Origin3d { x: x0 as u32, y: y0 as u32, z: 0 },
					aspect: wgpu::TextureAspect::All,
				},
				&build_pass_data_rect(project, x0, y0, x1, y1),
				wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(rw), rows_per_image: Some(rh) },
				wgpu::Extent3d { width: rw, height: rh, depth_or_array_layers: 1 },
			);
		}
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
		layer_mask: u32,
	) {
		queue.write_buffer(&self.uniforms_buffer, 0, bytemuck::bytes_of(&uniforms));
		// The overlay uniform doubles as the atlas-layout carrier (see project.wgsl
		// `Overlay`): [pass-overlay on, layer mask, atlas cols, tiles per layer].
		let overlay: [u32; 4] = [pass_overlay as u32, layer_mask, self.layout.cols, self.layout.tiles_per_layer];
		queue.write_buffer(&self.overlay_buffer, 0, bytemuck::cast_slice(&overlay));
		// Load (not Clear): the app-background steel is drawn first and the
		// shader discards out-of-map fragments, so it shows through.
		let mut pass = crate::render::load_pass(encoder, target, "project.pass");
		pass.set_pipeline(&self.core.pipeline);
		pass.set_bind_group(0, &self.bind_group, &[]);
		pass.draw(0..3, 0..1);
	}
}

/// Clamp an inclusive cell bbox to the current map, or `None` if it can't be
/// uploaded (empty map, or a stale region whose origin now lies outside a
/// since-shrunk map - the belt-and-suspenders guard against an out-of-bounds
/// upload; a fresh renderer also clears the dirty region).
fn clamp_rect(rect: Option<(u16, u16, u16, u16)>, w: u16, h: u16) -> Option<(u16, u16, u16, u16)> {
	let (x0, y0, x1, y1) = rect?;
	if w == 0 || h == 0 || x0 >= w || y0 >= h {
		return None;
	}
	Some((x0, y0, x1.min(w - 1), y1.min(h - 1)))
}

/// Per-cell pass value (0 land / 1 water / 2 shore / 3 blocked) for the
/// overlay; a cell whose top tile has no pass data reads as land (0). Full map.
fn build_pass_data(project: &Project) -> Vec<u8> {
	let (w, h) = (project.width, project.height);
	if w == 0 || h == 0 {
		return Vec::new();
	}
	build_pass_data_rect(project, 0, 0, w - 1, h - 1)
}

/// [`build_pass_data`] for the inclusive cell sub-rectangle `(x0,y0)..=(x1,y1)`,
/// row-major - the partial re-upload after a pass edit.
fn build_pass_data_rect(project: &Project, x0: u16, y0: u16, x1: u16, y1: u16) -> Vec<u8> {
	let (rw, rh) = ((x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
	let mut out = Vec::with_capacity(rw * rh);
	for y in y0..=y1 {
		for x in x0..=x1 {
			out.push(project.pass_at(x, y).unwrap_or(0));
		}
	}
	out
}

/// Width of the per-tile mask table (also its index split: x = idx & 255,
/// y = idx >> 8 - the same slot/layer split the atlas uses).
const MASK_W: usize = 256;

/// The per-tile transparency mask table in global atlas-index order: `0` for an
/// opaque family, else its mask color + 1. Padded to a full `MASK_W`-wide grid.
fn build_tile_mask_data(project: &Project) -> Vec<u16> {
	let total: usize = project.packs.iter().map(|p| p.tile_count() as usize).sum();
	let height = total.div_ceil(MASK_W).max(1);
	let mut data = vec![0u16; MASK_W * height];
	let mut gi = 0usize;
	for pack in &project.packs {
		for tile in 0..pack.tile_count() {
			data[gi] = pack.tile_mask(tile).map_or(0, |m| m as u16 + 1);
			gi += 1;
		}
	}
	data
}

/// The cell-stack texture bytes for the whole map (used at renderer build).
fn build_cell_data(project: &Project, pack_base: &[u32]) -> Vec<u16> {
	let (w, h) = (project.width, project.height);
	if w == 0 || h == 0 {
		return Vec::new();
	}
	build_cell_data_rect(project, pack_base, 0, 0, w - 1, h - 1)
}

/// [`build_cell_data`] for the inclusive cell sub-rectangle `(x0,y0)..=(x1,y1)`,
/// row-major (r=water idx+1, g=water transform, b=ground idx+1, a=ground
/// transform) - the partial re-upload after a tile edit.
fn build_cell_data_rect(project: &Project, pack_base: &[u32], x0: u16, y0: u16, x1: u16, y1: u16) -> Vec<u16> {
	let w = project.width as usize;
	let (rw, rh) = ((x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
	let mut cell_data = Vec::with_capacity(rw * rh * 4);
	for y in y0..=y1 {
		for x in x0..=x1 {
			let stack = &project.cells[y as usize * w + x as usize];
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
	}
	cell_data
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::picker::TileQuad;
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

	/// `draw_picker` paints tile quads clipped to the scissor: a quad inside
	/// shows the pack's tile art over the black clear, a quad below the
	/// scissor stays black, and the empty-tiles / zero-area-scissor calls are
	/// guarded no-ops.
	#[test]
	fn draw_picker_clips_to_scissor_and_guards_degenerates() {
		let (device, queue, _serial) = crate::visual_test::test_gpu();
		let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/assets/tilepacks");
		let project = Project::new(8, 6, &["GREEN".to_string()], &root, 42).expect("GREEN project");
		let renderer =
			ProjectRenderer::new(&device, &queue, &project, &RenderCore::new(&device, crate::capture::FORMAT));
		// The busiest tile of the first pack (most non-zero palette indices):
		// guaranteed-visible art whatever the pack's tile order.
		let pack = &project.packs[0];
		let tile = (0..pack.tile_count())
			.max_by_key(|&t| pack.tile_pixels(t).iter().filter(|&&p| p != 0).count())
			.expect("pack has tiles");
		let quad = |x: f32, y: f32| TileQuad { index: tile as u32, transform: 0, rect: Rect::new(x, y, 64.0, 64.0) };
		let (w, h) = (128u32, 128u32);
		let scissor = Rect::new(0.0, 0.0, 128.0, 64.0);
		let dir = std::env::temp_dir().join(format!("max-map-editor-picker-{}", std::process::id()));

		// The guarded no-ops: no tiles, then a zero-width scissor.
		let noop = dir.join("picker-noop.png");
		crate::capture::render_to_png(&device, &queue, w, h, &noop, None, None, |encoder, view| {
			clear(encoder, view, wgpu::Color::BLACK);
			renderer.draw_picker(&device, encoder, view, &[], scissor, (w, h), 1.0, 1.0);
			let none = Rect::new(0.0, 0.0, 0.0, 64.0);
			renderer.draw_picker(&device, encoder, view, &[quad(0.0, 0.0)], none, (w, h), 1.0, 1.0);
		});
		let (rgba, ..) = read_png(&noop);
		assert!(rgba.chunks_exact(4).all(|p| p == [0, 0, 0, 255]), "degenerate calls draw nothing");

		// A real draw: one quad inside the scissor, one below it.
		let shot = dir.join("picker.png");
		crate::capture::render_to_png(&device, &queue, w, h, &shot, None, None, |encoder, view| {
			clear(encoder, view, wgpu::Color::BLACK);
			renderer.draw_picker(&device, encoder, view, &[quad(0.0, 0.0), quad(0.0, 64.0)], scissor, (w, h), 1.0, 1.0);
		});
		let (rgba, ..) = read_png(&shot);
		let lit_rows = |lo: u32, hi: u32| {
			(lo..hi)
				.flat_map(|y| (0..w).map(move |x| ((y * w + x) * 4) as usize))
				.filter(|&i| rgba[i..i + 3] != [0, 0, 0])
				.count()
		};
		let top = lit_rows(0, 64);
		assert!(top > 300, "tile art inside the scissor rendered ({top} lit px)");
		assert_eq!(lit_rows(64, 128), 0, "the quad below the scissor is clipped out");
		let _ = std::fs::remove_dir_all(&dir);
	}
}
