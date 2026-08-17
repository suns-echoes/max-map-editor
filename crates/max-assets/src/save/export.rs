//! Save-file export helpers (S6.1): apply the editor's scalar property edits back
//! onto a decoded [`SaveFile`] before re-serializing it with
//! [`write_save`](super::serialize::write_save).
//!
//! An edit is expressed against an existing unit's spatial-hash `id`
//! ([`UnitScalarEdit`]) and patched **in place** into that unit's retained body
//! bytes ([`ObjMeta::body_raw`](super::types::ObjMeta::body_raw)) using the field
//! offsets captured at decode
//! ([`UnitBodyLayout`](super::types::UnitBodyLayout)). Only fixed-width scalars
//! are overwritten (plus the variable-length name, spliced); the object graph's
//! shape, unit positions, and spatial hashes are all untouched, so the opaque
//! tail stays byte-valid and the export needs no hash rebuild (`SAVE-FORMAT.md`
//! §6.9). Writing a field's current value back is a byte-level no-op, so an
//! unedited unit round-trips byte-identically. Adding / removing / moving units
//! and per-unit stat overrides are the graph- or position-touching edits left to
//! S6.2.

use super::error::EditError;
use super::integrity::reset_transient_prefix;
use super::mining::set_initial_mining;
use super::orders::ORDER_DISABLE;
use super::serialize::serialize_unit_values;
use super::types::{ObjMeta, SaveFile, SaveFormat, SaveObject, UNIT_END, UnitBodyLayout, UnitRecord, UnitValues};
use super::unit_types::deploy_orders;
use super::unit_types::flag::{BUILDING, GROUND_COVER};

/// Runtime data enabling [`add_unit`] to synthesize a **fresh, from-scratch
/// body** when the save holds no same-type template to clone — the same
/// deploy-constructor recipe the from-scratch synthesizer uses (Stage C).
/// Without it (no unit database / MAX.RES at hand) such a placement is
/// skipped and reported.
pub struct FreshBodyCtx<'a> {
	/// The runtime unit database (`PATCHES.RES`) — type flags + stat seeds.
	pub db: &'a crate::attribs::UnitStatsDb,
	/// The per-type `D_*` frame table (MAX.RES) — image bases and counts.
	pub frames: &'a [Option<crate::attribs::FrameInfo>; UNIT_END],
}

/// Registry type index of `UnitValues` (`smartfile` class order).
const UNIT_VALUES_TYPE: u32 = 6;

/// `HASH_TEAM_*` owner bit set in a unit's `flags` for each team slot 0..=4
/// (`enums.hpp`): Red, Green, Blue, Gray, then the alien/derelict slot.
const TEAM_FLAG: [u32; 5] = [0x2000, 0x1000, 0x0800, 0x0400, 0x8000];
/// All owner bits together, cleared before stamping a unit's new team.
const TEAM_FLAG_MASK: u32 = 0x2000 | 0x1000 | 0x0800 | 0x0400 | 0x8000;

/// Pixels per cell on the game grid (`GFX_MAP_TILE_SIZE`).
const CELL_PX: u16 = 64;

/// A scalar-property edit to one existing save unit, matched by its spatial-hash
/// `id`. Mirrors the editable fields of the editor's `ObjectProps` that live in a
/// `UnitInfo` body's fixed prefix, plus the post-`path` `connectors` mask.
#[derive(Debug, Clone)]
pub struct UnitScalarEdit<'a> {
	/// Spatial-hash id of the unit to patch (the editor's `ObjectProps::source_id`).
	pub id: u16,
	/// Desired owner slot 0..=4. Patches the body's team byte, the typed record,
	/// AND the flags' `HASH_TEAM_*` owner bit, so ownership stays consistent
	/// everywhere the engine reads it.
	pub team: u8,
	pub name: &'a str,
	pub angle: u8,
	pub turret_angle: u8,
	/// Desired current HP. Clamped to the on-disk field width (a `V70` save's
	/// `hits` is a `u8`, so it cannot exceed 255 regardless of the model value).
	pub hits: u16,
	pub ammo: u8,
	pub orders: u8,
	/// Turns the unit stays disabled. Written to the disable byte only when
	/// `orders == ORDER_DISABLE` in a `V70` save (see [`UnitBodyLayout::disabled_dual`]);
	/// always written in `V71` (a dedicated byte). Clamp to `0..=127` — a `V70` byte
	/// is signed and a value ≥ 128 reads back as negative (→ 0) in the engine.
	pub disabled_turns: u8,
	pub storage: i16,
	pub connectors: u16,
}

/// Patch the retained body of the unit with spatial-hash `edit.id` in `save`,
/// overwriting its scalar props. Returns `true` when a matching unit that carries
/// layout metadata was found and patched, `false` otherwise (unknown id, or an
/// object with no captured [`UnitBodyLayout`]).
pub fn patch_unit_scalars(save: &mut SaveFile, edit: &UnitScalarEdit) -> bool {
	let Some(slot) = save.objects.iter().position(|o| matches!(o, SaveObject::Unit(u) if u.id == edit.id)) else {
		return false;
	};
	let Some(layout) = save.object_meta[slot].unit_layout.clone() else {
		return false;
	};

	// `connectors` lives in the object-reference section, which the serializer emits
	// from the typed record — so set it on the model, not in the opaque prefix.
	// The team edit also reaches the typed record and the flags' `HASH_TEAM_*`
	// owner bit (both body and record): the engine classifies ownership by flags
	// as much as by the team byte, and the complex repair keys on the record.
	let new_flags = match &save.objects[slot] {
		SaveObject::Unit(u) => (u.flags & !TEAM_FLAG_MASK) | TEAM_FLAG[edit.team.min(4) as usize],
		_ => unreachable!("slot was found as a unit above"),
	};
	if let SaveObject::Unit(u) = &mut save.objects[slot] {
		u.connectors = edit.connectors;
		u.team = edit.team;
		u.flags = new_flags;
	}

	let body = &mut save.object_meta[slot].body_raw;

	// Fixed-width scalars in the opaque prefix (emitted verbatim), at their captured
	// offsets — all after `name`, all before the ref section. `flags` sits at the
	// fixed prefix start (`unit_type` u16, `id` u16, then `flags` u32).
	body[4..8].copy_from_slice(&new_flags.to_le_bytes());
	body[layout.team] = edit.team;
	body[layout.angle] = edit.angle;
	body[layout.turret_angle] = edit.turret_angle;
	body[layout.orders] = edit.orders;
	body[layout.ammo] = edit.ammo;
	// Disable countdown. In a V70 save the disable byte doubles as
	// `firing_recoil_frames`, so overwrite it only when the order is DISABLE —
	// otherwise a non-disabled unit's recoil value would be clobbered (and the
	// unedited-save write-safety guard would trip). V71 has a dedicated byte.
	if !layout.disabled_dual || edit.orders == ORDER_DISABLE {
		body[layout.disabled] = edit.disabled_turns;
	}
	let hits = clamp_to_width(edit.hits, layout.hits_width);
	write_uint(body, layout.hits, hits, layout.hits_width);
	body[layout.storage..layout.storage + 2].copy_from_slice(&edit.storage.to_le_bytes());

	// Name (length-prefixed, variable) is spliced last and only when it changed.
	// It precedes every field above, so the fixed patches — already written into
	// the pre-splice buffer — shift with the tail and stay correct.
	if body[layout.name + 2..layout.name + 2 + layout.name_len] != *edit.name.as_bytes() {
		let delta = edit.name.len() as isize - layout.name_len as isize;
		splice_name(body, &layout, edit.name.as_bytes());
		// The resize shifts every field after the name (and the ref section, which
		// the symbolic emit slices at `refs_off`), so keep the stored layout current.
		if let Some(stored) = &mut save.object_meta[slot].unit_layout {
			stored.shift_after_name(delta, edit.name.len());
		}
	}
	true
}

/// Move the unit with spatial-hash `id` to grid cell `(new_x, new_y)`: patch its
/// body's `grid`/`pixel` fields and re-key it in the map hash (`Hash_MapHash`).
/// Returns `true` when a matching unit with layout metadata was found and moved.
///
/// No object is added or removed, so every object index — and thus every other
/// spatial-hash / message / AI back-reference — stays valid; only the map hash
/// changes. The unit is pulled from *all* cells that list it (a mid-move mobile
/// unit may be hashed a cell off its grid) and re-added at its new footprint per
/// the engine's `MapHash::Add`.
pub fn move_unit(save: &mut SaveFile, id: u16, new_x: u16, new_y: u16) -> bool {
	let Some(slot) = save.objects.iter().position(|o| matches!(o, SaveObject::Unit(u) if u.id == id)) else {
		return false;
	};
	let (flags, unit_type) = match &save.objects[slot] {
		SaveObject::Unit(u) => (u.flags, u.unit_type),
		_ => return false,
	};
	let Some(layout) = save.object_meta[slot].unit_layout.clone() else {
		return false;
	};

	// Pixel centre = grid*64 + 32, plus 31 more for a building (`SetPosition`,
	// `unitinfo.cpp`). `grid` is unchanged by that recompute (the +31 stays within
	// the same cell).
	let building = flags & BUILDING != 0;
	let bump = 32 + if building { 31 } else { 0 };
	let px = new_x.wrapping_mul(CELL_PX).wrapping_add(bump);
	let py = new_y.wrapping_mul(CELL_PX).wrapping_add(bump);

	// Patch the body: pixel_x/pixel_y (u16) then grid_x/grid_y (i16).
	let body = &mut save.object_meta[slot].body_raw;
	body[layout.pixel_x..layout.pixel_x + 2].copy_from_slice(&px.to_le_bytes());
	body[layout.pixel_x + 2..layout.pixel_x + 4].copy_from_slice(&py.to_le_bytes());
	body[layout.grid_x..layout.grid_x + 2].copy_from_slice(&(new_x as i16).to_le_bytes());
	body[layout.grid_x + 2..layout.grid_x + 4].copy_from_slice(&(new_y as i16).to_le_bytes());

	// Keep the typed record consistent (a re-decode of the export must agree).
	if let SaveObject::Unit(u) = &mut save.objects[slot] {
		u.pixel_x = px;
		u.pixel_y = py;
		u.grid_x = new_x as i16;
		u.grid_y = new_y as i16;
	}

	// Re-key in the map hash: pull from every cell, then re-add at the new footprint.
	let ground_cover = flags & GROUND_COVER != 0;
	save.map_hash.remove_unit(slot);
	save.map_hash.add_unit(slot, new_x, new_y, unit_type, building, ground_cover);
	true
}

/// Give the unit with spatial-hash `id` a per-unit `base_values` override equal to
/// `values` (the S4.5 max-stats edit). If the unit already has its own inline
/// `UnitValues` (a prior override / an upgraded unit), overwrite it in place;
/// otherwise it shares a team stat table, so insert a fresh inline `UnitValues`
/// object at the unit's `base_values` position and point the unit at it. Returns
/// `Ok(true)` when a matching unit was found; `Err` only if the save's tail will
/// not follow the graph edit (see [`SaveFile::tail_follows_the_graph`]).
///
/// This mirrors the engine's clone-on-edit (`UnitInfo::UpgradeInt`): a diverging
/// unit gets a distinct `UnitValues`, serialized inline (type 6, 28-byte leaf).
/// Inserting shifts later object indices; [`SaveFile::insert_object`] remaps every
/// reference, and the symbolic serializer recomputes on-disk indices.
pub fn apply_stat_override(save: &mut SaveFile, id: u16, values: &UnitValues) -> Result<bool, EditError> {
	let Some(slot) = save.objects.iter().position(|o| matches!(o, SaveObject::Unit(u) if u.id == id)) else {
		return Ok(false);
	};
	let base = match &save.objects[slot] {
		SaveObject::Unit(u) => u.base_values,
		_ => return Ok(false),
	};

	match base {
		// Already a per-unit inline override (an inline object sits above the unit's
		// own slot): overwrite its body and typed value in place.
		Some(bv) if bv > slot => {
			save.object_meta[bv].body_raw = serialize_unit_values(values);
			if let SaveObject::Values(v) = &mut save.objects[bv] {
				*v = values.clone();
			}
			Ok(true)
		}
		// Shared team stat table (a low-index back-reference) or none: insert a new
		// inline `UnitValues` at the unit's `base_values` position — right after the
		// unit and its (leaf) inline path, if any — and repoint the unit.
		_ => {
			let at = base_values_pos(save, slot);
			let meta = ObjMeta {
				type_index: UNIT_VALUES_TYPE,
				contained: 1,
				body_raw: serialize_unit_values(values),
				unit_layout: None,
			};
			save.insert_object(at, SaveObject::Values(values.clone()), meta)?;
			// `slot < at`, so the unit did not move; point it at the new object.
			if let SaveObject::Unit(u) = &mut save.objects[slot] {
				u.base_values = Some(at);
			}
			Ok(true)
		}
	}
}

/// The first-seen object index where unit `slot`'s `base_values` is emitted — just
/// after the unit and its inline `path` (a `UnitPath` is a leaf, so it occupies a
/// single slot; a null/absent path occupies none).
fn base_values_pos(save: &SaveFile, slot: usize) -> usize {
	match &save.objects[slot] {
		SaveObject::Unit(u) if matches!(u.path, Some(p) if p > slot) => slot + 2,
		_ => slot + 1,
	}
}

/// Delete the unit with spatial-hash `id`: drop it — and the inline leaf objects
/// it *owns* (its `path` and any per-unit `base_values`) — from the object graph,
/// the five unit lists, and both spatial hashes, nulling every reference to it.
/// Returns `true` if found. Other units that referenced it as `parent` lose that
/// link (`enemy` is never persisted). Shared `base_values`/`complex` (low-index
/// back-references to the team tables) are left in place.
///
/// Removing only the unit's *owned* inline objects is what keeps the graph valid:
/// a forward-referenced neighbour (e.g. its parent unit) is also in a unit list,
/// so it simply becomes first-seen there instead — the symbolic serializer emits
/// it wherever it is now first referenced.
pub fn remove_unit(save: &mut SaveFile, id: u16) -> Result<bool, EditError> {
	let Some(slot) = save.objects.iter().position(|o| matches!(o, SaveObject::Unit(u) if u.id == id)) else {
		return Ok(false);
	};
	// The unit plus the inline leaves it owns (inline ⇒ index above the unit's own).
	let mut owned = vec![slot];
	if let SaveObject::Unit(u) = &save.objects[slot] {
		if let Some(p) = u.path.filter(|&p| p > slot) {
			owned.push(p);
		}
		if let Some(b) = u.base_values.filter(|&b| b > slot) {
			owned.push(b);
		}
	}
	// Remove the highest index first so the lower ones stay valid across the shifts.
	owned.sort_unstable();
	for &s in owned.iter().rev() {
		save.remove_object(s)?;
	}
	Ok(true)
}

/// Add a new unit of `unit_type` owned by `team` at grid cell `(x, y)`. Returns
/// the new unit's spatial-hash id, or `None` if the save has no same-type unit to
/// use as a body template, or no free id is available for the team.
///
/// A new unit's body is built by cloning an existing same-type unit's opaque
/// prefix (the display/animation fields the engine re-derives from `unit_type` in
/// `UnitInfo::Init` at load), then patching the load-critical fields: a fresh id
/// (`(team << 13) | counter`), the team flag bits, position, an empty name, full
/// HP, and a `base_values` pointing at the new team's shared stat table. It is
/// appended to the object graph, its source list, and both spatial hashes; the
/// encounter-order serializer assigns its on-disk index.
///
/// ⚠ Structural validity is checked by re-decode, but in-game loadability rests on
/// runtime assumptions (the `Init` re-derivation, id/flag conventions) that only a
/// real load in M.A.X. Port can fully confirm.
pub fn add_unit(
	save: &mut SaveFile,
	unit_type: u16,
	team: u8,
	x: u16,
	y: u16,
	fresh: Option<&FreshBodyCtx>,
) -> Result<Option<u16>, EditError> {
	let team = team.min(4);
	let Some((tmpl_slot, list)) = find_template(save, unit_type, team) else {
		// No template anywhere in the save: synthesize a fresh deploy-state body
		// when the runtime data for it is at hand (V71 only), else skip.
		if let Some(ctx) = fresh {
			return add_unit_fresh(save, unit_type, team, x, y, ctx);
		}
		return Ok(None);
	};
	let Some(id) = allocate_unit_id(save, team) else { return Ok(None) };

	let tmpl = match &save.objects[tmpl_slot] {
		SaveObject::Unit(u) => u.clone(),
		_ => return Ok(None),
	};
	let Some(layout) = save.object_meta[tmpl_slot].unit_layout.clone() else { return Ok(None) };
	let building = tmpl.flags & BUILDING != 0;
	let ground_cover = tmpl.flags & GROUND_COVER != 0;

	// The new team's shared stat table for this type (players 0..3); fall back to
	// the template's `base_values` for the alien slot / a missing entry.
	let base_values = team_shared_values(save, team, unit_type).or(tmpl.base_values);
	let max_hits = base_values.and_then(|b| save.values(b)).map(|v| v.hits).unwrap_or(tmpl.hits);

	let bump = 32 + if building { 31 } else { 0 };
	let flags = (tmpl.flags & !TEAM_FLAG_MASK) | TEAM_FLAG[team as usize];
	let (pixel_x, pixel_y) = (x.wrapping_mul(CELL_PX).wrapping_add(bump), y.wrapping_mul(CELL_PX).wrapping_add(bump));

	// A freshly-placed unit starts on its type's deploy order — AWAIT for most,
	// but a mining station powers on, a turret watches, a power plant idles off
	// (`deploy_orders`, mirroring `UnitsManager_DeployUnit`) — paired with the
	// engine's own deploy state: INIT for a power-on host (so the first game
	// tick runs PowerUp and the station really produces), else the steady
	// EXECUTING_ORDER. Prior mirrors current, like the engine's deploy.
	let orders = deploy_orders(unit_type);
	let state = super::orders::deploy_state_for(orders);
	let rec = UnitRecord {
		unit_type,
		id,
		flags,
		pixel_x,
		pixel_y,
		grid_x: x as i16,
		grid_y: y as i16,
		name: String::new(),
		team,
		angle: 0,
		turret_angle: 0,
		orders,
		state,
		prior_orders: orders,
		prior_state: state,
		disabled_turns: 0,
		hits: max_hits,
		ammo: 0,
		storage: 0,
		build_rate: 0,
		connectors: 0,
		turret_image_base: tmpl.turret_image_base,
		connector_image_base: tmpl.connector_image_base,
		path: None,
		base_values,
		complex: None,
		parent_unit: None,
		enemy_unit: None,
		build_list: Vec::new(),
	};

	// The body only needs its opaque prefix (the serializer emits refs from the
	// record). Clone the template prefix, patch the load-critical scalar fields,
	// then clear the name (splice it to empty and rebase the ref section).
	let mut body = save.object_meta[tmpl_slot].body_raw[..layout.refs_off].to_vec();
	body[2..4].copy_from_slice(&id.to_le_bytes());
	body[4..8].copy_from_slice(&flags.to_le_bytes());
	body[layout.pixel_x..layout.pixel_x + 2].copy_from_slice(&pixel_x.to_le_bytes());
	body[layout.pixel_x + 2..layout.pixel_x + 4].copy_from_slice(&pixel_y.to_le_bytes());
	body[layout.grid_x..layout.grid_x + 2].copy_from_slice(&(x as i16).to_le_bytes());
	body[layout.grid_x + 2..layout.grid_x + 4].copy_from_slice(&(y as i16).to_le_bytes());
	body[layout.team] = team;
	body[layout.angle] = 0;
	body[layout.turret_angle] = 0;
	body[layout.orders] = orders;
	body[layout.ammo] = 0;
	write_uint(&mut body, layout.hits, clamp_to_width(max_hits, layout.hits_width), layout.hits_width);
	body[layout.storage..layout.storage + 2].copy_from_slice(&0i16.to_le_bytes());
	let name_end = layout.name + 2 + layout.name_len;
	body.splice(layout.name..name_end, 0u16.to_le_bytes());
	let mut new_layout = layout.clone();
	new_layout.shift_after_name(-(layout.name_len as isize), 0);

	// The core fix for save-editor-bug.md: scrub the cloned template's transient
	// runtime state (image_index, sub-order state, build_time, move_to, moved,
	// recoil) so the placed unit is idle-valid regardless of what the template was
	// doing. Runs after the name splice, on the just-patched body via `new_layout`.
	let v71 = save.header.format == SaveFormat::V71;
	reset_transient_prefix(&mut body, &new_layout, v71);
	// Deploy state pair: `reset_transient_prefix` writes the settled
	// EXECUTING_ORDER; a power-on host needs the engine's INIT instead.
	body[new_layout.orders + 1] = state;
	body[new_layout.orders + 3] = state; // prior_state
	// Per-team visibility + display scale: a body cloned from another team's
	// unit (or one caught mid-expand / stored in a depot) is otherwise
	// invisible to its owner forever - the renderer gates on
	// `visible_to_team[own]`, which the engine writes only at construction.
	super::integrity::reset_placement_visibility(&mut body, &new_layout);
	// The four storage buildings show `image_base + 1` from deploy on
	// (`UnitsManager_DeployUnit` draws that frame explicitly).
	let bump = super::unit_types::deploy_frame_bump(unit_type);
	if bump != 0 {
		let base = i16::from_le_bytes([body[new_layout.image_block + 2], body[new_layout.image_block + 3]]);
		body[new_layout.image_block + 10..new_layout.image_block + 12].copy_from_slice(&(base + bump).to_le_bytes()); // image_index
	}
	// A mining station's sprite base is clan-derived (`image_base = (clan-1)*2`,
	// `unitinfo.cpp:274`) — a template cloned from another team carries the
	// source clan's base, so re-derive it for the owning team's clan.
	if unit_type == super::mining::MININGST
		&& let Some(clan) = save.teams.get(team as usize).map(|c| c.team_clan).filter(|&c| c >= 1)
	{
		let base = (clan as i16 - 1) * 2;
		body[new_layout.image_block + 2..new_layout.image_block + 4].copy_from_slice(&base.to_le_bytes());
		// The placement angle is 0, so the idle frame is the base itself.
		body[new_layout.image_block + 10..new_layout.image_block + 12].copy_from_slice(&base.to_le_bytes());
	}

	// Production bytes are per-instance state too: a cloned working mining
	// station would hand its live allocation to the new unit. Zero them for
	// every placement; a placed mining station then derives its own off the
	// ground it lands on, below (`save::mining`, HANDOFF Finding 3).
	let mining_off = new_layout.build_time + 1;
	body[mining_off..mining_off + 7].fill(0);

	// Appended, so no existing index moves - but the graph still grows by one,
	// and the tail's own inline bodies are numbered right after it, so this goes
	// through `insert_object` rather than a bare push.
	let slot = save.objects.len();
	let meta = ObjMeta { type_index: 5, contained: 1, body_raw: body, unit_layout: Some(new_layout) };
	save.insert_object(slot, SaveObject::Unit(rec), meta)?;
	save.list_by_index_mut(list).push(slot);
	let bucket = (id as usize) % save.unit_hash.len().max(1);
	save.unit_hash[bucket].push(slot);
	save.map_hash.add_unit(slot, x, y, unit_type, building, ground_cover);
	set_initial_mining(save, slot);
	Ok(Some(id))
}

/// Synthesize and insert a **from-scratch** unit body — the deploy-constructor
/// recipe (`UnitInfo::UnitInfo(type, team, id, angle)`, Stage C's
/// `encode_unit_prefix`) — for a placement whose type has no template in the
/// save. `V71` saves only (the fresh encoder writes the `V71` layout).
/// Returns `Ok(None)` when the type's flags or frame table are unavailable.
fn add_unit_fresh(
	save: &mut SaveFile,
	unit_type: u16,
	team: u8,
	x: u16,
	y: u16,
	ctx: &FreshBodyCtx,
) -> Result<Option<u16>, EditError> {
	use super::unit_types::UnitCategory;
	if save.header.format != SaveFormat::V71 {
		return Ok(None);
	}
	let ty = unit_type as usize;
	let Some(type_flags) = ctx.db.meta_for(unit_type).map(|m| m.flags).filter(|&f| f != 0) else { return Ok(None) };
	let Some(frame) = ctx.frames.get(ty).copied().flatten() else { return Ok(None) };
	let Some(id) = allocate_unit_id(save, team) else { return Ok(None) };
	let flags = (type_flags & !TEAM_FLAG_MASK) | TEAM_FLAG[team as usize];
	let clan = save.teams.get(team as usize).map_or(0, |c| c.team_clan);

	// Stat seed: the team's shared current `UnitValues` (what the engine's
	// deploy reads), else — the alien slot, or a table gap — a fresh object
	// from the unit database.
	let (values, values_ref) = match team_shared_values(save, team, unit_type) {
		Some(idx) => match save.values(idx) {
			Some(v) => (v.clone(), idx),
			None => return Err(EditError::Corrupt("team stat table references a non-values object".into())),
		},
		None => {
			let mut v = ctx.db.clan_unit_values(clan)[ty].clone();
			v.in_use = true;
			let idx = save.objects.len();
			let meta = ObjMeta {
				type_index: UNIT_VALUES_TYPE,
				contained: 1,
				body_raw: serialize_unit_values(&v),
				unit_layout: None,
			};
			save.insert_object(idx, SaveObject::Values(v.clone()), meta)?;
			(v, idx)
		}
	};

	let orders = deploy_orders(unit_type);
	let state = super::orders::deploy_state_for(orders);
	let fresh = super::encode::FreshUnit {
		unit_type,
		id,
		flags,
		grid_x: x as i16,
		grid_y: y as i16,
		name: String::new(),
		team,
		clan,
		unit_serial: 1,
		angle: 0,
		turret_angle: 0,
		orders,
		disabled_turns: 0,
		hits: values.hits,
		ammo: values.ammo.min(u8::MAX as u16) as u8,
		storage: 0,
		connectors: 0,
		values: values.clone(),
		frame,
		total_images: 0,
		mining: [0; 7],
	};
	let (body, layout) = super::encode::encode_unit_prefix(&fresh);
	let record = UnitRecord {
		unit_type,
		id,
		flags,
		pixel_x: x.wrapping_mul(CELL_PX).wrapping_add(32),
		pixel_y: y.wrapping_mul(CELL_PX).wrapping_add(32),
		grid_x: x as i16,
		grid_y: y as i16,
		name: String::new(),
		team,
		angle: 0,
		turret_angle: 0,
		orders,
		state,
		prior_orders: orders,
		prior_state: state,
		disabled_turns: 0,
		hits: values.hits,
		ammo: fresh.ammo,
		storage: 0,
		build_rate: 1,
		connectors: 0,
		turret_image_base: frame.turret_image_base,
		connector_image_base: frame.connector_image_base,
		path: None,
		base_values: Some(values_ref),
		// The complex pass (`repair_complexes`, run by every export right after
		// the add pass) attaches an engine-valid `Complex` to each host.
		complex: None,
		parent_unit: None,
		enemy_unit: None,
		build_list: Vec::new(),
	};
	let building = flags & BUILDING != 0;
	let ground_cover = flags & GROUND_COVER != 0;
	let list = match UnitCategory::from_flags(flags) {
		UnitCategory::GroundCover => 0,
		UnitCategory::MobileLandSea => 1,
		UnitCategory::Stationary => 2,
		UnitCategory::MobileAir => 3,
		UnitCategory::Particle => 4,
	};
	let slot = save.objects.len();
	let meta = ObjMeta { type_index: 5, contained: 1, body_raw: body, unit_layout: Some(layout) };
	save.insert_object(slot, SaveObject::Unit(record), meta)?;
	save.list_by_index_mut(list).push(slot);
	let bucket = (id as usize) % save.unit_hash.len().max(1);
	save.unit_hash[bucket].push(slot);
	save.map_hash.add_unit(slot, x, y, unit_type, building, ground_cover);
	set_initial_mining(save, slot);
	Ok(Some(id))
}

/// Find an existing unit of `unit_type` to clone, returning its object slot and
/// the index (0..=4) of the unit list it belongs to. Prefers a same-`team`
/// template: the cloned prefix carries team-adjacent bytes the patcher doesn't
/// model (unit_id build number, a MININGST's clan-derived `image_base`), which
/// are right by construction when the template already belongs to the team.
fn find_template(save: &SaveFile, unit_type: u16, team: u8) -> Option<(usize, usize)> {
	let mut any = None;
	for (li, (_, list)) in save.lists().iter().enumerate() {
		for &slot in *list {
			let Some(u) = save.unit(slot).filter(|u| u.unit_type == unit_type) else { continue };
			if u.team == team {
				return Some((slot, li));
			}
			any.get_or_insert((slot, li));
		}
	}
	any
}

/// Allocate a fresh spatial-hash id for `team` — `(team << 13) | counter` —
/// without ever colliding with an id the game assigns later (HANDOFF 2026-08-02
/// Finding 2). The engine's allocator (`units_manager.cpp:2236`) hands out
/// `number_of_objects_created + 1` per creation, **wrapping to 1 at `0x1FFF`**,
/// so "lowest free counter" alone is exactly the game's next id whenever the
/// team has no gaps — a guaranteed duplicate. Instead: prefer the lowest free
/// counter **at or below** the stored `number_of_objects_created` (a destroyed
/// unit's number, which the game never hands out again until the wrap it
/// already lives with); with no such gap — the common case — advance the
/// stored counter exactly like the engine and write it back
/// ([`set_objects_created`]), so the game numbers its next unit *after* ours.
/// `None` if the team's id space is somehow full.
fn allocate_unit_id(save: &mut SaveFile, team: u8) -> Option<u16> {
	let base = (team as u16) << 13;
	let used: std::collections::HashSet<u16> =
		save.units().filter(|u| (u.id >> 13) == team as u16).map(|u| u.id & 0x1FFF).collect();
	// The alien slot of a V70 save serializes no CTInfo. Derelicts never build,
	// so the game allocates nothing there and the lowest free counter is safe.
	let counter = save.teams.get(team as usize).map(|ct| ct.number_of_objects_created);
	let (k, bumped) = pick_counter(&used, counter)?;
	if bumped {
		set_objects_created(save, team, k);
	}
	Some(base | k)
}

/// The counter half of [`allocate_unit_id`], pure for testability: the chosen
/// counter and whether it advanced past the stored one (and so must be written
/// back). `counter` is `None` when the team serializes no CTInfo.
fn pick_counter(used: &std::collections::HashSet<u16>, counter: Option<u16>) -> Option<(u16, bool)> {
	let Some(counter) = counter else {
		return (1..0x1FFF).find(|k| !used.contains(k)).map(|k| (k, false));
	};
	// A free counter at or below the last one the game handed out.
	if let Some(k) = (1..=counter.min(0x1FFE)).find(|k| !used.contains(k)) {
		return Some((k, false));
	}
	// None free: advance like the engine (pre-increment, wrap at 0x1FFF -> 1),
	// but skip counters still in use — the wrapped engine mints duplicates
	// there; the editor must not.
	let mut k = counter;
	for _ in 0..0x1FFE {
		k += 1;
		if k >= 0x1FFF {
			k = 1;
		}
		if !used.contains(&k) {
			return Some((k, true));
		}
	}
	None
}

/// Write `team`'s advanced `number_of_objects_created` into both the typed
/// [`CtInfo`](super::types::CtInfo) and the retained region-16 block that
/// [`write_save`](super::serialize::write_save) emits verbatim. The counter is
/// a `u16` at a fixed offset in both formats: after `markers[10]` (40) +
/// `team_type`/`finished_turn`/`team_clan` (3) + `research_topics` (96) +
/// `team_points` (4) in `V70`; the same without the leading markers in `V71`
/// (`saveload.cpp` `SaveLoad_LoadFormatV70`/`V71`).
fn set_objects_created(save: &mut SaveFile, team: u8, value: u16) {
	if let Some(ct) = save.teams.get_mut(team as usize) {
		ct.number_of_objects_created = value;
	}
	let off = match save.header.format {
		SaveFormat::V70 => 143,
		SaveFormat::V71 => 103,
	};
	if let Some(block) = save.raw.ct_info.get_mut(team as usize)
		&& block.len() >= off + 2
	{
		block[off..off + 2].copy_from_slice(&value.to_le_bytes());
	}
}

/// The object index of `team`'s shared `UnitValues` for `unit_type` (its
/// `current_values` stat table), if present.
fn team_shared_values(save: &SaveFile, team: u8, unit_type: u16) -> Option<usize> {
	save.team_units.get(team as usize)?.current_values.get(unit_type as usize).copied().flatten()
}

/// Clamp `value` to the maximum an unsigned field of `width` bytes can hold, so a
/// too-large model value saturates instead of wrapping to a garbage low byte.
fn clamp_to_width(value: u16, width: usize) -> u16 {
	match width {
		1 => value.min(u8::MAX as u16),
		_ => value,
	}
}

/// Overwrite `width` little-endian bytes at `off` with the low bytes of `value`.
fn write_uint(body: &mut [u8], off: usize, value: u16, width: usize) {
	for (i, b) in body[off..off + width].iter_mut().enumerate() {
		*b = (value >> (8 * i)) as u8;
	}
}

/// Replace the length-prefixed name (`u16` len + raw bytes) at `layout.name` with
/// `new`. Resizes the body but touches no object indices, positions, or subtree
/// sizes, so the export stays graph- and hash-valid.
fn splice_name(body: &mut Vec<u8>, layout: &UnitBodyLayout, new: &[u8]) {
	let old_end = layout.name + 2 + layout.name_len;
	let mut replacement = Vec::with_capacity(2 + new.len());
	replacement.extend_from_slice(&(new.len() as u16).to_le_bytes());
	replacement.extend_from_slice(new);
	body.splice(layout.name..old_end, replacement);
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::save::{read_save, write_save};

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

	/// The first modeled unit's editable props, as a same-valued edit — the
	/// identity patch used to prove no-op safety.
	fn identity_edit(save: &SaveFile) -> UnitScalarEdit<'_> {
		let u = save.units().next().expect("save has at least one unit");
		UnitScalarEdit {
			id: u.id,
			team: u.team,
			name: &u.name,
			angle: u.angle,
			turret_angle: u.turret_angle,
			hits: u.hits,
			ammo: u.ammo,
			orders: u.orders,
			disabled_turns: u.disabled_turns,
			storage: u.storage,
			connectors: u.connectors,
		}
	}

	/// Re-writing a unit's current values changes nothing: the export stays
	/// byte-identical to the original (the export-safety invariant).
	#[test]
	fn identity_patch_is_byte_identical() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("identity_patch_is_byte_identical: SAVE10 fixture absent");
			return;
		};
		let original = write_save(&save).unwrap();
		let mut patched = save.clone();
		let edit = identity_edit(&save);
		let id = edit.id;
		assert!(patch_unit_scalars(&mut patched, &edit), "unit {id} should be found");
		assert!(write_save(&patched).unwrap() == original, "an identity patch must not change any byte");
	}

	/// A scalar edit survives a serialize → decode round-trip: the field reads
	/// back changed, and the object graph is still consistent.
	#[test]
	fn scalar_edit_round_trips() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("scalar_edit_round_trips: SAVE10 fixture absent");
			return;
		};
		let target = save.units().next().unwrap().clone();
		let new_angle = target.angle ^ 0x04; // flip a heading bit → a definitely-different value
		let mut patched = save.clone();
		let mut edit = identity_edit(&save);
		edit.angle = new_angle;
		edit.hits = target.hits.saturating_sub(1).max(1);
		assert!(patch_unit_scalars(&mut patched, &edit));

		let bytes = write_save(&patched).unwrap();
		let redecoded = read_save_from_bytes(&bytes, (save.width, save.height));
		let got = redecoded.units().find(|u| u.id == target.id).expect("unit still present");
		assert_eq!(got.angle, new_angle, "angle edit persisted");
		assert_eq!(got.hits, edit.hits, "hits edit persisted");
		assert_eq!(got.unit_type, target.unit_type, "unrelated fields intact");
		assert_eq!(redecoded.units().count(), save.units().count(), "no units gained or lost");
	}

	/// Disabling a unit (order → DISABLE) with a countdown round-trips: in a V70
	/// save the single recoil byte carries the disable turns, disambiguated by the
	/// order, so the re-decode reads the order AND the turns back.
	#[test]
	fn disable_edit_round_trips() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("disable_edit_round_trips: SAVE10 fixture absent");
			return;
		};
		let target = save.units().next().unwrap().clone();
		let mut patched = save.clone();
		let mut edit = identity_edit(&save);
		edit.orders = ORDER_DISABLE;
		edit.disabled_turns = 5;
		assert!(patch_unit_scalars(&mut patched, &edit));

		let redecoded = read_save_from_bytes(&write_save(&patched).unwrap(), (save.width, save.height));
		let got = redecoded.units().find(|u| u.id == target.id).expect("unit still present");
		assert_eq!(got.orders, ORDER_DISABLE, "the unit is now disabled");
		assert_eq!(got.disabled_turns, 5, "the disable countdown survives the round-trip");
	}

	/// A connector-mask edit round-trips. `connectors` lives in the symbolic ref
	/// section, so the patch must reach the typed record, not just `body_raw`.
	#[test]
	fn connectors_edit_round_trips() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("connectors_edit_round_trips: SAVE10 fixture absent");
			return;
		};
		let m = save.units().find(|u| u.unit_type == 0x28).expect("SAVE10 mining station").clone();
		let new_conn = m.connectors ^ 0x0F;
		let mut patched = save.clone();
		let edit = UnitScalarEdit {
			id: m.id,
			team: m.team,
			name: &m.name,
			angle: m.angle,
			turret_angle: m.turret_angle,
			hits: m.hits,
			ammo: m.ammo,
			orders: m.orders,
			disabled_turns: m.disabled_turns,
			storage: m.storage,
			connectors: new_conn,
		};
		assert!(patch_unit_scalars(&mut patched, &edit));
		let out = read_save_from_bytes(&write_save(&patched).unwrap(), (save.width, save.height));
		assert_eq!(out.units().find(|u| u.id == m.id).unwrap().connectors, new_conn, "connectors edit exported");
	}

	/// A rename resizes the body yet keeps the file decodable, and every other
	/// unit is untouched.
	#[test]
	fn rename_splices_and_round_trips() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("rename_splices_and_round_trips: SAVE10 fixture absent");
			return;
		};
		let target = save.units().next().unwrap().clone();
		let new_name = "A Much Longer Custom Name";
		let mut patched = save.clone();
		let mut edit = identity_edit(&save);
		edit.name = new_name;
		assert!(patch_unit_scalars(&mut patched, &edit));

		let bytes = write_save(&patched).unwrap();
		let redecoded = read_save_from_bytes(&bytes, (save.width, save.height));
		let got = redecoded.units().find(|u| u.id == target.id).expect("renamed unit present");
		assert_eq!(got.name, new_name, "name spliced in");
		assert_eq!(redecoded.units().count(), save.units().count(), "no units gained or lost");
	}

	fn read_save_from_bytes(bytes: &[u8], dims: (u16, u16)) -> SaveFile {
		crate::save::read_save_bytes(bytes, dims).expect("re-decode exported save")
	}

	/// Editing a unit that the graph stores INLINE (nested in another unit's body
	/// via a forward parent reference — SAVE10's engineer id 6 sits inside the
	/// SMLTAPE→ENGINEER→ADUMP cluster) must still take effect. This is the case that
	/// verbatim-body patching silently dropped before the symbolic re-emit.
	#[test]
	fn scalar_edit_of_inline_unit_round_trips() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("scalar_edit_of_inline_unit_round_trips: SAVE10 fixture absent");
			return;
		};
		let engineer = save.units().find(|u| u.id == 6).expect("SAVE10 engineer id 6").clone();
		// The engineer must actually be an inline (nested) object, or this test
		// wouldn't exercise the bug.
		let slot = save.objects.iter().position(|o| matches!(o, SaveObject::Unit(u) if u.id == 6)).unwrap();
		let inline = (0..slot).any(|s| slot < s + save.object_meta[s].contained);
		assert!(inline, "engineer id 6 is expected to be an inline object");

		let new_ammo = engineer.ammo.wrapping_add(3);
		let mut patched = save.clone();
		let mut edit = identity_edit(&save);
		edit.id = 6;
		edit.name = &engineer.name;
		edit.team = engineer.team;
		edit.angle = engineer.angle;
		edit.turret_angle = engineer.turret_angle;
		edit.hits = engineer.hits;
		edit.ammo = new_ammo;
		edit.orders = engineer.orders;
		edit.storage = engineer.storage;
		edit.connectors = engineer.connectors;
		assert!(patch_unit_scalars(&mut patched, &edit));

		let out = read_save_from_bytes(&write_save(&patched).unwrap(), (save.width, save.height));
		let got = out.units().find(|u| u.id == 6).expect("engineer present");
		assert_eq!(got.ammo, new_ammo, "an inline unit's scalar edit is exported");
	}

	/// A per-unit stat override (S4.5) inserts an inline `UnitValues` for a unit
	/// that shares a team stat table, points the unit at it, and round-trips; a
	/// second override overwrites in place (no new object). The object graph stays
	/// consistent (re-decode succeeds, unit count preserved).
	#[test]
	fn stat_override_inserts_then_overwrites() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("stat_override_inserts_then_overwrites: SAVE10 fixture absent");
			return;
		};
		// A unit sharing a team stat table: its `base_values` is a low-index back-ref.
		let (id, bv) = save
			.objects
			.iter()
			.enumerate()
			.find_map(|(s, o)| match o {
				SaveObject::Unit(u) => u.base_values.filter(|&b| b < s).map(|b| (u.id, b)),
				_ => None,
			})
			.expect("a unit sharing a team stat table");

		let mut new_values = save.values(bv).unwrap().clone();
		new_values.hits = new_values.hits.wrapping_add(100);
		new_values.in_use = true;

		let mut patched = save.clone();
		assert!(apply_stat_override(&mut patched, id, &new_values).expect("the tail follows"));
		let after_insert = patched.objects.len();
		assert_eq!(after_insert, save.objects.len() + 1, "one inline UnitValues added");

		let out = read_save_from_bytes(&write_save(&patched).unwrap(), (save.width, save.height));
		assert_eq!(out.units().count(), save.units().count(), "unit count preserved");
		let u = out.units().find(|u| u.id == id).unwrap();
		let u_slot = out.objects.iter().position(|o| matches!(o, SaveObject::Unit(uu) if uu.id == id)).unwrap();
		let bv2 = u.base_values.expect("still has base_values");
		assert!(bv2 > u_slot, "override is now the unit's own inline object");
		assert_eq!(out.values(bv2).unwrap().hits, new_values.hits, "overridden max HP round-trips");

		// A second override overwrites the existing inline object — no new insert.
		let mut new_values2 = new_values.clone();
		new_values2.attack = new_values2.attack.wrapping_add(5);
		assert!(apply_stat_override(&mut patched, id, &new_values2).expect("the tail follows"));
		assert_eq!(patched.objects.len(), after_insert, "re-override overwrites in place");
		let out2 = read_save_from_bytes(&write_save(&patched).unwrap(), (save.width, save.height));
		let u2 = out2.units().find(|u| u.id == id).unwrap();
		assert_eq!(out2.values(u2.base_values.unwrap()).unwrap().attack, new_values2.attack, "second edit round-trips");
	}

	/// Removing a unit that other units reference as `parent` — SAVE10's engineer
	/// id 6, itself an inline object in the cyclic SMLTAPE→ENGINEER→ADUMP cluster —
	/// deletes it, nulls the dangling links, and the graph re-decodes cleanly.
	#[test]
	fn remove_inline_unit_deletes_and_round_trips() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("remove_inline_unit_deletes_and_round_trips: SAVE10 fixture absent");
			return;
		};
		let before = save.units().count();
		let mut patched = save.clone();
		assert!(remove_unit(&mut patched, 6).expect("the tail follows"));

		let out = read_save_from_bytes(&write_save(&patched).unwrap(), (save.width, save.height));
		assert_eq!(out.units().count(), before - 1, "one unit removed");
		assert!(out.units().all(|u| u.id != 6), "engineer id 6 is gone");
		// Units that pointed at it lost the link; none dangles at a live unit's slot.
		assert!(out.units().all(|u| u.parent_unit != Some(usize::MAX)), "sanity: indices resolved");
		// The map hash no longer lists the removed unit (a broken index would fail
		// the re-decode above); every remaining cell reference still resolves.
		for cell in out.map_hash.buckets.iter().flatten() {
			for &idx in &cell.units {
				assert!(out.unit(idx).is_some(), "map-hash entry resolves after removal");
			}
		}
	}

	/// Removing a unit that OWNS an inline `base_values` (created here via a stat
	/// override) drops both the unit and its private `UnitValues` — no orphan is
	/// left to desync the first-seen index space.
	#[test]
	fn remove_unit_drops_its_owned_inline_values() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("remove_unit_drops_its_owned_inline_values: SAVE10 fixture absent");
			return;
		};
		let (id, bv) = save
			.objects
			.iter()
			.enumerate()
			.find_map(|(s, o)| match o {
				SaveObject::Unit(u) => u.base_values.filter(|&b| b < s).map(|b| (u.id, b)),
				_ => None,
			})
			.expect("a unit sharing a team stat table");

		let mut patched = save.clone();
		let values = save.values(bv).unwrap().clone();
		assert!(apply_stat_override(&mut patched, id, &values).expect("the tail follows")); // +1 inline UnitValues
		let objs = patched.objects.len();
		assert!(remove_unit(&mut patched, id).expect("the tail follows")); // must drop the unit AND its inline values
		assert_eq!(patched.objects.len(), objs - 2, "unit + its owned inline values removed");

		let out = read_save_from_bytes(&write_save(&patched).unwrap(), (save.width, save.height));
		assert_eq!(out.units().count(), save.units().count() - 1, "exactly one unit fewer");
		assert!(out.units().all(|u| u.id != id), "the unit is gone");
	}

	/// The pure counter picker (HANDOFF 2026-08-02 Finding 2) against the
	/// engine's allocator semantics: a gap at/below the stored counter is
	/// reused without a bump; a gapless team advances the counter; the advance
	/// wraps at 0x1FFF -> 1 like `units_manager.cpp:2236` while skipping
	/// counters still in use; a full id space yields `None`; a counter-less
	/// team (V70 alien) falls back to lowest-free.
	#[test]
	fn pick_counter_mirrors_the_engine_allocator() {
		use std::collections::HashSet;
		let used = |v: &[u16]| -> HashSet<u16> { v.iter().copied().collect() };

		// Gap below the counter: reused, no bump — the game never hands out a
		// destroyed unit's number again (until its own wrap).
		assert_eq!(pick_counter(&used(&[1, 2, 4, 5]), Some(5)), Some((3, false)));
		// No gap (a team that never lost a unit): advance the counter.
		assert_eq!(pick_counter(&used(&[1, 2, 3]), Some(3)), Some((4, true)));
		// A fresh team (counter 0, nothing used): the engine's first id is 1.
		assert_eq!(pick_counter(&used(&[]), Some(0)), Some((1, true)));
		// A freed number under a near-wrapped counter is still a gap, not a bump.
		assert_eq!(pick_counter(&used(&[1, 0x1FFE]), Some(0x1FFE)), Some((2, false)));
		// The advance skips counters still in use above the stored one — the
		// wrapped engine would mint duplicates there; the editor must not.
		assert_eq!(pick_counter(&used(&[1, 2, 3, 4, 5]), Some(3)), Some((6, true)));
		// Counters above the stored counter (a wrapped save) are not gaps.
		assert_eq!(pick_counter(&used(&[1, 2, 500]), Some(2)), Some((3, true)));
		// Full id space: refused rather than a duplicate.
		let full: HashSet<u16> = (1..0x1FFF).collect();
		assert_eq!(pick_counter(&full, Some(100)), None);
		// No CTInfo counter (V70 alien slot): lowest free, no bump to record.
		assert_eq!(pick_counter(&used(&[1, 3]), None), Some((2, false)));
	}

	/// The V70 decoder surfaces `number_of_objects_created` (it used to skip
	/// it): SAVE10's team counters cover every unit id counter in use.
	#[test]
	fn v70_ct_info_surfaces_the_object_counter() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("v70_ct_info_surfaces_the_object_counter: SAVE10 fixture absent");
			return;
		};
		for (t, ct) in save.teams.iter().enumerate() {
			let max_used = save.units().filter(|u| (u.id >> 13) as usize == t).map(|u| u.id & 0x1FFF).max();
			let Some(max_used) = max_used else { continue };
			assert!(
				ct.number_of_objects_created >= max_used,
				"team {t}: counter {} covers the highest used id counter {max_used}",
				ct.number_of_objects_created,
			);
		}
	}

	/// The id allocator never mints the game's next id: a gapless team advances
	/// the stored counter — typed AND the retained CTInfo bytes, proven by
	/// re-decoding the export — and a freed number is reused without a bump.
	#[test]
	fn allocated_ids_never_collide_with_the_games_next() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("allocated_ids_never_collide_with_the_games_next: SAVE10 fixture absent");
			return;
		};

		// Whatever SAVE10's gap situation, the allocation contract holds: the id
		// is unused, and it either sits at/below the counter (a reused gap, no
		// bump) or is exactly the advanced counter, written back.
		let mut patched = save.clone();
		let counter = save.teams[0].number_of_objects_created;
		let used: std::collections::HashSet<u16> =
			save.units().filter(|u| (u.id >> 13) == 0).map(|u| u.id & 0x1FFF).collect();
		let id = add_unit(&mut patched, 0x3D, 0, 40, 40, None).expect("tail follows").expect("engineer template");
		let k = id & 0x1FFF;
		assert!(!used.contains(&k), "the allocated counter was free");
		if k <= counter {
			assert_eq!(
				patched.teams[0].number_of_objects_created, counter,
				"a reused gap does not advance the counter"
			);
		} else {
			assert_eq!(k, counter + 1, "no gap: the counter advanced exactly like the engine");
			assert_eq!(patched.teams[0].number_of_objects_created, k, "the typed counter followed");
			let out = read_save_from_bytes(&write_save(&patched).unwrap(), (save.width, save.height));
			assert_eq!(
				out.teams[0].number_of_objects_created, k,
				"the advanced counter reached the retained CTInfo bytes"
			);
		}

		// Free a known number, then allocate: the freed number is preferred and
		// the counter does not move.
		let mut gapped = patched.clone();
		let victim = gapped.units().filter(|u| (u.id >> 13) == 0).map(|u| u.id).min().expect("team 0 has units");
		assert!(remove_unit(&mut gapped, victim).expect("tail follows"));
		let counter2 = gapped.teams[0].number_of_objects_created;
		let expect: u16 = {
			let used2: std::collections::HashSet<u16> =
				gapped.units().filter(|u| (u.id >> 13) == 0).map(|u| u.id & 0x1FFF).collect();
			(1..=counter2).find(|c| !used2.contains(c)).expect("removing a unit opened a gap")
		};
		let id2 = add_unit(&mut gapped, 0x3D, 0, 42, 40, None).expect("tail follows").expect("engineer template");
		assert_eq!(id2 & 0x1FFF, expect, "the lowest freed number is reused");
		assert_eq!(gapped.teams[0].number_of_objects_created, counter2, "no bump when a gap exists");
	}

	/// Adding a unit clones a same-type template, allocates a free id, and appends
	/// it to the graph / list / both hashes; the export re-decodes with the new
	/// unit present, positioned, owned, and stat-linked. A type with no template in
	/// the save is refused.
	#[test]
	fn add_unit_clones_a_template_and_round_trips() {
		let Some(save) = save10() else {
			crate::testutil::skip_fixture("add_unit_clones_a_template_and_round_trips: SAVE10 fixture absent");
			return;
		};
		let before = save.units().count();
		let mut patched = save.clone();

		// A type not present in the save has no template to clone.
		assert!(
			add_unit(&mut patched, 0xFFFF, 0, 5, 5, None).expect("the tail follows").is_none(),
			"unknown type is refused"
		);

		// Add an engineer (type 0x3D, present in SAVE10) for team 0 at an empty cell.
		let id =
			add_unit(&mut patched, 0x3D, 0, 40, 40, None).expect("the tail follows").expect("engineer template exists");
		assert_eq!(id >> 13, 0, "id encodes team 0");
		assert!(save.units().all(|u| u.id != id), "the new id was free");

		let out = read_save_from_bytes(&write_save(&patched).unwrap(), (save.width, save.height));
		assert_eq!(out.units().count(), before + 1, "one unit added");
		assert_eq!(out.units().filter(|u| u.id == id).count(), 1, "id is unique");
		let u = out.units().find(|u| u.id == id).expect("new unit present");
		assert_eq!(u.unit_type, 0x3D, "type set");
		assert_eq!(u.team, 0, "team set");
		assert_eq!((u.grid_x, u.grid_y), (40, 40), "positioned");
		assert!(u.name.is_empty(), "name cleared");
		assert!(u.base_values.and_then(|b| out.values(b)).is_some(), "resolves shared team stats");

		// It is hashed at its cell.
		let slot = out.objects.iter().position(|o| matches!(o, SaveObject::Unit(uu) if uu.id == id)).unwrap();
		let hashed = out.map_hash.buckets.iter().flatten().any(|c| (c.x, c.y) == (40, 40) && c.units.contains(&slot));
		assert!(hashed, "new unit is in the map hash at its cell");
		// And in the unit hash bucket for its id.
		assert!(out.unit_hash[id as usize % 512].contains(&slot), "new unit is in its unit-hash bucket");
	}
}
