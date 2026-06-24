//! Generate Random Terrain modal: a **generator** select, a **symmetry**
//! select, a **shore** select, and a table of per-generator + common knobs laid
//! out as `label | count | min | max` rows (see `RNG-GEN.md`). Generate starts a
//! stepped [`map_core::GenSession`] the shell drives per frame - a progress bar
//! fills, the Generate button becomes Abort, the UI never freezes, and the modal
//! **stays open** so seeds can be rerolled (leave the seed field empty for fresh
//! randomness each press; every run is one undo unit).
//!
//! Pure UI state here (plus the owned session); the shell drives `step` / abort
//! through `EditorState` so it can borrow the project. The rows shown depend on
//! the generator, so the dialog height is computed from the visible row list.

use std::collections::HashMap;

use map_core::{AccessibilityMode, GenParams, GenSession, Generator as Gen, Range, ShoreMethod, Span, Symmetry};

use crate::textinput::{Charset, TextInput};
use crate::theme;
use crate::ui::{self, Hot, Rect, UiQuads};

const W: f32 = 300.0;
const TITLE_H: f32 = 22.0;
const ROW_H: f32 = 24.0;
/// Row-to-row pitch.
const ROWGAP: f32 = ROW_H + 4.0;
const BTN_H: f32 = 20.0;
const LABEL_X: f32 = 10.0;
/// The three numeric columns (count / min / max).
const COL0: f32 = 118.0;
const COL_W: f32 = 46.0;
const COL_GAP: f32 = 8.0;
/// A select / seed field spans all three columns.
const WIDE_W: f32 = COL_W * 3.0 + COL_GAP * 2.0;
const STATUS_LINE_H: f32 = 16.0;
/// The most property rows any generator shows (Islands: 6 + 4 common). The
/// dialog is sized for this so it **never resizes** when you switch generator.
const MAX_PROP_ROWS: usize = 10;
const INSET_PAD: f32 = 4.0;
const LINE_H: f32 = crate::ui::FONT_SMALL + 4.0;
/// The fixed report/status inset fits this many lines (a generate report is at
/// most three: seed, counts, shore).
const REPORT_H: f32 = 3.0 * STATUS_LINE_H + 2.0 * INSET_PAD;

/// One numeric knob group; each owns up to three column fields (count, min, max
/// - distances use only min/max, accessibility only the first as a value).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Knob {
	MainIslands,
	MainDist,
	SmallIslands,
	SmallDist,
	Continents,
	Seas,
	Rivers,
	Lakes,
	Maze,
	DropZones,
	Obstructions,
	Accessibility,
	Decorations,
	Seed,
}

const ALL_KNOBS: [Knob; 14] = [
	Knob::MainIslands,
	Knob::MainDist,
	Knob::SmallIslands,
	Knob::SmallDist,
	Knob::Continents,
	Knob::Seas,
	Knob::Rivers,
	Knob::Maze,
	Knob::Lakes,
	Knob::DropZones,
	Knob::Obstructions,
	Knob::Accessibility,
	Knob::Decorations,
	Knob::Seed,
];

/// Which select a row hosts. Generator/Symmetry/Shore are the three top rows;
/// Access is the mode dropdown inline on the accessibility row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sel {
	Generator,
	Symmetry,
	Shore,
	Access,
}

/// Per-generator last-used parameters, remembered for the session so reopening
/// the modal or switching generator restores what you last set (kept on
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

pub struct Generator {
	pub generator: Gen,
	pub symmetry: Symmetry,
	pub shore: ShoreMethod,
	pub accessibility_mode: AccessibilityMode,
	/// Each knob's three column fields (count, min, max); only the shown ones
	/// are read. Seed uses field 0 (wide, longer max length).
	fields: HashMap<Knob, [TextInput; 3]>,
	/// Stashed parameters for the generators not currently shown (session memory).
	memory: HashMap<Gen, GenParams>,
	focus: Option<(Knob, usize)>,
	generator_open: bool,
	symmetry_open: bool,
	shore_open: bool,
	access_open: bool,
	drag: Option<(Knob, usize)>,
	pub running: bool,
	pub session: Option<GenSession>,
	pub started: Option<GenParams>,
	pub status: Vec<String>,
	armed: Option<ArmedBtn>,
	pub(crate) drag_offset: (f32, f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArmedBtn {
	Close,
	Generate,
	Surprise,
}

/// What a press resolved to (everything is consumed while a modal is open).
#[derive(Debug, PartialEq)]
pub enum Press {
	Consumed,
	Close,
	Start,
	Abort,
	/// Surprise Me pressed - the shell rolls it (it needs the map size).
	Surprise,
	Invalid(String),
}

/// The property rows (knob, label, shown column indices) for `generator`,
/// followed by the common rows. Selects + seed are laid out separately.
fn prop_rows(generator: Gen) -> Vec<(Knob, &'static str, &'static [usize])> {
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
	v.push((Knob::DropZones, "drop zones", &[0, 1, 2]));
	v.push((Knob::Obstructions, "obstructions", &[0, 1, 2]));
	v.push((Knob::Accessibility, "accessibility", &[0]));
	v.push((Knob::Decorations, "decorations", &[0, 1, 2]));
	v
}

/// A one-line hint for a knob row (shown in the hint box on hover). Sizes are
/// radii in tiles; distances are the cell gap; river width is tiles across.
fn knob_hint(k: Knob) -> &'static str {
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
		Knob::DropZones => "Flat obstruction-free start areas: count + radius",
		Knob::Obstructions => "Impassable feature patches: count + radius (tiles)",
		Knob::Accessibility => "Obstruction density % (paths/labyrinth: road count + width)",
		Knob::Decorations => "Passable decoration patches: count + radius (tiles)",
		Knob::Seed => "Reproducible seed - empty rolls a fresh map each press",
	}
}

/// The hint for one of the four selects (also fed into the hint-box sizing).
fn select_hint(sel: Sel) -> &'static str {
	match sel {
		Sel::Generator => "Overall land / water layout",
		Sel::Symmetry => "Mirror the map for fair-play layouts",
		Sel::Shore => "How land / water coastlines are tiled",
		Sel::Access => "Obstruction layout: random scatter, roads (paths), or a maze (labyrinth)",
	}
}

const SEED_HINT: &str = "Reproducible seed - empty rolls a fresh map each press";
const SURPRISE_HINT: &str = "Surprise Me! - random values for every property, plus a fresh seed";

/// The Surprise Me roll ranges for one knob of a generator: `(count[lo,hi],
/// min[lo,hi], max[lo,hi])` - a value is rolled in each range (`max` lifted to
/// ≥ the rolled `min`). Tuned per generator so a roll stays **balanced and
/// sensible** while showing off that generator's character (sizes in cells: a
/// radius, a river width, or a gap). Continents / Central Seas are sized by map
/// coverage in `surprise` instead; Accessibility / Seed are rolled separately.
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
		Knob::Accessibility | Knob::Seed => ([0, 0], [0, 0], [0, 0]), // rolled separately
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

impl Generator {
	pub fn new() -> Self {
		let digits = |s: &str, max| TextInput::new(s, max).charset(Charset::Digits);
		let d = GenParams::defaults(Gen::Islands);
		// [count, min, max] for a Range; distances put min/max in cols 1/2.
		let r =
			|g: Range| [digits(&g.count.to_string(), 3), digits(&g.min.to_string(), 3), digits(&g.max.to_string(), 3)];
		let s = |sp: Span| [digits("0", 3), digits(&sp.min.to_string(), 3), digits(&sp.max.to_string(), 3)];
		let one = |v: u8| [digits(&v.to_string(), 3), digits("0", 3), digits("0", 3)];
		let mut fields = HashMap::new();
		fields.insert(Knob::MainIslands, r(d.main_islands));
		fields.insert(Knob::MainDist, s(d.main_dist));
		fields.insert(Knob::SmallIslands, r(d.small_islands));
		fields.insert(Knob::SmallDist, s(d.small_dist));
		fields.insert(Knob::Continents, r(d.continents));
		fields.insert(Knob::Seas, r(d.seas));
		fields.insert(Knob::Rivers, r(d.rivers));
		fields.insert(Knob::Lakes, r(d.lakes));
		fields.insert(Knob::Maze, r(d.maze));
		fields.insert(Knob::DropZones, r(d.drop_zones));
		fields.insert(Knob::Obstructions, r(d.obstructions));
		fields.insert(Knob::Accessibility, one(d.accessibility));
		fields.insert(Knob::Decorations, r(d.decorations));
		fields.insert(Knob::Seed, [digits("", 20), digits("0", 3), digits("0", 3)]);
		Self {
			generator: Gen::Islands,
			symmetry: Symmetry::None,
			shore: ShoreMethod::Sweep,
			accessibility_mode: d.accessibility_mode,
			fields,
			memory: HashMap::new(),
			focus: None,
			generator_open: false,
			symmetry_open: false,
			shore_open: false,
			access_open: false,
			drag: None,
			running: false,
			session: None,
			started: None,
			status: Vec::new(),
			armed: None,
			drag_offset: (0.0, 0.0),
		}
	}

	fn cell(&self, k: Knob, col: usize) -> &TextInput {
		&self.fields[&k][col]
	}

	fn cell_mut(&mut self, k: Knob, col: usize) -> &mut TextInput {
		&mut self.fields.get_mut(&k).expect("knob present")[col]
	}

	/// The validated settings (`None` seed = the caller rolls a fresh one), or
	/// what's wrong with them. `max` is kept ≥ `min`; the algorithm clamps the
	/// rest, so only non-numeric / empty fields are rejected.
	pub fn params(&self) -> Result<(GenParams, Option<u64>), String> {
		let u = |k: Knob, col: usize, what: &str| -> Result<u8, String> {
			self.cell(k, col).text().parse::<u8>().map_err(|_| format!("{what} is not a number"))
		};
		let range = |k: Knob, what: &str| -> Result<Range, String> {
			let (count, min, max) = (u(k, 0, what)?, u(k, 1, what)?, u(k, 2, what)?);
			Ok(Range { count, min, max: max.max(min) })
		};
		let span = |k: Knob, what: &str| -> Result<Span, String> {
			let (min, max) = (u(k, 1, what)?, u(k, 2, what)?);
			Ok(Span { min, max: max.max(min) })
		};
		let seed = match self.cell(Knob::Seed, 0).text().is_empty() {
			true => None,
			false => Some(
				self.cell(Knob::Seed, 0).text().parse::<u64>().map_err(|_| "seed is not a number (u64)".to_string())?,
			),
		};
		let params = GenParams {
			generator: self.generator,
			seed: seed.unwrap_or(0),
			symmetry: self.symmetry,
			shore: self.shore,
			main_islands: range(Knob::MainIslands, "main islands")?,
			main_dist: span(Knob::MainDist, "main distance")?,
			small_islands: range(Knob::SmallIslands, "small islands")?,
			small_dist: span(Knob::SmallDist, "small distance")?,
			continents: range(Knob::Continents, "continents")?,
			seas: range(Knob::Seas, "seas")?,
			rivers: range(Knob::Rivers, "rivers")?,
			lakes: range(Knob::Lakes, "lakes")?,
			maze: range(Knob::Maze, "maze")?,
			drop_zones: range(Knob::DropZones, "drop zones")?,
			obstructions: range(Knob::Obstructions, "obstructions")?,
			accessibility: u(Knob::Accessibility, 0, "accessibility")?,
			accessibility_mode: self.accessibility_mode,
			decorations: range(Knob::Decorations, "decorations")?,
		};
		Ok((params, seed))
	}

	// ----- session memory + surprise -----------------------------------------

	/// Open with the session's remembered parameters (last generator + each
	/// generator's last-used knobs).
	pub fn from_memory(mem: &GenMemory) -> Self {
		let mut s = Self::new();
		s.memory = mem.params.clone();
		s.generator = mem.last;
		let p = s.memory.get(&s.generator).copied().unwrap_or_else(|| GenParams::defaults(s.generator));
		s.load_params(&p);
		s
	}

	/// The session memory to stash on close: every stashed generator plus the
	/// one on screen now.
	pub fn to_memory(&self) -> GenMemory {
		let mut params = self.memory.clone();
		params.insert(self.generator, self.snapshot());
		GenMemory { last: self.generator, params }
	}

	/// Current fields as a `GenParams` (best effort - invalid fields fall back to
	/// the generator's defaults).
	fn snapshot(&self) -> GenParams {
		self.params().map(|(p, _)| p).unwrap_or_else(|_| GenParams::defaults(self.generator))
	}

	fn put_range(&mut self, k: Knob, g: Range) {
		self.cell_mut(k, 0).set_text(&g.count.to_string());
		self.cell_mut(k, 1).set_text(&g.min.to_string());
		self.cell_mut(k, 2).set_text(&g.max.to_string());
	}

	fn put_span(&mut self, k: Knob, sp: Span) {
		self.cell_mut(k, 1).set_text(&sp.min.to_string());
		self.cell_mut(k, 2).set_text(&sp.max.to_string());
	}

	/// Load every field from `p` (the inverse of `params`; the seed is left as
	/// typed so it keeps rolling fresh).
	fn load_params(&mut self, p: &GenParams) {
		self.symmetry = p.symmetry;
		self.shore = p.shore;
		self.accessibility_mode = p.accessibility_mode;
		self.put_range(Knob::MainIslands, p.main_islands);
		self.put_span(Knob::MainDist, p.main_dist);
		self.put_range(Knob::SmallIslands, p.small_islands);
		self.put_span(Knob::SmallDist, p.small_dist);
		self.put_range(Knob::Continents, p.continents);
		self.put_range(Knob::Seas, p.seas);
		self.put_range(Knob::Rivers, p.rivers);
		self.put_range(Knob::Lakes, p.lakes);
		self.put_range(Knob::Maze, p.maze);
		self.put_range(Knob::DropZones, p.drop_zones);
		self.put_range(Knob::Obstructions, p.obstructions);
		self.cell_mut(Knob::Accessibility, 0).set_text(&p.accessibility.to_string());
		self.put_range(Knob::Decorations, p.decorations);
	}

	/// Fill the current generator's shown properties + the three side selects
	/// with random but sensible values (the Surprise Me button).
	pub fn surprise(&mut self, map_w: usize, map_h: usize) {
		let mut rng = SurpriseRng::seeded();
		// Now and then (~1 in 3) roll a dense obstruction field across the full
		// accessibility range, to show off heavy obstructions + the paths /
		// labyrinth carving; otherwise keep a balanced, sensible feature count.
		let heavy = rng.range(0, 2) == 0;
		for (k, _, cols) in prop_rows(self.generator) {
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
				self.cell_mut(k, 0).set_text(&count.to_string());
				self.cell_mut(k, 1).set_text(&r.to_string());
				self.cell_mut(k, 2).set_text(&r.to_string());
				continue;
			}
			if k == Knob::Accessibility {
				let v = if heavy { rng.range(0, 100) } else { rng.range(30, 85) };
				self.cell_mut(k, 0).set_text(&v.to_string());
				continue;
			}
			let (cnt, mn, mx) = if heavy && k == Knob::Obstructions {
				([12, 40], [4, 9], [8, 15]) // a wall of obstructions to carve through
			} else {
				surprise_spec(self.generator, k)
			};
			let count = rng.range(cnt[0], cnt[1]);
			let min = rng.range(mn[0], mn[1]);
			let max = rng.range(mx[0], mx[1]).max(min);
			for &c in cols {
				let v = match c {
					0 => count,
					1 => min,
					_ => max,
				};
				self.cell_mut(k, c).set_text(&v.to_string());
			}
		}
		// Symmetry and shore are left as set - only the properties roll.
		self.accessibility_mode = AccessibilityMode::ALL[rng.range(0, AccessibilityMode::ALL.len() as u8 - 1) as usize];
		// Pre-fill a fresh random seed so the surprise is reproducible.
		self.cell_mut(Knob::Seed, 0).set_text(&rng.next().to_string());
	}

	// ----- geometry ----------------------------------------------------------

	/// Row 0: Surprise Me. Rows 1-3: generator / symmetry / shore selects. Row 4:
	/// column headers. Rows 5..: property rows, then the seed row; the hint +
	/// report insets and buttons are anchored to the bottom.
	fn n_prop_rows(&self) -> usize {
		prop_rows(self.generator).len()
	}

	/// The multiline hint box height, sized for the *most spacious* hint (every
	/// knob / select hint wrapped to the inset width) so the box never resizes.
	fn hint_h() -> f32 {
		let inner = W - 2.0 * LABEL_X - 2.0 * INSET_PAD;
		let selects = [Sel::Generator, Sel::Symmetry, Sel::Shore, Sel::Access];
		let lines = ALL_KNOBS
			.iter()
			.map(|&k| knob_hint(k))
			.chain(selects.into_iter().map(select_hint))
			.chain([SEED_HINT, SURPRISE_HINT])
			.map(|s| crate::text::wrap_lines(s, crate::ui::FONT_SMALL, inner).len())
			.max()
			.unwrap_or(1)
			.max(1);
		lines as f32 * LINE_H + 2.0 * INSET_PAD
	}

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		// Fixed size: the top section sized for the worst case (max prop rows +
		// the seed), plus the fixed multiline hint and report insets - so the
		// window never resizes when the generator (or status) changes.
		let top = TITLE_H + 8.0 + (Self::SEED_ROW_MAX + 1) as f32 * ROWGAP;
		let dh = top + Self::hint_h() + 4.0 + REPORT_H + 8.0 + BTN_H + 10.0;
		Rect::centered(w, h, W, dh).translate(self.drag_offset.0, self.drag_offset.1)
	}

	/// A one-line description of the control under `cursor`, for the hint box.
	fn hint_at(&self, d: Rect, cursor: (f32, f32)) -> Option<&'static str> {
		let (x, y) = cursor;
		if self.surprise_rect(d).contains(x, y) {
			return Some(SURPRISE_HINT);
		}
		for (sel, row) in Self::SELECT_ROW {
			if Self::select_rect(d, row).contains(x, y) {
				return Some(select_hint(sel));
			}
		}
		if self.access_select_rect(d).is_some_and(|r| r.contains(x, y)) {
			return Some(select_hint(Sel::Access));
		}
		for (k, _, _) in prop_rows(self.generator) {
			let row = self.prop_row(k)?;
			let rr = Rect::new(d.x + LABEL_X, Self::row_y(d, row), Self::col_x(d, 2) + COL_W - (d.x + LABEL_X), BTN_H);
			if rr.contains(x, y) {
				return Some(knob_hint(k));
			}
		}
		let sr = Rect::new(d.x + LABEL_X, Self::row_y(d, self.seed_row()), COL0 - LABEL_X + WIDE_W, BTN_H);
		sr.contains(x, y).then_some(SEED_HINT)
	}

	fn row_y(d: Rect, row: usize) -> f32 {
		d.y + TITLE_H + 8.0 + row as f32 * ROWGAP
	}

	const SURPRISE_ROW: usize = 0;
	const SELECT_ROW: [(Sel, usize); 3] = [(Sel::Generator, 1), (Sel::Symmetry, 2), (Sel::Shore, 3)];
	const HEADER_ROW: usize = 4;
	const PROP_ROW0: usize = 5;
	/// The seed row in the worst case (max prop rows) - sets the fixed top height.
	const SEED_ROW_MAX: usize = Self::PROP_ROW0 + MAX_PROP_ROWS;

	/// The Surprise Me button, spanning the inner width at the very top.
	fn surprise_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + LABEL_X, Self::row_y(d, Self::SURPRISE_ROW), d.w - 2.0 * LABEL_X, BTN_H)
	}

	/// The report / status inset, anchored just above the buttons.
	fn report_rect(d: Rect) -> Rect {
		let y = d.y + d.h - BTN_H - 10.0 - 8.0 - REPORT_H;
		Rect::new(d.x + LABEL_X, y, d.w - 2.0 * LABEL_X, REPORT_H)
	}

	/// The multiline hint inset, anchored just above the report inset.
	fn hint_rect(d: Rect) -> Rect {
		let hh = Self::hint_h();
		Rect::new(d.x + LABEL_X, Self::report_rect(d).y - 4.0 - hh, d.w - 2.0 * LABEL_X, hh)
	}

	fn select_rect(d: Rect, row: usize) -> Rect {
		Rect::new(d.x + COL0, Self::row_y(d, row), WIDE_W, BTN_H)
	}

	fn col_x(d: Rect, col: usize) -> f32 {
		d.x + COL0 + col as f32 * (COL_W + COL_GAP)
	}

	/// The row index of a property knob, if shown for the current generator.
	fn prop_row(&self, k: Knob) -> Option<usize> {
		prop_rows(self.generator).iter().position(|&(g, _, _)| g == k).map(|i| Self::PROP_ROW0 + i)
	}

	fn seed_row(&self) -> usize {
		Self::PROP_ROW0 + self.n_prop_rows()
	}

	fn field_rect(&self, d: Rect, k: Knob, col: usize) -> Option<Rect> {
		if k == Knob::Seed {
			return Some(Rect::new(d.x + COL0, Self::row_y(d, self.seed_row()), WIDE_W, BTN_H));
		}
		let row = self.prop_row(k)?;
		Some(Rect::new(Self::col_x(d, col), Self::row_y(d, row), COL_W, BTN_H))
	}

	fn close_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + 10.0, d.y + d.h - BTN_H - 10.0, 90.0, BTN_H)
	}

	fn generate_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + d.w - 110.0, d.y + d.h - BTN_H - 10.0, 100.0, BTN_H)
	}

	fn copy_seed_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + d.w / 2.0 - 50.0, d.y + d.h - BTN_H - 10.0, 100.0, BTN_H)
	}

	fn reported_seed(&self) -> Option<u64> {
		self.started.as_ref().map(|p| p.seed)
	}

	/// The visible numeric fields in row order, then the seed - for Tab + drawing.
	fn visible_fields(&self) -> Vec<(Knob, usize)> {
		let mut v: Vec<(Knob, usize)> = Vec::new();
		for (k, _, cols) in prop_rows(self.generator) {
			for &c in cols {
				v.push((k, c));
			}
		}
		v.push((Knob::Seed, 0));
		v
	}

	// ----- selects -----------------------------------------------------------

	fn select_state(&self, sel: Sel) -> (bool, usize, usize) {
		match sel {
			Sel::Generator => {
				(self.generator_open, Gen::ALL.len(), Gen::ALL.iter().position(|&g| g == self.generator).unwrap_or(0))
			}
			Sel::Symmetry => (
				self.symmetry_open,
				Symmetry::ALL.len(),
				Symmetry::ALL.iter().position(|&s| s == self.symmetry).unwrap_or(0),
			),
			Sel::Shore => (
				self.shore_open,
				ShoreMethod::ALL.len(),
				ShoreMethod::ALL.iter().position(|&s| s == self.shore).unwrap_or(0),
			),
			Sel::Access => (
				self.access_open,
				AccessibilityMode::ALL.len(),
				AccessibilityMode::ALL.iter().position(|&m| m == self.accessibility_mode).unwrap_or(0),
			),
		}
	}

	fn select_label(&self, sel: Sel) -> &'static str {
		match sel {
			Sel::Generator => self.generator.label(),
			Sel::Symmetry => self.symmetry.label(),
			Sel::Shore => self.shore.label(),
			Sel::Access => self.accessibility_mode.label(),
		}
	}

	fn select_labels(sel: Sel) -> Vec<&'static str> {
		match sel {
			Sel::Generator => Gen::ALL.iter().map(|g| g.label()).collect(),
			Sel::Symmetry => Symmetry::ALL.iter().map(|s| s.label()).collect(),
			Sel::Shore => ShoreMethod::ALL.iter().map(|s| s.label()).collect(),
			Sel::Access => AccessibilityMode::ALL.iter().map(|m| m.label()).collect(),
		}
	}

	fn any_select_open(&self) -> bool {
		self.generator_open || self.symmetry_open || self.shore_open || self.access_open
	}

	fn close_selects(&mut self) {
		self.generator_open = false;
		self.symmetry_open = false;
		self.shore_open = false;
		self.access_open = false;
	}

	fn toggle_select(&mut self, sel: Sel) {
		let was = match sel {
			Sel::Generator => self.generator_open,
			Sel::Symmetry => self.symmetry_open,
			Sel::Shore => self.shore_open,
			Sel::Access => self.access_open,
		};
		self.close_selects();
		if !was {
			match sel {
				Sel::Generator => self.generator_open = true,
				Sel::Symmetry => self.symmetry_open = true,
				Sel::Shore => self.shore_open = true,
				Sel::Access => self.access_open = true,
			}
		}
	}

	fn choose_select(&mut self, sel: Sel, idx: usize) {
		match sel {
			Sel::Generator => {
				let next = Gen::ALL[idx.min(Gen::ALL.len() - 1)];
				if next != self.generator {
					// Stash the current generator's settings, restore the new one's
					// (session memory: each generator remembers what you last set).
					let snap = self.snapshot();
					self.memory.insert(self.generator, snap);
					self.generator = next;
					let p = self.memory.get(&next).copied().unwrap_or_else(|| GenParams::defaults(next));
					self.load_params(&p);
					self.focus = None; // the visible rows just changed
				}
			}
			Sel::Symmetry => self.symmetry = Symmetry::ALL[idx.min(Symmetry::ALL.len() - 1)],
			Sel::Shore => self.shore = ShoreMethod::ALL[idx.min(ShoreMethod::ALL.len() - 1)],
			Sel::Access => self.accessibility_mode = AccessibilityMode::ALL[idx.min(AccessibilityMode::ALL.len() - 1)],
		}
		self.close_selects();
	}

	/// The accessibility-mode dropdown box (cols 1-2 of the accessibility row).
	fn access_select_rect(&self, d: Rect) -> Option<Rect> {
		let row = self.prop_row(Knob::Accessibility)?;
		Some(Rect::new(Self::col_x(d, 1), Self::row_y(d, row), COL_W * 2.0 + COL_GAP, BTN_H))
	}

	fn press_selects(&mut self, d: Rect, x: f32, y: f32) -> bool {
		for (sel, row) in Self::SELECT_ROW {
			let (open, n, _) = self.select_state(sel);
			if let Some(hit) = crate::select::hit(Self::select_rect(d, row), open, n, false, x, y) {
				match hit {
					crate::select::Hit::Box => self.toggle_select(sel),
					crate::select::Hit::Option(i) => self.choose_select(sel, i),
				}
				return true;
			}
		}
		if let Some(r) = self.access_select_rect(d) {
			let (open, n, _) = self.select_state(Sel::Access);
			if let Some(hit) = crate::select::hit(r, open, n, false, x, y) {
				match hit {
					crate::select::Hit::Box => self.toggle_select(Sel::Access),
					crate::select::Hit::Option(i) => self.choose_select(Sel::Access, i),
				}
				return true;
			}
		}
		if self.any_select_open() {
			self.close_selects();
			return true;
		}
		false
	}

	// ----- events ------------------------------------------------------------

	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		if self.running {
			if self.generate_rect(d).contains(x, y) {
				self.armed = Some(ArmedBtn::Generate);
			}
			return Press::Consumed;
		}
		if self.press_selects(d, x, y) {
			return Press::Consumed;
		}
		for (k, col) in self.visible_fields() {
			let Some(r) = self.field_rect(d, k, col) else { continue };
			if r.contains(x, y) {
				self.focus = Some((k, col));
				self.cell_mut(k, col).on_press(x, y, r);
				self.drag = Some((k, col));
				return Press::Consumed;
			}
		}
		if let Some(seed) = self.reported_seed() {
			if self.copy_seed_rect(d).contains(x, y) {
				crate::textinput::clipboard_set(&seed.to_string());
				return Press::Consumed;
			}
		}
		if self.surprise_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Surprise);
			return Press::Consumed;
		}
		if self.close_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Close);
			return Press::Consumed;
		}
		if self.generate_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Generate);
			return Press::Consumed;
		}
		self.focus = None;
		Press::Consumed
	}

	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		self.drag = None;
		let d = self.dialog_rect(w, h);
		match self.armed.take() {
			Some(ArmedBtn::Close) if self.close_rect(d).contains(x, y) && !self.running => Press::Close,
			Some(ArmedBtn::Surprise) if self.surprise_rect(d).contains(x, y) && !self.running => Press::Surprise,
			Some(ArmedBtn::Generate) if self.generate_rect(d).contains(x, y) => {
				if self.running {
					Press::Abort
				} else {
					match self.params() {
						Ok(_) => Press::Start,
						Err(e) => Press::Invalid(e),
					}
				}
			}
			_ => Press::Consumed,
		}
	}

	pub fn on_drag(&mut self, x: f32, y: f32, w: f32, h: f32) {
		if let Some((k, col)) = self.drag {
			if let Some(r) = self.field_rect(self.dialog_rect(w, h), k, col) {
				self.cell_mut(k, col).on_drag(x, y, r);
			}
		}
	}

	pub fn edit_context(&self) -> Option<crate::modal::EditContext> {
		if self.running {
			return None;
		}
		let (k, col) = self.focus?;
		Some(self.cell(k, col).edit_context())
	}

	pub fn key(&mut self, key: &crate::modal::ModalKey) {
		if self.running {
			return;
		}
		let Some((k, col)) = self.focus else { return };
		self.cell_mut(k, col).on_key(key);
	}

	pub fn focus_next(&mut self) {
		let fields = self.visible_fields();
		if fields.is_empty() {
			return;
		}
		let next = match self.focus.and_then(|f| fields.iter().position(|&v| v == f)) {
			Some(i) => (i + 1) % fields.len(),
			None => 0,
		};
		self.focus = Some(fields[next]);
	}

	// ----- drawing -----------------------------------------------------------

	/// A recessed inset filled 50% darker than the dialog body (hint + report).
	fn dark_inset(q: &mut UiQuads, r: Rect, w: f32, h: f32) {
		q.inset(r, w, h, theme::PANEL, 1.0);
		q.rect(r, w, h, [0.0, 0.0, 0.0, 0.5]);
	}

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		// Non-blocking window: no scrim, so the live map shows through behind it.
		ui::modal_frame(&mut q, d, "Generate Random Terrain", TITLE_H, w, h);

		// Surprise Me button at the very top (centred label).
		let surprise = self.surprise_rect(d);
		if self.running {
			q.button_disabled(surprise, w, h);
		} else {
			q.button(surprise, w, h, hot);
		}
		let lw = crate::text::label_width("Surprise Me!", crate::ui::FONT_SMALL);
		q.label(
			"Surprise Me!",
			surprise.x + (surprise.w - lw) / 2.0,
			surprise.y + 4.0,
			crate::ui::FONT_SMALL,
			w,
			h,
			theme::INK,
		);

		// The three top selects.
		for (sel, row) in Self::SELECT_ROW {
			let label = match sel {
				Sel::Generator => "generator",
				Sel::Symmetry => "symmetry",
				Sel::Shore => "shore",
				Sel::Access => "",
			};
			q.label(label, d.x + LABEL_X, Self::row_y(d, row) + 4.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
			let (open, _, _) = self.select_state(sel);
			crate::select::draw_box(&mut q, Self::select_rect(d, row), self.select_label(sel), open, w, h, hot);
		}

		// Column headers over the numeric columns.
		let hy = Self::row_y(d, Self::HEADER_ROW) + 4.0;
		for (col, cap) in ["count", "min", "max"].iter().enumerate() {
			q.label(cap, Self::col_x(d, col), hy, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		}

		// Property rows: label + the shown column fields (the accessibility row
		// also hosts the mode dropdown across cols 1-2).
		for (k, label, cols) in prop_rows(self.generator) {
			let row = self.prop_row(k).expect("row present");
			q.label(label, d.x + LABEL_X, Self::row_y(d, row) + 4.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
			for &col in cols {
				let r = Rect::new(Self::col_x(d, col), Self::row_y(d, row), COL_W, BTN_H);
				q.field(r, w, h);
				if self.focus == Some((k, col)) {
					q.border(r, w, h, theme::INK);
				}
			}
			if k == Knob::Accessibility {
				if let Some(r) = self.access_select_rect(d) {
					crate::select::draw_box(&mut q, r, self.accessibility_mode.label(), self.access_open, w, h, hot);
				}
			}
		}

		// Seed (wide).
		let seed_r = Rect::new(d.x + COL0, Self::row_y(d, self.seed_row()), WIDE_W, BTN_H);
		q.label(
			"seed",
			d.x + LABEL_X,
			Self::row_y(d, self.seed_row()) + 4.0,
			crate::ui::FONT_SMALL,
			w,
			h,
			theme::INK_DIM,
		);
		q.field(seed_r, w, h);
		let seed_focused = self.focus == Some((Knob::Seed, 0));
		if seed_focused {
			q.border(seed_r, w, h, theme::INK);
		}
		if self.cell(Knob::Seed, 0).text().is_empty() && !seed_focused {
			q.label_in("random", seed_r, 6.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		}

		// Hint box: a fixed-size inset (50% darker) showing a wrapped, multiline
		// description of whatever the cursor is over.
		let hint_r = Self::hint_rect(d);
		Self::dark_inset(&mut q, hint_r, w, h);
		if let Some(hint) = hot.cursor.and_then(|c| self.hint_at(d, c)).filter(|s| !s.is_empty()) {
			q.label_wrapped(hint, hint_r, INSET_PAD + 2.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		}

		// Report inset (50% darker): the progress bar while running, else status.
		let report_r = Self::report_rect(d);
		Self::dark_inset(&mut q, report_r, w, h);
		if let (true, Some(session)) = (self.running, &self.session) {
			let (label, frac) = session.progress();
			q.label(label, report_r.x + 6.0, report_r.y + INSET_PAD, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
			let bar = Rect::new(
				report_r.x + COL0 - LABEL_X,
				report_r.y + INSET_PAD,
				report_r.w - (COL0 - LABEL_X) - 6.0,
				BTN_H - 4.0,
			);
			q.progress_bar(bar, frac, Some(&format!("{:.0}%", frac * 100.0)), crate::ui::FONT_SMALL, w, h);
		} else {
			for (i, line) in self.status.iter().take(3).enumerate() {
				let line = crate::text::fit_label(line, crate::ui::FONT_SMALL, report_r.w - 12.0);
				q.label(
					&line,
					report_r.x + 6.0,
					report_r.y + INSET_PAD + i as f32 * STATUS_LINE_H,
					crate::ui::FONT_SMALL,
					w,
					h,
					theme::INK_DIM,
				);
			}
		}

		if self.running {
			q.button_disabled(self.close_rect(d), w, h);
		} else {
			q.button(self.close_rect(d), w, h, hot);
		}
		q.label_in("Close", self.close_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		if !self.running && self.reported_seed().is_some() {
			let r = self.copy_seed_rect(d);
			q.button(r, w, h, hot);
			q.label_in("Copy Seed", r, 8.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		}
		q.button_primary(self.generate_rect(d), w, h, hot);
		let label = if self.running { "Abort" } else { "Generate" };
		q.label_in(label, self.generate_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK);
		q
	}

	/// The open select's popup (drawn last so it floats above the field text).
	pub fn popup(&self, w: f32, h: f32, hot: Hot) -> Option<UiQuads> {
		if self.running || !self.any_select_open() {
			return None;
		}
		let d = self.dialog_rect(w, h);
		let (sel, rect) = if self.access_open {
			(Sel::Access, self.access_select_rect(d)?)
		} else {
			let sel = if self.generator_open {
				Sel::Generator
			} else if self.symmetry_open {
				Sel::Symmetry
			} else {
				Sel::Shore
			};
			let row = Self::SELECT_ROW.iter().find(|(s, _)| *s == sel).map(|&(_, r)| r).unwrap_or(0);
			(sel, Self::select_rect(d, row))
		};
		let (_, _, idx) = self.select_state(sel);
		let labels = Self::select_labels(sel);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		crate::select::draw_popup(&mut q, rect, &labels, Some(idx), false, w, h, hot);
		Some(q)
	}

	/// Each visible field's text/caret/selection, with its clip rect.
	pub fn field_contents(&self, w: f32, h: f32) -> Vec<(UiQuads, Rect)> {
		let d = self.dialog_rect(w, h);
		self.visible_fields()
			.into_iter()
			.filter_map(|(k, col)| {
				let r = self.field_rect(d, k, col)?;
				Some((self.cell(k, col).content_quads(r, self.focus == Some((k, col)), w, h), r))
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::modal::ModalKey;

	#[test]
	fn params_build_from_defaults() {
		let m = Generator::new();
		let (p, seed) = m.params().unwrap();
		assert_eq!(p.generator, Gen::Islands);
		assert_eq!(p.symmetry, Symmetry::None);
		assert_eq!(p.shore, ShoreMethod::Sweep);
		assert_eq!(p.main_islands, GenParams::defaults(Gen::Islands).main_islands);
		assert_eq!(seed, None, "empty seed field = roll a fresh one");
	}

	#[test]
	fn ranges_keep_max_above_min_and_reject_empty() {
		let mut m = Generator::new();
		// max typed below min is lifted back to min.
		m.cell_mut(Knob::MainIslands, 1).set_text("9");
		m.cell_mut(Knob::MainIslands, 2).set_text("4");
		let p = m.params().unwrap().0;
		assert!(p.main_islands.max >= p.main_islands.min, "{} < {}", p.main_islands.max, p.main_islands.min);
		// An empty visible field is rejected.
		m.cell_mut(Knob::MainIslands, 0).set_text("");
		assert!(m.params().is_err());
	}

	#[test]
	fn generator_select_swaps_the_rows() {
		let mut m = Generator::new();
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		// Islands shows island knobs, not continents.
		assert!(m.visible_fields().iter().any(|&(k, _)| k == Knob::MainIslands));
		assert!(!m.visible_fields().iter().any(|&(k, _)| k == Knob::Continents));
		// Open the generator select, pick Continents (index 1).
		let r = Generator::select_rect(d, 1);
		assert_eq!(m.on_press(r.x + 2.0, r.y + 2.0, w, h), Press::Consumed);
		assert!(m.generator_open);
		let opt = crate::select::option_rect(r, 1, Gen::ALL.len(), false);
		m.on_press(opt.x + 2.0, opt.y + 2.0, w, h);
		assert_eq!(m.generator, Gen::Continents);
		assert!(m.visible_fields().iter().any(|&(k, _)| k == Knob::Continents));
		assert!(!m.visible_fields().iter().any(|&(k, _)| k == Knob::MainIslands));
	}

	#[test]
	fn symmetry_and_shore_selects_set_values() {
		let mut m = Generator::new();
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		let sr = Generator::select_rect(d, 2);
		m.on_press(sr.x + 2.0, sr.y + 2.0, w, h);
		let opt = crate::select::option_rect(sr, 3, Symmetry::ALL.len(), false); // Four Corners
		m.on_press(opt.x + 2.0, opt.y + 2.0, w, h);
		assert_eq!(m.symmetry, Symmetry::FourCorners);
		let hr = Generator::select_rect(d, 3);
		m.on_press(hr.x + 2.0, hr.y + 2.0, w, h);
		let opt = crate::select::option_rect(hr, 2, ShoreMethod::ALL.len(), false); // None
		m.on_press(opt.x + 2.0, opt.y + 2.0, w, h);
		assert_eq!(m.shore, ShoreMethod::None);
	}

	#[test]
	fn focus_type_and_generate() {
		let mut m = Generator::new();
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		let r = m.field_rect(d, Knob::MainIslands, 0).unwrap();
		m.on_press(r.x + 2.0, r.y + 2.0, w, h);
		assert_eq!(m.focus, Some((Knob::MainIslands, 0)));
		m.cell_mut(Knob::MainIslands, 0).set_text("");
		for c in "5x".chars() {
			m.key(&ModalKey::Char(c)); // non-digit ignored
		}
		assert_eq!(m.cell(Knob::MainIslands, 0).text(), "5");
		m.focus_next();
		assert_eq!(m.focus, Some((Knob::MainIslands, 1)));
		let g = m.generate_rect(d);
		m.on_press(g.x + 2.0, g.y + 2.0, w, h);
		assert_eq!(m.on_release(g.x + 2.0, g.y + 2.0, w, h), Press::Start);
		let c = m.close_rect(d);
		m.on_press(c.x + 2.0, c.y + 2.0, w, h);
		assert_eq!(m.on_release(c.x + 2.0, c.y + 2.0, w, h), Press::Close);
	}

	#[test]
	fn accessibility_mode_select_sets_mode() {
		let mut m = Generator::new();
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		assert_eq!(m.accessibility_mode, AccessibilityMode::Random);
		let r = m.access_select_rect(d).expect("accessibility row shown");
		m.on_press(r.x + 2.0, r.y + 2.0, w, h);
		assert!(m.access_open, "mode dropdown opened");
		let opt = crate::select::option_rect(r, 1, AccessibilityMode::ALL.len(), false); // paths
		m.on_press(opt.x + 2.0, opt.y + 2.0, w, h);
		assert_eq!(m.accessibility_mode, AccessibilityMode::Paths);
		assert_eq!(m.params().unwrap().0.accessibility_mode, AccessibilityMode::Paths);
	}

	#[test]
	fn hint_box_describes_hovered_control() {
		let m = Generator::new();
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		let gen_box = Generator::select_rect(d, 1);
		assert_eq!(m.hint_at(d, (gen_box.x + 2.0, gen_box.y + 2.0)), Some("Overall land / water layout"));
		let r = m.field_rect(d, Knob::MainIslands, 0).unwrap();
		assert_eq!(m.hint_at(d, (r.x + 2.0, r.y + 2.0)), Some(knob_hint(Knob::MainIslands)));
		assert_eq!(m.hint_at(d, (-50.0, -50.0)), None, "nothing under the cursor");
	}

	#[test]
	fn land_shows_rivers_and_lakes() {
		let mut m = Generator::new();
		m.generator = Gen::Land;
		let fields = m.visible_fields();
		assert!(fields.iter().any(|&(k, _)| k == Knob::Rivers));
		assert!(fields.iter().any(|&(k, _)| k == Knob::Lakes));
	}

	#[test]
	fn running_locks_controls() {
		let mut m = Generator::new();
		m.running = true;
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		let g = m.generate_rect(d);
		assert_eq!(m.on_press(g.x + 2.0, g.y + 2.0, w, h), Press::Consumed);
		assert_eq!(m.on_release(g.x + 2.0, g.y + 2.0, w, h), Press::Abort);
		let r = Generator::select_rect(d, 1);
		m.on_press(r.x + 2.0, r.y + 2.0, w, h);
		assert!(!m.generator_open, "selects locked mid-run");
	}

	#[test]
	fn dialog_never_resizes() {
		// Fixed size regardless of generator or status (the window must not jump).
		let mut m = Generator::new();
		let (w, h) = (1280.0, 800.0);
		let base = m.dialog_rect(w, h).h;
		m.generator = Gen::Rivers; // fewest rows
		assert_eq!(m.dialog_rect(w, h).h, base, "switching generator resized the dialog");
		m.status = vec!["islands: seed 42".into(), "a".into(), "b".into()];
		assert_eq!(m.dialog_rect(w, h).h, base, "status lines resized the dialog");
	}

	#[test]
	fn surprise_randomizes_props_seed_but_not_symmetry_or_shore() {
		let mut m = Generator::new();
		let (sym, shore) = (m.symmetry, m.shore);
		let before: Vec<String> = m.visible_fields().iter().map(|&(k, c)| m.cell(k, c).text().to_string()).collect();
		m.surprise(96, 96);
		let after: Vec<String> = m.visible_fields().iter().map(|&(k, c)| m.cell(k, c).text().to_string()).collect();
		// Every numeric property is a valid number; the seed is pre-filled.
		for (k, c) in m.visible_fields() {
			if k == Knob::Seed {
				continue;
			}
			assert!(m.cell(k, c).text().parse::<u8>().is_ok(), "{k:?}/{c} not a number after surprise");
		}
		assert!(m.cell(Knob::Seed, 0).text().parse::<u64>().is_ok(), "surprise should pre-fill a numeric seed");
		assert_ne!(before, after, "surprise changed nothing");
		// Symmetry and shore are left as the user set them.
		assert_eq!(m.symmetry, sym, "surprise must not touch symmetry");
		assert_eq!(m.shore, shore, "surprise must not touch shore");
		assert!(m.params().is_ok(), "surprise produced invalid params");
	}

	#[test]
	fn surprise_is_balanced_and_valid_for_every_generator() {
		// Across many rolls of every generator, Surprise must stay within sane,
		// playable bounds (no 0..=100 walls of obstructions, no degenerate sizes)
		// and always produce valid params with a pre-filled seed.
		for g in Gen::ALL {
			let mut m = Generator::new();
			m.generator = g;
			for _ in 0..24 {
				m.surprise(96, 96);
				let (p, seed) = m.params().expect("surprise must produce valid params");
				assert!(seed.is_some(), "{g:?}: surprise pre-fills a seed");
				// Bounds cover both the balanced roll and the occasional heavy /
				// full-accessibility obstruction showcase - but never degenerate.
				assert!(
					(2..=40).contains(&p.obstructions.count) && (3..=15).contains(&p.obstructions.max),
					"{g:?} obstructions {:?}",
					p.obstructions
				);
				assert!(
					(2..=12).contains(&p.decorations.count) && (3..=12).contains(&p.decorations.max),
					"{g:?} decorations {:?}",
					p.decorations
				);
				assert!((1..=4).contains(&p.drop_zones.count), "{g:?} drop zones {:?}", p.drop_zones);
				assert!(p.accessibility <= 100, "{g:?} accessibility {}", p.accessibility);
			}
		}
	}

	#[test]
	fn surprise_sizes_continents_and_seas_to_cover_the_map() {
		let area = 96.0 * 96.0;
		let target = |c: Range| c.count as f32 * std::f32::consts::PI * (c.min as f32).powi(2) / area;
		let mut m = Generator::new();
		m.generator = Gen::Continents;
		for _ in 0..16 {
			m.surprise(96, 96);
			let cov = target(m.params().unwrap().0.continents);
			assert!(cov >= 0.45, "continents should cover most of the map, got {cov:.2}");
		}
		m.generator = Gen::CentralSeas;
		for _ in 0..16 {
			m.surprise(96, 96);
			let cov = target(m.params().unwrap().0.seas);
			assert!((0.30..=0.95).contains(&cov), "central seas should cover ~40-80%, got {cov:.2}");
		}
	}

	#[test]
	fn memory_remembers_params_per_generator() {
		let mut m = Generator::new();
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		// Set a distinctive Islands value, switch to Continents, switch back.
		m.cell_mut(Knob::MainIslands, 0).set_text("7");
		let gr = Generator::select_rect(d, 1);
		m.on_press(gr.x + 2.0, gr.y + 2.0, w, h);
		let opt = crate::select::option_rect(gr, 1, Gen::ALL.len(), false); // Continents
		m.on_press(opt.x + 2.0, opt.y + 2.0, w, h);
		assert_eq!(m.generator, Gen::Continents);
		m.on_press(gr.x + 2.0, gr.y + 2.0, w, h);
		let opt = crate::select::option_rect(gr, 0, Gen::ALL.len(), false); // back to Islands
		m.on_press(opt.x + 2.0, opt.y + 2.0, w, h);
		assert_eq!(m.generator, Gen::Islands);
		assert_eq!(m.cell(Knob::MainIslands, 0).text(), "7", "Islands params not remembered across the switch");
	}

	#[test]
	fn memory_round_trips_through_gen_memory() {
		let mut m = Generator::new();
		m.cell_mut(Knob::Obstructions, 0).set_text("6");
		let mem = m.to_memory();
		let restored = Generator::from_memory(&mem);
		assert_eq!(restored.generator, Gen::Islands);
		assert_eq!(restored.cell(Knob::Obstructions, 0).text(), "6");
	}

	#[test]
	fn knob_table_is_complete() {
		// Every knob has a fields entry (new() inserts all of ALL_KNOBS).
		let m = Generator::new();
		for k in ALL_KNOBS {
			assert!(m.fields.contains_key(&k), "{k:?} missing from fields");
		}
	}
}
