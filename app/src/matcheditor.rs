//! Edit Tile Match Data MODEL (DEV only): the staged working state behind the
//! visual editor for a pack's `tiles.match.json` adjacency rules +
//! `tiles.variants.json` grouping, plus per-tile id and pass editing.
//!
//! This file owns the data + rules only; the dialog itself is composed in
//! [`crate::uikit_overlay`] out of wgpu-ui bricks plus the custom widgets in
//! [`crate::matchview`] (tile lists, adjacency cross, orientation picker).
//!
//! Everything is staged in a working copy; **Save** applies it (id renames
//! cascade to the pack files + every shipped map + template; pass writes the
//! pack pass table only; match/grouping write the pack match/variants),
//! **Reset** drops it. Matching is per group (`group_of` - a variant group,
//! else the id family); the cross/list operate on the selected tile's group.

use std::collections::{HashMap, HashSet};

use map_core::{MatchRule, Project, Transform, family_of};

/// A list/region filter: everything, only un-ruled groups, or one named group.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Filter {
	All,
	Unprocessed,
	Group(String),
}

/// One editable group: name (the match-rule key), member tile indices, whether it
/// came from `tiles.variants.json` (`real`), and whether the user changed its
/// membership (`modified`). Only real-or-modified non-empty groups are written
/// back as variant groups; the rest rely on the engine's id-family fallback.
#[derive(Clone)]
pub(crate) struct Group {
	pub(crate) name: String,
	pub(crate) tiles: Vec<u16>,
	pub(crate) real: bool,
	modified: bool,
}

/// The mutable state of one pack - cloned for Reset, snapshotted on Save.
#[derive(Clone)]
pub(crate) struct Snapshot {
	ids: Vec<String>,
	pub(crate) pass: Vec<u8>,
	pub(crate) groups: Vec<Group>,
	/// Tile bin index → index into `groups`.
	tile_group: Vec<usize>,
	/// Group name → the four ring-indexed (N,E,S,W) entry lists.
	pub(crate) matches: HashMap<String, [Vec<String>; 4]>,
}

/// The working copy for one pack (the dialog can switch between the project's packs
/// that carry match rules without touching the project until Save).
pub(crate) struct PackData {
	pub(crate) pack: usize,
	pub(crate) name: String,
	pub(crate) tile_count: u16,
	/// Working state (edited); `orig` is the on-disk baseline (rename source +
	/// Reset target).
	pub(crate) cur: Snapshot,
	orig: Snapshot,
	pub(crate) main_tile: u16,
	pub(crate) cand_tile: u16,
	pub(crate) main_filter: Filter,
	pub(crate) cand_filter: Filter,
	/// Collapsed list sections (their member tile rows are hidden): group indices,
	/// with the sentinel `usize::MAX` for the `[ungrouped]` bucket. A view concern,
	/// shared by both lists; not part of [`Self::dirty`].
	collapsed: HashSet<usize>,
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

pub struct MatchEditor {
	packs: Vec<PackData>,
	pub(crate) active: usize,
	pub(crate) cand_xform: Transform,
	/// Cross tile enlargement 1..=6 → cell = 16*size px.
	pub(crate) cross_size: u8,
	/// Selected group in the groups panel.
	pub(crate) group_sel: usize,
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

	/// Fill any missing reciprocal: for every `A`→`B:T` listed on `A`'s side `d`,
	/// ensure `B`'s facing side lists `A` at the inverse transform. Idempotent
	/// (a `contains` guard); wildcards need no reciprocal. Run on Save so a
	/// one-sided edit (or hand-authored asymmetry) can't slip through.
	fn symmetrize(&mut self) {
		let mut adds: Vec<(String, usize, String)> = Vec::new();
		for (g, dirs) in &self.matches {
			for (d, list) in dirs.iter().enumerate() {
				for e in list {
					if e.starts_with("__") {
						continue;
					}
					let (cg, t) = match e.split_once(':') {
						Some((cg, suf)) => match Transform::parse(suf) {
							Ok(t) => (cg, t),
							Err(_) => continue,
						},
						None => (e.as_str(), Transform::default()),
					};
					let rev_dir = t.screen_to_base((d + 2) % 4);
					let rev = format!("{g}{}", t.inverse().suffix());
					if !self.matches.get(cg).is_some_and(|cd| cd[rev_dir].contains(&rev)) {
						adds.push((cg.to_string(), rev_dir, rev));
					}
				}
			}
		}
		for (cg, d, rev) in adds {
			let entry = self.matches.entry(cg).or_default();
			if !entry[d].contains(&rev) {
				entry[d].push(rev);
			}
		}
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
			main_filter: Filter::All,
			cand_filter: Filter::All,
			collapsed: HashSet::new(),
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

	pub(crate) fn group_idx(&self, tile: u16) -> usize {
		self.cur.tile_group[tile as usize]
	}

	pub(crate) fn group_name(&self, tile: u16) -> &str {
		&self.cur.groups[self.group_idx(tile)].name
	}

	/// Does this group have any adjacency rule? (else "unprocessed").
	pub(crate) fn has_rule(&self, group: &str) -> bool {
		self.cur.matches.get(group).is_some_and(|d| d.iter().any(|l| !l.is_empty()))
	}

	fn dir(&self, group: &str, ring_dir: usize) -> &[String] {
		self.cur.matches.get(group).map(|d| d[ring_dir].as_slice()).unwrap_or(&[])
	}

	pub(crate) fn match_present(&self, main: u16, cand: u16, screen_dir: usize, cand_xform: Transform) -> bool {
		let mg = self.group_name(main);
		let cg = self.group_name(cand);
		if self.dir(mg, screen_dir).contains(&format!("{cg}{}", cand_xform.suffix())) {
			return true;
		}
		// Wildcard-to-wildcard: a `__WATER__`/`__LAND__` side matches any same-
		// wildcard side facing it, with no explicit per-pair entry needed.
		let rev_dir = cand_xform.screen_to_base((screen_dir + 2) % 4);
		["__WATER__", "__LAND__"].iter().any(|tok| {
			self.dir(mg, screen_dir).iter().any(|e| e == tok) && self.dir(cg, rev_dir).iter().any(|e| e == tok)
		})
	}

	/// The ring directions (N=0,E=1,S=2,W=3) on which `cand`'s group is a
	/// neighbour of `main`'s group: an explicit listing (any transform) or a
	/// shared `__WATER__`/`__LAND__` wildcard on the facing side. Drives the
	/// candidate-list match annotation for the selected main.
	pub(crate) fn matched_dirs(&self, main: u16, cand: u16) -> [bool; 4] {
		let mg = self.group_name(main);
		let cg = self.group_name(cand);
		std::array::from_fn(|d| {
			let md = self.dir(mg, d);
			let explicit = md.iter().any(|e| entry_group(e) == cg);
			let wild = ["__WATER__", "__LAND__"]
				.iter()
				.any(|tok| md.iter().any(|e| e == tok) && self.dir(cg, (d + 2) % 4).iter().any(|e| e == tok));
			explicit || wild
		})
	}

	/// Toggle the match on `screen_dir`, keeping the reciprocal rule in sync.
	pub(crate) fn toggle_match(&mut self, screen_dir: usize, cand_xform: Transform) {
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
	pub(crate) fn wildcard(&self, ring_dir: usize) -> Option<bool> {
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
	pub(crate) fn cycle_wildcard(&mut self, ring_dir: usize) {
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
	pub(crate) fn move_tile(&mut self, tile: u16, target: Option<&str>) {
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
	pub(crate) fn add_group(&mut self, name: &str) -> usize {
		if let Some(i) = self.cur.groups.iter().position(|g| g.name == name) {
			return i;
		}
		self.cur.groups.push(Group { name: name.to_string(), tiles: Vec::new(), real: true, modified: true });
		self.cur.groups.len() - 1
	}

	pub(crate) fn rename_group(&mut self, idx: usize, new: &str) {
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
	pub(crate) fn delete_group(&mut self, idx: usize) {
		let tiles = std::mem::take(&mut self.cur.groups[idx].tiles);
		let name = self.cur.groups[idx].name.clone();
		self.cur.matches.remove(&name);
		for t in tiles {
			self.move_tile(t, None);
		}
	}

	pub(crate) fn effective_id(&self, tile: u16) -> &str {
		&self.cur.ids[tile as usize]
	}

	/// Stage a tile-id rename; rejects a colliding id.
	pub(crate) fn set_id(&mut self, tile: u16, new: &str) -> bool {
		if new.is_empty() || self.cur.ids.iter().enumerate().any(|(i, s)| i != tile as usize && s == new) {
			return false;
		}
		self.cur.ids[tile as usize] = new.to_string();
		true
	}

	pub(crate) fn set_pass(&mut self, tile: u16, pass: u8) {
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
	pub(crate) fn real_groups(&self) -> Vec<usize> {
		let mut v: Vec<usize> = (0..self.cur.groups.len())
			.filter(|&i| self.cur.groups[i].real && !self.cur.groups[i].tiles.is_empty())
			.collect();
		v.sort_by(|&a, &b| self.cur.groups[a].name.cmp(&self.cur.groups[b].name));
		v
	}

	/// Tiles that belong to no explicit variant group (the engine resolves them
	/// by id family). Listed at the top of each list under `[ungrouped]`.
	pub(crate) fn ungrouped_tiles(&self) -> Vec<u16> {
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
pub(crate) enum Row {
	Ungrouped,
	Group(usize),
	Tile(u16),
}

/// A row's semantic tone; the view maps it to a color (green selection /
/// orange "needs data" / yellow "has rules" / plain ink).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowTone {
	Select,
	Warn,
	Rule,
	Plain,
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
		Some(Self { packs, active, cand_xform: Transform::default(), cross_size: 3, group_sel: 0 })
	}

	/// The pack names the dialog's pack select offers (each carries rules).
	pub(crate) fn pack_names(&self) -> Vec<String> {
		self.packs.iter().map(|pd| pd.name.clone()).collect()
	}

	/// Switch the active pack (the working copies persist across switches).
	pub(crate) fn set_active(&mut self, i: usize) {
		self.active = i.min(self.packs.len() - 1);
		self.group_sel = 0;
	}

	pub(crate) fn pd(&self) -> &PackData {
		&self.packs[self.active]
	}

	pub(crate) fn pd_mut(&mut self) -> &mut PackData {
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

	/// Fill missing reciprocal matches on every pack being saved (the dirty ones),
	/// so a one-sided rule can't reach disk. Called just before [`Self::commits`].
	pub fn symmetrize(&mut self) {
		for pd in &mut self.packs {
			if pd.dirty() {
				pd.cur.symmetrize();
			}
		}
	}

	/// Toggle a list section's collapsed state (group header or `[ungrouped]`).
	pub(crate) fn toggle_collapse(&mut self, row: Row) {
		let key = fold_key(row);
		let set = &mut self.pd_mut().collapsed;
		if !set.remove(&key) {
			set.insert(key);
		}
	}

	/// Whether a header row's section is collapsed (member rows hidden).
	pub(crate) fn is_collapsed(&self, row: Row) -> bool {
		self.pd().collapsed.contains(&fold_key(row))
	}

	// ----- row model ---------------------------------------------------------

	pub(crate) fn rows(&self, filter: &Filter) -> Vec<Row> {
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
				if !pd.collapsed.contains(&usize::MAX) {
					out.extend(ung.into_iter().map(Row::Tile));
				}
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
			if !pd.collapsed.contains(&gi) {
				let mut tiles = g.tiles.clone();
				tiles.sort_unstable();
				out.extend(tiles.into_iter().map(Row::Tile));
			}
		}
		out
	}

	pub(crate) fn filter_labels(&self) -> Vec<String> {
		let mut v = vec!["all".to_string(), "[unprocessed]".to_string()];
		for gi in self.pd().real_groups() {
			v.push(self.pd().cur.groups[gi].name.clone());
		}
		v
	}

	pub(crate) fn filter_of(&self, idx: usize) -> Filter {
		match idx {
			0 => Filter::All,
			1 => Filter::Unprocessed,
			_ => {
				let gi = self.pd().real_groups();
				gi.get(idx - 2).map(|&i| Filter::Group(self.pd().cur.groups[i].name.clone())).unwrap_or(Filter::All)
			}
		}
	}

	pub(crate) fn filter_index(&self, f: &Filter) -> usize {
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
	pub(crate) fn assign_labels(&self) -> Vec<String> {
		let mut v = vec!["[none]".to_string()];
		for gi in self.pd().real_groups() {
			v.push(self.pd().cur.groups[gi].name.clone());
		}
		v
	}

	pub(crate) fn assign_index(&self) -> usize {
		let pd = self.pd();
		let gi = pd.group_idx(pd.main_tile);
		if !pd.cur.groups[gi].real {
			return 0; // [none] = ungrouped (id-family fallback)
		}
		pd.real_groups().iter().position(|&i| i == gi).map(|p| p + 1).unwrap_or(0)
	}

	/// Select a row in the main (left) or candidate list: a tile row directly,
	/// a header row via its first member.
	pub(crate) fn select_row(&mut self, left: bool, row: Row) {
		let tile = match row {
			Row::Tile(t) => t,
			Row::Group(gi) => *self.pd().cur.groups[gi].tiles.iter().min().unwrap_or(&0),
			Row::Ungrouped => self.pd().ungrouped_tiles().first().copied().unwrap_or(0),
		};
		if left {
			self.pd_mut().main_tile = tile;
		} else {
			self.pd_mut().cand_tile = tile;
		}
	}

	/// Drop the active pack's staged edits (back to the on-disk baseline).
	pub fn reset(&mut self) {
		self.pd_mut().reset();
	}

	/// The semantic tone of a row against the list's selected tile `sel`.
	pub(crate) fn row_tone(&self, sel: u16, row: Row) -> RowTone {
		let pd = self.pd();
		match row {
			// Green when the selection is itself ungrouped; else the "needs data" tone.
			Row::Ungrouped => {
				if !pd.cur.groups[pd.group_idx(sel)].real {
					RowTone::Select
				} else {
					RowTone::Warn
				}
			}
			Row::Group(gi) => {
				if pd.group_idx(sel) == gi {
					RowTone::Select
				} else if !pd.has_rule(&pd.cur.groups[gi].name) {
					RowTone::Warn
				} else {
					RowTone::Rule
				}
			}
			Row::Tile(t) => {
				if t == sel {
					RowTone::Select
				} else if !pd.has_rule(pd.group_name(t)) {
					RowTone::Warn
				} else {
					RowTone::Plain
				}
			}
		}
	}
}

/// A match entry's group token (`"GSa:!S"` → `"GSa"`); wildcards return as-is.
fn entry_group(e: &str) -> &str {
	e.split(':').next().unwrap_or(e)
}

/// The [`PackData::collapsed`] key for a header row (`usize::MAX` = `[ungrouped]`).
pub(crate) fn fold_key(row: Row) -> usize {
	match row {
		Row::Group(gi) => gi,
		_ => usize::MAX,
	}
}

/// The compact matched-direction tag (`"N E S W"` subset) for a candidate row.
pub(crate) fn dirs_tag(dirs: [bool; 4]) -> String {
	["N", "E", "S", "W"].iter().enumerate().filter(|(i, _)| dirs[*i]).map(|(_, s)| *s).collect::<Vec<_>>().join("")
}

/// Highlight only a matched side's three outer edges (the seam to the centre stays
/// clear so the match is visible). `dir`: 0=N(top),1=E(right),2=S(bottom),3=W(left).
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
	fn symmetrize_fills_a_missing_reciprocal() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		// A one-sided rule X.S -> Y, with no reciprocal on Y.
		let mut d: [Vec<String>; 4] = Default::default();
		d[2].push("Y".into());
		m.pd_mut().cur.matches.insert("X".into(), d);
		assert!(!m.pd().cur.matches.contains_key("Y"));
		m.pd_mut().cur.symmetrize();
		// The reciprocal Y.N -> X is filled (S faces N at the identity transform).
		let y = m.pd().cur.matches.get("Y").expect("Y created");
		assert!(y[0].contains(&"X".to_string()), "Y.N -> X reciprocal filled");
		// Idempotent: a second pass changes nothing.
		let before = m.pd().cur.matches.get("Y").unwrap()[0].len();
		m.pd_mut().cur.symmetrize();
		assert_eq!(m.pd().cur.matches.get("Y").unwrap()[0].len(), before, "symmetrize is idempotent");
	}

	#[test]
	fn wildcard_sides_match_only_their_own_kind() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		let a = find_tile(m.pd(), "GSa000");
		let b = find_tile(m.pd(), "GSh000");
		m.pd_mut().main_tile = a;
		m.pd_mut().cand_tile = b;
		let (ga, gb) = (m.pd().group_name(a).to_string(), m.pd().group_name(b).to_string());
		let set = |m: &mut MatchEditor, g: &str, dir: usize, tok: &str| {
			let d = m.pd_mut().cur.matches.entry(g.to_string()).or_default();
			d[dir].clear();
			d[dir].push(tok.into());
		};
		// A's south and B's north both __WATER__ → they match (no explicit pair).
		set(&mut m, &ga, 2, "__WATER__");
		set(&mut m, &gb, 0, "__WATER__");
		let cx = Transform::default();
		assert!(m.pd().match_present(a, b, 2, cx), "two __WATER__ sides match");
		assert_eq!(m.pd().matched_dirs(a, b), [false, false, true, false], "matched on south only");
		// Water vs land does not match.
		set(&mut m, &gb, 0, "__LAND__");
		assert!(!m.pd().match_present(a, b, 2, cx), "water vs land does not match");
		assert_eq!(m.pd().matched_dirs(a, b), [false; 4]);
	}

	#[test]
	fn collapsing_a_group_hides_its_tiles() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		let gi = m.pd().real_groups()[0];
		let members = m.pd().cur.groups[gi].tiles.len();
		assert!(members > 0);
		let before = m.rows(&Filter::All).len();
		m.toggle_collapse(Row::Group(gi));
		assert_eq!(m.rows(&Filter::All).len(), before - members, "collapse hides the member rows");
		assert!(
			m.rows(&Filter::All).iter().any(|r| matches!(r, Row::Group(g) if *g == gi)),
			"the header stays visible",
		);
		m.toggle_collapse(Row::Group(gi));
		assert_eq!(m.rows(&Filter::All).len(), before, "expand restores the rows");
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

	/// The editor only opens over packs that carry match rules: a project whose
	/// packs all have empty rule tables yields no editor at all.
	#[test]
	fn new_returns_none_when_no_pack_has_rules() {
		let mut project = green();
		for p in &mut project.packs {
			p.matches.clear();
		}
		assert!(MatchEditor::new(&project, None).is_none(), "no rules anywhere -> no editor");
	}

	/// Pack switching clamps to the roster and resets the group selection;
	/// `mark_saved` re-baselines every pack so nothing stays dirty.
	#[test]
	fn pack_switch_clamps_and_mark_saved_rebaselines() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		assert!(m.packs.len() >= 2, "WATER + GREEN both carry rules");
		m.group_sel = 3;
		m.set_active(0);
		assert_eq!((m.active, m.group_sel), (0, 0), "switch selects the pack and resets the group selection");
		m.set_active(999);
		assert_eq!(m.active, m.packs.len() - 1, "an out-of-range index clamps to the last pack");

		// A staged pass edit makes the pack dirty and lands in its commit...
		m.pd_mut().set_pass(0, 3);
		assert!(m.dirty(), "pass edit dirties the pack");
		let commits = m.commits();
		assert_eq!(commits.len(), 1, "only the edited pack commits");
		assert!(commits[0].pass_changed, "commit flags the pass change");
		assert_eq!(commits[0].pass[0], 3, "commit carries the staged pass value");
		// ...until mark_saved makes the staged state the new baseline.
		m.mark_saved();
		assert!(!m.dirty(), "mark_saved clears dirt");
		assert!(m.commits().is_empty(), "nothing left to commit");
	}

	/// Removing a one-sided rule (its reciprocal absent, e.g. hand-authored)
	/// must not panic or resurrect anything: the forward entry goes away and
	/// the missing candidate-side rule stays missing.
	#[test]
	fn toggle_match_removes_a_one_sided_rule() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		let gsa = find_tile(m.pd(), "GSa000");
		let other = find_tile(m.pd(), "GSh000");
		m.pd_mut().main_tile = gsa;
		// Candidate in a fresh group that has no rule entry at all.
		m.pd_mut().move_tile(other, Some("QQQ"));
		m.pd_mut().cand_tile = other;
		let mg = m.pd().group_name(gsa).to_string();
		m.pd_mut().cur.matches.get_mut(&mg).expect("GSa has rules")[2].push("QQQ".into());
		let cx = Transform::default();
		assert!(m.pd().match_present(gsa, other, 2, cx), "one-sided rule reads as present");
		m.pd_mut().toggle_match(2, cx);
		assert!(!m.pd().match_present(gsa, other, 2, cx), "forward entry removed");
		assert!(!m.pd().cur.matches.contains_key("QQQ"), "no candidate-side entry appears");
	}

	/// The wildcard cycle is none -> water -> land -> none on the main group's side.
	#[test]
	fn wildcard_cycles_through_water_land_none() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		// A fresh group has no wildcard, making the full cycle deterministic.
		let t = find_tile(m.pd(), "GSa000");
		m.pd_mut().move_tile(t, Some("QQW"));
		m.pd_mut().main_tile = t;
		assert_eq!(m.pd().wildcard(1), None, "fresh group starts wildcard-free");
		m.pd_mut().cycle_wildcard(1);
		assert_eq!(m.pd().wildcard(1), Some(true), "none -> water");
		m.pd_mut().cycle_wildcard(1);
		assert_eq!(m.pd().wildcard(1), Some(false), "water -> land");
		m.pd_mut().cycle_wildcard(1);
		assert_eq!(m.pd().wildcard(1), None, "land -> none");
	}

	/// Moving a tile into the group it already lives in is a no-op (no dirt);
	/// moving it to an unknown name creates that group on the fly.
	#[test]
	fn move_tile_self_is_noop_and_unknown_target_creates_the_group() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		let t = find_tile(m.pd(), "GSa000");
		let home = m.pd().group_name(t).to_string();
		m.pd_mut().move_tile(t, Some(&home));
		assert_eq!(m.pd().group_name(t), home, "self-move keeps the group");
		assert!(!m.dirty(), "self-move stages nothing");
		m.pd_mut().move_tile(t, Some("FRESH"));
		assert_eq!(m.pd().group_name(t), "FRESH", "unknown target group is created");
		assert!(m.dirty(), "the move is staged");
	}

	/// add_group returns the existing index for a taken name; rename_group
	/// rejects empty/duplicate names and carries the group's rule to the new
	/// key; delete_group dissolves membership back to the id families and
	/// drops the group's rule.
	#[test]
	fn add_rename_delete_group_lifecycle() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		let existing = m.pd().real_groups()[0];
		let existing_name = m.pd().cur.groups[existing].name.clone();
		assert_eq!(m.pd_mut().add_group(&existing_name), existing, "taken name -> its index");
		let gi = m.pd_mut().add_group("ZATO");
		assert_eq!(m.pd_mut().add_group("ZATO"), gi, "repeat add returns the same group");

		m.pd_mut().rename_group(gi, "");
		assert_eq!(m.pd().cur.groups[gi].name, "ZATO", "empty name rejected");
		let taken = m.pd().cur.groups[existing].name.clone();
		m.pd_mut().rename_group(gi, &taken);
		assert_eq!(m.pd().cur.groups[gi].name, "ZATO", "duplicate name rejected");
		let mut rule: [Vec<String>; 4] = Default::default();
		rule[0].push("GSa".into());
		m.pd_mut().cur.matches.insert("ZATO".into(), rule);
		m.pd_mut().rename_group(gi, "ZETA");
		assert_eq!(m.pd().cur.groups[gi].name, "ZETA", "rename applies");
		assert!(m.pd().cur.matches.contains_key("ZETA"), "the rule follows the new name");
		assert!(!m.pd().cur.matches.contains_key("ZATO"), "the old rule key is gone");

		// Deleting a group returns its member to the id family and drops its rule.
		let t = find_tile(m.pd(), "GSa000");
		m.pd_mut().move_tile(t, Some("ZETA"));
		let zeta = m.pd().cur.groups.iter().position(|g| g.name == "ZETA").expect("ZETA exists");
		m.pd_mut().delete_group(zeta);
		assert_eq!(m.pd().group_name(t), "GSa", "member falls back to its id family");
		assert!(!m.pd().cur.matches.contains_key("ZETA"), "deleted group's rule is dropped");
	}

	/// Editor-level symmetrize touches only the dirty packs, and an entry with
	/// an unparseable transform suffix is skipped instead of inventing a
	/// reciprocal (or panicking).
	#[test]
	fn symmetrize_skips_clean_packs_and_bad_suffixes() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		let clean = (0..m.packs.len()).find(|&i| i != m.active).expect("a second pack");
		let clean_before = m.packs[clean].cur.matches.clone();
		let mut d: [Vec<String>; 4] = Default::default();
		d[2].push("VVV".into());
		d[1].push("GSa:QQ".into()); // ":QQ" is not a transform -> skipped
		m.pd_mut().cur.matches.insert("UUU".into(), d);
		m.symmetrize();
		let v = m.pd().cur.matches.get("VVV").expect("reciprocal group created");
		assert!(v[0].contains(&"UUU".to_string()), "S -> N reciprocal filled on the dirty pack");
		let gsa = m.pd().cur.matches.get("GSa").expect("GSa rules exist");
		assert!(gsa.iter().all(|l| l.iter().all(|e| !e.starts_with("UUU"))), "bad suffix adds no reciprocal");
		assert_eq!(m.packs[clean].cur.matches, clean_before, "clean packs stay untouched");
	}

	/// Collapsing the `[ungrouped]` bucket hides its tile rows but keeps the
	/// header, exactly like a group section.
	#[test]
	fn ungrouped_bucket_collapses_and_expands() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		let before = m.rows(&Filter::All);
		assert!(matches!(before.first(), Some(Row::Ungrouped)), "[ungrouped] leads the list");
		assert!(matches!(before.get(1), Some(Row::Tile(_))), "expanded bucket lists its tiles");
		m.toggle_collapse(Row::Ungrouped);
		assert!(m.is_collapsed(Row::Ungrouped));
		let rows = m.rows(&Filter::All);
		assert!(matches!(rows.first(), Some(Row::Ungrouped)), "the header survives the collapse");
		assert!(matches!(rows.get(1), Some(Row::Group(_))), "member tiles are hidden");
		m.toggle_collapse(Row::Ungrouped);
		assert_eq!(m.rows(&Filter::All).len(), before.len(), "expanding restores the rows");
	}

	/// The filter select round-trips: index -> Filter -> index, with
	/// out-of-range and unknown-group values degrading to All.
	#[test]
	fn filter_select_round_trips_and_clamps() {
		let project = green();
		let m = MatchEditor::new(&project, None).expect("rules");
		assert!(m.filter_of(0) == Filter::All && m.filter_index(&Filter::All) == 0);
		assert!(m.filter_of(1) == Filter::Unprocessed && m.filter_index(&Filter::Unprocessed) == 1);
		let first = m.pd().cur.groups[m.pd().real_groups()[0]].name.clone();
		assert!(m.filter_of(2) == Filter::Group(first.clone()), "index 2 is the first real group");
		assert_eq!(m.filter_index(&Filter::Group(first)), 2);
		assert!(m.filter_of(999) == Filter::All, "past-the-end index degrades to All");
		assert_eq!(m.filter_index(&Filter::Group("NOPE".into())), 0, "unknown group degrades to All");
	}

	/// The assign select maps `[none]` (index 0) to an ungrouped selection and
	/// offsets real groups by one.
	#[test]
	fn assign_index_tracks_the_selected_tiles_group() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		assert_eq!(m.assign_labels()[0], "[none]");
		let ung = m.pd().ungrouped_tiles()[0];
		m.pd_mut().main_tile = ung;
		assert_eq!(m.assign_index(), 0, "ungrouped selection reads [none]");
		let gsa = find_tile(m.pd(), "GSa000");
		m.pd_mut().main_tile = gsa;
		let gi = m.pd().group_idx(gsa);
		let expect = m.pd().real_groups().iter().position(|&i| i == gi).expect("GSa is real") + 1;
		assert_eq!(m.assign_index(), expect, "grouped selection points at its group entry");
	}

	/// Row selection: a tile row selects itself, a header row its first member,
	/// routed to the main (left) or candidate (right) slot.
	#[test]
	fn select_row_routes_tiles_and_headers() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		m.select_row(true, Row::Tile(5));
		assert_eq!(m.pd().main_tile, 5, "left tile row -> main tile");
		m.select_row(false, Row::Tile(7));
		assert_eq!(m.pd().cand_tile, 7, "right tile row -> candidate tile");
		let gi = m.pd().real_groups()[0];
		let first = *m.pd().cur.groups[gi].tiles.iter().min().expect("non-empty group");
		m.select_row(true, Row::Group(gi));
		assert_eq!(m.pd().main_tile, first, "group header selects its first member");
		let ung = m.pd().ungrouped_tiles()[0];
		m.select_row(false, Row::Ungrouped);
		assert_eq!(m.pd().cand_tile, ung, "[ungrouped] header selects its first tile");
	}

	/// Row tones: green for the selection's own row, orange for anything
	/// without rules ("needs data"), yellow for ruled group headers, plain ink
	/// for ruled tiles.
	#[test]
	fn row_tones_reflect_selection_and_rules() {
		let project = green();
		let mut m = MatchEditor::new(&project, None).expect("rules");
		let ruled = m
			.pd()
			.real_groups()
			.into_iter()
			.find(|&i| m.pd().has_rule(&m.pd().cur.groups[i].name))
			.expect("a ruled group");
		let member = *m.pd().cur.groups[ruled].tiles.iter().min().expect("member");
		let ung = m.pd().ungrouped_tiles()[0];

		assert!(m.row_tone(ung, Row::Ungrouped) == RowTone::Select, "[ungrouped] is green for an ungrouped selection");
		assert!(m.row_tone(member, Row::Ungrouped) == RowTone::Warn, "[ungrouped] warns for a grouped selection");
		assert!(m.row_tone(member, Row::Group(ruled)) == RowTone::Select, "own group header is green");
		assert!(m.row_tone(ung, Row::Group(ruled)) == RowTone::Rule, "ruled foreign group is yellow");
		let norule = m.pd_mut().add_group("NR");
		assert!(m.row_tone(member, Row::Group(norule)) == RowTone::Warn, "rule-less group warns");
		assert!(m.row_tone(member, Row::Tile(member)) == RowTone::Select, "own tile row is green");
		assert!(m.row_tone(ung, Row::Tile(member)) == RowTone::Plain, "ruled foreign tile is plain");
		let t2 = find_tile(m.pd(), "GSh000");
		m.pd_mut().move_tile(t2, Some("NR"));
		assert!(m.row_tone(member, Row::Tile(t2)) == RowTone::Warn, "tile of a rule-less group warns");
	}
}
