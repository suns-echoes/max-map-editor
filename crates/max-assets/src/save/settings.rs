//! Editable non-map save settings (S7.2) — the typed slice of a save the
//! Edit Save Data dialog works on, plus the settings-region re-encode that
//! makes such edits actually reach the emitted bytes.
//!
//! [`crate::save::write_save`] emits the header/options, extra-settings,
//! per-team `CTInfo` and game-scalar regions **verbatim** from
//! [`RawRegions`], so a typed edit alone would be silently dropped.
//! [`SaveFile::refresh_settings_regions`] closes that gap by re-encoding
//! those regions from the typed model with the Stage A encoders (byte-exact
//! on an unedited save), and [`SaveFile::settings_regions_lossless`] is the
//! guard a caller runs *before* editing: if any region does not already
//! re-encode byte-for-byte, an edit could silently mangle unmodeled bits,
//! so refuse instead.
//!
//! `team_type` **is** editable, but it is the one field whose edit reaches
//! past this module: the tail's heat-map count and AI blocks are a function of
//! it, so [`SaveSettings::apply_to`] re-shapes the retained tail through
//! [`super::tail`] and fails if it cannot.
//!
//! Deliberately **not** editable here: the world reference (`world_index` /
//! `world_hash` — bound to the map), the save category, and `game_state`.

use super::encode::{encode_ct_info, encode_extra_settings, encode_header, encode_scalars};
use super::error::EditError;
use super::serialize::serialize_unit_values;
use super::tail;
use super::types::{ObjMeta, SaveExtraSettings, SaveFile, SaveFormat, SaveObject, SaveOptions, TEAM_COUNT, UnitValues};

/// The eight research topics, in `CTInfo::research_topics` slot order
/// (max-port `enums.hpp` `RESEARCH_TOPIC_*`). Also the order of a
/// [`SaveSettings::team_upgrades`] entry: the gold-purchase menu upgrades the
/// same eight attributes (`ATTRIB_* == RESEARCH_TOPIC_*`), "Shots" being
/// `UnitValues::rounds` and "Cost" `UnitValues::turns`.
pub const RESEARCH_TOPICS: [&str; 8] = ["Attack", "Shots", "Range", "Armor", "Hits", "Speed", "Scan", "Cost"];

/// The registry class index a serialized `UnitValues` carries (`1 AirPath ...
/// 6 UnitValues` — the same constant the export path's stat override uses).
const UNIT_VALUES_TYPE: u32 = 6;

/// A `UnitValues`' eight gold-purchasable attributes, in [`RESEARCH_TOPICS`]
/// order.
fn upgrade_of(v: &UnitValues) -> [u16; 8] {
	[v.attack, v.rounds, v.range, v.armor, v.hits, v.speed, v.scan, v.turns]
}

/// Writes the eight purchasable attributes back (the inverse of
/// [`upgrade_of`]); everything else — ammo, storage, version, flags — stays.
fn set_upgrade(v: &mut UnitValues, u: [u16; 8]) {
	v.attack = u[0];
	v.rounds = u[1];
	v.range = u[2];
	v.armor = u[3];
	v.hits = u[4];
	v.speed = u[5];
	v.scan = u[6];
	v.turns = u[7];
}

/// How many places in the graph hold object `idx`: unit reference fields plus
/// every team table's base/current slots — the set a `UnitValues` can be
/// referenced from. `1` means the asking slot is the sole referent.
fn values_ref_count(save: &SaveFile, idx: usize) -> usize {
	let mut n = 0;
	for o in &save.objects {
		if let SaveObject::Unit(u) = o {
			for r in [u.path, u.base_values, u.complex, u.parent_unit, u.enemy_unit] {
				if r == Some(idx) {
					n += 1;
				}
			}
		}
	}
	for t in &save.team_units {
		n += t.base_values.iter().chain(&t.current_values).filter(|&&r| r == Some(idx)).count();
	}
	n
}

/// The `objects` position where a value first referenced at team `ti`'s
/// `current_values[ut]` slot belongs — [`SaveFile::insert_object`]'s contract
/// (`at` = the object's first-seen position). Counts the distinct objects the
/// serializer's walk encounters before that slot, mirroring
/// `serialize_object_graph`'s table order: gold, base refs, current refs,
/// complexes, team by team (the tables precede every unit list, so nothing
/// outside them can be first-seen earlier).
fn current_first_seen_pos(save: &SaveFile, ti: usize, ut: usize) -> usize {
	fn note(r: Option<usize>, seen: &mut [bool], count: &mut usize) {
		if let Some(i) = r
			&& !seen[i]
		{
			seen[i] = true;
			*count += 1;
		}
	}
	let mut seen = vec![false; save.objects.len()];
	let mut count = 0usize;
	for (t, table) in save.team_units.iter().enumerate() {
		for &r in &table.base_values {
			note(r, &mut seen, &mut count);
		}
		for (u, &r) in table.current_values.iter().enumerate() {
			if t == ti && u == ut {
				return count;
			}
			note(r, &mut seen, &mut count);
		}
		for &c in &table.complexes {
			note(Some(c), &mut seen, &mut count);
		}
	}
	count
}

/// Per-team editable stats — the `CTInfo` fields worth surfacing (score,
/// research progress, the end-screen build counters) plus nothing that
/// back-references the object graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamStats {
	/// The team's victory points (`CTInfo::team_points`).
	pub team_points: u32,
	/// Current research level per topic, in `RESEARCH_TOPICS` order
	/// (`research_topics[t].research_level`; turns-to-complete and lab
	/// allocation stay untouched).
	pub research_level: [i32; RESEARCH_TOPICS.len()],
	/// The end-screen build counters: factories, mines, buildings, units.
	pub stats: [i16; 4],
	/// Lifetime gold spent on upgrades (`stats_gold_spent_on_upgrades`).
	pub gold_spent_on_upgrades: u32,
}

/// Every non-map save setting the editor can change, extracted from and
/// applied back to a [`SaveFile`]. [`Self::apply_to`] also refreshes the
/// retained raw regions so the edit survives [`crate::save::write_save`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveSettings {
	/// The save title shown in the game's load menu.
	pub save_name: String,
	pub team_names: [String; TEAM_COUNT],
	/// `TEAM_CLAN_*`: 0 = random, 1..=8 a concrete clan. Mirrored into each
	/// team's `CTInfo::team_clan` on apply (the save stores it twice).
	pub team_clan: [u32; TEAM_COUNT],
	pub rng_seed: u32,
	/// The twelve game options. `world` is carried through unedited (it is
	/// bound to the map and not offered for editing).
	pub options: SaveOptions,
	/// The seven in-game preference toggles/values (region 13).
	pub extra: SaveExtraSettings,
	pub active_turn_team: u8,
	pub player_team: u8,
	pub turn_counter: i32,
	pub turn_timer: u16,
	/// The sticky cheater flag and the team that tripped it (region 18).
	pub is_cheater: u32,
	pub cheater_team: u32,
	/// Per-team stats, parallel to [`SaveFile::teams`] (five slots in `V71`).
	pub teams: Vec<TeamStats>,
	/// Gold reserves per team-units table, parallel to [`SaveFile::team_units`]
	/// (four tables — the alien slot has none). Emitted from the typed graph,
	/// so no region refresh is involved for these.
	pub team_gold: Vec<u32>,
	/// `TEAM_TYPE_*` per slot (stored twice on disk: the header and each
	/// team's `CTInfo`; apply keeps both in step). Any of the five values is
	/// allowed for slots 0-3 — the retained tail's shape *depends* on these
	/// (one heat map per non-NONE team, one AI block per COMPUTER team), so
	/// [`Self::apply_to`] re-shapes it through [`super::tail`] and reports
	/// `Err` if it cannot. The alien slot (4) is fixed: the game's own reader
	/// stops at slot 3, so a live alien slot is a save it could not load.
	pub team_types: [u32; TEAM_COUNT],
	/// Per-team master *current* (gold-upgraded) unit stats: one entry per
	/// team-units table (four in `V71` — no alien table), each `UNIT_END`
	/// long, `Some` where the table holds current values for that unit type —
	/// the eight purchasable attributes in [`RESEARCH_TOPICS`] order. A
	/// changed entry follows the game's own purchase commit
	/// (`AbstractUpgradeMenu::CommitUpgradeChanges`): a master shared with
	/// anything else (its base slot, another table, units built from it) is
	/// replaced by a fresh `UnitValues` (version bumped iff the old one
	/// reached a unit, installed not-in-use) so every other referent keeps its
	/// stats; a master only its own slot references is edited in place.
	pub team_upgrades: Vec<Vec<Option<[u16; 8]>>>,
}

impl SaveSettings {
	/// Snapshot the editable settings out of a decoded save.
	pub fn extract(save: &SaveFile) -> Self {
		let teams = save
			.teams
			.iter()
			.map(|ct| TeamStats {
				team_points: ct.team_points,
				research_level: std::array::from_fn(|t| ct.research_topics[t][0]),
				stats: ct.stats,
				gold_spent_on_upgrades: ct.stats_gold_spent_on_upgrades,
			})
			.collect();
		SaveSettings {
			save_name: save.header.save_name.clone(),
			team_names: save.header.team_names.clone(),
			team_clan: save.header.team_clan,
			rng_seed: save.header.rng_seed,
			options: save.header.options,
			extra: save.extra_settings,
			active_turn_team: save.active_turn_team,
			player_team: save.player_team,
			turn_counter: save.turn_counter,
			turn_timer: save.turn_timer,
			is_cheater: save.is_cheater,
			cheater_team: save.cheater_team,
			teams,
			team_gold: save.team_units.iter().map(|t| t.gold).collect(),
			team_types: save.header.team_type,
			team_upgrades: save
				.team_units
				.iter()
				.map(|t| t.current_values.iter().map(|r| r.and_then(|i| save.values(i)).map(upgrade_of)).collect())
				.collect(),
		}
	}

	/// Write these settings back into the typed model and refresh the retained
	/// raw regions so [`crate::save::write_save`] emits the edit. Everything
	/// not represented here (world reference, category, script, unit graph) is
	/// untouched. Extra `teams`/`team_gold` entries beyond what the save holds
	/// are ignored.
	///
	/// A **team-type** change re-shapes the retained tail with it
	/// ([`crate::save::tail::retype`]), which is the only step that can fail:
	/// `Err` means the tail could not be moved and *nothing* was written.
	/// [`Self::tail_follows_the_graph`] answers the same question up front, so a
	/// caller can refuse the edit before it reaches here.
	pub fn apply_to(&self, save: &mut SaveFile) -> Result<(), EditError> {
		if self.team_types != save.header.team_type {
			save.raw.tail = tail::retype(&save.raw.tail, &save.tail_shape(), &save.header.team_type, &self.team_types)?;
		}
		save.header.save_name = self.save_name.clone();
		save.header.team_names = self.team_names.clone();
		save.header.team_clan = self.team_clan;
		save.header.team_type = self.team_types;
		save.header.rng_seed = self.rng_seed;
		save.header.options = self.options;
		save.extra_settings = self.extra;
		save.active_turn_team = self.active_turn_team;
		save.player_team = self.player_team;
		save.turn_counter = self.turn_counter;
		save.turn_timer = self.turn_timer;
		save.is_cheater = self.is_cheater;
		save.cheater_team = self.cheater_team;
		for (i, ct) in save.teams.iter_mut().enumerate() {
			// The clan and type are stored twice on disk; keep both copies in step.
			ct.team_clan = *self.team_clan.get(i).unwrap_or(&0) as u8;
			ct.team_type = *self.team_types.get(i).unwrap_or(&0) as u8;
			let Some(ts) = self.teams.get(i) else { continue };
			ct.team_points = ts.team_points;
			for (t, level) in ts.research_level.iter().enumerate() {
				ct.research_topics[t][0] = *level;
			}
			ct.stats = ts.stats;
			ct.stats_gold_spent_on_upgrades = ts.gold_spent_on_upgrades;
		}
		for (table, &gold) in save.team_units.iter_mut().zip(&self.team_gold) {
			table.gold = gold;
		}
		self.apply_upgrades(save)?;
		save.refresh_settings_regions();
		Ok(())
	}

	/// Writes changed [`Self::team_upgrades`] entries into the graph's master
	/// *current* `UnitValues` (see the field's commit-semantics note). The
	/// serializer emits `UnitValues` from the typed model, so these edits reach
	/// the bytes with no region refresh; an unchanged entry touches nothing.
	fn apply_upgrades(&self, save: &mut SaveFile) -> Result<(), EditError> {
		for ti in 0..save.team_units.len() {
			let Some(team) = self.team_upgrades.get(ti) else { continue };
			for ut in 0..save.team_units[ti].current_values.len() {
				let Some(&Some(want)) = team.get(ut) else { continue };
				let Some(cur) = save.team_units[ti].current_values[ut] else { continue };
				let Some(have) = save.values(cur).map(upgrade_of) else { continue };
				if have == want {
					continue;
				}
				if values_ref_count(save, cur) > 1 {
					// Shared (base slot / other tables / built units): install a
					// fresh master, the game's purchase-commit move.
					let mut v = save.values(cur).expect("resolved above").clone();
					set_upgrade(&mut v, want);
					if v.in_use {
						v.version += 1;
					}
					v.in_use = false;
					let at = current_first_seen_pos(save, ti, ut);
					let meta = ObjMeta {
						type_index: UNIT_VALUES_TYPE,
						contained: 1,
						body_raw: serialize_unit_values(&v),
						unit_layout: None,
					};
					save.insert_object(at, SaveObject::Values(v), meta)?;
					save.team_units[ti].current_values[ut] = Some(at);
				} else if let Some(SaveObject::Values(v)) = save.objects.get_mut(cur) {
					// Sole referent (typically a master this path installed
					// earlier): edit in place, retained body included.
					set_upgrade(v, want);
					save.object_meta[cur].body_raw = serialize_unit_values(v);
				}
			}
		}
		Ok(())
	}
}

impl SaveFile {
	/// Re-encode the four settings regions ([`super::types::RawRegions`]
	/// `header` / `extra_settings` / `ct_info` / `scalars`) from the typed
	/// model, so a typed settings edit reaches [`crate::save::write_save`]'s
	/// output. A no-op byte-wise on an unedited save (the Stage A byte-exact
	/// guarantee). `V71` only — the encoders reject `V70`.
	pub fn refresh_settings_regions(&mut self) {
		let header = encode_header(&self.header);
		let extra = encode_extra_settings(&self.extra_settings);
		let ct_info: Vec<Vec<u8>> = self.teams.iter().map(|ct| encode_ct_info(ct, SaveFormat::V71)).collect();
		let scalars = encode_scalars(self);
		self.raw.header = header;
		self.raw.extra_settings = extra;
		self.raw.ct_info = ct_info;
		self.raw.scalars = scalars;
	}

	/// What this save's tail is measured against — the map it covers, the
	/// format, and the size of the object graph in front of it.
	pub fn tail_shape(&self) -> tail::TailShape {
		tail::TailShape { w: self.width, h: self.height, format: self.header.format, objects: self.objects.len() }
	}

	/// Whether this save's tail decomposes exactly, so it can be moved with the
	/// rest of the file ([`crate::save::tail`]). Two things need it:
	///
	/// - a **team-type** change that brings a slot in or out of the game, or on
	///   or off the AI (the region-25 blocks sit behind the message logs);
	/// - any **graph-structural** edit — adding or removing an object renumbers
	///   the very references the tail's message logs and AI state hold.
	///
	/// `false` narrows the offer to team-type swaps that leave the COMPUTER set
	/// alone (those never read past the tail's self-describing first region) and
	/// rules out add / remove entirely.
	pub fn tail_follows_the_graph(&self) -> bool {
		tail::decomposes(&self.raw.tail, &self.tail_shape(), &self.header.team_type)
	}

	/// Whether every settings region re-encodes byte-for-byte from the typed
	/// model — the guard to run *before* a settings edit. `false` means the
	/// decoder didn't model a region losslessly (or the save is `V70`, which
	/// is never re-encoded), so refreshing it would silently corrupt data.
	pub fn settings_regions_lossless(&self) -> bool {
		if self.header.format != SaveFormat::V71 {
			return false;
		}
		encode_header(&self.header) == self.raw.header
			&& encode_extra_settings(&self.extra_settings) == self.raw.extra_settings
			&& self.teams.len() == self.raw.ct_info.len()
			&& self.teams.iter().zip(&self.raw.ct_info).all(|(ct, raw)| &encode_ct_info(ct, SaveFormat::V71) == raw)
			&& encode_scalars(self) == self.raw.scalars
	}
}

#[cfg(test)]
mod tests {
	use super::super::encode::tests::load_fixture;
	use super::super::serialize::write_save;
	use super::*;

	const FIXTURE_DIMS: (u16, u16) = (50, 50);

	#[test]
	fn refresh_is_a_byte_noop_on_a_pristine_save_when_present() {
		let Some((raw, mut save)) = load_fixture() else { return };
		assert!(save.settings_regions_lossless(), "the fixture models every settings region losslessly");
		save.refresh_settings_regions();
		assert_eq!(write_save(&save).unwrap(), raw, "refreshing an unedited save must not change a byte");
	}

	#[test]
	fn extract_apply_round_trips_byte_exactly_when_present() {
		let Some((raw, mut save)) = load_fixture() else { return };
		let settings = SaveSettings::extract(&save);
		settings.apply_to(&mut save).expect("the fixture tail moves");
		assert_eq!(write_save(&save).unwrap(), raw, "an unchanged settings block must round-trip byte-exactly");
	}

	#[test]
	fn edited_settings_reach_the_emitted_bytes_when_present() {
		let Some((raw, mut save)) = load_fixture() else { return };
		let mut s = SaveSettings::extract(&save);
		s.save_name = "EDITED".into();
		s.team_names[1] = "New Green".into();
		s.team_clan[0] = 7;
		s.rng_seed ^= 0xDEAD_BEEF;
		s.options.start_gold = 777;
		s.options.opponent = 5;
		s.extra.effects ^= 1;
		s.turn_counter += 3;
		s.turn_timer = 12;
		s.is_cheater = 1;
		s.cheater_team = 2;
		s.teams[0].team_points = 4242;
		s.teams[0].research_level[3] = 9;
		s.teams[0].stats[0] = 21;
		s.teams[0].gold_spent_on_upgrades = 555;
		s.team_gold[0] = 1234;
		s.apply_to(&mut save).expect("the fixture tail moves");

		let bytes = write_save(&save).unwrap();
		assert_ne!(bytes, raw, "the edit must change the emitted bytes");
		let back = crate::save::read_save_bytes(&bytes, FIXTURE_DIMS).expect("edited save re-decodes");
		assert_eq!(back.header.save_name, "EDITED");
		assert_eq!(back.header.team_names[1], "New Green");
		assert_eq!(back.header.team_clan[0], 7);
		assert_eq!(back.teams[0].team_clan, 7, "the CTInfo clan copy tracks the header");
		assert_eq!(back.header.options.start_gold, 777);
		assert_eq!(back.header.options.opponent, 5);
		assert_eq!(back.extra_settings.effects, save.extra_settings.effects);
		assert_eq!(back.turn_counter, save.turn_counter);
		assert_eq!(back.turn_timer, 12);
		assert_eq!((back.is_cheater, back.cheater_team), (1, 2));
		assert_eq!(back.teams[0].team_points, 4242);
		assert_eq!(back.teams[0].research_topics[3][0], 9);
		assert_eq!(back.teams[0].stats[0], 21);
		assert_eq!(back.teams[0].stats_gold_spent_on_upgrades, 555);
		assert_eq!(back.team_units[0].gold, 1234);

		// Nothing outside the settings regions moved: same graph, same maps.
		assert_eq!(back.objects.len(), save.objects.len(), "object graph untouched");
		assert_eq!(back.surface_map, save.surface_map, "terrain untouched");
		assert_eq!(back.cargo_map, save.cargo_map, "resources untouched");
		let round = SaveSettings::extract(&back);
		assert_eq!(round, SaveSettings::extract(&save), "settings survive a full decode round-trip");
	}

	#[test]
	fn lossless_guard_detects_an_unmodeled_region_when_present() {
		let Some((_raw, mut save)) = load_fixture() else { return };
		assert!(save.settings_regions_lossless());
		// Flip a retained header byte the typed model doesn't carry: now the
		// region no longer re-encodes to its stored bytes.
		*save.raw.header.last_mut().expect("header region is non-empty") ^= 0xFF;
		assert!(!save.settings_regions_lossless(), "a diverged region must trip the guard");
	}

	/// A real save's tail decomposes: its message logs hold entries — including
	/// ones that inline a destroyed unit's whole body — and one AI block per
	/// COMPUTER team, and the walk lands on the last byte.
	#[test]
	fn a_real_saves_tail_decomposes_when_present() {
		let Some((_raw, save)) = load_fixture() else { return };
		assert!(save.tail_follows_the_graph(), "the fixture's tail decomposes, so any team type is on offer");
	}

	/// Every type a slot can take, applied in a chain and re-decoded each time:
	/// off the AI, out of the game, back in, and onto the AI again. The tail
	/// follows each step - the proof being that the edited bytes re-decode at
	/// all (a mis-shaped tail desyncs every region after the change).
	#[test]
	fn every_team_type_round_trips_through_a_real_save_when_present() {
		let Some((_raw, mut save)) = load_fixture() else { return };
		let slot = save.header.team_type.iter().position(|&t| t != 0).expect("an active slot exists");
		// Computer -> Player -> Remote -> None -> Eliminated -> Computer.
		for want in [1u32, 3, 0, 4, 2] {
			let mut s = SaveSettings::extract(&save);
			s.team_types[slot] = want;
			s.apply_to(&mut save).expect("the tail follows the type");
			let bytes = write_save(&save).unwrap();
			save = crate::save::read_save_bytes(&bytes, FIXTURE_DIMS).expect("the edited save re-decodes");
			assert_eq!(save.header.team_type[slot], want, "the header type stuck");
			assert_eq!(save.teams[slot].team_type as u32, want, "and the CTInfo copy tracks it");
			assert!(save.tail_follows_the_graph(), "and the tail still decomposes");
		}
	}

	/// The alien slot stays put: the game's own reader stops at slot 3, so a
	/// live alien slot is a save it could not load.
	#[test]
	fn the_alien_slot_refuses_a_type_when_present() {
		let Some((_raw, mut save)) = load_fixture() else { return };
		let mut s = SaveSettings::extract(&save);
		s.team_types[TEAM_COUNT - 1] = 1;
		let before = save.raw.tail.clone();
		assert!(s.apply_to(&mut save).is_err(), "refused");
		assert_eq!(save.raw.tail, before, "and nothing was written");
	}

	#[test]
	fn team_type_swap_reaches_both_stored_copies_when_present() {
		let Some((raw, mut save)) = load_fixture() else { return };
		// The fixture has no player/remote/eliminated slot, so establish the
		// shape first (a plain typed edit through the same path), then perform
		// the swap: PLAYER -> ELIMINATED keeps the heat-map set and the AI
		// state as they are (still non-NONE, still not computer).
		let slot = save.header.team_type.iter().position(|&t| t != 0).expect("an active slot exists");
		let mut s = SaveSettings::extract(&save);
		s.team_types[slot] = 1;
		s.apply_to(&mut save).expect("the fixture tail moves");

		let mut s = SaveSettings::extract(&save);
		assert_eq!(s.team_types[slot], 1, "the establishing edit stuck");
		s.team_types[slot] = 4;
		s.apply_to(&mut save).expect("the fixture tail moves");
		let bytes = write_save(&save).unwrap();
		assert_ne!(bytes, raw, "the swap must change the emitted bytes");
		let back = crate::save::read_save_bytes(&bytes, FIXTURE_DIMS).expect("edited save re-decodes");
		assert_eq!(back.header.team_type[slot], 4);
		assert_eq!(back.teams[slot].team_type, 4, "the CTInfo type copy tracks the header");
	}

	#[test]
	fn upgrade_edits_reach_the_master_current_values_when_present() {
		let Some((raw, mut save)) = load_fixture() else { return };
		let mut s = SaveSettings::extract(&save);
		let ut = s.team_upgrades[0].iter().position(Option::is_some).expect("table 0 holds current values");
		let mut want = s.team_upgrades[0][ut].unwrap();
		want[0] += 5; // attack
		want[7] += 2; // cost (turns)
		s.team_upgrades[0][ut] = Some(want);
		let base_before = save.team_units[0].base_values[ut].and_then(|i| save.values(i).cloned());
		s.apply_to(&mut save).expect("the fixture tail moves");

		let bytes = write_save(&save).unwrap();
		assert_ne!(bytes, raw, "the upgrade must change the emitted bytes");
		let back = crate::save::read_save_bytes(&bytes, FIXTURE_DIMS).expect("edited save re-decodes");
		assert_eq!(SaveSettings::extract(&back).team_upgrades[0][ut], Some(want), "the attributes round-trip");
		let base_after = back.team_units[0].base_values[ut].and_then(|i| back.values(i).cloned());
		assert_eq!(base_after, base_before, "base (factory) values never move");
	}

	#[test]
	fn a_shared_master_is_cloned_not_edited_in_place_when_present() {
		let Some((_raw, mut save)) = load_fixture() else { return };
		// Force the never-upgraded shape: table 0's current slot shares its
		// base object outright.
		let ut = (0..save.team_units[0].current_values.len())
			.find(|&u| save.team_units[0].base_values[u].is_some())
			.expect("a base slot exists");
		let base_idx = save.team_units[0].base_values[ut].unwrap();
		save.team_units[0].current_values[ut] = Some(base_idx);
		let objects_before = save.objects.len();
		let base_vals = save.values(base_idx).cloned().unwrap();

		let mut s = SaveSettings::extract(&save);
		let mut want = s.team_upgrades[0][ut].unwrap();
		want[3] += 7; // armor
		s.team_upgrades[0][ut] = Some(want);
		s.apply_to(&mut save).expect("the fixture tail moves");

		assert_eq!(save.objects.len(), objects_before + 1, "a fresh master was inserted");
		let cur_idx = save.team_units[0].current_values[ut].unwrap();
		let new_base_idx = save.team_units[0].base_values[ut].unwrap();
		assert_ne!(cur_idx, new_base_idx, "current no longer shares the base object");
		assert_eq!(save.values(new_base_idx), Some(&base_vals), "the base object kept its stats");
		assert_eq!(upgrade_of(save.values(cur_idx).unwrap()), want, "the fresh master carries the edit");

		// The regrown graph re-emits a structurally sound stream.
		let bytes = write_save(&save).unwrap();
		let back = crate::save::read_save_bytes(&bytes, FIXTURE_DIMS).expect("the cloned graph re-decodes");
		assert_eq!(SaveSettings::extract(&back).team_upgrades[0][ut], Some(want));
	}
}
