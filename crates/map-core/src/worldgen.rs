//! Random terrain generator: seeded, parameterized, **deterministic** - the
//! same seed + params always produce the same map, so a seed is a shareable
//! recipe. See `RNG-GEN.md` for the design spec.
//!
//! A map is built from one [`Generator`] (the land/water layout), a [`Symmetry`]
//! (mirroring for fair play), the common options (drop zones + obstruction /
//! decoration patches), a [`ShoreMethod`], and a seed. Land fills from the
//! tileset's LAND variant group (`tiles.props.json`); obstruction and decoration
//! patches stamp the editor's stock/user **templates**, clustered into patches;
//! coastlines are grown by the chosen auto-shore pass. One run = one undo unit.
//!
//! Sizes given as a percent are a fraction of the **map area** (so they scale
//! with map size); river widths and island distances are in tiles.

use crate::pack::{TileKind, Transformable, family_of};
use crate::project::{LAYER_GROUND, LAYER_WATER, Project, Rng, TileRef, Transform, splitmix};
use crate::template::{StampOp, Template};

/// The overall land/water layout. Each generator is dedicated to one purpose
/// and reads only the [`GenParams`] fields that purpose needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Generator {
	/// Separate land masses in open water (never touching each other or edges).
	Islands,
	/// One or more large land masses ringed by ocean (not touching edges/each other).
	Continents,
	/// The inverse of Continents - one or more seas enclosed by land.
	CentralSeas,
	/// A solid landmass, edge to edge.
	Land,
	/// Solid land cut by very curly, meandering rivers.
	Rivers,
	/// Solid land cut by random, nearly straight rivers.
	RiverRaid,
	/// A labyrinth of land corridors and water walls (a navigable terrain maze;
	/// obstructions are secondary to the land/water structure).
	Maze,
}

impl Generator {
	pub const ALL: [Generator; 7] = [
		Generator::Islands,
		Generator::Continents,
		Generator::CentralSeas,
		Generator::Land,
		Generator::Rivers,
		Generator::RiverRaid,
		Generator::Maze,
	];

	pub fn parse(s: &str) -> Result<Self, String> {
		match s {
			"islands" => Ok(Generator::Islands),
			"continents" => Ok(Generator::Continents),
			"central-seas" => Ok(Generator::CentralSeas),
			"land" => Ok(Generator::Land),
			"rivers" => Ok(Generator::Rivers),
			"river-raid" => Ok(Generator::RiverRaid),
			"maze" => Ok(Generator::Maze),
			other => Err(format!(
				"unknown generator '{other}' (islands|continents|central-seas|land|rivers|river-raid|maze)"
			)),
		}
	}

	/// The command-line word (`Generator::parse`'s inverse).
	pub fn name(self) -> &'static str {
		match self {
			Generator::Islands => "islands",
			Generator::Continents => "continents",
			Generator::CentralSeas => "central-seas",
			Generator::Land => "land",
			Generator::Rivers => "rivers",
			Generator::RiverRaid => "river-raid",
			Generator::Maze => "maze",
		}
	}

	/// Human label for the UI select.
	pub fn label(self) -> &'static str {
		match self {
			Generator::Islands => "Islands",
			Generator::Continents => "Continents",
			Generator::CentralSeas => "Central Seas",
			Generator::Land => "Land",
			Generator::Rivers => "Rivers",
			Generator::RiverRaid => "River Raid",
			Generator::Maze => "Maze",
		}
	}
}

/// How the generated terrain is mirrored for fair-play layouts. Applied to the
/// land/water mask and to the placed feature templates. Exact on any map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symmetry {
	/// No mirroring.
	None,
	/// Mirror across the vertical centre axis (left ↔ right halves match).
	LeftRight,
	/// Mirror across the horizontal centre axis (top ↔ bottom halves match).
	TopBottom,
	/// Mirror across both axes - all four quadrants match.
	FourCorners,
	/// 180° rotation through the centre (point symmetry).
	Rotate180,
}

impl Symmetry {
	pub const ALL: [Symmetry; 5] = [
		Symmetry::None,
		Symmetry::LeftRight,
		Symmetry::TopBottom,
		Symmetry::FourCorners,
		Symmetry::Rotate180,
	];

	pub fn parse(s: &str) -> Result<Self, String> {
		match s {
			"none" => Ok(Symmetry::None),
			"lr" => Ok(Symmetry::LeftRight),
			"tb" => Ok(Symmetry::TopBottom),
			"quad" => Ok(Symmetry::FourCorners),
			"rotate" => Ok(Symmetry::Rotate180),
			other => Err(format!("unknown symmetry '{other}' (none|lr|tb|quad|rotate)")),
		}
	}

	/// The command-line word (`Symmetry::parse`'s inverse).
	pub fn name(self) -> &'static str {
		match self {
			Symmetry::None => "none",
			Symmetry::LeftRight => "lr",
			Symmetry::TopBottom => "tb",
			Symmetry::FourCorners => "quad",
			Symmetry::Rotate180 => "rotate",
		}
	}

	/// Human label for the UI select.
	pub fn label(self) -> &'static str {
		match self {
			Symmetry::None => "None",
			Symmetry::LeftRight => "Left-Right",
			Symmetry::TopBottom => "Top-Bottom",
			Symmetry::FourCorners => "Four Corners",
			Symmetry::Rotate180 => "Rotate 180 deg",
		}
	}
}

/// How coastlines between land and water are tiled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShoreMethod {
	/// Uniform coastline (the sweep optimizer).
	Sweep,
	/// More varied coastline (the loop-walk pass).
	LoopWalk,
	/// Leave coastlines untiled.
	None,
}

impl ShoreMethod {
	pub const ALL: [ShoreMethod; 3] = [ShoreMethod::Sweep, ShoreMethod::LoopWalk, ShoreMethod::None];

	pub fn parse(s: &str) -> Result<Self, String> {
		match s {
			"sweep" => Ok(ShoreMethod::Sweep),
			"loop" => Ok(ShoreMethod::LoopWalk),
			"none" => Ok(ShoreMethod::None),
			other => Err(format!("unknown shore '{other}' (sweep|loop|none)")),
		}
	}

	pub fn name(self) -> &'static str {
		match self {
			ShoreMethod::Sweep => "sweep",
			ShoreMethod::LoopWalk => "loop",
			ShoreMethod::None => "none",
		}
	}

	pub fn label(self) -> &'static str {
		match self {
			ShoreMethod::Sweep => "Sweep",
			ShoreMethod::LoopWalk => "Loop-walk",
			ShoreMethod::None => "None",
		}
	}
}

/// How obstruction patches are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessibilityMode {
	/// Patches scattered by density (`accessibility` %), as-is.
	Random,
	/// Curved roads across the map's extremes; the `accessibility` value sets
	/// both their count and width, and the centre stays dense (only a thin
	/// spine is carved through it).
	Paths,
	/// A maze of corridors woven across the whole map - twisty roads with dead
	/// ends rather than sensible point-to-point paths.
	Labyrinth,
}

impl AccessibilityMode {
	pub const ALL: [AccessibilityMode; 3] = [AccessibilityMode::Random, AccessibilityMode::Paths, AccessibilityMode::Labyrinth];

	pub fn parse(s: &str) -> Result<Self, String> {
		match s {
			"random" => Ok(AccessibilityMode::Random),
			"paths" => Ok(AccessibilityMode::Paths),
			"labyrinth" => Ok(AccessibilityMode::Labyrinth),
			other => Err(format!("unknown accessibility mode '{other}' (random|paths|labyrinth)")),
		}
	}

	pub fn name(self) -> &'static str {
		match self {
			AccessibilityMode::Random => "random",
			AccessibilityMode::Paths => "paths",
			AccessibilityMode::Labyrinth => "labyrinth",
		}
	}

	pub fn label(self) -> &'static str {
		self.name()
	}
}

/// A count of things, each sized within a `[min, max]` range, in **cells**
/// (blob / patch radius; river width is tiles across). `count` is unused for a
/// few fields (e.g. accessibility).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
	pub count: u8,
	pub min: u8,
	pub max: u8,
}

/// A bare `[min, max]` range with no count - island spacing, the **cell** gap
/// between island edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
	pub min: u8,
	pub max: u8,
}

/// All knobs for one generation run. Generator-irrelevant fields are ignored.
/// Percent fields are a fraction of the **map area** unless noted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenParams {
	pub generator: Generator,
	pub seed: u64,
	pub symmetry: Symmetry,
	pub shore: ShoreMethod,
	/// Islands: large islands - count + radius (tiles); spacing in `main_dist`.
	pub main_islands: Range,
	/// Islands: large-island spacing, the cell gap between island edges.
	pub main_dist: Span,
	/// Islands: small islands - count + radius (tiles); spacing in `small_dist`.
	pub small_islands: Range,
	pub small_dist: Span,
	/// Continents: count + radius (tiles).
	pub continents: Range,
	/// Central Seas: count + radius (tiles).
	pub seas: Range,
	/// Rivers (Islands/Continents/Central Seas/Land/Rivers/River Raid): count +
	/// width range in tiles across.
	pub rivers: Range,
	/// Lakes (Islands/Continents/Land): count + radius (tiles).
	pub lakes: Range,
	/// Maze: `count` = extra loop openings (braid), `min`/`max` = corridor width
	/// range in cells (wall thickness is fixed). Only the Maze generator reads it.
	pub maze: Range,
	/// Common - drop zones: count + radius (tiles; flat, obstruction-free starts).
	pub drop_zones: Range,
	/// Common - obstruction patches: count + radius (tiles).
	pub obstructions: Range,
	/// Common - obstruction density %: lower = denser / fewer passages. In
	/// `Paths`/`Labyrinth` mode it also sets the road count + width.
	pub accessibility: u8,
	/// Common - obstruction layout: scattered (`Random`), roads across the
	/// extremes (`Paths`), or a maze (`Labyrinth`).
	pub accessibility_mode: AccessibilityMode,
	/// Common - decoration patches: count + radius (tiles).
	pub decorations: Range,
}

impl GenParams {
	/// Middle-of-the-road defaults for `generator` (seed 0, no symmetry, sweep
	/// shore). The single source of defaults shared by the modal and the
	/// `generate` command. `min`/`max` are radii in tiles, except rivers (width
	/// in tiles across) and the distances (cell gap between island edges).
	pub fn defaults(generator: Generator) -> Self {
		Self {
			generator,
			seed: 0,
			symmetry: Symmetry::None,
			shore: ShoreMethod::Sweep,
			main_islands: Range { count: 3, min: 8, max: 16 },
			main_dist: Span { min: 6, max: 20 },
			small_islands: Range { count: 6, min: 3, max: 6 },
			small_dist: Span { min: 4, max: 12 },
			continents: Range { count: 2, min: 18, max: 30 },
			seas: Range { count: 2, min: 12, max: 22 },
			rivers: Range { count: 4, min: 4, max: 6 },
			lakes: Range { count: 3, min: 2, max: 5 },
			maze: Range { count: 2, min: 3, max: 4 },
			drop_zones: Range { count: 2, min: 5, max: 9 },
			obstructions: Range { count: 4, min: 4, max: 9 },
			accessibility: 50,
			accessibility_mode: AccessibilityMode::Random,
			decorations: Range { count: 4, min: 4, max: 9 },
		}
	}
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

/// Per-`step` work budget for the shore-fix solver (an interactive generate runs
/// one bounded slice per frame; the run-to-completion path loops `step` until
/// done). The shape/stamp phases are cheap enough to run whole per step.
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

/// How wiggly carved rivers are. The tuple is `(drift_clamp, wave_amp,
/// wave_lo, wave_hi, jitter_clamp, branches, step_mult)`:
/// - `drift_clamp` - amplitude of the slow, mean-reverting course (large bends),
/// - `wave_amp` / `wave_lo..=wave_hi` - amplitude (in ~7.2° steps) and angular
///   speed of a smooth **sine meander** (the regular waviness),
/// - `jitter_clamp` - fast per-step wobble,
/// - `branches` - tributary deltas a river may fork,
/// - `step_mult` - walk length as a multiple of `w + h` (wavier rivers need
///   more steps to traverse).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Curliness {
	/// Nearly straight - tight wobble, no drift / waves / tributaries (River Raid).
	Straight,
	/// Meanders, oxbows, and a tributary (the inland rivers of Islands /
	/// Continents / Central Seas / Land).
	Curly,
	/// Heavy sine waves, big oxbows, and deltas (the Rivers generator).
	VeryCurly,
}

impl Curliness {
	/// `(drift_clamp, wave_amp, wave_lo, wave_hi, jitter_clamp, branches, step_mult)`.
	fn shape(self) -> (i32, i32, i32, i32, i32, u32, usize) {
		match self {
			Curliness::Straight => (0, 0, 0, 0, 4, 0, 2),
			Curliness::Curly => (5, 5, 2, 4, 3, 1, 3),
			Curliness::VeryCurly => (8, 9, 3, 6, 3, 2, 3),
		}
	}
}

/// One river/tributary the walker still has to draw.
struct RiverSeed {
	px: f32,
	py: f32,
	/// Overall heading - any angle across the map (drifts/waves around this).
	base: (f32, f32),
	/// Rotating unit vector whose `.1` is the sine-meander phase.
	wave: (f32, f32),
	/// Angular speed of the sine meander (~7.2° steps per iteration).
	wave_speed: i32,
	/// Constant stamp radius for this segment (≈ width/2 tiles across).
	radius: i32,
	steps: usize,
	/// Tributaries this segment may still spawn (deltas).
	branches: u32,
}

/// A random entry point on the perimeter and a heading aimed across to a random
/// point on a **different** edge - so rivers cross at any angle, not just the
/// horizontal / vertical that fixed top/left headings used to produce.
fn river_entry(w: usize, h: usize, rng: &mut Rng) -> (f32, f32, (f32, f32)) {
	let perim = |rng: &mut Rng| -> (f32, f32, u8) {
		match rng.below(4) {
			0 => (rng.below(w as u32) as f32, 0.0, 0),                  // top
			1 => (rng.below(w as u32) as f32, (h - 1) as f32, 1),       // bottom
			2 => (0.0, rng.below(h as u32) as f32, 2),                  // left
			_ => ((w - 1) as f32, rng.below(h as u32) as f32, 3),       // right
		}
	};
	let (ex, ey, edge) = perim(rng);
	let (tx, ty) = loop {
		let (a, b, e) = perim(rng);
		if e != edge {
			break (a, b);
		}
	};
	let (dx, dy) = (tx - ex, ty - ey);
	let len = (dx * dx + dy * dy).sqrt().max(1.0);
	(ex, ey, (dx / len, dy / len))
}

/// Carve `spec.count` rivers into `mask` (true = water). Each river enters at a
/// random perimeter point and heads across at **any angle** (`river_entry`); its
/// **width** (tiles across) is picked per river in `spec.min..=spec.max`. The
/// heading is the entry bearing rotated by a slow mean-reverting **drift** + a
/// smooth **sine meander** + a fast **jitter**; `Curly` / `VeryCurly` add ever
/// stronger waves, oxbows and tributary deltas (`Straight` barely bends). Width
/// is constant per segment (changing it mid-run leaves stair-step notches the
/// shore tileset struggles to continue).
fn carve_rivers(mask: &mut [bool], w: usize, h: usize, spec: Range, curl: Curliness, rng: &mut Rng) {
	if spec.count == 0 || spec.max == 0 {
		return;
	}
	let stamp = |mask: &mut [bool], x: i32, y: i32, r: i32| {
		for dy in -r..=r {
			for dx in -r..=r {
				if dx * dx + dy * dy > r * r {
					continue;
				}
				let (px, py) = (x + dx, y + dy);
				if px < 0 || py < 0 || px >= w as i32 || py >= h as i32 {
					continue;
				}
				mask[py as usize * w + px as usize] = true;
			}
		}
	};
	let (lo, hi) = (spec.min.max(1) as u32, spec.max.max(spec.min.max(1)) as u32);
	let (drift_clamp, wave_amp, wave_lo, wave_hi, jitter_clamp, max_branches, step_mult) = curl.shape();
	let steps_full = step_mult * (w + h);
	// A fresh wave phase + speed per river (deterministic from the rng).
	let new_wave = |rng: &mut Rng| -> (i32, (f32, f32)) {
		let speed = if wave_amp > 0 { wave_lo + rng.below((wave_hi - wave_lo + 1) as u32) as i32 } else { 0 };
		let phase = rng.below(50) as i32; // any starting phase (50 × 7.2° ≈ 360°)
		(speed, rotated((1.0, 0.0), phase))
	};
	let mut queue: Vec<RiverSeed> = Vec::new();
	let mut started = 0usize;
	let mut segments = 0usize;
	loop {
		if queue.is_empty() {
			if started >= spec.count as usize {
				break;
			}
			let width = lo + rng.below(hi - lo + 1); // tiles across
			let radius = (width.saturating_sub(1) / 2) as i32;
			let (px, py, base) = river_entry(w, h, rng);
			let (wave_speed, wave) = new_wave(rng);
			queue.push(RiverSeed { px, py, base, wave, wave_speed, radius, steps: steps_full, branches: max_branches });
			started += 1;
		}
		let RiverSeed { mut px, mut py, base, mut wave, wave_speed, radius, steps, mut branches } = queue.pop().unwrap();
		segments += 1;
		if segments > 512 {
			break; // runaway guard (deltas are bounded, but be safe)
		}
		// Heading = `base` rotated by `drift + meander + jitter`: `drift` is a
		// slow mean-reverting course (oxbows that still progress across),
		// `meander` a smooth sine (the regular waviness), `jitter` a fast wobble.
		let mut drift = 0i32;
		let mut jitter = 0i32;
		for step in 0..steps {
			stamp(mask, px.round() as i32, py.round() as i32, radius);
			if drift_clamp > 0 {
				drift += rng.below(3) as i32 - 1;
				if rng.below(3) == 0 {
					drift -= drift.signum();
				}
				drift = drift.clamp(-drift_clamp, drift_clamp);
			}
			let meander = if wave_amp > 0 {
				wave = rotated(wave, wave_speed);
				(wave.1 * wave_amp as f32).round() as i32
			} else {
				0
			};
			jitter += rng.below(5) as i32 - 2;
			if rng.below(2) == 0 {
				jitter -= jitter.signum();
			}
			jitter = jitter.clamp(-jitter_clamp, jitter_clamp);
			let dir = rotated(base, drift + meander + jitter);
			px += dir.0;
			py += dir.1;
			if branches > 0 && step > 8 && rng.below(30) == 0 {
				let off = (6 + rng.below(7) as i32) * if rng.below(2) == 0 { 1 } else { -1 };
				let (b_speed, b_wave) = new_wave(rng);
				queue.push(RiverSeed {
					px,
					py,
					base: rotated(dir, off),
					wave: b_wave,
					wave_speed: b_speed,
					radius,
					steps: steps.saturating_sub(step),
					branches: branches - 1,
				});
				branches -= 1;
			}
			if px < -1.0 || py < -1.0 || px > w as f32 || py > h as f32 {
				break;
			}
		}
	}
}

/// Carve `spec.count` organic blobs into `mask`, each of radius `spec.min..=max`
/// **cells** and set to `fill` (false = land, true = water). Blobs keep ≥2 cells
/// from one another (the shared `id` moat → never touching) and at least `inset`
/// cells from every map edge. Centres are spread so the **edge gap** between a
/// blob and its nearest neighbour is `dist` cells (`dist + this radius + their
/// radius` apart, centre to centre) and never start on existing `fill`. Coasts
/// are noise-perturbed (`warped_fbm`) for varied, non-circular shapes. `salt`
/// decorrelates noise + centres between calls; `id_base` offsets this call's ids
/// so a shared buffer keeps successive calls (main vs small islands)
/// disconnected from each other too.
#[allow(clippy::too_many_arguments)]
fn place_blobs(
	mask: &mut [bool],
	id: &mut [u32],
	w: usize,
	h: usize,
	seed: u64,
	salt: u64,
	id_base: u32,
	spec: Range,
	dist: Span,
	fill: bool,
	inset: i32,
	rng: &mut Rng,
) {
	if spec.count == 0 {
		return;
	}
	let short = w.min(h) as f32;
	let cap = (short / 2.0 - inset as f32 - 1.0).max(2.0);
	let (min_gap, max_gap) = (dist.min.min(dist.max) as f32, dist.max.max(dist.min) as f32);
	let (slo, shi) = (spec.min.min(spec.max) as u32, spec.max.max(spec.min) as u32);
	// (cx, cy, radius) of placed blobs - the radius enters the spacing test.
	let mut centers: Vec<(i32, i32, f32)> = Vec::new();
	for k in 0..spec.count as usize {
		let rr = (slo + rng.below(shi - slo + 1)) as f32; // radius in cells
		let rr = rr.clamp(2.0, cap);
		let lo_x = (rr as i32 + inset).min(w as i32 - 1);
		let hi_x = (w as i32 - 1 - rr as i32 - inset).max(lo_x);
		let lo_y = (rr as i32 + inset).min(h as i32 - 1);
		let hi_y = (h as i32 - 1 - rr as i32 - inset).max(lo_y);
		// Pick a centre: in open `!fill` terrain, with an edge gap to the
		// nearest blob of ≥ min_gap (≤ max_gap on early attempts).
		let mut cx = lo_x;
		let mut cy = lo_y;
		for attempt in 0..64 {
			let tx = lo_x + rng.below((hi_x - lo_x + 1) as u32) as i32;
			let ty = lo_y + rng.below((hi_y - lo_y + 1) as u32) as i32;
			cx = tx;
			cy = ty;
			if mask[ty as usize * w + tx as usize] == fill {
				continue;
			}
			// Smallest edge gap to an existing blob: centre distance minus both radii.
			let gap = centers
				.iter()
				.map(|&(ox, oy, or)| (((tx - ox).pow(2) + (ty - oy).pow(2)) as f32).sqrt() - rr - or)
				.fold(f32::INFINITY, f32::min);
			let ok = centers.is_empty() || (gap >= min_gap && (gap <= max_gap || attempt > 40));
			if ok {
				break;
			}
		}
		centers.push((cx, cy, rr));
		let this = id_base + k as u32;
		let warp_seed = seed ^ salt.wrapping_mul(this as u64 + 1);
		let freq = rr.max(4.0);
		let reach = rr.ceil() as i32 + 2;
		for dy in -reach..=reach {
			for dx in -reach..=reach {
				let (x, y) = (cx + dx, cy + dy);
				if x < inset || y < inset || x >= w as i32 - inset || y >= h as i32 - inset {
					continue; // stay inset from the edges
				}
				let nz = warped_fbm(warp_seed, x as f32 / freq, y as f32 / freq);
				let edge = rr * (0.6 + 0.8 * nz);
				if (dx * dx + dy * dy) as f32 > edge * edge {
					continue;
				}
				let i = y as usize * w + x as usize;
				if id[i] != u32::MAX {
					continue;
				}
				// Disconnection: no *other* blob within Chebyshev 2.
				let mut clash = false;
				'scan: for ny in (y - 2).max(0)..=(y + 2).min(h as i32 - 1) {
					for nx in (x - 2).max(0)..=(x + 2).min(w as i32 - 1) {
						let j = ny as usize * w + nx as usize;
						if id[j] != u32::MAX && id[j] != this {
							clash = true;
							break 'scan;
						}
					}
				}
				if clash {
					continue;
				}
				mask[i] = fill;
				id[i] = this;
			}
		}
	}
}

/// Stamp `count` lakes (water blobs) onto the land in `mask`. Each lake's radius
/// is picked in `[min_r, max_r]` cells; the outline is noise-perturbed.
fn place_lakes(mask: &mut [bool], w: usize, h: usize, seed: u64, count: u8, min_r: u8, max_r: u8, rng: &mut Rng) {
	if max_r == 0 {
		return;
	}
	let (lo, hi) = (min_r.min(max_r).max(1) as u32, max_r.max(min_r).max(1) as u32);
	for k in 0..count as usize {
		let rr = (lo + rng.below(hi - lo + 1)) as f32;
		let (cx, cy) = (rng.below(w as u32) as i32, rng.below(h as u32) as i32);
		let warp_seed = seed ^ 0x4c41_4b45_0000_0000u64.wrapping_mul(k as u64 + 1);
		let freq = rr.max(3.0);
		let reach = rr.ceil() as i32 + 2;
		for dy in -reach..=reach {
			for dx in -reach..=reach {
				let (x, y) = (cx + dx, cy + dy);
				if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
					continue;
				}
				let nz = fbm(warp_seed, x as f32 / freq, y as f32 / freq);
				let edge = rr * (0.65 + 0.7 * nz);
				if (dx * dx + dy * dy) as f32 <= edge * edge {
					mask[y as usize * w + x as usize] = true;
				}
			}
		}
	}
}

/// Set the square of half-width `half` around `(x, y)` to land (false). Square
/// (not disc) so adjacent corridor blocks butt together cleanly.
fn maze_block(mask: &mut [bool], w: usize, h: usize, x: i32, y: i32, half: i32) {
	for dy in -half..=half {
		for dx in -half..=half {
			let (px, py) = (x + dx, y + dy);
			if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
				mask[py as usize * w + px as usize] = false;
			}
		}
	}
}

/// Carve a straight land corridor (half-width `half`) from `a` to `b`.
fn maze_passage(mask: &mut [bool], w: usize, h: usize, a: (i32, i32), b: (i32, i32), half: i32) {
	let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1);
	for s in 0..=steps {
		let x = a.0 + (b.0 - a.0) * s / steps;
		let y = a.1 + (b.1 - a.1) * s / steps;
		maze_block(mask, w, h, x, y, half);
	}
}

/// Carve a labyrinth of **land corridors** into all-water `mask` (true = water):
/// a randomized depth-first maze on a coarse cell grid links every cell with a
/// `corridor`-wide land path, leaving water walls between them - a navigable
/// terrain maze. `spec.min..=max` is the corridor width (picked once); `count`
/// adds that many extra loop openings (braid) so it isn't all dead-ends. The
/// grid is inset `edge` cells so a water moat rings the maze; `symmetrize_mask`
/// later makes it symmetric.
fn carve_maze_land(mask: &mut [bool], w: usize, h: usize, spec: Range, edge: i32, rng: &mut Rng) {
	let edge = edge.max(0) as usize;
	if w < 2 * edge + 5 || h < 2 * edge + 5 {
		return;
	}
	let (lo, hi) = (spec.min.max(1) as u32, spec.max.max(spec.min.max(1)) as u32);
	let width = (lo + rng.below(hi - lo + 1)) as i32;
	let half = (width - 1).max(0) / 2;
	// Water walls are kept ≥3 wide so the coast tiler resolves every strait
	// (2-wide straits are unresolvable and make the Destructive fix grind).
	let wall = 3i32;
	let pitch = (2 * half + 1 + wall) as usize;
	let (avail_w, avail_h) = (w - 2 * edge, h - 2 * edge);
	let cols = (avail_w / pitch).max(1);
	let rows = (avail_h / pitch).max(1);
	if cols < 2 && rows < 2 {
		return;
	}
	let margin_x = edge + (avail_w - (cols - 1) * pitch) / 2;
	let margin_y = edge + (avail_h - (rows - 1) * pitch) / 2;
	let cell_xy = |c: usize, r: usize| -> (i32, i32) { ((margin_x + c * pitch) as i32, (margin_y + r * pitch) as i32) };
	let mut visited = vec![false; cols * rows];
	let mut stack: Vec<(usize, usize)> = Vec::new();
	let (sc, sr) = (rng.below(cols as u32) as usize, rng.below(rows as u32) as usize);
	visited[sr * cols + sc] = true;
	stack.push((sc, sr));
	let (x0, y0) = cell_xy(sc, sr);
	maze_block(mask, w, h, x0, y0, half);
	while let Some(&(c, r)) = stack.last() {
		let mut nbrs: Vec<(usize, usize)> = Vec::new();
		if c > 0 && !visited[r * cols + c - 1] {
			nbrs.push((c - 1, r));
		}
		if c + 1 < cols && !visited[r * cols + c + 1] {
			nbrs.push((c + 1, r));
		}
		if r > 0 && !visited[(r - 1) * cols + c] {
			nbrs.push((c, r - 1));
		}
		if r + 1 < rows && !visited[(r + 1) * cols + c] {
			nbrs.push((c, r + 1));
		}
		if nbrs.is_empty() {
			stack.pop();
			continue;
		}
		let (nc, nr) = nbrs[rng.below(nbrs.len() as u32) as usize];
		visited[nr * cols + nc] = true;
		maze_passage(mask, w, h, cell_xy(c, r), cell_xy(nc, nr), half);
		stack.push((nc, nr));
	}
	// Braid: open extra walls between random adjacent cells for loops.
	for _ in 0..spec.count {
		let c = rng.below(cols as u32) as usize;
		let r = rng.below(rows as u32) as usize;
		let (nc, nr) = match rng.below(4) {
			0 if c > 0 => (c - 1, r),
			1 if c + 1 < cols => (c + 1, r),
			2 if r > 0 => (c, r - 1),
			_ if r + 1 < rows => (c, r + 1),
			_ => continue,
		};
		maze_passage(mask, w, h, cell_xy(c, r), cell_xy(nc, nr), half);
	}
}

/// The source cell `(x, y)` copies its value from for symmetry `sym`, or `None`
/// when `(x, y)` is itself a source cell (left unchanged). Exact for any
/// rectangle.
fn sym_source(sym: Symmetry, x: usize, y: usize, w: usize, h: usize) -> Option<(usize, usize)> {
	match sym {
		Symmetry::None => None,
		Symmetry::LeftRight => (2 * x > w - 1).then(|| (w - 1 - x, y)),
		Symmetry::TopBottom => (2 * y > h - 1).then(|| (x, h - 1 - y)),
		// Four quadrants: fold into the top-left one (mirror both axes).
		Symmetry::FourCorners => {
			let sx = if 2 * x > w - 1 { w - 1 - x } else { x };
			let sy = if 2 * y > h - 1 { h - 1 - y } else { y };
			(sx != x || sy != y).then_some((sx, sy))
		}
		Symmetry::Rotate180 => {
			let (px, py) = (w - 1 - x, h - 1 - y);
			(y * w + x > py * w + px).then_some((px, py))
		}
	}
}

/// Mirror `mask` in place so it obeys `sym` (copy each destination cell from
/// its source). Idempotent and exact.
fn symmetrize_mask(mask: &mut [bool], w: usize, h: usize, sym: Symmetry) {
	if sym == Symmetry::None {
		return;
	}
	let src = mask.to_vec();
	for y in 0..h {
		for x in 0..w {
			if let Some((sx, sy)) = sym_source(sym, x, y, w, h) {
				mask[y * w + x] = src[sy * w + sx];
			}
		}
	}
}

/// Carve a water moat along a symmetry fold: every *source* cell within
/// Chebyshev 2 of the dest region becomes water. Run **before**
/// `symmetrize_mask`, which mirrors the moat, so the two halves keep a wide
/// enough water gap that the shore band can't bridge them (mirrored islands
/// stay disconnected). No-op without a symmetry.
fn carve_fold_moat(mask: &mut [bool], w: usize, h: usize, sym: Symmetry) {
	if sym == Symmetry::None {
		return;
	}
	let mut moat = Vec::new();
	for y in 0..h {
		for x in 0..w {
			if sym_source(sym, x, y, w, h).is_some() {
				continue; // dest cell - its source side carries the moat
			}
			let near_dest = (-2i32..=2).any(|dy| {
				(-2i32..=2).any(|dx| {
					let (nx, ny) = (x as i32 + dx, y as i32 + dy);
					nx >= 0
						&& ny >= 0
						&& nx < w as i32
						&& ny < h as i32
						&& sym_source(sym, nx as usize, ny as usize, w, h).is_some()
				})
			});
			if near_dest {
				moat.push(y * w + x);
			}
		}
	}
	for i in moat {
		mask[i] = true;
	}
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

/// `(dx, dy, tile)` ground entries relative to a template's top-left.
type Footprint = Vec<(i32, i32, TileRef)>;

/// One mirrored copy of a feature for a symmetry fold. All kinds preserve the
/// footprint dimensions, so the source `(w, h)` also sizes the mirror.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MirrorKind {
	LeftRight,
	TopBottom,
	Rotate180,
}

impl MirrorKind {
	/// The `StampOp` sequence that reorients the template for this mirror.
	fn ops(self) -> &'static [StampOp] {
		match self {
			MirrorKind::LeftRight => &[StampOp::FlipH],
			MirrorKind::TopBottom => &[StampOp::FlipV],
			MirrorKind::Rotate180 => &[StampOp::Cw, StampOp::Cw],
		}
	}

	/// The dest origin a source placement at `(x0, y0)` mirrors to.
	fn origin(self, x0: i32, y0: i32, fw: i32, fh: i32, w: usize, h: usize) -> (i32, i32) {
		match self {
			MirrorKind::LeftRight => (w as i32 - x0 - fw, y0),
			MirrorKind::TopBottom => (x0, h as i32 - y0 - fh),
			MirrorKind::Rotate180 => (w as i32 - x0 - fw, h as i32 - y0 - fh),
		}
	}
}

/// The mirror copies a symmetry needs (empty = features stay unmirrored).
/// Four-corner folds need all three; the diagonals/None need none.
fn mirror_kinds(sym: Symmetry) -> &'static [MirrorKind] {
	match sym {
		Symmetry::LeftRight => &[MirrorKind::LeftRight],
		Symmetry::TopBottom => &[MirrorKind::TopBottom],
		Symmetry::Rotate180 => &[MirrorKind::Rotate180],
		Symmetry::FourCorners => &[MirrorKind::LeftRight, MirrorKind::TopBottom, MirrorKind::Rotate180],
		_ => &[],
	}
}

/// A feature template resolved for stamping: its ground-layer footprint, plus a
/// mirrored footprint per fold of the active symmetry (a footprint equals the
/// source when the tiles refuse the mirror op - the user-approved "approximate
/// with templates"). `mirrors` is empty without a feature-mirroring symmetry.
#[derive(Clone)]
struct FeatureStamp {
	cells: Footprint,
	/// The template's full footprint (cells may not fill it - water/holes drop).
	w: i32,
	h: i32,
	mirrors: Vec<(MirrorKind, Footprint)>,
}

/// A template's ground-layer entries as `(dx, dy, tile)`. Water and other
/// layers drop - the generator owns the water beneath its features.
fn ground_footprint(t: &Template, project: &Project) -> Footprint {
	t.resolve(project)
		.map(|entries| {
			entries
				.into_iter()
				.filter(|&(_, _, layer, _)| layer == LAYER_GROUND)
				.map(|(dx, dy, _, tile)| (dx as i32, dy as i32, tile))
				.collect()
		})
		.unwrap_or_default()
}

/// Classify a feature template by its ground-layer tiles: any Obstruction tile
/// ⇒ obstruction; otherwise any passable Land tile ⇒ decoration; water/shore-
/// only templates are skipped (the generator makes its own water).
fn classify_template(t: &Template, project: &Project) -> Option<TileKind> {
	let mut land = false;
	for (_, _, tile) in ground_footprint(t, project) {
		let pack = &project.packs[tile.pack as usize];
		match pack.props.get(family_of(&pack.ids[tile.tile as usize])).and_then(|fp| fp.kind) {
			Some(TileKind::Obstruction) => return Some(TileKind::Obstruction),
			Some(TileKind::Land) => land = true,
			_ => {}
		}
	}
	land.then_some(TileKind::Land)
}

/// The template's ground footprint reoriented for one mirror fold (tiles
/// transformed per `Transformable`). Falls back to the unmirrored footprint
/// when a tile refuses the op - the user-approved "approximate with templates".
/// All folds preserve the footprint dimensions.
fn mirror_for(t: &Template, project: &Project, kind: MirrorKind) -> Footprint {
	let mut cur = t.clone();
	for &op in kind.ops() {
		match cur.transformed(project, op) {
			Ok(next) => cur = next,
			Err(_) => return ground_footprint(t, project),
		}
	}
	ground_footprint(&cur, project)
}

/// Every compatible template that classifies as `kind`, resolved to a
/// [`FeatureStamp`] (footprint + one mirror footprint per symmetry fold).
fn feature_pool(features: &[Template], project: &Project, kind: TileKind, sym: Symmetry) -> Vec<FeatureStamp> {
	features
		.iter()
		.filter(|t| classify_template(t, project) == Some(kind))
		.filter_map(|t| {
			let cells = ground_footprint(t, project);
			if cells.is_empty() {
				return None;
			}
			let mirrors = mirror_kinds(sym).iter().map(|&k| (k, mirror_for(t, project, k))).collect();
			Some(FeatureStamp { cells, w: t.width as i32, h: t.height as i32, mirrors })
		})
		.collect()
}

/// Whether `(x, y)` falls inside any reserved drop-zone circle.
fn in_drop_zone(zones: &[(i32, i32, i32)], x: i32, y: i32) -> bool {
	zones.iter().any(|&(cx, cy, r)| (x - cx).pow(2) + (y - cy).pow(2) <= r * r)
}

/// The mirror of point `(cx, cy)` across one fold.
fn mirror_point(kind: MirrorKind, cx: i32, cy: i32, w: usize, h: usize) -> (i32, i32) {
	match kind {
		MirrorKind::LeftRight => (w as i32 - 1 - cx, cy),
		MirrorKind::TopBottom => (cx, h as i32 - 1 - cy),
		MirrorKind::Rotate180 => (w as i32 - 1 - cx, h as i32 - 1 - cy),
	}
}

/// Stamp templates from `pool` inside one patch circle (centre `(cx, cy)`,
/// radius `pr`) toward `target` covered ground cells. A populated cell must be
/// land at Chebyshev ≥ `shore_margin` from water, inside the patch, outside
/// every drop zone, off the accessibility keep-clear mask, and unclaimed; under
/// a symmetry the placement sits wholly in the source region and its mirrors
/// are stamped too (so feature patches mirror). Because the keep-clear corridors
/// are vetoed per cell, a template is only ever placed whole - roads / mazes
/// never erase part of an already-stamped template. `placed` accumulates across
/// patches for the stats.
#[allow(clippy::too_many_arguments)]
fn stamp_patch(
	overlay: &mut [Option<TileRef>],
	pool: &[FeatureStamp],
	mask: &[bool],
	keepclear: &[bool],
	zones: &[(i32, i32, i32)],
	(w, h): (usize, usize),
	(cx, cy, pr): (i32, i32, i32),
	target: usize,
	shore_margin: i32,
	sym: Symmetry,
	placed: &mut usize,
	rng: &mut Rng,
) {
	if target == 0 || pool.is_empty() || pr <= 0 {
		return;
	}
	let in_patch = |x: i32, y: i32| (x - cx).pow(2) + (y - cy).pow(2) <= pr * pr;
	let eligible = |overlay: &[Option<TileRef>], x: i32, y: i32| -> bool {
		if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
			return false;
		}
		let i = y as usize * w + x as usize;
		if overlay[i].is_some() || keepclear[i] || in_drop_zone(zones, x, y) {
			return false;
		}
		for dy in -shore_margin..=shore_margin {
			for dx in -shore_margin..=shore_margin {
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
	let mirroring = !pool[0].mirrors.is_empty();
	let area = (std::f32::consts::PI * pr as f32 * pr as f32) as usize;
	let cap = 8 * area.max(16);
	let (mut covered, mut attempts) = (0usize, 0usize);
	while covered < target && attempts < cap {
		attempts += 1;
		let fs = &pool[rng.below(pool.len() as u32) as usize];
		if fs.w > w as i32 || fs.h > h as i32 {
			continue;
		}
		// A random origin inside the patch's bounding box.
		let bx_lo = (cx - pr).max(0).min(w as i32 - fs.w);
		let bx_hi = (cx + pr).min(w as i32 - fs.w).max(bx_lo);
		let by_lo = (cy - pr).max(0).min(h as i32 - fs.h);
		let by_hi = (cy + pr).min(h as i32 - fs.h).max(by_lo);
		let x0 = bx_lo + rng.below((bx_hi - bx_lo + 1) as u32) as i32;
		let y0 = by_lo + rng.below((by_hi - by_lo + 1) as u32) as i32;
		// Source cells: inside the patch, eligible, and (when mirroring) in the
		// source region so the mirrors land in the dest regions.
		let src_ok = fs.cells.iter().all(|&(dx, dy, _)| {
			let (x, y) = (x0 + dx, y0 + dy);
			in_patch(x, y)
				&& eligible(overlay, x, y)
				&& (!mirroring || sym_source(sym, x as usize, y as usize, w, h).is_none())
		});
		if !src_ok {
			continue;
		}
		let mirrors_ok = fs.mirrors.iter().all(|(kind, mcells)| {
			let (mx0, my0) = kind.origin(x0, y0, fs.w, fs.h, w, h);
			mx0 >= 0
				&& my0 >= 0
				&& mx0 + fs.w <= w as i32
				&& my0 + fs.h <= h as i32
				&& mcells.iter().all(|&(dx, dy, _)| eligible(overlay, mx0 + dx, my0 + dy))
		});
		if !mirrors_ok {
			continue;
		}
		for &(dx, dy, tile) in &fs.cells {
			overlay[(y0 + dy) as usize * w + (x0 + dx) as usize] = Some(tile);
		}
		covered += fs.cells.len();
		*placed += fs.cells.len();
		for (kind, mcells) in &fs.mirrors {
			let (mx0, my0) = kind.origin(x0, y0, fs.w, fs.h, w, h);
			for &(dx, dy, tile) in mcells {
				overlay[(my0 + dy) as usize * w + (mx0 + dx) as usize] = Some(tile);
			}
			covered += mcells.len();
			*placed += mcells.len();
		}
	}
}

/// Mark `(x, y)` and its `sym` mirrors keep-clear (one cell of a carved road /
/// maze). Obstructions are not stamped on a marked cell, so templates are never
/// broken or partially erased - the clear corridors are planned *before*
/// stamping, not cut out afterwards.
fn mark_clear(keep: &mut [bool], x: i32, y: i32, w: usize, h: usize, sym: Symmetry) {
	if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
		return;
	}
	keep[y as usize * w + x as usize] = true;
	for &kind in mirror_kinds(sym) {
		let (mx, my) = mirror_point(kind, x, y, w, h);
		if mx >= 0 && my >= 0 && (mx as usize) < w && (my as usize) < h {
			keep[my as usize * w + mx as usize] = true;
		}
	}
}

/// Mark a filled disc of radius `hw` around `(x, y)` keep-clear (with `sym`
/// mirrors). A `hw` of 0 marks just the single spine cell.
fn mark_disc(keep: &mut [bool], x: i32, y: i32, hw: i32, w: usize, h: usize, sym: Symmetry) {
	for dy in -hw..=hw {
		for dx in -hw..=hw {
			if dx * dx + dy * dy > hw * hw {
				continue;
			}
			mark_clear(keep, x + dx, y + dy, w, h, sym);
		}
	}
}

/// Meander from `a` to `b`, marking a `half`-wide keep-clear corridor (thinned
/// to a one-cell spine inside the centre keep-zone `(cx, cy, keep_r)`, so the
/// centre stays dense). The walk curves around the bearing, not straight.
#[allow(clippy::too_many_arguments)]
fn mark_corridor(keep: &mut [bool], a: (f32, f32), b: (f32, f32), half: i32, (cx, cy, keep_r): (f32, f32, f32), (w, h): (usize, usize), sym: Symmetry, rng: &mut Rng) {
	let (mut px, mut py) = a;
	let (bx, by) = b;
	let mut turn = 0i32;
	for _ in 0..4 * (w + h) {
		let in_centre = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() < keep_r;
		let hw = if in_centre { 0 } else { half };
		mark_disc(keep, px.round() as i32, py.round() as i32, hw, w, h, sym);
		let (dx, dy) = (bx - px, by - py);
		let dist = (dx * dx + dy * dy).sqrt();
		if dist <= 1.2 {
			break;
		}
		turn += rng.below(3) as i32 - 1;
		if rng.below(3) == 0 {
			turn -= turn.signum();
		}
		turn = turn.clamp(-6, 6);
		let dir = rotated((dx / dist, dy / dist), turn);
		px += dir.0;
		py += dir.1;
	}
}

/// `Paths` accessibility: mark `value`-driven walkable roads keep-clear of
/// obstructions. **One road per 5 accessibility**, each a *multi-step random
/// curve* - it starts at an extreme (a jittered corner), wanders through a few
/// random interior waypoints, then ends at another extreme. The centre
/// keep-zone is only ever cut to a thin spine so it stays a dense strongpoint.
/// Marks mirror for `sym`.
fn carve_access_paths(keep: &mut [bool], (w, h): (usize, usize), value: u8, sym: Symmetry, rng: &mut Rng) {
	let value = value.min(100) as i32;
	let count = (value / 5) as usize; // one road per 5 accessibility
	if count == 0 {
		return;
	}
	let half = (1 + value / 50).clamp(1, 3); // corridor half-width 1..3
	let centre = ((w as f32 - 1.0) / 2.0, (h as f32 - 1.0) / 2.0);
	let keep_r = w.min(h) as f32 * 0.22;
	// A jittered corner - a random point on the map's periphery (an "extreme").
	let extreme = |rng: &mut Rng| -> (f32, f32) {
		let c = rng.below(4) as u8;
		let jx = rng.below((w as u32 / 4).max(1)) as f32;
		let jy = rng.below((h as u32 / 4).max(1)) as f32;
		let x = if c & 1 == 0 { jx } else { (w as f32 - 1.0) - jx };
		let y = if c & 2 == 0 { jy } else { (h as f32 - 1.0) - jy };
		(x, y)
	};
	let interior = |rng: &mut Rng| -> (f32, f32) { (rng.below(w as u32) as f32, rng.below(h as u32) as f32) };
	for _ in 0..count {
		// extreme -> a few random waypoints -> extreme (the multi-step curve).
		let mut pts = vec![extreme(rng)];
		let waypoints = 2 + rng.below(3); // 2..=4 interior turns
		for _ in 0..waypoints {
			pts.push(interior(rng));
		}
		pts.push(extreme(rng));
		for seg in pts.windows(2) {
			mark_corridor(keep, seg[0], seg[1], half, (centre.0, centre.1, keep_r), (w, h), sym, rng);
		}
	}
}

/// `Labyrinth` accessibility: weave a maze of corridors across the whole map,
/// marking its passages keep-clear of obstructions. A randomized depth-first
/// traversal of a coarse cell grid knocks walls between cells; each visited
/// cell and each opened wall is marked as a `corr`-wide corridor, so the
/// stamped obstructions read as maze walls. Corridor width (how open the maze
/// is) rises with the accessibility `value`. Marks mirror for `sym`.
fn carve_labyrinth(keep: &mut [bool], (w, h): (usize, usize), value: u8, sym: Symmetry, rng: &mut Rng) {
	let value = value.min(100) as i32;
	let corr = (1 + value / 40).clamp(1, 3); // corridor half-width 1..3
	let wall = 2i32; // wall thickness between corridors
	let pitch = (2 * corr + 1 + wall) as usize; // cell-to-cell spacing
	let cols = (w / pitch).max(1);
	let rows = (h / pitch).max(1);
	if cols < 2 && rows < 2 {
		return;
	}
	// Centre the cell grid within the map.
	let margin_x = (w.saturating_sub((cols - 1) * pitch)) / 2;
	let margin_y = (h.saturating_sub((rows - 1) * pitch)) / 2;
	let cell_xy = |c: usize, r: usize| -> (i32, i32) { ((margin_x + c * pitch) as i32, (margin_y + r * pitch) as i32) };
	let mut visited = vec![false; cols * rows];
	let mut stack: Vec<(usize, usize)> = Vec::new();
	let (sc, sr) = (rng.below(cols as u32) as usize, rng.below(rows as u32) as usize);
	visited[sr * cols + sc] = true;
	stack.push((sc, sr));
	let (x0, y0) = cell_xy(sc, sr);
	mark_disc(keep, x0, y0, corr, w, h, sym);
	while let Some(&(c, r)) = stack.last() {
		// Unvisited orthogonal neighbours of the current cell.
		let mut nbrs: Vec<(usize, usize)> = Vec::new();
		if c > 0 && !visited[r * cols + c - 1] {
			nbrs.push((c - 1, r));
		}
		if c + 1 < cols && !visited[r * cols + c + 1] {
			nbrs.push((c + 1, r));
		}
		if r > 0 && !visited[(r - 1) * cols + c] {
			nbrs.push((c, r - 1));
		}
		if r + 1 < rows && !visited[(r + 1) * cols + c] {
			nbrs.push((c, r + 1));
		}
		if nbrs.is_empty() {
			stack.pop();
			continue;
		}
		let (nc, nr) = nbrs[rng.below(nbrs.len() as u32) as usize];
		visited[nr * cols + nc] = true;
		// Carve a straight corridor from the current cell centre to the new one.
		let (cxp, cyp) = cell_xy(c, r);
		let (nxp, nyp) = cell_xy(nc, nr);
		let steps = pitch as i32;
		for s in 0..=steps {
			let x = cxp + (nxp - cxp) * s / steps;
			let y = cyp + (nyp - cyp) * s / steps;
			mark_disc(keep, x, y, corr, w, h, sym);
		}
		stack.push((nc, nr));
	}
}

/// Where a [`GenSession`] is in its pipeline.
enum Phase {
	/// Build the final land/water mask for the generator (one step).
	Shape,
	/// Stamp feature-template patches (obstructions, then decorations).
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
	obstruction_pool: Vec<FeatureStamp>,
	decoration_pool: Vec<FeatureStamp>,
	rng: Rng,
	phase: Phase,
	mask: Vec<bool>,
	// Stamping state.
	overlay: Vec<Option<TileRef>>,
	/// Accessibility keep-clear mask (true = no obstruction here): the planned
	/// road / labyrinth corridors, computed before stamping so templates land
	/// whole and are never partially erased afterwards.
	keepclear: Vec<bool>,
	/// Reserved drop-zone circles (incl. symmetry mirrors) - kept clear of features.
	drop_zones: Vec<(i32, i32, i32)>,
	/// The current kind's patches `(cx, cy, radius, target cells)`.
	patches: Vec<(i32, i32, i32, usize)>,
	placed: usize,
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
	/// Validate params, resolve the land/water packs, and build the feature
	/// pools from `features` (the compatible stock/user templates the caller
	/// supplies). Cheap; the project is not touched.
	pub fn new(project: &Project, p: GenParams, features: &[Template]) -> Result<Self, String> {
		// The land pack: first pack with a LAND variant group (props-driven). The
		// terrain brush resolves the same way (`Project::variant_family`) so a
		// hand-painted coast matches a generated one.
		let (pack_idx, land_family) = project
			.variant_family(TileKind::Land)
			.ok_or("no pack with a LAND variant group (tiles.props.json) - add a tileset like GREEN")?;
		// The water pack + its WATER variant group: the run starts from a
		// clean slate - the whole bottom layer refills from this group and
		// the ground layer is fully rewritten, so nothing of the previous
		// map survives under the generated terrain.
		let (water_pack_idx, water_family) = project
			.variant_family(TileKind::Water)
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
		// Feature pools: the supplied templates, classified by their ground
		// tiles. Obstruction templates block; everything else passable decorates.
		let obstruction_pool = feature_pool(features, project, TileKind::Obstruction, p.symmetry);
		let decoration_pool = feature_pool(features, project, TileKind::Land, p.symmetry);

		let (w, h) = (project.width as usize, project.height as usize);
		let n = w * h;
		let s = Self {
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
			phase: Phase::Shape,
			mask: vec![false; n],
			overlay: Vec::new(),
			keepclear: Vec::new(),
			drop_zones: Vec::new(),
			patches: Vec::new(),
			placed: 0,
			obstructions_placed: 0,
			shore_changed: 0,
			fix: None,
			fix_found: 0,
			was_dirty: project.dirty(),
			mutated: false,
			stats: None,
		};
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
			Phase::Shape => ("terrain", 0.40),
			Phase::Stamp { decorations: false } => ("features", 0.50),
			Phase::Stamp { decorations: true } => ("decorations", 0.58),
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
			Phase::Shape => {
				self.build_shape();
				self.begin_stamps();
			}
			Phase::Stamp { decorations } => {
				// Stamp every patch of this kind (a clone of the pool sidesteps
				// the overlay/rng borrow; only two passes per run).
				let pool = if decorations { self.decoration_pool.clone() } else { self.obstruction_pool.clone() };
				let patches = std::mem::take(&mut self.patches);
				// Obstructions may sit right next to the shore at low accessibility
				// (margin 0); higher accessibility keeps coasts clearer. Decorations
				// keep a mild 1-cell buffer.
				let shore_margin = if decorations { 1 } else { (self.p.accessibility as i32 / 40).min(2) };
				for &(cx, cy, pr, target) in &patches {
					stamp_patch(
						&mut self.overlay,
						&pool,
						&self.mask,
						&self.keepclear,
						&self.drop_zones,
						(self.w, self.h),
						(cx, cy, pr),
						target,
						shore_margin,
						self.p.symmetry,
						&mut self.placed,
						&mut self.rng,
					);
				}
				if !decorations {
					// The roads / labyrinth were already planned keep-clear before
					// stamping (so templates land whole); obstructions simply
					// avoided them. Count what landed, then stamp decorations.
					self.obstructions_placed = self.overlay.iter().filter(|c| c.is_some()).count();
					self.placed = 0;
					self.patches = self.compute_patches(self.p.decorations);
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
				let (shore, unresolved) = match self.p.shore {
					ShoreMethod::Sweep => project.auto_shore(None),
					ShoreMethod::LoopWalk => project.auto_shore_alt(None),
					ShoreMethod::None => (0, 0),
				};
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

	/// Build the final land/water mask for the chosen generator: base shape,
	/// then rivers / lakes, the island fold moat, depinch, and the symmetry
	/// mirror last so the result is exactly symmetric. Bodies stay `EDGE` cells
	/// off the border (they never touch an edge).
	fn build_shape(&mut self) {
		const EDGE: i32 = 2;
		let (w, h) = (self.w, self.h);
		let n = w * h;
		let seed = self.p.seed;
		// A default spacing for Continents/Central Seas (the id moat already
		// stops them touching; this just spreads them).
		let spread = Span { min: 8, max: 100 };
		let mut mask = match self.p.generator {
			Generator::Islands => {
				let mut m = vec![true; n]; // all water
				let mut id = vec![u32::MAX; n];
				let main = self.p.main_islands;
				place_blobs(&mut m, &mut id, w, h, seed ^ 0x1111, 0x1500_0000_0000_0000, 0, main, self.p.main_dist, false, EDGE, &mut self.rng);
				place_blobs(
					&mut m, &mut id, w, h, seed ^ 0x2222, 0x2600_0000_0000_0000, main.count as u32,
					self.p.small_islands, self.p.small_dist, false, EDGE, &mut self.rng,
				);
				m
			}
			Generator::Continents => {
				let mut m = vec![true; n]; // all water
				let mut id = vec![u32::MAX; n];
				place_blobs(&mut m, &mut id, w, h, seed ^ 0x3333, 0x3700_0000_0000_0000, 0, self.p.continents, spread, false, EDGE, &mut self.rng);
				m
			}
			Generator::CentralSeas => {
				let mut m = vec![false; n]; // all land
				let mut id = vec![u32::MAX; n];
				place_blobs(&mut m, &mut id, w, h, seed ^ 0x4444, 0x4800_0000_0000_0000, 0, self.p.seas, spread, true, EDGE, &mut self.rng);
				m
			}
			Generator::Maze => {
				let mut m = vec![true; n]; // all water; carve land corridors
				carve_maze_land(&mut m, w, h, self.p.maze, EDGE, &mut self.rng);
				m
			}
			Generator::Land | Generator::Rivers | Generator::RiverRaid => vec![false; n], // all land
		};
		// Inland water: rivers (curly for everything but River Raid) + lakes. The
		// Maze's water *is* its structure, so it carves no extra rivers.
		match self.p.generator {
			Generator::Maze => {}
			Generator::RiverRaid => carve_rivers(&mut mask, w, h, self.p.rivers, Curliness::Straight, &mut self.rng),
			Generator::Rivers => carve_rivers(&mut mask, w, h, self.p.rivers, Curliness::VeryCurly, &mut self.rng),
			_ => carve_rivers(&mut mask, w, h, self.p.rivers, Curliness::Curly, &mut self.rng),
		}
		if matches!(self.p.generator, Generator::Islands | Generator::Continents | Generator::Land) {
			let l = self.p.lakes;
			place_lakes(&mut mask, w, h, seed ^ 0x5a5a, l.count, l.min, l.max, &mut self.rng);
		}
		// Islands keep a water moat along the fold so mirrored islands stay
		// disconnected (no global smoothing reconnects them).
		if self.p.generator == Generator::Islands {
			carve_fold_moat(&mut mask, w, h, self.p.symmetry);
		}
		depinch(&mut mask, w, h);
		symmetrize_mask(&mut mask, w, h, self.p.symmetry);
		// Drop zones overwrite the terrain with flat land LAST so they stay full
		// circles of the chosen radius (depinch can't nibble them). They are
		// carved with their mirrors, so the map stays symmetric; the coast pass
		// tidies their edges.
		self.drop_zones = self.compute_drop_zones();
		self.carve_drop_zones(&mut mask);
		self.mask = mask;
	}

	/// Plan the accessibility keep-clear corridors, then the overlay + the first
	/// (obstruction) patches. Drop zones were already placed in `build_shape`.
	fn begin_stamps(&mut self) {
		let n = self.w * self.h;
		self.overlay = vec![None; n];
		self.placed = 0;
		self.obstructions_placed = 0;
		// Plan roads / labyrinth as a keep-clear mask up front, so obstruction
		// templates avoid them and are never partially erased afterwards.
		self.keepclear = vec![false; n];
		match self.p.accessibility_mode {
			AccessibilityMode::Random => {}
			AccessibilityMode::Paths => carve_access_paths(&mut self.keepclear, (self.w, self.h), self.p.accessibility, self.p.symmetry, &mut self.rng),
			AccessibilityMode::Labyrinth => carve_labyrinth(&mut self.keepclear, (self.w, self.h), self.p.accessibility, self.p.symmetry, &mut self.rng),
		}
		self.patches = self.compute_patches(self.p.obstructions);
		self.phase = Phase::Stamp { decorations: false };
	}

	/// A land cell in the symmetry's canonical (source) region, or `None`.
	fn pick_land_center(&mut self) -> Option<(i32, i32)> {
		let (w, h) = (self.w, self.h);
		for _ in 0..256 {
			let (x, y) = (self.rng.below(w as u32) as usize, self.rng.below(h as u32) as usize);
			if !self.mask[y * w + x] && sym_source(self.p.symmetry, x, y, w, h).is_none() {
				return Some((x as i32, y as i32));
			}
		}
		None
	}

	/// The drop-zone circles (each centre in the canonical region, with its
	/// symmetry mirrors added). Drop zones **overwrite the terrain with flat,
	/// fully-accessible land** (`carve_drop_zones`), so a centre needn't start on
	/// land - it only has to be inset from the map edges (not at the very edge)
	/// and spread **far apart** from the others (each centre maximises its
	/// distance to those already placed). Features are kept out of them.
	fn compute_drop_zones(&mut self) -> Vec<(i32, i32, i32)> {
		let (w, h) = (self.w, self.h);
		let dz = self.p.drop_zones;
		if dz.count == 0 {
			return Vec::new();
		}
		let (lo, hi) = (dz.min.min(dz.max) as u32, dz.max.max(dz.min) as u32);
		// Cap so the inset centre band (disc on-map + a 2-cell edge margin) is
		// never empty.
		let cap = (w.min(h) as i32 / 2 - 3).max(2);
		let mut centers: Vec<(i32, i32)> = Vec::new(); // canonical centres, for spreading
		let mut zones = Vec::new();
		for _ in 0..dz.count {
			let r = ((lo + self.rng.below(hi - lo + 1)) as i32).clamp(1, cap);
			let margin = r + 2; // keep the disc + a 2-cell gap off the edge
			let (xlo, xhi) = (margin, w as i32 - 1 - margin);
			let (ylo, yhi) = (margin, h as i32 - 1 - margin);
			if xlo > xhi || ylo > yhi {
				continue; // no inset room for a zone this big
			}
			// Sample inset, canonical candidates; keep the one farthest from the
			// zones already placed (spreads them apart).
			let mut best: Option<(i32, i32)> = None;
			let mut best_score = -1.0f32;
			for _ in 0..300 {
				let cx = xlo + self.rng.below((xhi - xlo + 1) as u32) as i32;
				let cy = ylo + self.rng.below((yhi - ylo + 1) as u32) as i32;
				if sym_source(self.p.symmetry, cx as usize, cy as usize, w, h).is_some() {
					continue;
				}
				let score = centers.iter().map(|&(px, py)| ((cx - px).pow(2) + (cy - py).pow(2)) as f32).fold(f32::INFINITY, f32::min);
				if score > best_score {
					best_score = score;
					best = Some((cx, cy));
				}
			}
			let Some((cx, cy)) = best else { continue };
			centers.push((cx, cy));
			zones.push((cx, cy, r));
			for &kind in mirror_kinds(self.p.symmetry) {
				let (mx, my) = mirror_point(kind, cx, cy, w, h);
				zones.push((mx, my, r));
			}
		}
		zones
	}

	/// Stamp each drop zone as a solid disc of **flat land** into `mask`
	/// (overwriting water), so every landing zone is usable ground regardless of
	/// the underlying terrain.
	fn carve_drop_zones(&self, mask: &mut [bool]) {
		let (w, h) = (self.w as i32, self.h as i32);
		for &(cx, cy, r) in &self.drop_zones {
			for dy in -r..=r {
				for dx in -r..=r {
					if dx * dx + dy * dy > r * r {
						continue;
					}
					let (x, y) = (cx + dx, cy + dy);
					if x >= 0 && y >= 0 && x < w && y < h {
						mask[y as usize * self.w + x as usize] = false; // land
					}
				}
			}
		}
	}

	/// `count` feature patches `(cx, cy, radius, target cells)` sized by `spec`;
	/// `accessibility` sets each patch's target coverage (lower = denser).
	fn compute_patches(&mut self, spec: Range) -> Vec<(i32, i32, i32, usize)> {
		if spec.count == 0 {
			return Vec::new();
		}
		let (w, h) = (self.w, self.h);
		let cov = (1.0 - self.p.accessibility.min(100) as f32 / 100.0) * 0.55;
		let (lo, hi) = (spec.min.min(spec.max) as u32, spec.max.max(spec.min) as u32);
		let cap = (w.min(h) as f32 / 2.0).max(2.0);
		let mut out = Vec::new();
		for _ in 0..spec.count {
			let r = ((lo + self.rng.below(hi - lo + 1)) as f32).clamp(1.0, cap) as i32;
			let Some((cx, cy)) = self.pick_land_center() else { continue };
			let target = (cov * std::f32::consts::PI * r as f32 * r as f32) as usize;
			out.push((cx, cy, r, target));
		}
		out
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
	pub fn generate_terrain(&mut self, p: &GenParams, features: &[Template]) -> Result<GenStats, String> {
		let mut session = GenSession::new(self, *p, features)?;
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

	/// The stock GREEN templates, loaded straight off the asset tree (the app
	/// hands the generator its live library; tests load from disk).
	fn green_templates() -> Vec<Template> {
		let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets/templates/GREEN");
		let mut paths: Vec<_> = std::fs::read_dir(&dir)
			.unwrap()
			.flatten()
			.map(|e| e.path())
			.filter(|p| p.extension().is_some_and(|x| x == "json"))
			.collect();
		paths.sort();
		paths.iter().filter_map(|p| Template::load(p).ok()).collect()
	}

	fn green() -> Project {
		Project::new(64, 64, &["GREEN".into()], &assets_root(), 7).unwrap()
	}

	/// Build a RAW land/water mask by hand (the gnarly shapes a free-hand brush
	/// or an import makes - what the generator never produces): water everywhere,
	/// land where `f`.
	fn raw_mask(n: u16, f: impl Fn(u16, u16) -> bool) -> Project {
		use crate::{LAYER_GROUND, LAYER_WATER, TileRef, Transform};
		let mut p = Project::new(n, n, &["GREEN".into()], &assets_root(), 1).unwrap();
		let (lp, lf) = p.variant_family(TileKind::Land).unwrap();
		let (wp, wf) = p.variant_family(TileKind::Water).unwrap();
		let land = p.packs[lp].group_tiles(&lf)[0];
		let water = p.packs[wp].group_tiles(&wf)[0];
		let (lp, wp) = (lp as u8, wp as u8);
		let mut edits = Vec::new();
		for y in 0..n {
			for x in 0..n {
				edits.push((x, y, LAYER_WATER, Some(TileRef { pack: wp, tile: water, transform: Transform::default() })));
				let g = f(x, y).then_some(TileRef { pack: lp, tile: land, transform: Transform::default() });
				edits.push((x, y, LAYER_GROUND, g));
			}
		}
		p.place_many(&edits);
		p
	}

	#[test]
	fn shore_repair_reaches_a_perfect_coast_on_gnarly_masks() {
		use crate::FixStrength;
		// Dense mosaics with checkerboard diagonal contacts, thin straits, and
		// scattered noise - what a free-hand brush or import makes, where a single
		// fix pass leaves many broken seams (and the GREEN set can't tile some of
		// them without reshaping). Both accurate tiers must still reach a clean
		// coast: Aggressive escalates from re-tiling to reshaping when it stalls,
		// Full reshapes throughout.
		let cases: [(&str, fn(u16, u16) -> bool); 3] = [
			("checker-3", |x, y| ((x / 3) + (y / 3)) % 2 == 0),
			("thin-diag", |x, y| (x as i32 - y as i32).rem_euclid(8) < 2),
			("hash", |x, y| (x.wrapping_mul(2654) ^ y.wrapping_mul(40503)) % 5 < 2),
		];
		for (name, f) in cases {
			// The raw mask is riddled with missing coast (faithful detector).
			assert!(raw_mask(32, f).shore_defects(None) > 0, "{name}: raw mask should have defects");
			for (label, strength) in [("aggressive", FixStrength::Mangle), ("full", FixStrength::Destructive)] {
				let mut p = raw_mask(32, f);
				let (_, remaining) = p.shore_repair(None, false, strength);
				assert_eq!(remaining, 0, "{name}/{label}: repair must leave zero defects");
				assert_eq!(p.shore_defects(None), 0, "{name}/{label}: and the coast is genuinely clean");
			}
		}
	}

	#[test]
	fn variant_family_picks_the_land_and_water_groups_deterministically() {
		// The generator and the terrain brush both resolve "land"/"water" through
		// this, so they must agree and the pick must be stable across calls
		// (HashMap order isn't - the sort makes it so).
		let p = green();
		let (lp, lf) = p.variant_family(TileKind::Land).expect("GREEN ships a LAND variant group");
		let (wp, wf) = p.variant_family(TileKind::Water).expect("GREEN ships a WATER variant group");
		assert!(!p.packs[lp].group_tiles(&lf).is_empty(), "the LAND group has tiles");
		assert!(!p.packs[wp].group_tiles(&wf).is_empty(), "the WATER group has tiles");
		assert_ne!((lp, &lf), (wp, &wf), "land and water are different families");
		assert_eq!(p.variant_family(TileKind::Land), Some((lp, lf)), "same pick every call");
	}

	/// The shared defaults for `g` at `seed`, with feature patches off (tests
	/// turn the ones they care about back on).
	fn params(g: Generator, seed: u64) -> GenParams {
		let mut p = GenParams::defaults(g);
		p.seed = seed;
		p.drop_zones.count = 0;
		p.obstructions.count = 0;
		p.decorations.count = 0;
		p
	}

	fn make(p: GenParams) -> (Project, GenStats) {
		let mut proj = green();
		let stats = proj.generate_terrain(&p, &green_templates()).unwrap();
		(proj, stats)
	}

	/// `(component count, every distinct 4-connected land component keeps a
	/// gap from the others)` - the disconnection guarantee.
	fn land_components(land: &[bool], w: usize, h: usize) -> (usize, bool) {
		let mut label = vec![u32::MAX; w * h];
		let mut seen = vec![false; w * h];
		let mut count = 0u32;
		for start in 0..w * h {
			if !land[start] || seen[start] {
				continue;
			}
			let id = count;
			crate::grid::flood4(w, h, start, &mut seen, |j| land[j], |i| label[i] = id);
			count += 1;
		}
		// No two distinct components touch even diagonally (8-adjacency).
		let mut separated = true;
		for y in 0..h as i32 {
			for x in 0..w as i32 {
				let i = y as usize * w + x as usize;
				if !land[i] {
					continue;
				}
				for dy in -1i32..=1 {
					for dx in -1i32..=1 {
						let (nx, ny) = (x + dx, y + dy);
						if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
							continue;
						}
						let j = ny as usize * w + nx as usize;
						if land[j] && label[j] != label[i] {
							separated = false;
						}
					}
				}
			}
		}
		(count as usize, separated)
	}

	/// Border cells (the map's outer ring) that are water.
	fn border_water(p: &Project) -> (usize, usize) {
		let (w, h) = (p.width, p.height);
		let mut water = 0;
		let mut total = 0;
		for y in 0..h {
			for x in 0..w {
				if x != 0 && y != 0 && x != w - 1 && y != h - 1 {
					continue;
				}
				total += 1;
				if p.cell(x, y).unwrap()[crate::LAYER_GROUND].is_none() {
					water += 1;
				}
			}
		}
		(water, total)
	}

	/// The generated land/water mask just after the Shape step (before shore -
	/// the coast-retiling/seam-fix pass is a downstream concern). `land[i]` =
	/// the cell is land. `step` only mutates the project from Apply onward, so
	/// stepping through Shape leaves the passed project untouched.
	fn shape_land(proj: &mut Project, p: GenParams) -> Vec<bool> {
		let mut s = GenSession::new(proj, p, &[]).unwrap();
		while s.progress().0 == "terrain" {
			s.step(proj);
		}
		s.mask.iter().map(|&water| !water).collect()
	}

	#[test]
	fn generation_replaces_both_layers_fully() {
		let mut p = Project::new(48, 48, &["GREEN".into()], &assets_root(), 7).unwrap();
		// Sabotage the bottom layer the way a previous map would: a GREEN
		// land tile pretending to be water, and a hole. Both used to
		// survive the run and show through every generated water cell.
		p.place(3, 3, crate::LAYER_WATER, Some(TileRef { pack: 1, tile: 0, transform: Transform::default() }));
		p.place(5, 5, crate::LAYER_WATER, None);
		p.generate_terrain(&params(Generator::Islands, 9), &green_templates()).unwrap();
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
		let feats = green_templates();
		let run = |seed| {
			let mut p = green();
			let mut q = params(Generator::Islands, seed);
			q.obstructions.count = 2;
			q.decorations.count = 2;
			p.generate_terrain(&q, &feats).unwrap();
			p.hash()
		};
		assert_eq!(run(42), run(42), "same seed + params = same map");
		assert_ne!(run(42), run(43), "a different seed moves the terrain");
	}

	#[test]
	fn continents_ring_with_ocean_land_reaches_the_border() {
		// Continents sit inset from the edge - the border is mostly ocean.
		let mut cq = params(Generator::Continents, 3);
		cq.rivers.count = 0;
		let (cont, cstats) = make(cq);
		assert!(cstats.land > 0 && cstats.water > 0, "continents have both land and sea");
		let (cw, ct) = border_water(&cont);
		assert!(cw * 2 > ct, "continents border is mostly ocean ({cw}/{ct})");

		// Land fills edge to edge: its border is overwhelmingly land - only the
		// odd river mouth (rivers now reach any edge) breaks the coastline,
		// unlike Continents' mostly-ocean border.
		let (land, _) = make(params(Generator::Land, 3));
		let (lw, lt) = border_water(&land);
		assert!(lw * 3 < lt, "Land border is overwhelmingly land ({lw}/{lt})");
	}

	#[test]
	fn big_radius_continents_fill_most_and_sized_seas_cover_40_to_80() {
		let n = 64 * 64;
		// A huge continent radius clamps to ~half the map and dominates it.
		let mut c = params(Generator::Continents, 5);
		c.rivers.count = 0;
		c.lakes.count = 0;
		c.continents = Range { count: 1, min: 40, max: 40 };
		let land_pct = make(c).1.land * 100 / n;
		assert!(land_pct >= 55, "continents should fill most of the map, got {land_pct}%");
		// One central sea sized (radius 26) for ~50% water - in the 40-80% band.
		let mut s = params(Generator::CentralSeas, 5);
		s.rivers.count = 0;
		s.seas = Range { count: 1, min: 26, max: 26 };
		let water_pct = make(s).1.water * 100 / n;
		assert!((40..=80).contains(&water_pct), "central seas should cover 40-80%, got {water_pct}%");
	}

	#[test]
	fn maze_has_substantial_land_and_water() {
		// The Maze is land corridors + water walls - both must be prominent (it's
		// a land/water structure, not a sprinkle of either).
		let (_, stats) = make(params(Generator::Maze, 9));
		let n = 64 * 64;
		assert!(stats.land > n / 6, "maze land too sparse ({})", stats.land);
		assert!(stats.water > n / 6, "maze water too sparse ({})", stats.water);
		// A perfect maze carves a near-constant area, so compare the *layout*: the
		// walk differs by seed, and the same seed reproduces it.
		let mut p = green();
		let a = shape_land(&mut p, params(Generator::Maze, 9));
		let b = shape_land(&mut p, params(Generator::Maze, 10));
		let again = shape_land(&mut p, params(Generator::Maze, 9));
		assert_ne!(a, b, "different seeds should produce different mazes");
		assert_eq!(a, again, "same seed reproduces the maze");
	}

	#[test]
	fn central_seas_are_enclosed_by_land() {
		let mut q = params(Generator::CentralSeas, 4);
		q.rivers.count = 0; // rivers would reach the edge; isolate the seas
		let (p, stats) = make(q);
		assert!(stats.water > 0, "no sea carved");
		let (bw, _) = border_water(&p);
		assert_eq!(bw, 0, "the seas are enclosed - no water on the border");
	}

	#[test]
	fn islands_are_disconnected() {
		let mut q = params(Generator::Islands, 5);
		q.main_islands.count = 3;
		q.small_islands.count = 6;
		let mut proj = green();
		let (w, h) = (proj.width as usize, proj.height as usize);
		let (count, separated) = land_components(&shape_land(&mut proj, q), w, h);
		assert!(count >= 2, "expected several islands, got {count}");
		assert!(separated, "islands must not touch");
	}

	#[test]
	fn symmetric_islands_stay_disconnected() {
		// The fold moat keeps the mirrored halves apart in the generated shape.
		for sym in Symmetry::ALL.into_iter().filter(|&s| s != Symmetry::None) {
			let mut q = params(Generator::Islands, 8);
			q.main_islands.count = 3;
			q.small_islands.count = 4;
			q.symmetry = sym;
			let mut proj = green();
			let (w, h) = (proj.width as usize, proj.height as usize);
			let (_, separated) = land_components(&shape_land(&mut proj, q), w, h);
			assert!(separated, "{sym:?}: mirrored islands touched");
		}
	}

	#[test]
	fn land_is_all_land_without_water_features() {
		// Land with no rivers or lakes is solid; turning them on carves water.
		let mut dry = params(Generator::Land, 1);
		dry.rivers.count = 0;
		dry.lakes.count = 0;
		assert_eq!(make(dry).1.water, 0);

		let mut wet = params(Generator::Land, 1);
		wet.rivers = Range { count: 3, min: 1, max: 3 };
		wet.lakes = Range { count: 3, min: 3, max: 6 };
		assert!(make(wet).1.water > 0, "Land's rivers + lakes carve water");
	}

	#[test]
	fn paths_mode_clears_corridors_through_obstructions() {
		// `Paths` carves the obstruction field, so it covers fewer cells than the
		// same dense `Random` run.
		let dense = || {
			let mut q = params(Generator::Land, 7);
			q.rivers.count = 0;
			q.lakes.count = 0;
			q.obstructions = Range { count: 6, min: 4, max: 8 };
			q.accessibility = 15; // dense
			q
		};
		let random = make(dense()).1.obstructions;
		let mut pq = dense();
		pq.accessibility_mode = AccessibilityMode::Paths;
		let paths = make(pq).1.obstructions;
		let mut lq = dense();
		lq.accessibility_mode = AccessibilityMode::Labyrinth;
		let maze = make(lq).1.obstructions;
		assert!(random > 0, "no obstructions placed");
		assert!(paths < random, "paths should clear corridors ({paths} vs {random})");
		// The maze weaves corridors across the whole map, so it clears the most.
		assert!(maze < random, "labyrinth should clear corridors ({maze} vs {random})");
		assert!(maze < paths, "labyrinth clears more than a few paths ({maze} vs {paths})");
	}

	#[test]
	fn rivers_enter_at_oblique_angles() {
		// The old fixed top/left headings made rivers axis-aligned; `river_entry`
		// now aims them across the map at arbitrary angles, so most cross
		// obliquely (both heading components clearly non-zero).
		let mut rng = Rng::new(0x12345);
		let mut oblique = 0;
		for _ in 0..400 {
			let (_, _, (dx, dy)) = river_entry(64, 64, &mut rng);
			assert!((dx * dx + dy * dy - 1.0).abs() < 1e-3, "heading is a unit vector");
			if dx.abs() > 0.25 && dy.abs() > 0.25 {
				oblique += 1;
			}
		}
		assert!(oblique > 200, "rivers should mostly cross obliquely ({oblique}/400)");
	}

	#[test]
	fn island_sizes_are_radii_in_cells() {
		// A single island of radius ~10 spans roughly 2*r cells, not a % of area.
		let mut q = params(Generator::Islands, 4);
		q.main_islands = Range { count: 1, min: 10, max: 10 };
		q.small_islands.count = 0;
		q.rivers.count = 0;
		q.lakes.count = 0;
		let mut proj = green();
		let land = shape_land(&mut proj, q);
		let n = land.iter().filter(|&&l| l).count();
		// πr² ≈ 314 for r=10 (noise perturbs it); bounded well under a % model.
		assert!((150..650).contains(&n), "one r=10 island covered {n} cells");
	}

	#[test]
	fn rivers_and_river_raid_both_cut_land_and_differ() {
		// Both leave land dominant at a moderate count; curly vs straight rivers
		// consume the RNG differently, so the maps differ.
		let mut rv = params(Generator::Rivers, 5);
		rv.rivers = Range { count: 4, min: 1, max: 3 };
		let mut rr = params(Generator::RiverRaid, 5);
		rr.rivers = rv.rivers;
		let (_, a) = make(rv);
		let (b_proj, b) = make(rr);
		assert!(a.water > 0 && b.water > 0, "no rivers carved");
		assert!(a.water < 64 * 64 / 2 && b.water < 64 * 64 / 2, "rivers should leave land dominant");
		let (_, ra) = make(rv);
		assert!(a.water == ra.water, "same params reproduce");
		assert_ne!(a.water, b.water, "curly Rivers differ from straight River Raid");
		let _ = b_proj;
	}

	#[test]
	fn drop_zones_keep_obstructions_out() {
		// A big drop zone reserves flat land, so a dense obstruction run places
		// fewer cells than with no drop zone. Rivers off so a fully-land zone fits.
		let base = || {
			let mut q = params(Generator::Land, 7);
			q.rivers.count = 0;
			q.lakes.count = 0;
			q.obstructions = Range { count: 6, min: 4, max: 8 };
			q.accessibility = 20; // dense
			q
		};
		let (_, no_dz) = make(base());
		let mut with_dz = base();
		with_dz.drop_zones = Range { count: 1, min: 22, max: 22 };
		let (_, dz) = make(with_dz);
		assert!(no_dz.obstructions > 0, "no obstructions placed at all");
		assert!(dz.obstructions < no_dz.obstructions, "a drop zone should keep obstructions out ({} vs {})", dz.obstructions, no_dz.obstructions);
	}

	#[test]
	fn drop_zones_are_inset_and_fully_land() {
		// Islands seed 7 (lots of open water) - the drop zones overwrite it with
		// flat land, and their centres stay off the very edge.
		let mut q = params(Generator::Islands, 7);
		q.drop_zones = Range { count: 3, min: 5, max: 7 };
		let proj = green();
		let feats = green_templates();
		let mut s = GenSession::new(&proj, q, &feats).unwrap();
		s.build_shape(); // places + carves the drop zones into s.mask
		let zones = s.drop_zones.clone();
		assert!(!zones.is_empty(), "no drop zones placed");
		let (w, h) = (s.w as i32, s.h as i32);
		for &(cx, cy, r) in &zones {
			assert!(cx - r >= 2 && cy - r >= 2 && cx + r < w - 1 && cy + r < h - 1, "drop zone ({cx},{cy},{r}) not inset from the edge");
			for dy in -r..=r {
				for dx in -r..=r {
					if dx * dx + dy * dy > r * r {
						continue;
					}
					let (x, y) = (cx + dx, cy + dy);
					assert!(!s.mask[y as usize * s.w + x as usize], "drop zone disc cell ({x},{y}) is water, not solid land");
				}
			}
		}
	}

	#[test]
	fn obstructions_never_overlap_keepclear_corridors() {
		// In Paths / Labyrinth mode the corridors are planned keep-clear BEFORE
		// stamping, so no obstruction tile is ever placed on one - templates land
		// whole and are never partially erased.
		for mode in [AccessibilityMode::Paths, AccessibilityMode::Labyrinth] {
			let mut q = params(Generator::Land, 5);
			q.rivers.count = 0;
			q.lakes.count = 0;
			q.obstructions = Range { count: 8, min: 4, max: 8 };
			q.accessibility = 40;
			q.accessibility_mode = mode;
			let mut proj = green();
			let feats = green_templates();
			let mut s = GenSession::new(&proj, q, &feats).unwrap();
			s.step(&mut proj); // Shape + begin_stamps (plans keepclear)
			s.step(&mut proj); // stamp obstructions (decorations not yet placed)
			assert!(s.keepclear.iter().any(|&k| k), "no corridors planned for {mode:?}");
			for i in 0..s.w * s.h {
				assert!(!(s.keepclear[i] && s.overlay[i].is_some()), "obstruction placed on a keep-clear corridor ({mode:?})");
			}
		}
	}

	#[test]
	fn templates_classify_into_obstructions_and_decorations() {
		let p = green();
		let feats = green_templates();
		assert!(feats.iter().any(|t| classify_template(t, &p) == Some(TileKind::Obstruction)), "no obstruction templates");
		assert!(feats.iter().any(|t| classify_template(t, &p) == Some(TileKind::Land)), "no decoration templates");
	}

	#[test]
	fn obstructions_are_template_tiles() {
		let mut q = params(Generator::Continents, 3);
		q.obstructions = Range { count: 5, min: 5, max: 10 };
		q.accessibility = 30;
		let (p, stats) = make(q);
		assert!(stats.obstructions > 0, "no obstructions placed");
		// Every placed obstruction-kind tile comes from an obstruction template.
		let pack = &p.packs[1]; // 0 = WATER, 1 = GREEN
		let mut from_templates: std::collections::HashSet<u16> = std::collections::HashSet::new();
		for t in green_templates().iter().filter(|t| classify_template(t, &p) == Some(TileKind::Obstruction)) {
			for (_, _, tile) in ground_footprint(t, &p) {
				from_templates.insert(tile.tile);
			}
		}
		for y in 0..p.height {
			for x in 0..p.width {
				let Some(t) = p.cell(x, y).unwrap()[crate::LAYER_GROUND] else { continue };
				if t.pack == 1
					&& pack.props.get(family_of(&pack.ids[t.tile as usize])).and_then(|fp| fp.kind) == Some(TileKind::Obstruction)
				{
					assert!(from_templates.contains(&t.tile), "obstruction tile {} not from a template", t.tile);
				}
			}
		}
	}

	#[test]
	fn decorations_place_no_obstruction_tiles() {
		let mut q = params(Generator::Continents, 4);
		q.decorations = Range { count: 5, min: 5, max: 10 };
		let (p, stats) = make(q);
		assert!(stats.decorations > 0, "no decorations placed");
		assert_eq!(stats.obstructions, 0);
		let pack = &p.packs[1];
		for y in 0..p.height {
			for x in 0..p.width {
				let Some(t) = p.cell(x, y).unwrap()[crate::LAYER_GROUND] else { continue };
				if t.pack != 1 {
					continue;
				}
				let kind = pack.props.get(family_of(&pack.ids[t.tile as usize])).and_then(|fp| fp.kind);
				assert_ne!(kind, Some(TileKind::Obstruction), "obstruction tile placed with no obstruction patches");
			}
		}
	}

	#[test]
	fn symmetrize_mask_is_exact_and_idempotent() {
		// A deterministic pseudo-random mask on a square and a rectangle.
		for (w, h) in [(16usize, 16usize), (20, 12)] {
			let mut rng = Rng::new(0xABCD);
			let base: Vec<bool> = (0..w * h).map(|_| rng.below(2) == 0).collect();
			for sym in Symmetry::ALL {
				let mut m = base.clone();
				symmetrize_mask(&mut m, w, h, sym);
				// Every destination cell equals its source.
				for y in 0..h {
					for x in 0..w {
						if let Some((sx, sy)) = sym_source(sym, x, y, w, h) {
							assert_eq!(m[y * w + x], m[sy * w + sx], "{sym:?} not symmetric at ({x},{y}) on {w}x{h}");
						}
					}
				}
				// Idempotent: a second pass changes nothing.
				let mut again = m.clone();
				symmetrize_mask(&mut again, w, h, sym);
				assert_eq!(again, m, "{sym:?} not idempotent on {w}x{h}");
			}
		}
	}

	#[test]
	fn one_undo_unit() {
		let mut p = Project::new(48, 48, &["GREEN".into()], &assets_root(), 7).unwrap();
		let before = p.hash();
		let mut q = params(Generator::Islands, 42);
		q.obstructions.count = 2;
		q.decorations.count = 2;
		p.generate_terrain(&q, &green_templates()).unwrap();
		assert_ne!(p.hash(), before);
		assert!(p.undo(), "generation undoes");
		assert_eq!(p.hash(), before, "one Ctrl+Z restores the pre-gen map");
		assert!(p.redo());
	}

	#[test]
	fn shore_methods_differ_and_none_skips_shoring() {
		let feats = green_templates();
		let run = |shore| {
			let mut p = green();
			let mut q = params(Generator::Continents, 9);
			q.shore = shore;
			let s = p.generate_terrain(&q, &feats).unwrap();
			(p.hash(), s.shore)
		};
		let (sweep, sweep_shore) = run(ShoreMethod::Sweep);
		let (loopw, _) = run(ShoreMethod::LoopWalk);
		let (_, none_shore) = run(ShoreMethod::None);
		assert!(sweep_shore > 0, "sweep tiles the coast");
		assert_eq!(none_shore, 0, "None leaves the coast untiled");
		assert_ne!(sweep, loopw, "the loop-walk pass tiles the coast differently");
	}

	#[test]
	fn abort_rolls_the_document_back() {
		let mut p = Project::new(48, 48, &["GREEN".into()], &assets_root(), 7).unwrap();
		let before = p.hash();
		let q = params(Generator::Islands, 42);
		// Abort before anything mutates: a pure no-op.
		let mut s = GenSession::new(&p, q, &[]).unwrap();
		assert!(!s.step(&mut p));
		s.abort(&mut p);
		assert_eq!(p.hash(), before);
		assert!(!p.dirty());
		// Step past Apply (the doc is mutated mid-run), then abort.
		let mut s = GenSession::new(&p, q, &[]).unwrap();
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
		let feats = green_templates();
		let mut q = params(Generator::Continents, 6);
		q.obstructions.count = 4;
		q.decorations.count = 4;
		let mut stepped = green();
		let mut s = GenSession::new(&stepped, q, &feats).unwrap();
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
		let mut sync = green();
		sync.generate_terrain(&q, &feats).unwrap();
		assert_eq!(stepped.hash(), sync.hash(), "stepped and sync runs are the same map");
	}

	#[test]
	fn generator_symmetry_and_shore_names_round_trip() {
		for g in Generator::ALL {
			assert_eq!(Generator::parse(g.name()).unwrap(), g);
		}
		assert!(Generator::parse("volcano").is_err());
		for sym in Symmetry::ALL {
			assert_eq!(Symmetry::parse(sym.name()).unwrap(), sym);
		}
		assert!(Symmetry::parse("kaleidoscope").is_err());
		for sh in ShoreMethod::ALL {
			assert_eq!(ShoreMethod::parse(sh.name()).unwrap(), sh);
		}
		assert!(ShoreMethod::parse("erode").is_err());
	}

	#[test]
	fn noise_primitives_are_pinned() {
		// Golden vectors. The worldgen noise is float math, so a toolchain or
		// optimization change that perturbs it would silently re-roll every map
		// (the end-to-end `hash` tests would change too, but wouldn't say why).
		assert_eq!(splitmix(0), 0, "splitmix finalizer of 0 is 0");
		assert_eq!(splitmix(0x1234_5678), 11071400828549884513);
		assert_eq!(value_noise(42, 1.5, 2.5), 0.7627137);
		assert_eq!(fbm(42, 1.5, 2.5), 0.5670068);
		assert_eq!(rotated((1.0, 0.0), 5), (0.80901897, 0.58778495));
		// Structural invariants alongside the goldens.
		assert_eq!(rotated((1.0, 0.0), 0), (1.0, 0.0), "no turn = identity");
		assert!((0.0..1.0).contains(&value_noise(42, 1.5, 2.5)), "value noise in [0, 1)");
	}
}
