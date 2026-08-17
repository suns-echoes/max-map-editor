//! The Edit Tile Match Data editor (DEV): stages the shared match model,
//! edits mutual/one-way/group relations over the composed orientation strip,
//! and saves tiles.match.json through the shell.

use super::*;

#[derive(Clone, Copy)]
pub(super) struct MatchEditIds {
	/// Top row: pack switch + cross-cell size.
	pub(super) pack: WidgetId,
	pub(super) size: WidgetId,
	/// The two grouped tile lists, each with a filter select + its ScrollArea.
	pub(super) main_filter: WidgetId,
	pub(super) cand_filter: WidgetId,
	pub(super) main_list: WidgetId,
	pub(super) cand_list: WidgetId,
	pub(super) main_scroll: WidgetId,
	pub(super) cand_scroll: WidgetId,
	/// "main: X  cand: Y" line over the cross.
	pub(super) names: WidgetId,
	pub(super) cross: WidgetId,
	pub(super) orient: WidgetId,
	/// Per-tile controls: staged id + Set, the four pass buttons, the group
	/// assign select.
	pub(super) id_field: WidgetId,
	pub(super) id_set: WidgetId,
	pub(super) pass: [WidgetId; 4],
	pub(super) assign: WidgetId,
	/// Groups panel: the list + name field + add/rename/delete.
	pub(super) groups_list: WidgetId,
	pub(super) group_name: WidgetId,
	pub(super) group_add: WidgetId,
	pub(super) group_rename: WidgetId,
	pub(super) group_delete: WidgetId,
	/// Inline save-failure line.
	pub(super) error: WidgetId,
	pub(super) close: WidgetId,
	pub(super) reset: WidgetId,
	pub(super) save: WidgetId,
}

impl Overlay {
	/// Opens the Edit Tile Match Data editor (DEV) over the staged model.
	/// `strip` is the composed 9×64-cell texture (main tile + candidate at all
	/// 8 orientations); `atlas` is the shared rest-palette tile atlas the list
	/// thumbnails uv (`(tex, total count, active pack's base index)`).
	pub fn open_match_edit(
		&mut self,
		chrome: &mut MenuChrome,
		me: crate::matcheditor::MatchEditor,
		strip: &[u8],
		atlas: (TextureId, u32, u32),
	) {
		match self.me_strip_tex {
			Some(id) => chrome.replace_texture(id, strip, 9 * 64, 64),
			None => self.me_strip_tex = Some(chrome.register_texture(strip, 9 * 64, 64)),
		}
		self.me = Some(me);
		self.me_atlas = Some(atlas);
		self.build_match_edit();
		self.events.clear();
		self.visible = true;
	}

	/// (Re)builds the match-editor tree from the model. Called on open, a pack
	/// switch, and any group-set change (the filter + assign option lists bake
	/// group names in); everything else goes through per-frame setters.
	fn build_match_edit(&mut self) {
		use crate::matchview::{CrossView, LIST_W, OrientPicker, RowList};
		let me = self.me.as_ref().expect("model set in open_match_edit");
		let strip = self.me_strip_tex.expect("strip registered in open_match_edit");
		let atlas_tex = self.me_atlas.map(|(t, _, _)| t).unwrap_or(strip);
		let pd = me.pd();

		let pack = Select::new(me.pack_names()).small().with_selected(me.active);
		let size = Select::new((1..=6).map(|n| format!("x{n}"))).small().with_selected(me.cross_size as usize - 1);
		let filters = me.filter_labels();
		let main_filter = Select::new(filters.clone()).small().with_selected(me.filter_index(&pd.main_filter));
		let cand_filter = Select::new(filters).small().with_selected(me.filter_index(&pd.cand_filter));
		let main_list = RowList::new(atlas_tex, LIST_W);
		let cand_list = RowList::new(atlas_tex, LIST_W);
		let (main_list_id, cand_list_id) = (main_list.id(), cand_list.id());
		let main_scroll = ScrollArea::new(main_list);
		let cand_scroll = ScrollArea::new(cand_list);
		let names = Label::new("").small().with_id();
		let cross = CrossView::new(strip, 16.0 * me.cross_size as f32);
		let orient = OrientPicker::new(strip);
		let id_field = TextInput::with_text(pd.effective_id(pd.main_tile)).charset(Charset::Identifier).max_len(12);
		let id_set = Button::new("Set");
		let assign = Select::new(me.assign_labels()).small().with_selected(me.assign_index());
		let groups_list = RowList::new(atlas_tex, 150.0);
		let groups_list_id = groups_list.id();
		let groups_scroll = ScrollArea::new(groups_list);
		let group_name = TextInput::with_text("").charset(Charset::Identifier).max_len(12);
		let group_add = Button::new("Add");
		let group_rename = Button::new("Rename");
		let group_delete = Button::new("Delete");
		let error = Label::new("").small().with_id();
		let close = Button::new("Close").secondary();
		let dirty = me.dirty();
		let reset = Button::new("Reset").disabled(!dirty);
		let save = Button::new("Save").primary().disabled(!dirty);

		let mut ids = MatchEditIds {
			pack: pack.id(),
			size: size.id(),
			main_filter: main_filter.id(),
			cand_filter: cand_filter.id(),
			main_list: main_list_id,
			cand_list: cand_list_id,
			main_scroll: main_scroll.id(),
			cand_scroll: cand_scroll.id(),
			names: names.id(),
			cross: cross.id(),
			orient: orient.id(),
			id_field: id_field.id(),
			id_set: id_set.id(),
			pass: [WidgetId::NONE; 4],
			assign: assign.id(),
			groups_list: groups_list_id,
			group_name: group_name.id(),
			group_add: group_add.id(),
			group_rename: group_rename.id(),
			group_delete: group_delete.id(),
			error: error.id(),
			close: close.id(),
			reset: reset.id(),
			save: save.id(),
		};

		let row = || Linear::row().spacing(8.0).cross_align(CrossAlign::Center);
		let spacer = || Fill::new(Rgba::rgba(0, 0, 0, 0), Size::ZERO);
		let top = row()
			.push(Label::new("Pack").small().muted())
			.child(pack, Length::Fixed(170.0))
			.child(spacer(), Length::Flex(1.0))
			.push(Label::new("Cross").small().muted())
			.child(size, Length::Fixed(70.0));

		let main_col = column().push(main_filter).child(main_scroll, Length::Flex(1.0));
		let cand_col = column().push(cand_filter).child(cand_scroll, Length::Flex(1.0));
		let center = Linear::column()
			.spacing(6.0)
			.cross_align(CrossAlign::Center)
			.push(names)
			.push(cross)
			.push(Label::new("orientation").small().muted())
			.push(orient);
		let mid = Linear::row()
			.spacing(10.0)
			.cross_align(CrossAlign::Stretch)
			.child(main_col, Length::Fixed(LIST_W))
			.child(center, Length::Flex(1.0))
			.child(cand_col, Length::Fixed(LIST_W));

		let mut pass_row = row().push(Label::new("Pass").small().muted());
		for (i, name) in ["land", "water", "shore", "block"].iter().enumerate() {
			let b = Button::new(*name).with_selected(pd.cur.pass[pd.main_tile as usize] as usize == i);
			ids.pass[i] = b.id();
			pass_row = pass_row.push(b);
		}
		let left_col = column()
			.push(row().push(Label::new("Id").small().muted()).child(id_field, Length::Fixed(150.0)).push(id_set))
			.push(pass_row)
			.push(row().push(Label::new("Group").small().muted()).child(assign, Length::Fixed(150.0)));
		let groups_panel = Linear::row()
			.spacing(8.0)
			.cross_align(CrossAlign::Stretch)
			.child(groups_scroll, Length::Fixed(150.0))
			.child(
				Linear::column().spacing(4.0).push(group_name).push(group_add).push(group_rename).push(group_delete),
				Length::Fixed(100.0),
			);
		let lower = Linear::row()
			.spacing(16.0)
			.cross_align(CrossAlign::Stretch)
			.child(left_col, Length::Flex(1.0))
			.child(groups_panel, Length::Flex(1.0));

		let buttons_row =
			Linear::row().spacing(8.0).push(close).child(spacer(), Length::Flex(1.0)).push(reset).push(save);
		// The lower band is height-capped so the groups ScrollArea clips (a
		// ScrollArea measures at its content height; the cap is what scrolls
		// it) and the flexed list row above keeps the leftover.
		let lower_capped = Linear::column().cross_align(CrossAlign::Stretch).child(lower, Length::Fixed(118.0));
		let content = column().push(top).child(mid, Length::Flex(1.0)).push(lower_capped).push(error).push(buttons_row);
		// Fixed size, but still keep its place across rebuilds (see `dialog_kept`).
		let win = Window::new("Edit Tile Match Data", content).size(Size::new(860.0, 620.0)).resizable(true);
		let win = match self.hold_pos(matches!(self.dialog, Dialog::MatchEdit(_))) {
			Some(pos) => win.pos(pos),
			None => win.centered(),
		};
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.dialog = Dialog::MatchEdit(ids);
		self.blocking = true;
	}

	/// The row view-models for the main (left) or candidate list.
	fn match_rows(&self, left: bool) -> Vec<crate::matchview::ListRow> {
		use crate::matcheditor::{Row, RowTone, dirs_tag};
		let Some(me) = self.me.as_ref() else { return Vec::new() };
		let Some((_, count, base)) = self.me_atlas else { return Vec::new() };
		let pd = me.pd();
		let (filter, sel) = if left { (&pd.main_filter, pd.main_tile) } else { (&pd.cand_filter, pd.cand_tile) };
		me.rows(filter)
			.into_iter()
			.map(|row| {
				let (rep, header) = match row {
					Row::Ungrouped => (pd.ungrouped_tiles().first().copied().unwrap_or(0), true),
					Row::Group(gi) => (*pd.cur.groups[gi].tiles.iter().min().unwrap_or(&0), true),
					Row::Tile(t) => (t, false),
				};
				let label = match row {
					Row::Ungrouped => "[ungrouped]".to_string(),
					Row::Group(gi) => pd.cur.groups[gi].name.clone(),
					Row::Tile(t) => pd.effective_id(t).to_string(),
				};
				// Candidate rows carry the matched-direction tag vs the main tile.
				let tag = if left || matches!(row, Row::Ungrouped) {
					String::new()
				} else {
					dirs_tag(pd.matched_dirs(pd.main_tile, rep))
				};
				let tone = me.row_tone(sel, row);
				crate::matchview::ListRow {
					label,
					tag,
					thumb: Some(crate::tile_atlas::uv(base + rep as u32, count)),
					header,
					collapsed: header && me.is_collapsed(row),
					tone,
					selected: tone == RowTone::Select,
				}
			})
			.collect()
	}

	/// The groups-panel row view-models (explicit variant groups only).
	fn match_group_rows(&self) -> Vec<crate::matchview::ListRow> {
		use crate::matcheditor::RowTone;
		let Some(me) = self.me.as_ref() else { return Vec::new() };
		let pd = me.pd();
		me.pd()
			.real_groups()
			.into_iter()
			.map(|gi| {
				let g = &pd.cur.groups[gi];
				crate::matchview::ListRow {
					label: format!("{} ({})", g.name, g.tiles.len()),
					tag: String::new(),
					thumb: None,
					header: false,
					collapsed: false,
					tone: if pd.has_rule(&g.name) { RowTone::Rule } else { RowTone::Warn },
					selected: gi == me.group_sel,
				}
			})
			.collect()
	}

	/// Pushes the model into the match-editor widgets (rows, cross, previews,
	/// pass/assign/dirty states) - called every frame after the events resolve.
	fn sync_match_view(&mut self, ids: MatchEditIds) {
		use crate::matchview::{CrossView, OrientPicker, RowList, SideView, strip_uv};
		let rows_main = self.match_rows(true);
		let rows_cand = self.match_rows(false);
		let rows_groups = self.match_group_rows();
		let Some(me) = self.me.as_ref() else { return };
		let pd = me.pd();
		let names = format!("main: {}  cand: {}", pd.group_name(pd.main_tile), pd.group_name(pd.cand_tile));
		let cell = 16.0 * me.cross_size as f32;
		let bits = me.cand_xform.bits() as usize;
		let sides: [SideView; 4] = std::array::from_fn(|d| SideView {
			wildcard: pd.wildcard(d),
			matched: pd.wildcard(d).is_none() && pd.match_present(pd.main_tile, pd.cand_tile, d, me.cand_xform),
		});
		let mut orient_sides = [[None; 4]; 8];
		for (k, per) in orient_sides.iter_mut().enumerate() {
			let xf = map_core::Transform { rot: (k & 3) as u8, mirror: k & 4 != 0 };
			for (d, slot) in per.iter_mut().enumerate() {
				if pd.wildcard(d).is_none() {
					*slot = Some((strip_uv(1 + k, 9), pd.match_present(pd.main_tile, pd.cand_tile, d, xf)));
				}
			}
		}
		let dirty = me.dirty();
		let pass_val = pd.cur.pass[pd.main_tile as usize] as usize;
		let assign_idx = me.assign_index();
		if let Some(l) = self.ui.get_mut::<RowList>(ids.main_list) {
			l.set_rows(rows_main);
		}
		if let Some(l) = self.ui.get_mut::<RowList>(ids.cand_list) {
			l.set_rows(rows_cand);
		}
		if let Some(l) = self.ui.get_mut::<RowList>(ids.groups_list) {
			l.set_rows(rows_groups);
		}
		self.set_label(ids.names, &names);
		if let Some(c) = self.ui.get_mut::<CrossView>(ids.cross) {
			c.set_cell(cell);
			c.set_state(sides, strip_uv(0, 9), strip_uv(1 + bits, 9));
		}
		if let Some(o) = self.ui.get_mut::<OrientPicker>(ids.orient) {
			o.set_state(bits, strip_uv(0, 9), orient_sides);
		}
		for (i, id) in ids.pass.into_iter().enumerate() {
			if let Some(b) = self.ui.get_mut::<Button>(id) {
				b.set_selected(i == pass_val);
			}
		}
		if let Some(s) = self.ui.get_mut::<Select>(ids.assign) {
			s.set_selected(assign_idx);
		}
		for id in [ids.reset, ids.save] {
			if let Some(b) = self.ui.get_mut::<Button>(id) {
				b.set_disabled(!dirty);
			}
		}
	}

	/// The open match editor's `(pack, main tile, cand tile)` - the shell's key
	/// for recomposing the orientation strip texture when it moves.
	pub fn match_strip_key(&self) -> Option<(usize, u16, u16)> {
		if !matches!(self.dialog, Dialog::MatchEdit(_)) {
			return None;
		}
		let pd = self.me.as_ref()?.pd();
		Some((pd.pack, pd.main_tile, pd.cand_tile))
	}

	/// Rewrites the strip texture in place (9 cells of 64×64 RGBA).
	pub fn update_match_strip(&mut self, chrome: &MenuChrome, rgba: &[u8]) {
		if let Some(id) = self.me_strip_tex {
			chrome.update_texture(id, rgba);
		}
	}

	/// Re-syncs the shared tile atlas the list thumbnails uv (it recomposes on
	/// palette / document changes; the id is stable but the count can move).
	pub fn sync_match_atlas(&mut self, tex: TextureId, count: u32, pack_base: u32) {
		if matches!(self.dialog, Dialog::MatchEdit(_)) {
			self.me_atlas = Some((tex, count, pack_base));
		}
	}

	/// Save succeeded: the staged edits become the new baseline.
	pub fn match_saved(&mut self) {
		if let Some(me) = self.me.as_mut() {
			me.mark_saved();
		}
		if let Dialog::MatchEdit(ids) = self.dialog {
			self.set_label(ids.error, "");
		}
	}

	/// Save failed: show the message inline (the dialog stays open).
	pub fn match_error(&mut self, message: &str) {
		if let Dialog::MatchEdit(ids) = self.dialog {
			self.set_label(ids.error, message);
		}
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_match_edit(&mut self, ids: MatchEditIds) -> Outcome {
		let mut outcome = Outcome::Idle;
		use crate::matchview::{CrossView, OrientPicker, RowList};
		let mut rebuild = false;
		// Pack switch: the working copies persist; the tree rebuilds
		// (its option lists + id field bake the active pack in).
		if self.ui.fired(ids.pack) {
			let i = self.ui.get::<Select>(ids.pack).map(Select::selected).unwrap_or(0);
			if let Some(me) = self.me.as_mut() {
				me.set_active(i);
			}
			rebuild = true;
		}
		if self.ui.fired(ids.size) {
			let i = self.ui.get::<Select>(ids.size).map(Select::selected).unwrap_or(2);
			if let Some(me) = self.me.as_mut() {
				me.cross_size = (i as u8 + 1).clamp(1, 6);
			}
		}
		// Filters reset their list's scroll.
		for (fid, sid, left) in [(ids.main_filter, ids.main_scroll, true), (ids.cand_filter, ids.cand_scroll, false)] {
			if self.ui.fired(fid) {
				let i = self.ui.get::<Select>(fid).map(Select::selected).unwrap_or(0);
				if let Some(me) = self.me.as_mut() {
					let f = me.filter_of(i);
					if left {
						me.pd_mut().main_filter = f;
					} else {
						me.pd_mut().cand_filter = f;
					}
				}
				if let Some(s) = self.ui.get_mut::<ScrollArea>(sid) {
					s.set_offset(0.0);
				}
			}
		}
		// List clicks: fold gutter collapses, anywhere else selects.
		for (lid, left) in [(ids.main_list, true), (ids.cand_list, false)] {
			let clicked = self.ui.get_mut::<RowList>(lid).and_then(RowList::take_clicked);
			if let (Some((i, fold)), Some(me)) = (clicked, self.me.as_mut()) {
				let filter = if left { me.pd().main_filter.clone() } else { me.pd().cand_filter.clone() };
				if let Some(&row) = me.rows(&filter).get(i) {
					if fold {
						me.toggle_collapse(row);
					} else {
						me.select_row(left, row);
						if left {
							let id = me.pd().effective_id(me.pd().main_tile).to_string();
							self.set_text(ids.id_field, &id);
						}
					}
				}
			}
		}
		// Groups-panel selection loads the name field.
		let gclick = self.ui.get_mut::<RowList>(ids.groups_list).and_then(RowList::take_clicked);
		if let (Some((i, _)), Some(me)) = (gclick, self.me.as_mut()) {
			if let Some(&gi) = me.pd().real_groups().get(i) {
				me.group_sel = gi;
				let name = me.pd().cur.groups[gi].name.clone();
				self.set_text(ids.group_name, &name);
			}
		}
		// Cross: LMB toggles the match (unless wildcarded), RMB cycles
		// the side's edge type tile→water→land.
		let presses = self.ui.get_mut::<CrossView>(ids.cross).map(CrossView::take_presses).unwrap_or_default();
		if let Some(me) = self.me.as_mut() {
			for p in presses {
				if p.primary {
					if me.pd().wildcard(p.dir).is_none() {
						let cx = me.cand_xform;
						me.pd_mut().toggle_match(p.dir, cx);
					}
				} else {
					me.pd_mut().cycle_wildcard(p.dir);
				}
			}
		}
		if let Some(k) = self.ui.get_mut::<OrientPicker>(ids.orient).and_then(OrientPicker::take_picked) {
			if let Some(me) = self.me.as_mut() {
				me.cand_xform = map_core::Transform { rot: (k & 3) as u8, mirror: k & 4 != 0 };
			}
		}
		// Per-tile controls.
		if self.ui.fired(ids.id_set) {
			let new = self.text(ids.id_field);
			if let Some(me) = self.me.as_mut() {
				let t = me.pd().main_tile;
				if !me.pd_mut().set_id(t, new.trim()) {
					let cur = me.pd().effective_id(t).to_string();
					self.set_text(ids.id_field, &cur); // reject: restore
				}
			}
		}
		if let Some(i) = ids.pass.iter().position(|id| self.ui.fired(*id)) {
			if let Some(me) = self.me.as_mut() {
				let t = me.pd().main_tile;
				me.pd_mut().set_pass(t, i as u8);
			}
		}
		if self.ui.fired(ids.assign) {
			let i = self.ui.get::<Select>(ids.assign).map(Select::selected).unwrap_or(0);
			if let Some(me) = self.me.as_mut() {
				let labels = me.assign_labels();
				let target = (i > 0).then(|| labels.get(i).cloned().unwrap_or_default());
				let t = me.pd().main_tile;
				me.pd_mut().move_tile(t, target.as_deref());
			}
		}
		// Group ops change the option lists → rebuild.
		if self.ui.fired(ids.group_add) {
			let name = self.text(ids.group_name).trim().to_string();
			if !name.is_empty() {
				if let Some(me) = self.me.as_mut() {
					me.group_sel = me.pd_mut().add_group(&name);
				}
				rebuild = true;
			}
		}
		if self.ui.fired(ids.group_rename) {
			let name = self.text(ids.group_name).trim().to_string();
			if let Some(me) = self.me.as_mut() {
				let gi = me.group_sel;
				if gi < me.pd().cur.groups.len() {
					me.pd_mut().rename_group(gi, &name);
				}
			}
			rebuild = true;
		}
		if self.ui.fired(ids.group_delete) {
			if let Some(me) = self.me.as_mut() {
				let gi = me.group_sel;
				if gi < me.pd().cur.groups.len() {
					me.pd_mut().delete_group(gi);
				}
				me.group_sel = 0;
			}
			rebuild = true;
		}
		// Bottom row.
		if self.ui.fired(ids.close) {
			self.hide();
			outcome = Outcome::MatchClose;
		} else if self.ui.fired(ids.reset) {
			if let Some(me) = self.me.as_mut() {
				me.reset();
				let id = me.pd().effective_id(me.pd().main_tile).to_string();
				self.set_text(ids.id_field, &id);
			}
		} else if self.ui.fired(ids.save) {
			if let Some(me) = self.me.as_mut() {
				me.symmetrize();
				let commits = me.commits();
				if commits.is_empty() {
					// A save with nothing staged used to do literally
					// nothing - no ack, no line - which reads as a dead
					// button. Every other outcome here reports itself.
					self.set_label(ids.error, "no changes to save");
				} else {
					outcome = Outcome::MatchSave(commits);
				}
			}
		}
		if matches!(self.dialog, Dialog::MatchEdit(_)) {
			if rebuild {
				self.build_match_edit();
			} else {
				self.sync_match_view(ids);
			}
		}
		outcome
	}
}
