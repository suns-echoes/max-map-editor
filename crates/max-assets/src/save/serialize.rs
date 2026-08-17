//! Byte-exact re-serializer for M.A.X. save files (the inverse of
//! [`super::decode::read_save`]).
//!
//! Reconstructs the exact original byte stream from the typed model plus the
//! retained raw regions ([`RawRegions`](super::types::RawRegions) /
//! [`ObjMeta`](super::types::ObjMeta)). The shared object graph is rebuilt
//! structurally: because the reader consumes the file strictly in order, the
//! writer emits objects in the same order and reproduces every index/back-
//! reference. Fields the decoder skips (unit display state, most of `CTInfo`,
//! the header) are re-emitted from their retained bytes.
//!
//! This is the S0.4 write-back foundation. Editing hooks (patch a scalar field,
//! add/remove units + rebuild the hashes) build on top of it; today it proves
//! `read_save → write_save == original` for real saves.

use super::error::EditError;
use super::types::{Complex, ObjMeta, SaveFile, SaveFormat, SaveObject, UnitValues};

/// Serialize a [`UnitValues`] to its 28-byte `FileSave` body (`unitvalues.cpp`) —
/// the leaf body used when emitting a per-unit `base_values` override (S6.2). The
/// layout is identical in `V70` and `V71` (the runtime `fuel` field is not
/// serialized); the trailing byte is `is_in_use` (`V71`) / a legacy `units_built`
/// count (`V70`), both non-zero meaning "in use".
pub fn serialize_unit_values(v: &UnitValues) -> Vec<u8> {
	let mut b = Vec::with_capacity(28);
	for x in [v.turns, v.hits, v.armor, v.attack, v.speed, v.range, v.rounds] {
		b.extend_from_slice(&x.to_le_bytes());
	}
	b.push(v.move_and_fire);
	for x in [v.scan, v.storage, v.ammo, v.attack_radius, v.agent_adjust, v.version] {
		b.extend_from_slice(&x.to_le_bytes());
	}
	b.push(v.in_use as u8);
	b
}

/// Serialize a [`Complex`] to its 14-byte `FileSave` body (`complex.cpp`): seven
/// `i16`s in disk order `material, fuel, gold, power, workers, buildings, id` —
/// identical in `V70` and `V71`. Used when the complex-invariant repair
/// (`crate::save::complexes`) creates or updates a complex.
pub fn serialize_complex(c: &Complex) -> Vec<u8> {
	let mut b = Vec::with_capacity(14);
	for x in [c.material, c.fuel, c.gold, c.power, c.workers, c.buildings, c.id] {
		b.extend_from_slice(&x.to_le_bytes());
	}
	b
}

/// Writer state: the growing output, the format (index/count width), the object
/// graph (records + re-serialization metadata), and the first-seen bookkeeping.
///
/// On-disk object indices are assigned by **encounter order** (`next_emit`), not
/// by position in `records`: `emitted[vec_index]` records the on-disk index an
/// object got the first time it was written. This decouples the emit order from
/// the model's vector order, so an edit may insert / remove / reorder objects
/// freely — the walk still produces a consistent `SmartFileWriter` index space.
/// For an unedited save the walk encounters objects in their original order, so
/// the output stays byte-identical.
pub(crate) struct Writer<'a> {
	out: Vec<u8>,
	format: SaveFormat,
	records: &'a [SaveObject],
	object_meta: &'a [ObjMeta],
	emitted: Vec<Option<usize>>,
	next_emit: usize,
}

impl<'a> Writer<'a> {
	/// A writer for the save's **tail**, which is emitted after the graph and so
	/// continues its index space: `emitted` already holds the on-disk index every
	/// graph object got (`None` = the object is gone, and a reference to it is
	/// emitted as null), `next_emit` is where the tail's own inline bodies carry
	/// on from. `records`/`object_meta` must cover the graph slots (whose bodies
	/// are never re-emitted, being back-references) followed by the objects the
	/// tail inlines.
	pub(crate) fn for_tail(
		records: &'a [SaveObject],
		object_meta: &'a [ObjMeta],
		format: SaveFormat,
		emitted: Vec<Option<usize>>,
		next_emit: usize,
	) -> Self {
		Writer { out: Vec::new(), format, records, object_meta, emitted, next_emit }
	}

	pub(crate) fn into_vec(self) -> Vec<u8> {
		self.out
	}

	/// Copy bytes through untouched — the spans between reference sites.
	pub(crate) fn raw(&mut self, b: &[u8]) {
		self.bytes(b);
	}

	/// A format-width count (`WriteObjectCount`).
	pub(crate) fn count(&mut self, n: u32) {
		self.wide(n);
	}

	/// One `WriteObject` — see [`Self::object`].
	pub(crate) fn reference(&mut self, r: Option<usize>) {
		self.object(r);
	}

	fn u16(&mut self, v: u16) {
		self.out.extend_from_slice(&v.to_le_bytes());
	}

	fn u32(&mut self, v: u32) {
		self.out.extend_from_slice(&v.to_le_bytes());
	}

	fn bytes(&mut self, b: &[u8]) {
		self.out.extend_from_slice(b);
	}

	/// A format-width value — `u16` in `V70`, `u32` in `V71`. Mirrors
	/// `WriteIndex` / `WriteObjectCount` and the widened scalars (team gold).
	fn wide(&mut self, v: u32) {
		match self.format {
			SaveFormat::V70 => self.u16(v as u16),
			SaveFormat::V71 => self.u32(v),
		}
	}

	/// `SmartFileWriter::WriteObject` mirror. `None` → a null reference. An
	/// already-emitted object → its 1-based index (a back-reference, no body). The
	/// next-new object → its index, its type index, then its body.
	///
	/// Leaf objects (`UnitValues`/`Complex`/`UnitPath` — no nested references) emit
	/// their body verbatim. A `UnitInfo` instead emits its opaque prefix verbatim
	/// then re-emits its object references **symbolically** ([`Self::emit_unit_refs`]),
	/// so inline (forward/cyclic) nested objects recurse through `object` and every
	/// index is recomputed from the current model — the property that lets a
	/// unit's fields be edited even when it is nested inside another unit's body,
	/// and that lets objects be added/removed.
	fn object(&mut self, r: Option<usize>) {
		let (records, metas) = (self.records, self.object_meta); // slice refs, decoupled from `self`
		let i = match r {
			None => return self.wide(0),
			Some(i) => i,
		};
		if let Some(idx) = self.emitted[i] {
			// Already written earlier in the walk — emit a back-reference.
			return self.wide(idx as u32 + 1);
		}
		// First encounter: assign the next on-disk index, then emit the body.
		let idx = self.next_emit;
		self.emitted[i] = Some(idx);
		self.next_emit += 1;
		self.wide(idx as u32 + 1);
		self.wide(metas[i].type_index);
		match &records[i] {
			SaveObject::Unit(u) => {
				let layout = metas[i].unit_layout.as_ref().expect("a unit body carries its field layout");
				self.bytes(&metas[i].body_raw[..layout.refs_off]);
				self.emit_unit_refs(u);
			}
			// A leaf: no nested objects, so the body is re-emitted verbatim.
			_ => self.bytes(&metas[i].body_raw),
		}
	}

	/// Re-emit a unit's object-reference section (everything after the opaque
	/// prefix) from the typed [`UnitRecord`]: `path`, the interleaved `connectors`
	/// mask, `base_values`, `complex`, `parent_unit`, `enemy_unit`, then the build
	/// list. Each reference goes through [`Self::object`], so a nested inline object
	/// is emitted from its own model at the right stream position.
	fn emit_unit_refs(&mut self, u: &super::types::UnitRecord) {
		self.object(u.path);
		self.u16(u.connectors);
		self.object(u.base_values);
		self.object(u.complex);
		self.object(u.parent_unit);
		self.object(u.enemy_unit);
		self.wide(u.build_list.len() as u32);
		for &id in &u.build_list {
			self.u16(id);
		}
	}
}

/// Re-serializes a decoded [`SaveFile`] back to its exact original byte stream.
///
/// Round-trips byte-for-byte for an unedited save; edits applied to the typed
/// model (or the retained raw regions) flow through unchanged. Object indices are
/// reproduced deterministically, so this is also the basis for add/remove once
/// the two spatial hashes are rebuilt (S6.2).
///
/// # Errors
///
/// Only when a graph-structural edit moved the objects the tail references and
/// that tail will not decompose - emitting it anyway would write references
/// pointing at the wrong units. Callers that want to refuse *before* editing
/// check [`SaveFile::tail_follows_the_graph`].
pub fn write_save(save: &SaveFile) -> Result<Vec<u8>, EditError> {
	Ok(serialize_with_hash_span(save)?.0)
}

/// Like [`write_save`], but also returns the byte range the two spatial-hash
/// regions (`Hash_UnitHash` + `Hash_MapHash`) occupy in the output. The focused
/// A4 test uses the span to assert just those regions re-emit byte-exact from the
/// structural model, isolating the hash emission from the rest of the file.
pub(crate) fn serialize_with_hash_span(save: &SaveFile) -> Result<(Vec<u8>, std::ops::Range<usize>), EditError> {
	let v71 = save.header.format == SaveFormat::V71;
	let mut out = Vec::new();

	// Header + options (verbatim), then V71's up-front extra-settings block.
	out.extend_from_slice(&save.raw.header);
	if v71 {
		out.extend_from_slice(&save.raw.extra_settings);
	}

	// Surface map (u8) then cargo/resource map (u16), from the model.
	out.extend_from_slice(&save.surface_map);
	for &v in &save.cargo_map {
		out.extend_from_slice(&v.to_le_bytes());
	}

	// Per-team CTInfo blocks (verbatim).
	for block in &save.raw.ct_info {
		out.extend_from_slice(block);
	}

	// Game scalars (verbatim; the V71 cheater pair is inside this block), then
	// V70's extra-settings block, which trails the scalars.
	out.extend_from_slice(&save.raw.scalars);
	if !v71 {
		out.extend_from_slice(&save.raw.extra_settings);
	}

	// Regions 19–22: the four team-unit tables, five unit lists, and both spatial
	// hashes, from the structural object graph.
	let graph_at = out.len();
	let (graph, span, order) = serialize_object_graph(save);
	out.extend_from_slice(&graph);

	// The tail (heat maps, message logs, AI). Its message-log and AI entries hold
	// object references numbered in the graph's index space, so they follow the
	// walk's answer rather than the model's vector order — see
	// [`super::tail::remap`]. The overwhelmingly common case is that the two agree
	// (nothing added or removed), and then the bytes are copied straight through.
	out.extend_from_slice(&super::tail::follow_graph(save, &order)?);

	Ok((out, (graph_at + span.start)..(graph_at + span.end)))
}

/// Emits regions 19–22 — the four `TeamUnits` tables, the five unit lists, and the
/// unit + map spatial hashes — from the structural object graph via the encounter-
/// order [`Writer`]. Returns the bytes plus the byte range the two hash regions
/// occupy *within them*. Shared by [`write_save`] (real-save re-emit) and
/// [`super::encode::encode_save`] (synthesis), which differ only in the regions
/// that surround the graph. The object-index space is self-contained here (it
/// starts fresh at the first `TeamUnits` reference), so the graph serializes the
/// same regardless of what precedes it.
pub(crate) fn serialize_object_graph(save: &SaveFile) -> (Vec<u8>, std::ops::Range<usize>, GraphOrder) {
	let mut w = Writer {
		out: Vec::new(),
		format: save.header.format,
		records: &save.objects,
		object_meta: &save.object_meta,
		emitted: vec![None; save.objects.len()],
		next_emit: 0,
	};

	// Team unit stat tables (four): gold (format-width), UNIT_END base + UNIT_END
	// current `UnitValues` references, then the complex list.
	for team in &save.team_units {
		w.wide(team.gold);
		for &r in &team.base_values {
			w.object(r);
		}
		for &r in &team.current_values {
			w.object(r);
		}
		w.wide(team.complexes.len() as u32);
		for &idx in &team.complexes {
			w.object(Some(idx));
		}
	}

	// The five unit lists, in load order.
	for list in [&save.ground_cover, &save.mobile_land_sea, &save.stationary, &save.mobile_air, &save.particles] {
		w.wide(list.len() as u32);
		for &idx in list {
			w.object(Some(idx));
		}
	}

	// Hash_UnitHash: a u16 bucket count then each bucket as a unit list (all
	// back-references to units already emitted above).
	let hash_start = w.out.len();
	w.u16(save.unit_hash.len() as u16);
	for bucket in &save.unit_hash {
		w.wide(bucket.len() as u32);
		for &idx in bucket {
			w.object(Some(idx));
		}
	}

	// Hash_MapHash: hash_size + x_shift, then each bucket as a list of occupied
	// cells (`{x, y, unit-list}`), the units being back-references. Emitted from the
	// structural model so a move/add/remove re-derives it.
	w.u16(save.map_hash.hash_size);
	w.u16(save.map_hash.x_shift);
	for bucket in &save.map_hash.buckets {
		w.wide(bucket.len() as u32);
		for cell in bucket {
			w.u16(cell.x);
			w.u16(cell.y);
			w.wide(cell.units.len() as u32);
			for &idx in &cell.units {
				w.object(Some(idx));
			}
		}
	}

	let hash_end = w.out.len();
	(w.out, hash_start..hash_end, GraphOrder { emitted: w.emitted, next_emit: w.next_emit })
}

/// Where the graph's objects ended up on disk — the answer the tail needs, since
/// its references are numbered in the same space.
///
/// The walk assigns indices by **encounter order**, which is not the model's
/// vector order once an object has been added or removed: appending a unit to a
/// list, for one, first-sees it partway through the walk and pushes every object
/// after it up. A tail emitted verbatim would then point at the wrong objects,
/// which is what [`super::tail::remap`] exists to prevent.
pub(crate) struct GraphOrder {
	/// Vector slot -> the 0-based on-disk index it was emitted at, or `None` for
	/// an object the walk never reached.
	pub(crate) emitted: Vec<Option<usize>>,
	/// How many objects the graph emitted — where the tail's inline bodies carry on.
	pub(crate) next_emit: usize,
}

impl GraphOrder {
	/// Whether the walk emitted every object exactly at its vector position — the
	/// unedited (and most edited) case, where the tail's references already point
	/// where they should and can be copied through untouched.
	pub(crate) fn is_identity(&self, objects: usize) -> bool {
		self.next_emit == objects && self.emitted.iter().enumerate().all(|(i, e)| *e == Some(i))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::save::read_save;

	/// Byte-exact round-trip of the real turn-3 autosave (V70), when present.
	#[test]
	fn round_trips_save10_when_present() {
		let Some(home) = std::env::var_os("HOME") else { return };
		let save_path = std::path::Path::new(&home).join("MAX/SAVE10.DTA");
		let wrl_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals/SNOW_1.WRL");
		if !save_path.is_file() || !wrl_path.is_file() {
			crate::testutil::skip_fixture("round_trips_save10_when_present: fixtures not found");
			return;
		}
		let original = std::fs::read(&save_path).unwrap();
		let wrl = crate::wrl::read_wrl_header(&wrl_path).unwrap();
		let save = read_save(&save_path, (wrl.width, wrl.height)).unwrap();
		let rebuilt = write_save(&save).unwrap();
		assert_eq!(rebuilt.len(), original.len(), "re-serialized length must match");
		assert!(rebuilt == original, "V70 re-serialize must be byte-identical to the original");
	}

	/// Every decoded [`UnitValues`] re-serializes to its exact original body bytes,
	/// so the per-unit stat-override export (S6.2) emits a leaf the game reads back
	/// identically. Checks against all the team-table stat blocks in SAVE10.
	#[test]
	fn unit_values_serialize_matches_decode_when_present() {
		let Some(home) = std::env::var_os("HOME") else { return };
		let save_path = std::path::Path::new(&home).join("MAX/SAVE10.DTA");
		let wrl_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals/SNOW_1.WRL");
		if !save_path.is_file() || !wrl_path.is_file() {
			crate::testutil::skip_fixture("unit_values_serialize_matches_decode_when_present: fixtures not found");
			return;
		}
		let wrl = crate::wrl::read_wrl_header(&wrl_path).unwrap();
		let save = read_save(&save_path, (wrl.width, wrl.height)).unwrap();
		let mut checked = 0;
		for (slot, obj) in save.objects.iter().enumerate() {
			if let SaveObject::Values(v) = obj {
				assert_eq!(serialize_unit_values(v), save.object_meta[slot].body_raw, "UnitValues body at slot {slot}");
				checked += 1;
			}
		}
		assert!(checked > 0, "SAVE10 has UnitValues objects to check");
	}

	/// Byte-exact round-trip of the V71 fixture, when present.
	#[test]
	fn round_trips_v71_fixture_when_present() {
		let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/saves");
		let save_path = base.join("save11-green3-50x50.dta");
		let wrl_path = base.join("GREEN_3-50x50.WRL");
		if !save_path.is_file() || !wrl_path.is_file() {
			crate::testutil::skip_fixture("round_trips_v71_fixture_when_present: fixtures not found");
			return;
		}
		let original = std::fs::read(&save_path).unwrap();
		let wrl = crate::wrl::read_wrl_header(&wrl_path).unwrap();
		let save = read_save(&save_path, (wrl.width, wrl.height)).unwrap();
		let rebuilt = write_save(&save).unwrap();
		assert_eq!(rebuilt.len(), original.len(), "re-serialized length must match");
		assert!(rebuilt == original, "V71 re-serialize must be byte-identical to the original");
	}

	/// Sweep: byte-exact round-trip of every `~/MAX` save whose world resolves to
	/// a bundled pristine `.WRL`. Broad confidence across unit counts (5–807),
	/// worlds, and mission categories — the same corpus the decoder was verified
	/// against, now proven to survive a full re-serialize unchanged.
	#[test]
	fn round_trips_all_max_saves_when_present() {
		let Some(home) = std::env::var_os("HOME") else { return };
		let max_dir = std::path::Path::new(&home).join("MAX");
		let originals = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals");
		if !max_dir.is_dir() || !originals.is_dir() {
			crate::testutil::skip_fixture("round_trips_all_max_saves_when_present: fixtures not found");
			return;
		}
		let Ok(entries) = std::fs::read_dir(&max_dir) else { return };
		let mut checked = 0;
		for entry in entries.flatten() {
			let path = entry.path();
			let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_uppercase);
			if !matches!(ext.as_deref(), Some("DTA" | "CAM" | "SCE" | "TRA" | "MPS" | "DMO")) {
				continue;
			}
			// Resolve dimensions from the pristine bundled world (the save was
			// authored against it; the ~/MAX install copy may have diverged).
			let Ok(header) = crate::save::read_save_header(&path) else { continue };
			let Some(world_file) = header.world_file else { continue };
			let wrl_path = originals.join(world_file);
			let Ok(wrl) = crate::wrl::read_wrl_header(&wrl_path) else { continue };
			let Ok(save) = read_save(&path, (wrl.width, wrl.height)) else { continue };

			let original = std::fs::read(&path).unwrap();
			assert!(write_save(&save).unwrap() == original, "byte-exact round-trip failed for {}", path.display());
			checked += 1;
		}
		assert!(checked > 0, "no ~/MAX saves were round-tripped");
		eprintln!("round-tripped {checked} ~/MAX saves byte-exactly");
	}

	/// The static [`is_connector_host_type`] id set must agree with the engine's
	/// own flag rule — `(CONNECTOR_UNIT | BUILDING | STANDALONE) && !GROUND_COVER`
	/// (`units_manager.cpp`) — for every unit in every stock save, and it must be
	/// a superset of "actually carries a non-zero mask" (some hosts store 0). This
	/// is what makes the hand-transcribed id ranges trustworthy (S4.4). Also
	/// asserts the mask is 8-bit, as the editor's per-side toggles assume.
	#[test]
	fn connector_host_matches_flag_rule() {
		use crate::save::is_connector_host_type;
		use crate::save::unit_types::flag::{BUILDING, CONNECTOR_UNIT, GROUND_COVER, STANDALONE};
		let Some(home) = std::env::var_os("HOME") else { return };
		let max_dir = std::path::Path::new(&home).join("MAX");
		let originals = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals");
		if !max_dir.is_dir() || !originals.is_dir() {
			crate::testutil::skip_fixture("connector_host_matches_flag_rule: fixtures not found");
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
			for u in save.units() {
				let by_flags = u.flags & (CONNECTOR_UNIT | BUILDING | STANDALONE) != 0 && u.flags & GROUND_COVER == 0;
				assert_eq!(
					is_connector_host_type(u.unit_type),
					by_flags,
					"{:#04x} flags={:#010x}: id-predicate vs. flag rule disagree",
					u.unit_type,
					u.flags,
				);
				// A non-zero mask only ever appears on a host, and fits 8 bits.
				if u.connectors != 0 {
					assert!(
						is_connector_host_type(u.unit_type),
						"{:#04x} carries a mask but isn't a host",
						u.unit_type
					);
					assert!(u.connectors <= 0xFF, "connector mask {:#06x} exceeds 8 bits", u.connectors);
				}
				checked += 1;
			}
		}
		assert!(checked > 0, "no ~/MAX save units were checked");
		eprintln!("connector-host predicate agrees with flags on {checked} units");
	}

	/// The static [`is_building_type`] (the connector geometry's `unit_size = 2`)
	/// must equal `(BUILDING && !GROUND_COVER)` for every unit in every stock save
	/// — the engine's own `unit_size` test (`units_manager.cpp`). This is what
	/// makes the hand-transcribed 2×2-building id set trustworthy for auto-connect.
	#[test]
	fn building_type_matches_flag() {
		use crate::save::is_building_type;
		use crate::save::unit_types::flag::{BUILDING, GROUND_COVER};
		let Some(home) = std::env::var_os("HOME") else { return };
		let max_dir = std::path::Path::new(&home).join("MAX");
		let originals = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals");
		if !max_dir.is_dir() || !originals.is_dir() {
			crate::testutil::skip_fixture("building_type_matches_flag: fixtures not found");
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
			for u in save.units() {
				let by_flags = u.flags & BUILDING != 0 && u.flags & GROUND_COVER == 0;
				assert_eq!(
					is_building_type(u.unit_type),
					by_flags,
					"{:#04x} flags={:#010x}: is_building_type vs. (BUILDING && !GROUND_COVER) disagree",
					u.unit_type,
					u.flags,
				);
				checked += 1;
			}
		}
		assert!(checked > 0, "no ~/MAX save units were checked");
		eprintln!("is_building_type agrees with flags on {checked} units");
	}
}
