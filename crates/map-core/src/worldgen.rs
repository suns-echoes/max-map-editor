//! Random terrain generator: seeded, parameterized,
//! **deterministic** - the same seed + params always produce the same map,
//! so a seed is a shareable recipe. Semantic family classes come from
//! `tiles.props.json` (the LAND variant group fills ground); obstructions
//! and passable decorations stamp as whole multi-tile formations from
//! `tiles.patterns.json` (extracted from the original maps); coastlines
//! are grown by the auto-shore pass of the user's choice (the sweep or
//! loop-walk), so generated maps obey the same shore law as painted ones.
//! One run = one undo unit.
//!
//! Water amount: the field patterns bisect their threshold until the
//! smoothed mask hits the requested % (usually to the cell); River Raid
//! carves until the quota is met, so it may overshoot by part of a stamp.

use crate::pack::{TileKind, Transformable, family_of};
use crate::project::{LAYER_GROUND, LAYER_WATER, Project, Rng, TileRef, Transform, splitmix};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenPattern {
	/// Many separate land blobs; the map border tends to open sea.
	Islands,
	/// One central landmass ringed by ocean.
	Continent,
	/// Connected land filling the map; the water % forms lakes and seas
	/// (smaller land fragments are bridged back to the main mass).
	LandMass,
	/// Solid land cut by meandering rivers.
	RiverRaid,
}

impl GenPattern {
	pub const ALL: [GenPattern; 4] =
		[GenPattern::Islands, GenPattern::Continent, GenPattern::LandMass, GenPattern::RiverRaid];

	pub fn parse(s: &str) -> Result<Self, String> {
		match s {
			"islands" => Ok(GenPattern::Islands),
			"continent" => Ok(GenPattern::Continent),
			"land-mass" => Ok(GenPattern::LandMass),
			"river-raid" => Ok(GenPattern::RiverRaid),
			other => Err(format!("unknown pattern '{other}' (islands|continent|land-mass|river-raid)")),
		}
	}

	/// The command-line word (`GenPattern::parse`'s inverse).
	pub fn name(self) -> &'static str {
		match self {
			GenPattern::Islands => "islands",
			GenPattern::Continent => "continent",
			GenPattern::LandMass => "land-mass",
			GenPattern::RiverRaid => "river-raid",
		}
	}

	/// Human label for UI buttons.
	pub fn label(self) -> &'static str {
		match self {
			GenPattern::Islands => "Islands",
			GenPattern::Continent => "Continent",
			GenPattern::LandMass => "Land Mass",
			GenPattern::RiverRaid => "River Raid",
		}
	}
}

#[derive(Debug, Clone, Copy)]
pub struct GenParams {
	pub pattern: GenPattern,
	/// Percent of cells that become open water (0..=100).
	pub water: u8,
	/// Percent of land cells covered by obstruction formations (0..=100).
	pub obstructions: u8,
	/// Percent of land cells covered by passable decorations (0..=100).
	pub decorations: u8,
	pub seed: u64,
	/// Shore the coastlines with the loop-walk pass (`auto_shore_alt` -
	/// varied coastline) instead of the sweep optimizer.
	pub alt_shore: bool,
}

/// What a run produced (cell counts from the generated mask; `shore` and
/// `unresolved` are the auto-shore pass's report).
pub struct GenStats {
	pub water: usize,
	pub land: usize,
	pub obstructions: usize,
	pub decorations: usize,
	pub shore: usize,
	pub unresolved: usize,
}

/// Per-`step` work budgets. An interactive generate runs one bounded slice per
/// frame so the UI stays responsive; the run-to-completion path just loops
/// `step` until done. (Raising these speeds headless runs at the cost of frame
/// latency; they don't affect the generated map.)
const FIELD_CELLS_PER_STEP: usize = 32_768; // land-ness field samples per step
const STAMP_ATTEMPTS_PER_STEP: usize = 16_384; // formation placement tries per step
const FIX_WORK_PER_STEP: i64 = 200_000; // shore-fix work units per step

// ----- deterministic value noise ---------------------------------------------

/// Lattice value in [0, 1) for an integer grid point.
fn lattice(seed: u64, x: i64, y: i64) -> f32 {
	let h = splitmix(
		seed ^ (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (y as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f),
	);
	(h >> 40) as f32 / (1u64 << 24) as f32
}

fn smooth(t: f32) -> f32 {
	t * t * (3.0 - 2.0 * t)
}

/// Bilinear value noise in [0, 1).
fn value_noise(seed: u64, fx: f32, fy: f32) -> f32 {
	let (x0f, y0f) = (fx.floor(), fy.floor());
	let (tx, ty) = (smooth(fx - x0f), smooth(fy - y0f));
	let (x0, y0) = (x0f as i64, y0f as i64);
	let v00 = lattice(seed, x0, y0);
	let v10 = lattice(seed, x0 + 1, y0);
	let v01 = lattice(seed, x0, y0 + 1);
	let v11 = lattice(seed, x0 + 1, y0 + 1);
	let a = v00 + (v10 - v00) * tx;
	let b = v01 + (v11 - v01) * tx;
	a + (b - a) * ty
}

/// Fractal sum, 4 octaves - per-octave seed offsets decorrelate the lattices.
fn fbm(seed: u64, fx: f32, fy: f32) -> f32 {
	let (mut sum, mut amp, mut norm, mut f) = (0.0f32, 1.0f32, 0.0f32, 1.0f32);
	for octave in 0..4u64 {
		sum += amp * value_noise(seed.wrapping_add(octave.wrapping_mul(0x9e37_79b9_7f4a_7c15)), fx * f, fy * f);
		norm += amp;
		amp *= 0.55;
		f *= 2.0;
	}
	sum / norm
}

// ----- water mask --------------------------------------------------------------

/// `fbm` sampled through two displacement noises (domain warping) - the
/// lattice's straight contour runs wander into organic shapes.
fn warped_fbm(seed: u64, fx: f32, fy: f32) -> f32 {
	const AMP: f32 = 0.9; // displacement in lattice cells
	let wx = fx + AMP * (fbm(seed ^ 0x5741_5250, fx, fy) - 0.5) * 2.0; // "WARP"
	let wy = fy + AMP * (fbm(seed ^ 0x7072_6157, fx, fy) - 0.5) * 2.0;
	fbm(seed, wx, wy)
}

/// Land-ness at one cell, higher = more land (the field the quantile
/// threshold cuts). Bias terms (border sink, radial falloff) are noise-
/// displaced so they cannot draw straight or perfectly circular coasts.
fn field_at(pattern: GenPattern, seed: u64, w: usize, h: usize, x: usize, y: usize) -> f32 {
	let span = w.min(h).max(8) as f32;
	let (xf, yf) = (x as f32, y as f32);
	match pattern {
		// Small blobs + sinking borders → an archipelago. The rim the
		// falloff measures against is itself wavy.
		GenPattern::Islands => {
			let d = xf.min(w as f32 - 1.0 - xf).min(yf).min(h as f32 - 1.0 - yf);
			let rim = (span / 6.0) * (0.5 + fbm(seed ^ 0x52494d, xf / (span / 4.0), yf / (span / 4.0)));
			let edge = (d / rim).min(1.0);
			warped_fbm(seed, xf / (span / 7.0), yf / (span / 7.0)) * 0.8 + edge * 0.2
		}
		// Coarse noise + a noise-perturbed radial bias → one central mass
		// with a lobed, irregular outline.
		GenPattern::Continent => {
			let (cx, cy) = ((w as f32 - 1.0) / 2.0, (h as f32 - 1.0) / 2.0);
			let mut d = (((xf - cx) / (w as f32 / 2.0)).powi(2) + ((yf - cy) / (h as f32 / 2.0)).powi(2)).sqrt();
			d *= 0.8 + 0.4 * fbm(seed ^ 0x524144, xf / (span / 3.0), yf / (span / 3.0));
			warped_fbm(seed, xf / (span / 2.5), yf / (span / 2.5)) * 0.55 + (1.0 - d.min(1.0)) * 0.45
		}
		// Free coarse noise; connectivity is repaired after.
		_ => warped_fbm(seed, xf / (span / 4.0), yf / (span / 4.0)),
	}
}

/// Rotate a direction by `turn` steps of ~7.2°. Built from one hardcoded
/// (cos, sin) pair - pure arithmetic, so walks are deterministic on every
/// platform (std trig is not guaranteed bit-identical across libms).
fn rotated(base: (f32, f32), turn: i32) -> (f32, f32) {
	const C: f32 = 0.992_115; // cos 7.2°
	const S: f32 = 0.125_333; // sin 7.2°
	let s = if turn < 0 { -S } else { S };
	let mut d = base;
	for _ in 0..turn.unsigned_abs() {
		d = (d.0 * C - d.1 * s, d.0 * s + d.1 * C);
	}
	d
}

/// One meander update: the heading offset random-walks with a pull back
/// toward straight, clamped to `limit` rotation steps.
fn meander(turn: &mut i32, limit: i32, rng: &mut Rng) {
	*turn += rng.below(3) as i32 - 1;
	if rng.below(4) == 0 {
		*turn -= turn.signum();
	}
	*turn = (*turn).clamp(-limit, limit);
}

/// Two rounds of 9-cell majority vote: single-cell jags, hairline straits,
/// and pinholes melt into the coast, leaving blob shapes the shore band can
/// actually wrap.
fn smooth_mask(mask: &mut [bool], w: usize, h: usize) {
	for _round in 0..2 {
		let prev = mask.to_vec();
		for y in 0..h as i32 {
			for x in 0..w as i32 {
				let (mut wet, mut total) = (0u32, 0u32);
				for dy in -1i32..=1 {
					for dx in -1i32..=1 {
						let (nx, ny) = (x + dx, y + dy);
						if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
							continue;
						}
						total += 1;
						wet += prev[ny as usize * w + nx as usize] as u32;
					}
				}
				mask[y as usize * w + x as usize] = wet * 2 > total;
			}
		}
	}
}

/// Open diagonal water pinches: two water cells touching only at a corner
/// force the shore bands to cross diagonally, which the tilesets cannot
/// express - widen the pinch into water until none remain.
fn depinch(mask: &mut [bool], w: usize, h: usize) {
	loop {
		let mut changed = false;
		for y in 0..h - 1 {
			for x in 0..w - 1 {
				let i = y * w + x;
				// The 2×2 quad: a b / c d.
				let (a, b, c, d) = (mask[i], mask[i + 1], mask[i + w], mask[i + w + 1]);
				if a && d && !b && !c {
					mask[i + 1] = true;
					changed = true;
				} else if b && c && !a && !d {
					mask[i] = true;
					changed = true;
				}
			}
		}
		if !changed {
			break;
		}
	}
}

/// All land, then meandering rivers carve across the map until the water
/// quota is met. Each river enters on one edge aimed at the opposite one
/// and walks with a random-walking heading (clamped to ±~58°, so it always
/// crosses) - curves, not corridors. Constant width per river: width
/// changes mid-run leave stair-step notches the shore tileset struggles to
/// continue.
fn rivers_mask(p: &GenParams, w: usize, h: usize, rng: &mut Rng) -> Vec<bool> {
	let n = w * h;
	let mut mask = vec![false; n];
	let quota = p.water as usize * n / 100;
	let mut wet = 0usize;
	let stamp = |mask: &mut Vec<bool>, x: i32, y: i32, r: i32, wet: &mut usize| {
		for dy in -r..=r {
			for dx in -r..=r {
				if dx * dx + dy * dy > r * r {
					continue;
				}
				let (px, py) = (x + dx, y + dy);
				if px < 0 || py < 0 || px >= w as i32 || py >= h as i32 {
					continue;
				}
				let i = py as usize * w + px as usize;
				if !mask[i] {
					mask[i] = true;
					*wet += 1;
				}
			}
		}
	};
	for _river in 0..256 {
		if wet >= quota {
			break;
		}
		let vertical = rng.below(2) == 0;
		let radius = 1 + rng.below(2) as i32;
		let (mut px, mut py, base) = if vertical {
			(rng.below(w as u32) as f32, 0.0, (0.0f32, 1.0f32))
		} else {
			(0.0, rng.below(h as u32) as f32, (1.0, 0.0))
		};
		let mut turn = 0i32;
		for _step in 0..3 * (w + h) {
			stamp(&mut mask, px.round() as i32, py.round() as i32, radius, &mut wet);
			meander(&mut turn, 8, rng);
			let dir = rotated(base, turn);
			px += dir.0;
			py += dir.1;
			if px < -1.0 || py < -1.0 || px > w as f32 || py > h as f32 {
				break;
			}
		}
	}
	depinch(&mut mask, w, h);
	mask
}

/// Bridge every secondary land component back to the largest one with a
/// 3-wide land causeway (LandMass promises *connected* land).
fn connect_land(mask: &mut [bool], w: usize, h: usize, rng: &mut Rng) {
	let n = w * h;
	// Label 4-connected land components (one shared `seen` across the sweep so
	// each cell is visited once; `comp` carries the labels downstream).
	let mut comp = vec![u32::MAX; n];
	let mut sizes: Vec<usize> = Vec::new();
	let mut seen = vec![false; n];
	for start in 0..n {
		if mask[start] || seen[start] {
			continue;
		}
		let id = sizes.len() as u32;
		let mut size = 0usize;
		crate::grid::flood4(
			w,
			h,
			start,
			&mut seen,
			|j| !mask[j],
			|i| {
				comp[i] = id;
				size += 1;
			},
		);
		sizes.push(size);
	}
	if sizes.len() <= 1 {
		return;
	}
	let main = sizes.iter().enumerate().max_by_key(|&(_, s)| *s).unwrap().0 as u32;

	// One causeway per secondary component: from a random cell of it, walk
	// home toward a random cell of the main mass with a wobbling heading
	// (re-aimed every step, clamped to ±~43° - arrival is guaranteed, the
	// path is not straight), stamping a plus-shape (3-wide - auto-shore
	// needs room to ring both banks).
	let cells_of = |comp: &[u32], id: u32, rng: &mut Rng| -> usize {
		let cells: Vec<usize> = (0..n).filter(|&i| comp[i] == id).collect();
		cells[rng.below(cells.len() as u32) as usize]
	};
	for id in 0..sizes.len() as u32 {
		if id == main {
			continue;
		}
		let from = cells_of(&comp, id, rng);
		let to = cells_of(&comp, main, rng);
		let (mut px, mut py) = ((from % w) as f32, (from / w) as f32);
		let (tx, ty) = ((to % w) as f32, (to / w) as f32);
		let stamp = |mask: &mut [bool], x: i32, y: i32| {
			for (dx, dy) in [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)] {
				let (qx, qy) = (x + dx, y + dy);
				if qx >= 0 && qy >= 0 && qx < w as i32 && qy < h as i32 {
					mask[qy as usize * w + qx as usize] = false;
				}
			}
		};
		let mut turn = 0i32;
		for _step in 0..4 * (w + h) {
			stamp(mask, px.round() as i32, py.round() as i32);
			let (dx, dy) = (tx - px, ty - py);
			let dist = (dx * dx + dy * dy).sqrt();
			if dist <= 1.2 {
				stamp(mask, tx as i32, ty as i32);
				break;
			}
			meander(&mut turn, 6, rng);
			let dir = rotated((dx / dist, dy / dist), turn);
			px += dir.0;
			py += dir.1;
		}
	}
	depinch(mask, w, h);
}

// ----- tiles ------------------------------------------------------------------

/// A random transform a family's props allow.
fn random_transform(t: Transformable, rng: &mut Rng) -> Transform {
	match t {
		Transformable::Free => Transform { rot: rng.below(4) as u8, mirror: rng.below(2) == 1 },
		Transformable::Invert => Transform { rot: rng.below(2) as u8 * 2, mirror: false },
		Transformable::No | Transformable::Sync => Transform::default(),
	}
}

/// One thing the generator can put down whole.
enum Stamp {
	/// A `tiles.patterns.json` formation: populated cells as offsets,
	/// stamped untransformed (the formation's light is baked as authored).
	Pattern { w: i32, h: i32, cells: Vec<(i32, i32, u16)> },
	/// A single tile from an interchangeable variant group (e.g. CRATER's
	/// `CHa` hills) - picked and transformed per placement.
	Single { tiles: Vec<u16>, spin: Transformable },
}

/// Stamp from `pool` onto `overlay` toward `target` covered cells, at most
/// `chunk` placement attempts per call (the session steps this between
/// frames). Every populated cell must be land at Chebyshev distance ≥ 2
/// from water - clear of the shore band and the seam-fix solver's reach -
/// and not already claimed. Returns `true` when the pass is finished
/// (target met or the attempt budget ran out).
#[allow(clippy::too_many_arguments)]
fn stamp_chunk(
	overlay: &mut [Option<TileRef>],
	pool: &[Stamp],
	target: usize,
	placed: &mut usize,
	attempts: &mut usize,
	chunk: usize,
	mask: &[bool],
	(w, h): (usize, usize),
	pack_idx: u8,
	rng: &mut Rng,
) -> bool {
	if target == 0 || pool.is_empty() {
		return true;
	}
	let n = w * h;
	let eligible = |overlay: &[Option<TileRef>], x: i32, y: i32| -> bool {
		if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
			return false;
		}
		if overlay[y as usize * w + x as usize].is_some() {
			return false;
		}
		for dy in -2i32..=2 {
			for dx in -2i32..=2 {
				let (px, py) = (x + dx, y + dy);
				if px < 0 || py < 0 || px >= w as i32 || py >= h as i32 {
					continue; // the map edge is fine to hug
				}
				if mask[py as usize * w + px as usize] {
					return false;
				}
			}
		}
		true
	};
	let stop = (*attempts + chunk).min(8 * n.max(64));
	while *placed < target && *attempts < stop {
		*attempts += 1;
		match &pool[rng.below(pool.len() as u32) as usize] {
			Stamp::Single { tiles, spin } => {
				let at = rng.below(n as u32) as usize;
				let (x, y) = ((at % w) as i32, (at / w) as i32);
				if eligible(overlay, x, y) {
					let tile = tiles[rng.below(tiles.len() as u32) as usize];
					overlay[at] = Some(TileRef { pack: pack_idx, tile, transform: random_transform(*spin, rng) });
					*placed += 1;
				}
			}
			Stamp::Pattern { w: pw, h: ph, cells } => {
				if *pw > w as i32 || *ph > h as i32 {
					continue;
				}
				let x0 = rng.below((w as i32 - pw + 1) as u32) as i32;
				let y0 = rng.below((h as i32 - ph + 1) as u32) as i32;
				if cells.iter().all(|&(dx, dy, _)| eligible(overlay, x0 + dx, y0 + dy)) {
					for &(dx, dy, tile) in cells {
						overlay[(y0 + dy) as usize * w + (x0 + dx) as usize] =
							Some(TileRef { pack: pack_idx, tile, transform: Transform::default() });
					}
					*placed += cells.len();
				}
			}
		}
	}
	*placed >= target || *attempts >= 8 * n.max(64)
}

/// Where a [`GenSession`] is in its pipeline.
enum Phase {
	/// Build the land-ness field, a chunk of rows per step.
	Field {
		row: usize,
	},
	/// One quantile probe per step: threshold + smooth + depinch + count,
	/// bisecting until the smoothed mask hits the water target.
	Bisect {
		iter: u32,
	},
	/// River Raid: carve the rivers (one step - it's cheap).
	Rivers,
	/// Land Mass: bridge secondary islands (one step).
	Connect,
	/// Stamp formations, a chunk of attempts per step.
	Stamp {
		decorations: bool,
	},
	/// Open the stroke and lay both layers - fresh water across the whole
	/// bottom layer, the generated terrain on the ground layer (one step).
	Apply,
	/// Grow the coastlines (one step - the chosen auto-shore pass is not
	/// resumable; the longest single hitch on very large maps).
	Shore,
	/// Run the Destructive seam-fix solver, a budget slice per step.
	Fix,
	Done,
}

/// A resumable terrain generation: created cheaply, advanced by [`step`]
/// (bounded work per call - the shell drives it per frame so the UI never
/// freezes), abortable mid-run. Nothing touches the project before the
/// Apply phase; from there every edit lives in one open stroke, so
/// [`abort`] rolls the document back as if the run never happened, and a
/// completed run is one undo unit.
///
/// [`step`]: GenSession::step
/// [`abort`]: GenSession::abort
pub struct GenSession {
	p: GenParams,
	w: usize,
	h: usize,
	pack_no: u8,
	land_tiles: Vec<u16>,
	land_spin: Transformable,
	// The water refill: Apply rewrites the whole bottom layer from this
	// variant group, so nothing of the previous map survives the run.
	water_pack_no: u8,
	water_tiles: Vec<u16>,
	water_spin: Transformable,
	obstruction_pool: Vec<Stamp>,
	decoration_pool: Vec<Stamp>,
	rng: Rng,
	phase: Phase,
	// Field + bisection state.
	field: Vec<f32>,
	sorted: Vec<f32>,
	lo: usize,
	hi: usize,
	best: usize,
	target: usize,
	mask: Vec<bool>,
	// Stamping state.
	overlay: Vec<Option<TileRef>>,
	placed: usize,
	attempts: usize,
	stamp_target: usize,
	obstructions_placed: usize,
	// Post-pass state.
	shore_changed: usize,
	fix: Option<crate::FixSession>,
	fix_found: usize,
	/// `project.dirty()` at session start - restored on abort.
	was_dirty: bool,
	/// The stroke is open (Apply ran) - abort must roll back.
	mutated: bool,
	stats: Option<GenStats>,
}

impl GenSession {
	/// Validate params, resolve the land pack and stamp pools. Cheap; the
	/// project is not touched.
	pub fn new(project: &Project, p: GenParams) -> Result<Self, String> {
		if p.water > 100 || p.obstructions > 100 || p.decorations > 100 {
			return Err("water/obstruction/decoration percentages are 0..=100".into());
		}
		// The land pack: first pack with a LAND variant group (props-driven).
		let (pack_idx, land_family) = project
			.packs
			.iter()
			.enumerate()
			.find_map(|(i, pack)| {
				let mut families: Vec<&String> = pack
					.props
					.iter()
					.filter(|(_, fp)| fp.kind == Some(TileKind::Land) && fp.has_variants)
					.map(|(f, _)| f)
					.collect();
				families.sort(); // HashMap order is not deterministic - the run must be
				families.first().map(|f| (i, (*f).clone()))
			})
			.ok_or("no pack with a LAND variant group (tiles.props.json) - add a tileset like GREEN")?;
		// The water pack + its WATER variant group: the run starts from a
		// clean slate - the whole bottom layer refills from this group and
		// the ground layer is fully rewritten, so nothing of the previous
		// map survives under the generated terrain.
		let (water_pack_idx, water_family) = project
			.packs
			.iter()
			.enumerate()
			.find_map(|(i, pack)| {
				let mut families: Vec<&String> = pack
					.props
					.iter()
					.filter(|(_, fp)| fp.kind == Some(TileKind::Water) && fp.has_variants)
					.map(|(f, _)| f)
					.collect();
				families.sort(); // HashMap order is not deterministic - the run must be
				families.first().map(|f| (i, (*f).clone()))
			})
			.ok_or("no pack with a WATER variant group (tiles.props.json) - add the WATER tileset")?;
		let water_pack = &project.packs[water_pack_idx];
		let water_tiles = water_pack.group_tiles(&water_family);
		let water_spin = water_pack.props[&water_family].transformable;
		if water_tiles.is_empty() {
			return Err(format!("{}: WATER family '{water_family}' has no tiles", water_pack.name));
		}
		let pack = &project.packs[pack_idx];
		let land_tiles = pack.group_tiles(&land_family);
		let land_spin = pack.props[&land_family].transformable;
		if land_tiles.is_empty() {
			return Err(format!("{}: LAND family '{land_family}' has no tiles", pack.name));
		}
		// Stamp pools, all from the land pack (cohesion: no snow rocks on
		// green grass). A pattern's kind = its family's props type - the
		// props are definitive (MAX's own pass data is inconsistent).
		let pattern_kind = |pt: &crate::TilePattern| -> Option<TileKind> {
			let idx = pt.cells.iter().flatten().next()?;
			pack.props.get(family_of(&pack.ids[*idx as usize])).and_then(|fp| fp.kind)
		};
		let pattern_stamps = |kind: TileKind| -> Vec<Stamp> {
			pack.patterns
				.iter()
				.filter(|pt| pattern_kind(pt) == Some(kind))
				.map(|pt| Stamp::Pattern {
					w: pt.width as i32,
					h: pt.height as i32,
					cells: pt
						.cells
						.iter()
						.enumerate()
						.filter_map(|(i, c)| {
							c.map(|tile| ((i % pt.width as usize) as i32, (i / pt.width as usize) as i32, tile))
						})
						.collect(),
				})
				.collect()
		};
		let single_stamps = |kind: TileKind, skip: &str| -> Vec<Stamp> {
			let mut fams: Vec<&String> = pack
				.props
				.iter()
				.filter(|(f, fp)| fp.kind == Some(kind) && fp.has_variants && f.as_str() != skip)
				.map(|(f, _)| f)
				.collect();
			fams.sort();
			fams.iter()
				.map(|f| Stamp::Single { tiles: pack.group_tiles(f), spin: pack.props[f.as_str()].transformable })
				.filter(|s| !matches!(s, Stamp::Single { tiles, .. } if tiles.is_empty()))
				.collect()
		};
		// Obstructions: whole formations + interchangeable singles (CHa…).
		let mut obstruction_pool = pattern_stamps(TileKind::Obstruction);
		obstruction_pool.extend(single_stamps(TileKind::Obstruction, ""));
		// Passable decorations: LAND formations + the LAND variant groups
		// that aren't the base fill (terrain variation: DLb, SLb/SLc, …).
		let mut decoration_pool = pattern_stamps(TileKind::Land);
		decoration_pool.extend(single_stamps(TileKind::Land, &land_family));

		let (w, h) = (project.width as usize, project.height as usize);
		let n = w * h;
		// Extremes skip the field entirely; rivers carve their own mask.
		let phase = if p.water >= 100 || p.water == 0 {
			Phase::Stamp { decorations: false }
		} else if p.pattern == GenPattern::RiverRaid {
			Phase::Rivers
		} else {
			Phase::Field { row: 0 }
		};
		let mut s = Self {
			p,
			w,
			h,
			pack_no: pack_idx as u8,
			land_tiles,
			land_spin,
			water_pack_no: water_pack_idx as u8,
			water_tiles,
			water_spin,
			obstruction_pool,
			decoration_pool,
			rng: Rng::new(p.seed ^ 0x574f_524c_4447_454e), // "WORLDGEN"
			phase,
			field: Vec::new(),
			sorted: Vec::new(),
			lo: 0,
			hi: n,
			best: usize::MAX,
			target: p.water as usize * n / 100,
			mask: vec![p.water >= 100; n],
			overlay: Vec::new(),
			placed: 0,
			attempts: 0,
			stamp_target: 0,
			obstructions_placed: 0,
			shore_changed: 0,
			fix: None,
			fix_found: 0,
			was_dirty: project.dirty(),
			mutated: false,
			stats: None,
		};
		if matches!(s.phase, Phase::Stamp { .. }) {
			s.begin_stamps();
		}
		Ok(s)
	}

	pub fn is_done(&self) -> bool {
		matches!(self.phase, Phase::Done)
	}

	/// The finished run's stats (`None` until done).
	pub fn stats(&self) -> Option<&GenStats> {
		self.stats.as_ref()
	}

	/// `(phase label, overall fraction 0..=1)` for the progress bar.
	pub fn progress(&self) -> (&'static str, f32) {
		match &self.phase {
			Phase::Field { row } => ("terrain", 0.20 * *row as f32 / self.h.max(1) as f32),
			Phase::Bisect { iter } => ("terrain", 0.20 + 0.20 * (*iter as f32 / 18.0).min(1.0)),
			Phase::Rivers => ("rivers", 0.25),
			Phase::Connect => ("bridging", 0.42),
			Phase::Stamp { decorations: false } => {
				("features", 0.45 + 0.10 * (self.placed as f32 / self.stamp_target.max(1) as f32).min(1.0))
			}
			Phase::Stamp { decorations: true } => {
				("decorations", 0.55 + 0.10 * (self.placed as f32 / self.stamp_target.max(1) as f32).min(1.0))
			}
			Phase::Apply => ("tiles", 0.66),
			Phase::Shore => ("shore", 0.72),
			Phase::Fix => {
				let frac = match (self.fix_found, &self.fix) {
					(found, Some(fix)) if found > 0 => 1.0 - fix.remaining() as f32 / found as f32,
					_ => 0.0,
				};
				("shore seams", 0.80 + 0.20 * frac.clamp(0.0, 1.0))
			}
			Phase::Done => ("done", 1.0),
		}
	}

	/// Advance one bounded slice of work; returns `true` when done. Call
	/// repeatedly (the shell does so within a per-frame time budget).
	pub fn step(&mut self, project: &mut Project) -> bool {
		match std::mem::replace(&mut self.phase, Phase::Done) {
			Phase::Field { row } => {
				if self.field.is_empty() {
					self.field = Vec::with_capacity(self.w * self.h);
				}
				let rows = (FIELD_CELLS_PER_STEP / self.w.max(1)).max(1);
				let end = (row + rows).min(self.h);
				for y in row..end {
					for x in 0..self.w {
						self.field.push(field_at(self.p.pattern, self.p.seed, self.w, self.h, x, y));
					}
				}
				if end < self.h {
					self.phase = Phase::Field { row: end };
				} else {
					self.sorted = self.field.clone();
					self.sorted.sort_by(|a, b| a.total_cmp(b));
					self.phase = Phase::Bisect { iter: 0 };
				}
			}
			Phase::Bisect { iter } => {
				// One probe: threshold at a quantile, smooth, depinch, count.
				// The smoothing shifts the count, so the quantile bisects
				// until the smoothed mask hits the water target to the cell.
				let n = self.w * self.h;
				let q = if iter == 0 { self.target } else { (self.lo + self.hi) / 2 };
				let cut = self.sorted[q.min(n - 1)];
				let mut mask: Vec<bool> = self.field.iter().map(|&v| v < cut).collect();
				smooth_mask(&mut mask, self.w, self.h);
				depinch(&mut mask, self.w, self.h);
				let wet = mask.iter().filter(|&&m| m).count();
				if wet.abs_diff(self.target) < self.best || iter == 0 {
					self.best = wet.abs_diff(self.target);
					self.mask = mask;
				}
				if iter > 0 {
					if wet < self.target {
						self.lo = q + 1;
					} else {
						self.hi = q;
					}
				}
				if self.best != 0 && self.lo < self.hi {
					self.phase = Phase::Bisect { iter: iter + 1 };
				} else if self.p.pattern == GenPattern::LandMass {
					self.phase = Phase::Connect;
				} else {
					self.begin_stamps();
				}
			}
			Phase::Rivers => {
				self.mask = rivers_mask(&self.p, self.w, self.h, &mut self.rng);
				self.begin_stamps();
			}
			Phase::Connect => {
				connect_land(&mut self.mask, self.w, self.h, &mut self.rng);
				self.begin_stamps();
			}
			Phase::Stamp { decorations } => {
				let pool = if decorations { &self.decoration_pool } else { &self.obstruction_pool };
				let finished = stamp_chunk(
					&mut self.overlay,
					pool,
					self.stamp_target,
					&mut self.placed,
					&mut self.attempts,
					STAMP_ATTEMPTS_PER_STEP,
					&self.mask,
					(self.w, self.h),
					self.pack_no,
					&mut self.rng,
				);
				if !finished {
					self.phase = Phase::Stamp { decorations };
				} else if !decorations {
					self.obstructions_placed = self.placed;
					let land = self.w * self.h - self.mask.iter().filter(|&&m| m).count();
					self.placed = 0;
					self.attempts = 0;
					self.stamp_target = land * self.p.decorations as usize / 100;
					self.phase = Phase::Stamp { decorations: true };
				} else {
					self.phase = Phase::Apply;
				}
			}
			Phase::Apply => {
				// Lay both layers in one stroke - fresh water across the whole
				// bottom layer, land fill + stamps on the ground layer, then
				// the shore passes - all of it one Ctrl+Z. Rewriting the water
				// layer too is the clean slate: whatever the previous map kept
				// there (an imported WRL's tiles, stale variants) would
				// otherwise show through every generated water cell.
				let (w, n) = (self.w, self.w * self.h);
				let mut edits: Vec<(u16, u16, usize, Option<TileRef>)> = Vec::with_capacity(2 * n);
				for i in 0..n {
					let (x, y) = ((i % w) as u16, (i / w) as u16);
					let water = self.water_tiles[self.rng.below(self.water_tiles.len() as u32) as usize];
					edits.push((
						x,
						y,
						LAYER_WATER,
						Some(TileRef {
							pack: self.water_pack_no,
							tile: water,
							transform: random_transform(self.water_spin, &mut self.rng),
						}),
					));
					let entry = if self.mask[i] {
						None
					} else if let Some(stamped) = self.overlay[i] {
						Some(stamped)
					} else {
						let tile = self.land_tiles[self.rng.below(self.land_tiles.len() as u32) as usize];
						Some(TileRef {
							pack: self.pack_no,
							tile,
							transform: random_transform(self.land_spin, &mut self.rng),
						})
					};
					edits.push((x, y, LAYER_GROUND, entry));
				}
				project.begin_stroke();
				self.mutated = true;
				project.place_many(&edits);
				project.clear_pass_overrides();
				self.phase = Phase::Shore;
			}
			Phase::Shore => {
				let (shore, unresolved) =
					if self.p.alt_shore { project.auto_shore_alt(None) } else { project.auto_shore(None) };
				self.shore_changed = shore;
				if unresolved > 0 {
					// The Destructive seam-fix solver: the terrain is ours to
					// reshape, and it converges to zero broken seams where
					// the milder passes plateau (~1% of river-heavy coast).
					let fix = project.fix_session(None, crate::FixStrength::Destructive);
					self.fix_found = fix.found();
					self.fix = Some(fix);
					self.phase = Phase::Fix;
				} else {
					self.finish(project, 0, 0);
				}
			}
			Phase::Fix => {
				let mut fix = self.fix.take().expect("fix session in Fix phase");
				fix.step(FIX_WORK_PER_STEP);
				if fix.is_done() {
					let fixed = fix.apply(project);
					self.finish(project, fixed, fix.remaining());
				} else {
					self.fix = Some(fix);
					self.phase = Phase::Fix;
				}
			}
			Phase::Done => {}
		}
		self.is_done()
	}

	/// Cancel the run. Everything the session wrote is rolled back - the
	/// document (content, dirty flag) is as if Generate was never pressed.
	pub fn abort(mut self, project: &mut Project) {
		self.fix = None;
		if self.mutated {
			project.rollback_stroke();
			if !self.was_dirty {
				project.mark_saved();
			}
		}
	}

	/// Stamp targets + overlay once the water mask is final.
	fn begin_stamps(&mut self) {
		let n = self.w * self.h;
		let land = n - self.mask.iter().filter(|&&m| m).count();
		self.overlay = vec![None; n];
		self.placed = 0;
		self.attempts = 0;
		self.stamp_target = land * self.p.obstructions as usize / 100;
		self.phase = Phase::Stamp { decorations: false };
	}

	fn finish(&mut self, project: &mut Project, fixed: usize, unresolved: usize) {
		project.end_stroke();
		let n = self.w * self.h;
		let water = self.mask.iter().filter(|&&m| m).count();
		let decorations = self.placed;
		self.stats = Some(GenStats {
			water,
			land: n - water,
			obstructions: self.obstructions_placed,
			decorations,
			shore: self.shore_changed + fixed,
			unresolved,
		});
		self.phase = Phase::Done;
	}
}

impl Project {
	/// Replace the terrain with a generated one, synchronously (scripts,
	/// tests, the `generate` command) - the whole water layer refills and
	/// the whole ground layer is rewritten, so nothing of the previous map
	/// survives. Interactive runs drive a [`GenSession`] per frame instead -
	/// same code path, stepped.
	pub fn generate_terrain(&mut self, p: &GenParams) -> Result<GenStats, String> {
		let mut session = GenSession::new(self, *p)?;
		while !session.step(self) {}
		Ok(session.stats.take().expect("stats set when done"))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::Path;

	fn assets_root() -> std::path::PathBuf {
		Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets/tilepacks")
	}

	fn make(pattern: GenPattern, water: u8, obstructions: u8, seed: u64) -> (Project, GenStats) {
		let mut p = Project::new(64, 64, &["GREEN".into()], &assets_root(), 7).unwrap();
		let params = GenParams { pattern, water, obstructions, decorations: 0, seed, alt_shore: false };
		let stats = p.generate_terrain(&params).unwrap();
		(p, stats)
	}

	#[test]
	fn generation_replaces_both_layers_fully() {
		let mut p = Project::new(48, 48, &["GREEN".into()], &assets_root(), 7).unwrap();
		// Sabotage the bottom layer the way a previous map would: a GREEN
		// land tile pretending to be water, and a hole. Both used to
		// survive the run and show through every generated water cell.
		p.place(3, 3, crate::LAYER_WATER, Some(TileRef { pack: 1, tile: 0, transform: Transform::default() }));
		p.place(5, 5, crate::LAYER_WATER, None);
		let params = GenParams {
			pattern: GenPattern::Islands,
			water: 45,
			obstructions: 0,
			decorations: 0,
			seed: 9,
			alt_shore: false,
		};
		p.generate_terrain(&params).unwrap();
		// Clean slate: every bottom-layer cell is a fresh water-pack tile.
		for y in 0..p.height {
			for x in 0..p.width {
				let t = p.cell(x, y).unwrap()[crate::LAYER_WATER].unwrap_or_else(|| panic!("hole at ({x},{y})"));
				assert_eq!(t.pack, 0, "non-water-pack tile at ({x},{y})");
			}
		}
	}

	#[test]
	fn deterministic_and_seed_sensitive() {
		let (a, _) = make(GenPattern::Islands, 45, 10, 42);
		let (b, _) = make(GenPattern::Islands, 45, 10, 42);
		let (c, _) = make(GenPattern::Islands, 45, 10, 43);
		assert_eq!(a.hash(), b.hash(), "same seed + params = same map");
		assert_ne!(a.hash(), c.hash(), "a different seed moves the terrain");
	}

	#[test]
	fn field_patterns_land_on_the_water_quota() {
		// Bisection vs. the smoothed mask: within 1% of the request.
		for pattern in [GenPattern::Islands, GenPattern::Continent] {
			let (_, stats) = make(pattern, 40, 0, 9);
			let target = 64 * 64 * 40 / 100;
			assert!(stats.water.abs_diff(target) <= 64 * 64 / 100, "{pattern:?}: {} vs {target}", stats.water);
		}
	}

	#[test]
	fn river_raid_carves_at_least_the_quota() {
		let (_, stats) = make(GenPattern::RiverRaid, 20, 0, 5);
		let quota = 64 * 64 * 20 / 100;
		assert!(stats.water >= quota, "{} < {quota}", stats.water);
		assert!(stats.water < quota + 200, "{} overshoots wildly", stats.water);
	}

	#[test]
	fn land_mass_is_one_connected_component() {
		let (p, _) = make(GenPattern::LandMass, 40, 0, 11);
		// Flood-fill ground tiles (shore included - it's walkable coastline).
		let (w, h) = (p.width as usize, p.height as usize);
		let land: Vec<bool> = (0..w * h)
			.map(|i| p.cell((i % w) as u16, (i / w) as u16).unwrap()[crate::LAYER_GROUND].is_some())
			.collect();
		let total = land.iter().filter(|&&l| l).count();
		assert!(total > 0);
		let start = land.iter().position(|&l| l).unwrap();
		let mut seen = vec![false; w * h];
		let mut reached = 0;
		crate::grid::flood4(w, h, start, &mut seen, |j| land[j], |_| reached += 1);
		assert_eq!(reached, total, "land mass split into islands");
	}

	#[test]
	fn obstructions_cluster_on_land_away_from_water() {
		let (p, stats) = make(GenPattern::Continent, 30, 12, 3);
		assert!(stats.obstructions > 0, "no obstructions placed");
		// Every obstruction-family tile stands on a cell with no water in
		// its 8-neighborhood (the shore ring claimed the coast itself).
		let pack = &p.packs[1]; // 0 = WATER, 1 = GREEN
		let mut found = 0;
		for y in 0..p.height {
			for x in 0..p.width {
				let Some(t) = p.cell(x, y).unwrap()[crate::LAYER_GROUND] else { continue };
				if t.pack != 1 {
					continue;
				}
				let fam = family_of(&pack.ids[t.tile as usize]);
				if pack.props.get(fam).and_then(|fp| fp.kind) != Some(TileKind::Obstruction) {
					continue;
				}
				found += 1;
				for dy in -1i32..=1 {
					for dx in -1i32..=1 {
						let (nx, ny) = (x as i32 + dx, y as i32 + dy);
						if nx < 0 || ny < 0 || nx >= p.width as i32 || ny >= p.height as i32 {
							continue;
						}
						let wet = p.cell(nx as u16, ny as u16).unwrap()[crate::LAYER_GROUND].is_none();
						assert!(!wet, "obstruction at ({x},{y}) touches water at ({nx},{ny})");
					}
				}
			}
		}
		assert_eq!(found, stats.obstructions);
	}

	#[test]
	fn one_undo_unit_and_clean_shore() {
		let mut p = Project::new(48, 48, &["GREEN".into()], &assets_root(), 7).unwrap();
		let before = p.hash();
		let params = GenParams {
			pattern: GenPattern::Islands,
			water: 45,
			obstructions: 8,
			decorations: 5,
			seed: 42,
			alt_shore: false,
		};
		let stats = p.generate_terrain(&params).unwrap();
		// Pinned seed: the GREEN tileset closes every generated coastline.
		assert_eq!(stats.unresolved, 0, "seed 42 used to shore cleanly");
		assert_ne!(p.hash(), before);
		assert!(p.undo(), "generation undoes");
		assert_eq!(p.hash(), before, "one Ctrl+Z restores the pre-gen map");
		assert!(p.redo());
	}

	#[test]
	fn full_water_and_full_land_extremes() {
		let (p, stats) = make(GenPattern::Islands, 100, 0, 1);
		assert_eq!(stats.water, 64 * 64);
		assert!((0..64u16).all(|x| p.cell(x, 0).unwrap()[crate::LAYER_GROUND].is_none()));
		let (_, stats) = make(GenPattern::LandMass, 0, 0, 1);
		assert_eq!(stats.water, 0);
	}

	/// Collect `(x, y, tile)` of every ground tile whose family has `kind`.
	fn cells_of_kind(p: &Project, kind: TileKind) -> Vec<(u16, u16, u16)> {
		let pack = &p.packs[1]; // 0 = WATER, 1 = GREEN
		let mut out = Vec::new();
		for y in 0..p.height {
			for x in 0..p.width {
				let Some(t) = p.cell(x, y).unwrap()[crate::LAYER_GROUND] else { continue };
				if t.pack != 1 {
					continue;
				}
				let fam = family_of(&pack.ids[t.tile as usize]);
				if pack.props.get(fam).and_then(|fp| fp.kind) == Some(kind) {
					out.push((x, y, t.tile));
				}
			}
		}
		out
	}

	#[test]
	fn obstructions_are_whole_formations() {
		// GREEN has no variant obstruction group, so every obstruction cell
		// must belong to a complete stamped tiles.patterns.json formation.
		let (p, stats) = make(GenPattern::Continent, 30, 12, 3);
		assert!(stats.obstructions > 0, "no obstructions placed");
		let placed = cells_of_kind(&p, TileKind::Obstruction);
		assert_eq!(placed.len(), stats.obstructions);
		let by_cell: std::collections::HashMap<(u16, u16), u16> = placed.iter().map(|&(x, y, t)| ((x, y), t)).collect();
		let pack = &p.packs[1];
		let mut covered: std::collections::HashSet<(u16, u16)> = std::collections::HashSet::new();
		for pt in &pack.patterns {
			// Anchor on each placed cell that matches the pattern's first
			// populated cell, then require the full footprint to match.
			let cells: Vec<(u16, u16, u16)> = pt
				.cells
				.iter()
				.enumerate()
				.filter_map(|(i, c)| c.map(|t| ((i % pt.width as usize) as u16, (i / pt.width as usize) as u16, t)))
				.collect();
			let (ax, ay, at) = cells[0];
			for (&(x, y), &t) in &by_cell {
				if t != at || x < ax || y < ay {
					continue;
				}
				let (ox, oy) = (x - ax, y - ay);
				if cells.iter().all(|&(dx, dy, pt)| by_cell.get(&(ox + dx, oy + dy)) == Some(&pt)) {
					covered.extend(cells.iter().map(|&(dx, dy, _)| (ox + dx, oy + dy)));
				}
			}
		}
		for &(x, y, _) in &placed {
			assert!(covered.contains(&(x, y)), "obstruction at ({x},{y}) is not part of a whole formation");
		}
	}

	#[test]
	fn decorations_stamp_passable_land_families() {
		let mut p = Project::new(64, 64, &["GREEN".into()], &assets_root(), 7).unwrap();
		let params = GenParams {
			pattern: GenPattern::Continent,
			water: 30,
			obstructions: 0,
			decorations: 8,
			seed: 4,
			alt_shore: false,
		};
		let stats = p.generate_terrain(&params).unwrap();
		assert!(stats.decorations > 0, "no decorations placed");
		assert_eq!(stats.obstructions, 0);
		// Decorations are LAND tiles outside the base fill group (GLb/GLc).
		let pack = &p.packs[1];
		let decor = cells_of_kind(&p, TileKind::Land)
			.into_iter()
			.filter(|&(_, _, t)| family_of(&pack.ids[t as usize]) != "GLa")
			.count();
		assert_eq!(decor, stats.decorations);
	}

	#[test]
	fn alt_shore_is_deterministic_and_differs_from_sweep() {
		let run = |alt_shore: bool| {
			let mut p = Project::new(64, 64, &["GREEN".into()], &assets_root(), 7).unwrap();
			let params = GenParams {
				pattern: GenPattern::Islands,
				water: 45,
				obstructions: 0,
				decorations: 0,
				seed: 9,
				alt_shore,
			};
			p.generate_terrain(&params).unwrap();
			p.hash()
		};
		assert_eq!(run(true), run(true), "alt shore is deterministic");
		assert_ne!(run(true), run(false), "the loop-walk pass tiles the coast differently");
	}

	#[test]
	fn abort_rolls_the_document_back() {
		let mut p = Project::new(48, 48, &["GREEN".into()], &assets_root(), 7).unwrap();
		let before = p.hash();
		let params = GenParams {
			pattern: GenPattern::Islands,
			water: 45,
			obstructions: 8,
			decorations: 5,
			seed: 42,
			alt_shore: false,
		};
		// Abort before anything mutates: a pure no-op.
		let mut s = GenSession::new(&p, params).unwrap();
		assert!(!s.step(&mut p));
		s.abort(&mut p);
		assert_eq!(p.hash(), before);
		assert!(!p.dirty());
		// Step past Apply (the doc is mutated mid-run), then abort.
		let mut s = GenSession::new(&p, params).unwrap();
		while s.progress().0 != "shore" {
			assert!(!s.step(&mut p), "session finished before the abort point");
		}
		assert_ne!(p.hash(), before, "Apply mutated the document");
		assert!(p.dirty());
		s.abort(&mut p);
		assert_eq!(p.hash(), before, "abort rolled everything back");
		assert!(!p.dirty(), "abort restored the clean flag");
		assert!(!p.undo(), "nothing landed on the undo stack");
	}

	#[test]
	fn session_progress_is_monotonic_and_matches_sync_run() {
		let params = GenParams {
			pattern: GenPattern::LandMass,
			water: 40,
			obstructions: 10,
			decorations: 5,
			seed: 6,
			alt_shore: false,
		};
		let mut stepped = Project::new(64, 64, &["GREEN".into()], &assets_root(), 7).unwrap();
		let mut s = GenSession::new(&stepped, params).unwrap();
		let mut last = 0.0f32;
		let mut steps = 0;
		while !s.step(&mut stepped) {
			let (_, frac) = s.progress();
			assert!(frac >= last, "progress went backwards: {frac} < {last}");
			last = frac;
			steps += 1;
			assert!(steps < 10_000, "session never finished");
		}
		assert!(steps > 1, "a 64x64 run should take several steps");
		let mut sync = Project::new(64, 64, &["GREEN".into()], &assets_root(), 7).unwrap();
		sync.generate_terrain(&params).unwrap();
		assert_eq!(stepped.hash(), sync.hash(), "stepped and sync runs are the same map");
	}

	#[test]
	fn pattern_names_round_trip() {
		for pattern in GenPattern::ALL {
			assert_eq!(GenPattern::parse(pattern.name()).unwrap(), pattern);
		}
		assert!(GenPattern::parse("volcano").is_err());
	}

	#[test]
	fn noise_primitives_are_pinned() {
		// Golden vectors. The worldgen noise is float math, so a toolchain or
		// optimization change that perturbs it would silently re-roll every map
		// (the end-to-end `hash` tests would change too, but wouldn't say why).
		// Pin the primitives to their reference values; regenerate deliberately
		// only when the generator algorithm itself changes.
		assert_eq!(splitmix(0), 0, "splitmix finalizer of 0 is 0");
		assert_eq!(splitmix(0x1234_5678), 11071400828549884513);
		assert_eq!(value_noise(42, 1.5, 2.5), 0.7627137);
		assert_eq!(fbm(42, 1.5, 2.5), 0.5670068);
		assert_eq!(field_at(GenPattern::Continent, 42, 64, 64, 10, 20), 0.46555495);
		assert_eq!(rotated((1.0, 0.0), 5), (0.80901897, 0.58778495));
		// Structural invariants alongside the goldens.
		assert_eq!(rotated((1.0, 0.0), 0), (1.0, 0.0), "no turn = identity");
		assert!((0.0..1.0).contains(&value_noise(42, 1.5, 2.5)), "value noise in [0, 1)");
	}

	#[test]
	fn gen_session_rejects_out_of_range_percentages() {
		let p = Project::new(8, 8, &["GREEN".into()], &assets_root(), 1).unwrap();
		let params = |water, obs, dec| GenParams {
			pattern: GenPattern::Islands,
			water,
			obstructions: obs,
			decorations: dec,
			seed: 1,
			alt_shore: false,
		};
		let err = |w, o, d| GenSession::new(&p, params(w, o, d)).err().expect("expected a validation error");
		assert!(err(101, 0, 0).contains("0..=100"), "water > 100");
		assert!(err(0, 101, 0).contains("0..=100"), "obstructions > 100");
		assert!(err(0, 0, 101).contains("0..=100"), "decorations > 100");
		// The boundary values are accepted.
		assert!(GenSession::new(&p, params(100, 100, 100)).is_ok());
		assert!(GenSession::new(&p, params(0, 0, 0)).is_ok());
	}
}
