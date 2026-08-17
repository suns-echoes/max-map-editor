//! The save-editor session half of [`Project`]: attaching an opened .DTA,
//! synthesizing one from scratch, save settings swap, the byte-exact export
//! passes (in-place patch vs graph-reshaping rebuild, export-onto-base), and
//! the integrity/unexported-edits reports - plus the free helpers that seed
//! the object model from a save and add placed objects onto a file. Split
//! from `project/mod.rs` (audit 2026-08-07): this block imported ~25
//! `max_assets::save` symbols nothing else in the file used.

use super::*;
use max_assets::save::{
	FreshBodyCtx, TransientIssue, UnitScalarEdit, add_unit, apply_stat_override, check_complexes,
	check_transient_state, dead_listed_complexes, move_unit, patch_unit_scalars, remove_unit, repair_complexes,
	repair_mining, repair_transient_state, write_save,
};

/// Seed the editable [`MapObject`] list from an opened save's unit records:
/// every on-map, non-particle unit, ground cover first (so it layers beneath
/// the units it sits under, matching the render order). Off-map (`grid < 0`)
/// and particle (FX, not placeable) records are dropped. The gameplay fields
/// are captured into [`ObjectProps`] so nothing is lost across a save/reload.
pub(crate) fn objects_from_save(file: &SaveFile) -> Vec<MapObject> {
	file.lists()
		.into_iter()
		.filter(|(name, _)| *name != "particles")
		.flat_map(|(_, list)| list.iter().copied())
		.filter_map(|idx| file.unit(idx))
		.filter(|u| u.grid_x >= 0 && u.grid_y >= 0)
		.map(|u| MapObject {
			unit_type: u.unit_type,
			x: u.grid_x as u16,
			y: u.grid_y as u16,
			team: u.team,
			props: ObjectProps {
				name: u.name.clone(),
				angle: u.angle,
				turret_angle: u.turret_angle,
				hits: u.hits,
				ammo: u.ammo,
				orders: u.orders,
				disabled_turns: u.disabled_turns,
				storage: u.storage,
				connectors: u.connectors,
				source_id: Some(u.id),
				// No override at seed time: unedited units inherit the save's shared
				// `base_values` on the fly (`Project::object_base_values`), so the
				// model stays lean and "no override" is the natural default.
				base_values: None,
			},
		})
		.collect()
}

/// Add one placed (source-less) object onto `file` by cloning a same-type
/// template and carrying over the placed unit's edited props. Returns
/// `Ok(false)` (nothing added) when the type has no template in `file`; `Err`
/// when a per-unit stat override could not move the save's tail with it. Shared
/// by [`Project::export_save`]'s add pass and [`Project::export_onto_base`].
fn add_placed_object(file: &mut SaveFile, obj: &MapObject, fresh: Option<&FreshBodyCtx>) -> Result<bool, String> {
	let Some(new_id) = add_unit(file, obj.unit_type, obj.team, obj.x, obj.y, fresh)? else { return Ok(false) };
	// `add_unit` fills full HP + defaults; carry over the placed unit's edits.
	// `hits == 0` means "use max", so keep the added unit's HP in that case.
	let hits =
		if obj.props.hits == 0 { file.units().find(|u| u.id == new_id).map_or(0, |u| u.hits) } else { obj.props.hits };
	patch_unit_scalars(
		file,
		&UnitScalarEdit {
			id: new_id,
			team: obj.team,
			name: &obj.props.name,
			angle: obj.props.angle,
			turret_angle: obj.props.turret_angle,
			hits,
			ammo: obj.props.ammo,
			orders: obj.props.orders,
			disabled_turns: obj.props.disabled_turns,
			storage: obj.props.storage,
			connectors: obj.props.connectors,
		},
	);
	if let Some(values) = &obj.props.base_values {
		apply_stat_override(file, new_id, values)?;
	}
	Ok(true)
}

// The connector half-edge geometry (`CONNECTOR_BITS`, `connector_neighbor`)
// lives in `max_assets::save::complexes` — the engine-transcribed source both
// the auto-connect and the save-side complex repair share.

impl Project {
	/// Attaches an opened M.A.X. save (its raw `.DTA` bytes) to this project,
	/// decoding it at the project's dimensions. The project must already be built
	/// on the save's world so `width`×`height` match the map the save was authored
	/// on — the `.DTA` does not store map dimensions (`SAVE-EDITOR.md` S1.3). A
	/// decode failure (wrong world / corrupt file) is returned, not stored.
	/// Synthesize a fresh, loadable **V71 save from this project alone** — no
	/// base `.DTA` (Stage C3, `SAVE-FROM-SCRATCH.md`). The placed objects
	/// become the save's units (each team owning one becomes a human player),
	/// the terrain's passability becomes the engine `surface_map`
	/// (`PassData[]`, max-port `world.cpp:382`: 0 land→1, 1 water→2,
	/// 2 shore→4, 3 blocked→8), the project's cargo map (or zeros) becomes the
	/// resource map, and the result is **attached** like an opened save — so
	/// resources become editable and File ▸ Export Save File works
	/// immediately. Replaces any currently attached save; not undoable (like
	/// opening a save).
	pub fn synthesize_save(
		&mut self,
		opts: &SynthesizeSaveOptions,
		db: &max_assets::attribs::UnitStatsDb,
		frames: &[Option<max_assets::attribs::FrameInfo>; max_assets::save::UNIT_END],
	) -> Result<SynthesisSummary, String> {
		use max_assets::save::encode::{SynthUnit, SynthesisParams, synthesize_save_bytes};
		// Teams: every slot 0-3 owning an object plays; slot 4 (alien) can't.
		let mut team_types = [0u8; 5];
		for obj in &self.objects {
			match obj.team {
				0..=3 => team_types[obj.team as usize] = 1,
				_ => {
					return Err("synthesize: the alien slot cannot be a player - reassign its units".into());
				}
			}
		}
		if team_types.iter().all(|&t| t == 0) {
			return Err("synthesize: place at least one unit first (a save needs a player team)".into());
		}
		let team_clans = opts.team_clans;
		let team_names: [String; 5] =
			std::array::from_fn(|s| if team_types[s] == 1 { format!("Player {}", s + 1) } else { String::new() });
		// Surface map from per-cell passability via the engine's PassData table.
		const PASS_DATA: [u8; 4] = [1, 2, 4, 8]; // land, water, coast, air/blocked
		let cells = self.width as usize * self.height as usize;
		let mut surface_map = Vec::with_capacity(cells);
		for y in 0..self.height {
			for x in 0..self.width {
				let pass = self.pass_at(x, y).unwrap_or(0).min(3);
				surface_map.push(PASS_DATA[pass as usize]);
			}
		}
		let cargo_map = if self.cargo_map.len() == cells { self.cargo_map.clone() } else { vec![0u16; cells] };
		// A mining station placed over resources starts mining them, exactly
		// like the engine's own deploy (`UnitsManager_SetInitialMining`,
		// `units_manager.cpp:1410`) — the derivation + greedy split shared with
		// the save editor's placement and repaint passes (`max_assets::save::mining`).
		let mining_for_station = |gx: u16, gy: u16| -> [u8; 7] {
			let (raw, gold, fuel) = max_assets::save::derive_mining(
				&cargo_map,
				&surface_map,
				(self.width, self.height),
				gx as i32,
				gy as i32,
			);
			max_assets::save::mining_bytes(raw, gold, fuel)
		};
		let miningst = max_assets::save::unit_type_id("MININGST");
		let units: Vec<SynthUnit> = self
			.objects
			.iter()
			.map(|o| SynthUnit {
				unit_type: o.unit_type,
				grid_x: o.x as i16,
				grid_y: o.y as i16,
				team: o.team,
				angle: o.props.angle,
				turret_angle: o.props.turret_angle,
				name: o.props.name.clone(),
				orders: o.props.orders,
				disabled_turns: o.props.disabled_turns,
				hits: (o.props.hits > 0).then_some(o.props.hits),
				ammo: (o.props.ammo > 0).then_some(o.props.ammo),
				storage: (o.props.storage != 0).then_some(o.props.storage),
				connectors: o.props.connectors,
				base_values: o.props.base_values.clone(),
				mining: if Some(o.unit_type) == miningst { mining_for_station(o.x, o.y) } else { [0; 7] },
			})
			.collect();
		let unit_count = units.len();
		let params = SynthesisParams {
			save_name: opts.save_name.clone(),
			world_hash: max_assets::save::stock_world_hash(opts.world_index).map(str::to_string),
			world: opts.world_index as i32,
			width: self.width,
			height: self.height,
			rng_seed: opts.rng_seed,
			team_types,
			team_clans,
			team_names,
			start_gold: opts.start_gold,
			surface_map,
			cargo_map,
			units,
		};
		let bytes = synthesize_save_bytes(&params, db, frames)?;
		let size = bytes.len();
		self.attach_save(bytes)?;
		Ok(SynthesisSummary { bytes: size, units: unit_count, teams: team_types.iter().filter(|&&t| t == 1).count() })
	}

	pub fn attach_save(&mut self, raw: Vec<u8>) -> Result<(), String> {
		let file = read_save_bytes(&raw, (self.width, self.height)).map_err(|e| format!("decode save: {e}"))?;
		// Seed the editable object model from the save's units (the model owns
		// them so edits are undoable); the raw `.DTA` stays the export anchor.
		self.objects = objects_from_save(&file);
		// Seed the editable resource map from the save's pristine cargo map (S5);
		// the `.json` later persists only the diff against this seed.
		self.cargo_map = file.cargo_map.clone();
		self.save = Some(EmbeddedSave { raw, file });
		// Attaching a save re-seeds `objects` and `cargo_map` wholesale, so every
		// patch already on the journal describes a *different* document. Object
		// patches swap the whole vector, so one undo across this boundary would
		// restore the pre-attach list - the save's seeded units (`source_id`s and
		// all) would vanish, and `export_save`'s delete pass would then remove
		// every one of them from the file as "deleted by the user". Same reasoning
		// as `resize`: a structural change can't be a per-cell patch, so the
		// journal goes.
		self.undo_stack.clear();
		self.redo_stack.clear();
		self.stroke = None;
		Ok(())
	}

	/// The attached save's editable non-map settings (S7.2), when one is open.
	pub fn save_settings(&self) -> Option<SaveSettings> {
		self.save.as_ref().map(|s| SaveSettings::extract(&s.file))
	}

	/// Apply an edited settings block to the attached save. Undoable. `Err`
	/// when no save is attached or when a settings region fails the lossless
	/// re-encode guard (re-encoding it would silently drop unmodeled bytes —
	/// refuse rather than risk the user's save; also covers `V70`, which is
	/// never re-encoded). A block equal to the current settings is a no-op
	/// (no undo entry).
	///
	/// On success the embedded raw anchor is rebased to the re-emitted bytes,
	/// so export, project persistence and the S6.6 write-safety guard all see
	/// the edit as if the save had been opened with it.
	pub fn apply_save_settings(&mut self, settings: &SaveSettings) -> Result<(), String> {
		let embedded = self.save.as_ref().ok_or("no save open (open a `.DTA` first)")?;
		if !embedded.file.settings_regions_lossless() {
			return Err("refusing to edit: a settings region of this save did not re-encode losslessly (an \
			            unmodeled byte would be lost)"
				.into());
		}
		// The upgrades part of a settings block edits the object graph, so the
		// whole file must round-trip too (the S6.6 export guard, run up front:
		// rebasing the anchor onto a lossy re-emit would corrupt silently).
		if write_save(&embedded.file)? != embedded.raw {
			return Err("refusing to edit: this save did not round-trip losslessly (an unmodeled region would \
			            be lost)"
				.into());
		}
		if SaveSettings::extract(&embedded.file) == *settings {
			return Ok(());
		}
		let before = self.swap_save_settings(settings)?.expect("a save is attached");
		self.push_undo(Patch { save_settings: Some(Box::new(before)), ..Patch::default() });
		self.redo_stack.clear();
		self.bump();
		Ok(())
	}

	/// Write `settings` into the attached save and rebase the raw anchor,
	/// returning the settings block it displaced — the shared primitive under
	/// [`Self::apply_save_settings`] and the undo/redo swap. `None` when no
	/// save is attached (nothing to do).
	pub(super) fn swap_save_settings(&mut self, settings: &SaveSettings) -> Result<Option<SaveSettings>, String> {
		let Some(embedded) = self.save.as_mut() else { return Ok(None) };
		let displaced = SaveSettings::extract(&embedded.file);
		settings.apply_to(&mut embedded.file)?;
		embedded.raw = write_save(&embedded.file)?;
		Ok(Some(displaced))
	}

	/// Reconstitute the opened save (`.DTA`) as a byte stream that carries this
	/// project's edits — the Export Save File path (S6). Clones the retained
	/// [`SaveFile`], flushes the edited resource (cargo) map, then in passes:
	/// (1) patches each existing unit's scalar props + applies a **move**
	/// (grid/pixel + `Hash_MapHash` re-key) when its cell diverged; (2) applies
	/// per-unit **max-stat overrides** (inline `UnitValues`); (3) **removes**
	/// deleted units; (4) **adds** placed units by cloning a same-type template.
	/// Then re-serializes with [`max_assets::save::write_save`].
	///
	/// A placed unit whose type has no same-type template in the save gets a
	/// **fresh, from-scratch body** when `fresh` supplies the runtime unit
	/// database + frame table (`V71` saves); without them (or on `V70`) it is
	/// skipped and named in the returned drop list. An unedited save exports
	/// identical to the original file.
	///
	/// Returns the exported bytes plus one `TYPE at x,y` entry per placement
	/// that could **not** be exported — surface these loudly; an empty list
	/// means the export is faithful.
	///
	/// `Err` when no save is attached, or when the retained save fails the
	/// write-safety guard ([`Self::save_exports_losslessly`]).
	pub fn export_save_with(&self, fresh: Option<&FreshBodyCtx>) -> Result<(Vec<u8>, Vec<String>), String> {
		let embedded = self.save.as_ref().ok_or("no save attached to export")?;

		// Write-safety guard (S6.6): only export when the *unedited* retained save
		// re-serializes byte-for-byte (S0.4). If it doesn't, the decoder didn't
		// model this file losslessly, so an export could silently corrupt it —
		// refuse rather than risk the user's save.
		if write_save(&embedded.file)? != embedded.raw {
			return Err(
				"refusing to export: this save did not round-trip losslessly (an unmodeled region would be lost)"
					.into(),
			);
		}
		// Second write-safety guard: adding or removing an object renumbers the
		// object references the retained tail's message logs and AI state hold, so
		// the tail has to be walkable for those edits to be safe. Refuse up front
		// rather than let a pass fail halfway.
		if self.export_reshapes_the_graph(&embedded.file) && !embedded.file.tail_follows_the_graph() {
			return Err("refusing to export: this save's message-log / AI state will not decompose, so the object \
			            references in it cannot follow a unit being added or removed (edits to the units already \
			            there are still fine)"
				.into());
		}

		let mut file = embedded.file.clone();

		// Flush the edited resource map; `write_save` serializes it from the model.
		// Lengths always match (both are width×height), but guard defensively.
		if self.cargo_map.len() == file.cargo_map.len() {
			file.cargo_map.clone_from(&self.cargo_map);
		}

		// Patch each edited existing unit (added units carry no source id and are
		// handled elsewhere in S6.2): its scalar props, then — if it was moved off
		// its original cell — its grid/pixel position + map-hash key.
		for obj in &self.objects {
			let Some(id) = obj.props.source_id else { continue };
			let edit = UnitScalarEdit {
				id,
				team: obj.team,
				name: &obj.props.name,
				angle: obj.props.angle,
				turret_angle: obj.props.turret_angle,
				hits: obj.props.hits,
				ammo: obj.props.ammo,
				orders: obj.props.orders,
				disabled_turns: obj.props.disabled_turns,
				storage: obj.props.storage,
				connectors: obj.props.connectors,
			};
			patch_unit_scalars(&mut file, &edit);

			// Position: `objects_from_save` seeds `x`/`y` from the unit's grid, so a
			// divergence means the user moved it.
			if let Some(u) = embedded.file.units().find(|u| u.id == id) {
				if u.grid_x as u16 != obj.x || u.grid_y as u16 != obj.y {
					move_unit(&mut file, id, obj.x, obj.y);
				}
			}
		}

		// Second pass: per-unit max-stat overrides (S4.5). Each inserts an inline
		// `UnitValues` into the object graph and shifts later indices, so it runs
		// after the in-place scalar/move edits (which look units up by id anyway).
		for obj in &self.objects {
			let Some(id) = obj.props.source_id else { continue };
			if let Some(values) = &obj.props.base_values {
				apply_stat_override(&mut file, id, values)?;
			}
		}

		// Third pass: delete units the user removed — seeded on-map units whose id is
		// no longer present in the object model.
		let present: std::collections::HashSet<u16> = self.objects.iter().filter_map(|o| o.props.source_id).collect();
		for id in objects_from_save(&embedded.file).iter().filter_map(|o| o.props.source_id) {
			if !present.contains(&id) {
				remove_unit(&mut file, id)?;
			}
		}

		// Fourth pass: add units the user placed since opening (no source id).
		// Each clones a same-type template already in the save, or - with the
		// runtime unit database at hand - synthesizes a fresh deploy-state body.
		// Whatever still cannot export is named in the returned drop list so the
		// caller can surface it loudly. The new unit inherits the placed
		// object's edited props.
		let mut dropped = Vec::new();
		for obj in &self.objects {
			if obj.props.source_id.is_some() {
				continue;
			}
			if !add_placed_object(&mut file, obj, fresh)? {
				let name = max_assets::save::unit_type_name(obj.unit_type).unwrap_or("?");
				dropped.push(format!("{name} at {},{}", obj.x, obj.y));
			}
		}

		// Complex pass (HANDOFF 2026-08-02 Finding 1): every placed, team-edited,
		// bridged or split connector host ends with an engine-valid `Complex` —
		// the engine dereferences a host's complex unguarded, and nothing at load
		// time repairs a null. Runs after the passes above so the final connector
		// masks, teams and membership are what it sees; a no-op (nothing written)
		// when the save already satisfies the invariant. Dead listed complexes the
		// pristine save carried are tolerated drift and stay byte-identical.
		repair_complexes(&mut file, &dead_listed_complexes(&embedded.file))?;

		// Mining pass (HANDOFF Finding 3): a station whose footprint resources an
		// edit changed (repainted under, or moved onto different ground) re-derives
		// its stored production the way the engine's deploy does — the engine never
		// re-derives at load, so stale bytes would mine ground that no longer
		// exists. Event-driven against the pristine save: untouched stations (stock
		// drift and player allocations included) stay byte-identical.
		repair_mining(&mut file, &embedded.file);

		// Correctness pass before writing (`save-editor-bug.md`): repair any unit left
		// in an impossible idle+in-progress transient state — chiefly units an older
		// editor placed by cloning a mid-build template. Placements added above are
		// already idle-valid (`add_unit`), so on a save authored here this is a no-op.
		repair_transient_state(&mut file);

		Ok((write_save(&file)?, dropped))
	}

	/// [`Self::export_save_with`] without a fresh-body context — placements
	/// with no template are dropped (and reported by [`Self::unexported_edits`]).
	pub fn export_save(&self) -> Result<Vec<u8>, String> {
		self.export_save_with(None).map(|(bytes, _)| bytes)
	}

	/// Transient-state corruption in the attached save (`save-editor-bug.md`): units
	/// carrying an impossible idle+in-progress state — the fingerprint of a unit an
	/// older editor placed by cloning a mid-build template. [`Self::export_save`]
	/// repairs these before writing; this read-only check lets a caller warn on open
	/// and note the repair on export. Empty for a clean save (and when none is open).
	pub fn save_integrity_issues(&self) -> Vec<TransientIssue> {
		self.save.as_ref().map(|s| check_transient_state(&s.file)).unwrap_or_default()
	}

	/// Complex-invariant violations in the attached save (HANDOFF 2026-08-02
	/// Finding 1) — e.g. a building an older editor exported with a null
	/// `Complex`, which the engine dereferences unguarded. [`Self::export_save`]
	/// repairs these before writing; this read-only check lets a caller warn on
	/// open. Empty for a clean save (and when none is open).
	pub fn save_complex_issues(&self) -> Vec<String> {
		self.save.as_ref().map(|s| check_complexes(&s.file)).unwrap_or_default()
	}

	/// Whether exporting would add or remove an object in the graph — the edits
	/// whose index renumbering the retained tail has to follow. A scalar or
	/// position edit does not move the graph, so it needs nothing of the tail.
	fn export_reshapes_the_graph(&self, file: &SaveFile) -> bool {
		// A placement adds; a per-unit stat override may insert an inline
		// `UnitValues`; a seeded unit that is no longer in the model was deleted.
		if self.objects.iter().any(|o| o.props.source_id.is_none() || o.props.base_values.is_some()) {
			return true;
		}
		let present: std::collections::HashSet<u16> = self.objects.iter().filter_map(|o| o.props.source_id).collect();
		objects_from_save(file).iter().filter_map(|o| o.props.source_id).any(|id| !present.contains(&id))
	}

	/// Export this project's placed units onto a **base save** the caller supplies —
	/// the "save a normal map as a `.DTA`" path, used when no save is attached to the
	/// project (so [`Self::export_save`] has nothing to clone from). The base provides
	/// the whole game-state skeleton, resource map, and terrain (kept verbatim); each
	/// placed unit is added by cloning a same-type template already present in the base
	/// (exactly like `export_save`'s add pass). Returns the `.DTA` bytes and the number
	/// of placements skipped because their type has no template in the base.
	///
	/// The base must decode at this project's `width`×`height` (the `.DTA` carries no
	/// dimensions — it must have been authored on a same-size world) and re-serialize
	/// losslessly (the same write-safety guard as [`Self::export_save`]); otherwise
	/// `Err`. The project's own terrain edits are **not** written — the exported save
	/// keeps the base's world.
	pub fn export_onto_base(
		&self,
		base_raw: &[u8],
		fresh: Option<&FreshBodyCtx>,
	) -> Result<(Vec<u8>, Vec<String>), String> {
		let base = read_save_bytes(base_raw, (self.width, self.height))
			.map_err(|e| format!("decode base save (must be a {}x{} world): {e}", self.width, self.height))?;
		// Write-safety guard (S6.6), as in `export_save`: only build on a base that
		// itself round-trips byte-for-byte, so we never silently drop an unmodeled
		// region of the user's chosen save.
		if write_save(&base)?.as_slice() != base_raw {
			return Err(
				"refusing to export: the base save did not round-trip losslessly (an unmodeled region would be lost)"
					.into(),
			);
		}
		// Every placement adds an object, which renumbers the references the base's
		// message logs and AI state hold - so its tail has to be walkable.
		if !self.objects.is_empty() && !base.tail_follows_the_graph() {
			return Err("refusing to export: the base save's message-log / AI state will not decompose, so the \
			            object references in it cannot follow a unit being added"
				.into());
		}
		let mut file = base.clone();
		let mut dropped = Vec::new();
		for obj in &self.objects {
			if !add_placed_object(&mut file, obj, fresh)? {
				let name = max_assets::save::unit_type_name(obj.unit_type).unwrap_or("?");
				dropped.push(format!("{name} at {},{}", obj.x, obj.y));
			}
		}
		// Complex pass (HANDOFF Finding 1), as in `export_save`: every placed
		// connector host gets an engine-valid `Complex` before writing.
		repair_complexes(&mut file, &dead_listed_complexes(&base))?;
		// Mining pass (HANDOFF Finding 3), as in `export_save`: a placed station's
		// production is derived off the base's ground (a no-op re-derivation —
		// `add_unit` already ran it; the base's own stations never fire the event).
		repair_mining(&mut file, &base);
		// Correctness pass (`save-editor-bug.md`), as in `export_save`: repair any unit
		// left in an impossible idle+in-progress transient state before writing.
		repair_transient_state(&mut file);
		Ok((write_save(&file)?, dropped))
	}

	/// Whether the attached save re-serializes byte-for-byte from the decoded
	/// model — the write-safety guard [`Self::export_save`] enforces (S6.6). `true`
	/// for every save the decoder models losslessly (all stock saves, per S0.4);
	/// `false` (export refused) only if a file carried a region the model dropped.
	/// `false` too when no save is attached (nothing to export).
	pub fn save_exports_losslessly(&self) -> bool {
		self.save.as_ref().is_some_and(|s| write_save(&s.file).is_ok_and(|b| b == s.raw))
	}

	/// The edits an [`Self::export_save`] cannot represent, so a caller can warn
	/// rather than drop them silently. Scalar edits, moves, stat overrides, unit
	/// removals, and placements of a type already in the save all export; the only
	/// gap is a placed unit whose type has **no same-type template** in the save to
	/// clone a body from. All-zero when the export is faithful.
	pub fn unexported_edits(&self) -> UnexportedEdits {
		let Some(embedded) = self.save.as_ref() else { return UnexportedEdits::default() };
		let mut report = UnexportedEdits::default();
		// The add pass clones from the save as it stands AFTER the removal pass,
		// so a template the user deleted or restamped over (its seeded object is
		// gone from the model) is not clonable — even though the pristine save
		// still lists it. Anything `objects_from_save` never seeded (off-map,
		// particles) is never removed and stays clonable.
		let present: std::collections::HashSet<u16> = self.objects.iter().filter_map(|o| o.props.source_id).collect();
		let removed: std::collections::HashSet<u16> = objects_from_save(&embedded.file)
			.iter()
			.filter_map(|o| o.props.source_id)
			.filter(|id| !present.contains(id))
			.collect();
		for obj in &self.objects {
			if obj.props.source_id.is_none()
				&& !embedded.file.units().any(|u| u.unit_type == obj.unit_type && !removed.contains(&u.id))
			{
				let name = max_assets::save::unit_type_name(obj.unit_type).unwrap_or("?");
				report.added.push(format!("{name} at {},{}", obj.x, obj.y));
			}
		}
		report
	}
}
