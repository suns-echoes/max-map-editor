//! Palette compatibility conversion — the planning half of
//! [`crate::Project::convert_to_compatible_palette`].
//!
//! A WRL's internal palette can say anything; the game ignores every static
//! slot and substitutes [`crate::GAME_PALETTE`] at runtime (contract §1), and
//! it color-cycles the animated ranges (9-31 system effects, 96-127 water
//! cycles). The plan keeps the look as close as possible under the contract:
//!
//! 1. Only colors **actually used** by tile pixels matter — unused slots are
//!    skipped entirely.
//! 2. Pixels on the game-animated slots (9-31) always move off — the engine
//!    cycles its own colors there, so those slots are never trusted and
//!    never used as remap targets.
//! 3. The water cycles (96-127) are map-owned: with
//!    [`ConvertOptions::preserve_water`] their pixels and colors stay
//!    exactly as the map says (the default); without it their pixels are
//!    flattened to static approximations like everything else.
//! 4. Used dynamic non-animated slots (64-95, 128-159) already hold
//!    map-owned, game-legal colors — they stay pinned and double as targets.
//! 5. Every color that has to move reuses an in-game static color when one
//!    matches; the rest are approximated into the **unused** dynamic slots,
//!    refined with a weighted k-means pass (fixed centroids = the reusable
//!    colors, free centroids = the unused dynamic slots) so heavy colors
//!    pull their slot toward them — the lossy best-approximation part.
//!
//! Slot 0 is the art pipeline's transparent index: it never remaps and is
//! never a remap target. Everything is deterministic (stable orderings, no
//! RNG) so converting the same WRL twice gives identical bytes.

use crate::game_palette::GAME_PALETTE;
use crate::project::DYNAMIC_SLOTS;

/// Refinement passes over the free-slot centroids (k-means with the fixed
/// game/static colors participating in assignment only).
const REFINE_ITERS: usize = 10;

/// What the conversion may touch (the Convert Palette modal's checkboxes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConvertOptions {
	/// Keep the water cycle blocks (96-127) byte-identical: their pixels stay
	/// on their slots and their colors stay the map's. Off = de-animate the
	/// water into static approximations.
	pub preserve_water: bool,
}

impl Default for ConvertOptions {
	fn default() -> Self {
		Self { preserve_water: true }
	}
}

/// What the conversion did — the console line's raw material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConvertReport {
	/// Used slots whose pixels moved and whose color survives exactly.
	pub exact: usize,
	/// Used slots whose pixels landed on an approximate color.
	pub approximated: usize,
	/// Used animated slots whose pixels were moved off a cycle (9-31 always;
	/// 96-127 too when water is not preserved).
	pub de_animated: usize,
}

/// The computed conversion: the per-slot pixel remap, the new (compatible)
/// 256×RGB palette, and the tally.
pub struct Plan {
	pub map: [u8; 256],
	pub palette: Vec<u8>,
	pub report: ConvertReport,
}

/// Is a slot color-cycled by the game engine (9-31)? Never trusted, never a
/// remap target.
fn game_animated(slot: u8) -> bool {
	(9..=31).contains(&slot)
}

/// Is a slot in a water cycle block (96-127)? Map-owned but cycled in-game.
fn water(slot: u8) -> bool {
	(96..=127).contains(&slot)
}

/// A slot the plan may fill with a new color when unused: dynamic, not cycled.
fn assignable(slot: u8) -> bool {
	DYNAMIC_SLOTS.contains(&slot) && !water(slot)
}

/// A slot pixels may be remapped onto: its post-conversion color is stable
/// (game-fixed or map-owned) and not cycled. Slot 0 is transparency.
fn target(slot: u8) -> bool {
	slot != 0 && !game_animated(slot) && !water(slot)
}

fn rgb(palette: &[u8], slot: u8) -> [u8; 3] {
	let at = slot as usize * 3;
	[palette[at], palette[at + 1], palette[at + 2]]
}

/// Perceptually weighted squared RGB distance ("redmean") — cheap, decent.
fn dist(a: [u8; 3], b: [u8; 3]) -> u64 {
	let rmean = (a[0] as i64 + b[0] as i64) / 2;
	let dr = a[0] as i64 - b[0] as i64;
	let dg = a[1] as i64 - b[1] as i64;
	let db = a[2] as i64 - b[2] as i64;
	(((512 + rmean) * dr * dr + 1024 * dg * dg + (767 - rmean) * db * db) >> 8) as u64
}

fn dist_f(a: [u8; 3], b: [f64; 3]) -> f64 {
	dist(a, [b[0].round() as u8, b[1].round() as u8, b[2].round() as u8]) as f64
}

/// Does a used slot's pixel content have to move for the map to render
/// in-game as its internal palette says?
fn must_move(slot: u8, internal: &[u8], opts: ConvertOptions) -> bool {
	if slot == 0 {
		return false; // transparency contract
	}
	if game_animated(slot) {
		return true; // the engine cycles its own colors here — never use
	}
	if water(slot) {
		return !opts.preserve_water;
	}
	if DYNAMIC_SLOTS.contains(&slot) {
		return false; // map-owned, game-legal as-is
	}
	// A static slot renders as the game color — off-spec content must move.
	rgb(internal, slot) != rgb(&GAME_PALETTE, slot)
}

/// Plan the conversion of `internal` (the document's internal 256×RGB
/// palette) given per-slot pixel `usage`. `None` when no used slot has to
/// move — the palette is already compatible under `opts`.
pub fn plan(internal: &[u8], usage: &[u64; 256], opts: ConvertOptions) -> Option<Plan> {
	assert_eq!(internal.len(), 768, "palette must be 256*3 bytes");
	let used = |s: u8| usage[s as usize] > 0;
	let moving: Vec<u8> = (0..=255u8).filter(|&s| used(s) && must_move(s, internal, opts)).collect();
	if moving.is_empty() {
		return None;
	}

	// The colors that need a compatible home, weighted by pixel count.
	let mut weight: Vec<([u8; 3], u64)> = Vec::new();
	for &slot in &moving {
		let color = rgb(internal, slot);
		match weight.iter_mut().find(|(c, _)| *c == color) {
			Some((_, w)) => *w += usage[slot as usize],
			None => weight.push((color, usage[slot as usize])),
		}
	}

	// Reusable fixed colors: the in-game statics at their slots, plus every
	// *used* dynamic non-animated slot's own (map-owned, pinned) color.
	let fixed: Vec<([u8; 3], u8)> = (1..=255u8)
		.filter(|&s| target(s) && (!assignable(s) || used(s)))
		.map(|s| {
			let pal = if assignable(s) { internal } else { &GAME_PALETTE[..] };
			(rgb(pal, s), s)
		})
		.collect();
	// The free slots new colors may claim: *unused* dynamic non-animated.
	let free: Vec<u8> = (0..=255u8).filter(|&s| assignable(s) && !used(s)).collect();

	// Seed the free centroids with the colors that gain most from surviving
	// exactly: weight × the miss of the nearest reusable color. Ties break on
	// the color bytes so the plan is deterministic.
	let nearest_fixed = |color: [u8; 3]| fixed.iter().map(|&(c, _)| dist(color, c)).min().unwrap_or(u64::MAX);
	let mut ranked: Vec<([u8; 3], u64)> = weight.iter().map(|&(c, w)| (c, w * nearest_fixed(c))).collect();
	ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
	let mut centroids: Vec<[f64; 3]> = ranked
		.iter()
		.take_while(|&&(_, benefit)| benefit > 0)
		.take(free.len())
		.map(|&(c, _)| [c[0] as f64, c[1] as f64, c[2] as f64])
		.collect();

	// Weighted k-means: every moving color goes to its nearest home (a fixed
	// color or a free centroid); free centroids re-center on what they
	// attracted. Fixed colors participate but never move.
	for _ in 0..REFINE_ITERS {
		let mut sum = vec![[0.0f64; 3]; centroids.len()];
		let mut wsum = vec![0.0f64; centroids.len()];
		for &(c, w) in &weight {
			let best_fixed = nearest_fixed(c) as f64;
			let mut best: Option<(usize, f64)> = None;
			for (k, ctr) in centroids.iter().enumerate() {
				let d = dist_f(c, *ctr);
				if best.is_none_or(|(_, bd)| d < bd) {
					best = Some((k, d));
				}
			}
			if let Some((k, d)) = best.filter(|&(_, d)| d < best_fixed) {
				let _ = d;
				for j in 0..3 {
					sum[k][j] += c[j] as f64 * w as f64;
				}
				wsum[k] += w as f64;
			}
		}
		let mut moved = 0.0;
		for k in 0..centroids.len() {
			if wsum[k] > 0.0 {
				let nc = [sum[k][0] / wsum[k], sum[k][1] / wsum[k], sum[k][2] / wsum[k]];
				moved += (nc[0] - centroids[k][0]).abs() + (nc[1] - centroids[k][1]).abs() + (nc[2] - centroids[k][2]).abs();
				centroids[k] = nc;
			}
		}
		if moved < 0.5 {
			break;
		}
	}

	// The compatible palette: game statics over the internal palette (the
	// dynamic slots — water cycles included — keep the map's colors), then
	// the free slots take the refined centroids.
	let mut palette = internal.to_vec();
	crate::game_palette::apply_game_statics(&mut palette);
	let mut targets: Vec<([u8; 3], u8)> = fixed.clone();
	for (&slot, ctr) in free.iter().zip(&centroids) {
		let color = [ctr[0].round() as u8, ctr[1].round() as u8, ctr[2].round() as u8];
		let at = slot as usize * 3;
		palette[at..at + 3].copy_from_slice(&color);
		targets.push((color, slot));
	}

	// Remap every moving slot to its nearest stable home (lowest slot on a
	// tie); everything else — animated kept ranges included — stays put.
	let mut map = [0u8; 256];
	for s in 0..=255u16 {
		map[s as usize] = s as u8;
	}
	let mut report = ConvertReport::default();
	for &slot in &moving {
		let want = rgb(internal, slot);
		let (best, d) = targets
			.iter()
			.map(|&(c, s)| (s, dist(want, c)))
			.min_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)))
			.expect("target slots exist");
		map[slot as usize] = best;
		if game_animated(slot) || water(slot) {
			report.de_animated += 1;
		}
		if d == 0 {
			report.exact += 1;
		} else {
			report.approximated += 1;
		}
	}

	Some(Plan { map, palette, report })
}

#[cfg(test)]
mod tests {
	use super::*;

	const KEEP: ConvertOptions = ConvertOptions { preserve_water: true };
	const DROP: ConvertOptions = ConvertOptions { preserve_water: false };

	/// A palette that already follows the contract: game statics + arbitrary
	/// dynamic colors.
	fn compatible() -> Vec<u8> {
		let mut p = vec![0u8; 768];
		for slot in 0..=255u8 {
			let at = slot as usize * 3;
			if DYNAMIC_SLOTS.contains(&slot) {
				p[at..at + 3].copy_from_slice(&[slot, slot.wrapping_add(1), slot.wrapping_add(2)]);
			} else {
				p[at..at + 3].copy_from_slice(&GAME_PALETTE[at..at + 3]);
			}
		}
		p
	}

	fn usage_on(slots: &[u8]) -> [u64; 256] {
		let mut u = [0u64; 256];
		for &s in slots {
			u[s as usize] = 100;
		}
		u
	}

	#[test]
	fn compatible_palette_needs_no_plan() {
		// Used statics on game colors + used dynamics + used water: nothing
		// has to move while water is preserved.
		let p = compatible();
		assert!(plan(&p, &usage_on(&[1, 40, 70, 100, 200]), KEEP).is_none());
		// An off-spec static slot that no pixel uses is skipped entirely.
		let mut q = p.clone();
		q[40 * 3] ^= 0xff;
		assert!(plan(&q, &usage_on(&[70]), KEEP).is_none());
	}

	#[test]
	fn game_animated_pixels_always_move_off_their_cycle() {
		// Slot 20 (game-cycled) used — even with the palette byte matching
		// the game's, the engine cycles there, so pixels must move.
		let p = compatible();
		let plan = plan(&p, &usage_on(&[20]), KEEP).expect("9-31 in use forces a plan");
		let to = plan.map[20];
		assert_ne!(to, 20);
		assert!(target(to), "moved to a stable slot, got {to}");
		assert_eq!(rgb(&plan.palette, to), rgb(&p, 20), "the cycle's rest color survives");
		assert_eq!(plan.report.de_animated, 1);
		// And nothing ever remaps ONTO 9-31 or 96-127.
		assert!((0..256).all(|i| !game_animated(plan.map[i]) && !water(plan.map[i]) || plan.map[i] == i as u8));
	}

	#[test]
	fn water_preservation_is_the_callers_choice() {
		let mut p = compatible();
		p[100 * 3..100 * 3 + 3].copy_from_slice(&[4, 5, 6]);
		// Preserved: used water stays put — no plan needed at all.
		assert!(plan(&p, &usage_on(&[100]), KEEP).is_none());
		// Dropped: water pixels flatten to a static approximation.
		let dropped = plan(&p, &usage_on(&[100]), DROP).expect("water in use, not preserved");
		let to = dropped.map[100];
		assert!(to != 100 && target(to), "water flattened to a stable slot, got {to}");
		assert_eq!(rgb(&dropped.palette, to), [4, 5, 6]);
		assert_eq!(dropped.report.de_animated, 1);
	}

	#[test]
	fn off_spec_static_reuses_game_colors_or_claims_a_free_slot() {
		let mut p = compatible();
		// Slot 40 claims the exact color of game slot 45 — reuse, no slot spent.
		p[40 * 3..40 * 3 + 3].copy_from_slice(&GAME_PALETTE[45 * 3..45 * 3 + 3]);
		// Slot 41 claims hot pink — nothing close in the game ramps.
		p[41 * 3..41 * 3 + 3].copy_from_slice(&[0xff, 0x00, 0xee]);
		let plan = plan(&p, &usage_on(&[40, 41]), KEEP).expect("off-spec statics");
		assert_eq!(plan.map[40], 45, "in-game static color reused");
		let to = plan.map[41];
		assert!(assignable(to), "pink claimed a free dynamic slot, got {to}");
		assert_eq!(rgb(&plan.palette, to), [0xff, 0x00, 0xee]);
		assert_eq!(plan.report.exact, 2);
		assert_eq!(plan.report.approximated, 0);
	}

	#[test]
	fn used_dynamic_slots_stay_pinned_and_serve_as_targets() {
		let mut p = compatible();
		p[70 * 3..70 * 3 + 3].copy_from_slice(&[10, 200, 30]);
		// Off-spec static slot 40 wants exactly the color dynamic 70 holds.
		p[40 * 3..40 * 3 + 3].copy_from_slice(&[10, 200, 30]);
		let plan = plan(&p, &usage_on(&[40, 70]), KEEP).expect("plan");
		assert_eq!(plan.map[70], 70, "a used dynamic slot never moves");
		assert_eq!(rgb(&plan.palette, 70), [10, 200, 30], "and keeps its color");
		assert_eq!(plan.map[40], 70, "the static's pixels reuse it");
	}

	#[test]
	fn overflow_colors_approximate_and_kmeans_weights_the_heavy_ones() {
		// More distinct off-spec colors than free slots: every moving slot
		// still lands on a stable target, and a heavily-used color gets an
		// exact (or near-exact) home while a one-pixel tail color may not.
		let free = (0..=255u8).filter(|&s| assignable(s)).count();
		let mut p = compatible();
		let mut usage = [0u64; 256];
		let slots: Vec<u8> = (0..=255u8).filter(|&s| s != 0 && !DYNAMIC_SLOTS.contains(&s)).collect();
		assert!(slots.len() > free);
		for (i, &slot) in slots.iter().enumerate() {
			let at = slot as usize * 3;
			p[at..at + 3].copy_from_slice(&[17 + (i % 13) as u8 * 18, 7 + (i % 7) as u8 * 33, 211 - i as u8]);
			usage[slot as usize] = if i == 0 { 1_000_000 } else { 1 + i as u64 };
		}
		let plan = plan(&p, &usage, KEEP).expect("plan");
		for &slot in &slots {
			assert!(target(plan.map[slot as usize]), "{slot} → stable");
		}
		let heavy = rgb(&p, slots[0]);
		let landed = rgb(&plan.palette, plan.map[slots[0] as usize]);
		assert!(dist(heavy, landed) <= 64, "the dominant color lands near-exactly: {heavy:?} → {landed:?}");
		assert_eq!(plan.report.exact + plan.report.approximated, slots.len());
	}

	#[test]
	fn planning_is_deterministic() {
		let mut p = compatible();
		for slot in 32..=63u8 {
			let at = slot as usize * 3;
			p[at..at + 3].copy_from_slice(&[slot * 2, 255 - slot, 99]);
		}
		let usage = [7u64; 256];
		let a = plan(&p, &usage, KEEP).expect("plan");
		let b = plan(&p, &usage, KEEP).expect("plan");
		assert_eq!(a.map, b.map);
		assert_eq!(a.palette, b.palette);
		assert_eq!(a.report, b.report);
	}

	#[test]
	fn slot_zero_never_moves_and_is_never_a_target() {
		let mut p = compatible();
		p[0..3].copy_from_slice(&[9, 9, 9]); // off-spec "transparent" color
		p[40 * 3..40 * 3 + 3].copy_from_slice(&[0, 0, 0]); // wants pure black
		let mut usage = usage_on(&[40]);
		usage[0] = 50;
		let plan = plan(&p, &usage, KEEP).expect("plan");
		assert_eq!(plan.map[0], 0);
		assert!((1..256).all(|i| plan.map[i] != 0), "nothing remaps onto transparency");
	}
}
