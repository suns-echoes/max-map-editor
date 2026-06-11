//! Interactive minimap dockable: the whole map fitted into
//! the panel body with the current view as a draggable rectangle — click or
//! drag pans, wheel over it zooms. Three sources (header radios):
//! **overworld** (the composed map, sampled per panel pixel), **pass**
//! (passability colors), **minimap** (the in-game minimap bytes).
//!
//! Geometry/hit logic is pure (tested); pixels are CPU-built into a small
//! RGBA texture (rebuilt on revision/mode/size change, palette snapshot at
//! build time) and blitted by `blit.wgsl`.

use crate::blit::BlitPass;
use crate::state::EditorState;
use crate::theme;
use crate::ui::{Rect, SteelMap, UiQuads};

pub const HEADER_H: f32 = 22.0;
const PAD: f32 = 4.0;

/// Pass colors (sRGB): land / water / shore / blocked.
const PASS_RGBA: [[u8; 4]; 4] = [[58, 140, 58, 255], [42, 90, 223, 255], [200, 180, 0, 255], [140, 42, 42, 255]];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
	Overworld,
	Pass,
	Minimap,
}

impl Mode {
	pub const ALL: [Mode; 3] = [Mode::Overworld, Mode::Pass, Mode::Minimap];

	pub fn name(self) -> &'static str {
		match self {
			Mode::Overworld => "overworld",
			Mode::Pass => "pass",
			Mode::Minimap => "minimap",
		}
	}

	pub fn parse(s: &str) -> Option<Mode> {
		Self::ALL.iter().copied().find(|m| m.name() == s)
	}
}

/// The fitted map rect inside a panel body (below the header) + panel px
/// per map cell.
pub fn map_area(map: (u16, u16), body: Rect) -> (Rect, f32) {
	let avail = Rect::new(
		body.x + PAD,
		body.y + HEADER_H + PAD,
		(body.w - 2.0 * PAD).max(1.0),
		(body.h - HEADER_H - 2.0 * PAD).max(1.0),
	);
	let scale = (avail.w / map.0 as f32).min(avail.h / map.1 as f32).max(0.001);
	let (mw, mh) = (map.0 as f32 * scale, map.1 as f32 * scale);
	(Rect::new(avail.x + (avail.w - mw) / 2.0, avail.y + (avail.h - mh) / 2.0, mw, mh), scale)
}

/// Cursor → map cell coords (fractional), clamped to the map — drag-pans
/// keep tracking even when the cursor leaves the fitted rect.
pub fn pan_target(map: (u16, u16), body: Rect, x: f32, y: f32) -> (f32, f32) {
	let (area, scale) = map_area(map, body);
	(
		((x.clamp(area.x, area.x + area.w) - area.x) / scale).min(map.0 as f32),
		((y.clamp(area.y, area.y + area.h) - area.y) / scale).min(map.1 as f32),
	)
}

fn radio_rect(body: Rect, i: usize) -> Rect {
	let w = ((body.w - 4.0) / 3.0 - 2.0).clamp(20.0, 70.0);
	Rect::new(body.x + 2.0 + i as f32 * (w + 2.0), body.y + 2.0, w, HEADER_H - 4.0)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Click {
	Mode(Mode),
	/// Pan the map view to these cell coords (fractional).
	Pan(f32, f32),
}

pub fn click(map: (u16, u16), body: Rect, x: f32, y: f32) -> Option<Click> {
	for (i, m) in Mode::ALL.iter().enumerate() {
		if radio_rect(body, i).contains(x, y) {
			return Some(Click::Mode(*m));
		}
	}
	let (area, _) = map_area(map, body);
	if area.contains(x, y) {
		let (tx, ty) = pan_target(map, body, x, y);
		return Some(Click::Pan(tx, ty));
	}
	None
}

/// Header radios + the current-view rectangle.
pub fn overlay(editor: &EditorState, body: Rect, w: f32, h: f32, map: SteelMap, hot: crate::ui::Hot) -> UiQuads {
	let mut q = UiQuads::with_steel_map(map);
	q.material(body.strip_top(HEADER_H), w, h, theme::TITLE);
	for (i, m) in Mode::ALL.iter().enumerate() {
		let r = radio_rect(body, i);
		let active = *m == editor.minimap_mode;
		q.button_active(r, w, h, active, hot);
		let label = ["over", "pass", "mini"][i];
		q.label_in(label, r, 6.0, crate::ui::FONT_SMALL, w, h, if active { theme::ACCENT } else { theme::INK_DIM });
	}

	// View rectangle: the visible world window, mapped into the fitted rect.
	let map = editor.map_size();
	let (area, scale) = map_area(map, body);
	let zoom = editor.view.zoom;
	let cell_px = crate::render::TILE_PX as f32;
	let (sw, sh) = (editor.screen.0 as f32, editor.screen.1 as f32);
	let x0 = area.x + editor.view.pan[0] / cell_px * scale;
	let y0 = area.y + editor.view.pan[1] / cell_px * scale;
	let vw = sw / zoom / cell_px * scale;
	let vh = sh / zoom / cell_px * scale;
	// Clamp to the fitted rect so an off-map view doesn't bleed out.
	let cx0 = x0.clamp(area.x, area.x + area.w);
	let cy0 = y0.clamp(area.y, area.y + area.h);
	let cx1 = (x0 + vw).clamp(area.x, area.x + area.w);
	let cy1 = (y0 + vh).clamp(area.y, area.y + area.h);
	if cx1 - cx0 >= 2.0 && cy1 - cy0 >= 2.0 {
		q.border(Rect::new(cx0, cy0, cx1 - cx0, cy1 - cy0), w, h, theme::INK);
	}
	q
}

/// Build the source texture's RGBA for `mode` at `tex` resolution.
fn build_rgba(editor: &EditorState, mode: Mode, tex: (u32, u32)) -> Vec<u8> {
	let (tw, th) = tex;
	let map = editor.map_size();
	let mut out = Vec::with_capacity((tw * th * 4) as usize);

	let palette_rgba = |palette: &[u8], index: u8, out: &mut Vec<u8>| {
		let i = index as usize * 3;
		out.extend_from_slice(&[palette[i], palette[i + 1], palette[i + 2], 255]);
	};

	let project = &editor.project;
	match mode {
		Mode::Overworld => {
			// One composed-world sample per texel (nearest "downscale").
			for j in 0..th {
				for i in 0..tw {
					let wx = (i as f32 + 0.5) / tw as f32 * map.0 as f32 * 64.0;
					let wy = (j as f32 + 0.5) / th as f32 * map.1 as f32 * 64.0;
					let (cx, cy) = ((wx / 64.0) as u16, (wy / 64.0) as u16);
					let sub = ((wx % 64.0) as usize, (wy % 64.0) as usize);
					palette_rgba(&project.palette, project.pixel_at(cx, cy, sub), &mut out);
				}
			}
		}
		Mode::Pass => {
			for y in 0..map.1 {
				for x in 0..map.0 {
					let pass = project.pass_at(x, y).unwrap_or(0).min(3);
					out.extend_from_slice(&PASS_RGBA[pass as usize]);
				}
			}
		}
		Mode::Minimap => {
			for y in 0..map.1 {
				for x in 0..map.0 {
					palette_rgba(&project.palette, project.minimap_pixel(x, y), &mut out);
				}
			}
		}
	}
	out
}

/// Texture resolution for a mode: overworld samples at panel resolution,
/// pass/minimap are one texel per cell (blit upscales nearest = chunky).
fn tex_size(editor: &EditorState, mode: Mode, area: Rect) -> (u32, u32) {
	let map = editor.map_size();
	match mode {
		Mode::Overworld => ((area.w.max(1.0)) as u32, (area.h.max(1.0)) as u32),
		_ => (map.0 as u32, map.1 as u32),
	}
}

// ----- GPU side (texture cache over the shared BlitPass) ---------------------

struct Cache {
	mode: Mode,
	revision: u64,
	size: (u32, u32),
	bind_group: wgpu::BindGroup,
}

/// The minimap's source-texture cache; drawing goes through [`BlitPass`].
pub struct MinimapPass {
	cache: Option<Cache>,
}

impl MinimapPass {
	pub fn new() -> Self {
		Self { cache: None }
	}

	/// Drop the cached texture (document replaced).
	pub fn invalidate(&mut self) {
		self.cache = None;
	}

	/// Draw the minimap content into the panel body.
	#[allow(clippy::too_many_arguments)]
	pub fn draw(
		&mut self,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		encoder: &mut wgpu::CommandEncoder,
		target: &wgpu::TextureView,
		blit: &BlitPass,
		editor: &EditorState,
		body: Rect,
		screen: (u32, u32),
	) {
		let mode = editor.minimap_mode;
		let (area, _) = map_area(editor.map_size(), body);
		let size = tex_size(editor, mode, area);
		if size.0 == 0 || size.1 == 0 {
			return;
		}

		let stale = !matches!(
			&self.cache,
			Some(c) if c.mode == mode && c.revision == editor.revision() && c.size == size,
		);
		if stale {
			let rgba = build_rgba(editor, mode, size);
			let bind_group = blit.upload(device, queue, &rgba, size);
			self.cache = Some(Cache { mode, revision: editor.revision(), size, bind_group });
		}
		blit.draw(
			device,
			encoder,
			target,
			&self.cache.as_ref().expect("cache built").bind_group,
			area,
			[0.0, 0.0, 1.0, 1.0],
			body,
			screen,
		);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use map_core::Project;
	use std::path::{Path, PathBuf};

	fn assets_root() -> PathBuf {
		Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/assets")
	}

	fn editor() -> EditorState {
		let project = Project::new(8, 6, &["GREEN".to_string()], &assets_root(), 42).unwrap();
		EditorState::new(project, (800, 600), None, assets_root())
	}

	#[test]
	fn map_area_letterboxes_and_centers() {
		let body = Rect::new(10.0, 30.0, 200.0, 300.0);
		// 8×6 map in a tall body: width-bound, vertically centered.
		let (area, scale) = map_area((8, 6), body);
		assert_eq!(scale, (200.0 - 2.0 * PAD) / 8.0);
		assert_eq!(area.w, 8.0 * scale);
		assert_eq!(area.h, 6.0 * scale);
		assert_eq!(area.x, body.x + PAD);
		let avail_top = body.y + HEADER_H + PAD;
		let avail_h = body.h - HEADER_H - 2.0 * PAD;
		assert!((area.y - (avail_top + (avail_h - area.h) / 2.0)).abs() < 0.01);
	}

	#[test]
	fn pan_target_clamps_and_round_trips() {
		let body = Rect::new(0.0, 0.0, 200.0, 300.0);
		let map = (8u16, 6u16);
		let (area, scale) = map_area(map, body);
		// A point at cell (2.5, 3.0) maps back to itself.
		let (px, py) = (area.x + 2.5 * scale, area.y + 3.0 * scale);
		let (tx, ty) = pan_target(map, body, px, py);
		assert!((tx - 2.5).abs() < 0.01 && (ty - 3.0).abs() < 0.01);
		// Way off the rect clamps to the map edge.
		let (tx, ty) = pan_target(map, body, -999.0, 9999.0);
		assert_eq!((tx, ty), (0.0, 6.0));
	}

	#[test]
	fn click_routes_radios_and_pan() {
		let body = Rect::new(0.0, 0.0, 220.0, 300.0);
		let r1 = radio_rect(body, 1);
		assert!(matches!(click((8, 6), body, r1.x + 2.0, r1.y + 2.0), Some(Click::Mode(Mode::Pass)),));
		let (area, _) = map_area((8, 6), body);
		assert!(matches!(click((8, 6), body, area.x + 5.0, area.y + 5.0), Some(Click::Pan(..)),));
		// The letterbox margin between header and map picks nothing.
		assert!(click((8, 6), body, area.x + 5.0, body.y + HEADER_H + 1.0).is_none());
		assert_eq!(Mode::parse("pass"), Some(Mode::Pass));
		assert_eq!(Mode::parse("nope"), None);
	}

	#[test]
	fn pass_texture_is_all_water_on_a_fresh_map() {
		let e = editor();
		let rgba = build_rgba(&e, Mode::Pass, (8, 6));
		assert_eq!(rgba.len(), 8 * 6 * 4);
		for px in rgba.chunks_exact(4) {
			assert_eq!(px, PASS_RGBA[1], "fresh map is water everywhere");
		}
	}

	#[test]
	fn minimap_texture_uses_palette_colors() {
		let e = editor();
		let p = &e.project;
		let rgba = build_rgba(&e, Mode::Minimap, (8, 6));
		let index = p.minimap_pixel(0, 0) as usize;
		assert_eq!(&rgba[0..3], &p.palette[index * 3..index * 3 + 3]);
	}

	#[test]
	fn overworld_texture_samples_the_composed_world() {
		let e = editor();
		let p = &e.project;
		let rgba = build_rgba(&e, Mode::Overworld, (16, 12));
		// Texel (0,0) samples world center of its footprint: cell (0,0),
		// sub (16,16) for a 16×12 texture over an 8×6 map.
		let index = p.pixel_at(0, 0, (16, 16)) as usize;
		assert_eq!(&rgba[0..3], &p.palette[index * 3..index * 3 + 3]);
	}
}
