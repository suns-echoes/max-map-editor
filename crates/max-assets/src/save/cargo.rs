//! Encoding of the save's per-cell **cargo map** (`ResourceManager_CargoMap`,
//! `resource_allocator.cpp`): one `u16` per cell packing a surveyed resource.
//!
//! Layout (from the engine): the low five bits (`& 0x1F`) are the **amount**
//! (0-31); bits 5-7 are the mutually-exclusive **material** flag (`CARGO_FUEL`
//! `0x20`, `CARGO_GOLD` `0x40`, `CARGO_MATERIALS` `0x80`); the high byte holds
//! per-team **survey** flags (`survey.cpp` ORs in `hash_team_id`) which the editor
//! preserves verbatim. A cell with no material flag holds no resource.

/// Amount mask — the resource quantity lives in bits 0-4 (max 31).
pub const CARGO_AMOUNT_MASK: u16 = 0x1F;
/// Fuel material flag (`enums.hpp` `CARGO_FUEL`).
pub const CARGO_FUEL: u16 = 0x20;
/// Gold material flag (`enums.hpp` `CARGO_GOLD`).
pub const CARGO_GOLD: u16 = 0x40;
/// Raw-materials flag (`enums.hpp` `CARGO_MATERIALS`).
pub const CARGO_RAW: u16 = 0x80;
/// The bits the editor rewrites (material + amount); the rest (survey flags) are
/// preserved on every edit.
pub const CARGO_LOW_MASK: u16 = 0x00FF;

/// The per-team survey flags for the four playable teams (`enums.hpp`
/// `HASH_TEAM_*`: Gray `0x400`, Blue `0x800`, Green `0x1000`, Red `0x2000`).
/// `survey.cpp` gates a cell's resource visibility + minability on the querying
/// team's bit, so a resource with none set is inert in play. The editor marks a
/// painted resource surveyed by all players ([`cargo_surveyed`]) so it's usable.
pub const CARGO_SURVEY_ALL: u16 = 0x400 | 0x800 | 0x1000 | 0x2000;

/// The same cargo value marked surveyed by all four playable teams (S5.5) — what
/// the editor stores when it paints a resource, so the placement is actually
/// usable in-game. A no-material (empty) value is returned unchanged.
pub fn cargo_surveyed(value: u16) -> u16 {
	if cargo_material(value).is_some() { value | CARGO_SURVEY_ALL } else { value }
}

/// A surveyable resource material. A cargo cell holds at most one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CargoMaterial {
	/// Raw materials (`CARGO_MATERIALS`) — the mining feedstock.
	Raw,
	Fuel,
	Gold,
}

impl CargoMaterial {
	/// The material flag bit for this type.
	pub fn flag(self) -> u16 {
		match self {
			CargoMaterial::Raw => CARGO_RAW,
			CargoMaterial::Fuel => CARGO_FUEL,
			CargoMaterial::Gold => CARGO_GOLD,
		}
	}

	/// The lowercase command/UI slug (`raw` / `fuel` / `gold`).
	pub fn slug(self) -> &'static str {
		match self {
			CargoMaterial::Raw => "raw",
			CargoMaterial::Fuel => "fuel",
			CargoMaterial::Gold => "gold",
		}
	}

	/// Parse a material slug (`raw`/`materials`, `fuel`, `gold`), case-insensitive.
	pub fn from_slug(s: &str) -> Option<Self> {
		match s.to_ascii_lowercase().as_str() {
			"raw" | "materials" => Some(CargoMaterial::Raw),
			"fuel" => Some(CargoMaterial::Fuel),
			"gold" => Some(CargoMaterial::Gold),
			_ => None,
		}
	}

	/// Every material, in display order.
	pub const ALL: [CargoMaterial; 3] = [CargoMaterial::Raw, CargoMaterial::Fuel, CargoMaterial::Gold];
}

/// The material a cargo value holds, or `None` when it carries no resource. The
/// flags are mutually exclusive in stock data; if several are set, raw wins, then
/// gold, then fuel (matching the engine's precedence).
pub fn cargo_material(value: u16) -> Option<CargoMaterial> {
	if value & CARGO_RAW != 0 {
		Some(CargoMaterial::Raw)
	} else if value & CARGO_GOLD != 0 {
		Some(CargoMaterial::Gold)
	} else if value & CARGO_FUEL != 0 {
		Some(CargoMaterial::Fuel)
	} else {
		None
	}
}

/// The resource amount (0-31) a cargo value carries, regardless of material.
pub fn cargo_amount(value: u16) -> u16 {
	value & CARGO_AMOUNT_MASK
}

/// Compose a new cargo value: `material` + `amount` (clamped to 0-31), preserving
/// the survey/team flags in the high byte of `old`. `None` clears the resource
/// (material + amount both zero) while keeping those survey bits.
pub fn cargo_compose(old: u16, material: Option<CargoMaterial>, amount: u16) -> u16 {
	let survey = old & !CARGO_LOW_MASK;
	match material {
		Some(m) => survey | m.flag() | amount.min(CARGO_AMOUNT_MASK),
		None => survey,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn decompose_reads_material_and_amount() {
		assert_eq!(cargo_material(CARGO_RAW | 12), Some(CargoMaterial::Raw));
		assert_eq!(cargo_material(CARGO_FUEL | 5), Some(CargoMaterial::Fuel));
		assert_eq!(cargo_material(CARGO_GOLD | 31), Some(CargoMaterial::Gold));
		assert_eq!(cargo_material(0), None, "no flag -> no resource");
		assert_eq!(cargo_amount(CARGO_RAW | 12), 12);
		assert_eq!(cargo_amount(CARGO_GOLD | 31), 31);
		// The amount reads only the low five bits (a survey flag doesn't leak in).
		assert_eq!(cargo_amount(0x8000 | CARGO_FUEL | 7), 7);
	}

	#[test]
	fn compose_sets_low_byte_and_preserves_survey() {
		// A cell surveyed by some team (high byte set) keeps those bits on edit.
		let surveyed = 0x2000 | CARGO_FUEL | 3;
		let out = cargo_compose(surveyed, Some(CargoMaterial::Gold), 20);
		assert_eq!(out & !CARGO_LOW_MASK, 0x2000, "survey flags preserved");
		assert_eq!(cargo_material(out), Some(CargoMaterial::Gold));
		assert_eq!(cargo_amount(out), 20);
		// Clearing keeps the survey bits but drops the material + amount.
		let cleared = cargo_compose(out, None, 9);
		assert_eq!(cleared, 0x2000, "clear leaves only the survey flags");
		assert_eq!(cargo_material(cleared), None);
		// The amount clamps to five bits.
		assert_eq!(cargo_amount(cargo_compose(0, Some(CargoMaterial::Raw), 99)), 31, "amount clamps to 31");
	}

	#[test]
	fn surveyed_sets_player_bits_on_resources_only() {
		// A painted resource gains all four player survey bits (usable in-game).
		let raw = cargo_compose(0, Some(CargoMaterial::Raw), 12);
		let s = cargo_surveyed(raw);
		assert_eq!(s & CARGO_SURVEY_ALL, CARGO_SURVEY_ALL, "all player bits set");
		assert_eq!((cargo_material(s), cargo_amount(s)), (Some(CargoMaterial::Raw), 12), "material/amount intact");
		// An empty cell stays empty (no phantom survey of nothing).
		assert_eq!(cargo_surveyed(0), 0, "no resource -> unchanged");
		assert_eq!(cargo_surveyed(0x8000), 0x8000, "a bare survey bit with no material is left alone");
	}

	#[test]
	fn slug_round_trips() {
		for m in CargoMaterial::ALL {
			assert_eq!(CargoMaterial::from_slug(m.slug()), Some(m));
		}
		assert_eq!(CargoMaterial::from_slug("materials"), Some(CargoMaterial::Raw));
		assert_eq!(CargoMaterial::from_slug("MATERIALS"), Some(CargoMaterial::Raw));
		assert_eq!(CargoMaterial::from_slug("plutonium"), None);
	}
}
