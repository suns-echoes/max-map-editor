//! The multi-project tab model and the save-open flow that lands in it: tab
//! metadata (names, dirty stars, closability), the park/restore Document
//! swap, switch/add/close, and opening a .DTA save onto its WRL / stock /
//! swapped world. Split from `state.rs` (audit 2026-08-07); a child module,
//! so it reads the parent's private fields directly.

use super::*;

impl EditorState {
	/// The active tab index.
	pub fn active_tab(&self) -> usize {
		self.tabs.active
	}

	/// `(label, dirty, saved)` for each open project, in tab order - the tab
	/// strip. `saved` flags an open save-editor session (a real `.DTA` save),
	/// which the strip marks with a warning colour + `/!\` prefix.
	pub fn tab_infos(&self) -> Vec<(String, bool, bool)> {
		(0..self.tabs.slots.len()).map(|i| (self.name_at(i), self.dirty_at(i), self.saved_at(i))).collect()
	}

	/// Whether tabs show a close `x`: false for the lone blank scratch (the
	/// "no project open" state - nothing to close).
	pub fn tabs_closable(&self) -> bool {
		!(self.tabs.replace_scratch && self.tabs.slots.len() == 1)
	}

	/// Any open project has unsaved changes - the quit guard.
	pub(super) fn any_dirty(&self) -> bool {
		self.project.dirty() || self.tabs.slots.iter().flatten().any(|d| d.project.dirty())
	}

	/// A prompt summarizing the unsaved work for the quit confirm: names the one
	/// dirty map, or counts them when several tabs are unsaved.
	pub(super) fn dirty_summary(&self) -> String {
		let dirty: Vec<usize> = (0..self.tabs.slots.len()).filter(|&i| self.dirty_at(i)).collect();
		match dirty.as_slice() {
			[i] => format!("\"{}\" has unsaved changes.", self.name_at(*i)),
			many => format!("{} maps have unsaved changes.", many.len()),
		}
	}

	/// The save path of tab `i` (the active tab reads the live field).
	fn path_at(&self, i: usize) -> Option<&Path> {
		if i == self.tabs.active { self.path.as_deref() } else { self.tabs.slots[i].as_ref()?.path.as_deref() }
	}

	/// The dirty flag of tab `i`.
	pub(super) fn dirty_at(&self, i: usize) -> bool {
		if i == self.tabs.active {
			self.project.dirty()
		} else {
			self.tabs.slots[i].as_ref().is_some_and(|d| d.project.dirty())
		}
	}

	/// Whether tab `i` is an open save-editor session (a real `.DTA` save) - the
	/// tab strip flags these with a warning colour + `/!\` prefix.
	fn saved_at(&self, i: usize) -> bool {
		if i == self.tabs.active {
			self.project.save.is_some()
		} else {
			self.tabs.slots[i].as_ref().is_some_and(|d| d.project.save.is_some())
		}
	}

	/// The world a save-editor session sits on (e.g. `"SNOW_1"`), for chrome -
	/// `None` for an ordinary map project.
	pub(super) fn save_world(project: &Project) -> Option<&'static str> {
		let world_file = project.save.as_ref()?.file.header.world_file?;
		Some(world_file.strip_suffix(".WRL").unwrap_or(world_file))
	}

	/// Tab `i`'s label: the save file name, else the project's own name; a
	/// save-editor session appends its world (` · SNOW_1`) so it's clear at a
	/// glance which save/world the tab holds.
	pub(super) fn name_at(&self, i: usize) -> String {
		let (path, project_name, world) = if i == self.tabs.active {
			(self.path.as_deref(), self.project.name.as_str(), Self::save_world(&self.project))
		} else {
			let d = self.tabs.slots[i].as_ref();
			(
				d.and_then(|d| d.path.as_deref()),
				d.map(|d| d.project.name.as_str()).unwrap_or(""),
				d.and_then(|d| Self::save_world(&d.project)),
			)
		};
		let base = path
			.and_then(|p| p.file_name())
			.map(|n| n.to_string_lossy().into_owned())
			.or_else(|| (!project_name.is_empty()).then(|| project_name.to_string()))
			.unwrap_or_else(|| "untitled".into());
		match world {
			Some(w) => format!("{base} - {w}"),
			None => base,
		}
	}

	/// The tab already showing `path`, if any (re-opening switches, not stacks).
	fn tab_index_of(&self, path: &Path) -> Option<usize> {
		(0..self.tabs.slots.len()).find(|&i| self.path_at(i) == Some(path))
	}

	/// Snapshot the live (active) fields into a parked [`Document`].
	fn capture_doc(&mut self) -> Document {
		Document {
			project: std::mem::replace(&mut self.project, Project::empty()),
			path: self.path.take(),
			origin: self.origin.take(),
			view: std::mem::replace(&mut self.view, View { pan: [0.0, 0.0], zoom: 1.0 }),
			active_tile: self.active_tile.take(),
			active_color: self.active_color.take(),
		}
	}

	/// Load a parked [`Document`] into the live fields; re-derives the cycler.
	fn restore_doc(&mut self, d: Document) {
		self.project = d.project;
		self.path = d.path;
		self.origin = d.origin;
		self.view = d.view;
		self.active_tile = d.active_tile;
		self.active_color = d.active_color;
		self.palettes.sel_end = None;
		self.refresh_palette();
	}

	/// Switch the active tab. `Ok` (no redraw) when already active / out of range.
	pub(super) fn switch_to(&mut self, i: usize) -> Outcome {
		if i == self.tabs.active || i >= self.tabs.slots.len() {
			return Outcome::Ok;
		}
		let parked = self.capture_doc();
		self.tabs.slots[self.tabs.active] = Some(parked);
		let d = self.tabs.slots[i].take().expect("an inactive tab is parked");
		self.tabs.active = i;
		self.restore_doc(d);
		Outcome::DocReplaced
	}

	/// Open `project` (loaded from `path`) and make it active: switch to an
	/// already-open tab with the same path, replace the bootstrap scratch tab,
	/// or push a new tab.
	pub(super) fn add_doc(&mut self, project: Project, path: Option<PathBuf>, origin: Option<PathBuf>) -> Outcome {
		if let Some(p) = path.as_deref() {
			if let Some(i) = self.tab_index_of(p) {
				return self.switch_to(i);
			}
		}
		let view = self.fit_center((project.width, project.height));
		let doc = Document { project, path, origin, view, active_tile: None, active_color: None };
		if self.tabs.replace_scratch {
			self.tabs.replace_scratch = false;
			self.restore_doc(doc);
		} else {
			let parked = self.capture_doc();
			self.tabs.slots[self.tabs.active] = Some(parked);
			self.tabs.slots.push(None);
			self.tabs.active = self.tabs.slots.len() - 1;
			self.restore_doc(doc);
		}
		Outcome::DocReplaced
	}

	/// The Open-Save error shown when a file isn't the format the editor edits.
	/// The editor only round-trips version 71 — the format M.A.X. Port v0.7.X
	/// writes. Deliberately does not quote a version number: a non-save file
	/// (e.g. a text file) yields a nonsense "version", so naming it misleads.
	/// Names the file by its base name only, not its full path.
	pub(super) fn incompatible_save_message(path: &Path) -> String {
		let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_else(|| path.to_string_lossy());
		format!(
			"Can't open {name} for editing:\n\
			 the save editor only works with version 71 saves - the format M.A.X. Port v0.7.X writes.\n\n\
			 This file isn't a compatible save.",
		)
	}

	/// Build a save-editor project from a `.WRL` map file — the map installed at
	/// the save's slot (`MaxPath/<world>.WRL`), the actual map the save references
	/// (possibly swapped/resized) — and attach the save, decoding at that map's
	/// dimensions. This is what lets a save made on a swapped map open with the
	/// right terrain *and* dimensions.
	pub(super) fn open_save_on_wrl(wrl_path: &Path, world_name: &str, bytes: &[u8]) -> Result<Project, String> {
		let wrl = read_wrl_file(wrl_path).map_err(|e| e.to_string())?;
		let mut project = Project::from_wrl(&wrl, world_name);
		project.attach_save(bytes.to_vec())?;
		Ok(project)
	}

	/// Build a save-editor project from the bundled pristine stock world (its
	/// `.json` in `assets/maps`) and attach the save — the fallback when the
	/// installed map isn't there or doesn't fit the save's dimensions.
	pub(super) fn open_save_on_stock(&self, world_name: &str, bytes: &[u8]) -> Result<Project, String> {
		let map_json = self.resources_root.join("assets/maps").join(format!("{world_name}.json"));
		let mut project = Project::load(&map_json, &self.assets_root)?;
		project.attach_save(bytes.to_vec())?;
		Ok(project)
	}

	/// Name a freshly save-attached `project` after the save (a clean Save-As
	/// base) and build the console inventory line (S1.5). Shared by the silent
	/// open and the "Open Anyway" commit.
	pub(super) fn name_save_project(
		project: &mut Project,
		header: &max_assets::save::SaveHeader,
		world_name: &str,
	) -> String {
		let save_name = header.save_name.trim().to_string();
		if !save_name.is_empty() {
			project.name = save_name.clone();
		}
		let save = project.save.as_ref().expect("attach_save populated the save");
		let units = save.file.units().count();
		let [ground, land_sea, buildings, air, _particles] = save.file.lists().map(|(_, l)| l.len());
		let label = if save_name.is_empty() { "(unnamed)" } else { save_name.as_str() };
		format!(
			"opened save \"{label}\" ({}) on {world_name} {}x{}: {units} units - \
			 {buildings} buildings, {ground} ground-cover, {land_sea} land/sea, {air} air",
			header.category.label(),
			project.width,
			project.height,
		)
	}

	/// Open a fully-built, save-attached `project` as a new tab, echoing `summary`
	/// to the console/stderr first (the shared tail of every save-open path).
	pub(super) fn commit_save_open(
		&mut self,
		mut project: Project,
		header: &max_assets::save::SaveHeader,
		world_name: &str,
	) -> Outcome {
		let summary = Self::name_save_project(&mut project, header, world_name);
		eprintln!("{summary}");
		self.console.push_line(summary);
		// Correctness pass on load (`save-editor-bug.md`): warn if the opened save
		// carries units in an impossible idle+in-progress state (the fingerprint of a
		// unit an older editor placed by cloning a mid-build template). Non-mutating —
		// Export Save File repairs them on write.
		let issues = project.save_integrity_issues();
		if !issues.is_empty() {
			let warning = format!(
				"warning: {} unit(s) in this save have corrupt runtime state ({}) - Export Save File will repair them",
				issues.len(),
				issues[0].kind.describe(),
			);
			eprintln!("{warning}");
			self.console.push_line(warning);
		}
		// Same idea for the complex invariant (HANDOFF Finding 1): a building an
		// older editor exported with a null Complex crashes the game at run time.
		// Non-mutating - Export Save File repairs on write.
		let complex_issues = project.save_complex_issues();
		if !complex_issues.is_empty() {
			let warning = format!(
				"warning: {} complex problem(s) in this save ({}) - Export Save File will repair them",
				complex_issues.len(),
				complex_issues[0],
			);
			eprintln!("{warning}");
			self.console.push_line(warning);
		}
		// A save-editor session has no project `.json` yet - Save writes one
		// (Save-As). The `.DTA` is not added to Quick Load (its recent entries
		// dispatch `open!`, which can't open a save).
		self.add_doc(project, None, None)
	}

	/// Close the active tab. A dirty tab needs `force` (the confirm modal - see
	/// the `CloseProject` handler - gates this). Closing the **last** project
	/// is allowed: it resets to a blank scratch (the app stays open), which the
	/// next `open`/`new` replaces.
	pub(super) fn close_active(&mut self, force: bool) -> Outcome {
		if self.project.dirty() && !force {
			return Outcome::Failed("close-project: unsaved changes - `save` first or use `close-project!`".into());
		}
		if self.tabs.slots.len() <= 1 {
			let view = self.fit_center((1, 1));
			let blank = Document {
				project: Project::empty(),
				path: None,
				origin: None,
				view,
				active_tile: None,
				active_color: None,
			};
			self.tabs.slots = vec![None];
			self.tabs.active = 0;
			self.tabs.replace_scratch = true;
			self.restore_doc(blank);
			return Outcome::DocReplaced;
		}
		// Drop the active doc (its `None` slot), then activate a neighbour.
		self.tabs.slots.remove(self.tabs.active);
		let i = self.tabs.active.min(self.tabs.slots.len() - 1);
		let d = self.tabs.slots[i].take().expect("a neighbour tab is parked");
		self.tabs.active = i;
		self.restore_doc(d);
		Outcome::DocReplaced
	}
}
