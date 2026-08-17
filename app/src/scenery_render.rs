//! GPU half of scenery (`SCENERY.md` stage B): one `Rgba8Uint` atlas holding
//! every cut-out the open project's libraries ship, plus a quad pipeline that
//! samples atlas -> working palette (`shaders/scenery.wgsl`).
//!
//! A sprite's whole cut rides in one texel - `r` = body palette index, `g` =
//! shadow alpha, `b` = how deep inside its own silhouette the pixel sits, `a` =
//! how high the object stands there - so a placement is a single quad, not a
//! shadow quad plus a body quad the way a unit is. `r` and `g` are mutually
//! exclusive by construction; `b` and `a` are derived here from the body plane
//! rather than shipped, so a library baked before blending existed still blends.
//!
//! ## Two objects can meet by height
//!
//! `higher` keeps the ink of whichever object stands taller at a pixel, which is
//! a *condition* - one value deciding whether another is written - and no blend
//! function expresses that. So the ink layer carries a real depth buffer holding
//! `height / 255`, `higher` draws with `GreaterEqual`, and the hardware settles
//! it. Every other mode writes its height without testing, so the layer always
//! describes the object whose ink is actually there.
//!
//! The relief is **inferred from the art** (`Sprite::height_field`, and
//! DESIGN.md 4.4 for what that means and what it costs), and its
//! luma term reads the palette - the one this atlas was built with. A palette
//! edit afterwards re-ranks `brighter` / `darker` (those recompute per upload)
//! but does not re-infer relief, while `bake.rs` derives it from the project's
//! palette at export time. The two can therefore disagree along the contour
//! where two objects stand within a pixel of equally high, and nowhere else;
//! re-opening the project rebuilds the atlas and the disagreement with it.
//!
//! ## The shadows are one layer
//!
//! Drawing each quad's shade straight onto the map darkened the ground once per
//! placement, so two objects standing close together stamped a black blot where
//! their shadows crossed. Instead [`SceneryGpu::draw`] puts every placement's
//! shade into a screen-sized R8 layer under `max` blending ([`ShadowTarget`]),
//! that layer darkens the map exactly once, and the objects' own ink draws over
//! it. Overlapping shadows therefore merge at the deeper of their two alphas,
//! and no object is dimmed by its neighbour's shadow - `bake.rs` composites the
//! WRL export by the same rule, so the screen and the export agree.
//!
//! ## A piece dithers into its own family
//!
//! Two mountains laid over each other are two faces of one landform, so the
//! newer one's rim grades into the older instead of cutting a silhouette out of
//! it ([`map_core::blend_keeps`]). That needs to know, per pixel, whether a
//! *earlier* piece of the same family is underneath, which no fragment can read
//! off the target it is drawing into - so [`FamilyTarget`] holds one screen-sized
//! layer per blending family, filled under `min` blending with the earliest draw
//! order covering each pixel. A family earns a layer only when two of its quads
//! actually overlap ([`family_layers`]).
//!
//! The four passes are therefore: shade -> family coverage -> one darkening ->
//! the ink.
//!
//! The atlas is shelf-packed and its height is measured, not fixed: a scenery
//! sprite runs from a few dozen pixels to the 1599x1055 SNOW_DARK cliff, so a
//! fixed slot size like the units atlas uses would waste most of the texture.
//! Sprites pack tallest-first, and the texture is allocated to exactly the
//! height that used.
//!
//! Rebuilt when the loaded libraries change (opening a project on another tile
//! pack), which [`SceneryGpu::signature`] detects without holding a reference
//! to the document.

use std::collections::HashMap;

use wgpu::util::DeviceExt;

use crate::ui::Rect;
use map_core::{Project, SceneryBlend, SceneryPack};

/// Atlas width. Wide enough for the widest shipped sprite (1599px) with room to
/// shelve smaller ones beside it; the height follows from the packing.
const ATLAS_W: u32 = 4096;
/// Height ceiling, so a corrupt or hand-made library cannot ask for a texture
/// no device will allocate. The shipped packs need ~1.5k rows.
const ATLAS_H_MAX: u32 = 4096;

/// The merged shadow layer's format: one alpha channel, nothing else needed.
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// The merged ink layer's format: one **brightness rank** per pixel
/// (`map_core::ink_ranks`), `0` = no scenery. A byte is exactly enough - 255
/// inks plus the empty value - and `R8Unorm` is blendable everywhere, which a
/// float format is not.
const INK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// The **height layer**, which is a real depth buffer: `higher` keeps the ink of
/// whichever object stands taller at a pixel, and "write my colour only if my
/// value beats the stored one" is a depth test, not a blend. A blend function
/// cannot express it - that is exactly why `brighter` had to become `max` over
/// brightness *ranks* - and the hardware does it for free.
///
/// `Depth32Float` because it is the one depth format every backend guarantees;
/// the values it carries are `height / 255`.
const HEIGHT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Where one sprite landed in the atlas.
#[derive(Clone, Copy)]
pub struct Slot {
	pub origin: (u32, u32),
	pub size: (u32, u32),
}

/// One quad to draw: a screen rect and the atlas sprite that fills it.
pub struct SceneryQuad {
	pub rect: Rect,
	pub origin: (u32, u32),
	pub sprite: (u32, u32),
	/// `1.0` for a placed object, less for the placement tool's ghost. Per quad
	/// rather than per call so the ghost can ride along with the placements and
	/// its shadow merge with theirs - a preview that double-darkened where it
	/// crossed one would show an artifact the placement then wouldn't have.
	pub alpha: f32,
	/// How this quad's ink meets the ink already in the layer. A placement picks
	/// the pipeline for its run; the ghost never enters the layer, so it carries
	/// its mode into the shader instead and applies it against what it reads.
	pub blend: SceneryBlend,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SceneryVertex {
	pos: [f32; 2],
	uv: [f32; 2],
	origin: [u32; 2],
	alpha: f32,
	/// The quad's blend mode as an index into [`SceneryBlend::ALL`]. Read only
	/// by the ghost pass - a placement's mode is its pipeline's blend op.
	mode: u32,
}

pub struct SceneryGpu {
	/// One quad's own ink straight onto the map, blended by hand against the ink
	/// layer it reads - the ghost (pass 6).
	ghost_pipeline: wgpu::RenderPipeline,
	/// Every placement's shade, `max`-blended into the shadow layer (pass 1).
	shadow_pipeline: wgpu::RenderPipeline,
	/// A placement's ink into the ink layer, one pipeline per blend mode, in
	/// [`SceneryBlend::ALL`] order - replace, `max`, `min`, and the depth-tested
	/// one. The replace one is also the base pass (2) that seeds the layer.
	ink_pipelines: [wgpu::RenderPipeline; 4],
	/// One full-viewport darkening through the shadow layer (pass 4).
	merge_pipeline: wgpu::RenderPipeline,
	/// ...and one painting of the ink layer over it (pass 5).
	resolve_pipeline: wgpu::RenderPipeline,
	bind_group: wgpu::BindGroup,
	/// Kept to rebuild [`Layers::bind_group`] whenever the screen resizes.
	layer_layout: wgpu::BindGroupLayout,
	layers: Option<Layers>,
	palette_texture: wgpu::Texture,
	rank_texture: wgpu::Texture,
	slots: HashMap<(String, String), Slot>,
	signature: String,
}

/// The two screen-sized merge layers, cleared and rebuilt per draw call so a
/// call's own placements merge with each other and nothing leaks between the
/// map, the panel thumbnails and a later frame.
///
/// * `shadow` - the deepest shade alpha any placement casts on each pixel.
/// * `ink` - the brightness rank the placements' blend modes settle on.
struct Layers {
	shadow_view: wgpu::TextureView,
	ink_view: wgpu::TextureView,
	/// The depth buffer the ink passes test `higher` against - see
	/// [`HEIGHT_FORMAT`]. Bound as a texture only by the ghost, which is drawn
	/// after those passes have let go of it.
	height_view: wgpu::TextureView,
	bind_group: wgpu::BindGroup,
	size: (u32, u32),
}

/// What the atlas was built from: the loaded libraries and how many pieces each
/// holds. Cheap to compute every frame, and it changes exactly when a rebuild
/// is due.
pub fn signature(packs: &[SceneryPack]) -> String {
	packs.iter().map(|p| format!("{}:{}", p.pack, p.pieces.len())).collect::<Vec<_>>().join(",")
}

impl SceneryGpu {
	/// Build the atlas for `packs`. `None` when they hold nothing to draw - the
	/// caller then has no pass to keep, and retries if a project with scenery
	/// opens later.
	pub fn new(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		packs: &[SceneryPack],
		format: wgpu::TextureFormat,
		palette_rgba: &[u8],
	) -> Option<Self> {
		// ---- shelf-pack, tallest first, and measure the height used ----------
		let mut order: Vec<(&SceneryPack, usize)> =
			packs.iter().flat_map(|p| (0..p.pieces.len()).map(move |i| (p, i))).collect();
		order.sort_by_key(|(p, i)| std::cmp::Reverse(p.pieces[*i].sprite.height));

		let mut slots: HashMap<(String, String), Slot> = HashMap::new();
		let (mut shelf_y, mut shelf_h, mut pen_x) = (0u32, 0u32, 0u32);
		let mut dropped = 0usize;
		for (pack, i) in &order {
			let piece = &pack.pieces[*i];
			let (w, h) = (piece.sprite.width as u32, piece.sprite.height as u32);
			if w == 0 || h == 0 || w > ATLAS_W {
				dropped += 1;
				continue;
			}
			if pen_x + w > ATLAS_W {
				shelf_y += shelf_h;
				shelf_h = 0;
				pen_x = 0;
			}
			if shelf_y + h > ATLAS_H_MAX {
				dropped += 1;
				continue;
			}
			slots.insert((pack.pack.clone(), piece.id.clone()), Slot { origin: (pen_x, shelf_y), size: (w, h) });
			pen_x += w;
			shelf_h = shelf_h.max(h);
		}
		if slots.is_empty() {
			return None;
		}
		if dropped > 0 {
			eprintln!("scenery atlas: {dropped} sprite(s) did not fit and will not draw");
		}
		let atlas_h = (shelf_y + shelf_h).max(1);

		// ---- blit the four planes into one Rgba8Uint texel each ---------------
		// The edge distance and the height field are derived here rather than
		// shipped: both follow from the body plane, so a library baked before
		// either existed still blends.
		let rgb: Vec<u8> = palette_rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
		let brightness = map_core::brightness_table(&rgb);
		let mut pixels = vec![0u8; (ATLAS_W * atlas_h) as usize * 4];
		for pack in packs {
			for piece in &pack.pieces {
				let Some(slot) = slots.get(&(pack.pack.clone(), piece.id.clone())) else { continue };
				let w = piece.sprite.width as usize;
				let edge = piece.sprite.edge_distance();
				let height = piece.height_field(&brightness);
				for y in 0..piece.sprite.height as usize {
					for x in 0..w {
						let at = ((slot.origin.1 as usize + y) * ATLAS_W as usize + slot.origin.0 as usize + x) * 4;
						pixels[at] = piece.sprite.body[y * w + x];
						pixels[at + 1] = piece.sprite.shade[y * w + x];
						pixels[at + 2] = edge[y * w + x];
						pixels[at + 3] = height[y * w + x];
					}
				}
			}
		}

		let atlas_texture = device.create_texture_with_data(
			queue,
			&wgpu::TextureDescriptor {
				label: Some("scenery.atlas"),
				size: wgpu::Extent3d { width: ATLAS_W, height: atlas_h, depth_or_array_layers: 1 },
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: wgpu::TextureFormat::Rgba8Uint,
				usage: wgpu::TextureUsages::TEXTURE_BINDING,
				view_formats: &[],
			},
			wgpu::util::TextureDataOrder::LayerMajor,
			&pixels,
		);

		let palette_texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("scenery.palette"),
			size: wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rgba8UnormSrgb,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		// Row 0 = an ink's brightness rank, row 1 = the ink of a rank. Both
		// follow the working palette, so both are re-uploaded with it.
		let rank_texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("scenery.ranks"),
			size: wgpu::Extent3d { width: 256, height: 2, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::R8Uint,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});

		let texture_entry = |binding: u32, uint: bool| wgpu::BindGroupLayoutEntry {
			binding,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Texture {
				sample_type: if uint {
					wgpu::TextureSampleType::Uint
				} else {
					wgpu::TextureSampleType::Float { filterable: false }
				},
				view_dimension: wgpu::TextureViewDimension::D2,
				multisampled: false,
			},
			count: None,
		};
		let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("scenery.bg_layout"),
			entries: &[texture_entry(0, true), texture_entry(1, false), texture_entry(2, true)],
		});
		let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
		let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("scenery.bg"),
			layout: &bgl,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(&view(&atlas_texture)),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::TextureView(&view(&palette_texture)),
				},
				wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&view(&rank_texture)) },
			],
		});

		// The shadow layer at binding 0 and the ink layer at 1, in one group -
		// they are the *targets* of the quad passes, so only the two resolve
		// pipelines declare it, and each reads just its own.
		let layer_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("scenery.layer_layout"),
			entries: &[
				texture_entry(0, false),
				texture_entry(1, false),
				// The height layer, for the ghost's hand-run `higher`. A depth
				// texture samples as `Depth`, not as a plain float.
				wgpu::BindGroupLayoutEntry {
					binding: 2,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Depth,
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
					count: None,
				},
			],
		});

		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("scenery.shader"),
			source: wgpu::ShaderSource::Wgsl(include_str!("shaders/scenery.wgsl").into()),
		});
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("scenery.layout"),
			bind_group_layouts: &[Some(&bgl)],
			immediate_size: 0,
		});
		// The layer group as well: the two full-viewport resolves read it, and so
		// does the ghost quad pass, which blends against the ink layer by hand.
		let resolve_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("scenery.resolve_layout"),
			bind_group_layouts: &[Some(&bgl), Some(&layer_layout)],
			immediate_size: 0,
		});
		// The quad pipelines differ only in what they write and how it blends:
		// the ghost's ink onto the map, the shade into the shadow layer under
		// `max`, or a brightness rank into the ink layer under one blend mode.
		let quads = |label: &str,
		             entry: &str,
		             target: wgpu::ColorTargetState,
		             pl: &wgpu::PipelineLayout,
		             depth: Option<wgpu::DepthStencilState>| {
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(label),
				layout: Some(pl),
				vertex: wgpu::VertexState {
					module: &shader,
					entry_point: Some("vs_main"),
					compilation_options: Default::default(),
					buffers: &[Some(wgpu::VertexBufferLayout {
						array_stride: std::mem::size_of::<SceneryVertex>() as u64,
						step_mode: wgpu::VertexStepMode::Vertex,
						attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32x2, 3 => Float32, 4 => Uint32],
					})],
				},
				fragment: Some(wgpu::FragmentState {
					module: &shader,
					entry_point: Some(entry),
					compilation_options: Default::default(),
					targets: &[Some(target)],
				}),
				primitive: wgpu::PrimitiveState::default(),
				depth_stencil: depth,
				multisample: wgpu::MultisampleState::default(),
				multiview_mask: None,
				cache: None,
			})
		};
		let ghost_pipeline = quads(
			"scenery.ghost_pipeline",
			"fs_ghost",
			wgpu::ColorTargetState {
				format,
				blend: Some(wgpu::BlendState::ALPHA_BLENDING),
				write_mask: wgpu::ColorWrites::ALL,
			},
			&resolve_layout,
			None,
		);
		// A min/max operation ignores its factors, but wgpu still insists they
		// both be `One`; only `Add` reads them, and there `One`/`Zero` is the
		// plain replace the `normal` mode wants.
		let op = |operation| wgpu::BlendComponent {
			src_factor: wgpu::BlendFactor::One,
			dst_factor: if operation == wgpu::BlendOperation::Add {
				wgpu::BlendFactor::Zero
			} else {
				wgpu::BlendFactor::One
			},
			operation,
		};
		// `max`, not `over`: the shadow layer holds the deepest shadow any one
		// placement casts on a pixel, which is what makes two of them merge
		// rather than compound.
		let deepest = wgpu::BlendComponent {
			src_factor: wgpu::BlendFactor::One,
			dst_factor: wgpu::BlendFactor::One,
			operation: wgpu::BlendOperation::Max,
		};
		let shadow_pipeline = quads(
			"scenery.shadow_pipeline",
			"fs_shadow",
			wgpu::ColorTargetState {
				format: SHADOW_FORMAT,
				blend: Some(wgpu::BlendState { color: deepest, alpha: deepest }),
				write_mask: wgpu::ColorWrites::RED,
			},
			&layout,
			None,
		);
		// One per blend mode, in `SceneryBlend::ALL` order. Because the layer
		// holds a brightness *rank*, "keep the brighter ink" is exactly `max`.
		//
		// The depth side is what settles `higher`: it alone is *tested*, and
		// `GreaterEqual` because a tie keeps the ink being painted, exactly as
		// `SceneryBlend::pick` does. Every other mode writes its height without
		// testing, so the layer always holds the height of the object whose ink
		// is actually there and a later `higher` placement compares with the
		// right one - the rule `bake.rs` mirrors for the export.
		let ink_pipelines = [
			(SceneryBlend::Normal, wgpu::BlendOperation::Add, wgpu::CompareFunction::Always),
			(SceneryBlend::Brighter, wgpu::BlendOperation::Max, wgpu::CompareFunction::Always),
			(SceneryBlend::Darker, wgpu::BlendOperation::Min, wgpu::CompareFunction::Always),
			(SceneryBlend::Higher, wgpu::BlendOperation::Add, wgpu::CompareFunction::GreaterEqual),
		]
		.map(|(mode, operation, depth_compare)| {
			quads(
				&format!("scenery.ink_pipeline.{}", mode.name()),
				"fs_ink",
				wgpu::ColorTargetState {
					format: INK_FORMAT,
					blend: Some(wgpu::BlendState { color: op(operation), alpha: op(operation) }),
					write_mask: wgpu::ColorWrites::RED,
				},
				&layout,
				Some(wgpu::DepthStencilState {
					format: HEIGHT_FORMAT,
					depth_write_enabled: Some(true),
					depth_compare: Some(depth_compare),
					stencil: wgpu::StencilState::default(),
					bias: wgpu::DepthBiasState::default(),
				}),
			)
		});

		// Both full-viewport passes: one darkens the map through the shadow
		// layer, the other paints the ink layer over it.
		let resolves = |label: &str, entry: &str| {
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(label),
				layout: Some(&resolve_layout),
				vertex: wgpu::VertexState {
					module: &shader,
					entry_point: Some("vs_full"),
					compilation_options: Default::default(),
					buffers: &[],
				},
				fragment: Some(wgpu::FragmentState {
					module: &shader,
					entry_point: Some(entry),
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
			})
		};
		let merge_pipeline = resolves("scenery.merge_pipeline", "fs_merge");
		let resolve_pipeline = resolves("scenery.resolve_pipeline", "fs_resolve");

		let gpu = Self {
			ghost_pipeline,
			shadow_pipeline,
			ink_pipelines,
			merge_pipeline,
			resolve_pipeline,
			bind_group,
			layer_layout,
			layers: None,
			palette_texture,
			rank_texture,
			slots,
			signature: signature(packs),
		};
		gpu.update_palette(queue, palette_rgba);
		Some(gpu)
	}

	/// (Re)allocate the shadow and ink layers at the target's physical size.
	fn ensure_layers(&mut self, device: &wgpu::Device, size: (u32, u32)) {
		if self.layers.as_ref().is_some_and(|l| l.size == size) {
			return;
		}
		let plane = |label: &str, format| {
			device
				.create_texture(&wgpu::TextureDescriptor {
					label: Some(label),
					size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
					mip_level_count: 1,
					sample_count: 1,
					dimension: wgpu::TextureDimension::D2,
					format,
					usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
					view_formats: &[],
				})
				.create_view(&wgpu::TextureViewDescriptor::default())
		};
		let shadow_view = plane("scenery.shadow", SHADOW_FORMAT);
		let ink_view = plane("scenery.ink", INK_FORMAT);
		let height_view = plane("scenery.height", HEIGHT_FORMAT);
		let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("scenery.layer_bg"),
			layout: &self.layer_layout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&shadow_view) },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&ink_view) },
				wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&height_view) },
			],
		});
		self.layers = Some(Layers { shadow_view, ink_view, height_view, bind_group, size });
	}

	/// The library set this atlas was built from - compare against
	/// [`signature`] to know a rebuild is due.
	pub fn signature(&self) -> &str {
		&self.signature
	}

	pub fn slot(&self, pack: &str, piece: &str) -> Option<&Slot> {
		self.slots.get(&(pack.to_string(), piece.to_string()))
	}

	/// Re-upload the working palette (256 RGBA bytes) - call alongside the map
	/// renderer's palette update. The brightness ranks the blend modes sort by
	/// follow from the colours, so they are rebuilt here too.
	///
	/// Unlike units, **no** game statics are applied: scenery is terrain art cut
	/// from the pack's own tiles, so it has to read against the very palette the
	/// map renderer draws those tiles with, or a placed mountain would not match
	/// the ground beside it.
	pub fn update_palette(&self, queue: &wgpu::Queue, rgba: &[u8]) {
		queue.write_texture(
			self.palette_texture.as_image_copy(),
			rgba,
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 4), rows_per_image: Some(1) },
			wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
		);
		let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
		let (rank_of, ink_of) = map_core::ink_ranks(&rgb);
		let mut rows = rank_of.to_vec();
		rows.extend_from_slice(&ink_of);
		queue.write_texture(
			self.rank_texture.as_image_copy(),
			&rows,
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: Some(2) },
			wgpu::Extent3d { width: 256, height: 2, depth_or_array_layers: 1 },
		);
	}

	/// Draw scenery quads. `scissor` clips panel content (thumbnails); the map
	/// overlay passes `None` and `scale = 1.0`.
	///
	/// The last quad may be the placement tool's ghost (`ghost = true`): it casts
	/// its shadow with the rest, but stays out of the ink layer so it can draw
	/// translucent - a preview, not a placement.
	///
	/// Six passes - see the module doc. Every quad in one call merges with every
	/// other, and nothing carries over between calls (the layers are cleared per
	/// call).
	///
	/// Panel quads are laid out in **logical** px, so they project from the
	/// logical size (physical / scale) while the scissor converts the other way -
	/// the same split `units_render` makes.
	pub fn draw(
		&mut self,
		device: &wgpu::Device,
		encoder: &mut wgpu::CommandEncoder,
		target: &wgpu::TextureView,
		quads: &[SceneryQuad],
		ghost: bool,
		scissor: Option<Rect>,
		screen: (u32, u32),
		scale: f32,
	) {
		if quads.is_empty() || screen.0 == 0 || screen.1 == 0 {
			return;
		}
		let (pw, ph) = (screen.0 as f32, screen.1 as f32);
		let (w, h) = (pw / scale, ph / scale);
		let (sx, sy, sw, sh) = match scissor {
			Some(r) => {
				let sx = (r.x * scale).clamp(0.0, pw) as u32;
				let sy = (r.y * scale).clamp(0.0, ph) as u32;
				let sw = ((r.x + r.w) * scale).clamp(0.0, pw) as u32 - sx;
				let sh = ((r.y + r.h) * scale).clamp(0.0, ph) as u32 - sy;
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
			let (x0, y0, x1, y1) = (q.rect.x, q.rect.y, q.rect.x + q.rect.w, q.rect.y + q.rect.h);
			let (uw, uh) = (q.sprite.0 as f32, q.sprite.1 as f32);
			let mode = blend_index(q.blend) as u32;
			let v = |x: f32, y: f32, u: f32, vv: f32| SceneryVertex {
				pos: [nx(x), ny(y)],
				uv: [u, vv],
				origin: [q.origin.0, q.origin.1],
				alpha: q.alpha,
				mode,
			};
			let (tl, tr, br, bl) = (v(x0, y0, 0.0, 0.0), v(x1, y0, uw, 0.0), v(x1, y1, uw, uh), v(x0, y1, 0.0, uh));
			verts.extend_from_slice(&[tl, bl, br, tl, br, tr]);
		}
		let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
			label: Some("scenery.vertices"),
			contents: bytemuck::cast_slice(&verts),
			usage: wgpu::BufferUsages::VERTEX,
		});
		// The placements, in placement order, as runs of one blend mode - each
		// run is one draw with that mode's pipeline.
		let placed = quads.len() - usize::from(ghost);
		let mut runs: Vec<(usize, std::ops::Range<u32>)> = Vec::new();
		for (i, q) in quads[..placed].iter().enumerate() {
			let mode = blend_index(q.blend);
			let (from, to) = (i as u32 * 6, i as u32 * 6 + 6);
			match runs.last_mut() {
				Some((m, range)) if *m == mode && range.end == from => range.end = to,
				_ => runs.push((mode, from..to)),
			}
		}

		self.ensure_layers(device, screen);
		let layers = self.layers.as_ref().expect("allocated just above");
		let quad_pass = |encoder: &'_ mut wgpu::CommandEncoder, label, view, clear, depth| {
			let mut pass = layer_pass(encoder, label, view, clear, depth, (sx, sy, sw, sh));
			pass.set_bind_group(0, &self.bind_group, &[]);
			pass.set_vertex_buffer(0, buffer.slice(..));
			pass.forget_lifetime()
		};
		// 1. Every shade plane into the shadow layer, deepest wins. Cleared here
		// rather than once a frame: each call is its own merge group.
		{
			let mut pass = quad_pass(encoder, "scenery.shadow_pass", &layers.shadow_view, true, None);
			pass.set_pipeline(&self.shadow_pipeline);
			pass.draw(0..verts.len() as u32, 0..1);
		}
		// 2. Seed the ink layer with the *earliest* ink covering each pixel, by
		// drawing the placements in reverse with plain replace. Without it a
		// `darker` placement would meet a cleared 0 and lose to nothing - and a
		// `higher` one would compare its height against a cleared 0 rather than
		// against the object it is actually landing on. The height layer is
		// seeded in the same stroke, since the pipeline writes depth.
		{
			let mut pass =
				quad_pass(encoder, "scenery.ink_base_pass", &layers.ink_view, true, Some((&layers.height_view, true)));
			pass.set_pipeline(&self.ink_pipelines[0]);
			for i in (0..placed).rev() {
				pass.draw(i as u32 * 6..i as u32 * 6 + 6, 0..1);
			}
		}
		// 3. ...then the placements in order, each meeting what is already there
		// by its own blend mode.
		{
			let mut pass =
				quad_pass(encoder, "scenery.ink_pass", &layers.ink_view, false, Some((&layers.height_view, false)));
			for (mode, range) in &runs {
				pass.set_pipeline(&self.ink_pipelines[*mode]);
				pass.draw(range.clone(), 0..1);
			}
		}
		// 4. One darkening of the map through the shadow layer, and 5. the ink
		// layer painted over it.
		{
			let mut pass = crate::render::load_pass(encoder, target, "scenery.resolve_pass");
			pass.set_scissor_rect(sx, sy, sw, sh);
			pass.set_bind_group(0, &self.bind_group, &[]);
			pass.set_bind_group(1, &layers.bind_group, &[]);
			pass.set_pipeline(&self.merge_pipeline);
			pass.draw(0..3, 0..1);
			pass.set_pipeline(&self.resolve_pipeline);
			pass.draw(0..3, 0..1);
		}
		// 6. The ghost last, translucent, over the lot - reading the ink layer
		// (group 1) so its own blend mode meets the placements under it.
		if ghost {
			let mut pass = crate::render::load_pass(encoder, target, "scenery.ghost_pass");
			pass.set_scissor_rect(sx, sy, sw, sh);
			pass.set_pipeline(&self.ghost_pipeline);
			pass.set_bind_group(0, &self.bind_group, &[]);
			pass.set_bind_group(1, &layers.bind_group, &[]);
			pass.set_vertex_buffer(0, buffer.slice(..));
			pass.draw(placed as u32 * 6..verts.len() as u32, 0..1);
		}
	}
}

/// A blend mode as its index into [`SceneryBlend::ALL`] - which is also its
/// index into `ink_pipelines` and the number the ghost shader switches on.
fn blend_index(blend: SceneryBlend) -> usize {
	SceneryBlend::ALL.iter().position(|&m| m == blend).unwrap_or(0)
}

/// One quad pass into a merge layer, scissored like the rest of the call.
/// `depth` is the height layer and whether to clear it: `0.0` is bare ground, so
/// a cleared buffer lets anything through, and the ink base pass then replaces it
/// with the real heights before any comparison is made.
fn layer_pass<'a>(
	encoder: &'a mut wgpu::CommandEncoder,
	label: &str,
	view: &wgpu::TextureView,
	clear: bool,
	depth: Option<(&wgpu::TextureView, bool)>,
	(sx, sy, sw, sh): (u32, u32, u32, u32),
) -> wgpu::RenderPass<'a> {
	let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
		label: Some(label),
		color_attachments: &[Some(wgpu::RenderPassColorAttachment {
			view,
			resolve_target: None,
			ops: wgpu::Operations {
				load: if clear { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) } else { wgpu::LoadOp::Load },
				store: wgpu::StoreOp::Store,
			},
			depth_slice: None,
		})],
		depth_stencil_attachment: depth.map(|(view, clear)| wgpu::RenderPassDepthStencilAttachment {
			view,
			depth_ops: Some(wgpu::Operations {
				load: if clear { wgpu::LoadOp::Clear(0.0) } else { wgpu::LoadOp::Load },
				store: wgpu::StoreOp::Store,
			}),
			stencil_ops: None,
		}),
		timestamp_writes: None,
		occlusion_query_set: None,
		multiview_mask: None,
	});
	pass.set_scissor_rect(sx, sy, sw, sh);
	pass
}

/// The map-overlay quads for a project's placements, in placement order - a
/// later object draws over an earlier one, which is the order `bake.rs` paints
/// them in too, so the screen and the export agree.
///
/// A placement whose piece no library resolves, or whose sprite never reached
/// the atlas, is silently skipped: it stays in the document (and in the saved
/// file) but has nothing to draw.
pub fn map_quads(project: &Project, gpu: &SceneryGpu, pan: [f32; 2], zoom: f32) -> Vec<SceneryQuad> {
	project
		.scenery
		.iter()
		.filter_map(|spot| {
			let piece = project.scenery_piece(spot)?;
			let slot = gpu.slot(&spot.pack, &spot.piece)?;
			let (ox, oy) = piece.sprite_origin(spot);
			Some(SceneryQuad {
				rect: Rect::new(
					(ox as f32 - pan[0]) * zoom,
					(oy as f32 - pan[1]) * zoom,
					slot.size.0 as f32 * zoom,
					slot.size.1 as f32 * zoom,
				),
				origin: slot.origin,
				sprite: slot.size,
				alpha: 1.0,
				blend: spot.blend,
			})
		})
		.collect()
}

/// Thumbnail quads for the Scenery panel's grid: each piece fit-scaled into its
/// cell and centred, never magnified past 1:1 (a 40px boulder should read as a
/// boulder, not fill the well). `cells` is the window the grid widget reports,
/// so the wells, the sprites and the rings are laid out exactly once.
pub fn thumb_quads(project: &Project, gpu: &SceneryGpu, cells: &[(usize, Rect)]) -> Vec<SceneryQuad> {
	let mut quads = Vec::new();
	for &(i, cell) in cells {
		let Some((pack, piece)) = crate::scenery::piece_at(project, i) else { continue };
		let Some(slot) = gpu.slot(pack, &piece.id) else { continue };
		let (sw, sh) = (slot.size.0 as f32, slot.size.1 as f32);
		if sw <= 0.0 || sh <= 0.0 {
			continue;
		}
		let s = (cell.w / sw).min(cell.h / sh).min(1.0);
		let (dw, dh) = (sw * s, sh * s);
		quads.push(SceneryQuad {
			rect: Rect::new(cell.x + (cell.w - dw) * 0.5, cell.y + (cell.h - dh) * 0.5, dw, dh),
			origin: slot.origin,
			sprite: slot.size,
			alpha: 1.0,
			blend: SceneryBlend::Normal,
		});
	}
	quads
}

/// The ghost quad for the placement tool: the active piece under the cursor,
/// anchored the way a click would place it - `(px, py)` is the map pixel under
/// the cursor, and the piece hangs from its centre of mass exactly as
/// `Command::SceneryPlace` will drop it.
///
/// It rides in the *same* call as the placements, at [`GHOST_ALPHA`] - so its
/// shadow merges with theirs, and the preview shows exactly the ground the
/// placement will produce. `blend` is the mode a click would place with
/// ([`crate::state::EditorState::scenery_blend`]), so the preview meets the
/// scenery under it the way the placement will rather than always painting over.
pub fn ghost_quad(
	project: &Project,
	gpu: &SceneryGpu,
	active: usize,
	px: i32,
	py: i32,
	pan: [f32; 2],
	zoom: f32,
	blend: SceneryBlend,
) -> Option<SceneryQuad> {
	let (pack, piece) = crate::scenery::piece_at(project, active)?;
	let slot = gpu.slot(pack, &piece.id)?;
	let (fx, fy) = piece.centered_at(px, py);
	let (ox, oy) = (fx + piece.sprite.origin_x as i32, fy + piece.sprite.origin_y as i32);
	Some(SceneryQuad {
		rect: Rect::new(
			(ox as f32 - pan[0]) * zoom,
			(oy as f32 - pan[1]) * zoom,
			slot.size.0 as f32 * zoom,
			slot.size.1 as f32 * zoom,
		),
		origin: slot.origin,
		sprite: slot.size,
		alpha: GHOST_ALPHA,
		blend,
	})
}

/// How solid the placement tool's ghost draws - see [`ghost_quad`].
pub const GHOST_ALPHA: f32 = 0.55;
