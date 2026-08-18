//! Tile templates: reusable chunks of map (a mountain range, a forest, an
//! oasis) captured from a selection and stamped back anywhere - on any map
//! that uses the same tile packs.
//!
//! A template file is JSON: a `use` manifest naming the packs its tiles come
//! from, and a cell grid in the **project save encoding** (`"WTR005,GSd004:!N"`
//! per cell, layers comma-joined, `""` = a hole). Cells reference tiles by
//! **id**, never by index, so a template survives pack reordering and resolves
//! against whatever pack roster the open map has - [`Template::compatible`]
//! answers whether every id resolves there.
//!
//! Capture takes the selection's bounding box and keeps only selected cells
//! (holes stay holes); apply skips holes, so irregular shapes stamp exactly
//! what was selected. The same struct doubles as the copy/paste clipboard -
//! paste is "apply a transient template".

use std::path::Path;

use crate::pack::Transformable;
use crate::project::{MAX_LAYERS, Project, TileRef, Transform};
use crate::selection::Selection;

/// One captured cell: the save-encoded stack spec (`""` = hole).
type CellSpec = String;

/// A quarter-turn or mirror the transform tool applies to a whole template
/// stamp - the same four ops the toolbox offers for a single tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StampOp {
	Cw,
	Ccw,
	FlipH,
	FlipV,
}

impl StampOp {
	/// The op's name as the command line spells it (`cw`/`ccw`/`flip-h`/`flip-v`).
	pub fn parse(s: &str) -> Option<Self> {
		match s {
			"cw" => Some(Self::Cw),
			"ccw" => Some(Self::Ccw),
			"flip-h" => Some(Self::FlipH),
			"flip-v" => Some(Self::FlipV),
			_ => None,
		}
	}

	/// This op composed onto a tile's existing transform.
	fn applied(self, t: Transform) -> Transform {
		match self {
			Self::Cw => t.rotated_cw(),
			Self::Ccw => t.rotated_ccw(),
			Self::FlipH => t.flipped_h(),
			Self::FlipV => t.flipped_v(),
		}
	}

	fn verb(self) -> &'static str {
		match self {
			Self::Cw | Self::Ccw => "rotated",
			Self::FlipH | Self::FlipV => "flipped",
		}
	}
}

/// May a tile whose family is `kind` carry transform `t`? (The art is only
/// drawn for the orientations the family permits - rotating past that corrupts
/// baked light/shadow.) Sync tiles (water) ride along at a fixed orientation
/// and are handled before this is ever consulted.
pub fn family_allows(kind: Transformable, t: Transform) -> bool {
	match kind {
		Transformable::Free | Transformable::Sync => true,
		Transformable::Invert => !t.mirror && t.rot.is_multiple_of(2),
		Transformable::No => t == Transform::default(),
	}
}

/// One cell spec with `op` applied to each tile in its stack. Sync tiles
/// (water) are kept verbatim - they animate at a fixed orientation. A tile
/// whose family can't take the op aborts the whole transform (`Err` names it).
fn transform_cell(project: &Project, spec: &str, op: StampOp) -> Result<String, String> {
	let mut parts: Vec<String> = Vec::new();
	for part in spec.split(',').filter(|p| !p.is_empty()) {
		let (tref, _) = project.resolve_ref(part)?;
		let kind = project.packs[tref.pack as usize].tile_transformable(tref.tile);
		if kind == Transformable::Sync {
			parts.push(part.to_string());
			continue;
		}
		let next = op.applied(tref.transform);
		if !family_allows(kind, next) {
			let id = part.split(':').next().unwrap_or(part);
			let why = match kind {
				Transformable::Invert => "only rotates 180\u{b0}",
				_ => "can't be transformed",
			};
			return Err(format!("'{id}' {why}, so the stamp can't be {}", op.verb()));
		}
		let id = part.split(':').next().unwrap_or(part);
		parts.push(format!("{id}{}", next.suffix()));
	}
	Ok(parts.join(","))
}

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

	/// Stamp the template with its top-left at `(x, y)` - one undo
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

	/// The template rotated or mirrored as a whole: the footprint turns and
	/// every tile's own transform composes with `op`. Constrained by what the
	/// tiles allow - if any tile's family can't represent the resulting
	/// orientation (a non-rotatable obstruction, an `invert`-only tile asked
	/// for a quarter turn), the op is refused with an error naming it, so the
	/// stamp never bakes a corrupt orientation. Water (sync) rides along at its
	/// fixed orientation and never blocks. Holes stay holes.
	pub fn transformed(&self, project: &Project, op: StampOp) -> Result<Self, String> {
		let (w, h) = (self.width as usize, self.height as usize);
		// Quarter turns swap the footprint; mirrors keep it.
		let (nw, nh) = match op {
			StampOp::Cw | StampOp::Ccw => (h, w),
			StampOp::FlipH | StampOp::FlipV => (w, h),
		};
		let mut cells = vec![String::new(); nw * nh];
		for y in 0..h {
			for x in 0..w {
				let (nx, ny) = match op {
					StampOp::Cw => (h - 1 - y, x),
					StampOp::Ccw => (y, w - 1 - x),
					StampOp::FlipH => (w - 1 - x, y),
					StampOp::FlipV => (x, h - 1 - y),
				};
				cells[ny * nw + nx] = transform_cell(project, &self.cells[y * w + x], op)?;
			}
		}
		Ok(Self { name: self.name.clone(), width: nw as u16, height: nh as u16, uses: self.uses.clone(), cells })
	}

	/// This template (an **identity-orientation base**) brought to the absolute
	/// orientation `target`: mirror first (a horizontal flip), then `target.rot`
	/// clockwise quarter turns - the sequence `oriented` and the transform-tool
	/// share, so the 8-orientation stamp grid can show every orientation from one
	/// base. `Err` (naming the offending tile) if any tile's family can't take
	/// `target`, exactly like [`Self::transformed`].
	pub fn oriented(&self, project: &Project, target: Transform) -> Result<Self, String> {
		let mut t = self.clone();
		if target.mirror {
			t = t.transformed(project, StampOp::FlipH)?;
		}
		for _ in 0..target.rot {
			t = t.transformed(project, StampOp::Cw)?;
		}
		Ok(t)
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
		let rows = crate::project::encode_cell_grid(self.width as usize, self.height as usize, |x, y| {
			self.cells[y * self.width as usize + x].clone()
		});
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

	// An inherent constructor (like `INI::from_str`) - the `FromStr` trait
	// would force callers through `.parse()` for no gain.
	#[allow(clippy::should_implement_trait)]
	pub fn from_str(text: &str) -> Result<Self, String> {
		let root = json::parse(text)?;
		// The display name comes from the JSON `name` (empty if absent; `load`
		// falls back to the file stem then).
		let name = root.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
		let width = root.get("width").and_then(|v| v.as_f64()).ok_or("template: missing width")? as u16;
		let height = root.get("height").and_then(|v| v.as_f64()).ok_or("template: missing height")? as u16;
		crate::project::check_map_size(width, height).map_err(|e| format!("template: {e}"))?;
		let mut uses = Vec::new();
		if let Some(list) = root.get("use").and_then(|v| v.as_array()) {
			for u in list {
				let pack = u.get("name").and_then(|v| v.as_str()).ok_or("template: a use entry has no name")?;
				// The pack names decide the templates subdirectory this template
				// is filed under (`template_pack` joins them), so an imported
				// file would otherwise choose the directory the editor writes to.
				crate::project::check_name_component("template use", pack)?;
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
		// Display uses the JSON `name`; only fall back to the file stem when the
		// file carries no name of its own.
		if t.name.is_empty() {
			if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
				t.name = stem.to_string();
			}
		}
		Ok(t)
	}

	pub fn save(&self, path: &Path) -> Result<(), String> {
		if let Some(dir) = path.parent() {
			std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
		}
		std::fs::write(path, self.save_string()).map_err(|e| format!("write {}: {e}", path.display()))
	}

	/// Which layers a cell spec occupies in `project` - the thumbnail wants
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

/// Clear one `layer` of every selected cell (one undo transaction or part of
/// the open stroke). The active-layer eraser/Delete uses this, so deleting on
/// the water layer drops water exactly like deleting ground drops ground.
/// Returns whether anything changed.
pub fn clear_selection_layer(project: &mut Project, selection: &Selection, layer: usize) -> bool {
	let mut edits = Vec::new();
	let (w, h) = (project.width, project.height);
	for y in 0..h {
		for x in 0..w {
			if selection.contains(x, y) {
				edits.push((x, y, layer, None));
			}
		}
	}
	project.place_many(&edits)
}

/// Clear **every** layer of every selected cell (Shift+Delete) - the cells
/// become true holes, water and ground both gone. One undo transaction (or
/// part of the open stroke). Returns whether anything changed.
pub fn clear_selection(project: &mut Project, selection: &Selection) -> bool {
	let mut edits = Vec::new();
	let (w, h) = (project.width, project.height);
	for y in 0..h {
		for x in 0..w {
			if selection.contains(x, y) {
				for layer in 0..MAX_LAYERS {
					edits.push((x, y, layer, None));
				}
			}
		}
	}
	project.place_many(&edits)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::project::{LAYER_GROUND, LAYER_WATER};
	use crate::selection::SelectMode;
	use std::path::PathBuf;

	fn assets_root() -> PathBuf {
		Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets/tilepacks")
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
	fn transformed_rotates_footprint_and_composes_free_tiles_keeping_water() {
		let p = project();
		// A fresh cell is water only (sync, no suffix) - a real, resolvable spec.
		let water = p.cell_spec(0, 0).unwrap();
		assert!(!water.contains(':'), "water is sync -> no transform suffix");
		// 2-wide, 1-tall row: ground over water, then a pre-rotated ground tile.
		let t = Template {
			name: "row".into(),
			width: 2,
			height: 1,
			uses: vec![("GREEN".into(), String::new()), ("WATER".into(), String::new())],
			cells: vec![format!("{water},GLa000"), "GLa001:E".into()],
		};

		let cw = t.transformed(&p, StampOp::Cw).unwrap();
		// Quarter turn swaps the footprint 2x1 → 1x2.
		assert_eq!((cw.width, cw.height), (1, 2));
		// Old (0,0) → new (0,0): water rides along verbatim; GLa000 (free, default)
		// composes one clockwise quarter turn → rot 1 → `:W` suffix.
		assert_eq!(cw.cells[0], format!("{water},GLa000:W"));
		// Old (1,0) → new (0,1): GLa001:E (= rot 3) + cw = rot 0 → suffix drops.
		assert_eq!(cw.cells[1], "GLa001");

		// Mirrors keep the footprint; the water part still passes through untouched.
		let fh = t.transformed(&p, StampOp::FlipH).unwrap();
		assert_eq!((fh.width, fh.height), (2, 1));
		assert!(fh.cells[1].starts_with(&format!("{water},")), "water unchanged under flip");
	}

	#[test]
	fn transformed_refuses_ops_the_tiles_cant_take() {
		let p = project();
		// GLc has no `transformable` entry → Transformable::No.
		let rock = Template {
			name: "rock".into(),
			width: 1,
			height: 1,
			uses: vec![("GREEN".into(), String::new())],
			cells: vec!["GLc000".into()],
		};
		for op in [StampOp::Cw, StampOp::Ccw, StampOp::FlipH, StampOp::FlipV] {
			let Err(err) = rock.transformed(&p, op) else {
				panic!("{op:?} should be refused for a non-transformable tile");
			};
			assert!(err.contains("GLc000"), "the error names the offending tile: {err}");
		}
		// One non-rotatable tile in an otherwise-free stamp blocks the whole op.
		let mixed = Template {
			name: "mixed".into(),
			width: 2,
			height: 1,
			uses: vec![("GREEN".into(), String::new())],
			cells: vec!["GLa000".into(), "GLc000".into()],
		};
		assert!(mixed.transformed(&p, StampOp::Cw).is_err(), "a single No tile vetoes the stamp rotation");
	}

	/// `oriented` reaches every absolute orientation from an identity base (the
	/// footprint swaps on the odd quarter turns), and `Project::tile_allows`
	/// gates by the tile's family: a Free tile takes any orientation, a No tile
	/// only identity.
	#[test]
	fn oriented_reaches_absolute_orientations_and_tile_allows_gates() {
		let p = project();
		let base = Template {
			name: "b".into(),
			width: 2,
			height: 1,
			uses: vec![("GREEN".into(), String::new())],
			cells: vec!["GLa000".into(), "GLa001".into()],
		};
		for mirror in [false, true] {
			for rot in 0..4u8 {
				let t = Transform { rot, mirror };
				let o = base.oriented(&p, t).unwrap_or_else(|e| panic!("{t:?}: {e}"));
				let (ew, eh) = if rot % 2 == 1 { (1, 2) } else { (2, 1) };
				assert_eq!((o.width, o.height), (ew, eh), "footprint for {t:?}");
			}
		}
		assert_eq!(base.oriented(&p, Transform::default()).unwrap().cells, base.cells, "identity is the base");

		// `oriented` composes with `transformed`: applying an op to an oriented
		// template equals orienting to the op-composed transform - so the grid's
		// absolute orientations and the console's relative ops never diverge.
		let same = |a: &Template, b: &Template| (a.width, a.height, &a.cells) == (b.width, b.height, &b.cells);
		for mirror in [false, true] {
			for rot in 0..4u8 {
				let t = Transform { rot, mirror };
				let cw = base.oriented(&p, t).unwrap().transformed(&p, StampOp::Cw).unwrap();
				assert!(same(&cw, &base.oriented(&p, t.rotated_cw()).unwrap()), "cw compose {t:?}");
				let fh = base.oriented(&p, t).unwrap().transformed(&p, StampOp::FlipH).unwrap();
				assert!(same(&fh, &base.oriented(&p, t.flipped_h()).unwrap()), "flip-h compose {t:?}");
			}
		}

		let gla = p.resolve_ref("GLa000").unwrap().0; // Free
		let glc = p.resolve_ref("GLc000").unwrap().0; // No
		assert!(p.tile_allows(gla.pack, gla.tile, Transform { rot: 1, mirror: true }));
		assert!(p.tile_allows(glc.pack, glc.tile, Transform::default()));
		assert!(!p.tile_allows(glc.pack, glc.tile, Transform { rot: 1, mirror: false }));
	}

	#[test]
	fn stamp_op_parse_speaks_the_command_line_names() {
		assert_eq!(StampOp::parse("cw"), Some(StampOp::Cw));
		assert_eq!(StampOp::parse("ccw"), Some(StampOp::Ccw));
		assert_eq!(StampOp::parse("flip-h"), Some(StampOp::FlipH));
		assert_eq!(StampOp::parse("flip-v"), Some(StampOp::FlipV));
		assert_eq!(StampOp::parse("rot90"), None, "unknown op names are refused");
	}

	/// Invert-only families (DESERT's DLb) take a 180° result but refuse quarter
	/// turns and mirrors, with the "only rotates 180°" reason.
	#[test]
	fn invert_tiles_allow_180_and_refuse_the_rest() {
		let p = Project::new(4, 4, &["DESERT".to_string()], &assets_root(), 7).unwrap();
		let desert = &p.packs[1];
		let tile = desert.group_tiles("DLb")[0];
		let id = desert.ids[tile as usize].clone();
		// From a quarter-turned start (`:W` = one cw turn), one more cw turn
		// reaches 180° - a legal invert orientation.
		let quarter = Template {
			name: "q".into(),
			width: 1,
			height: 1,
			uses: vec![("DESERT".into(), String::new())],
			cells: vec![format!("{id}:W")],
		};
		let turned = quarter.transformed(&p, StampOp::Cw).expect("90°+90° = 180° is allowed for invert");
		assert_eq!(turned.cells[0], format!("{id}:S"), "the composed transform is the 180° suffix");
		// From the base orientation, a single quarter turn (or a mirror) is refused.
		let base = Template { cells: vec![id.clone()], ..quarter.clone() };
		for op in [StampOp::Cw, StampOp::FlipH] {
			let err = base.transformed(&p, op).err().unwrap();
			assert!(err.contains("only rotates 180"), "{op:?} names the invert restriction: {err}");
			assert!(err.contains(&id), "the offending tile is named: {err}");
		}
	}

	/// A selection mask larger than the map captures the off-map cells as holes
	/// instead of panicking (the mask is editor state and may outlive a resize).
	#[test]
	fn capture_treats_offmap_selected_cells_as_holes() {
		let (p, _) = painted();
		let mut wide = Selection::new(p.width + 4, p.height);
		// One on-map painted cell + one past the right edge.
		wide.apply_cell(2, 2, SelectMode::Add);
		wide.apply_cell(p.width + 2, 2, SelectMode::Add);
		let t = Template::capture(&p, &wide, "wide").unwrap();
		assert_eq!((t.width, t.height), (p.width + 1, 1));
		assert_eq!(t.cells[0], p.cell_spec(2, 2).unwrap(), "the on-map cell captured");
		assert!(t.cells.last().unwrap().is_empty(), "the off-map cell became a hole");
	}

	#[test]
	fn capture_of_tile_free_cells_is_an_error() {
		let (mut p, s) = painted();
		assert!(clear_selection(&mut p, &s), "empty the selected cells first");
		let err = Template::capture(&p, &s, "void").err().unwrap();
		assert!(err.contains("empty"), "an all-hole capture is refused: {err}");
	}

	#[test]
	fn from_str_validates_the_grid_shape() {
		// No `use` block is fine (an empty roster); no name defaults to "".
		let bare = Template::from_str(r#"{"width":1,"height":1,"map":[["GLa000"]]}"#).unwrap();
		assert!(bare.uses.is_empty() && bare.name.is_empty());
		assert_eq!(bare.cells, vec!["GLa000".to_string()]);
		// Row count must match the height, row length the width.
		let e = Template::from_str(r#"{"width":1,"height":2,"map":[["a"]]}"#).err().unwrap();
		assert!(e.contains("1 rows, height says 2"), "{e}");
		let e = Template::from_str(r#"{"width":2,"height":1,"map":[["a"]]}"#).err().unwrap();
		assert!(e.contains("1 cells, width says 2"), "{e}");
		// A cell may stack at most MAX_LAYERS parts.
		let e = Template::from_str(r#"{"width":1,"height":1,"map":[["a,b,c"]]}"#).err().unwrap();
		assert!(e.contains("more layers"), "{e}");
	}

	/// `load` keeps the file's own name and only falls back to the file stem
	/// when the JSON carries none.
	#[test]
	fn load_falls_back_to_the_file_stem_only_when_unnamed() {
		let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/mc-cov-template-load");
		let _ = std::fs::remove_dir_all(&dir);
		let unnamed = dir.join("oasis.json");
		let named = dir.join("stemless.json");
		let (p, s) = painted();
		let mut t = Template::capture(&p, &s, "ridge").unwrap();
		t.name = String::new();
		t.save(&unnamed).unwrap();
		assert_eq!(Template::load(&unnamed).unwrap().name, "oasis", "unnamed file -> stem");
		t.name = "Real Name".into();
		t.save(&named).unwrap();
		assert_eq!(Template::load(&named).unwrap().name, "Real Name", "the JSON name wins over the stem");
		std::fs::remove_dir_all(&dir).ok();
	}

	#[test]
	fn cell_layers_out_of_footprint_is_empty() {
		let (p, s) = painted();
		let t = Template::capture(&p, &s, "ridge").unwrap();
		assert_eq!(t.cell_layers(&p, t.width, 0), [None; MAX_LAYERS], "past the footprint -> empty stack");
		assert!(t.cell_layers(&p, 0, 0).iter().any(Option::is_some), "an in-footprint cell resolves");
	}

	#[test]
	fn clear_selection_layer_drops_only_that_layer() {
		let (mut p, s) = painted();
		let water_before = p.cell(2, 2).unwrap()[LAYER_WATER];
		// Clearing ground leaves the water base (the Cut / active-layer=ground path).
		assert!(clear_selection_layer(&mut p, &s, LAYER_GROUND));
		assert!(p.cell(2, 2).unwrap()[LAYER_GROUND].is_none(), "selected ground cleared");
		assert_eq!(p.cell(2, 2).unwrap()[LAYER_WATER], water_before, "water base stays");
		assert!(p.cell(2, 4).unwrap()[LAYER_GROUND].is_some(), "unselected cell kept");
		// Clearing water drops the base too - no land/water distinction.
		assert!(clear_selection_layer(&mut p, &s, LAYER_WATER));
		assert!(p.cell(2, 2).unwrap()[LAYER_WATER].is_none(), "selected water cleared");
	}

	#[test]
	fn clear_selection_empties_every_layer() {
		let (mut p, s) = painted();
		assert!(p.cell(2, 2).unwrap()[LAYER_WATER].is_some(), "starts with a water base");
		assert!(clear_selection(&mut p, &s));
		// A selected cell is now a true hole - both layers gone.
		assert!(p.cell(2, 2).unwrap()[LAYER_GROUND].is_none(), "ground gone");
		assert!(p.cell(2, 2).unwrap()[LAYER_WATER].is_none(), "water gone");
		// An unselected cell keeps its water.
		assert!(p.cell(2, 4).unwrap()[LAYER_WATER].is_some(), "unselected water kept");
	}

	/// Security regression: a template's `use` names become the templates
	/// subdirectory it is filed under (`template_pack` in the app joins them),
	/// and `Template::save` runs `create_dir_all` on that. An imported file
	/// therefore got to choose the directory the editor writes into - relative
	/// (`../../..`) or, worse, absolute, since `Path::join` drops the base when
	/// handed one. Refuse such a name at the parse boundary.
	#[test]
	fn from_str_refuses_a_use_name_that_is_not_one_path_component() {
		let doc = |pack: &str| format!(r#"{{"width":1,"height":1,"use":[{{"name":"{pack}"}}],"map":[["GLa000"]]}}"#);
		// `r"a\\b"` is the JSON escape for a single backslash - written raw so
		// the parser hands the checker `a\b` and not the `\b` control character.
		for pack in ["../../../../tmp/ESCAPED", "/home/u/.config/autostart", "..", ".", "a/b", r"a\\b", ""] {
			let e = Template::from_str(&doc(pack)).err().unwrap_or_else(|| panic!("{pack:?} must be refused"));
			assert!(e.contains("illegal name"), "{pack:?}: {e}");
		}
		// Real names still parse - a WRL-import pack is named after the file
		// stem, so spaces and mixed case are legitimate.
		for pack in ["GREEN", "SNOW_DARK", "My Map", "import_extra"] {
			let t = Template::from_str(&doc(pack)).unwrap_or_else(|e| panic!("{pack:?} must parse: {e}"));
			assert_eq!(t.uses[0].0, pack);
		}
	}
}
