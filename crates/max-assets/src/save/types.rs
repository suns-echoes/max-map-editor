//! Types for M.A.X. saved-game (`.DTA` and stock-mission) files.
//!
//! Field names and layout follow the M.A.X. Port reference implementation
//! (`saveload.cpp`, `smartfile.cpp`). See `SAVE-FORMAT.md` for the byte-level
//! spec this mirrors.

use super::error::EditError;

/// Number of team slots in a save: red, green, blue, gray, alien.
pub const TEAM_COUNT: usize = 5;

/// Display labels for the five team slots, in `team_*` array order.
pub const TEAM_LABELS: [&str; TEAM_COUNT] = ["Red", "Green", "Blue", "Gray", "Alien"];

/// On-disk save format version. The original DOS game and its shipped stock
/// missions are `V70`; M.A.X. Port writes `V71`. Both are flat little-endian
/// struct dumps, differing mainly in string encoding (fixed `char[30]` vs
/// length-prefixed) and index/count width (`u16` vs `u32`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveFormat {
	V70,
	V71,
}

impl SaveFormat {
	pub fn version(self) -> u16 {
		match self {
			SaveFormat::V70 => 70,
			SaveFormat::V71 => 71,
		}
	}
}

/// What kind of save/mission this is. In `V70` it is derived from a one-byte
/// game-type field (`SaveLoad_TranslateSaveFileCategory`); in `V71` it is
/// stored directly as a `MissionCategory` index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveCategory {
	Custom,
	Training,
	Campaign,
	HotSeat,
	Multi,
	Demo,
	Scenario,
	MultiPlayerScenario,
}

impl SaveCategory {
	/// Maps the `V70` game-type byte. Types `6` (debug) and `7` (text) are not
	/// valid save categories and yield `None`.
	pub fn from_game_type(byte: u8) -> Option<Self> {
		Some(match byte {
			0 => SaveCategory::Custom,
			1 => SaveCategory::Training,
			2 => SaveCategory::Campaign,
			3 => SaveCategory::HotSeat,
			4 => SaveCategory::Multi,
			5 => SaveCategory::Demo,
			8 => SaveCategory::Scenario,
			9 => SaveCategory::MultiPlayerScenario,
			_ => return None,
		})
	}

	/// Maps a `V71` `MissionCategory` index (`0..=7`).
	pub fn from_mission_category(index: u32) -> Option<Self> {
		Some(match index {
			0 => SaveCategory::Custom,
			1 => SaveCategory::Training,
			2 => SaveCategory::Campaign,
			3 => SaveCategory::HotSeat,
			4 => SaveCategory::Multi,
			5 => SaveCategory::Demo,
			6 => SaveCategory::Scenario,
			7 => SaveCategory::MultiPlayerScenario,
			_ => return None,
		})
	}

	/// The `V71` `MissionCategory` index this category writes to disk — the inverse
	/// of [`Self::from_mission_category`] (region 2 of the save header).
	pub fn mission_category_index(self) -> u32 {
		match self {
			SaveCategory::Custom => 0,
			SaveCategory::Training => 1,
			SaveCategory::Campaign => 2,
			SaveCategory::HotSeat => 3,
			SaveCategory::Multi => 4,
			SaveCategory::Demo => 5,
			SaveCategory::Scenario => 6,
			SaveCategory::MultiPlayerScenario => 7,
		}
	}

	pub fn label(self) -> &'static str {
		match self {
			SaveCategory::Custom => "custom",
			SaveCategory::Training => "training",
			SaveCategory::Campaign => "campaign",
			SaveCategory::HotSeat => "hot-seat",
			SaveCategory::Multi => "multiplayer",
			SaveCategory::Demo => "demo",
			SaveCategory::Scenario => "scenario",
			SaveCategory::MultiPlayerScenario => "mp-scenario",
		}
	}
}

/// Game-rule options block (`SaveLoad_LoadOptions`): twelve `i32`s written
/// after the header in both formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveOptions {
	pub world: i32,
	pub timer: i32,
	pub endturn: i32,
	pub start_gold: i32,
	pub play_mode: i32,
	pub victory_type: i32,
	pub victory_limit: i32,
	pub opponent: i32,
	pub raw_resource: i32,
	pub fuel_resource: i32,
	pub gold_resource: i32,
	pub alien_derelicts: i32,
}

/// The seven "extra settings" `i32`s (`saveload.cpp` region 13) — UI/interaction
/// preferences the engine persists alongside the game options. In `V71` they are
/// written contiguously after the twelve game options (all nineteen in one block);
/// in `V70` they trail the game scalars instead. Typed so the header/settings
/// region re-emits from the model rather than from retained bytes (Stage A).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SaveExtraSettings {
	pub effects: i32,
	pub click_scroll: i32,
	pub quick_scroll: i32,
	pub fast_movement: i32,
	pub follow_unit: i32,
	pub auto_select: i32,
	pub enemy_halt: i32,
}

/// The parsed save header + options — everything up to (but not including) the
/// surface/cargo maps and unit lists. Enough to identify a save and resolve the
/// world (`.WRL`) it references.
#[derive(Debug, Clone)]
pub struct SaveHeader {
	pub format: SaveFormat,
	pub category: SaveCategory,
	pub save_name: String,
	/// Stock world index `0..=23` (`SNOW_1 + index`). `Some` for `V70` always,
	/// and for `V71` when the stored world hash matches a stock world.
	pub world_index: Option<u8>,
	/// Stock world file this save references (e.g. `"SNOW_1.WRL"`), when known.
	pub world_file: Option<&'static str>,
	/// The `V71` world content hash (SHA-256 hex), if present.
	pub world_hash: Option<String>,
	/// `V70` stock-mission index (1-based on disk, stored raw here); `0` for `V71`.
	pub mission_index: u16,
	/// The embedded mission binary script (`V71` region 3), retained verbatim so it
	/// re-emits byte-exactly. Empty for `V70` and for a scriptless `V71` save.
	pub script: Vec<u8>,
	pub team_names: [String; TEAM_COUNT],
	pub team_type: [u32; TEAM_COUNT],
	pub team_clan: [u32; TEAM_COUNT],
	pub rng_seed: u32,
	pub options: SaveOptions,
}

/// Number of physical unit types (`UNIT_END` in `resourcetable.hpp`). Sizes the
/// per-team `base_values`/`current_values` stat tables.
pub const UNIT_END: usize = 93;

/// Per-unit base stats (`UnitValues::FileLoad`, `unitvalues.cpp`). A 28-byte
/// record referenced through the save's object graph — units share these via
/// back-references, and the per-team tables hold the master copies. Note the
/// engine's runtime `fuel` field is never serialized, so it is absent here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitValues {
	pub turns: u16,
	/// Maximum hit points (the `hits` cap; a unit's *current* HP lives on
	/// [`UnitRecord::hits`]).
	pub hits: u16,
	pub armor: u16,
	pub attack: u16,
	pub speed: u16,
	pub range: u16,
	pub rounds: u16,
	pub move_and_fire: u8,
	pub scan: u16,
	/// Maximum cargo/storage capacity.
	pub storage: u16,
	/// Maximum ammunition.
	pub ammo: u16,
	pub attack_radius: u16,
	pub agent_adjust: u16,
	pub version: u16,
	pub in_use: bool,
}

/// A power/mining complex shared by a team's connected buildings
/// (`Complex::FileLoad`, `complex.cpp`). Seven `i16`s, 14 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Complex {
	pub material: i16,
	pub fuel: i16,
	pub gold: i16,
	pub power: i16,
	pub workers: i16,
	pub buildings: i16,
	pub id: i16,
}

/// The straight-line flight path of an air unit (`AirPath::FileLoad`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AirPath {
	pub length: i16,
	pub angle: i8,
	pub start_x: i16,
	pub start_y: i16,
	pub end_x: i16,
	pub end_y: i16,
	pub step_x: i32,
	pub step_y: i32,
	pub delta_x: i32,
	pub delta_y: i32,
}

/// A unit's movement path — one of the three registered `UnitPath` subclasses
/// (`airpath.cpp`, `builderpath.cpp`, `groundpath.cpp`). Preserved so the object
/// graph round-trips; the editor does not interpret path contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitPath {
	Air(AirPath),
	Builder { direction_x: i16, direction_y: i16 },
	Ground { end_x: i16, end_y: i16, step_index: u32, steps: Vec<(i8, i8)> },
}

/// One deserialized unit record (`UnitInfo::FileLoad`, `unitinfo.cpp`). The
/// editor surfaces the gameplay-relevant fields; the many display/animation
/// fields the engine also stores are consumed for byte-alignment but dropped.
/// Object-reference fields (`path`, `base_values`, …) are indices into
/// [`SaveFile::objects`] (`None` = a null reference on disk).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitRecord {
	/// `ResourceID` (`resourcetable.hpp`) — e.g. `0x33` = TANK, `0x11` = LRGSLAB.
	pub unit_type: u16,
	/// Spatial-hash id (unique per unit within the save).
	pub id: u16,
	/// Type/owner flag bits (`enums.hpp`): ground-cover, building, mobile-*,
	/// `HASH_TEAM_*` owner, etc.
	pub flags: u32,
	/// Pixel position (64 px per cell).
	pub pixel_x: u16,
	pub pixel_y: u16,
	/// Cell position on the same grid as `Project.width/height`.
	pub grid_x: i16,
	pub grid_y: i16,
	pub name: String,
	pub team: u8,
	pub angle: u8,
	/// Turret heading (0..7), independent of the body `angle` (a turret can be
	/// rotated to track a target while the chassis faces elsewhere). On deploy
	/// the engine sets it equal to `angle`.
	pub turret_angle: u8,
	pub orders: u8,
	pub state: u8,
	pub prior_orders: u8,
	pub prior_state: u8,
	/// Turns this unit stays disabled (`disabled_turns_remaining`, `unitinfo.hpp`).
	/// Meaningful only while `orders == ORDER_DISABLE`; in a `V70` save it shares
	/// the recoil byte with `firing_recoil_frames` (disambiguated by the order).
	pub disabled_turns: u8,
	/// Current hit points (`u8` on disk in V70; `u16` in V71).
	pub hits: u16,
	pub ammo: u8,
	/// Cargo carried / experience (context-dependent per unit type).
	pub storage: i16,
	pub build_rate: u16,
	/// Connector adjacency bitmask (slabs/roads/connectors).
	pub connectors: u16,
	/// First frame of this unit's 8-frame turret strip (`image_base + turret_angle`
	/// selects the drawn turret sprite). Retained for the map-overlay turret render.
	pub turret_image_base: i16,
	/// First frame of this unit's 8-frame connector-strut strip; one strut sprite
	/// per set `connectors` bit is drawn at `connector_image_base + side_offset`.
	pub connector_image_base: i16,
	pub path: Option<usize>,
	pub base_values: Option<usize>,
	pub complex: Option<usize>,
	pub parent_unit: Option<usize>,
	pub enemy_unit: Option<usize>,
	/// Queued build order (a factory's production list), as `ResourceID`s.
	pub build_list: Vec<u16>,
}

/// A node in the save's shared object graph (`SmartFileReader` registry). Every
/// object materialized via `ReadObject` lands in [`SaveFile::objects`] in
/// first-seen order; 1-based on-disk indices map to `objects[index - 1]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveObject {
	/// A slot reserved mid-recursion (for cyclic/forward references); never
	/// remains after a successful decode.
	Reserved,
	Unit(UnitRecord),
	Values(UnitValues),
	Complex(Complex),
	Path(UnitPath),
}

/// Per-team session state (`CTInfo`, `ctinfo.hpp`), 988 bytes in `V71`. Every
/// field the `V71` writer emits is typed here so the block re-emits from the model
/// (`crate::save::encode::encode_ct_info`); the array fields carry `V71`-native
/// widths. A `V70` block has a different, narrower layout (leading `markers[10]`,
/// `u8` counters, `u16` casualties, `ScreenLocation[6]`); the `V70` decoder fills
/// only the shared scalar fields and leaves the `V71`-shaped arrays zeroed, since
/// `V70` is never re-encoded — see [`SaveFile::raw`] / `SAVE-FROM-SCRATCH.md` §11.
#[derive(Debug, Clone)]
pub struct CtInfo {
	pub team_type: u8,
	pub finished_turn: bool,
	pub team_clan: u8,
	/// Per-team research state — `ResearchTopic[8]`, each three `i32`.
	pub research_topics: [[i32; 3]; 8],
	pub team_points: u32,
	pub number_of_objects_created: u16,
	/// Count of each physical unit type this team has built (`ResourceID`-indexed).
	pub unit_counters: [u32; UNIT_END],
	/// Saved minimap/screen bookmark positions — `ScreenLocation[4]`, each `{x, y}`.
	pub screen_locations: [[i8; 2]; 4],
	/// Per-turn team-points history for the score graph.
	pub score_graph: [i16; 50],
	/// The team's currently-selected unit id, or `0xFFFF` when none.
	pub selected_unit_id: u16,
	pub zoom_level: u16,
	pub camera_x: i16,
	pub camera_y: i16,
	/// The eleven map display-toggle button states (range, scan, status, colors,
	/// hits, ammo, names, minimap-2x, minimap-tnt, grid, survey), in write order.
	pub display_buttons: [i8; 11],
	/// Build-tally stats: factories, mines, buildings, units built.
	pub stats: [i16; 4],
	/// Units lost per physical type (`ResourceID`-indexed).
	pub casualties: [u32; UNIT_END],
	pub stats_gold_spent_on_upgrades: u32,
}

impl CtInfo {
	/// A `CTInfo` with the given shared scalar fields and every `V71`-shaped array
	/// zeroed — the `V70` decode path, which surfaces only the scalars (the arrays
	/// differ in width and `V70` is never re-encoded). The object-created counter
	/// is a shared `u16` in both formats and feeds the unit-id allocator
	/// (`crate::save::export`).
	pub(crate) fn v70(
		team_type: u8,
		finished_turn: bool,
		team_clan: u8,
		team_points: u32,
		number_of_objects_created: u16,
		zoom_level: u16,
		camera: (i16, i16),
	) -> Self {
		CtInfo {
			team_type,
			finished_turn,
			team_clan,
			research_topics: [[0; 3]; 8],
			team_points,
			number_of_objects_created,
			unit_counters: [0; UNIT_END],
			screen_locations: [[0; 2]; 4],
			score_graph: [0; 50],
			selected_unit_id: 0,
			zoom_level,
			camera_x: camera.0,
			camera_y: camera.1,
			display_buttons: [0; 11],
			stats: [0; 4],
			casualties: [0; UNIT_END],
			stats_gold_spent_on_upgrades: 0,
		}
	}
}

/// A team's unit stat/upgrade tables (`TeamUnits::FileLoad`, `teamunits.cpp`),
/// read just before the five unit lists. The value entries are object indices
/// into [`SaveFile::objects`] (pointing at [`SaveObject::Values`]).
#[derive(Debug, Clone)]
pub struct TeamUnitsTable {
	pub gold: u32,
	/// Base (un-upgraded) stats, one per `ResourceID` (`UNIT_END` entries).
	pub base_values: Vec<Option<usize>>,
	/// Current (upgraded) stats, one per `ResourceID`.
	pub current_values: Vec<Option<usize>>,
	/// This team's power/mining complexes.
	pub complexes: Vec<usize>,
}

/// Per-object data needed to re-emit the shared object graph byte-exactly
/// (`crate::save::serialize`). The decoder skips many display/animation fields,
/// so a field-by-field re-serialize is impossible; instead each object's body is
/// retained verbatim. `contained` is the size of this object's inline subtree —
/// itself plus any objects the reader materialized *inside* its body (cyclic /
/// forward references are always written inline) — and `body_raw` already holds
/// those nested bytes. Because the reader consumes the file strictly in order,
/// first-write order equals first-read order, so emitting `body_raw` verbatim and
/// advancing the write cursor by `contained` reproduces every object index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjMeta {
	/// Registry class index (`1 AirPath … 6 UnitValues`).
	pub type_index: u32,
	/// Objects in this object's inline subtree (>= 1, includes itself).
	pub contained: usize,
	/// The object's body bytes exactly as read (nested inline objects included).
	pub body_raw: Vec<u8>,
	/// For `UnitInfo` bodies (`type_index == 5`): the byte offsets of the editable
	/// scalar fields *inside* `body_raw`, captured during decode so an export can
	/// overwrite individual fields without re-walking the layout. `None` for every
	/// other object class. See [`UnitBodyLayout`] and `crate::save::export`.
	pub unit_layout: Option<UnitBodyLayout>,
}

/// Byte offsets of a decoded `UnitInfo`'s editable scalar fields, relative to the
/// start of its [`ObjMeta::body_raw`] (i.e. just after the object's type index).
///
/// Captured during decode (the single source of truth for the field walk) so the
/// save exporter (`crate::save::export`) can patch individual fields in place —
/// robust against the `V70`/`V71` layout differences and the variable-width object
/// references that precede `connectors`/`base_values`, which are not derivable
/// from the name length alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitBodyLayout {
	/// Offset of the `u16` name-length prefix; the name bytes follow immediately.
	pub name: usize,
	/// The current name's byte length (what the `name` prefix holds).
	pub name_len: usize,
	/// Offset of the `u16` `pixel_x` (render position; `pixel_y` follows at `+2`).
	pub pixel_x: usize,
	/// Offset of the `i16` `grid_x` (cell position; `grid_y` follows at `+2`).
	pub grid_x: usize,
	pub team: usize,
	pub angle: usize,
	pub turret_angle: usize,
	/// Offset of the 8×`i16` image block (`total_images`, `image_base`,
	/// `turret_image_base`, `firing_image_base`, `connector_image_base`,
	/// `image_index`, `turret_image_index`, `image_index_max`) — the display frame
	/// state the integrity pass re-derives so a placed unit shows its idle frame
	/// (`image_base + angle`) instead of a template's inherited animation frame
	/// (`save-editor-bug.md`). Address a field as `image_block + 2*field_index`.
	pub image_block: usize,
	pub orders: usize,
	/// Offset of `hits`; its on-disk width is [`Self::hits_width`] (1 in `V70`,
	/// 2 in `V71`).
	pub hits: usize,
	pub hits_width: usize,
	/// Offset of the grid-target block: `move_to_grid_x`/`move_to_grid_y` (`2×i16`),
	/// followed in `V71` by `fire_on_grid_x`/`fire_on_grid_y`. The integrity pass
	/// resets these to the unit's own cell so a placed unit isn't left "heading to"
	/// the template's cell (`save-editor-bug.md`).
	pub move_to: usize,
	/// Offset of the `build_time` countdown byte (reset to 0 for an idle unit).
	pub build_time: usize,
	/// Offset of the `moved` byte (pixels moved this turn; reset to 0 on placement).
	pub moved: usize,
	pub ammo: usize,
	/// Offset of the `disabled_turns_remaining` byte. In a `V70` save this is the
	/// single recoil byte (see [`Self::disabled_dual`]); in `V71` it's the dedicated
	/// second byte of the two-byte recoil block.
	pub disabled: usize,
	/// `true` in a `V70` save, where the [`Self::disabled`] byte doubles as
	/// `firing_recoil_frames` — so the exporter overwrites it only when the unit's
	/// order is `ORDER_DISABLE` (else the firing-recoil value is left verbatim).
	pub disabled_dual: bool,
	/// Offset of the `i16` `storage` field.
	pub storage: usize,
	/// Offset of the `u16` `connectors` adjacency mask (sits after the `path`
	/// reference, so its position is not fixed relative to the name).
	pub connectors: usize,
	/// Offset of the `base_values` object reference (right after `connectors`).
	/// Retained for the per-unit stat-override export (S6.2).
	pub base_values_ref: usize,
	/// Offset where the object-reference section begins — i.e. the end of the
	/// verbatim "opaque prefix" (display + editable scalar fields, `140 + name_len`
	/// bytes in `V70`), just before the `path` reference. The serializer emits
	/// `body_raw[..refs_off]` verbatim, then re-emits the refs (`path`, `connectors`,
	/// `base_values`, `complex`, `parent`, `enemy`, `build_list`) symbolically from
	/// the [`UnitRecord`] so inline objects recurse and every index is recomputed.
	pub refs_off: usize,
}

impl UnitBodyLayout {
	/// Re-base every offset that follows the (length-prefixed) name by `delta` and
	/// record the new name length — after a name splice resizes `body_raw`. `name`,
	/// `pixel_x`, and `grid_x` precede the name and are unchanged.
	pub fn shift_after_name(&mut self, delta: isize, new_name_len: usize) {
		let adj = |o: &mut usize| *o = (*o as isize + delta) as usize;
		adj(&mut self.team);
		adj(&mut self.angle);
		adj(&mut self.turret_angle);
		adj(&mut self.image_block);
		adj(&mut self.orders);
		adj(&mut self.move_to);
		adj(&mut self.build_time);
		adj(&mut self.moved);
		adj(&mut self.hits);
		adj(&mut self.ammo);
		adj(&mut self.disabled);
		adj(&mut self.storage);
		adj(&mut self.connectors);
		adj(&mut self.base_values_ref);
		adj(&mut self.refs_off);
		self.name_len = new_name_len;
	}
}

/// One occupied cell in the map spatial hash (`MapHashObject`, `hash.cpp`): its
/// grid position and the units standing on it, in on-disk (game-history) order.
/// A building occupies four cells (its top-left plus the three toward `+x/+y`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapCell {
	pub x: u16,
	pub y: u16,
	/// Object indices of the units on this cell — back-references into
	/// [`SaveFile::objects`] (the map hash adds no new objects).
	pub units: Vec<usize>,
}

/// The map spatial hash (`Hash_MapHash`, `hash.cpp`): `hash_size` buckets keyed by
/// `(grid_y ^ (grid_x << x_shift)) % hash_size`, each a list of occupied cells.
///
/// The engine **trusts** this on load (it never rebuilds it — `saveload.cpp`), and
/// every spatial query reads it, so an export must reproduce it. It is decoded
/// **structurally** (not kept opaque like the rest of the tail) so a unit move /
/// add / remove can re-derive the affected buckets while leaving the rest
/// byte-identical (S6.2). Sits immediately after [`SaveFile::unit_hash`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MapHash {
	/// Bucket count (always `HASH_HASH_SIZE` = 512 in stock saves).
	pub hash_size: u16,
	/// Position-mix shift (2 for a 512-bucket hash).
	pub x_shift: u16,
	/// `hash_size` buckets, each the occupied cells hashing to it, in on-disk order.
	pub buckets: Vec<Vec<MapCell>>,
}

/// `ResourceID`s of the two "tape" ground-cover markers, which the engine inserts
/// at a cell's front rather than back (`MapHash::AddEx`, `hash.cpp`).
const LRGTAPE: u16 = 0x0F;
const SMLTAPE: u16 = 0x10;

impl MapHash {
	/// The bucket a grid cell hashes to — the engine's
	/// `(grid_y ^ (grid_x << x_shift)) % hash_size` (`hash.cpp`).
	fn bucket_of(&self, x: u16, y: u16) -> usize {
		(y ^ (x << self.x_shift)) as usize % self.hash_size as usize
	}

	/// Remove object `idx` from every cell that lists it, dropping cells left empty.
	/// Scans all buckets rather than the unit's computed cells: a mid-move mobile
	/// unit can be hashed one cell off its stored grid, and a re-key must not strand
	/// such a stale entry.
	pub fn remove_unit(&mut self, idx: usize) {
		for bucket in &mut self.buckets {
			for cell in bucket.iter_mut() {
				cell.units.retain(|&u| u != idx);
			}
			bucket.retain(|cell| !cell.units.is_empty());
		}
	}

	/// Insert `idx` into grid cell `(x, y)`, mirroring `MapHash::AddEx`: reuse the
	/// cell's `MapHashObject` if the bucket already has one, else create it at the
	/// bucket's front; then insert the unit at the cell's front or back.
	fn add_to_cell(&mut self, idx: usize, x: u16, y: u16, push_front: bool) {
		let b = self.bucket_of(x, y);
		let bucket = &mut self.buckets[b];
		let pos = bucket.iter().position(|c| c.x == x && c.y == y).unwrap_or_else(|| {
			bucket.insert(0, MapCell { x, y, units: Vec::new() });
			0
		});
		if push_front {
			bucket[pos].units.insert(0, idx);
		} else {
			bucket[pos].units.push(idx);
		}
	}

	/// Add object `idx` at grid `(x, y)` to every cell its footprint covers, per
	/// `MapHash::Add`: the top-left cell plus three toward `+x`/`+y` iff `building`.
	/// Units insert at the cell front unless they are non-tape ground cover, which
	/// insert at the back (`AddEx`'s `mode` = `flags & GROUND_COVER`).
	pub fn add_unit(&mut self, idx: usize, x: u16, y: u16, unit_type: u16, building: bool, ground_cover: bool) {
		let push_front = !ground_cover || unit_type == LRGTAPE || unit_type == SMLTAPE;
		self.add_to_cell(idx, x, y, push_front);
		if building {
			self.add_to_cell(idx, x + 1, y, push_front);
			self.add_to_cell(idx, x, y + 1, push_front);
			self.add_to_cell(idx, x + 1, y + 1, push_front);
		}
	}
}

/// Verbatim byte regions retained for byte-exact re-serialization of the parts
/// the decoder does not fully model — the header + options, the per-team `CTInfo`
/// blocks, the extra-settings and game-scalar (+ `V71` cheater) blocks, and the
/// opaque post-`Hash_MapHash` tail (heat maps, message logs, AI). With these plus
/// the typed model, [`crate::save::serialize::write_save`] reproduces the original
/// file byte-for-byte.
#[derive(Debug, Clone, Default)]
pub struct RawRegions {
	/// File start through the options block (header + pre-options + options).
	pub header: Vec<u8>,
	/// The seven "extra settings" `i32`s (28 B) — written up front in `V71`,
	/// after the game scalars in `V70`.
	pub extra_settings: Vec<u8>,
	/// One retained `CTInfo` block per team (four in `V70`, five in `V71`).
	pub ct_info: Vec<Vec<u8>>,
	/// The game-scalar block (active/player/turn/game_state/turn_timer), plus the
	/// `is_cheater`/`cheater_team` pair in `V71`.
	pub scalars: Vec<u8>,
	/// Everything after `Hash_MapHash`: heat maps, message logs, and AI state —
	/// retained opaque. (`Hash_MapHash` itself is now decoded structurally into
	/// [`SaveFile::map_hash`].) ⚠ Message-log and AI entries carry object-graph
	/// back-references, so this stays byte-valid only while object indices are
	/// unchanged; a graph-structural edit (add/remove/stat-override) must re-emit
	/// these with remapped indices (S6.2, not yet done).
	pub tail: Vec<u8>,
}

/// A fully decoded M.A.X. save (`V70` or `V71`): header, surface + resource
/// maps, per-team state, and the five unit lists resolved against the shared
/// object graph.
///
/// The five list fields hold object indices into [`SaveFile::objects`] (each an
/// [`SaveObject::Unit`]), in the exact on-disk order.
#[derive(Debug, Clone)]
pub struct SaveFile {
	pub header: SaveHeader,
	/// The seven "extra settings" (`saveload.cpp` region 13), decoded typed. Kept on
	/// the body rather than [`SaveHeader`] because in `V70` they sit past the header
	/// (after the game scalars), so the header-only reader never sees them.
	pub extra_settings: SaveExtraSettings,
	pub width: u16,
	pub height: u16,
	/// Terrain passability, row-major `y * width + x` (`SURFACE_TYPE_*`: 0 none,
	/// 1 land, 2 water, 4 coast, 8 air).
	pub surface_map: Vec<u8>,
	/// Surveyed resource distribution per cell (raw/fuel/gold, `u16`), row-major.
	pub cargo_map: Vec<u16>,
	pub active_turn_team: u8,
	pub player_team: u8,
	pub turn_counter: i32,
	pub game_state: u16,
	pub turn_timer: u16,
	/// `V71` sticky cheater flag and the team that triggered it (region 18). Both
	/// `0` in `V70` (no such region on disk) and in a fresh human-only game.
	pub is_cheater: u32,
	pub cheater_team: u32,
	/// Per-team CTInfo: four teams in `V70` (Red, Green, Blue, Gray); five in
	/// `V71` (adding the alien slot).
	pub teams: Vec<CtInfo>,
	pub team_units: Vec<TeamUnitsTable>,
	/// The shared object graph, in first-seen order.
	pub objects: Vec<SaveObject>,
	pub ground_cover: Vec<usize>,
	pub mobile_land_sea: Vec<usize>,
	pub stationary: Vec<usize>,
	pub mobile_air: Vec<usize>,
	pub particles: Vec<usize>,
	/// The trailing `Hash_UnitHash` (`hash.cpp`): `HASH_HASH_SIZE`(512) buckets,
	/// each an in-file-order list of object indices for the units whose
	/// `id % 512` selects that bucket. Every entry back-references a unit already
	/// in [`SaveFile::objects`] (the hash adds no new objects). The intra-bucket
	/// order is game-history order, **not** derivable from ids — retained here so
	/// a future byte-exact re-serialize can reproduce it.
	pub unit_hash: Vec<Vec<usize>>,
	/// The map spatial hash (`Hash_MapHash`), immediately after [`Self::unit_hash`],
	/// decoded structurally so unit moves/add/remove can re-derive it (S6.2). See
	/// [`MapHash`].
	pub map_hash: MapHash,
	/// Per-object re-serialization data, parallel to [`SaveFile::objects`] — the
	/// verbatim body bytes and subtree size that let [`crate::save::serialize`]
	/// rebuild the object graph byte-exactly. See [`ObjMeta`].
	pub object_meta: Vec<ObjMeta>,
	/// Verbatim byte regions for the parts not fully modeled. See [`RawRegions`].
	pub raw: RawRegions,
}

impl SaveFile {
	/// Resolves an object index to a [`UnitRecord`], if that slot holds a unit.
	pub fn unit(&self, index: usize) -> Option<&UnitRecord> {
		match self.objects.get(index) {
			Some(SaveObject::Unit(u)) => Some(u),
			_ => None,
		}
	}

	/// Resolves an object index to a [`UnitValues`], if that slot holds one.
	pub fn values(&self, index: usize) -> Option<&UnitValues> {
		match self.objects.get(index) {
			Some(SaveObject::Values(v)) => Some(v),
			_ => None,
		}
	}

	/// Insert `obj` (with re-serialization `meta`) at object index `at`, shifting
	/// every existing object at `>= at` up by one and remapping every stored index
	/// so the graph stays internally consistent — unit reference fields, the five
	/// unit lists, both spatial hashes, the team stat tables **and the retained
	/// tail**, whose message-log and AI entries reference the graph too. The
	/// inserted object's index is `at`. The serializer recomputes on-disk indices
	/// from the (now-updated) first-seen order, so `at` must be that object's
	/// first-seen position. Basis for a per-unit `UnitValues` override and unit
	/// insertion (S6.2).
	///
	/// `Err` only if the tail will not decompose ([`Self::tail_follows_the_graph`],
	/// which a caller should check up front) — and then **nothing** is written.
	pub fn insert_object(&mut self, at: usize, obj: SaveObject, meta: ObjMeta) -> Result<(), EditError> {
		// Measured against the graph as it stands, so it must come first.
		self.raw.tail = super::tail::follow_shift(self, &super::tail::Shift::Inserted(at))?;
		self.objects.insert(at, obj);
		self.object_meta.insert(at, meta);
		self.remap_indices(&|i| if i >= at { i + 1 } else { i });
		Ok(())
	}

	/// Remove object `at` and shift every reference above it down by one. Any
	/// reference *to* `at` is set to null (`None`) / dropped from a list — callers
	/// must ensure nothing still logically needs it. The retained tail follows:
	/// a message-log line loses its unit, a spotted unit is dropped outright.
	/// Basis for unit deletion (S6.2).
	///
	/// `Err` only if the tail will not decompose ([`Self::tail_follows_the_graph`],
	/// which a caller should check up front) — and then **nothing** is written.
	pub fn remove_object(&mut self, at: usize) -> Result<(), EditError> {
		// Measured against the graph as it stands, so it must come first.
		self.raw.tail = super::tail::follow_shift(self, &super::tail::Shift::Removed(at))?;
		self.objects.remove(at);
		self.object_meta.remove(at);
		// References to `at` itself become dangling; map them to a sentinel that the
		// list/ref cleanup below drops. Shift everything above `at` down by one.
		self.retain_refs(at);
		self.remap_indices(&|i| if i > at { i - 1 } else { i });
		Ok(())
	}

	/// Drop every reference that points *at* object `at`: null it in unit ref fields
	/// and team tables, and remove it from the lists / hashes. (Used by
	/// [`Self::remove_object`] before the downward index shift.)
	fn retain_refs(&mut self, at: usize) {
		let clear = |r: &mut Option<usize>| {
			if *r == Some(at) {
				*r = None;
			}
		};
		for o in &mut self.objects {
			if let SaveObject::Unit(u) = o {
				clear(&mut u.path);
				clear(&mut u.base_values);
				clear(&mut u.complex);
				clear(&mut u.parent_unit);
				clear(&mut u.enemy_unit);
			}
		}
		for list in [
			&mut self.ground_cover,
			&mut self.mobile_land_sea,
			&mut self.stationary,
			&mut self.mobile_air,
			&mut self.particles,
		] {
			list.retain(|&i| i != at);
		}
		for bucket in &mut self.unit_hash {
			bucket.retain(|&i| i != at);
		}
		for bucket in &mut self.map_hash.buckets {
			for cell in bucket.iter_mut() {
				cell.units.retain(|&i| i != at);
			}
			bucket.retain(|cell| !cell.units.is_empty());
		}
		for t in &mut self.team_units {
			for r in &mut t.base_values {
				clear(r);
			}
			for r in &mut t.current_values {
				clear(r);
			}
			t.complexes.retain(|&i| i != at);
		}
	}

	/// Apply `f` to every object index stored anywhere in the graph — reference
	/// fields on units, the five unit lists, both spatial hashes, and the team stat
	/// tables. The shared index-shift helper for insertion and removal.
	fn remap_indices(&mut self, f: &dyn Fn(usize) -> usize) {
		let opt = |r: &mut Option<usize>| {
			if let Some(i) = r {
				*i = f(*i);
			}
		};
		for o in &mut self.objects {
			if let SaveObject::Unit(u) = o {
				opt(&mut u.path);
				opt(&mut u.base_values);
				opt(&mut u.complex);
				opt(&mut u.parent_unit);
				opt(&mut u.enemy_unit);
			}
		}
		for list in [
			&mut self.ground_cover,
			&mut self.mobile_land_sea,
			&mut self.stationary,
			&mut self.mobile_air,
			&mut self.particles,
		] {
			for i in list.iter_mut() {
				*i = f(*i);
			}
		}
		for bucket in &mut self.unit_hash {
			for i in bucket.iter_mut() {
				*i = f(*i);
			}
		}
		for bucket in &mut self.map_hash.buckets {
			for cell in bucket.iter_mut() {
				for i in cell.units.iter_mut() {
					*i = f(*i);
				}
			}
		}
		for t in &mut self.team_units {
			for r in &mut t.base_values {
				opt(r);
			}
			for r in &mut t.current_values {
				opt(r);
			}
			for i in t.complexes.iter_mut() {
				*i = f(*i);
			}
		}
	}

	/// The maximum HP of the unit with spatial-hash `id` — its `base_values`
	/// ([`UnitValues::hits`]), the cap the editor clamps a unit's *current* hits
	/// to (S4.5). `None` if no modeled unit carries that id or it has no stats
	/// block (e.g. some ground cover shares no per-unit values).
	pub fn unit_max_hits(&self, id: u16) -> Option<u16> {
		let rec = self.units().find(|u| u.id == id)?;
		self.values(rec.base_values?).map(|v| v.hits)
	}

	/// Every unit in the save, in object-graph order (all five lists combined).
	pub fn units(&self) -> impl Iterator<Item = &UnitRecord> {
		self.objects.iter().filter_map(|o| match o {
			SaveObject::Unit(u) => Some(u),
			_ => None,
		})
	}

	/// A mutable reference to the `i`-th unit list, in [`Self::lists`] order
	/// (0 ground-cover, 1 mobile-land-sea, 2 stationary, 3 mobile-air, 4 particles);
	/// out-of-range indices map to the particle list. Used to register a new unit.
	pub fn list_by_index_mut(&mut self, i: usize) -> &mut Vec<usize> {
		match i {
			0 => &mut self.ground_cover,
			1 => &mut self.mobile_land_sea,
			2 => &mut self.stationary,
			3 => &mut self.mobile_air,
			_ => &mut self.particles,
		}
	}

	/// The five unit lists with their names, in on-disk load order.
	pub fn lists(&self) -> [(&'static str, &[usize]); 5] {
		[
			("ground-cover", &self.ground_cover),
			("mobile-land-sea", &self.mobile_land_sea),
			("stationary", &self.stationary),
			("mobile-air", &self.mobile_air),
			("particles", &self.particles),
		]
	}
}

/// Stock world files in `world_index` order (`SNOW_1 + index`), per the M.A.X.
/// `ResourceID` enum: SNOW, CRATER, GREEN, DESERT.
///
/// NOTE: this deliberately differs from the editor's display-ordered
/// `max_assets::wrl::INSTALLED_MAP_FILE_NAMES` (SNOW, CRATER, DESERT, GREEN).
/// Do **not** use that array to resolve a save's `world_index` — it swaps the
/// GREEN and DESERT blocks (indices 12..=23).
pub const WORLD_FILE_NAMES: [&str; 24] = [
	"SNOW_1.WRL",
	"SNOW_2.WRL",
	"SNOW_3.WRL",
	"SNOW_4.WRL",
	"SNOW_5.WRL",
	"SNOW_6.WRL",
	"CRATER_1.WRL",
	"CRATER_2.WRL",
	"CRATER_3.WRL",
	"CRATER_4.WRL",
	"CRATER_5.WRL",
	"CRATER_6.WRL",
	"GREEN_1.WRL",
	"GREEN_2.WRL",
	"GREEN_3.WRL",
	"GREEN_4.WRL",
	"GREEN_5.WRL",
	"GREEN_6.WRL",
	"DESERT_1.WRL",
	"DESERT_2.WRL",
	"DESERT_3.WRL",
	"DESERT_4.WRL",
	"DESERT_5.WRL",
	"DESERT_6.WRL",
];

/// Resolves a stock `world_index` (`0..=23`) to its `.WRL` file name.
pub fn world_file_name(world_index: u8) -> Option<&'static str> {
	WORLD_FILE_NAMES.get(world_index as usize).copied()
}
