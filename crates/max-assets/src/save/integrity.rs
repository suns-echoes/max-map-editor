//! Save-data correctness pass — detect and repair the transient runtime state a
//! placed/cloned unit must not inherit from its template (`save-editor-bug.md`).
//!
//! When the exporter adds a unit it clones a same-type unit already in the save
//! as a body template ([`super::export::add_unit`]). That template is a real
//! in-game unit, so its body carries **per-instance runtime state** — the current
//! animation frame (`image_index`), the sub-order `state`, a build countdown
//! (`build_time`), a movement target (`move_to`), `moved`. Copied onto a
//! freshly-placed unit those are nonsensical: e.g. a constructor cloned mid-build
//! keeps `state = BUILD_IN_PROGRESS` and a build-animation `image_index`, which the
//! engine's deploy animation (`image_index + 16`) can run off the end of the
//! sprite sheet → an out-of-bounds read / crash on load-and-play.
//!
//! This module is the single source of the fix, used two ways:
//! - [`reset_transient_prefix`] normalizes a unit body to a clean *idle* runtime
//!   state; [`super::export::add_unit`] calls it on every placement (the choke
//!   point, so a placed unit is idle-valid regardless of the template).
//! - [`check_transient_state`] / [`repair_transient_state`] are the backstop: they
//!   find (and, on export, fix) any unit already carrying the impossible
//!   idle-plus-in-progress state — e.g. a save an *older* editor corrupted before
//!   this fix, re-opened here.
//!
//! The reset only rewrites fixed-width scalars inside a unit's body; it never
//! touches the object graph, a unit's grid position, or the spatial hashes, so it
//! needs no re-key and leaves an already-idle unit byte-identical.

use super::orders::{ORDER_AWAIT, ORDER_STATE_EXECUTING_ORDER};
use super::types::{SaveFile, SaveFormat, SaveObject, UnitBodyLayout};

/// `UnitOrderStateType::ORDER_STATE_INIT` (`enums.hpp`) — the freshly-constructed
/// sub-order state, before any order runs. Together with
/// [`ORDER_STATE_EXECUTING_ORDER`](super::orders::ORDER_STATE_EXECUTING_ORDER) it
/// forms the set of states an idle (`AWAIT`) unit may legitimately carry.
const ORDER_STATE_INIT: u8 = 0x00;

// Byte offsets of the fields the reset touches within the 8×`i16` image block
// ([`UnitBodyLayout::image_block`]), in on-disk field order.
const IMAGE_BASE: usize = 2; // image_base
const TURRET_IMAGE_BASE: usize = 4; // turret_image_base
const IMAGE_INDEX: usize = 10; // image_index
const TURRET_IMAGE_INDEX: usize = 12; // turret_image_index

fn read_i16(body: &[u8], off: usize) -> i16 {
	i16::from_le_bytes([body[off], body[off + 1]])
}

fn write_i16(body: &mut [u8], off: usize, v: i16) {
	body[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

/// Normalize a unit body's **transient per-instance runtime state** to a clean,
/// idle-valid configuration, in place, using the field offsets captured at decode
/// ([`UnitBodyLayout`]). `v71` selects the format-specific field widths.
///
/// Resets exactly the fields that make an inherited template body invalid on a
/// freshly-placed unit — re-deriving the display frame the way the engine does on
/// deploy (`unitinfo.cpp` `image_index = image_base + angle`) and forcing the
/// order/build/move state to idle:
/// - `image_index` → `image_base + angle`; `turret_image_index` → `turret_image_base + angle`
/// - `state` → `EXECUTING_ORDER`; `prior_orders` → `orders`; `prior_state` → `EXECUTING_ORDER`
/// - `move_to` (and `V71`'s `fire_on`) grid target → `(0, 0)`, the "no target"
///   sentinel real idle units carry (and, in `V70`, what zeroes the derived
///   `transfer_cargo`/`stealth_dice_roll`)
/// - `build_time`, `moved`, and the recoil/disable countdown → 0
///
/// `orders`, `angle`, and the grid position are read from the body and left as-is
/// — the caller sets those (a placement writes `orders = AWAIT`, `angle = 0`); a
/// repair keeps whatever idle order the unit already carries.
pub fn reset_transient_prefix(body: &mut [u8], layout: &UnitBodyLayout, v71: bool) {
	let angle = body[layout.angle] as i16;
	let orders = body[layout.orders];

	// Display frame: re-derive it from the placed angle (`unitinfo.cpp:279-280`)
	// rather than inherit the template's animation frame.
	let image_base = read_i16(body, layout.image_block + IMAGE_BASE);
	let turret_image_base = read_i16(body, layout.image_block + TURRET_IMAGE_BASE);
	write_i16(body, layout.image_block + IMAGE_INDEX, image_base.wrapping_add(angle));
	write_i16(body, layout.image_block + TURRET_IMAGE_INDEX, turret_image_base.wrapping_add(angle));

	// Sub-order state: idle and self-consistent. `state`/`prior_orders`/`prior_state`
	// are the three bytes right after `orders`.
	body[layout.orders + 1] = ORDER_STATE_EXECUTING_ORDER; // state
	body[layout.orders + 2] = orders; // prior_orders
	body[layout.orders + 3] = ORDER_STATE_EXECUTING_ORDER; // prior_state

	// Movement/fire target → the "no target" sentinel `(0, 0)` real idle units store
	// (a placed unit isn't heading anywhere), not the template's target cell.
	write_i16(body, layout.move_to, 0);
	write_i16(body, layout.move_to + 2, 0);
	if v71 {
		write_i16(body, layout.move_to + 4, 0); // fire_on_grid_x
		write_i16(body, layout.move_to + 6, 0); // fire_on_grid_y
	}

	body[layout.build_time] = 0;
	body[layout.moved] = 0;
	// Recoil / disable countdown → 0. V70 packs both into one byte (`layout.disabled`);
	// V71 has a dedicated `firing_recoil_frames` byte just before it.
	body[layout.disabled] = 0;
	if v71 && layout.disabled > 0 {
		body[layout.disabled - 1] = 0; // firing_recoil_frames
	}
}

/// Reset a **placed** unit's per-team visibility and display scale to the
/// engine's deploy values (`InitStealthStatus`, `unitinfo.cpp:2763`):
/// `visible_to_team` = own team only, `spotted_by_team` all clear,
/// `scaler_adjust` = 0.
///
/// Cloned verbatim, these three are lethal: the renderer draws a unit only
/// when `visible_to_team[owner's team]` is set (`unitinfogroup.cpp:59`) and
/// own-team visibility is written **only** at construction — never recomputed
/// on load — so a body cloned from another team's unit is invisible to its
/// owner forever. A template caught mid-expand (or stored in a depot) carries
/// `scaler_adjust = 4`, drawn at 1/32 scale or culled outright, and nothing
/// unwinds it once the placement overwrites `orders`.
///
/// The three blocks sit at fixed offsets after `angle` in both formats
/// (`decode.rs`'s body walk): `visible_to_team[5]`, `spotted_by_team[5]`,
/// then `max_velocity`/`velocity`/`sound`/`scaler_adjust`.
pub fn reset_placement_visibility(body: &mut [u8], layout: &UnitBodyLayout) {
	let team = body[layout.team] as usize;
	let visible = layout.angle + 1;
	for t in 0..5 {
		body[visible + t] = u8::from(t == team); // visible_to_team
		body[visible + 5 + t] = 0; // spotted_by_team
	}
	body[layout.angle + 14] = 0; // scaler_adjust
}

/// A transient-state defect found in a save unit (`save-editor-bug.md`): a unit
/// whose runtime state is impossible for an idle unit — the fingerprint of one
/// cloned from a busy template by an editor without the placement reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueKind {
	/// An idle (`AWAIT`) unit carrying a mid-action sub-order **state** (e.g.
	/// build-in-progress) — the state can't coexist with having no order.
	StaleOrderState,
	/// An idle (`AWAIT`) unit with a non-zero **build countdown** still ticking.
	StaleBuildCountdown,
}

impl IssueKind {
	pub fn describe(self) -> &'static str {
		match self {
			IssueKind::StaleOrderState => "idle unit stuck in a mid-action state (e.g. build-in-progress)",
			IssueKind::StaleBuildCountdown => "idle unit with a lingering build countdown",
		}
	}
}

/// One flagged unit: its spatial-hash id, type, cell, and what is wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransientIssue {
	pub id: u16,
	pub unit_type: u16,
	pub grid: (i16, i16),
	pub kind: IssueKind,
}

/// Whether `state` is one an idle (`AWAIT`) unit may legitimately carry — the
/// freshly-built [`ORDER_STATE_INIT`] or the steady-state
/// [`ORDER_STATE_EXECUTING_ORDER`](super::orders::ORDER_STATE_EXECUTING_ORDER).
/// Any other sub-order state (build/move/fire in-progress, ready-to-*, …) is a
/// mid-action state that can't coexist with having no order.
fn is_idle_state(state: u8) -> bool {
	state == ORDER_STATE_INIT || state == ORDER_STATE_EXECUTING_ORDER
}

/// Scan every unit for an impossible idle+in-progress runtime state, returning
/// each defect paired with its object slot (for the repair pass).
///
/// Gated on `orders == ORDER_AWAIT` — a *definitionally idle* order. A unit with
/// no active order cannot legitimately be part-way through an action, so any
/// non-idle `state` or a live `build_time` is stale runtime state left over from
/// the template it was cloned from. This keeps the pass safe on any save
/// (including a pristine in-game one): a genuinely busy unit carries a non-`AWAIT`
/// order (a real mid-build constructor is `ORDER_BUILD`), so it is never touched,
/// and normalizing an *idle* unit's transient state can lose nothing meaningful.
fn scan(save: &SaveFile) -> Vec<(usize, TransientIssue)> {
	let mut out = Vec::new();
	for (slot, obj) in save.objects.iter().enumerate() {
		let SaveObject::Unit(u) = obj else { continue };
		if u.orders != ORDER_AWAIT {
			continue;
		}
		let issue = |kind| TransientIssue { id: u.id, unit_type: u.unit_type, grid: (u.grid_x, u.grid_y), kind };
		if !is_idle_state(u.state) {
			out.push((slot, issue(IssueKind::StaleOrderState)));
			continue;
		}
		// `build_time` lives in the opaque prefix (not modeled on the record), so read
		// it from the retained body at its captured offset. An idle unit whose build
		// countdown is still ticking is the half-reset cloned-template tell (state was
		// cleared but the countdown wasn't).
		let meta = &save.object_meta[slot];
		let build_time = meta.unit_layout.as_ref().and_then(|l| meta.body_raw.get(l.build_time)).copied().unwrap_or(0);
		if build_time != 0 {
			out.push((slot, issue(IssueKind::StaleBuildCountdown)));
		}
	}
	out
}

/// Every transient-state defect in `save` (`save-editor-bug.md`) — a read-only
/// diagnostic for warning on open. Empty for a clean save.
pub fn check_transient_state(save: &SaveFile) -> Vec<TransientIssue> {
	scan(save).into_iter().map(|(_, issue)| issue).collect()
}

/// Repair every transient-state defect in `save` in place, normalizing each
/// flagged unit to a clean idle state ([`reset_transient_prefix`]) and returning
/// what was fixed. A no-op (returns empty, changes no byte) on a clean save, so it
/// is safe to run unconditionally before every export.
pub fn repair_transient_state(save: &mut SaveFile) -> Vec<TransientIssue> {
	let found = scan(save);
	let v71 = save.header.format == SaveFormat::V71;
	for &(slot, _) in &found {
		let Some(layout) = save.object_meta[slot].unit_layout.clone() else { continue };
		reset_transient_prefix(&mut save.object_meta[slot].body_raw, &layout, v71);
		// Keep the typed record consistent with the bytes just written, so a re-decode
		// of the export agrees with the in-memory model.
		if let SaveObject::Unit(u) = &mut save.objects[slot] {
			u.prior_orders = u.orders;
			u.state = ORDER_STATE_EXECUTING_ORDER;
			u.prior_state = ORDER_STATE_EXECUTING_ORDER;
		}
	}
	found.into_iter().map(|(_, issue)| issue).collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::save::orders::{ORDER_BUILD, ORDER_STATE_BUILD_IN_PROGRESS};
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

	/// Read an `i16` field of the unit with spatial-hash `id`, addressed via a
	/// closure over its captured layout — for asserting on the *unmodeled* body
	/// fields (image_index, move_to) after a round-trip.
	fn field_i16(save: &SaveFile, id: u16, off: impl Fn(&UnitBodyLayout) -> usize) -> i16 {
		let slot = save.objects.iter().position(|o| matches!(o, SaveObject::Unit(u) if u.id == id)).unwrap();
		let layout = save.object_meta[slot].unit_layout.as_ref().unwrap();
		read_i16(&save.object_meta[slot].body_raw, off(layout))
	}

	fn field_u8(save: &SaveFile, id: u16, off: impl Fn(&UnitBodyLayout) -> usize) -> u8 {
		let slot = save.objects.iter().position(|o| matches!(o, SaveObject::Unit(u) if u.id == id)).unwrap();
		let layout = save.object_meta[slot].unit_layout.as_ref().unwrap();
		save.object_meta[slot].body_raw[off(layout)]
	}

	/// A placed unit cloned from a *busy* template comes out idle-valid — the core
	/// fix. SAVE10's engineer (type 0x3D) sits at `state = UNIT_READY` (0x1F, a
	/// non-idle state), the exact kind of transient state an old editor would copy
	/// onto every placement; after the fix the clone is `EXECUTING_ORDER`, its
	/// display frame is the idle `image_base + angle`, and it has no lingering
	/// target/countdown — all verified through a serialize → decode round-trip.
	#[test]
	fn placed_unit_is_idle_valid_not_inherited() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("placed_unit_is_idle_valid_not_inherited: SAVE10 fixture absent");
			return;
		};
		// Confirm the template really is in a non-idle state, or the test proves nothing.
		let engineer = save.units().find(|u| u.unit_type == 0x3D).expect("SAVE10 engineer template");
		assert!(!is_idle_state(engineer.state), "engineer template is in a non-idle state to inherit");

		let mut patched = save.clone();
		let id =
			add_unit(&mut patched, 0x3D, 0, 40, 40, None).expect("the tail follows").expect("engineer template exists");

		let out = read_save_bytes(&write_save(&patched).unwrap(), (save.width, save.height)).unwrap();
		let placed = out.units().find(|u| u.id == id).expect("placed unit present");
		assert_eq!(placed.orders, ORDER_AWAIT, "placed unit is idle");
		assert_eq!(placed.state, ORDER_STATE_EXECUTING_ORDER, "state normalized, not inherited (was 0x1F)");
		assert_eq!(placed.prior_orders, ORDER_AWAIT, "prior_orders mirrors orders");
		assert_eq!(placed.prior_state, ORDER_STATE_EXECUTING_ORDER, "prior_state mirrors state");

		// The unmodeled body fields the reset scrubs: idle display frame, no target,
		// no countdown.
		let image_base = field_i16(&out, id, |l| l.image_block + IMAGE_BASE);
		let image_index = field_i16(&out, id, |l| l.image_block + IMAGE_INDEX);
		assert_eq!(image_index, image_base, "image_index = image_base + angle(0), not a build frame");
		assert_eq!(field_i16(&out, id, |l| l.move_to), 0, "move_to cleared");
		assert_eq!(field_i16(&out, id, |l| l.move_to + 2), 0, "move_to cleared");
		assert_eq!(field_u8(&out, id, |l| l.build_time), 0, "build_time cleared");
		assert_eq!(field_u8(&out, id, |l| l.moved), 0, "moved cleared");

		// And the whole save still re-decodes to a consistent graph.
		assert_eq!(out.units().count(), save.units().count() + 1, "exactly one unit added");
		assert!(check_transient_state(&out).is_empty(), "the placed unit is not flagged");
	}

	/// A placed unit is visible to its OWN team regardless of the template's:
	/// `visible_to_team`/`spotted_by_team`/`scaler_adjust` sit in the unmodeled
	/// prefix and used to clone verbatim — a cross-team clone was invisible to
	/// its owner forever (the renderer gates on `visible_to_team[owner]`, which
	/// the engine writes only at construction, never on load), and an inherited
	/// `scaler_adjust` drew it at 1/32 scale or culled it.
	#[test]
	fn placed_unit_is_visible_to_its_own_team() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("placed_unit_is_visible_to_its_own_team: SAVE10 fixture absent");
			return;
		};
		// Place for a team the template does NOT belong to, so the reset (not the
		// clone) must produce the ownership bytes.
		let tmpl = save.units().find(|u| u.unit_type == 0x3D).expect("SAVE10 engineer template");
		let team = if tmpl.team == 0 { 1 } else { 0 };
		let mut patched = save.clone();
		let id = add_unit(&mut patched, 0x3D, team, 40, 40, None)
			.expect("the tail follows")
			.expect("engineer template exists");
		let out = read_save_bytes(&write_save(&patched).unwrap(), (save.width, save.height)).unwrap();
		for t in 0..5u8 {
			assert_eq!(
				field_u8(&out, id, |l| l.angle + 1 + t as usize),
				u8::from(t == team),
				"visible_to_team[{t}] = own team only"
			);
			assert_eq!(field_u8(&out, id, |l| l.angle + 6 + t as usize), 0, "spotted_by_team[{t}] clear");
		}
		assert_eq!(field_u8(&out, id, |l| l.angle + 14), 0, "scaler_adjust reset to full scale");
	}

	/// The corruption from `save-editor-bug.md` — an idle unit stuck in
	/// `BUILD_IN_PROGRESS` with a build countdown — is detected and repaired to a
	/// clean idle state, and the repair round-trips. Repair is idempotent.
	#[test]
	fn detects_and_repairs_a_corrupt_idle_unit() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("detects_and_repairs_a_corrupt_idle_unit: SAVE10 fixture absent");
			return;
		};
		// A clean save flags nothing.
		assert!(check_transient_state(&save).is_empty(), "SAVE10 is clean");

		// Forge the exact bug fingerprint on an idle unit: orders left AWAIT but the
		// sub-order state and build countdown inherited from a mid-build template.
		let victim = save.units().find(|u| u.orders == ORDER_AWAIT).expect("an idle unit").id;
		let mut bad = save.clone();
		let slot = bad.objects.iter().position(|o| matches!(o, SaveObject::Unit(u) if u.id == victim)).unwrap();
		let layout = bad.object_meta[slot].unit_layout.clone().unwrap();
		bad.object_meta[slot].body_raw[layout.orders + 1] = ORDER_STATE_BUILD_IN_PROGRESS; // state
		bad.object_meta[slot].body_raw[layout.build_time] = 6;
		write_i16(&mut bad.object_meta[slot].body_raw, layout.image_block + IMAGE_INDEX, 36); // build frame
		if let SaveObject::Unit(u) = &mut bad.objects[slot] {
			u.state = ORDER_STATE_BUILD_IN_PROGRESS;
		}

		let found = check_transient_state(&bad);
		assert_eq!(found.len(), 1, "one corrupt unit detected");
		assert_eq!(found[0].id, victim);
		assert_eq!(found[0].kind, IssueKind::StaleOrderState);

		let repaired = repair_transient_state(&mut bad);
		assert_eq!(repaired.len(), 1, "one unit repaired");
		assert!(check_transient_state(&bad).is_empty(), "no issues remain after repair");

		// The repair round-trips: re-decode reads back the clean idle state + frame.
		let out = read_save_bytes(&write_save(&bad).unwrap(), (save.width, save.height)).unwrap();
		let fixed = out.units().find(|u| u.id == victim).unwrap();
		assert_eq!(fixed.state, ORDER_STATE_EXECUTING_ORDER, "state cleared");
		assert_eq!(field_u8(&out, victim, |l| l.build_time), 0, "build_time cleared");
		let base = field_i16(&out, victim, |l| l.image_block + IMAGE_BASE);
		assert_eq!(field_i16(&out, victim, |l| l.image_block + IMAGE_INDEX), base + fixed.angle as i16, "frame reset");
		assert!(check_transient_state(&out).is_empty(), "the exported save is clean");

		// Idempotent: repairing an already-clean save changes nothing.
		assert!(repair_transient_state(&mut bad).is_empty(), "second repair is a no-op");
	}

	/// A real mid-build unit (a genuine `ORDER_BUILD` constructor, `state =
	/// BUILD_IN_PROGRESS`) is **not** flagged — the pass keys on the idle `AWAIT`
	/// order, so it never disturbs a legitimately busy unit.
	#[test]
	fn a_genuinely_busy_unit_is_not_flagged() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("a_genuinely_busy_unit_is_not_flagged: SAVE10 fixture absent");
			return;
		};
		let victim = save.units().next().unwrap().id;
		let mut busy = save.clone();
		let slot = busy.objects.iter().position(|o| matches!(o, SaveObject::Unit(u) if u.id == victim)).unwrap();
		let layout = busy.object_meta[slot].unit_layout.clone().unwrap();
		busy.object_meta[slot].body_raw[layout.orders] = ORDER_BUILD; // a real build order
		busy.object_meta[slot].body_raw[layout.orders + 1] = ORDER_STATE_BUILD_IN_PROGRESS;
		busy.object_meta[slot].body_raw[layout.build_time] = 6;
		if let SaveObject::Unit(u) = &mut busy.objects[slot] {
			u.orders = ORDER_BUILD;
			u.state = ORDER_STATE_BUILD_IN_PROGRESS;
		}
		assert!(check_transient_state(&busy).is_empty(), "an ORDER_BUILD unit mid-build is legitimate");
	}

	/// No false positives across the whole `~/MAX` corpus: every real save the
	/// decoder models is transient-state-clean, so [`check_transient_state`] finds
	/// nothing and an export leaves those units byte-identical. This is the evidence
	/// the (broad) idle-state predicate is safe to run on any save.
	#[test]
	fn all_max_saves_are_transient_clean() {
		let Some(home) = std::env::var_os("HOME") else { return };
		let max_dir = std::path::Path::new(&home).join("MAX");
		let originals = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals");
		if !max_dir.is_dir() || !originals.is_dir() {
			crate::testutil::skip_fixture("all_max_saves_are_transient_clean: fixtures not found");
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
			let issues = check_transient_state(&save);
			assert!(issues.is_empty(), "{} has unexpected transient issues: {issues:?}", path.display());
			// A repair therefore changes nothing — the export stays byte-identical.
			let mut clone = save.clone();
			assert!(repair_transient_state(&mut clone).is_empty());
			assert!(
				write_save(&clone).unwrap() == write_save(&save).unwrap(),
				"repair is a no-op on {}",
				path.display()
			);
			checked += 1;
		}
		assert!(checked > 0, "no ~/MAX saves were checked");
		eprintln!("{checked} ~/MAX saves are transient-state-clean");
	}
}
