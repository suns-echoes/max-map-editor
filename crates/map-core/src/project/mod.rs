//! Map project - the editor's primary document.
//!
//! v1 format: `resources/**/maps/*.json` - see `docs/design/tileset-contract.md`
//! §3. Each cell is a bottom-up stack (water layer, ground layer); tile refs
//! carry a transform (rotation + mirror). `compose_cell` flattens a stack to
//! raw pixels - the kernel of the future WRL export bake, and the
//! thing the 24-map equivalence test verifies against original WRLs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use max_assets::save::{CONNECTOR_BITS, SaveFile, SaveSettings, UnitValues, connector_neighbor, read_save_bytes};
use max_assets::wrl::{TILE_DATA_SIZE, TILE_SIZE, WrlFile};

use crate::pack::TilePack;
use crate::scenery::{SceneryPack, SceneryPiece, ScenerySpot};

mod palette_reimport;
mod save_session;
pub(crate) use save_session::objects_from_save;
mod serde;

pub use palette_reimport::PaletteReimport;

pub const LAYER_WATER: usize = 0;
pub const LAYER_GROUND: usize = 1;
pub const MAX_LAYERS: usize = 2;

/// The project-file format version this editor writes, `"MAJOR.MINOR"`, stored
/// under `"mme_project_file_version"`. The **MAJOR** is the compatibility
/// guard: a file with the same MAJOR opens and is migrated up to this MINOR; a
/// different MAJOR is unsupported (a hard break). A pre-scheme `"version": "1"`
/// is grandfathered in and migrated to this version.
///
/// * `2.0` - the tile stack, the object list and the palette.
/// * `2.1` - adds the **`"scenery"`** block (`SCENERY.md`): the free-placed
///   cut-outs, each `{pack, piece, x, y}` in map pixels. A MINOR, not a MAJOR,
///   deliberately - the block is purely additive, and a MAJOR bump would refuse
///   to open all 24 shipped maps and every project saved before it. The cost of
///   that choice is that a build older than this one drops a map's scenery
///   silently on re-save.
pub const PROJECT_VERSION: &str = "2.1";

/// Undo depth cap - beyond this the oldest patches are dropped.
const MAX_UNDO: usize = 256;

/// The tileset-editable palette slots (contract §1: dynamic 64–159).
pub const DYNAMIC_SLOTS: std::ops::RangeInclusive<u8> = 64..=159;

/// The game-animated shimmer slots (contract §1): the engine re-tints this
/// fixed cycle each frame, so pixels are never quantized onto them.
pub const ANIMATED_SLOTS: std::ops::RangeInclusive<u8> = 9..=31;

/// The water / special-effect cycle band (contract §1; see [`WATER_CYCLES`]).
pub const WATER_SLOTS: std::ops::RangeInclusive<u8> = 96..=127;

/// The dynamic **animated** water cycle classes (contract §1) - each block
/// is one in-game color gradient; block re-tints keep it coherent.
pub const WATER_CYCLES: [(u8, u8); 5] = [(96, 102), (103, 109), (110, 116), (117, 122), (123, 127)];

/// The scenery libraries for the packs a project uses, shipped cut-outs plus
/// the user's own.
///
/// `assets_root` points at `resources/assets/tilepacks`, so the shipped set is
/// its sibling `resources/assets/scenery` and the user's is
/// `resources/user/scenery` - the same two roots a user *tile* pack is found
/// under. Loading is best-effort: a pack that ships none (WATER) and has none
/// authored is simply absent, and placements naming it stay inert rather than
/// failing the open.
pub(crate) fn load_scenery_packs(assets_root: &Path, uses: &[UseEntry]) -> Vec<SceneryPack> {
	let (Some(shipped), Some(user)) = (scenery_root(assets_root), user_scenery_root(assets_root)) else {
		return Vec::new();
	};
	uses.iter().filter_map(|u| SceneryPack::load_merged(&shipped, &user, &u.name)).collect()
}

/// Where the shipped cut-outs live, given the tile-pack root: `resources/assets`.
pub fn scenery_root(assets_root: &Path) -> Option<PathBuf> {
	assets_root.parent().map(Path::to_path_buf)
}

/// Where the user's own cut-outs live: `resources/user`. The same derivation
/// `append_user_packs` uses for user tile packs, so the two stay together.
pub fn user_scenery_root(assets_root: &Path) -> Option<PathBuf> {
	assets_root.parent().and_then(Path::parent).map(|r| r.join("user"))
}

/// How much of a cell a scenery placement's **body** must cover before the cell
/// takes the placement's pass value. An eighth: enough that a stray pixel of
/// canopy does not wall off a cell, low enough that the ragged edge of a
/// mountain still blocks the cell it stands on.
const SCENERY_PASS_COVERAGE: usize = TILE_DATA_SIZE / 8;

/// The largest map dimension (cells per side) a template may have.
pub(crate) const MAX_DIM: u16 = 1024;

/// Validate a map's dimensions (both in `1..=MAX_DIM`).
pub(crate) fn check_map_size(width: u16, height: u16) -> Result<(), String> {
	if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM {
		return Err(format!("bad map size {width}x{height} (1..=1024)"));
	}
	Ok(())
}

/// Encode a `width`×`height` cell grid as JSON rows (`[[String; width]; height]`),
/// each cell rendered by `cell(x, y)` - the shared map-body writer for the
/// project file and templates.
pub(crate) fn encode_cell_grid(
	width: usize,
	height: usize,
	cell: impl Fn(usize, usize) -> String,
) -> Vec<json::JsonValue> {
	(0..height)
		.map(|y| json::JsonValue::Array((0..width).map(|x| json::JsonValue::String(cell(x, y))).collect()))
		.collect()
}

/// Options for [`Project::synthesize_save`] — the fresh-game parameters the
/// caller (UI/command) chooses; teams derive from the placed objects.
#[derive(Debug, Clone)]
pub struct SynthesizeSaveOptions {
	pub save_name: String,
	/// Stock world slot (0..=23) the save claims — the swapped-`.WRL` workflow
	/// installs the actual map at that slot.
	pub world_index: u8,
	/// Per-slot clan (`TEAM_CLAN_*` 1..=8; only playing slots matter).
	pub team_clans: [u8; 5],
	pub start_gold: i32,
	pub rng_seed: u32,
}

/// What [`Project::synthesize_save`] produced (for the console line).
#[derive(Debug, Clone, Copy)]
pub struct SynthesisSummary {
	pub bytes: usize,
	pub units: usize,
	pub teams: usize,
}

/// Which layer a tile belongs on, by its passability: water (pass 1) is the
/// opaque base; land / shore / obstruction sit on the ground layer. This is
/// how an imported WRL is decomposed into the two editor layers, and how a
/// reloaded WRL-import project recovers the same split.
pub(crate) fn pass_layer(pass: u8) -> usize {
	if pass == 1 { LAYER_WATER } else { LAYER_GROUND }
}

/// A pass value's class glyph + dense class index (for `XXXY###` tile ids):
/// `W`ater / `S`hore / `L`and / obstruction (`X`).
pub(crate) fn pass_class(pass: u8) -> (char, usize) {
	match pass {
		1 => ('W', 1),
		2 => ('S', 2),
		3 => ('X', 3),
		_ => ('L', 0),
	}
}

/// The 3-letter id prefix for a pack built from a WRL: the first three
/// consonants of its name (upper-cased), topped up with vowels then `X` when a
/// name has fewer than three consonants. `GREEN_1` → `GRN`, `GO` → `GOX`.
pub(crate) fn pack_prefix(name: &str) -> String {
	let letters: Vec<char> = name.chars().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_uppercase()).collect();
	let vowel = |c: &char| matches!(c, 'A' | 'E' | 'I' | 'O' | 'U');
	let mut out: Vec<char> = letters.iter().copied().filter(|c| !vowel(c)).take(3).collect();
	for c in letters.iter().copied().filter(vowel) {
		if out.len() == 3 {
			break;
		}
		out.push(c);
	}
	while out.len() < 3 {
		out.push('X');
	}
	out.into_iter().collect()
}

/// Tiny deterministic PRNG (splitmix64) - the new-map fill and future
/// generators must reproduce exactly from a seed, on every
/// platform, forever. Never swap this for a library RNG.
pub struct Rng(u64);

/// The splitmix64 finalizer - the bit mixer behind both [`Rng`] and worldgen's
/// lattice hash. A pure function of its input; never change it, seeded output
/// must reproduce forever on every platform.
pub(crate) fn splitmix(mut z: u64) -> u64 {
	z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
	z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
	z ^ (z >> 31)
}

impl Rng {
	pub fn new(seed: u64) -> Self {
		Self(seed)
	}

	pub fn next_u64(&mut self) -> u64 {
		self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
		splitmix(self.0)
	}

	/// Uniform in `0..n` (`n > 0`; modulo bias is negligible at u64 width).
	pub fn below(&mut self, n: u32) -> u32 {
		(self.next_u64() % n as u64) as u32
	}
}

/// Rotation (quarter turns clockwise) + horizontal mirror (applied first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Transform {
	pub rot: u8, // 0..=3 - N, E, S, W
	pub mirror: bool,
}

impl Transform {
	/// Suffix convention (verified empirically against all 24 original WRLs
	/// in `tests/equivalence.rs`): plain letters are counter-clockwise
	/// quarter turns (`E` = 1 ccw = 3 cw, `W` = 3 ccw = 1 cw); with the `!`
	/// mirror prefix the letter counts clockwise (`!E` = 1 cw + mirror).
	/// A bare `!` (the `tiles.match.json` shorthand) is mirror-only = `!N`.
	pub fn parse(s: &str) -> Result<Self, String> {
		let (mirror, dir) = match s.strip_prefix('!') {
			Some(rest) => (true, rest),
			None => (false, s),
		};
		let cw = match dir {
			"N" => 0,
			"E" => 1,
			"S" => 2,
			"W" => 3,
			"" if mirror => 0,
			_ => return Err(format!("bad transform '{s}'")),
		};
		let rot = if mirror { cw } else { (4 - cw) % 4 };
		Ok(Self { rot, mirror })
	}

	pub fn suffix(&self) -> String {
		if !self.mirror && self.rot == 0 {
			return String::new();
		}
		let cw = if self.mirror { self.rot } else { (4 - self.rot) % 4 };
		let dir = ["N", "E", "S", "W"][cw as usize];
		format!(":{}{}", if self.mirror { "!" } else { "" }, dir)
	}

	/// Pack into 3 bits (rot in bits 0–1, mirror in bit 2) for the GPU.
	pub fn bits(&self) -> u32 {
		self.rot as u32 | ((self.mirror as u32) << 2)
	}

	// Composition: a stored transform is `R(rot) ∘ M(mirror)` (mirror first,
	// then clockwise quarter turns). The toolbox ops apply a further
	// operation *after* it and re-normalize to that form; `M ∘ R(r) =
	// R(-r) ∘ M` is the only identity needed. Verified pixel-for-pixel by
	// `transform_ops_match_pixel_operations`.

	/// This transform followed by one more clockwise quarter turn.
	pub fn rotated_cw(self) -> Self {
		Self { rot: (self.rot + 1) % 4, mirror: self.mirror }
	}

	/// This transform followed by one counter-clockwise quarter turn.
	pub fn rotated_ccw(self) -> Self {
		Self { rot: (self.rot + 3) % 4, mirror: self.mirror }
	}

	/// This transform followed by a horizontal mirror.
	pub fn flipped_h(self) -> Self {
		Self { rot: (4 - self.rot) % 4, mirror: !self.mirror }
	}

	/// This transform followed by a vertical mirror (= mirror + 180°).
	pub fn flipped_v(self) -> Self {
		Self { rot: (6 - self.rot) % 4, mirror: !self.mirror }
	}

	/// `self ∘ inner` - apply `inner` first, then `self`, re-normalized to
	/// the stored `R ∘ M` form (`R(a)M(α)R(b)M(β) = R(a∓b)M(α⊕β)`). The
	/// match rules describe neighbors relative to a family's base
	/// orientation; placing the family transformed means composing its
	/// transform onto every listed neighbor spec (auto-shore seams).
	pub fn compose(self, inner: Self) -> Self {
		Self {
			rot: (self.rot + if self.mirror { 4 - inner.rot } else { inner.rot }) % 4,
			mirror: self.mirror ^ inner.mirror,
		}
	}

	/// The transform that undoes `self`: `self.compose(self.inverse())` and
	/// `self.inverse().compose(self)` both equal [`Transform::default`] (the
	/// D4 group inverse). Used to author the reciprocal neighbor entry when the
	/// match-data editor confirms a two-way adjacency.
	pub fn inverse(self) -> Self {
		if self.mirror { self } else { Self { rot: (4 - self.rot) % 4, mirror: false } }
	}

	/// The base-orientation direction (ring N=0,E=1,S=2,W=3) that faces screen
	/// direction `dir` once a tile is placed with this transform - undo the
	/// rotation, then the mirror. The match rules are stored base-relative, so
	/// `shore.rs`'s seam matcher and the match-data editor share this mapping.
	pub fn screen_to_base(self, dir: usize) -> usize {
		let d = (dir + 4 - self.rot as usize) % 4;
		if self.mirror { (4 - d) % 4 } else { d }
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileRef {
	pub pack: u8,
	pub tile: u16,
	pub transform: Transform,
}

#[derive(Clone)]
pub struct UseEntry {
	pub name: String,
	pub tileset: bool,
	pub palette: bool,
	pub version: String,
}

/// Append user-owned packs (`resources/user/tilepacks/<NAME>/`) that mirror
/// a stock pack already in `packs` - the Tile Painter stores new/cloned tiles
/// there, named after the pack they derive from. Appended *after* the stock
/// packs (so their ids resolve, and indices stay stable for the session), and
/// only when the matching stock pack is loaded (a GREEN user pack only joins a
/// map that uses GREEN). Best-effort: an unreadable user pack is skipped.
///
/// `assets_root` is `resources/assets/tilepacks`, so the user packs sit two
/// levels up under `user/tilepacks`.
fn append_user_packs(packs: &mut Vec<TilePack>, assets_root: &Path) {
	let Some(user_root) = assets_root.parent().and_then(Path::parent).map(|r| r.join("user/tilepacks")) else {
		return;
	};
	let Ok(dir) = std::fs::read_dir(&user_root) else { return };
	let stock: Vec<String> = packs.iter().filter(|p| !p.user).map(|p| p.name.clone()).collect();
	let mut names: Vec<String> = dir
		.flatten()
		.filter(|e| e.path().is_dir())
		.map(|e| e.file_name().to_string_lossy().into_owned())
		.filter(|name| stock.contains(name) && !packs.iter().any(|p| p.user && &p.name == name))
		.collect();
	names.sort(); // deterministic append order
	for name in names {
		if let Ok(mut pack) = TilePack::load(&user_root, &name) {
			pack.user = true;
			packs.push(pack);
		}
	}
}

pub struct Project {
	pub version: String,
	pub name: String,
	pub description: String,
	/// Map Metadata (the Edit-menu dialog) - all optional, never affect the bake.
	/// Suggested player count (2–4); `None` = unspecified.
	pub players: Option<u8>,
	/// Free-text date (no enforced format).
	pub date: String,
	/// Author-facing map version string (distinct from the file format version).
	pub map_version: String,
	pub author: String,
	pub width: u16,
	pub height: u16,
	pub uses: Vec<UseEntry>,
	pub packs: Vec<TilePack>,
	/// `width * height` cell stacks, bottom-up: `[water, ground]`.
	pub cells: Vec<[Option<TileRef>; MAX_LAYERS]>,
	/// Per-cell pass-value override (Pass Table Editor) - `None`
	/// falls back to the derived stack-top pass. `width * height` long.
	pass_overrides: Vec<Option<u8>>,
	/// Working 256×RGB palette: the owner pack's palette + this map's
	/// dynamic-slot overrides (edited via `set_color`/`hsl_shift_block`).
	pub palette: Vec<u8>,
	/// The owner pack's pristine palette - the diff against it is what
	/// `save_string` writes as the project's `"palette"` override block.
	pack_palette: Vec<u8>,
	/// The document's palette exactly as its source carries it - the WRL's
	/// internal palette bytes (or the pack's `palette.json`), **before** the
	/// game statics replace the static slots. Debug rendering and the WRL
	/// Internal Palette panel read it via [`Self::internal_palette`].
	source_palette: Vec<u8>,
	/// Index of the pack that fills the water layer (v1: named "WATER").
	pub water_pack: Option<u8>,
	/// First-class objects placed on the map: preview annotations on an ordinary
	/// map, or the units / slabs / rubble of an opened save (seeded by
	/// [`Self::attach_save`]). Saved in the project (`"objects"` block), never
	/// baked into the WRL, and **undoable** (mutated via [`Self::place_object`]
	/// etc., journaled through [`Patch::objects`]).
	pub objects: Vec<MapObject>,
	/// Scenery placed on the map (`SCENERY.md` stage C): cut-outs of the shipped
	/// templates, positioned by pixel rather than by cell. Saved in the project
	/// (`"scenery"` block), composed into the cells by [`Self::compose_cell`] so
	/// the WRL export carries them, and **undoable** - journaled through
	/// [`Patch::scenery`] the same wholesale way `objects` is.
	pub scenery: Vec<ScenerySpot>,
	/// The cut-out libraries the placements resolve against - one per pack in
	/// `uses` that ships a `resources/assets/scenery/<PACK>/`. Loaded beside the
	/// tile packs; empty when the asset set is absent, which leaves every
	/// placement unresolved rather than failing the open.
	pub scenery_packs: Vec<SceneryPack>,
	/// An opened M.A.X. saved game (`.DTA`), when this project is a save-editor
	/// session. The raw file image round-trips through the project `.json` (the
	/// `"save"` base64 block), and the decoded [`SaveFile`] overlays its units /
	/// slabs / resources onto the world. `None` for an ordinary map project.
	pub save: Option<EmbeddedSave>,
	/// The editable per-cell resource (cargo) map of an opened save (S5): raw /
	/// fuel / gold amounts, one `u16` per cell (`max_assets::save::cargo`
	/// encoding), row-major `y * width + x`. Seeded from the save's pristine
	/// `cargo_map` on [`Self::attach_save`] and edited undoably via
	/// [`Self::set_cargo`]; the `.json` persists only the diff against that seed
	/// (`"resources"` block). Empty for a project with no save attached.
	cargo_map: Vec<u16>,

	dirty: bool,
	revision: u64,
	/// Bumped whenever document *structure* changes - pack tile tables /
	/// palette tables swapped (palette conversion and its undo/redo). The
	/// shell compares it across a command to know the GPU atlas must rebuild.
	structure: u64,
	undo_stack: Vec<Patch>,
	redo_stack: Vec<Patch>,
	/// Open stroke: edits accumulate here and undo as one unit.
	stroke: Option<Patch>,
	/// A label for the next committed undo patch, set by the app before an
	/// editing command (`label_next_undo`); `None` derives one from the patch.
	pending_label: Option<String>,
	/// Bumped whenever the undo stack changes (push / undo / redo), so the shell
	/// can rebuild the Undo History submenu only when it actually changed.
	undo_seq: u64,
	/// The map region edited since the renderer last consumed it, so an edit
	/// re-uploads only its sub-rectangle instead of the whole map every frame
	/// (drained by [`Self::take_render_dirty`]). `cells` = cells whose tile
	/// stack changed (their *derived* pass follows, so those also enter `pass`);
	/// `pass` = cells whose displayed pass changed. Inclusive `(x0, y0, x1, y1)`.
	/// A whole-map pass retint (Pass Table Editor) marks the full extent.
	render_dirty_cells: Option<(u16, u16, u16, u16)>,
	render_dirty_pass: Option<(u16, u16, u16, u16)>,
}

/// The map sub-rectangles a renderer must re-upload after edits, drained from
/// [`Project::take_render_dirty`]. Each is an inclusive `(x0, y0, x1, y1)` cell
/// bbox, or `None` when that texture is already current.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderDirty {
	pub cells: Option<(u16, u16, u16, u16)>,
	pub pass: Option<(u16, u16, u16, u16)>,
}

/// A first-class, undoable object placed on the map: a game unit, building, or
/// ground cover (slab / rubble / road / connector). It serves two roles:
///
/// - On an ordinary map project it is a **preview annotation** - a unit stamped
///   on a cell as a palette-tuning aid (the former `UnitNote`), carrying only a
///   type, position, and team (default [`ObjectProps`]).
/// - On a save-editor session it is a **save object** seeded from the opened
///   `.DTA` (`Project::attach_save`), carrying its gameplay [`ObjectProps`].
///
/// The sprite itself lives in the user's MAX.RES; the project records what
/// stands where. `unit_type` is a `ResourceID` (see
/// [`max_assets::save::unit_type_name`]) - also the sprite tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapObject {
	/// `ResourceID` (`0x33` = TANK, `0x11` = LRGSLAB, …); the sprite tag too.
	pub unit_type: u16,
	/// Cell position on the same grid as `Project.width/height`.
	pub x: u16,
	pub y: u16,
	/// Owner team (0-4: red green blue gray/alien yellow).
	pub team: u8,
	/// Gameplay state - seeded from a save record, edited in later stages (S4);
	/// all-default for a preview annotation.
	pub props: ObjectProps,
}

/// The editable gameplay state of a [`MapObject`], mirroring the save-relevant
/// fields of a `.DTA` unit record. All-default for a preview annotation; seeded
/// from the opened save otherwise so nothing is lost across a save/reload (the
/// retained raw `.DTA` stays the byte-exact export anchor either way).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectProps {
	/// Custom unit name (empty = the type's default name).
	pub name: String,
	/// Facing / body angle (0..7).
	pub angle: u8,
	/// Turret heading (0..7), independent of the body `angle`. Seeded from the
	/// save; for a fresh placement it defaults to 0 (= the body's default facing).
	pub turret_angle: u8,
	/// Current hit points (0 = "use the type's max", filled in on edit; S4).
	pub hits: u16,
	pub ammo: u8,
	/// Current order (idle / building / …), engine `UnitOrderType`.
	pub orders: u8,
	/// Turns the unit stays disabled (`disabled_turns_remaining`). Meaningful only
	/// while `orders == ORDER_DISABLE`; the save editor exposes it as "disabled for
	/// N turns" and the export writes the engine's disable byte.
	pub disabled_turns: u8,
	/// Cargo carried / accrued experience (context-dependent per type).
	pub storage: i16,
	/// Connector adjacency bitmask (slabs / roads / connectors).
	pub connectors: u16,
	/// The source save record's spatial-hash `id` when this object was seeded
	/// from an opened save - the join key a future byte-aware export (S6) uses to
	/// match it back onto the retained `SaveFile`. `None` for a fresh placement.
	pub source_id: Option<u16>,
	/// A per-unit override of this object's maximum stats (`UnitValues`: max HP,
	/// attack, armor, …). `None` = inherit the save's shared `base_values` seed
	/// (via [`Project::object_base_values`]); `Some` is this unit's own cloned copy
	/// after an edit (S4.5), mirroring the engine's per-unit clone-on-edit
	/// (`UnitInfo` sets `base_values = new UnitValues(*base_values)` when a unit's
	/// stats diverge from its team's). Persisted so an upgrade survives a
	/// save/reload; a byte-aware export (S6) re-emits it onto the object graph.
	pub base_values: Option<UnitValues>,
}

/// A M.A.X. saved game (`.DTA`) opened into a project (the save editor). The
/// raw file image is retained verbatim as the byte-exact export/round-trip
/// anchor (per `SAVE-EDITOR.md` D1); `file` is that image decoded at the map's
/// dimensions, the source the editor reads to overlay units and resources.
/// Persisted in the project `.json` as base64 under `"save"`, re-decoded on load.
#[derive(Debug, Clone)]
pub struct EmbeddedSave {
	/// The original `.DTA` bytes, exactly as opened.
	pub raw: Vec<u8>,
	/// `raw` decoded at the project's `width`×`height`.
	pub file: SaveFile,
}

/// The edits [`Project::export_save`] cannot represent. Scalar edits, moves,
/// stat overrides, unit removals, and placements of a type already present all
/// export; the only gap is a placed unit whose type has no same-type body
/// template in the save. Empty means the export is faithful; the save editor
/// surfaces every entry as a warning so no edit is silently dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnexportedEdits {
	/// One `TYPE at x,y` description per placed unit skipped for lack of a
	/// same-type template to clone.
	pub added: Vec<String>,
}

impl UnexportedEdits {
	/// Whether every count is zero — the export reflects all edits.
	pub fn is_empty(&self) -> bool {
		*self == Self::default()
	}
}

/// One undoable edit: cells with their *previous* layer entries, palette
/// slots with their *previous* colors.
#[derive(Default)]
struct Patch {
	/// A human label for the Undo History submenu (set at commit from the app's
	/// hint, else derived from the patch contents). Preserved across undo/redo.
	label: String,
	cells: Vec<(u16, u16, usize, Option<TileRef>)>,
	colors: Vec<(u8, [u8; 3])>,
	/// Pass-override edits with their *previous* value (`None` = unset).
	passes: Vec<(u16, u16, Option<u8>)>,
	/// Resource (cargo) map edits with their *previous* `u16` value (S5). Sparse
	/// per-cell `(x, y, prev)`, like `passes`; captured once per cell per stroke.
	resources: Vec<(u16, u16, u16)>,
	/// Per-tile passability edits (Pass Table Editor): `(pack, tile,
	/// previous pass)`. The pass lives in the pack, so one edit retints every
	/// cell that uses the tile.
	tile_passes: Vec<(u8, u16, u8)>,
	/// A whole-document swap (palette conversion rewrites tile pixel data -
	/// not expressible as per-cell edits). Applying swaps the stored state
	/// with the live one, so the patch is its own inverse carrier.
	doc: Option<Box<DocState>>,
	/// The `objects` list *before* this edit. Objects are few and change
	/// wholesale (place / move / delete), so - like `doc` - the patch snapshots
	/// the whole vector and `apply` swaps it (its own inverse). Captured once per
	/// stroke (the first object edit records the pre-stroke state).
	objects: Option<Vec<MapObject>>,
	/// The `scenery` list *before* this edit - snapshotted wholesale like
	/// `objects`, for the same reason: placements are few and change whole
	/// (place / move / delete). Captured once per stroke.
	scenery: Option<Vec<ScenerySpot>>,
	/// The attached save's editable settings *before* this edit (S7.2). Like
	/// `doc`, applying swaps the stored block with the live one (its own
	/// inverse), rebasing the embedded raw anchor each way.
	save_settings: Option<Box<SaveSettings>>,
}

impl Patch {
	fn is_empty(&self) -> bool {
		self.cells.is_empty()
			&& self.colors.is_empty()
			&& self.passes.is_empty()
			&& self.resources.is_empty()
			&& self.tile_passes.is_empty()
			&& self.doc.is_none()
			&& self.objects.is_none()
			&& self.scenery.is_none()
			&& self.save_settings.is_none()
	}

	/// A label derived from the patch contents, used when the app didn't supply
	/// one - so the Undo History always reads something meaningful.
	fn default_label(&self) -> String {
		if self.doc.is_some() {
			"Document change".into()
		} else if self.save_settings.is_some() {
			"Save data".into()
		} else if self.objects.is_some() {
			"Objects".into()
		} else if self.scenery.is_some() {
			"Scenery".into()
		} else if !self.tile_passes.is_empty() {
			"Passability".into()
		} else if !self.cells.is_empty() {
			let n = self.cells.len();
			format!("Paint {n} cell{}", if n == 1 { "" } else { "s" })
		} else if !self.passes.is_empty() {
			"Pass override".into()
		} else if !self.resources.is_empty() {
			"Resources".into()
		} else if !self.colors.is_empty() {
			"Palette".into()
		} else {
			"Edit".into()
		}
	}
}

/// Everything a document-level operation may replace (same map dimensions).
struct DocState {
	uses: Vec<UseEntry>,
	packs: Vec<TilePack>,
	cells: Vec<[Option<TileRef>; MAX_LAYERS]>,
	pass_overrides: Vec<Option<u8>>,
	palette: Vec<u8>,
	pack_palette: Vec<u8>,
	source_palette: Vec<u8>,
	water_pack: Option<u8>,
}

impl Project {
	pub fn load(path: &Path, assets_root: &Path) -> Result<Self, String> {
		let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
		// A project saved from an imported WRL co-locates its synthetic pack
		// in a sibling folder; search there too (see `TilePack::dump`).
		Self::from_str_in(&text, assets_root, path.parent())
	}

	pub fn from_str(text: &str, assets_root: &Path) -> Result<Self, String> {
		Self::from_str_in(text, assets_root, None)
	}

	/// Import a flat WRL as a Project - the in-memory form for an opened
	/// `.WRL` (the document-model convergence). The WRL's tile table
	/// becomes one synthetic in-memory pack; every cell references its bigmap
	/// tile on the water (opaque base) layer with identity transform, so
	/// `compose_cell` reproduces the WRL's pixels byte-for-byte.
	///
	/// The synthetic pack carries the WRL palette and per-tile pass table but
	/// no adjacency rules, so auto-shore / suggestions / tile-pack browsing
	/// don't apply to an imported WRL. It exports back to a WRL via `bake`,
	/// but can't be saved as a `.json` project (the pack isn't on disk).
	pub fn from_wrl(wrl: &WrlFile, name: &str) -> Self {
		let tile_count = wrl.tile_count as usize;
		// Tile ids carry meaning: `XXXY###` - `XXX` from the map name, `Y` the
		// passability class (W/S/L/X), `###` a per-class running index.
		let prefix = pack_prefix(name);
		let mut class_seq = [0u32; 4];
		let ids: Vec<String> = (0..tile_count)
			.map(|i| {
				let (letter, slot) = pass_class(wrl.pass_table.get(i).copied().unwrap_or(0));
				let n = class_seq[slot];
				class_seq[slot] += 1;
				format!("{prefix}{letter}{n:03}")
			})
			.collect();
		let index_of: HashMap<String, u16> = ids.iter().enumerate().map(|(i, id)| (id.clone(), i as u16)).collect();

		// Static slots belong to the game (contract §1); resolve them to the
		// in-game values, matching how `from_str` treats a pack palette. The
		// WRL's own bytes are kept as the source palette for debug rendering.
		let source_palette = wrl.palette.clone();
		let mut palette = wrl.palette.clone();
		crate::game_palette::apply_game_statics(&mut palette);

		let pack = TilePack {
			name: name.to_string(),
			version: "wrl".to_string(),
			tiles: wrl.tiles.clone(),
			ids,
			index_of,
			palette: Some(palette.clone()),
			palette_name: None,
			pass: Some(wrl.pass_table.clone()),
			matches: HashMap::new(),
			variant_groups: Vec::new(),
			variant_of: vec![None; tile_count],
			variant_named: HashMap::new(),
			props: HashMap::new(),
			patterns: Vec::new(),
			user: false,
		};

		// Decompose the flat map into the two editor layers by passability:
		// water cells ride the opaque base layer, everything else (land, shore,
		// obstructions) goes on the ground layer. A lone tile composes the same
		// on either layer, so this is pixel-lossless - it just makes the layers
		// meaningful (e.g. for "show only selected").
		let cells: Vec<[Option<TileRef>; MAX_LAYERS]> = wrl
			.bigmap
			.iter()
			.map(|&tile| {
				let mut stack = [None; MAX_LAYERS];
				let layer = pass_layer(wrl.pass_table.get(tile as usize).copied().unwrap_or(0));
				stack[layer] = Some(TileRef { pack: 0, tile, transform: Transform::default() });
				stack
			})
			.collect();

		Self {
			version: PROJECT_VERSION.to_string(),
			name: name.to_string(),
			description: String::new(),
			players: None,
			date: String::new(),
			map_version: String::new(),
			author: String::new(),
			width: wrl.width,
			height: wrl.height,
			uses: vec![UseEntry { name: name.to_string(), tileset: true, palette: true, version: "wrl".to_string() }],
			packs: vec![pack],
			cells,
			pass_overrides: vec![None; wrl.width as usize * wrl.height as usize],
			pack_palette: palette.clone(),
			source_palette,
			palette,
			water_pack: Some(0),
			objects: Vec::new(),
			scenery: Vec::new(),
			scenery_packs: Vec::new(),
			save: None,
			cargo_map: Vec::new(),
			dirty: false,
			revision: 0,
			structure: 0,
			undo_stack: Vec::new(),
			redo_stack: Vec::new(),
			stroke: None,
			pending_label: None,
			undo_seq: 0,
			render_dirty_cells: None,
			render_dirty_pass: None,
		}
	}

	/// 1×1 placeholder Project - the document the editor holds before the
	/// initial `open` runs (replaces the old `MapDoc::empty`).
	pub fn empty() -> Self {
		Self::from_wrl(
			&WrlFile {
				header: vec![0; 5],
				width: 1,
				height: 1,
				minimap: vec![0],
				bigmap: vec![0],
				tile_count: 1,
				tiles: vec![0; TILE_DATA_SIZE],
				palette: vec![0; 768],
				pass_table: vec![0],
			},
			"empty",
		)
	}

	/// Blank project: the bottom layer fully covered with
	/// randomly distributed water variants (identity transform - WATER is
	/// `sync`), ground empty. Deterministic from `seed`. WATER is implied
	/// when not listed; the first listed pack with a palette owns it.
	pub fn new(width: u16, height: u16, pack_names: &[String], assets_root: &Path, seed: u64) -> Result<Self, String> {
		check_map_size(width, height)?;

		// WATER first (it fills the bottom layer), then the rest, deduped.
		let mut names: Vec<String> = vec!["WATER".to_string()];
		for name in pack_names {
			if !names.contains(name) {
				names.push(name.clone());
			}
		}
		let mut packs: Vec<TilePack> =
			names.iter().map(|name| TilePack::load(assets_root, name)).collect::<Result<_, _>>()?;
		// User-owned packs (custom tiles) join so they're paintable on a new map.
		append_user_packs(&mut packs, assets_root);

		// First pack with a palette owns it (compatibility verdicts).
		let owner = packs
			.iter()
			.position(|p| p.palette.is_some())
			.ok_or("no palette-owning pack - add a tileset (e.g. GREEN)")?;
		let mut palette = packs[owner].palette.clone().unwrap();
		let source_palette = palette.clone();
		crate::game_palette::apply_game_statics(&mut palette);
		let uses: Vec<UseEntry> = names
			.iter()
			.enumerate()
			.map(|(i, name)| UseEntry {
				name: name.clone(),
				tileset: true,
				palette: i == owner,
				version: packs[i].version.clone(),
			})
			.collect();

		let scenery_packs = load_scenery_packs(assets_root, &uses);

		let water_tiles = packs[0].tile_count();
		if water_tiles == 0 {
			return Err("WATER pack has no tiles".into());
		}
		let mut rng = Rng::new(seed);
		let cells = (0..width as usize * height as usize)
			.map(|_| {
				let mut stack = [None; MAX_LAYERS];
				stack[LAYER_WATER] = Some(TileRef {
					pack: 0,
					tile: rng.below(water_tiles as u32) as u16,
					transform: Transform::default(),
				});
				stack
			})
			.collect();

		Ok(Self {
			version: PROJECT_VERSION.to_string(),
			name: "Untitled".to_string(),
			description: String::new(),
			players: None,
			date: String::new(),
			map_version: String::new(),
			author: String::new(),
			width,
			height,
			uses,
			packs,
			cells,
			pass_overrides: vec![None; width as usize * height as usize],
			pack_palette: palette.clone(),
			source_palette,
			palette,
			water_pack: Some(0),
			objects: Vec::new(),
			scenery: Vec::new(),
			scenery_packs,
			save: None,
			cargo_map: Vec::new(),
			dirty: false,
			revision: 0,
			structure: 0,
			undo_stack: Vec::new(),
			redo_stack: Vec::new(),
			stroke: None,
			pending_label: None,
			undo_seq: 0,
			render_dirty_cells: None,
			render_dirty_pass: None,
		})
	}

	pub fn dirty(&self) -> bool {
		self.dirty
	}
	pub fn revision(&self) -> u64 {
		self.revision
	}
	/// Bumped on structural changes - tile/palette table swaps (palette
	/// conversion and its undo/redo). When it moves across a command, the
	/// renderer's tile atlas is stale and must rebuild.
	pub fn structure_revision(&self) -> u64 {
		self.structure
	}

	pub fn mark_saved(&mut self) {
		self.dirty = false;
	}

	/// Snapshot the current `objects` list into the undo journal before an
	/// object edit, so the edit undoes to the pre-edit state. In an open stroke
	/// the snapshot is captured *once* (the first object edit of the stroke), so
	/// a whole drag undoes as one unit; otherwise it commits a solo patch that
	/// carries the pre-edit vector (`apply` swaps it back). Call this immediately
	/// **before** mutating `self.objects`. `redo_stack` is cleared by the caller.
	fn snapshot_objects(&mut self) {
		match &mut self.stroke {
			Some(stroke) => {
				if stroke.objects.is_none() {
					stroke.objects = Some(self.objects.clone());
				}
			}
			None => {
				let objects = Some(self.objects.clone());
				self.push_undo(Patch { objects, ..Patch::default() });
			}
		}
	}

	/// Place (or restamp) an object on a cell, replacing any already on that
	/// exact cell **in the same layer**. Ground cover (slabs, rubble, roads) is
	/// its own layer, so a building sits *on* its slab rather than evicting it —
	/// the way the game keeps the two in separate unit lists, and the stacking
	/// the object tools already read (`EditorState::object_at_cycling`).
	/// Restamping therefore replaces only what is being stamped. Undoable.
	pub fn place_object(&mut self, obj: MapObject) {
		self.snapshot_objects();
		let cover = max_assets::save::is_ground_cover_type(obj.unit_type);
		self.objects
			.retain(|o| (o.x, o.y) != (obj.x, obj.y) || max_assets::save::is_ground_cover_type(o.unit_type) != cover);
		self.objects.push(obj);
		self.redo_stack.clear();
		self.bump();
	}

	/// Move object `index` to cell `(x, y)`. Undoable (part of the open stroke,
	/// so a whole drag is one undo unit); `false` (no patch) when the index is
	/// out of range or the object is already there. Collision is the caller's
	/// concern - the footprint lives in the sprite library (app side).
	pub fn move_object_to(&mut self, index: usize, x: u16, y: u16) -> bool {
		let Some(o) = self.objects.get(index) else { return false };
		if (o.x, o.y) == (x, y) {
			return false;
		}
		self.snapshot_objects();
		self.objects[index].x = x;
		self.objects[index].y = y;
		self.redo_stack.clear();
		self.bump();
		true
	}

	// ----- scenery -------------------------------------------------------------

	/// The library piece a placement names, or `None` when its pack or id is not
	/// among the loaded libraries - a project opened without the scenery assets,
	/// or one naming a piece a later bake dropped. An unresolved placement is
	/// inert: it draws nothing, blocks nothing, and survives a save/load
	/// round-trip so the assets can come back.
	pub fn scenery_piece(&self, spot: &ScenerySpot) -> Option<&SceneryPiece> {
		self.scenery_packs.iter().find(|p| p.pack == spot.pack)?.piece(&spot.piece)
	}

	/// Place a scenery object at a footprint origin in map pixels. Undoable;
	/// returns its index in `scenery`. Placements stack - a later one draws over
	/// an earlier one - so this never displaces anything.
	pub fn place_scenery(&mut self, spot: ScenerySpot) -> usize {
		self.snapshot_scenery();
		self.mark_scenery_dirty(&spot);
		self.scenery.push(spot);
		self.redo_stack.clear();
		self.bump();
		self.scenery.len() - 1
	}

	/// Move placement `index` to a new footprint origin. Part of the open
	/// stroke, so a whole drag undoes as one unit. `false` (no patch) when the
	/// index is out of range or nothing moves.
	pub fn move_scenery_to(&mut self, index: usize, x: i32, y: i32) -> bool {
		let Some(spot) = self.scenery.get(index) else { return false };
		if (spot.x, spot.y) == (x, y) {
			return false;
		}
		let from = spot.clone();
		self.snapshot_scenery();
		self.mark_scenery_dirty(&from); // the cells it leaves
		self.scenery[index].x = x;
		self.scenery[index].y = y;
		let to = self.scenery[index].clone();
		self.mark_scenery_dirty(&to); // and the ones it arrives on
		self.redo_stack.clear();
		self.bump();
		true
	}

	/// Set placement `index`'s blend mode - how its ink meets the scenery under
	/// it. Undoable; `false` when the index is out of range or it already has
	/// that mode.
	pub fn set_scenery_blend(&mut self, index: usize, blend: crate::scenery::SceneryBlend) -> bool {
		let Some(spot) = self.scenery.get(index) else { return false };
		if spot.blend == blend {
			return false;
		}
		self.snapshot_scenery();
		self.scenery[index].blend = blend;
		let spot = self.scenery[index].clone();
		self.mark_scenery_dirty(&spot);
		self.redo_stack.clear();
		self.bump();
		true
	}

	/// Remove placement `index`. Undoable; `false` when out of range.
	pub fn remove_scenery(&mut self, index: usize) -> bool {
		if index >= self.scenery.len() {
			return false;
		}
		self.snapshot_scenery();
		let gone = self.scenery.remove(index);
		self.mark_scenery_dirty(&gone);
		self.redo_stack.clear();
		self.bump();
		true
	}

	/// The topmost placement whose sprite paints map pixel `(px, py)` - what a
	/// click there picks up. Later placements draw over earlier ones, so the
	/// search runs backwards. Shadow counts: a shadow is part of the object, and
	/// grabbing one is how you grab an object whose body is off screen.
	pub fn scenery_at(&self, px: i32, py: i32) -> Option<usize> {
		self.scenery.iter().enumerate().rev().find_map(|(index, spot)| {
			let piece = self.scenery_piece(spot)?;
			let (ox, oy) = piece.sprite_origin(spot);
			let (body, shade) = piece.texel(px - ox, py - oy);
			(body != 0 || shade != 0).then_some(index)
		})
	}

	/// The pass a placement imposes on cell `(x, y)`, or `None` when no
	/// placement covers enough of it. Coverage is measured in *body* pixels -
	/// a shadow falls on ground that stays walkable - and the value comes from
	/// the source template's own pass grid, so an object blocks exactly the
	/// cells its template blocked. The topmost qualifying placement wins.
	pub fn scenery_pass_at(&self, x: u16, y: u16) -> Option<u8> {
		if self.scenery.is_empty() {
			return None;
		}
		let (cx, cy) = (x as i32 * TILE_SIZE as i32, y as i32 * TILE_SIZE as i32);
		self.scenery.iter().rev().find_map(|spot| {
			let piece = self.scenery_piece(spot)?;
			let pass = piece.pass_under(spot, cx + TILE_SIZE as i32 / 2, cy + TILE_SIZE as i32 / 2)?;
			let (ox, oy) = piece.sprite_origin(spot);
			let mut covered = 0usize;
			for py in cy..cy + TILE_SIZE as i32 {
				for px in cx..cx + TILE_SIZE as i32 {
					if piece.texel(px - ox, py - oy).0 != 0 {
						covered += 1;
					}
				}
			}
			(covered >= SCENERY_PASS_COVERAGE).then_some(pass)
		})
	}

	fn snapshot_scenery(&mut self) {
		match &mut self.stroke {
			Some(stroke) => {
				if stroke.scenery.is_none() {
					stroke.scenery = Some(self.scenery.clone());
				}
			}
			None => {
				let scenery = Some(self.scenery.clone());
				self.push_undo(Patch { scenery, ..Patch::default() });
			}
		}
	}

	/// Mark every cell a placement's sprite reaches as needing a re-upload -
	/// the composed pixels there changed, and so may the pass.
	fn mark_scenery_dirty(&mut self, spot: &ScenerySpot) {
		let Some(piece) = self.scenery_piece(spot) else { return };
		let (ox, oy) = piece.sprite_origin(spot);
		let x0 = ox.div_euclid(TILE_SIZE as i32).clamp(0, self.width as i32 - 1) as u16;
		let y0 = oy.div_euclid(TILE_SIZE as i32).clamp(0, self.height as i32 - 1) as u16;
		let x1 =
			(ox + piece.sprite.width as i32 - 1).div_euclid(TILE_SIZE as i32).clamp(0, self.width as i32 - 1) as u16;
		let y1 =
			(oy + piece.sprite.height as i32 - 1).div_euclid(TILE_SIZE as i32).clamp(0, self.height as i32 - 1) as u16;
		for y in y0..=y1 {
			for x in x0..=x1 {
				self.mark_render_cell(x, y);
			}
		}
	}

	/// The effective maximum stats ([`UnitValues`]) of object `index`: its per-unit
	/// override ([`ObjectProps::base_values`]) when edited, else the shared seed
	/// resolved from the opened save (the source unit's `base_values`, S4.5).
	/// `None` for a fresh placement (no save, no override) or a record with no
	/// stats block (e.g. some ground cover) — callers then treat max stats as
	/// unknown/unbounded, exactly as the hits cap already does.
	pub fn object_base_values(&self, index: usize) -> Option<UnitValues> {
		let obj = self.objects.get(index)?;
		if let Some(values) = &obj.props.base_values {
			return Some(values.clone());
		}
		let id = obj.props.source_id?;
		let save = self.save.as_ref()?;
		let rec = save.file.units().find(|u| u.id == id)?;
		save.file.values(rec.base_values?).cloned()
	}

	/// Replace the editable state (owner `team` + gameplay [`ObjectProps`]) of
	/// object `index` — the Unit Properties panel's prop edits (S4). Undoable
	/// (each edit its own patch outside a stroke); `false` (no patch) when the
	/// index is out of range or nothing changed. The sprite frame and map label
	/// re-derive from the new state on the next frame (`angle` → frame, `name` →
	/// label), so an edit shows live. Position (`x`/`y`) is moved separately via
	/// [`Self::move_object_to`]; this never touches it.
	pub fn set_object_state(&mut self, index: usize, team: u8, props: ObjectProps) -> bool {
		let Some(o) = self.objects.get(index) else { return false };
		if o.team == team && o.props == props {
			return false;
		}
		self.snapshot_objects();
		self.objects[index].team = team;
		self.objects[index].props = props;
		self.redo_stack.clear();
		self.bump();
		true
	}

	/// Remove the object on a cell; `true` when one was there. Undoable (records
	/// no patch when nothing was removed).
	pub fn remove_object_at(&mut self, x: u16, y: u16) -> bool {
		if !self.objects.iter().any(|o| (o.x, o.y) == (x, y)) {
			return false;
		}
		self.snapshot_objects();
		self.objects.retain(|o| (o.x, o.y) != (x, y));
		self.redo_stack.clear();
		self.bump();
		true
	}

	/// Remove every object; returns how many there were. Undoable (records no
	/// patch when already empty).
	pub fn clear_objects(&mut self) -> usize {
		let n = self.objects.len();
		if n == 0 {
			return 0;
		}
		self.snapshot_objects();
		self.objects.clear();
		self.redo_stack.clear();
		self.bump();
		n
	}

	/// Auto-connect adjacent same-team **connector hosts** (buildings, the 4-way
	/// connector, standalone fixtures) via their `connectors` mask — mirroring the
	/// game's behaviour when you build structures next to each other. For every
	/// host, each half-edge whose neighbour cell (per the engine's connector
	/// geometry, `unit_size = 2` for a building else `1`) is covered by a same-team
	/// host gets its bit **set**. **Add-only** (never clears an existing bit), so it
	/// only ever *ensures* connections — a loaded save's exact mask (incl. any
	/// deliberately-broken links) is preserved, and running it twice is idempotent.
	/// Undoable as one patch; returns whether anything changed. `false` (no patch)
	/// when everything is already connected.
	pub fn auto_connect_buildings(&mut self) -> bool {
		use max_assets::save::{is_building_type, is_connector_host_type};
		let size = |ut: u16| if is_building_type(ut) { 2i32 } else { 1 };
		// Map every connector host's footprint cells → (team, object index).
		let mut occ: HashMap<(i32, i32), (u8, usize)> = HashMap::new();
		for (i, o) in self.objects.iter().enumerate() {
			if !is_connector_host_type(o.unit_type) {
				continue;
			}
			let s = size(o.unit_type);
			for dx in 0..s {
				for dy in 0..s {
					occ.insert((o.x as i32 + dx, o.y as i32 + dy), (o.team, i));
				}
			}
		}
		// Compute each host's add-only new mask from same-team adjacency.
		let mut updates: Vec<(usize, u16)> = Vec::new();
		for (i, o) in self.objects.iter().enumerate() {
			if !is_connector_host_type(o.unit_type) {
				continue;
			}
			let s = size(o.unit_type);
			let valid: u16 = if s == 2 { 0xFF } else { 0x55 };
			let (x, y) = (o.x as i32, o.y as i32);
			let mut add = 0u16;
			for bit in CONNECTOR_BITS {
				if bit & valid == 0 {
					continue; // NR/EB/SR/WB don't exist on a 1×1
				}
				let cell = connector_neighbor(x, y, s, bit);
				if occ.get(&cell).is_some_and(|&(team, ni)| team == o.team && ni != i) {
					add |= bit;
				}
			}
			let new = o.props.connectors | add;
			if new != o.props.connectors {
				updates.push((i, new));
			}
		}
		if updates.is_empty() {
			return false;
		}
		self.snapshot_objects();
		for (i, mask) in updates {
			self.objects[i].props.connectors = mask;
		}
		self.redo_stack.clear();
		self.bump();
		true
	}

	/// Resolve a `"GSd004:!N"`-style reference to a tile ref + its layer
	/// (water-pack tiles go to the water layer, the rest to ground).
	pub fn resolve_ref(&self, text: &str) -> Result<(TileRef, usize), String> {
		let (id, transform) = match text.split_once(':') {
			Some((id, t)) => (id, Transform::parse(t)?),
			None => (text, Transform::default()),
		};
		for (pack_index, pack) in self.packs.iter().enumerate() {
			if let Some(&tile) = pack.index_of.get(id) {
				let pack_index = pack_index as u8;
				let layer = if Some(pack_index) == self.water_pack { LAYER_WATER } else { LAYER_GROUND };
				return Ok((TileRef { pack: pack_index, tile, transform }, layer));
			}
		}
		Err(format!("unknown tile id '{id}'"))
	}

	/// Encode a cell's stack in the save format (`"WTR005,GSd004:!N"`,
	/// empty string for an empty stack) - also the `assert-cell` syntax.
	pub fn cell_spec(&self, x: u16, y: u16) -> Option<String> {
		let stack = self.cell(x, y)?;
		let mut text = String::new();
		for layer in stack.iter().flatten() {
			if !text.is_empty() {
				text.push(',');
			}
			text.push_str(&self.packs[layer.pack as usize].ids[layer.tile as usize]);
			text.push_str(&layer.transform.suffix());
		}
		Some(text)
	}

	/// Set layer entries (one undo transaction - or part of the open stroke);
	/// `None` erases. Out-of-range and no-op edits are skipped; returns
	/// whether anything changed.
	pub fn place_many(&mut self, edits: &[(u16, u16, usize, Option<TileRef>)]) -> bool {
		let mut cells = Vec::new();
		for &(x, y, layer, entry) in edits {
			if x >= self.width || y >= self.height || layer >= MAX_LAYERS {
				continue;
			}
			if let Some(t) = entry {
				let Some(pack) = self.packs.get(t.pack as usize) else { continue };
				if t.tile >= pack.tile_count() {
					continue;
				}
			}
			let i = y as usize * self.width as usize + x as usize;
			if self.cells[i][layer] == entry {
				continue;
			}
			cells.push((x, y, layer, self.cells[i][layer]));
			self.cells[i][layer] = entry;
			self.mark_render_cell(x, y);
		}
		if cells.is_empty() {
			return false;
		}
		match &mut self.stroke {
			Some(stroke) => stroke.cells.extend(cells),
			None => {
				self.push_undo(Patch { cells, ..Patch::default() });
			}
		}
		self.redo_stack.clear();
		self.bump();
		true
	}

	/// Set a dynamic palette slot (the map's color override). Undoable; part
	/// of the open stroke when one is active (slider drags = one undo unit).
	pub fn set_color(&mut self, slot: u8, rgb: [u8; 3]) -> Result<bool, String> {
		if !DYNAMIC_SLOTS.contains(&slot) {
			return Err(format!("slot {slot} is game-static (editable: 64..=159)"));
		}
		let at = slot as usize * 3;
		let prev = [self.palette[at], self.palette[at + 1], self.palette[at + 2]];
		if prev == rgb {
			return Ok(false);
		}
		self.palette[at..at + 3].copy_from_slice(&rgb);
		match &mut self.stroke {
			Some(stroke) => stroke.colors.push((slot, prev)),
			None => {
				self.push_undo(Patch { colors: vec![(slot, prev)], ..Patch::default() });
			}
		}
		self.redo_stack.clear();
		self.bump();
		Ok(true)
	}

	/// Shift a whole water cycle block (the one containing `slot`) in HSL -
	/// re-tints the animated gradient coherently. One undo unit.
	/// `dh` degrees, `ds`/`dl` in 0..1 units.
	pub fn hsl_shift_block(&mut self, slot: u8, dh: f32, ds: f32, dl: f32) -> Result<bool, String> {
		let Some(&(start, end)) = WATER_CYCLES.iter().find(|(s, e)| (*s..=*e).contains(&slot)) else {
			return Err(format!(
				"slot {slot} is not in a water cycle block (blocks: 96-102 103-109 110-116 117-122 123-127)",
			));
		};
		let solo = self.stroke.is_none();
		if solo {
			self.begin_stroke();
		}
		let mut changed = false;
		for s in start..=end {
			let at = s as usize * 3;
			let (h, sat, l) = crate::color::rgb_to_hsl([self.palette[at], self.palette[at + 1], self.palette[at + 2]]);
			changed |= self.set_color(s, crate::color::hsl_to_rgb(h + dh, sat + ds, l + dl))?;
		}
		if solo {
			self.end_stroke();
		}
		Ok(changed)
	}

	/// Apply a loaded 256-colour palette (768 RGB bytes) to the editable
	/// dynamic slots (64..=159) as one undo unit; the game-static slots are
	/// left untouched. Returns how many slots changed.
	pub fn load_palette(&mut self, colors: &[u8]) -> Result<u32, String> {
		if colors.len() != 768 {
			return Err(format!("palette: {} bytes, want 768", colors.len()));
		}
		let solo = self.stroke.is_none();
		if solo {
			self.begin_stroke();
		}
		let mut changed = 0;
		for slot in 64u8..=159 {
			let at = slot as usize * 3;
			if self.set_color(slot, [colors[at], colors[at + 1], colors[at + 2]])? {
				changed += 1;
			}
		}
		if solo {
			self.end_stroke();
		}
		Ok(changed)
	}

	/// The document's internal palette: the source file's bytes (the WRL's
	/// internal palette / the pack's `palette.json` - game statics **not**
	/// applied) with this map's live dynamic-slot edits merged in. What the
	/// game would ignore, but what the file actually says - the debug render
	/// (`map-palette`) and the WRL Internal Palette panel read this.
	pub fn internal_palette(&self) -> Vec<u8> {
		let mut out = self.source_palette.clone();
		for slot in DYNAMIC_SLOTS {
			let at = slot as usize * 3;
			out[at..at + 3].copy_from_slice(&self.palette[at..at + 3]);
		}
		out
	}

	/// Is this document an imported flat WRL (a synthetic in-memory pack)?
	/// Palette conversion rewrites tile pixels, which only makes sense when
	/// the tiles belong to the document (a `.json` project's packs are shared
	/// on disk - mutating them would not persist).
	pub fn is_wrl_import(&self) -> bool {
		!self.uses.is_empty() && self.uses.iter().all(|u| u.version == "wrl")
	}

	/// Apply Map Metadata (all optional) and mark the document dirty. These
	/// are metadata - never baked into the WRL, never part of undo. Carriage
	/// returns in the description are stripped; newlines are kept, so it may be
	/// multi-line (escaped as `\n` in the project JSON).
	pub fn set_info(
		&mut self,
		name: String,
		players: Option<u8>,
		description: String,
		date: String,
		map_version: String,
		author: String,
	) {
		self.name = name;
		self.players = players.map(|p| p.clamp(2, 4));
		self.description = description.replace('\r', "");
		self.date = date;
		self.map_version = map_version;
		self.author = author;
		self.dirty = true;
	}

	/// Snapshot everything a document-level operation may replace - the undo
	/// half of a [`Patch::doc`] swap.
	fn doc_state(&self) -> Box<DocState> {
		Box::new(DocState {
			uses: self.uses.clone(),
			packs: self.packs.clone(),
			cells: self.cells.clone(),
			pass_overrides: self.pass_overrides.clone(),
			palette: self.palette.clone(),
			pack_palette: self.pack_palette.clone(),
			source_palette: self.source_palette.clone(),
			water_pack: self.water_pack,
		})
	}

	/// Commit a document-level change as one undo unit: `before` is the
	/// pre-change snapshot (see [`Self::doc_state`]). Structural - the
	/// renderer must rebuild its atlas (see [`Self::structure_revision`]).
	fn push_doc_patch(&mut self, before: Box<DocState>) {
		self.end_stroke(); // a doc swap must not interleave with an open stroke
		self.push_undo(Patch { doc: Some(before), ..Patch::default() });
		self.redo_stack.clear();
		self.structure += 1;
		self.bump();
	}

	/// Per-slot pixel usage over every pack's tile table.
	fn slot_usage(&self) -> [u64; 256] {
		let mut usage = [0u64; 256];
		for pack in &self.packs {
			for &b in &pack.tiles {
				usage[b as usize] += 1;
			}
		}
		usage
	}

	/// Remap the internal palette onto a MAX-compatible one (the "best match
	/// colors" method - see [`crate::palette_convert`] for the rules: only
	/// used colors move, game-animated slots are never used, water cycles are
	/// preserved per `opts`, in-game statics are reused when possible and the
	/// rest approximate into the unused dynamic slots). Tile pixels are
	/// rewritten through the slot mapping, so the rendered map keeps
	/// (approximately) its internal-palette look while becoming game-correct.
	///
	/// Lossy but undoable - the change lands as one document-swap undo unit.
	/// `None` when the palette is already compatible (nothing changed).
	pub fn convert_to_compatible_palette(
		&mut self,
		opts: crate::palette_convert::ConvertOptions,
	) -> Option<crate::palette_convert::ConvertReport> {
		let internal = self.internal_palette();
		let plan = crate::palette_convert::plan(&internal, &self.slot_usage(), opts)?;
		let before = self.doc_state();
		for pack in &mut self.packs {
			for b in &mut pack.tiles {
				*b = plan.map[*b as usize];
			}
		}
		// The compatible palette becomes the document's palette on every
		// level: the working copy, the source ("internal") palette - they now
		// agree - and the owner pack's (the save/export baseline).
		self.palette = plan.palette.clone();
		self.source_palette = plan.palette.clone();
		self.pack_palette = plan.palette.clone();
		for (i, u) in self.uses.iter().enumerate() {
			if u.palette {
				self.packs[i].palette = Some(plan.palette.clone());
			}
		}
		self.push_doc_patch(before);
		Some(plan.report)
	}

	/// Convert the palette by rasterizing the whole map through its internal
	/// palette and re-importing the raster exactly like New-from-Image does
	/// (k-means quantization into the dynamic slots + dither + reblock +
	/// dedupe). With `preserve_water`, pixels on the water cycle blocks
	/// (96-127) are pinned: they keep their slot and the blocks keep the
	/// map's colors, so the water still animates in-game. Per-cell pass
	/// values survive as pass overrides (the rebuilt tiles carry none).
	///
	/// Lossy but undoable - one document-swap undo unit. Errors leave the
	/// document untouched.
	pub fn convert_palette_by_reimport(
		&mut self,
		preserve_water: bool,
		dedupe: crate::image_import::Dedupe,
		threshold: f32,
	) -> Result<u16, String> {
		let mut session = PaletteReimport::new(self, preserve_water, dedupe, threshold);
		while !session.is_done() {
			session.step(self, usize::MAX);
		}
		let wrl = session.finish()?;
		Ok(self.apply_reimport(&wrl))
	}

	/// Swap a re-imported [`WrlFile`] (see [`PaletteReimport`]) in as the
	/// document's content - one document-swap undo unit. Pass truth lives in
	/// per-cell overrides afterwards (the reimported tiles carry none).
	pub fn apply_reimport(&mut self, wrl: &WrlFile) -> u16 {
		let (w, h) = (self.width as usize, self.height as usize);
		let before = self.doc_state();
		let pass_overrides = (0..h * w).map(|i| self.pass_at((i % w) as u16, (i / w) as u16)).collect();
		let name = self.uses.first().map_or_else(|| self.name.clone(), |u| u.name.clone());
		let rebuilt = Self::from_wrl(wrl, &name);
		self.uses = rebuilt.uses;
		self.packs = rebuilt.packs;
		self.cells = rebuilt.cells;
		self.pass_overrides = pass_overrides;
		self.palette = rebuilt.palette;
		self.pack_palette = rebuilt.pack_palette;
		self.source_palette = rebuilt.source_palette;
		self.water_pack = rebuilt.water_pack;
		self.push_doc_patch(before);
		wrl.tile_count
	}

	/// Open a stroke: subsequent edits merge into one undo unit (one brush
	/// drag = one Ctrl+Z). An already-open stroke is committed first.
	pub fn begin_stroke(&mut self) {
		self.end_stroke();
		self.stroke = Some(Patch::default());
	}

	/// Whether a stroke is currently open (edits are coalescing into one undo
	/// unit). Lets a caller that already opened the stroke — a unit-place drag —
	/// avoid nesting `begin_stroke`/`end_stroke` (which would split the drag).
	pub fn in_stroke(&self) -> bool {
		self.stroke.is_some()
	}

	/// Whether the open stroke has already edited the object list — i.e. at
	/// least one object was placed/removed since `begin_stroke`. `false` outside
	/// a stroke. Distinguishes the first placement of a place-tool drag (the
	/// press, which keeps restamp-on-click semantics) from the drag's
	/// continuation cells (which must not overpaint what the drag just laid).
	pub fn stroke_touched_objects(&self) -> bool {
		self.stroke.as_ref().is_some_and(|s| s.objects.is_some())
	}

	/// Abort the open stroke: revert its edits right now and discard them -
	/// nothing lands on the undo/redo stacks. A cancelled generation
	/// (worldgen) never happened.
	pub fn rollback_stroke(&mut self) -> bool {
		let Some(stroke) = self.stroke.take() else { return false };
		if stroke.is_empty() {
			return false;
		}
		let _ = self.apply(stroke);
		self.bump();
		true
	}

	/// Commit the open stroke to the undo stack (no-op when empty/closed).
	pub fn end_stroke(&mut self) {
		let Some(stroke) = self.stroke.take() else { return };
		if stroke.is_empty() {
			return;
		}
		self.push_undo(stroke);
	}

	pub fn place(&mut self, x: u16, y: u16, layer: usize, entry: Option<TileRef>) -> bool {
		self.place_many(&[(x, y, layer, entry)])
	}

	/// A random interchangeable variant of `t` (same pack + transform); returns
	/// `t` unchanged when the tile has no variant group. The
	/// random-paint toggle swaps a placed tile for a sibling so a painted
	/// region doesn't visibly tile.
	pub fn random_variant(&self, t: TileRef, rng: &mut Rng) -> TileRef {
		let Some(pack) = self.packs.get(t.pack as usize) else { return t };
		let group = pack.variants_of(t.tile);
		if group.len() < 2 {
			return t;
		}
		TileRef { tile: group[rng.below(group.len() as u32) as usize], ..t }
	}

	/// The first pack + variant-group family of the given `kind` (LAND, WATER,
	/// ...), chosen by **sorted** family name so the pick is deterministic
	/// (`HashMap` order is not). The terrain generator and the terrain brush both
	/// resolve "land"/"water" tiles through this, so a hand-painted coast matches
	/// a generated one. `(pack index, family name)`; `None` if no pack ships one.
	pub fn variant_family(&self, kind: crate::pack::TileKind) -> Option<(usize, String)> {
		self.variant_family_in(kind, None)
	}

	/// Whether tile `(pack, tile)` may carry orientation `t` - its family's
	/// `Transformable` permits it (the 8-orientation grid greys out the rest).
	pub fn tile_allows(&self, pack: u8, tile: u16, t: Transform) -> bool {
		let kind = self.packs[pack as usize].tile_transformable(tile);
		crate::template::family_allows(kind, t)
	}

	/// Like [`Self::variant_family`], but preferring pack `preferred` when it
	/// ships a matching variant family - so the terrain brush's land follows the
	/// active tile's tileset. Falls back to the global first-pack scan when the
	/// preferred pack has no family of that `kind`.
	pub fn variant_family_in(&self, kind: crate::pack::TileKind, preferred: Option<usize>) -> Option<(usize, String)> {
		let first_family = |pack: &crate::pack::TilePack| -> Option<String> {
			let mut families: Vec<&String> =
				pack.props.iter().filter(|(_, fp)| fp.kind == Some(kind) && fp.has_variants).map(|(f, _)| f).collect();
			families.sort();
			families.first().map(|f| (*f).clone())
		};
		if let Some(pack) = preferred.and_then(|i| self.packs.get(i)) {
			if let Some(f) = first_family(pack) {
				return Some((preferred.unwrap(), f));
			}
		}
		self.packs.iter().enumerate().find_map(|(i, pack)| first_family(pack).map(|f| (i, f)))
	}

	/// Flood-fill (4-connected) the region of cells whose `layer` entry equals
	/// the clicked cell's, replacing each with `entry` - or a random variant of
	/// it when `randomize`. One undo unit; returns whether anything changed.
	pub fn fill(&mut self, x: u16, y: u16, entry: TileRef, layer: usize, randomize: bool, rng: &mut Rng) -> bool {
		if x >= self.width || y >= self.height || layer >= MAX_LAYERS {
			return false;
		}
		let w = self.width as usize;
		let h = self.height as usize;
		let idx = |x: u16, y: u16| y as usize * w + x as usize;
		let target = self.cells[idx(x, y)][layer];
		// Flood the connected run of `target` cells, collecting indices in pop
		// order; the tile (and its rng-rolled variant) is resolved afterwards so
		// the rng-consumption order matches the original in-traversal version.
		let mut seen = vec![false; w * h];
		let mut visited = Vec::new();
		crate::grid::flood4(w, h, idx(x, y), &mut seen, |n| self.cells[n][layer] == target, |i| visited.push(i));
		let edits: Vec<_> = visited
			.iter()
			.map(|&i| {
				let tile = if randomize { self.random_variant(entry, rng) } else { entry };
				((i % w) as u16, (i / w) as u16, layer, Some(tile))
			})
			.collect();
		self.place_many(&edits)
	}

	/// Set per-cell pass overrides (Pass Table Editor). Undoable -
	/// part of the open stroke when one is active (a paint drag = one undo
	/// unit). Returns whether anything changed.
	pub fn set_pass_many(&mut self, edits: &[(u16, u16, u8)]) -> bool {
		let mut passes = Vec::new();
		for &(x, y, value) in edits {
			if x >= self.width || y >= self.height || value > 3 {
				continue;
			}
			let i = y as usize * self.width as usize + x as usize;
			if self.pass_overrides[i] == Some(value) {
				continue;
			}
			passes.push((x, y, self.pass_overrides[i]));
			self.pass_overrides[i] = Some(value);
			self.mark_render_pass(x, y);
		}
		if passes.is_empty() {
			return false;
		}
		match &mut self.stroke {
			Some(stroke) => stroke.passes.extend(passes),
			None => {
				self.push_undo(Patch { passes, ..Patch::default() });
			}
		}
		self.redo_stack.clear();
		self.bump();
		true
	}

	pub fn set_pass(&mut self, x: u16, y: u16, value: u8) -> bool {
		self.set_pass_many(&[(x, y, value)])
	}

	/// Set (`Some`) or clear (`None`) a single cell's pass override (Local Pass
	/// Override Editor). Undoable - joins the open stroke. Returns whether
	/// anything changed.
	pub fn set_pass_override(&mut self, x: u16, y: u16, value: Option<u8>) -> bool {
		if x >= self.width || y >= self.height || value.is_some_and(|v| v > 3) {
			return false;
		}
		let i = y as usize * self.width as usize + x as usize;
		if self.pass_overrides[i] == value {
			return false;
		}
		let passes = vec![(x, y, self.pass_overrides[i])];
		self.pass_overrides[i] = value;
		self.mark_render_pass(x, y);
		match &mut self.stroke {
			Some(stroke) => stroke.passes.extend(passes),
			None => {
				self.push_undo(Patch { passes, ..Patch::default() });
			}
		}
		self.redo_stack.clear();
		self.bump();
		true
	}

	/// The full editable resource (cargo) map, row-major (empty when no save is
	/// attached). Read by the resource overlay / inspector (S5); edits go through
	/// [`Self::set_cargo`].
	pub fn cargo_map(&self) -> &[u16] {
		&self.cargo_map
	}

	/// The resource (cargo) `u16` at cell `(x, y)`, or `None` when out of range or
	/// no save is attached.
	pub fn cargo_at(&self, x: u16, y: u16) -> Option<u16> {
		if x >= self.width || y >= self.height {
			return None;
		}
		self.cargo_map.get(y as usize * self.width as usize + x as usize).copied()
	}

	/// Set the resource (cargo) `u16` at cell `(x, y)` (S5). Undoable — joins the
	/// open stroke (drag-paint) or commits its own patch. Returns whether anything
	/// changed; a no-op (unchanged value or out-of-range cell). On a save-less
	/// project the map materializes zero-filled on first paint (Stage D:
	/// resources are placeable on any map; save synthesis carries them).
	pub fn set_cargo(&mut self, x: u16, y: u16, value: u16) -> bool {
		if x >= self.width || y >= self.height {
			return false;
		}
		let cells = self.width as usize * self.height as usize;
		if self.cargo_map.len() != cells {
			self.cargo_map = vec![0; cells];
		}
		let i = y as usize * self.width as usize + x as usize;
		let Some(cur) = self.cargo_map.get_mut(i) else {
			return false;
		};
		if *cur == value {
			return false;
		}
		let resources = vec![(x, y, *cur)];
		*cur = value;
		self.mark_render_pass(x, y);
		match &mut self.stroke {
			Some(stroke) => stroke.resources.extend(resources),
			None => self.push_undo(Patch { resources, ..Patch::default() }),
		}
		self.redo_stack.clear();
		self.bump();
		true
	}

	/// Drop every per-cell pass override back to the derived value (undoable;
	/// joins the open stroke). Wholesale terrain replacement (worldgen)
	/// must not inherit stale hand-painted pass data.
	pub fn clear_pass_overrides(&mut self) -> bool {
		let mut passes = Vec::new();
		for i in 0..self.pass_overrides.len() {
			if let Some(prev) = self.pass_overrides[i].take() {
				let (x, y) = ((i % self.width as usize) as u16, (i / self.width as usize) as u16);
				passes.push((x, y, Some(prev)));
				self.mark_render_pass(x, y);
			}
		}
		if passes.is_empty() {
			return false;
		}
		match &mut self.stroke {
			Some(stroke) => stroke.passes.extend(passes),
			None => {
				self.push_undo(Patch { passes, ..Patch::default() });
			}
		}
		self.redo_stack.clear();
		self.bump();
		true
	}

	/// Set the **per-tile** passability of the tile under each cell (the Pass
	/// Table Editor): looks up the cell's top tile and rewrites its pack pass,
	/// so every cell sharing that tile id retints at once. Undoable - joins the
	/// open stroke (a drag = one unit). Cells whose top tile's pack has no pass
	/// table are skipped. Returns whether anything changed.
	pub fn set_tile_pass_at_many(&mut self, edits: &[(u16, u16, u8)]) -> bool {
		let mut tile_passes: Vec<(u8, u16, u8)> = Vec::new();
		for &(x, y, value) in edits {
			if value > 3 {
				continue;
			}
			let (pack, tile) = {
				let Some(stack) = self.cell(x, y) else { continue };
				let Some(top) = stack[LAYER_GROUND].or(stack[LAYER_WATER]) else { continue };
				(top.pack, top.tile)
			};
			let Some(pass) = self.packs[pack as usize].pass.as_mut() else { continue };
			let prev = pass[tile as usize];
			if prev == value {
				continue;
			}
			pass[tile as usize] = value;
			// One tile may sit under many painted cells - record its original
			// pass once so undo restores it exactly.
			if !tile_passes.iter().any(|&(p, t, _)| p == pack && t == tile) {
				tile_passes.push((pack, tile, prev));
			}
		}
		if tile_passes.is_empty() {
			return false;
		}
		// A per-tile pass edit retints every cell using that tile, anywhere.
		self.mark_render_pass_all();
		match &mut self.stroke {
			Some(stroke) => {
				for (pack, tile, prev) in tile_passes {
					if !stroke.tile_passes.iter().any(|&(p, t, _)| p == pack && t == tile) {
						stroke.tile_passes.push((pack, tile, prev));
					}
				}
			}
			None => {
				self.push_undo(Patch { tile_passes, ..Patch::default() });
			}
		}
		self.redo_stack.clear();
		self.bump();
		true
	}

	pub fn set_tile_pass_at(&mut self, x: u16, y: u16, value: u8) -> bool {
		self.set_tile_pass_at_many(&[(x, y, value)])
	}

	/// Reset every tile's pack pass to the canonical tileset value. `canonical`
	/// is indexed by pack: `Some(pass)` gives that pack's authoritative per-tile
	/// pass (already mapped to this pack's current tile indices by the caller),
	/// `None` leaves the pack untouched (a synthetic pack has no source tileset).
	/// Applies as one undo unit; per-cell pass overrides are untouched (this only
	/// reverts Pass Table Editor edits). Returns whether anything changed.
	pub fn reset_tile_pass(&mut self, canonical: &[Option<Vec<u8>>]) -> bool {
		self.end_stroke(); // a deliberate whole-map reset is never part of a stroke
		let mut tile_passes: Vec<(u8, u16, u8)> = Vec::new();
		for (pi, pack) in self.packs.iter_mut().enumerate() {
			let Some(want) = canonical.get(pi).and_then(|o| o.as_ref()) else { continue };
			let Some(pass) = pack.pass.as_mut() else { continue };
			for ti in 0..pass.len().min(want.len()) {
				if pass[ti] != want[ti] {
					tile_passes.push((pi as u8, ti as u16, pass[ti]));
					pass[ti] = want[ti];
				}
			}
		}
		if tile_passes.is_empty() {
			return false;
		}
		self.mark_render_pass_all(); // pack pass changed → every cell's derived pass may retint
		self.push_undo(Patch { tile_passes, ..Patch::default() });
		self.redo_stack.clear();
		self.bump();
		true
	}

	/// Set the water (base) layer tile by raw index - the flat-document edit
	/// behind `set-tile`, used to edit an imported WRL (its only tiles are the
	/// synthetic base pack). Validates against the base pack; `false` if out of
	/// range, off-map, or unchanged.
	pub fn set_base_tile(&mut self, x: u16, y: u16, tile: u16) -> bool {
		let pack = self.water_pack.unwrap_or(0);
		if x >= self.width || y >= self.height || tile >= self.packs[pack as usize].tile_count() {
			return false;
		}
		self.place(x, y, LAYER_WATER, Some(TileRef { pack, tile, transform: Transform::default() }))
	}

	/// The water (base) layer tile index at a cell (`set-tile`/`assert-tile`).
	pub fn base_tile(&self, x: u16, y: u16) -> Option<u16> {
		self.cell(x, y).and_then(|s| s[LAYER_WATER]).map(|t| t.tile)
	}

	/// Resize the canvas: the existing map is placed at
	/// `(off_x, off_y)` within the new `new_w × new_h` grid. Enlarging
	/// fills the new territory with water; a negative offset (or a smaller
	/// size) crops. Cell stacks and pass overrides move together. This is a
	/// structural change, so the per-cell undo journal is cleared.
	pub fn resize(&mut self, new_w: u16, new_h: u16, off_x: i32, off_y: i32) -> Result<(), String> {
		check_map_size(new_w, new_h)?;
		let water = self.water_pack;
		let water_tiles = water.and_then(|w| self.packs.get(w as usize)).map(|p| p.tile_count()).unwrap_or(0);
		let mut cells = Vec::with_capacity(new_w as usize * new_h as usize);
		let mut passes = Vec::with_capacity(new_w as usize * new_h as usize);
		for ny in 0..new_h as i32 {
			for nx in 0..new_w as i32 {
				let (ox, oy) = (nx - off_x, ny - off_y);
				if ox >= 0 && oy >= 0 && (ox as u16) < self.width && (oy as u16) < self.height {
					let oi = oy as usize * self.width as usize + ox as usize;
					cells.push(self.cells[oi]);
					passes.push(self.pass_overrides[oi]);
				} else {
					// New territory fills with water (deterministic per cell).
					let mut stack = [None; MAX_LAYERS];
					if let (Some(w), true) = (water, water_tiles > 0) {
						let mut rng = Rng::new(0x5245_5349_5a45 ^ ((nx as u64) << 32 | ny as u64));
						stack[LAYER_WATER] = Some(TileRef {
							pack: w,
							tile: rng.below(water_tiles as u32) as u16,
							transform: Transform::default(),
						});
					}
					cells.push(stack);
					passes.push(None);
				}
			}
		}
		self.cells = cells;
		self.pass_overrides = passes;
		self.width = new_w;
		self.height = new_h;
		// A dimension change can't be a per-cell patch - drop the journal.
		self.undo_stack.clear();
		self.redo_stack.clear();
		self.stroke = None;
		self.bump();
		Ok(())
	}

	/// Push a finished patch onto the undo journal, dropping the oldest once
	/// the stack exceeds [`MAX_UNDO`]. (The caller clears `redo_stack` / bumps
	/// the revision as appropriate - this only manages the bounded stack.)
	fn push_undo(&mut self, mut patch: Patch) {
		// Label the committed patch: the app's hint if set, else derived from the
		// contents. `apply` carries the label through undo/redo.
		patch.label = self.pending_label.take().unwrap_or_else(|| patch.default_label());
		self.undo_stack.push(patch);
		if self.undo_stack.len() > MAX_UNDO {
			self.undo_stack.remove(0);
		}
		self.undo_seq = self.undo_seq.wrapping_add(1);
	}

	pub fn undo(&mut self) -> bool {
		self.end_stroke(); // a mid-drag undo must not orphan the stroke
		let Some(patch) = self.undo_stack.pop() else { return false };
		let inverse = self.apply(patch);
		self.redo_stack.push(inverse);
		self.undo_seq = self.undo_seq.wrapping_add(1);
		self.bump();
		true
	}

	pub fn redo(&mut self) -> bool {
		self.end_stroke();
		let Some(patch) = self.redo_stack.pop() else { return false };
		let inverse = self.apply(patch);
		self.undo_stack.push(inverse);
		self.undo_seq = self.undo_seq.wrapping_add(1);
		self.bump();
		true
	}

	/// Label the next committed undo patch (the app calls this with an action
	/// name before an editing command; unlabelled patches derive one from their
	/// contents). Overwritten by a later call before the patch commits.
	pub fn label_next_undo(&mut self, label: impl Into<String>) {
		self.pending_label = Some(label.into());
	}

	/// A monotonically-increasing counter that changes only when the undo stack
	/// changes (push / undo / redo) - a cheap "did the history change?" signal.
	pub fn undo_seq(&self) -> u64 {
		self.undo_seq
	}

	/// The labels of the most recent `max` undo-stack entries, newest first -
	/// the Undo History submenu. An open stroke isn't listed (it commits first).
	pub fn undo_labels(&self, max: usize) -> Vec<String> {
		self.undo_stack.iter().rev().take(max).map(|p| p.label.clone()).collect()
	}

	/// Undo `n` steps at once (the Undo History submenu jumps back to an entry).
	/// Returns how many actually ran. One undo unit each, like [`Self::undo`].
	pub fn undo_steps(&mut self, n: usize) -> usize {
		(0..n).take_while(|_| self.undo()).count()
	}

	fn apply(&mut self, patch: Patch) -> Patch {
		// A document swap is its own inverse: swap the stored state with the
		// live fields and carry the displaced state back out. Structural -
		// the renderer's atlas is stale either way.
		if let Some(mut doc) = patch.doc {
			std::mem::swap(&mut self.uses, &mut doc.uses);
			std::mem::swap(&mut self.packs, &mut doc.packs);
			std::mem::swap(&mut self.cells, &mut doc.cells);
			std::mem::swap(&mut self.pass_overrides, &mut doc.pass_overrides);
			std::mem::swap(&mut self.palette, &mut doc.palette);
			std::mem::swap(&mut self.pack_palette, &mut doc.pack_palette);
			std::mem::swap(&mut self.source_palette, &mut doc.source_palette);
			std::mem::swap(&mut self.water_pack, &mut doc.water_pack);
			self.structure += 1;
			return Patch { label: patch.label, doc: Some(doc), ..Patch::default() };
		}
		// A save-settings swap is its own inverse: apply the stored block, carry
		// the displaced one back out. Committed solo, so early-return like `doc`.
		// Degrades to an empty patch if the save vanished (a new project clears
		// the journal, so this shouldn't happen in practice).
		if let Some(stored) = patch.save_settings {
			// The inverse of a settings edit that landed: the tail it produced
			// decomposes by construction, so re-shaping it back cannot fail.
			let displaced = self.swap_save_settings(&stored).expect("an applied settings edit is reversible");
			return Patch { label: patch.label, save_settings: displaced.map(Box::new), ..Patch::default() };
		}
		// Objects swap wholesale (their own inverse), like a doc swap but able to
		// coexist with cell/color edits in one patch. `None` = untouched.
		let objects = patch.objects.map(|mut prev| {
			std::mem::swap(&mut self.objects, &mut prev);
			prev
		});
		// Scenery swaps wholesale too. Both lists are dirtied cell-wise so the
		// renderer re-uploads what the undo moved.
		let scenery = patch.scenery.map(|mut prev| {
			std::mem::swap(&mut self.scenery, &mut prev);
			let touched: Vec<ScenerySpot> = prev.iter().chain(&self.scenery).cloned().collect();
			for spot in &touched {
				self.mark_scenery_dirty(spot);
			}
			prev
		});
		let mut cells = Vec::with_capacity(patch.cells.len());
		for &(x, y, layer, entry) in patch.cells.iter().rev() {
			let i = y as usize * self.width as usize + x as usize;
			cells.push((x, y, layer, self.cells[i][layer]));
			self.cells[i][layer] = entry;
			self.mark_render_cell(x, y);
		}
		let mut colors = Vec::with_capacity(patch.colors.len());
		for &(slot, rgb) in patch.colors.iter().rev() {
			let at = slot as usize * 3;
			colors.push((slot, [self.palette[at], self.palette[at + 1], self.palette[at + 2]]));
			self.palette[at..at + 3].copy_from_slice(&rgb);
		}
		let mut passes = Vec::with_capacity(patch.passes.len());
		for &(x, y, value) in patch.passes.iter().rev() {
			let i = y as usize * self.width as usize + x as usize;
			passes.push((x, y, self.pass_overrides[i]));
			self.pass_overrides[i] = value;
			self.mark_render_pass(x, y);
		}
		let mut resources = Vec::with_capacity(patch.resources.len());
		for &(x, y, value) in patch.resources.iter().rev() {
			let i = y as usize * self.width as usize + x as usize;
			resources.push((x, y, self.cargo_map[i]));
			self.cargo_map[i] = value;
			self.mark_render_pass(x, y); // the resource overlay rides the pass texture region
		}
		if !patch.tile_passes.is_empty() {
			self.mark_render_pass_all(); // per-tile pass reverted → any cell using it retints
		}
		let mut tile_passes = Vec::with_capacity(patch.tile_passes.len());
		for &(pack, tile, value) in patch.tile_passes.iter().rev() {
			if let Some(pass) = self.packs[pack as usize].pass.as_mut() {
				tile_passes.push((pack, tile, pass[tile as usize]));
				pass[tile as usize] = value;
			}
		}
		Patch {
			label: patch.label,
			cells,
			colors,
			passes,
			resources,
			scenery,
			tile_passes,
			doc: None,
			objects,
			save_settings: None,
		}
	}

	fn bump(&mut self) {
		self.dirty = true;
		self.revision += 1;
	}

	/// Grow a dirty bbox to include cell `(x, y)`.
	fn grow_dirty(slot: &mut Option<(u16, u16, u16, u16)>, x: u16, y: u16) {
		*slot = Some(match *slot {
			Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
			None => (x, y, x, y),
		});
	}

	/// Note that cell `(x, y)`'s tile stack changed: the renderer must re-upload
	/// that cell, and its *derived* pass may have changed too, so it enters the
	/// pass region as well.
	fn mark_render_cell(&mut self, x: u16, y: u16) {
		Self::grow_dirty(&mut self.render_dirty_cells, x, y);
		Self::grow_dirty(&mut self.render_dirty_pass, x, y);
	}

	/// Note that cell `(x, y)`'s displayed pass value changed (a per-cell
	/// override edit - the tile stack is untouched).
	fn mark_render_pass(&mut self, x: u16, y: u16) {
		Self::grow_dirty(&mut self.render_dirty_pass, x, y);
	}

	/// Note that the pass overlay changed map-wide (a per-*tile* pass retint
	/// affects every cell using that tile, so the region can't be localized).
	fn mark_render_pass_all(&mut self) {
		if self.width > 0 && self.height > 0 {
			Self::grow_dirty(&mut self.render_dirty_pass, 0, 0);
			Self::grow_dirty(&mut self.render_dirty_pass, self.width - 1, self.height - 1);
		}
	}

	/// Drain the regions edited since the last call - the renderer re-uploads
	/// only these sub-rectangles instead of the whole map every frame.
	pub fn take_render_dirty(&mut self) -> RenderDirty {
		RenderDirty { cells: self.render_dirty_cells.take(), pass: self.render_dirty_pass.take() }
	}

	/// Drop any pending dirty region (the caller just rebuilt the renderer from
	/// scratch, so every texture is already current).
	pub fn clear_render_dirty(&mut self) {
		self.render_dirty_cells = None;
		self.render_dirty_pass = None;
	}

	/// Remove tile `tile` from pack `pack`, shifting every higher tile index
	/// down by one across the pack's tables, this map's cells, and the pack's
	/// variant groups / patterns. Refuses if the tile is still painted anywhere
	/// (erase it first). Not undoable - a deliberate asset edit, like Bake.
	pub fn delete_tile(&mut self, pack: u8, tile: u16) -> Result<(), String> {
		let pk = pack as usize;
		let Some(p) = self.packs.get_mut(pk) else { return Err(format!("no pack {pack}")) };
		if tile >= p.tile_count() {
			return Err(format!("tile {tile} out of range"));
		}
		let id = p.ids[tile as usize].clone();
		// In use? (a cell still references this exact tile.)
		let used = self.cells.iter().flatten().flatten().any(|t| t.pack == pack && t.tile == tile);
		if used {
			return Err(format!("'{id}' is painted on the map - erase it first"));
		}
		// Drop the tile from the pack tables.
		let p = &mut self.packs[pk];
		let at = tile as usize * TILE_DATA_SIZE;
		p.tiles.drain(at..at + TILE_DATA_SIZE);
		p.ids.remove(tile as usize);
		p.variant_of.remove(tile as usize);
		if let Some(pass) = p.pass.as_mut() {
			pass.remove(tile as usize);
		}
		// Variant groups hold tile indices: drop the deleted one, shift the rest.
		for group in &mut p.variant_groups {
			group.retain(|&i| i != tile);
			for i in group.iter_mut() {
				if *i > tile {
					*i -= 1;
				}
			}
		}
		// Patterns reference tile indices too; a hole where the tile was used.
		for pat in &mut p.patterns {
			for cell in pat.cells.iter_mut() {
				match *cell {
					Some(i) if i == tile => *cell = None,
					Some(i) if i > tile => *cell = Some(i - 1),
					_ => {}
				}
			}
		}
		// Rebuild the id→index map (positions past `tile` all shifted).
		p.index_of = p.ids.iter().enumerate().map(|(i, id)| (id.clone(), i as u16)).collect();
		// Shift this map's cell references in the same pack.
		for stack in &mut self.cells {
			for t in stack.iter_mut().flatten() {
				if t.pack == pack && t.tile > tile {
					t.tile -= 1;
				}
			}
		}
		self.structure += 1;
		self.bump();
		Ok(())
	}

	pub fn cell(&self, x: u16, y: u16) -> Option<&[Option<TileRef>; MAX_LAYERS]> {
		if x >= self.width || y >= self.height {
			return None;
		}
		Some(&self.cells[y as usize * self.width as usize + x as usize])
	}

	/// Flatten one cell's stack to raw 64×64 indexed pixels: ground over
	/// water, ground index-0 = transparent. The bake kernel.
	pub fn compose_cell(&self, x: u16, y: u16) -> [u8; TILE_DATA_SIZE] {
		match self.cell(x, y) {
			Some(stack) => self.compose_stack(stack),
			None => [0u8; TILE_DATA_SIZE],
		}
	}

	/// Compose an arbitrary stack (used by the bake's water-phase
	/// canonicalization as well as `compose_cell`).
	pub fn compose_stack(&self, stack: &[Option<TileRef>; MAX_LAYERS]) -> [u8; TILE_DATA_SIZE] {
		let mut out = [0u8; TILE_DATA_SIZE];
		if let Some(water) = stack[LAYER_WATER] {
			let pixels = self.packs[water.pack as usize].tile_pixels(water.tile);
			transform_into(&mut out, pixels, water.transform, None);
		}
		if let Some(ground) = stack[LAYER_GROUND] {
			let pixels = self.packs[ground.pack as usize].tile_pixels(ground.tile);
			// Only families with a mask are transparent (over the water beneath);
			// opaque families fully cover.
			transform_into(&mut out, pixels, ground.transform, self.tile_mask(ground));
		}
		out
	}

	/// The transparency mask color of a tile - its family's `"mask"`, or `None`
	/// when the family is fully opaque.
	pub fn tile_mask(&self, t: TileRef) -> Option<u8> {
		self.packs[t.pack as usize].tile_mask(t.tile)
	}

	/// One composed pixel of a cell - the single-pixel form of
	/// `compose_cell` (O(1); minimap/overworld previews sample with this).
	pub fn pixel_at(&self, x: u16, y: u16, sub: (usize, usize)) -> u8 {
		let Some(stack) = self.cell(x, y) else { return 0 };
		if let Some(ground) = stack[LAYER_GROUND] {
			let pixel = self.tile_pixel(ground, sub);
			// Opaque family, or a non-mask pixel: the ground pixel wins.
			if self.tile_mask(ground) != Some(pixel) {
				return pixel;
			}
		}
		match stack[LAYER_WATER] {
			Some(water) => self.tile_pixel(water, sub),
			None => 0,
		}
	}

	/// A single tile pixel under its transform (the point form of
	/// `transform_into`'s inverse mapping).
	fn tile_pixel(&self, t: TileRef, (dx, dy): (usize, usize)) -> u8 {
		let n = TILE_SIZE;
		let (mut sx, mut sy) = (dx, dy);
		for _ in 0..t.transform.rot {
			let (rx, ry) = (sy, n - 1 - sx);
			sx = rx;
			sy = ry;
		}
		if t.transform.mirror {
			sx = n - 1 - sx;
		}
		self.packs[t.pack as usize].tile_pixels(t.tile)[sy * n + sx]
	}

	/// The in-game minimap byte for a cell (composed center pixel - the
	/// same derivation the bake uses).
	pub fn minimap_pixel(&self, x: u16, y: u16) -> u8 {
		self.pixel_at(x, y, (32, 32))
	}

	/// Pass value of a cell: the Pass Table Editor override if set, else the
	/// pass a scenery placement imposes ([`Self::scenery_pass_at`]), else the
	/// stack-top tile's pack pass (0 land / 1 water / 2 shore / 3 blocked).
	/// `None` when none is available. Empty stacks read as land (0). Drives the
	/// pass overlay and the bake, so all three agree by construction.
	pub fn pass_at(&self, x: u16, y: u16) -> Option<u8> {
		if x >= self.width || y >= self.height {
			return None;
		}
		let i = y as usize * self.width as usize + x as usize;
		if let Some(v) = self.pass_overrides[i] {
			return Some(v);
		}
		if let Some(v) = self.scenery_pass_at(x, y) {
			return Some(v);
		}
		let stack = self.cell(x, y)?;
		let Some(top) = stack[LAYER_GROUND].or(stack[LAYER_WATER]) else {
			return Some(0);
		};
		self.packs[top.pack as usize].pass.as_ref().map(|pass| pass[top.tile as usize])
	}

	/// How many cells read as each pass value, plus how many carry an explicit
	/// override: `([land, water, shore, blocked], overrides)`. Counted through
	/// [`pass_at`](Self::pass_at), so the tally is of what the map *is* — the
	/// overrides included, exactly as the overlay paints it and the bake writes
	/// it. A cell whose pack ships no pass table counts as nothing (it is the
	/// one case `pass_at` cannot answer).
	pub fn pass_counts(&self) -> ([u32; 4], u32) {
		let mut counts = [0u32; 4];
		let mut overrides = 0;
		for y in 0..self.height {
			for x in 0..self.width {
				if let Some(v) = self.pass_at(x, y)
					&& let Some(slot) = counts.get_mut(v as usize)
				{
					*slot += 1;
				}
				if self.pass_override(x, y).is_some() {
					overrides += 1;
				}
			}
		}
		(counts, overrides)
	}

	/// Whether a cell carries an explicit pass override.
	pub fn pass_override(&self, x: u16, y: u16) -> Option<u8> {
		if x >= self.width || y >= self.height {
			return None;
		}
		self.pass_overrides[y as usize * self.width as usize + x as usize]
	}

	/// FNV-1a over the cell grid (document identity for scripts/asserts).
	pub fn hash(&self) -> u64 {
		let mut h = 0xcbf2_9ce4_8422_2325u64;
		let mut eat = |bytes: &[u8]| {
			for &b in bytes {
				h ^= b as u64;
				h = h.wrapping_mul(0x0000_0100_0000_01b3);
			}
		};
		eat(&self.width.to_le_bytes());
		eat(&self.height.to_le_bytes());
		eat(&self.palette); // the map's colors are document state
		for stack in &self.cells {
			for layer in stack {
				match layer {
					None => eat(&[0xff]),
					Some(t) => {
						eat(&[t.pack]);
						eat(&t.tile.to_le_bytes());
						eat(&[t.transform.bits() as u8]);
					}
				}
			}
		}
		// Pass overrides are document state.
		for v in &self.pass_overrides {
			eat(&[v.map(|p| p + 1).unwrap_or(0)]);
		}
		// Scenery is document state the *bake* writes, so it belongs to the map's
		// identity - two maps that export differently must not hash the same. An
		// empty list contributes nothing, so every scenery-free map keeps the hash
		// it had before scenery existed (the script goldens depend on that).
		for spot in &self.scenery {
			eat(spot.pack.as_bytes());
			eat(spot.piece.as_bytes());
			eat(&spot.x.to_le_bytes());
			eat(&spot.y.to_le_bytes());
		}
		h
	}
}

/// Apply a transform to a 64×64 tile (used by tests and the bake; the GPU
/// shader mirrors this addressing).
pub fn transform_tile(src: &[u8], transform: Transform) -> [u8; TILE_DATA_SIZE] {
	let mut out = [0u8; TILE_DATA_SIZE];
	transform_into(&mut out, src, transform, None);
	out
}

/// Write `src` into `dst` with `transform` applied; pixels equal to `mask`
/// (the family's transparency color, when it has one) keep the existing `dst`
/// value (layer fall-through). `None` = fully opaque.
fn transform_into(dst: &mut [u8; TILE_DATA_SIZE], src: &[u8], transform: Transform, mask: Option<u8>) {
	let n = TILE_SIZE; // 64
	for dy in 0..n {
		for dx in 0..n {
			// Map destination coords back to source coords (inverse of
			// mirror-then-rotate-cw).
			let (mut sx, mut sy) = (dx, dy);
			// Undo rotation: rotate counter-clockwise `rot` times.
			for _ in 0..transform.rot {
				let (rx, ry) = (sy, n - 1 - sx);
				sx = rx;
				sy = ry;
			}
			// Undo mirror (horizontal flip is its own inverse).
			if transform.mirror {
				sx = n - 1 - sx;
			}
			let pixel = src[sy * n + sx];
			if mask != Some(pixel) {
				dst[dy * n + dx] = pixel;
			}
		}
	}
}
#[cfg(test)]
mod tests;
