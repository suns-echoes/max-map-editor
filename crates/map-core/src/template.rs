//! Tile templates: reusable chunks of map (a mountain range, a forest, an
//! oasis) captured from a selection and stamped back anywhere — on any map
//! that uses the same tile packs.
//!
//! A template file is JSON: a `use` manifest naming the packs its tiles come
//! from, and a cell grid in the **project save encoding** (`"WATR05,GSd004:!N"`
//! per cell, layers comma-joined, `""` = a hole). Cells reference tiles by
//! **id**, never by index, so a template survives pack reordering and resolves
//! against whatever pack roster the open map has — [`Template::compatible`]
//! answers whether every id resolves there.
//!
//! Capture takes the selection's bounding box and keeps only selected cells
//! (holes stay holes); apply skips holes, so irregular shapes stamp exactly
//! what was selected. The same struct doubles as the copy/paste clipboard —
//! paste is "apply a transient template".

use std::path::Path;

use crate::project::{LAYER_GROUND, MAX_LAYERS, Project, TileRef};
use crate::selection::Selection;

/// One captured cell: the save-encoded stack spec (`""` = hole).
type CellSpec = String;

#[derive(Clone)]
pub struct Template {
	pub name: String,
	/// Footprint in cells.
	pub width: u16,
	pub height: u16,
	/// Pack names (+ versions, informational) the cell ids resolve against.
	pub uses: Vec<(String, String)>,
	/// Row-major `width × height` cell specs.
	pub cells: Vec<CellSpec>,
}

impl Template {
	/// Capture the selected cells of `project` (bounding-box window;
	/// unselected cells inside the box become holes). Water-layer entries are
	/// captured too, so a lake template carries its water; on land-only
	/// selections the ground spec is all a cell records... both layers ride
	/// in the cell spec exactly as the save format writes them.
	pub fn capture(project: &Project, selection: &Selection, name: &str) -> Result<Self, String> {
		let (x0, y0, x1, y1) = selection.bounds().ok_or("nothing selected")?;
		let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
		let mut cells = Vec::with_capacity(w as usize * h as usize);
		let mut used: Vec<u8> = Vec::new();
		for y in y0..=y1 {
			for x in x0..=x1 {
				if !selection.contains(x, y) {
					cells.push(String::new());
					continue;
				}
				cells.push(project.cell_spec(x, y).unwrap_or_default());
				if let Some(stack) = project.cell(x, y) {
					for t in stack.iter().flatten() {
						if !used.contains(&t.pack) {
							used.push(t.pack);
						}
					}
				}
			}
		}
		if cells.iter().all(|c| c.is_empty()) {
			return Err("the selection is empty (no tiles)".into());
		}
		let uses = used
			.iter()
			.map(|&p| {
				let pack = &project.packs[p as usize];
				(pack.name.clone(), pack.version.clone())
			})
			.collect();
		Ok(Self { name: name.to_string(), width: w, height: h, uses, cells })
	}

	/// Capture for the clipboard: same as [`Self::capture`] but unnamed.
	pub fn capture_clipboard(project: &Project, selection: &Selection) -> Result<Self, String> {
		Self::capture(project, selection, "clipboard")
	}

	/// Does every tile id resolve in `project`'s packs? (The explorer hides
	/// incompatible templates; apply refuses them with the missing id.)
	pub fn compatible(&self, project: &Project) -> bool {
		self.missing_id(project).is_none()
	}

	/// The first cell id that does not resolve in `project`, if any.
	pub fn missing_id(&self, project: &Project) -> Option<String> {
		for spec in &self.cells {
			for part in spec.split(',').filter(|p| !p.is_empty()) {
				if project.resolve_ref(part).is_err() {
					return Some(part.split(':').next().unwrap_or(part).to_string());
				}
			}
		}
		None
	}

	/// The template's entries resolved against `project`, as
	/// `(dx, dy, layer, tile)` placements relative to the top-left cell.
	/// Holes contribute nothing. Errors name the first unresolvable id.
	pub fn resolve(&self, project: &Project) -> Result<Vec<(u16, u16, usize, TileRef)>, String> {
		let mut out = Vec::new();
		for (i, spec) in self.cells.iter().enumerate() {
			let (dx, dy) = ((i % self.width as usize) as u16, (i / self.width as usize) as u16);
			for part in spec.split(',').filter(|p| !p.is_empty()) {
				let (tile, layer) = project.resolve_ref(part)?;
				out.push((dx, dy, layer, tile));
			}
		}
		Ok(out)
	}

	/// Stamp the template with its top-left at `(x, y)` — one undo
	/// transaction (or part of the open stroke). Cells past the map edge
	/// clip; holes leave the map untouched. A cell that carries only a
	/// ground entry keeps the map's water beneath it.
	pub fn apply(&self, project: &mut Project, x: u16, y: u16) -> Result<bool, String> {
		let entries = self.resolve(project)?;
		let edits: Vec<(u16, u16, usize, Option<TileRef>)> = entries
			.into_iter()
			.filter_map(|(dx, dy, layer, tile)| {
				let (cx, cy) = (x.checked_add(dx)?, y.checked_add(dy)?);
				(cx < project.width && cy < project.height).then_some((cx, cy, layer, Some(tile)))
			})
			.collect();
		Ok(project.place_many(&edits))
	}

	// ----- persistence ---------------------------------------------------------

	/// Serialize to the template JSON (same conventions as the project file).
	pub fn save_string(&self) -> String {
		use json::JsonValue as J;
		let uses: Vec<J> = self
			.uses
			.iter()
			.map(|(name, version)| {
				J::Object(vec![
					("name".to_string(), J::String(name.clone())),
					("version".to_string(), J::String(version.clone())),
				])
			})
			.collect();
		let mut rows = Vec::with_capacity(self.height as usize);
		for y in 0..self.height as usize {
			let row: Vec<J> =
				(0..self.width as usize).map(|x| J::String(self.cells[y * self.width as usize + x].clone())).collect();
			rows.push(J::Array(row));
		}
		J::Object(vec![
			("version".to_string(), J::String("1".to_string())),
			("name".to_string(), J::String(self.name.clone())),
			("width".to_string(), J::Number(self.width as f64)),
			("height".to_string(), J::Number(self.height as f64)),
			("use".to_string(), J::Array(uses)),
			("map".to_string(), J::Array(rows)),
		])
		.to_pretty()
	}

	// An inherent constructor (like `INI::from_str`) — the `FromStr` trait
	// would force callers through `.parse()` for no gain.
	#[allow(clippy::should_implement_trait)]
	pub fn from_str(text: &str) -> Result<Self, String> {
		let root = json::parse(text)?;
		let name = root.get("name").and_then(|v| v.as_str()).unwrap_or("unnamed").to_string();
		let width = root.get("width").and_then(|v| v.as_f64()).ok_or("template: missing width")? as u16;
		let height = root.get("height").and_then(|v| v.as_f64()).ok_or("template: missing height")? as u16;
		if width == 0 || height == 0 || width > 1024 || height > 1024 {
			return Err(format!("template: bad size {width}x{height}"));
		}
		let mut uses = Vec::new();
		if let Some(list) = root.get("use").and_then(|v| v.as_array()) {
			for u in list {
				let pack = u.get("name").and_then(|v| v.as_str()).ok_or("template: a use entry has no name")?;
				let version = u.get("version").and_then(|v| v.as_str()).unwrap_or("");
				uses.push((pack.to_string(), version.to_string()));
			}
		}
		let rows = root.get("map").and_then(|v| v.as_array()).ok_or("template: missing map")?;
		if rows.len() != height as usize {
			return Err(format!("template: {} rows, height says {height}", rows.len()));
		}
		let mut cells = Vec::with_capacity(width as usize * height as usize);
		for row in rows {
			let row = row.as_array().ok_or("template: a map row is not an array")?;
			if row.len() != width as usize {
				return Err(format!("template: a row has {} cells, width says {width}", row.len()));
			}
			for cell in row {
				let spec = cell.as_str().ok_or("template: a cell is not a string")?;
				if spec.split(',').filter(|p| !p.is_empty()).count() > MAX_LAYERS {
					return Err("template: a cell stacks more layers than the format allows".into());
				}
				cells.push(spec.to_string());
			}
		}
		Ok(Self { name, width, height, uses, cells })
	}

	pub fn load(path: &Path) -> Result<Self, String> {
		let text = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
		let mut t = Self::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
		// The file stem is the canonical name (rename = rename the file).
		if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
			t.name = stem.to_string();
		}
		Ok(t)
	}

	pub fn save(&self, path: &Path) -> Result<(), String> {
		if let Some(dir) = path.parent() {
			std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
		}
		std::fs::write(path, self.save_string()).map_err(|e| format!("write {}: {e}", path.display()))
	}

	/// Which layers a cell spec occupies in `project` — the thumbnail wants
	/// ground over water just like the map.
	pub fn cell_layers(&self, project: &Project, dx: u16, dy: u16) -> [Option<TileRef>; MAX_LAYERS] {
		let mut stack = [None; MAX_LAYERS];
		if dx >= self.width || dy >= self.height {
			return stack;
		}
		let spec = &self.cells[dy as usize * self.width as usize + dx as usize];
		for part in spec.split(',').filter(|p| !p.is_empty()) {
			if let Ok((tile, layer)) = project.resolve_ref(part) {
				stack[layer] = Some(tile);
			}
		}
		stack
	}
}

/// Cut helper: clear the **ground layer** of every selected cell (one undo
/// transaction or part of the open stroke) — the water base stays, exactly
/// like the eraser. Returns whether anything changed.
pub fn clear_selection_ground(project: &mut Project, selection: &Selection) -> bool {
	let mut edits = Vec::new();
	let (w, h) = (project.width, project.height);
	for y in 0..h {
		for x in 0..w {
			if selection.contains(x, y) {
				edits.push((x, y, LAYER_GROUND, None));
			}
		}
	}
	project.place_many(&edits)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::project::LAYER_WATER;
	use crate::selection::SelectMode;
	use std::path::PathBuf;

	fn assets_root() -> PathBuf {
		Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets")
	}

	fn project() -> Project {
		Project::new(16, 12, &["GREEN".to_string()], &assets_root(), 7).unwrap()
	}

	/// Paint an L-shaped ground patch and select part of it.
	fn painted() -> (Project, Selection) {
		let mut p = project();
		let (land, layer) = p.resolve_ref("GLa000").unwrap();
		for (x, y) in [(2u16, 2u16), (3, 2), (4, 2), (2, 3), (2, 4)] {
			p.place(x, y, layer, Some(land));
		}
		let mut s = Selection::new(p.width, p.height);
		s.apply_rect(2, 2, 4, 2, SelectMode::Add);
		s.apply_cell(2, 3, SelectMode::Add);
		(p, s)
	}

	#[test]
	fn capture_keeps_holes_and_round_trips_through_json() {
		let (p, s) = painted();
		let t = Template::capture(&p, &s, "ridge").unwrap();
		assert_eq!((t.width, t.height), (3, 2));
		// (3,3)/(4,3) were never selected → holes.
		assert_eq!(t.cells.iter().filter(|c| c.is_empty()).count(), 2);
		assert!(t.uses.iter().any(|(n, _)| n == "GREEN"));
		assert!(t.uses.iter().any(|(n, _)| n == "WATER"), "water layer rides along");

		let re = Template::from_str(&t.save_string()).unwrap();
		assert_eq!((re.width, re.height), (t.width, t.height));
		assert_eq!(re.cells, t.cells);
		assert_eq!(re.uses, t.uses);
	}

	#[test]
	fn apply_stamps_resolved_tiles_and_skips_holes() {
		let (p, s) = painted();
		let t = Template::capture(&p, &s, "ridge").unwrap();
		let mut target = project();
		let before_hole = target.cell_spec(9, 9).unwrap(); // the hole position
		assert!(t.apply(&mut target, 8, 8).unwrap());
		// Selected cells stamped...
		assert_eq!(target.cell_spec(8, 8).unwrap(), p.cell_spec(2, 2).unwrap());
		assert_eq!(target.cell_spec(10, 8).unwrap(), p.cell_spec(4, 2).unwrap());
		// ...the hole left the target untouched.
		assert_eq!(target.cell_spec(9, 9).unwrap(), before_hole);
		// One undo unit restores everything.
		assert!(target.undo());
		assert_eq!(target.cell_spec(8, 8).unwrap(), before_hole.clone());
	}

	#[test]
	fn apply_clips_at_the_map_edge() {
		let (p, s) = painted();
		let t = Template::capture(&p, &s, "ridge").unwrap();
		let mut target = project();
		// Top-left at the last cell: only (0,0) of the template fits.
		let (lx, ly) = (target.width - 1, target.height - 1);
		assert!(t.apply(&mut target, lx, ly).unwrap());
	}

	#[test]
	fn incompatible_templates_name_the_missing_id() {
		let (mut p, s) = painted();
		let mut t = Template::capture(&p, &s, "ridge").unwrap();
		t.cells[0] = "ZZZ999".to_string();
		assert!(!t.compatible(&p));
		assert_eq!(t.missing_id(&p).as_deref(), Some("ZZZ999"));
		// Apply refuses before touching the document (resolution comes first).
		let rev = p.revision();
		assert!(t.apply(&mut p, 0, 0).is_err());
		assert_eq!(p.revision(), rev);
	}

	#[test]
	fn cut_clears_only_selected_ground() {
		let (mut p, s) = painted();
		let water_before = p.cell(2, 2).unwrap()[LAYER_WATER];
		assert!(clear_selection_ground(&mut p, &s));
		assert!(p.cell(2, 2).unwrap()[LAYER_GROUND].is_none(), "selected ground cleared");
		assert_eq!(p.cell(2, 2).unwrap()[LAYER_WATER], water_before, "water base stays");
		assert!(p.cell(2, 4).unwrap()[LAYER_GROUND].is_some(), "unselected cell kept");
	}
}
