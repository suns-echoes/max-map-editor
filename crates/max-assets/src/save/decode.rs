//! Full-body decoder for M.A.X. `V70` save files.
//!
//! Picks up where `read.rs` leaves off (after the header + options block) and
//! decodes the rest of `SaveLoad_LoadFormatV70` (`saveload.cpp`): the surface
//! and cargo (resource) maps, the four per-team `CTInfo` blocks, the game
//! scalars, the four team unit stat tables, and the five unit lists — resolving
//! the shared object graph (`SmartFileReader`, `smartfile.cpp`) that links units
//! to their `UnitValues`, `Complex`, and `UnitPath` objects.
//!
//! Layout is derived byte-for-byte from the M.A.X. Port reference; see
//! `SAVE-FORMAT.md` §6+ for the annotated spec. The object registry maps a
//! type index to one of six classes, in ASCII-sorted-name order:
//! `1 AirPath, 2 BuilderPath, 3 Complex, 4 GroundPath, 5 UnitInfo, 6 UnitValues`.
//!
//! Both `V70` (all stock/retail saves and the user's `~/MAX` files) and `V71`
//! (M.A.X. Port's format) bodies are decoded; the two differ only in the body's
//! top-level section ordering and a few field widths (see [`read_save`]) — the
//! shared object graph is otherwise byte-identical modulo the format-width
//! branches the object loaders already carry. The decode stops after the five
//! unit lists — the trailing spatial hashes, heat maps, message logs, and AI
//! state are out of scope until write-back (S0.4).

use std::io::Cursor;
use std::path::Path;

use super::orders::ORDER_DISABLE;
use super::read::{SaveError, read_header_from};
use super::types::{
	AirPath, Complex, CtInfo, MapCell, MapHash, ObjMeta, RawRegions, SaveExtraSettings, SaveFile, SaveFormat,
	SaveObject, TeamUnitsTable, UNIT_END, UnitBodyLayout, UnitPath, UnitRecord, UnitValues,
};

/// A `SmartFileReader`-equivalent cursor over an in-memory save buffer, carrying
/// the running object-dedup table the M.A.X. object graph relies on.
struct Reader<'a> {
	data: &'a [u8],
	pos: usize,
	format: SaveFormat,
	/// Materialized objects in first-seen order; 1-based on-disk indices map to
	/// `objects[index - 1]`. A slot is reserved ([`SaveObject::Reserved`]) before
	/// its body is read so cyclic/forward references resolve.
	objects: Vec<SaveObject>,
	/// Re-serialization metadata parallel to `objects` (verbatim body bytes +
	/// subtree size), captured so the graph can be re-emitted byte-exactly.
	object_meta: Vec<ObjMeta>,
	/// Set by [`Self::load_unit_info`] with the just-decoded unit's field offsets
	/// (relative to that body's start), consumed by [`Self::read_object`] into the
	/// object's [`ObjMeta::unit_layout`]. `None` between units.
	pending_layout: Option<UnitBodyLayout>,
	/// How many inline object bodies are currently open on the stack. Guards the
	/// recursive descent - see [`MAX_OBJECT_DEPTH`].
	depth: u32,
}

/// How deep inline object bodies may nest before the decoder calls the file
/// corrupt. A body can legitimately contain another (a unit's path, its stat
/// block, a destroyed unit inlined in a message log), but only a handful of
/// levels: every stock save decodes within single digits. Without a limit the
/// depth is bounded only by file size - a crafted `.DTA` chaining nested
/// `UnitInfo` bodies overflows the stack, which no `Result` can catch.
const MAX_OBJECT_DEPTH: u32 = 64;

/// The registry name of a materialized object's class, for error messages.
fn class_name(obj: &SaveObject) -> &'static str {
	match obj {
		SaveObject::Reserved => "(in progress)",
		SaveObject::Unit(_) => "UnitInfo",
		SaveObject::Values(_) => "UnitValues",
		SaveObject::Complex(_) => "Complex",
		SaveObject::Path(_) => "UnitPath",
	}
}

impl<'a> Reader<'a> {
	fn need(&self, n: usize) -> Result<(), SaveError> {
		if self.pos + n > self.data.len() {
			Err(SaveError::UnexpectedEof { offset: self.pos, needed: (self.pos + n) - self.data.len() })
		} else {
			Ok(())
		}
	}

	fn u8(&mut self) -> Result<u8, SaveError> {
		self.need(1)?;
		let v = self.data[self.pos];
		self.pos += 1;
		Ok(v)
	}

	fn i8(&mut self) -> Result<i8, SaveError> {
		Ok(self.u8()? as i8)
	}

	fn u16(&mut self) -> Result<u16, SaveError> {
		self.need(2)?;
		let v = u16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
		self.pos += 2;
		Ok(v)
	}

	fn i16(&mut self) -> Result<i16, SaveError> {
		Ok(self.u16()? as i16)
	}

	fn u32(&mut self) -> Result<u32, SaveError> {
		self.need(4)?;
		let v = u32::from_le_bytes([
			self.data[self.pos],
			self.data[self.pos + 1],
			self.data[self.pos + 2],
			self.data[self.pos + 3],
		]);
		self.pos += 4;
		Ok(v)
	}

	fn i32(&mut self) -> Result<i32, SaveError> {
		Ok(self.u32()? as i32)
	}

	fn skip(&mut self, n: usize) -> Result<(), SaveError> {
		self.need(n)?;
		self.pos += n;
		Ok(())
	}

	fn read_vec_u8(&mut self, n: usize) -> Result<Vec<u8>, SaveError> {
		self.need(n)?;
		let v = self.data[self.pos..self.pos + n].to_vec();
		self.pos += n;
		Ok(v)
	}

	/// A format-width unsigned value: `u16` in `V70`, `u32` in `V71`. Covers
	/// `ReadIndex`, `ReadObjectCount`, and the handful of scalars the engine
	/// widened between formats (team gold, `GroundPath` step index).
	fn read_wide(&mut self) -> Result<u32, SaveError> {
		match self.format {
			SaveFormat::V70 => Ok(self.u16()? as u32),
			SaveFormat::V71 => self.u32(),
		}
	}

	fn read_index(&mut self) -> Result<u32, SaveError> {
		self.read_wide()
	}

	fn read_object_count(&mut self) -> Result<u32, SaveError> {
		self.read_wide()
	}

	/// Byte width of one index/count field in this format - used to point an
	/// error back at the field just consumed.
	fn index_width(&self) -> usize {
		match self.format {
			SaveFormat::V70 => 2,
			SaveFormat::V71 => 4,
		}
	}

	/// Capacity to reserve for a `count` read out of the file, clamped to what
	/// the remaining buffer could possibly supply at `elem` bytes apiece.
	///
	/// Counts are untrusted, and `V71` widened them to `u32`: a corrupt one asks
	/// for billions of elements. Every read loop is already bounded and errors
	/// cleanly at the first short read, so an unclamped `with_capacity` buys
	/// nothing but an out-of-memory abort before that error can be reported.
	fn capacity_for(&self, count: usize, elem: usize) -> usize {
		count.min(self.data.len().saturating_sub(self.pos) / elem.max(1))
	}

	/// A `UnitInfo` custom name: a bespoke `u16` length then that many raw bytes,
	/// no terminator and no format branch — distinct from the generic
	/// `u32`-length-prefixed string the `SmartFileReader` uses elsewhere.
	fn read_unit_name(&mut self) -> Result<String, SaveError> {
		let len = self.u16()? as usize;
		self.need(len)?;
		let s = String::from_utf8_lossy(&self.data[self.pos..self.pos + len]).into_owned();
		self.pos += len;
		Ok(s)
	}

	/// `SmartFileReader::ReadObject` — resolve one object reference to its index
	/// into [`Reader::objects`] (or `None` for a null reference on disk).
	fn read_object(&mut self) -> Result<Option<usize>, SaveError> {
		let index = self.read_index()?;
		if index == 0 {
			return Ok(None);
		}
		let count = self.objects.len() as u32;
		if index <= count {
			// Back-reference to an already-materialized object.
			return Ok(Some(index as usize - 1));
		}
		// New object: the writer only ever mints the *next* index, so anything
		// past it is a corrupt file. Accepting it would silently desync the whole
		// graph (every later back-reference resolving to the wrong object) instead
		// of failing at the first bad byte.
		if index != count + 1 {
			return Err(SaveError::ObjectIndexOutOfRange { index, count, offset: self.pos - self.index_width() });
		}
		// A type index and inline body follow.
		let type_at = self.pos;
		let type_index = self.read_index()?;
		let body_start = self.pos;
		let slot = self.objects.len();
		self.objects.push(SaveObject::Reserved); // reserve before recursing
		self.object_meta.push(ObjMeta::default()); // parallel placeholder
		self.depth += 1;
		if self.depth > MAX_OBJECT_DEPTH {
			return Err(SaveError::ObjectGraphTooDeep { offset: type_at, limit: MAX_OBJECT_DEPTH });
		}
		let obj = self.load_object_body(type_index, type_at)?;
		self.depth -= 1;
		self.objects[slot] = obj;
		// Capture the verbatim body (nested inline objects included) and this
		// object's subtree size, so the graph can be re-emitted byte-exactly. For a
		// `UnitInfo` body, `load_unit_info` also stashed its editable-field offsets
		// (`pending_layout`) — take them here so the exporter can patch in place.
		self.object_meta[slot] = ObjMeta {
			type_index,
			contained: self.objects.len() - slot,
			body_raw: self.data[body_start..self.pos].to_vec(),
			unit_layout: self.pending_layout.take(),
		};
		Ok(Some(slot))
	}

	/// [`Self::read_object`] plus the class the field is declared to hold. A
	/// mismatch means the graph has drifted - a `path` field resolving to a
	/// `Complex` is a corrupt file, and reporting it here beats carrying the
	/// wrong object into the typed model.
	///
	/// A still-[`SaveObject::Reserved`] slot passes: that is a reference back into
	/// an object whose own body is still being decoded (the reserve-before-recurse
	/// order exists precisely so those cycles resolve), and its class is not known
	/// until the body finishes.
	fn read_object_as(
		&mut self,
		expected: &'static str,
		ok: fn(&SaveObject) -> bool,
	) -> Result<Option<usize>, SaveError> {
		let found = self.read_object()?;
		if let Some(idx) = found {
			let obj = &self.objects[idx];
			if !matches!(obj, SaveObject::Reserved) && !ok(obj) {
				return Err(SaveError::ObjectTypeMismatch { index: idx, expected, actual: class_name(obj) });
			}
		}
		Ok(found)
	}

	fn load_object_body(&mut self, type_index: u32, at: usize) -> Result<SaveObject, SaveError> {
		Ok(match type_index {
			1 => SaveObject::Path(self.load_air_path()?),
			2 => SaveObject::Path(self.load_builder_path()?),
			3 => SaveObject::Complex(self.load_complex()?),
			4 => SaveObject::Path(self.load_ground_path()?),
			5 => SaveObject::Unit(self.load_unit_info()?),
			6 => SaveObject::Values(self.load_unit_values()?),
			other => return Err(SaveError::UnknownObjectType(other, at)),
		})
	}

	// --- registered-class FileLoad bodies ---------------------------------

	/// `UnitValues::FileLoad` — 28 bytes in `V70`. `fuel` is not serialized;
	/// `move_and_fire` is the lone `u8` stat.
	fn load_unit_values(&mut self) -> Result<UnitValues, SaveError> {
		Ok(UnitValues {
			turns: self.u16()?,
			hits: self.u16()?,
			armor: self.u16()?,
			attack: self.u16()?,
			speed: self.u16()?,
			range: self.u16()?,
			rounds: self.u16()?,
			move_and_fire: self.u8()?,
			scan: self.u16()?,
			storage: self.u16()?,
			ammo: self.u16()?,
			attack_radius: self.u16()?,
			agent_adjust: self.u16()?,
			version: self.u16()?,
			// V70 stores a legacy `units_built` byte; V71 a `bool`. Both are one
			// byte, non-zero meaning "in use".
			in_use: self.u8()? != 0,
		})
	}

	/// `Complex::FileLoad` — seven `i16`s, 14 bytes (disk order, not header order).
	fn load_complex(&mut self) -> Result<Complex, SaveError> {
		Ok(Complex {
			material: self.i16()?,
			fuel: self.i16()?,
			gold: self.i16()?,
			power: self.i16()?,
			workers: self.i16()?,
			buildings: self.i16()?,
			id: self.i16()?,
		})
	}

	/// `AirPath::FileLoad` — 27 bytes (note the 1-byte `angle` puts the rest at
	/// odd offsets; there is no alignment padding).
	fn load_air_path(&mut self) -> Result<UnitPath, SaveError> {
		Ok(UnitPath::Air(AirPath {
			length: self.i16()?,
			angle: self.i8()?,
			start_x: self.i16()?,
			start_y: self.i16()?,
			end_x: self.i16()?,
			end_y: self.i16()?,
			step_x: self.i32()?,
			step_y: self.i32()?,
			delta_x: self.i32()?,
			delta_y: self.i32()?,
		}))
	}

	/// `BuilderPath::FileLoad` — two `i16`s, 4 bytes.
	fn load_builder_path(&mut self) -> Result<UnitPath, SaveError> {
		Ok(UnitPath::Builder { direction_x: self.i16()?, direction_y: self.i16()? })
	}

	/// `GroundPath::FileLoad` — end point, a format-width step index, then a
	/// `PathStep` array (each a raw `i8,i8`, counted by `ReadObjectCount`).
	fn load_ground_path(&mut self) -> Result<UnitPath, SaveError> {
		let end_x = self.i16()?;
		let end_y = self.i16()?;
		let step_index = self.read_wide()?;
		let count = self.read_object_count()? as usize;
		let mut steps = Vec::with_capacity(self.capacity_for(count, 2));
		for _ in 0..count {
			let x = self.i8()?;
			let y = self.i8()?;
			steps.push((x, y));
		}
		Ok(UnitPath::Ground { end_x, end_y, step_index, steps })
	}

	/// `UnitInfo::FileLoad`. The long scalar prefix (140 + name bytes in `V70`)
	/// is read in exact order, keeping only the gameplay fields; then the object
	/// references. `V71` widens several fields — handled inline so the reader is
	/// correct for both, though only `V70` bodies are exercised today.
	fn load_unit_info(&mut self) -> Result<UnitRecord, SaveError> {
		let v71 = self.format == SaveFormat::V71;
		// Field offsets are captured relative to this body's start (== `self.pos`
		// on entry, just after the object's type index) so the exporter can patch
		// scalars in place. `off()` reads the current body-relative offset.
		let body_start = self.pos;
		let off = |pos: usize| pos - body_start;

		let unit_type = self.u16()?;
		let id = self.u16()?;
		let flags = self.u32()?;
		let pixel_x_off = off(self.pos);
		let pixel_x = self.u16()?;
		let pixel_y = self.u16()?;
		let grid_x_off = off(self.pos);
		let grid_x = self.i16()?;
		let grid_y = self.i16()?;
		let name_off = off(self.pos);
		let name = self.read_unit_name()?;
		// On-disk name byte length (may differ from `name.len()` if the raw bytes
		// were not valid UTF-8 and `from_utf8_lossy` substituted): total consumed
		// minus the 2-byte length prefix.
		let name_len = off(self.pos) - name_off - 2;
		self.skip(4)?; // shadow_offset: Point (2×i16)
		let team_off = off(self.pos);
		let team = self.u8()?;
		self.skip(1)?; // unit_id
		self.skip(1)?; // brightness
		let angle_off = off(self.pos);
		let angle = self.u8()?;
		self.skip(5)?; // visible_to_team[5]
		self.skip(5)?; // spotted_by_team[5]
		self.skip(4)?; // max_velocity, velocity, sound, scaler_adjust (4×u8)
		self.skip(16)?; // sprite_bounds: Rect (4×i32)
		self.skip(16)?; // shadow_bounds: Rect (4×i32)
		let turret_angle_off = off(self.pos);
		let turret_angle = self.u8()?;
		self.skip(2)?; // turret_offset_x/y (2×i8)
		// 8 image/index i16 fields: total_images, image_base, turret_image_base,
		// firing_image_base, connector_image_base, image_index, turret_image_index,
		// image_index_max. Keep the two frame bases the map overlay draws from; the
		// block offset is captured so the integrity pass can re-derive image_index.
		let image_block_off = off(self.pos);
		self.skip(4)?; // total_images, image_base
		let turret_image_base = self.i16()?;
		self.skip(2)?; // firing_image_base
		let connector_image_base = self.i16()?;
		self.skip(6)?; // image_index, turret_image_index, image_index_max
		let orders_off = off(self.pos);
		let orders = self.u8()?;
		let state = self.u8()?;
		let prior_orders = self.u8()?;
		let prior_state = self.u8()?;
		self.skip(1)?; // laying_state
		// Grid-target block: V70 stores one pair (4B), V71 two pairs (8B).
		let move_to_off = off(self.pos);
		self.skip(if v71 { 8 } else { 4 })?;
		let build_time_off = off(self.pos);
		self.skip(1)?; // build_time
		self.skip(7)?; // total/raw/fuel/gold mining + raw/gold/fuel mining_max (7×u8)
		let hits_off = off(self.pos);
		let hits_width = if v71 { 2 } else { 1 };
		let hits = if v71 { self.u16()? } else { self.u8()? as u16 };
		let _speed = if v71 { self.u16()? } else { self.u8()? as u16 };
		self.skip(1)?; // shots
		self.skip(1)?; // move_and_fire
		let storage_off = off(self.pos);
		let storage = self.i16()?;
		if v71 {
			self.skip(5)?; // experience i16, transfer_cargo i16, stealth_dice_roll u8 (V70 derives these)
		}
		let ammo_off = off(self.pos);
		let ammo = self.u8()?;
		self.skip(3)?; // targeting_mode, enter_mode, cursor (3×u8)
		// Recoil / disabled block. V70: one signed byte `recoil_delay` — when
		// orders == ORDER_DISABLE it is the disable countdown (negatives clamp to 0),
		// else firing-recoil frames. V71: two bytes (firing_recoil, disabled_turns).
		let recoil_off = off(self.pos);
		let (disabled_off, disabled_dual, disabled_turns) = if v71 {
			self.skip(1)?; // firing_recoil_frames
			let dt = self.u8()?; // disabled_turns_remaining
			(recoil_off + 1, false, dt)
		} else {
			let rd = (self.u8()? as i8).max(0) as u8;
			(recoil_off, true, if orders == ORDER_DISABLE { rd } else { 0 })
		};
		// delayed_reaction, damaged_this_turn, research_topic, moved, bobbed,
		// shake_effect_state, engine, weapon — `moved` is the 4th byte.
		let moved_off = off(self.pos) + 3;
		self.skip(8)?;
		// Comm/move-fraction block: V70 four bytes, V71 one.
		self.skip(if v71 { 1 } else { 4 })?;
		self.skip(1)?; // repeat_build
		let build_rate = self.u16()?;
		self.skip(1)?; // disabled_reaction_fire
		self.skip(1)?; // auto_survey
		self.skip(4)?; // ai_state_bits

		// Object references. `connectors` sits between `path` and `base_values`;
		// both `connectors` and the `base_values` reference follow the variable-
		// width `path` reference, so their offsets are captured, not computed. The
		// ref section starts here — the end of the verbatim opaque prefix.
		let refs_off = off(self.pos);
		let path = self.read_object_as("UnitPath", |o| matches!(o, SaveObject::Path(_)))?;
		let connectors_off = off(self.pos);
		let connectors = self.u16()?;
		let base_values_ref_off = off(self.pos);
		let base_values = self.read_object_as("UnitValues", |o| matches!(o, SaveObject::Values(_)))?;
		let complex = self.read_object_as("Complex", |o| matches!(o, SaveObject::Complex(_)))?;
		let parent_unit = self.read_object_as("UnitInfo", |o| matches!(o, SaveObject::Unit(_)))?;
		let enemy_unit = self.read_object_as("UnitInfo", |o| matches!(o, SaveObject::Unit(_)))?;
		let build_list = self.read_build_list()?;

		// Hand the editable-field offsets to `read_object`, which stores them in
		// this unit's `ObjMeta::unit_layout` for the exporter (S6.1).
		self.pending_layout = Some(UnitBodyLayout {
			name: name_off,
			name_len,
			pixel_x: pixel_x_off,
			grid_x: grid_x_off,
			team: team_off,
			angle: angle_off,
			turret_angle: turret_angle_off,
			image_block: image_block_off,
			orders: orders_off,
			move_to: move_to_off,
			build_time: build_time_off,
			moved: moved_off,
			hits: hits_off,
			hits_width,
			ammo: ammo_off,
			disabled: disabled_off,
			disabled_dual,
			storage: storage_off,
			connectors: connectors_off,
			base_values_ref: base_values_ref_off,
			refs_off,
		});

		Ok(UnitRecord {
			unit_type,
			id,
			flags,
			pixel_x,
			pixel_y,
			grid_x,
			grid_y,
			name,
			team,
			angle,
			turret_angle,
			orders,
			state,
			prior_orders,
			prior_state,
			disabled_turns,
			hits,
			ammo,
			storage,
			build_rate,
			connectors,
			turret_image_base,
			connector_image_base,
			path,
			base_values,
			complex,
			parent_unit,
			enemy_unit,
			build_list,
		})
	}

	/// `UnitInfo_BuildList_FileLoad` — a count then that many raw `ResourceID`s
	/// (`u16`), *not* object references.
	fn read_build_list(&mut self) -> Result<Vec<u16>, SaveError> {
		let count = self.read_object_count()? as usize;
		let mut list = Vec::with_capacity(self.capacity_for(count, 2));
		for _ in 0..count {
			list.push(self.u16()?);
		}
		Ok(list)
	}

	// --- top-level sections -----------------------------------------------

	/// One per-team `CTInfo` block — 565 bytes in `V70`, all scalar/array (no
	/// object references). Keeps only the fields a teams editor needs.
	fn load_ct_info(&mut self) -> Result<CtInfo, SaveError> {
		self.skip(40)?; // markers_v70: Point[10]
		let team_type = self.u8()?;
		let finished_turn = self.u8()? != 0;
		let team_clan = self.u8()?;
		self.skip(96)?; // research_topics: ResearchTopic[8] (8×3×i32)
		let team_points = self.u32()?;
		let number_of_objects_created = self.u16()?;
		self.skip(93)?; // unit_counters_v70: u8[93]
		self.skip(12)?; // screen_locations_v70: ScreenLocation[6] (6×2)
		self.skip(100)?; // score_graph: i16[50]
		self.skip(2)?; // selected_unit_ids[team]
		let zoom_level = self.u16()?;
		let camera_x = self.i16()?;
		let camera_y = self.i16()?;
		self.skip(11)?; // 11 display-toggle buttons (i8)
		self.skip(8)?; // stats_factories/mines/buildings/units_built (4×i16)
		self.skip(186)?; // casualties_v70: u16[93]
		self.skip(2)?; // stats_gold_spent_on_upgrades_v70: i16
		Ok(CtInfo::v70(
			team_type,
			finished_turn,
			team_clan,
			team_points,
			number_of_objects_created,
			zoom_level,
			(camera_x, camera_y),
		))
	}

	/// One per-team `CTInfo` block in `V71` (`SaveLoad_LoadFormatV71`), 988 bytes,
	/// decoded in full (every field surfaced typed — the inverse of
	/// `encode::encode_ct_info`). Unlike the `V70` variant it has no leading
	/// `markers[10]`, and the counter/casualty arrays plus
	/// `stats_gold_spent_on_upgrades` are stored at native width (`u32`).
	fn load_ct_info_v71(&mut self) -> Result<CtInfo, SaveError> {
		let team_type = self.u8()?;
		let finished_turn = self.u8()? != 0;
		let team_clan = self.u8()?;
		let mut research_topics = [[0i32; 3]; 8];
		for topic in research_topics.iter_mut() {
			for v in topic.iter_mut() {
				*v = self.i32()?;
			}
		}
		let team_points = self.u32()?;
		let number_of_objects_created = self.u16()?;
		let mut unit_counters = [0u32; UNIT_END];
		for c in unit_counters.iter_mut() {
			*c = self.u32()?;
		}
		let mut screen_locations = [[0i8; 2]; 4];
		for loc in screen_locations.iter_mut() {
			loc[0] = self.i8()?;
			loc[1] = self.i8()?;
		}
		let mut score_graph = [0i16; 50];
		for s in score_graph.iter_mut() {
			*s = self.i16()?;
		}
		let selected_unit_id = self.u16()?;
		let zoom_level = self.u16()?;
		let camera_x = self.i16()?;
		let camera_y = self.i16()?;
		let mut display_buttons = [0i8; 11];
		for b in display_buttons.iter_mut() {
			*b = self.i8()?;
		}
		let mut stats = [0i16; 4];
		for s in stats.iter_mut() {
			*s = self.i16()?;
		}
		let mut casualties = [0u32; UNIT_END];
		for c in casualties.iter_mut() {
			*c = self.u32()?;
		}
		let stats_gold_spent_on_upgrades = self.u32()?;
		Ok(CtInfo {
			team_type,
			finished_turn,
			team_clan,
			research_topics,
			team_points,
			number_of_objects_created,
			unit_counters,
			screen_locations,
			score_graph,
			selected_unit_id,
			zoom_level,
			camera_x,
			camera_y,
			display_buttons,
			stats,
			casualties,
			stats_gold_spent_on_upgrades,
		})
	}

	/// `TeamUnits::FileLoad` — gold, then `UNIT_END` base + `UNIT_END` current
	/// `UnitValues` references, then a `Complex` list.
	fn load_team_units(&mut self) -> Result<TeamUnitsTable, SaveError> {
		let gold = self.read_wide()?;
		let is_values = |o: &SaveObject| matches!(o, SaveObject::Values(_));
		let mut base_values = Vec::with_capacity(UNIT_END);
		for _ in 0..UNIT_END {
			base_values.push(self.read_object_as("UnitValues", is_values)?);
		}
		let mut current_values = Vec::with_capacity(UNIT_END);
		for _ in 0..UNIT_END {
			current_values.push(self.read_object_as("UnitValues", is_values)?);
		}
		let complex_count = self.read_object_count()? as usize;
		let mut complexes = Vec::with_capacity(self.capacity_for(complex_count, self.index_width()));
		for _ in 0..complex_count {
			if let Some(idx) = self.read_object_as("Complex", |o| matches!(o, SaveObject::Complex(_)))? {
				complexes.push(idx);
			}
		}
		Ok(TeamUnitsTable { gold, base_values, current_values, complexes })
	}

	/// One `SmartList<UnitInfo>` — a count then that many `UnitInfo` references.
	fn load_unit_list(&mut self) -> Result<Vec<usize>, SaveError> {
		let count = self.read_object_count()? as usize;
		let mut list = Vec::with_capacity(self.capacity_for(count, self.index_width()));
		for _ in 0..count {
			if let Some(idx) = self.read_object_as("UnitInfo", |o| matches!(o, SaveObject::Unit(_)))? {
				list.push(idx);
			}
		}
		Ok(list)
	}

	/// `Hash_UnitHash::FileLoad` (`hash.cpp`) — a `u16` bucket count (always
	/// `HASH_HASH_SIZE` = 512) then that many `SmartList<UnitInfo>` buckets. Every
	/// entry back-references a unit already materialized by the five lists, so no
	/// new objects are created; this is the first structure of the save's tail.
	fn load_unit_hash(&mut self) -> Result<Vec<Vec<usize>>, SaveError> {
		let buckets = self.u16()? as usize;
		let mut hash = Vec::with_capacity(self.capacity_for(buckets, self.index_width()));
		for _ in 0..buckets {
			hash.push(self.load_unit_list()?);
		}
		Ok(hash)
	}

	/// The seven "extra settings" `i32`s (`saveload.cpp` region 13), read typed. The
	/// caller still captures the same bytes verbatim for the current re-emit path;
	/// this promotes them to the typed model (Stage A) so an encoder can rebuild the
	/// region from the model alone.
	fn read_extra_settings(&mut self) -> Result<SaveExtraSettings, SaveError> {
		Ok(SaveExtraSettings {
			effects: self.i32()?,
			click_scroll: self.i32()?,
			quick_scroll: self.i32()?,
			fast_movement: self.i32()?,
			follow_unit: self.i32()?,
			auto_select: self.i32()?,
			enemy_halt: self.i32()?,
		})
	}

	/// `Hash_MapHash::FileLoad` (`hash.cpp`) — `hash_size:u16`, `x_shift:u16`, then
	/// `hash_size` buckets; each bucket is a `ReadObjectCount` of `MapHashObject`s,
	/// each `{x:u16, y:u16, SmartList<UnitInfo>}` (the cell's units as back-refs).
	/// The second tail structure, decoded structurally so moves/add/remove can
	/// re-derive it (the rest of the tail stays opaque).
	fn load_map_hash(&mut self) -> Result<MapHash, SaveError> {
		let hash_size = self.u16()?;
		let x_shift = self.u16()?;
		let mut buckets = Vec::with_capacity(self.capacity_for(hash_size as usize, self.index_width()));
		for _ in 0..hash_size {
			let cell_count = self.read_object_count()? as usize;
			// x + y + a list count, at minimum.
			let mut cells = Vec::with_capacity(self.capacity_for(cell_count, 4 + self.index_width()));
			for _ in 0..cell_count {
				let x = self.u16()?;
				let y = self.u16()?;
				let units = self.load_unit_list()?;
				cells.push(MapCell { x, y, units });
			}
			buckets.push(cells);
		}
		Ok(MapHash { hash_size, x_shift, buckets })
	}
}

/// A bounds-checked cursor over the save's **tail** that can also walk a
/// `SmartFileReader::ReadObject` reference — the reason this lives next to the
/// decoder rather than in [`crate::save::tail`].
///
/// A tail reference is usually a bare back-index into the graph the body
/// already wrote, but not always: a message log holding the **last** reference
/// to a unit the game has since destroyed inlines that unit's whole body right
/// there (`smartfile.cpp:239`), and the only honest way to know how long that
/// body is, is to decode it. So the walker is a [`Reader`] seeded with as many
/// placeholder slots as the graph materialized — indices resolve, inline bodies
/// are decoded to measure them, and everything it decodes is dropped. The save
/// keeps the tail verbatim; this only measures it.
pub(crate) struct TailWalker<'a>(Reader<'a>);

impl<'a> TailWalker<'a> {
	/// A walker over `tail` (positions are relative to it), for a save whose
	/// object graph already holds `objects` entries.
	pub(crate) fn new(tail: &'a [u8], format: SaveFormat, objects: usize) -> Self {
		TailWalker(Reader {
			data: tail,
			pos: 0,
			format,
			objects: vec![SaveObject::Reserved; objects],
			object_meta: vec![ObjMeta::default(); objects],
			pending_layout: None,
			depth: 0,
		})
	}

	pub(crate) fn pos(&self) -> usize {
		self.0.pos
	}

	/// See [`Reader::capacity_for`] - clamps an untrusted tail count to what the
	/// bytes left could actually supply.
	pub(crate) fn capacity_for(&self, count: usize, elem: usize) -> usize {
		self.0.capacity_for(count, elem)
	}

	pub(crate) fn at_end(&self) -> bool {
		self.0.pos == self.0.data.len()
	}

	/// How many objects the graph holds as of here — it grows by one for every
	/// body the tail inlined, which is how a caller tells a span that merely
	/// *refers* to the graph from one that *extends* it.
	pub(crate) fn objects_seen(&self) -> usize {
		self.0.objects.len()
	}

	pub(crate) fn skip(&mut self, n: usize) -> Result<(), SaveError> {
		self.0.skip(n)
	}

	pub(crate) fn u16(&mut self) -> Result<u16, SaveError> {
		self.0.u16()
	}

	/// A `ReadObjectCount` (`u32` in `V71`).
	pub(crate) fn count(&mut self) -> Result<u32, SaveError> {
		self.0.read_object_count()
	}

	/// One object reference: the slot it resolves to (`None` = a null reference)
	/// and the byte span it occupied — an inline body's whole subtree included,
	/// so a caller can copy or re-emit the reference as one unit.
	pub(crate) fn object_ref(&mut self) -> Result<(Option<usize>, std::ops::Range<usize>), SaveError> {
		let from = self.0.pos;
		let slot = self.0.read_object()?;
		Ok((slot, from..self.0.pos))
	}

	/// The objects the tail materialized, appended to the placeholder slots the
	/// graph was seeded with — the records and re-serialization metadata a
	/// [`super::serialize::Writer`] needs to emit those inline bodies again.
	pub(crate) fn into_parts(self) -> (Vec<SaveObject>, Vec<ObjMeta>) {
		(self.0.objects, self.0.object_meta)
	}
}

/// Fully decodes a M.A.X. save file (`V70` or `V71`).
///
/// `dims` is the referenced world's `(width, height)` in cells — the save does
/// not store map size, so the caller must resolve it from the actual `.WRL` the
/// save was authored on. For stock worlds this is the pristine bundled map (see
/// [`crate::save::world_file_name`]); a `V71` save's stored world hash is a
/// per-index table value, not a content hash, so it cannot convey a custom map's
/// size — the caller must supply the true dimensions there. The surface and
/// cargo maps are sized `width * height`.
///
/// The two formats differ only in the body's top-level layout: `V71` writes the
/// seven "extra settings" ints up front (before the surface map), its per-team
/// `CTInfo` blocks cover five teams at native field width (no leading markers
/// array), and its game scalars are `u32` in a slightly different order with an
/// extra `is_cheater`/`cheater_team` pair. Every registered object body is
/// byte-identical modulo the format-width branches the object loaders handle.
pub fn read_save(path: &Path, dims: (u16, u16)) -> Result<SaveFile, SaveError> {
	read_save_bytes(&std::fs::read(path)?, dims)
}

/// Decodes an in-memory save image — the raw `.DTA` bytes — at the given world
/// dimensions. [`read_save`] is the file-reading wrapper; this variant decodes
/// bytes a caller already holds, e.g. a save embedded in an editor project
/// (`map_core`) that is re-parsed on load without touching the filesystem.
pub fn read_save_bytes(data: &[u8], dims: (u16, u16)) -> Result<SaveFile, SaveError> {
	let mut cursor = Cursor::new(data);
	let header = read_header_from(&mut cursor)?;
	let start = cursor.position() as usize;
	let v71 = header.format == SaveFormat::V71;

	let (width, height) = dims;
	let cells = width as usize * height as usize;

	// Retained header + options (re-emitted verbatim; not re-serialized field by
	// field — teams/options editing is a later stage).
	let header_raw = data[..start].to_vec();

	let mut r = Reader {
		data,
		pos: start,
		format: header.format,
		objects: Vec::new(),
		object_meta: Vec::new(),
		pending_layout: None,
		depth: 0,
	};

	// V71 writes the seven "extra settings" ints (effects, click/quick scroll,
	// fast_movement, follow_unit, auto_select, enemy_halt) up front, right after
	// the options block; V70 writes them after the game scalars instead. Retained
	// verbatim in either position.
	let mut extra_settings = Vec::new();
	let mut extra_typed = SaveExtraSettings::default();
	if v71 {
		let s = r.pos;
		extra_typed = r.read_extra_settings()?;
		extra_settings = r.data[s..r.pos].to_vec();
	}

	// Surface map (u8 × cells), then cargo/resource map (u16 × cells).
	let surface_map = r.read_vec_u8(cells)?;
	let mut cargo_map = Vec::with_capacity(cells);
	for _ in 0..cells {
		cargo_map.push(r.u16()?);
	}

	// Per-team CTInfo: four teams (Red/Green/Blue/Gray) in V70, five (adding the
	// alien slot) in V71, whose record is also wider (see `load_ct_info_v71`).
	// Each block is retained verbatim for re-serialization.
	let team_count = if v71 { 5 } else { 4 };
	let mut teams = Vec::with_capacity(team_count);
	let mut ct_info_raw = Vec::with_capacity(team_count);
	for _ in 0..team_count {
		let ct_start = r.pos;
		teams.push(if v71 { r.load_ct_info_v71()? } else { r.load_ct_info()? });
		ct_info_raw.push(r.data[ct_start..r.pos].to_vec());
	}

	// Game scalars (retained verbatim). V70: active/player team u8, turn_counter
	// i32, game_state u16, turn_timer u16, then the seven extra-settings ints.
	// V71: all u32, ordered active/player/turn_counter/turn_timer/game_state, then
	// an is_cheater/cheater_team pair the current engine always writes.
	let (active_turn_team, player_team, turn_counter, game_state, turn_timer);
	let (mut is_cheater, mut cheater_team) = (0u32, 0u32);
	let scalars_start = r.pos;
	let scalars;
	if v71 {
		active_turn_team = r.u32()? as u8;
		player_team = r.u32()? as u8;
		turn_counter = r.i32()?;
		turn_timer = r.u32()? as u16;
		game_state = r.u32()? as u16;
		is_cheater = r.u32()?;
		cheater_team = r.u32()?;
		scalars = r.data[scalars_start..r.pos].to_vec();
	} else {
		active_turn_team = r.u8()?;
		player_team = r.u8()?;
		turn_counter = r.i32()?;
		game_state = r.u16()?;
		turn_timer = r.u16()?;
		scalars = r.data[scalars_start..r.pos].to_vec();
		let s = r.pos;
		extra_typed = r.read_extra_settings()?;
		extra_settings = r.data[s..r.pos].to_vec();
	}

	// Team unit stat tables (always four — only Red/Green/Blue/Gray have them),
	// then the five unit lists. The object graph grows across all of these, so
	// order is load-bearing.
	let mut team_units = Vec::with_capacity(4);
	for _ in 0..4 {
		team_units.push(r.load_team_units()?);
	}

	let ground_cover = r.load_unit_list()?;
	let mobile_land_sea = r.load_unit_list()?;
	let stationary = r.load_unit_list()?;
	let mobile_air = r.load_unit_list()?;
	let particles = r.load_unit_list()?;

	// The first two tail structures: the unit spatial hash then the map spatial
	// hash (both back-references only), decoded structurally so an export can
	// re-derive them. The rest of the tail (heat maps, message logs, AI) stays
	// opaque — see SAVE-FORMAT.md §6/§8 and the S0.4 write-back plan.
	let unit_hash = r.load_unit_hash()?;
	let map_hash = r.load_map_hash()?;
	let tail = r.data[r.pos..].to_vec();

	let raw = RawRegions { header: header_raw, extra_settings, ct_info: ct_info_raw, scalars, tail };
	let object_meta = r.object_meta;

	Ok(SaveFile {
		header,
		extra_settings: extra_typed,
		width,
		height,
		surface_map,
		cargo_map,
		active_turn_team,
		player_team,
		turn_counter,
		game_state,
		turn_timer,
		is_cheater,
		cheater_team,
		teams,
		team_units,
		objects: r.objects,
		ground_cover,
		mobile_land_sea,
		stationary,
		mobile_air,
		particles,
		unit_hash,
		map_hash,
		object_meta,
		raw,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::save::{UnitCategory, is_ground_cover_type};

	/// The [`UnitCategory`] a save-list name should classify to, or `None` for
	/// the particle list (particles are not routed by the flag classifier).
	fn expected_category(list_name: &str) -> Option<UnitCategory> {
		match list_name {
			"ground-cover" => Some(UnitCategory::GroundCover),
			"stationary" => Some(UnitCategory::Stationary),
			"mobile-land-sea" => Some(UnitCategory::MobileLandSea),
			"mobile-air" => Some(UnitCategory::MobileAir),
			_ => None,
		}
	}

	/// Asserts the decode lands correctly on the trailing `Hash_UnitHash`: it has
	/// 512 buckets, references exactly the units in the five lists (no more, no
	/// fewer, all back-references), and each unit sits in bucket `id % 512`. A
	/// misaligned parse could not satisfy all three.
	fn assert_unit_hash_consistent(save: &SaveFile) {
		use std::collections::BTreeSet;
		assert_eq!(save.unit_hash.len(), 512, "HASH_HASH_SIZE buckets");
		let listed: BTreeSet<usize> = save.lists().iter().flat_map(|(_, l)| l.iter().copied()).collect();
		let hashed: BTreeSet<usize> = save.unit_hash.iter().flatten().copied().collect();
		assert_eq!(hashed, listed, "unit hash references exactly the listed units");
		for (bucket, entries) in save.unit_hash.iter().enumerate() {
			for &idx in entries {
				let u = save.unit(idx).expect("unit-hash entry resolves to a unit");
				assert_eq!(u.id as usize % 512, bucket, "unit id {} is in the wrong bucket", u.id);
			}
		}
	}

	/// Asserts the `Hash_MapHash` decode is not just byte-round-tripping but
	/// *interpreted* correctly — the property a move/add/remove rebuild relies on.
	/// Each occupied cell must hash to the bucket it sits in
	/// (`(y ^ (x << x_shift)) % hash_size`), reference only real units, and each
	/// referenced unit's footprint must cover the cell (its top-left cell, or one
	/// of a building's four cells).
	fn assert_map_hash_consistent(save: &SaveFile) {
		use crate::save::unit_types::flag::BUILDING;
		let mh = &save.map_hash;
		assert_eq!(mh.hash_size, 512, "HASH_HASH_SIZE map buckets");
		for (bucket, cells) in mh.buckets.iter().enumerate() {
			for cell in cells {
				let key = (cell.y ^ (cell.x << mh.x_shift)) as usize % mh.hash_size as usize;
				assert_eq!(key, bucket, "cell ({},{}) is in the wrong map bucket", cell.x, cell.y);
				for &idx in &cell.units {
					let u = save.unit(idx).expect("map-hash entry resolves to a unit");
					let (dx, dy) = (cell.x as i32 - u.grid_x as i32, cell.y as i32 - u.grid_y as i32);
					// A unit is added to its top-left cell, plus 3 more toward +x/+y
					// iff `flags & BUILDING` (`hash.cpp` MapHash::Add) — which covers
					// large 2×2 ground cover (LRGSLAB/LRGTAPE, BUILDING-flagged despite
					// being ground cover), so `is_building_type` (BUILDING &&
					// !GROUND_COVER) would be the wrong test. A 1-cell margin tolerates
					// a mobile unit whose hash cell lags its grid mid-move (its pixel
					// straddles a boundary). A gross grid/index misinterpretation would
					// place a unit far outside this box and still trip.
					let span = if u.flags & BUILDING != 0 { 1 } else { 0 };
					assert!(
						(-1..=span + 1).contains(&dx) && (-1..=span + 1).contains(&dy),
						"map cell ({},{}) references unit id {} at ({},{}) far from its footprint",
						cell.x,
						cell.y,
						u.id,
						u.grid_x,
						u.grid_y,
					);
				}
			}
		}
	}

	/// A little-endian buffer builder mirroring `SmartFileReader` writes, so a
	/// test can lay out a record from the spec and confirm the reader consumes it
	/// byte-for-byte.
	#[derive(Default)]
	struct Buf(Vec<u8>);

	impl Buf {
		fn u8(mut self, v: u8) -> Self {
			self.0.push(v);
			self
		}
		fn i8(self, v: i8) -> Self {
			self.u8(v as u8)
		}
		fn u16(mut self, v: u16) -> Self {
			self.0.extend_from_slice(&v.to_le_bytes());
			self
		}
		fn i16(self, v: i16) -> Self {
			self.u16(v as u16)
		}
		fn u32(mut self, v: u32) -> Self {
			self.0.extend_from_slice(&v.to_le_bytes());
			self
		}
		fn i32(self, v: i32) -> Self {
			self.u32(v as u32)
		}
		fn bytes(mut self, b: &[u8]) -> Self {
			self.0.extend_from_slice(b);
			self
		}
		fn zeros(mut self, n: usize) -> Self {
			self.0.resize(self.0.len() + n, 0);
			self
		}
	}

	fn reader(buf: &[u8]) -> Reader<'_> {
		Reader {
			data: buf,
			pos: 0,
			format: SaveFormat::V70,
			objects: Vec::new(),
			object_meta: Vec::new(),
			pending_layout: None,
			depth: 0,
		}
	}

	/// A distinct 28-byte `UnitValues` payload and the value it decodes to.
	fn sample_values() -> (Buf, UnitValues) {
		let buf = Buf::default()
			.u16(100) // turns
			.u16(200) // hits
			.u16(3) // armor
			.u16(4) // attack
			.u16(5) // speed
			.u16(6) // range
			.u16(7) // rounds
			.u8(1) // move_and_fire
			.u16(8) // scan
			.u16(9) // storage
			.u16(10) // ammo
			.u16(11) // attack_radius
			.u16(12) // agent_adjust
			.u16(13) // version
			.u8(1); // in_use (legacy_units_built != 0)
		let values = UnitValues {
			turns: 100,
			hits: 200,
			armor: 3,
			attack: 4,
			speed: 5,
			range: 6,
			rounds: 7,
			move_and_fire: 1,
			scan: 8,
			storage: 9,
			ammo: 10,
			attack_radius: 11,
			agent_adjust: 12,
			version: 13,
			in_use: true,
		};
		(buf, values)
	}

	#[test]
	fn unit_values_layout_is_28_bytes() {
		let (buf, expected) = sample_values();
		assert_eq!(buf.0.len(), 28);
		let mut r = reader(&buf.0);
		assert_eq!(r.load_unit_values().unwrap(), expected);
		assert_eq!(r.pos, 28);
	}

	#[test]
	fn complex_layout_is_14_bytes() {
		let buf = Buf::default().i16(-1).i16(2).i16(3).i16(4).i16(5).i16(6).i16(7);
		assert_eq!(buf.0.len(), 14);
		let mut r = reader(&buf.0);
		let c = r.load_complex().unwrap();
		assert_eq!(c, Complex { material: -1, fuel: 2, gold: 3, power: 4, workers: 5, buildings: 6, id: 7 });
		assert_eq!(r.pos, 14);
	}

	#[test]
	fn air_path_layout_is_27_bytes() {
		let buf = Buf::default().i16(50).i8(3).i16(1).i16(2).i16(3).i16(4).i32(10).i32(20).i32(30).i32(40);
		assert_eq!(buf.0.len(), 27);
		let mut r = reader(&buf.0);
		match r.load_air_path().unwrap() {
			UnitPath::Air(a) => {
				assert_eq!(a.length, 50);
				assert_eq!(a.angle, 3);
				assert_eq!((a.start_x, a.start_y), (1, 2));
				assert_eq!((a.end_x, a.end_y), (3, 4));
				assert_eq!((a.step_x, a.step_y, a.delta_x, a.delta_y), (10, 20, 30, 40));
			}
			other => panic!("expected AirPath, got {other:?}"),
		}
		assert_eq!(r.pos, 27);
	}

	#[test]
	fn ground_path_reads_step_list() {
		// end_x, end_y, step_index(u16), count(u16)=2, then 2 PathSteps.
		let buf = Buf::default().i16(7).i16(8).u16(1).u16(2).i8(-1).i8(0).i8(1).i8(1);
		let mut r = reader(&buf.0);
		match r.load_ground_path().unwrap() {
			UnitPath::Ground { end_x, end_y, step_index, steps } => {
				assert_eq!((end_x, end_y, step_index), (7, 8, 1));
				assert_eq!(steps, vec![(-1, 0), (1, 1)]);
			}
			other => panic!("expected GroundPath, got {other:?}"),
		}
		assert_eq!(r.pos, buf.0.len());
	}

	#[test]
	fn object_graph_materializes_deduplicates_and_nulls() {
		// Three ReadObject records: a fresh UnitValues, a back-reference to it,
		// then a null. In V70 indices/types are u16.
		let (values_buf, expected) = sample_values();
		let buf = Buf::default()
			.u16(1) // object_index 1 (== count+1 -> new)
			.u16(6) // type_index 6 (UnitValues)
			.bytes(&values_buf.0) // the 28-byte body
			.u16(1) // back-reference to object #1
			.u16(0); // null

		let mut r = reader(&buf.0);
		let a = r.read_object().unwrap();
		let b = r.read_object().unwrap();
		let c = r.read_object().unwrap();

		assert_eq!(a, Some(0));
		assert_eq!(b, Some(0), "back-reference must resolve to the same slot");
		assert_eq!(c, None, "index 0 is a null reference");
		assert_eq!(r.objects.len(), 1, "a back-reference must not allocate a new object");
		assert_eq!(r.objects[0], SaveObject::Values(expected));
		assert_eq!(r.pos, buf.0.len());
	}

	/// A `UnitInfo` body whose `parent_unit` reference is a fresh **inline**
	/// `UnitInfo` (`index`, then type 5, then `inner`) instead of null - the
	/// nesting a crafted file uses to drive the decoder's recursion. The ref
	/// block ends `.. parent_unit, enemy_unit, build_list_count`, so the field
	/// sits 6 bytes from the end and its inline body follows 4 from the end.
	fn unit_with_parent(index: u16, inner: &[u8]) -> Vec<u8> {
		let mut body = build_unit_info("u").0;
		let cut = body.len() - 4;
		body[cut - 2..cut].copy_from_slice(&index.to_le_bytes());
		let mut nested = vec![5u8, 0]; // type_index 5 = UnitInfo
		nested.extend_from_slice(inner);
		body.splice(cut..cut, nested);
		body
	}

	#[test]
	fn a_deeply_nested_object_graph_is_rejected_instead_of_overflowing_the_stack() {
		// Chain more `UnitInfo` bodies through `parent_unit` than the limit allows.
		// Depth is otherwise bounded only by file size, and a stack overflow is an
		// abort no `Result` can catch - so this must come back as an error.
		let levels = MAX_OBJECT_DEPTH as usize + 5;
		let mut body = build_unit_info("leaf").0;
		for k in (1..levels).rev() {
			body = unit_with_parent(k as u16 + 1, &body);
		}
		let mut buf = vec![1u8, 0, 5, 0]; // object #1, type UnitInfo
		buf.extend_from_slice(&body);

		let mut r = reader(&buf);
		match r.read_object() {
			Err(SaveError::ObjectGraphTooDeep { limit, .. }) => assert_eq!(limit, MAX_OBJECT_DEPTH),
			other => panic!("expected a depth error, got {other:?}"),
		}
	}

	#[test]
	fn an_object_reference_past_the_next_index_is_rejected() {
		// The writer only ever mints `count + 1`. Accepting `count + 2` would
		// silently re-point every later back-reference by one.
		let buf = Buf::default().u16(2).u16(6);
		let mut r = reader(&buf.0);
		match r.read_object() {
			Err(SaveError::ObjectIndexOutOfRange { index, count, .. }) => assert_eq!((index, count), (2, 0)),
			other => panic!("expected an out-of-range error, got {other:?}"),
		}
	}

	#[test]
	fn a_reference_of_the_wrong_class_is_rejected() {
		// Object #1 is a `UnitValues`; a field declared to hold a `UnitInfo` may
		// not resolve to it.
		let (values_buf, _) = sample_values();
		let buf = Buf::default().u16(1).u16(6).bytes(&values_buf.0).u16(1);
		let mut r = reader(&buf.0);
		r.read_object().expect("the UnitValues materializes");
		match r.read_object_as("UnitInfo", |o| matches!(o, SaveObject::Unit(_))) {
			Err(SaveError::ObjectTypeMismatch { expected, actual, .. }) => {
				assert_eq!((expected, actual), ("UnitInfo", "UnitValues"));
			}
			other => panic!("expected a class mismatch, got {other:?}"),
		}
	}

	#[test]
	fn an_untrusted_count_is_clamped_to_the_bytes_that_remain() {
		// `V71` widened counts to `u32`: reserving one unclamped is an OOM abort
		// before the short read it really is can be reported.
		let buf = [0u8; 16];
		let r = reader(&buf);
		assert_eq!(r.capacity_for(4_000_000_000, 2), 8, "clamped to what 16 bytes could hold");
		assert_eq!(r.capacity_for(3, 2), 3, "an honest count is left alone");
	}

	#[test]
	fn an_oversized_step_count_reports_a_short_read() {
		// End to end: a `GroundPath` claiming 65535 steps in a 4-byte tail errors
		// cleanly rather than reserving the count.
		let buf = Buf::default().i16(0).i16(0).u16(0).u16(u16::MAX);
		let mut r = reader(&buf.0);
		assert!(matches!(r.load_ground_path(), Err(SaveError::UnexpectedEof { .. })));
	}

	/// Builds a minimal V70 `UnitInfo` body with the given name and all object
	/// references null, matching the exact 140 + N scalar prefix.
	fn build_unit_info(name: &str) -> Buf {
		let n = name.len();
		Buf::default()
			.u16(0x33) // unit_type = TANK
			.u16(42) // id
			.u32(0x0000_2100) // flags (mobile land + red owner, illustrative)
			.u16(1216) // pixel_x
			.u16(1280) // pixel_y
			.i16(19) // grid_x
			.i16(20) // grid_y
			.u16(n as u16) // name length
			.bytes(name.as_bytes()) // name (no terminator)
			.zeros(4) // shadow_offset
			.u8(1) // team
			.zeros(1) // unit_id
			.zeros(1) // brightness
			.u8(5) // angle
			.zeros(5) // visible_to_team
			.zeros(5) // spotted_by_team
			.zeros(4) // max_velocity, velocity, sound, scaler_adjust
			.zeros(16) // sprite_bounds
			.zeros(16) // shadow_bounds
			.zeros(3) // turret_angle + 2 offsets
			.zeros(16) // 8 image i16 fields
			.u8(2) // orders = MOVE
			.u8(1) // state
			.u8(0) // prior_orders
			.u8(0) // prior_state
			.zeros(1) // laying_state
			.zeros(4) // grid-target block (V70: 2×i16)
			.zeros(1) // build_time
			.zeros(7) // mining fields
			.u8(30) // hits (u8 in V70)
			.u8(8) // speed (u8 in V70)
			.zeros(1) // shots
			.zeros(1) // move_and_fire
			.i16(-5) // storage
			.u8(3) // ammo
			.zeros(3) // targeting_mode, enter_mode, cursor
			.zeros(1) // recoil (V70: 1 byte)
			.zeros(8) // delayed_reaction..weapon
			.zeros(4) // comm block (V70: 4 bytes)
			.zeros(1) // repeat_build
			.u16(1) // build_rate
			.zeros(1) // disabled_reaction_fire
			.zeros(1) // auto_survey
			.zeros(4) // ai_state_bits
			// object refs — all null, and connectors between path and base_values:
			.u16(0) // path -> null
			.u16(0xABCD) // connectors
			.u16(0) // base_values -> null
			.u16(0) // complex -> null
			.u16(0) // parent_unit -> null
			.u16(0) // enemy_unit -> null
			.u16(0) // build_list count = 0
	}

	#[test]
	fn unit_info_scalar_prefix_alignment() {
		let name = "Rex";
		let buf = build_unit_info(name);
		// 140 + N scalar prefix, then 6×2 (path,connectors,base,complex,parent,
		// enemy) + 2 (build_list count) = 14 bytes of refs.
		assert_eq!(buf.0.len(), 140 + name.len() + 14);

		let mut r = reader(&buf.0);
		let u = r.load_unit_info().unwrap();
		assert_eq!(u.unit_type, 0x33);
		assert_eq!(u.id, 42);
		assert_eq!(u.flags, 0x0000_2100);
		assert_eq!((u.pixel_x, u.pixel_y), (1216, 1280));
		assert_eq!((u.grid_x, u.grid_y), (19, 20));
		assert_eq!(u.name, "Rex");
		assert_eq!(u.team, 1);
		assert_eq!(u.angle, 5);
		assert_eq!(u.orders, 2);
		assert_eq!(u.state, 1);
		assert_eq!(u.hits, 30);
		assert_eq!(u.ammo, 3);
		assert_eq!(u.storage, -5);
		assert_eq!(u.build_rate, 1);
		assert_eq!(u.connectors, 0xABCD);
		assert!(u.path.is_none() && u.base_values.is_none() && u.build_list.is_empty());
		assert_eq!(r.pos, buf.0.len(), "the whole record must be consumed exactly");
	}

	/// Full decode of the real turn-3 autosave, when the fixtures are present on
	/// this machine (skipped otherwise so CI without game assets stays green).
	#[test]
	fn decodes_real_save10_when_present() {
		let Some(home) = std::env::var_os("HOME") else { return };
		let save_path = std::path::Path::new(&home).join("MAX/SAVE10.DTA");
		let wrl_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals/SNOW_1.WRL");
		if !save_path.is_file() || !wrl_path.is_file() {
			crate::testutil::skip_fixture("decodes_real_save10_when_present: fixtures not found");
			return;
		}

		let wrl = crate::wrl::read_wrl_header(&wrl_path).unwrap();
		let save = read_save(&save_path, (wrl.width, wrl.height)).unwrap();

		assert_eq!((save.width, save.height), (112, 112));
		assert_eq!(save.turn_counter, 3);
		assert_eq!(save.header.save_name, "Auto-Saved turn 3");

		let total: usize = save.lists().iter().map(|(_, l)| l.len()).sum();
		assert_eq!(total, 9, "SAVE10 has nine units");
		assert_eq!(save.ground_cover.len(), 5);
		assert_eq!(save.mobile_land_sea.len(), 1);
		assert_eq!(save.stationary.len(), 3);
		assert!(save.mobile_air.is_empty() && save.particles.is_empty());

		// Every unit resolves to a named physical type whose flag classification
		// matches the list it came from, and buildings carry base stats.
		for (name, list) in save.lists() {
			let expected = expected_category(name);
			for &idx in list {
				let u = save.unit(idx).expect("list entry is a unit");
				assert!((u.unit_type as usize) < UNIT_END);
				assert!(u.type_name().is_some(), "unit_type {:#04x} has a name", u.unit_type);
				if let Some(cat) = expected {
					assert_eq!(u.category(), cat, "{name} unit {:#04x} classifies to its list", u.unit_type);
				}
				// The static id classifier agrees with the record's GROUND_COVER flag.
				assert_eq!(
					is_ground_cover_type(u.unit_type),
					u.is_ground_cover(),
					"{name} unit {:#04x} ground-cover id-classifier == flag",
					u.unit_type
				);
			}
		}
		for &idx in &save.stationary {
			let u = save.unit(idx).expect("stationary entry is a unit");
			assert!(u.base_values.and_then(|i| save.values(i)).is_some(), "building has base_values");
		}

		// The two connected buildings retain their strut geometry: the mining
		// station's stored WB connector (0x80, frames from connector_image_base
		// 16) and the power generator's ET connector (0x04, frames from base 2) -
		// the per-type bases the map overlay draws struts from. turret_angle is a
		// heading only for turret units (none in SAVE10); it holds engine scratch
		// otherwise, so it is used at render time only when < the turret frame count.
		let mining = save.units().find(|u| u.unit_type == 0x28).expect("mining station present");
		assert_eq!((mining.connectors, mining.connector_image_base), (0x0080, 16));
		let powgen = save.units().find(|u| u.unit_type == 0x02).expect("power generator present");
		assert_eq!((powgen.connectors, powgen.connector_image_base), (0x0004, 2));

		assert_unit_hash_consistent(&save);
		assert_map_hash_consistent(&save);

		// The engineer (ResourceID 0x3D) is mid-build (ORDER_BUILD = 4) and its
		// current HP equals its base max HP.
		let engineer = save.units().find(|u| u.unit_type == 0x3D).expect("engineer present");
		assert_eq!(engineer.orders, 4);
		let max_hits = engineer.base_values.and_then(|i| save.values(i)).unwrap().hits;
		assert_eq!(engineer.hits, max_hits);
		// The editor's hits-clamp helper resolves the same cap by the unit's id.
		assert_eq!(save.unit_max_hits(engineer.id), Some(max_hits), "unit_max_hits resolves by id");
	}

	/// The V71 fixture: a M.A.X. Port save on a custom 50×50 GREEN_3 map (paired
	/// with its world in `testdata/saves/`), decoded end-to-end. See
	/// `testdata/saves/README.md` for why the map must be bundled (the stored
	/// world hash is the *stock* GREEN_3 hash, so it can't convey the 50×50 size).
	#[test]
	fn decodes_v71_save11_when_present() {
		let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/saves");
		let save_path = base.join("save11-green3-50x50.dta");
		let wrl_path = base.join("GREEN_3-50x50.WRL");
		if !save_path.is_file() || !wrl_path.is_file() {
			crate::testutil::skip_fixture("decodes_v71_save11_when_present: fixtures not found");
			return;
		}

		let header = crate::save::read_save_header(&save_path).unwrap();
		assert_eq!(header.format, SaveFormat::V71);
		assert_eq!(header.category, crate::save::SaveCategory::Custom);
		assert_eq!(header.save_name, "WIP");
		assert_eq!(header.world_index, Some(14));
		assert_eq!(header.world_file, Some("GREEN_3.WRL"));

		// The paired world is the real 50×50 map (NOT the pristine 112×112 stock),
		// which is why it must be bundled with the save; the save does not store
		// its own dimensions.
		let wrl = crate::wrl::read_wrl_header(&wrl_path).unwrap();
		assert_eq!((wrl.width, wrl.height), (50, 50));

		let save = read_save(&save_path, (wrl.width, wrl.height)).unwrap();

		// V71 keeps a CTInfo per team including the alien slot (V70 has four).
		assert_eq!(save.teams.len(), 5);
		assert_eq!((save.width, save.height), (50, 50));
		assert_eq!(save.surface_map.len(), 2500);
		assert_eq!(save.cargo_map.len(), 2500);

		// Game scalars (u32, and turn_timer/game_state ordered oppositely to V70).
		assert_eq!(save.turn_counter, 79);
		assert_eq!(save.active_turn_team, 0);
		assert_eq!(save.player_team, 0);
		assert_eq!(save.game_state, 8);
		assert_eq!(save.turn_timer, 31);

		// Exact inventory — a regression lock on the whole object-graph walk.
		let counts: Vec<usize> = save.lists().iter().map(|(_, l)| l.len()).collect();
		assert_eq!(counts, vec![572, 154, 389, 14, 0], "per-list unit counts");
		assert_eq!(save.objects.len(), 1566, "total objects in graph");

		// If the walk were misaligned by even one byte the object types/positions
		// would go wild, so hold every unit to the engine's own invariants
		// (`UnitInfo::FileLoad` asserts grid bounds; types index the stat tables)
		// and to the flag classifier agreeing with the on-disk list it came from.
		let mut units = 0;
		for (name, list) in save.lists() {
			let expected = expected_category(name);
			for &idx in list {
				let u = save.unit(idx).expect("every list entry is a unit");
				assert!((0..50).contains(&u.grid_x), "grid_x {} out of bounds", u.grid_x);
				assert!((0..50).contains(&u.grid_y), "grid_y {} out of bounds", u.grid_y);
				assert!((u.unit_type as usize) < UNIT_END, "unit_type {} out of range", u.unit_type);
				assert!(u.team < 5, "team {} out of range", u.team);
				assert!(u.type_name().is_some(), "unit_type {:#04x} has a name", u.unit_type);
				if let Some(cat) = expected {
					assert_eq!(u.category(), cat, "{name} unit {:#04x} classifies to its list", u.unit_type);
				}
				units += 1;
			}
		}
		assert_eq!(units, 1129);

		// Buildings carry resolvable base stats through the shared object graph.
		let building = save.stationary.iter().find_map(|&i| save.unit(i)).expect("a stationary unit");
		assert!(building.base_values.and_then(|i| save.values(i)).is_some(), "building has base_values");

		assert_unit_hash_consistent(&save);
		assert_map_hash_consistent(&save);
	}
}
