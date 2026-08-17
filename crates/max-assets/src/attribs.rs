//! max-port unit database — obfuscated JSON resources in `PATCHES.RES`.
//!
//! max-port ships its unit database as `SC_*` entries inside its own
//! `PATCHES.RES` archive: `SC_ATTRI` (12 base attributes per unit type),
//! `SC_CLANS` (per-clan attribute deltas + credits) and `SC_UNITS`
//! (flags / cargo-type metadata). Each entry is UTF-8 JSON XOR-obfuscated
//! with a fixed 32-byte table (`ResourceManager_GenericTable`,
//! max-port `resource_manager.cpp:150`; applied in `attributes.cpp:122`).
//!
//! This module holds the de-obfuscation primitive and the raw-entry reader;
//! the typed parsers build on it. Everything is read at **runtime** from the
//! user's max-port install — no game data is vendored into the repo.

use std::io;
use std::path::Path;

use crate::res::read_res_entry;
use crate::save::types::{UNIT_END, UnitValues};
use crate::save::{unit_type_id, unit_type_name};

/// `ResourceManager_GenericTable` — the XOR keystream for `SC_*` resources.
const GENERIC_TABLE: [u8; 32] = [
	0x6c, 0x57, 0x36, 0xe6, 0x81, 0xe0, 0x72, 0x8a, 0xd3, 0xd9, 0xff, 0x54, 0x48, 0x3a, 0xcd, 0x75, 0xd5, 0x0d, 0xe9,
	0xe7, 0x7a, 0x57, 0xae, 0xeb, 0x61, 0x16, 0xb0, 0x35, 0x55, 0x62, 0xed, 0xd1,
];

/// XOR-(de)obfuscates a `SC_*` resource in place (position mod 32 keystream).
/// The transform is an involution: applying it twice restores the input.
pub fn deobfuscate(data: &mut [u8]) {
	for (i, byte) in data.iter_mut().enumerate() {
		*byte ^= GENERIC_TABLE[i % GENERIC_TABLE.len()];
	}
}

/// Reads one obfuscated `SC_*` entry out of a `PATCHES.RES` archive and
/// returns its clear-text contents. `Ok(None)` = the archive has no such tag.
pub fn read_script_entry(patches_res: &Path, tag: &str) -> io::Result<Option<String>> {
	let Some(mut data) = read_res_entry(patches_res, tag)? else {
		return Ok(None);
	};
	deobfuscate(&mut data);
	String::from_utf8(data).map(Some).map_err(|e| {
		io::Error::new(io::ErrorKind::InvalidData, format!("{tag} did not de-obfuscate to UTF-8 text: {e}"))
	})
}

/// The 12 base attributes `SC_ATTRI` stores per unit type (max-port
/// `UnitAttributes`, `attributes.hpp` / the `from_json` mapping in
/// `attributes.cpp:37`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnitAttributes {
	pub turns_to_build: u32,
	pub hit_points: u32,
	pub armor_rating: u32,
	pub attack_rating: u32,
	pub move_and_fire: u32,
	pub movement_points: u32,
	pub attack_range: u32,
	pub shots_per_turn: u32,
	pub scan_range: u32,
	pub storage_capacity: u32,
	pub ammunition: u32,
	pub blast_radius: u32,
}

/// Reads one required non-negative integer attribute field.
fn attr_field(unit: &str, obj: &json::JsonValue, key: &str) -> Result<u32, String> {
	let value = obj.get(key).ok_or_else(|| format!("{unit}: missing attribute {key:?}"))?;
	let n = value.as_f64().ok_or_else(|| format!("{unit}: attribute {key:?} is not a number"))?;
	if n < 0.0 || n > u32::MAX as f64 || n.fract() != 0.0 {
		return Err(format!("{unit}: attribute {key:?} = {n} is not a non-negative integer"));
	}
	Ok(n as u32)
}

/// Parses clear-text `SC_ATTRI` JSON into per-unit-name attributes, in file
/// order. Every listed unit must carry all 12 fields (the engine validates the
/// same via the `SC_SCHEA` schema).
pub fn parse_attributes(text: &str) -> Result<Vec<(String, UnitAttributes)>, String> {
	let doc = json::parse(text)?;
	let units = doc.get("attributes").and_then(|a| a.as_object()).ok_or("SC_ATTRI: no \"attributes\" object")?;
	let mut out = Vec::with_capacity(units.len());
	for (name, fields) in units {
		let a = UnitAttributes {
			turns_to_build: attr_field(name, fields, "Turns to build")?,
			hit_points: attr_field(name, fields, "Hit points")?,
			armor_rating: attr_field(name, fields, "Armor rating")?,
			attack_rating: attr_field(name, fields, "Attack rating")?,
			move_and_fire: attr_field(name, fields, "Move and fire")?,
			movement_points: attr_field(name, fields, "Movement points")?,
			attack_range: attr_field(name, fields, "Attack range")?,
			shots_per_turn: attr_field(name, fields, "Shots per turn")?,
			scan_range: attr_field(name, fields, "Scan range")?,
			storage_capacity: attr_field(name, fields, "Storage capacity")?,
			ammunition: attr_field(name, fields, "Ammunition")?,
			blast_radius: attr_field(name, fields, "Blast radius")?,
		};
		out.push((name.clone(), a));
	}
	Ok(out)
}

/// Clamps to `u16` exactly like `TeamUnits::Init`'s
/// `std::min(x, UINT16_MAX)` narrowing.
fn to_u16(value: u32) -> u16 {
	value.min(u16::MAX as u32) as u16
}

/// Converts one unit's attributes into its base [`UnitValues`], mirroring
/// `TeamUnits::Init` (max-port `teamunits.cpp:50-63`): the 12 mapped fields;
/// fuel is set to 0 there and never serialized at all; `agent_adjust` stays 0,
/// `version` 1 and `in_use` false (the `UnitValues()` constructor state).
pub fn unit_values_from_attributes(a: &UnitAttributes) -> UnitValues {
	UnitValues {
		turns: to_u16(a.turns_to_build),
		hits: to_u16(a.hit_points),
		armor: to_u16(a.armor_rating),
		attack: to_u16(a.attack_rating),
		speed: to_u16(a.movement_points),
		range: to_u16(a.attack_range),
		rounds: to_u16(a.shots_per_turn),
		move_and_fire: a.move_and_fire.min(u8::MAX as u32) as u8,
		scan: to_u16(a.scan_range),
		storage: to_u16(a.storage_capacity),
		ammo: to_u16(a.ammunition),
		attack_radius: to_u16(a.blast_radius),
		agent_adjust: 0,
		version: 1,
		in_use: false,
	}
}

/// Builds the 93-entry base `UnitValues` table from parsed `SC_ATTRI`
/// attributes. Every physical unit type must be present — a gap would leave
/// the engine's own init asserting (`teamunits.cpp:68`), so it is an error
/// here, not a default.
pub fn base_unit_values(attribs: &[(String, UnitAttributes)]) -> Result<[UnitValues; UNIT_END], String> {
	let mut out: [Option<UnitValues>; UNIT_END] = [const { None }; UNIT_END];
	for (name, a) in attribs {
		if let Some(id) = unit_type_id(name) {
			out[id as usize] = Some(unit_values_from_attributes(a));
		}
	}
	let missing: Vec<&str> =
		(0..UNIT_END).filter(|&id| out[id].is_none()).map(|id| unit_type_name(id as u16).unwrap_or("?")).collect();
	if !missing.is_empty() {
		return Err(format!("SC_ATTRI is missing unit types: {}", missing.join(", ")));
	}
	Ok(out.map(|v| v.expect("checked above")))
}

/// Number of clans (`TEAM_CLAN_THE_CHOSEN`..`TEAM_CLAN_AXIS_INC` = 1..=8;
/// 0 is `TEAM_CLAN_RANDOM`).
pub const CLAN_COUNT: usize = 8;

/// One clan's attribute deltas for a single unit type (`SC_CLANS`
/// `advantages.attributes.<UNIT>`, max-port `UnitTradedoffs`). Signed —
/// e.g. Clan A's FIGHTER builds one turn *faster*. Absent keys stay 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClanTradeoffs {
	pub turns_to_build: i32,
	pub hit_points: i32,
	pub armor_rating: i32,
	pub attack_rating: i32,
	pub move_and_fire: i32,
	pub movement_points: i32,
	pub attack_range: i32,
	pub shots_per_turn: i32,
	pub scan_range: i32,
	pub storage_capacity: i32,
	pub ammunition: i32,
	pub blast_radius: i32,
	/// `"Experience"` — applied to `UnitValues::agent_adjust`
	/// (`ATTRIB_AGENT_ADJUST`, the `attrib_map` in
	/// `ResourceManager_SetClanUpgradesFromClans`).
	pub experience: i32,
}

/// One clan's advantages: starting-credits bonus + per-unit-name deltas.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClanAdvantages {
	/// English display name (`name."en-US"`), e.g. "The Chosen".
	pub name: String,
	pub credits: i32,
	pub units: Vec<(String, ClanTradeoffs)>,
}

impl ClanAdvantages {
	/// This clan's deltas for a unit type name (`None` = no advantage).
	pub fn tradeoffs_for(&self, unit_name: &str) -> Option<&ClanTradeoffs> {
		self.units.iter().find(|(name, _)| name == unit_name).map(|(_, t)| t)
	}
}

/// Reads one signed integer tradeoff field, 0 when absent.
fn tradeoff_field(clan: &str, unit: &str, obj: &json::JsonValue, key: &str) -> Result<i32, String> {
	let Some(value) = obj.get(key) else {
		return Ok(0);
	};
	let n = value.as_f64().ok_or_else(|| format!("{clan}/{unit}: tradeoff {key:?} is not a number"))?;
	if n.fract() != 0.0 || n < i32::MIN as f64 || n > i32::MAX as f64 {
		return Err(format!("{clan}/{unit}: tradeoff {key:?} = {n} is not an integer"));
	}
	Ok(n as i32)
}

/// All 13 recognized tradeoff attribute names (the 12 base ones +
/// `"Experience"`). Anything else in a unit's delta object is an error —
/// silently dropping a mistyped key would ship wrong stats.
const TRADEOFF_KEYS: [&str; 13] = [
	"Turns to build",
	"Hit points",
	"Armor rating",
	"Attack rating",
	"Move and fire",
	"Movement points",
	"Attack range",
	"Shots per turn",
	"Scan range",
	"Storage capacity",
	"Ammunition",
	"Blast radius",
	"Experience",
];

/// Parses clear-text `SC_CLANS` JSON into the 8 clans, indexed by
/// `TEAM_CLAN_*` − 1 (index 0 = `"Clan A"` = The Chosen … 7 = `"Clan H"`).
pub fn parse_clans(text: &str) -> Result<[ClanAdvantages; CLAN_COUNT], String> {
	let doc = json::parse(text)?;
	let clans = doc.get("clans").and_then(|c| c.as_object()).ok_or("SC_CLANS: no \"clans\" object")?;
	let mut out: [ClanAdvantages; CLAN_COUNT] = Default::default();
	let mut seen = [false; CLAN_COUNT];
	for (key, clan) in clans {
		// "Clan A".."Clan H" ↔ TEAM_CLAN 1..=8 (resource_manager.cpp:87).
		let letter = key.strip_prefix("Clan ").and_then(|s| s.chars().next());
		let index = match letter {
			Some(c @ 'A'..='H') if key.len() == 6 => (c as u8 - b'A') as usize,
			_ => return Err(format!("SC_CLANS: unrecognized clan key {key:?}")),
		};
		let name = clan
			.get("name")
			.and_then(|n| n.get("en-US"))
			.and_then(|n| n.as_str())
			.ok_or_else(|| format!("{key}: missing name.en-US"))?
			.to_string();
		let advantages = clan.get("advantages").ok_or_else(|| format!("{key}: missing advantages"))?;
		let credits = tradeoff_field(key, "-", advantages, "credits")?;
		let mut units = Vec::new();
		if let Some(attrs) = advantages.get("attributes") {
			let attrs = attrs.as_object().ok_or_else(|| format!("{key}: advantages.attributes is not an object"))?;
			for (unit_name, deltas) in attrs {
				let fields = deltas.as_object().ok_or_else(|| format!("{key}/{unit_name}: not an object"))?;
				if let Some((bad, _)) = fields.iter().find(|(k, _)| !TRADEOFF_KEYS.contains(&k.as_str())) {
					return Err(format!("{key}/{unit_name}: unknown tradeoff attribute {bad:?}"));
				}
				let t = ClanTradeoffs {
					turns_to_build: tradeoff_field(key, unit_name, deltas, "Turns to build")?,
					hit_points: tradeoff_field(key, unit_name, deltas, "Hit points")?,
					armor_rating: tradeoff_field(key, unit_name, deltas, "Armor rating")?,
					attack_rating: tradeoff_field(key, unit_name, deltas, "Attack rating")?,
					move_and_fire: tradeoff_field(key, unit_name, deltas, "Move and fire")?,
					movement_points: tradeoff_field(key, unit_name, deltas, "Movement points")?,
					attack_range: tradeoff_field(key, unit_name, deltas, "Attack range")?,
					shots_per_turn: tradeoff_field(key, unit_name, deltas, "Shots per turn")?,
					scan_range: tradeoff_field(key, unit_name, deltas, "Scan range")?,
					storage_capacity: tradeoff_field(key, unit_name, deltas, "Storage capacity")?,
					ammunition: tradeoff_field(key, unit_name, deltas, "Ammunition")?,
					blast_radius: tradeoff_field(key, unit_name, deltas, "Blast radius")?,
					experience: tradeoff_field(key, unit_name, deltas, "Experience")?,
				};
				units.push((unit_name.clone(), t));
			}
		}
		out[index] = ClanAdvantages { name, credits, units };
		seen[index] = true;
	}
	if let Some(missing) = seen.iter().position(|s| !s) {
		return Err(format!("SC_CLANS: missing \"Clan {}\"", (b'A' + missing as u8) as char));
	}
	Ok(out)
}

/// Adds a signed delta onto a `u16` stat. The engine does plain integer
/// addition (`UnitValues::AddAttribute`); stock data never overflows, so
/// saturation here is a guard, not a behavior difference.
fn add_u16(stat: &mut u16, delta: i32) {
	*stat = (*stat as i32 + delta).clamp(0, u16::MAX as i32) as u16;
}

/// Applies one clan's deltas for one unit onto its base [`UnitValues`],
/// mirroring `ResourceManager_SetClanUpgradesFromClans`
/// (`resource_manager.cpp:1178`): each nonzero tradeoff is *added*; fuel
/// cannot occur; `"Experience"` lands on `agent_adjust`.
pub fn apply_clan_upgrades(t: &ClanTradeoffs, v: &mut UnitValues) {
	add_u16(&mut v.attack, t.attack_rating);
	add_u16(&mut v.rounds, t.shots_per_turn);
	add_u16(&mut v.range, t.attack_range);
	add_u16(&mut v.armor, t.armor_rating);
	add_u16(&mut v.hits, t.hit_points);
	add_u16(&mut v.speed, t.movement_points);
	add_u16(&mut v.scan, t.scan_range);
	add_u16(&mut v.turns, t.turns_to_build);
	add_u16(&mut v.ammo, t.ammunition);
	v.move_and_fire = (v.move_and_fire as i32 + t.move_and_fire).clamp(0, u8::MAX as i32) as u8;
	add_u16(&mut v.storage, t.storage_capacity);
	add_u16(&mut v.attack_radius, t.blast_radius);
	add_u16(&mut v.agent_adjust, t.experience);
}

/// `Unit::Flags` (max-port `unit.hpp:77`) by their `SC_UNITS` JSON names
/// (`FlagMap`, `units.cpp:39`). The full engine set — the save's per-unit
/// `flags` word uses these bits (plus the runtime `HASH_TEAM_*` owner bits).
pub const FLAG_NAMES: [(&str, u32); 23] = [
	("GROUND_COVER", 0x1),
	("EXPLODING", 0x2),
	("ANIMATED", 0x4),
	("CONNECTOR_UNIT", 0x8),
	("BUILDING", 0x10),
	("MISSILE_UNIT", 0x20),
	("MOBILE_AIR_UNIT", 0x40),
	("MOBILE_SEA_UNIT", 0x80),
	("MOBILE_LAND_UNIT", 0x100),
	("STATIONARY", 0x200),
	("UPGRADABLE", 0x4000),
	("HOVERING", 0x10000),
	("HAS_FIRING_SPRITE", 0x20000),
	("FIRES_MISSILES", 0x40000),
	("CONSTRUCTOR_UNIT", 0x80000),
	("ELECTRONIC_UNIT", 0x200000),
	("SELECTABLE", 0x400000),
	("STANDALONE", 0x800000),
	("REQUIRES_SLAB", 0x1000000),
	("TURRET_SPRITE", 0x2000000),
	("SENTRY_UNIT", 0x4000000),
	("SPINNING_TURRET", 0x8000000),
	("REGENERATING_UNIT", 0x10000000),
];

/// What a unit's `storage` holds (`SC_UNITS` `cargo_type`,
/// `CARGO_TYPE_*` in max-port `enums.hpp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CargoType {
	#[default]
	None,
	Raw,
	Fuel,
	Gold,
	Land,
	Sea,
	Air,
}

impl CargoType {
	fn parse(s: &str) -> Option<CargoType> {
		Some(match s {
			"CARGO_TYPE_NONE" => CargoType::None,
			"CARGO_TYPE_RAW" => CargoType::Raw,
			"CARGO_TYPE_FUEL" => CargoType::Fuel,
			"CARGO_TYPE_GOLD" => CargoType::Gold,
			"CARGO_TYPE_LAND" => CargoType::Land,
			"CARGO_TYPE_SEA" => CargoType::Sea,
			"CARGO_TYPE_AIR" => CargoType::Air,
			_ => return None,
		})
	}
}

/// Static per-unit-type metadata from `SC_UNITS`: the engine flag word, what
/// the unit's `storage` stat means, and its `D_*` data-resource tag. Flags +
/// cargo type gate the stats editor; the data tag names the 24-byte
/// frame-info resource (in MAX.RES) that unit-body synthesis reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnitMeta {
	pub flags: u32,
	pub cargo_type: CargoType,
	/// `SC_UNITS` `"data"` — the frame-info resource tag, zero-padded ASCII
	/// (RES tags are ≤ 8 bytes; may be a shared class tag like `D_LRGBLD`).
	/// All zero = `INVALID_ID` (no data resource).
	pub data_tag: [u8; 8],
}

impl UnitMeta {
	/// The `D_*` tag as a string (`None` when the unit has no data resource).
	pub fn data_tag_str(&self) -> Option<&str> {
		let end = self.data_tag.iter().position(|&b| b == 0).unwrap_or(8);
		if end == 0 {
			return None;
		}
		std::str::from_utf8(&self.data_tag[..end]).ok()
	}
}

/// Parses clear-text `SC_UNITS` JSON into per-unit-name metadata (flags +
/// cargo type; the sprite/sound fields are not the editor's concern here).
pub fn parse_units_meta(text: &str) -> Result<Vec<(String, UnitMeta)>, String> {
	let doc = json::parse(text)?;
	let units = doc.get("units").and_then(|u| u.as_object()).ok_or("SC_UNITS: no \"units\" object")?;
	let mut out = Vec::with_capacity(units.len());
	for (name, unit) in units {
		let mut flags = 0u32;
		let flag_list =
			unit.get("flags").and_then(|f| f.as_array()).ok_or_else(|| format!("{name}: missing flags array"))?;
		for flag in flag_list {
			let flag = flag.as_str().ok_or_else(|| format!("{name}: non-string flag"))?;
			let (_, bit) =
				FLAG_NAMES.iter().find(|(n, _)| *n == flag).ok_or_else(|| format!("{name}: unknown flag {flag:?}"))?;
			flags |= bit;
		}
		let cargo =
			unit.get("cargo_type").and_then(|c| c.as_str()).ok_or_else(|| format!("{name}: missing cargo_type"))?;
		let cargo_type = CargoType::parse(cargo).ok_or_else(|| format!("{name}: unknown cargo_type {cargo:?}"))?;
		let data = unit.get("data").and_then(|d| d.as_str()).ok_or_else(|| format!("{name}: missing data tag"))?;
		let mut data_tag = [0u8; 8];
		if data != "INVALID_ID" {
			if data.len() > 8 || !data.is_ascii() {
				return Err(format!("{name}: bad data tag {data:?}"));
			}
			data_tag[..data.len()].copy_from_slice(data.as_bytes());
		}
		out.push((name.clone(), UnitMeta { flags, cargo_type, data_tag }));
	}
	Ok(out)
}

/// One unit type's sprite frame table — the 24-byte `D_*` data resource
/// (max-port `unit.cpp:51`): 8 signed base/count bytes, then 8 per-angle
/// turret offset pairs. The bases/indices the save's image fields store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameInfo {
	pub image_base: i16,
	pub image_count: i16,
	pub turret_image_base: i16,
	pub turret_image_count: i16,
	pub firing_image_base: i16,
	pub firing_image_count: i16,
	pub connector_image_base: i16,
	pub connector_image_count: i16,
	/// Turret draw offset per body angle (only meaningful for
	/// `TURRET_SPRITE`/`SPINNING_TURRET` types).
	pub angle_offsets: [(i8, i8); 8],
}

/// Parses a `D_*` data resource (≥ 8 bytes; the 16 offset bytes may be absent
/// for non-turret class templates — they read as zero).
pub fn parse_frame_info(data: &[u8]) -> Result<FrameInfo, String> {
	if data.len() < 8 {
		return Err(format!("frame-info resource too short: {} bytes", data.len()));
	}
	let b = |i: usize| data[i] as i8 as i16;
	let mut fi = FrameInfo {
		image_base: b(0),
		image_count: b(1),
		turret_image_base: b(2),
		turret_image_count: b(3),
		firing_image_base: b(4),
		firing_image_count: b(5),
		connector_image_base: b(6),
		connector_image_count: b(7),
		angle_offsets: [(0, 0); 8],
	};
	if data.len() >= 24 {
		for i in 0..8 {
			fi.angle_offsets[i] = (data[8 + i * 2] as i8, data[8 + i * 2 + 1] as i8);
		}
	}
	Ok(fi)
}

/// Loads the frame-info table for all 93 unit types from a RES archive
/// (normally the user's `MAX.RES`) using each type's `SC_UNITS` data tag.
/// `None` = the type has no data resource, or the archive lacks the tag —
/// callers that synthesize such a type surface the gap then.
pub fn load_frame_infos(res_path: &Path, meta: &[UnitMeta; UNIT_END]) -> [Option<FrameInfo>; UNIT_END] {
	// Distinct tags first — class templates (D_LRGBLD…) are shared by many
	// types and the index scan per read is cheap but not free.
	let mut cache: Vec<(String, Option<FrameInfo>)> = Vec::new();
	std::array::from_fn(|id| {
		let tag = meta[id].data_tag_str()?;
		if let Some((_, fi)) = cache.iter().find(|(t, _)| t == tag) {
			return *fi;
		}
		let fi = match read_res_entry(res_path, tag) {
			Ok(Some(data)) => parse_frame_info(&data).map_err(|e| log::warn!("frame info {tag}: {e}")).ok(),
			Ok(None) => {
				log::warn!("frame info {tag}: not in {}", res_path.display());
				None
			}
			Err(e) => {
				log::warn!("frame info {tag}: {e}");
				None
			}
		};
		cache.push((tag.to_string(), fi));
		fi
	})
}

/// Fills the 93-entry [`UnitMeta`] table; every physical unit type must be
/// listed (same rule as [`base_unit_values`]).
pub fn unit_meta_table(meta: &[(String, UnitMeta)]) -> Result<[UnitMeta; UNIT_END], String> {
	let mut out = [UnitMeta::default(); UNIT_END];
	let mut seen = [false; UNIT_END];
	for (name, m) in meta {
		if let Some(id) = unit_type_id(name) {
			out[id as usize] = *m;
			seen[id as usize] = true;
		}
	}
	let missing: Vec<&str> =
		(0..UNIT_END).filter(|&id| !seen[id]).map(|id| unit_type_name(id as u16).unwrap_or("?")).collect();
	if !missing.is_empty() {
		return Err(format!("SC_UNITS is missing unit types: {}", missing.join(", ")));
	}
	Ok(out)
}

/// The loaded max-port unit database: stock base stats, the 8 clans'
/// advantages, and per-type metadata — everything the editor needs to seed
/// `UnitValues` without a save and to gate stat applicability.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitStatsDb {
	pub base: [UnitValues; UNIT_END],
	pub clans: [ClanAdvantages; CLAN_COUNT],
	pub meta: [UnitMeta; UNIT_END],
	/// The `PATCHES.RES` this db was read from (for diagnostics / reload).
	pub source: std::path::PathBuf,
}

impl UnitStatsDb {
	/// Loads the whole database from a `PATCHES.RES` archive.
	pub fn load(patches_res: &Path) -> Result<UnitStatsDb, String> {
		let read = |tag: &str| -> Result<String, String> {
			read_script_entry(patches_res, tag)
				.map_err(|e| format!("{}: {tag}: {e}", patches_res.display()))?
				.ok_or_else(|| format!("{}: no {tag} entry", patches_res.display()))
		};
		let base = base_unit_values(&parse_attributes(&read("SC_ATTRI")?).map_err(|e| format!("SC_ATTRI: {e}"))?)?;
		let clans = parse_clans(&read("SC_CLANS")?).map_err(|e| format!("SC_CLANS: {e}"))?;
		let meta = unit_meta_table(&parse_units_meta(&read("SC_UNITS")?).map_err(|e| format!("SC_UNITS: {e}"))?)?;
		Ok(UnitStatsDb { base, clans, meta, source: patches_res.to_path_buf() })
	}

	/// Stock base values for a unit type (`None` past `UNIT_END`).
	pub fn base_for(&self, unit_type: u16) -> Option<&UnitValues> {
		self.base.get(unit_type as usize)
	}

	/// Metadata for a unit type (`None` past `UNIT_END`).
	pub fn meta_for(&self, unit_type: u16) -> Option<&UnitMeta> {
		self.meta.get(unit_type as usize)
	}

	/// One unit type's values under a clan (`TEAM_CLAN_*` 1..=8; anything
	/// else — 0/random — returns plain base values), mirroring
	/// `ResourceManager_InitClanUnitValues`'s per-unit step.
	pub fn clan_values(&self, clan: u8, unit_type: u16) -> Option<UnitValues> {
		let mut v = self.base_for(unit_type)?.clone();
		if (1..=CLAN_COUNT as u8).contains(&clan) {
			let name = unit_type_name(unit_type)?;
			if let Some(t) = self.clans[clan as usize - 1].tradeoffs_for(name) {
				apply_clan_upgrades(t, &mut v);
			}
		}
		Some(v)
	}

	/// The full 93-entry `UnitValues` table for a team of the given clan —
	/// the `TeamUnits` base/current seed for save synthesis.
	pub fn clan_unit_values(&self, clan: u8) -> [UnitValues; UNIT_END] {
		std::array::from_fn(|id| self.clan_values(clan, id as u16).expect("id < UNIT_END"))
	}
}

/// The editable [`UnitValues`] stats, for per-unit applicability gating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatKind {
	Turns,
	Hits,
	Armor,
	Attack,
	Speed,
	Range,
	Rounds,
	MoveAndFire,
	Scan,
	Storage,
	Ammo,
	AttackRadius,
	AgentAdjust,
	Version,
}

impl StatKind {
	/// Reads this stat off a [`UnitValues`].
	pub fn get(self, v: &UnitValues) -> u32 {
		match self {
			StatKind::Turns => v.turns as u32,
			StatKind::Hits => v.hits as u32,
			StatKind::Armor => v.armor as u32,
			StatKind::Attack => v.attack as u32,
			StatKind::Speed => v.speed as u32,
			StatKind::Range => v.range as u32,
			StatKind::Rounds => v.rounds as u32,
			StatKind::MoveAndFire => v.move_and_fire as u32,
			StatKind::Scan => v.scan as u32,
			StatKind::Storage => v.storage as u32,
			StatKind::Ammo => v.ammo as u32,
			StatKind::AttackRadius => v.attack_radius as u32,
			StatKind::AgentAdjust => v.agent_adjust as u32,
			StatKind::Version => v.version as u32,
		}
	}
}

/// `MOBILE_AIR_UNIT | MOBILE_SEA_UNIT | MOBILE_LAND_UNIT`.
const MOBILE_MASK: u32 = 0x40 | 0x80 | 0x100;

/// Whether a stat is *game-meaningful* for a unit type, judged the way the
/// engine consumes it: the attack cluster only matters for units that can
/// fight (nonzero base attack/shots/ammo — the report screen gates its rows
/// the same value-driven way, `reportstats.cpp:515`), speed only for mobile
/// units, storage only when the type has a cargo kind, and `agent_adjust`
/// only for the COMMANDO (the one unit whose `experience` reads it,
/// `units_manager.cpp:2264`). Hits/armor/scan/turns/version apply to all.
///
/// The editor uses this to hide/refuse editors for stats the game would
/// ignore — not to constrain values of applicable ones.
pub fn stat_applicable(kind: StatKind, unit_type: u16, meta: &UnitMeta, base: &UnitValues) -> bool {
	let armed = base.attack > 0 || base.rounds > 0 || base.ammo > 0;
	let mobile = meta.flags & MOBILE_MASK != 0;
	match kind {
		StatKind::Turns | StatKind::Hits | StatKind::Armor | StatKind::Scan | StatKind::Version => true,
		StatKind::Attack | StatKind::Rounds | StatKind::Range | StatKind::Ammo | StatKind::AttackRadius => armed,
		StatKind::MoveAndFire => armed && mobile,
		StatKind::Speed => mobile,
		StatKind::Storage => meta.cargo_type != CargoType::None,
		StatKind::AgentAdjust => unit_type_name(unit_type) == Some("COMMANDO"),
	}
}

/// Finds a `PATCHES.RES` among the candidate locations, in order. Each
/// candidate may be the archive itself or a directory containing it. Returns
/// the first hit, or an error listing every path tried (for the console).
pub fn locate_patches_res<I, P>(candidates: I) -> Result<std::path::PathBuf, String>
where
	I: IntoIterator<Item = P>,
	P: AsRef<Path>,
{
	let mut tried = Vec::new();
	for candidate in candidates {
		let candidate = candidate.as_ref();
		let path = if candidate.is_dir() { candidate.join("PATCHES.RES") } else { candidate.to_path_buf() };
		if path.is_file() {
			return Ok(path);
		}
		tried.push(path.display().to_string());
	}
	Err(if tried.is_empty() {
		"no candidate paths for PATCHES.RES (set MaxPortDataPath)".to_string()
	} else {
		format!("PATCHES.RES not found; tried: {}", tried.join(", "))
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A local copy of the user's max-port `PATCHES.RES` (git-ignored game
	/// asset, like the save fixtures). Tests skip clean when absent.
	pub(crate) fn fixture_path() -> Option<std::path::PathBuf> {
		let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/PATCHES.RES");
		if path.exists() {
			Some(path)
		} else {
			crate::testutil::skip_fixture("testdata/PATCHES.RES fixture not present");
			None
		}
	}

	#[test]
	fn deobfuscate_is_an_involution() {
		let original: Vec<u8> = (0u16..300).map(|i| (i % 251) as u8).collect();
		let mut data = original.clone();
		deobfuscate(&mut data);
		assert_ne!(data, original, "one pass must change the bytes");
		deobfuscate(&mut data);
		assert_eq!(data, original, "two passes must restore the input");
	}

	#[test]
	fn missing_tag_reads_as_none() {
		let Some(path) = fixture_path() else {
			return;
		};
		assert_eq!(read_script_entry(&path, "NO_SUCH0").unwrap(), None);
	}

	#[test]
	fn parse_attributes_reads_all_12_fields() {
		let text = r#"{"attributes":{"TANK":{"Turns to build":4,"Hit points":24,"Armor rating":10,
			"Attack rating":16,"Move and fire":0,"Movement points":6,"Attack range":4,
			"Shots per turn":2,"Scan range":4,"Storage capacity":0,"Ammunition":14,"Blast radius":0}}}"#;
		let attribs = parse_attributes(text).unwrap();
		assert_eq!(attribs.len(), 1);
		let (name, a) = &attribs[0];
		assert_eq!(name, "TANK");
		assert_eq!(
			*a,
			UnitAttributes {
				turns_to_build: 4,
				hit_points: 24,
				armor_rating: 10,
				attack_rating: 16,
				move_and_fire: 0,
				movement_points: 6,
				attack_range: 4,
				shots_per_turn: 2,
				scan_range: 4,
				storage_capacity: 0,
				ammunition: 14,
				blast_radius: 0
			}
		);
		let v = unit_values_from_attributes(a);
		assert_eq!((v.turns, v.hits, v.armor, v.attack, v.speed), (4, 24, 10, 16, 6));
		assert_eq!((v.range, v.rounds, v.move_and_fire, v.scan), (4, 2, 0, 4));
		assert_eq!((v.storage, v.ammo, v.attack_radius), (0, 14, 0));
		assert_eq!((v.agent_adjust, v.version, v.in_use), (0, 1, false));
	}

	#[test]
	fn parse_attributes_rejects_missing_and_bad_fields() {
		let missing = r#"{"attributes":{"TANK":{"Turns to build":4}}}"#;
		assert!(parse_attributes(missing).unwrap_err().contains("Hit points"));
		let negative = r#"{"attributes":{"TANK":{"Turns to build":-1,"Hit points":24,"Armor rating":10,
			"Attack rating":16,"Move and fire":0,"Movement points":6,"Attack range":4,
			"Shots per turn":2,"Scan range":4,"Storage capacity":0,"Ammunition":14,"Blast radius":0}}}"#;
		assert!(parse_attributes(negative).unwrap_err().contains("Turns to build"));
	}

	#[test]
	fn base_unit_values_requires_every_unit_type() {
		let attribs = vec![("TANK".to_string(), UnitAttributes::default())];
		let err = base_unit_values(&attribs).unwrap_err();
		assert!(err.contains("missing unit types"));
		assert!(err.contains("COMMTWR"), "names the gaps: {err}");
	}

	#[test]
	fn fixture_base_values_cover_all_93_units() {
		let Some(path) = fixture_path() else {
			return;
		};
		let text = read_script_entry(&path, "SC_ATTRI").unwrap().expect("SC_ATTRI present");
		let attribs = parse_attributes(&text).unwrap();
		assert_eq!(attribs.len(), UNIT_END, "stock SC_ATTRI lists exactly the 93 physical unit types");
		let base = base_unit_values(&attribs).unwrap();
		// Stock spot checks (values eyeballed from the user-decoded SC_ATTRI.json).
		let tank = &base[unit_type_id("TANK").unwrap() as usize];
		assert_eq!((tank.hits, tank.attack, tank.rounds, tank.ammo, tank.speed), (24, 16, 2, 14, 6));
		let fighter = &base[unit_type_id("FIGHTER").unwrap() as usize];
		assert_eq!((fighter.hits, fighter.move_and_fire, fighter.speed, fighter.range), (12, 1, 24, 5));
		let miningst = &base[unit_type_id("MININGST").unwrap() as usize];
		assert_eq!((miningst.turns, miningst.hits, miningst.storage, miningst.attack), (12, 56, 25, 0));
	}

	#[test]
	fn parse_clans_maps_letters_and_deltas() {
		let text = r#"{"clans":{
			"Clan A":{"name":{"en-US":"The Chosen"},"advantages":{"credits":0,"attributes":{
				"FIGHTER":{"Attack range":1,"Turns to build":-1}}}},
			"Clan B":{"name":{"en-US":"Crimson Path"},"advantages":{"credits":0,"attributes":{}}},
			"Clan C":{"name":{"en-US":"Von Griffin's"},"advantages":{"credits":0,"attributes":{}}},
			"Clan D":{"name":{"en-US":"Ayer's Hand"},"advantages":{"credits":0,"attributes":{}}},
			"Clan E":{"name":{"en-US":"Musashi"},"advantages":{"credits":0,"attributes":{}}},
			"Clan F":{"name":{"en-US":"Sacred Eights"},"advantages":{"credits":0,"attributes":{}}},
			"Clan G":{"name":{"en-US":"7 Knights"},"advantages":{"credits":0,"attributes":{}}},
			"Clan H":{"name":{"en-US":"Axis Inc."},"advantages":{"credits":100,"attributes":{
				"COMMANDO":{"Experience":2}}}}}}"#;
		let clans = parse_clans(text).unwrap();
		assert_eq!(clans[0].name, "The Chosen");
		let fighter = clans[0].tradeoffs_for("FIGHTER").unwrap();
		assert_eq!((fighter.attack_range, fighter.turns_to_build), (1, -1));
		assert_eq!(clans[7].credits, 100);
		assert_eq!(clans[7].tradeoffs_for("COMMANDO").unwrap().experience, 2);
		assert_eq!(clans[1].tradeoffs_for("FIGHTER"), None);

		// Applying Clan A's FIGHTER deltas onto stock base values.
		let mut v = unit_values_from_attributes(&UnitAttributes {
			turns_to_build: 8,
			hit_points: 12,
			armor_rating: 4,
			attack_rating: 16,
			move_and_fire: 1,
			movement_points: 24,
			attack_range: 5,
			shots_per_turn: 1,
			scan_range: 5,
			storage_capacity: 0,
			ammunition: 4,
			blast_radius: 0,
		});
		apply_clan_upgrades(fighter, &mut v);
		assert_eq!((v.range, v.turns), (6, 7), "range +1, one turn faster");
		assert_eq!((v.hits, v.attack), (12, 16), "untouched stats stay");
	}

	#[test]
	fn parse_clans_rejects_unknown_attributes_and_gaps() {
		let bad_key = r#"{"clans":{"Clan A":{"name":{"en-US":"x"},"advantages":{"credits":0,
			"attributes":{"TANK":{"Atack rating":1}}}}}}"#;
		assert!(parse_clans(bad_key).unwrap_err().contains("Atack rating"));
		let one_clan = r#"{"clans":{"Clan B":{"name":{"en-US":"x"},"advantages":{"credits":0,"attributes":{}}}}}"#;
		assert!(parse_clans(one_clan).unwrap_err().contains("Clan A"));
	}

	#[test]
	fn fixture_clans_match_stock_advantages() {
		let Some(path) = fixture_path() else {
			return;
		};
		let text = read_script_entry(&path, "SC_CLANS").unwrap().expect("SC_CLANS present");
		let clans = parse_clans(&text).unwrap();
		assert_eq!(clans[0].name, "The Chosen");
		// Clan A: better air units — FIGHTER range +1, turns −1 (stock data).
		let fighter = clans[0].tradeoffs_for("FIGHTER").unwrap();
		assert_eq!((fighter.attack_range, fighter.turns_to_build), (1, -1));
		// Clan E (Musashi): better TANK.
		assert!(clans[4].tradeoffs_for("TANK").is_some(), "Musashi upgrades the TANK");
	}

	#[test]
	fn parse_units_meta_maps_flags_and_cargo() {
		let text = r#"{"units":{"TANK":{"flags":["MOBILE_LAND_UNIT","TURRET_SPRITE"],"cargo_type":"CARGO_TYPE_NONE","data":"D_TANK"},
			"ENGINEER":{"flags":["MOBILE_SEA_UNIT","MOBILE_LAND_UNIT"],"cargo_type":"CARGO_TYPE_RAW","data":"INVALID_ID"}}}"#;
		let meta = parse_units_meta(text).unwrap();
		assert_eq!(meta[0].0, "TANK");
		assert_eq!(meta[0].1.flags, 0x100 | 0x0200_0000);
		assert_eq!(meta[0].1.cargo_type, CargoType::None);
		assert_eq!(meta[0].1.data_tag_str(), Some("D_TANK"));
		assert_eq!(meta[1].1.cargo_type, CargoType::Raw);
		assert_eq!(meta[1].1.data_tag_str(), None, "INVALID_ID -> no data resource");

		let bad = r#"{"units":{"TANK":{"flags":["NO_SUCH_FLAG"],"cargo_type":"CARGO_TYPE_NONE","data":"D_TANK"}}}"#;
		assert!(parse_units_meta(bad).unwrap_err().contains("NO_SUCH_FLAG"));
	}

	#[test]
	fn parse_frame_info_reads_bases_and_offsets() {
		// 8 signed bases/counts + 8 (x, y) turret offset pairs.
		let mut data = vec![0u8, 8, 16, 8, 24, 8, 32, 8];
		data.extend((0..16).map(|i| if i % 2 == 0 { 2u8 } else { 0xFF }));
		let fi = parse_frame_info(&data).unwrap();
		assert_eq!((fi.image_base, fi.image_count), (0, 8));
		assert_eq!((fi.turret_image_base, fi.connector_image_base), (16, 32));
		assert_eq!(fi.angle_offsets[0], (2, -1), "signed per-angle offsets");
		// A bases-only class template (8 bytes) still parses; offsets read zero.
		let fi = parse_frame_info(&data[..8]).unwrap();
		assert_eq!(fi.angle_offsets, [(0, 0); 8]);
		assert!(parse_frame_info(&[1, 2, 3]).is_err(), "short resource is an error");
	}

	#[test]
	fn fixture_frame_infos_resolve_from_max_res() {
		let Some(path) = fixture_path() else {
			return;
		};
		let max_res = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("MAX/MAX.RES");
		if !max_res.is_file() {
			crate::testutil::skip_fixture("~/MAX/MAX.RES not present");
			return;
		}
		let db = UnitStatsDb::load(&path).unwrap();
		let frames = load_frame_infos(&max_res, &db.meta);
		let resolved = frames.iter().filter(|f| f.is_some()).count();
		assert!(resolved >= 90, "nearly every type has frame info (got {resolved})");
		// TANK has a turret strip; its turret base sits past the 8 body frames.
		let tank = frames[unit_type_id("TANK").unwrap() as usize].expect("TANK frame info");
		assert!(tank.turret_image_base >= 8, "turret frames follow the body frames: {tank:?}");
	}

	#[test]
	fn locate_patches_res_walks_candidates() {
		let dir = std::env::temp_dir().join(format!("mme-attribs-test-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let missing = dir.join("nowhere");
		let err = locate_patches_res([&missing]).unwrap_err();
		assert!(err.contains("nowhere"), "error lists tried paths: {err}");

		let file = dir.join("PATCHES.RES");
		std::fs::write(&file, b"stub").unwrap();
		// A directory candidate resolves to the archive inside it…
		assert_eq!(locate_patches_res([&dir]).unwrap(), file);
		// …and a direct file candidate wins as-is, even after misses.
		assert_eq!(locate_patches_res([missing.as_path(), file.as_path()]).unwrap(), file);
		std::fs::remove_dir_all(&dir).ok();
	}

	#[test]
	fn stat_applicability_follows_unit_role() {
		// A TANK-like unit: mobile, armed, no cargo.
		let tank_meta = UnitMeta { flags: 0x100 | 0x0200_0000, cargo_type: CargoType::None, ..Default::default() };
		let tank_base = unit_values_from_attributes(&UnitAttributes {
			attack_rating: 16,
			shots_per_turn: 2,
			ammunition: 14,
			movement_points: 6,
			..Default::default()
		});
		let tank_id = unit_type_id("TANK").unwrap();
		for kind in [StatKind::Attack, StatKind::Range, StatKind::Speed, StatKind::MoveAndFire, StatKind::Hits] {
			assert!(stat_applicable(kind, tank_id, &tank_meta, &tank_base), "{kind:?} applies to an armed mobile");
		}
		assert!(!stat_applicable(StatKind::Storage, tank_id, &tank_meta, &tank_base), "no cargo -> no storage");
		assert!(!stat_applicable(StatKind::AgentAdjust, tank_id, &tank_meta, &tank_base), "not an agent");

		// A RADAR-like unit: stationary, unarmed.
		let radar_meta = UnitMeta { flags: 0x200, cargo_type: CargoType::None, ..Default::default() };
		let radar_base = unit_values_from_attributes(&UnitAttributes { scan_range: 18, ..Default::default() });
		let radar_id = unit_type_id("RADAR").unwrap();
		for kind in [StatKind::Attack, StatKind::Rounds, StatKind::Ammo, StatKind::Speed, StatKind::AttackRadius] {
			assert!(!stat_applicable(kind, radar_id, &radar_meta, &radar_base), "{kind:?} is meaningless on a radar");
		}
		for kind in [StatKind::Hits, StatKind::Armor, StatKind::Scan, StatKind::Turns, StatKind::Version] {
			assert!(stat_applicable(kind, radar_id, &radar_meta, &radar_base), "{kind:?} applies to everything");
		}

		// A storage carrier gets the storage editor; the COMMANDO its agent skill.
		let truck_meta = UnitMeta { flags: 0x100, cargo_type: CargoType::Raw, ..Default::default() };
		assert!(stat_applicable(StatKind::Storage, 0, &truck_meta, &radar_base));
		let commando_id = unit_type_id("COMMANDO").unwrap();
		assert!(stat_applicable(StatKind::AgentAdjust, commando_id, &radar_meta, &radar_base));

		// StatKind::get reads the matching field.
		assert_eq!(StatKind::Attack.get(&tank_base), 16);
		assert_eq!(StatKind::Speed.get(&tank_base), 6);
	}

	#[test]
	fn fixture_db_loads_and_applies_clans() {
		let Some(path) = fixture_path() else {
			return;
		};
		let db = UnitStatsDb::load(&path).unwrap();
		// TANK: mobile land unit with a turret, no cargo.
		let tank_id = unit_type_id("TANK").unwrap();
		let tank_meta = db.meta_for(tank_id).unwrap();
		assert_ne!(tank_meta.flags & 0x100, 0, "TANK is MOBILE_LAND_UNIT");
		assert_ne!(tank_meta.flags & 0x0200_0000, 0, "TANK has TURRET_SPRITE");
		assert_eq!(tank_meta.cargo_type, CargoType::None);
		// ENGINEER carries raw materials.
		assert_eq!(db.meta_for(unit_type_id("ENGINEER").unwrap()).unwrap().cargo_type, CargoType::Raw);
		// Musashi (clan 5) upgrades the TANK relative to base…
		let base = db.base_for(tank_id).unwrap().clone();
		let musashi = db.clan_values(5, tank_id).unwrap();
		assert_ne!(musashi, base);
		// …while clan 0 (random/none) and a clan without TANK deltas stay base.
		assert_eq!(db.clan_values(0, tank_id).unwrap(), base);
		let table = db.clan_unit_values(5);
		assert_eq!(table[tank_id as usize], musashi);
	}

	#[test]
	fn sc_attri_deobfuscates_to_json() {
		let Some(path) = fixture_path() else {
			return;
		};
		let text = read_script_entry(&path, "SC_ATTRI").unwrap().expect("PATCHES.RES carries SC_ATTRI");
		let doc = json::parse(&text).expect("clear-text SC_ATTRI parses as JSON");
		// Known stock values: COMMTWR builds in 12 turns with 56 hit points.
		let commtwr = doc.get("attributes").and_then(|a| a.get("COMMTWR")).expect("attributes.COMMTWR");
		assert_eq!(commtwr.get("Turns to build").and_then(|v| v.as_f64()), Some(12.0));
		assert_eq!(commtwr.get("Hit points").and_then(|v| v.as_f64()), Some(56.0));
	}
}
