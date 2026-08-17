//! Edit Save Data (S7.2): the dialog's form model, its validation, and the
//! Auto Fix that repairs every invalid value — shared by `state` (which
//! prepares [`SaveDataInit`]) and `uikit_overlay` (which hosts the form).
//!
//! The form keeps every numeric field as the typed string so the dialog can
//! round-trip exactly what the user sees; [`SaveDataForm::validate`] turns
//! bad entries into [`Issue`]s (each naming the field, explaining the valid
//! range, and carrying the nearest-valid replacement), and only a form with
//! no issues converts to a [`SaveSettings`] block for
//! [`map_core::Project::apply_save_settings`]. Corrupt data therefore never
//! reaches the save: the dialog refuses OK until the issues are fixed —
//! by hand or via the Auto Fix button.

use max_assets::save::{RESEARCH_TOPICS, SaveSettings, TEAM_COUNT, TEAM_LABELS};

/// Clan display names in `TEAM_CLAN_*` order 1..=8 (max-port `enums.hpp`),
/// used when `PATCHES.RES` isn't loaded.
pub const CLAN_FALLBACK: [&str; 8] =
	["The Chosen", "Crimson Path", "Von Griffin", "Ayer's Hand", "Musashi", "Sacred Eights", "7 Knights", "Axis Inc"];

/// `PLAY_MODE_*` labels (0 turn based, 1 simultaneous moves).
pub const PLAY_MODES: [&str; 2] = ["Turn based", "Simultaneous moves"];

/// `VICTORY_TYPE_*` labels (0 duration in turns, 1 score in points).
pub const VICTORY_TYPES: [&str; 2] = ["Duration (turns)", "Score (points)"];

/// `OPPONENT_TYPE_*` labels (the AI difficulty), 0..=5.
pub const OPPONENTS: [&str; 6] = ["Clueless", "Apprentice", "Average", "Expert", "Master", "God"];

/// The three resource-density steps the game setup offers (0..=2).
pub const RESOURCE_LEVELS: [&str; 3] = ["Poor", "Medium", "Rich"];

/// Alien-derelict density steps (0..=2).
pub const DERELICTS: [&str; 3] = ["None", "Rare", "Common"];

/// A `TEAM_TYPE_*` code as a display word.
pub fn team_type_label(t: u32) -> &'static str {
	match t {
		0 => "None",
		1 => "Player",
		2 => "Computer",
		3 => "Remote",
		4 => "Eliminated",
		_ => "Unknown",
	}
}

/// The `TEAM_TYPE_*` codes a slot may take, in code order (so the select's
/// option index *is* the code). All five are on offer: the save's tail is
/// re-shaped to follow the change (`max_assets::save::tail`), so a slot can be
/// brought into the game, taken out of it, or handed to the AI.
pub const TYPE_CHOICES: [u32; 5] = [0, 1, 2, 3, 4];

/// Whether slot `i`'s type select is offered. Slots 0-3 take any of the five;
/// the alien slot is fixed, because the game's own reader stops at slot 3
/// (`SAVE-FROM-SCRATCH.md` §6.3) - a live alien slot is a save it could not
/// load.
pub fn type_editable(slot: usize) -> bool {
	slot + 1 < TEAM_COUNT
}

/// Whether slot `i`'s type moves it on or off the AI - the change that has to
/// reach past the tail's self-describing first region, and so the one a save
/// whose tail does not decompose cannot take
/// (`SaveDataInit::retype_supported`).
fn changes_ai(before: u32, after: u32) -> bool {
	(before == 2) != (after == 2)
}

/// The caption over the teams block: what the type selects will accept. The
/// alien slot is always fixed; a save whose tail does not decompose also holds
/// the Computer set where it is.
pub fn teams_hint(init: &SaveDataInit) -> &'static str {
	if init.retype_supported {
		"Teams (any type; Alien is fixed - the game reads only four teams)"
	} else {
		"Teams (this save's tail is opaque: Computer cannot be set or cleared; Alien is fixed)"
	}
}

/// A unit type's display name — the proper in-game name ("Tank"), the
/// `ResourceID` tag where the rules carry none, or a numeric fallback for an
/// id the table carries but the registry does not name.
pub fn unit_label(ut: usize) -> String {
	match max_assets::save::unit_type_name(ut as u16) {
		Some(tag) => max_assets::save::unit_display_name(tag).unwrap_or(tag).to_string(),
		None => format!("TYPE {ut}"),
	}
}

/// The slots that take part in the game (type != NONE), in slot order — the
/// Stats tab's team list and the Active-team select.
pub fn active_slots(team_types: &[u32; TEAM_COUNT]) -> Vec<usize> {
	(0..TEAM_COUNT).filter(|&i| team_types[i] != 0).collect()
}

/// The slots a human can own (type == PLAYER) — the Player-team select.
/// Falls back to every active slot if the save names no player (defensive:
/// such a save is already odd, but the select must offer something).
pub fn player_slots(team_types: &[u32; TEAM_COUNT]) -> Vec<usize> {
	let players: Vec<usize> = (0..TEAM_COUNT).filter(|&i| team_types[i] == 1).collect();
	if players.is_empty() { active_slots(team_types) } else { players }
}

/// Everything the dialog opens with — the editable settings block plus
/// display-only context resolved by `EditorState::execute` (so the overlay
/// never reaches into editor state). The save's original `TEAM_TYPE_*` codes
/// ride in `settings.team_types`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveDataInit {
	/// The attached save's current settings ([`map_core::Project::save_settings`]).
	pub settings: SaveSettings,
	/// Display line for the world reference, e.g. "GREEN_3.WRL".
	pub world: String,
	/// The save category label, e.g. "Custom game".
	pub category: String,
	/// The engine game-state code (display only).
	pub game_state: u16,
	/// Clan display names indexed by `TEAM_CLAN_*`: `[0]` = "Random", 1..=8
	/// the eight clans (`PATCHES.RES` names when loaded, else
	/// [`CLAN_FALLBACK`]).
	pub clan_names: Vec<String>,
	/// `SaveFile::tail_follows_the_graph` — whether this save's tail decomposes,
	/// so a slot can also be moved on or off the AI. `false` still allows every
	/// other type change (those never read past the tail's first region);
	/// [`SaveDataForm::validate`] raises the ones it cannot take.
	pub retype_supported: bool,
}

/// One team's Stats-tab fields, as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TeamStatsForm {
	pub points: String,
	/// `None` = this slot has no team-units table (the alien slot) — the gold
	/// field is blank and skipped.
	pub gold: Option<String>,
	/// Factories, mines, buildings, units built.
	pub built: [String; 4],
	pub gold_spent: String,
	/// Research level per topic, in [`RESEARCH_TOPICS`] order.
	pub research: [String; RESEARCH_TOPICS.len()],
}

/// The whole form, as typed — the canonical copy lives on the overlay and
/// survives tab switches and the Issues dialog round-trip.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveDataForm {
	pub save_name: String,
	pub team_name: [String; TEAM_COUNT],
	/// `TEAM_TYPE_*` per slot as currently edited. Editable slots swap within
	/// [`TYPE_CHOICES`]; locked slots always hold their original code. The
	/// active/player slot lists and the tables' columns derive from these.
	pub team_type: [u32; TEAM_COUNT],
	/// Clan select index per slot: 0 = Random, 1..=8 a clan (== `TEAM_CLAN_*`).
	pub team_clan: [usize; TEAM_COUNT],
	pub start_gold: String,
	pub timer: String,
	pub endturn: String,
	pub play_mode: usize,
	pub victory_type: usize,
	pub victory_limit: String,
	pub opponent: usize,
	pub raw_res: usize,
	pub fuel_res: usize,
	pub gold_res: usize,
	pub derelicts: usize,
	/// Per-slot stats (parallel to `settings.teams`, five in V71).
	pub teams: Vec<TeamStatsForm>,
	/// Per team-units table x unit type, the eight purchasable attributes as
	/// typed (parallel to `settings.team_upgrades`; `None` where the save
	/// holds no master current values for that type).
	pub upgrades: Vec<Vec<Option<[String; 8]>>>,
	pub turn_counter: String,
	pub turn_timer: String,
	/// Slot numbers (not list positions): the selects map them through the
	/// *current* [`active_slots`] / [`player_slots`] lists, which move when a
	/// team's type is re-picked.
	pub active_team: usize,
	pub player_team: usize,
	pub rng_seed: String,
	pub cheater: bool,
	/// Slot index 0..=4 (a plain five-team select).
	pub cheater_team: usize,
	pub effects: bool,
	pub click_scroll: bool,
	pub quick_scroll: String,
	pub fast_movement: bool,
	pub follow_unit: bool,
	pub auto_select: bool,
	pub enemy_halt: bool,
}

/// Which form field an [`Issue`]'s fix writes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
	SaveName,
	TeamName(usize),
	/// A slot's `TEAM_TYPE_*` (the fix string carries the code to revert to).
	TeamType(usize),
	StartGold,
	Timer,
	Endturn,
	VictoryLimit,
	TurnCounter,
	TurnTimer,
	RngSeed,
	QuickScroll,
	Points(usize),
	Gold(usize),
	Built(usize, usize),
	GoldSpent(usize),
	Research(usize, usize),
	/// An upgrades-table cell: (team-units table, unit type, attribute).
	Upgrade(usize, usize, usize),
}

/// One invalid field: where it lives (for the list and the fix), what is
/// wrong and what to enter instead, and the nearest-valid replacement the
/// Auto Fix button writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
	/// Display path, e.g. "Stats (Red) / Points".
	pub field: String,
	/// What is wrong and how to fix it by hand, e.g. "is empty - enter 0 to 9999".
	pub message: String,
	pub target: Target,
	/// The value Auto Fix writes into the field.
	pub fixed: String,
}

/// The numeric bounds, named so the dialog and the tests agree on one truth.
pub const START_GOLD_MAX: i64 = 9999;
pub const TIMER_MAX: i64 = 32767;
pub const VICTORY_LIMIT_MAX: i64 = 99999;
pub const TURN_COUNTER_MAX: i64 = 999_999;
pub const TURN_TIMER_MAX: i64 = 65535;
pub const RNG_SEED_MAX: i64 = u32::MAX as i64;
pub const QUICK_SCROLL_RANGE: (i64, i64) = (1, 255);
pub const POINTS_MAX: i64 = 9_999_999;
pub const BUILT_MAX: i64 = 32767;
pub const RESEARCH_MAX: i64 = 1000;
/// Upgrade attributes are `u16`s in the save.
pub const UPGRADE_MAX: i64 = 65535;
/// The game UI draws save titles and team names into fixed slots.
pub const NAME_MAX: usize = 30;

/// The lower bound of upgrade attribute `a` (in [`RESEARCH_TOPICS`] order):
/// zero hit points or a zero-turn build cost are degenerate, the rest may be 0.
pub fn upgrade_min(a: usize) -> i64 {
	if a == 4 || a == 7 { 1 } else { 0 }
}

impl SaveDataForm {
	/// Seed the form from the opened save's settings.
	pub fn from_init(init: &SaveDataInit) -> Self {
		let s = &init.settings;
		let teams = (0..s.teams.len())
			.map(|i| {
				let ts = &s.teams[i];
				TeamStatsForm {
					points: ts.team_points.to_string(),
					gold: s.team_gold.get(i).map(|g| g.to_string()),
					built: std::array::from_fn(|k| ts.stats[k].to_string()),
					gold_spent: ts.gold_spent_on_upgrades.to_string(),
					research: std::array::from_fn(|t| ts.research_level[t].to_string()),
				}
			})
			.collect();
		let upgrades = s
			.team_upgrades
			.iter()
			.map(|table| table.iter().map(|cell| cell.map(|vals| vals.map(|v| v.to_string()))).collect())
			.collect();
		SaveDataForm {
			save_name: s.save_name.clone(),
			team_name: s.team_names.clone(),
			team_type: s.team_types,
			team_clan: std::array::from_fn(|i| (s.team_clan[i] as usize).min(8)),
			start_gold: s.options.start_gold.to_string(),
			timer: s.options.timer.to_string(),
			endturn: s.options.endturn.to_string(),
			play_mode: (s.options.play_mode as usize).min(PLAY_MODES.len() - 1),
			victory_type: (s.options.victory_type as usize).min(VICTORY_TYPES.len() - 1),
			victory_limit: s.options.victory_limit.to_string(),
			opponent: (s.options.opponent as usize).min(OPPONENTS.len() - 1),
			raw_res: (s.options.raw_resource as usize).min(RESOURCE_LEVELS.len() - 1),
			fuel_res: (s.options.fuel_resource as usize).min(RESOURCE_LEVELS.len() - 1),
			gold_res: (s.options.gold_resource as usize).min(RESOURCE_LEVELS.len() - 1),
			derelicts: (s.options.alien_derelicts as usize).min(DERELICTS.len() - 1),
			teams,
			upgrades,
			turn_counter: s.turn_counter.to_string(),
			turn_timer: s.turn_timer.to_string(),
			active_team: s.active_turn_team as usize,
			player_team: s.player_team as usize,
			rng_seed: s.rng_seed.to_string(),
			cheater: s.is_cheater != 0,
			cheater_team: (s.cheater_team as usize).min(TEAM_COUNT - 1),
			effects: s.extra.effects != 0,
			click_scroll: s.extra.click_scroll != 0,
			quick_scroll: s.extra.quick_scroll.to_string(),
			fast_movement: s.extra.fast_movement != 0,
			follow_unit: s.extra.follow_unit != 0,
			auto_select: s.extra.auto_select != 0,
			enemy_halt: s.extra.enemy_halt != 0,
		}
	}

	/// Every invalid field, in tab order. Empty = the form is safe to apply.
	pub fn validate(&self, init: &SaveDataInit) -> Vec<Issue> {
		let mut out = Vec::new();
		check_name(&mut out, "Game Setup / Save name", Target::SaveName, &self.save_name, "UNTITLED");
		for (i, &t) in self.team_type.iter().enumerate() {
			// Defense in depth behind the constrained selects: a type change
			// the tail cannot follow is refused with a revert fix.
			let original = init.settings.team_types[i];
			if t == original {
				continue;
			}
			let why = if !type_editable(i) {
				Some("the game's own reader stops at slot 3, so a live alien slot would not load".to_string())
			} else if changes_ai(original, t) && !init.retype_supported {
				Some(format!(
					"this save's tail does not decompose, so {} cannot be moved {} the AI",
					TEAM_LABELS[i],
					if t == 2 { "onto" } else { "off" }
				))
			} else {
				None
			};
			if let Some(why) = why {
				out.push(Issue {
					field: format!("Game Setup / {} team type", TEAM_LABELS[i]),
					message: format!("cannot change {} to {} - {why}", team_type_label(original), team_type_label(t)),
					target: Target::TeamType(i),
					fixed: original.to_string(),
				});
			}
		}
		for (i, name) in self.team_name.iter().enumerate() {
			// Only slots that take part need a usable name.
			if self.team_type[i] != 0 {
				let field = format!("Game Setup / {} team name", TEAM_LABELS[i]);
				check_name_in(&mut out, field, Target::TeamName(i), name, TEAM_LABELS[i]);
			}
		}
		check_num(&mut out, "Game Setup / Start gold", Target::StartGold, &self.start_gold, 0, START_GOLD_MAX);
		check_num(&mut out, "Game Setup / Timer", Target::Timer, &self.timer, 0, TIMER_MAX);
		check_num(&mut out, "Game Setup / End turn", Target::Endturn, &self.endturn, 0, TIMER_MAX);
		check_num(
			&mut out,
			"Game Setup / Victory limit",
			Target::VictoryLimit,
			&self.victory_limit,
			1,
			VICTORY_LIMIT_MAX,
		);
		for (i, ts) in self.teams.iter().enumerate() {
			// Stats of a slot that isn't in the game stay zero and unedited;
			// validating them would only nag about a hidden column.
			if self.team_type.get(i).copied().unwrap_or(0) == 0 {
				continue;
			}
			let team = TEAM_LABELS[i.min(TEAM_LABELS.len() - 1)];
			check_num(&mut out, &format!("Stats ({team}) / Points"), Target::Points(i), &ts.points, 0, POINTS_MAX);
			if let Some(gold) = &ts.gold {
				check_num(&mut out, &format!("Stats ({team}) / Gold"), Target::Gold(i), gold, 0, POINTS_MAX);
			}
			for (k, v) in ts.built.iter().enumerate() {
				let label = ["Factories built", "Mines built", "Buildings built", "Units built"][k];
				check_num(&mut out, &format!("Stats ({team}) / {label}"), Target::Built(i, k), v, 0, BUILT_MAX);
			}
			check_num(
				&mut out,
				&format!("Stats ({team}) / Gold spent on upgrades"),
				Target::GoldSpent(i),
				&ts.gold_spent,
				0,
				POINTS_MAX,
			);
			for (t, v) in ts.research.iter().enumerate() {
				let field = format!("Research ({team}) / {}", RESEARCH_TOPICS[t]);
				check_num(&mut out, &field, Target::Research(i, t), v, 0, RESEARCH_MAX);
			}
		}
		for (ti, table) in self.upgrades.iter().enumerate() {
			// Same active-only rule per table (table index == slot, 0..4).
			if self.team_type.get(ti).copied().unwrap_or(0) == 0 {
				continue;
			}
			let team = TEAM_LABELS[ti.min(TEAM_LABELS.len() - 1)];
			for (ut, cell) in table.iter().enumerate() {
				// Only units players can build and control are offered for
				// editing; the FX/decoration rows a save also carries hold
				// legitimately degenerate stats (zero hits, zero cost) and
				// must neither nag nor be "fixed".
				if !max_assets::save::is_player_unit_type(ut as u16) {
					continue;
				}
				let Some(vals) = cell else { continue };
				let unit = unit_label(ut);
				for (a, v) in vals.iter().enumerate() {
					let field = format!("Upgrades ({team} / {unit}) / {}", RESEARCH_TOPICS[a]);
					check_num(&mut out, &field, Target::Upgrade(ti, ut, a), v, upgrade_min(a), UPGRADE_MAX);
				}
			}
		}
		check_num(&mut out, "Advanced / Turn counter", Target::TurnCounter, &self.turn_counter, 1, TURN_COUNTER_MAX);
		check_num(&mut out, "Advanced / Time left", Target::TurnTimer, &self.turn_timer, 0, TURN_TIMER_MAX);
		// The remaining turn time cannot exceed the per-turn limit (when one is set).
		if let (Ok(timer), Ok(left)) = (parse(&self.timer), parse(&self.turn_timer)) {
			if timer > 0 && left > timer && (0..=TIMER_MAX).contains(&timer) {
				out.push(Issue {
					field: "Advanced / Time left".into(),
					message: format!("is {left} but the turn timer is {timer} - enter 0 to {timer}"),
					target: Target::TurnTimer,
					fixed: timer.to_string(),
				});
			}
		}
		check_num(&mut out, "Advanced / RNG seed", Target::RngSeed, &self.rng_seed, 0, RNG_SEED_MAX);
		let (qmin, qmax) = QUICK_SCROLL_RANGE;
		check_num(&mut out, "Advanced / Quick scroll", Target::QuickScroll, &self.quick_scroll, qmin, qmax);
		out
	}

	/// Write each issue's nearest-valid value back into the form (Auto Fix).
	pub fn apply_fixes(&mut self, issues: &[Issue]) {
		for issue in issues {
			let fixed = issue.fixed.clone();
			match issue.target {
				Target::SaveName => self.save_name = fixed,
				Target::TeamName(i) => self.team_name[i] = fixed,
				Target::TeamType(i) => self.team_type[i] = fixed.parse().unwrap_or(self.team_type[i]),
				Target::StartGold => self.start_gold = fixed,
				Target::Timer => self.timer = fixed,
				Target::Endturn => self.endturn = fixed,
				Target::VictoryLimit => self.victory_limit = fixed,
				Target::TurnCounter => self.turn_counter = fixed,
				Target::TurnTimer => self.turn_timer = fixed,
				Target::RngSeed => self.rng_seed = fixed,
				Target::QuickScroll => self.quick_scroll = fixed,
				Target::Points(i) => self.teams[i].points = fixed,
				Target::Gold(i) => self.teams[i].gold = Some(fixed),
				Target::Built(i, k) => self.teams[i].built[k] = fixed,
				Target::GoldSpent(i) => self.teams[i].gold_spent = fixed,
				Target::Research(i, t) => self.teams[i].research[t] = fixed,
				Target::Upgrade(ti, ut, a) => {
					if let Some(Some(vals)) = self.upgrades.get_mut(ti).and_then(|t| t.get_mut(ut)) {
						vals[a] = fixed;
					}
				}
			}
		}
	}

	/// Convert the (validated) form into a settings block. Fields the form
	/// doesn't edit — the world option, team types, category — ride through
	/// from the opened settings. Call only when [`Self::validate`] is empty;
	/// unparseable values degrade to their range minimum rather than panic.
	pub fn to_settings(&self, init: &SaveDataInit) -> SaveSettings {
		let actives = active_slots(&self.team_type);
		let players = player_slots(&self.team_type);
		let mut s = init.settings.clone();
		s.save_name = self.save_name.clone();
		s.team_names = self.team_name.clone();
		s.team_types = self.team_type;
		s.team_clan = std::array::from_fn(|i| self.team_clan[i] as u32);
		s.rng_seed = num(&self.rng_seed, 0, RNG_SEED_MAX) as u32;
		s.options.timer = num(&self.timer, 0, TIMER_MAX) as i32;
		s.options.endturn = num(&self.endturn, 0, TIMER_MAX) as i32;
		s.options.start_gold = num(&self.start_gold, 0, START_GOLD_MAX) as i32;
		s.options.play_mode = self.play_mode as i32;
		s.options.victory_type = self.victory_type as i32;
		s.options.victory_limit = num(&self.victory_limit, 1, VICTORY_LIMIT_MAX) as i32;
		s.options.opponent = self.opponent as i32;
		s.options.raw_resource = self.raw_res as i32;
		s.options.fuel_resource = self.fuel_res as i32;
		s.options.gold_resource = self.gold_res as i32;
		s.options.alien_derelicts = self.derelicts as i32;
		s.extra.effects = self.effects as i32;
		s.extra.click_scroll = self.click_scroll as i32;
		s.extra.quick_scroll = num(&self.quick_scroll, QUICK_SCROLL_RANGE.0, QUICK_SCROLL_RANGE.1) as i32;
		s.extra.fast_movement = self.fast_movement as i32;
		s.extra.follow_unit = self.follow_unit as i32;
		s.extra.auto_select = self.auto_select as i32;
		s.extra.enemy_halt = self.enemy_halt as i32;
		// Slot numbers, kept valid against the *edited* type set: a slot whose
		// team was re-typed out of the list falls back to the list's first.
		let keep = |slot: usize, list: &[usize]| {
			if list.contains(&slot) { slot } else { list.first().copied().unwrap_or(0) }
		};
		s.active_turn_team = keep(self.active_team, &actives) as u8;
		s.player_team = keep(self.player_team, &players) as u8;
		s.turn_counter = num(&self.turn_counter, 1, TURN_COUNTER_MAX) as i32;
		s.turn_timer = num(&self.turn_timer, 0, TURN_TIMER_MAX) as u16;
		s.is_cheater = self.cheater as u32;
		s.cheater_team = self.cheater_team as u32;
		for (i, ts) in s.teams.iter_mut().enumerate() {
			let Some(f) = self.teams.get(i) else { continue };
			ts.team_points = num(&f.points, 0, POINTS_MAX) as u32;
			for (k, v) in f.built.iter().enumerate() {
				ts.stats[k] = num(v, 0, BUILT_MAX) as i16;
			}
			ts.gold_spent_on_upgrades = num(&f.gold_spent, 0, POINTS_MAX) as u32;
			for (t, v) in f.research.iter().enumerate() {
				ts.research_level[t] = num(v, 0, RESEARCH_MAX) as i32;
			}
		}
		for (i, gold) in s.team_gold.iter_mut().enumerate() {
			if let Some(Some(g)) = self.teams.get(i).map(|f| &f.gold) {
				*gold = num(g, 0, POINTS_MAX) as u32;
			}
		}
		for (ti, table) in s.team_upgrades.iter_mut().enumerate() {
			let Some(form_table) = self.upgrades.get(ti) else { continue };
			for (ut, cell) in table.iter_mut().enumerate() {
				// Rows outside the player-unit set ride through untouched —
				// they are never offered for editing, and clamping their
				// legitimate zeros to the Hits/Cost floors would corrupt them.
				if !max_assets::save::is_player_unit_type(ut as u16) {
					continue;
				}
				let (Some(vals), Some(Some(typed))) = (cell.as_mut(), form_table.get(ut)) else { continue };
				for (a, v) in typed.iter().enumerate() {
					vals[a] = num(v, upgrade_min(a), UPGRADE_MAX) as u16;
				}
			}
		}
		s
	}
}

fn parse(text: &str) -> Result<i64, ()> {
	let t = text.trim();
	if t.is_empty() { Err(()) } else { t.parse::<i64>().map_err(|_| ()) }
}

/// Parse-with-clamp for [`SaveDataForm::to_settings`] (validation has already
/// flagged anything out of range; this keeps the conversion total).
fn num(text: &str, min: i64, max: i64) -> i64 {
	parse(text).unwrap_or(min).clamp(min, max)
}

/// Range-check one numeric field into `issues`.
fn check_num(issues: &mut Vec<Issue>, field: &str, target: Target, text: &str, min: i64, max: i64) {
	let message = match parse(text) {
		Ok(v) if (min..=max).contains(&v) => return,
		Ok(v) => format!("is {v} - enter {min} to {max}"),
		Err(()) if text.trim().is_empty() => format!("is empty - enter {min} to {max}"),
		Err(()) => format!("is not a number - enter {min} to {max}"),
	};
	issues.push(Issue { field: field.to_string(), message, target, fixed: num(text, min, max).to_string() });
}

fn check_name(issues: &mut Vec<Issue>, field: &str, target: Target, name: &str, fallback: &str) {
	check_name_in(issues, field.to_string(), target, name, fallback);
}

/// Names must be non-empty, ASCII (the game font guarantees only that range)
/// and short enough for the game's fixed slots.
fn check_name_in(issues: &mut Vec<Issue>, field: String, target: Target, name: &str, fallback: &str) {
	let (message, fixed) = if name.trim().is_empty() {
		(format!("is empty - enter a name (up to {NAME_MAX} characters)"), fallback.to_string())
	} else if !name.chars().all(|c| (' '..='~').contains(&c)) {
		let cleaned: String = name.chars().map(|c| if (' '..='~').contains(&c) { c } else { '_' }).collect();
		("has non-ASCII characters the game font cannot show - use plain ASCII".to_string(), clip(&cleaned))
	} else if name.chars().count() > NAME_MAX {
		(format!("is longer than {NAME_MAX} characters - shorten it"), clip(name))
	} else {
		return;
	};
	issues.push(Issue { field, message, target, fixed });
}

fn clip(name: &str) -> String {
	name.chars().take(NAME_MAX).collect()
}

#[cfg(test)]
pub(crate) mod tests {
	use max_assets::save::{SaveExtraSettings, SaveOptions, TeamStats};

	use super::*;

	/// A small but fully-populated init: two players, one computer, stock-ish
	/// options — no fixture needed. Shared with the overlay's dialog tests.
	pub(crate) fn init() -> SaveDataInit {
		let options = SaveOptions {
			world: 14,
			timer: 180,
			endturn: 45,
			start_gold: 150,
			play_mode: 1,
			victory_type: 0,
			victory_limit: 50,
			opponent: 3,
			raw_resource: 1,
			fuel_resource: 1,
			gold_resource: 0,
			alien_derelicts: 0,
		};
		let team = |points: u32| TeamStats {
			team_points: points,
			research_level: [0, 1, 2, 3, 4, 5, 6, 7],
			stats: [1, 2, 3, 4],
			gold_spent_on_upgrades: 10,
		};
		SaveDataInit {
			settings: SaveSettings {
				save_name: "WIP".into(),
				team_names: ["Human".into(), "AI".into(), String::new(), String::new(), String::new()],
				team_clan: [1, 2, 0, 0, 0],
				rng_seed: 12345,
				options,
				extra: SaveExtraSettings {
					effects: 1,
					click_scroll: 1,
					quick_scroll: 16,
					fast_movement: 1,
					follow_unit: 0,
					auto_select: 0,
					enemy_halt: 1,
				},
				active_turn_team: 0,
				player_team: 0,
				turn_counter: 79,
				turn_timer: 31,
				is_cheater: 0,
				cheater_team: 0,
				teams: vec![team(100), team(50), team(0), team(0), team(0)],
				team_gold: vec![40, 30, 0, 0],
				team_types: [1, 2, 0, 0, 0],
				// Two unit types carry master current values (slots 1 and 2)
				// on the two active tables - a small but real upgrades page.
				team_upgrades: (0..4)
					.map(|t| {
						let mut col: Vec<Option<[u16; 8]>> = vec![None; 8];
						if t < 2 {
							col[1] = Some([22, 1, 4, 12, 24, 6, 4, 8]);
							col[2] = Some([14, 1, 3, 4, 16, 14, 7, 4]);
						}
						col
					})
					.collect(),
			},
			world: "GREEN_3.WRL".into(),
			category: "Custom game".into(),
			game_state: 8,
			clan_names: std::iter::once("Random".to_string()).chain(CLAN_FALLBACK.map(String::from)).collect(),
			retype_supported: true,
		}
	}

	#[test]
	fn an_unedited_form_validates_clean_and_round_trips() {
		let init = init();
		let form = SaveDataForm::from_init(&init);
		assert_eq!(form.validate(&init), vec![], "seeded values are valid");
		assert_eq!(form.to_settings(&init), init.settings, "no edit = the identical settings block");
	}

	#[test]
	fn edits_reach_the_settings_block() {
		let init = init();
		let mut form = SaveDataForm::from_init(&init);
		form.save_name = "NEW NAME".into();
		form.team_clan[0] = 8;
		form.start_gold = "250".into();
		form.play_mode = 0;
		form.teams[0].points = "4242".into();
		form.teams[1].gold = Some("77".into());
		form.teams[0].research[7] = "12".into();
		form.cheater = true;
		form.cheater_team = 1;
		form.active_team = 1; // slot 1 = Green
		form.upgrades[0][1].as_mut().unwrap()[0] = "30".into(); // Red tank attack
		assert_eq!(form.validate(&init), vec![]);
		let s = form.to_settings(&init);
		assert_eq!(s.save_name, "NEW NAME");
		assert_eq!(s.team_clan[0], 8);
		assert_eq!(s.options.start_gold, 250);
		assert_eq!(s.options.play_mode, 0);
		assert_eq!(s.teams[0].team_points, 4242);
		assert_eq!(s.team_gold[1], 77);
		assert_eq!(s.teams[0].research_level[7], 12);
		assert_eq!((s.is_cheater, s.cheater_team), (1, 1));
		assert_eq!(s.active_turn_team, 1, "the slot select carries through");
		assert_eq!(s.team_upgrades[0][1].unwrap()[0], 30, "the upgrade cell reaches the block");
	}

	#[test]
	fn invalid_values_are_flagged_with_fixes_and_auto_fix_clears_them() {
		let init = init();
		let mut form = SaveDataForm::from_init(&init);
		form.save_name = String::new(); // empty
		form.team_name[0] = "x".repeat(40); // too long
		form.start_gold = "700000".into(); // out of range
		form.victory_limit = String::new(); // empty
		form.turn_counter = "0".into(); // below the 1 floor
		form.turn_timer = "500".into(); // exceeds the 180s turn timer
		form.teams[0].points = "not a number".into();
		form.teams[1].research[0] = "99999".into();
		form.quick_scroll = "0".into();
		form.upgrades[1][2].as_mut().unwrap()[4] = "0".into(); // Green scout hits: below the 1 floor
		form.upgrades[0][1].as_mut().unwrap()[0] = "junk".into(); // Red tank attack

		let issues = form.validate(&init);
		let fields: Vec<&str> = issues.iter().map(|i| i.field.as_str()).collect();
		assert!(fields.contains(&"Game Setup / Save name"), "{fields:?}");
		assert!(fields.contains(&"Game Setup / Red team name"));
		assert!(fields.contains(&"Game Setup / Start gold"));
		assert!(fields.contains(&"Game Setup / Victory limit"));
		assert!(fields.contains(&"Advanced / Turn counter"));
		assert!(fields.contains(&"Advanced / Time left"));
		assert!(fields.contains(&"Stats (Red) / Points"));
		assert!(fields.contains(&"Research (Green) / Attack"));
		assert!(fields.contains(&"Advanced / Quick scroll"));
		let scout = unit_label(2);
		assert!(fields.contains(&format!("Upgrades (Green / {scout}) / Hits").as_str()), "{fields:?}");
		// Every message tells the user what to enter.
		assert!(issues.iter().all(|i| i.message.contains("enter") || i.message.contains("shorten")), "{issues:?}");

		form.apply_fixes(&issues);
		assert_eq!(form.validate(&init), vec![], "Auto Fix leaves a clean form");
		assert_eq!(form.save_name, "UNTITLED");
		assert_eq!(form.start_gold, "9999", "clamped to the nearest valid value");
		assert_eq!(form.turn_counter, "1");
		assert_eq!(form.turn_timer, "180", "time left clamps to the turn timer");
		assert_eq!(form.teams[0].points, "0", "a non-number falls to the range floor");
		assert_eq!(form.upgrades[1][2].as_ref().unwrap()[4], "1", "hits clamps to its 1 floor");
		assert_eq!(form.upgrades[0][1].as_ref().unwrap()[0], "0", "a non-number falls to the attack floor");
	}

	#[test]
	fn unplayable_unit_rows_are_neither_validated_nor_written() {
		let mut init = init();
		// SHIELDGN (0x04) is not a player unit; a save legitimately carries
		// degenerate master values for such rows (cf. the all-zero FX rows).
		init.settings.team_upgrades[0][4] = Some([0; 8]);
		let mut form = SaveDataForm::from_init(&init);
		assert_eq!(form.validate(&init), vec![], "zero hits/cost on an unplayable unit never nags");
		// Even a (hypothetical) typed edit is ignored: the row is not offered
		// for editing, and to_settings must not clamp its zeros to the floors.
		form.upgrades[0][4].as_mut().unwrap()[4] = "9".into();
		assert_eq!(form.validate(&init), vec![]);
		assert_eq!(form.to_settings(&init).team_upgrades[0][4], Some([0; 8]), "the row rides through untouched");
	}

	#[test]
	fn stats_of_inactive_slots_are_not_validated() {
		let init = init();
		let mut form = SaveDataForm::from_init(&init);
		form.teams[3].points = "garbage".into(); // Gray is TEAM_TYPE_NONE here
		assert_eq!(form.validate(&init), vec![], "an out-of-game slot never nags");
	}

	/// Every one of the five types is accepted on a player slot, including the
	/// two the tail has to be re-shaped for: bringing a slot into the game and
	/// handing one to the AI.
	#[test]
	fn every_type_is_accepted_on_a_player_slot() {
		let init = init();
		for want in [0, 1, 2, 3, 4] {
			let mut form = SaveDataForm::from_init(&init);
			form.team_type[3] = want; // Gray starts out of the game
			// A slot that takes part needs a name, whatever its type.
			form.team_name[3] = "Gray".into();
			assert_eq!(form.validate(&init), vec![], "Gray -> {}", team_type_label(want));
			assert_eq!(form.to_settings(&init).team_types[3], want);
		}
	}

	#[test]
	fn team_type_swaps_reach_the_settings_block() {
		let init = init();
		let mut form = SaveDataForm::from_init(&init);
		form.team_type[0] = 4; // Player -> Eliminated
		assert_eq!(form.validate(&init), vec![]);
		let s = form.to_settings(&init);
		assert_eq!(s.team_types[0], 4);
		// No player slot remains, so the player-team select fell back to the
		// active list, which still holds slot 0.
		assert_eq!(s.player_team, 0);
		assert_eq!(s.active_turn_team, 0);
	}

	/// The alien slot never takes a type - the game reads four teams, so a live
	/// alien slot is a save it could not load.
	#[test]
	fn the_alien_slot_is_flagged_and_reverted() {
		let init = init();
		let mut form = SaveDataForm::from_init(&init);
		form.team_type[4] = 1;
		form.team_name[4] = "Alien".into(); // so only the type is at issue
		let issues = form.validate(&init);
		assert_eq!(issues.len(), 1, "{issues:?}");
		assert_eq!(issues[0].target, Target::TeamType(4));
		form.apply_fixes(&issues);
		assert_eq!(form.team_type[4], 0, "the fix reverts to the original type");
		assert_eq!(form.validate(&init), vec![]);
	}

	/// On a save whose tail does not decompose, the AI set stays put - every
	/// other type change still goes through, because none of them reads past
	/// the tail's self-describing first region.
	#[test]
	fn an_ai_set_change_is_flagged_when_the_tail_is_opaque() {
		let init = SaveDataInit { retype_supported: false, ..init() };
		let mut form = SaveDataForm::from_init(&init);
		form.team_type[1] = 1; // Green: Computer -> Player
		let issues = form.validate(&init);
		assert_eq!(issues.len(), 1, "{issues:?}");
		assert_eq!(issues[0].field, "Game Setup / Green team type");
		assert_eq!(issues[0].target, Target::TeamType(1));
		assert!(issues[0].message.contains("does not decompose"), "{}", issues[0].message);
		form.apply_fixes(&issues);
		assert_eq!(form.team_type[1], 2, "the fix reverts to the original type");
		assert_eq!(form.validate(&init), vec![]);

		let mut form = SaveDataForm::from_init(&init);
		form.team_type[0] = 4; // Red: Player -> Eliminated, no AI involved
		assert_eq!(form.validate(&init), vec![], "a non-AI change is unaffected");
	}

	#[test]
	fn player_slots_fall_back_to_active_when_no_player_exists() {
		assert_eq!(player_slots(&[1, 2, 0, 0, 0]), vec![0]);
		assert_eq!(player_slots(&[0, 2, 2, 0, 0]), vec![1, 2], "computer-only saves offer the active slots");
		assert_eq!(active_slots(&[1, 2, 0, 0, 4]), vec![0, 1, 4]);
	}
}
