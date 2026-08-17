//! The Generate Random Terrain form's data logic, used by the wgpu-ui dialog
//! in [`crate::uikit_overlay`]: the per-generator row list (which knobs show),
//! the hover hints, the map-aware Surprise Me roll, and the session memory
//! (per-generator last-used parameters). Everything here is a pure
//! `GenParams` transform — the dialog owns the widgets, `EditorState` owns
//! the stepped run.

use std::collections::HashMap;

use map_core::{AccessibilityMode, GenParams, Generator as Gen, Range, Span};

/// Per-generator last-used parameters, remembered for the session so reopening
/// the dialog or switching generator restores what you last set (kept on
/// `EditorState`; not persisted across restarts).
#[derive(Clone)]
pub struct GenMemory {
	pub last: Gen,
	pub params: HashMap<Gen, GenParams>,
}

impl Default for GenMemory {
	fn default() -> Self {
		Self { last: Gen::Islands, params: HashMap::new() }
	}
}

/// One numeric knob group; each shows up to three columns (count, min, max —
/// distances use only min/max, accessibility only a single value).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Knob {
	MainIslands,
	MainDist,
	SmallIslands,
	SmallDist,
	Continents,
	Seas,
	Rivers,
	Lakes,
	Maze,
	Shape,
	DropZones,
	Obstructions,
	Accessibility,
	Decorations,
}

/// The property rows (knob, label, shown column indices) for `generator`,
/// followed by the common rows. The selects + seed are laid out separately.
pub fn rows(generator: Gen) -> Vec<(Knob, &'static str, &'static [usize])> {
	let mut v: Vec<(Knob, &'static str, &'static [usize])> = match generator {
		Gen::Islands => vec![
			(Knob::MainIslands, "main islands", &[0, 1, 2]),
			(Knob::MainDist, "main distance", &[1, 2]),
			(Knob::SmallIslands, "small islands", &[0, 1, 2]),
			(Knob::SmallDist, "small distance", &[1, 2]),
			(Knob::Rivers, "rivers", &[0, 1, 2]),
			(Knob::Lakes, "lakes", &[0, 1, 2]),
		],
		Gen::Continents => vec![
			(Knob::Continents, "continents", &[0, 1, 2]),
			(Knob::Rivers, "rivers", &[0, 1, 2]),
			(Knob::Lakes, "lakes", &[0, 1, 2]),
		],
		Gen::CentralSeas => vec![(Knob::Seas, "seas", &[0, 1, 2]), (Knob::Rivers, "rivers", &[0, 1, 2])],
		Gen::Land => vec![(Knob::Rivers, "rivers", &[0, 1, 2]), (Knob::Lakes, "lakes", &[0, 1, 2])],
		Gen::Rivers | Gen::RiverRaid => vec![(Knob::Rivers, "rivers", &[0, 1, 2])],
		Gen::Maze => vec![(Knob::Maze, "maze", &[0, 1, 2])],
	};
	// Shape only bites where there are organic bodies to shape.
	if matches!(generator, Gen::Islands | Gen::Continents | Gen::CentralSeas | Gen::Land) {
		v.push((Knob::Shape, "shape", &[0]));
	}
	v.push((Knob::DropZones, "drop zones", &[0, 1, 2]));
	v.push((Knob::Obstructions, "obstructions", &[0, 1, 2]));
	v.push((Knob::Accessibility, "accessibility", &[0]));
	v.push((Knob::Decorations, "decorations", &[0, 1, 2]));
	v
}

/// A one-line hint for a knob row (shown in the hint box on hover). Sizes are
/// radii in tiles; distances are the cell gap; river width is tiles across.
pub fn knob_hint(k: Knob) -> &'static str {
	match k {
		Knob::MainIslands => "Large islands: count, then radius range (tiles)",
		Knob::MainDist => "Gap between large islands (tiles)",
		Knob::SmallIslands => "Small islands: count, then radius range (tiles)",
		Knob::SmallDist => "Gap between small islands (tiles)",
		Knob::Continents => "Continents: count, then radius range (tiles)",
		Knob::Seas => "Enclosed seas: count, then radius range (tiles)",
		Knob::Rivers => "Rivers: count, then width range (tiles across)",
		Knob::Lakes => "Lakes: count, then radius range (tiles)",
		Knob::Maze => "Maze: extra loops, then corridor width range (cells)",
		Knob::Shape => "Island / lake outline: 0 = round, 100 = fully random (fractal)",
		Knob::DropZones => "Flat obstruction-free start areas: count + radius",

		Knob::Obstructions => "Impassable feature patches: count + radius (tiles)",
		Knob::Accessibility => "Obstruction density % (paths/labyrinth: road count + width)",
		Knob::Decorations => "Passable decoration patches: count + radius (tiles)",
	}
}

pub const GENERATOR_HINT: &str = "Overall land / water layout";
pub const SYMMETRY_HINT: &str = "Mirror the map for fair-play layouts";
pub const SHORE_HINT: &str = "How land / water coastlines are tiled";
pub const ACCESS_HINT: &str = "Obstruction layout: random scatter, roads (paths), or a maze (labyrinth)";
pub const SEED_HINT: &str = "Reproducible seed - empty rolls a fresh map each press";
pub const SURPRISE_HINT: &str = "Surprise Me! - random values for every property, plus a fresh seed";

/// Read a knob's `(count, min, max)` triple from `p` (a `Span` reads 0 for
/// count; accessibility reads its single value into count).
pub fn get(p: &GenParams, k: Knob) -> (u8, u8, u8) {
	let r = |g: Range| (g.count, g.min, g.max);
	let s = |sp: Span| (0, sp.min, sp.max);
	match k {
		Knob::MainIslands => r(p.main_islands),
		Knob::MainDist => s(p.main_dist),
		Knob::SmallIslands => r(p.small_islands),
		Knob::SmallDist => s(p.small_dist),
		Knob::Continents => r(p.continents),
		Knob::Seas => r(p.seas),
		Knob::Rivers => r(p.rivers),
		Knob::Lakes => r(p.lakes),
		Knob::Maze => r(p.maze),
		Knob::DropZones => r(p.drop_zones),
		Knob::Obstructions => r(p.obstructions),
		Knob::Shape => (p.shape, 0, 0),
		Knob::Accessibility => (p.accessibility, 0, 0),
		Knob::Decorations => r(p.decorations),
	}
}

/// Write a knob's `(count, min, max)` triple into `p` (`max` is kept ≥ `min`;
/// the algorithm clamps the rest).
pub fn set(p: &mut GenParams, k: Knob, count: u8, min: u8, max: u8) {
	let max = max.max(min);
	let r = Range { count, min, max };
	let s = Span { min, max };
	match k {
		Knob::MainIslands => p.main_islands = r,
		Knob::MainDist => p.main_dist = s,
		Knob::SmallIslands => p.small_islands = r,
		Knob::SmallDist => p.small_dist = s,
		Knob::Continents => p.continents = r,
		Knob::Seas => p.seas = r,
		Knob::Rivers => p.rivers = r,
		Knob::Lakes => p.lakes = r,
		Knob::Maze => p.maze = r,
		Knob::DropZones => p.drop_zones = r,
		Knob::Obstructions => p.obstructions = r,
		Knob::Shape => p.shape = count.min(100),
		Knob::Accessibility => p.accessibility = count,
		Knob::Decorations => p.decorations = r,
	}
}

/// A tiny splitmix64 for the Surprise Me button (seeded from the wall clock so
/// each press differs; hand-rolled, no dependency).
struct SurpriseRng(u64);

impl SurpriseRng {
	fn seeded() -> Self {
		let n = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map(|d| d.as_nanos() as u64)
			.unwrap_or(0x1234_5678_9abc_def0);
		Self(n ^ 0x9E37_79B9_7F4A_7C15)
	}
	fn next(&mut self) -> u64 {
		self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
		let mut z = self.0;
		z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
		z ^ (z >> 31)
	}
	/// An inclusive `lo..=hi`.
	fn range(&mut self, lo: u8, hi: u8) -> u8 {
		if hi <= lo {
			return lo;
		}
		lo + (self.next() % (hi - lo + 1) as u64) as u8
	}
}

/// The Surprise Me roll ranges for one knob of a generator: `(count[lo,hi],
/// min[lo,hi], max[lo,hi])` - a value is rolled in each range (`max` lifted to
/// ≥ the rolled `min`). Tuned per generator so a roll stays **balanced and
/// sensible** while showing off that generator's character (sizes in cells: a
/// radius, a river width, or a gap). Continents / Central Seas are sized by map
/// coverage in [`surprise`] instead; Accessibility / Seed are rolled separately.
fn surprise_spec(generator: Gen, k: Knob) -> ([u8; 2], [u8; 2], [u8; 2]) {
	match k {
		// Common knobs - a modest, playable feature field (not a wall of patches).
		Knob::DropZones => ([1, 4], [4, 8], [7, 11]),
		Knob::Obstructions => ([2, 10], [3, 6], [7, 12]),
		Knob::Decorations => ([2, 12], [3, 6], [7, 12]),
		// Islands: a few big islands among many small ones, clearly separated.
		Knob::MainIslands => ([2, 5], [8, 14], [14, 20]),
		Knob::MainDist => ([0, 0], [4, 10], [12, 22]),
		Knob::SmallIslands => ([4, 12], [2, 4], [4, 7]),
		Knob::SmallDist => ([0, 0], [3, 8], [8, 16]),
		// Coverage-targeted in `surprise` (map-aware); unused fallbacks here.
		Knob::Continents => ([1, 2], [20, 30], [28, 40]),
		Knob::Seas => ([1, 3], [12, 20], [18, 26]),
		// Lakes are a feature of Land; elsewhere they're a light accent.
		Knob::Lakes => match generator {
			Gen::Land => ([2, 5], [3, 6], [6, 10]),
			_ => ([1, 4], [2, 5], [4, 8]),
		},
		// Rivers headline Rivers / River Raid / Land; a light accent elsewhere.
		Knob::Rivers => match generator {
			Gen::Rivers => ([2, 5], [5, 5], [16, 16]),     // wide, very curly: width 5-16
			Gen::RiverRaid => ([5, 20], [5, 5], [16, 16]), // many straight: count 5-20, width 5-16
			Gen::Land => ([2, 5], [3, 6], [6, 10]),
			Gen::Continents => ([1, 3], [2, 5], [4, 8]),
			Gen::Islands | Gen::CentralSeas | Gen::Maze => ([0, 2], [2, 4], [3, 6]),
		},
		// Maze: a few loop openings (braid) + a corridor width of ~3-8 cells.
		Knob::Maze => ([0, 4], [3, 5], [5, 8]),
		// A shape from nearly round to fully abstract - never a bare circle.
		Knob::Shape => ([20, 100], [0, 0], [0, 0]),
		Knob::Accessibility => ([0, 0], [0, 0], [0, 0]), // rolled separately
	}
}

/// The blob radius (cells) for `count` bodies to cover `frac` of a `w`×`h` map,
/// clamped to what `place_blobs` allows (~half the short side). Scaling to the
/// map is why Surprise needs the map size.
fn coverage_radius(frac: f32, count: u8, w: usize, h: usize) -> u8 {
	let area = (w * h) as f32;
	let cap = (w.min(h) as f32 / 2.0 - 3.0).max(2.0);
	let r = (frac * area / (count.max(1) as f32 * std::f32::consts::PI)).sqrt();
	r.clamp(2.0, cap).round() as u8
}

/// Fill `p`'s visible properties (for its generator) with random but sensible
/// values — the Surprise Me button. Symmetry and shore are left as set; the
/// accessibility mode and a fresh reproducible seed are rolled (the seed is
/// stored in `p.seed` and returned so the dialog can show it).
pub fn surprise(p: &mut GenParams, map_w: usize, map_h: usize) -> u64 {
	let mut rng = SurpriseRng::seeded();
	// Now and then (~1 in 3) roll a dense obstruction field across the full
	// accessibility range, to show off heavy obstructions + the paths /
	// labyrinth carving; otherwise keep a balanced, sensible feature count.
	let heavy = rng.range(0, 2) == 0;
	for (k, _, _) in rows(p.generator) {
		// Continents / Central Seas are sized to cover a fraction of the map:
		// continents fill MOST of it; the seas span 40-80%. A single body is
		// used because its coverage tracks the target accurately (two bodies
		// pack with moats and fall short).
		let coverage = match k {
			Knob::Continents => Some((62u8, 88u8, 1u8, 1u8)),
			Knob::Seas => Some((40u8, 82u8, 1u8, 1u8)),
			_ => None,
		};
		if let Some((flo, fhi, clo, chi)) = coverage {
			let count = rng.range(clo, chi);
			let frac = rng.range(flo, fhi) as f32 / 100.0;
			let r = coverage_radius(frac, count, map_w, map_h);
			set(p, k, count, r, r);
			continue;
		}
		if k == Knob::Accessibility {
			let v = if heavy { rng.range(0, 100) } else { rng.range(30, 85) };
			set(p, k, v, 0, 0);
			continue;
		}
		let (cnt, mn, mx) = if heavy && k == Knob::Obstructions {
			([12, 40], [4, 9], [8, 15]) // a wall of obstructions to carve through
		} else {
			surprise_spec(p.generator, k)
		};
		let count = rng.range(cnt[0], cnt[1]);
		let min = rng.range(mn[0], mn[1]);
		let max = rng.range(mx[0], mx[1]).max(min);
		set(p, k, count, min, max);
	}
	p.accessibility_mode = AccessibilityMode::ALL[rng.range(0, AccessibilityMode::ALL.len() as u8 - 1) as usize];
	// Pre-fill a fresh random seed so the surprise is reproducible.
	let seed = rng.next();
	p.seed = seed;
	seed
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn get_set_round_trip_every_knob() {
		let mut p = GenParams::defaults(Gen::Islands);
		for k in [
			Knob::MainIslands,
			Knob::MainDist,
			Knob::SmallIslands,
			Knob::SmallDist,
			Knob::Continents,
			Knob::Seas,
			Knob::Rivers,
			Knob::Lakes,
			Knob::Maze,
			Knob::Shape,
			Knob::DropZones,
			Knob::Obstructions,
			Knob::Accessibility,
			Knob::Decorations,
		] {
			set(&mut p, k, 3, 5, 9);
			let (c, mn, mx) = get(&p, k);
			match k {
				// Spans have no count; accessibility is a single value.
				Knob::MainDist | Knob::SmallDist => assert_eq!((c, mn, mx), (0, 5, 9)),
				Knob::Shape | Knob::Accessibility => assert_eq!((c, mn, mx), (3, 0, 0)),
				_ => assert_eq!((c, mn, mx), (3, 5, 9)),
			}
		}
		// max is lifted to ≥ min on write.
		set(&mut p, Knob::Rivers, 1, 9, 2);
		assert_eq!(get(&p, Knob::Rivers), (1, 9, 9));
	}

	#[test]
	fn surprise_scales_continents_to_the_map_and_rolls_a_seed() {
		let mut p = GenParams::defaults(Gen::Continents);
		let seed = surprise(&mut p, 64, 64);
		assert_eq!(p.seed, seed, "the rolled seed is stored for reproducibility");
		let (count, min, max) = get(&p, Knob::Continents);
		assert!(count >= 1 && min >= 2 && max >= min, "coverage-sized body: {count} r{min}..{max}");
		assert!(max <= 29, "radius capped to ~half the short side (64/2 - 3)");
		// Every generator's row list rolls without panicking.
		for g in Gen::ALL {
			let mut p = GenParams::defaults(g);
			surprise(&mut p, 48, 96);
		}
	}

	const ALL_KNOBS: [Knob; 14] = [
		Knob::MainIslands,
		Knob::MainDist,
		Knob::SmallIslands,
		Knob::SmallDist,
		Knob::Continents,
		Knob::Seas,
		Knob::Rivers,
		Knob::Lakes,
		Knob::Maze,
		Knob::Shape,
		Knob::DropZones,
		Knob::Obstructions,
		Knob::Accessibility,
		Knob::Decorations,
	];

	#[test]
	fn every_knob_carries_a_distinct_hover_hint() {
		let mut seen: Vec<&str> = Vec::new();
		for k in ALL_KNOBS {
			let h = knob_hint(k);
			assert!(!h.is_empty(), "{k:?} has a hover hint");
			assert!(!seen.contains(&h), "{k:?} repeats another row's hint");
			seen.push(h);
		}
	}

	#[test]
	fn surprise_specs_keep_ordered_ranges_for_every_generator() {
		// Every generator × knob spec keeps each [lo, hi] pair ordered, so the
		// roll's `range(lo, hi)` never sees an inverted range — including the
		// Continents / Seas fallbacks `surprise` normally sizes map-aware.
		for g in Gen::ALL {
			for k in ALL_KNOBS {
				let (cnt, mn, mx) = surprise_spec(g, k);
				for ([lo, hi], what) in [(cnt, "count"), (mn, "min"), (mx, "max")] {
					assert!(lo <= hi, "{g:?} {k:?} {what} range inverted: [{lo}, {hi}]");
				}
			}
		}
		// Accessibility rolls separately in `surprise`; its table entry is inert.
		let inert = ([0, 0], [0, 0], [0, 0]);
		assert_eq!(surprise_spec(Gen::Islands, Knob::Accessibility), inert, "accessibility is not spec-rolled");
	}
}
