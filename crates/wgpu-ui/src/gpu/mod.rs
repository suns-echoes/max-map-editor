//! The wgpu backend: turns a [`DrawList`] into draw calls.
//!
//! [`Renderer`] presents to a window surface (anything `Into<SurfaceTarget>`,
//! e.g. an `Arc<winit::window::Window>`). [`HeadlessRenderer`] renders offscreen
//! and reads the pixels back — the harness used by the test suite for automated
//! visual verification, so the whole pipeline is exercised without a window.
//!
//! Both share [`UiKit`]: one pipeline, one atlas, one uniform buffer. Drawing is
//! a single tessellation pass — every command becomes a quad in pixel space, and
//! clip regions become scissor rectangles. Consecutive quads under the same clip
//! batch into one draw call; a clip change starts a new batch, preserving
//! painter's order.

mod atlas;
mod block_on;
mod png;

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use bytemuck::{Pod, Zeroable};

use atlas::UiAtlas;
use block_on::block_on;

use crate::color::Rgba;
use crate::draw::{DrawCmd, DrawList, TexRect, TextureId};
use crate::geom::Rect;
use crate::text::Fonts;

/// Side length of the (square) UI atlas texture in pixels.
const ATLAS_SIZE: u32 = 1024;

/// Batch texture sentinel for the color-glyph atlas (see
/// `UiKit::cache_color_glyph`) — never a valid [`TextureId`] index.
const COLOR_ATLAS_TEX: u32 = u32::MAX;

/// Errors from GPU setup.
#[derive(Debug)]
pub enum RenderError {
    CreateSurface(wgpu::CreateSurfaceError),
    NoAdapter(wgpu::RequestAdapterError),
    /// No adapter's name matched the requested filter (see
    /// [`HeadlessRenderer::with_adapter`]); carries the filter.
    NoMatchingAdapter(String),
    NoDevice(wgpu::RequestDeviceError),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreateSurface(e) => write!(f, "failed to create surface: {e}"),
            Self::NoAdapter(e) => write!(f, "no suitable GPU adapter: {e}"),
            Self::NoMatchingAdapter(filter) => {
                write!(f, "no GPU adapter matching {filter:?}")
            }
            Self::NoDevice(e) => write!(f, "failed to open GPU device: {e}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// Why a frame could not be presented — the non-success outcomes of acquiring
/// the window's next surface texture, which wgpu reports as
/// [`wgpu::CurrentSurfaceTexture`] rather than as an error type.
///
/// Only [`Timeout`](Self::Timeout) and [`Occluded`](Self::Occluded) are
/// routine: skip the frame and try again on the next redraw. The rest mean the
/// surface needs attention — [`Renderer::render`] already spends one
/// reconfigure-and-retry on `Outdated`/`Lost` before handing either back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// Acquiring the next frame timed out.
    Timeout,
    /// The window is minimized or fully covered, so there is nothing to draw.
    Occluded,
    /// The surface no longer matches the window and reconfiguring didn't help.
    Outdated,
    /// The surface (or the device behind it) is gone; recreate it.
    Lost,
    /// The acquire raised a validation error.
    Validation,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match self {
            Self::Timeout => "timed out acquiring the next frame",
            Self::Occluded => "the window is occluded",
            Self::Outdated => "the surface configuration is outdated",
            Self::Lost => "the surface was lost",
            Self::Validation => "the surface acquire failed validation",
        };
        f.write_str(what)
    }
}

impl std::error::Error for FrameError {}

/// Splits an acquire outcome into the texture to draw into and the reason there
/// isn't one. `Suboptimal` still carries a usable texture — it only asks for a
/// reconfigure eventually — so it counts as success, as it did when this was a
/// `Result` from wgpu itself.
fn acquired(current: wgpu::CurrentSurfaceTexture) -> Result<wgpu::SurfaceTexture, FrameError> {
    use wgpu::CurrentSurfaceTexture as Current;
    match current {
        Current::Success(t) | Current::Suboptimal(t) => Ok(t),
        Current::Timeout => Err(FrameError::Timeout),
        Current::Occluded => Err(FrameError::Occluded),
        Current::Outdated => Err(FrameError::Outdated),
        Current::Lost => Err(FrameError::Lost),
        Current::Validation => Err(FrameError::Validation),
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
    /// 1 = sample the glyph atlas as single-channel coverage (white RGB from the
    /// vertex color); 0 = sample a host RGBA texture normally. Lets the R8
    /// coverage atlas and RGBA sprites share one pipeline.
    mode: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    screen: [f32; 2],
    _pad: [f32; 2],
}

/// A scissor rectangle in physical pixels: `[x, y, w, h]`.
type Scissor = [u32; 4];

/// One batch: a scissor region, the texture index to bind, and the index range
/// drawn under it. A new batch starts whenever the scissor *or* texture changes.
type Batch = (Scissor, u32, Range<u32>);

/// Tessellated geometry for one frame: vertices, indices, and batches.
type Mesh = (Vec<Vertex>, Vec<u32>, Vec<Batch>);

/// The destination of one [`UiKit::record`] call.
struct Frame<'a> {
    view: &'a wgpu::TextureView,
    width: u32,
    height: u32,
    /// Logical → physical scale to upscale the (logical) draw list by.
    scale: f32,
}

/// Borrowed GPU handles passed into [`UiKit::record`].
struct GpuRef<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
}

/// A rasterized glyph packed into the atlas: where it lives and how to place it
/// relative to the pen origin on the baseline.
#[derive(Clone, Copy)]
struct CachedGlyph {
    uv: TexRect,
    left: i32,
    top: i32,
    w: u32,
    h: u32,
}

/// Glyph cache key: `(font index, glyph id, rounded em pixels, quarter-pixel
/// x bin)`. The bin is always 0 unless the backend supports subpixel bins.
type GlyphKey = (usize, u16, u32, u8);

/// Splits a physical pen x into an integer pixel and a quarter-pixel bin
/// (`0..=3`), carrying a rounded-up fraction into the next pixel — the same
/// quantization cosmic-text's `SubpixelBin::new` performs, so the placement
/// matches the offset baked into the rasterized bitmap.
fn quarter_bin(x: f32) -> (f32, u8) {
    let floor = x.floor();
    let quarters = ((x - floor) * 4.0).round() as u8;
    if quarters == 4 {
        (floor + 1.0, 0)
    } else {
        (floor, quarters)
    }
}

/// The pipeline, atlas, and bind groups shared by both front ends.
///
/// Bind group 0 (globals: uniforms + sampler) is picked per `record` by target
/// size (see `globals_index`); bind group 1 is the texture, swapped per batch.
/// `tex_groups[0]` is the atlas (solids + glyphs); host-registered textures
/// append at index `1..`, indexed by [`TextureId`]. `user_textures` keeps those
/// alive (and their size, for re-uploads), parallel to `tex_groups[1..]`.
struct UiKit {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    globals_layout: wgpu::BindGroupLayout,
    /// One 16-byte uniform buffer + globals bind group per physical target size
    /// seen, keyed by `(w, h)`. Each buffer is written once at creation and
    /// never again, so any number of `record`s — into differently sized targets
    /// — can share one encoder/submit without clobbering each other's uniforms
    /// (the same hazard the per-call geometry buffers guard against; a
    /// `queue.write_buffer` would apply before the submit's passes run).
    globals: Vec<((u32, u32), wgpu::BindGroup)>,
    tex_layout: wgpu::BindGroupLayout,
    tex_groups: Vec<wgpu::BindGroup>,
    user_textures: Vec<(wgpu::Texture, u32, u32)>,
    atlas: UiAtlas,
    glyph_cache: HashMap<GlyphKey, CachedGlyph>,
    /// The RGBA atlas COLOR glyphs (emoji) pack into, with its bind group —
    /// created on the first color glyph a backend hands back (the default
    /// backend never does). Cached separately from `glyph_cache` so main-atlas
    /// grows/repacks (which clear that cache) don't re-pack color glyphs into
    /// a sheet that was never cleared.
    color_atlas: Option<(UiAtlas, wgpu::BindGroup)>,
    color_cache: HashMap<GlyphKey, CachedGlyph>,
    /// Set once the atlas has grown to the device's max texture size and still
    /// can't fit a glyph — so the "dropping text" warning is emitted only once.
    atlas_maxed: bool,
    /// Mirror of the backend's [`Fonts::subpixel_bins`], captured in
    /// `prepare_glyphs` so `tessellate` (which has no fonts handle) places
    /// pens the same way the glyphs were rasterized: floored + binned when
    /// true, rounded to whole pixels when false.
    subpixel: bool,
    /// Tessellation output reused across `record` calls; the mesh is copied
    /// into per-call GPU buffers before each `record` returns.
    scratch: Mesh,
}

impl UiKit {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let atlas = UiAtlas::new(device, queue, ATLAS_SIZE);

        // Nearest sampling: glyphs are rasterized at the exact target size and
        // UI sprites are typically pixel art (the M.A.X. look), so neither wants
        // bilinear smoothing.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("wgpu-ui sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        // Group 0: per-frame uniforms (vertex) + the shared sampler (fragment).
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wgpu-ui globals layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
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
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // Group 1: the texture to sample (swapped per batch).
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wgpu-ui texture layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });

        let atlas_bg = make_texture_bind_group(device, &tex_layout, atlas.view());

        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wgpu-ui pipeline layout"),
            bind_group_layouts: &[Some(&globals_layout), Some(&tex_layout)],
            immediate_size: 0,
        });

        const ATTRS: [wgpu::VertexAttribute; 4] =
            wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Uint32];
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRS,
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wgpu-ui pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout)],
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

        Self {
            pipeline,
            sampler,
            globals_layout,
            globals: Vec::new(),
            tex_layout,
            tex_groups: vec![atlas_bg],
            user_textures: Vec::new(),
            atlas,
            glyph_cache: HashMap::new(),
            color_atlas: None,
            color_cache: HashMap::new(),
            atlas_maxed: false,
            subpixel: false,
            scratch: Mesh::default(),
        }
    }

    /// Returns (the index of) the globals bind group for a `(w, h)` physical
    /// target, creating and caching it on first sight. The uniform contents
    /// depend only on the size, so each buffer is written through its creation
    /// mapping and never touched again — no queued write can race an earlier
    /// pass in the same submit. Steady state is one entry per live target size.
    fn globals_index(&mut self, device: &wgpu::Device, w: u32, h: u32) -> usize {
        if let Some(i) = self.globals.iter().position(|(size, _)| *size == (w, h)) {
            return i;
        }
        // A live window resize mints a one-frame size per event; don't hoard
        // them. In-flight passes keep their bind groups alive on the GPU side.
        if self.globals.len() >= 16 {
            self.globals.clear();
        }
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wgpu-ui uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        buf.slice(..)
            .get_mapped_range_mut()
            .expect("mapped at creation")
            .copy_from_slice(bytemuck::bytes_of(&Uniforms {
                screen: [w as f32, h as f32],
                _pad: [0.0, 0.0],
            }));
        buf.unmap();
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wgpu-ui globals"),
            layout: &self.globals_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.globals.push(((w, h), bg));
        self.globals.len() - 1
    }

    /// Packs a COLOR glyph into the RGBA side atlas (created on first use),
    /// caching its placement. Emoji counts are small, so this atlas neither
    /// grows nor repacks; on exhaustion the glyph is dropped with a one-line
    /// warning (matching the maxed-main-atlas last resort).
    fn cache_color_glyph(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: GlyphKey,
        bmp: &crate::text::GlyphBitmap,
    ) {
        if self.color_atlas.is_none() {
            let atlas = UiAtlas::new_rgba(device, queue, ATLAS_SIZE);
            let bg = make_texture_bind_group(device, &self.tex_layout, atlas.view());
            self.color_atlas = Some((atlas, bg));
        }
        let (atlas, _) = self.color_atlas.as_mut().expect("just created");
        match atlas.alloc(bmp.width, bmp.height) {
            Some((x, y)) => {
                atlas.upload(queue, x, y, bmp.width, bmp.height, &bmp.coverage);
                self.color_cache.insert(
                    key,
                    CachedGlyph {
                        uv: atlas.uv(x, y, bmp.width, bmp.height),
                        left: bmp.left,
                        top: bmp.top,
                        w: bmp.width,
                        h: bmp.height,
                    },
                );
            }
            None => {
                eprintln!(
                    "wgpu-ui: color-glyph atlas exhausted ({}px); an emoji will not render",
                    atlas.size()
                );
                self.color_cache.insert(
                    key,
                    CachedGlyph {
                        uv: TexRect::new(0.0, 0.0, 0.0, 0.0),
                        left: 0,
                        top: 0,
                        w: 0,
                        h: 0,
                    },
                );
            }
        }
    }

    /// Re-creates the glyph atlas at double the side length (capped at the
    /// device's max texture dimension), rebuilds its bind group, and clears the
    /// glyph cache so glyphs re-pack into the larger sheet. Returns `false` when
    /// already at the device limit (the caller then repacks at the same size —
    /// see `prepare_glyphs` — before dropping the glyph as a logged last
    /// resort). Called when [`UiAtlas::alloc`] fails, so text is never silently
    /// dropped just because the default 1024² sheet filled up.
    fn grow_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        let max = device.limits().max_texture_dimension_2d;
        if self.atlas.size() >= max {
            return false;
        }
        let new_size = self.atlas.size().saturating_mul(2).min(max);
        self.rebuild_atlas(device, queue, new_size);
        true
    }

    /// Re-creates the atlas at `size` and clears the glyph cache, so the next
    /// packing walk re-packs only what it references. Passes already encoded
    /// against the old atlas keep it alive through their recorded bind group.
    fn rebuild_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, size: u32) {
        self.atlas = UiAtlas::new(device, queue, size);
        self.tex_groups[0] = make_texture_bind_group(device, &self.tex_layout, self.atlas.view());
        self.glyph_cache.clear();
    }

    /// Uploads a host RGBA8 image and returns its [`TextureId`] for use in
    /// [`DrawList::image`]/[`DrawList::sprite`]. `rgba` is tightly packed
    /// `w * h * 4` bytes, interpreted as sRGB.
    fn register_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rgba: &[u8],
        w: u32,
        h: u32,
    ) -> TextureId {
        let (texture, view) = create_user_texture(device, w, h);
        upload_texture(queue, &texture, w, h, rgba);
        let bg = make_texture_bind_group(device, &self.tex_layout, &view);
        let id = TextureId(self.tex_groups.len() as u32);
        self.tex_groups.push(bg);
        self.user_textures.push((texture, w, h));
        id
    }

    /// Replaces the pixels of a previously [`register_texture`](Self::register_texture)d
    /// image (same `w`x`h`). Ignores the atlas id and out-of-range ids.
    fn update_texture(&self, queue: &wgpu::Queue, id: TextureId, rgba: &[u8]) {
        // id 0 is the atlas (not host-owned); ids 1.. index `user_textures`.
        let Some(slot) = (id.0 as usize).checked_sub(1) else {
            return;
        };
        let Some((texture, w, h)) = self.user_textures.get(slot) else {
            return;
        };
        upload_texture(queue, texture, *w, *h, rgba);
    }

    /// Re-creates a host texture at a possibly *different* size, reusing its
    /// [`TextureId`] slot (the old GPU texture drops). For a host that shows a
    /// varying-size image in one long-lived slot — a modal preview whose
    /// contents change per open — so it never leaks a slot per update. Ignores
    /// the atlas id and out-of-range ids.
    fn replace_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: TextureId,
        rgba: &[u8],
        w: u32,
        h: u32,
    ) {
        let Some(slot) = (id.0 as usize).checked_sub(1) else {
            return;
        };
        if slot >= self.user_textures.len() {
            return;
        }
        let (texture, view) = create_user_texture(device, w, h);
        upload_texture(queue, &texture, w, h, rgba);
        self.tex_groups[id.0 as usize] = make_texture_bind_group(device, &self.tex_layout, &view);
        self.user_textures[slot] = (texture, w, h);
    }

    /// Records the tessellated draw list into `frame`, rasterizing and uploading
    /// any not-yet-cached glyphs first. `clear` is `Some(color)` to clear first
    /// (UI owns the frame) or `None` to draw over existing contents (overlay).
    fn record(
        &mut self,
        gpu: GpuRef,
        fonts: &Fonts,
        encoder: &mut wgpu::CommandEncoder,
        frame: Frame,
        list: &DrawList,
        clear: Option<Rgba>,
    ) {
        let Frame {
            view,
            width: w,
            height: h,
            scale,
        } = frame;
        self.prepare_glyphs(gpu.device, gpu.queue, fonts, list, scale);
        self.tessellate(list, w, h, scale);
        let globals = self.globals_index(gpu.device, w, h);
        let (verts, indices, batches) = &self.scratch;
        // Fresh per-call geometry buffers, filled through their creation mapping
        // (no staging copy). A single persistent buffer re-written at offset 0 is
        // WRONG when several `record`s share one encoder/submit (the editor draws
        // every panel + overlay per frame): `queue.write_buffer`s all apply before
        // the submit's commands, so every render pass reads the LAST list's
        // vertices — later `record`s clobber earlier ones (floating-panel chrome +
        // late overlays silently vanished). A per-call buffer is owned by its own
        // draw; wgpu keeps it alive via the encoded command until submit. The
        // per-size uniforms (`globals_index`) exist for the same reason.
        let geometry = (!verts.is_empty() && !indices.is_empty()).then(|| {
            let vbuf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("wgpu-ui vertices"),
                size: std::mem::size_of_val(verts.as_slice()) as u64,
                usage: wgpu::BufferUsages::VERTEX,
                mapped_at_creation: true,
            });
            vbuf.slice(..)
                .get_mapped_range_mut()
                .expect("mapped at creation")
                .copy_from_slice(bytemuck::cast_slice(verts));
            vbuf.unmap();
            let ibuf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("wgpu-ui indices"),
                size: std::mem::size_of_val(indices.as_slice()) as u64,
                usage: wgpu::BufferUsages::INDEX,
                mapped_at_creation: true,
            });
            ibuf.slice(..)
                .get_mapped_range_mut()
                .expect("mapped at creation")
                .copy_from_slice(bytemuck::cast_slice(indices));
            ibuf.unmap();
            (vbuf, ibuf)
        });

        let load = match clear {
            Some(c) => {
                let [r, g, b, a] = c.to_linear_f32();
                wgpu::LoadOp::Clear(wgpu::Color {
                    r: r as f64,
                    g: g as f64,
                    b: b as f64,
                    a: a as f64,
                })
            }
            None => wgpu::LoadOp::Load,
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("wgpu-ui pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        if let Some((vbuf, ibuf)) = &geometry {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.globals[globals].1, &[]);
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
            let mut bound: Option<u32> = None;
            for (scissor, tex, range) in batches {
                if bound != Some(*tex) {
                    // Atlas (0) is always present; a stale id from a dropped
                    // texture falls back to it rather than panicking. The
                    // sentinel binds the color-glyph atlas (only ever batched
                    // after `cache_color_glyph` created it).
                    let bg = if *tex == COLOR_ATLAS_TEX {
                        self.color_atlas
                            .as_ref()
                            .map(|(_, bg)| bg)
                            .unwrap_or(&self.tex_groups[0])
                    } else {
                        self.tex_groups
                            .get(*tex as usize)
                            .unwrap_or(&self.tex_groups[0])
                    };
                    pass.set_bind_group(1, bg, &[]);
                    bound = Some(*tex);
                }
                pass.set_scissor_rect(scissor[0], scissor[1], scissor[2], scissor[3]);
                pass.draw_indexed(range.clone(), 0, 0..1);
            }
        }
    }

    /// Rasterizes and uploads any glyphs referenced by `list` that are not yet
    /// in the cache. Coverage is stored as white RGB with the coverage in alpha.
    fn prepare_glyphs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        fonts: &Fonts,
        list: &DrawList,
        scale: f32,
    ) {
        let empty = CachedGlyph {
            uv: TexRect::new(0.0, 0.0, 0.0, 0.0),
            left: 0,
            top: 0,
            w: 0,
            h: 0,
        };
        // Cache every referenced glyph. If the atlas fills, grow it and restart
        // from scratch so no glyph is left uncached — and thus silently dropped
        // at draw time. Each grow strictly enlarges the sheet up to the device
        // limit, so this loop runs at most a handful of times (plus at most one
        // same-size repack at the limit).
        self.subpixel = fonts.subpixel_bins();
        let mut repacked = false;
        'pack: loop {
            for cmd in &list.cmds {
                let DrawCmd::Glyph {
                    font,
                    glyph,
                    px,
                    pen,
                    ..
                } = *cmd
                else {
                    continue;
                };
                // Rasterize at the physical size (logical em × ui scale) for
                // crisp text on HiDPI; the cache key buckets by that rounded
                // size (plus the pen's quarter-pixel bin when the backend
                // subpixel-positions — `tessellate` computes the same bin).
                let bucket = (px * scale).round().max(1.0) as u32;
                let bin = if self.subpixel {
                    quarter_bin(pen.x * scale).1
                } else {
                    0
                };
                let key = (font.0, glyph, bucket, bin);
                if self.glyph_cache.contains_key(&key) || self.color_cache.contains_key(&key) {
                    continue;
                }
                let bmp = fonts.rasterize_sub(font, glyph, bucket as f32, bin);
                if bmp.is_empty() {
                    self.glyph_cache.insert(key, empty);
                    continue;
                }
                // A COLOR glyph (emoji from a shaping backend) packs into the
                // RGBA side atlas and draws untinted through the plain-texture
                // mode; it never competes for coverage-atlas space.
                if bmp.color {
                    self.cache_color_glyph(device, queue, key, &bmp);
                    continue;
                }
                let (x, y) = match self.atlas.alloc(bmp.width, bmp.height) {
                    Some(pos) => pos,
                    None => {
                        // Atlas full: grow and re-pack everything into the larger
                        // sheet (the cache was cleared, so restart the walk).
                        if self.grow_atlas(device, queue) {
                            continue 'pack;
                        }
                        // At the device's max texture size. Stale entries — old
                        // px buckets from animated/zoomed text, glyphs no longer
                        // on screen — may be holding the space: rebuild once at
                        // the same size, so the restarted walk re-packs only
                        // what this list references (a long session evicts its
                        // history instead of degrading to missing text).
                        if !repacked {
                            repacked = true;
                            let size = self.atlas.size();
                            self.rebuild_atlas(device, queue, size);
                            continue 'pack;
                        }
                        // Even the live set alone doesn't fit — drop this glyph,
                        // warning once so the failure isn't silent.
                        if !self.atlas_maxed {
                            self.atlas_maxed = true;
                            eprintln!(
                                "wgpu-ui: glyph atlas exhausted at the device's max \
                                 texture size ({}px); some text will not render",
                                self.atlas.size()
                            );
                        }
                        self.glyph_cache.insert(key, empty);
                        continue;
                    }
                };
                // The atlas is single-channel coverage now — upload the bitmap's
                // coverage bytes straight in (no RGBA expansion).
                self.atlas
                    .upload(queue, x, y, bmp.width, bmp.height, &bmp.coverage);
                self.glyph_cache.insert(
                    key,
                    CachedGlyph {
                        uv: self.atlas.uv(x, y, bmp.width, bmp.height),
                        left: bmp.left,
                        top: bmp.top,
                        w: bmp.width,
                        h: bmp.height,
                    },
                );
            }
            break;
        }
    }

    /// Walks the display list into vertex/index buffers plus
    /// `(scissor, texture, range)` batches, filling the reused `self.scratch`
    /// mesh (the caller copies it into per-call GPU buffers). The clip stack
    /// starts at the full target; each quad is emitted whole and cropped by its
    /// scissor.
    ///
    /// Everything is authored in **logical** pixels; positions are multiplied by
    /// `scale` here to fill the physical target, so the same `DrawList` renders
    /// crisply at any HiDPI factor. Glyph quads are the exception: their bitmaps
    /// are already rasterized at the physical size, so only the pen origin is
    /// scaled (the bitmap offset/extent are added in physical pixels). Glyphs
    /// resolve through the cache populated by [`UiKit::prepare_glyphs`].
    fn tessellate(&mut self, list: &DrawList, w: u32, h: u32, scale: f32) {
        // The clip stack stays in logical space; clips become physical scissors
        // at emit time.
        let logical = Rect::new(0.0, 0.0, w as f32 / scale, h as f32 / scale);
        let mut clips = vec![logical];
        let white = self.atlas.white_uv();
        let (verts, indices, batches) = &mut self.scratch;
        verts.clear();
        indices.clear();
        batches.clear();
        let mut cur: Option<(Scissor, u32)> = None;
        let mut batch_start: u32 = 0;

        // `rect` is already in physical pixels; `clip` is logical.
        let mut quad = |rect: Rect, uv: TexRect, color: Rgba, tex: u32, clip: Rect| {
            if clip.is_empty() {
                return;
            }
            let sc = scissor_of(scale_rect(clip, scale), w, h);
            if sc[2] == 0 || sc[3] == 0 {
                return;
            }
            if cur != Some((sc, tex)) {
                if let Some((prev_sc, prev_tex)) = cur {
                    batches.push((prev_sc, prev_tex, batch_start..indices.len() as u32));
                }
                cur = Some((sc, tex));
                batch_start = indices.len() as u32;
            }
            // tex 0 is the coverage atlas (solids/glyphs); ≥1 are host RGBA
            // textures.
            let mode = u32::from(tex == 0);
            push_quad(verts, indices, rect, uv, color.to_srgb_f32(), mode);
        };

        for cmd in &list.cmds {
            match *cmd {
                DrawCmd::PushClip(r) => {
                    let top = *clips.last().expect("clip stack never empty");
                    clips.push(top.intersect(&r));
                }
                DrawCmd::PopClip => {
                    if clips.len() > 1 {
                        clips.pop();
                    }
                }
                DrawCmd::Solid { rect, color } => {
                    let clip = *clips.last().expect("clip stack never empty");
                    quad(snap_rect(scale_rect(rect, scale)), white, color, 0, clip);
                }
                DrawCmd::Image {
                    tex,
                    rect,
                    uv,
                    color,
                } => {
                    let clip = *clips.last().expect("clip stack never empty");
                    quad(snap_rect(scale_rect(rect, scale)), uv, color, tex.0, clip);
                }
                DrawCmd::Glyph {
                    font,
                    glyph,
                    px,
                    pen,
                    color,
                } => {
                    let bucket = (px * scale).round().max(1.0) as u32;
                    // Subpixel backends: floor the pen and rasterize the
                    // fraction in (quarter bins) — identical glyphs keep
                    // identical spacing even on fractional advances (the
                    // per-glyph rounding jitter was plainly visible on
                    // masked-input bullet strings). Otherwise: pen snapped
                    // to the pixel grid, the pixel-locked default.
                    let (pen_x, bin) = if self.subpixel {
                        quarter_bin(pen.x * scale)
                    } else {
                        ((pen.x * scale).round(), 0)
                    };
                    let key = (font.0, glyph, bucket, bin);
                    // Coverage glyphs tint by the draw color; COLOR glyphs
                    // (emoji, in the RGBA side atlas) draw as-is — untinted.
                    let hit = self
                        .glyph_cache
                        .get(&key)
                        .map(|g| (g, color, 0))
                        .or_else(|| {
                            self.color_cache
                                .get(&key)
                                .map(|g| (g, Rgba::WHITE, COLOR_ATLAS_TEX))
                        });
                    if let Some((g, tint, tex)) = hit
                        && g.w > 0
                    {
                        let clip = *clips.last().expect("clip stack never empty");
                        // Bitmap offset/extent are physical (rasterized at
                        // `px × scale`), so the glyph blits 1:1.
                        let rect = Rect::new(
                            pen_x + g.left as f32,
                            (pen.y * scale).round() - g.top as f32,
                            g.w as f32,
                            g.h as f32,
                        );
                        quad(rect, g.uv, tint, tex, clip);
                    }
                }
            }
        }

        if let Some((sc, tex)) = cur {
            let end = indices.len() as u32;
            if batch_start < end {
                batches.push((sc, tex, batch_start..end));
            }
        }
    }
}

/// Scales a rectangle's position and size by `s` (logical → physical pixels).
fn scale_rect(r: Rect, s: f32) -> Rect {
    Rect::new(r.x * s, r.y * s, r.w * s, r.h * s)
}

/// Snaps a physical-pixel rect to the integer pixel grid, rounding each edge
/// independently so fills, bevels and frames land on exact pixels — the
/// "pixel-locked" UI look, free of half-pixel blur at fractional scales or
/// centred (`.5`) positions. Adjacent rects stay seamless: a shared edge rounds
/// to the same integer from either side, so no gap or overlap opens up.
fn snap_rect(r: Rect) -> Rect {
    let x0 = r.x.round();
    let y0 = r.y.round();
    let x1 = r.right().round();
    let y1 = r.bottom().round();
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

fn make_texture_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("wgpu-ui texture"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(view),
        }],
    })
}

/// Creates a sampleable sRGB texture (and its view) for a host image.
fn create_user_texture(
    device: &wgpu::Device,
    w: u32,
    h: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("wgpu-ui user texture"),
        size: wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Uploads tightly packed RGBA8 into `texture` (full `w`x`h`).
fn upload_texture(queue: &wgpu::Queue, texture: &wgpu::Texture, w: u32, h: u32, rgba: &[u8]) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
}

/// Appends the two triangles of `rect` (uv from `uv`, color `color`).
fn push_quad(
    verts: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    rect: Rect,
    uv: TexRect,
    color: [f32; 4],
    mode: u32,
) {
    let base = verts.len() as u32;
    let (x0, y0, x1, y1) = (rect.x, rect.y, rect.right(), rect.bottom());
    verts.push(Vertex {
        pos: [x0, y0],
        uv: [uv.u0, uv.v0],
        color,
        mode,
    });
    verts.push(Vertex {
        pos: [x1, y0],
        uv: [uv.u1, uv.v0],
        color,
        mode,
    });
    verts.push(Vertex {
        pos: [x1, y1],
        uv: [uv.u1, uv.v1],
        color,
        mode,
    });
    verts.push(Vertex {
        pos: [x0, y1],
        uv: [uv.u0, uv.v1],
        color,
        mode,
    });
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Integer scissor rect for `clip`, clamped to the `w`x`h` target.
fn scissor_of(clip: Rect, w: u32, h: u32) -> Scissor {
    let x0 = clip.x.clamp(0.0, w as f32).floor();
    let y0 = clip.y.clamp(0.0, h as f32).floor();
    let x1 = clip.right().clamp(0.0, w as f32).ceil();
    let y1 = clip.bottom().clamp(0.0, h as f32).ceil();
    [
        x0 as u32,
        y0 as u32,
        (x1 - x0).max(0.0) as u32,
        (y1 - y0).max(0.0) as u32,
    ]
}

/// A best-effort cross-process lock guarding GPU adapter/device creation, held
/// only while `WGPU_UI_GPU_LOCK` is set (tests). Acquired by atomically creating
/// a lockfile in the temp dir; released by removing it on drop. A crashed holder
/// leaves a stale file, which the next waiter steals after a grace period.
struct GpuCreateLock(Option<std::path::PathBuf>);

impl GpuCreateLock {
    fn acquire_if_enabled() -> Self {
        if std::env::var_os("WGPU_UI_GPU_LOCK").is_none() {
            return GpuCreateLock(None);
        }
        use std::io::Write;
        let path = std::env::temp_dir().join("wgpu-ui-gpu-create.lock");
        let mut waited_ms = 0u64;
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    let _ = write!(f, "{}", std::process::id());
                    return GpuCreateLock(Some(path));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Steal a stale lock (crashed holder) after ~30s; give up
                    // after 60s and proceed unlocked rather than hang forever.
                    let stale = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|m| m.elapsed().ok())
                        .is_some_and(|age| age.as_secs() > 30);
                    if stale {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if waited_ms > 60_000 {
                        return GpuCreateLock(None);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(15));
                    waited_ms += 15;
                }
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_millis(15));
                    waited_ms += 15;
                    if waited_ms > 60_000 {
                        return GpuCreateLock(None);
                    }
                }
            }
        }
    }
}

impl Drop for GpuCreateLock {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn request_device(
    instance: &wgpu::Instance,
    compatible_surface: Option<&wgpu::Surface<'_>>,
) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), RenderError> {
    // Some drivers segfault when several processes create a GPU adapter/device
    // at once — which the test suite does, running a dozen GPU test binaries in
    // parallel. When `WGPU_UI_GPU_LOCK` is set (the workspace `.cargo/config.toml`
    // sets it for every `cargo test`/`run`), serialize creation with a
    // cross-process file lock. Unset in a shipped binary, so production is
    // untouched (and a real app creates its one device at startup uncontended).
    let _create_guard = GpuCreateLock::acquire_if_enabled();

    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface,
        ..Default::default()
    }))
    .map_err(RenderError::NoAdapter)?;
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("wgpu-ui"),
        ..Default::default()
    }))
    .map_err(RenderError::NoDevice)?;
    Ok((adapter, device, queue))
}

/// Reads a `w`x`h` Rgba8 texture back into tightly packed RGBA bytes, depadding
/// the 256-byte row alignment required for texture-to-buffer copies.
fn read_target_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::Texture,
    w: u32,
    h: u32,
) -> Vec<u8> {
    let stride = w as usize * 4;
    let padded = stride.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wgpu-ui readback"),
        size: (padded * h as usize) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        tx.send(result).expect("receiver alive");
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .expect("device poll");
    rx.recv().expect("map callback").expect("buffer map");

    let mapped = slice.get_mapped_range().expect("buffer mapped for read");
    let mut pixels = Vec::with_capacity(stride * h as usize);
    for row in mapped.chunks_exact(padded) {
        pixels.extend_from_slice(&row[..stride]);
    }
    pixels
}

fn offscreen_target(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("wgpu-ui offscreen target"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// The embeddable UI renderer: shares the host's [`wgpu::Device`]/[`wgpu::Queue`]
/// and draws a [`DrawList`] into a target the host owns. This is the path for
/// apps that render their own content (a game world, an editor map) and want the
/// UI composited on top — construct it from your existing device/queue and call
/// [`render_into`](Self::render_into) as an overlay each frame.
pub struct UiRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    kit: UiKit,
    scale: f32,
}

impl UiRenderer {
    /// Creates a UI renderer sharing the host's GPU. `format` is the format of
    /// the target it draws into (typically your surface format); prefer an sRGB
    /// format so colors and glyph anti-aliasing blend correctly.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let kit = UiKit::new(device, queue, format);
        Self {
            device: device.clone(),
            queue: queue.clone(),
            kit,
            scale: 1.0,
        }
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Sets the UI scale (logical → physical factor, e.g. the window's DPI scale
    /// factor). The widget tree lays out and draws in logical pixels; the
    /// renderer upscales the resulting [`DrawList`] to the physical target and
    /// rasterizes glyphs at the physical size for crisp HiDPI text. Default
    /// `1.0` (logical == physical).
    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale.max(1e-4);
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Uploads a host RGBA8 image (tightly packed `w * h * 4` sRGB bytes) and
    /// returns its [`TextureId`] for [`DrawList::image`]/[`DrawList::sprite`] and
    /// the [`Image`](crate::widgets::Image) widget. Register textures once and
    /// reuse the ids across frames.
    pub fn register_texture(&mut self, rgba: &[u8], w: u32, h: u32) -> TextureId {
        self.kit
            .register_texture(&self.device, &self.queue, rgba, w, h)
    }

    /// Replaces the pixels of a previously [`register_texture`](Self::register_texture)d
    /// image (same `w`x`h`) — e.g. a live world/minimap preview.
    pub fn update_texture(&self, id: TextureId, rgba: &[u8]) {
        self.kit.update_texture(&self.queue, id, rgba);
    }

    /// Re-creates a registered texture at a possibly different size, reusing its
    /// [`TextureId`]. For one long-lived slot whose contents change size per
    /// use (a modal preview), so a host never leaks a slot per update.
    pub fn replace_texture(&mut self, id: TextureId, rgba: &[u8], w: u32, h: u32) {
        self.kit
            .replace_texture(&self.device, &self.queue, id, rgba, w, h);
    }

    /// Draws `list` into `view` **over existing contents** (no clear), recording
    /// into the caller's `encoder`. The host submits the encoder and presents.
    /// `size` is the target's physical pixel size. Glyph uploads are queued on
    /// the shared queue, ordered before the host's submit.
    ///
    /// Target contract: `view`'s format must equal the `format` this renderer
    /// was constructed with, and the target must be non-multisampled (the
    /// pipeline is built once, `sample_count` 1). Prefer an sRGB format — the
    /// shader emits linear light and a non-sRGB target washes colors out. Any
    /// number of `render_into` calls, into any mix of target sizes, may share
    /// one encoder/submit.
    pub fn render_into(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        size: (u32, u32),
        fonts: &Fonts,
        list: &DrawList,
    ) {
        self.record(encoder, view, size, fonts, list, None);
    }

    /// Like [`render_into`](Self::render_into), but clears `view` to `clear`
    /// before drawing — for the host whose UI pass is the frame's **first**
    /// pass: the UI owns the window's base contents and everything else
    /// (world, hosted content) loads over it. Same target contract, and it
    /// composes the same way: any mix of `render_into_clear` / `render_into`
    /// calls may share one encoder/submit (a clear wipes the whole target,
    /// so it belongs on the frame's first record only).
    pub fn render_into_clear(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        size: (u32, u32),
        fonts: &Fonts,
        list: &DrawList,
        clear: Rgba,
    ) {
        self.record(encoder, view, size, fonts, list, Some(clear));
    }

    /// Records the draw list, clearing first when `clear` is `Some` (UI owns the
    /// target) or loading existing contents when `None` (overlay).
    fn record(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        size: (u32, u32),
        fonts: &Fonts,
        list: &DrawList,
        clear: Option<Rgba>,
    ) {
        self.kit.record(
            GpuRef {
                device: &self.device,
                queue: &self.queue,
            },
            fonts,
            encoder,
            Frame {
                view,
                width: size.0,
                height: size.1,
                scale: self.scale,
            },
            list,
            clear,
        );
    }
}

/// Presents UI draw lists to a window surface — the convenience front end for
/// **UI-only** apps. Apps that render their own world should use [`UiRenderer`]
/// and composite the UI as an overlay instead.
pub struct Renderer {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    ui: UiRenderer,
}

impl Renderer {
    /// Creates a renderer for `target` (e.g. an `Arc<winit::window::Window>`) at
    /// `width`x`height` physical pixels.
    pub fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(target)
            .map_err(RenderError::CreateSurface)?;
        let (adapter, device, queue) = request_device(&instance, Some(&surface))?;

        let capabilities = surface.get_capabilities(&adapter);
        // Prefer an sRGB surface format so the shader's linear output is encoded
        // back to sRGB (matching the offscreen target used by tests).
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(capabilities.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            // `Auto` is the historical behaviour: sRGB for the sRGB format
            // picked above, which is what the shader's linear output expects.
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: Vec::new(),
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let ui = UiRenderer::new(&device, &queue, format);

        Ok(Self {
            surface,
            config,
            ui,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(self.ui.device(), &self.config);
    }

    /// The shared GPU handles, e.g. to register fonts/textures or to drive your
    /// own rendering with the same device.
    pub fn device(&self) -> &wgpu::Device {
        self.ui.device()
    }

    pub fn queue(&self) -> &wgpu::Queue {
        self.ui.queue()
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Sets the UI scale (see [`UiRenderer::set_scale`]).
    pub fn set_scale(&mut self, scale: f32) {
        self.ui.set_scale(scale);
    }

    pub fn scale(&self) -> f32 {
        self.ui.scale()
    }

    /// Registers a host RGBA8 texture (see [`UiRenderer::register_texture`]).
    pub fn register_texture(&mut self, rgba: &[u8], w: u32, h: u32) -> TextureId {
        self.ui.register_texture(rgba, w, h)
    }

    /// Updates a registered texture's pixels (see [`UiRenderer::update_texture`]).
    pub fn update_texture(&self, id: TextureId, rgba: &[u8]) {
        self.ui.update_texture(id, rgba);
    }

    /// Renders one frame: clears to `clear`, then draws `list` (resolving its
    /// glyphs through `fonts`).
    pub fn render(
        &mut self,
        fonts: &Fonts,
        list: &DrawList,
        clear: Rgba,
    ) -> Result<(), FrameError> {
        let frame = match acquired(self.surface.get_current_texture()) {
            Err(FrameError::Outdated | FrameError::Lost) => {
                self.surface.configure(self.ui.device(), &self.config);
                acquired(self.surface.get_current_texture())?
            }
            other => other?,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .ui
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let size = (self.config.width, self.config.height);
        self.ui
            .record(&mut encoder, &view, size, fonts, list, Some(clear));
        self.ui.queue().submit([encoder.finish()]);
        // Presenting is the queue's job as of wgpu 30 (it was
        // `SurfaceTexture::present` before), and must follow the submit.
        self.ui.queue().present(frame);
        Ok(())
    }
}

/// Renders UI draw lists offscreen and reads the pixels back — the harness used
/// for automated visual tests. Construction fails ([`RenderError::NoAdapter`])
/// when no GPU is available, so tests can skip gracefully in headless CI.
pub struct HeadlessRenderer {
    ui: UiRenderer,
    target: wgpu::Texture,
    width: u32,
    height: u32,
    adapter_name: Option<String>,
}

impl HeadlessRenderer {
    /// Opens the default adapter — unless the **`WGPU_UI_TEST_ADAPTER`**
    /// environment variable names one, in which case it pins exactly like
    /// [`with_adapter`](Self::with_adapter). The env route is what makes a
    /// whole existing test suite adapter-pinnable without touching its
    /// construction sites: export `WGPU_UI_TEST_ADAPTER=llvmpipe` in CI and
    /// every snapshot renders on the software rasterizer, byte-exact across
    /// machines (the same convention as `WGPU_UI_GPU_LOCK`).
    pub fn new(width: u32, height: u32) -> Result<Self, RenderError> {
        let env = std::env::var("WGPU_UI_TEST_ADAPTER").ok();
        Self::with_adapter(width, height, env.as_deref())
    }

    /// Opens a caller-chosen adapter: `filter` is a **case-insensitive
    /// substring match** on the adapter name (e.g. `"llvmpipe"` forces the
    /// software rasterizer), so byte-exact baselines can gate on one known
    /// rasterizer instead of whatever the machine has. No adapter matching →
    /// [`RenderError::NoMatchingAdapter`]. `None` keeps the default
    /// high-performance pick.
    pub fn with_adapter(
        width: u32,
        height: u32,
        filter: Option<&str>,
    ) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let (adapter, device, queue) = match filter {
            None => request_device(&instance, None)?,
            Some(f) => {
                let _create_guard = GpuCreateLock::acquire_if_enabled();
                let needle = f.to_lowercase();
                let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::PRIMARY))
                    .into_iter()
                    .find(|a| a.get_info().name.to_lowercase().contains(&needle))
                    .ok_or_else(|| RenderError::NoMatchingAdapter(f.to_string()))?;
                let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    label: Some("wgpu-ui"),
                    ..Default::default()
                }))
                .map_err(RenderError::NoDevice)?;
                (adapter, device, queue)
            }
        };
        let adapter_name = Some(adapter.get_info().name);
        let ui = UiRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
        let target = offscreen_target(&device, width.max(1), height.max(1));
        Ok(Self {
            ui,
            target,
            width: width.max(1),
            height: height.max(1),
            adapter_name,
        })
    }

    /// Builds the offscreen harness over a host-owned device/queue — the
    /// external-device twin of [`UiRenderer::new`], for hosts that pin
    /// their own adapter (a shared per-binary test context, an app
    /// device). The readback target is `Rgba8UnormSrgb`, like the
    /// self-constructed paths.
    pub fn from_device(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> Self {
        let ui = UiRenderer::new(device, queue, wgpu::TextureFormat::Rgba8UnormSrgb);
        let target = offscreen_target(device, width.max(1), height.max(1));
        Self {
            ui,
            target,
            width: width.max(1),
            height: height.max(1),
            adapter_name: None,
        }
    }

    /// The name of the adapter this renderer opened, for harnesses that
    /// refuse to (re)write baselines off the pinned rasterizer. `None` when
    /// constructed [`from_device`](Self::from_device) — the host owns the
    /// adapter and knows what it picked.
    pub fn adapter_name(&self) -> Option<&str> {
        self.adapter_name.as_deref()
    }

    pub fn device(&self) -> &wgpu::Device {
        self.ui.device()
    }

    pub fn queue(&self) -> &wgpu::Queue {
        self.ui.queue()
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Sets the UI scale (see [`UiRenderer::set_scale`]).
    pub fn set_scale(&mut self, scale: f32) {
        self.ui.set_scale(scale);
    }

    /// Registers a host RGBA8 texture (see [`UiRenderer::register_texture`]).
    pub fn register_texture(&mut self, rgba: &[u8], w: u32, h: u32) -> TextureId {
        self.ui.register_texture(rgba, w, h)
    }

    /// Updates a registered texture's pixels (see [`UiRenderer::update_texture`]).
    pub fn update_texture(&self, id: TextureId, rgba: &[u8]) {
        self.ui.update_texture(id, rgba);
    }

    /// Renders `list` over `clear` (resolving glyphs through `fonts`) and returns
    /// tightly packed RGBA8 pixels.
    pub fn render_to_rgba(&mut self, fonts: &Fonts, list: &DrawList, clear: Rgba) -> Vec<u8> {
        let view = self
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .ui
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let size = (self.width, self.height);
        self.ui
            .record(&mut encoder, &view, size, fonts, list, Some(clear));
        self.ui.queue().submit([encoder.finish()]);
        read_target_rgba(
            self.ui.device(),
            self.ui.queue(),
            &self.target,
            self.width,
            self.height,
        )
    }

    /// Renders `list` and writes the result as a PNG (creating parent dirs).
    pub fn screenshot(
        &mut self,
        fonts: &Fonts,
        list: &DrawList,
        clear: Rgba,
        path: &Path,
    ) -> std::io::Result<()> {
        let pixels = self.render_to_rgba(fonts, list, clear);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        png::write_rgba(path, self.width, self.height, &pixels)
    }
}

/// Serializes GPU work across the lib unittests binary's parallel test threads.
/// Every GPU test holds this for its whole body, so at most one touches the GPU
/// at a time — some drivers segfault otherwise (concurrent device use). Held via
/// [`test_gpu`] for the rendering tests; acquired directly by the error/limits
/// tests that build their own device.
#[cfg(test)]
pub(crate) fn gpu_serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// One shared headless device for every rendering test in the lib unittests
/// binary (this module and its submodules, e.g. `atlas`), plus the [`gpu_serial`]
/// guard that keeps GPU work single-threaded. Bind the guard for the test body:
/// `let Some((device, queue, _serial)) = test_gpu() else { return };`. `None`
/// when no adapter exists, so tests skip cleanly.
#[cfg(test)]
pub(crate) fn test_gpu() -> Option<(
    wgpu::Device,
    wgpu::Queue,
    std::sync::MutexGuard<'static, ()>,
)> {
    use std::sync::OnceLock;
    static GPU: OnceLock<Option<(wgpu::Device, wgpu::Queue)>> = OnceLock::new();
    let serial = gpu_serial();
    GPU.get_or_init(|| {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        request_device(&instance, None)
            .ok()
            .map(|(_a, d, q)| (d, q))
    })
    .clone()
    .map(|(d, q)| (d, q, serial))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::DrawList;
    use crate::geom::Rect;
    use crate::text::Fonts;

    /// The bin quantization must mirror cosmic-text's `SubpixelBin::new`:
    /// nearest quarter, with the 4/4 case carrying into the next pixel —
    /// otherwise a rasterized offset and its placement disagree by a pixel.
    #[test]
    fn quarter_bin_rounds_to_nearest_quarter_and_carries() {
        assert_eq!(quarter_bin(5.0), (5.0, 0));
        assert_eq!(quarter_bin(5.1), (5.0, 0));
        assert_eq!(quarter_bin(5.2), (5.0, 1));
        assert_eq!(quarter_bin(5.264), (5.0, 1));
        assert_eq!(quarter_bin(5.5), (5.0, 2));
        assert_eq!(quarter_bin(5.7), (5.0, 3));
        assert_eq!(quarter_bin(5.9), (6.0, 0));
        // Uniform fractional advances stay uniform: consecutive bullet
        // pens at k × 5.264 land 5 or 6 px apart under ROUNDING, but
        // floor+bin keeps every placement within a quarter pixel of true.
        for k in 0..8 {
            let x = k as f32 * 5.264;
            let (px, bin) = quarter_bin(x);
            let placed = px + bin as f32 / 4.0;
            assert!((placed - x).abs() <= 0.125 + 1e-4, "k={k}: {placed} vs {x}");
        }
    }

    /// The overlay path ([`UiRenderer::render_into`]) must composite over host
    /// content instead of clearing it — the contract for embedding in a game or
    /// editor that draws its own world first.
    #[test]
    fn overlay_preserves_host_content() {
        let Some((device, queue, _serial)) = test_gpu() else {
            eprintln!("no GPU adapter; skipping overlay_preserves_host_content");
            return;
        };
        let (w, h) = (64u32, 64u32);
        let target = offscreen_target(&device, w, h);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut ui = UiRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
        let fonts = Fonts::new();

        // Pass 1: "host" content — clear to blue.
        let mut host = DrawList::new();
        host.fill_rect(Rect::new(0.0, 0.0, 64.0, 64.0), Rgba::rgb(0, 0, 255));
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        ui.record(&mut enc, &view, (w, h), &fonts, &host, Some(Rgba::BLACK));
        queue.submit([enc.finish()]);

        // Pass 2: UI overlay — a red square, NO clear.
        let mut overlay = DrawList::new();
        overlay.fill_rect(Rect::new(16.0, 16.0, 32.0, 32.0), Rgba::rgb(255, 0, 0));
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        ui.render_into(&mut enc, &view, (w, h), &fonts, &overlay);
        queue.submit([enc.finish()]);

        let buf = read_target_rgba(&device, &queue, &target, w, h);
        let px = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
        };
        assert_eq!(
            px(2, 2),
            [0, 0, 255, 255],
            "host content survives the overlay"
        );
        assert_eq!(px(32, 32), [255, 0, 0, 255], "overlay composites on top");
    }

    /// Several `record`s into ONE encoder/submit must not share mutable GPU
    /// state: each pass keeps its own geometry *and* its own screen-size
    /// uniforms. Regression guard: a persistent uniform buffer written per
    /// `record` made every pass in the submit read the LAST record's target
    /// size (a full-target fill on a small target then covered only its
    /// top-left corner).
    #[test]
    fn shared_submit_keeps_per_record_uniforms() {
        let Some((device, queue, _serial)) = test_gpu() else {
            eprintln!("no GPU adapter; skipping shared_submit_keeps_per_record_uniforms");
            return;
        };
        let mut ui = UiRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
        let fonts = Fonts::new();

        let (aw, ah) = (32u32, 32u32);
        let (bw, bh) = (128u32, 128u32);
        let ta = offscreen_target(&device, aw, ah);
        let tb = offscreen_target(&device, bw, bh);
        let va = ta.create_view(&wgpu::TextureViewDescriptor::default());
        let vb = tb.create_view(&wgpu::TextureViewDescriptor::default());

        // A full-target fill on each target, both recorded into one encoder.
        let mut la = DrawList::new();
        la.fill_rect(
            Rect::new(0.0, 0.0, aw as f32, ah as f32),
            Rgba::rgb(255, 0, 0),
        );
        let mut lb = DrawList::new();
        lb.fill_rect(
            Rect::new(0.0, 0.0, bw as f32, bh as f32),
            Rgba::rgb(0, 0, 255),
        );
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        ui.record(&mut enc, &va, (aw, ah), &fonts, &la, Some(Rgba::BLACK));
        ui.record(&mut enc, &vb, (bw, bh), &fonts, &lb, Some(Rgba::BLACK));
        queue.submit([enc.finish()]);

        // The far corner of each target proves its pass scaled by its OWN size.
        let corner = |buf: &[u8], w: u32, h: u32| {
            let i = (((h - 1) * w + (w - 1)) * 4) as usize;
            [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
        };
        let a = read_target_rgba(&device, &queue, &ta, aw, ah);
        assert_eq!(
            corner(&a, aw, ah),
            [255, 0, 0, 255],
            "the small target's pass uses its own screen-size uniforms"
        );
        let b = read_target_rgba(&device, &queue, &tb, bw, bh);
        assert_eq!(
            corner(&b, bw, bh),
            [0, 0, 255, 255],
            "the large target's pass fills fully too"
        );
    }

    use crate::geom::Vec2;
    use crate::text::FontId;

    /// A window-handle source whose handles are unavailable — the
    /// deterministic way to make surface creation fail without a windowing
    /// system.
    struct HandleLess;

    impl wgpu::rwh::HasWindowHandle for HandleLess {
        fn window_handle(&self) -> Result<wgpu::rwh::WindowHandle<'_>, wgpu::rwh::HandleError> {
            Err(wgpu::rwh::HandleError::Unavailable)
        }
    }

    impl wgpu::rwh::HasDisplayHandle for HandleLess {
        fn display_handle(&self) -> Result<wgpu::rwh::DisplayHandle<'_>, wgpu::rwh::HandleError> {
            Err(wgpu::rwh::HandleError::Unavailable)
        }
    }

    /// Each [`RenderError`] variant's Display must name what failed — these
    /// strings are all a host gets to log when GPU setup goes wrong.
    #[test]
    fn render_errors_display_their_cause() {
        let _serial = gpu_serial();
        // No adapter: an instance with every backend disabled cannot yield one.
        let none = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::empty(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let err = request_device(&none, None).expect_err("no backends, no adapter");
        assert!(matches!(err, RenderError::NoAdapter(_)));
        assert!(err.to_string().contains("no suitable GPU adapter"), "{err}");

        // Surface creation: a window whose handles are unavailable fails in
        // the public `Renderer::new` before any GPU work happens.
        let err = match Renderer::new(HandleLess, 8, 8) {
            Ok(_) => panic!("surface creation must fail without window handles"),
            Err(e) => e,
        };
        assert!(matches!(err, RenderError::CreateSurface(_)));
        assert!(
            err.to_string().contains("failed to create surface"),
            "{err}"
        );

        // No device: limits beyond the adapter make it refuse politely.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) =
            block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        else {
            eprintln!(
                "no GPU adapter; skipping the NoDevice half of render_errors_display_their_cause"
            );
            return;
        };
        let err = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("wgpu-ui impossible"),
            required_limits: wgpu::Limits {
                max_texture_dimension_2d: u32::MAX,
                ..Default::default()
            },
            ..Default::default()
        }))
        .expect_err("limits beyond any adapter");
        let err = RenderError::NoDevice(err);
        assert!(
            err.to_string().contains("failed to open GPU device"),
            "{err}"
        );

        // No matching adapter: the filter miss names what was asked for.
        let err = RenderError::NoMatchingAdapter("llvmpipe".into());
        assert!(
            err.to_string()
                .contains("no GPU adapter matching \"llvmpipe\""),
            "{err}"
        );
    }

    /// The per-size globals cache holds at most 16 entries: a live window
    /// resize mints a one-frame size per event, and the cache must reset
    /// rather than hoard a uniform buffer per size forever. Evicted sizes are
    /// re-created on next use, never stale-indexed.
    #[test]
    fn globals_cache_evicts_after_sixteen_sizes() {
        let Some((device, queue, _serial)) = test_gpu() else {
            eprintln!("no GPU adapter; skipping globals_cache_evicts_after_sixteen_sizes");
            return;
        };
        let mut kit = UiKit::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
        for i in 0..16u32 {
            let idx = kit.globals_index(&device, 100 + i, 50);
            assert_eq!(idx, i as usize, "each new size appends");
        }
        assert_eq!(
            kit.globals_index(&device, 100, 50),
            0,
            "known sizes hit the cache"
        );
        assert_eq!(kit.globals.len(), 16);
        assert_eq!(
            kit.globals_index(&device, 999, 999),
            0,
            "the 17th size clears the cache and restarts"
        );
        assert_eq!(kit.globals.len(), 1);
        assert_eq!(
            kit.globals_index(&device, 100, 50),
            1,
            "an evicted size is re-created, not stale-indexed"
        );
    }

    /// The bundled font plus a draw list referencing 100 distinct glyph
    /// sizes — more coverage area than a 1024² atlas can hold.
    fn oversized_glyph_list() -> (Fonts, FontId, u16, DrawList) {
        let mut fonts = Fonts::new();
        let id = fonts
            .add(include_bytes!("../../assets/DejaVuSans.ttf").to_vec())
            .expect("bundled font parses");
        let gid = fonts.get(id).glyph_index('M');
        let mut list = DrawList::new();
        for px in 100..200 {
            list.glyph(id, gid, px as f32, Vec2::new(0.0, 0.0), Rgba::WHITE);
        }
        (fonts, id, gid, list)
    }

    /// When the default 1024² atlas can't hold a frame's glyphs, the renderer
    /// must grow the sheet and re-pack everything referenced — no glyph may
    /// be silently dropped while the device still has headroom.
    #[test]
    fn atlas_grows_until_all_referenced_glyphs_fit() {
        let Some((device, queue, _serial)) = test_gpu() else {
            eprintln!("no GPU adapter; skipping atlas_grows_until_all_referenced_glyphs_fit");
            return;
        };
        let mut kit = UiKit::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
        assert_eq!(kit.atlas.size(), ATLAS_SIZE);
        let (fonts, id, gid, list) = oversized_glyph_list();
        kit.prepare_glyphs(&device, &queue, &fonts, &list, 1.0);
        assert!(
            kit.atlas.size() >= 2 * ATLAS_SIZE,
            "the atlas grew, got {}",
            kit.atlas.size()
        );
        assert!(!kit.atlas_maxed, "growth sufficed; nothing was dropped");
        for px in 100..200u32 {
            let g = kit
                .glyph_cache
                .get(&(id.0, gid, px, 0))
                .expect("glyph cached");
            assert!(g.w > 0, "the glyph at {px}px packed with a real extent");
        }
    }

    /// At the device's maximum texture size the atlas can no longer grow: it
    /// re-packs once at the same size (evicting stale entries), then drops
    /// what still doesn't fit — caching empties and warning once — instead of
    /// looping or panicking.
    #[test]
    fn atlas_exhaustion_repacks_then_drops_glyphs() {
        let _serial = gpu_serial();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) =
            block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        else {
            eprintln!("no GPU adapter; skipping atlas_exhaustion_repacks_then_drops_glyphs");
            return;
        };
        // A device capped at the default atlas size: the sheet can never grow.
        let Ok((device, queue)) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("wgpu-ui capped"),
            required_limits: wgpu::Limits {
                max_texture_dimension_2d: ATLAS_SIZE,
                ..wgpu::Limits::downlevel_defaults()
            },
            ..Default::default()
        })) else {
            eprintln!(
                "adapter rejects capped limits; skipping atlas_exhaustion_repacks_then_drops_glyphs"
            );
            return;
        };
        let mut kit = UiKit::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
        assert!(
            !kit.grow_atlas(&device, &queue),
            "at the device limit the atlas refuses to grow"
        );
        assert_eq!(kit.atlas.size(), ATLAS_SIZE);

        let (fonts, id, gid, list) = oversized_glyph_list();
        kit.prepare_glyphs(&device, &queue, &fonts, &list, 1.0);
        assert!(kit.atlas_maxed, "exhaustion was reported (once)");
        assert_eq!(
            kit.atlas.size(),
            ATLAS_SIZE,
            "size stays at the device limit"
        );
        let widths: Vec<u32> = (100..200u32)
            .map(|px| {
                kit.glyph_cache
                    .get(&(id.0, gid, px, 0))
                    .expect("every referenced glyph gets a cache entry")
                    .w
            })
            .collect();
        assert!(
            widths.iter().any(|&w| w > 0),
            "the live set packs what fits"
        );
        assert!(
            widths.contains(&0),
            "what cannot fit is cached empty (dropped), not retried forever"
        );
    }

    /// Renders `list` into a fresh `w`x`h` offscreen target through `ui` and
    /// reads the pixels back.
    fn render_pixels(
        ui: &mut UiRenderer,
        fonts: &Fonts,
        list: &DrawList,
        w: u32,
        h: u32,
    ) -> Vec<u8> {
        let device = ui.device().clone();
        let queue = ui.queue().clone();
        let target = offscreen_target(&device, w, h);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        ui.record(&mut enc, &view, (w, h), fonts, list, Some(Rgba::BLACK));
        queue.submit([enc.finish()]);
        read_target_rgba(&device, &queue, &target, w, h)
    }

    /// [`UiRenderer::update_texture`] swaps a registered texture's pixels in
    /// place, and must ignore the atlas id (0) and unknown ids — a stale id
    /// from host bookkeeping must never corrupt the atlas or panic.
    #[test]
    fn update_texture_swaps_pixels_and_ignores_bad_ids() {
        let Some((device, queue, _serial)) = test_gpu() else {
            eprintln!("no GPU adapter; skipping update_texture_swaps_pixels_and_ignores_bad_ids");
            return;
        };
        let mut ui = UiRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
        let fonts = Fonts::new();
        let id = ui.register_texture(&[255, 0, 0, 255], 1, 1);
        let mut list = DrawList::new();
        list.sprite(id, Rect::new(0.0, 0.0, 8.0, 8.0));
        let px0 = |buf: &[u8]| [buf[0], buf[1], buf[2], buf[3]];

        let buf = render_pixels(&mut ui, &fonts, &list, 8, 8);
        assert_eq!(px0(&buf), [255, 0, 0, 255], "registered pixels render");

        ui.update_texture(id, &[0, 255, 0, 255]);
        let buf = render_pixels(&mut ui, &fonts, &list, 8, 8);
        assert_eq!(
            px0(&buf),
            [0, 255, 0, 255],
            "updated pixels replace the old"
        );

        ui.update_texture(TextureId(0), &[7, 7, 7, 255]);
        ui.update_texture(TextureId(42), &[7, 7, 7, 255]);
        let buf = render_pixels(&mut ui, &fonts, &list, 8, 8);
        assert_eq!(px0(&buf), [0, 255, 0, 255], "bogus ids change nothing");
    }

    /// [`UiRenderer::replace_texture`] re-creates a texture at a NEW size in
    /// the same [`TextureId`] slot (a modal preview must not leak a slot per
    /// open), and ignores the atlas id and out-of-range ids.
    #[test]
    fn replace_texture_resizes_in_place_and_ignores_bad_ids() {
        let Some((device, queue, _serial)) = test_gpu() else {
            eprintln!(
                "no GPU adapter; skipping replace_texture_resizes_in_place_and_ignores_bad_ids"
            );
            return;
        };
        let mut ui = UiRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
        let fonts = Fonts::new();
        let id = ui.register_texture(&[255, 0, 0, 255], 1, 1);

        // Replace the 1×1 red with a 2×1 green|blue: same id, new size.
        ui.replace_texture(id, &[0, 255, 0, 255, 0, 0, 255, 255], 2, 1);
        let mut list = DrawList::new();
        list.sprite(id, Rect::new(0.0, 0.0, 8.0, 8.0));
        let buf = render_pixels(&mut ui, &fonts, &list, 8, 8);
        let px = |x: u32| {
            [
                buf[x as usize * 4],
                buf[x as usize * 4 + 1],
                buf[x as usize * 4 + 2],
                buf[x as usize * 4 + 3],
            ]
        };
        assert_eq!(
            px(1),
            [0, 255, 0, 255],
            "left half samples the new left texel"
        );
        assert_eq!(
            px(6),
            [0, 0, 255, 255],
            "right half samples the new right texel"
        );
        assert_eq!(
            ui.kit.user_textures.len(),
            1,
            "the slot was reused, not leaked"
        );
        assert_eq!(
            ui.kit.user_textures[0].1, 2,
            "the slot's stored size tracks the replacement"
        );

        // The atlas id and ids never registered are no-ops.
        ui.replace_texture(TextureId(0), &[9, 9, 9, 255], 1, 1);
        ui.replace_texture(TextureId(5), &[9, 9, 9, 255], 1, 1);
        assert_eq!(ui.kit.user_textures.len(), 1);
        let buf = render_pixels(&mut ui, &fonts, &list, 8, 8);
        assert_eq!(
            [buf[0], buf[1], buf[2], buf[3]],
            [0, 255, 0, 255],
            "bogus replace ids change nothing"
        );
    }
}
