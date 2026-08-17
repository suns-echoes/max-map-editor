//! Mining-station production: derivation and event-driven re-derivation — the
//! HANDOFF 2026-08-02 Finding 3 fix.
//!
//! A `MININGST` stores seven production bytes (right after `build_time` in its
//! body): the current allocation `total/raw/fuel/gold` plus the per-resource
//! ceilings `raw/gold/fuel` max. The engine writes them **once**, at deploy,
//! via `UnitsManager_SetInitialMining` (`units_manager.cpp`): the maxes are
//! summed off the cargo map under the station's 2x2 footprint
//! (`Survey_GetTotalResourcesInArea`, radius 1), and the allocation is a greedy
//! split of the 16-unit processing capacity — fuel first up to the power
//! generator's consumption rate (2), then raw, then the rest of the fuel, then
//! gold. After that the fields are player state (the mining menu re-allocates
//! within the maxes) and `FileLoad` reads them verbatim — **nothing at load
//! time re-derives them from the map**.
//!
//! Two editor consequences, both handled here:
//! - a **placed** station must run the deploy derivation or it produces
//!   nothing ([`super::export::add_unit`] calls [`set_initial_mining`]);
//! - **repainting resources** (or moving a station) leaves the stored
//!   production describing ground that no longer exists, so the export pass
//!   ([`repair_mining`]) re-runs the deploy derivation for exactly the
//!   stations whose footprint resources an edit changed.
//!
//! **There is no invariant to check** — unlike the complex pass, *nothing*
//! here is a load requirement. Stock scenarios ship hand-authored drift the
//! engine happily tolerates (a 63-save corpus scan found stored maxes above
//! and below the derived values, and even allocations exceeding their max —
//! e.g. SAVE5.MPS's four stations all store `raw 12/fuel 8` over ground that
//! derives `raw 4/gold 12/fuel 4`), and allocations are player choices. So the
//! repair is purely **event-driven**: a station whose derived footprint triple
//! is unchanged between the pristine and edited save is never touched, drift
//! and player allocations included.

use super::complexes::under_construction;
use super::types::{SaveFile, SaveObject};

/// `ResourceID` of the mining station — the one type with mining production.
pub const MININGST: u16 = 0x28;

/// `SURFACE_TYPE_AIR` (`enums.hpp`) — the one surface the resource sum skips.
/// (`Access_GetModifiedSurfaceType` only ever *upgrades* water/coast under a
/// platform or bridge; it can never produce AIR, so the save's own surface map
/// is exact here.)
const SURFACE_AIR: u8 = 0x8;

/// The per-resource ceilings under a station whose top-left cell is `(x, y)` —
/// `(raw, gold, fuel)`, the mirror of `Survey_GetTotalResourcesInArea(x, y, 1,
/// ..., true, team)` (`survey.cpp`): every cell of the 2x2 footprint (clamped
/// at the map edge) that holds a nonzero cargo value on a non-AIR surface
/// contributes `min(amount, 16)` to its material's sum — gold first by flag,
/// then fuel, else raw. `mode = true` makes the team mask all-ones, so the
/// result is team-independent. (The engine also caps each sum at 255; four
/// cells of 16 max out at 64, so the cap can never engage.)
pub fn derive_mining(cargo: &[u16], surface: &[u8], dims: (u16, u16), x: i32, y: i32) -> (u8, u8, u8) {
	let (w, h) = (dims.0 as i32, dims.1 as i32);
	let (mut raw, mut gold, mut fuel) = (0u8, 0u8, 0u8);
	for i in x.max(0)..=(x + 1).min(w - 1) {
		for j in y.max(0)..=(y + 1).min(h - 1) {
			let idx = (j * w + i) as usize;
			let value = cargo[idx];
			if value != 0 && surface[idx] != SURFACE_AIR {
				let amount = (value & super::cargo::CARGO_AMOUNT_MASK).min(16) as u8;
				if value & super::cargo::CARGO_GOLD != 0 {
					gold += amount;
				} else if value & super::cargo::CARGO_FUEL != 0 {
					fuel += amount;
				} else {
					raw += amount;
				}
			}
		}
	}
	(raw, gold, fuel)
}

/// The deploy-time allocation for ceilings `(raw, gold, fuel)` — the greedy
/// split from `UnitsManager_SetInitialMining`, returned in on-disk byte order
/// `[total, raw, fuel, gold]`: fuel first up to the power generator's
/// consumption rate (`Cargo_GetFuelConsumptionRate(POWGEN)` = 2), then raw
/// into the remaining 16-unit capacity, then the rest of the fuel, then gold.
pub fn initial_allocation(raw: u8, gold: u8, fuel: u8) -> [u8; 4] {
	let fuel_mining = (fuel as i32).min(2);
	let mut free = 16 - fuel_mining;
	let raw_mining = (raw as i32).min(free);
	free -= raw_mining;
	let extra = (fuel as i32 - fuel_mining).min(free);
	let fuel_mining = fuel_mining + extra;
	free -= extra;
	let gold_mining = (gold as i32).min(free);
	[(raw_mining + fuel_mining + gold_mining) as u8, raw_mining as u8, fuel_mining as u8, gold_mining as u8]
}

/// The seven on-disk production bytes for ceilings `(raw, gold, fuel)`:
/// `[total, raw, fuel, gold]` allocation ([`initial_allocation`]) followed by
/// the `[raw, gold, fuel]` maxes — what sits at `build_time + 1` in the body.
/// Also the shape the from-scratch synthesizer's `SynthUnit.mining` takes.
pub fn mining_bytes(raw: u8, gold: u8, fuel: u8) -> [u8; 7] {
	let [total, raw_m, fuel_m, gold_m] = initial_allocation(raw, gold, fuel);
	[total, raw_m, fuel_m, gold_m, raw, gold, fuel]
}

/// Run the deploy derivation for the station at object `slot`: ceilings off
/// the cargo map under its footprint, greedy initial allocation, written into
/// the retained body ([`mining_bytes`]). The typed record is untouched — the
/// production fields are not modeled on `UnitRecord`; they live in the opaque
/// prefix the serializer emits verbatim. A no-op for a slot that is not a
/// mining station (or has no captured layout).
pub fn set_initial_mining(save: &mut SaveFile, slot: usize) {
	let Some(SaveObject::Unit(u)) = save.objects.get(slot) else { return };
	if u.unit_type != MININGST {
		return;
	}
	let (x, y) = (u.grid_x as i32, u.grid_y as i32);
	let (raw, gold, fuel) = derive_mining(&save.cargo_map, &save.surface_map, (save.width, save.height), x, y);
	let meta = &mut save.object_meta[slot];
	let Some(layout) = meta.unit_layout.as_ref() else { return };
	let off = layout.build_time + 1;
	meta.body_raw[off..off + 7].copy_from_slice(&mining_bytes(raw, gold, fuel));
}

/// Re-derive the stored production of every completed mining station whose
/// footprint resources an edit changed — the export-time half of the Finding 3
/// fix, run by `Project::export_save` / `export_onto_base` after the edit
/// passes with `pristine` = the save as opened (the base save there).
///
/// Event detection, per station in `save`: derive the footprint triple from
/// the **edited** cargo map at its current cell and from the **pristine** map
/// at its pristine cell (same spatial-hash id, station then too). Equal
/// triples mean no edit touched its ground — the station keeps its bytes,
/// preserving player allocations and DOS-era authored drift alike (see the
/// module doc). A differing triple (repaint under it, or a move onto different
/// ground) or a station `pristine` lacks (a placement — already derived by
/// `add_unit`, so writing the same bytes again is a no-op) gets
/// [`set_initial_mining`]. A station still under construction is skipped: the
/// engine derives at completion. Returns one ASCII description per station
/// whose bytes actually changed; empty for an untouched save.
pub fn repair_mining(save: &mut SaveFile, pristine: &SaveFile) -> Vec<String> {
	let mut fixed = Vec::new();
	for slot in 0..save.objects.len() {
		let Some(SaveObject::Unit(u)) = save.objects.get(slot) else { continue };
		if u.unit_type != MININGST || under_construction(u) {
			continue;
		}
		let (id, x, y) = (u.id, u.grid_x as i32, u.grid_y as i32);
		let new = derive_mining(&save.cargo_map, &save.surface_map, (save.width, save.height), x, y);
		if let Some(p) = pristine.units().find(|p| p.id == id && p.unit_type == MININGST) {
			let old = derive_mining(
				&pristine.cargo_map,
				&pristine.surface_map,
				(pristine.width, pristine.height),
				p.grid_x as i32,
				p.grid_y as i32,
			);
			if old == new {
				continue;
			}
		}
		let (raw, gold, fuel) = new;
		let target = mining_bytes(raw, gold, fuel);
		let layout = save.object_meta[slot].unit_layout.clone();
		let Some(layout) = layout else { continue };
		let off = layout.build_time + 1;
		if save.object_meta[slot].body_raw[off..off + 7] == target {
			continue;
		}
		save.object_meta[slot].body_raw[off..off + 7].copy_from_slice(&target);
		fixed.push(format!(
			"mining station id {id} at {x},{y}: production re-derived (raw {raw} gold {gold} fuel {fuel})"
		));
	}
	fixed
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::save::cargo::{CARGO_FUEL, CARGO_GOLD, CARGO_RAW};
	use crate::save::orders::{ORDER_BUILD, ORDER_POWER_ON, ORDER_STATE_BUILD_CLEARING};
	use crate::save::{add_unit, read_save, read_save_bytes, write_save};

	/// Locate SAVE10 + its pristine world, or `None` to skip when absent.
	fn save10() -> Option<SaveFile> {
		let home = std::env::var_os("HOME")?;
		let save_path = std::path::Path::new(&home).join("MAX/SAVE10.DTA");
		let wrl_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals/SNOW_1.WRL");
		if !save_path.is_file() || !wrl_path.is_file() {
			return None;
		}
		let wrl = crate::wrl::read_wrl_header(&wrl_path).ok()?;
		read_save(&save_path, (wrl.width, wrl.height)).ok()
	}

	/// The seven production bytes of the unit with spatial-hash `id`.
	fn stored(save: &SaveFile, id: u16) -> [u8; 7] {
		let slot = save.objects.iter().position(|o| matches!(o, SaveObject::Unit(u) if u.id == id)).unwrap();
		let l = save.object_meta[slot].unit_layout.as_ref().unwrap();
		save.object_meta[slot].body_raw[l.build_time + 1..l.build_time + 8].try_into().unwrap()
	}

	/// The greedy split, against hand-derived vectors including two stock
	/// ground truths (SAVE3.SCE id 16's footprint, SAVE5.MPS's gold field).
	#[test]
	fn allocation_mirrors_set_initial_mining() {
		// Empty ground: nothing to mine.
		assert_eq!(initial_allocation(0, 0, 0), [0, 0, 0, 0]);
		// Fuel first (up to 2), then raw, then the rest of the fuel, then gold.
		assert_eq!(initial_allocation(12, 0, 8), [16, 12, 4, 0], "raw-rich: fuel tops past its floor");
		assert_eq!(initial_allocation(4, 11, 2), [16, 4, 2, 10], "gold takes what capacity remains");
		assert_eq!(initial_allocation(4, 12, 4), [16, 4, 4, 8], "SAVE5.MPS ground: gold-heavy split");
		assert_eq!(initial_allocation(16, 16, 16), [16, 14, 2, 0], "saturated: raw fills after the fuel floor");
		assert_eq!(initial_allocation(0, 0, 30), [16, 0, 16, 0], "fuel-only ground spills fuel to capacity");
		assert_eq!(initial_allocation(3, 0, 2), [5, 3, 2, 0], "sparse ground leaves capacity idle");
	}

	/// The footprint sum: 2x2 reach, per-cell 16 cap, material flags, AIR skip,
	/// map-edge clamp.
	#[test]
	fn derive_sums_the_footprint_like_the_survey() {
		let w = 4u16;
		let mut cargo = vec![0u16; 16];
		let mut surface = vec![0x1u8; 16]; // land
		cargo[5] = CARGO_RAW | 12; // (1,1)
		cargo[6] = CARGO_GOLD | 31; // (2,1) - amount caps at 16
		cargo[9] = CARGO_FUEL | 5; // (1,2)
		cargo[10] = CARGO_RAW | 7; // (2,2)
		assert_eq!(derive_mining(&cargo, &surface, (w, 4), 1, 1), (19, 16, 5), "all four cells, gold capped");
		// An AIR cell contributes nothing.
		surface[6] = SURFACE_AIR;
		assert_eq!(derive_mining(&cargo, &surface, (w, 4), 1, 1), (19, 0, 5), "AIR cell skipped");
		// The footprint clamps at the map edge (a 2x2 read at the corner).
		assert_eq!(derive_mining(&cargo, &surface, (w, 4), 2, 2), (7, 0, 0), "edge clamp: only in-map cells");
		// A surveyed-only cell (flags, zero amount) adds nothing.
		cargo[5] = 0x2000 | CARGO_RAW;
		assert_eq!(derive_mining(&cargo, &surface, (w, 4), 1, 1), (7, 0, 5), "zero amount contributes zero");
	}

	/// A placed mining station derives its production from the ground it lands
	/// on — not the template's — and starts powered on (the deploy order), so
	/// it actually produces in-game (Finding 3). Verified through a serialize →
	/// decode round-trip.
	#[test]
	fn a_placed_station_derives_its_own_production() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("a_placed_station_derives_its_own_production: SAVE10 fixture absent");
			return;
		};
		// The template (SAVE10's working station) carries live production bytes
		// that must NOT be inherited.
		let tmpl = save.units().find(|u| u.unit_type == MININGST).expect("SAVE10 mining station");
		assert_ne!(stored(&save, tmpl.id), [0; 7], "template carries live production to (not) inherit");

		// Paint a known field at an empty spot, then place a station on it.
		let mut patched = save.clone();
		let w = patched.width as usize;
		patched.cargo_map[40 * w + 40] = CARGO_RAW | 9;
		patched.cargo_map[40 * w + 41] = CARGO_GOLD | 4;
		patched.cargo_map[41 * w + 41] = CARGO_FUEL | 6;
		let id = add_unit(&mut patched, MININGST, 0, 40, 40, None).expect("the tail follows").expect("template exists");

		let out = read_save_bytes(&write_save(&patched).unwrap(), (save.width, save.height)).unwrap();
		let placed = out.units().find(|u| u.id == id).expect("placed station present");
		assert_eq!(placed.orders, ORDER_POWER_ON, "a deployed station starts powered on");
		assert_eq!(
			placed.state,
			crate::save::ORDER_STATE_INIT,
			"deploy state INIT - the engine's first tick runs PowerUp only from it"
		);
		assert_eq!(placed.prior_orders, ORDER_POWER_ON, "prior mirrors");
		// Ceilings raw 9 / gold 4 / fuel 6; greedy: fuel 2, raw 9, fuel +4, gold 1.
		assert_eq!(stored(&out, id), [16, 9, 6, 1, 9, 4, 6], "derived + greedy split, not the template's bytes");

		// A station placed on barren ground produces nothing - but is set up to.
		let mut barren = save.clone();
		let id2 = add_unit(&mut barren, MININGST, 0, 60, 60, None).expect("the tail follows").expect("template exists");
		let out2 = read_save_bytes(&write_save(&barren).unwrap(), (save.width, save.height)).unwrap();
		let d = derive_mining(&out2.cargo_map, &out2.surface_map, (out2.width, out2.height), 60, 60);
		assert_eq!(stored(&out2, id2), mining_bytes(d.0, d.1, d.2), "derived from its own (possibly bare) ground");
	}

	/// Repainting the ground under a station re-derives its production on
	/// repair; painting anywhere else leaves every station byte-identical.
	#[test]
	fn repair_rederives_exactly_the_repainted_station() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("repair_rederives_exactly_the_repainted_station: SAVE10 fixture absent");
			return;
		};
		let mine = save.units().find(|u| u.unit_type == MININGST).expect("SAVE10 mining station");
		let (id, x, y) = (mine.id, mine.grid_x as usize, mine.grid_y as usize);
		let w = save.width as usize;

		// A paint far from any station: repair touches nothing.
		let mut edited = save.clone();
		edited.cargo_map[70 * w + 70] = CARGO_GOLD | 10;
		assert!(repair_mining(&mut edited, &save).is_empty(), "no station's ground changed");
		assert_eq!(stored(&edited, id), stored(&save, id), "the station's bytes are untouched");

		// A paint under the station's footprint: exactly that station re-derives.
		let mut under = save.clone();
		under.cargo_map[y * w + x] = crate::save::cargo::cargo_surveyed(CARGO_GOLD | 13);
		let fixed = repair_mining(&mut under, &save);
		assert_eq!(fixed.len(), 1, "one station re-derived: {fixed:?}");
		let d = derive_mining(&under.cargo_map, &under.surface_map, (under.width, under.height), x as i32, y as i32);
		assert_eq!(stored(&under, id), mining_bytes(d.0, d.1, d.2), "bytes re-derived from the new ground");
		assert!(d.1 >= 13, "the painted gold is in the ceiling");
		// Idempotent: a second repair against the new state changes nothing.
		let again = under.clone();
		assert!(repair_mining(&mut under, &again).is_empty(), "second repair is a no-op");
	}

	/// A station still under construction is left alone even when its ground
	/// changes — the engine runs the derivation itself at completion.
	#[test]
	fn an_under_construction_station_is_skipped() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("an_under_construction_station_is_skipped: SAVE10 fixture absent");
			return;
		};
		let mine = save.units().find(|u| u.unit_type == MININGST).expect("SAVE10 mining station");
		let (id, x, y) = (mine.id, mine.grid_x as usize, mine.grid_y as usize);
		let mut forged = save.clone();
		let slot = forged.objects.iter().position(|o| matches!(o, SaveObject::Unit(u) if u.id == id)).unwrap();
		if let SaveObject::Unit(u) = &mut forged.objects[slot] {
			u.orders = ORDER_BUILD;
			u.state = ORDER_STATE_BUILD_CLEARING;
		}
		let before = stored(&forged, id);
		let pristine = forged.clone();
		let w = forged.width as usize;
		forged.cargo_map[y * w + x] = CARGO_RAW | 16;
		assert!(repair_mining(&mut forged, &pristine).is_empty(), "mid-construction: engine derives at completion");
		assert_eq!(stored(&forged, id), before, "bytes untouched");
	}

	/// No stock save is touched: with pristine == current the event never
	/// fires, so hand-authored drift and player allocations alike survive
	/// byte-identically — the export-identity guarantee.
	#[test]
	fn repair_is_a_noop_on_every_stock_save() {
		let Some(home) = std::env::var_os("HOME") else { return };
		let max_dir = std::path::Path::new(&home).join("MAX");
		let originals = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals");
		if !max_dir.is_dir() || !originals.is_dir() {
			crate::testutil::skip_fixture("repair_is_a_noop_on_every_stock_save: fixtures not found");
			return;
		}
		let mut checked = 0;
		for entry in std::fs::read_dir(&max_dir).into_iter().flatten().flatten() {
			let path = entry.path();
			let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_uppercase);
			if !matches!(ext.as_deref(), Some("DTA" | "CAM" | "SCE" | "TRA" | "MPS" | "DMO")) {
				continue;
			}
			let Ok(header) = crate::save::read_save_header(&path) else { continue };
			let Some(world_file) = header.world_file else { continue };
			let Ok(wrl) = crate::wrl::read_wrl_header(&originals.join(world_file)) else { continue };
			let Ok(save) = read_save(&path, (wrl.width, wrl.height)) else { continue };
			let mut clone = save.clone();
			let fixed = repair_mining(&mut clone, &save);
			assert!(fixed.is_empty(), "{}: unexpected re-derivation: {fixed:?}", path.display());
			assert!(
				write_save(&clone).unwrap() == write_save(&save).unwrap(),
				"repair is a no-op on {}",
				path.display()
			);
			checked += 1;
		}
		assert!(checked > 0, "no ~/MAX saves were checked");
		eprintln!("{checked} ~/MAX saves: mining repair is a no-op");
	}
}
