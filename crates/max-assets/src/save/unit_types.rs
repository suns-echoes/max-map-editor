//! `ResourceID` unit-type metadata: the canonical id → name table plus a
//! flag-based list classifier.
//!
//! A decoded [`UnitRecord::unit_type`](super::types::UnitRecord::unit_type) is a
//! `ResourceID` (`resourcetable.hpp`). The editor renders it by looking its
//! **name** up as a sprite tag (`app/src/units.rs` `STRIPS`, whose tags are the
//! very same enum names), so the name table here is the bridge from a parsed
//! save to editor sprites. The classifier mirrors the engine's own routing
//! (`Task_GetUnitList`, `task.cpp`) so a placed unit lands in the right one of
//! the five unit lists.
//!
//! Footprint is intentionally *not* duplicated here — the editor derives it from
//! the loaded sprite's size (`UnitEntry::footprint`, one source of truth).

use super::types::{UNIT_END, UnitRecord};

/// The M.A.X. object flag bits the editor cares about (`enums.hpp`). Only the
/// subset used for classification/ownership is transcribed; the unit's stored
/// `flags` word carries the rest.
pub mod flag {
	pub const GROUND_COVER: u32 = 0x1;
	pub const CONNECTOR_UNIT: u32 = 0x8;
	pub const BUILDING: u32 = 0x10;
	pub const MOBILE_AIR_UNIT: u32 = 0x40;
	pub const MOBILE_SEA_UNIT: u32 = 0x80;
	pub const MOBILE_LAND_UNIT: u32 = 0x100;
	pub const STATIONARY: u32 = 0x200;
	pub const STANDALONE: u32 = 0x0080_0000;
	pub const HASH_TEAM_GRAY: u32 = 0x400;
	pub const HASH_TEAM_BLUE: u32 = 0x800;
	pub const HASH_TEAM_GREEN: u32 = 0x1000;
	pub const HASH_TEAM_RED: u32 = 0x2000;
	pub const HASH_TEAM_DERELICT: u32 = 0x8000;
}

/// Which of the five save unit lists a unit belongs to. `Particle` is a special
/// case: particle effects are not routed by [`UnitCategory::from_flags`] (they
/// are never placed by the editor) — a unit is a particle only by virtue of
/// living in the on-disk `ParticleUnits` list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitCategory {
	/// Slabs, rubble, roads, connectors, mines, tape/cones — paintable dressing.
	GroundCover,
	/// Buildings.
	Stationary,
	/// Land and sea vehicles.
	MobileLandSea,
	/// Air units.
	MobileAir,
	/// Transient FX (explosions, projectiles, smoke).
	Particle,
}

impl UnitCategory {
	/// Classifies a unit from its stored `flags`, mirroring the engine's
	/// `Task_GetUnitList` (`task.cpp`): `STATIONARY` splits into ground-cover
	/// (also `GROUND_COVER`) vs. buildings, then air, then everything else is
	/// land/sea. Never returns [`UnitCategory::Particle`] — particle membership
	/// is known only from the on-disk list.
	pub fn from_flags(flags: u32) -> UnitCategory {
		if flags & flag::STATIONARY != 0 {
			if flags & flag::GROUND_COVER != 0 { UnitCategory::GroundCover } else { UnitCategory::Stationary }
		} else if flags & flag::MOBILE_AIR_UNIT != 0 {
			UnitCategory::MobileAir
		} else {
			UnitCategory::MobileLandSea
		}
	}
}

/// `ResourceID` names for every physical unit type `0x00..UNIT_END` (`0x5D`), in
/// enum order (`resourcetable.hpp`). Indexed directly by `unit_type`. These are
/// also the editor's sprite tags (`app/src/units.rs`), except the FX/particle
/// and internal types (`*EXPLD`, `MASTER`, `ROCKET`, `HARVSTER`, `WALDO`, …),
/// which have no placeable sprite.
pub const UNIT_TYPE_NAMES: [&str; UNIT_END] = [
	"COMMTWR",  // 0x00
	"POWERSTN", // 0x01
	"POWGEN",   // 0x02
	"BARRACKS", // 0x03
	"SHIELDGN", // 0x04
	"RADAR",    // 0x05
	"ADUMP",    // 0x06
	"FDUMP",    // 0x07
	"GOLDSM",   // 0x08
	"DEPOT",    // 0x09
	"HANGAR",   // 0x0A
	"DOCK",     // 0x0B
	"CNCT_4W",  // 0x0C
	"LRGRUBLE", // 0x0D
	"SMLRUBLE", // 0x0E
	"LRGTAPE",  // 0x0F
	"SMLTAPE",  // 0x10
	"LRGSLAB",  // 0x11
	"SMLSLAB",  // 0x12
	"LRGCONES", // 0x13
	"SMLCONES", // 0x14
	"ROAD",     // 0x15
	"LANDPAD",  // 0x16
	"SHIPYARD", // 0x17
	"LIGHTPLT", // 0x18
	"LANDPLT",  // 0x19
	"SUPRTPLT", // 0x1A
	"AIRPLT",   // 0x1B
	"HABITAT",  // 0x1C
	"RESEARCH", // 0x1D
	"GREENHSE", // 0x1E
	"RECCENTR", // 0x1F
	"TRAINHAL", // 0x20
	"WTRPLTFM", // 0x21
	"GUNTURRT", // 0x22
	"ANTIAIR",  // 0x23
	"ARTYTRRT", // 0x24
	"ANTIMSSL", // 0x25
	"BLOCK",    // 0x26
	"BRIDGE",   // 0x27
	"MININGST", // 0x28
	"LANDMINE", // 0x29
	"SEAMINE",  // 0x2A
	"LNDEXPLD", // 0x2B
	"AIREXPLD", // 0x2C
	"SEAEXPLD", // 0x2D
	"BLDEXPLD", // 0x2E
	"HITEXPLD", // 0x2F
	"MASTER",   // 0x30
	"CONSTRCT", // 0x31
	"SCOUT",    // 0x32
	"TANK",     // 0x33
	"ARTILLRY", // 0x34
	"ROCKTLCH", // 0x35
	"MISSLLCH", // 0x36
	"SP_FLAK",  // 0x37
	"MINELAYR", // 0x38
	"SURVEYOR", // 0x39
	"SCANNER",  // 0x3A
	"SPLYTRCK", // 0x3B
	"GOLDTRCK", // 0x3C
	"ENGINEER", // 0x3D
	"BULLDOZR", // 0x3E
	"REPAIR",   // 0x3F
	"FUELTRCK", // 0x40
	"CLNTRANS", // 0x41
	"COMMANDO", // 0x42
	"INFANTRY", // 0x43
	"FASTBOAT", // 0x44
	"CORVETTE", // 0x45
	"BATTLSHP", // 0x46
	"SUBMARNE", // 0x47
	"SEATRANS", // 0x48
	"MSSLBOAT", // 0x49
	"SEAMNLYR", // 0x4A
	"CARGOSHP", // 0x4B
	"FIGHTER",  // 0x4C
	"BOMBER",   // 0x4D
	"AIRTRANS", // 0x4E
	"AWAC",     // 0x4F
	"JUGGRNT",  // 0x50
	"ALNTANK",  // 0x51
	"ALNASGUN", // 0x52
	"ALNPLANE", // 0x53
	"ROCKET",   // 0x54
	"TORPEDO",  // 0x55
	"ALNMISSL", // 0x56
	"ALNTBALL", // 0x57
	"ALNABALL", // 0x58
	"RKTSMOKE", // 0x59
	"TRPBUBLE", // 0x5A
	"HARVSTER", // 0x5B
	"WALDO",    // 0x5C
];

/// The `ResourceID` name for a physical unit type, or `None` for an id at/above
/// `UNIT_END` (graphics/derelict ids past the unit range).
pub fn unit_type_name(unit_type: u16) -> Option<&'static str> {
	UNIT_TYPE_NAMES.get(unit_type as usize).copied()
}

/// The `unit_type` (`ResourceID`) for a `ResourceID` / sprite-tag name, the
/// inverse of [`unit_type_name`]. `None` for a name that isn't a physical unit
/// type. The editor bridges a sprite tag (`app/src/units.rs`) back to a
/// `unit_type` through this when promoting a preview annotation to a first-class
/// object (`MapObject`).
pub fn unit_type_id(name: &str) -> Option<u16> {
	UNIT_TYPE_NAMES.iter().position(|&n| n == name).map(|i| i as u16)
}

impl UnitRecord {
	/// This unit's `ResourceID` name — the editor's sprite tag (see
	/// [`unit_type_name`]).
	pub fn type_name(&self) -> Option<&'static str> {
		unit_type_name(self.unit_type)
	}

	/// Which unit list this record's `flags` route it to (see
	/// [`UnitCategory::from_flags`]).
	pub fn category(&self) -> UnitCategory {
		UnitCategory::from_flags(self.flags)
	}

	/// Whether this is paintable ground cover (slab / rubble / road / …).
	pub fn is_ground_cover(&self) -> bool {
		self.flags & flag::GROUND_COVER != 0
	}
}

/// Whether a `ResourceID` is a **ground-cover** type from its id alone —
/// connectors, rubble, tape, slabs, cones, road, land pad, water platform,
/// block, bridge, mines. These store a decorative *variant* in `angle` (not a
/// heading) and carry no orders, so the editor gates those controls (S4.5).
/// The engine sets the `GROUND_COVER` flag on exactly these ids; a decoded
/// save's per-record [`UnitRecord::is_ground_cover`] agrees (cross-checked in
/// tests) — this static form also classifies fresh, save-less placements.
pub fn is_ground_cover_type(unit_type: u16) -> bool {
	matches!(
		unit_type,
		0x0C            // CNCT_4W
		| 0x0D
			..=0x16   // LRGRUBLE..LANDPAD (rubble / tape / slabs / cones / road / land pad)
		| 0x21          // WTRPLTFM
		| 0x26          // BLOCK
		| 0x27          // BRIDGE
		| 0x29 | 0x2A // LANDMINE, SEAMINE
	)
}

/// Whether a `ResourceID` carries a meaningful **connector** adjacency mask
/// (`UnitRecord::connectors`) — the 4-way connector `CNCT_4W`, plus buildings and
/// standalone fixtures that latch onto the connector grid. The engine sets the
/// mask on exactly the units matching `(CONNECTOR_UNIT | BUILDING | STANDALONE)
/// && !GROUND_COVER` (`units_manager.cpp`); this static id form mirrors that for
/// the property panel's connector-editor gating (S4.4) and for fresh, save-less
/// placements. The id set was harvested from every stock save's flags and is
/// re-checked against the flag rule for every decoded unit
/// (`save::serialize` `connector_host_matches_flag_rule`). Note the overlap with
/// [`is_ground_cover_type`]: `CNCT_4W` is both (it stores a variant in `angle`
/// yet still connects), while the ground-cover slabs/tape/cones that happen to
/// carry `BUILDING` are excluded here by the `!GROUND_COVER` half of the rule.
pub fn is_connector_host_type(unit_type: u16) -> bool {
	matches!(
		unit_type,
		0x00..=0x0C   // COMMTWR..CNCT_4W (core buildings, standalone fixtures, the 4-way connector)
		| 0x17..=0x20 // SHIPYARD..TRAINHAL (production + support buildings)
		| 0x22..=0x26 // GUNTURRT..BLOCK (turrets + block)
		| 0x28        // MININGST
	)
}

/// Whether a `ResourceID` is a **2×2 building** structure (`BUILDING` flag and
/// NOT ground cover) — as opposed to the 1×1 standalone fixtures (power gen,
/// radar, dumps, turrets, block), the 1×1 connector `CNCT_4W`, and mobile units,
/// which are all one cell. This is the connector geometry's `unit_size` (the
/// engine's `(flags & BUILDING) ? 2 : 1`, `units_manager.cpp`): where a unit's
/// eight vs. four half-edges live, and how far its east/south neighbours sit.
/// Static so it also sizes fresh, save-less placements; cross-checked against
/// `(BUILDING && !GROUND_COVER)` for every decoded unit
/// (`save::serialize` `building_type_matches_flag`). The 2×2 ground-cover slabs
/// (LRGSLAB/LRGTAPE/LRGCONES also carry `BUILDING`) are deliberately excluded —
/// they're dressing, never connector hosts.
pub fn is_building_type(unit_type: u16) -> bool {
	matches!(
		unit_type,
		0x00 | 0x01 | 0x03 | 0x04 // COMMTWR, POWERSTN, BARRACKS, SHIELDGN
		| 0x09 | 0x0A | 0x0B    // DEPOT, HANGAR, DOCK
		| 0x17
			..=0x20           // SHIPYARD..TRAINHAL (production + support)
		| 0x28 // MININGST
	)
}

/// The order a freshly **deployed** `unit_type` starts on — the engine's
/// per-type switch in `UnitsManager_DeployUnit` (`units_manager.cpp`), which
/// runs at construction for every new unit. Placement mirrors it so a placed
/// unit actually *works*: a `MININGST` produces only on
/// [`ORDER_POWER_ON`](super::orders::ORDER_POWER_ON) (`Cargo_GetNetProduction`
/// gates on the order — HANDOFF Finding 3), a turret fires only on sentry.
/// Everything outside the three special groups keeps the constructor default
/// [`ORDER_AWAIT`](super::orders::ORDER_AWAIT). The paired sub-order `state`
/// comes from [`deploy_state_for`](super::orders::deploy_state_for): the
/// power-on group starts at `ORDER_STATE_INIT` — exactly like the engine's
/// deploy — so the first game tick runs `PowerUp` and the station actually
/// produces; everything else starts settled at `ORDER_STATE_EXECUTING_ORDER`.
pub fn deploy_orders(unit_type: u16) -> u8 {
	use super::orders::{ORDER_AWAIT, ORDER_POWER_OFF, ORDER_POWER_ON, ORDER_SENTRY};
	match unit_type {
		// COMMTWR, HABITAT, MININGST: powered hosts start running.
		0x00 | 0x1C | 0x28 => ORDER_POWER_ON,
		// POWERSTN, POWGEN, RESEARCH: power plants start off (powered on demand).
		0x01 | 0x02 | 0x1D => ORDER_POWER_OFF,
		// RADAR, GUNTURRT..ANTIMSSL, LANDMINE, SEAMINE: defensive fixtures watch.
		0x05 | 0x22..=0x25 | 0x29 | 0x2A => ORDER_SENTRY,
		_ => ORDER_AWAIT,
	}
}

/// The orders a `unit_type` can legitimately **hold at rest** on the map —
/// what the editor offers when placing or editing a unit. Everything else in
/// the `UnitOrderType` enum is either transient runtime state (move/build/
/// attack/load orders carry paths, targets and countdowns a placed unit does
/// not have) or impossible outside a container (`ORDER_IDLE` marks a unit
/// STORED in a depot/hangar — the renderer culls an on-map IDLE unit
/// outright, `unitinfogroup.cpp` `IsRelevant`). Ground cover holds no orders
/// at all (its `angle` is a decorative variant, its record's orders byte is
/// meaningless).
pub fn resting_orders(unit_type: u16) -> &'static [u8] {
	use super::orders::{ORDER_AWAIT, ORDER_DISABLE, ORDER_POWER_OFF, ORDER_POWER_ON, ORDER_SENTRY};
	const MOBILE: &[u8] = &[ORDER_AWAIT, ORDER_SENTRY, ORDER_DISABLE];
	const STATIONARY: &[u8] = &[ORDER_AWAIT, ORDER_SENTRY, ORDER_POWER_ON, ORDER_POWER_OFF, ORDER_DISABLE];
	if is_ground_cover_type(unit_type) {
		&[]
	} else if is_building_type(unit_type) || matches!(unit_type, 0x02 | 0x05..=0x08 | 0x22..=0x25) {
		// Buildings plus the 1x1 fixtures (POWGEN, RADAR, the three dumps, the
		// four turrets): the powered/sentried set.
		STATIONARY
	} else {
		MOBILE
	}
}

/// One if the engine advances this type's sprite one frame past `image_base`
/// at deploy, else zero. `UnitsManager_DeployUnit` calls
/// `DrawSpriteFrame(image_base + 1)` for the four storage buildings — DEPOT,
/// DOCK, HANGAR, BARRACKS (`units_manager.cpp:2338`) — so a placed one must
/// store that frame, not the bare `image_base + angle`.
pub fn deploy_frame_bump(unit_type: u16) -> i16 {
	i16::from(matches!(unit_type, 0x03 | 0x09 | 0x0A | 0x0B))
}

/// `ResourceID` of the 2x2 concrete slab.
pub const LRGSLAB: u16 = 0x11;
/// `ResourceID` of the 1x1 concrete slab.
pub const SMLSLAB: u16 = 0x12;

/// The slab a freshly placed `unit_type` lays under itself, or `None` for a
/// type that needs no foundation.
///
/// The engine deploys one whenever the unit's `REQUIRES_SLAB` flag is set,
/// sized by its `BUILDING` flag — `LRGSLAB` for a 2x2 structure, `SMLSLAB` for
/// a 1x1 fixture (`game_manager.cpp` `GameManager_DeployUnit`, `unitinfo.cpp`
/// `UnitInfo::Deploy`). The editor has no flag word for a *fresh* placement, so
/// the flag's id set is transcribed here from the re-MAX rules' `UsePavement`
/// key — the same source `tools/gen-unit-names.mjs` reads for the display
/// names.
///
/// The two water buildings (`DOCK`, `SHIPYARD`) are the reason this is not
/// simply [`is_building_type`]: they float, so they lay nothing. The engine
/// also skips the slab when the site holds no land at all
/// (`Access_IsAnyLandPresent`); the editor does not enforce placement legality
/// anywhere, so it does not check that either.
pub fn slab_for_type(unit_type: u16) -> Option<u16> {
	match unit_type {
		// Land buildings: COMMTWR, POWERSTN, BARRACKS, SHIELDGN, DEPOT, HANGAR,
		// the production/support block (LIGHTPLT..TRAINHAL), MININGST.
		0x00 | 0x01 | 0x03 | 0x04 | 0x09 | 0x0A | 0x18..=0x20 | 0x28 => Some(LRGSLAB),
		// 1x1 fixtures: POWGEN, RADAR, ADUMP, FDUMP, GOLDSM, and the four turrets.
		0x02 | 0x05..=0x08 | 0x22..=0x25 => Some(SMLSLAB),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The slab table against the engine's own rule: a type that lays one is a
	/// building, it lays the *large* slab exactly when it is a 2x2 structure,
	/// and the two water buildings lay nothing.
	#[test]
	fn the_slab_table_follows_the_building_flag() {
		assert_eq!(slab_for_type(unit_type_id("POWERSTN").unwrap()), Some(LRGSLAB), "a 2x2 building");
		assert_eq!(slab_for_type(unit_type_id("POWGEN").unwrap()), Some(SMLSLAB), "a 1x1 fixture");
		assert_eq!(slab_for_type(unit_type_id("GUNTURRT").unwrap()), Some(SMLSLAB), "a turret");
		assert_eq!(slab_for_type(unit_type_id("DOCK").unwrap()), None, "a water building floats");
		assert_eq!(slab_for_type(unit_type_id("SHIPYARD").unwrap()), None, "so does the shipyard");
		assert_eq!(slab_for_type(unit_type_id("TANK").unwrap()), None, "a mobile unit lays nothing");
		assert_eq!(slab_for_type(unit_type_id("ROAD").unwrap()), None, "and neither does dressing");
		assert_eq!(slab_for_type(LRGSLAB), None, "least of all a slab");

		for ut in 0..UNIT_END as u16 {
			let Some(slab) = slab_for_type(ut) else { continue };
			let name = unit_type_name(ut).expect("a real type");
			assert!(!is_ground_cover_type(ut), "{name} is dressing, it cannot need a foundation");
			// The engine picks the slab off the BUILDING flag, which for a
			// non-ground-cover type is exactly `is_building_type`.
			assert_eq!(slab == LRGSLAB, is_building_type(ut), "{name}'s slab must match its footprint");
		}
	}

	#[test]
	fn names_cover_the_unit_range_in_order() {
		assert_eq!(UNIT_TYPE_NAMES.len(), UNIT_END);
		assert_eq!(unit_type_name(0x00), Some("COMMTWR"));
		assert_eq!(unit_type_name(0x11), Some("LRGSLAB"));
		assert_eq!(unit_type_name(0x28), Some("MININGST"));
		assert_eq!(unit_type_name(0x33), Some("TANK"));
		assert_eq!(unit_type_name(0x3D), Some("ENGINEER"));
		assert_eq!(unit_type_name(0x5C), Some("WALDO"));
		// Ids at/after UNIT_END are not physical unit types.
		assert_eq!(unit_type_name(UNIT_END as u16), None);
		assert_eq!(unit_type_name(0xFFFF), None);
	}

	#[test]
	fn classifier_matches_the_engine_routing() {
		use UnitCategory::*;
		// Ground cover carries both STATIONARY and GROUND_COVER.
		assert_eq!(UnitCategory::from_flags(flag::STATIONARY | flag::GROUND_COVER), GroundCover);
		// A building is STATIONARY without GROUND_COVER.
		assert_eq!(UnitCategory::from_flags(flag::STATIONARY | flag::BUILDING), Stationary);
		// Air beats land/sea; land/sea is the fall-through.
		assert_eq!(UnitCategory::from_flags(flag::MOBILE_AIR_UNIT), MobileAir);
		assert_eq!(UnitCategory::from_flags(flag::MOBILE_LAND_UNIT), MobileLandSea);
		assert_eq!(UnitCategory::from_flags(flag::MOBILE_SEA_UNIT), MobileLandSea);
		// Owner bits must not disturb classification.
		assert_eq!(UnitCategory::from_flags(flag::STATIONARY | flag::GROUND_COVER | flag::HASH_TEAM_RED), GroundCover);
	}

	#[test]
	fn ground_cover_type_classifies_by_id() {
		// The paving + fixtures are ground cover…
		for &t in &[0x0Cu16, 0x0D, 0x11, 0x12, 0x15, 0x16, 0x21, 0x26, 0x27, 0x29, 0x2A] {
			assert!(is_ground_cover_type(t), "{t:#x} is ground cover");
		}
		// …buildings, units and out-of-range ids are not.
		for &t in &[0x00u16, 0x28, 0x33, 0x3D, 0x4C, 0x17, 0xFFFF] {
			assert!(!is_ground_cover_type(t), "{t:#x} is not ground cover");
		}
	}

	#[test]
	fn connector_host_classifies_by_id() {
		// Connector hosts: the 4-way connector, buildings, standalone fixtures.
		for &t in &[
			0x00u16, // COMMTWR
			0x02,    // POWGEN (standalone)
			0x0B,    // DOCK
			0x0C,    // CNCT_4W (also ground cover — still connects)
			0x17,    // SHIPYARD
			0x1F,    // RECCENTR (a building; can host connectors even when none are set)
			0x20,    // TRAINHAL
			0x22,    // GUNTURRT
			0x26,    // BLOCK
			0x28,    // MININGST
		] {
			assert!(is_connector_host_type(t), "{t:#x} hosts connectors");
		}
		// Not hosts: ground-cover paving (even with a BUILDING flag), the land
		// pad, water platform, bridge, mines, mobile units, out-of-range ids.
		for &t in &[
			0x0Du16, // LRGRUBLE
			0x11,    // LRGSLAB (BUILDING flag but GROUND_COVER → excluded)
			0x15,    // ROAD
			0x16,    // LANDPAD
			0x21,    // WTRPLTFM
			0x27,    // BRIDGE
			0x29,    // LANDMINE
			0x33,    // TANK
			0x4C,    // FIGHTER
			0xFFFF,
		] {
			assert!(!is_connector_host_type(t), "{t:#x} does not host connectors");
		}
	}

	/// The baked display-name/player-roster tables (unit_names.rs, generated
	/// from the re-MAX rules inis) must stay consistent with the canonical
	/// tag table - the generator filters on it, this pins the contract.
	#[test]
	fn baked_unit_names_agree_with_the_canonical_tags() {
		use super::super::unit_names::*;
		for (tag, name) in UNIT_DISPLAY_NAMES {
			assert!(super::super::unit_type_id(tag).is_some(), "{tag} is a save unit type");
			assert!(!name.is_empty() && name.is_ascii(), "{tag}: usable ASCII name");
		}
		for tag in PLAYER_UNIT_TAGS {
			assert!(super::super::unit_type_id(tag).is_some(), "{tag} is a save unit type");
		}
		assert_eq!(unit_display_name("TANK"), Some("Tank"));
		assert_eq!(unit_display_name("COMMTWR"), Some("Gold Refinery"), "the proper in-game name, not the tag");
		assert_eq!(unit_display_name("LRGRUBLE"), None, "decoration carries no name");
		// The roster is the buildable-and-controllable set: combat/support
		// units and mines in; FX, rubble, aliens and the factory-less
		// Master Builder out.
		for &t in &[0x33u16, 0x34, 0x29, 0x2A, 0x00, 0x3D] {
			assert!(is_player_unit_type(t), "{t:#x} is a player unit");
		}
		for &t in &[0x30u16, 0x0D, 0x2B, 0x51, 0x54, 0x5C, 0xFFFF] {
			assert!(!is_player_unit_type(t), "{t:#x} is not a player unit");
		}
	}

	#[test]
	fn building_type_sizes_by_id() {
		// 2×2 building structures.
		for &t in &[0x00u16, 0x01, 0x03, 0x04, 0x09, 0x0A, 0x0B, 0x17, 0x1F, 0x20, 0x28] {
			assert!(is_building_type(t), "{t:#x} is a 2x2 building");
		}
		// 1×1: standalone fixtures, the connector, mines, mobile units, and the
		// 2×2 ground-cover slabs (BUILDING flag but dressing, not a structure).
		for &t in &[
			0x02u16, // POWGEN (standalone 1×1)
			0x05,    // RADAR
			0x0C,    // CNCT_4W (connector 1×1)
			0x11,    // LRGSLAB (2×2 but ground cover — not a building here)
			0x22,    // GUNTURRT (turret 1×1)
			0x26,    // BLOCK
			0x33,    // TANK
			0xFFFF,
		] {
			assert!(!is_building_type(t), "{t:#x} is not a 2x2 building");
		}
	}
}
