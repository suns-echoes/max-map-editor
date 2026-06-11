//! Screen-space text: the console overlay (embedded Hack mono atlas,
//! `console_font`) and proportional UI **labels** (embedded MAX atlas,
//! `max_font`). Builds clip-space textured quads on the CPU
//! (one per glyph, plus solid rects) and owns both atlas textures + the
//! alpha-blended pipeline. (Ported from world-editor `text.rs`.)

use wgpu::util::DeviceExt;

use crate::console::Console;
use crate::console_font::{ATLAS, ATLAS_H, ATLAS_W, CELL_H, CELL_W, COUNT, FIRST};
use crate::ui::UiQuads;

const SCALE: f32 = 1.0; // atlas px → screen px (atlas baked at a readable size)
const PANEL_FRAC: f32 = 0.55; // console height as a fraction of the window
const PAD: f32 = 8.0; // inner margin
const GAP: f32 = 6.0; // between the log area and the input line

const PANEL_COLOR: [f32; 4] = [0.04, 0.05, 0.07, 0.88];
const BORDER_COLOR: [f32; 4] = [0.25, 0.70, 0.92, 0.85];
const LOG_COLOR: [f32; 4] = [0.80, 0.86, 0.82, 1.0];
const INPUT_COLOR: [f32; 4] = [0.96, 0.98, 0.92, 1.0];
const CARET_COLOR: [f32; 4] = [0.95, 0.95, 0.55, 0.75];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TextVertex {
	pos: [f32; 2], // clip space
	uv: [f32; 2],  // atlas uv, or (<0) sentinel = solid fill
	color: [f32; 4],
}

impl TextVertex {
	pub fn layout() -> wgpu::VertexBufferLayout<'static> {
		const ATTRS: [wgpu::VertexAttribute; 3] =
			wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];
		wgpu::VertexBufferLayout {
			array_stride: std::mem::size_of::<TextVertex>() as wgpu::BufferAddress,
			step_mode: wgpu::VertexStepMode::Vertex,
			attributes: &ATTRS,
		}
	}
}

pub fn glyph_px() -> (f32, f32) {
	(CELL_W as f32 * SCALE, CELL_H as f32 * SCALE)
}

fn panel_height(h: u32) -> f32 {
	(h as f32 * PANEL_FRAC).floor()
}

fn input_top(h: u32) -> f32 {
	panel_height(h) - PAD - glyph_px().1
}

/// How many log rows fit above the input line — feed to
/// [`Console::set_view_rows`] so scrolling clamps correctly.
pub fn rows_for(h: u32) -> usize {
	let avail = input_top(h) - GAP - PAD;
	(avail / glyph_px().1).floor().max(0.0) as usize
}

/// Build the console overlay: panel, scrollback lines, and the input line.
pub fn console_quads(c: &Console, w: u32, h: u32) -> Vec<TextVertex> {
	let (wf, hf) = (w as f32, h as f32);
	let (gw, gh) = glyph_px();
	let ph = panel_height(h);
	let mut v = Vec::new();

	push_rect(&mut v, 0.0, 0.0, wf, ph, wf, hf, PANEL_COLOR);
	push_rect(&mut v, 0.0, ph - 2.0, wf, ph, wf, hf, BORDER_COLOR);

	// Monospace columns that fit the panel — over-long lines clip instead of
	// running off the window edge.
	let max_chars = (((wf - 2.0 * PAD) / gw).floor() as usize).max(1);

	// Input line at the bottom of the panel, with a caret when live. When the
	// line outgrows the panel, keep the tail (where the caret is) visible.
	let iy = input_top(h);
	let prompt = format!("] {}", c.input());
	let count = prompt.chars().count();
	let shown: String =
		if count + 1 > max_chars { prompt.chars().skip(count + 1 - max_chars).collect() } else { prompt };
	push_text(&mut v, &shown, PAD, iy, wf, hf, INPUT_COLOR);
	if c.scroll() == 0 {
		let cx = PAD + shown.chars().count() as f32 * gw;
		push_rect(&mut v, cx, iy, cx + gw, iy + gh, wf, hf, CARET_COLOR);
	}

	// Scrollback: newest just above the input, older lines stacking upward.
	let rows = rows_for(h);
	let total = c.log().len();
	let end = total - c.scroll().min(total);
	let start = end.saturating_sub(rows);
	let mut y = iy - GAP - gh;
	for line in c.log()[start..end].iter().rev() {
		if line.chars().count() > max_chars {
			let cut: String = line.chars().take(max_chars).collect();
			push_text(&mut v, &cut, PAD, y, wf, hf, LOG_COLOR);
		} else {
			push_text(&mut v, line, PAD, y, wf, hf, LOG_COLOR);
		}
		y -= gh;
	}

	v
}

/// Proportional **label** text in the MAX font, drawn 1:1 from the prerendered
/// atlas for `size` (see [`crate::font`]) — these vertices must go in a
/// `Batch::Label(size)` run (the matching bind group), never mixed with the
/// console/Hack quads. Pixel-snapped (integer origin + integer advances) so
/// nothing drifts. Returns the advance width (px).
#[allow(clippy::too_many_arguments)]
pub fn push_label(v: &mut Vec<TextVertex>, s: &str, x: f32, y: f32, size: u32, w: f32, h: f32, color: [f32; 4]) -> f32 {
	use crate::max_font::{COUNT, FIRST};
	let f = crate::font::sized(size);
	let (gh, aw) = (f.px as f32, f.atlas_w as f32);
	let (x0, y0) = (x.round(), y.round());
	let mut cx = x0;
	for ch in s.chars() {
		let code = ch as u32;
		if code >= FIRST as u32 && code < FIRST as u32 + COUNT {
			let g = (code - FIRST as u32) as usize;
			let adv = f.advance[g] as f32;
			let ox = f.offset[g] as f32;
			let (u0, u1) = (ox / aw, (ox + adv) / aw);
			push_quad(v, cx, y0, cx + adv, y0 + gh, u0, 0.0, u1, 1.0, w, h, color);
			cx += adv;
		}
	}
	cx - x0
}

/// Advance width (px) of a MAX-font label rendered at `px` (snapped to a baked
/// size) — for right-alignment / fit-to-content layout.
pub fn label_width(s: &str, px: f32) -> f32 {
	use crate::max_font::{COUNT, FIRST};
	let f = crate::font::sized(crate::font::snap(px));
	s.chars()
		.filter_map(|c| {
			let code = c as u32;
			(code >= FIRST as u32 && code < FIRST as u32 + COUNT)
				.then(|| f.advance[(code - FIRST as u32) as usize] as f32)
		})
		.sum()
}

/// Truncate `s` with a trailing `...` so it renders no wider than `max_w` px
/// at `px` (snapped). Returns the input unchanged when it already fits — so
/// fixed containers can take dynamic text (file names, status lines) without
/// it escaping the box.
pub fn fit_label(s: &str, px: f32, max_w: f32) -> String {
	if label_width(s, px) <= max_w {
		return s.to_string();
	}
	let budget = max_w - label_width("...", px);
	let mut out = String::new();
	let mut used = 0.0f32;
	for ch in s.chars() {
		let cw = label_width(ch.encode_utf8(&mut [0; 4]), px);
		if used + cw > budget {
			break;
		}
		out.push(ch);
		used += cw;
	}
	out.push_str("...");
	out
}

/// Greedily word-wrap `s` to lines no wider than `max_w` px (at `px`, snapped).
/// Words longer than `max_w` are character-broken so any container width fits.
/// Collapses runs of whitespace; an empty result yields one empty line.
pub fn wrap_lines(s: &str, px: f32, max_w: f32) -> Vec<String> {
	if max_w <= 0.0 {
		return vec![s.to_string()];
	}
	let space = label_width(" ", px).max(1.0);
	let mut lines: Vec<String> = Vec::new();
	let mut cur = String::new();
	let mut cur_w = 0.0f32;
	for word in s.split_whitespace() {
		let ww = label_width(word, px);
		if ww > max_w {
			// Too long for any line: flush, then break the word by characters.
			if !cur.is_empty() {
				lines.push(std::mem::take(&mut cur));
				cur_w = 0.0;
			}
			for ch in word.chars() {
				let cw = label_width(ch.encode_utf8(&mut [0; 4]), px);
				if cur_w + cw > max_w && !cur.is_empty() {
					lines.push(std::mem::take(&mut cur));
					cur_w = 0.0;
				}
				cur.push(ch);
				cur_w += cw;
			}
			continue;
		}
		let add = if cur.is_empty() { ww } else { space + ww };
		if cur_w + add > max_w && !cur.is_empty() {
			lines.push(std::mem::take(&mut cur));
			cur = word.to_string();
			cur_w = ww;
		} else {
			if !cur.is_empty() {
				cur.push(' ');
			}
			cur.push_str(word);
			cur_w += add;
		}
	}
	if !cur.is_empty() {
		lines.push(cur);
	}
	if lines.is_empty() {
		lines.push(String::new());
	}
	lines
}

pub fn push_text(v: &mut Vec<TextVertex>, s: &str, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
	let (gw, gh) = glyph_px();
	let mut cx = x;
	for ch in s.chars() {
		let code = ch as u32;
		if code >= FIRST as u32 && code < FIRST as u32 + COUNT {
			let g = code - FIRST as u32;
			let u0 = (g * CELL_W) as f32 / ATLAS_W as f32;
			let u1 = ((g + 1) * CELL_W) as f32 / ATLAS_W as f32;
			push_quad(v, cx, y, cx + gw, y + gh, u0, 0.0, u1, 1.0, w, h, color);
		}
		cx += gw; // monospace advance (covers space + unknown glyphs)
	}
}

#[allow(clippy::too_many_arguments)]
pub fn push_rect(v: &mut Vec<TextVertex>, x0: f32, y0: f32, x1: f32, y1: f32, w: f32, h: f32, color: [f32; 4]) {
	push_quad(v, x0, y0, x1, y1, -1.0, -1.0, -1.0, -1.0, w, h, color);
}

/// A textured quad with explicit uv corners — the steel pass maps screen
/// pixels to the (REPEAT-sampled) sheet, `color` is the per-vertex tint.
#[allow(clippy::too_many_arguments)]
pub fn push_textured(
	v: &mut Vec<TextVertex>,
	x0: f32,
	y0: f32,
	x1: f32,
	y1: f32,
	uv: [f32; 4],
	w: f32,
	h: f32,
	color: [f32; 4],
) {
	push_quad(v, x0, y0, x1, y1, uv[0], uv[1], uv[2], uv[3], w, h, color);
}

/// A solid-filled triangle from three screen-space points (sentinel uv =
/// solid). For chrome accents like the floating resize-handle grip.
pub fn push_tri(
	v: &mut Vec<TextVertex>,
	p0: (f32, f32),
	p1: (f32, f32),
	p2: (f32, f32),
	w: f32,
	h: f32,
	color: [f32; 4],
) {
	// Snap to the pixel grid so chrome (incl. the bevel trapezoids) stays crisp.
	let nx = |x: f32| x.round() / w * 2.0 - 1.0;
	let ny = |y: f32| 1.0 - y.round() / h * 2.0;
	let vtx = |p: (f32, f32)| TextVertex { pos: [nx(p.0), ny(p.1)], uv: [-1.0, -1.0], color };
	v.extend_from_slice(&[vtx(p0), vtx(p1), vtx(p2)]);
}

#[allow(clippy::too_many_arguments)]
fn push_quad(
	v: &mut Vec<TextVertex>,
	x0: f32,
	y0: f32,
	x1: f32,
	y1: f32,
	u0: f32,
	v0: f32,
	u1: f32,
	v1: f32,
	w: f32,
	h: f32,
	color: [f32; 4],
) {
	// Snap quad edges to the pixel grid (no font/chrome drift); uv is left
	// unsnapped — the sub-pixel sheet/atlas offset is invisible.
	let nx = |x: f32| x.round() / w * 2.0 - 1.0;
	let ny = |y: f32| 1.0 - y.round() / h * 2.0;
	let tl = TextVertex { pos: [nx(x0), ny(y0)], uv: [u0, v0], color };
	let tr = TextVertex { pos: [nx(x1), ny(y0)], uv: [u1, v0], color };
	let br = TextVertex { pos: [nx(x1), ny(y1)], uv: [u1, v1], color };
	let bl = TextVertex { pos: [nx(x0), ny(y1)], uv: [u0, v1], color };
	v.extend_from_slice(&[tl, bl, br, tl, br, tr]);
}

// ----- GPU resources --------------------------------------------------------

/// Upload an R8 coverage atlas and bind it with a sampler (`linear` for the
/// scalable MAX labels, nearest for the pixel-grid console font).
fn make_atlas_bind_group(
	device: &wgpu::Device,
	queue: &wgpu::Queue,
	bgl: &wgpu::BindGroupLayout,
	label: &str,
	data: &[u8],
	width: u32,
	height: u32,
	linear: bool,
) -> wgpu::BindGroup {
	let texture = device.create_texture(&wgpu::TextureDescriptor {
		label: Some(label),
		size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: wgpu::TextureFormat::R8Unorm,
		usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
		view_formats: &[],
	});
	queue.write_texture(
		texture.as_image_copy(),
		data,
		wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width), rows_per_image: Some(height) },
		wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
	);
	let view = texture.create_view(&Default::default());
	let filter = if linear { wgpu::FilterMode::Linear } else { wgpu::FilterMode::Nearest };
	let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
		label: Some(label),
		mag_filter: filter,
		min_filter: filter,
		..Default::default()
	});
	device.create_bind_group(&wgpu::BindGroupDescriptor {
		label: Some(label),
		layout: bgl,
		entries: &[
			wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
			wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
		],
	})
}

/// Upload the brushed-steel sheet (RGBA, sRGB) with a REPEAT/linear sampler —
/// chrome quads tile it by screen position, so the whole UI is one metal sheet.
fn make_steel_bind_group(
	device: &wgpu::Device,
	queue: &wgpu::Queue,
	bgl: &wgpu::BindGroupLayout,
	steel: &crate::skin::Image,
) -> wgpu::BindGroup {
	let (width, height) = steel.size;
	let texture = device.create_texture(&wgpu::TextureDescriptor {
		label: Some("text.steel"),
		size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: wgpu::TextureFormat::Rgba8UnormSrgb,
		usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
		view_formats: &[],
	});
	queue.write_texture(
		texture.as_image_copy(),
		&steel.rgba,
		wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
		wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
	);
	let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
		label: Some("text.steel"),
		address_mode_u: wgpu::AddressMode::Repeat,
		address_mode_v: wgpu::AddressMode::Repeat,
		mag_filter: wgpu::FilterMode::Linear,
		min_filter: wgpu::FilterMode::Linear,
		..Default::default()
	});
	device.create_bind_group(&wgpu::BindGroupDescriptor {
		label: Some("text.steel"),
		layout: bgl,
		entries: &[
			wgpu::BindGroupEntry {
				binding: 0,
				resource: wgpu::BindingResource::TextureView(&texture.create_view(&Default::default())),
			},
			wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
		],
	})
}

/// Both font atlases, the brushed-steel sheet, and the alpha-blended
/// pipelines; draws quad batches over the scene. Shapes/labels sample the
/// coverage atlases (`cover_pipeline`); chrome fills sample the steel sheet
/// (`steel_pipeline`).
pub struct TextPass {
	cover_pipeline: wgpu::RenderPipeline,
	steel_pipeline: wgpu::RenderPipeline,
	bind_group: wgpu::BindGroup,
	/// One prerendered label atlas per `font::SIZES` entry, keyed by px.
	label_bgs: Vec<(u32, wgpu::BindGroup)>,
	steel_bind_group: wgpu::BindGroup,
}

impl TextPass {
	pub fn new(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		format: wgpu::TextureFormat,
		steel: &crate::skin::Image,
	) -> Self {
		let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("text.bg_layout"),
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

		let bind_group = make_atlas_bind_group(device, queue, &bgl, "text.hack_atlas", ATLAS, ATLAS_W, ATLAS_H, false);
		// One prerendered label atlas per size, sampled 1:1 with Nearest so
		// glyphs stay crisp (no per-frame GPU shrink of the 60-px master).
		let label_bgs = crate::font::all()
			.iter()
			.map(|f| {
				let bg = make_atlas_bind_group(device, queue, &bgl, "text.max_atlas", &f.atlas, f.atlas_w, f.px, false);
				(f.px, bg)
			})
			.collect();
		let steel_bind_group = make_steel_bind_group(device, queue, &bgl, steel);

		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("text.shader"),
			source: wgpu::ShaderSource::Wgsl(include_str!("shaders/text.wgsl").into()),
		});
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("text.layout"),
			bind_group_layouts: &[&bgl],
			push_constant_ranges: &[],
		});
		// Two pipelines, identical but for the fragment entry point — coverage
		// (fonts/shapes) vs. steel (chrome fills). Same vertex layout + blend.
		let pipeline = |label: &str, fs: &str| {
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(label),
				layout: Some(&layout),
				vertex: wgpu::VertexState {
					module: &shader,
					entry_point: Some("vs_main"),
					compilation_options: Default::default(),
					buffers: &[TextVertex::layout()],
				},
				fragment: Some(wgpu::FragmentState {
					module: &shader,
					entry_point: Some(fs),
					compilation_options: Default::default(),
					targets: &[Some(wgpu::ColorTargetState {
						format,
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
			})
		};
		let cover_pipeline = pipeline("text.cover_pipeline", "fs_cover");
		let steel_pipeline = pipeline("text.steel_pipeline", "fs_steel");

		Self { cover_pipeline, steel_pipeline, bind_group, label_bgs, steel_bind_group }
	}

	/// The bind group for label size `px` (falls back to the first baked size).
	fn label_bg(&self, px: u32) -> &wgpu::BindGroup {
		&self.label_bgs.iter().find(|(s, _)| *s == px).unwrap_or(&self.label_bgs[0]).1
	}

	/// Draw a quad batch over the existing target contents.
	pub fn draw(
		&self,
		device: &wgpu::Device,
		encoder: &mut wgpu::CommandEncoder,
		target: &wgpu::TextureView,
		vertices: &[TextVertex],
	) {
		let mut quads = UiQuads::default();
		quads.raw_shapes(vertices);
		self.draw_ui(device, encoder, target, &quads);
	}

	/// Draw a UI frame: one vertex buffer, one pass, replaying the quads'
	/// runs **in push order** — switching between the Hack atlas (shapes)
	/// and the MAX atlas (labels) per run, so later panels cover earlier
	/// panels' labels (z-order by draw order).
	pub fn draw_ui(
		&self,
		device: &wgpu::Device,
		encoder: &mut wgpu::CommandEncoder,
		target: &wgpu::TextureView,
		quads: &UiQuads,
	) {
		self.draw_ui_inner(device, encoder, target, quads, None);
	}

	/// `draw_ui` clipped to a screen-space rect (scrollable panel content).
	/// `screen` bounds the scissor — panels can hang off the window.
	pub fn draw_ui_clipped(
		&self,
		device: &wgpu::Device,
		encoder: &mut wgpu::CommandEncoder,
		target: &wgpu::TextureView,
		quads: &UiQuads,
		scissor: crate::ui::Rect,
		screen: (u32, u32),
	) {
		self.draw_ui_inner(device, encoder, target, quads, Some((scissor, screen)));
	}

	fn draw_ui_inner(
		&self,
		device: &wgpu::Device,
		encoder: &mut wgpu::CommandEncoder,
		target: &wgpu::TextureView,
		quads: &UiQuads,
		scissor: Option<(crate::ui::Rect, (u32, u32))>,
	) {
		if quads.verts.is_empty() {
			return;
		}
		let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
			label: Some("text.vertices"),
			contents: bytemuck::cast_slice(&quads.verts),
			usage: wgpu::BufferUsages::VERTEX,
		});
		let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("text.pass"),
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
		if let Some((s, (sw, sh))) = scissor {
			// Clamp to the target; skip the draw when nothing remains.
			let x0 = (s.x.max(0.0) as u32).min(sw);
			let y0 = (s.y.max(0.0) as u32).min(sh);
			let x1 = ((s.x + s.w).max(0.0) as u32).min(sw);
			let y1 = ((s.y + s.h).max(0.0) as u32).min(sh);
			if x1 <= x0 || y1 <= y0 {
				return;
			}
			pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
		}
		pass.set_vertex_buffer(0, buffer.slice(..));
		let mut start = 0u32;
		let mut bound = None;
		for &(kind, end) in &quads.runs {
			if bound != Some(kind) {
				// Steel chrome samples the metal sheet; shapes/labels the
				// coverage atlases — each run picks its pipeline + bind group.
				let (pipeline, bg) = match kind {
					crate::ui::Batch::Shape => (&self.cover_pipeline, &self.bind_group),
					crate::ui::Batch::Label(px) => (&self.cover_pipeline, self.label_bg(px)),
					crate::ui::Batch::Steel => (&self.steel_pipeline, &self.steel_bind_group),
				};
				pass.set_pipeline(pipeline);
				pass.set_bind_group(0, bg, &[]);
				bound = Some(kind);
			}
			pass.draw(start..end as u32, 0..1);
			start = end as u32;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn wrap_breaks_when_narrow_not_when_wide() {
		let s = "tool buttons land with UI-13";
		assert_eq!(wrap_lines(s, 12.0, 10_000.0).len(), 1, "one line when wide");
		let many = wrap_lines(s, 12.0, 80.0);
		assert!(many.len() > 1, "wraps when narrow");
		for line in &many {
			assert!(label_width(line, 12.0) <= 80.5, "no line exceeds the width: {line:?}");
		}
	}

	#[test]
	fn fit_label_truncates_with_ellipsis() {
		let s = "a-very-long-project-name-that-cannot-fit.json";
		assert_eq!(fit_label(s, 12.0, 10_000.0), s, "unchanged when it fits");
		let cut = fit_label(s, 12.0, 90.0);
		assert!(cut.ends_with("..."), "truncated: {cut:?}");
		assert!(cut.len() < s.len());
		assert!(label_width(&cut, 12.0) <= 90.5, "fits the budget: {cut:?}");
	}

	#[test]
	fn wrap_char_breaks_an_overlong_word() {
		// A single word wider than the box must still be broken down to fit.
		let lines = wrap_lines("Supercalifragilistic", 12.0, 40.0);
		assert!(lines.len() > 1, "long word is character-broken");
		for line in &lines {
			assert!(label_width(line, 12.0) <= 40.5, "each piece fits: {line:?}");
		}
	}
}
