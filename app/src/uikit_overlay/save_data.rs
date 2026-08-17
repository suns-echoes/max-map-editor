//! The Edit Save Data dialog: the whole-form editor over a save's
//! [`SaveDataForm`](crate::savedata::SaveDataForm), plus its Issues list
//! (the validation step OK routes through before applying).

use super::*;

#[derive(Clone)]
pub(super) struct SaveDataIds {
	pub(super) tabs: WidgetId,
	// Game Setup.
	pub(super) save_name: WidgetId,
	/// Per-slot type select ([`crate::savedata::type_editable`] slots only fire).
	pub(super) team_type: [WidgetId; 5],
	pub(super) team_clan: [WidgetId; 5],
	pub(super) team_name: [WidgetId; 5],
	pub(super) start_gold: WidgetId,
	pub(super) timer: WidgetId,
	pub(super) endturn: WidgetId,
	pub(super) play_mode: WidgetId,
	pub(super) victory_type: WidgetId,
	pub(super) victory_limit: WidgetId,
	pub(super) opponent: WidgetId,
	pub(super) raw_res: WidgetId,
	pub(super) fuel_res: WidgetId,
	pub(super) gold_res: WidgetId,
	pub(super) derelicts: WidgetId,
	// Stats: one column per active slot - points, gold, built x4, gold spent.
	pub(super) stats_cols: Vec<(usize, [WidgetId; 7])>,
	// Research: one column per active slot - the eight topic levels.
	pub(super) research_cols: Vec<(usize, [WidgetId; 8])>,
	// Upgrades: the unit-type select + one column per active table slot.
	pub(super) utype: WidgetId,
	pub(super) upgrade_cols: Vec<(usize, [WidgetId; 8])>,
	// Advanced.
	pub(super) turn_counter: WidgetId,
	pub(super) turn_timer: WidgetId,
	pub(super) active_team: WidgetId,
	pub(super) player_team: WidgetId,
	pub(super) rng_seed: WidgetId,
	pub(super) cheater: WidgetId,
	pub(super) cheater_team: WidgetId,
	pub(super) effects: WidgetId,
	pub(super) click_scroll: WidgetId,
	pub(super) quick_scroll: WidgetId,
	pub(super) fast_movement: WidgetId,
	pub(super) follow_unit: WidgetId,
	pub(super) auto_select: WidgetId,
	pub(super) enemy_halt: WidgetId,
	pub(super) check: WidgetId,
	pub(super) cancel: WidgetId,
	pub(super) ok: WidgetId,
}

#[derive(Clone, Copy)]
pub(super) struct SaveDataIssuesIds {
	pub(super) back: WidgetId,
	pub(super) fix: WidgetId,
}

/// Edit Save Data: the one form width every tab shares — the Game Setup
/// page's natural width (its two-column Options grid). The tab subtree is
/// both strutted *and* capped to it (`Constrained::max_width`), so the
/// all-players tables shrink their columns to fit instead of widening the
/// dialog, and a tab switch never resizes the window.
const SD_W: f32 = 588.0;
/// Edit Save Data: the fixed tab-page height. All five pages share it (each
/// scrolls if it must) so the auto-sized window never jumps on a tab switch.
/// Sized to the tallest page (Game Setup: teams block + the Options section).
const SD_TAB_H: f32 = 468.0;
/// Edit Save Data: the label column of a form row and of a table (wider than
/// [`field_row`]'s 78px — stat labels like "Buildings built" need the room).
const SD_LABEL_W: f32 = 110.0;
/// The Issues list shows up to eight rows at [`SD_ISSUE_H`] each; more (or
/// wrapped-to-two-lines rows) scroll.
const SD_ISSUE_H: f32 = 20.0;
/// Edit Save Data: the page/footer padding. The window itself runs padding-0
/// so the tab bar mounts flush under the titlebar ([`Tabs::framed`]); each
/// page and the footer re-apply this inset.
const SD_PAD: f32 = 8.0;
/// Edit Save Data: how far the tab subtree clears the window's own frame on
/// each side. A padding-0 window hands its content the *whole* rect, frame
/// included, so a [`Tabs::framed`] band — which paints a raised face edge to
/// edge — would bury the dialog's left/right border. This is the frame's
/// thickness: the theme's border stroke plus its bevel ring (`2 * bevel`).
const SD_FRAME: f32 = 2.0;
/// Extra margin a column-split section gets: vertical around the block, and
/// the horizontal inset of its body under the caption.
const SD_SECTION_VPAD: f32 = 4.0;
const SD_SECTION_HPAD: f32 = 12.0;
/// An all-players table's cell height (a control row), the gap between cells,
/// and a column's inner padding inside its team wash.
const SD_CELL_H: f32 = 24.0;
const SD_CELL_GAP: f32 = 4.0;
const SD_TABLE_PAD: f32 = 4.0;

/// Edit Save Data dialog state: the open context (settings + display labels),
/// the canonical form values (captured from the visible tab each frame - a
/// hidden tab's widgets are unreachable, so the form is the truth), the
/// form's widget ids (kept here, not on the `Dialog` variant, so the ~45
/// ids don't dwarf the enum), the active-slot list backing the Stats team
/// select, the shown team/tab, the parked validation issues, and the status
/// note (Auto Fix report).
///
/// One struct, replaced wholesale by [`Overlay::open_save_data`] and reset
/// to default by [`Overlay::hide`].
#[derive(Default)]
pub(super) struct SaveDataState {
	pub(super) init: Option<SaveDataInit>,
	pub(super) ids: Option<SaveDataIds>,
	pub(super) form: SaveDataForm,
	pub(super) slots: Vec<usize>,
	/// Unit types the Upgrades tab offers (any active table holds current
	/// values for them), and the offered index currently shown.
	pub(super) utypes: Vec<usize>,
	pub(super) utype: usize,
	pub(super) tab: usize,
	pub(super) issues: Vec<savedata::Issue>,
	pub(super) note: String,
}

impl Overlay {
	/// Opens Edit Save Data (Edit > Experimental, S7.2): five tabs over every
	/// non-map setting of the embedded save. OK validates the whole form; an
	/// invalid form swaps to the Issues list (Back / Auto Fix) and nothing is
	/// applied until it validates clean. Check Errors runs the same validation
	/// without applying.
	pub fn open_save_data(&mut self, init: SaveDataInit) {
		self.sd = SaveDataState { form: SaveDataForm::from_init(&init), init: Some(init), ..Default::default() };
		self.build_save_data();
	}

	/// (Re)builds the Edit Save Data tree from the canonical [`Self::sd.form`]
	/// — on open, on a tab switch, on a team-type or unit-type re-pick, and on
	/// return from the Issues list. Rebuilding (rather than revealing a stale
	/// page) keeps every widget seeded from the form, which the visible tab
	/// re-captures each frame. The active-slot list (the tables' columns) and
	/// the Upgrades unit list derive from the form's *current* team types.
	pub(super) fn build_save_data(&mut self) {
		let init = self.sd.init.clone().expect("open_save_data seeded the context");
		let f = self.sd.form.clone();
		let types = f.team_type;
		self.sd.slots = savedata::active_slots(&types);
		// The unit types the Upgrades tab offers: units players can build and
		// control (mobile + stationary; not the FX/decoration rows a save
		// also carries) for which any active table holds master current
		// values. Validation skips the same rows (savedata::validate).
		let table_len = f.upgrades.iter().map(Vec::len).max().unwrap_or(0);
		self.sd.utypes = (0..table_len)
			.filter(|&ut| {
				max_assets::save::is_player_unit_type(ut as u16)
					&& (0..f.upgrades.len()).any(|ti| {
						types.get(ti).copied().unwrap_or(0) != 0 && f.upgrades[ti].get(ut).is_some_and(Option::is_some)
					})
			})
			.collect();
		self.sd.utype = self.sd.utype.min(self.sd.utypes.len().saturating_sub(1));

		// -- Game Setup ----------------------------------------------------
		let save_name = TextInput::with_text(&f.save_name).max_len(savedata::NAME_MAX);
		let mut type_ids = [WidgetId::NONE; 5];
		let mut clan_ids = [WidgetId::NONE; 5];
		let mut name_ids = [WidgetId::NONE; 5];
		let mut teams_col = column();
		for i in 0..5 {
			// A player slot picks any of the five types; the alien slot shows
			// its fixed one as a disabled select (same column, no lever).
			let ty = if savedata::type_editable(i) {
				Select::new(savedata::TYPE_CHOICES.map(savedata::team_type_label))
					.with_selected(savedata::TYPE_CHOICES.iter().position(|&t| t == types[i]).unwrap_or(0))
					.small()
			} else {
				Select::new([savedata::team_type_label(types[i])]).disabled(true).small()
			};
			let clan = Select::new(init.clan_names.iter().cloned()).with_selected(f.team_clan[i]).small();
			let name = TextInput::with_text(&f.team_name[i]).max_len(savedata::NAME_MAX);
			type_ids[i] = ty.id();
			clan_ids[i] = clan.id();
			name_ids[i] = name.id();
			teams_col = teams_col.push(
				Linear::row()
					.spacing(8.0)
					.cross_align(CrossAlign::Center)
					.child(Label::new(max_assets::save::TEAM_LABELS[i]).small(), Length::Fixed(44.0))
					.child(ty, Length::Fixed(96.0))
					.child(clan, Length::Fixed(150.0))
					.child(name, Length::Flex(1.0)),
			);
		}
		let start_gold = digits(&f.start_gold, 4);
		let timer = digits(&f.timer, 5);
		let endturn = digits(&f.endturn, 5);
		let play_mode = Select::new(savedata::PLAY_MODES).with_selected(f.play_mode).small();
		let victory_type = Select::new(savedata::VICTORY_TYPES).with_selected(f.victory_type).small();
		let victory_limit = digits(&f.victory_limit, 5);
		let opponent = Select::new(savedata::OPPONENTS).with_selected(f.opponent).small();
		let raw_res = Select::new(savedata::RESOURCE_LEVELS).with_selected(f.raw_res).small();
		let fuel_res = Select::new(savedata::RESOURCE_LEVELS).with_selected(f.fuel_res).small();
		let gold_res = Select::new(savedata::RESOURCE_LEVELS).with_selected(f.gold_res).small();
		let derelicts = Select::new(savedata::DERELICTS).with_selected(f.derelicts).small();
		let sg = (start_gold.id(), timer.id(), endturn.id(), play_mode.id(), victory_type.id());
		let sv = (victory_limit.id(), opponent.id(), raw_res.id(), fuel_res.id(), gold_res.id(), derelicts.id());
		let save_name_id = save_name.id();
		let setup = column()
			.push(sd_row("Save name", save_name))
			.push(Label::new(savedata::teams_hint(&init)).small().muted())
			.push(teams_col)
			.push(sd_section(
				"Options",
				column()
					.push(sd_pair(sd_row("Timer (s)", timer), sd_row("End turn (s)", endturn)))
					.push(sd_pair(sd_row("Play mode", play_mode), sd_row("Opponent", opponent)))
					.push(sd_pair(sd_row("Victory", victory_type), sd_row("Limit", victory_limit)))
					.push(sd_pair(sd_row("Start gold", start_gold), sd_row("Derelicts", derelicts)))
					.push(sd_pair(sd_row("Raw materials", raw_res), sd_row("Fuel", fuel_res)))
					.push(sd_half(sd_row("Gold", gold_res))),
			))
			.push(Label::new(format!("World {} - {} - format V71", init.world, init.category)).small().muted());

		// -- Stats: one all-players table (research moved to its own tab) ----
		let stats_rows =
			["Points", "Gold", "Factories built", "Mines built", "Buildings built", "Units built", "Gold spent"];
		let mut stats_cols: Vec<(usize, [WidgetId; 7])> = Vec::new();
		let mut stats_table = sd_table(&stats_rows);
		for &slot in &self.sd.slots {
			let ts = f.teams.get(slot).cloned().unwrap_or_default();
			let cells: Vec<TextInput> = vec![
				digits(&ts.points, 7),
				match &ts.gold {
					Some(g) => digits(g, 7),
					None => TextInput::with_text("").disabled(true),
				},
				digits(&ts.built[0], 5),
				digits(&ts.built[1], 5),
				digits(&ts.built[2], 5),
				digits(&ts.built[3], 5),
				digits(&ts.gold_spent, 7),
			];
			stats_cols.push((slot, std::array::from_fn(|k| cells[k].id())));
			stats_table = stats_table.child(sd_team_col(slot, cells), Length::Flex(1.0));
		}
		let stats = column().push(sd_section("Score and build counters", stats_table));

		// -- Research: topic levels, a column per active slot ----------------
		let mut research_cols: Vec<(usize, [WidgetId; 8])> = Vec::new();
		let mut research_table = sd_table(&max_assets::save::RESEARCH_TOPICS);
		for &slot in &self.sd.slots {
			let ts = f.teams.get(slot).cloned().unwrap_or_default();
			let cells: Vec<TextInput> = ts.research.iter().map(|v| digits(v, 4)).collect();
			research_cols.push((slot, std::array::from_fn(|t| cells[t].id())));
			research_table = research_table.child(sd_team_col(slot, cells), Length::Flex(1.0));
		}
		let research = column().push(sd_section("Research levels", research_table));

		// -- Upgrades: one unit type's master current values per view --------
		let utype_options: Vec<String> = self.sd.utypes.iter().map(|&ut| savedata::unit_label(ut)).collect();
		let no_tables = utype_options.is_empty();
		let utype = Select::new(if no_tables { vec!["(no unit tables)".to_string()] } else { utype_options })
			.with_selected(self.sd.utype)
			.disabled(no_tables)
			.max_visible(12)
			.small();
		let utype_id = utype.id();
		let ut = self.sd.utypes.get(self.sd.utype).copied();
		let mut upgrade_cols: Vec<(usize, [WidgetId; 8])> = Vec::new();
		let mut upgrade_table = sd_table(&max_assets::save::RESEARCH_TOPICS);
		for &slot in &self.sd.slots {
			// The alien slot has no team-units table and therefore no column.
			let Some(table) = f.upgrades.get(slot) else { continue };
			let cell_vals = ut.and_then(|ut| table.get(ut).cloned().flatten());
			let cells: Vec<TextInput> = match &cell_vals {
				Some(vals) => vals.iter().map(|v| digits(v, 5)).collect(),
				None => (0..8).map(|_| TextInput::with_text("").disabled(true)).collect(),
			};
			upgrade_cols.push((slot, std::array::from_fn(|a| cells[a].id())));
			upgrade_table = upgrade_table.child(sd_team_col(slot, cells), Length::Flex(1.0));
		}
		let upgrades = column()
			.push(sd_row("Unit type", utype))
			.push(sd_section("Purchased upgrade levels (the team's current unit stats)", upgrade_table))
			.push(
				Label::new("An edit installs a new master version - units already built keep their stats.")
					.small()
					.muted(),
			);

		// -- Advanced --------------------------------------------------------
		let players = savedata::player_slots(&types);
		let turn_counter = digits(&f.turn_counter, 6);
		let turn_timer = digits(&f.turn_timer, 5);
		let active_team = Select::new(self.sd.slots.iter().map(|&s| max_assets::save::TEAM_LABELS[s].to_string()))
			.with_selected(self.sd.slots.iter().position(|&s| s == f.active_team).unwrap_or(0))
			.small();
		let player_team = Select::new(players.iter().map(|&s| max_assets::save::TEAM_LABELS[s].to_string()))
			.with_selected(players.iter().position(|&s| s == f.player_team).unwrap_or(0))
			.small();
		let rng_seed = digits(&f.rng_seed, 10);
		let cheater = Checkbox::new("Marked as cheated").with_checked(f.cheater);
		let cheater_team = Select::new(max_assets::save::TEAM_LABELS).with_selected(f.cheater_team).small();
		let effects = Checkbox::new("Effects").with_checked(f.effects);
		let click_scroll = Checkbox::new("Click to scroll").with_checked(f.click_scroll);
		let quick_scroll = digits(&f.quick_scroll, 3);
		let fast_movement = Checkbox::new("Fast movement").with_checked(f.fast_movement);
		let follow_unit = Checkbox::new("Follow unit").with_checked(f.follow_unit);
		let auto_select = Checkbox::new("Auto select").with_checked(f.auto_select);
		let enemy_halt = Checkbox::new("Halt on enemy").with_checked(f.enemy_halt);
		let av = (turn_counter.id(), turn_timer.id(), active_team.id(), player_team.id(), rng_seed.id());
		let ax = (cheater.id(), cheater_team.id(), effects.id(), click_scroll.id(), quick_scroll.id());
		let ay = (fast_movement.id(), follow_unit.id(), auto_select.id(), enemy_halt.id());
		let advanced = column()
			.push(sd_section(
				"Game",
				column()
					.push(sd_pair(sd_row("Turn counter", turn_counter), sd_row("Time left (s)", turn_timer)))
					.push(sd_pair(sd_row("Active team", active_team), sd_row("Player team", player_team)))
					.push(sd_pair(sd_row("RNG seed", rng_seed), sd_row("By team", cheater_team)))
					.push(sd_half(Linear::row().push(cheater))),
			))
			.push(sd_section(
				"In-game preferences",
				column()
					.push(sd_pair(Linear::row().push(effects), Linear::row().push(click_scroll)))
					.push(sd_pair(Linear::row().push(fast_movement), Linear::row().push(follow_unit)))
					.push(sd_pair(Linear::row().push(auto_select), Linear::row().push(enemy_halt)))
					.push(sd_half(sd_row("Quick scroll", quick_scroll))),
			))
			.push(
				Label::new(format!("Game state {} - shown for reference, not editable", init.game_state))
					.small()
					.muted(),
			);

		// The tab bar sits flush under the titlebar on its raised band (the
		// window runs padding-0); each page carries the padding instead.
		let mut tabs = Tabs::new()
			.tab("Game Setup", sd_page(setup))
			.tab("Stats", sd_page(stats))
			.tab("Research", sd_page(research))
			.tab("Upgrades", sd_page(upgrades))
			.tab("Advanced", sd_page(advanced))
			.framed();
		tabs.set_active(self.sd.tab);
		let check = Button::new("Check Errors").secondary();
		let cancel = Button::new("Cancel").secondary();
		let ok = Button::new("OK").primary();
		let note = Label::new(self.sd.note.clone()).small().muted();
		let ids = SaveDataIds {
			tabs: tabs.id(),
			save_name: save_name_id,
			team_type: type_ids,
			team_clan: clan_ids,
			team_name: name_ids,
			start_gold: sg.0,
			timer: sg.1,
			endturn: sg.2,
			play_mode: sg.3,
			victory_type: sg.4,
			victory_limit: sv.0,
			opponent: sv.1,
			raw_res: sv.2,
			fuel_res: sv.3,
			gold_res: sv.4,
			derelicts: sv.5,
			stats_cols,
			research_cols,
			utype: utype_id,
			upgrade_cols,
			turn_counter: av.0,
			turn_timer: av.1,
			active_team: av.2,
			player_team: av.3,
			rng_seed: av.4,
			cheater: ax.0,
			cheater_team: ax.1,
			effects: ax.2,
			click_scroll: ax.3,
			quick_scroll: ax.4,
			fast_movement: ay.0,
			follow_unit: ay.1,
			auto_select: ay.2,
			enemy_halt: ay.3,
			check: check.id(),
			cancel: cancel.id(),
			ok: ok.id(),
		};
		let btn_row =
			Linear::row().spacing(8.0).push(check).child(Spacer::new(), Length::Flex(1.0)).push(cancel).push(ok);
		let footer = Linear::column().spacing(6.0).cross_align(CrossAlign::Stretch).padding(Insets {
			left: SD_PAD,
			top: 6.0,
			right: SD_PAD,
			bottom: SD_PAD,
		});
		let footer = status_slot(footer, note, SD_W - 2.0 * SD_PAD, 1).push(btn_row);
		// The band clears the window frame on both sides ([`SD_FRAME`]) — a
		// framed bar paints its face edge to edge, and the window runs
		// padding-0, so without this it would cover the dialog's own border.
		let band = Linear::column().cross_align(CrossAlign::Stretch).padding(Insets {
			left: SD_FRAME,
			top: 0.0,
			right: SD_FRAME,
			bottom: 0.0,
		});
		let content = Linear::column()
			.cross_align(CrossAlign::Stretch)
			.push(width_strut(SD_W))
			.child(band.push(Constrained::new(tabs).max_width(SD_W - 2.0 * SD_FRAME)), Length::Fixed(SD_TAB_H))
			.push(footer);
		// Only a form -> form rebuild (tab/team/unit switch) keeps its spot;
		// coming back from the small centred Issues window re-centres —
		// adopting its position could hang the taller form off the viewport
		// bottom.
		let same = matches!(self.dialog, Dialog::SaveData);
		let win = self.dialog_kept("Edit Save Data", content, same).padding(0.0);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.ui.focus_first();
		self.sd.ids = Some(ids);
		self.dialog = Dialog::SaveData;
		self.events.clear();
		self.visible = true;
	}

	/// Builds the Edit Save Data validation list: one line per invalid field
	/// (what it is, what to enter), Back to fix by hand, Auto Fix to write the
	/// nearest valid values. The form values stay parked in [`Self::sd.form`].
	pub(super) fn build_save_data_issues(&mut self) {
		let back = Button::new("Back").secondary();
		let fix = Button::new("Auto Fix").primary().focusable();
		let ids = SaveDataIssuesIds { back: back.id(), fix: fix.id() };
		let mut rows = column();
		for issue in &self.sd.issues {
			rows = rows.push(Label::new(format!("{} {}", issue.field, issue.message)).small().wrap_at(430.0));
		}
		let cap = (self.sd.issues.len().min(8) as f32) * SD_ISSUE_H + 2.0 * LIST_PAD;
		let n = self.sd.issues.len();
		let content = column()
			.push(width_strut(460.0))
			.push(
				Label::new(format!(
					"{n} value{} cannot be saved as entered - a corrupt value never reaches the save:",
					if n == 1 { "" } else { "s" }
				))
				.small()
				.wrap_at(460.0),
			)
			.child(Well::new(ScrollArea::new(rows)).padding(LIST_PAD).shaded(77), Length::Fixed(cap))
			.push(Label::new("Auto Fix replaces each value with the nearest valid one.").small().muted())
			.push(buttons(back, fix));
		let win = dialog("Invalid Save Data", content);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.ui.focus_first();
		self.dialog = Dialog::SaveDataIssues(ids);
		self.events.clear();
		self.visible = true;
	}

	/// Captures the visible tab's widget values into the canonical
	/// [`Self::sd.form`] — run every frame before acting on fires. Hidden tabs
	/// are unreachable (their widgets keep nothing newer than the form), so
	/// only the active page can differ. The one theoretical gap: an edit and a
	/// tab switch in the same event batch loses that edit — two pointer/key
	/// gestures never share a batch in practice.
	fn capture_save_data(&mut self, ids: &SaveDataIds) {
		let active = self.ui.get::<Tabs>(ids.tabs).map(Tabs::active).unwrap_or(self.sd.tab);
		match active {
			0 => {
				self.sd.form.save_name = self.text(ids.save_name);
				for i in 0..5 {
					// The alien slot holds a disabled single-option select; only
					// an editable slot's pick maps to a code.
					if savedata::type_editable(i) {
						let pos = self.sel(ids.team_type[i], usize::MAX);
						if let Some(&t) = savedata::TYPE_CHOICES.get(pos) {
							self.sd.form.team_type[i] = t;
						}
					}
					self.sd.form.team_clan[i] = self.sel(ids.team_clan[i], self.sd.form.team_clan[i]);
					self.sd.form.team_name[i] = self.text(ids.team_name[i]);
				}
				self.sd.form.start_gold = self.text(ids.start_gold);
				self.sd.form.timer = self.text(ids.timer);
				self.sd.form.endturn = self.text(ids.endturn);
				self.sd.form.play_mode = self.sel(ids.play_mode, self.sd.form.play_mode);
				self.sd.form.victory_type = self.sel(ids.victory_type, self.sd.form.victory_type);
				self.sd.form.victory_limit = self.text(ids.victory_limit);
				self.sd.form.opponent = self.sel(ids.opponent, self.sd.form.opponent);
				self.sd.form.raw_res = self.sel(ids.raw_res, self.sd.form.raw_res);
				self.sd.form.fuel_res = self.sel(ids.fuel_res, self.sd.form.fuel_res);
				self.sd.form.gold_res = self.sel(ids.gold_res, self.sd.form.gold_res);
				self.sd.form.derelicts = self.sel(ids.derelicts, self.sd.form.derelicts);
			}
			1 => {
				for (slot, c) in ids.stats_cols.clone() {
					let vals: [String; 7] = std::array::from_fn(|k| self.text(c[k]));
					let keep_gold = self.sd.form.teams.get(slot).map(|t| t.gold.is_some()).unwrap_or(false);
					let Some(ts) = self.sd.form.teams.get_mut(slot) else { continue };
					let [points, gold, b0, b1, b2, b3, gold_spent] = vals;
					ts.points = points;
					if keep_gold {
						ts.gold = Some(gold);
					}
					ts.built = [b0, b1, b2, b3];
					ts.gold_spent = gold_spent;
				}
			}
			2 => {
				for (slot, c) in ids.research_cols.clone() {
					let vals: [String; 8] = std::array::from_fn(|t| self.text(c[t]));
					if let Some(ts) = self.sd.form.teams.get_mut(slot) {
						ts.research = vals;
					}
				}
			}
			3 => {
				let Some(ut) = self.sd.utypes.get(self.sd.utype).copied() else { return };
				for (slot, c) in ids.upgrade_cols.clone() {
					let vals: [String; 8] = std::array::from_fn(|a| self.text(c[a]));
					// Disabled placeholder columns have no Some cell to write.
					if let Some(Some(cell)) = self.sd.form.upgrades.get_mut(slot).and_then(|t| t.get_mut(ut)) {
						*cell = vals;
					}
				}
			}
			_ => {
				self.sd.form.turn_counter = self.text(ids.turn_counter);
				self.sd.form.turn_timer = self.text(ids.turn_timer);
				// The selects hold list positions; the form stores slots.
				let players = savedata::player_slots(&self.sd.form.team_type);
				if let Some(&s) = self.sd.slots.get(self.sel(ids.active_team, usize::MAX)) {
					self.sd.form.active_team = s;
				}
				if let Some(&s) = players.get(self.sel(ids.player_team, usize::MAX)) {
					self.sd.form.player_team = s;
				}
				self.sd.form.rng_seed = self.text(ids.rng_seed);
				self.sd.form.cheater = self.checked(ids.cheater, self.sd.form.cheater);
				self.sd.form.cheater_team = self.sel(ids.cheater_team, self.sd.form.cheater_team);
				self.sd.form.effects = self.checked(ids.effects, self.sd.form.effects);
				self.sd.form.click_scroll = self.checked(ids.click_scroll, self.sd.form.click_scroll);
				self.sd.form.quick_scroll = self.text(ids.quick_scroll);
				self.sd.form.fast_movement = self.checked(ids.fast_movement, self.sd.form.fast_movement);
				self.sd.form.follow_unit = self.checked(ids.follow_unit, self.sd.form.follow_unit);
				self.sd.form.auto_select = self.checked(ids.auto_select, self.sd.form.auto_select);
				self.sd.form.enemy_halt = self.checked(ids.enemy_halt, self.sd.form.enemy_halt);
			}
		}
	}

	/// Visual-suite hook: show tab `tab` of the open Edit Save Data form
	/// (rebuilds from the canonical values, exactly like a header click).
	#[cfg(test)]
	pub(crate) fn show_save_data_tab_for_test(&mut self, tab: usize) {
		self.sd.tab = tab;
		self.build_save_data();
	}

	/// Visual-suite hook: park `issues` and show the Invalid Save Data list.
	#[cfg(test)]
	pub(crate) fn open_save_data_issues_for_test(&mut self, issues: Vec<savedata::Issue>) {
		self.sd.issues = issues;
		self.build_save_data_issues();
	}

	/// A select's current pick, or `fallback` when the widget is unreachable
	/// (a hidden tab).
	fn sel(&self, id: WidgetId, fallback: usize) -> usize {
		self.ui.get::<Select>(id).map(Select::selected).unwrap_or(fallback)
	}

	/// A checkbox's current state, or `fallback` when unreachable.
	fn checked(&self, id: WidgetId, fallback: bool) -> bool {
		self.ui.get::<Checkbox>(id).map(Checkbox::checked).unwrap_or(fallback)
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_save_data(&mut self) -> Outcome {
		let mut outcome = Outcome::Idle;
		let ids = self.sd.ids.clone().expect("build_save_data stored the ids");
		// The visible tab's widgets are the live copies — fold them into
		// the canonical form before acting on any fire.
		self.capture_save_data(&ids);
		if self.ui.fired(ids.cancel) {
			self.hide();
		} else if self.ui.fired(ids.ok) {
			// OK always checks first: an invalid form never applies.
			let init = self.sd.init.as_ref().expect("open seeded the context");
			let issues = self.sd.form.validate(init);
			if issues.is_empty() {
				outcome = Outcome::ApplySaveData(Box::new(self.sd.form.to_settings(init)));
				self.hide();
			} else {
				// Nothing is applied: park the list and show it, with
				// Back / Auto Fix. The form values survive in `sd_form`.
				self.sd.issues = issues;
				self.build_save_data_issues();
			}
		} else if self.ui.fired(ids.check) {
			// The same validation OK runs, without applying: issues open
			// the list, a clean form reports into the status slot.
			let init = self.sd.init.as_ref().expect("open seeded the context");
			let issues = self.sd.form.validate(init);
			if issues.is_empty() {
				self.sd.note = "No errors found.".into();
				self.build_save_data();
			} else {
				self.sd.issues = issues;
				self.build_save_data_issues();
			}
		} else if self.ui.fired(ids.tabs) {
			// The Tabs widget already switched its visible page; rebuild
			// so the revealed page is seeded from the canonical form
			// (its retained widgets may predate newer captures).
			self.sd.tab = self.ui.get::<Tabs>(ids.tabs).map(Tabs::active).unwrap_or(0);
			self.build_save_data();
		} else if self.ui.fired(ids.utype) {
			let picked = self.sel(ids.utype, self.sd.utype);
			if picked != self.sd.utype {
				// The outgoing type's cells were captured above; show
				// the picked type's stored values.
				self.sd.utype = picked;
				self.build_save_data();
			}
		} else if ids.team_type.iter().any(|&id| self.ui.fired(id)) {
			// A slot brought into the game needs a name; give it the
			// game's own default rather than leave a blank field for
			// Check Errors to nag about.
			for i in 0..max_assets::save::TEAM_COUNT {
				if self.sd.form.team_type[i] != 0 && self.sd.form.team_name[i].is_empty() {
					max_assets::save::TEAM_LABELS[i].clone_into(&mut self.sd.form.team_name[i]);
				}
			}
			// A re-typed team moves the active/player lists and the
			// tables' columns; rebuild from the captured form.
			self.build_save_data();
		}
		outcome
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_save_data_issues(&mut self, ids: SaveDataIssuesIds) -> Outcome {
		let outcome = Outcome::Idle;
		if self.ui.fired(ids.back) {
			self.build_save_data();
		} else if self.ui.fired(ids.fix) {
			let issues = std::mem::take(&mut self.sd.issues);
			self.sd.form.apply_fixes(&issues);
			self.sd.note = format!(
				"Auto Fix corrected {} value{} - review and press OK.",
				issues.len(),
				if issues.len() == 1 { "" } else { "s" }
			);
			self.build_save_data();
		}
		outcome
	}
}

/// Edit Save Data's `label : control` row — like [`field_row`] with the wider
/// [`SD_LABEL_W`] label column its stat labels need.
fn sd_row(label: &str, control: impl Widget + 'static) -> Linear {
	Linear::row()
		.spacing(8.0)
		.cross_align(CrossAlign::Center)
		.child(Label::new(label).small(), Length::Fixed(SD_LABEL_W))
		.child(control, Length::Flex(1.0))
}

/// Two [`sd_row`]s side by side (the Edit Save Data two-column grid).
fn sd_pair(a: Linear, b: Linear) -> Linear {
	Linear::row().spacing(16.0).child(a, Length::Flex(1.0)).child(b, Length::Flex(1.0))
}

/// A lone [`sd_row`] occupying the left column (the right stays empty).
fn sd_half(a: Linear) -> Linear {
	Linear::row().spacing(16.0).child(a, Length::Flex(1.0)).child(Spacer::new(), Length::Flex(1.0))
}

/// One Edit Save Data tab page: the content column re-applies the padding the
/// window gave up ([`SD_PAD`]; the tab bar is flush, pages are not), inside a
/// scroll area.
fn sd_page(content: Linear) -> ScrollArea {
	ScrollArea::new(content.padding(Insets::all(SD_PAD)))
}

/// A titled section whose body splits into columns (a pair grid or an
/// all-players table): the muted caption above, the body inset by the extra
/// section margins so the column split reads as its own block.
fn sd_section(title: &str, body: impl Widget + 'static) -> Linear {
	Linear::column()
		.spacing(6.0)
		.cross_align(CrossAlign::Stretch)
		.padding(Insets { left: 0.0, top: SD_SECTION_VPAD, right: 0.0, bottom: SD_SECTION_VPAD })
		.push(Label::new(title).small().muted())
		.child(
			Linear::column()
				.cross_align(CrossAlign::Stretch)
				.padding(Insets { left: SD_SECTION_HPAD, top: 0.0, right: SD_SECTION_HPAD, bottom: 0.0 })
				.push(body),
			Length::Fit,
		)
}

/// The start of an Edit Save Data all-players table: the row-label column (an
/// empty header slot over one label per row); the caller appends one
/// [`sd_team_col`] per shown team. Every column uses the same fixed cell
/// heights and gaps, so the rows align across columns.
fn sd_table(rows: &[&str]) -> Linear {
	let mut labels = Linear::column().spacing(SD_CELL_GAP).padding(Insets::all(SD_TABLE_PAD));
	labels = labels.child(Spacer::new(), Length::Fixed(SD_CELL_H));
	for label in rows {
		labels = labels.child(
			Linear::row().cross_align(CrossAlign::Center).push(Label::new(*label).small()),
			Length::Fixed(SD_CELL_H),
		);
	}
	Linear::row().spacing(SD_CELL_GAP).child(labels, Length::Fixed(SD_LABEL_W))
}

/// One team's table column: the team-coloured wash ([`crate::theme::TEAM_WASH`])
/// behind a centred team-name header over one fixed-height cell per row.
fn sd_team_col(slot: usize, cells: Vec<TextInput>) -> impl Widget + 'static {
	let mut col =
		Linear::column().spacing(SD_CELL_GAP).cross_align(CrossAlign::Stretch).padding(Insets::all(SD_TABLE_PAD));
	col = col.child(
		Linear::row()
			.main_align(MainAlign::Center)
			.cross_align(CrossAlign::Center)
			.push(Label::new(max_assets::save::TEAM_LABELS[slot]).small()),
		Length::Fixed(SD_CELL_H),
	);
	for cell in cells {
		col = col.child(cell, Length::Fixed(SD_CELL_H));
	}
	Stack::new().push(Fill::new(crate::uikit_theme::rgba(crate::theme::TEAM_WASH[slot]), Size::ZERO)).push(col)
}
