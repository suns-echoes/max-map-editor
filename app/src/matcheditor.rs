//! Edit Tile Match Data modal (DEV only): a visual editor for a pack's
//! `tiles.match.json` adjacency rules + `tiles.variants.json` grouping, plus
//! per-tile id and pass editing.
//!
//! A resizable dialog with a **main** tile list and a **candidate** list (each
//! grouped under selectable group headers, with a filter), a borderless **cross**
//! (main tile centre, candidate on the four sides; LMB toggles the match on a
//! side, RMB cycles that side's edge tile→water→land), an **orientation picker**
//! (eight mini-cross previews), a **groups** panel (add/rename/delete + a per-tile
//! group select), and per-tile **id** / **pass** controls.
//!
//! Everything is staged in a working copy; **Save** applies it (id renames cascade
//! to the pack files + every shipped map + template; pass writes the pack pass
//! table only; match/grouping write the pack match/variants), **Reset** drops it.
//! Matching is per group (`group_of` - a variant group, else the id family); the
//! cross/list operate on the selected tile's group.

use std::collections::{HashMap, HashSet};

use map_core::{MatchRule, Project, TileRef, Transform, family_of};

use crate::picker::{TileQuad, global_index};
use crate::textinput::{Charset, TextInput};
use crate::theme;
use crate::ui::{self, Hot, Rect, UiQuads};

const TITLE_H: f32 = 22.0;
const PAD: f32 = 10.0;
const BTN_H: f32 = 20.0;
const LIST_W: f32 = 168.0;
const ROW_H: f32 = 20.0;
const THUMB: f32 = 16.0;
const SEL_H: f32 = 20.0;
const HANDLE: f32 = 14.0;
/// Default + minimum dialog size.
const DEF_W: f32 = 860.0;
const DEF_H: f32 = 620.0;
const MIN_W: f32 = 680.0;
const MIN_H: f32 = 520.0;

const WATER_COL: [f32; 4] = [0.12, 0.45, 0.95, 1.0];
const LAND_COL: [f32; 4] = [0.10, 0.62, 0.16, 1.0];
const GREEN: [f32; 4] = theme::ACCENT;
const YELLOW: [f32; 4] = [0.93, 0.83, 0.22, 1.0];
const ORANGE: [f32; 4] = [0.96, 0.56, 0.16, 1.0];

/// A list/region filter: everything, only un-ruled groups, or one named group.
#[derive(Clone, PartialEq, Eq)]
enum Filter {
	All,
	Unprocessed,
	Group(String),
}

/// One editable group: name (the match-rule key), member tile indices, whether it
/// came from `tiles.variants.json` (`real`), and whether the user changed its
/// membership (`modified`). Only real-or-modified non-empty groups are written
/// back as variant groups; the rest rely on the engine's id-family fallback.
#[derive(Clone)]
struct Group {
	name: String,
	tiles: Vec<u16>,
	real: bool,
	modified: bool,
}

/// The mutable state of one pack - cloned for Reset, snapshotted on Save.
#[derive(Clone)]
struct Snapshot {
	ids: Vec<String>,
	pass: Vec<u8>,
	groups: Vec<Group>,
	/// Tile bin index → index into `groups`.
	tile_group: Vec<usize>,
	/// Group name → the four ring-indexed (N,E,S,W) entry lists.
	matches: HashMap<String, [Vec<String>; 4]>,
}

/// The working copy for one pack (the modal can switch between the project's packs
/// that carry match rules without touching the project until Save).
struct PackData {
	pack: usize,
	name: String,
	tile_count: u16,
	/// Working state (edited); `orig` is the on-disk baseline (rename source +
	/// Reset target).
	cur: Snapshot,
	orig: Snapshot,
	main_tile: u16,
	cand_tile: u16,
	main_scroll: f32,
	cand_scroll: f32,
	main_filter: Filter,
	cand_filter: Filter,
}

/// The commit for one dirty pack, handed to the shell to apply + save.
pub struct PackCommit {
	pub pack: usize,
	pub groups: Vec<(String, Vec<u16>)>,
	pub matches: HashMap<String, MatchRule>,
	/// `(old_id, new_id)` per staged rename (drives the map/template cascade).
	pub renames: Vec<(String, String)>,
	pub pass: Vec<u8>,
	pub pass_changed: bool,
}

/// Which dropdown is open (only one at a time).
#[derive(Clone, PartialEq)]
enum OpenSel {
	Pack,
	Size,
	MainFilter,
	CandFilter,
	Assign,
}

/// A focusable text field.
#[derive(Clone, Copy, PartialEq)]
enum Focus {
	Id,
	GroupName,
}

/// Which scroll region a drag is moving.
#[derive(Clone, Copy, PartialEq)]
enum ScrollId {
	Main,
	Cand,
	Groups,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArmedBtn {
	Close,
	Save,
	Reset,
}

/// What a press resolved to.
#[derive(Debug, PartialEq)]
pub enum Press {
	Consumed,
	Close,
	Save,
	Reset,
}

pub struct MatchEditor {
	packs: Vec<PackData>,
	active: usize,
	cand_xform: Transform,
	/// Cross tile enlargement 1..=6 → cell = 16*size px.
	cross_size: u8,
	open: Option<OpenSel>,
	focus: Option<Focus>,
	id_field: TextInput,
	group_name_field: TextInput,
	/// Selected group in the groups panel.
	group_sel: usize,
	groups_scroll: f32,
	scroll_drag: Option<ScrollId>,
	/// A press is dragging inside the id field (extends its selection).
	text_drag: bool,
	resizing: bool,
	resize_anchor: (f32, f32),
	armed: Option<ArmedBtn>,
	size: (f32, f32),
	pub(crate) drag_offset: (f32, f32),
}

impl Snapshot {
	fn build(project: &Project, pack_idx: usize) -> Self {
		let pack = &project.packs[pack_idx];
		let count = pack.tile_count();
		let mut groups: Vec<Group> = Vec::new();
		let mut by_name: HashMap<String, usize> = HashMap::new();
		let mut tile_group = vec![0usize; count as usize];
		for t in 0..count {
			let name = pack.group_of(t).to_string();
			let gi = *by_name.entry(name.clone()).or_insert_with(|| {
				let real = pack.variant_named.contains_key(&name);
				groups.push(Group { name: name.clone(), tiles: Vec::new(), real, modified: false });
				groups.len() - 1
			});
			groups[gi].tiles.push(t);
			tile_group[t as usize] = gi;
		}
		let pass = (0..count).map(|t| pack.pass.as_ref().map_or(0, |p| p[t as usize])).collect();
		let matches = pack.matches.iter().map(|(name, rule)| (name.clone(), rule.dirs.clone())).collect();
		Self { ids: pack.ids.clone(), pass, groups, tile_group, matches }
	}
}

impl PackData {
	fn from_pack(project: &Project, pack_idx: usize) -> Self {
		let snap = Snapshot::build(project, pack_idx);
		Self {
			pack: pack_idx,
			name: project.packs[pack_idx].name.clone(),
			tile_count: project.packs[pack_idx].tile_count(),
			cur: snap.clone(),
			orig: snap,
			main_tile: 0,
			cand_tile: 0,
			main_scroll: 0.0,
			cand_scroll: 0.0,
			main_filter: Filter::All,
			cand_filter: Filter::All,
		}
	}

	fn dirty(&self) -> bool {
		// Cheap-ish structural compare against the baseline.
		self.cur.ids != self.orig.ids
			|| self.cur.pass != self.orig.pass
			|| self.cur.matches != self.orig.matches
			|| self.cur.tile_group != self.orig.tile_group
			|| self.cur.groups.iter().map(|g| (&g.name, &g.tiles)).ne(self
				.orig
				.groups
				.iter()
				.map(|g| (&g.name, &g.tiles)))
	}

	fn group_idx(&self, tile: u16) -> usize {
		self.cur.tile_group[tile as usize]
	}

	fn group_name(&self, tile: u16) -> &str {
		&self.cur.groups[self.group_idx(tile)].name
	}

	/// Does this group have any adjacency rule? (else "unprocessed").
	fn has_rule(&self, group: &str) -> bool {
		self.cur.matches.get(group).is_some_and(|d| d.iter().any(|l| !l.is_empty()))
	}

	fn dir(&self, group: &str, ring_dir: usize) -> &[String] {
		self.cur.matches.get(group).map(|d| d[ring_dir].as_slice()).unwrap_or(&[])
	}

	fn match_present(&self, main: u16, cand: u16, screen_dir: usize, cand_xform: Transform) -> bool {
		let mg = self.group_name(main);
		let cg = self.group_name(cand);
		self.dir(mg, screen_dir).contains(&format!("{cg}{}", cand_xform.suffix()))
	}

	/// Toggle the match on `screen_dir`, keeping the reciprocal rule in sync.
	fn toggle_match(&mut self, screen_dir: usize, cand_xform: Transform) {
		let mg = self.group_name(self.main_tile).to_string();
		let cg = self.group_name(self.cand_tile).to_string();
		let fwd = format!("{cg}{}", cand_xform.suffix());
		let rev = format!("{mg}{}", cand_xform.inverse().suffix());
		let rev_dir = cand_xform.screen_to_base((screen_dir + 2) % 4);
		let present = self.dir(&mg, screen_dir).contains(&fwd);
		if present {
			if let Some(d) = self.cur.matches.get_mut(&mg) {
				d[screen_dir].retain(|e| e != &fwd);
			}
			if let Some(d) = self.cur.matches.get_mut(&cg) {
				d[rev_dir].retain(|e| e != &rev);
			}
		} else {
			let d = self.cur.matches.entry(mg).or_default();
			if !d[screen_dir].contains(&fwd) {
				d[screen_dir].push(fwd);
			}
			let d = self.cur.matches.entry(cg).or_default();
			if !d[rev_dir].contains(&rev) {
				d[rev_dir].push(rev);
			}
		}
	}

	/// The main group's wildcard on `ring_dir`: `Some(true)`=water, `Some(false)`=
	/// land, `None`=neither.
	fn wildcard(&self, ring_dir: usize) -> Option<bool> {
		let d = self.dir(self.group_name(self.main_tile), ring_dir);
		if d.iter().any(|e| e == "__WATER__") {
			Some(true)
		} else if d.iter().any(|e| e == "__LAND__") {
			Some(false)
		} else {
			None
		}
	}

	/// Cycle the main group's wildcard: none → water → land → none.
	fn cycle_wildcard(&mut self, ring_dir: usize) {
		let cur = self.wildcard(ring_dir);
		let mg = self.group_name(self.main_tile).to_string();
		let d = self.cur.matches.entry(mg).or_default();
		d[ring_dir].retain(|e| e != "__WATER__" && e != "__LAND__");
		match cur {
			None => d[ring_dir].push("__WATER__".into()),
			Some(true) => d[ring_dir].push("__LAND__".into()),
			Some(false) => {}
		}
	}

	/// Move `tile` into `target` group name, creating it if missing. `None` ⇒ its
	/// id family (i.e. ungroup to the engine's fallback).
	fn move_tile(&mut self, tile: u16, target: Option<&str>) {
		let fam = family_of(&self.cur.ids[tile as usize]).to_string();
		let name = target.map(|s| s.to_string()).unwrap_or(fam);
		let from = self.group_idx(tile);
		if self.cur.groups[from].name == name {
			return;
		}
		self.cur.groups[from].tiles.retain(|&t| t != tile);
		self.cur.groups[from].modified = true;
		let to = match self.cur.groups.iter().position(|g| g.name == name) {
			Some(i) => i,
			None => {
				self.cur.groups.push(Group { name, tiles: Vec::new(), real: false, modified: true });
				self.cur.groups.len() - 1
			}
		};
		self.cur.groups[to].tiles.push(tile);
		self.cur.groups[to].modified = true;
		self.cur.tile_group[tile as usize] = to;
	}

	/// Create a new empty group; returns its index (existing one if the name is
	/// taken).
	fn add_group(&mut self, name: &str) -> usize {
		if let Some(i) = self.cur.groups.iter().position(|g| g.name == name) {
			return i;
		}
		self.cur.groups.push(Group { name: name.to_string(), tiles: Vec::new(), real: true, modified: true });
		self.cur.groups.len() - 1
	}

	fn rename_group(&mut self, idx: usize, new: &str) {
		if new.is_empty() || self.cur.groups.iter().any(|g| g.name == new) {
			return;
		}
		let old = std::mem::replace(&mut self.cur.groups[idx].name, new.to_string());
		self.cur.groups[idx].modified = true;
		self.cur.groups[idx].real = true;
		if let Some(rule) = self.cur.matches.remove(&old) {
			self.cur.matches.insert(new.to_string(), rule);
		}
	}

	/// Dissolve a group: its tiles fall back to their id families.
	fn delete_group(&mut self, idx: usize) {
		let tiles = std::mem::take(&mut self.cur.groups[idx].tiles);
		let name = self.cur.groups[idx].name.clone();
		self.cur.matches.remove(&name);
		for t in tiles {
			self.move_tile(t, None);
		}
	}

	fn effective_id(&self, tile: u16) -> &str {
		&self.cur.ids[tile as usize]
	}

	/// Stage a tile-id rename; rejects a colliding id.
	fn set_id(&mut self, tile: u16, new: &str) -> bool {
		if new.is_empty() || self.cur.ids.iter().enumerate().any(|(i, s)| i != tile as usize && s == new) {
			return false;
		}
		self.cur.ids[tile as usize] = new.to_string();
		true
	}

	fn set_pass(&mut self, tile: u16, pass: u8) {
		self.cur.pass[tile as usize] = pass;
	}

	fn reset(&mut self) {
		self.cur = self.orig.clone();
	}

	fn snapshot_saved(&mut self) {
		self.orig = self.cur.clone();
	}

	/// Sorted group indices (stable display order).
	/// The explicit (variant) groups, non-empty, sorted by name. Family-fallback
	/// buckets (`real == false`) are NOT listed here - their tiles live in the
	/// `[ungrouped]` section (see [`Self::ungrouped_tiles`]).
	fn real_groups(&self) -> Vec<usize> {
		let mut v: Vec<usize> =
			(0..self.cur.groups.len()).filter(|&i| self.cur.groups[i].real && !self.cur.groups[i].tiles.is_empty()).collect();
		v.sort_by(|&a, &b| self.cur.groups[a].name.cmp(&self.cur.groups[b].name));
		v
	}

	/// Tiles that belong to no explicit variant group (the engine resolves them
	/// by id family). Listed at the top of each list under `[ungrouped]`.
	fn ungrouped_tiles(&self) -> Vec<u16> {
		let mut v: Vec<u16> = (0..self.tile_count).filter(|&t| !self.cur.groups[self.group_idx(t)].real).collect();
		v.sort_unstable();
		v
	}

	fn commit(&self) -> PackCommit {
		let groups: Vec<(String, Vec<u16>)> = self
			.cur
			.groups
			.iter()
			.filter(|g| !g.tiles.is_empty() && (g.real || g.modified))
			.map(|g| (g.name.clone(), g.tiles.clone()))
			.collect();
		let live: HashSet<&str> =
			self.cur.groups.iter().filter(|g| !g.tiles.is_empty()).map(|g| g.name.as_str()).collect();
		let matches: HashMap<String, MatchRule> = self
			.cur
			.matches
			.iter()
			.filter(|(name, dirs)| live.contains(name.as_str()) && dirs.iter().any(|d| !d.is_empty()))
			.map(|(name, dirs)| (name.clone(), MatchRule { dirs: dirs.clone() }))
			.collect();
		let renames: Vec<(String, String)> = (0..self.tile_count as usize)
			.filter(|&i| self.cur.ids[i] != self.orig.ids[i])
			.map(|i| (self.orig.ids[i].clone(), self.cur.ids[i].clone()))
			.collect();
		PackCommit {
			pack: self.pack,
			groups,
			matches,
			renames,
			pass: self.cur.pass.clone(),
			pass_changed: self.cur.pass != self.orig.pass,
		}
	}
}

/// A list row: the `[ungrouped]` header, a group header, or a member tile.
#[derive(Clone, Copy)]
enum Row {
	Ungrouped,
	Group(usize),
	Tile(u16),
}

impl MatchEditor {
	pub fn new(project: &Project, preferred: Option<usize>) -> Option<Self> {
		let with_rules: Vec<usize> =
			(0..project.packs.len()).filter(|&i| !project.packs[i].matches.is_empty()).collect();
		if with_rules.is_empty() {
			return None;
		}
		let packs: Vec<PackData> = with_rules.iter().map(|&i| PackData::from_pack(project, i)).collect();
		let active = preferred
			.and_then(|p| packs.iter().position(|pd| pd.pack == p))
			.or_else(|| packs.iter().position(|pd| pd.pack != 0))
			.unwrap_or(0);
		let id0 = packs[active].effective_id(0).to_string();
		Some(Self {
			packs,
			active,
			cand_xform: Transform::default(),
			cross_size: 3,
			open: None,
			focus: None,
			id_field: TextInput::new(&id0, 12).charset(Charset::Identifier),
			group_name_field: TextInput::new("", 12).charset(Charset::Identifier),
			group_sel: 0,
			groups_scroll: 0.0,
			scroll_drag: None,
			text_drag: false,
			resizing: false,
			resize_anchor: (0.0, 0.0),
			armed: None,
			size: (DEF_W, DEF_H),
			drag_offset: (0.0, 0.0),
		})
	}

	fn pd(&self) -> &PackData {
		&self.packs[self.active]
	}

	fn pd_mut(&mut self) -> &mut PackData {
		&mut self.packs[self.active]
	}

	pub fn dirty(&self) -> bool {
		self.packs.iter().any(|pd| pd.dirty())
	}

	pub fn commits(&self) -> Vec<PackCommit> {
		self.packs.iter().filter(|pd| pd.dirty()).map(|pd| pd.commit()).collect()
	}

	pub fn mark_saved(&mut self) {
		for pd in &mut self.packs {
			pd.snapshot_saved();
		}
	}

	/// Reload the id field to the selected tile's effective id (after a selection
	/// change / reset).
	fn sync_id_field(&mut self) {
		let id = self.pd().effective_id(self.pd().main_tile).to_string();
		self.id_field.set_text(&id);
	}

	// ----- geometry ----------------------------------------------------------

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		Rect::centered(w, h, self.size.0, self.size.1).translate(self.drag_offset.0, self.drag_offset.1)
	}

	fn body_top(d: Rect) -> f32 {
		d.y + TITLE_H + 6.0
	}

	fn pack_sel_rect(d: Rect) -> Rect {
		Rect::new(d.x + PAD + 36.0, Self::body_top(d), 170.0, SEL_H)
	}

	fn size_sel_rect(d: Rect) -> Rect {
		Rect::new(d.x + d.w - PAD - 70.0, Self::body_top(d), 70.0, SEL_H)
	}

	fn lists_top(d: Rect) -> f32 {
		Self::body_top(d) + SEL_H + 6.0
	}

	/// Bottom of the list area (above the lower panel band).
	fn lists_bottom(d: Rect) -> f32 {
		Self::lower_y(d) - 8.0
	}

	fn filter_rect(d: Rect, left: bool) -> Rect {
		let x = if left { d.x + PAD } else { d.x + d.w - PAD - LIST_W };
		Rect::new(x, Self::lists_top(d), LIST_W, SEL_H)
	}

	fn well(d: Rect, left: bool) -> Rect {
		let x = if left { d.x + PAD } else { d.x + d.w - PAD - LIST_W };
		let top = Self::lists_top(d) + SEL_H + 4.0;
		Rect::new(x, top, LIST_W, Self::lists_bottom(d) - top)
	}

	fn center_x(d: Rect) -> f32 {
		(d.x + PAD + LIST_W + (d.x + d.w - PAD - LIST_W)) / 2.0
	}

	fn cell_px(&self) -> f32 {
		16.0 * self.cross_size as f32
	}

	fn cross_origin(&self, d: Rect) -> (f32, f32) {
		let c = self.cell_px();
		(Self::center_x(d) - 1.5 * c, Self::lists_top(d) + 22.0)
	}

	fn cross_cell(&self, d: Rect, row: usize, col: usize) -> Rect {
		let (ox, oy) = self.cross_origin(d);
		let c = self.cell_px();
		Rect::new(ox + col as f32 * c, oy + row as f32 * c, c, c)
	}

	fn center_rect(&self, d: Rect) -> Rect {
		self.cross_cell(d, 1, 1)
	}

	fn side_rect(&self, d: Rect, screen_dir: usize) -> Rect {
		match screen_dir {
			0 => self.cross_cell(d, 0, 1),
			1 => self.cross_cell(d, 1, 2),
			2 => self.cross_cell(d, 2, 1),
			_ => self.cross_cell(d, 1, 0),
		}
	}

	/// Orientation-picker origin (below the cross).
	fn picker_top(&self, d: Rect) -> f32 {
		self.cross_origin(d).1 + 3.0 * self.cell_px() + 22.0
	}

	const MINI: f32 = 16.0;

	/// The k-th orientation preview box (2 rows × 4).
	fn orient_rect(&self, d: Rect, k: usize) -> Rect {
		let cell = Self::MINI * 3.0;
		let pitch = cell + 10.0;
		let row = k / 4;
		let col = k % 4;
		let total = 4.0 * pitch - 10.0;
		let x0 = Self::center_x(d) - total / 2.0;
		Rect::new(x0 + col as f32 * pitch, self.picker_top(d) + 12.0 + row as f32 * (pitch + 4.0), cell, cell)
	}

	/// Lower panel band (per-tile + groups), anchored above the buttons.
	fn lower_h() -> f32 {
		140.0
	}

	fn lower_y(d: Rect) -> f32 {
		d.y + d.h - BTN_H - 12.0 - 10.0 - Self::lower_h()
	}

	/// Per-tile panel (left half of the lower band).
	fn id_field_rect(d: Rect) -> Rect {
		Rect::new(d.x + PAD + 24.0, Self::lower_y(d) + 16.0, 150.0, SEL_H)
	}

	fn id_apply_rect(d: Rect) -> Rect {
		let f = Self::id_field_rect(d);
		Rect::new(f.x + f.w + 6.0, f.y, 44.0, SEL_H)
	}

	fn pass_rect(d: Rect, i: usize) -> Rect {
		Rect::new(d.x + PAD + 24.0 + i as f32 * 62.0, Self::lower_y(d) + 16.0 + SEL_H + 22.0, 60.0, BTN_H)
	}

	/// Groups panel (right half of the lower band).
	fn groups_x(d: Rect) -> f32 {
		d.x + d.w / 2.0 + 8.0
	}

	/// The per-tile group select - in the LEFT (per-tile) panel, below the pass
	/// buttons; it shows the selected main tile's group ([none] = ungrouped).
	fn assign_rect(d: Rect) -> Rect {
		Rect::new(d.x + PAD + 44.0, Self::pass_rect(d, 0).y + BTN_H + 18.0, 150.0, SEL_H)
	}

	fn groups_well(d: Rect) -> Rect {
		let y = Self::lower_y(d) + 16.0 + SEL_H + 6.0;
		Rect::new(Self::groups_x(d), y, 150.0, Self::lower_y(d) + Self::lower_h() - y)
	}

	fn group_name_rect(d: Rect) -> Rect {
		let w = Self::groups_well(d);
		Rect::new(w.x + w.w + 8.0, w.y, 100.0, SEL_H)
	}

	fn group_btn_rect(d: Rect, i: usize) -> Rect {
		let n = Self::group_name_rect(d);
		Rect::new(n.x, n.y + (SEL_H + 4.0) * (i as f32 + 1.0), 100.0, BTN_H)
	}

	fn handle_rect(d: Rect) -> Rect {
		Rect::new(d.x + d.w - HANDLE, d.y + d.h - HANDLE, HANDLE, HANDLE)
	}

	fn close_rect(d: Rect) -> Rect {
		Rect::new(d.x + PAD, d.y + d.h - BTN_H - 12.0, 80.0, BTN_H)
	}

	fn reset_rect(d: Rect) -> Rect {
		Rect::new(d.x + d.w / 2.0 - 40.0, d.y + d.h - BTN_H - 12.0, 80.0, BTN_H)
	}

	fn save_rect(d: Rect) -> Rect {
		Rect::new(d.x + d.w - PAD - 80.0, d.y + d.h - BTN_H - 12.0, 80.0, BTN_H)
	}

	// ----- row model ---------------------------------------------------------

	fn rows(&self, filter: &Filter) -> Vec<Row> {
		let pd = self.pd();
		let mut out = Vec::new();
		// `[ungrouped]` bucket at the very top (only the broad filters show it).
		if matches!(filter, Filter::All | Filter::Unprocessed) {
			let ung: Vec<u16> = pd
				.ungrouped_tiles()
				.into_iter()
				.filter(|&t| !matches!(filter, Filter::Unprocessed) || !pd.has_rule(pd.group_name(t)))
				.collect();
			if !ung.is_empty() {
				out.push(Row::Ungrouped);
				out.extend(ung.into_iter().map(Row::Tile));
			}
		}
		// Then the explicit (variant) groups.
		for gi in pd.real_groups() {
			let g = &pd.cur.groups[gi];
			let keep = match filter {
				Filter::All => true,
				Filter::Unprocessed => !pd.has_rule(&g.name),
				Filter::Group(name) => &g.name == name,
			};
			if !keep {
				continue;
			}
			out.push(Row::Group(gi));
			let mut tiles = g.tiles.clone();
			tiles.sort_unstable();
			out.extend(tiles.into_iter().map(Row::Tile));
		}
		out
	}

	fn filter_labels(&self) -> Vec<String> {
		let mut v = vec!["all".to_string(), "[unprocessed]".to_string()];
		for gi in self.pd().real_groups() {
			v.push(self.pd().cur.groups[gi].name.clone());
		}
		v
	}

	fn filter_of(&self, idx: usize) -> Filter {
		match idx {
			0 => Filter::All,
			1 => Filter::Unprocessed,
			_ => {
				let gi = self.pd().real_groups();
				gi.get(idx - 2).map(|&i| Filter::Group(self.pd().cur.groups[i].name.clone())).unwrap_or(Filter::All)
			}
		}
	}

	fn filter_index(&self, f: &Filter) -> usize {
		match f {
			Filter::All => 0,
			Filter::Unprocessed => 1,
			Filter::Group(name) => self
				.pd()
				.real_groups()
				.iter()
				.position(|&i| &self.pd().cur.groups[i].name == name)
				.map(|p| p + 2)
				.unwrap_or(0),
		}
	}

	/// The group-select labels + the current value index for the selected tile.
	fn assign_labels(&self) -> Vec<String> {
		let mut v = vec!["[none]".to_string()];
		for gi in self.pd().real_groups() {
			v.push(self.pd().cur.groups[gi].name.clone());
		}
		v
	}

	fn assign_index(&self) -> usize {
		let pd = self.pd();
		let gi = pd.group_idx(pd.main_tile);
		if !pd.cur.groups[gi].real {
			return 0; // [none] = ungrouped (id-family fallback)
		}
		pd.real_groups().iter().position(|&i| i == gi).map(|p| p + 1).unwrap_or(0)
	}

	// ----- selects -----------------------------------------------------------

	fn toggle_open(&mut self, which: OpenSel) {
		self.open = if self.open.as_ref() == Some(&which) { None } else { Some(which) };
	}

	// ----- scroll ------------------------------------------------------------

	fn content_h(&self, filter: &Filter) -> f32 {
		self.rows(filter).len() as f32 * ROW_H
	}

	/// Scroll offset that puts the thumb under cursor `cy` (track click / drag).
	fn scroll_for_cursor(well: Rect, content: f32, cy: f32) -> f32 {
		let max = ui::scroll_max(content, well.h);
		if max <= 0.0 {
			return 0.0;
		}
		let thumb_h = (well.h * (well.h / content)).clamp(16.0_f32.min(well.h), well.h);
		let t = ((cy - well.y - thumb_h / 2.0) / (well.h - thumb_h)).clamp(0.0, 1.0);
		t * max
	}

	fn track_rect(well: Rect) -> Rect {
		Rect::new(well.x + well.w - ui::SCROLLBAR_W, well.y, ui::SCROLLBAR_W, well.h)
	}

	// ----- events ------------------------------------------------------------

	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		// Resize handle.
		if Self::handle_rect(d).contains(x, y) {
			self.resizing = true;
			self.resize_anchor = (x - self.size.0, y - self.size.1);
			return Press::Consumed;
		}
		// Open popup: route clicks to it first.
		if self.press_open_popup(d, x, y) {
			return Press::Consumed;
		}
		// Closed select boxes.
		if Self::pack_sel_rect(d).contains(x, y) {
			self.toggle_open(OpenSel::Pack);
			return Press::Consumed;
		}
		if Self::size_sel_rect(d).contains(x, y) {
			self.toggle_open(OpenSel::Size);
			return Press::Consumed;
		}
		if Self::filter_rect(d, true).contains(x, y) {
			self.toggle_open(OpenSel::MainFilter);
			return Press::Consumed;
		}
		if Self::filter_rect(d, false).contains(x, y) {
			self.toggle_open(OpenSel::CandFilter);
			return Press::Consumed;
		}
		if Self::assign_rect(d).contains(x, y) {
			self.toggle_open(OpenSel::Assign);
			return Press::Consumed;
		}
		self.open = None;
		// Scroll tracks.
		for (id, well, content) in self.scroll_regions(d) {
			if Self::track_rect(well).contains(x, y) {
				self.scroll_drag = Some(id);
				let s = Self::scroll_for_cursor(well, content, y);
				self.set_scroll(id, s);
				return Press::Consumed;
			}
		}
		// List rows.
		if let Some(row) = self.row_at(d, true, x, y) {
			self.select_row(true, row);
			return Press::Consumed;
		}
		if let Some(row) = self.row_at(d, false, x, y) {
			self.select_row(false, row);
			return Press::Consumed;
		}
		// Groups-panel rows.
		if let Some(gi) = self.groups_row_at(d, x, y) {
			self.group_sel = gi;
			let name = self.pd().cur.groups[gi].name.clone();
			self.group_name_field.set_text(&name);
			return Press::Consumed;
		}
		// Cross sides: LMB toggles the match (unless wildcarded).
		for dir in 0..4 {
			if self.side_rect(d, dir).contains(x, y) {
				if self.pd().wildcard(dir).is_none() {
					let cx = self.cand_xform;
					self.pd_mut().toggle_match(dir, cx);
				}
				return Press::Consumed;
			}
		}
		// Orientation previews.
		for k in 0..8 {
			if self.orient_rect(d, k).contains(x, y) {
				self.cand_xform = Transform { rot: (k & 3) as u8, mirror: k & 4 != 0 };
				return Press::Consumed;
			}
		}
		// Id field + apply.
		if Self::id_field_rect(d).contains(x, y) {
			self.focus = Some(Focus::Id);
			self.text_drag = true;
			let r = Self::id_field_rect(d);
			self.id_field.on_press(x, y, r);
			return Press::Consumed;
		}
		if Self::id_apply_rect(d).contains(x, y) {
			self.apply_id();
			return Press::Consumed;
		}
		// Pass buttons.
		for i in 0..4 {
			if Self::pass_rect(d, i).contains(x, y) {
				let t = self.pd().main_tile;
				self.pd_mut().set_pass(t, i as u8);
				return Press::Consumed;
			}
		}
		// Group name field + add/rename/delete.
		if Self::group_name_rect(d).contains(x, y) {
			self.focus = Some(Focus::GroupName);
			let r = Self::group_name_rect(d);
			self.group_name_field.on_press(x, y, r);
			return Press::Consumed;
		}
		if Self::group_btn_rect(d, 0).contains(x, y) {
			self.group_add();
			return Press::Consumed;
		}
		if Self::group_btn_rect(d, 1).contains(x, y) {
			self.group_rename();
			return Press::Consumed;
		}
		if Self::group_btn_rect(d, 2).contains(x, y) {
			self.group_delete();
			return Press::Consumed;
		}
		// Bottom buttons.
		self.focus = None;
		if Self::close_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Close);
		} else if Self::reset_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Reset);
		} else if Self::save_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Save);
		}
		Press::Consumed
	}

	/// Right-press cycles a cross side's edge type (tile→water→land→tile).
	pub fn on_right_press(&mut self, x: f32, y: f32, w: f32, h: f32) {
		let d = self.dialog_rect(w, h);
		for dir in 0..4 {
			if self.side_rect(d, dir).contains(x, y) {
				self.pd_mut().cycle_wildcard(dir);
				return;
			}
		}
	}

	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		self.scroll_drag = None;
		self.text_drag = false;
		self.resizing = false;
		let d = self.dialog_rect(w, h);
		match self.armed.take() {
			Some(ArmedBtn::Close) if Self::close_rect(d).contains(x, y) => Press::Close,
			Some(ArmedBtn::Reset) if Self::reset_rect(d).contains(x, y) && self.dirty() => Press::Reset,
			Some(ArmedBtn::Save) if Self::save_rect(d).contains(x, y) && self.dirty() => Press::Save,
			_ => Press::Consumed,
		}
	}

	pub fn on_drag(&mut self, x: f32, y: f32, w: f32, h: f32) {
		if self.resizing {
			self.size.0 = (x - self.resize_anchor.0).clamp(MIN_W, w.max(MIN_W));
			self.size.1 = (y - self.resize_anchor.1).clamp(MIN_H, h.max(MIN_H));
			return;
		}
		if let Some(id) = self.scroll_drag {
			let d = self.dialog_rect(w, h);
			if let Some((_, well, content)) = self.scroll_regions(d).into_iter().find(|(i, _, _)| *i == id) {
				let s = Self::scroll_for_cursor(well, content, y);
				self.set_scroll(id, s);
			}
			return;
		}
		if self.text_drag && self.focus == Some(Focus::Id) {
			let d = self.dialog_rect(w, h);
			let r = Self::id_field_rect(d);
			self.id_field.on_drag(x, y, r);
		}
	}

	pub fn on_wheel_at(&mut self, steps: f32, x: f32, y: f32, w: f32, h: f32) {
		let d = self.dialog_rect(w, h);
		for (id, well, content) in self.scroll_regions(d) {
			if well.contains(x, y) {
				let max = ui::scroll_max(content, well.h);
				let s = (self.scroll_of(id) - steps * ROW_H * 2.0).clamp(0.0, max);
				self.set_scroll(id, s);
				return;
			}
		}
	}

	pub fn on_key(&mut self, key: &crate::modal::ModalKey) {
		match self.focus {
			Some(Focus::Id) => {
				if matches!(key, crate::modal::ModalKey::Enter) {
					self.apply_id();
				} else {
					self.id_field.on_key(key);
				}
			}
			Some(Focus::GroupName) => {
				self.group_name_field.on_key(key);
			}
			None => {}
		}
	}

	// ----- press helpers -----------------------------------------------------

	/// The three scroll regions with each one's own content height (so the groups
	/// panel clamps to the group count, not the tile-list rows).
	fn scroll_regions(&self, d: Rect) -> Vec<(ScrollId, Rect, f32)> {
		vec![
			(ScrollId::Main, Self::well(d, true), self.content_h(&self.pd().main_filter)),
			(ScrollId::Cand, Self::well(d, false), self.content_h(&self.pd().cand_filter)),
			(ScrollId::Groups, Self::groups_well(d), self.pd().real_groups().len() as f32 * ROW_H),
		]
	}

	fn scroll_of(&self, id: ScrollId) -> f32 {
		match id {
			ScrollId::Main => self.pd().main_scroll,
			ScrollId::Cand => self.pd().cand_scroll,
			ScrollId::Groups => self.groups_scroll,
		}
	}

	fn set_scroll(&mut self, id: ScrollId, s: f32) {
		match id {
			ScrollId::Main => self.pd_mut().main_scroll = s,
			ScrollId::Cand => self.pd_mut().cand_scroll = s,
			ScrollId::Groups => self.groups_scroll = s,
		}
	}

	fn row_at(&self, d: Rect, left: bool, x: f32, y: f32) -> Option<Row> {
		let well = Self::well(d, left);
		if !well.contains(x, y) {
			return None;
		}
		let (scroll, filter) = if left {
			(self.pd().main_scroll, &self.pd().main_filter)
		} else {
			(self.pd().cand_scroll, &self.pd().cand_filter)
		};
		let rows = self.rows(filter);
		let i = ((y - well.y + scroll) / ROW_H).floor() as i64;
		(i >= 0 && (i as usize) < rows.len()).then(|| rows[i as usize])
	}

	fn select_row(&mut self, left: bool, row: Row) {
		let tile = match row {
			Row::Tile(t) => t,
			Row::Group(gi) => *self.pd().cur.groups[gi].tiles.iter().min().unwrap_or(&0),
			Row::Ungrouped => self.pd().ungrouped_tiles().first().copied().unwrap_or(0),
		};
		if left {
			self.pd_mut().main_tile = tile;
			self.sync_id_field();
		} else {
			self.pd_mut().cand_tile = tile;
		}
	}

	fn groups_row_at(&self, d: Rect, x: f32, y: f32) -> Option<usize> {
		let well = Self::groups_well(d);
		if !well.contains(x, y) {
			return None;
		}
		let groups = self.pd().real_groups();
		let i = ((y - well.y + self.groups_scroll) / ROW_H).floor() as i64;
		(i >= 0 && (i as usize) < groups.len()).then(|| groups[i as usize])
	}

	fn press_open_popup(&mut self, d: Rect, x: f32, y: f32) -> bool {
		let Some(open) = self.open.clone() else { return false };
		let (rect, n) = self.open_select_geom(d, &open);
		if let Some(hit) = crate::select::hit(rect, true, n, false, x, y) {
			match hit {
				crate::select::Hit::Box => self.open = None,
				crate::select::Hit::Option(i) => self.choose_option(open, i),
			}
			true
		} else {
			self.open = None;
			true
		}
	}

	fn open_select_geom(&self, d: Rect, open: &OpenSel) -> (Rect, usize) {
		match open {
			OpenSel::Pack => (Self::pack_sel_rect(d), self.packs.len()),
			OpenSel::Size => (Self::size_sel_rect(d), 6),
			OpenSel::MainFilter => (Self::filter_rect(d, true), self.filter_labels().len()),
			OpenSel::CandFilter => (Self::filter_rect(d, false), self.filter_labels().len()),
			OpenSel::Assign => (Self::assign_rect(d), self.assign_labels().len()),
		}
	}

	fn choose_option(&mut self, open: OpenSel, i: usize) {
		match open {
			OpenSel::Pack => {
				self.active = i.min(self.packs.len() - 1);
				self.group_sel = 0;
				self.sync_id_field();
			}
			OpenSel::Size => self.cross_size = (i as u8 + 1).clamp(1, 6),
			OpenSel::MainFilter => {
				let f = self.filter_of(i);
				self.pd_mut().main_filter = f;
				self.pd_mut().main_scroll = 0.0;
			}
			OpenSel::CandFilter => {
				let f = self.filter_of(i);
				self.pd_mut().cand_filter = f;
				self.pd_mut().cand_scroll = 0.0;
			}
			OpenSel::Assign => {
				let labels = self.assign_labels();
				let target = (i > 0).then(|| labels[i].clone());
				let t = self.pd().main_tile;
				self.pd_mut().move_tile(t, target.as_deref());
			}
		}
		self.open = None;
	}

	fn apply_id(&mut self) {
		let new = self.id_field.text().to_string();
		let t = self.pd().main_tile;
		if !self.pd_mut().set_id(t, &new) {
			self.sync_id_field(); // reject: restore
		}
		self.focus = None;
	}

	fn group_add(&mut self) {
		let name = self.group_name_field.text().trim().to_string();
		if name.is_empty() {
			return;
		}
		self.group_sel = self.pd_mut().add_group(&name);
		self.groups_scroll = 0.0;
	}

	fn group_rename(&mut self) {
		let new = self.group_name_field.text().trim().to_string();
		let gi = self.group_sel;
		if gi < self.pd().cur.groups.len() {
			self.pd_mut().rename_group(gi, &new);
		}
	}

	fn group_delete(&mut self) {
		let gi = self.group_sel;
		if gi < self.pd().cur.groups.len() {
			self.pd_mut().delete_group(gi);
		}
		self.group_sel = 0;
		self.groups_scroll = 0.0;
	}

	pub fn reset(&mut self) {
		self.pd_mut().reset();
		self.sync_id_field();
	}

	// ----- drawing -----------------------------------------------------------

	fn row_color(&self, left: bool, row: Row) -> [f32; 4] {
		let pd = self.pd();
		let sel = if left { pd.main_tile } else { pd.cand_tile };
		match row {
			// Green when the selection is itself ungrouped; else the "needs data" tone.
			Row::Ungrouped => {
				if !pd.cur.groups[pd.group_idx(sel)].real {
					GREEN
				} else {
					ORANGE
				}
			}
			Row::Group(gi) => {
				if pd.group_idx(sel) == gi {
					GREEN
				} else if !pd.has_rule(&pd.cur.groups[gi].name) {
					ORANGE
				} else {
					YELLOW
				}
			}
			Row::Tile(t) => {
				if t == sel {
					GREEN
				} else if !pd.has_rule(pd.group_name(t)) {
					ORANGE
				} else {
					theme::INK
				}
			}
		}
	}

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let pd = self.pd();
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		let title = if self.dirty() { "Edit Tile Match Data  *" } else { "Edit Tile Match Data" };
		ui::modal_frame(&mut q, d, title, TITLE_H, w, h);

		// Top row: pack + cross size.
		q.label("pack", d.x + PAD, Self::body_top(d) + 4.0, ui::FONT_SMALL, w, h, theme::INK_DIM);
		crate::select::draw_box(&mut q, Self::pack_sel_rect(d), &pd.name, self.open == Some(OpenSel::Pack), w, h, hot);
		crate::select::draw_box(
			&mut q,
			Self::size_sel_rect(d),
			&format!("x{}", self.cross_size),
			self.open == Some(OpenSel::Size),
			w,
			h,
			hot,
		);

		// Lists with filters.
		for (left, label, scroll, filter, sel) in [
			(true, "main", pd.main_scroll, &pd.main_filter, pd.main_tile),
			(false, "candidate", pd.cand_scroll, &pd.cand_filter, pd.cand_tile),
		] {
			let fr = Self::filter_rect(d, left);
			crate::select::draw_box(
				&mut q,
				fr,
				&filter_label(filter),
				self.open == Some(if left { OpenSel::MainFilter } else { OpenSel::CandFilter }),
				w,
				h,
				hot,
			);
			let well = Self::well(d, left);
			q.field(well, w, h);
			q.border(well, w, h, theme::PANEL_BORDER);
			q.scrollbar(well, self.content_h(filter), scroll, w, h, hot);
			let _ = (label, sel);
		}

		// Cross (borderless, touching). Centre + 4 sides; corners empty.
		let center = self.center_rect(d);
		q.field(center, w, h);
		for dir in 0..4 {
			let r = self.side_rect(d, dir);
			match pd.wildcard(dir) {
				Some(water) => {
					q.rect(r, w, h, if water { WATER_COL } else { LAND_COL });
					q.label(if water { "WTR" } else { "LND" }, r.x + 3.0, r.y + 3.0, ui::FONT_SMALL, w, h, theme::INK);
				}
				None => q.field(r, w, h),
			}
			// Match highlights are drawn in `overlay()` (after the tiles) so the
			// borderless tiles don't cover them.
		}
		// Group labels under the cross-block + selected ids.
		let (ox, oy) = self.cross_origin(d);
		q.label(
			&format!("main: {}  cand: {}", pd.group_name(pd.main_tile), pd.group_name(pd.cand_tile)),
			ox,
			oy - 16.0,
			ui::FONT_SMALL,
			w,
			h,
			theme::INK,
		);

		// Orientation picker (backgrounds only; the selected ring + per-side match
		// highlights are drawn in `overlay()`, above the tiles).
		q.label("orientation", Self::center_x(d) - 30.0, self.picker_top(d), ui::FONT_SMALL, w, h, theme::INK_DIM);
		for k in 0..8 {
			let r = self.orient_rect(d, k);
			q.field(r, w, h);
			if hot.hover(r) {
				q.border(r, w, h, theme::INK_DIM);
			}
		}

		// Lower band: per-tile (left), groups (right).
		q.label("id", d.x + PAD, Self::id_field_rect(d).y + 4.0, ui::FONT_SMALL, w, h, theme::INK_DIM);
		q.field(Self::id_field_rect(d), w, h);
		if self.focus == Some(Focus::Id) {
			q.border(Self::id_field_rect(d), w, h, theme::INK);
		}
		q.button(Self::id_apply_rect(d), w, h, hot);
		q.label_in("set", Self::id_apply_rect(d), 8.0, ui::FONT_SMALL, w, h, theme::INK);
		q.label("pass", d.x + PAD, Self::pass_rect(d, 0).y + 4.0, ui::FONT_SMALL, w, h, theme::INK_DIM);
		let cur_pass = pd.cur.pass[pd.main_tile as usize];
		for (i, name) in ["land", "water", "shore", "block"].iter().enumerate() {
			q.button_active(Self::pass_rect(d, i), w, h, cur_pass as usize == i, hot);
			q.label_in(name, Self::pass_rect(d, i), 6.0, ui::FONT_SMALL, w, h, theme::INK);
		}

		// Per-tile group select (left panel, below pass).
		q.label("group", d.x + PAD, Self::assign_rect(d).y + 4.0, ui::FONT_SMALL, w, h, theme::INK_DIM);
		crate::select::draw_box(
			&mut q,
			Self::assign_rect(d),
			&self.assign_labels()[self.assign_index()],
			self.open == Some(OpenSel::Assign),
			w,
			h,
			hot,
		);
		let gw = Self::groups_well(d);
		q.field(gw, w, h);
		q.border(gw, w, h, theme::PANEL_BORDER);
		q.scrollbar(gw, self.pd().real_groups().len() as f32 * ROW_H, self.groups_scroll, w, h, hot);
		q.field(Self::group_name_rect(d), w, h);
		if self.focus == Some(Focus::GroupName) {
			q.border(Self::group_name_rect(d), w, h, theme::INK);
		}
		for (i, name) in ["add", "rename", "delete"].iter().enumerate() {
			q.button(Self::group_btn_rect(d, i), w, h, hot);
			q.label_in(name, Self::group_btn_rect(d, i), 8.0, ui::FONT_SMALL, w, h, theme::INK);
		}

		// Bottom buttons + resize handle.
		q.button(Self::close_rect(d), w, h, hot);
		q.label_in("Close", Self::close_rect(d), 8.0, ui::FONT_SMALL, w, h, theme::INK_DIM);
		if self.dirty() {
			q.button(Self::reset_rect(d), w, h, hot);
		} else {
			q.button_disabled(Self::reset_rect(d), w, h);
		}
		q.label_in("Reset", Self::reset_rect(d), 8.0, ui::FONT_SMALL, w, h, theme::INK_DIM);
		if self.dirty() {
			q.button_primary(Self::save_rect(d), w, h, hot);
		} else {
			q.button_disabled(Self::save_rect(d), w, h);
		}
		q.label_in("Save", Self::save_rect(d), 8.0, ui::FONT_SMALL, w, h, theme::INK);
		let hr = Self::handle_rect(d);
		q.tri((hr.x + hr.w, hr.y), (hr.x + hr.w, hr.y + hr.h), (hr.x, hr.y + hr.h), w, h, theme::RESIZE_HANDLE);
		q
	}

	/// Highlights that must sit ON TOP of the (borderless) tile thumbnails: the
	/// cross's matched-side external borders, and the orientation picker's selected
	/// ring + per-preview match highlights. Drawn after the tile pass.
	pub fn overlay(&self, w: f32, h: f32) -> UiQuads {
		let pd = self.pd();
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		for dir in 0..4 {
			if pd.wildcard(dir).is_none() && pd.match_present(pd.main_tile, pd.cand_tile, dir, self.cand_xform) {
				external_highlight(&mut q, self.side_rect(d, dir), dir, w, h);
			}
		}
		for k in 0..8 {
			let r = self.orient_rect(d, k);
			let xf = Transform { rot: (k & 3) as u8, mirror: k & 4 != 0 };
			if self.cand_xform.bits() as usize == k {
				q.border(r, w, h, GREEN);
				q.border(inset(r, 1.0), w, h, GREEN);
			}
			let cell = |row: usize, col: usize| {
				Rect::new(r.x + col as f32 * Self::MINI, r.y + row as f32 * Self::MINI, Self::MINI, Self::MINI)
			};
			for dir in 0..4 {
				if pd.wildcard(dir).is_none() && pd.match_present(pd.main_tile, pd.cand_tile, dir, xf) {
					let cr = match dir {
						0 => cell(0, 1),
						1 => cell(1, 2),
						2 => cell(2, 1),
						_ => cell(1, 0),
					};
					external_highlight(&mut q, cr, dir, w, h);
				}
			}
		}
		q
	}

	/// Clipped content per region: each list's row backgrounds + text, the groups
	/// list, and the two text fields. Drawn through `draw_ui_clipped`.
	pub fn field_contents(&self, w: f32, h: f32) -> Vec<(UiQuads, Rect)> {
		let d = self.dialog_rect(w, h);
		let mut out = Vec::new();
		for left in [true, false] {
			let well = Self::well(d, left);
			let (scroll, filter, sel) = if left {
				(self.pd().main_scroll, &self.pd().main_filter, self.pd().main_tile)
			} else {
				(self.pd().cand_scroll, &self.pd().cand_filter, self.pd().cand_tile)
			};
			let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(well));
			for (i, row) in self.rows(filter).iter().enumerate() {
				let ry = well.y + i as f32 * ROW_H - scroll;
				if ry + ROW_H <= well.y || ry >= well.y + well.h {
					continue;
				}
				let r = Rect::new(well.x, ry, well.w - ui::SCROLLBAR_W, ROW_H);
				let is_sel = matches!(row, Row::Tile(t) if *t == sel)
					|| matches!(row, Row::Group(gi) if self.pd().group_idx(sel) == *gi)
					|| matches!(row, Row::Ungrouped if !self.pd().cur.groups[self.pd().group_idx(sel)].real);
				if is_sel {
					q.rect(r, w, h, theme::SELECTION);
				}
				let color = self.row_color(left, *row);
				let (txt, indent) = match row {
					Row::Ungrouped => ("[ungrouped]".to_string(), 0.0),
					Row::Group(gi) => (self.pd().cur.groups[*gi].name.clone(), 0.0),
					Row::Tile(t) => (self.pd().effective_id(*t).to_string(), 12.0),
				};
				let tx = r.x + indent + THUMB + 6.0;
				q.label_fit(&txt, Rect::new(tx, r.y, r.w - (tx - r.x) - 2.0, r.h), 0.0, ui::FONT_SMALL, w, h, color);
			}
			out.push((q, well));
		}
		// Groups panel list.
		let gw = Self::groups_well(d);
		let mut gq = UiQuads::with_steel_map(ui::SteelMap::anchored(gw));
		for (i, &gi) in self.pd().real_groups().iter().enumerate() {
			let ry = gw.y + i as f32 * ROW_H - self.groups_scroll;
			if ry + ROW_H <= gw.y || ry >= gw.y + gw.h {
				continue;
			}
			let r = Rect::new(gw.x, ry, gw.w - ui::SCROLLBAR_W, ROW_H);
			if gi == self.group_sel {
				gq.rect(r, w, h, theme::SELECTION);
			}
			let g = &self.pd().cur.groups[gi];
			let color = if !self.pd().has_rule(&g.name) { ORANGE } else { YELLOW };
			gq.label_fit(
				&format!("{} ({})", g.name, g.tiles.len()),
				Rect::new(r.x + 4.0, r.y, r.w - 6.0, r.h),
				0.0,
				ui::FONT_SMALL,
				w,
				h,
				color,
			);
		}
		out.push((gq, gw));
		// Text fields.
		let idr = Self::id_field_rect(d);
		out.push((self.id_field.content_quads(idr, self.focus == Some(Focus::Id), w, h), idr));
		let gnr = Self::group_name_rect(d);
		out.push((self.group_name_field.content_quads(gnr, self.focus == Some(Focus::GroupName), w, h), gnr));
		out
	}

	pub fn popup(&self, w: f32, h: f32, hot: Hot) -> Option<UiQuads> {
		let open = self.open.clone()?;
		let d = self.dialog_rect(w, h);
		let (rect, _) = self.open_select_geom(d, &open);
		let (labels, sel): (Vec<String>, usize) = match open {
			OpenSel::Pack => (self.packs.iter().map(|p| p.name.clone()).collect(), self.active),
			OpenSel::Size => ((1..=6).map(|n| format!("x{n}")).collect(), self.cross_size as usize - 1),
			OpenSel::MainFilter => (self.filter_labels(), self.filter_index(&self.pd().main_filter)),
			OpenSel::CandFilter => (self.filter_labels(), self.filter_index(&self.pd().cand_filter)),
			OpenSel::Assign => (self.assign_labels(), self.assign_index()),
		};
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		crate::select::draw_popup(&mut q, rect, &labels, Some(sel), false, w, h, hot);
		Some(q)
	}

	/// Tile thumbnails grouped by clip region (lists, cross, orientation previews,
	/// groups list). The shell draws each `(quads, scissor)` through the tile pass.
	pub fn tile_layers(&self, project: &Project, w: f32, h: f32) -> Vec<(Vec<TileQuad>, Rect)> {
		let pd = self.pd();
		let pack = pd.pack as u8;
		let d = self.dialog_rect(w, h);
		let mut layers = Vec::new();
		// Lists.
		for left in [true, false] {
			let well = Self::well(d, left);
			let (scroll, filter) =
				if left { (pd.main_scroll, &pd.main_filter) } else { (pd.cand_scroll, &pd.cand_filter) };
			let mut tiles = Vec::new();
			for (i, row) in self.rows(filter).iter().enumerate() {
				let ry = well.y + i as f32 * ROW_H - scroll;
				if ry + ROW_H <= well.y || ry >= well.y + well.h {
					continue;
				}
				let (tile, indent) = match row {
					Row::Ungrouped => (pd.ungrouped_tiles().first().copied().unwrap_or(0), 0.0),
					Row::Group(gi) => (*pd.cur.groups[*gi].tiles.iter().min().unwrap_or(&0), 0.0),
					Row::Tile(t) => (*t, 12.0),
				};
				let r = Rect::new(well.x + indent + 2.0, ry + 2.0, THUMB, THUMB);
				tiles.push(TileQuad {
					index: global_index(project, TileRef { pack, tile, transform: Transform::default() }),
					transform: 0,
					rect: r,
				});
			}
			layers.push((tiles, well));
		}
		// Cross: centre identity; non-wildcard sides show the candidate orientation.
		let mut cross = Vec::new();
		cross.push(TileQuad {
			index: global_index(project, TileRef { pack, tile: pd.main_tile, transform: Transform::default() }),
			transform: 0,
			rect: self.center_rect(d),
		});
		for dir in 0..4 {
			if pd.wildcard(dir).is_some() {
				continue;
			}
			cross.push(TileQuad {
				index: global_index(project, TileRef { pack, tile: pd.cand_tile, transform: self.cand_xform }),
				transform: self.cand_xform.bits(),
				rect: self.side_rect(d, dir),
			});
		}
		let (ox, oy) = self.cross_origin(d);
		let cs = 3.0 * self.cell_px();
		layers.push((cross, Rect::new(ox, oy, cs, cs)));
		// Orientation previews (mini crosses).
		let mut minis = Vec::new();
		for k in 0..8 {
			let xf = Transform { rot: (k & 3) as u8, mirror: k & 4 != 0 };
			let r = self.orient_rect(d, k);
			let cell = |row: usize, col: usize| {
				Rect::new(r.x + col as f32 * Self::MINI, r.y + row as f32 * Self::MINI, Self::MINI, Self::MINI)
			};
			minis.push(TileQuad {
				index: global_index(project, TileRef { pack, tile: pd.main_tile, transform: Transform::default() }),
				transform: 0,
				rect: cell(1, 1),
			});
			for dir in 0..4 {
				if pd.wildcard(dir).is_some() {
					continue;
				}
				let cr = match dir {
					0 => cell(0, 1),
					1 => cell(1, 2),
					2 => cell(2, 1),
					_ => cell(1, 0),
				};
				minis.push(TileQuad {
					index: global_index(project, TileRef { pack, tile: pd.cand_tile, transform: xf }),
					transform: xf.bits(),
					rect: cr,
				});
			}
		}
		// Clip to the whole picker block.
		let p0 = self.orient_rect(d, 0);
		let p7 = self.orient_rect(d, 7);
		layers.push((minis, Rect::new(p0.x, p0.y, (p7.x + p7.w) - p0.x, (p7.y + p7.h) - p0.y)));
		// (The groups panel shows names as text only - no thumbnails, to keep it
		// compact.)
		layers
	}
}

fn filter_label(f: &Filter) -> String {
	match f {
		Filter::All => "all".to_string(),
		Filter::Unprocessed => "[unprocessed]".to_string(),
		Filter::Group(n) => n.clone(),
	}
}

/// Highlight only a matched side's three outer edges (the seam to the centre stays
/// clear so the match is visible). `dir`: 0=N(top),1=E(right),2=S(bottom),3=W(left).
fn external_highlight(q: &mut UiQuads, r: Rect, dir: usize, w: f32, h: f32) {
	let t = 2.0;
	let top = Rect::new(r.x, r.y, r.w, t);
	let bottom = Rect::new(r.x, r.y + r.h - t, r.w, t);
	let left = Rect::new(r.x, r.y, t, r.h);
	let right = Rect::new(r.x + r.w - t, r.y, t, r.h);
	// The edge facing the centre (excluded): N→bottom, E→left, S→top, W→right.
	let edges: [(Rect, bool); 4] = [(top, dir != 2), (bottom, dir != 0), (left, dir != 1), (right, dir != 3)];
	for (rect, show) in edges {
		if show {
			q.rect(rect, w, h, GREEN);
		}
	}
}

/// Shrink a rect by `m` on every side.
fn inset(r: Rect, m: f32) -> Rect {
	Rect::new(r.x + m, r.y + m, r.w - 2.0 * m, r.h - 2.0 * m)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn assets_root() -> std::path::PathBuf {
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/assets/tilepacks")
	}

	fn green() -> Project {
		Project::new(16, 16, &["GREEN".to_string()], &assets_root(), 7).expect("GREEN project")
	}

	fn find_tile(pd: &PackData, id: &str) -> u16 {
		pd.cur.ids.iter().position(|s| s == id).expect("tile id present") as u16
	}

	#[test]
	fn toggle_adds_and_removes_reciprocal_rule() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		let gsa = find_tile(m.pd(), "GSa000");
		let gsh = find_tile(m.pd(), "GSh000");
		m.pd_mut().main_tile = gsa;
		m.pd_mut().cand_tile = gsh;
		let cx = Transform::default();
		assert!(!m.pd().match_present(gsa, gsh, 2, cx));
		m.pd_mut().toggle_match(2, cx);
		assert!(m.pd().match_present(gsa, gsh, 2, cx), "forward added");
		assert!(m.pd().dir("GSh", 0).iter().any(|e| e == "GSa"), "reciprocal added");
		assert!(m.dirty());
		m.pd_mut().toggle_match(2, cx);
		assert!(!m.pd().match_present(gsa, gsh, 2, cx), "forward removed");
	}

	#[test]
	fn row_model_ungrouped_first_then_groups() {
		let project = green();
		let m = MatchEditor::new(&project, None).expect("rules");
		let rows = m.rows(&Filter::All);
		// GREEN has families with no variant group (GLb/GMa/GTa…) → [ungrouped] leads.
		assert!(matches!(rows.first(), Some(Row::Ungrouped)), "starts with the [ungrouped] header");
		assert_eq!(rows.iter().filter(|r| matches!(r, Row::Ungrouped)).count(), 1, "one [ungrouped] header");
		assert!(rows.iter().any(|r| matches!(r, Row::Group(_))), "has explicit group headers");
		assert!(rows.iter().any(|r| matches!(r, Row::Tile(_))), "has tile rows");
		// Group headers are only real variant groups.
		for r in &rows {
			if let Row::Group(gi) = r {
				assert!(m.pd().cur.groups[*gi].real, "headers are real variant groups");
			}
		}
		// Unprocessed filter: only no-rule groups.
		for r in m.rows(&Filter::Unprocessed) {
			if let Row::Group(gi) = r {
				assert!(!m.pd().has_rule(&m.pd().cur.groups[gi].name));
			}
		}
		// A single-group filter shows that group and no [ungrouped] bucket.
		let only = m.rows(&Filter::Group("GSa".into()));
		assert!(only.iter().all(|r| !matches!(r, Row::Ungrouped)), "group filter hides [ungrouped]");
		assert!(matches!(only.first(), Some(Row::Group(_))), "group filter starts at the group");
	}

	#[test]
	fn group_assign_and_none_round_trip() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		let gsa = find_tile(m.pd(), "GSa000");
		m.pd_mut().main_tile = gsa;
		m.pd_mut().move_tile(gsa, Some("GSh"));
		assert_eq!(m.pd().group_name(gsa), "GSh");
		m.pd_mut().move_tile(gsa, None); // [none] → family fallback
		assert_eq!(m.pd().group_name(gsa), "GSa");
	}

	#[test]
	fn orientation_bits_round_trip() {
		for k in 0..8u32 {
			let xf = Transform { rot: (k & 3) as u8, mirror: k & 4 != 0 };
			assert_eq!(xf.bits(), k);
		}
	}

	#[test]
	fn wildcard_cycle_then_reset_clears_dirty() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		let gsf = find_tile(m.pd(), "GSf000");
		m.pd_mut().main_tile = gsf;
		while m.pd().wildcard(0).is_some() {
			m.pd_mut().cycle_wildcard(0);
		}
		let before_dirty = m.dirty();
		m.pd_mut().cycle_wildcard(0);
		assert_eq!(m.pd().wildcard(0), Some(true));
		assert!(m.dirty());
		m.reset();
		assert_eq!(m.dirty(), before_dirty, "reset restores baseline");
	}

	#[test]
	fn staged_rename_reflected_and_collision_rejected() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		let t = find_tile(m.pd(), "GSa000");
		assert!(m.pd_mut().set_id(t, "GSa999"));
		assert_eq!(m.pd().effective_id(t), "GSa999");
		assert!(m.dirty());
		// Renaming to an id another tile already has is rejected.
		assert!(!m.pd_mut().set_id(t, "GSa001"));
		assert_eq!(m.pd().effective_id(t), "GSa999", "rejected rename keeps the staged id");
		let commit = &m.commits()[0];
		assert!(commit.renames.iter().any(|(o, n)| o == "GSa000" && n == "GSa999"));
	}
}
