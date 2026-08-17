//! De-animate tile pixels: remap any pixel that sits on a game-animated palette
//! slot (effect shimmer 9..=31 or a water cycle 96..=127) to the nearest
//! NON-animated slot in the tile's own palette.
//!
//! Non-water / non-shore art must never reference a cycled slot - the engine
//! re-tints those every frame, so a land or obstruction pixel parked there
//! shimmers in-game. This canonicalises such a pixel to a static look-alike:
//! near-identical at rest, but no longer animated. Water and shore tiles are
//! the caller's to skip - their animated pixels are legitimate.
//!
//! Defined once, used by two callers:
//!   * the shipped-pack data fix (`examples/deanimate_packs.rs`), which rewrites
//!     LAND / OBSTRUCTION tiles in place; and
//!   * the WRL importer ([`crate::WrlImport`]), which de-animates a non-water /
//!     non-shore WRL tile the same way before matching it, so an imported
//!     original map still lands on the de-animated pack tiles.

use crate::game_palette::apply_game_statics;
use crate::palette::slot_rgb;
use crate::project::{ANIMATED_SLOTS, WATER_SLOTS};

/// Is slot `s` color-cycled by the engine - an effect shimmer slot (9..=31) or
/// a water cycle slot (96..=127)? Never a valid home for non-water / non-shore
/// art.
pub fn animated_slot(s: u8) -> bool {
	ANIMATED_SLOTS.contains(&s) || WATER_SLOTS.contains(&s)
}

/// The nearest usable static slot to `rgb`: the minimum squared-RGB distance
/// over every slot that is neither transparent (0) nor animated. Ties resolve
/// to the lowest slot index, so the mapping is deterministic.
fn nearest_static(palette: &[u8], rgb: [u8; 3]) -> u8 {
	let mut best = 0u8;
	let mut best_d = u32::MAX;
	for s in 0..=255u8 {
		if s == 0 || animated_slot(s) {
			continue;
		}
		let c = slot_rgb(palette, s);
		let d: u32 = (0..3).map(|i| (c[i] as i32 - rgb[i] as i32).pow(2) as u32).sum();
		if d < best_d {
			best_d = d;
			best = s;
		}
	}
	best
}

/// Build the 256-entry remap for `palette`: every animated slot maps to its
/// nearest static look-alike, every other slot maps to itself. The game-owned
/// static slots are baked to their in-game colours first ([`apply_game_statics`])
/// so an effect slot resolves against the colour the engine actually shows -
/// and the result is independent of any stale bytes in `palette`'s static range.
pub fn deanimate_remap(palette: &[u8]) -> [u8; 256] {
	let mut baked = palette.to_vec();
	apply_game_statics(&mut baked);
	let mut remap = [0u8; 256];
	for s in 0..=255u8 {
		remap[s as usize] = if animated_slot(s) { nearest_static(&baked, slot_rgb(&baked, s)) } else { s };
	}
	remap
}

/// Apply a precomputed [`deanimate_remap`] to `pixels` in place; returns how
/// many pixels changed.
pub fn deanimate_with(pixels: &mut [u8], remap: &[u8; 256]) -> usize {
	let mut changed = 0;
	for p in pixels.iter_mut() {
		let r = remap[*p as usize];
		if r != *p {
			*p = r;
			changed += 1;
		}
	}
	changed
}

/// De-animate `pixels` in place under `palette`. Builds the remap each call -
/// prefer [`deanimate_remap`] + [`deanimate_with`] when de-animating many tiles
/// against one palette. Returns how many pixels changed.
pub fn deanimate_pixels(pixels: &mut [u8], palette: &[u8]) -> usize {
	deanimate_with(pixels, &deanimate_remap(palette))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::game_palette::GAME_PALETTE;
	use crate::palette::set_slot_rgb;

	#[test]
	fn animated_ranges() {
		for s in [9u8, 20, 31, 96, 116, 127] {
			assert!(animated_slot(s), "{s} is animated");
		}
		for s in [0u8, 8, 32, 63, 95, 128, 200, 255] {
			assert!(!animated_slot(s), "{s} is static");
		}
	}

	#[test]
	fn maps_animated_pixels_to_an_exact_static_look_alike() {
		// A distinctive colour lives at a dynamic non-water slot (130); park the
		// same colour on a water-cycle slot (100). De-animating slot 100 must
		// resolve to the exact match at 130 (distance 0), the lowest such slot.
		let mut pal = GAME_PALETTE.to_vec();
		let unique = [1u8, 2, 3];
		set_slot_rgb(&mut pal, 130, unique);
		set_slot_rgb(&mut pal, 100, unique);
		let remap = deanimate_remap(&pal);
		assert_eq!(remap[100], 130, "water slot 100 -> its exact static twin 130");
		assert!(!animated_slot(remap[100]) && remap[100] != 0);
		// Identity for a non-animated slot.
		assert_eq!(remap[130], 130);
		assert_eq!(remap[200], 200);
	}

	#[test]
	fn rewrites_only_animated_pixels_and_is_idempotent() {
		let mut pal = GAME_PALETTE.to_vec();
		set_slot_rgb(&mut pal, 130, [1, 2, 3]);
		set_slot_rgb(&mut pal, 100, [1, 2, 3]);
		let remap = deanimate_remap(&pal);
		let mut px = [130u8, 100, 0, 20, 250];
		let changed = deanimate_with(&mut px, &remap);
		assert_eq!(px[0], 130, "static pixel untouched");
		assert_eq!(px[1], 130, "water pixel de-animated");
		assert_eq!(px[2], 0, "transparent untouched");
		assert!(!animated_slot(px[3]), "effect pixel de-animated off its slot");
		assert_eq!(px[4], 250, "static pixel untouched");
		assert_eq!(changed, 2, "exactly the two animated pixels changed");
		// A second pass changes nothing.
		assert_eq!(deanimate_with(&mut px, &remap), 0, "idempotent");
	}
}
