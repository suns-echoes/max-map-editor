//! Resource-marker sprites (View ▸ Resources): the surveyed-resource markers
//! the game paints on the map — RAW / FUEL / GOLD, 17 amount frames each
//! (`RAWMSK0..16`, `FUELMK0..16`, `GOLDMK0..16`). The frame is the cell's
//! resource amount clamped to 0-16, exactly `survey.cpp Survey_RenderMarker`'s
//! `marker_big + min(amount, 16)`; the sprite's hotspot lands on the tile
//! centre. Loaded from the user's own MAX.RES like the unit sprites
//! (`units.rs`); the GPU half (atlas + quad pass) is `markers_render.rs`,
//! input/wiring in `main.rs`.

use std::path::Path;

use max_assets::image::{IndexedFrame, parse_simple_image_indexed};
use max_assets::res::read_res_entry;
use max_assets::save::{CargoMaterial, cargo_amount, cargo_material};

use crate::markers_render::MarkerSlots;
use crate::ui::Rect;
use crate::units::find_max_res;

/// The big markers carry 17 amount frames (0-16); the game shows frame 0 for a
/// surveyed-but-empty cell and steps up to 16 for a rich one.
pub const MARKER_FRAMES: usize = 17;

/// The RES tag prefix for each material's big-marker strip, in
/// [`CargoMaterial::ALL`] order (Raw, Fuel, Gold).
const MARKER_TAGS: [&str; 3] = ["RAWMSK", "FUELMK", "GOLDMK"];

/// The surveyed-resource marker sprites, loaded from MAX.RES. `frames[row]`
/// holds material `row`'s amount frames ([`material_row`] order); a cell picks
/// frame `min(amount, 16)`.
pub struct MarkerLibrary {
	pub frames: Vec<Vec<IndexedFrame>>,
}

impl MarkerLibrary {
	/// Load the 3×17 big resource markers from `<max_path>/MAX.RES`. Each is a
	/// *simple image* (8-byte header + indexed raster) whose header transparent
	/// colour maps to index 0 (the shader discards it) — the game's
	/// `has_transparency` marker path. Errors if MAX.RES is unreachable, or no
	/// marker loads at all.
	pub fn load(max_path: &Path) -> Result<MarkerLibrary, String> {
		let res = find_max_res(max_path)
			.ok_or_else(|| format!("MAX.RES not found in {} - check MaxPath", max_path.display()))?;
		let mut frames = Vec::with_capacity(MARKER_TAGS.len());
		for base in MARKER_TAGS {
			// Frames are contiguous (0..16); stop at the first gap so a partial
			// strip stays index-aligned rather than shifting later amounts.
			let mut strip = Vec::with_capacity(MARKER_FRAMES);
			for i in 0..MARKER_FRAMES {
				let Ok(Some(bytes)) = read_res_entry(&res, &format!("{base}{i}")) else { break };
				let Some(frame) = parse_simple_image_indexed(&bytes) else { break };
				strip.push(frame);
			}
			frames.push(strip);
		}
		if frames.iter().all(Vec::is_empty) {
			return Err(format!("no resource markers found in {}", res.display()));
		}
		Ok(MarkerLibrary { frames })
	}

	/// Material-row `row`'s frame `fi`, clamped to what the strip actually holds
	/// (so a clamped amount and a short strip both resolve to a real frame).
	/// `None` only when that whole strip failed to load.
	pub fn frame_at(&self, row: usize, fi: usize) -> Option<&IndexedFrame> {
		let strip = self.frames.get(row)?;
		strip.get(fi.min(strip.len().checked_sub(1)?))
	}
}

/// Row index of a material in the [`MarkerLibrary::frames`] / atlas layout
/// (matches the [`MARKER_TAGS`] / `CargoMaterial::ALL` order).
pub fn material_row(m: CargoMaterial) -> usize {
	match m {
		CargoMaterial::Raw => 0,
		CargoMaterial::Fuel => 1,
		CargoMaterial::Gold => 2,
	}
}

/// One resource-marker sprite quad: its screen rect, the atlas pixel origin of
/// the frame, and the frame's pixel size (the fragment shader's UV span).
pub struct MarkerQuad {
	pub rect: Rect,
	pub origin: (u32, u32),
	pub sprite: (u32, u32),
}

/// Build the marker quads for the surveyed cargo cells in view — one big marker
/// per resource cell, its hotspot centred on the tile, frame = `min(amount, 16)`
/// (mirrors `Survey_RenderMarker`). `pan`/`zoom` are in map px; `(w, h)` is the
/// physical viewport, so the visible cell span is culled like the other map
/// overlays. Empty when no cargo map is loaded.
pub fn marker_quads(
	cargo: &[u16],
	(mw, mh): (u16, u16),
	lib: &MarkerLibrary,
	slots: &MarkerSlots,
	pan: [f32; 2],
	zoom: f32,
	(w, h): (f32, f32),
) -> Vec<MarkerQuad> {
	let mut quads = Vec::new();
	if cargo.is_empty() || zoom <= 0.0 || mw == 0 || mh == 0 {
		return quads;
	}
	let ts = crate::render::TILE_PX as f32;
	let x0 = (pan[0] / ts).floor().max(0.0) as u16;
	let y0 = (pan[1] / ts).floor().max(0.0) as u16;
	let x1 = (((pan[0] + w / zoom) / ts).ceil().max(0.0) as u16).min(mw.saturating_sub(1));
	let y1 = (((pan[1] + h / zoom) / ts).ceil().max(0.0) as u16).min(mh.saturating_sub(1));
	for cy in y0..=y1 {
		for cx in x0..=x1 {
			let value = cargo[cy as usize * mw as usize + cx as usize];
			let Some(material) = cargo_material(value) else { continue };
			let row = material_row(material);
			// The strip carries frames 0..16; a higher stored amount clamps to 16.
			let fi = (cargo_amount(value) as usize).min(MARKER_FRAMES - 1);
			let (Some(meta), Some(frame)) = (slots.frame(row, fi), lib.frame_at(row, fi)) else { continue };
			// The marker hotspot lands on the tile centre (grid*64 + 32), so the
			// sprite sits over the cell exactly as the game draws it.
			let (center_x, center_y) = (cx as f32 * ts + ts * 0.5, cy as f32 * ts + ts * 0.5);
			quads.push(MarkerQuad {
				rect: Rect::new(
					(center_x - frame.hot_spot_x as f32 - pan[0]) * zoom,
					(center_y - frame.hot_spot_y as f32 - pan[1]) * zoom,
					meta.size.0 as f32 * zoom,
					meta.size.1 as f32 * zoom,
				),
				origin: meta.origin,
				sprite: meta.size,
			});
		}
	}
	quads
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::units_render::SlotMeta;
	use max_assets::save::cargo_compose;

	/// A synthetic 62×62 frame with a centred hotspot, like the real big markers.
	fn frame() -> IndexedFrame {
		IndexedFrame { width: 62, height: 62, hot_spot_x: 31, hot_spot_y: 31, pixels: vec![0; 62 * 62] }
	}

	/// A parallel library + atlas-slot pair: 3 materials × 17 frames.
	fn lib_and_slots() -> (MarkerLibrary, MarkerSlots) {
		let frames: Vec<Vec<IndexedFrame>> = (0..3).map(|_| (0..MARKER_FRAMES).map(|_| frame()).collect()).collect();
		let metas = frames
			.iter()
			.enumerate()
			.map(|(r, strip)| {
				strip
					.iter()
					.enumerate()
					.map(|(f, _)| SlotMeta { origin: ((r * 64) as u32, (f * 64) as u32), size: (62, 62) })
					.collect()
			})
			.collect();
		(MarkerLibrary { frames }, MarkerSlots::from_meta(metas))
	}

	#[test]
	fn material_row_matches_tag_order() {
		assert_eq!(material_row(CargoMaterial::Raw), 0);
		assert_eq!(material_row(CargoMaterial::Fuel), 1);
		assert_eq!(material_row(CargoMaterial::Gold), 2);
	}

	#[test]
	fn frame_at_clamps_to_the_strip() {
		let (lib, _) = lib_and_slots();
		assert!(lib.frame_at(0, 0).is_some(), "frame 0 exists");
		assert!(lib.frame_at(0, MARKER_FRAMES - 1).is_some(), "last frame exists");
		// An amount past the strip clamps to the last frame rather than vanishing.
		assert!(lib.frame_at(0, 999).is_some(), "out-of-range amount clamps, never None");
	}

	#[test]
	fn one_marker_per_resource_cell_centred_on_the_tile() {
		let (lib, slots) = lib_and_slots();
		let (mw, mh) = (4u16, 4u16);
		let mut cargo = vec![0u16; mw as usize * mh as usize];
		cargo[mw as usize + 1] = cargo_compose(0, Some(CargoMaterial::Raw), 15); // cell (1,1), raw 15
		let quads = marker_quads(&cargo, (mw, mh), &lib, &slots, [0.0, 0.0], 1.0, (256.0, 256.0));
		assert_eq!(quads.len(), 1, "exactly one marker for the single resource cell");
		let q = &quads[0];
		// Tile (1,1) centre is (96, 96); the hotspot (31,31) lands there → rect (65,65).
		assert_eq!((q.rect.x, q.rect.y), (65.0, 65.0), "sprite hotspot centred on the tile");
		assert_eq!(q.sprite, (62, 62));
		// Raw is atlas row 0, amount 15 → slot origin (0, 15*64).
		assert_eq!(q.origin, (0, 15 * 64));
	}

	#[test]
	fn cells_outside_the_viewport_are_culled() {
		let (lib, slots) = lib_and_slots();
		let (mw, mh) = (200u16, 200u16);
		let mut cargo = vec![0u16; mw as usize * mh as usize];
		cargo[150 * mw as usize + 150] = cargo_compose(0, Some(CargoMaterial::Gold), 3);
		// A 128×128 viewport at the origin can't reach cell (150,150).
		let quads = marker_quads(&cargo, (mw, mh), &lib, &slots, [0.0, 0.0], 1.0, (128.0, 128.0));
		assert!(quads.is_empty(), "a resource cell off-screen is culled");
	}

	#[test]
	fn empty_cargo_draws_nothing() {
		let (lib, slots) = lib_and_slots();
		assert!(marker_quads(&[], (0, 0), &lib, &slots, [0.0, 0.0], 1.0, (256.0, 256.0)).is_empty());
	}
}
