//! Auto-shore: generate shoreline on the **water cells** bordering
//! land — the original maps' anatomy (a shore tile rides the water cell's
//! ground layer: `"WATR06,GSh004"`), so painted land keeps its shape and the
//! coast grows seaward.
//!
//! Tile choice maximizes *unbroken* shoreline. Two layers of rules, matching
//! how the original maps actually use `tiles.match.json` (all 24 were
//! probed; the contract below validates them exactly):
//!
//! - **Law** (placement + validity): directions facing open water must
//!   allow `__WATER__`; directions facing land must not be water-only;
//!   between band tiles anything goes. And the boundary law that drives
//!   targeting: **water never touches land orthogonally** (diagonal contact
//!   is legal — CRATER/DESERT cliffs sit corner-to-corner with the sea).
//! - **Preference** (the score): the per-family continuation lists
//!   (`"GSa:!S"`, …). Where a listed continuation matches, the shoreline
//!   pixels flow across the seam — so worklist sweeps pick the tiles with
//!   the most matched continuations. The lists are one-directional by
//!   design; both directions of a seam are counted. Ties seed from the
//!   continuation-richest family (generic straights chain plain-to-plain;
//!   sparse end-pieces appear only where corners demand them — no
//!   mirror-alternation "saw").
//!
//! **Impossible pockets close into the terrain.** No band family accepts
//! land on three sides or on two opposite sides (probed across all packs) —
//! so a 1-wide notch, 1-wide channel, or 1×1 pond can never wear a legal
//! shore. Instead of leaving a broken seam, such water cells fill with the
//! adjacent terrain (cloned from a neighbor, deterministic pixel variant)
//! and the shoreline closes straight over them. Fills cascade (a 1-wide
//! channel disappears cell by cell) and only trigger off impossible water
//! cells, so pristine maps never change.
//!
//! Targets are the water cells orthogonally touching land, plus diagonal
//! chain-closers (water diagonal to land whose flanks are band and at least
//! one newly grown) and the fringe (law-valid band cells adjacent to
//! targets re-resolve, so edits knit into existing coastline). Straights
//! resolve in the first sweep, corner pieces close the chain next, and
//! broken seams that single-cell moves can't fix re-pick both endpoint
//! cells jointly (pair moves) — the "multi iterations".
//!
//! Determinism: richness-sorted candidates, row-major worklist sweeps, and
//! per-cell splitmix64 pixel-variant picks — a run is reproducible and a
//! second run changes nothing. Valid existing shore keeps its tiles;
//! orphaned shore dissolves to open water. One run = one undo unit.

use std::collections::HashMap;

use crate::pack::family_of;
use crate::project::{LAYER_GROUND, LAYER_WATER, Project, Rng, TileRef, Transform};

/// Direction ring (N=0, E=1, S=2, W=3) as cell offsets.
const RING: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
/// The 8-neighborhood — outer-corner shore sits diagonally off land.
const RING8: [(i32, i32); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
/// Sweep cap — coastlines settle in 2–3; the cap only guards pathological
/// flip-flops.
const MAX_SWEEPS: usize = 12;

fn opp(dir: usize) -> usize {
	(dir + 2) % 4
}

/// The base-orientation direction that faces screen direction `dir` once a
/// tile is transformed by `t` — the direction form of `tile_pixel`'s
/// dest→src mapping (undo the rotation, then undo the mirror).
fn base_dir(dir: usize, t: Transform) -> usize {
	let d = (dir + 4 - t.rot as usize) % 4;
	if t.mirror { (4 - d) % 4 } else { d }
}

/// One direction of a family's rules, parsed: wildcards + concrete
/// continuations (base orientation, families interned to indices).
struct DirRule {
	water: bool,
	/// `[__WATER__]` and nothing else — this edge is open sea.
	water_only: bool,
	/// Lists `__LAND__` — this edge belongs against land.
	land: bool,
	tiles: Vec<(u16, Transform)>,
}

/// A ruled family, run-ready.
struct Family {
	pack: u8,
	name: String,
	dirs: [DirRule; 4],
	/// Concrete pixel variants (bin indices).
	variants: Vec<u16>,
	/// Part of the shore band: lists water or continuations somewhere.
	/// The rest (CRATER's all-`__LAND__` CLa/CHa) count as land.
	band: bool,
	/// Total continuation entries — rich families are the generic pieces;
	/// ties seed from them so runs stay uniform.
	richness: u16,
}

/// Every ruled family across the project's packs, deterministic order,
/// continuation specs interned to family indices.
fn parse_families(project: &Project) -> Vec<Family> {
	// Pass 1: who exists (name → index must be known before specs intern).
	let mut heads: Vec<(u8, String, Vec<u16>)> = Vec::new();
	for (pack_index, pack) in project.packs.iter().enumerate() {
		for name in pack.matches.keys() {
			let variants: Vec<u16> =
				(0..pack.tile_count()).filter(|&i| family_of(&pack.ids[i as usize]) == name).collect();
			if !variants.is_empty() {
				heads.push((pack_index as u8, name.clone(), variants));
			}
		}
	}
	heads.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
	let name_idx: HashMap<&str, u16> = heads.iter().enumerate().map(|(i, h)| (h.1.as_str(), i as u16)).collect();

	// Pass 2: parse rules with interned continuations.
	heads
		.iter()
		.map(|(pack_index, name, variants)| {
			let rule = &project.packs[*pack_index as usize].matches[name];
			let dirs: [DirRule; 4] = std::array::from_fn(|d| DirRule {
				water: rule.dirs[d].iter().any(|s| s == "__WATER__"),
				water_only: rule.dirs[d].len() == 1 && rule.dirs[d][0] == "__WATER__",
				land: rule.dirs[d].iter().any(|s| s == "__LAND__"),
				tiles: rule.dirs[d]
					.iter()
					.filter(|s| !s.starts_with("__"))
					.filter_map(|s| {
						let (id, t) = match s.split_once(':') {
							Some((id, t)) => match Transform::parse(t) {
								Ok(t) => (id, t),
								Err(e) => {
									// A dropped continuation is conservative
									// (no false matches) — but bad pack data
									// should scream in tests.
									debug_assert!(false, "{name}/{s}: {e}");
									return None;
								}
							},
							None => (s.as_str(), Transform::default()),
						};
						// Refs to families without tiles (WTR) can never
						// match a placed neighbor — drop them.
						name_idx.get(id).map(|&f| (f, t))
					})
					.collect(),
			});
			let band = dirs.iter().any(|d| d.water || !d.tiles.is_empty());
			let richness = dirs.iter().map(|d| d.tiles.len() as u16).sum();
			Family { pack: *pack_index, name: name.clone(), dirs, variants: variants.clone(), band, richness }
		})
		.collect()
}

/// Does `a` (placed with `ta`) list `b` as its continuation toward `dir`?
/// Rule specs are base-relative — placing the family transformed composes
/// its transform onto every listed continuation. (The run itself uses the
/// pre-composed `comp` lists; this reference form pins the semantics in
/// tests.)
#[cfg(test)]
fn continues(families: &[Family], a: u16, ta: Transform, dir: usize, b: u16, tb: Transform) -> bool {
	families[a as usize].dirs[base_dir(dir, ta)].tiles.iter().any(|&(f, ts)| f == b && ta.compose(ts) == tb)
}

/// Seam quality 0–2: each side's continuation list counts independently
/// (the shipped lists are one-directional; either direction means the
/// shoreline pixels flow across this seam).
#[cfg(test)]
fn seam_score(families: &[Family], a: u16, ta: Transform, dir: usize, b: u16, tb: Transform) -> usize {
	continues(families, a, ta, dir, b, tb) as usize + continues(families, b, tb, opp(dir), a, ta) as usize
}

/// Continuations pre-composed per (family, placed transform, screen dir):
/// the matchers' hot loop becomes a scan over small int lists. Indexed
/// `[family * 8 + transform.bits()][dir]`, entries `(family, bits)`.
fn build_comp(families: &[Family]) -> Vec<[Vec<(u16, u8)>; 4]> {
	(0..families.len() * 8)
		.map(|k| {
			let (fi, bits) = (k / 8, (k % 8) as u8);
			let t = Transform { rot: bits & 3, mirror: bits & 4 != 0 };
			std::array::from_fn(|d| {
				families[fi].dirs[base_dir(d, t)].tiles.iter().map(|&(f, ts)| (f, t.compose(ts).bits() as u8)).collect()
			})
		})
		.collect()
}

/// Whole-map cell snapshot (the matcher never reads live cells mid-run).
#[derive(Clone, Copy, PartialEq)]
enum Cell {
	/// Pass-1 top tile: what shores form against.
	Water,
	/// Top tile of a ruled family.
	Ruled { fam: u16, t: Transform },
	/// Land/blocked/unruled/empty.
	Plain,
}

/// A neighbor as the matcher sees it during the run.
#[derive(Clone, Copy)]
enum Nb {
	/// Off the map — unconstrained.
	Edge,
	/// Water that is not becoming shore.
	OpenWater,
	/// A target cell not yet assigned this sweep — unconstrained for now.
	Pending,
	/// Shore (existing or assigned): continuations are scored.
	Shore(u16, Transform),
	/// Land-side (land, blocked, belts) — anything but open sea.
	Hard,
}

fn cell_at(snap: &[Cell], w: i32, h: i32, x: i32, y: i32) -> Option<Cell> {
	(x >= 0 && x < w && y >= 0 && y < h).then(|| snap[(y * w + x) as usize])
}

/// Land for targeting: what shore forms against (off-map is not land).
fn is_land(families: &[Family], snap: &[Cell], w: i32, h: i32, x: i32, y: i32) -> bool {
	match cell_at(snap, w, h, x, y) {
		Some(Cell::Plain) => true,
		Some(Cell::Ruled { fam, .. }) => !families[fam as usize].band,
		_ => false,
	}
}

/// Snapshot every cell once (the matchers never read live cells mid-run).
fn snapshot_cells(project: &Project, name_idx: &HashMap<&str, u16>) -> Vec<Cell> {
	let (w, h) = (project.width as i32, project.height as i32);
	(0..h)
		.flat_map(|y| (0..w).map(move |x| (x, y)))
		.map(|(x, y)| {
			let stack = project.cell(x as u16, y as u16).unwrap();
			let Some(top) = stack[LAYER_GROUND].or(stack[LAYER_WATER]) else {
				return Cell::Plain;
			};
			let pack = &project.packs[top.pack as usize];
			if pack.pass.as_ref().map(|p| p[top.tile as usize]) == Some(1) {
				return Cell::Water;
			}
			match name_idx.get(family_of(&pack.ids[top.tile as usize])) {
				Some(&fam) => Cell::Ruled { fam, t: top.transform },
				None => Cell::Plain,
			}
		})
		.collect()
}

/// The pass's working rectangle: cells inclusive, expanded by one;
/// `None` = the whole map.
fn region_rect(region: Option<(u16, u16, u16, u16)>, w: i32, h: i32) -> (i32, i32, i32, i32) {
	match region {
		Some((ax, ay, bx, by)) => (
			ax.min(bx).saturating_sub(1) as i32,
			ay.min(by).saturating_sub(1) as i32,
			(ax.max(bx) as i32 + 1).min(w - 1),
			(ay.max(by) as i32 + 1).min(h - 1),
		),
		None => (0, 0, w - 1, h - 1),
	}
}

/// Landfill: impossible water pockets close into the terrain. No band
/// family accepts land on 3 sides or 2 opposite sides, so such water cells
/// can never wear a legal shore. They fill with the adjacent terrain
/// instead — 1-wide notches and channels and 1×1 ponds disappear, and the
/// shoreline closes straight over them. Fills cascade (a 1-wide channel
/// disappears cell by cell) and only trigger off impossible water cells,
/// so pristine maps never change. Mutates `snap` to match; returns the
/// fill edits.
fn landfill(
	project: &Project,
	families: &[Family],
	snap: &mut [Cell],
	rect: (i32, i32, i32, i32),
) -> Vec<(u16, u16, TileRef)> {
	let (w, h) = (project.width as i32, project.height as i32);
	let (x0, y0, x1, y1) = rect;
	let mut fills: Vec<(u16, u16, TileRef)> = Vec::new();
	for _round in 0..16 {
		let mut filled = false;
		for y in y0..=y1 {
			for x in x0..=x1 {
				if snap[(y * w + x) as usize] != Cell::Water {
					continue;
				}
				let mut mask = 0u8;
				for (d, &(dx, dy)) in RING.iter().enumerate() {
					if is_land(families, snap, w, h, x + dx, y + dy) {
						mask |= 1 << d;
					}
				}
				let n = mask.count_ones();
				let opposite = mask & 0b0101 == 0b0101 || mask & 0b1010 == 0b1010;
				if n < 3 && !opposite {
					continue;
				}
				// Clone the first orthogonal land donor, deterministic
				// pixel variant within the donor's family.
				let entry = RING.iter().enumerate().find_map(|(d, &(dx, dy))| {
					if mask & (1 << d) == 0 {
						return None;
					}
					let (nx, ny) = (x + dx, y + dy);
					let donor = project.cell(nx as u16, ny as u16)?[LAYER_GROUND]?;
					let pack = &project.packs[donor.pack as usize];
					let fam = family_of(&pack.ids[donor.tile as usize]);
					let variants: Vec<u16> =
						(0..pack.tile_count()).filter(|&i| family_of(&pack.ids[i as usize]) == fam).collect();
					let mut rng = Rng::new(0x53484f5245 ^ ((x as u64) << 32 | y as u64));
					let tile = variants[rng.below(variants.len() as u32) as usize];
					Some(TileRef { pack: donor.pack, tile, transform: donor.transform })
				});
				let Some(entry) = entry else { continue };
				snap[(y * w + x) as usize] = Cell::Plain;
				fills.push((x as u16, y as u16, entry));
				filled = true;
			}
		}
		if !filled {
			break;
		}
	}
	fills
}

/// Branch-and-bound minimiser for one repair window (`auto_shore_alt`'s
/// second pass). Assign each window cell a tile from its candidate list to
/// minimise broken seams — window-internal pairs (both endpoints in the
/// window, in any direction, so 2-D folds are handled exactly) and pairs
/// against the fixed border. `best` is seeded with the current assignment's
/// cost so only a STRICT improvement replaces it; candidates dive
/// cheapest-first (ties by family/bits) so the search is deterministic and
/// finds a good leaf early; `budget` caps the node count. Returns once the
/// budget is spent or the tree is exhausted — `best_assign` then holds the
/// best found.
#[allow(clippy::too_many_arguments)]
fn repair_window<T, F, K>(
	lvl: usize,
	partial: i32,
	assign: &mut [T],
	best: &mut i32,
	best_assign: &mut Vec<T>,
	cand: &[Vec<T>],
	win_nb: &[[Option<usize>; 4]],
	fixed_nb: &[[Option<T>; 4]],
	seam: &F,
	key: &K,
	budget: &mut i64,
) where
	T: Copy,
	F: Fn(T, usize, T) -> usize,
	K: Fn(T) -> (u16, u8),
{
	if lvl == assign.len() {
		if partial < *best {
			*best = partial;
			best_assign.clear();
			best_assign.extend_from_slice(assign);
		}
		return;
	}
	// Cost each candidate against the already-determined neighbours (fixed
	// border + lower-index window cells), then dive cheapest-first.
	let mut scored: Vec<(i32, T)> = cand[lvl]
		.iter()
		.map(|&t| {
			let mut add = 0;
			for d in 0..4 {
				if let Some(j) = win_nb[lvl][d] {
					if j < lvl {
						add += (seam(t, d, assign[j]) == 0) as i32;
					}
				} else if let Some(nb) = fixed_nb[lvl][d] {
					add += (seam(t, d, nb) == 0) as i32;
				}
			}
			(add, t)
		})
		.collect();
	scored.sort_by_key(|&(add, t)| (add, key(t)));
	for (add, t) in scored {
		// Sorted ascending: once one can't beat the incumbent, none can.
		if partial + add >= *best {
			break;
		}
		if *budget <= 0 {
			return;
		}
		*budget -= 1;
		assign[lvl] = t;
		repair_window(lvl + 1, partial + add, assign, best, best_assign, cand, win_nb, fixed_nb, seam, key, budget);
	}
}

/// How much terrain the fix pass may rewrite (the Auto Fix Shore modes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixStrength {
	/// Re-tile broken shore-band cells only (the Fast mode).
	Shore,
	/// Also re-tile ruled land adjacent to the band — each such cell keeps
	/// its own tile as a candidate, so land changes only when it closes a
	/// seam (the Aggressive mode; the old `mangle` flag).
	Mangle,
	/// Total freedom: adjacent land AND water join the re-tile set, any cell
	/// may additionally become open water, and seams against fixed water /
	/// unruled land are scored too. Where the window solver *proves* no
	/// local fix exists, the 3×3 around the break is erased to open water
	/// (each cell at most once) and the shore regrows against the new edge —
	/// a broken seam never survives for lack of trying.
	Destructive,
}

/// A cell's trial state during a fix run. `Plain` never appears in a
/// candidate list — it only models a fixed unruled neighbor so destructive
/// scoring can refuse to flood naked water against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trial {
	Tile(u16, Transform),
	Water,
	Plain,
}

impl Trial {
	/// Deterministic sort key (candidate dive order, ties stable).
	fn key(self) -> (u16, u8) {
		match self {
			Trial::Tile(f, t) => (f, t.bits() as u8),
			Trial::Water => (u16::MAX - 1, 0),
			Trial::Plain => (u16::MAX, 0),
		}
	}
}

/// A resumable fix-shore run: the bounded backtracking repair of
/// `fix_shore`, split so the caller can drive it across frames — `step`
/// does a budgeted slice of work and reports progress, `apply` commits the
/// result as one undo unit. This is what lets the Auto Fix Shore modal show
/// live stats and offer a Stop button without ever freezing the UI.
pub struct FixSession {
	w: i32,
	h: i32,
	families: Vec<Family>,
	comp: Vec<[Vec<(u16, u8)>; 4]>,
	snap: Vec<Cell>,
	/// Which cells may be re-tiled (band shore, plus adjacent land in
	/// `Mangle`, plus adjacent water in `Destructive`).
	retile: Vec<bool>,
	/// Re-tileable cells, row-major (the work list).
	cells: Vec<(i32, i32)>,
	/// Trial state per cell (`None` outside the re-tile set).
	work: Vec<Option<Trial>>,
	strength: FixStrength,
	/// The session's region bounds (inclusive) — escalation stays inside.
	bounds: (i32, i32, i32, i32),
	/// Destructive escalation bookkeeping: a cell may be force-flattened to
	/// water at most once, so blast-and-regrow can't oscillate forever.
	blasted: Vec<bool>,
	found: usize,
	done: bool,
	/// In-progress pass state (so `step` can resume across calls): the
	/// broken cells left to visit this pass, the cells already covered by a
	/// window this pass, and whether the pass has improved anything yet.
	queue: std::collections::VecDeque<(i32, i32)>,
	handled: std::collections::HashSet<(i32, i32)>,
	pass_improved: bool,
}

/// Window size and per-window node cap (shared by `fix_shore` and the
/// session): a window covers the local fold; the cap bounds the cost of
/// proving an unfixable spot (an impossible tileset corner).
const FIX_WMAX: usize = 8;
const FIX_PER_WIN: i64 = 30_000;

impl FixSession {
	pub fn new(project: &Project, region: Option<(u16, u16, u16, u16)>, strength: FixStrength) -> Self {
		let (w, h) = (project.width as i32, project.height as i32);
		let families = parse_families(project);
		let name_idx: HashMap<&str, u16> =
			families.iter().enumerate().map(|(i, f)| (f.name.as_str(), i as u16)).collect();
		let comp = build_comp(&families);
		let snap = snapshot_cells(project, &name_idx);
		let (x0, y0, x1, y1) = region_rect(region, w, h);

		let mut work: Vec<Option<Trial>> = vec![None; (w * h) as usize];
		let mut retile = vec![false; (w * h) as usize];
		let mut cells: Vec<(i32, i32)> = Vec::new();
		let band_or_none = families.iter().all(|f| !f.band);
		if !band_or_none {
			for y in y0..=y1 {
				for x in x0..=x1 {
					if let Some(Cell::Ruled { fam, t }) = cell_at(&snap, w, h, x, y) {
						if families[fam as usize].band {
							let ci = (y * w + x) as usize;
							retile[ci] = true;
							work[ci] = Some(Trial::Tile(fam, t));
							cells.push((x, y));
						}
					}
				}
			}
			// Aggressive may also re-tile land next to broken shore — it
			// keeps each such cell's own tile as a candidate, so it only
			// changes when that closes a seam. Destructive additionally
			// claims adjacent water, so a hopeless fold can flood instead.
			if strength != FixStrength::Shore {
				let mut extra: Vec<(i32, i32)> = Vec::new();
				for &(x, y) in &cells {
					for &(dx, dy) in &RING {
						let (nx, ny) = (x + dx, y + dy);
						if nx < x0 || ny < y0 || nx > x1 || ny > y1 {
							continue;
						}
						let ci = (ny * w + nx) as usize;
						if retile[ci] {
							continue;
						}
						match cell_at(&snap, w, h, nx, ny) {
							Some(Cell::Ruled { fam, t }) if !families[fam as usize].band => {
								retile[ci] = true;
								work[ci] = Some(Trial::Tile(fam, t));
								extra.push((nx, ny));
							}
							Some(Cell::Water) if strength == FixStrength::Destructive => {
								retile[ci] = true;
								work[ci] = Some(Trial::Water);
								extra.push((nx, ny));
							}
							_ => {}
						}
					}
				}
				cells.extend(extra);
				cells.sort_by_key(|&(x, y)| (y, x));
			}
		}

		let mut s = Self {
			w,
			h,
			families,
			comp,
			snap,
			retile,
			cells,
			work,
			strength,
			bounds: (x0, y0, x1, y1),
			blasted: vec![false; (w * h) as usize],
			found: 0,
			done: false,
			queue: std::collections::VecDeque::new(),
			handled: std::collections::HashSet::new(),
			pass_improved: false,
		};
		s.found = s.broken_total();
		s.done = s.cells.is_empty() || s.found == 0;
		s
	}

	fn cseam(&self, a: u16, ta: Transform, dir: usize, b: u16, tb: Transform) -> usize {
		let admits = |a: u16, ta: Transform, dir: usize, b: u16, tb: Transform| {
			self.comp[a as usize * 8 + ta.bits() as usize][dir].contains(&(b, tb.bits() as u8))
		};
		admits(a, ta, dir, b, tb) as usize + admits(b, tb, opp(dir), a, ta) as usize
	}

	/// Seam validity between two trial states (0 = broken). Tile×Tile is the
	/// rule-table seam; water against a tile consults that tile's dir rule
	/// (its transform unrolled); water beside water is fine; naked water
	/// against unruled land is exactly the kind of seam Destructive must not
	/// create, so it scores broken.
	fn tseam(&self, a: Trial, dir: usize, b: Trial) -> usize {
		let water_ok = |f: u16, t: Transform, d: usize| -> usize {
			2 * self.families[f as usize].dirs[base_dir(d, t)].water as usize
		};
		match (a, b) {
			(Trial::Tile(fa, ta), Trial::Tile(fb, tb)) => self.cseam(fa, ta, dir, fb, tb),
			(Trial::Tile(f, t), Trial::Water) => water_ok(f, t, dir),
			(Trial::Water, Trial::Tile(f, t)) => water_ok(f, t, opp(dir)),
			(Trial::Water, Trial::Water) => 2,
			(Trial::Water, Trial::Plain) | (Trial::Plain, Trial::Water) => 0,
			// Tile↔Plain stays uncounted-OK (parity with the original tool,
			// which never judged band-against-unruled seams).
			(Trial::Tile(..), Trial::Plain) | (Trial::Plain, Trial::Tile(..)) => 2,
			(Trial::Plain, Trial::Plain) => 2,
		}
	}

	/// The band tile at a cell: trial value if re-tileable, else the fixed
	/// band tile from the snapshot (`None` for land/water/off-map). The
	/// Fast/Aggressive view of the world — water and unruled land are
	/// invisible to scoring, exactly like the original tool.
	fn tile_at(&self, x: i32, y: i32) -> Option<(u16, Transform)> {
		if x < 0 || y < 0 || x >= self.w || y >= self.h {
			return None;
		}
		let ci = (y * self.w + x) as usize;
		if self.retile[ci] {
			return match self.work[ci] {
				Some(Trial::Tile(f, t)) => Some((f, t)),
				_ => None,
			};
		}
		match self.snap[ci] {
			Cell::Ruled { fam, t } if self.families[fam as usize].band => Some((fam, t)),
			_ => None,
		}
	}

	/// The trial state at a cell for the current strength. For re-tileable
	/// cells it's the live trial; for fixed cells, Destructive sees the full
	/// terrain (any ruled tile / water / unruled land), the other strengths
	/// see only band tiles (`None` elsewhere — uncounted, the old behavior).
	fn trial_at(&self, x: i32, y: i32) -> Option<Trial> {
		if x < 0 || y < 0 || x >= self.w || y >= self.h {
			return None;
		}
		let ci = (y * self.w + x) as usize;
		if self.retile[ci] {
			return self.work[ci];
		}
		if self.strength == FixStrength::Destructive {
			return match self.snap[ci] {
				Cell::Ruled { fam, t } => Some(Trial::Tile(fam, t)),
				Cell::Water => Some(Trial::Water),
				Cell::Plain => Some(Trial::Plain),
			};
		}
		match self.snap[ci] {
			Cell::Ruled { fam, t } if self.families[fam as usize].band => Some(Trial::Tile(fam, t)),
			_ => None,
		}
	}

	fn lawful_at(&self, x: i32, y: i32) -> Vec<Trial> {
		let dir_ok = |fam: &Family, t: Transform, dir: usize, nb: Nb| {
			let rule = &fam.dirs[base_dir(dir, t)];
			match nb {
				Nb::Edge | Nb::Pending | Nb::Shore(..) => true,
				Nb::OpenWater => rule.water,
				Nb::Hard => !rule.water_only,
			}
		};
		let views: [Nb; 4] = std::array::from_fn(|d| {
			let (nx, ny) = (x + RING[d].0, y + RING[d].1);
			if self.is_retile(nx, ny) && self.work[(ny * self.w + nx) as usize] == Some(Trial::Water) {
				// A neighbor flooded by this run reads as open water.
				Nb::OpenWater
			} else if self.tile_at(nx, ny).is_some() {
				Nb::Shore(0, Transform::default())
			} else if nx < 0 || ny < 0 || nx >= self.w || ny >= self.h {
				Nb::Edge
			} else if self.snap[(ny * self.w + nx) as usize] == Cell::Water {
				Nb::OpenWater
			} else {
				Nb::Hard
			}
		});
		let mut out = Vec::new();
		for (fi, fam) in self.families.iter().enumerate() {
			if !fam.band {
				continue;
			}
			for rot in 0..4u8 {
				for mirror in [false, true] {
					let t = Transform { rot, mirror };
					if (0..4).all(|d| dir_ok(fam, t, d, views[d])) {
						out.push(Trial::Tile(fi as u16, t));
					}
				}
			}
		}
		// Mangle keeps a cell's own (possibly non-band) tile as an option,
		// so re-tiling land is opt-in per cell, not forced. Destructive adds
		// open water — the universal solvent.
		if self.strength != FixStrength::Shore {
			if let Some(cur) = self.work[(y * self.w + x) as usize] {
				if !out.contains(&cur) {
					out.push(cur);
				}
			}
		}
		if self.strength == FixStrength::Destructive && !out.contains(&Trial::Water) {
			out.push(Trial::Water);
		}
		out
	}

	fn broken_at(&self, x: i32, y: i32) -> bool {
		let Some(a) = self.trial_at(x, y) else { return false };
		(0..4).any(|d| {
			let (nx, ny) = (x + RING[d].0, y + RING[d].1);
			matches!(self.trial_at(nx, ny), Some(b) if self.tseam(a, d, b) == 0)
		})
	}

	/// Broken seams over the whole re-tile set, each adjacent pair once:
	/// right/down (the original tool's accounting — exact parity for
	/// Fast/Aggressive). Destructive additionally counts left/up seams
	/// against *fixed* neighbors, so a flooded edge facing fixed water/land
	/// on any side is judged.
	pub fn broken_total(&self) -> usize {
		let destructive = self.strength == FixStrength::Destructive;
		let mut n = 0;
		for &(x, y) in &self.cells {
			let Some(a) = self.trial_at(x, y) else { continue };
			for dir in 0..4usize {
				let (nx, ny) = (x + RING[dir].0, y + RING[dir].1);
				// Right/down pairs always count (the original accounting);
				// left/up additionally count against *fixed* neighbors in
				// Destructive, so boundary seams on any side are judged.
				let counted = matches!(dir, 1 | 2) || (destructive && !self.is_retile(nx, ny));
				if !counted {
					continue;
				}
				if let Some(b) = self.trial_at(nx, ny) {
					if self.tseam(a, dir, b) == 0 {
						n += 1;
					}
				}
			}
		}
		n
	}

	pub fn found(&self) -> usize {
		self.found
	}
	pub fn remaining(&self) -> usize {
		self.broken_total()
	}
	pub fn fixed(&self) -> usize {
		self.found.saturating_sub(self.remaining())
	}
	pub fn is_done(&self) -> bool {
		self.done
	}

	fn is_retile(&self, x: i32, y: i32) -> bool {
		x >= 0 && y >= 0 && x < self.w && y < self.h && self.retile[(y * self.w + x) as usize]
	}

	/// Destructive escalation: erase the 3×3 around an unfixable break to
	/// open water, and claim the surrounding 5×5 ring into the re-tile set
	/// at its original state — the new pond needs its shore ring built on
	/// the *outside*, on cells the band never covered (a lone water pocket
	/// in unruled land is inexpressible otherwise). Each cell flattens at
	/// most once; returns whether anything changed.
	fn blast(&mut self, bx: i32, by: i32) -> bool {
		let (x0, y0, x1, y1) = self.bounds;
		let mut changed = false;
		let mut grew = false;
		for dy in -2..=2i32 {
			for dx in -2..=2i32 {
				let (x, y) = (bx + dx, by + dy);
				if x < x0 || y < y0 || x > x1 || y > y1 {
					continue;
				}
				let ci = (y * self.w + x) as usize;
				if !self.retile[ci] {
					self.retile[ci] = true;
					self.work[ci] = Some(match self.snap[ci] {
						Cell::Ruled { fam, t } => Trial::Tile(fam, t),
						Cell::Water => Trial::Water,
						Cell::Plain => Trial::Plain,
					});
					self.cells.push((x, y));
					grew = true;
				}
				// Only the inner 3×3 is flattened — once per cell.
				if dx.abs() <= 1 && dy.abs() <= 1 && !self.blasted[ci] {
					self.blasted[ci] = true;
					if self.work[ci] != Some(Trial::Water) {
						self.work[ci] = Some(Trial::Water);
						changed = true;
					}
				}
			}
		}
		if grew {
			// Keep the work list row-major so pass order stays deterministic.
			self.cells.sort_by_key(|&(x, y)| (y, x));
		}
		changed || grew
	}

	/// Do roughly `budget` nodes of repair work, resuming the current pass
	/// across calls. Each broken window is solved (or proven unfixable) in
	/// one go — the per-window backtracking can't resume mid-window, so a
	/// `step` overshoots `budget` by up to one window. A full pass with no
	/// improvement sets `done` (converged). Returns the nodes spent.
	pub fn step(&mut self, budget: i64) -> i64 {
		if self.done {
			return 0;
		}
		let mut spent = 0i64;
		loop {
			// Start a new pass when the queue drains: a pass that improved
			// nothing means we've converged.
			if self.queue.is_empty() {
				if !self.handled.is_empty() && !self.pass_improved {
					self.done = true;
					return spent;
				}
				let mut brk: Vec<(i32, i32)> =
					self.cells.iter().copied().filter(|&(x, y)| self.broken_at(x, y)).collect();
				if brk.is_empty() {
					self.done = true;
					return spent;
				}
				brk.sort_by_key(|&(x, y)| (y, x));
				self.queue = brk.into();
				self.handled.clear();
				self.pass_improved = false;
			}

			let Some((bx, by)) = self.queue.pop_front() else { break };
			if spent >= budget {
				// Put it back; resume here next call.
				self.queue.push_front((bx, by));
				return spent;
			}
			if self.handled.contains(&(bx, by)) || !self.broken_at(bx, by) {
				continue;
			}
			// BFS window of re-tileable cells around the break.
			let mut win: Vec<(i32, i32)> = Vec::new();
			let mut seen: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
			let mut queue = std::collections::VecDeque::new();
			queue.push_back((bx, by));
			seen.insert((bx, by));
			while let Some((cx, cy)) = queue.pop_front() {
				if win.len() >= FIX_WMAX {
					break;
				}
				win.push((cx, cy));
				for &(dx, dy) in &RING {
					let (nx, ny) = (cx + dx, cy + dy);
					if self.is_retile(nx, ny) && seen.insert((nx, ny)) {
						queue.push_back((nx, ny));
					}
				}
			}
			for &c in &win {
				self.handled.insert(c);
			}

			let m = win.len();
			let widx = |cx: i32, cy: i32| win.iter().position(|&(a, b)| a == cx && b == cy);
			let mut win_nb = vec![[None; 4]; m];
			let mut fixed_nb: Vec<[Option<Trial>; 4]> = vec![[None; 4]; m];
			let mut cand: Vec<Vec<Trial>> = Vec::with_capacity(m);
			for (i, &(cx, cy)) in win.iter().enumerate() {
				for d in 0..4 {
					let (nx, ny) = (cx + RING[d].0, cy + RING[d].1);
					match widx(nx, ny) {
						Some(j) => win_nb[i][d] = Some(j),
						None if self.is_retile(nx, ny) => {
							// A re-tileable cell outside this window: its
							// live trial is the border this window sees.
							fixed_nb[i][d] = self.work[(ny * self.w + nx) as usize];
						}
						None => fixed_nb[i][d] = self.trial_at(nx, ny),
					}
				}
				let mut c = self.lawful_at(cx, cy);
				c.sort_by_key(|t| t.key());
				cand.push(c);
			}
			let current: Vec<Trial> =
				win.iter().map(|&(cx, cy)| self.work[(cy * self.w + cx) as usize].unwrap()).collect();
			let mut cur = 0;
			for i in 0..m {
				for d in 0..4 {
					if let Some(j) = win_nb[i][d] {
						if j < i {
							cur += (self.tseam(current[i], d, current[j]) == 0) as i32;
						}
					} else if let Some(nb) = fixed_nb[i][d] {
						cur += (self.tseam(current[i], d, nb) == 0) as i32;
					}
				}
			}
			if cur == 0 {
				continue;
			}
			let mut best = cur;
			let mut best_assign = current.clone();
			let mut assign = current.clone();
			// A window runs to completion (full per-window cap) so it always
			// resolves or proves itself unfixable — the backtracking can't
			// resume mid-window. `step` therefore overshoots `budget` by at
			// most one window.
			let mut wbud = FIX_PER_WIN;
			let comp = &self.comp;
			let families = &self.families;
			let tseam = |a: Trial, dir: usize, b: Trial| -> usize {
				let admits = |a: u16, ta: Transform, dir: usize, b: u16, tb: Transform| {
					comp[a as usize * 8 + ta.bits() as usize][dir].contains(&(b, tb.bits() as u8))
				};
				let water_ok =
					|f: u16, t: Transform, d: usize| 2 * families[f as usize].dirs[base_dir(d, t)].water as usize;
				match (a, b) {
					(Trial::Tile(fa, ta), Trial::Tile(fb, tb)) => {
						admits(fa, ta, dir, fb, tb) as usize + admits(fb, tb, opp(dir), fa, ta) as usize
					}
					(Trial::Tile(f, t), Trial::Water) => water_ok(f, t, dir),
					(Trial::Water, Trial::Tile(f, t)) => water_ok(f, t, opp(dir)),
					(Trial::Water, Trial::Water) => 2,
					(Trial::Water, Trial::Plain) | (Trial::Plain, Trial::Water) => 0,
					(Trial::Tile(..), Trial::Plain) | (Trial::Plain, Trial::Tile(..)) => 2,
					(Trial::Plain, Trial::Plain) => 2,
				}
			};
			repair_window(
				0,
				0,
				&mut assign,
				&mut best,
				&mut best_assign,
				&cand,
				&win_nb,
				&fixed_nb,
				&tseam,
				&|t: Trial| t.key(),
				&mut wbud,
			);
			spent += FIX_PER_WIN - wbud;
			if best < cur {
				for (i, &(cx, cy)) in win.iter().enumerate() {
					self.work[(cy * self.w + cx) as usize] = Some(best_assign[i]);
				}
				self.pass_improved = true;
			} else if self.strength == FixStrength::Destructive && self.blast(bx, by) {
				// No lawful local fix exists — erase the 3×3 around the break
				// to open water; the next passes regrow shore from the edge.
				self.pass_improved = true;
			}
		}
		spent
	}

	/// Commit the re-tiled cells to the project as one undo unit; returns
	/// the number of cells changed. A cell that became open water gets its
	/// ground layer erased — the water base beneath shows through.
	pub fn apply(&self, project: &mut Project) -> usize {
		let mut edits: Vec<(u16, u16, usize, Option<TileRef>)> = Vec::new();
		for &(x, y) in &self.cells {
			let ci = (y * self.w + x) as usize;
			let orig = match self.snap[ci] {
				Cell::Ruled { fam, t } => Some(Trial::Tile(fam, t)),
				Cell::Water => Some(Trial::Water),
				Cell::Plain => Some(Trial::Plain),
			};
			if self.work[ci] == orig {
				continue;
			}
			let entry = match self.work[ci] {
				Some(Trial::Tile(fi, t)) => {
					let fam = &self.families[fi as usize];
					let mut rng = Rng::new(0x53484f5245 ^ ((x as u64) << 32 | y as u64));
					let tile = fam.variants[rng.below(fam.variants.len() as u32) as usize];
					Some(TileRef { pack: fam.pack, tile, transform: t })
				}
				_ => None,
			};
			edits.push((x as u16, y as u16, LAYER_GROUND, entry));
		}
		let count = edits.len();
		project.place_many(&edits);
		count
	}
}

impl Project {
	/// Open a resumable fix-shore session — the steppable form of
	/// `fix_shore` for the Auto Fix Shore modal; see [`FixStrength`] for what
	/// each mode may rewrite.
	pub fn fix_session(&self, region: Option<(u16, u16, u16, u16)>, strength: FixStrength) -> FixSession {
		FixSession::new(self, region, strength)
	}

	/// Fix the land/water boundary in `region` (cells inclusive, expanded
	/// by one; `None` = whole map): water cells touching land grow shore,
	/// law-breaking shore re-resolves, orphaned shore dissolves, and
	/// impossible pockets (1-wide notches/channels, 1×1 ponds) close into
	/// the terrain. One undo unit. Returns
	/// `(cells changed, seams left without a listed continuation)`.
	pub fn auto_shore(&mut self, region: Option<(u16, u16, u16, u16)>) -> (usize, usize) {
		let (w, h) = (self.width as i32, self.height as i32);
		let families = parse_families(self);
		if families.iter().all(|f| !f.band) {
			return (0, 0);
		}
		let name_idx: HashMap<&str, u16> =
			families.iter().enumerate().map(|(i, f)| (f.name.as_str(), i as u16)).collect();

		let comp = build_comp(&families);
		let admits = |a: u16, ta: Transform, dir: usize, b: u16, tb: Transform| -> bool {
			comp[a as usize * 8 + ta.bits() as usize][dir].contains(&(b, tb.bits() as u8))
		};
		let cseam = |a: u16, ta: Transform, dir: usize, b: u16, tb: Transform| -> usize {
			admits(a, ta, dir, b, tb) as usize + admits(b, tb, opp(dir), a, ta) as usize
		};

		let mut snap = snapshot_cells(self, &name_idx);
		let rect = region_rect(region, w, h);
		let (x0, y0, x1, y1) = rect;
		let fills = landfill(self, &families, &mut snap, rect);

		// Geometry is frozen from here on.
		let snap = snap;
		let at = |x: i32, y: i32| cell_at(&snap, w, h, x, y);
		let land = |x: i32, y: i32| is_land(&families, &snap, w, h, x, y);

		// ---- Find targets --------------------------------------------------
		struct Target {
			x: i32,
			y: i32,
			candidates: Vec<(u16, Transform)>,
			/// What stands there now (`None` = open water).
			original: Option<(u16, Transform)>,
			/// Any land in the 8-neighborhood? (gates orphan removal)
			near_land: bool,
		}
		let mut targets: Vec<Target> = Vec::new();
		// Target index per cell, -1 = none (flat grid: the hot lookup).
		let mut tat: Vec<i32> = vec![-1; (w * h) as usize];

		// The matcher's neighbor view; empty `picks` = detection time.
		let view = |x: i32, y: i32, dir: usize, tat: &[i32], picks: &[Option<(u16, Transform)>]| {
			let (dx, dy) = RING[dir];
			let (nx, ny) = (x + dx, y + dy);
			let Some(cell) = at(nx, ny) else { return Nb::Edge };
			let ti = tat[(ny * w + nx) as usize];
			if ti >= 0 {
				return match picks.get(ti as usize).copied().flatten() {
					Some((fam, t)) => Nb::Shore(fam, t),
					None => Nb::Pending,
				};
			}
			match cell {
				Cell::Water => Nb::OpenWater,
				Cell::Ruled { fam, t } if families[fam as usize].band => Nb::Shore(fam, t),
				Cell::Ruled { .. } | Cell::Plain => Nb::Hard,
			}
		};
		// The law: water edges face water, sea-only edges never face LAND —
		// but between band tiles anything goes (originals legally press
		// water-only edges against band neighbors in double-thick corners).
		let dir_ok = |fam: &Family, t: Transform, dir: usize, nb: Nb| {
			let rule = &fam.dirs[base_dir(dir, t)];
			match nb {
				Nb::Edge | Nb::Pending | Nb::Shore(..) => true,
				Nb::OpenWater => rule.water,
				Nb::Hard => !rule.water_only,
			}
		};

		let no_picks: Vec<Option<(u16, Transform)>> = Vec::new();
		let no_targets: Vec<i32> = vec![-1; (w * h) as usize];
		for y in y0..=y1 {
			for x in x0..=x1 {
				let target = match at(x, y).unwrap() {
					// The boundary law: water never touches land orthogonally.
					Cell::Water => RING.iter().any(|&(dx, dy)| land(x + dx, y + dy)).then_some(None),
					// Existing band that breaks the law re-resolves;
					// valid shore is left alone (no coastline churn).
					Cell::Ruled { fam, t } if families[fam as usize].band => {
						let f = &families[fam as usize];
						(0..4)
							.any(|d| !dir_ok(f, t, d, view(x, y, d, &no_targets, &no_picks)))
							.then_some(Some((fam, t)))
					}
					_ => None,
				};
				if let Some(original) = target {
					let near_land = RING8.iter().any(|&(dx, dy)| land(x + dx, y + dy));
					tat[(y * w + x) as usize] = targets.len() as i32;
					targets.push(Target { x, y, candidates: Vec::new(), original, near_land });
				}
			}
		}
		// Chain-closers: water diagonal to land, both flanks band-or-target,
		// at least one flank NEWLY GROWN (water becoming shore) — a painted
		// ring closes its corners, but pristine coastline (where diagonal
		// contact is legal) never grows, not even around re-resolving cells.
		let mut corners = Vec::new();
		{
			let bandish = |x: i32, y: i32| {
				tat[(y * w + x) as usize] >= 0
					|| matches!(at(x, y), Some(Cell::Ruled { fam, .. }) if families[fam as usize].band)
			};
			let grown = |x: i32, y: i32| {
				let ti = tat[(y * w + x) as usize];
				ti >= 0 && targets[ti as usize].original.is_none()
			};
			for y in y0..=y1 {
				for x in x0..=x1 {
					if at(x, y) != Some(Cell::Water) || tat[(y * w + x) as usize] >= 0 {
						continue;
					}
					let closes = [(-1, -1), (1, -1), (-1, 1), (1, 1)].iter().any(|&(dx, dy)| {
						land(x + dx, y + dy)
							&& bandish(x + dx, y) && bandish(x, y + dy)
							&& (grown(x + dx, y) || grown(x, y + dy))
					});
					if closes {
						corners.push((x, y));
					}
				}
			}
		}
		for (x, y) in corners {
			let near_land = RING8.iter().any(|&(dx, dy)| land(x + dx, y + dy));
			tat[(y * w + x) as usize] = targets.len() as i32;
			targets.push(Target { x, y, candidates: Vec::new(), original: None, near_land });
		}
		if targets.is_empty() && fills.is_empty() {
			return (0, 0);
		}
		// The edit's blast radius: a law-valid band cell next to a target may
		// no longer *continue* into the reworked coast — let it re-resolve
		// (keep-current-on-tie protects it unless something strictly fits
		// better). Pristine maps have no targets, so nothing ever churns.
		let mut fringe = Vec::new();
		for t in &targets {
			for &(dx, dy) in &RING {
				let (nx, ny) = (t.x + dx, t.y + dy);
				if nx < 0 || ny < 0 || nx >= w || ny >= h {
					continue;
				}
				if tat[(ny * w + nx) as usize] >= 0 || fringe.contains(&(nx, ny)) {
					continue;
				}
				if let Some(Cell::Ruled { fam, .. }) = at(nx, ny) {
					if families[fam as usize].band {
						fringe.push((nx, ny));
					}
				}
			}
		}
		for (x, y) in fringe {
			let Some(Cell::Ruled { fam, t }) = at(x, y) else { unreachable!() };
			let near_land = RING8.iter().any(|&(dx, dy)| land(x + dx, y + dy));
			tat[(y * w + x) as usize] = targets.len() as i32;
			targets.push(Target { x, y, candidates: Vec::new(), original: Some((fam, t)), near_land });
		}

		let stamp = std::env::var("SHORE_TIME").is_ok();
		let mut t0 = std::time::Instant::now();
		let mut mark = |label: &str| {
			if stamp {
				eprintln!("  {label}: {:?}", t0.elapsed());
				t0 = std::time::Instant::now();
			}
		};
		mark("detect");

		// ---- Static candidates (the law doesn't move in sweeps) ------------
		// Water targets that nothing fits stay water — and once one does,
		// its neighbors see open water there, so their candidates recompute
		// (the loop converges: targets only ever drop out). Candidates sort
		// richest-first: ties seed the generic pieces.
		loop {
			for i in 0..targets.len() {
				let (x, y) = (targets[i].x, targets[i].y);
				if tat[(y * w + x) as usize] < 0 {
					continue; // dropped earlier
				}
				let views: [Nb; 4] = std::array::from_fn(|d| view(x, y, d, &tat, &no_picks));
				let mut candidates = Vec::new();
				for (fi, fam) in families.iter().enumerate() {
					if !fam.band {
						continue; // auto-shore places band tiles only
					}
					for rot in 0..4u8 {
						for mirror in [false, true] {
							let t = Transform { rot, mirror };
							if (0..4).all(|d| dir_ok(fam, t, d, views[d])) {
								candidates.push((fi as u16, t));
							}
						}
					}
				}
				candidates.sort_by_key(|&(fi, t)| (std::cmp::Reverse(families[fi as usize].richness), fi, t.bits()));
				targets[i].candidates = candidates;
			}
			let mut dropped = false;
			for t in &targets {
				let i = (t.y * w + t.x) as usize;
				if t.original.is_none() && t.candidates.is_empty() && tat[i] >= 0 {
					tat[i] = -1;
					dropped = true;
				}
			}
			if !dropped {
				break;
			}
		}

		mark("candidates");
		// ---- Sweeps: maximize unbroken seams -------------------------------
		// A law-breaking original must not survive on tie — start it from
		// nothing so the sweep is forced to choose a legal candidate.
		let mut picks: Vec<Option<(u16, Transform)>> =
			targets.iter().map(|t| t.original.filter(|o| t.candidates.contains(o))).collect();
		// Orphaned shore (nothing fits, no land anywhere near) dissolves;
		// with land near, an unfixable cell keeps what it has.
		for (i, t) in targets.iter().enumerate() {
			if t.candidates.is_empty() {
				picks[i] = if t.near_land { t.original } else { None };
			}
		}
		// Edge semantics + continuations. A `__LAND__` edge against land is
		// worth a seam-pair — that separates straights from corner pieces
		// long before continuations resolve (corner families never list
		// `__LAND__`). Matched continuations stack on top, so unbroken
		// chains win every tie that matters. Sea-only edges facing shore
		// are deliberately NOT penalized: ravine caps (GSc over GSg-columns)
		// and double-thick bands legally point their water edge along the
		// coast.
		let score_views = |vs: &[Nb; 4], fam: u16, t: Transform| {
			let f = &families[fam as usize];
			(0..4)
				.map(|d| {
					let rule = &f.dirs[base_dir(d, t)];
					match vs[d] {
						Nb::Edge | Nb::OpenWater | Nb::Pending => 0,
						Nb::Hard => {
							if rule.land {
								2
							} else {
								0
							}
						}
						Nb::Shore(nf, tn) => cseam(fam, t, d, nf, tn) as i32,
					}
				})
				.sum::<i32>()
		};
		let score = |x: i32, y: i32, fam: u16, t: Transform, picks: &[Option<(u16, Transform)>]| {
			let vs: [Nb; 4] = std::array::from_fn(|d| view(x, y, d, &tat, picks));
			score_views(&vs, fam, t)
		};
		// Worklist: a cell only re-scores when a neighbor changed since its
		// last evaluation (same results as full sweeps, far less work).
		let mut dirty: Vec<bool> = vec![true; targets.len()];
		let mark_around = |x: i32, y: i32, tat: &[i32], dirty: &mut Vec<bool>| {
			for &(dx, dy) in &RING {
				let (nx, ny) = (x + dx, y + dy);
				if nx >= 0 && ny >= 0 && nx < w && ny < h {
					let ti = tat[(ny * w + nx) as usize];
					if ti >= 0 {
						dirty[ti as usize] = true;
					}
				}
			}
		};
		let sweep_once = |picks: &mut Vec<Option<(u16, Transform)>>, dirty: &mut Vec<bool>| -> bool {
			let mut changed = false;
			for (i, target) in targets.iter().enumerate() {
				if !dirty[i] || target.candidates.is_empty() {
					continue;
				}
				dirty[i] = false;
				let (x, y) = (target.x, target.y);
				// One set of neighbor views serves every candidate.
				let vs: [Nb; 4] = std::array::from_fn(|d| view(x, y, d, &tat, picks.as_slice()));
				let current = picks[i];
				// An empty pick always loses — water targets must dress.
				let current_score = current.map(|(f, t)| score_views(&vs, f, t)).unwrap_or(i32::MIN);
				let mut best = current;
				let mut best_score = current_score;
				for &(fi, t) in &target.candidates {
					let s = score_views(&vs, fi, t);
					if s > best_score {
						best = Some((fi, t));
						best_score = s;
					}
				}
				if best != current {
					picks[i] = best;
					changed = true;
					mark_around(x, y, &tat, dirty);
				}
			}
			changed
		};
		// Pair moves: a broken seam two single-cell argmaxes can't fix (the
		// chain must bend on BOTH sides at once — e.g. a ravine cap whose
		// approach piece must change with it) re-picks the two cells
		// jointly. Sweeps settle, pairs unlock, sweeps settle again — the
		// "multi iterations".
		let pair_once = |picks: &mut Vec<Option<(u16, Transform)>>, dirty: &mut Vec<bool>| -> bool {
			let mut changed = false;
			for i in 0..targets.len() {
				let (x, y) = (targets[i].x, targets[i].y);
				for dir in [1usize, 2] {
					// E and S — each adjacent pair visited once.
					let (dx, dy) = RING[dir];
					let (nx, ny) = (x + dx, y + dy);
					if nx < 0 || ny < 0 || nx >= w || ny >= h {
						continue;
					}
					let j = tat[(ny * w + nx) as usize];
					if j < 0 {
						continue;
					}
					let j = j as usize;
					let (Some(a0), Some(b0)) = (picks[i], picks[j]) else { continue };
					if targets[i].candidates.is_empty() || targets[j].candidates.is_empty() {
						continue;
					}
					if cseam(a0.0, a0.1, dir, b0.0, b0.1) > 0 {
						continue; // this seam already flows
					}
					let joint =
						|picks: &mut Vec<Option<(u16, Transform)>>, a: (u16, Transform), b: (u16, Transform)| {
							picks[i] = Some(a);
							picks[j] = Some(b);
							score(x, y, a.0, a.1, picks.as_slice()) + score(nx, ny, b.0, b.1, picks.as_slice())
						};
					// Only pairs that actually CONNECT across this seam can
					// beat the broken pair — enumerate them through the
					// pre-composed lists instead of the full cross
					// product (the speed of the whole pass lives here).
					let mut pairs: Vec<((u16, Transform), (u16, Transform))> = Vec::new();
					for &a in &targets[i].candidates {
						for &(fb, bits) in &comp[a.0 as usize * 8 + a.1.bits() as usize][dir] {
							let tb = Transform { rot: bits & 3, mirror: bits & 4 != 0 };
							if targets[j].candidates.contains(&(fb, tb)) {
								pairs.push((a, (fb, tb)));
							}
						}
					}
					for &b in &targets[j].candidates {
						for &(fa, bits) in &comp[b.0 as usize * 8 + b.1.bits() as usize][opp(dir)] {
							let ta = Transform { rot: bits & 3, mirror: bits & 4 != 0 };
							if targets[i].candidates.contains(&(fa, ta)) {
								pairs.push(((fa, ta), b));
							}
						}
					}
					let mut best = (a0, b0);
					let mut best_score = joint(picks, a0, b0);
					for &(a, b) in &pairs {
						let s = joint(picks, a, b);
						if s > best_score {
							best = (a, b);
							best_score = s;
						}
					}
					picks[i] = Some(best.0);
					picks[j] = Some(best.1);
					if best != (a0, b0) {
						changed = true;
						mark_around(x, y, &tat, &mut *dirty);
						mark_around(nx, ny, &tat, &mut *dirty);
					}
				}
			}
			changed
		};
		// Settle, unlock pairs, settle again — always END sweep-stable.
		for _round in 0..3 {
			for _sweep in 0..MAX_SWEEPS {
				if !sweep_once(&mut picks, &mut dirty) {
					break;
				}
			}
			if !pair_once(&mut picks, &mut dirty) {
				break;
			}
		}
		for _sweep in 0..MAX_SWEEPS {
			if !sweep_once(&mut picks, &mut dirty) {
				break;
			}
		}
		mark("sweeps+pairs");

		// What the tileset couldn't express: target seams with no listed
		// continuation in either direction (each pair counted once).
		let mut unresolved = 0;
		for (i, target) in targets.iter().enumerate() {
			let Some((fa, ta)) = picks[i] else { continue };
			let (x, y) = (target.x, target.y);
			for dir in [1usize, 2] {
				let (dx, dy) = RING[dir];
				let (nx, ny) = (x + dx, y + dy);
				if nx < 0 || ny < 0 || nx >= w || ny >= h {
					continue;
				}
				let ti = tat[(ny * w + nx) as usize];
				let nb = if ti >= 0 {
					picks[ti as usize]
				} else {
					match at(nx, ny) {
						Some(Cell::Ruled { fam, t }) if families[fam as usize].band => Some((fam, t)),
						_ => None,
					}
				};
				let Some((fb, tb)) = nb else { continue };
				if cseam(fa, ta, dir, fb, tb) == 0 {
					unresolved += 1;
				}
			}
		}

		mark("unresolved");
		// ---- Apply ---------------------------------------------------------
		let mut edits: Vec<(u16, u16, usize, Option<TileRef>)> = Vec::new();
		for (i, target) in targets.iter().enumerate() {
			if picks[i] == target.original {
				continue;
			}
			let (x, y) = (target.x as u16, target.y as u16);
			let entry = picks[i].map(|(fi, t)| {
				let fam = &families[fi as usize];
				// Deterministic pixel variety, streamed from the cell.
				let mut rng = Rng::new(0x53484f5245 ^ ((x as u64) << 32 | y as u64));
				let tile = fam.variants[rng.below(fam.variants.len() as u32) as usize];
				TileRef { pack: fam.pack, tile, transform: t }
			});
			edits.push((x, y, LAYER_GROUND, entry));
		}
		for (fx, fy, entry) in fills {
			edits.push((fx, fy, LAYER_GROUND, Some(entry)));
		}
		let count = edits.len();
		self.place_many(&edits); // one batch = one undo unit
		(count, unresolved)
	}

	/// Auto-shore, the loop-walk variant (`shore alt`): instead of the
	/// sweeps' argmax, trace every land/water boundary loop, seed each at
	/// one cell, and walk the loop placing a **random** tile among the
	/// candidates that continue the chain (transform-composed) — one die
	/// roll per cell, so straight runs vary instead of repeating.
	///
	/// Same law, same landfill, same anatomy as `auto_shore` — only the
	/// placement strategy differs. The walk dresses only what the strict
	/// pass would target (water touching land, law-breaking band, corner
	/// closers gated on newly grown flanks), so pristine coastline stays
	/// untouched and a second run changes nothing. Existing valid shore on
	/// the path is a fixed constraint the chain knits into; per cell the
	/// candidate pool narrows in tiers — matches every settled shore
	/// neighbor, else matches the walk predecessor, else law-valid — then
	/// keeps the best land-edge fit (corner pieces at corners, straights on
	/// straights) and rolls the die among what's left. Deterministic:
	/// per-cell splitmix64 streams, row-major loop discovery.
	///
	/// The walk is single-pass, so it can leave a discontinuity where a
	/// cell matched its predecessor but stranded its successor. It does NOT
	/// repair them — `fix_shore` (`shore fix`) is the separate, deliberate,
	/// hard-bounded pass for that, so no paint stroke can hang on a
	/// pathological tileset. The count of those open seams is returned for
	/// the caller to report.
	///
	/// Orphaned shore in open water sits on no boundary loop and is left
	/// alone — run the plain pass to dissolve it. Returns
	/// `(cells changed, seams without a listed continuation)`.
	pub fn auto_shore_alt(&mut self, region: Option<(u16, u16, u16, u16)>) -> (usize, usize) {
		const SALT: u64 = 0x414c_5453_484f_5245; // "ALTSHORE"
		let (w, h) = (self.width as i32, self.height as i32);
		let families = parse_families(self);
		if families.iter().all(|f| !f.band) {
			return (0, 0);
		}
		let stamp = std::env::var("SHORE_TIME").is_ok();
		let mut t0 = std::time::Instant::now();
		let mut mark = |label: &str| {
			if stamp {
				eprintln!("  alt/{label}: {:?}", t0.elapsed());
				t0 = std::time::Instant::now();
			}
		};
		let name_idx: HashMap<&str, u16> =
			families.iter().enumerate().map(|(i, f)| (f.name.as_str(), i as u16)).collect();
		let comp = build_comp(&families);
		let admits = |a: u16, ta: Transform, dir: usize, b: u16, tb: Transform| -> bool {
			comp[a as usize * 8 + ta.bits() as usize][dir].contains(&(b, tb.bits() as u8))
		};
		let cseam = |a: u16, ta: Transform, dir: usize, b: u16, tb: Transform| -> usize {
			admits(a, ta, dir, b, tb) as usize + admits(b, tb, opp(dir), a, ta) as usize
		};

		let mut snap = snapshot_cells(self, &name_idx);
		let rect = region_rect(region, w, h);
		let (x0, y0, x1, y1) = rect;
		let fills = landfill(self, &families, &mut snap, rect);
		let snap = snap;
		let at = |x: i32, y: i32| cell_at(&snap, w, h, x, y);
		let land = |x: i32, y: i32| is_land(&families, &snap, w, h, x, y);
		let in_rect = |x: i32, y: i32| x >= x0 && x <= x1 && y >= y0 && y <= y1;

		// The same law as the sweeps; raw views serve classification.
		let dir_ok = |fam: &Family, t: Transform, dir: usize, nb: Nb| {
			let rule = &fam.dirs[base_dir(dir, t)];
			match nb {
				Nb::Edge | Nb::Pending | Nb::Shore(..) => true,
				Nb::OpenWater => rule.water,
				Nb::Hard => !rule.water_only,
			}
		};
		let nb_raw = |x: i32, y: i32, dir: usize| -> Nb {
			let (dx, dy) = RING[dir];
			match at(x + dx, y + dy) {
				None => Nb::Edge,
				Some(Cell::Water) => Nb::OpenWater,
				Some(Cell::Ruled { fam, t }) if families[fam as usize].band => Nb::Shore(fam, t),
				Some(_) => Nb::Hard,
			}
		};
		let law_valid = |fam: u16, t: Transform, x: i32, y: i32| {
			(0..4).all(|d| dir_ok(&families[fam as usize], t, d, nb_raw(x, y, d)))
		};

		// ---- Trace the boundary loops --------------------------------------
		// Directed edges (land cell, dir → not-land); each edge has one
		// successor walking clockwise around the land (counterclockwise
		// around lakes — the same turn rule), so orbits are closed loops.
		// The not-land cell alongside each edge is a path slot; a convex
		// turn contributes its diagonal as the chain-closing corner slot.
		struct Slot {
			x: i32,
			y: i32,
			/// Convex-corner diagonal, not boundary-adjacent.
			corner: bool,
		}
		let next_edge = |x: i32, y: i32, d: usize| -> (i32, i32, usize) {
			let dr = (d + 1) % 4;
			let (fx, fy) = RING[d];
			let (sx, sy) = RING[dr];
			if land(x + fx + sx, y + fy + sy) {
				(x + fx + sx, y + fy + sy, (d + 3) % 4) // concave turn
			} else if land(x + sx, y + sy) {
				(x + sx, y + sy, d) // straight on
			} else {
				(x, y, dr) // convex turn
			}
		};
		let mut visited = vec![0u8; (w * h) as usize];
		let mut loops: Vec<Vec<Option<Slot>>> = Vec::new();
		for y in 0..h {
			for x in 0..w {
				if !land(x, y) {
					continue;
				}
				for d0 in 0..4 {
					if visited[(y * w + x) as usize] & (1 << d0) != 0 {
						continue;
					}
					let (dx, dy) = RING[d0];
					if land(x + dx, y + dy) {
						continue;
					}
					let mut slots: Vec<Option<Slot>> = Vec::new();
					let (mut cx, mut cy, mut cd) = (x, y, d0);
					loop {
						visited[(cy * w + cx) as usize] |= 1 << cd;
						let (ox, oy) = (cx + RING[cd].0, cy + RING[cd].1);
						slots.push((ox >= 0 && oy >= 0 && ox < w && oy < h).then_some(Slot {
							x: ox,
							y: oy,
							corner: false,
						}));
						let (nx, ny, nd) = next_edge(cx, cy, cd);
						if (nx, ny) == (cx, cy) {
							let (qx, qy) = (cx + RING[cd].0 + RING[nd].0, cy + RING[cd].1 + RING[nd].1);
							if qx >= 0 && qy >= 0 && qx < w && qy < h {
								slots.push(Some(Slot { x: qx, y: qy, corner: true }));
							}
						}
						(cx, cy, cd) = (nx, ny, nd);
						if (cx, cy, cd) == (x, y, d0) {
							break;
						}
					}
					// A concave corner is two consecutive visits of one water
					// cell — collapse them (and the cyclic wrap-around).
					slots.dedup_by(|a, b| match (&a, &b) {
						(Some(a), Some(b)) => (a.x, a.y) == (b.x, b.y),
						(None, None) => true,
						_ => false,
					});
					while slots.len() > 1 {
						let wrap = match (&slots[0], &slots[slots.len() - 1]) {
							(Some(a), Some(b)) => (a.x, a.y) == (b.x, b.y),
							(None, None) => true,
							_ => false,
						};
						if !wrap {
							break;
						}
						slots.pop();
					}
					loops.push(slots);
				}
			}
		}

		// ---- Classify the slots --------------------------------------------
		enum Kind {
			/// Off-map, out of region, or a gate-failed corner — stays as is,
			/// and the chain link resets across it.
			Gap,
			/// Valid shore on the path: a fixed constraint to knit into.
			Fixed,
			/// Dress me: target water or a law-breaking band cell.
			Free,
		}
		let mut plans: Vec<Vec<Kind>> = loops
			.iter()
			.map(|slots| {
				let n = slots.len();
				(0..n)
					.map(|i| {
						let Some(s) = &slots[i] else { return Kind::Gap };
						match at(s.x, s.y).unwrap() {
							Cell::Water if !in_rect(s.x, s.y) => Kind::Gap,
							Cell::Water if s.corner => {
								// Corners close only off a newly grown flank —
								// pristine diagonal contact never grows.
								let flank = |j: usize| {
									matches!(&slots[j], Some(f) if !f.corner
										&& at(f.x, f.y) == Some(Cell::Water)
										&& in_rect(f.x, f.y))
								};
								if flank((i + n - 1) % n) || flank((i + 1) % n) { Kind::Free } else { Kind::Gap }
							}
							Cell::Water => Kind::Free,
							Cell::Ruled { fam, t } => {
								// Band by construction (non-band rules as land).
								if in_rect(s.x, s.y) && !law_valid(fam, t, s.x, s.y) { Kind::Free } else { Kind::Fixed }
							}
							Cell::Plain => unreachable!("path slots are never land"),
						}
					})
					.collect()
			})
			.collect();

		// Free cells become Pending for everyone before any walk decides.
		const ST_FREE: u8 = 1;
		const ST_DECIDED: u8 = 2;
		let mut state = vec![0u8; (w * h) as usize];
		let mut choice: Vec<Option<(u16, Transform)>> = vec![None; (w * h) as usize];
		let mut original: Vec<Option<(u16, Transform)>> = vec![None; (w * h) as usize];
		for (slots, kinds) in loops.iter().zip(&plans) {
			for (slot, kind) in slots.iter().zip(kinds) {
				if let (Some(s), Kind::Free) = (slot, kind) {
					let i = (s.y * w + s.x) as usize;
					state[i] = ST_FREE;
					if let Some(Cell::Ruled { fam, t }) = at(s.x, s.y) {
						original[i] = Some((fam, t));
					}
				}
			}
		}

		// Fringe: a kept boundary tile orthogonally adjacent to a target
		// re-resolves too, so edits knit into the old coast — but it keeps
		// its own tile unless something now fits strictly better (the
		// keep-if-valid rule in the walk). One ring only (decided off the
		// original frees), so pristine maps — which have no frees — never
		// promote and never churn.
		let mut promote: Vec<(usize, usize, usize, (u16, Transform))> = Vec::new();
		for (li, (slots, kinds)) in loops.iter().zip(&plans).enumerate() {
			for (si, (slot, kind)) in slots.iter().zip(kinds).enumerate() {
				let (Some(s), Kind::Fixed) = (slot, kind) else { continue };
				if !in_rect(s.x, s.y) {
					continue;
				}
				let Some(Cell::Ruled { fam, t }) = at(s.x, s.y) else { continue };
				let adj_free = RING.iter().any(|&(dx, dy)| {
					let (nx, ny) = (s.x + dx, s.y + dy);
					nx >= 0 && ny >= 0 && nx < w && ny < h && state[(ny * w + nx) as usize] == ST_FREE
				});
				if adj_free {
					promote.push((li, si, (s.y * w + s.x) as usize, (fam, t)));
				}
			}
		}
		for (li, si, ci, orig) in promote {
			plans[li][si] = Kind::Free;
			state[ci] = ST_FREE;
			original[ci] = Some(orig);
		}

		// The run view: settled cells show their choice, free ones Pending.
		let nb_run = |x: i32, y: i32, dir: usize, state: &[u8], choice: &[Option<(u16, Transform)>]| -> Nb {
			let (dx, dy) = RING[dir];
			let (nx, ny) = (x + dx, y + dy);
			let Some(cell) = at(nx, ny) else { return Nb::Edge };
			let i = (ny * w + nx) as usize;
			match state[i] {
				ST_DECIDED => match choice[i] {
					Some((f, t)) => Nb::Shore(f, t),
					None => Nb::OpenWater, // nothing fit — stays water
				},
				ST_FREE => Nb::Pending,
				_ => match cell {
					Cell::Water => Nb::OpenWater,
					Cell::Ruled { fam, t } if families[fam as usize].band => Nb::Shore(fam, t),
					_ => Nb::Hard,
				},
			}
		};

		// ---- Walk each loop --------------------------------------------------
		let mut decided: Vec<(i32, i32)> = Vec::new();
		for (slots, kinds) in loops.iter().zip(&plans) {
			let n = slots.len();
			if !kinds.iter().any(|k| matches!(k, Kind::Free)) {
				continue;
			}
			// Seed after existing shore or a gap; a fully free closed loop
			// (a fresh island) starts at a random cell.
			let start = (0..n).find(|&i| !matches!(kinds[(i + n - 1) % n], Kind::Free)).unwrap_or_else(|| {
				let (my, mx) = slots.iter().flatten().map(|s| (s.y, s.x)).min().unwrap();
				let mut rng = Rng::new(SALT ^ 0x4c4f_4f50 ^ ((mx as u64) << 32 | my as u64));
				rng.below(n as u32) as usize
			});
			let before = (start + n - 1) % n;
			let mut prev: Option<(i32, i32)> = match (&slots[before], &kinds[before]) {
				(Some(s), Kind::Fixed) => Some((s.x, s.y)),
				_ => None,
			};
			for k in 0..n {
				let i = (start + k) % n;
				let Some(s) = &slots[i] else {
					prev = None;
					continue;
				};
				match kinds[i] {
					Kind::Gap => prev = None,
					Kind::Fixed => prev = Some((s.x, s.y)),
					Kind::Free => {
						let ci = (s.y * w + s.x) as usize;
						if state[ci] == ST_DECIDED {
							// Settled from another loop — a fixed link now.
							prev = choice[ci].is_some().then_some((s.x, s.y));
							continue;
						}
						let views: [Nb; 4] = std::array::from_fn(|d| nb_run(s.x, s.y, d, &state, &choice));
						let mut lawful: Vec<(u16, Transform)> = Vec::new();
						for (fi, fam) in families.iter().enumerate() {
							if !fam.band {
								continue;
							}
							for rot in 0..4u8 {
								for mirror in [false, true] {
									let t = Transform { rot, mirror };
									if (0..4).all(|d| dir_ok(fam, t, d, views[d])) {
										lawful.push((fi as u16, t));
									}
								}
							}
						}
						// Tiers, strongest first. Two facts drive the pool:
						// settled shore neighbors must be MATCHED (cseam), and
						// the loop's own predecessor/successor slots WILL be
						// shore — toward a still-pending one the tile must
						// present a continuation edge (else a corner picks a
						// water-sided tip and the next cell can't connect).
						let shore_dirs: Vec<(usize, u16, Transform)> = (0..4)
							.filter_map(|d| match views[d] {
								Nb::Shore(f, t) => Some((d, f, t)),
								_ => None,
							})
							.collect();
						// Loop-link directions (prev/next slot), if orthogonal
						// and not a gap — those neighbors are or become shore.
						let link_dir = |j: usize| -> Option<usize> {
							if matches!(kinds[j], Kind::Gap) {
								return None;
							}
							let s2 = slots[j].as_ref()?;
							(0..4).find(|&d| (s.x + RING[d].0, s.y + RING[d].1) == (s2.x, s2.y))
						};
						let links = [link_dir((i + n - 1) % n), link_dir((i + 1) % n)];
						let pending_links: Vec<usize> =
							links.iter().flatten().copied().filter(|&d| matches!(views[d], Nb::Pending)).collect();
						let cont_capable = |f: u16, t: Transform, d: usize| {
							!families[f as usize].dirs[base_dir(d, t)].tiles.is_empty()
						};
						let matches_shore =
							|f: u16, t: Transform| shore_dirs.iter().all(|&(d, nf, nt)| cseam(f, t, d, nf, nt) > 0);
						let prev_dir =
							prev.and_then(|(px, py)| (0..4).find(|&d| (s.x + RING[d].0, s.y + RING[d].1) == (px, py)));
						// Tier 1: match settled shore AND stay shore-open toward
						// pending loop links.
						let mut pool: Vec<(u16, Transform)> = lawful
							.iter()
							.copied()
							.filter(|&(f, t)| {
								matches_shore(f, t) && pending_links.iter().all(|&d| cont_capable(f, t, d))
							})
							.collect();
						// Keep-if-valid: a re-resolved old tile that still sits
						// in the strongest tier stays put — no coastline churn,
						// and idempotence falls out. Fresh water (no original)
						// always rolls, so straights vary.
						if let Some(orig) = original[ci].filter(|o| pool.contains(o)) {
							state[ci] = ST_DECIDED;
							choice[ci] = Some(orig);
							decided.push((s.x, s.y));
							prev = Some((s.x, s.y));
							continue;
						}
						// Tier 2: match settled shore (drop the open-edge wish).
						if pool.is_empty() {
							pool = lawful.iter().copied().filter(|&(f, t)| matches_shore(f, t)).collect();
						}
						// Tier 3: at least continue the walk predecessor.
						if pool.is_empty() {
							if let Some(d) = prev_dir {
								if let Nb::Shore(nf, nt) = views[d] {
									pool =
										lawful.iter().copied().filter(|&(f, t)| cseam(f, t, d, nf, nt) > 0).collect();
								}
							}
						}
						if pool.is_empty() {
							pool = lawful;
						}
						// Land edges belong against land: keep the best fit
						// (corner pieces at corners, straights on straights),
						// then roll the die among what's left.
						let fitness = |&(fi, t): &(u16, Transform)| -> i32 {
							(0..4)
								.map(|d| {
									let rule = &families[fi as usize].dirs[base_dir(d, t)];
									match (rule.land, matches!(views[d], Nb::Hard)) {
										(true, true) => 1,
										(true, false) => -1,
										_ => 0,
									}
								})
								.sum()
						};
						let best = pool.iter().map(fitness).max();
						pool.retain(|c| Some(fitness(c)) == best);
						let pick = match pool.len() {
							0 => original[ci], // law-broken band keeps itself
							1 => Some(pool[0]),
							_ => {
								let mut rng = Rng::new(SALT ^ ((s.x as u64) << 32 | s.y as u64));
								Some(pool[rng.below(pool.len() as u32) as usize])
							}
						};
						state[ci] = ST_DECIDED;
						choice[ci] = pick;
						decided.push((s.x, s.y));
						prev = pick.is_some().then_some((s.x, s.y));
					}
				}
			}
		}
		mark("walk");

		// Seams the walk couldn't close (the sweeps' report semantics).
		// A discontinuity the single-pass walk strands here is closed by the
		// separate `fix_shore` pass — run it deliberately (`shore fix`); it
		// is not chained on automatically, so a pathological tileset can
		// never make a paint stroke hang.
		let mut unresolved = 0;
		for &(x, y) in &decided {
			let Some((fa, ta)) = choice[(y * w + x) as usize] else { continue };
			for dir in 0..4 {
				let (dx, dy) = RING[dir];
				let (nx, ny) = (x + dx, y + dy);
				if nx < 0 || ny < 0 || nx >= w || ny >= h {
					continue;
				}
				let ni = (ny * w + nx) as usize;
				let nb = if state[ni] == ST_DECIDED {
					if dir == 0 || dir == 3 {
						continue; // settled pairs count once, on their E/S side
					}
					choice[ni]
				} else {
					match at(nx, ny) {
						Some(Cell::Ruled { fam, t }) if families[fam as usize].band => Some((fam, t)),
						_ => None,
					}
				};
				let Some((fb, tb)) = nb else { continue };
				if cseam(fa, ta, dir, fb, tb) == 0 {
					unresolved += 1;
				}
			}
		}

		// ---- Apply ---------------------------------------------------------
		let mut edits: Vec<(u16, u16, usize, Option<TileRef>)> = Vec::new();
		for &(x, y) in &decided {
			let ci = (y * w + x) as usize;
			if choice[ci] == original[ci] {
				continue;
			}
			let entry = choice[ci].map(|(fi, t)| {
				let fam = &families[fi as usize];
				// The same pixel-variant stream as the sweeps.
				let mut rng = Rng::new(0x53484f5245 ^ ((x as u64) << 32 | y as u64));
				let tile = fam.variants[rng.below(fam.variants.len() as u32) as usize];
				TileRef { pack: fam.pack, tile, transform: t }
			});
			edits.push((x as u16, y as u16, LAYER_GROUND, entry));
		}
		for (fx, fy, entry) in fills {
			edits.push((fx, fy, LAYER_GROUND, Some(entry)));
		}
		let count = edits.len();
		self.place_many(&edits); // one batch = one undo unit
		(count, unresolved)
	}

	/// Fix-shore (`shore fix`): a deliberate, **separate** pass that re-tiles
	/// existing shore to remove the discontinuities the single-pass walks
	/// (or hand-editing, or a user's own tileset) leave behind — where two
	/// adjacent shore tiles don't continue across their seam. It is NOT
	/// chained onto `auto_shore`/`auto_shore_alt`, so no paint stroke can
	/// ever hang on a pathological tileset; the cost lives only here, and is
	/// hard-bounded.
	///
	/// Only the **band (shore) cells** in `region` are re-tileable — land,
	/// water, and off-region coast are fixed constraints, never touched. For
	/// each broken seam it gathers a small SPATIAL window of shore cells
	/// (BFS, ≤`WMAX`) and re-solves it with branch-and-bound on the *exact*
	/// broken-seam count — spatial (not 1-D) so a coastline step, where two
	/// columns of shore touch and the breaks are 2-D, is fixable. Two hard
	/// caps guarantee termination on ANY tile data: a per-window node cap
	/// (`PER_WIN`, bounds proving an impossible spot unfixable) and a total
	/// budget (scales with the break count; `SHORE_REPAIR_BUDGET` overrides,
	/// `SHORE_TIME=1` prints timing). Only STRICT improvements are applied,
	/// so a clean coast never churns, the pass is deterministic and
	/// idempotent, and it is one undo unit. Returns
	/// `(cells changed, broken seams remaining)`.
	pub fn fix_shore(&mut self, region: Option<(u16, u16, u16, u16)>) -> (usize, usize) {
		// Drive the resumable session to convergence in one shot. The total
		// budget scales with the break count and hard-bounds the work on any
		// tileset; `SHORE_REPAIR_BUDGET` overrides, `SHORE_TIME=1` times it.
		let stamp = std::env::var("SHORE_TIME").is_ok();
		let t0 = std::time::Instant::now();
		let mut session = FixSession::new(self, region, FixStrength::Shore);
		let found = session.found();
		let mut budget: i64 = std::env::var("SHORE_REPAIR_BUDGET")
			.ok()
			.and_then(|v| v.parse().ok())
			.unwrap_or_else(|| (found as i64 * 100_000).clamp(500_000, 20_000_000));
		while !session.is_done() && budget > 0 {
			let spent = session.step(budget);
			budget -= spent;
			if spent == 0 {
				break;
			}
		}
		let remaining = session.remaining();
		if stamp {
			eprintln!("  fix_shore: {found} broken -> {remaining} in {:?}", t0.elapsed());
		}
		let count = session.apply(self);
		(count, remaining)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn assets_root() -> std::path::PathBuf {
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets")
	}

	/// Hand-computed direction table: a feature on the base tile's N edge
	/// faces E after one cw turn; a horizontal mirror swaps E and W.
	#[test]
	fn base_dir_matches_pixel_convention() {
		let t = |rot, mirror| Transform { rot, mirror };
		// Identity.
		assert_eq!(base_dir(0, t(0, false)), 0);
		// 1 cw: screen E shows the base N edge.
		assert_eq!(base_dir(1, t(1, false)), 0);
		// 2 cw: screen S shows the base N edge.
		assert_eq!(base_dir(2, t(2, false)), 0);
		// Mirror only: N/S fixed, E/W swapped.
		assert_eq!(base_dir(0, t(0, true)), 0);
		assert_eq!(base_dir(1, t(0, true)), 3);
		assert_eq!(base_dir(2, t(0, true)), 2);
		assert_eq!(base_dir(3, t(0, true)), 1);
		// Mirror + 1 cw (`:!E`): screen E shows the base N edge mirrored…
		assert_eq!(base_dir(1, t(1, true)), 0);
		// …and screen S shows the base W edge (mirror swapped E/W, then the
		// cw turn carried W around to S).
		assert_eq!(base_dir(2, t(1, true)), 3);
	}

	/// Continuations compose transforms: rotating a pair together keeps the
	/// listed continuation, rotating one side alone breaks it. The shipped
	/// lists are one-directional, and bare `:!` parses as mirror.
	#[test]
	fn continuations_follow_transforms() {
		let root = assets_root();
		let p = Project::new(4, 4, &["GREEN".to_string()], &root, 1).unwrap();
		let families = parse_families(&p);
		let idx = |n: &str| families.iter().position(|f| f.name == n).unwrap() as u16;
		let (gsh, gsi, gse) = (idx("GSh"), idx("GSi"), idx("GSe"));
		let id = Transform::default();
		// Straight shore continues E: GSh lists GSi in base orientation…
		assert!(continues(&families, gsh, id, 1, gsi, id));
		// …and the whole pair rotated 1 cw continues S.
		let cw = Transform { rot: 1, mirror: false };
		assert!(continues(&families, gsh, cw, 2, gsi, cw));
		// A rotated tile does NOT continue an unrotated one sideways.
		assert!(!continues(&families, gsh, cw, 1, gsi, id));
		// One-directional: GSi's W does not list GSh.
		assert!(!continues(&families, gsi, id, 3, gsh, id));
		assert_eq!(seam_score(&families, gsh, id, 1, gsi, id), 1);
		// `GSe:!` is GSe MIRRORED — it must not admit plain GSe (the parse
		// regression behind the saw-pattern bug).
		let m = Transform { rot: 0, mirror: true };
		assert!(continues(&families, gse, id, 1, gse, m), "GSe lists GSe:! eastward");
		assert!(!continues(&families, gse, id, 1, gse, id), "but not plain GSe");
	}

	/// Paint a 3×3 island into open water — the shore grows on the WATER
	/// ring around it (16 cells incl. corners), the land stays untouched,
	/// every seam along the ring is unbroken, and the pass settles.
	#[test]
	fn shores_the_water_ring_around_an_island() {
		let root = assets_root();
		let mut p = Project::new(9, 9, &["GREEN".to_string()], &root, 42).unwrap();
		let land = p.resolve_ref("GLa000").unwrap().0;
		p.begin_stroke();
		for y in 3..6 {
			for x in 3..6 {
				p.place(x, y, LAYER_GROUND, Some(land));
			}
		}
		p.end_stroke();
		let painted = p.hash();

		let (changed, unresolved) = p.auto_shore(None);
		assert_eq!(changed, 16, "the 16-cell water ring");
		assert_eq!(unresolved, 0, "a square island is fully expressible");

		// Land untouched.
		for y in 3..6 {
			for x in 3..6 {
				let top = p.cell(x, y).unwrap()[LAYER_GROUND].unwrap();
				assert_eq!(p.packs[top.pack as usize].ids[top.tile as usize], "GLa000");
			}
		}
		// The ring wears ruled shore tiles with every seam unbroken.
		let families = parse_families(&p);
		let shore_of = |x: u16, y: u16| -> (u16, Transform) {
			let top = p.cell(x, y).unwrap()[LAYER_GROUND].expect("ring cell has shore");
			let name = family_of(&p.packs[top.pack as usize].ids[top.tile as usize]).to_string();
			(families.iter().position(|f| f.name == name).expect("ruled") as u16, top.transform)
		};
		let ring: Vec<(u16, u16)> = (2..7)
			.flat_map(|i| [(i, 2), (i, 6), (2, i), (6, i)])
			.filter(|&(x, y)| !(3..6).contains(&x) || !(3..6).contains(&y))
			.collect::<std::collections::HashSet<_>>()
			.into_iter()
			.collect();
		assert_eq!(ring.len(), 16);
		for &(x, y) in &ring {
			let (fa, ta) = shore_of(x, y);
			for (dir, &(dx, dy)) in RING.iter().enumerate() {
				let (nx, ny) = (x as i32 + dx, y as i32 + dy);
				if ring.contains(&(nx as u16, ny as u16)) {
					let (fb, tb) = shore_of(nx as u16, ny as u16);
					assert!(seam_score(&families, fa, ta, dir, fb, tb) > 0, "broken seam ({x},{y})→({nx},{ny})",);
				}
			}
		}

		// Determinism + idempotence + one undo unit.
		assert_eq!(p.auto_shore(None).0, 0, "second pass settles");
		let shored = p.hash();
		assert!(p.undo());
		assert_eq!(p.hash(), painted, "auto-shore undoes as one unit");
		p.redo();
		assert_eq!(p.hash(), shored, "and replays deterministically");
	}

	/// Straight runs must not alternate mirrored pieces (the "saw"): a long
	/// coast seeds from the continuation-richest family and chains
	/// plain-to-plain.
	#[test]
	fn straight_runs_do_not_saw() {
		let root = assets_root();
		let mut p = Project::new(16, 8, &["GREEN".to_string()], &root, 7).unwrap();
		let land = p.resolve_ref("GLa000").unwrap().0;
		for x in 0..16 {
			for y in 4..8 {
				p.place(x, y, LAYER_GROUND, Some(land));
			}
		}
		p.auto_shore(None);
		// Mid-run shore cells (away from map edges) all share one family
		// and one transform — uniform, not alternating.
		let mut seen = std::collections::HashSet::new();
		for x in 3..13u16 {
			let top = p.cell(x, 3).unwrap()[LAYER_GROUND].expect("shore");
			let fam = family_of(&p.packs[top.pack as usize].ids[top.tile as usize]).to_string();
			seen.insert((fam, top.transform.bits()));
		}
		assert_eq!(seen.len(), 1, "uniform straight run, got {seen:?}");
	}

	/// A 1-wide notch is unshoreable (no family takes land on 3 sides) —
	/// it closes into the terrain and the coast runs straight over it.
	#[test]
	fn impossible_notch_closes_into_terrain() {
		let root = assets_root();
		let mut p = Project::new(12, 10, &["GREEN".to_string()], &root, 42).unwrap();
		let land = p.resolve_ref("GLa000").unwrap().0;
		// Land below y=5, except a 1-wide notch column at x=6 (water at y=5).
		for x in 0..12u16 {
			for y in 5..10u16 {
				if (x, y) == (6, 5) {
					continue;
				}
				p.place(x, y, LAYER_GROUND, Some(land));
			}
		}
		let (changed, unresolved) = p.auto_shore(None);
		assert!(changed > 0);
		assert_eq!(unresolved, 0, "the closed notch leaves no broken seams");
		// The notch filled with the neighboring terrain family.
		let top = p.cell(6, 5).unwrap()[LAYER_GROUND].expect("notch filled");
		assert_eq!(family_of(&p.packs[top.pack as usize].ids[top.tile as usize]), "GLa");
		assert_eq!(p.auto_shore(None).0, 0, "idempotent after the fill");
	}

	/// A 1×1 pond (no family takes land on all sides) closes into the
	/// terrain instead of staying a hole the tileset can't rim.
	#[test]
	fn tiny_pond_closes_into_terrain() {
		let root = assets_root();
		let mut p = Project::new(11, 11, &["GREEN".to_string()], &root, 42).unwrap();
		let land = p.resolve_ref("GLa000").unwrap().0;
		for x in 0..11u16 {
			for y in 0..11u16 {
				if (x, y) == (5, 5) {
					continue;
				}
				p.place(x, y, LAYER_GROUND, Some(land));
			}
		}
		let (changed, unresolved) = p.auto_shore(None);
		assert_eq!((changed, unresolved), (1, 0), "just the fill, no seams");
		let top = p.cell(5, 5).unwrap()[LAYER_GROUND].expect("pond filled");
		assert_eq!(family_of(&p.packs[top.pack as usize].ids[top.tile as usize]), "GLa");
		assert_eq!(p.auto_shore(None).0, 0, "and settles");
	}

	/// The loop walk dresses the same 16-cell ring as the sweeps — every
	/// seam a listed continuation, one undo unit, idempotent.
	#[test]
	fn alt_shores_the_island_ring() {
		let root = assets_root();
		let mut p = Project::new(9, 9, &["GREEN".to_string()], &root, 42).unwrap();
		let land = p.resolve_ref("GLa000").unwrap().0;
		for y in 3..6 {
			for x in 3..6 {
				p.place(x, y, LAYER_GROUND, Some(land));
			}
		}
		let painted = p.hash();

		let (changed, unresolved) = p.auto_shore_alt(None);
		assert_eq!(changed, 16, "the 16-cell water ring");
		assert_eq!(unresolved, 0, "a square island is fully expressible");

		// Land untouched, ring all matched (the loop closure included).
		let families = parse_families(&p);
		let shore_of = |x: u16, y: u16| -> (u16, Transform) {
			let top = p.cell(x, y).unwrap()[LAYER_GROUND].expect("ring cell has shore");
			let name = family_of(&p.packs[top.pack as usize].ids[top.tile as usize]).to_string();
			(families.iter().position(|f| f.name == name).expect("ruled") as u16, top.transform)
		};
		let ring: Vec<(u16, u16)> = (2..7)
			.flat_map(|i| [(i, 2), (i, 6), (2, i), (6, i)])
			.filter(|&(x, y)| !(3..6).contains(&x) || !(3..6).contains(&y))
			.collect::<std::collections::HashSet<_>>()
			.into_iter()
			.collect();
		for &(x, y) in &ring {
			let (fa, ta) = shore_of(x, y);
			for (dir, &(dx, dy)) in RING.iter().enumerate() {
				let (nx, ny) = (x as i32 + dx, y as i32 + dy);
				if ring.contains(&(nx as u16, ny as u16)) {
					let (fb, tb) = shore_of(nx as u16, ny as u16);
					assert!(seam_score(&families, fa, ta, dir, fb, tb) > 0, "broken seam ({x},{y})→({nx},{ny})",);
				}
			}
		}

		// Idempotent, one undo unit.
		assert_eq!(p.auto_shore_alt(None).0, 0, "second pass settles");
		let shored = p.hash();
		assert!(p.undo());
		assert_eq!(p.hash(), painted, "one undo unit");
		p.redo();
		assert_eq!(p.hash(), shored, "replays deterministically");
	}

	/// The walk's whole point: a long straight run VARIES (several distinct
	/// family/transform picks) while every seam stays a listed continuation
	/// — where the sweeps version pins one uniform tile.
	#[test]
	fn alt_straight_runs_vary_but_connect() {
		let root = assets_root();
		let mut p = Project::new(24, 8, &["GREEN".to_string()], &root, 7).unwrap();
		let land = p.resolve_ref("GLa000").unwrap().0;
		for x in 0..24 {
			for y in 4..8 {
				p.place(x, y, LAYER_GROUND, Some(land));
			}
		}
		let (_, unresolved) = p.auto_shore_alt(None);
		assert_eq!(unresolved, 0, "the straight coast connects everywhere");

		let families = parse_families(&p);
		let mut seen = std::collections::HashSet::new();
		let mut row = Vec::new();
		for x in 0..24u16 {
			let top = p.cell(x, 3).unwrap()[LAYER_GROUND].expect("shore");
			let name = family_of(&p.packs[top.pack as usize].ids[top.tile as usize]).to_string();
			let fam = families.iter().position(|f| f.name == name).unwrap() as u16;
			seen.insert((fam, top.transform.bits()));
			row.push((fam, top.transform));
		}
		assert!(seen.len() > 1, "the run varies, got {seen:?}");
		for x in 0..23 {
			let (fa, ta) = row[x];
			let (fb, tb) = row[x + 1];
			assert!(seam_score(&families, fa, ta, 1, fb, tb) > 0, "broken seam at x={x}",);
		}
	}

	/// Extending a shored island re-shores only the new coast, knitting
	/// into the kept shore with matched seams.
	#[test]
	fn alt_knits_into_existing_shore() {
		let root = assets_root();
		let mut p = Project::new(14, 9, &["GREEN".to_string()], &root, 42).unwrap();
		let land = p.resolve_ref("GLa000").unwrap().0;
		for y in 3..6 {
			for x in 3..6 {
				p.place(x, y, LAYER_GROUND, Some(land));
			}
		}
		p.auto_shore_alt(None);
		// Grow the island east and re-run: the old west coast keeps its
		// tiles, the new east coast connects through.
		let west_before: Vec<_> = (2..7).map(|y| p.cell_spec(2, y).unwrap()).collect();
		for y in 3..6 {
			for x in 6..9 {
				p.place(x, y, LAYER_GROUND, Some(land));
			}
		}
		let (changed, unresolved) = p.auto_shore_alt(None);
		assert!(changed > 0, "the new coast dresses");
		assert_eq!(unresolved, 0, "and knits without breaks");
		let west_after: Vec<_> = (2..7).map(|y| p.cell_spec(2, y).unwrap()).collect();
		assert_eq!(west_before, west_after, "the kept coast does not churn");
		assert_eq!(p.auto_shore_alt(None).0, 0, "and settles");
	}

	/// The alt region form only touches the rectangle (+1 ring).
	#[test]
	fn alt_region_limits_the_pass() {
		let root = assets_root();
		let mut p = Project::new(16, 8, &["GREEN".to_string()], &root, 1).unwrap();
		let land = p.resolve_ref("GLa000").unwrap().0;
		p.place(3, 3, LAYER_GROUND, Some(land));
		p.place(3, 4, LAYER_GROUND, Some(land));
		p.place(12, 3, LAYER_GROUND, Some(land));
		p.place(12, 4, LAYER_GROUND, Some(land));
		let snapshot = |p: &Project| -> Vec<String> {
			(10..16).flat_map(|x| (2..6).map(|y| p.cell_spec(x, y).unwrap()).collect::<Vec<_>>()).collect()
		};
		let right_before = snapshot(&p);
		let (changed, _) = p.auto_shore_alt(Some((2, 2, 4, 5)));
		assert!(changed > 0, "left stub shored");
		assert_eq!(right_before, snapshot(&p), "right stub untouched");
	}

	/// `fix_shore` closes the discontinuities the greedy walk leaves on a
	/// steep coast. This `sin(x/3)·8` coastline is fully expressible, but
	/// the single-pass `auto_shore_alt` strands 3 broken seams at its folds
	/// (it no longer auto-repairs — `unresolved == 3`); the separate
	/// `fix_shore` pass re-tiles those local windows down to zero. It is one
	/// undo unit and idempotent (a second pass changes nothing).
	#[test]
	fn fix_shore_closes_steep_coast() {
		let root = assets_root();
		let (w, h) = (48u16, 36u16);
		let mut p = Project::new(w, h, &["GREEN".to_string()], &root, 5).unwrap();
		let land = p.resolve_ref("GLa000").unwrap().0;
		for x in 0..w {
			let top = (h as f64 / 2.0 + 8.0 * (x as f64 / 3.0).sin()).round() as u16;
			for y in top.min(h)..h {
				p.place(x, y, LAYER_GROUND, Some(land));
			}
		}
		// The walk alone leaves broken seams (no automatic repair).
		let (_, walk_unresolved) = p.auto_shore_alt(None);
		assert!(walk_unresolved > 0, "the steep walk strands seams");
		let shored = p.hash();

		let (changed, remaining) = p.fix_shore(None);
		assert!(changed > 0, "fix_shore re-tiled the broken windows");
		assert_eq!(remaining, 0, "and closed every seam");

		// One undo unit, deterministic, and a second pass settles.
		let fixed = p.hash();
		assert!(p.undo());
		assert_eq!(p.hash(), shored, "fix_shore stays inside one batch");
		p.redo();
		assert_eq!(p.hash(), fixed);
		assert_eq!(p.fix_shore(None).0, 0, "idempotent after the fix");
	}

	/// The resumable session reaches the same result as `fix_shore`,
	/// stepping in tiny budget slices — what the Auto Fix Shore modal does
	/// frame by frame (live found/fixed/remaining, then apply as one unit).
	#[test]
	fn fix_session_steps_to_the_same_result() {
		let root = assets_root();
		let (w, h) = (48u16, 36u16);
		let mut p = Project::new(w, h, &["GREEN".to_string()], &root, 5).unwrap();
		let land = p.resolve_ref("GLa000").unwrap().0;
		for x in 0..w {
			let top = (h as f64 / 2.0 + 8.0 * (x as f64 / 3.0).sin()).round() as u16;
			for y in top.min(h)..h {
				p.place(x, y, LAYER_GROUND, Some(land));
			}
		}
		p.auto_shore_alt(None);
		let shored = p.hash();

		let mut s = p.fix_session(None, FixStrength::Shore);
		assert!(s.found() > 0, "the steep walk left broken seams");
		let mut guard = 0;
		while !s.is_done() {
			s.step(2_000); // tiny slices, like a frame budget
			assert!(s.fixed() <= s.found());
			guard += 1;
			assert!(guard < 100_000, "session must converge");
		}
		assert_eq!(s.remaining(), 0, "the steep coast fully closes");
		let changed = s.apply(&mut p);
		assert!(changed > 0);
		assert_ne!(p.hash(), shored, "the fix changed the map");
		assert!(p.undo());
		assert_eq!(p.hash(), shored, "applied as one undo unit");
	}

	/// Destructive mode sees and resolves what the band-only modes cannot:
	/// a lone shore tile stranded in open water has no band neighbors, so
	/// Shore/Mangle find nothing — Destructive counts its seams against the
	/// fixed water and flattens the orphan back into the sea.
	#[test]
	fn destructive_resolves_what_band_modes_cannot_see() {
		let root = assets_root();
		let mut p = Project::new(16, 16, &["GREEN".to_string()], &root, 7).unwrap();
		// One shore tile alone in open water: at least one edge of any
		// orientation faces water it doesn't admit (a shore band always has
		// a land side), so some seam against fixed water is broken.
		let (shore, layer) = p.resolve_ref("GSd004").unwrap();
		p.place(8, 8, layer, Some(shore));

		let band_only = p.fix_session(None, FixStrength::Shore);
		assert_eq!(band_only.found(), 0, "band-only accounting can't see the orphan");

		let mut s = p.fix_session(None, FixStrength::Destructive);
		assert!(s.found() > 0, "destructive counts seams against fixed water");
		let mut guard = 0;
		while !s.is_done() {
			s.step(2_000);
			guard += 1;
			assert!(guard < 100_000, "destructive session must converge");
		}
		assert_eq!(s.remaining(), 0, "total freedom always closes");
		let changed = s.apply(&mut p);
		assert!(changed > 0, "the orphan was rewritten");
		let cleared = p.fix_session(None, FixStrength::Destructive);
		assert_eq!(cleared.found(), 0, "the applied result is seam-free");
	}

	/// The destructive escalation: blasting claims a 3×3 into the re-tile
	/// set as open water (region-clamped), each cell at most once — and the
	/// session still converges to a seam-free result afterwards.
	#[test]
	fn destructive_blast_claims_water_once_then_regrows() {
		let root = assets_root();
		let (w, h) = (24u16, 24u16);
		let mut p = Project::new(w, h, &["GREEN".to_string()], &root, 5).unwrap();
		let land = p.resolve_ref("GLa000").unwrap().0;
		for x in 0..w {
			for y in 12..h {
				p.place(x, y, LAYER_GROUND, Some(land));
			}
		}
		p.auto_shore(None);

		let mut s = p.fix_session(None, FixStrength::Destructive);
		// Force-escalate a coast spot: the 3×3 becomes water trials in the
		// re-tile set; a second blast on the same spot is a no-op.
		assert!(s.blast(6, 12), "first blast changes the trial state");
		for dy in -1..=1i32 {
			for dx in -1..=1i32 {
				let ci = ((12 + dy) * s.w + (6 + dx)) as usize;
				assert_eq!(s.work[ci], Some(Trial::Water), "3x3 flattened");
			}
		}
		assert!(!s.blast(6, 12), "each cell blasts at most once");

		// The session was born clean (done) — waking it after the manual
		// blast mirrors the real flow, where blast happens mid-run.
		s.done = false;

		// The hole it tore is repaired by the normal passes.
		let mut guard = 0;
		while !s.is_done() {
			s.step(5_000);
			guard += 1;
			assert!(guard < 100_000, "must converge after a blast");
		}
		assert_eq!(s.remaining(), 0, "the shore regrows around the blast");
		s.apply(&mut p);
		assert_eq!(p.fix_session(None, FixStrength::Destructive).found(), 0);
	}

	/// `fix_shore` leaves a clean coast alone: shoring an island, then
	/// fixing it, changes nothing (no churn) — it only ever applies strict
	/// improvements.
	#[test]
	fn fix_shore_leaves_clean_coast_alone() {
		let root = assets_root();
		let mut p = Project::new(9, 9, &["GREEN".to_string()], &root, 42).unwrap();
		let land = p.resolve_ref("GLa000").unwrap().0;
		for y in 3..6 {
			for x in 3..6 {
				p.place(x, y, LAYER_GROUND, Some(land));
			}
		}
		p.auto_shore_alt(None);
		let clean = p.hash();
		let (changed, remaining) = p.fix_shore(None);
		assert_eq!((changed, remaining), (0, 0), "nothing to fix on a clean ring");
		assert_eq!(p.hash(), clean);
	}

	/// Orphaned shore — the island it hugged is gone — dissolves to water.
	#[test]
	fn orphaned_shore_dissolves() {
		let root = assets_root();
		let mut p = Project::new(9, 9, &["GREEN".to_string()], &root, 42).unwrap();
		let shore = p.resolve_ref("GSh000").unwrap().0;
		p.place(4, 4, LAYER_GROUND, Some(shore));
		assert_eq!(p.auto_shore(None).0, 1);
		assert_eq!(p.cell(4, 4).unwrap()[LAYER_GROUND], None, "back to open water");
	}

	/// The region form only touches the rectangle (+1 ring).
	#[test]
	fn region_limits_the_pass() {
		let root = assets_root();
		let mut p = Project::new(16, 8, &["GREEN".to_string()], &root, 1).unwrap();
		let land = p.resolve_ref("GLa000").unwrap().0;
		p.place(3, 3, LAYER_GROUND, Some(land));
		p.place(3, 4, LAYER_GROUND, Some(land));
		p.place(12, 3, LAYER_GROUND, Some(land));
		p.place(12, 4, LAYER_GROUND, Some(land));
		let snapshot = |p: &Project| -> Vec<String> {
			(10..16).flat_map(|x| (2..6).map(|y| p.cell_spec(x, y).unwrap()).collect::<Vec<_>>()).collect()
		};
		let right_before = snapshot(&p);
		let (changed, _) = p.auto_shore(Some((2, 2, 4, 5)));
		assert!(changed > 0, "left stub shored");
		assert_eq!(right_before, snapshot(&p), "right stub untouched");
	}
}
