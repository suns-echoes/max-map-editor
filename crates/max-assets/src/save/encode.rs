//! Typed structural encoder + **from-scratch save synthesis** for M.A.X. `V71`
//! save files (`SAVE-FROM-SCRATCH.md`, Stages A + C).
//!
//! Where [`super::serialize::write_save`] re-emits a *decoded* save byte-exactly
//! by leaning on the retained raw regions ([`super::types::RawRegions`]), this
//! module rebuilds each region **purely from the typed model**, so a save can be
//! constructed with no base `.DTA` at all. `write_save` stays the byte-exact
//! anchor for editing real saves (its S6.6 round-trip guard); the two paths agree
//! on the shared object graph via [`super::serialize::serialize_object_graph`].
//!
//! Two entry points:
//! - [`encode_save`] — the Stage A capstone: a full `V71` byte stream from a
//!   [`SaveFile`] (proven byte-exact against the reference fixture, region by
//!   region, by the tests below).
//! - [`synthesize_save`] / [`synthesize_save_bytes`] — the Stage C synthesizer:
//!   a complete, loadable [`SaveFile`] for a fresh human-only game built from a
//!   unit list + terrain/resource maps, with no base save. Fresh unit bodies come
//!   from [`encode_unit_prefix`] (the deploy-constructor recipe); stat tables,
//!   `Complex`es, spatial hashes (A4) and the human-only tail (A5) are all derived.
//!
//! Region encoders (`encode_header`/`encode_ct_info`/`encode_scalars`/
//! `encode_extra_settings`) are the inverses of the matching `decode` readers.

use super::error::EditError;
use super::types::{CtInfo, MapHash, SaveExtraSettings, SaveFile, SaveFormat, SaveHeader, SaveObject, TEAM_COUNT};
use super::unit_types::flag::{BUILDING, GROUND_COVER};

/// `HASH_HASH_SIZE` (`hash.cpp`) — the fixed 512-bucket count both spatial hashes
/// use. Every stock save carries exactly this.
const HASH_SIZE: usize = 512;
/// The map hash's position-mix shift for a 512-bucket hash (`(y ^ (x << 2))`).
const MAP_HASH_X_SHIFT: u16 = 2;

/// A little-endian byte sink mirroring `SmartFileWriter` (`smartfile.cpp`) — the
/// inverse of `decode`'s `Reader`. The shared workhorse for the Stage A encoders;
/// methods are added as tickets need them.
#[derive(Default)]
struct LeWriter(Vec<u8>);

impl LeWriter {
	fn u8(&mut self, v: u8) {
		self.0.push(v);
	}

	fn i8(&mut self, v: i8) {
		self.0.push(v as u8);
	}

	fn u16(&mut self, v: u16) {
		self.0.extend_from_slice(&v.to_le_bytes());
	}

	fn i16(&mut self, v: i16) {
		self.0.extend_from_slice(&v.to_le_bytes());
	}

	fn u32(&mut self, v: u32) {
		self.0.extend_from_slice(&v.to_le_bytes());
	}

	fn i32(&mut self, v: i32) {
		self.0.extend_from_slice(&v.to_le_bytes());
	}

	fn bytes(&mut self, b: &[u8]) {
		self.0.extend_from_slice(b);
	}

	/// Appends `n` zero bytes — a run of default/empty state (heat maps, padding).
	fn zeros(&mut self, n: usize) {
		self.0.resize(self.0.len() + n, 0);
	}

	/// A `SmartFileWriter` length-prefixed block: a `u32` length then the raw bytes
	/// (no terminator) — how strings and `vector<u8>` are written in `V71`.
	fn v71_bytes(&mut self, b: &[u8]) {
		self.u32(b.len() as u32);
		self.bytes(b);
	}

	fn v71_str(&mut self, s: &str) {
		self.v71_bytes(s.as_bytes());
	}

	fn into_vec(self) -> Vec<u8> {
		self.0
	}
}

/// Encodes the `V71` save header + options (regions 1–12, `saveload.cpp:384-556`)
/// purely from the typed [`SaveHeader`] — the inverse of `read_header_from`. Byte-
/// exact against a real save's retained `raw.header`.
///
/// Two regions the decoder consumes but does not surface are **derived** from the
/// options block, which max-port fills them from: the per-team difficulty[5]
/// (region 9) is the `opponent` setting written five times, and the pre-options
/// timer/endturn/play_mode triple (region 11) duplicates the same three options.
/// The byte-exact round-trip test is the guard that this derivation holds.
pub(crate) fn encode_header(h: &SaveHeader) -> Vec<u8> {
	debug_assert_eq!(h.format, SaveFormat::V71, "the encoder emits V71 only (SAVE-FROM-SCRATCH.md §1)");
	let o = &h.options;
	let mut w = LeWriter::default();

	w.u32(SaveFormat::V71.version() as u32); // 1: version
	w.u32(h.category.mission_category_index()); // 2: mission category
	w.v71_bytes(&h.script); // 3: mission binary script
	w.v71_str(&h.save_name); // 4: save title
	w.v71_str(h.world_hash.as_deref().unwrap_or("")); // 5: world hash
	for name in &h.team_names {
		w.v71_str(name); // 6: team names[5]
	}
	for &t in &h.team_type {
		w.u32(t); // 7: team types[5]
	}
	for &c in &h.team_clan {
		w.u32(c); // 8: team clans[5]
	}
	for _ in 0..TEAM_COUNT {
		w.i32(o.opponent); // 9: difficulty[5] == the opponent setting, per team
	}
	w.u32(h.rng_seed); // 10: rng seed
	w.i32(o.timer); // 11: pre-options duplicate of…
	w.i32(o.endturn);
	w.i32(o.play_mode);
	// 12: the twelve game options, in disk order.
	for v in [
		o.world,
		o.timer,
		o.endturn,
		o.start_gold,
		o.play_mode,
		o.victory_type,
		o.victory_limit,
		o.opponent,
		o.raw_resource,
		o.fuel_resource,
		o.gold_resource,
		o.alien_derelicts,
	] {
		w.i32(v);
	}
	w.into_vec()
}

/// Encodes one `V71` per-team `CTInfo` block (988 bytes, `saveload.cpp:598-634`)
/// from the typed [`CtInfo`] — the inverse of `decode`'s `load_ct_info_v71`, in the
/// exact writer field order. `format` documents the target layout; only `V71` is
/// emitted (`SAVE-FROM-SCRATCH.md` §11).
pub(crate) fn encode_ct_info(ct: &CtInfo, format: SaveFormat) -> Vec<u8> {
	debug_assert_eq!(format, SaveFormat::V71, "only the V71 CTInfo layout is encoded");
	let mut w = LeWriter::default();
	w.u8(ct.team_type);
	w.u8(ct.finished_turn as u8);
	w.u8(ct.team_clan);
	for topic in &ct.research_topics {
		for &v in topic {
			w.i32(v);
		}
	}
	w.u32(ct.team_points);
	w.u16(ct.number_of_objects_created);
	for &c in &ct.unit_counters {
		w.u32(c);
	}
	for loc in &ct.screen_locations {
		w.i8(loc[0]);
		w.i8(loc[1]);
	}
	for &s in &ct.score_graph {
		w.i16(s);
	}
	w.u16(ct.selected_unit_id);
	w.u16(ct.zoom_level);
	w.i16(ct.camera_x);
	w.i16(ct.camera_y);
	for &b in &ct.display_buttons {
		w.i8(b);
	}
	for &s in &ct.stats {
		w.i16(s);
	}
	for &c in &ct.casualties {
		w.u32(c);
	}
	w.u32(ct.stats_gold_spent_on_upgrades);
	debug_assert_eq!(w.0.len(), 988, "a V71 CTInfo block is exactly 988 bytes");
	w.into_vec()
}

/// Encodes the `V71` game scalars + cheater pair (regions 17–18, 28 bytes,
/// `saveload.cpp:638-682`) from the typed model, in exact writer order: active
/// team, player team, turn counter, turn timer, game state, then the sticky
/// is_cheater / cheater_team pair. `V71`-only (`V70` uses a narrower, differently
/// ordered scalar layout with no cheater pair, never re-encoded).
pub(crate) fn encode_scalars(save: &SaveFile) -> Vec<u8> {
	debug_assert_eq!(save.header.format, SaveFormat::V71, "only the V71 scalar layout is encoded");
	let mut w = LeWriter::default();
	w.u32(save.active_turn_team as u32);
	w.u32(save.player_team as u32);
	w.i32(save.turn_counter);
	w.u32(save.turn_timer as u32);
	w.u32(save.game_state as u32);
	w.u32(save.is_cheater);
	w.u32(save.cheater_team);
	debug_assert_eq!(w.0.len(), 28, "V71 scalars + cheater pair are 28 bytes");
	w.into_vec()
}

/// Encodes the seven "extra settings" `i32`s (region 13, `saveload.cpp:556-564`)
/// from the typed [`SaveExtraSettings`]. In `V71` this block follows the header's
/// options contiguously; byte-exact against a real save's `raw.extra_settings`.
pub(crate) fn encode_extra_settings(e: &SaveExtraSettings) -> Vec<u8> {
	let mut w = LeWriter::default();
	for v in [e.effects, e.click_scroll, e.quick_scroll, e.fast_movement, e.follow_unit, e.auto_select, e.enemy_halt] {
		w.i32(v);
	}
	w.into_vec()
}

/// Builds a `Hash_UnitHash` structure (512 buckets of object indices) from the
/// units alone — each listed unit dropped into `bucket = id % 512`
/// (`hash.cpp:294`). For save synthesis, where there is no retained on-disk hash to
/// copy. Intra-bucket order is free for loadability (the engine's lookup is a linear
/// id scan), so this need not reproduce a captured save's game-history order. `lists`
/// are the unit lists whose entries index `objects` — pass all five for the full
/// hash. The structural inverse of `serialize`'s emission consumes this directly.
pub(crate) fn derive_unit_hash(objects: &[SaveObject], lists: &[&[usize]]) -> Vec<Vec<usize>> {
	let mut buckets = vec![Vec::new(); HASH_SIZE];
	for list in lists {
		for &idx in *list {
			if let Some(SaveObject::Unit(u)) = objects.get(idx) {
				buckets[u.id as usize % HASH_SIZE].push(idx);
			}
		}
	}
	buckets
}

/// Builds a `Hash_MapHash` from the units alone by replaying [`MapHash::add_unit`]
/// for each unit at its grid cell (`hash.cpp` `MapHash::Add`). For save synthesis.
/// `lists` are the on-map unit lists (ground cover, mobile land/sea, stationary,
/// mobile air) in on-disk order, so a building's 2×2 footprint insertion matches the
/// game. A synthesized save places units at rest, so every cell sits exactly on its
/// unit's footprint (no mid-move off-grid entries a captured save may carry).
pub(crate) fn derive_map_hash(objects: &[SaveObject], lists: &[&[usize]]) -> MapHash {
	let mut mh =
		MapHash { hash_size: HASH_SIZE as u16, x_shift: MAP_HASH_X_SHIFT, buckets: vec![Vec::new(); HASH_SIZE] };
	for list in lists {
		for &idx in *list {
			if let Some(SaveObject::Unit(u)) = objects.get(idx) {
				let building = u.flags & BUILDING != 0;
				let ground_cover = u.flags & GROUND_COVER != 0;
				mh.add_unit(idx, u.grid_x as u16, u.grid_y as u16, u.unit_type, building, ground_cover);
			}
		}
	}
	mh
}

/// Encodes the tail of a fresh **human-only** game (regions 23–25), which is all
/// zero. Per non-NONE team among slots 0–3 it emits a `w·h·12`-byte all-zero heat
/// map (`HeatMap::Save` writes `width·height` cells of three `u32`s each); slot 4
/// (alien) is never emitted because the `V71` reader stops at slot 3 (`SAVE-FROM-
/// SCRATCH.md` §6.3 desync trap). It then writes four `u32` message-log counts of
/// zero (16 B — the reader always reads four, one per non-alien team, empty here),
/// and finally nothing: with no `TEAM_TYPE_COMPUTER` team, `Ai_FileSave` emits zero
/// bytes (§6). `team_types` are the five slots' `TEAM_TYPE_*` values (0 = NONE).
pub(crate) fn encode_tail_fresh(w: u16, h: u16, team_types: [u8; TEAM_COUNT]) -> Vec<u8> {
	const TEAM_TYPE_NONE: u8 = 0;
	let heat_map_bytes = w as usize * h as usize * 12;
	let mut out = LeWriter::default();
	// Region 23: an all-zero heat map per non-NONE team among the reader's slots 0–3.
	for &tt in team_types.iter().take(4) {
		if tt != TEAM_TYPE_NONE {
			out.zeros(heat_map_bytes);
		}
	}
	// Region 24: four u32 message-log counts, all zero (empty logs).
	for _ in 0..4 {
		out.u32(0);
	}
	// Region 25: AI state — nothing for a human-only game.
	out.into_vec()
}

/// The Stage A capstone: encodes a complete `V71` save from the typed model.
/// Regions 1–18 come from the typed encoders (A1–A3), regions 19–22 (the object
/// graph + spatial hashes) from the shared structural serializer, and the tail
/// (regions 23–25) from `save.raw.tail` — a real save's verbatim tail, or, for a
/// synthesized save, the [`encode_tail_fresh`] bytes the synthesizer stored there.
///
/// Proves the whole structural spine regenerates from typed data with no reliance
/// on the retained header/ct_info/scalar raw regions — the foundation Stages B–D
/// build on. `V71`-only (`SAVE-FROM-SCRATCH.md` §1).
pub(crate) fn encode_save(save: &SaveFile) -> Vec<u8> {
	debug_assert_eq!(save.header.format, SaveFormat::V71, "encode_save emits V71 only");
	let mut out = LeWriter::default();
	out.bytes(&encode_header(&save.header)); // 1–12
	out.bytes(&encode_extra_settings(&save.extra_settings)); // 13
	out.bytes(&save.surface_map); // 14
	for &v in &save.cargo_map {
		out.u16(v); // 15
	}
	for ct in &save.teams {
		out.bytes(&encode_ct_info(ct, SaveFormat::V71)); // 16
	}
	out.bytes(&encode_scalars(save)); // 17–18
	let (graph, _span, _order) = super::serialize::serialize_object_graph(save); // 19–22
	out.bytes(&graph);
	out.bytes(&save.raw.tail); // 23–25
	out.into_vec()
}

/// Everything needed to synthesize one fresh, idle unit body (Stage C1) — the
/// inputs `UnitInfo::UnitInfo(type, team, id, angle)` (max-port
/// `unitinfo.cpp:170`) takes plus the editor-visible properties. All runtime
/// scratch is emitted at the deploy-constructor's values; the engine re-derives
/// the rest on load (`FileLoad` → `Init()` + `UpdateUnitDrawZones()`).
#[derive(Debug, Clone)]
pub struct FreshUnit {
	pub unit_type: u16,
	/// Spatial-hash id (unique per unit; the synthesizer allocates
	/// `(team << 13) | counter` like the exporter's add path).
	pub id: u16,
	/// Full engine flag word: the type's `SC_UNITS` flags | the owning team's
	/// `HASH_TEAM_*` bit.
	pub flags: u32,
	pub grid_x: i16,
	pub grid_y: i16,
	/// Custom name ("" = the type's default name).
	pub name: String,
	pub team: u8,
	/// Clan of the owning team (`TEAM_CLAN_*` 1..=8) — only the MININGST
	/// `image_base = (clan−1)·2` quirk reads it.
	pub clan: u8,
	/// This team's per-type build number (`unit_id` byte; the deploy path hands
	/// out `unit_counters[type]` then increments — the synthesizer does the same).
	pub unit_serial: u8,
	pub angle: u8,
	pub turret_angle: u8,
	pub orders: u8,
	pub disabled_turns: u8,
	/// Current hit points (normally `values.hits`).
	pub hits: u16,
	pub ammo: u8,
	/// Cargo carried / experience.
	pub storage: i16,
	/// Connector adjacency bitmask.
	pub connectors: u16,
	/// The team's current `UnitValues` for this type — seeds speed/shots/
	/// move&fire and the weapon/engine bytes.
	pub values: super::types::UnitValues,
	/// The type's `D_*` frame table (image bases + turret offsets).
	pub frame: crate::attribs::FrameInfo,
	/// Frame count of the sprite resource (`ImageMultiHeader::image_count`) —
	/// pass 0 when unknown; the engine only reads frames it indexes, and every
	/// index we emit is bounded by `image_index_max`.
	pub total_images: i16,
	/// `total/raw/fuel/gold` mining + `raw/gold/fuel` mining caps — nonzero
	/// only for a mining station placed over surveyed resources (the engine
	/// trusts these on load; it derives them at *build* time, not load time).
	pub mining: [u8; 7],
}

/// `MOBILE_AIR_UNIT | HOVERING` — such units draw their shadow a full tile
/// offset (`UnitInfo::Redraw`, `unitinfo.cpp:1433`).
const AIR_SHADOW_FLAGS: u32 = 0x40 | 0x10000;

/// Encodes a fresh unit's **opaque body prefix** (everything before the object
/// references) exactly as `UnitInfo::FileSave` would write a just-deployed,
/// idle unit, and returns it with the standard [`UnitBodyLayout`] so the
/// shared graph emitter, the scalar exporter and the integrity pass treat the
/// synthesized unit like any decoded one. The refs themselves (`path`,
/// `base_values`, `complex`, …) are emitted by `serialize_object_graph` from
/// the typed [`super::types::UnitRecord`].
pub(crate) fn encode_unit_prefix(u: &FreshUnit) -> (Vec<u8>, super::types::UnitBodyLayout) {
	let mut w = LeWriter::default();
	w.u16(u.unit_type);
	w.u16(u.id);
	w.u32(u.flags);
	let pixel_x = w.0.len();
	// Pixel position = the cell centre (grid·64 + 32) — where the engine puts
	// an at-rest unit; `UpdateUnitDrawZones` rebuilds the draw rects from it.
	w.u16((u.grid_x as u16).wrapping_mul(64).wrapping_add(32));
	w.u16((u.grid_y as u16).wrapping_mul(64).wrapping_add(32));
	let grid_x = w.0.len();
	w.i16(u.grid_x);
	w.i16(u.grid_y);
	let name = w.0.len();
	let name_len = u.name.len();
	w.u16(name_len as u16);
	w.bytes(u.name.as_bytes());
	// shadow_offset: (0,0), except hovering air units (−64,−64) (`Redraw:1433`).
	if u.flags & AIR_SHADOW_FLAGS == AIR_SHADOW_FLAGS {
		w.i16(-64);
		w.i16(-64);
	} else {
		w.i16(0);
		w.i16(0);
	}
	let team = w.0.len();
	w.u8(u.team);
	w.u8(u.unit_serial); // unit_id — this team's per-type build number
	w.u8(0xFF); // brightness (constructor value)
	let angle = w.0.len();
	w.u8(u.angle);
	for t in 0..5u8 {
		w.u8((t == u.team) as u8); // visible_to_team: own team only
	}
	w.zeros(5); // spotted_by_team
	w.zeros(4); // max_velocity, velocity, sound (SFX_TYPE_INVALID), scaler_adjust
	w.zeros(32); // sprite_bounds + shadow_bounds — recomputed by UpdateUnitDrawZones
	let turret_angle = w.0.len();
	w.u8(u.turret_angle);
	let (tox, toy) = u.frame.angle_offsets[(u.angle & 7) as usize];
	w.i8(tox);
	w.i8(toy);
	let image_block = w.0.len();
	// The 8×i16 image block, at deploy values (`unitinfo.cpp:263-288`).
	let image_base = if super::unit_types::unit_type_name(u.unit_type) == Some("MININGST") && u.clan >= 1 {
		(u.clan as i16 - 1) * 2
	} else {
		u.frame.image_base
	};
	// The four storage buildings show `image_base + 1` from deploy on
	// (`UnitsManager_DeployUnit` draws that frame explicitly).
	let image_index = image_base + u.angle as i16 + super::unit_types::deploy_frame_bump(u.unit_type);
	let turret_image_index = u.frame.turret_image_base + u.angle as i16;
	let image_index_max = match super::unit_types::unit_type_name(u.unit_type) {
		Some("COMMANDO") | Some("INFANTRY") => 103,
		_ => u.frame.image_count + image_index - 1,
	};
	w.i16(u.total_images);
	w.i16(image_base);
	w.i16(u.frame.turret_image_base);
	w.i16(u.frame.firing_image_base);
	w.i16(u.frame.connector_image_base);
	w.i16(image_index);
	w.i16(turret_image_index);
	w.i16(image_index_max);
	let orders = w.0.len();
	// The engine's deploy state pair: a power-on host starts at INIT so the
	// first game tick runs PowerUp (complex bookkeeping + lit frame); every
	// other order starts settled at EXECUTING_ORDER.
	let state = super::orders::deploy_state_for(u.orders);
	w.u8(u.orders);
	w.u8(state);
	w.u8(u.orders); // prior_orders (constructor mirrors orders)
	w.u8(state);
	w.u8(0); // laying_state
	let move_to = w.0.len();
	w.zeros(8); // move_to / fire_on grid targets (idle: none)
	let build_time = w.0.len();
	w.u8(0); // build_time
	w.bytes(&u.mining); // total/raw/fuel/gold mining + raw/gold/fuel caps
	let hits = w.0.len();
	w.u16(u.hits);
	w.u16(u.values.speed); // full movement points
	w.u8(u.values.rounds.min(u8::MAX as u16) as u8); // shots this turn
	w.u8(u.values.move_and_fire);
	let storage = w.0.len();
	w.i16(u.storage);
	w.i16(0); // experience
	w.i16(0); // transfer_cargo
	w.u8(0); // stealth_dice_roll
	let ammo = w.0.len();
	w.u8(u.ammo);
	w.zeros(3); // targeting_mode, enter_mode, cursor (CURSOR_HIDDEN)
	w.u8(0); // firing_recoil_frames
	let disabled = w.0.len();
	w.u8(u.disabled_turns);
	w.zeros(3); // delayed_reaction, damaged_this_turn, research_topic
	let moved = w.0.len();
	w.u8(0); // moved
	w.u8(0); // bobbed
	w.u8(0); // shake_effect_state
	w.u8(2); // engine (constructor value)
	w.u8(if u.values.attack > 0 { 2 } else { 0 }); // weapon
	w.u8(0); // move_fraction
	w.u8(0); // repeat_build
	w.u16(1); // build_rate (FileLoad floors 0 → 1 anyway)
	w.u8(0); // disabled_reaction_fire
	w.u8(0); // auto_survey
	w.u32(0); // ai_state_bits
	let refs_off = w.0.len();
	let layout = super::types::UnitBodyLayout {
		name,
		name_len,
		pixel_x,
		grid_x,
		team,
		angle,
		turret_angle,
		image_block,
		orders,
		move_to,
		build_time,
		moved,
		hits,
		hits_width: 2,
		ammo,
		disabled,
		disabled_dual: false,
		storage,
		// `connectors` and the `base_values` ref live in the ref section the
		// graph emitter writes from the typed record; a fresh unit's `path` is
		// null = one 4-byte V71 index of 0, so connectors follows at +4. (These
		// offsets are only read again after the synthesized bytes round-trip
		// through the decoder, which recaptures them authoritatively.)
		connectors: refs_off + 4,
		base_values_ref: refs_off + 6,
		refs_off,
	};
	(w.into_vec(), layout)
}

/// One unit to synthesize into a fresh save (Stage C2) — the editor-authored
/// properties; everything else derives from the unit database + frame table.
#[derive(Debug, Clone)]
pub struct SynthUnit {
	pub unit_type: u16,
	pub grid_x: i16,
	pub grid_y: i16,
	pub team: u8,
	pub angle: u8,
	pub turret_angle: u8,
	pub name: String,
	pub orders: u8,
	pub disabled_turns: u8,
	/// Current hit points (`None` = full, from the effective max stats).
	pub hits: Option<u16>,
	/// Current ammunition (`None` = full).
	pub ammo: Option<u8>,
	/// Cargo carried / experience (`None` = 0, the engine's fresh-deploy state).
	pub storage: Option<i16>,
	pub connectors: u16,
	/// Per-unit max-stat override (the editor's `object-values` fork); `None` =
	/// the team's shared table entry.
	pub base_values: Option<super::types::UnitValues>,
	/// `total/raw/fuel/gold` mining + `raw/gold/fuel` caps — for mining
	/// stations placed over resources (zeros otherwise).
	pub mining: [u8; 7],
}

/// Inputs for [`synthesize_save`] — a fresh, human-only, turn-1 game.
#[derive(Debug, Clone)]
pub struct SynthesisParams {
	pub save_name: String,
	/// The stored world hash — a stock slot's hash (swapped-`.WRL` workflow) or
	/// `None`/empty for a custom world.
	pub world_hash: Option<String>,
	/// The `options.world` index (the slot the save claims).
	pub world: i32,
	pub width: u16,
	pub height: u16,
	pub rng_seed: u32,
	/// Per-slot team type: 0 = none, 1 = player. **No computer teams** (a
	/// computer team forces full AI serialization, §6.2).
	pub team_types: [u8; TEAM_COUNT],
	/// Per-slot clan (`TEAM_CLAN_*` 1..=8; 0 = none/random → plain base stats).
	pub team_clans: [u8; TEAM_COUNT],
	pub team_names: [String; TEAM_COUNT],
	pub start_gold: i32,
	/// Terrain passability per cell (`SURFACE_TYPE_*`), row-major `w·h`.
	pub surface_map: Vec<u8>,
	/// Resource distribution per cell, row-major `w·h`.
	pub cargo_map: Vec<u16>,
	pub units: Vec<SynthUnit>,
}

/// The mission JSON a Custom save embeds as its `V71` region-3 script — the
/// exact bytes max-port itself writes into every custom save (verified against
/// a real max-port save, byte-for-byte). The engine's load path XOR-decodes
/// region 3 and JSON-parses it against its mission schema
/// (`saveloadmenu.cpp:505` -> `Mission::LoadBinaryBuffer`, `mission.cpp:574`),
/// and a parse failure REJECTS the save: the slot lists fine in the load menu
/// but Load silently bounces back to the main menu. So an empty script — what
/// the synthesizer used to emit — is a save the game can never load.
const CUSTOM_MISSION_JSON: &str = concat!(
	"{\"$schema\":\"http://json-schema.org/draft-07/schema#\",",
	"\"author\":\"Interplay Productions\",",
	"\"category\":\"Custom\",",
	"\"copyright\":\"(c) 1996 Interplay Productions\",",
	"\"description\":{\"text\":{\"en-US\":\"\"}},",
	"\"license\":\"All rights reserved\",",
	"\"title\":{\"text\":{\"en-US\":\"\"}}}"
);

/// [`CUSTOM_MISSION_JSON`] obfuscated the way the engine stores it (XOR with
/// the generic table — an involution, so [`crate::attribs::deobfuscate`]
/// applies it in either direction).
pub fn custom_mission_script() -> Vec<u8> {
	let mut bytes = CUSTOM_MISSION_JSON.as_bytes().to_vec();
	crate::attribs::deobfuscate(&mut bytes);
	bytes
}

/// `TEAM_TYPE_PLAYER`.
const TEAM_TYPE_PLAYER: u8 = 1;
/// The flag set whose owners get a `Complex` attached on deploy
/// (`unitinfo.cpp:310`): `CONNECTOR_UNIT | BUILDING | STANDALONE`, minus
/// ground cover.
const COMPLEX_FLAGS: u32 = 0x8 | 0x10 | 0x0080_0000;
/// `HASH_TEAM_*` owner bits by team slot (`unit_types::flag`).
const HASH_TEAM_BITS: [u32; TEAM_COUNT] = [0x2000, 0x1000, 0x800, 0x400, 0x8000];

/// Synthesizes a complete, structurally-valid V71 [`SaveFile`] for a fresh
/// human-only game from a unit list + maps — **no base save**. The result
/// encodes via [`encode_save`] and decodes back through the standard reader;
/// engine-load verification is Stage E.
pub fn synthesize_save(
	p: &SynthesisParams,
	db: &crate::attribs::UnitStatsDb,
	frames: &[Option<crate::attribs::FrameInfo>; super::types::UNIT_END],
) -> Result<SaveFile, EditError> {
	use super::types::{ObjMeta, SaveCategory, SaveObject, UNIT_END, UnitRecord};
	let cells = p.width as usize * p.height as usize;
	if p.surface_map.len() != cells || p.cargo_map.len() != cells {
		return Err(EditError::InvalidInput(format!(
			"synthesize: map sizes ({}/{}) do not match {}x{}",
			p.surface_map.len(),
			p.cargo_map.len(),
			p.width,
			p.height
		)));
	}
	if p.team_types.iter().any(|&t| t > TEAM_TYPE_PLAYER) {
		return Err(EditError::InvalidInput(
			"synthesize: only NONE/PLAYER team types are supported (no AI state is written)".into(),
		));
	}
	for (i, u) in p.units.iter().enumerate() {
		if u.unit_type as usize >= UNIT_END {
			return Err(EditError::InvalidInput(format!("synthesize: unit {i} has unknown type {}", u.unit_type)));
		}
		let name = super::unit_types::unit_type_name(u.unit_type).unwrap_or("?");
		if u.team as usize >= TEAM_COUNT || p.team_types[u.team as usize] != TEAM_TYPE_PLAYER {
			return Err(EditError::InvalidInput(format!(
				"synthesize: {name} belongs to team {} which is not a player",
				u.team + 1
			)));
		}
		if u.grid_x < 0 || u.grid_y < 0 || u.grid_x as u16 >= p.width || u.grid_y as u16 >= p.height {
			return Err(EditError::InvalidInput(format!(
				"synthesize: {name} is off the map at {},{}",
				u.grid_x, u.grid_y
			)));
		}
		if frames[u.unit_type as usize].is_none() {
			return Err(EditError::InvalidInput(format!(
				"synthesize: no frame info for {name} (MAX.RES missing its D_* resource?)"
			)));
		}
		// A zero flag word would route the unit into the wrong list, hash it as
		// a 1-cell mobile, and leave it in no render bucket in-game — refuse
		// rather than emit an invisible unit.
		if db.meta_for(u.unit_type).is_none_or(|m| m.flags == 0) {
			return Err(EditError::InvalidInput(format!(
				"synthesize: the unit database has no flags for {name} (SC_UNITS entry missing?)"
			)));
		}
	}
	// The engine null-derefs GetFirstRelevantUnit(player) on load (§6.1) — every
	// player team must own at least one unit.
	for slot in 0..TEAM_COUNT {
		if p.team_types[slot] == TEAM_TYPE_PLAYER && !p.units.iter().any(|u| u.team as usize == slot) {
			return Err(EditError::InvalidInput(format!(
				"synthesize: player team {} owns no units - place at least one",
				slot + 1
			)));
		}
	}

	let mut objects: Vec<SaveObject> = Vec::new();
	let mut metas: Vec<ObjMeta> = Vec::new();

	// --- Team stat tables (4 slots; the alien slot has no TeamUnits) --------
	// A fresh game's base and current tables point at the SAME object per type
	// (`InitClanUnitValues` stores one clone into both), so each team gets 93
	// UnitValues objects referenced from both columns.
	let mut team_units = Vec::with_capacity(4);
	let mut values_index = [[0usize; UNIT_END]; 4]; // per team slot, per type
	for (slot, values) in values_index.iter_mut().enumerate() {
		let clan = p.team_clans[slot];
		let table = db.clan_unit_values(if p.team_types[slot] == TEAM_TYPE_PLAYER { clan } else { 0 });
		let mut base = Vec::with_capacity(UNIT_END);
		for (ty, mut v) in table.into_iter().enumerate() {
			// The engine marks a stat block in-use when a unit takes it.
			v.in_use = p.units.iter().any(|u| u.team as usize == slot && u.unit_type as usize == ty);
			let idx = objects.len();
			metas.push(ObjMeta {
				type_index: 6, // UnitValues
				contained: 1,
				body_raw: super::serialize::serialize_unit_values(&v),
				unit_layout: None,
			});
			objects.push(SaveObject::Values(v));
			values[ty] = idx;
			base.push(Some(idx));
		}
		let gold = if p.team_types[slot] == TEAM_TYPE_PLAYER {
			(p.start_gold + db.clans.get(clan.wrapping_sub(1) as usize).map_or(0, |c| c.credits)).max(0) as u32
		} else {
			0
		};
		team_units.push(super::types::TeamUnitsTable {
			gold,
			base_values: base.clone(),
			current_values: base,
			complexes: Vec::new(), // filled below
		});
	}

	// --- Complexes: connected components of complex-hosting units -----------
	// Two hosts join when their footprints are edge-adjacent and both carry a
	// connector mask (the editor's auto-connect sets the masks). Everything
	// else gets its own complex, like the deploy constructor.
	let type_flags = |ty: u16| db.meta_for(ty).map_or(0, |m| m.flags);
	let hosts: Vec<usize> = (0..p.units.len())
		.filter(|&i| {
			let f = type_flags(p.units[i].unit_type);
			f & COMPLEX_FLAGS != 0 && f & GROUND_COVER == 0
		})
		.collect();
	let footprint = |u: &SynthUnit| if type_flags(u.unit_type) & BUILDING != 0 { 2i16 } else { 1 };
	let mut component: Vec<usize> = (0..hosts.len()).collect();
	fn root(component: &mut [usize], mut i: usize) -> usize {
		while component[i] != i {
			component[i] = component[component[i]];
			i = component[i];
		}
		i
	}
	for a in 0..hosts.len() {
		for b in a + 1..hosts.len() {
			let (ua, ub) = (&p.units[hosts[a]], &p.units[hosts[b]]);
			if ua.team != ub.team || ua.connectors == 0 || ub.connectors == 0 {
				continue;
			}
			let (fa, fb) = (footprint(ua), footprint(ub));
			// Edge-adjacent footprints: ranges overlap on one axis, touch on the other.
			let overlap_x = ua.grid_x < ub.grid_x + fb && ub.grid_x < ua.grid_x + fa;
			let overlap_y = ua.grid_y < ub.grid_y + fb && ub.grid_y < ua.grid_y + fa;
			let touch_x = ua.grid_x + fa == ub.grid_x || ub.grid_x + fb == ua.grid_x;
			let touch_y = ua.grid_y + fa == ub.grid_y || ub.grid_y + fb == ua.grid_y;
			if (overlap_x && touch_y) || (overlap_y && touch_x) {
				let (ra, rb) = (root(&mut component, a), root(&mut component, b));
				component[ra] = rb;
			}
		}
	}
	// One Complex object per component, ids 1.. per team (CreateComplex order).
	let mut complex_of_unit: Vec<Option<usize>> = vec![None; p.units.len()];
	let mut comp_object: Vec<Option<usize>> = vec![None; hosts.len()];
	for (slot, team) in team_units.iter_mut().enumerate() {
		let mut next_id = 1i16;
		for h in 0..hosts.len() {
			if p.units[hosts[h]].team as usize != slot || root(&mut component, h) != h {
				continue;
			}
			let members = (0..hosts.len()).filter(|&k| root(&mut component, k) == h).count();
			let complex = super::types::Complex {
				material: 0,
				fuel: 0,
				gold: 0,
				power: 0,
				workers: 0,
				buildings: members as i16,
				id: next_id,
			};
			next_id += 1;
			let body = super::serialize::serialize_complex(&complex);
			let idx = objects.len();
			metas.push(ObjMeta { type_index: 3, contained: 1, body_raw: body, unit_layout: None });
			objects.push(SaveObject::Complex(complex));
			comp_object[h] = Some(idx);
			team.complexes.push(idx);
		}
	}
	for h in 0..hosts.len() {
		let r = root(&mut component, h);
		complex_of_unit[hosts[h]] = comp_object[r];
	}

	// --- Units ---------------------------------------------------------------
	let mut lists: [Vec<usize>; 5] = Default::default();
	let mut id_counter = [0u16; TEAM_COUNT];
	let mut serials = [[0u8; UNIT_END]; TEAM_COUNT];
	let mut unit_counters = [[1u32; UNIT_END]; TEAM_COUNT];
	for (i, su) in p.units.iter().enumerate() {
		let slot = su.team as usize;
		let ty = su.unit_type as usize;
		let flags = type_flags(su.unit_type) | HASH_TEAM_BITS[slot];
		// Per-unit override, else the team's shared (possibly clan-upgraded) stats.
		let shared = match &objects[values_index[slot][ty]] {
			SaveObject::Values(v) => v.clone(),
			_ => unreachable!("team table slots hold UnitValues"),
		};
		let (values, values_ref) = match &su.base_values {
			Some(v) => {
				let mut v = v.clone();
				v.in_use = true;
				let idx = objects.len();
				metas.push(ObjMeta {
					type_index: 6,
					contained: 1,
					body_raw: super::serialize::serialize_unit_values(&v),
					unit_layout: None,
				});
				objects.push(SaveObject::Values(v.clone()));
				(v, idx)
			}
			None => (shared, values_index[slot][ty]),
		};
		id_counter[slot] += 1;
		serials[slot][ty] = serials[slot][ty].wrapping_add(1);
		unit_counters[slot][ty] += 1;
		let fresh = FreshUnit {
			unit_type: su.unit_type,
			id: ((slot as u16) << 13) | id_counter[slot],
			flags,
			grid_x: su.grid_x,
			grid_y: su.grid_y,
			name: su.name.clone(),
			team: su.team,
			clan: p.team_clans[slot],
			unit_serial: serials[slot][ty],
			angle: su.angle & 7,
			turret_angle: su.turret_angle & 7,
			orders: su.orders,
			disabled_turns: su.disabled_turns,
			hits: su.hits.unwrap_or(values.hits),
			ammo: su.ammo.unwrap_or(values.ammo.min(u8::MAX as u16) as u8),
			storage: su.storage.unwrap_or(0),
			connectors: su.connectors,
			values: values.clone(),
			frame: frames[ty].expect("validated above"),
			total_images: 0,
			mining: su.mining,
		};
		let (body, layout) = encode_unit_prefix(&fresh);
		let state = super::orders::deploy_state_for(fresh.orders);
		let record = UnitRecord {
			unit_type: fresh.unit_type,
			id: fresh.id,
			flags,
			pixel_x: (fresh.grid_x as u16) * 64 + 32,
			pixel_y: (fresh.grid_y as u16) * 64 + 32,
			grid_x: fresh.grid_x,
			grid_y: fresh.grid_y,
			name: fresh.name.clone(),
			team: fresh.team,
			angle: fresh.angle,
			turret_angle: fresh.turret_angle,
			orders: fresh.orders,
			state,
			prior_orders: fresh.orders,
			prior_state: state,
			disabled_turns: fresh.disabled_turns,
			hits: fresh.hits,
			ammo: fresh.ammo,
			storage: fresh.storage,
			build_rate: 1,
			connectors: fresh.connectors,
			turret_image_base: fresh.frame.turret_image_base,
			connector_image_base: fresh.frame.connector_image_base,
			path: None,
			base_values: Some(values_ref),
			complex: complex_of_unit[i],
			parent_unit: None,
			enemy_unit: None,
			build_list: Vec::new(),
		};
		let idx = objects.len();
		metas.push(ObjMeta { type_index: 5, contained: 1, body_raw: body, unit_layout: Some(layout) });
		objects.push(SaveObject::Unit(record));
		use super::unit_types::UnitCategory as C;
		let list = match C::from_flags(flags) {
			C::GroundCover => 0,
			C::MobileLandSea => 1,
			C::Stationary => 2,
			C::MobileAir => 3,
			C::Particle => 4,
		};
		lists[list].push(idx);
	}
	let [ground_cover, mobile_land_sea, stationary, mobile_air, particles] = lists;

	// --- Spatial hashes (derived, A4) ---------------------------------------
	let all_lists: [&[usize]; 5] = [&ground_cover, &mobile_land_sea, &stationary, &mobile_air, &particles];
	let unit_hash = derive_unit_hash(&objects, &all_lists);
	let map_hash = derive_map_hash(&objects, &all_lists);

	// --- Per-team CTInfo (CTInfo::Reset quirks, §4) --------------------------
	let teams = (0..TEAM_COUNT)
		.map(|slot| CtInfo {
			team_type: p.team_types[slot],
			finished_turn: false,
			team_clan: if p.team_types[slot] == TEAM_TYPE_PLAYER { p.team_clans[slot] } else { 0 },
			research_topics: [[0; 3]; 8],
			team_points: 0,
			number_of_objects_created: if slot < 4 {
				p.units.iter().filter(|u| u.team as usize == slot).count() as u16
			} else {
				0
			},
			unit_counters: if slot < 4 { unit_counters[slot] } else { [1; UNIT_END] },
			screen_locations: [[-1, -1]; 4],
			score_graph: [0; 50],
			selected_unit_id: 0xFFFF,
			zoom_level: 0,
			camera_x: 0,
			camera_y: 0,
			display_buttons: [0; 11],
			stats: [0; 4],
			casualties: [0; UNIT_END],
			stats_gold_spent_on_upgrades: 0,
		})
		.collect();

	// --- Header + scalars (§4 defaults) --------------------------------------
	let header =
		SaveHeader {
			format: SaveFormat::V71,
			category: SaveCategory::Custom,
			save_name: p.save_name.clone(),
			world_index: None,
			world_file: None,
			world_hash: p.world_hash.clone(),
			mission_index: 0,
			script: custom_mission_script(),
			team_names: p.team_names.clone(),
			team_type: p.team_types.map(u32::from),
			team_clan: std::array::from_fn(|s| {
				if p.team_types[s] == TEAM_TYPE_PLAYER { p.team_clans[s] as u32 } else { 0 }
			}),
			rng_seed: p.rng_seed,
			options: super::types::SaveOptions {
				world: p.world,
				timer: 180,
				endturn: 45,
				start_gold: p.start_gold,
				play_mode: 1,
				victory_type: 0,
				victory_limit: 50,
				opponent: 1,
				raw_resource: 1,
				fuel_resource: 1,
				gold_resource: 1,
				alien_derelicts: 0,
			},
		};
	let tail = encode_tail_fresh(p.width, p.height, p.team_types);
	Ok(SaveFile {
		header,
		extra_settings: SaveExtraSettings {
			effects: 1,
			click_scroll: 1,
			quick_scroll: 16,
			fast_movement: 1,
			follow_unit: 0,
			auto_select: 0,
			enemy_halt: 1,
		},
		width: p.width,
		height: p.height,
		surface_map: p.surface_map.clone(),
		cargo_map: p.cargo_map.clone(),
		active_turn_team: 0,
		player_team: 0,
		turn_counter: 1,
		game_state: 8, // GAME_STATE_8_IN_GAME
		turn_timer: 0,
		is_cheater: 0,
		cheater_team: 0,
		teams,
		team_units,
		raw: super::types::RawRegions { tail, ..Default::default() },
		objects,
		ground_cover,
		mobile_land_sea,
		stationary,
		mobile_air,
		particles,
		unit_hash,
		map_hash,
		object_meta: metas,
	})
}

/// [`synthesize_save`] straight to encoded `V71` bytes — what a caller attaches
/// or writes to disk.
pub fn synthesize_save_bytes(
	p: &SynthesisParams,
	db: &crate::attribs::UnitStatsDb,
	frames: &[Option<crate::attribs::FrameInfo>; super::types::UNIT_END],
) -> Result<Vec<u8>, EditError> {
	Ok(encode_save(&synthesize_save(p, db, frames)?))
}

#[cfg(test)]
pub(crate) mod tests {
	use std::collections::BTreeSet;

	use super::{
		BUILDING, FreshUnit, HASH_SIZE, SynthUnit, SynthesisParams, derive_map_hash, derive_unit_hash, encode_ct_info,
		encode_extra_settings, encode_header, encode_save, encode_scalars, encode_tail_fresh, encode_unit_prefix,
		synthesize_save, synthesize_save_bytes,
	};
	use crate::save::{SaveFile, SaveFormat, read_save_bytes};

	/// The `V71` round-trip fixture: a real M.A.X. Port save on a custom 50×50
	/// GREEN_3 map (`testdata/saves/`, git-ignored — local game assets only). Its
	/// dimensions are fixed by the map it was authored on and are **not** stored in
	/// the save, so they are supplied here (matching the paired
	/// `GREEN_3-50x50.WRL`; see `testdata/saves/README.md`).
	const FIXTURE_DIMS: (u16, u16) = (50, 50);

	/// Loads the git-ignored `V71` fixture: its raw on-disk bytes and the decoded
	/// [`SaveFile`]. Returns `None` (with a skip log) when the local game assets are
	/// absent, so CI without them stays green — every Stage A test threads through
	/// this the same way (mirroring the existing `*_when_present` skip pattern).
	pub(crate) fn load_fixture() -> Option<(Vec<u8>, SaveFile)> {
		let path =
			std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/saves/save11-green3-50x50.dta");
		if !path.is_file() {
			crate::testutil::skip_fixture(&format!("save-from-scratch fixture not found at {}", path.display()));
			return None;
		}
		let raw = std::fs::read(&path).expect("read fixture bytes");
		let save = read_save_bytes(&raw, FIXTURE_DIMS).expect("decode fixture");
		Some((raw, save))
	}

	/// A0 placeholder: the harness skips cleanly with no fixture, and — with the
	/// fixture present — `decode(raw)` succeeds and lands on the expected `V71`
	/// 50×50 save. The typed-encoder tickets (A1+) build their byte-for-byte
	/// comparisons on top of this same `load_fixture` handle.
	#[test]
	fn fixture_decodes_when_present() {
		let Some((raw, save)) = load_fixture() else {
			return;
		};
		// `load_fixture` already decoded once; prove `decode(raw)` is a repeatable
		// success on the raw bytes the encoders will be measured against.
		assert!(read_save_bytes(&raw, FIXTURE_DIMS).is_ok(), "fixture bytes re-decode cleanly");
		assert_eq!(save.header.format, SaveFormat::V71);
		assert_eq!((save.width, save.height), FIXTURE_DIMS);
	}

	/// The synthesized Custom mission script is byte-identical to the one the
	/// game itself wrote into the real fixture save (region 3) — the engine
	/// JSON-parses it on load and rejects the save when that fails, so this is
	/// a loadability gate, not cosmetics.
	#[test]
	fn custom_mission_script_matches_the_games_own_when_present() {
		let script = super::custom_mission_script();
		assert_eq!(script.len(), 249, "the Custom mission JSON is 249 bytes");
		let Some((_, save)) = load_fixture() else {
			return;
		};
		assert_eq!(save.header.script, script, "identical to the script max-port wrote into the fixture");
	}

	/// A1: the header + options (regions 1–12) re-emit byte-for-byte from the typed
	/// [`crate::save::SaveHeader`] — including the difficulty[5] and pre-options
	/// regions the decoder drops, which the encoder derives from the options block.
	#[test]
	fn encode_header_matches_raw_when_present() {
		let Some((_, save)) = load_fixture() else {
			return;
		};
		assert_eq!(encode_header(&save.header), save.raw.header, "regions 1–12 must re-emit byte-exactly");
	}

	/// A1: the seven extra settings (region 13) re-emit byte-for-byte from the typed
	/// [`crate::save::SaveExtraSettings`].
	#[test]
	fn encode_extra_settings_matches_raw_when_present() {
		let Some((_, save)) = load_fixture() else {
			return;
		};
		assert_eq!(
			encode_extra_settings(&save.extra_settings),
			save.raw.extra_settings,
			"region 13 must re-emit byte-exactly"
		);
	}

	/// A2: each of the five `V71` per-team CTInfo blocks (988 B) re-emits
	/// byte-for-byte from the fully-typed [`crate::save::CtInfo`].
	#[test]
	fn encode_ct_info_matches_raw_when_present() {
		let Some((_, save)) = load_fixture() else {
			return;
		};
		assert_eq!(save.teams.len(), save.raw.ct_info.len(), "one typed team per retained block");
		for (i, (ct, raw)) in save.teams.iter().zip(&save.raw.ct_info).enumerate() {
			let block = encode_ct_info(ct, SaveFormat::V71);
			assert_eq!(block.len(), 988, "team {i} CTInfo is 988 bytes");
			assert_eq!(&block, raw, "team {i} CTInfo must re-emit byte-exactly");
		}
	}

	/// A3: the game scalars + cheater pair (regions 17–18, 28 B) re-emit
	/// byte-for-byte from the typed [`crate::save::SaveFile`].
	#[test]
	fn encode_scalars_matches_raw_when_present() {
		let Some((_, save)) = load_fixture() else {
			return;
		};
		let block = encode_scalars(&save);
		assert_eq!(block.len(), 28, "V71 scalars block is 28 bytes");
		assert_eq!(block, save.raw.scalars, "regions 17–18 must re-emit byte-exactly");
	}

	/// A4 (verbatim): the two spatial-hash regions (`Hash_UnitHash` +
	/// `Hash_MapHash`) re-emit byte-for-byte from the structural model, isolated via
	/// the serializer's reported hash span. Also confirms the tail sits exactly after
	/// them, so the span is the true hash/tail boundary.
	#[test]
	fn hash_regions_reemit_byte_exact_when_present() {
		let Some((original, save)) = load_fixture() else {
			return;
		};
		let (out, span) = crate::save::serialize::serialize_with_hash_span(&save).unwrap();
		assert_eq!(out.len(), original.len(), "re-serialized length matches the original");
		assert!(span.start < span.end && span.end <= original.len(), "hash span lies within the file");
		assert!(span.len() > 2 * HASH_SIZE, "hash regions span more than the two bucket-count tables");
		assert_eq!(out[span.clone()], original[span.clone()], "unit + map hash regions must re-emit byte-exact");
		assert_eq!(out[span.end..], original[span.end..], "the tail sits exactly after the hash regions");
	}

	/// A4 (derive): hashes built from the units alone are structurally valid, and —
	/// because unit-hash bucket membership is fully determined by `id % 512` — the
	/// derived unit-hash buckets match the retained ones as sets (only intra-bucket
	/// order, which is free for loadability, may differ). The map hash is checked for
	/// validity only: a captured save can hash a mid-move unit off its grid cell, so
	/// its retained cells need not set-match a rest-state derivation.
	#[test]
	fn derived_hashes_are_structurally_valid_when_present() {
		let Some((_, save)) = load_fixture() else {
			return;
		};
		let all_lists: Vec<&[usize]> = save.lists().iter().map(|(_, l)| *l).collect();
		let uh = derive_unit_hash(&save.objects, &all_lists);

		assert_eq!(uh.len(), HASH_SIZE, "512 unit-hash buckets");
		let listed: BTreeSet<usize> = all_lists.iter().flat_map(|l| l.iter().copied()).collect();
		let hashed: BTreeSet<usize> = uh.iter().flatten().copied().collect();
		assert_eq!(hashed, listed, "derived unit hash references exactly the listed units");
		for (bucket, entries) in uh.iter().enumerate() {
			for &idx in entries {
				assert_eq!(save.unit(idx).unwrap().id as usize % HASH_SIZE, bucket, "unit sits in bucket id % 512");
			}
		}
		for (bucket, (derived, retained)) in uh.iter().zip(&save.unit_hash).enumerate() {
			let d: BTreeSet<usize> = derived.iter().copied().collect();
			let r: BTreeSet<usize> = retained.iter().copied().collect();
			assert_eq!(d, r, "derived unit-hash bucket {bucket} matches the retained bucket as a set");
		}

		let spatial: [&[usize]; 4] = [&save.ground_cover, &save.mobile_land_sea, &save.stationary, &save.mobile_air];
		let mh = derive_map_hash(&save.objects, &spatial);
		assert_eq!(mh.hash_size as usize, HASH_SIZE, "512 map-hash buckets");
		for (bucket, cells) in mh.buckets.iter().enumerate() {
			for cell in cells {
				let key = (cell.y ^ (cell.x << mh.x_shift)) as usize % mh.hash_size as usize;
				assert_eq!(key, bucket, "derived map cell ({},{}) hashes to its bucket", cell.x, cell.y);
				for &idx in &cell.units {
					let u = save.unit(idx).expect("derived map cell references a real unit");
					let (dx, dy) = (cell.x as i32 - u.grid_x as i32, cell.y as i32 - u.grid_y as i32);
					let span = if u.flags & BUILDING != 0 { 1 } else { 0 };
					assert!(
						(0..=span).contains(&dx) && (0..=span).contains(&dy),
						"derived cell ({},{}) lies on unit id {}'s footprint",
						cell.x,
						cell.y,
						u.id
					);
				}
			}
		}
	}

	/// A5: a fresh human-only tail is all zero, its length matches the formula
	/// (active heat maps + four `u32` log counts), and it parses the way the V71
	/// reader consumes it — heat maps for the active slots, four empty logs, no AI.
	#[test]
	fn encode_tail_fresh_is_zeroed_and_correctly_framed() {
		const NONE: u8 = 0;
		const PLAYER: u8 = 1;
		let (w, h) = (50u16, 50u16);
		let cell_bytes = w as usize * h as usize * 12;

		// Human-vs-human MVP: RED + GREEN active, the rest NONE.
		let team_types = [PLAYER, PLAYER, NONE, NONE, NONE];
		let tail = encode_tail_fresh(w, h, team_types);

		let active = team_types.iter().take(4).filter(|&&t| t != NONE).count();
		assert_eq!(tail.len(), active * cell_bytes + 16, "length = active heat maps + four u32 log counts");
		assert!(tail.iter().all(|&b| b == 0), "a fresh human-only tail is all zero");

		// Structural parse mirroring the V71 reader's tail consumption.
		let mut pos = 0usize;
		for &tt in team_types.iter().take(4) {
			if tt != NONE {
				pos += cell_bytes;
			}
		}
		for _ in 0..4 {
			let count = u32::from_le_bytes(tail[pos..pos + 4].try_into().unwrap());
			assert_eq!(count, 0, "empty message log");
			pos += 4;
		}
		assert_eq!(pos, tail.len(), "no AI state trails a human-only game");
	}

	/// A5: even with the alien slot marked active, only slots 0–3 get heat maps —
	/// the V71 reader never reads slot 4 (§6.3 desync trap).
	#[test]
	fn encode_tail_fresh_never_emits_the_alien_heat_map() {
		const PLAYER: u8 = 1;
		let (w, h) = (8u16, 8u16);
		let tail = encode_tail_fresh(w, h, [PLAYER; 5]);
		assert_eq!(tail.len(), 4 * (8 * 8 * 12) + 16, "four heat maps only; the alien slot is excluded");
	}

	/// A6 (capstone): `encode_save` assembles the whole V71 file from the typed
	/// model — regions 1–18 via the typed encoders, 19–22 via the structural graph
	/// serializer, and the verbatim tail. For the real fixture this reproduces the
	/// original byte-for-byte, and the emitted bytes are a decode∘encode fixed point
	/// (the typed round-trip identity the ticket calls for, at byte granularity).
	#[test]
	fn encode_save_reproduces_the_fixture_and_round_trips() {
		let Some((original, save)) = load_fixture() else {
			return;
		};
		let bytes = encode_save(&save);
		assert_eq!(bytes.len(), original.len(), "encoded length matches the original");
		assert!(bytes == original, "encode_save regenerates the fixture byte-for-byte (regions 1–22 + tail)");

		// Typed round-trip identity: decoding the typed-emitted bytes and re-emitting
		// yields the identical stream — the structural spine is self-reproducing.
		let reparsed = read_save_bytes(&bytes, FIXTURE_DIMS).expect("re-decode the encoded save");
		assert!(encode_save(&reparsed) == bytes, "decode∘encode is a fixed point");
	}

	/// A fresh TANK-like unit for the C1 prefix tests.
	fn fresh_tank() -> FreshUnit {
		FreshUnit {
			unit_type: crate::save::unit_type_id("TANK").unwrap(),
			id: (1 << 13) | 1,
			flags: 0x100 | 0x0200_0000 | 0x1000, // MOBILE_LAND | TURRET_SPRITE | HASH_TEAM_GREEN
			grid_x: 6,
			grid_y: 5,
			name: String::new(),
			team: 1,
			clan: 3,
			unit_serial: 1,
			angle: 2,
			turret_angle: 2,
			orders: 0,
			disabled_turns: 0,
			hits: 24,
			ammo: 14,
			storage: 0,
			connectors: 0,
			values: crate::attribs::unit_values_from_attributes(&crate::attribs::UnitAttributes {
				turns_to_build: 4,
				hit_points: 24,
				armor_rating: 10,
				attack_rating: 16,
				movement_points: 6,
				attack_range: 4,
				shots_per_turn: 2,
				scan_range: 4,
				ammunition: 14,
				..Default::default()
			}),
			frame: crate::attribs::FrameInfo {
				image_base: 0,
				image_count: 8,
				turret_image_base: 8,
				turret_image_count: 8,
				angle_offsets: [(1, -1); 8],
				..Default::default()
			},
			total_images: 16,
			mining: [0; 7],
		}
	}

	/// C1: the fresh-unit prefix has the exact V71 body length, its layout
	/// offsets address the right bytes, and the deploy-constructor values land
	/// where `UnitInfo::FileLoad` reads them.
	#[test]
	fn encode_unit_prefix_writes_a_v71_idle_body() {
		let u = fresh_tank();
		let (body, layout) = encode_unit_prefix(&u);
		// V71 fixed prefix = 149 bytes + the name bytes (empty here).
		assert_eq!(body.len(), 149, "V71 prefix length (empty name)");
		assert_eq!(layout.refs_off, body.len(), "refs start right after the prefix");
		let u16_at = |off: usize| u16::from_le_bytes([body[off], body[off + 1]]);
		assert_eq!(u16_at(0), u.unit_type);
		assert_eq!(u16_at(2), u.id);
		assert_eq!(u32::from_le_bytes([body[4], body[5], body[6], body[7]]), u.flags);
		assert_eq!(u16_at(layout.pixel_x), 6 * 64 + 32, "pixel = cell centre");
		assert_eq!(u16_at(layout.grid_x), 6);
		assert_eq!(u16_at(layout.name), 0, "empty name length");
		assert_eq!(body[layout.team], 1);
		assert_eq!(body[layout.team + 2], 0xFF, "brightness at deploy");
		assert_eq!(body[layout.angle], 2);
		let vis = layout.angle + 1;
		assert_eq!(&body[vis..vis + 5], &[0, 1, 0, 0, 0], "visible to the owning team only");
		assert_eq!(body[layout.turret_angle], 2);
		assert_eq!(
			(body[layout.turret_angle + 1] as i8, body[layout.turret_angle + 2] as i8),
			(1, -1),
			"turret offset for the angle",
		);
		// Image block: total, base, turret_base, firing, connector, index,
		// turret_index, index_max.
		let img = |k: usize| u16_at(layout.image_block + 2 * k) as i16;
		assert_eq!(
			[img(0), img(1), img(2), img(3), img(4), img(5), img(6), img(7)],
			[16, 0, 8, 0, 0, 2, 10, 9],
			"deploy image state: index = base+angle, max = count+index−1",
		);
		assert_eq!(&body[layout.orders..layout.orders + 5], &[0, 1, 0, 1, 0], "await/executing, idle laying");
		assert_eq!(&body[layout.move_to..layout.move_to + 8], &[0; 8], "no move/fire targets");
		assert_eq!(body[layout.build_time], 0);
		assert_eq!(u16_at(layout.hits), 24);
		assert_eq!(u16_at(layout.hits + 2), 6, "speed = full movement points");
		assert_eq!(body[layout.hits + 4], 2, "shots = rounds");
		assert_eq!(body[layout.ammo], 14);
		assert_eq!(body[layout.disabled], 0);
		assert_eq!(body[layout.moved], 0);
		// engine/weapon sit at moved+3 / moved+4; armed unit → weapon 2.
		assert_eq!((body[layout.moved + 3], body[layout.moved + 4]), (2, 2));
		// Tail of the prefix: repeat_build 0, build_rate 1 (u16), then
		// disabled_reaction_fire / auto_survey / ai_state_bits all zero.
		let tail = &body[layout.refs_off - 9..];
		assert_eq!(tail, &[0, 1, 0, 0, 0, 0, 0, 0, 0], "repeat_build, build_rate 1, zeros");
	}

	/// C1 quirks: MININGST takes its clan image base; COMMANDO caps
	/// `image_index_max` at 103; an unarmed unit gets weapon 0; hovering air
	/// units shift their shadow a tile.
	#[test]
	fn encode_unit_prefix_applies_type_quirks() {
		let mut m = fresh_tank();
		m.unit_type = crate::save::unit_type_id("MININGST").unwrap();
		m.clan = 3;
		m.angle = 0;
		m.values.attack = 0;
		let (body, layout) = encode_unit_prefix(&m);
		let img = |k: usize| {
			u16::from_le_bytes([body[layout.image_block + 2 * k], body[layout.image_block + 2 * k + 1]]) as i16
		};
		assert_eq!(img(1), 4, "MININGST image_base = (clan−1)-2");
		assert_eq!(img(5), 4, "image_index follows the clan base");
		assert_eq!(body[layout.moved + 4], 0, "no attack -> weapon 0");

		let mut c = fresh_tank();
		c.unit_type = crate::save::unit_type_id("COMMANDO").unwrap();
		let (body, layout) = encode_unit_prefix(&c);
		let img_max = u16::from_le_bytes([body[layout.image_block + 14], body[layout.image_block + 15]]) as i16;
		assert_eq!(img_max, 103, "COMMANDO walk-strip cap");

		let mut a = fresh_tank();
		a.unit_type = crate::save::unit_type_id("AWAC").unwrap();
		a.flags = 0x40 | 0x10000 | 0x1000; // MOBILE_AIR | HOVERING | team
		let (body, layout) = encode_unit_prefix(&a);
		let shadow = layout.name + 2; // empty name → shadow_offset follows the length
		let sx = i16::from_le_bytes([body[shadow], body[shadow + 1]]);
		assert_eq!(sx, -64, "hovering air shadow offset");
		let _ = layout;
	}

	/// A synthetic unit database + frame table covering the types the C2 tests
	/// place — no game files needed.
	fn synth_db() -> (crate::attribs::UnitStatsDb, [Option<crate::attribs::FrameInfo>; crate::save::UNIT_END]) {
		use crate::attribs::{
			CargoType, FrameInfo, UnitAttributes, UnitMeta, UnitStatsDb, unit_values_from_attributes,
		};
		let mut base = std::array::from_fn(|_| {
			unit_values_from_attributes(&UnitAttributes { hit_points: 10, scan_range: 2, ..Default::default() })
		});
		let mut meta = [UnitMeta::default(); crate::save::UNIT_END];
		let set = |base: &mut [crate::save::UnitValues; crate::save::UNIT_END],
		           meta: &mut [UnitMeta; crate::save::UNIT_END],
		           name: &str,
		           a: UnitAttributes,
		           flags: u32,
		           cargo: CargoType| {
			let id = crate::save::unit_type_id(name).unwrap() as usize;
			base[id] = unit_values_from_attributes(&a);
			meta[id] = UnitMeta { flags, cargo_type: cargo, ..Default::default() };
		};
		// Flags per SC_UNITS: MININGST = BUILDING|STATIONARY…, POWGEN =
		// STATIONARY|STANDALONE…, LRGSLAB = GROUND_COVER|BUILDING|STATIONARY,
		// TANK = MOBILE_LAND|TURRET_SPRITE….
		set(
			&mut base,
			&mut meta,
			"MININGST",
			UnitAttributes {
				turns_to_build: 12,
				hit_points: 56,
				armor_rating: 8,
				scan_range: 3,
				storage_capacity: 25,
				..Default::default()
			},
			0x10 | 0x200,
			CargoType::Raw,
		);
		set(
			&mut base,
			&mut meta,
			"POWGEN",
			UnitAttributes { turns_to_build: 4, hit_points: 14, armor_rating: 8, scan_range: 3, ..Default::default() },
			0x200 | 0x0080_0000,
			CargoType::None,
		);
		set(
			&mut base,
			&mut meta,
			"LRGSLAB",
			UnitAttributes { hit_points: 1, ..Default::default() },
			0x1 | 0x10 | 0x200,
			CargoType::None,
		);
		set(
			&mut base,
			&mut meta,
			"TANK",
			UnitAttributes {
				turns_to_build: 4,
				hit_points: 24,
				armor_rating: 10,
				attack_rating: 16,
				movement_points: 6,
				attack_range: 4,
				shots_per_turn: 2,
				scan_range: 4,
				ammunition: 14,
				..Default::default()
			},
			0x100 | 0x0200_0000,
			CargoType::None,
		);
		let db = UnitStatsDb { base, clans: Default::default(), meta, source: std::path::PathBuf::from("synthetic") };
		let frames = std::array::from_fn(|_| {
			Some(FrameInfo { image_base: 0, image_count: 8, turret_image_base: 8, ..Default::default() })
		});
		(db, frames)
	}

	fn synth_unit(name: &str, x: i16, y: i16, team: u8) -> SynthUnit {
		SynthUnit {
			unit_type: crate::save::unit_type_id(name).unwrap(),
			grid_x: x,
			grid_y: y,
			team,
			angle: 0,
			turret_angle: 0,
			name: String::new(),
			orders: 0,
			disabled_turns: 0,
			hits: None,
			ammo: None,
			storage: None,
			connectors: 0,
			base_values: None,
			mining: [0; 7],
		}
	}

	fn synth_params(units: Vec<SynthUnit>) -> SynthesisParams {
		SynthesisParams {
			save_name: "Synthesized".into(),
			world_hash: Some("feedface".into()),
			world: 0,
			width: 16,
			height: 12,
			rng_seed: 0x1234,
			team_types: [1, 1, 0, 0, 0],
			team_clans: [1, 2, 0, 0, 0],
			team_names: ["Player 1".into(), "Player 2".into(), String::new(), String::new(), String::new()],
			start_gold: 150,
			surface_map: vec![1; 16 * 12],
			cargo_map: vec![0; 16 * 12],
			units,
		}
	}

	/// C2: input validation — a player team with no units, a unit on a dead
	/// team, and a missing frame table are all refused with clear messages.
	#[test]
	fn synthesize_save_validates_inputs() {
		let (db, frames) = synth_db();
		let err =
			synthesize_save(&synth_params(vec![synth_unit("TANK", 3, 3, 0)]), &db, &frames).unwrap_err().to_string();
		assert!(err.contains("team 2 owns no units"), "{err}");

		let err =
			synthesize_save(&synth_params(vec![synth_unit("TANK", 3, 3, 2)]), &db, &frames).unwrap_err().to_string();
		assert!(err.contains("not a player"), "{err}");

		let mut no_frames = frames;
		no_frames[crate::save::unit_type_id("TANK").unwrap() as usize] = None;
		let units = vec![synth_unit("TANK", 3, 3, 0), synth_unit("TANK", 5, 5, 1)];
		let err = synthesize_save(&synth_params(units), &db, &no_frames).unwrap_err().to_string();
		assert!(err.contains("no frame info for TANK"), "{err}");

		let err = synthesize_save(
			&synth_params(vec![synth_unit("TANK", 99, 3, 0), synth_unit("TANK", 5, 5, 1)]),
			&db,
			&frames,
		)
		.unwrap_err()
		.to_string();
		assert!(err.contains("off the map"), "{err}");
	}

	/// C2 capstone: a synthesized save encodes, decodes back through the
	/// standard reader with a consistent graph, passes the transient-state
	/// integrity scan, and is an encode∘decode fixed point.
	#[test]
	fn synthesized_save_round_trips_through_the_decoder() {
		let (db, frames) = synth_db();
		// RED: a mining base — MININGST (2×2 at 2,2) + POWGEN at (4,2), edge-
		// adjacent and both connector-flagged → one shared complex; plus a slab
		// (ground cover, no complex). GREEN: a lone TANK.
		let mut mining = synth_unit("MININGST", 2, 2, 0);
		mining.connectors = 0xFF;
		mining.mining = [8, 4, 2, 2, 8, 4, 6];
		let mut powgen = synth_unit("POWGEN", 4, 2, 0);
		powgen.connectors = 0x40; // west edge, toward the station
		let slab = synth_unit("LRGSLAB", 2, 2, 0);
		let mut tank = synth_unit("TANK", 9, 7, 1);
		tank.name = "Spearhead".into();
		tank.hits = Some(20);
		let bytes = synthesize_save_bytes(&synth_params(vec![mining, powgen, slab, tank]), &db, &frames).unwrap();

		let save = read_save_bytes(&bytes, (16, 12)).expect("synthesized bytes decode");
		assert_eq!(save.header.format, SaveFormat::V71);
		assert_eq!(save.header.save_name, "Synthesized");
		assert_eq!(save.header.team_type, [1, 1, 0, 0, 0]);
		assert_eq!(save.width, 16);
		assert_eq!((save.turn_counter, save.game_state), (1, 8));
		assert_eq!(save.teams.len(), 5, "V71 carries five CTInfo blocks");
		assert_eq!(save.teams[0].screen_locations, [[-1, -1]; 4], "CTInfo::Reset quirk");
		assert_eq!(save.teams[0].selected_unit_id, 0xFFFF);
		assert_eq!(save.team_units.len(), 4);
		assert!(save.team_units[0].gold >= 150, "start gold lands in the team table");

		// Lists: slab → ground cover; station + generator → stationary; tank →
		// mobile land.
		assert_eq!(
			(save.ground_cover.len(), save.stationary.len(), save.mobile_land_sea.len(), save.mobile_air.len()),
			(1, 2, 1, 0),
		);
		// The tank carries its name, damage, and the GREEN hash-team bit.
		let tank_rec = save.unit(save.mobile_land_sea[0]).unwrap();
		assert_eq!(tank_rec.name, "Spearhead");
		assert_eq!(tank_rec.hits, 20);
		assert_ne!(tank_rec.flags & 0x1000, 0, "HASH_TEAM_GREEN");
		let tank_values = save.values(tank_rec.base_values.unwrap()).unwrap();
		assert_eq!((tank_values.hits, tank_values.attack), (24, 16), "shared stock stats");
		// …and its base_values IS the team table's shared entry (fresh-game rule).
		let tank_ty = crate::save::unit_type_id("TANK").unwrap() as usize;
		assert_eq!(save.team_units[1].current_values[tank_ty], tank_rec.base_values);
		assert_eq!(save.team_units[1].base_values[tank_ty], tank_rec.base_values, "base == current object");

		// Complexes: the mining station and generator share one; the RED team
		// has exactly that one complex; mining scalars persisted.
		let station = save.unit(save.stationary[0]).unwrap();
		let generator = save.unit(save.stationary[1]).unwrap();
		assert_eq!(station.complex, generator.complex, "adjacent connected buildings share a complex");
		assert!(station.complex.is_some());
		assert_eq!(save.team_units[0].complexes.len(), 1);
		// Hashes: every unit is present in both derived hashes.
		let hashed: usize = save.unit_hash.iter().map(|b| b.len()).sum();
		assert_eq!(hashed, 4);
		// A 2×2 BUILDING occupies 4 map-hash cells; 1×1s one each.
		let cells: usize = save.map_hash.buckets.iter().flatten().map(|c| c.units.len()).sum();
		assert_eq!(cells, 4 + 1 + 4 + 1, "station 4 + generator 1 + slab(BUILDING 2x2) 4 + tank 1");

		// The synthesized units are clean idle bodies.
		assert!(crate::save::check_transient_state(&save).is_empty(), "no transient-state issues");

		// Fixed point: re-encoding the decoded save reproduces the bytes.
		assert!(encode_save(&save) == bytes, "decode∘encode fixed point for a synthesized save");

		// The fresh tail: two active teams of zero heat maps + 4 empty logs.
		assert_eq!(save.raw.tail.len(), 2 * 16 * 12 * 12 + 16);
		assert!(save.raw.tail.iter().all(|&b| b == 0));
	}

	/// A placement whose type has **no template** in the save synthesizes a
	/// fresh deploy-state body when the runtime unit data is supplied — the fix
	/// for the "building disappears, only its slab exports" report. Without the
	/// context the placement is still refused (and reported by the caller).
	#[test]
	fn add_unit_synthesizes_a_fresh_body_without_a_template() {
		let (db, frames) = synth_db();
		let ty = crate::save::unit_type_id("POWGEN").unwrap();
		let bytes = synthesize_save_bytes(
			&synth_params(vec![synth_unit("MININGST", 3, 3, 0), synth_unit("TANK", 9, 7, 1)]),
			&db,
			&frames,
		)
		.expect("synthesis succeeds");
		let mut save = read_save_bytes(&bytes, (16, 12)).expect("decodes");
		assert!(save.units().all(|u| u.unit_type != ty), "no POWGEN template in the save");
		let pristine = save.clone();

		assert!(
			crate::save::add_unit(&mut save, ty, 0, 5, 5, None).unwrap().is_none(),
			"without the runtime data the placement is refused, not guessed"
		);
		let ctx = crate::save::FreshBodyCtx { db: &db, frames: &frames };
		let id = crate::save::add_unit(&mut save, ty, 0, 5, 5, Some(&ctx)).unwrap().expect("fresh body synthesized");
		// The export flow runs the complex pass right after the add pass; a
		// standalone host gets its engine-valid Complex there.
		crate::save::repair_complexes(&mut save, &crate::save::dead_listed_complexes(&pristine)).unwrap();

		let out = read_save_bytes(&crate::save::write_save(&save).unwrap(), (16, 12)).expect("re-decodes");
		let u = out.units().find(|u| u.id == id).expect("placed unit present");
		assert_eq!((u.grid_x, u.grid_y), (5, 5));
		assert_eq!(u.team, 0);
		assert_eq!(u.orders, crate::save::ORDER_POWER_OFF, "POWGEN deploys powered off");
		assert_eq!(u.state, crate::save::ORDER_STATE_EXECUTING_ORDER);
		assert!(u.complex.is_some(), "the complex pass attached a host complex");
		assert!(u.base_values.is_some(), "stat seed shared from the team table");
		let in_stationary =
			out.lists()[2].1.iter().any(|&s| matches!(&out.objects[s], crate::save::SaveObject::Unit(u) if u.id == id));
		assert!(in_stationary, "flags route the building into StationaryUnits");
		assert!(crate::save::check_transient_state(&out).is_empty(), "idle-valid");
		assert!(crate::save::check_complexes(&out).is_empty(), "complex-valid");
	}

	/// The byte-exactness invariant, pinned to bytes on disk and provable
	/// **without the game**.
	///
	/// Every other proof of this property reads the user's `~/MAX` saves or
	/// `testdata/`, so on a fresh clone (or CI) the crate's hardest guarantee
	/// went unchecked - and the synthesis fixed-point test next door compares the
	/// encoder against itself, so a change to both halves could move the on-disk
	/// format without any test noticing. This fixture is a synthesized V71 save
	/// (no copyrighted bytes, fully deterministic - fixed seed, fixed names),
	/// committed so it can only change deliberately.
	///
	/// Regenerate with `UPDATE_SNAPSHOTS=1 cargo test -p max-assets
	/// synthesized_v71_fixture` **after** confirming the format really was meant
	/// to change; `git diff --stat` on the fixture is then the review.
	#[test]
	fn synthesized_v71_fixture_round_trips_byte_exactly() {
		let (db, frames) = synth_db();
		let mut mining = synth_unit("MININGST", 3, 3, 0);
		mining.connectors = 0xFF;
		mining.mining = [8, 4, 2, 2, 8, 4, 6];
		let tank = synth_unit("TANK", 9, 7, 1);
		let bytes = synthesize_save_bytes(&synth_params(vec![mining, tank]), &db, &frames).expect("synthesis succeeds");

		let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/synthesized-v71.dta");
		if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
			std::fs::create_dir_all(path.parent().unwrap()).unwrap();
			std::fs::write(&path, &bytes).unwrap();
			eprintln!("wrote {} ({} bytes)", path.display(), bytes.len());
		}
		let stored = std::fs::read(&path).expect("the fixture is committed - regenerate with UPDATE_SNAPSHOTS=1");

		assert!(bytes == stored, "the encoder no longer reproduces the committed save byte for byte");
		let save = read_save_bytes(&stored, (16, 12)).expect("the fixture decodes");
		assert!(
			crate::save::write_save(&save).expect("the tail follows") == stored,
			"decode -> write_save is not byte-exact on the committed save"
		);
	}

	/// C1: a custom name lands length-prefixed in the body and every later
	/// layout offset shifts with it.
	#[test]
	fn encode_unit_prefix_carries_a_custom_name() {
		let mut u = fresh_tank();
		u.name = "Spearhead".into();
		let (body, layout) = encode_unit_prefix(&u);
		assert_eq!(body.len(), 149 + 9);
		assert_eq!(u16::from_le_bytes([body[layout.name], body[layout.name + 1]]), 9);
		assert_eq!(&body[layout.name + 2..layout.name + 11], b"Spearhead");
		assert_eq!(layout.name_len, 9);
		assert_eq!(body[layout.team], u.team, "offsets shifted past the name");
	}
}
