//! Editor state + the single command mutator.
//!
//! `EditorState` owns the document and the viewport; `execute` is the only
//! place either is mutated. GPU-side effects (screenshot, quit) are returned
//! as `Outcome`s for the shell (windowed or headless) to act on.

use std::path::{Path, PathBuf};

use map_core::{
	LAYER_GROUND, LAYER_WATER, PaletteReimport, Project, Rng, SelectMode, Selection, Template, TileKind, TileRef,
	Transform, clear_selection, clear_selection_layer,
};
use max_assets::wrl::{read_wrl_file, read_wrl_header, write_wrl_file};

use crate::command::{Command, FilePurpose, ShoreMode};
use crate::console::Console;
use crate::menu::{self, MenuBar};
use crate::minimap;
use crate::palette::PaletteCycler;
use crate::picker::{self, PickerState};
use crate::render::{TILE_PX, Uniforms};
use crate::workspace::{LayoutGroup, Workspace, WorkspaceLayout};

mod doc_tabs;
mod scenery_authoring;
#[cfg(test)]
mod tests;

const ZOOM_MIN: f32 = 0.0625;
const ZOOM_MAX: f32 = 8.0;

/// Read a PNG's dimensions from its header only - no pixel decode (the
/// New-from-Image modal opens instantly; pixels load at Convert).
fn png_dimensions(path: &Path) -> Result<(u32, u32), String> {
	let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
	let reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().map_err(|e| e.to_string())?;
	let info = reader.info();
	Ok((info.width, info.height))
}

/// Write tightly-packed RGBA8 to an 8-bit PNG (the Tile Painter's export).
fn write_tile_png(path: &Path, rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
	let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
	let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
	encoder.set_color(png::ColorType::Rgba);
	encoder.set_depth(png::BitDepth::Eight);
	let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
	writer.write_image_data(rgba).map_err(|e| e.to_string())
}

/// Write one byte per pixel to an 8-bit greyscale PNG - a **height map** on its
/// way out to be painted on. One channel, because that is what a height map is:
/// anything wider would invite a paint program to store a colour nobody could
/// read back as an elevation.
fn write_grey_png(path: &Path, grey: &[u8], width: u32, height: u32) -> Result<(), String> {
	let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
	let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
	encoder.set_color(png::ColorType::Grayscale);
	encoder.set_depth(png::BitDepth::Eight);
	let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
	writer.write_image_data(grey).map_err(|e| e.to_string())
}

/// The palette index whose RGB is visually closest to `(r, g, b)` by squared
/// distance. Slot 0 (the transparent/mask slot) is skipped so an opaque pixel
/// never silently maps to "transparent"; transparency is handled by the caller.
fn nearest_palette_index(palette: &[u8], r: u8, g: u8, b: u8) -> u8 {
	let (mut best, mut best_d) = (1u8, u32::MAX);
	for i in 1..=255u8 {
		let o = i as usize * 3;
		let (dr, dg, db) =
			(palette[o] as i32 - r as i32, palette[o + 1] as i32 - g as i32, palette[o + 2] as i32 - b as i32);
		let d = (dr * dr + dg * dg + db * db) as u32;
		if d < best_d {
			best_d = d;
			best = i;
		}
	}
	best
}

/// Decode an 8-bit PNG to tightly-packed RGBA8 + dimensions.
/// Handles RGB/RGBA/grayscale/indexed 8-bit sources (the formats `png` emits
/// without 16-bit depth); other inputs are converted offline.
fn decode_png_rgba(path: &Path) -> Result<(Vec<u8>, u32, u32), String> {
	let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
	let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().map_err(|e| e.to_string())?;
	let mut buf = vec![0; reader.output_buffer_size().ok_or("png: image too large")?];
	let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
	if info.bit_depth != png::BitDepth::Eight {
		return Err(format!("{:?} PNG unsupported - re-export as 8-bit", info.bit_depth));
	}
	let src = &buf[..info.buffer_size()];
	let (w, h) = (info.width, info.height);
	let px = (w as usize) * (h as usize);
	let mut rgba = Vec::with_capacity(px * 4);
	match info.color_type {
		png::ColorType::Rgba => rgba.extend_from_slice(src),
		png::ColorType::Rgb => {
			for p in src.chunks_exact(3) {
				rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
			}
		}
		png::ColorType::Grayscale => {
			for &g in src {
				rgba.extend_from_slice(&[g, g, g, 255]);
			}
		}
		png::ColorType::GrayscaleAlpha => {
			for p in src.chunks_exact(2) {
				rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
			}
		}
		png::ColorType::Indexed => {
			let pal = reader.info().palette.as_ref().ok_or("indexed PNG without a palette")?;
			let trns = reader.info().trns.as_ref();
			for &i in src {
				let at = i as usize * 3;
				// The `png` crate hands back raw indices; a crafted file can
				// point past its own PLTE palette, so bounds-check rather
				// than index (which would panic).
				let rgb = pal.get(at..at + 3).ok_or("indexed PNG: a pixel references a color outside the palette")?;
				let a = trns.and_then(|t| t.get(i as usize)).copied().unwrap_or(255);
				rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], a]);
			}
		}
	}
	Ok((rgba, w, h))
}

/// Classify an RGBA image into a per-tile land mask (`true` = land, row-major
/// `w×h`) for use as a New Map shape template. A pixel reads as **water** when
/// blue is its dominant channel ("most blue color is water"); everything else
/// is land. Each tile samples the image region it covers (backward-mapped, so
/// images larger *or* smaller than the map both resample cleanly) and takes the
/// majority vote; ties default to water - the new map's base fill - so only a
/// clear land majority carves land.
fn shape_land_mask(rgba: &[u8], iw: u32, ih: u32, w: u16, h: u16) -> Vec<bool> {
	let (w, h) = (w as usize, h as usize);
	let (iw, ih) = (iw as usize, ih as usize);
	if w == 0 || h == 0 || iw == 0 || ih == 0 || rgba.len() < iw * ih * 4 {
		return vec![false; w * h];
	}
	let mut mask = vec![false; w * h];
	for ty in 0..h {
		let py0 = ty * ih / h;
		let py1 = ((ty + 1) * ih / h).max(py0 + 1).min(ih); // always ≥ 1 row
		for tx in 0..w {
			let px0 = tx * iw / w;
			let px1 = ((tx + 1) * iw / w).max(px0 + 1).min(iw); // always ≥ 1 col
			let (mut land, mut total) = (0u32, 0u32);
			for py in py0..py1 {
				let row = py * iw * 4;
				for px in px0..px1 {
					let at = row + px * 4;
					let (r, g, b) = (rgba[at] as u16, rgba[at + 1] as u16, rgba[at + 2] as u16);
					total += 1;
					if !(b > r && b > g) {
						land += 1;
					}
				}
			}
			mask[ty * w + tx] = land * 2 > total; // tie (incl. all-equal) → water
		}
	}
	mask
}

/// Decode the modal's image and build a conversion session from its settings -
/// the conversion's first stage, shared by the stepped and synchronous
/// (`convert`) paths.
/// Decode `path` and build a conversion session for `opts`.
fn build_convert_session(path: &Path, opts: map_core::ConvertOpts) -> Result<map_core::ConvertSession, String> {
	let (rgba, w, h) = decode_png_rgba(path)?;
	map_core::ConvertSession::new(rgba, w, h, opts)
}

/// Viewport: pan in world px (top-left), zoom in screen px per world px.
pub struct View {
	pub pan: [f32; 2],
	pub zoom: f32,
}

impl View {
	pub fn fit(map_tiles: (u16, u16), screen_w: f32, screen_h: f32) -> Self {
		Self::fit_rect(map_tiles, (0.0, 0.0, screen_w, screen_h))
	}

	/// Fit the map into a screen-space rect (the workspace's center area -
	/// docked panels don't cover a fitted map). `(x, y, w, h)` in px.
	pub fn fit_rect(map_tiles: (u16, u16), r: (f32, f32, f32, f32)) -> Self {
		let map_px = [map_tiles.0 as f32 * TILE_PX as f32, map_tiles.1 as f32 * TILE_PX as f32];
		let zoom = (r.2 / map_px[0]).min(r.3 / map_px[1]);
		// World w under screen point s satisfies w = s / zoom + pan; put the
		// map's center under the rect's center.
		Self { pan: [map_px[0] / 2.0 - (r.0 + r.2 / 2.0) / zoom, map_px[1] / 2.0 - (r.1 + r.3 / 2.0) / zoom], zoom }
	}

	/// Multiply zoom keeping the world point under `(sx, sy)` stationary.
	pub fn zoom_at(&mut self, sx: f32, sy: f32, factor: f32) {
		let new_zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
		self.pan[0] += sx / self.zoom - sx / new_zoom;
		self.pan[1] += sy / self.zoom - sy / new_zoom;
		self.zoom = new_zoom;
	}
}

/// The active map tool - what LMB does on the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
	/// Paint the active tile.
	Pencil,
	/// Eyedropper: pick a cell's top tile as the brush.
	Picker,
	/// Erase the topmost layer of painted cells.
	Eraser,
	/// Flood-fill the connected same-tile region with the active tile.
	Fill,
	/// Free-hand terrain brush: paint a land/water mask (the active material is
	/// `mask_water`). Drag lays flat land or water; the coast (beach + animated
	/// coastal waves) grows over the stroke when it's released.
	PaintMask,
	/// Stamp a unit preview at the clicked cell (Units panel - palette aid).
	/// Also the object-editor's **Place** tool (save editing, S3).
	Unit,
	/// Remove the unit preview on the clicked cell. Also the object-editor's
	/// **Delete** tool.
	UnitEraser,
	/// Object **Select**: click picks the topmost object at a cell (footprint +
	/// z aware) and highlights it (save editing, S3.2).
	ObjSelect,
	/// Object **Pick** (eyedropper): click arms the unit type + team under the
	/// cursor for placing (like the tile eyedropper, but for objects).
	ObjPick,
	/// Object **Move**: drag an object to a new cell (undoable, collision-aware).
	ObjMove,
	/// Object **Clone** (clone stamp): click an object to take it as the source -
	/// type, team and every per-unit property - then click bare cells to stamp
	/// copies of it.
	ObjClone,
	/// Freehand cell selection: drag paints the mask (Shift adds,
	/// Ctrl subtracts, plain drag starts fresh).
	Select,
	/// Rectangle selection: drag spans a rect, applied on release (same
	/// modifier logic).
	SelectRect,
	/// Place the armed scenery cut-out where the pointer is (SCENERY.md D).
	/// Free-positioned: the click's map pixel is the footprint origin, not a
	/// cell.
	Scenery,
	/// Drag a placed scenery object to a new pixel position (one undo unit per
	/// drag).
	SceneryMove,
	/// Remove the topmost scenery object under the pointer.
	SceneryEraser,
	/// Resource brush (save editing, S5.3): drag paints the current resource
	/// material / amount into the cargo map (mode = set / add / subtract), one
	/// stroke = one undo unit. Only meaningful with a save open.
	ResourceBrush,
}

impl Tool {
	/// The `tool NAME` word this tool is selected by — the canonical alias of the
	/// several [`Command::ToolSelect`] accepts, so a tool the editor picks for
	/// itself (`tool default`) echoes the same word a user would have typed.
	pub fn slug(self) -> &'static str {
		match self {
			Tool::Pencil => "pencil",
			Tool::Picker => "picker",
			Tool::Eraser => "eraser",
			Tool::Fill => "fill",
			Tool::PaintMask => "paint-mask",
			Tool::Unit => "unit",
			Tool::UnitEraser => "unit-eraser",
			Tool::ObjSelect => "obj-select",
			Tool::ObjPick => "obj-pick",
			Tool::ObjMove => "obj-move",
			Tool::ObjClone => "obj-clone",
			Tool::Scenery => "scenery",
			Tool::SceneryMove => "scenery-move",
			Tool::SceneryEraser => "scenery-eraser",
			Tool::Select => "select",
			Tool::SelectRect => "select-rect",
			Tool::ResourceBrush => "resource-brush",
		}
	}
}

/// The Scenery edit layer (the Layers menu).
///
/// It sits in the same `active_layer` slot as the two tile layers because it is
/// the same choice to the user - *what am I editing* - but it is deliberately
/// **not** a tile-layer index: the cut-outs are a free-placed list, not a cell
/// stack, so this value is `MAX_LAYERS` and every tile path routes through
/// [`EditorState::tile_layer`] rather than reading `active_layer` raw. Picking
/// it re-points the three tools the toolbox already has (see
/// [`scenery_twin`]) instead of adding any.
pub const LAYER_SCENERY: usize = map_core::MAX_LAYERS;

/// The Scenery-layer twin of a terrain tool, and back again ([`terrain_twin`]).
///
/// Selecting a layer never adds a tool - it re-points the ones already on the
/// toolbox: the pencil drops a cut-out, the eraser removes one, the arrow drags
/// one. Everything else (fill, the terrain brushes, rect select, the unit and
/// object tools) has no scenery meaning and is left exactly as it is.
fn scenery_twin(tool: Tool) -> Tool {
	match tool {
		Tool::Pencil => Tool::Scenery,
		Tool::Eraser => Tool::SceneryEraser,
		Tool::Select => Tool::SceneryMove,
		other => other,
	}
}

/// The top-left cell a multi-cell chunk lands on when its footprint is
/// **centred** on cell `(x, y)` - the rule for placing chunks (a paste, a
/// template stamp), matching the pencil brush, which is centred on the cursor
/// too. Saturates at the map's top and left edges, where a centred footprint
/// would run off (the right and bottom edges clip instead, in
/// [`map_core::Template::apply`]).
pub fn stamp_origin(t: &Template, x: u16, y: u16) -> (u16, u16) {
	(x.saturating_sub(t.width / 2), y.saturating_sub(t.height / 2))
}

/// The terrain twin of a scenery tool - [`scenery_twin`] inverted, run when the
/// active layer goes back to water or ground so the toolbox key that is lit
/// stays the one the user pressed.
fn terrain_twin(tool: Tool) -> Tool {
	match tool {
		Tool::Scenery => Tool::Pencil,
		Tool::SceneryEraser => Tool::Eraser,
		Tool::SceneryMove => Tool::Select,
		other => other,
	}
}

/// How full the WRL tile budget has to get before an export says so. Far enough
/// below the ceiling that a map still has room to lose some scenery, close
/// enough that an ordinary map never trips it (the originals bake ~1-2k tiles of
/// 65,535).
const BUDGET_WARN_PERCENT: usize = 80;

/// How the resource brush ([`Tool::ResourceBrush`], S5.3) combines its amount
/// with a cell's existing value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceMode {
	/// Replace the cell with the brush material + amount.
	Set,
	/// Raise the cell's amount by the brush amount (capped at 31), setting the
	/// material to the brush's.
	Add,
	/// Lower the cell's amount by the brush amount (down to 0 = cleared), keeping
	/// the cell's own material.
	Sub,
}

impl ResourceMode {
	/// The lowercase command/UI slug.
	pub fn slug(self) -> &'static str {
		match self {
			ResourceMode::Set => "set",
			ResourceMode::Add => "add",
			ResourceMode::Sub => "sub",
		}
	}

	/// Parse a mode slug (`set`/`add`/`sub`).
	pub fn from_slug(s: &str) -> Option<Self> {
		match s {
			"set" => Some(ResourceMode::Set),
			"add" => Some(ResourceMode::Add),
			"sub" | "subtract" => Some(ResourceMode::Sub),
			_ => None,
		}
	}

	/// Combine `amount` of `material` (None = erase) with a cell's current cargo
	/// `cur` under this mode → the new cargo value (survey bits preserved). Set
	/// replaces; Add raises the amount (capped 31) and sets the brush material;
	/// Sub lowers it (0 = cleared), keeping the cell's own material.
	pub fn apply(self, cur: u16, material: Option<max_assets::save::CargoMaterial>, amount: u16) -> u16 {
		use max_assets::save::{cargo_amount, cargo_compose, cargo_material, cargo_surveyed};
		let Some(mat) = material else {
			return cargo_compose(cur, None, 0); // erase, regardless of mode
		};
		let value = match self {
			ResourceMode::Set => cargo_compose(cur, Some(mat), amount),
			ResourceMode::Add => cargo_compose(cur, Some(mat), (cargo_amount(cur) + amount).min(31)),
			ResourceMode::Sub => {
				let new_amt = cargo_amount(cur).saturating_sub(amount);
				let m = if new_amt == 0 { None } else { cargo_material(cur).or(Some(mat)) };
				cargo_compose(cur, m, new_amt)
			}
		};
		// A painted resource is marked surveyed by all players so it's usable
		// in-game; a Sub that emptied the cell stays empty (S5.5).
		cargo_surveyed(value)
	}
}

/// Editor mode (Mode menu) - what the map surface edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
	/// Tile painting - pencil/picker/transform/shore.
	Map,
	/// Pass Table editing - LMB sets the *tile's* passability, so every cell
	/// sharing that tile id retints at once (passability is tile-dependent).
	Pass,
	/// Local Pass Override editing - LMB sets a *per-cell* override on top of
	/// the tile's passability (eraser clears it).
	LocalPass,
	/// Save-file editing (experimental). The map surface behaves like [`Map`];
	/// the mode exists so the save editor gets its own dock layout. Reached via
	/// Mode ▸ Experimental ▸ Save Editor.
	SaveEditor,
}

impl EditorMode {
	/// Which dock-layout group this mode uses. Map has the main layout, the two
	/// pass editors share one, and the save editor has its own.
	pub fn layout_group(self) -> LayoutGroup {
		match self {
			EditorMode::Map => LayoutGroup::Main,
			EditorMode::Pass | EditorMode::LocalPass => LayoutGroup::Pass,
			EditorMode::SaveEditor => LayoutGroup::Save,
		}
	}

	/// The tool this mode falls back to when the armed one stops making sense —
	/// **its own select tool**, because selecting is the one gesture that edits
	/// nothing. Cell selection in the map/pass editors, object selection in the
	/// save editor: same intent, different domain. Reached through
	/// `tool default`, so no caller has to know which is which.
	pub fn default_tool(self) -> Tool {
		match self {
			EditorMode::Map | EditorMode::Pass | EditorMode::LocalPass => Tool::Select,
			EditorMode::SaveEditor => Tool::ObjSelect,
		}
	}

	/// Whether `tool` is one of this mode's own — i.e. some visible toolbox in
	/// this mode offers it. A tool that is *not* reads as "no tool selected":
	/// nothing lights, and the map does something the mode's UI never offered.
	/// [`Command::Mode`] reverts such a tool to [`default_tool`](Self::default_tool).
	///
	/// The object place/erase pair is shared: the Units panel arms it as a map
	/// annotation aid, and the Save Toolbox as its place/delete tools.
	pub fn owns_tool(self, tool: Tool) -> bool {
		match self {
			// The terrain toolbox's keys (its "pass type" group moved out to the
			// Pass Types Palette, which offers no tool of its own — so the two pass
			// editors keep the map's set; the eraser is what clears an override).
			EditorMode::Map | EditorMode::Pass | EditorMode::LocalPass => matches!(
				tool,
				Tool::Pencil
					| Tool::Picker | Tool::Eraser
					| Tool::Fill | Tool::PaintMask
					| Tool::Select | Tool::SelectRect
					| Tool::Unit | Tool::UnitEraser
			),
			// The Save Toolbox's keys.
			EditorMode::SaveEditor => matches!(
				tool,
				Tool::ObjSelect
					| Tool::ObjPick | Tool::ObjMove
					| Tool::ObjClone
					| Tool::Unit | Tool::UnitEraser
					| Tool::ResourceBrush
			),
		}
	}
}

/// Brush footprint shape, paired with the brush size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrushShape {
	Square,
	Circle,
}

/// What the terrain brush ([`Tool::PaintMask`]) does to the coast when a stroke
/// is released: leave it raw, or auto-shore the painted region one way or the
/// other (placement only - the heavy fixes live in the Fix Shore dialog).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrushShore {
	/// Leave the painted land/water raw (no coast).
	Off,
	/// Sweep auto-shore the painted region (uniform coast).
	Sweep,
	/// Loop-walk auto-shore the painted region (varied coast).
	LoopWalk,
}

/// Pass-type swatch colors (simple-wrl-editor parity), straight RGBA:
/// 0 land, 1 water, 2 shore, 3 blocked.
pub const PASS_COLORS: [[f32; 4]; 4] = [
	[0.224, 0.710, 0.290, 1.0], // #39b54a land
	[0.110, 0.553, 0.843, 1.0], // #1c8dd7 water
	[0.843, 0.800, 0.184, 1.0], // #d7cc2f shore
	[0.843, 0.188, 0.188, 1.0], // #d73030 blocked
];
pub const PASS_LABELS: [&str; 4] = ["land", "water", "shore", "block"];

/// One console line for a finished generation run - shared by the
/// scripted `generate` command and the modal's live run.
fn generate_report(p: &map_core::GenParams, s: &map_core::GenStats) -> String {
	let sym = match p.symmetry {
		map_core::Symmetry::None => String::new(),
		other => format!(" [{}]", other.label()),
	};
	format!(
		"generate {}{sym}: seed {} - {} water / {} land, {} obstruction / {} decoration cells, {} shore tiles{}",
		p.generator.name(),
		p.seed,
		s.water,
		s.land,
		s.obstructions,
		s.decorations,
		s.shore,
		match s.unresolved {
			0 => String::new(),
			n => format!(" ({n} seams left - run Auto Fix Shore)"),
		},
	)
}

/// The same report split into short lines for the Generate modal - a single
/// line gets cropped by the dialog width, so each fact gets its own row and
/// the dialog grows to fit (the seed line stays first: it's what you copy to
/// re-make the map).
fn generate_status_lines(p: &map_core::GenParams, s: &map_core::GenStats) -> Vec<String> {
	let sym = match p.symmetry {
		map_core::Symmetry::None => String::new(),
		other => format!(" [{}]", other.label()),
	};
	let mut lines = vec![
		format!("{}{sym}: seed {}", p.generator.name(), p.seed),
		format!("{} water / {} land cells", s.water, s.land),
		format!("{} obstructions, {} decorations", s.obstructions, s.decorations),
		format!("{} shore tiles", s.shore),
	];
	if s.unresolved > 0 {
		lines.push(format!("{} seams left - run Auto Fix Shore", s.unresolved));
	}
	lines
}

/// Shell-level effects of a command.
pub enum Outcome {
	Ok,
	Redraw,
	/// The document was replaced (`open`) - renderer must be rebuilt.
	DocReplaced,
	Screenshot {
		path: PathBuf,
		crop: Option<(u32, u32, u32, u32)>,
		resize: Option<(u32, u32)>,
	},
	/// Open a wgpu-ui overlay dialog (shell-routed; see [`DialogRequest`]).
	OpenDialog(DialogRequest),
	Quit,
	Failed(String),
}

/// A wgpu-ui overlay dialog for the shell to open. Travels as an [`Outcome`]
/// so every opener - menu click, console command, script - goes through
/// `execute` and the shell routes the request in `App::act_on` (headless runs
/// have no overlay and drop it harmlessly).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogRequest {
	About,
	/// The Map Metadata form. `save_after` marks the first-save flow: a
	/// never-saved map's Save-As prompts for metadata first, and the dialog's
	/// Save resumes the file dialog (Cancel abandons the save).
	Metadata {
		save_after: bool,
	},
	/// The New Map form; `shape` carries the land/water PNG when opened via
	/// File → New Terrain from Image (Create then carves it in).
	NewMap {
		shape: Option<PathBuf>,
	},
	Resize,
	PaletteSave,
	PaletteRename,
	PaletteDelete,
	/// The Save / Discard / Cancel unsaved-changes guard. `quit` picks the
	/// fired command pair (`save-and-quit`/`quit!` vs
	/// `save-and-close`/`close-project!`); `prompt` names what's unsaved.
	ConfirmClose {
		quit: bool,
		prompt: String,
	},
	/// Remove Duplicate Templates: the duplicate names to confirm removing
	/// (Remove fires `template-dedupe!`; empty = an acknowledgement).
	DedupeTemplates {
		names: Vec<String>,
	},
	/// Delete Template: the selected template's name, footprint, and a composed
	/// RGBA thumbnail `(rgba, w_px, h_px)`; Delete fires `template-delete!`.
	DeleteTemplate {
		name: String,
		footprint: (u16, u16),
		preview: (Vec<u8>, u32, u32),
	},
	/// Rename Template: the source name, footprint, sibling names (collision
	/// check), and a composed thumbnail; Save fires `template-rename "from" "to"`.
	RenameTemplate {
		from: String,
		footprint: (u16, u16),
		existing: Vec<String>,
		preview: (Vec<u8>, u32, u32),
	},
	/// New Scenery (author a cut-out from an image); the run's live state is
	/// [`EditorState::scenerypaint`] and the dialog owns the derived piece.
	SceneryNew,
	/// Delete Scenery: the armed piece's pack, id, display name, and how many
	/// times it is placed on the open map (deleting it makes those inert).
	/// Delete fires `scenery-delete!`.
	DeleteScenery {
		pack: String,
		id: String,
		name: String,
		placed: usize,
	},
	/// Rename Scenery: the armed piece's pack, id and current display name.
	/// Save fires `scenery-rename "to"`.
	RenameScenery {
		pack: String,
		id: String,
		from: String,
	},
	/// The (non-blocking) Fix Shore window; its live run state is
	/// [`EditorState::autofix`], synced into the dialog by the shell per frame.
	AutoFix,
	/// Convert to Compatible Palette (a WRL import's internal palette); the
	/// rasterize run's live state is [`EditorState::pconvert`].
	ConvertPalette,
	/// New from Image (PNG → tiles); the conversion's live state is
	/// [`EditorState::newimage`].
	NewFromImage,
	/// Generate Random Terrain (a non-blocking float over the live map); the
	/// run's live state is [`EditorState::genrun`].
	Generate,
	/// Import WRL (pack picker → unmapped review); the parked import is
	/// [`EditorState::wrlimport`].
	ImportWrl,
	/// The Tile Painter (New/Clone/Edit context is [`EditorState::tilepaint`];
	/// the dialog owns the working canvas and tool state).
	TilePaint,
	/// The UI Tests font/raster probe (DEV): a dismiss-only diagnostic that
	/// derives everything it shows from the theme and the UI scale, so it
	/// carries nothing.
	UiTests,
	/// The Edit Tile Match Data editor (DEV): the staged model is parked in
	/// [`EditorState::matchedit_stage`] for the shell to hand to the dialog
	/// (which owns it; Save returns self-contained
	/// [`crate::matcheditor::PackCommit`]s).
	MatchEdit,
	/// The resource brush's exact-amount entry (S5.4): a one-field modal seeded
	/// with the current brush amount; OK runs `resource-brush amount N`.
	ResourceAmount,
	/// Experimental-feature warning shown *before* the Open Save File picker: the
	/// save editor can break real saves. Cancel / I Understand — confirming runs
	/// `file-dialog open-save`.
	ConfirmExperimentalOpenSave,
	/// Save-open confirm (swapped map): the installed map at the slot didn't fit
	/// but the pristine stock world did (dimensions match). Abort / Open Anyway —
	/// Open Anyway runs `open-save-anyway`.
	ConfirmOpenSave {
		message: String,
	},
	/// Save-open error (dimension mismatch / unresolvable world): a dismiss-only
	/// notice with a single **Abort** button.
	OpenSaveError {
		message: String,
	},
	/// Editor Preferences: the M.A.X. + M.A.X. Port folder paths (with Browse
	/// pickers) and a "don't ask again" toggle.
	EditorPreferences,
	/// Edit Save Data (Edit > Experimental): the tabbed non-map settings form,
	/// pre-extracted so the overlay never reaches into editor state.
	EditSaveData(Box<crate::savedata::SaveDataInit>),
}

/// The live Fix Shore run — owned by [`EditorState`] (not the dialog), so the
/// stepping (`autofix_tick`) works with or without a window; the Fix Shore
/// overlay is a pure view the shell syncs from this each frame.
pub struct FixRun {
	pub running: bool,
	/// The live session for the current pass (the run loops passes).
	pub session: Option<map_core::FixSession>,
	/// Cumulative cells changed across every pass (placement + fixes).
	pub total_changed: usize,
	/// Faithful defect count after the first placement (the baseline).
	pub found: usize,
	pub fixed: usize,
	pub remaining: usize,
	/// Lowest defect count seen, passes completed, and stalled passes - the
	/// multi-pass loop's convergence bookkeeping.
	pub best: usize,
	pub passes: u32,
	pub stall: u32,
	pub elapsed: f32,
	/// Cells changed once a run finishes (Stop / converged). `None` after Abort.
	pub applied: Option<usize>,
	/// The map region the run is confined to - the active selection's bounds when
	/// the run opened (`None` = the whole map). Every shore pass runs within it.
	pub region: Option<(u16, u16, u16, u16)>,
}

impl FixRun {
	/// Idle, seeded with the initial broken-seam count and the run's region
	/// (the active selection, or `None` for the whole map).
	pub fn new(found: usize, region: Option<(u16, u16, u16, u16)>) -> Self {
		Self {
			running: false,
			session: None,
			total_changed: 0,
			found,
			fixed: 0,
			remaining: found,
			best: usize::MAX,
			passes: 0,
			stall: 0,
			elapsed: 0.0,
			applied: None,
			region,
		}
	}
}

/// A parked WRL import — state-owned; the Import WRL dialog is a view. `Some`
/// while the dialog is open: the settings stage picks packs, then `wrl_match`
/// parks the heavy [`map_core::WrlImport`] in `result` for the unmapped-review
/// stage, and `wrl_finish` commits it.
pub struct WrlImportRun {
	pub path: PathBuf,
	/// The WRL's base name (the converted project + extras pack id).
	pub name: String,
	/// (width, height, tile_count) from the WRL header.
	pub info: (u16, u16, u16),
	/// The parked match result (the dialog shows the unmapped review while set).
	pub result: Option<map_core::WrlImport>,
	/// One display row per unmapped tile (id · class · cell count).
	pub rows: Vec<String>,
	pub matched: usize,
	pub used: usize,
}

/// A save-editor open that decoded on the *fallback* map (the pristine stock
/// world) after the installed map at the slot didn't fit — parked here while the
/// "Open Anyway" confirm dialog is up. Committing it (`open-save-anyway`) hands
/// the ready project to `add_doc`; aborting drops it.
pub struct PendingSaveOpen {
	/// The fully-built, save-attached, named project ready to become a tab.
	pub project: Project,
	/// The console inventory line to echo when it's committed.
	pub summary: String,
}

/// The terrain class of a pass byte, for the unmapped-tile rows.
fn class_name(pass: u8) -> &'static str {
	match pass {
		1 => "water",
		2 => "shore",
		3 => "blocked",
		_ => "land",
	}
}

/// The live terrain generation run — state-owned; the Generate dialog is a
/// synced view. `Some` while the dialog is open (it stays open across runs so
/// seeds can be rerolled).
#[derive(Default)]
pub struct GenerateRun {
	pub running: bool,
	pub session: Option<map_core::GenSession>,
	/// The parameters the last run started with (its rolled seed is what
	/// Copy Seed copies).
	pub started: Option<map_core::GenParams>,
	/// The report lines shown once a run finishes (seed / counts / shore).
	pub status: Vec<String>,
}

/// The live New-from-Image conversion — state-owned; the dialog is a synced
/// view. `Some` from Convert until it finishes (opens the new tab), aborts
/// (back to settings, kept), or the dialog closes.
pub struct NewImageRun {
	/// The image file (pixels are decoded on the first tick).
	pub path: PathBuf,
	/// The base name for the converted project.
	pub name: String,
	/// The validated settings the run started with.
	pub opts: map_core::ConvertOpts,
	pub session: Option<map_core::ConvertSession>,
	pub running: bool,
	pub progress: f32,
	pub stage: String,
	pub elapsed: f32,
}

/// The live rasterize palette conversion — state-owned; the Convert Palette
/// overlay dialog is a view the shell syncs from this each frame. `Some` from
/// Convert until the dialog closes or the document swaps in (finish drops it).
pub struct PaletteConvertRun {
	pub running: bool,
	pub session: Option<PaletteReimport>,
	pub progress: f32,
	pub stage: String,
	pub elapsed: f32,
	/// The options the run started with (validated by the dialog).
	pub water: bool,
	pub relaxed: bool,
	/// Relaxed similarity threshold as a fraction (0..=1).
	pub threshold: f32,
}

/// A fresh random seed from the wall clock (nanos since the epoch), 0 if the
/// clock is before the epoch. Used wherever a generate/new map needs a seed
/// the caller didn't pin (interactive default); scripts pass one explicitly.
fn roll_seed() -> u64 {
	std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

/// Collect every `*.json` file under `dir` (recursively) into `out` - the match
/// editor's id-rename cascade target set (maps + templates).
fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) {
	let Ok(rd) = std::fs::read_dir(dir) else { return };
	for entry in rd.flatten() {
		let path = entry.path();
		if path.is_dir() {
			collect_json_files(&path, out);
		} else if path.extension().is_some_and(|e| e == "json") {
			out.push(path);
		}
	}
}

/// Guard a pass value to the editor's 0..=3 range; `Some(Failed)` (naming the
/// offending `verb`) when out of range, `None` when it's fine.
fn check_pass(value: u8, verb: &str) -> Option<Outcome> {
	(value > 3).then(|| Outcome::Failed(format!("{verb}: value is 0..3")))
}

/// Which directory a file dialog for `purpose` should open in (pure policy, no
/// rfd). Save destinations are created on first use so the dialog always has
/// somewhere to land: palettes → `user/palettes`; templates → the user
/// templates dir; maps → the open doc's folder, else MaxPath / `assets/maps`
/// (Load) or `user/maps` (Save). `doc_path` is the active document's path,
/// `max_path` the configured game directory, `user_templates` the user's saved-
/// templates dir.
fn dialog_default_dir(
	purpose: FilePurpose,
	resources_root: &Path,
	doc_path: Option<&Path>,
	max_path: Option<&Path>,
	max_port_path: Option<&Path>,
	user_templates: Option<&Path>,
) -> PathBuf {
	use FilePurpose::*;
	match purpose {
		// Saved games live in the M.A.X. Port directory (open + export there).
		OpenSave | ExportSave | ExportWrlAndSave => max_port_path
			.filter(|p| p.is_dir())
			.map(Path::to_path_buf)
			.unwrap_or_else(|| resources_root.join("assets/maps")),
		LoadPalette | SavePalette | ImportPalette | ExportPalette => {
			let dir = resources_root.join("user/palettes");
			let _ = std::fs::create_dir_all(&dir);
			dir
		}
		ImportTemplate | ExportTemplate | ExportTemplatePng => match user_templates {
			Some(d) => {
				let _ = std::fs::create_dir_all(d);
				d.to_path_buf()
			}
			None => PathBuf::from("."),
		},
		_ => {
			let fallback = match purpose {
				Load => max_path
					.filter(|p| p.is_dir())
					.map(Path::to_path_buf)
					.or_else(|| Some(resources_root.join("assets/maps"))),
				_ => {
					// User-written maps live beside the other user content, not at
					// the resources root (which is the shipped, tracked tree).
					let maps = resources_root.join("user/maps");
					let _ = std::fs::create_dir_all(&maps);
					Some(maps)
				}
			};
			doc_path.and_then(Path::parent).map(Path::to_path_buf).or(fallback).unwrap_or_else(|| PathBuf::from("."))
		}
	}
}

/// A write failure, phrased for the error dialog. A bare "Access is denied.
/// (os error 5)" reads as a crash; a protected destination (`C:\Program Files`,
/// a game dir installed there) is the usual cause and the user needs to be told
/// to pick somewhere else. Pure policy, no io.
fn write_error(target: &Path, e: &std::io::Error) -> String {
	let hint = match e.kind() {
		std::io::ErrorKind::PermissionDenied => {
			" - this location needs administrator rights; save somewhere you own (Documents, or a non-system drive)"
		}
		std::io::ErrorKind::NotFound => " - the folder no longer exists",
		_ => "",
	};
	format!("{}: {e}{hint}", target.display())
}

/// The pre-filled filename for a `purpose` dialog (`.json` ensured), or `None`
/// (purposes that don't pre-fill a name). Pure policy, no rfd.
fn dialog_suggested_name(purpose: FilePurpose, doc_path: Option<&Path>, project_name: &str) -> Option<String> {
	use FilePurpose::*;
	let raw = match purpose {
		SaveAs | SaveCopy | ExportWrl => doc_path
			.and_then(Path::file_name)
			.map(|n| n.to_string_lossy().into_owned())
			.or_else(|| Some(project_name.to_string())),
		SavePalette | ExportPalette => Some(project_name.to_string()),
		// A save session has no doc `.json`; suggest the save's name as the base.
		ExportSave => Some(project_name.to_string()),
		_ => None,
	};
	// WRL export carries a `.WRL` name, a save export a `.dta`; else a `.json` doc.
	let ext = match purpose {
		ExportWrl => "WRL",
		ExportSave => "dta",
		_ => "json",
	};
	raw.map(|n| {
		let stem = n
			.strip_suffix(".json")
			.or_else(|| n.strip_suffix(".wrl"))
			.or_else(|| n.strip_suffix(".WRL"))
			.or_else(|| n.strip_suffix(".dta"))
			.or_else(|| n.strip_suffix(".DTA"))
			.unwrap_or(n.as_str());
		format!("{stem}.{ext}")
	})
}

/// Force a WRL export path (from the file picker) to end in exactly one
/// uppercase `.WRL`: replace an existing `.wrl`/`.WRL` extension - so a
/// user-typed `MAP.WRL` is kept as-is, not doubled to `MAP.WRL.wrl` - or append
/// `.WRL` when there's none. M.A.X. world files are conventionally uppercase and
/// the game loads them case-insensitively.
fn wrl_export_path(path: PathBuf) -> PathBuf {
	match path.extension() {
		Some(ext) if ext.eq_ignore_ascii_case("wrl") => path.with_extension("WRL"),
		_ => {
			let mut name = path.into_os_string();
			name.push(".WRL");
			PathBuf::from(name)
		}
	}
}

/// How many prior versions of an overwritten save to retain (`NAME.bak1..bak5`).
const SAVE_BACKUP_KEEP: usize = 5;

/// Rotate the backup history before overwriting `path` (S6.5): drop the oldest
/// kept backup (`.bak{keep}`), shift each `.bak{n}` up to `.bak{n+1}`, then move
/// the current file to `.bak1`. Returns `true` when a backup was made (the file
/// existed), `false` when `path` was absent (a fresh write, nothing to preserve).
/// The editor never overwrites a save without first preserving the prior bytes.
fn rotate_backups(path: &Path, keep: usize) -> std::io::Result<bool> {
	if !path.exists() {
		return Ok(false);
	}
	let backup = |n: usize| -> PathBuf {
		let mut name = path.as_os_str().to_owned();
		name.push(format!(".bak{n}"));
		PathBuf::from(name)
	};
	// Drop the oldest kept backup, then shift the rest up by one.
	let oldest = backup(keep);
	if oldest.exists() {
		std::fs::remove_file(&oldest)?;
	}
	for n in (1..keep).rev() {
		let from = backup(n);
		if from.exists() {
			std::fs::rename(&from, backup(n + 1))?;
		}
	}
	// The current file becomes the newest backup; the caller writes the new one.
	std::fs::rename(path, backup(1))?;
	Ok(true)
}

/// A project `.json`'s top-level `"name"` (for Template Maps labels); `None`
/// when the file can't be read or carries no name.
fn read_map_name(path: &Path) -> Option<String> {
	let text = std::fs::read_to_string(path).ok()?;
	let root = json::parse(&text).ok()?;
	root.get("name").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(|s| s.to_string())
}

/// Scan the shipped maps dir into Template Maps entries, each labelled
/// `"<map name> (<file stem>)"` - or just the stem when the name is missing.
fn template_map_entries(maps_dir: &Path) -> Vec<crate::menu::MapEntry> {
	let Ok(dir) = std::fs::read_dir(maps_dir) else { return Vec::new() };
	let mut paths: Vec<PathBuf> =
		dir.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
	paths.sort();
	paths
		.into_iter()
		.map(|path| {
			let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
			// Map name as the label (file name as a fallback). The Template Maps
			// submenu groups by terrain column, so no right-aligned file name.
			let label = read_map_name(&path).unwrap_or_else(|| stem.clone());
			crate::menu::MapEntry { label, note: None, path }
		})
		.collect()
}

/// One open project's per-tab state. The **active** document's state
/// lives directly on [`EditorState`] (`project`/`path`/`view`/…) so the editor
/// body needn't thread an index through every access; inactive tabs are parked
/// here and swapped in on a tab switch. The cycler is re-derived from the
/// project's palette on restore, so it isn't stored.
struct Document {
	project: Project,
	path: Option<PathBuf>,
	/// The file this map was opened from (see [`EditorState::origin`]).
	origin: Option<PathBuf>,
	view: View,
	active_tile: Option<String>,
	active_color: Option<u8>,
}

/// Tile Painter operation state: stock packs mutated this session and awaiting
/// a bake, plus the copied tile-pixel clipboard for paste.
#[derive(Default)]
pub struct TileOps {
	/// Stock packs mutated this session (dev repaints / new / deleted tiles) and
	/// not yet baked. Bake rewrites these (reordered, dense pass; see
	/// `TilePack::bake_changed`).
	pub dirty_packs: std::collections::BTreeSet<String>,
	/// Copied tile pixels (raw 64×64 indices) for the Tile Painter's paste.
	pub clipboard: Option<Vec<u8>>,
}

/// Open projects (tabs) + the active index. The **active** tab's live state is
/// on [`EditorState`] directly; the others are parked here as `Some(Document)`
/// (the active slot is `None`).
struct TabSet {
	/// Per-tab parked state, in tab order; the active slot is `None`.
	slots: Vec<Option<Document>>,
	/// Index into `slots` of the active document.
	active: usize,
	/// The bootstrap (empty) document is replaced by the first `open`/`new`
	/// rather than stacked - so the editor starts with one real tab, not two.
	replace_scratch: bool,
}

/// Templates Explorer state: the known templates (stock + user) plus the
/// panel's scroll / selection / thumbnail-size / dropdown state.
#[derive(Default)]
pub struct TemplateLibrary {
	/// Templates known to the explorer (stock + user), rescanned on changes.
	/// Production code replaces the list only through [`Self::set_entries`],
	/// which stamps [`Self::revision`] — the template atlas's staleness key.
	pub entries: Vec<TemplateEntry>,
	/// Bumped by every [`Self::set_entries`]; the atlas compares this instead
	/// of re-joining every template name into a `String` each frame.
	revision: u64,
	/// The explorer's selected template (index into `entries`).
	pub sel: Option<usize>,
	/// Thumbnail size (px), chosen from the panel's size dropdown (32..128).
	pub cell: f32,
	/// Restrict the grid to templates of one tileset by its `template_pack`
	/// label (None = all). Stored by label so it survives map switches; a label
	/// absent from the current map reads as "all" (nothing matches → empty).
	pub tileset: Option<String>,
}

impl TemplateLibrary {
	/// Replaces the entry list and stamps a new [`Self::revision`]. The one
	/// mutation path production code uses (the rescan) — a direct `entries`
	/// write would leave the atlas key stale.
	pub fn set_entries(&mut self, entries: Vec<TemplateEntry>) {
		self.entries = entries;
		self.revision += 1;
	}

	/// The entry-list generation the template atlas keys on.
	pub fn revision(&self) -> u64 {
		self.revision
	}
}

/// Color Palette + WRL-palette panel state (selection range, scrolls, the
/// saved-palettes list). The anchor slot is [`EditorState::active_color`]
/// (cross-cutting - it's also the paint colour), so it stays on the editor.
#[derive(Default)]
pub struct PaletteManager {
	/// The far end of a shift-click selection range; `None` = a single slot.
	/// The selection is `active_color..=sel_end` (ordered).
	pub sel_end: Option<u8>,
	/// Ctrl-click multi-selection: a non-contiguous set of slots. When non-empty
	/// it's the active selection (the range is cleared); block re-tint applies
	/// to all of them.
	pub multi: Vec<u8>,
	/// An offset `palette scroll N` asked for, pending until the Color Palette
	/// panel widget drains it (U2.5) — the offsets themselves live in that
	/// widget's `Scroller`, one per panel.
	pub scroll_request: Option<f32>,
	/// Panel tab: false = the grid, true = the saved-palettes list.
	pub show_saved: bool,
	/// Saved/installed palette files for the "saved" tab - scanned on switching.
	pub files: Vec<PathBuf>,
	/// The selected row in the saved list (index into `files`) - the target for
	/// Edit/Delete/Export.
	pub sel: Option<usize>,
}

pub struct EditorState {
	/// The **active** in-memory document. A `.json` loads directly; a
	/// `.WRL` is imported via `Project::from_wrl` (a synthetic in-memory
	/// pack). Everything - render, edit, save, export - goes through it.
	/// Other open projects are parked in `tabs`.
	pub project: Project,
	pub view: View,
	/// Render-target size in px (window inner size / `--size` headless).
	pub screen: (u32, u32),
	/// Where the document came from / was last saved to.
	pub path: Option<PathBuf>,
	/// The `.json` file this map was opened from, kept even for shipped maps
	/// (which load path-less). DEV ▸ Update Map overwrites it - the only way to
	/// write back to a stock map. `None` for New / WRL / image imports.
	pub origin: Option<PathBuf>,
	/// The `resources/` root - the base for every shipped/user content dir
	/// (`assets/{tilepacks,maps}`, `user/{tilepacks,maps,palettes}`).
	pub resources_root: PathBuf,
	/// Where tile packs live (`resources/assets/tilepacks`); the dir handed to
	/// map-core (`TilePack::load`, `Project::new`, the New-Map pack scan).
	pub assets_root: PathBuf,
	/// Where settings persist (`--settings`, or `resources/user/config/mme.ini`); `None`
	/// disables persistence (e.g. headless without `--settings`).
	pub settings_path: Option<PathBuf>,
	/// `[Paths] MaxPath` from `mme.ini`: the user's M.A.X. install directory.
	/// Load dialogs start there; future features (open MAX dir from the menu,
	/// install maps into the game) build on it.
	pub max_path: Option<PathBuf>,
	/// `[Paths] MaxPortPath` from `mme.ini`: the M.A.X. Port directory where the
	/// user's saved games (`.DTA` and stock missions) live. The save editor's
	/// Open Save File dialog starts here (and, later, Export Save File).
	pub max_port_path: Option<PathBuf>,
	/// `[Paths] MaxPortDataPath` from `mme.ini`: the M.A.X. Port *game data*
	/// directory — where `PATCHES.RES` lives (the install/assets dir, distinct
	/// from `MaxPortPath`, which is the pref dir holding saves). Source of the
	/// unit-stats database ([`Self::unit_stats`]).
	pub max_port_data_path: Option<PathBuf>,
	/// The max-port unit database (`SC_ATTRI`/`SC_CLANS`/`SC_UNITS` out of
	/// `PATCHES.RES`): stock base `UnitValues` per unit type, clan advantages,
	/// and per-type applicability metadata. `None` until located — the stats
	/// editor then falls back to save-provided seeds only.
	pub unit_stats: Option<max_assets::attribs::UnitStatsDb>,
	/// `[Paths] SkipPathPrompt`: the "don't ask again" toggle from Editor
	/// Preferences. When set, a missing path no longer pops the dialog on start;
	/// the user opens it manually (Edit ▸ Editor Preferences).
	pub skip_path_prompt: bool,
	/// Why the Preferences dialog was opened by a *missing-path* trigger (opening
	/// a save / using the Units panel). `Some` marks the dialog "required": a
	/// cancel then leads to the Attention notice. Cleared on Save / a menu open.
	pub paths_prompt_reason: Option<String>,
	/// `[Preferences] PalettePreview`: the New Map dialog's palette-preview
	/// toggle (off = strips show original pack colours). Persisted on toggle.
	pub palette_preview: bool,
	/// One-shot: the Map Metadata dialog was just applied for a first save,
	/// so the next Save-As file dialog proceeds without re-prompting (set by
	/// the shell on `Outcome::ApplyMetadata { save_after: true }`).
	pub first_save_meta: bool,
	/// Working palette with the original M.A.X. color-cycle ranges.
	pub cycler: PaletteCycler,
	pub animate: bool,
	/// In-Game render mode: palette cycling + 6-bit colour quantization.
	pub ingame: bool,
	/// CRT post-process effect over the whole app.
	pub crt: bool,
	/// Debug: render with the document's **internal** (map/WRL) palette -
	/// what the file says, not what the game would substitute. The cycler is
	/// re-seeded on toggle, so everything palette-driven follows.
	pub debug_map_palette: bool,
	/// Cell grid overlay on?
	pub show_grid: bool,
	/// Pass-value overlay on? - auto-on in Pass Table Editor mode.
	pub show_pass_overlay: bool,
	/// Resource-distribution overlay on? (View ▸ Resources, S5) - tints each
	/// surveyed cargo cell by material, over an opened save's map.
	pub show_resources: bool,
	/// Resource brush material (`Tool::ResourceBrush`, S5.3); `None` = erase.
	pub resource_material: Option<max_assets::save::CargoMaterial>,
	/// Resource brush amount (0-31).
	pub resource_amount: u8,
	/// Resource brush combine mode (set / add / subtract).
	pub resource_mode: ResourceMode,
	/// Outline broken / missing shore cells over the map (Tools ▸ Shore ▸ Show
	/// Shore Bugs)?
	pub show_shore_bugs: bool,
	/// Outline every tile that violates its match rules (Tools ▸ Validate ▸ Show
	/// Problems)?
	pub show_match_problems: bool,
	/// Cached problem-overlay cells (the shell outlines them in red), recomputed
	/// lazily while the matching toggle is on and the map has changed since.
	pub shore_bug_cells: Vec<(u16, u16)>,
	shore_bug_rev: u64,
	pub match_problem_cells: Vec<(u16, u16)>,
	match_problem_rev: u64,
	/// View filter: composite only the active layer, hiding the others.
	/// A view-only flag - the document is untouched.
	pub show_only_layer: bool,
	/// Bottom status bar visible? (View ▸ Status Bar.)
	pub status_bar: bool,
	/// UI scale factor (View ▸ UI Scale): 1.0 (small), 1.25 (medium), 1.5
	/// (large). The whole chrome + fonts lay out in **logical** px = physical /
	/// this, so a larger factor makes every panel, button, and label bigger. The
	/// map itself renders at native resolution (it's the document, not chrome).
	pub ui_scale: f32,
	pub console: Console,
	/// Where the pointer is over the **map**, in logical UI px - written by the
	/// shell from winit events, read by the map layer alone (the brush outline,
	/// the ghost previews, the cell readout). Every *widget's* hover is its own
	/// `Ui`'s and the panel frame's is the `Workspace`'s, so this is all that is
	/// left of the shell's old pointer snapshot (U6.2). Stays `None` in headless
	/// runs, so captures are mouse-free.
	pub cursor: Option<(f32, f32)>,
	/// Dockable panels around the map view.
	pub workspace: Workspace,
	/// One saved dock layout per [`LayoutGroup`], indexed by its discriminant.
	/// The active group's slot is a stale copy (its live truth is
	/// [`workspace`](Self::workspace)); the inactive slots hold the stored
	/// layouts, swapped in when the mode switches groups.
	pub saved_layouts: [WorkspaceLayout; 3],
	/// The main menu bar: a `wgpu_ui::MenuBar` widget (input + draw both)
	/// hosted in a retained `Ui`, rebuilt from [`EditorState::menu_tree`] when
	/// the structure changes. Command handlers reach the widget via
	/// [`EditorState::menu`]/[`menu_ref`] (close / open-by-title / is-open).
	pub menu_panel: crate::panel_ui::PanelUi,
	pub menu_id: wgpu_ui::WidgetId,
	/// The menu model (labels, command lines, toggle keys, hints) the widget
	/// is built from — the editor mutates THIS (dev menu, Quick Load, shortcut
	/// hints), then rebuilds the widget.
	pub menu_tree: menu::MenuBar,
	/// Fired widget action id → what it runs (parallel to the built items).
	pub menu_acts: Vec<menu::Act>,
	/// Toggle action ids ↔ live state keys, re-synced each frame before draw.
	pub menu_toggles: Vec<(u64, &'static str)>,
	/// The project's undo sequence at the last Undo History rebuild, so the
	/// submenu is only rebuilt when the history actually changed.
	last_undo_seq: u64,
	/// The right-click context menu, when open - items snapshot the state
	/// at open time (selection, clipboard, stamp, the cell under the click).
	pub context_menu: Option<menu::ContextMenu>,
	/// Shortcut hints from the loaded bindings: normalized command line →
	/// chord label (`"copy"` → `"Ctrl+C"`). Set once by the shell; menus and
	/// the context menu annotate their items from it.
	shortcut_hints: Vec<(String, String)>,
	/// Per-generator last-used terrain-generator parameters, remembered for the
	/// session so reopening the Generate modal restores them.
	pub gen_memory: crate::genform::GenMemory,
	/// Headless run (`--headless`/`--screenshot`): native dialogs can't open.
	pub headless: bool,
	/// The window every native (`rfd`) dialog is parented to. An unparented
	/// dialog is ownerless: Windows is free to place it *behind* the editor,
	/// and because the modal blocks the event loop the window underneath can
	/// no longer be moved, minimized or closed - the app looks hung and has to
	/// be killed. Windows raises exactly such a nested prompt when a save
	/// target needs elevation, which is how the freeze was first reported.
	/// `None` in headless runs and tests (no window to own the dialog).
	pub dialog_parent: Option<std::sync::Arc<winit::window::Window>>,
	/// `--dev` mode: unlock editing shipped (stock) assets in the Tile Painter
	/// and show the Bake menu item.
	pub dev_mode: bool,
	/// Tile Painter operation state: stock packs awaiting a bake + the
	/// tile-pixel clipboard.
	pub tile_ops: TileOps,
	/// Tile Explorer state: filter / display size / scroll.
	pub picker: PickerState,
	/// Minimap source: overworld / pass / in-game minimap.
	pub minimap_mode: minimap::Mode,
	/// Unit sprite library from the user's MAX.RES (`None` until loaded -
	/// needs `MaxPath`). Loaded once; failures land in the console.
	pub units: Option<crate::units::UnitLibrary>,
	/// Guards the one load attempt (a missing MAX.RES shouldn't retry per
	/// frame or per command).
	units_loaded: bool,
	/// Resource-marker sprite library from MAX.RES (`None` until loaded - needs
	/// `MaxPath`). Drives the sprite resource overlay (View ▸ Resources); when it
	/// can't load, the overlay falls back to the flat material tint.
	pub markers: Option<crate::markers::MarkerLibrary>,
	/// Guards the one marker load attempt (mirrors `units_loaded`).
	markers_loaded: bool,
	/// A save-open parked behind the "Open Anyway" confirm dialog (the installed
	/// map at the slot didn't fit, but the pristine stock world did). Committed by
	/// `open-save-anyway`, dropped on abort.
	pub pending_save_open: Option<PendingSaveOpen>,
	/// Selected unit in the Units panel (index into `units`). The placed
	/// objects themselves live in `project.objects` (saved with the map).
	pub active_unit: Option<usize>,
	/// The armed scenery cut-out ([`crate::scenery::piece_at`] index) - what
	/// [`Tool::Scenery`] drops and what the panel rings. A display index only:
	/// the document names a placement's piece by string, so re-baking a library
	/// cannot move a placed object.
	pub active_scenery: Option<usize>,
	/// The blend mode a *new* placement takes - the Scenery panel's header
	/// dropdown, and `scenery-blend MODE` on the console. Changing it never
	/// touches placements already on the map; `scenery-blend INDEX MODE` does
	/// that one at a time.
	pub scenery_blend: map_core::SceneryBlend,
	/// The Scenery panel's thumbnail size in px - one of
	/// [`crate::scenery::PREVIEW_SIZES`], chosen from its header dropdown and
	/// persisted as `SceneryPreview` beside the two explorers' own.
	pub scenery_cell: f32,
	/// The Scenery panel's pack filter, by library name (`None` = every
	/// library). A name no loaded library answers to lists nothing, exactly as
	/// a stale tileset filter does in the Templates Explorer.
	pub scenery_pack: Option<String>,
	/// Show the placed unit previews on the map (View ▸ Show Units). Auto-
	/// enables when a unit is picked or stamped.
	pub show_units: bool,
	/// The Clone tool's source object ([`Tool::ObjClone`]): a whole
	/// [`map_core::MapObject`] taken off the map, so a stamp reproduces its
	/// per-unit properties - name, hits, ammo, storage, orders, stat overrides -
	/// not just its type and owner the way the eyedropper does. Its `x`/`y` are
	/// the cell it came from and are overwritten on every stamp.
	pub clone_source: Option<map_core::MapObject>,
	/// Team color for new previews (0..5 - red green blue gray yellow).
	pub unit_team: u8,
	/// The picked object (index into `project.objects`), highlighted on the map
	/// and the target for the Unit Properties panel (S4). `None` = nothing
	/// selected. Validated against the list length at use (edits can shift it).
	pub selected_object: Option<usize>,
	/// Whether the Unit Properties values section shows the *advanced* (static)
	/// stats — build turns, attack radius, move-and-fire, … — as well as the
	/// always-shown dynamic combat stats (S4.5). Toggled by the panel's "advanced"
	/// checkbox.
	pub unitprops_advanced: bool,
	/// The selected-cell mask (editor state, never in the undo journal) -
	/// the select tools edit it; copy/cut and template capture read it.
	pub selection: Selection,
	/// A live rect-select drag's preview `(x0, y0, x1, y1)` in cells - set
	/// by the shell while dragging, drawn as a dashed-intent outline.
	pub select_preview: Option<(u16, u16, u16, u16)>,
	/// The coast cells the Fix Shore tool currently judges broken (against
	/// `tiles.match.json`), refreshed as a run progresses; the shell outlines
	/// each in red while the Fix Shore modal is open. Empty otherwise.
	pub autofix_defects: Vec<(u16, u16)>,
	/// The live Fix Shore run (`Some` while its window is open — idle or
	/// running); the wgpu-ui dialog is a view the shell syncs from this.
	pub autofix: Option<FixRun>,
	/// The live rasterize palette conversion (`Some` from Convert until the
	/// dialog closes / the document swaps); the dialog is a synced view.
	pub pconvert: Option<PaletteConvertRun>,
	/// The live New-from-Image conversion (`Some` while its dialog is open);
	/// the dialog is a synced view.
	pub newimage: Option<NewImageRun>,
	/// The live terrain generation run (`Some` while its dialog is open);
	/// the dialog is a synced view.
	pub genrun: Option<GenerateRun>,
	/// A parked WRL import (`Some` while the Import WRL dialog is open).
	pub wrlimport: Option<WrlImportRun>,
	/// The open Tile Painter's context (`Some` while its dialog is open): the
	/// commit target plus a canvas mirror the shell re-syncs after every edit,
	/// so command paths (`tile-commit`, PNG export/import) work on current
	/// pixels without reaching into the dialog.
	pub tilepaint: Option<crate::tilepaint::TilePaintRun>,
	/// The open New Scenery dialog's context (`Some` while it is open): the
	/// destination packs plus the *source image*, so a PNG chosen through the
	/// native file dialog - a command path, outside any frame - can be written
	/// here and picked up on the next sync.
	pub scenerypaint: Option<crate::scenerypaint::SceneryPaintRun>,
	/// A freshly-built match-editor model awaiting its dialog (set by the
	/// `match-editor` command, taken by the shell when the dialog opens -
	/// the dialog owns it from there).
	pub matchedit_stage: Option<Box<crate::matcheditor::MatchEditor>>,
	/// The copy/cut clipboard (a transient unnamed template).
	pub clipboard: Option<Template>,
	/// The armed ghost stamp riding under the cursor (paste or a picked
	/// template); a map click places it, Esc disarms.
	pub stamp: Option<Template>,
	/// The armed stamp's identity-orientation **base** and current orientation,
	/// so the 8-orientation grid can show every absolute orientation from one
	/// base. `stamp == stamp_base.oriented(stamp_xform)` whenever a stamp is
	/// armed; `None` when no stamp is armed.
	pub stamp_base: Option<Template>,
	pub stamp_xform: map_core::Transform,
	/// The base stamp at each of the 8 orientations (`None` = the tiles forbid
	/// it), cached when the stamp is armed - the 8-orientation grid greys out the
	/// `None`s and renders the `Some`s. Empty (all `None`) when no stamp is armed.
	pub stamp_orients: [Option<Template>; 8],
	/// Templates Explorer state (the known templates + selection/size).
	pub templates: TemplateLibrary,
	/// Recently-opened maps for File ▸ Quick Load: most-recent first, ≤10,
	/// templates excluded. Loaded from / saved to `[Workspace] Recent0..` and
	/// pushed into the menu via [`MenuBar::set_recent`] as maps open.
	pub recent: Vec<PathBuf>,
	/// Open projects + the active index (the active tab's live state is the
	/// fields above; the others are parked in `tabs.slots`).
	tabs: TabSet,
	/// The active map tool: pencil paints, picker eyedrops.
	pub tool: Tool,
	/// Randomize-variants toggle: when on, painting/filling places a
	/// random sibling from the tile's `tiles.variants.json` group.
	pub randomize: bool,
	/// RNG for the randomize toggle - fixed-seeded so a replayed script paints
	/// the same "random" sequence (scripts/tests stay reproducible).
	paint_rng: Rng,
	/// Active edit layer: paint + erase act only on it. Default
	/// Ground (the detail layer over the water base). One of
	/// [`map_core::LAYER_WATER`], [`map_core::LAYER_GROUND`] or
	/// [`LAYER_SCENERY`].
	pub active_layer: usize,
	/// Brush/eraser footprint: an odd-sided square (`1` = single cell)
	/// centred on the cursor. Drives pencil paint and the eraser.
	pub brush_size: u16,
	/// Brush footprint shape (square or circle).
	pub brush_shape: BrushShape,
	/// Terrain brush ([`Tool::PaintMask`]) material: `false` paints land, `true`
	/// paints water. The toolbox "land"/"water" buttons (and the Q/W keys) set it.
	pub mask_water: bool,
	/// Terrain brush coast behaviour on stroke release (toolbox "auto shore"
	/// select). Default Sweep.
	pub brush_shore: BrushShore,
	/// The cell bounds painted by the in-progress terrain-brush stroke (inclusive
	/// `x0,y0,x1,y1`). Accumulated as the brush drags; consumed on release to
	/// shore just that region. `None` between strokes.
	mask_dirty: Option<(u16, u16, u16, u16)>,
	/// Editor mode: tile painting vs pass-table painting.
	pub mode: EditorMode,
	/// Active pass value for the Pass Table Editor (0..3).
	pub active_pass: u8,
	/// Selected palette slot in the Color Palette panel - the anchor of
	/// a multi-select range.
	pub active_color: Option<u8>,
	/// Color Palette + WRL-palette panel state (selection range, scrolls, saved list).
	pub palettes: PaletteManager,
	/// The tile spec `paint` stamps - set by the `tile` command or
	/// a Tile Explorer click. Resolved per paint, so it re-validates
	/// after document switches.
	active_tile: Option<String>,
	clock: f32,
}

/// One template known to the explorer: where it lives and the parsed file.
/// Stock entries (shipped under `resources/assets/templates`) can be picked
/// and cloned but never deleted; user entries live in
/// `resources/user/templates`.
pub struct TemplateEntry {
	pub name: String,
	pub path: PathBuf,
	pub stock: bool,
	pub template: Template,
}

/// A filesystem-safe file stem from a display name: lowercased, whitespace and
/// dashes collapsed to a single `-`, only `[a-z0-9_-]` kept (other characters
/// dropped), leading/trailing dashes trimmed. Empty result falls back to
/// `template`. The display name (the JSON `name`) keeps the user's text; only
/// the filename is normalized this way.
pub fn sanitize_filename(name: &str) -> String {
	let mut out = String::with_capacity(name.len());
	for c in name.trim().chars() {
		let c = c.to_ascii_lowercase();
		if c.is_ascii_alphanumeric() || c == '_' {
			out.push(c);
		} else if (c.is_whitespace() || c == '-') && !out.ends_with('-') && !out.is_empty() {
			out.push('-');
		}
	}
	while out.ends_with('-') {
		out.pop();
	}
	if out.is_empty() {
		out.push_str("template");
	}
	out
}

/// The pack subdir a template belongs in: the **terrain** packs it uses, sorted
/// and joined with `+` (e.g. `GREEN+DESERT`). `WATER` is excluded - it's the
/// universal base layer nearly every template touches, so it would just be noise
/// on every directory; a template that uses *only* water lands in `WATER`, and
/// one that uses no packs in `MISC`. Templates live under `templates/<PACKS>/`
/// so the directory names the tileset(s) a template needs.
fn template_pack(t: &Template) -> String {
	let mut names: Vec<&str> = t.uses.iter().map(|(n, _)| n.as_str()).filter(|&n| n != "WATER").collect();
	names.sort_unstable();
	names.dedup();
	if !names.is_empty() {
		return names.join("+");
	}
	// Nothing but water (or nothing at all): WATER if it's used, else MISC.
	if t.uses.iter().any(|(n, _)| n == "WATER") { "WATER".to_string() } else { "MISC".to_string() }
}

/// The first free `<base>.json` / `<base>-N.json` stem in `dir` (ignoring
/// `exclude`, the file being renamed in place) - the numeral-postfix bump used
/// on save/clone/import/rename collisions.
fn free_stem_in(dir: &std::path::Path, base: &str, exclude: Option<&std::path::Path>) -> String {
	let taken = |s: &str| {
		let p = dir.join(format!("{s}.json"));
		p.exists() && exclude != Some(p.as_path())
	};
	if !taken(base) {
		return base.to_string();
	}
	(2..).map(|n| format!("{base}-{n}")).find(|c| !taken(c)).expect("an unbounded counter finds a free stem")
}

/// A path's file stem as a `&str` (empty if it has none / isn't UTF-8).
fn stem(p: &std::path::Path) -> &str {
	p.file_stem().and_then(|s| s.to_str()).unwrap_or("")
}

/// Strip leading zeros from a digit run, keeping at least one digit.
fn trim_zeros(d: &[u8]) -> &[u8] {
	let mut k = 0;
	while k + 1 < d.len() && d[k] == b'0' {
		k += 1;
	}
	&d[k..]
}

/// Human/natural string order: digit runs compare by numeric value (so
/// `3 < 20 < 100`), other runs compare case-insensitively (case as a tiebreak).
/// Hand-rolled - no external crate.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
	use std::cmp::Ordering::Equal;
	let (a, b) = (a.as_bytes(), b.as_bytes());
	let (mut i, mut j) = (0, 0);
	while i < a.len() && j < b.len() {
		if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
			let (si, sj) = (i, j);
			while i < a.len() && a[i].is_ascii_digit() {
				i += 1;
			}
			while j < b.len() && b[j].is_ascii_digit() {
				j += 1;
			}
			let (na, nb) = (trim_zeros(&a[si..i]), trim_zeros(&b[sj..j]));
			// Equal length of zero-trimmed digits -> lexical == numeric.
			match na.len().cmp(&nb.len()).then_with(|| na.cmp(nb)) {
				Equal => {}
				ord => return ord,
			}
		} else {
			match a[i].to_ascii_lowercase().cmp(&b[j].to_ascii_lowercase()).then(a[i].cmp(&b[j])) {
				Equal => {}
				ord => return ord,
			}
			i += 1;
			j += 1;
		}
	}
	(a.len() - i).cmp(&(b.len() - j))
}

/// Open `dir` in the OS file manager (best-effort, fire-and-forget). Uses the
/// platform launcher; no extra dependency.
fn open_in_file_manager(dir: &std::path::Path) -> Result<(), String> {
	let program = if cfg!(target_os = "macos") {
		"open"
	} else if cfg!(target_os = "windows") {
		"explorer"
	} else {
		"xdg-open"
	};
	std::process::Command::new(program).arg(dir).spawn().map(|_| ()).map_err(|e| format!("{program}: {e}"))
}

/// The supported UI scale factors (View ▸ UI Scale): small / medium / large.
pub const UI_SCALES: [f32; 3] = [1.0, 1.25, 1.5];

/// Resolve a menu/context-menu `command` to the chord label its row shows:
/// its own binding (`hints`: normalized command line → chord), the binding of
/// the action it confirms/varies ([`crate::input::binding_alias`]), or a fixed
/// shell shortcut ([`crate::input::fixed_hint`]). Free fn so it can run while
/// `EditorState` borrows `menu` mutably to bake the bar's hints.
fn resolve_hint(hints: &[(String, String)], command: &str) -> Option<String> {
	let direct = |line: &str| hints.iter().find(|(l, _)| l == line).map(|(_, label)| label.clone());
	direct(command)
		.or_else(|| crate::input::binding_alias(command).and_then(direct))
		.or_else(|| crate::input::fixed_hint(command).map(str::to_string))
}

impl EditorState {
	pub fn new(project: Project, screen: (u32, u32), path: Option<PathBuf>, resources_root: PathBuf) -> Self {
		let assets_root = resources_root.join("assets/tilepacks");
		let (project_w, project_h) = (project.width, project.height);
		let view = View::fit((project.width, project.height), screen.0 as f32, screen.1 as f32);
		let cycler = PaletteCycler::from_rgb(&project.palette);
		// Template Maps lists the shipped read-only maps (Quick Load is the
		// user's own recent maps, filled in later from settings).
		let templates_dir = resources_root.join("assets/maps");
		let template_maps = template_map_entries(&templates_dir);
		let menu_tree = MenuBar::new(&template_maps, &[]);
		let (menu_widget, menu_acts, menu_toggles) = menu_tree.build_bar();
		let menu_id = menu_widget.id();
		let mut workspace = Workspace::default();
		// The menu bar + project tab strip reserve the top strip; the status bar
		// reserves the bottom (shown by default).
		workspace.top = menu::BAR_H + crate::tabs::BAR_H;
		workspace.bottom = crate::statusbar::BAR_H;
		// Every layout group starts at the default arrangement; a settings load
		// (see `seed_mode_layouts`) then overrides each with its saved section.
		let default_layout = workspace.save_layout();
		let mut s = Self {
			project,
			view,
			screen,
			path,
			origin: None,
			resources_root,
			assets_root,
			settings_path: None,
			max_path: None,
			max_port_path: None,
			max_port_data_path: None,
			unit_stats: None,
			skip_path_prompt: false,
			paths_prompt_reason: None,
			palette_preview: false,
			first_save_meta: false,
			cycler,
			animate: false,
			ingame: false,
			crt: false,
			debug_map_palette: false,
			show_grid: false,
			show_pass_overlay: false,
			show_resources: false,
			resource_material: Some(max_assets::save::CargoMaterial::Raw),
			resource_amount: 15,
			resource_mode: ResourceMode::Set,
			show_shore_bugs: false,
			show_match_problems: false,
			shore_bug_cells: Vec::new(),
			shore_bug_rev: u64::MAX,
			match_problem_cells: Vec::new(),
			match_problem_rev: u64::MAX,
			show_only_layer: false,
			status_bar: true,
			ui_scale: 1.0,
			console: Console::new(),
			cursor: None,
			menu_panel: crate::panel_ui::PanelUi::new(menu_widget),
			menu_id,
			menu_tree,
			menu_acts,
			menu_toggles,
			last_undo_seq: u64::MAX, // force the first Undo History build
			context_menu: None,
			shortcut_hints: Vec::new(),
			gen_memory: crate::genform::GenMemory::default(),
			headless: false,
			dialog_parent: None,
			units: None,
			units_loaded: false,
			markers: None,
			markers_loaded: false,
			pending_save_open: None,
			active_unit: None,
			active_scenery: None,
			scenery_blend: map_core::SceneryBlend::default(),
			scenery_cell: crate::scenery::DEFAULT_PREVIEW,
			scenery_pack: None,
			show_units: true,
			clone_source: None,
			unit_team: 0,
			selected_object: None,
			unitprops_advanced: false,
			selection: Selection::new(project_w, project_h),
			select_preview: None,
			autofix_defects: Vec::new(),
			autofix: None,
			pconvert: None,
			newimage: None,
			genrun: None,
			wrlimport: None,
			tilepaint: None,
			scenerypaint: None,
			matchedit_stage: None,
			clipboard: None,
			stamp: None,
			stamp_base: None,
			stamp_xform: map_core::Transform::default(),
			stamp_orients: std::array::from_fn(|_| None),
			templates: TemplateLibrary { cell: 64.0, ..Default::default() },
			recent: Vec::new(),

			dev_mode: false,
			tile_ops: TileOps::default(),
			workspace,
			saved_layouts: [default_layout.clone(), default_layout.clone(), default_layout],
			picker: PickerState::default(),
			minimap_mode: minimap::Mode::Overworld,
			// one tab; the active live fields above are its state.
			tabs: TabSet { slots: vec![None], active: 0, replace_scratch: true },
			tool: Tool::Pencil,
			active_layer: LAYER_GROUND,
			brush_size: 1,
			brush_shape: BrushShape::Square,
			mask_water: false,
			mask_dirty: None,
			brush_shore: BrushShore::Sweep,
			randomize: false,
			paint_rng: Rng::new(0x004d_4158_5f56_4152), // "MAX_VAR"
			mode: EditorMode::Map,
			active_pass: 1,
			active_color: None,
			palettes: PaletteManager::default(),
			active_tile: None,
			clock: 0.0,
		};
		s.scan_templates();
		s
	}

	/// Install the loaded bindings' shortcut hints and bake them onto the main
	/// menu through the shared resolver ([`Self::menu_hint`]), so aliases and
	/// fixed shell shortcuts annotate rows too, not just exact-match bindings.
	pub fn apply_shortcut_hints(&mut self, hints: Vec<(String, String)>) {
		self.shortcut_hints = hints;
		// Stamp the hints onto the model, then rebuild the widget from it.
		let table = self.shortcut_hints.clone();
		self.menu_tree.apply_shortcuts(&|command| resolve_hint(&table, command));
		self.rebuild_menu();
	}

	/// The chord a menu / context-menu row advertises for `command`: its own
	/// configured binding, the binding of the action it confirms or varies
	/// (Exit → quit), or a fixed shell shortcut (the text-field edit menu, the
	/// stamp's Esc cancel). The single place every row resolves its shortcut -
	/// new items need no per-item wiring.
	fn menu_hint(&self, command: &str) -> Option<String> {
		resolve_hint(&self.shortcut_hints, command)
	}

	/// The main menu bar widget (mutable) — close / open-by-title / checkmarks.
	pub fn menu(&mut self) -> &mut wgpu_ui::MenuBar {
		self.menu_panel.ui.get_mut::<wgpu_ui::MenuBar>(self.menu_id).expect("menu widget")
	}

	/// The main menu bar widget (shared) — for the open-state gating reads.
	pub fn menu_ref(&self) -> &wgpu_ui::MenuBar {
		self.menu_panel.ui.get::<wgpu_ui::MenuBar>(self.menu_id).expect("menu widget")
	}

	/// Rebuild the menu widget from [`menu_tree`](Self::menu_tree) — after any
	/// structure change (dev menu, Quick Load entries, shortcut hints).
	fn rebuild_menu(&mut self) {
		let (bar, acts, toggles) = self.menu_tree.build_bar();
		self.menu_id = bar.id();
		self.menu_panel = crate::panel_ui::PanelUi::new(bar);
		self.menu_acts = acts;
		self.menu_toggles = toggles;
	}

	/// Rebuild the Edit ▸ Undo History submenu from the project's undo stack,
	/// but only when it changed since last time (cheap `undo_seq` check). The
	/// render loop calls this each frame; edits happen with menus closed, so the
	/// widget rebuild never disrupts an open menu.
	pub fn sync_undo_history(&mut self) {
		let seq = self.project.undo_seq();
		if seq == self.last_undo_seq {
			return;
		}
		// Never rebuild a live menu — the rebuild mints a fresh (closed) widget,
		// so an open cascade would vanish mid-frame. Interactively edits close
		// the menu first, but the script path can open one right after an edit
		// (`open! …` then `menu file`). `last_undo_seq` stays stale, so the
		// deferred rebuild lands on the first frame after the menu closes.
		if self.menu_ref().is_open() {
			return;
		}
		self.last_undo_seq = seq;
		let labels = self.project.undo_labels(10);
		self.menu_tree.set_undo_history(&labels);
		self.rebuild_menu();
	}

	/// Unlock the DEV menu (a `--dev` launch) and rebuild the widget.
	pub fn menu_set_dev(&mut self, dev: bool, packs: &[String]) {
		self.menu_tree.set_dev(dev, packs);
		self.rebuild_menu();
	}

	/// The right-click context menu for the current state. `cell` is the map
	/// cell under the click (`None` over chrome / outside the map); cell-bound
	/// entries bake it into their command line.
	fn context_menu_items(&self, cell: Option<(u16, u16)>) -> Vec<menu::Item> {
		let act = |label: &str, command: &str| menu::Item::Action {
			label: label.into(),
			hint: self.menu_hint(command),
			command: command.into(),
		};
		let mut items = Vec::new();
		if self.stamp.is_some() {
			if let Some((x, y)) = cell {
				items.push(act("Place Here", &format!("stamp {x} {y}")));
			}
			items.push(act("Cancel Stamp", "stamp cancel"));
			items.push(menu::Item::Sep);
		}
		// The unit place / erase tools stay armed until cancelled (they paint many
		// via drag, like a stamp). Offer the cancel here — `tool default` disarms to
		// the mode's select tool — mirroring Cancel Stamp above.
		match self.tool {
			Tool::Unit => {
				items.push(act("Cancel Placement", "tool default"));
				items.push(menu::Item::Sep);
			}
			Tool::UnitEraser => {
				items.push(act("Cancel Erase", "tool default"));
				items.push(menu::Item::Sep);
			}
			// The scenery tools are the Scenery *layer's* pencil, eraser and
			// arrow, so cancelling them means leaving the layer - `tool default`
			// alone would only hand back the same three, re-pointed.
			Tool::Scenery | Tool::SceneryMove | Tool::SceneryEraser => {
				items.push(act("Leave the Scenery Layer", "layer ground"));
				items.push(menu::Item::Sep);
			}
			_ => {}
		}
		if !self.selection.is_empty() {
			items.push(act("Cut", "cut"));
			items.push(act("Copy", "copy"));
			items.push(act("Delete", "delete"));
			items.push(act("Save as Template", "template-save"));
			items.push(act("Clear Selection", "select clear"));
			items.push(menu::Item::Sep);
		}
		if self.clipboard.is_some() {
			items.push(act("Paste", "paste"));
		}
		items.push(act("Select All", "select all"));
		items.push(menu::Item::Sep);
		if let Some((x, y)) = cell {
			items.push(act("Pick Tile", &format!("pick {x} {y}")));
			items.push(act("Center Here", &format!("pan-to {} {}", x as f32 + 0.5, y as f32 + 0.5)));
		}
		items.push(act("Fit Map", "fit"));
		items
	}

	/// The Templates Explorer item menu (right-click on a thumbnail), built from
	/// the current `templates.sel` - the right-click selects the entry first.
	/// Stock templates are read-only, so rename/delete give way to Duplicate.
	fn template_context_items(&self) -> Vec<menu::Item> {
		let act = |label: &str, command: &str| menu::Item::Action {
			label: label.into(),
			hint: self.menu_hint(command),
			command: command.into(),
		};
		let Some(i) = self.templates.sel else { return Vec::new() };
		let entry = &self.templates.entries[i];
		// Stock templates are read-only - rename/delete need `--dev` (otherwise
		// only Duplicate). Quote the name so spaces survive the command split;
		// `template-pick` prefers the selection, so cross-tileset dups resolve right.
		let editable = !entry.stock || self.dev_mode;
		let mut items = vec![act("Use", &format!("template-pick \"{}\"", entry.name)), menu::Item::Sep];
		if editable {
			items.push(act("Rename", "template-rename"));
		}
		items.push(act("Duplicate", "template-clone"));
		if editable {
			items.push(act("Delete", "template-delete"));
		}
		items.push(menu::Item::Sep);
		items.push(act("Export as PNG", "template-export-png"));
		items
	}

	/// Open the explorer item menu at a logical-space point (the right-click has
	/// already selected the template it targets).
	pub fn open_template_context_menu(&mut self, pos: (f32, f32)) {
		let items = self.template_context_items();
		if items.is_empty() {
			return;
		}
		self.context_menu = Some(menu::ContextMenu::new(items, pos));
		self.menu().close();
	}

	/// Re-seed the cycling palette after a project palette edit (or its
	/// undo/redo) so the working palette + GPU upload follow. The Debug ▸
	/// map-palette toggle swaps the source to the document's internal palette.
	fn refresh_palette(&mut self) {
		let rgb = if self.debug_map_palette { self.project.internal_palette() } else { self.project.palette.clone() };
		self.cycler = PaletteCycler::from_rgb(&rgb);
		self.cycler.set_ingame(self.ingame);
	}

	/// Scan the installed-tileset (`resources/assets/tilepacks/*/palette.json`) and
	/// user (`resources/user/palettes/*.json`) palettes for the "saved" tab.
	fn scan_palette_files(&mut self) {
		let mut files = Vec::new();
		if let Ok(rd) = std::fs::read_dir(&self.assets_root) {
			let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
			dirs.sort();
			files.extend(dirs.into_iter().map(|d| d.join("palette.json")).filter(|p| p.is_file()));
		}
		let pal_dir = self.resources_root.join("user/palettes");
		if let Ok(rd) = std::fs::read_dir(&pal_dir) {
			let mut jsons: Vec<PathBuf> = rd
				.flatten()
				.map(|e| e.path())
				.filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")))
				.collect();
			jsons.sort();
			files.extend(jsons);
		}
		self.palettes.files = files;
	}

	/// Where saved (user) palettes live: `resources/user/palettes`.
	pub fn user_palettes_dir(&self) -> PathBuf {
		self.resources_root.join("user/palettes")
	}

	/// Report a palette-manager file op that succeeded: log `msg`, rescan the
	/// saved palettes, and select `sel` (the new/renamed file, or `None` after a
	/// delete). The shared tail of the `palette-*` write commands.
	fn palette_saved(&mut self, msg: String, sel: Option<PathBuf>) -> Outcome {
		self.console.push_line(msg);
		self.scan_palette_files();
		self.palettes.sel = sel.and_then(|p| self.palettes.files.iter().position(|f| *f == p));
		Outcome::Redraw
	}

	/// Report a template-manager file op that wrote `path`: log `msg`, rescan
	/// the library, and select the template now at `path`. The shared tail of
	/// template save / clone / import / rename.
	fn template_saved(&mut self, msg: String, path: &Path) -> Outcome {
		self.console.push_line(msg);
		self.scan_templates();
		self.templates.sel = self.templates.entries.iter().position(|t| t.path == *path);
		Outcome::Redraw
	}

	/// The selected saved palette's path, if a row is selected.
	pub fn selected_palette(&self) -> Option<&PathBuf> {
		self.palettes.sel.and_then(|i| self.palettes.files.get(i))
	}

	/// Whether the selected palette is a user palette (editable/deletable);
	/// tileset `palette.json` files are read-only.
	pub fn selected_palette_is_user(&self) -> bool {
		let dir = self.user_palettes_dir();
		self.selected_palette().is_some_and(|p| p.starts_with(&dir))
	}

	/// Display names for the saved-palette list: a tileset `palette.json` shows
	/// its tileset (parent) name; a user palette shows its file stem.
	pub fn palette_file_names(&self) -> Vec<String> {
		self.palettes
			.files
			.iter()
			.map(|p| {
				let stem = if p.file_name().and_then(|n| n.to_str()) == Some("palette.json") {
					p.parent().and_then(|d| d.file_name())
				} else {
					p.file_stem()
				};
				stem.map_or_else(|| "palette".into(), |s| s.to_string_lossy().into_owned())
			})
			.collect()
	}

	/// The active paint tile spec (the picker highlights it).
	pub fn active_tile(&self) -> Option<&str> {
		self.active_tile.as_deref()
	}

	/// Fit a map into the workspace's center area (between the docks).
	fn fit_center(&self, map_tiles: (u16, u16)) -> View {
		let l = self.workspace.layout(self.screen.0 as f32, self.screen.1 as f32);
		View::fit_rect(map_tiles, (l.center.x, l.center.y, l.center.w, l.center.h))
	}

	/// Map a screen-px position to the **map pixel** under it - what a scenery
	/// placement is positioned by. Unclamped and signed on purpose: an object
	/// may legitimately hang off the map's left or top edge, and clamping here
	/// would make a drag stick at the border.
	pub fn world_at(&self, sx: f32, sy: f32) -> (i32, i32) {
		(
			(sx / self.view.zoom + self.view.pan[0]).floor() as i32,
			(sy / self.view.zoom + self.view.pan[1]).floor() as i32,
		)
	}

	/// The armed scenery piece as `(pack, id)`, or `None` when nothing is armed
	/// or the index no longer resolves (the libraries changed under it).
	pub fn armed_scenery(&self) -> Option<(String, String)> {
		let i = self.active_scenery?;
		crate::scenery::piece_at(&self.project, i).map(|(pack, piece)| (pack.to_string(), piece.id.clone()))
	}

	/// Map a screen-px position to the cell under it (`None` off-map).
	pub fn cell_at(&self, sx: f32, sy: f32) -> Option<(u16, u16)> {
		let tx = (sx / self.view.zoom + self.view.pan[0]) / TILE_PX as f32;
		let ty = (sy / self.view.zoom + self.view.pan[1]) / TILE_PX as f32;
		let (w, h) = self.map_size();
		(tx >= 0.0 && ty >= 0.0 && tx < w as f32 && ty < h as f32).then_some((tx as u16, ty as u16))
	}

	/// Whether an LMB drag can paint right now (an active tile is set).
	pub fn can_paint(&self) -> bool {
		self.active_tile.is_some()
	}

	/// Take the painted-cell bounds of the just-finished terrain-brush stroke,
	/// grown by one cell and clamped to the map, then clear them. `auto_shore`
	/// re-tiles this region (it widens by another cell internally), growing the
	/// beach + coastal waves along everything the stroke painted. `None` when the
	/// stroke painted nothing. `(x0, y0, x1, y1)` inclusive.
	pub fn take_mask_region(&mut self) -> Option<(u16, u16, u16, u16)> {
		let (x0, y0, x1, y1) = self.mask_dirty.take()?;
		let (w, h) = (self.project.width, self.project.height);
		Some((x0.saturating_sub(1), y0.saturating_sub(1), (x1 + 1).min(w - 1), (y1 + 1).min(h - 1)))
	}

	/// Map dimensions in tiles.
	pub fn map_size(&self) -> (u16, u16) {
		(self.project.width, self.project.height)
	}

	/// The **logical** UI size (px): the physical render target divided by the
	/// UI scale. All chrome + label layout works in these units; the projection
	/// scales them up to fill the physical framebuffer. The map scene keeps using
	/// the physical [`screen`](Self::screen) (it renders at native resolution).
	pub fn ui_screen(&self) -> (f32, f32) {
		(self.screen.0 as f32 / self.ui_scale, self.screen.1 as f32 / self.ui_scale)
	}

	/// The on-screen body rect of dockable `id`, or `None` when it isn't shown
	/// (hidden panels aren't in the layout). `(w, h)` is the logical UI size.
	fn panel_body(&self, id: &str, w: f32, h: f32) -> Option<crate::ui::Rect> {
		let pi = self.workspace.find(id)?;
		let r = self.workspace.layout(w, h).panels.into_iter().find(|(i, _)| *i == pi)?.1;
		Some(self.workspace.body_of(pi, r))
	}

	/// Scroll the Tile Explorer so the active (just-picked) tile is in view. A
	/// no-op when the explorer is closed; falls back to the All filter so a tile
	/// the current filter would hide is still revealed. Since U2.4 the offset
	/// lives in the panel widget, so this queues a [`picker::ScrollRequest`] the
	/// widget drains at its next layout.
	fn reveal_active_tile_in_explorer(&mut self) {
		let (w, h) = self.ui_screen();
		let Some(_body) = self.panel_body("tiles", w, h) else { return };
		let Some(spec) = self.active_tile.clone() else { return };
		let base = spec.split(':').next().unwrap_or(&spec);
		let cur = self.picker.filter;
		let cur_ts = picker::tileset_index(&self.project, self.picker.tileset.as_deref());
		// Try the current view (pass filter + tileset); if that hides the tile,
		// fall back to All / every pack so it's still revealed.
		let (filter, tileset, idx) = match picker::items(&self.project, cur, cur_ts).iter().position(|it| it.id == base)
		{
			Some(i) => (cur, self.picker.tileset.clone(), i),
			None => match picker::items(&self.project, picker::Filter::All, None).iter().position(|it| it.id == base) {
				Some(i) => (picker::Filter::All, None, i),
				None => return,
			},
		};
		self.picker.filter = filter;
		self.picker.tileset = tileset;
		self.picker.scroll_request = Some(picker::ScrollRequest::Reveal(idx));
	}

	/// Set the UI scale (label measurement is float and scale-free now - the
	/// renderer rasterizes at the scaled size; layout stays in logical px).
	pub fn set_ui_scale(&mut self, scale: f32) {
		self.ui_scale = scale;
	}

	/// The active edit layer's name (`"water"`/`"ground"`/`"scenery"`) - for the
	/// eraser tool's `Erase` command and the toolbox highlight.
	pub fn active_layer_name(&self) -> &'static str {
		match self.active_layer {
			LAYER_WATER => "water",
			LAYER_SCENERY => "scenery",
			_ => "ground",
		}
	}

	/// The **tile** layer edits land on. Same as [`Self::active_layer`] for the
	/// two real layers; the Scenery layer is not one, so a tile edit that runs
	/// anyway (a script's `place`, a fill, the terrain brush) falls back to the
	/// ground layer rather than addressing a layer index that does not exist.
	pub fn tile_layer(&self) -> usize {
		self.active_layer.min(LAYER_GROUND)
	}

	/// [`Self::tile_layer`]'s label - what a cell edit reports, so a console line
	/// can never claim it cleared cells "on the scenery layer" (there are none)
	/// and an `erase LAYER` it builds is always a layer `erase` will accept.
	pub fn tile_layer_name(&self) -> &'static str {
		if self.tile_layer() == LAYER_WATER { "water" } else { "ground" }
	}

	/// Whether the free-placed cut-outs are the active layer - so the pencil
	/// drops one, the eraser removes one and the arrow drags one.
	pub fn on_scenery_layer(&self) -> bool {
		self.active_layer == LAYER_SCENERY
	}

	/// Arming a *terrain* thing - a tile from the Tile Explorer or the
	/// eyedropper, a template from its explorer - while the Scenery layer is live
	/// hands the editor back to `layer`, the mirror of [`Command::SceneryPick`]
	/// arming the Scenery layer with its piece: one click in a panel is enough to
	/// use what was clicked, whatever was live before. The tool rides along
	/// ([`terrain_twin`]), so the scenery pencil becomes the pencil again instead
	/// of dropping cut-outs at a tile the user just picked.
	///
	/// Only from the Scenery layer: between water and ground the active layer is
	/// the user's own choice (a water tile painted on ground is a real move), and
	/// only in Map mode - the pass and save editors own their tools.
	fn leave_scenery_layer(&mut self, layer: usize) {
		if self.mode != EditorMode::Map || !self.on_scenery_layer() {
			return;
		}
		self.active_layer = layer.min(LAYER_GROUND);
		self.tool = terrain_twin(self.tool);
	}

	/// Which layers the map view composites, as a bitmask (bit `n` = layer `n`).
	/// All layers normally; only the active layer when "show only selected" is
	/// on. Consumed by the project shader.
	///
	/// The Scenery layer sets no tile bit at all, which is the honest reading of
	/// "show only this layer" for it: the terrain drops out and the cut-outs -
	/// drawn by their own pass, over the composed map - are left alone on the
	/// canvas.
	pub fn layer_mask(&self) -> u32 {
		if self.show_only_layer { 1 << self.active_layer } else { (1 << map_core::MAX_LAYERS) - 1 }
	}

	/// The cells the brush covers when centred on `(x, y)`: an odd-sided square
	/// of side `brush_size`, clamped to the map. `brush_size == 1` → just the
	/// one cell.
	pub fn brush_cells(&self, x: u16, y: u16) -> Vec<(u16, u16)> {
		let r = (self.brush_size.max(1) / 2) as i32;
		let (w, h) = (self.project.width as i32, self.project.height as i32);
		// Circle: keep cells whose centre lies within `r + 0.5` of the brush
		// centre (disk rasterization); square keeps the whole block.
		let rad2 = (r as f32 + 0.5).powi(2);
		let mut out = Vec::new();
		for dy in -r..=r {
			for dx in -r..=r {
				if self.brush_shape == BrushShape::Circle && (dx * dx + dy * dy) as f32 > rad2 {
					continue;
				}
				let (cx, cy) = (x as i32 + dx, y as i32 + dy);
				if (0..w).contains(&cx) && (0..h).contains(&cy) {
					out.push((cx as u16, cy as u16));
				}
			}
		}
		out
	}

	/// One-line context hint for the status bar, by editor mode + active tool.
	pub fn status_hint(&self) -> &'static str {
		match self.mode {
			EditorMode::Pass => "Pass Table Editor: drag to set the tile's passability (retints every cell using it)",
			EditorMode::LocalPass => "Local Pass Override: drag to set a per-cell override; the eraser tool clears it",
			EditorMode::SaveEditor => {
				"Save Editor (experimental): edit a loaded save's units & resources - open one via File > Experimental"
			}
			EditorMode::Map => match self.tool {
				Tool::Pencil => "Pencil: drag to paint the active tile - pick one in the Tile Explorer",
				Tool::Eraser => "Eraser: drag to clear cells on the active layer",
				Tool::Picker => "Eyedropper: click a cell to make its tile the brush",
				Tool::Fill => "Flood Fill: click to fill a region - an active selection confines it",
				Tool::PaintMask => {
					"Terrain Brush: drag to paint land or water (Q/W or the toolbox); the coast grows on release"
				}
				Tool::Select => "Select: drag to select cells (Shift adds, Ctrl subtracts); Del clears them",
				Tool::SelectRect => "Rect Select: drag a rectangle (Shift adds, Ctrl subtracts)",
				Tool::Scenery => "Scenery: click to drop the armed object - any pixel, no grid",
				Tool::SceneryMove => "Scenery Move: drag a placed object to reposition it",
				Tool::SceneryEraser => "Scenery Delete: click an object to remove it",
				Tool::Unit => "Unit: click to stamp the active unit preview",
				Tool::UnitEraser => "Unit Eraser: click a unit preview to remove it",
				Tool::ObjSelect => "Select: click an object to select it (any cell of a multi-cell footprint)",
				Tool::ObjPick => "Pick: click an object to arm its type + team for placing",
				Tool::ObjMove => "Move: drag an object to a new cell (blocked if a building is in the way)",
				Tool::ObjClone => match self.clone_source {
					Some(_) => "Clone: click a bare cell to stamp the source, or another object to re-source",
					None => "Clone: click an object to take it - type, team and all its properties - as the source",
				},
				Tool::ResourceBrush => {
					"Resource Brush: drag to paint the material/amount into the cargo map (set/add/sub in the toolbox)"
				}
			},
		}
	}

	/// Resize the render target, keeping the world point under the old
	/// viewport centre still centred - so a window resize doesn't drift the
	/// map. `pan_new = pan_old + (old_centre - new_centre) / zoom`.
	pub fn on_resize(&mut self, w: u32, h: u32) {
		let (nw, nh) = (w.max(1), h.max(1));
		let (ow, oh) = (self.screen.0 as f32, self.screen.1 as f32);
		self.view.pan[0] += (ow - nw as f32) / 2.0 / self.view.zoom;
		self.view.pan[1] += (oh - nh as f32) / 2.0 / self.view.zoom;
		self.screen = (nw, nh);
		// Keep windows within sensible sizes + on-screen after a viewport change.
		self.reclamp_workspace();
	}

	/// Re-apply the workspace's size and on-screen bounds at the current
	/// **logical** UI size.
	///
	/// Logical, not physical: the whole workspace - docks, floats, the reserved
	/// top strip - is laid out in logical px (`layout(wf, hf)` is fed
	/// [`ui_screen`](Self::ui_screen)), so clamping against the physical size let
	/// a float at 125% UI scale sit a quarter of the window further out than the
	/// rule allows.
	///
	/// Called after a resize and after every command that can *place* a panel
	/// (`window`, `dock`, `layout reset`): a stored or typed position is
	/// untrusted the same way a loaded one is, so a panel can never open with its
	/// titlebar under the menu bar.
	fn reclamp_workspace(&mut self) {
		let (w, h) = self.ui_screen();
		self.workspace.clamp_sizes(w, h);
		self.workspace.clamp_floating(w, h);
	}

	/// Swap the live dock layout from `from`'s group to `to`'s: stash the live
	/// layout into `from`'s slot, then restore `to`'s. A no-op when they're the
	/// same group, so switching between the two pass editors leaves their one
	/// shared layout untouched.
	fn switch_layout_group(&mut self, from: LayoutGroup, to: LayoutGroup) {
		if from == to {
			return;
		}
		let (w, h) = (self.screen.0 as f32, self.screen.1 as f32);
		self.saved_layouts[from as usize] = self.workspace.save_layout();
		let target = self.saved_layouts[to as usize].clone();
		self.workspace.load_layout(&target, w, h);
	}

	/// Populate every layout group's slot from a loaded settings INI. The live
	/// workspace already holds the applied `[Workspace]` (main) layout, so
	/// snapshot it as the Main slot; each other group loads its own
	/// `[Workspace.<Group>]` section, or - when absent - seeds from that main
	/// layout (its documented default). Call once at startup after `apply_ini`.
	pub fn seed_mode_layouts(&mut self, ini: &ini::INI, w: f32, h: f32) {
		self.saved_layouts[LayoutGroup::Main as usize] = self.workspace.save_layout();
		for group in [LayoutGroup::Pass, LayoutGroup::Save] {
			self.saved_layouts[group as usize] = match ini.get_section(group.ini_section()) {
				Some(section) => Workspace::layout_from_ini(section, w, h),
				None => self.saved_layouts[LayoutGroup::Main as usize].clone(),
			};
		}
	}

	/// Unsaved changes.
	pub fn dirty(&self) -> bool {
		self.project.dirty()
	}

	/// Edit revision (renderer watch).
	pub fn revision(&self) -> u64 {
		self.project.revision()
	}

	/// Advance the animation clock (real frame time when windowed, scripted
	/// `tick` when headless - same code path, deterministic under scripts).
	pub fn tick(&mut self, dt: f32) {
		self.clock += dt;
		self.cycler.tick(self.clock);
	}

	/// A shipped (stock) pack: not user-owned, and its folder lives under
	/// `assets_root`. Stock tiles are read-only outside `--dev`; user packs +
	/// synthetic WRL packs are editable.
	fn is_stock_pack(&self, idx: usize) -> bool {
		self.project.packs.get(idx).is_some_and(|p| !p.user && self.assets_root.join(&p.name).is_dir())
	}

	/// Where user-owned packs live: `resources/user/tilepacks`.
	fn user_tilepacks_dir(&self) -> PathBuf {
		self.resources_root.join("user/tilepacks")
	}

	/// A fresh, globally-unique tile id in `family`, matching its digit width
	/// (`GLa000` → 3). Scans every loaded pack so ids never collide (resolve_ref
	/// is by id).
	fn fresh_tile_id(&self, family: &str, width: usize) -> String {
		let used: std::collections::HashSet<u32> = self
			.project
			.packs
			.iter()
			.flat_map(|p| p.ids.iter())
			.filter(|id| map_core::family_of(id) == family)
			.filter_map(|id| id[family.len()..].parse::<u32>().ok())
			.collect();
		let mut n = 0u32;
		while used.contains(&n) {
			n += 1;
		}
		format!("{family}{n:0width$}")
	}

	/// Resolve the active brush tile to `(pack index, tile index)`.
	fn active_tile_ref(&self) -> Result<(usize, u16), String> {
		let spec = self.active_tile.as_deref().ok_or("select a tile in the Tile Explorer first")?;
		let (t, _) = self.project.resolve_ref(spec)?;
		Ok((t.pack as usize, t.tile))
	}

	/// Arm `t` as the ghost stamp at identity orientation, recording it as the
	/// base for the 8-orientation grid. The grid then shows every orientation
	/// from this base and `Command::Orient` re-derives the stamp from it.
	fn arm_stamp(&mut self, t: Template) {
		// Cache the 8 orientations of the base once (the set doesn't change as the
		// user re-orients; only the current index does).
		self.stamp_orients =
			std::array::from_fn(|i| t.oriented(&self.project, crate::toolbox::orient_transform(i)).ok());
		self.stamp = Some(t.clone());
		self.stamp_base = Some(t);
		self.stamp_xform = map_core::Transform::default();
	}

	/// Put the ghost stamp away (`Esc`, `stamp cancel`) - and whatever else the
	/// user arms instead, because the stamp takes the map click before the tool
	/// does: leaving one armed would swallow the very clicks the new thing was
	/// picked for.
	fn disarm_stamp(&mut self) {
		self.stamp = None;
		self.stamp_base = None;
		self.stamp_xform = map_core::Transform::default();
		self.stamp_orients = std::array::from_fn(|_| None);
	}

	/// Whether the armed thing (stamp or single tile) may take orientation `t` -
	/// the grid greys out the rest, and a click on a greyed cell is a no-op.
	pub fn orient_allowed(&self, t: map_core::Transform) -> bool {
		if self.stamp_base.is_some() {
			return self.stamp_orients[crate::toolbox::orient_index(t)].is_some();
		}
		match self.active_tile_ref() {
			Ok((pack, tile)) => self.project.tile_allows(pack as u8, tile, t),
			Err(_) => false,
		}
	}

	/// Set the armed thing - the stamp if one is armed, else the single active
	/// tile - to absolute orientation `t` (the 8-orientation grid's action, and
	/// the transform tool's). Refuses an orientation the tiles' families forbid.
	fn orient_armed(&mut self, t: map_core::Transform) -> Outcome {
		if let Some(base) = self.stamp_base.clone() {
			return match base.oriented(&self.project, t) {
				Ok(s) => {
					self.stamp = Some(s);
					self.stamp_xform = t;
					Outcome::Redraw
				}
				Err(e) => Outcome::Failed(format!("orient: {e}")),
			};
		}
		let Some(spec) = self.active_tile.clone() else {
			return Outcome::Failed("orient: no active tile or stamp".into());
		};
		let id = spec.split(':').next().unwrap_or(&spec);
		if let Ok((tref, _)) = self.project.resolve_ref(id) {
			if !self.project.tile_allows(tref.pack, tref.tile, t) {
				return Outcome::Failed(format!("orient: '{id}' can't take that orientation"));
			}
		}
		self.active_tile = Some(format!("{id}{}", t.suffix()));
		Outcome::Redraw
	}

	/// The bare id (no transform suffix) of the topmost tile at cell `(x, y)` -
	/// the status bar's hover readout. `None` for an empty or off-map cell.
	pub fn hovered_tile_id(&self, x: u16, y: u16) -> Option<&str> {
		let stack = self.project.cell(x, y)?;
		let top = stack[LAYER_GROUND].or(stack[LAYER_WATER])?;
		Some(&self.project.packs[top.pack as usize].ids[top.tile as usize])
	}

	/// Open the Tile Painter to edit the selected tile in place. Stock tiles
	/// need `--dev` (clone them otherwise).
	fn open_tile_edit(&mut self) -> Outcome {
		let (pack_idx, tile) = match self.active_tile_ref() {
			Ok(v) => v,
			Err(e) => return Outcome::Failed(format!("edit tile: {e}")),
		};
		if self.is_stock_pack(pack_idx) && !self.dev_mode {
			return Outcome::Failed("edit tile: shipped tiles are read-only (clone it instead)".into());
		}
		let pack = &self.project.packs[pack_idx];
		let tile_id = pack.ids[tile as usize].clone();
		self.tilepaint = Some(crate::tilepaint::TilePaintRun {
			mode: crate::tilepaint::Mode::Edit,
			tile_id: tile_id.clone(),
			pack_name: pack.name.clone(),
			mask: pack.tile_mask(tile),
			canvas: pack.tile_pixels(tile).to_vec(),
			canvas_rev: 0,
			pass: pack.pass.as_ref().map_or(0, |p| p[tile as usize]),
			// The id field starts at the current id (a rename on Save).
			id_text: tile_id,
			packs: Vec::new(),
		});
		Outcome::OpenDialog(DialogRequest::TilePaint)
	}

	/// Open the visual Edit Tile Match Data editor (DEV only) over the active
	/// map's packs, preferring the pack of the tile selected in the Tile
	/// Explorer. The staged model travels with the dialog request (the dialog
	/// owns it; Save hands self-contained commits back).
	fn open_match_editor(&mut self) -> Outcome {
		if !self.dev_mode {
			return Outcome::Failed("match editor: requires --dev".into());
		}
		let preferred = self.active_tile_ref().ok().map(|(pk, _)| pk);
		match crate::matcheditor::MatchEditor::new(&self.project, preferred) {
			Some(m) => {
				self.matchedit_stage = Some(Box::new(m));
				Outcome::OpenDialog(DialogRequest::MatchEdit)
			}
			None => Outcome::Failed("match editor: no pack has match rules to edit".into()),
		}
	}

	/// Apply the match editor's staged (symmetrized) commits to the project
	/// packs and write the changed `tiles.match.json` / `tiles.variants.json`
	/// (DEV: stock packs to `assets_root`, user packs to `user/tilepacks`).
	pub fn match_editor_save(&mut self, commits: Vec<crate::matcheditor::PackCommit>) -> Result<(), String> {
		let mut saved: Vec<String> = Vec::new();
		for c in commits {
			let name = self.project.packs[c.pack].name.clone();
			let dir = if self.is_stock_pack(c.pack) {
				if !self.dev_mode {
					return Err("match editor: shipped packs need --dev".into());
				}
				self.assets_root.join(&name)
			} else {
				self.user_tilepacks_dir().join(&name)
			};
			// 1. Tile-id renames: in-memory, then cascade the old→new id across every
			// shipped map + template (+ the pack's patterns sidecar).
			for (old, new) in &c.renames {
				if let Some(&idx) = self.project.packs[c.pack].index_of.get(old) {
					self.project.packs[c.pack].rename_tile(idx, new);
				}
			}
			if let Err(e) = self.cascade_renames(&dir, &c.renames) {
				return Err(format!("match editor: {e}"));
			}
			// 2. Pass (pack table only - maps keep their own tilepass).
			if c.pass_changed {
				for (i, &p) in c.pass.iter().enumerate() {
					self.project.packs[c.pack].set_tile_pass(i as u16, p);
				}
			}
			// 3. Grouping + match rules.
			self.project.packs[c.pack].set_match_data(c.groups, c.matches);
			// 4. Persist the pack's own files.
			if let Err(e) = self.project.packs[c.pack].save_match_data(&dir) {
				return Err(format!("match editor: {e}"));
			}
			if let Err(e) = self.project.packs[c.pack].save_ids_pass(&dir) {
				return Err(format!("match editor: {e}"));
			}
			saved.push(name);
		}
		self.console.push_line(format!("match data saved: {}", saved.join(", ")));
		Ok(())
	}

	/// Cascade tile-id renames (`old`→`new`) across every shipped map + template
	/// (cells reference ids by string) and the pack's own `tiles.patterns.json`.
	/// Token-precise (see [`map_core::replace_id_token`]); only rewrites files that
	/// actually change.
	fn cascade_renames(&self, pack_dir: &std::path::Path, renames: &[(String, String)]) -> Result<(), String> {
		if renames.is_empty() {
			return Ok(());
		}
		let mut files: Vec<PathBuf> = Vec::new();
		collect_json_files(&self.resources_root.join("assets/maps"), &mut files);
		collect_json_files(&self.resources_root.join("assets/templates"), &mut files);
		let patterns = pack_dir.join("tiles.patterns.json");
		if patterns.is_file() {
			files.push(patterns);
		}
		for f in files {
			let Ok(text) = std::fs::read_to_string(&f) else { continue };
			let mut cur = text.clone();
			let mut hits = 0usize;
			for (old, new) in renames {
				let (t, n) = map_core::replace_id_token(&cur, old, new);
				cur = t;
				hits += n;
			}
			if hits > 0 && cur != text {
				std::fs::write(&f, cur).map_err(|e| format!("{}: {e}", f.display()))?;
			}
		}
		Ok(())
	}

	/// Open the Tile Painter to clone the selected tile into a new one.
	fn open_tile_clone(&mut self) -> Outcome {
		let (pack_idx, tile) = match self.active_tile_ref() {
			Ok(v) => v,
			Err(e) => return Outcome::Failed(format!("clone tile: {e}")),
		};
		let src_id = self.project.packs[pack_idx].ids[tile as usize].clone();
		// Suggest a fresh id in the source family for the editable id field.
		let family = map_core::family_of(&src_id).to_string();
		let width = src_id.len().saturating_sub(family.len()).max(3);
		let suggested = self.fresh_tile_id(&family, width);
		let pack = &self.project.packs[pack_idx];
		self.tilepaint = Some(crate::tilepaint::TilePaintRun {
			mode: crate::tilepaint::Mode::Clone,
			tile_id: src_id,
			pack_name: pack.name.clone(),
			mask: pack.tile_mask(tile),
			canvas: pack.tile_pixels(tile).to_vec(),
			canvas_rev: 0,
			pass: pack.pass.as_ref().map_or(0, |p| p[tile as usize]),
			id_text: suggested,
			packs: Vec::new(),
		});
		Outcome::OpenDialog(DialogRequest::TilePaint)
	}

	/// Delete the selected tile from its pack. Stock tiles need `--dev`; user
	/// (and synthetic-WRL) packs delete in normal mode. The pack mutation +
	/// cell remap live in `Project::delete_tile`, which refuses an in-use tile.
	fn delete_active_tile(&mut self) -> Outcome {
		let (pack_idx, tile) = match self.active_tile_ref() {
			Ok(v) => v,
			Err(e) => return Outcome::Failed(format!("delete tile: {e}")),
		};
		let stock = self.is_stock_pack(pack_idx);
		let user = self.project.packs[pack_idx].user;
		if stock && !self.dev_mode {
			return Outcome::Failed("delete tile: shipped tiles need --dev".into());
		}
		let name = self.project.packs[pack_idx].name.clone();
		let id = self.project.packs[pack_idx].ids[tile as usize].clone();
		match self.project.delete_tile(pack_idx as u8, tile) {
			Ok(()) => {
				self.active_tile = None; // the brush pointed at the now-gone tile
				if user {
					if let Err(e) = self.persist_user_pack(&name) {
						return Outcome::Failed(format!("delete tile: {e}"));
					}
				} else if stock {
					self.tile_ops.dirty_packs.insert(name.clone());
				}
				self.console.push_line(format!("deleted tile {id} from {name}"));
				Outcome::DocReplaced
			}
			Err(e) => Outcome::Failed(format!("delete tile: {e}")),
		}
	}

	/// The packs a newly authored asset may be filed under: the map's own
	/// tilesets, in the order it declares them, minus WATER (which holds no
	/// authorable art). The map's `uses` rather than its loaded packs, because
	/// a user sidecar pack carries the *same* name as the tileset it extends -
	/// listing `packs` offers "GREEN" twice for one destination. The first
	/// entry is what both New dialogs prefill.
	pub fn authoring_pack_names(&self) -> Vec<String> {
		let mut out: Vec<String> = Vec::new();
		for u in &self.project.uses {
			if u.name != "WATER" && !out.contains(&u.name) {
				out.push(u.name.clone());
			}
		}
		out
	}

	/// Open the Tile Painter on a blank new tile (the target pack is chosen in
	/// the dialog). New tiles get no mask (fully opaque, as the map renders).
	fn open_tile_new(&mut self) -> Outcome {
		let packs = self.authoring_pack_names();
		if packs.is_empty() {
			return Outcome::Failed("new tile: no editable pack loaded".into());
		}
		self.tilepaint = Some(crate::tilepaint::TilePaintRun {
			mode: crate::tilepaint::Mode::New,
			tile_id: String::new(),
			pack_name: String::new(),
			mask: None,
			canvas: vec![0u8; crate::tilepaint::TILE * crate::tilepaint::TILE],
			canvas_rev: 0,
			pass: 0,
			id_text: String::new(),
			packs,
		});
		Outcome::OpenDialog(DialogRequest::TilePaint)
	}

	/// Commit the open Tile Painter with the dialog's values (`typed` id, `pass`,
	/// target `pack` - the shell reads them from the widgets; the script path
	/// passes the run's defaults). An Edit repaints the tile in its pack;
	/// New/Clone append a fresh tile to the per-source-name user pack under
	/// `resources/user/tilepacks/<NAME>/` (created on first use) and persist it.
	/// On success the run clears and the atlas rebuilds (DocReplaced).
	pub fn tile_paint_commit(&mut self, typed: String, pass: u8, pack: String) -> Outcome {
		use crate::tilepaint::Mode;
		let Some(run) = self.tilepaint.as_ref() else { return Outcome::Ok };
		let (mode, pixels) = (run.mode, run.canvas.clone());
		let typed = typed.trim().to_string();
		match mode {
			Mode::Edit => {
				let (pack_name, tile_id) = (run.pack_name.clone(), run.tile_id.clone());
				self.commit_tile_edit(pack_name, tile_id, typed, &pixels, pass)
			}
			Mode::Clone => {
				// A clone defaults to a fresh id in the source family; the user
				// may have typed their own. Seed the new family's props from the
				// source so the clone renders like its origin (mask/kind).
				let src_family = map_core::family_of(&run.tile_id).to_string();
				let width = run.tile_id.len().saturating_sub(src_family.len()).max(3);
				let pack_name = run.pack_name.clone();
				let id = if typed.is_empty() { self.fresh_tile_id(&src_family, width) } else { typed };
				let seed = self
					.project
					.packs
					.iter()
					.find(|p| p.name == pack_name)
					.and_then(|p| p.props.get(&src_family).cloned());
				self.commit_tile_new(pack_name, id, seed, &pixels, pass)
			}
			Mode::New => {
				// A typed id keeps its family; an empty one parks under "NEW".
				let id = if typed.is_empty() { self.fresh_tile_id("NEW", 3) } else { typed };
				self.commit_tile_new(pack, id, None, &pixels, pass)
			}
		}
	}

	/// Export the open painter's tile as a 64×64 RGBA PNG (palette colors → RGB;
	/// the family's mask color, if any, is written transparent so it round-trips).
	/// Reads the run's canvas mirror (re-synced by the shell after every edit).
	fn tile_export_png(&mut self, path: &Path) -> Outcome {
		let Some(run) = self.tilepaint.as_ref() else {
			return Outcome::Failed("tile-export: open a tile in the painter first".into());
		};
		let mask = run.mask;
		let pal = &self.project.palette;
		let mut rgba = Vec::with_capacity(run.canvas.len() * 4);
		for &i in &run.canvas {
			let o = i as usize * 3;
			let a = if Some(i) == mask { 0 } else { 255 };
			rgba.extend_from_slice(&[pal[o], pal[o + 1], pal[o + 2], a]);
		}
		let tile = crate::tilepaint::TILE as u32;
		match write_tile_png(path, &rgba, tile, tile) {
			Ok(()) => {
				let line = format!("exported tile to {}", path.display());
				eprintln!("{line}");
				self.console.push_line(line);
				Outcome::Redraw
			}
			Err(e) => Outcome::Failed(format!("tile-export: {e}")),
		}
	}

	/// Render the explorer's selected template to an RGBA PNG: each cell's tile
	/// stack composited bottom-up (water under ground, transforms applied, the
	/// family mask color → transparent so shore reveals water, holes stay clear).
	/// Full 64 px per tile, scaled down only so the long side stays within
	/// ~2048 px (a huge template stays a reasonable file).
	fn template_export_png(&mut self, path: &Path) -> Outcome {
		const TILE: u32 = 64;
		let Some(i) = self.templates.sel else {
			return Outcome::Failed("template-export-png: no template selected".into());
		};
		let t = &self.templates.entries[i].template;
		let project = &self.project;
		let (tw, th) = (t.width as u32, t.height as u32);
		if tw == 0 || th == 0 {
			return Outcome::Failed("template-export-png: the template is empty".into());
		}
		let cell = (2048 / tw.max(th)).clamp(1, TILE);
		let (out_w, out_h) = (tw * cell, th * cell);
		let pal = &project.palette;
		let mut rgba = vec![0u8; (out_w * out_h * 4) as usize]; // fully transparent
		for dy in 0..t.height {
			for dx in 0..t.width {
				// Bottom-up (water, then ground) so a masked ground pixel reveals
				// the water beneath, exactly as the map composites the stack.
				for tile in t.cell_layers(project, dx, dy).into_iter().flatten() {
					let pack = &project.packs[tile.pack as usize];
					let src = map_core::transform_tile(pack.tile_pixels(tile.tile), tile.transform);
					let mask = pack.tile_mask(tile.tile);
					for sy in 0..cell {
						let ty = (sy * TILE / cell) as usize; // nearest source row when scaled
						for sx in 0..cell {
							let tx = (sx * TILE / cell) as usize;
							let idx = src[ty * TILE as usize + tx];
							if Some(idx) == mask {
								continue; // transparent: leave the lower layer showing
							}
							let (ox, oy) = (dx as u32 * cell + sx, dy as u32 * cell + sy);
							let o = ((oy * out_w + ox) * 4) as usize;
							let p = idx as usize * 3;
							rgba[o..o + 4].copy_from_slice(&[pal[p], pal[p + 1], pal[p + 2], 255]);
						}
					}
				}
			}
		}
		match write_tile_png(path, &rgba, out_w, out_h) {
			Ok(()) => {
				let line = format!("exported template to {} ({out_w}x{out_h})", path.display());
				eprintln!("{line}");
				self.console.push_line(line);
				Outcome::Redraw
			}
			Err(e) => Outcome::Failed(format!("template-export-png: {e}")),
		}
	}

	/// Load a PNG into the open painter, mapping each pixel to its visually
	/// closest palette color (nearest RGB). Non-64×64 images are nearest-sampled
	/// to the tile; transparent pixels become the family's mask color. Writes
	/// the run's canvas and bumps its revision, so the dialog reloads its copy.
	fn tile_import_png(&mut self, path: &Path) -> Outcome {
		let Some(run) = self.tilepaint.as_ref() else {
			return Outcome::Failed("tile-import: open a tile in the painter first".into());
		};
		let mask = run.mask.unwrap_or(0);
		let (rgba, w, h) = match decode_png_rgba(path) {
			Ok(v) => v,
			Err(e) => return Outcome::Failed(format!("tile-import: {e}")),
		};
		if w == 0 || h == 0 {
			return Outcome::Failed("tile-import: empty image".into());
		}
		let tile = crate::tilepaint::TILE;
		let pal = &self.project.palette;
		let mut indices = vec![0u8; tile * tile];
		for ty in 0..tile {
			for tx in 0..tile {
				// Nearest-neighbour sample so any image size maps onto the tile.
				let sx = (tx * w as usize / tile).min(w as usize - 1);
				let sy = (ty * h as usize / tile).min(h as usize - 1);
				let p = (sy * w as usize + sx) * 4;
				let a = rgba[p + 3];
				indices[ty * tile + tx] =
					if a < 128 { mask } else { nearest_palette_index(pal, rgba[p], rgba[p + 1], rgba[p + 2]) };
			}
		}
		let run = self.tilepaint.as_mut().unwrap();
		run.canvas = indices;
		run.canvas_rev += 1;
		let line = format!("imported {} ({w}x{h}) into the tile painter", path.display());
		eprintln!("{line}");
		self.console.push_line(line);
		Outcome::Redraw
	}

	/// A valid, available tile id. `allow` is the id the caller may keep (an
	/// in-place rename to itself); any other collision across all packs fails.
	fn validate_tile_id(&self, id: &str, allow: Option<&str>) -> Result<(), String> {
		if id.is_empty() {
			return Err("id is empty".into());
		}
		if !id.chars().all(crate::tilepaint::is_id_char) {
			return Err("id: only letters, digits and _".into());
		}
		if Some(id) != allow && self.project.packs.iter().any(|p| p.index_of.contains_key(id)) {
			return Err(format!("id '{id}' already exists"));
		}
		Ok(())
	}

	/// Repaint (and optionally rename) an existing tile in place (Edit). Stock
	/// tiles need `--dev`.
	fn commit_tile_edit(
		&mut self,
		pack_name: String,
		tile_id: String,
		new_id: String,
		pixels: &[u8],
		pass: u8,
	) -> Outcome {
		let Some(pack_idx) = self.project.packs.iter().position(|p| p.name == pack_name) else {
			return Outcome::Failed(format!("tile: pack '{pack_name}' is not loaded"));
		};
		if self.is_stock_pack(pack_idx) && !self.dev_mode {
			return Outcome::Failed("tile: editing shipped tiles needs --dev".into());
		}
		let Some(&tile) = self.project.packs[pack_idx].index_of.get(&tile_id) else {
			return Outcome::Failed(format!("tile: '{tile_id}' not found in {pack_name}"));
		};
		let renaming = new_id != tile_id;
		if renaming {
			if let Err(e) = self.validate_tile_id(&new_id, Some(&tile_id)) {
				return Outcome::Failed(format!("tile: {e}"));
			}
		}
		let stock = self.is_stock_pack(pack_idx);
		{
			let pack = &mut self.project.packs[pack_idx];
			pack.set_tile_pixels(tile, pixels);
			pack.set_tile_pass(tile, pass);
			if renaming {
				pack.rename_tile(tile, &new_id);
			}
		}
		if renaming && self.active_tile.as_deref() == Some(&tile_id) {
			self.active_tile = Some(new_id.clone());
		}
		// A user pack persists to its own folder now; a dev edit of a stock pack
		// persists only on Bake (recorded as dirty); a synthetic WRL pack rides
		// the project's own save.
		if self.project.packs[pack_idx].user {
			if let Err(e) = self.persist_user_pack(&pack_name) {
				return Outcome::Failed(format!("tile: {e}"));
			}
		} else if stock {
			self.tile_ops.dirty_packs.insert(pack_name.clone());
		}
		self.tilepaint = None;
		self.console.push_line(format!("edited tile {new_id} in {pack_name}"));
		Outcome::DocReplaced
	}

	/// Append a fresh tile to the target pack. In `--dev` a new/cloned tile may
	/// extend the stock pack directly (Bake ships it); otherwise it lands in the
	/// user pack mirroring the stock pack's name, persisted at once.
	fn commit_tile_new(
		&mut self,
		stock_name: String,
		new_id: String,
		seed_props: Option<map_core::FamilyProps>,
		pixels: &[u8],
		pass: u8,
	) -> Outcome {
		if let Err(e) = self.validate_tile_id(&new_id, None) {
			return Outcome::Failed(format!("tile: {e}"));
		}
		// In dev mode, grow the stock pack itself (so Bake writes it back);
		// otherwise grow (or create) the matching user pack.
		let target_user = !self.dev_mode;
		let pack_idx = if target_user {
			match self.find_or_make_user_pack(&stock_name) {
				Ok(i) => i,
				Err(e) => return Outcome::Failed(format!("tile: {e}")),
			}
		} else {
			match self.project.packs.iter().position(|p| p.name == stock_name && !p.user) {
				Some(i) => i,
				None => return Outcome::Failed(format!("tile: pack '{stock_name}' is not loaded")),
			}
		};

		// Seed the new id's family props (mask/kind) so the tile renders like its
		// kin: the source family's props for a clone, else any pack already
		// defining the family, else a plain opaque-land default.
		let family = map_core::family_of(&new_id).to_string();
		if !self.project.packs[pack_idx].props.contains_key(&family) {
			let props = seed_props
				.or_else(|| self.project.packs.iter().find_map(|p| p.props.get(&family).cloned()))
				.unwrap_or_default();
			self.project.packs[pack_idx].props.insert(family.clone(), props);
		}

		let pack = &mut self.project.packs[pack_idx];
		pack.push_tile(new_id.clone(), pixels, pass);
		let pack_user = pack.user;
		let pack_name = pack.name.clone();

		if pack_user {
			if let Err(e) = self.persist_user_pack(&pack_name) {
				return Outcome::Failed(format!("tile: {e}"));
			}
		} else {
			// A new tile grown into a stock pack (dev) ships on Bake.
			self.tile_ops.dirty_packs.insert(pack_name.clone());
		}
		// Make the new tile the active brush, ready to paint.
		self.active_tile = Some(new_id.clone());
		self.tilepaint = None;
		let where_ = if pack_user { format!("user pack {pack_name}") } else { pack_name.clone() };
		self.console.push_line(format!("added tile {new_id} to {where_}"));
		Outcome::DocReplaced
	}

	/// The index of the user pack named `stock_name`, creating + appending an
	/// empty one if the session doesn't have it yet.
	fn find_or_make_user_pack(&mut self, stock_name: &str) -> Result<usize, String> {
		if let Some(i) = self.project.packs.iter().position(|p| p.user && p.name == stock_name) {
			return Ok(i);
		}
		self.project.packs.push(map_core::TilePack::empty_user(stock_name));
		Ok(self.project.packs.len() - 1)
	}

	/// Write a user pack to `resources/user/tilepacks/<NAME>/`.
	fn persist_user_pack(&self, name: &str) -> Result<(), String> {
		let root = self.user_tilepacks_dir();
		let pack = self.project.packs.iter().find(|p| p.user && p.name == name).ok_or("user pack vanished")?;
		pack.dump(&root.join(name))
	}

	/// Bake the stock packs edited this session back to `resources/assets/tilepacks/<NAME>/`
	/// (`--dev` only) - repaints, passability, and any new tiles. `dump` rewrites
	/// pixels/ids/pass/props/variants and leaves match/pattern files intact.
	fn bake(&mut self) -> Outcome {
		if !self.dev_mode {
			return Outcome::Failed("bake: requires --dev".into());
		}
		let dirty: Vec<String> = self.tile_ops.dirty_packs.iter().cloned().collect();
		let mut report = Vec::new();
		for name in dirty {
			let Some(idx) = self.project.packs.iter().position(|p| p.name == name && !p.user) else { continue };
			if !self.is_stock_pack(idx) {
				continue; // only shipped packs bake to assets_root
			}
			match self.project.packs[idx].bake_changed(&self.assets_root.join(&name)) {
				Ok(files) => {
					self.tile_ops.dirty_packs.remove(&name);
					if !files.is_empty() {
						report.push(format!("{name} ({})", files.join(", ")));
					}
				}
				Err(e) => return Outcome::Failed(format!("bake: {e}")),
			}
		}
		if report.is_empty() {
			return Outcome::Failed("bake: nothing changed - paint or add tiles in --dev first".into());
		}
		let line = format!("baked to {}: {}", self.assets_root.display(), report.join("; "));
		eprintln!("{line}");
		self.console.push_line(line);
		Outcome::Redraw
	}

	/// Reset the map's per-tile passability to each tileset's shipped values
	/// (Tools ▸ Reset Pass Table to Tileset) - reverting Pass Table Editor edits
	/// and any `tilepass` block a loaded map carried. Per-cell overrides stay.
	/// Each pack's canonical pass is taken from a fresh load of its source
	/// tileset (shipped under `assets_root`, else a user pack), mapped by tile
	/// **id** so `--dev` session tiles aren't disturbed. Synthetic (WRL) packs
	/// with no source tileset are left as-is.
	fn reset_tile_pass(&mut self) -> Outcome {
		let user_root = self.user_tilepacks_dir();
		let mut canonical: Vec<Option<Vec<u8>>> = Vec::with_capacity(self.project.packs.len());
		for i in 0..self.project.packs.len() {
			let Some(mut want) = self.project.packs[i].pass.clone() else {
				canonical.push(None); // no pass table → nothing to reset
				continue;
			};
			let name = self.project.packs[i].name.clone();
			let fresh = map_core::TilePack::load(&self.assets_root, &name)
				.or_else(|_| map_core::TilePack::load(&user_root, &name))
				.ok();
			let Some(fresh) = fresh.filter(|f| f.pass.is_some()) else {
				canonical.push(None); // synthetic/WRL pack: no tileset to reset to
				continue;
			};
			let fresh_pass = fresh.pass.as_ref().unwrap();
			// Map by id: a tile present in the tileset takes its shipped pass; a
			// tile added this session (absent there) keeps its current value.
			for ti in 0..self.project.packs[i].tile_count() as usize {
				if let Some(&fi) = fresh.index_of.get(&self.project.packs[i].ids[ti]) {
					want[ti] = fresh_pass[fi as usize];
				}
			}
			canonical.push(Some(want));
		}
		if self.project.reset_tile_pass(&canonical) {
			let line = "reset pass table to the tileset values".to_string();
			self.console.push_line(line);
			Outcome::Redraw
		} else {
			self.console.push_line("reset pass: already matches the tileset".to_string());
			Outcome::Ok
		}
	}

	/// Log a failed command to the console scrollback. Interactive runs also
	/// raise the error dialog (a wgpu-ui overlay) — the shell does that in
	/// `App::act_on`, since the overlay lives shell-side.
	pub fn raise_error(&mut self, message: &str) {
		self.console.push_line(format!("error: {message}"));
	}

	/// Whether the Auto Fix Shore run is live (the shell keeps redrawing +
	/// ticking it while so).
	pub fn autofix_running(&self) -> bool {
		self.autofix.as_ref().is_some_and(|a| a.running)
	}

	/// Whether the Fix Shore window is open (running or idle). The shell paints
	/// the red defect outlines whenever it is.
	pub fn autofix_open(&self) -> bool {
		self.autofix.is_some()
	}

	/// Open the Fix Shore run state seeded with the live (match.json) defect
	/// count, stashing the broken cells so the map outlines them in red the
	/// moment the window appears - before the run even starts. The caller
	/// returns [`DialogRequest::AutoFix`] so the shell shows the window.
	/// Recompute the problem-overlay cell sets for whichever "Show ..." toggles
	/// are on, but only when the map has changed since the last compute (the
	/// render loop calls this each frame; the toggle handlers reset the stamp to
	/// `u64::MAX` to force one). Clears the cache when a toggle is off.
	pub fn refresh_problem_overlays(&mut self) {
		let rev = self.project.revision();
		if self.show_shore_bugs {
			if self.shore_bug_rev != rev {
				self.shore_bug_cells = self.project.shore_defect_cells(None);
				self.shore_bug_rev = rev;
			}
		} else if !self.shore_bug_cells.is_empty() {
			self.shore_bug_cells.clear();
		}
		if self.show_match_problems {
			if self.match_problem_rev != rev {
				self.match_problem_cells = self.project.match_defect_cells(None);
				self.match_problem_rev = rev;
			}
		} else if !self.match_problem_cells.is_empty() {
			self.match_problem_cells.clear();
		}
	}

	pub fn open_autofix(&mut self) {
		// Confine the whole run to the active selection (its bounding rect); with
		// no selection the run covers the whole map.
		let region = self.selection.bounds();
		self.autofix_defects = self.project.shore_defect_cells(region);
		let found = self.autofix_defects.len();
		self.autofix = Some(FixRun::new(found, region));
	}

	/// Close the Fix Shore window: a running fix commits what's already laid
	/// (its one undo unit); the defect outlines disappear with the run state.
	pub fn autofix_close(&mut self) {
		if self.autofix.as_ref().is_some_and(|a| a.running) {
			self.project.end_stroke();
		}
		self.autofix = None;
	}

	/// Carve a freshly-created (all-water) map's coastline from a shape image,
	/// then open the Fix Shore modal so the user picks a shoring method. The
	/// image classifies each tile as land or water (see [`shape_land_mask`]);
	/// land tiles are laid flat on the ground layer exactly as the terrain brush
	/// does, leaving the boundary unshored - the user's chosen Fix Shore pass
	/// grows the beach + animated coast over it (one undo unit, like a brush
	/// stroke). Called right after the `new!` command, on the new map.
	pub fn apply_shape_image(&mut self, image: &Path) -> Outcome {
		let (rgba, iw, ih) = match decode_png_rgba(image) {
			Ok(v) => v,
			Err(e) => return Outcome::Failed(format!("new map shape: {e}")),
		};
		let (w, h) = (self.project.width, self.project.height);
		let land = shape_land_mask(&rgba, iw, ih, w, h);

		let Some((pack_idx, family)) = self.project.variant_family(TileKind::Land) else {
			return Outcome::Failed("new map shape: no pack has a LAND variant group (tiles.props.json)".into());
		};
		let tiles = self.project.packs[pack_idx].group_tiles(&family);
		if tiles.is_empty() {
			return Outcome::Failed(format!("new map shape: '{family}' has no tiles"));
		}

		self.project.begin_stroke();
		let mut edits = Vec::new();
		for y in 0..h {
			for x in 0..w {
				if land[y as usize * w as usize + x as usize] {
					let tile = tiles[self.paint_rng.below(tiles.len() as u32) as usize];
					let tref = TileRef { pack: pack_idx as u8, tile, transform: Transform::default() };
					edits.push((x, y, LAYER_GROUND, Some(tref)));
				}
			}
		}
		self.project.place_many(&edits);
		self.project.end_stroke();

		// Hand off to Fix Shore (the auto-shore window) on the raw land/water
		// boundary; the user picks the method and Starts it from there.
		self.open_autofix();
		Outcome::OpenDialog(DialogRequest::AutoFix)
	}

	pub fn autofix_start(&mut self) {
		if self.autofix.is_none() {
			return;
		}
		use map_core::FixStrength;
		let region = self.autofix.as_ref().and_then(|a| a.region);
		self.project.begin_stroke();
		// Placement: lay missing shore with the backtracking loop-walk.
		let (placed, _) = self.project.auto_shore_alt(region);
		// Clear shore tiles stranded in the land (a shore tile in the middle of
		// the land is always a mistake): replace each with a random land tile.
		let cleared = self.project.replace_orphan_shore(region);
		self.autofix_defects = self.project.shore_defect_cells(region);
		let defects = self.autofix_defects.len();
		if defects > 0 {
			// Resolve the residue across frames + passes, re-tiling shore-band
			// cells ONLY (`Shore`): the tool never reshapes land or water, so a
			// seam the tileset cannot close stays flagged rather than blasting
			// the terrain to force it shut.
			let session = self.project.fix_session(region, FixStrength::Shore);
			if let Some(af) = self.autofix.as_mut() {
				af.total_changed = placed + cleared;
				af.found = defects;
				af.fixed = 0;
				af.remaining = defects;
				af.best = defects;
				af.passes = 0;
				af.stall = 0;
				af.elapsed = 0.0;
				af.applied = None;
				af.running = true;
				af.session = Some(session);
			}
		} else {
			// Already clean after placement + orphan cleanup: commit now.
			self.project.end_stroke();
			if let Some(af) = self.autofix.as_mut() {
				af.total_changed = placed + cleared;
				af.found = defects;
				af.fixed = 0;
				af.remaining = defects;
				af.running = false;
				af.session = None;
				af.applied = Some(placed + cleared);
			}
		}
	}

	/// Abort a running fix: revert the whole run (its one undo unit) back to the
	/// coast it started from, and reset the dialog to idle.
	pub fn autofix_abort(&mut self) {
		if self.autofix.as_ref().is_none_or(|a| !a.running) {
			return;
		}
		self.project.end_stroke();
		self.project.undo();
		let region = self.autofix.as_ref().and_then(|a| a.region);
		self.autofix_defects = self.project.shore_defect_cells(region);
		let defects = self.autofix_defects.len();
		if let Some(af) = self.autofix.as_mut() {
			af.running = false;
			af.session = None;
			af.applied = None;
			af.total_changed = 0;
			af.found = defects;
			af.fixed = 0;
			af.remaining = defects;
			af.best = usize::MAX;
			af.passes = 0;
			af.stall = 0;
			af.elapsed = 0.0;
		}
	}

	/// Step the live fix run a bounded slice per frame, looping place + fix
	/// passes until the coast stops improving. Every tick steps the current pass
	/// a small budget and **applies its in-progress tiles straight to the map**,
	/// so the coast is seen resolving cell-by-cell (not only at pass
	/// boundaries); when a pass finishes it re-lays any still-missing coast,
	/// re-detects the defects (the red outlines update from `autofix_defects`),
	/// then starts a fresh pass or finishes. The fix re-tiles **shore-band cells
	/// only** (`FixStrength::Shore`) - it never reshapes land or water, so the
	/// run converges to a clean coast where the tileset allows it and otherwise
	/// settles, leaving the seams it cannot close flagged in red rather than
	/// destroying terrain to force them shut. `stop` finalises early. Everything
	/// commits as one undo unit via `begin_stroke` in `autofix_start` and
	/// `end_stroke` here or on close. The small per-tick budget (the shell loops
	/// it within a wall-clock slice) keeps the UI responsive and the map
	/// updating while the run grinds.
	pub fn autofix_tick(&mut self, elapsed: f32, stop: bool) -> Outcome {
		// One window's worth of nodes per tick - small enough that a single tick
		// never blocks; the shell runs several within its per-frame time budget.
		const STEP_BUDGET: i64 = 60_000;
		let Some(mut af) = self.autofix.take() else { return Outcome::Ok };
		if af.running {
			use map_core::FixStrength;
			af.elapsed = elapsed;
			let pass_over = if let Some(session) = af.session.as_mut() {
				if !stop {
					session.step(STEP_BUDGET);
				}
				// Live: push the pass's in-progress tiles to the map every tick.
				// `place_many` no-ops unchanged cells, so this records only the
				// frame's delta into the run's single undo unit - the user
				// watches the coast settle instead of waiting for the pass.
				af.total_changed += session.apply(&mut self.project);
				stop || session.is_done()
			} else {
				true
			};
			if pass_over {
				// The pass's tiles are already on the map (applied live above);
				// re-lay any still-missing coast, clear shore stranded in the land
				// or floating on open water, then faithfully re-detect. Repeated
				// each pass so stranded chunks resolve until nothing improves.
				let region = af.region;
				let (placed, _) = self.project.auto_shore_alt(region);
				af.total_changed += placed + self.project.replace_orphan_shore(region);
				self.autofix_defects = self.project.shore_defect_cells(region);
				let defects = self.autofix_defects.len();
				af.remaining = defects;
				af.fixed = af.found.saturating_sub(defects);
				if defects < af.best {
					af.best = defects;
					af.stall = 0;
				} else {
					af.stall += 1;
				}
				af.passes += 1;
				// Shore-only fixing converges fast; stop once it stops improving
				// (or is clean / hits the hard cap). What it cannot close stays
				// flagged - the tool never reshapes terrain to force a seam shut.
				let finish = stop || defects == 0 || af.passes >= 64 || af.stall >= 2;
				if finish {
					af.running = false;
					af.session = None;
					self.project.end_stroke();
					af.applied = Some(af.total_changed);
				} else {
					// Next pass on the freshly re-laid coast (band cells only).
					af.session = Some(self.project.fix_session(region, FixStrength::Shore));
				}
			}
		}
		self.autofix = Some(af);
		Outcome::Redraw
	}

	/// Whether a terrain generation run is live (the shell keeps redrawing +
	/// stepping it while so).
	pub fn generate_running(&self) -> bool {
		self.genrun.as_ref().is_some_and(|g| g.running)
	}

	/// Open the Generate run state (the dialog stays open across runs). The
	/// caller returns [`DialogRequest::Generate`] so the shell shows the window.
	pub fn open_generate(&mut self) {
		self.genrun = Some(GenerateRun::default());
	}

	/// Drop the Generate run state (the dialog closed; Close is disabled while
	/// a run is live, so nothing needs rolling back here).
	pub fn generate_close(&mut self) {
		self.genrun = None;
	}

	/// The stock + user templates compatible with the current map - the feature
	/// pool the generator stamps as obstructions / decorations (classified by
	/// their tiles in `map-core`). Empty for a tileset with no templates.
	pub fn feature_templates(&self) -> Vec<map_core::Template> {
		self.templates
			.entries
			.iter()
			.filter(|e| e.template.compatible(&self.project))
			.map(|e| e.template.clone())
			.collect()
	}

	/// Begin a generation run with the dialog's (validated) settings. A `None`
	/// seed rolls a fresh one (reported, so the map can be re-made).
	pub fn generate_start(&mut self, mut params: map_core::GenParams, seed: Option<u64>) -> Outcome {
		if self.genrun.as_ref().is_none_or(|g| g.running) {
			return Outcome::Ok;
		}
		params.seed = seed.unwrap_or_else(roll_seed);
		let feats = self.feature_templates();
		match map_core::GenSession::new(&self.project, params, &feats) {
			Ok(session) => {
				if let Some(g) = self.genrun.as_mut() {
					g.session = Some(session);
					g.started = Some(params);
					g.running = true;
					g.status = vec![format!("seed {}", params.seed)];
				}
				Outcome::Redraw
			}
			Err(e) => Outcome::Failed(format!("generate: {e}")),
		}
	}

	/// Step (or abort) the live generation run - the shell calls this per
	/// frame within a time budget. Completion reports to the console; an
	/// abort rolls the document back to before the run.
	pub fn generate_tick(&mut self, abort: bool) -> Outcome {
		let Some(mut run) = self.genrun.take() else { return Outcome::Ok };
		if run.running {
			if let Some(mut session) = run.session.take() {
				if abort {
					session.abort(&mut self.project);
					run.running = false;
					run.status = vec!["aborted".into()];
					self.console.push_line("generate: aborted, map rolled back");
				} else if session.step(&mut self.project) {
					let stats = session.stats().expect("stats set when done");
					let started = run.started.as_ref().expect("started set on start");
					run.status = generate_status_lines(started, stats);
					self.console.push_line(generate_report(started, stats));
					run.running = false;
				} else {
					run.session = Some(session);
				}
			} else {
				run.running = false;
			}
		}
		self.genrun = Some(run);
		Outcome::Redraw
	}

	/// Whether the New-from-Image conversion is live (the shell keeps redrawing
	/// + stepping it while so).
	pub fn converting(&self) -> bool {
		self.newimage.as_ref().is_some_and(|m| m.running)
	}

	/// Open the New-from-Image run (dialog settings; nothing decoded yet). `path`
	/// pixels are read on the first `convert_tick`.
	pub fn open_newimage(&mut self, path: PathBuf, name: String, opts: map_core::ConvertOpts) {
		self.newimage = Some(NewImageRun {
			path,
			name,
			opts,
			session: None,
			running: false,
			progress: 0.0,
			stage: String::new(),
			elapsed: 0.0,
		});
	}

	/// Drop the New-from-Image run (the dialog closed).
	pub fn newimage_cancel(&mut self) {
		self.newimage = None;
	}

	/// Begin the New-from-Image conversion with the dialog's (validated) `opts`;
	/// the image pixels load on the first `convert_tick` (the "Loading image"
	/// stage), so a click on Convert is instant.
	pub fn convert_start(&mut self, opts: map_core::ConvertOpts) -> Outcome {
		let Some(m) = self.newimage.as_mut() else { return Outcome::Ok };
		if m.running {
			return Outcome::Ok;
		}
		m.opts = opts;
		m.session = None;
		m.running = true;
		m.progress = 0.0;
		m.elapsed = 0.0;
		m.stage = "Loading image...".to_string();
		Outcome::Redraw
	}

	/// Step the live conversion a bounded slice; `elapsed` is wall-clock since
	/// Convert (for the display + ETA). On completion, opens the result as a new
	/// tab; `abort` stops the run and returns to the settings.
	pub fn convert_tick(&mut self, elapsed: f32, abort: bool) -> Outcome {
		let Some(mut m) = self.newimage.take() else { return Outcome::Ok };
		let mut outcome = Outcome::Redraw;
		if m.running {
			m.elapsed = elapsed;
			if abort {
				m.running = false;
				m.session = None;
				m.stage = "Aborted".to_string();
			} else if m.session.is_none() {
				// First stage: load the image pixels and prepare the session.
				match build_convert_session(&m.path, m.opts) {
					Ok(session) => {
						m.session = Some(session);
						m.stage = "Loading image...".to_string();
					}
					Err(e) => {
						m.running = false;
						m.stage = format!("Failed: {e}");
						outcome = Outcome::Failed(format!("convert: {e}"));
					}
				}
			} else if let Some(session) = m.session.as_mut() {
				// ~300k pixel-units/frame keeps a frame responsive; the shell
				// loops this while `converting()`.
				session.step(300_000);
				m.progress = session.progress();
				m.stage = session.stage().to_string();
				if session.is_done() {
					m.running = false;
					let result = m.session.take().unwrap().finish();
					match result {
						Ok(wrl) => {
							let name = m.name.clone();
							let project = Project::from_wrl(&wrl, &name);
							eprintln!(
								"imported image: {}x{} cells, {} tiles",
								project.width, project.height, wrl.tile_count
							);
							// Modal done - open the new tab (drops `m`).
							return self.add_doc(project, None, None);
						}
						Err(e) => {
							m.stage = format!("Failed: {e}");
							outcome = Outcome::Failed(format!("convert: {e}"));
						}
					}
				}
			}
		}
		self.newimage = Some(m);
		outcome
	}

	/// Run the parked WRL import's match against the dialog's chosen packs. The
	/// match is fast (a hashmap over the packs), so it runs synchronously on the
	/// Import press: a clean match opens the converted map at once, otherwise the
	/// dialog switches to its unmapped-review stage. A failure drops the run
	/// (the error dialog replaces the picker).
	pub fn wrl_match(&mut self, packs: Vec<String>, owner: String) -> Outcome {
		let Some(run) = self.wrlimport.as_ref() else { return Outcome::Ok };
		let path = run.path.clone();
		let name = run.name.clone();
		let wrl = match read_wrl_file(&path) {
			Ok(w) => w,
			Err(e) => {
				self.wrlimport = None;
				return Outcome::Failed(format!("import-wrl {}: {e}", path.display()));
			}
		};
		// Deterministic seed: the water fill beneath matched/dropped cells (and
		// thus the import) reproduces exactly for the same WRL + pack choice.
		let import = match map_core::WrlImport::new(wrl, &name, &owner, &packs, &self.assets_root, 0) {
			Ok(i) => i,
			Err(e) => {
				self.wrlimport = None;
				return Outcome::Failed(format!("import-wrl: {e}"));
			}
		};
		// A clean match: drop the run (the shell hides the dialog) and open the
		// converted map straight away.
		if import.unmapped().is_empty() {
			self.wrlimport = None;
			let (project, _) = import.finish(map_core::ExtrasDest::Ignore);
			return self.add_doc(project, None, None);
		}
		if let Some(run) = self.wrlimport.as_mut() {
			run.used = import.used_tiles();
			run.matched = import.matched_tiles();
			run.rows = import
				.unmapped()
				.iter()
				.map(|u| {
					format!(
						"{}   {}   {} cell{}",
						u.id,
						class_name(u.pass),
						u.cells,
						if u.cells == 1 { "" } else { "s" }
					)
				})
				.collect();
			run.result = Some(import);
		}
		Outcome::Redraw
	}

	/// Commit the parked WRL import: place its unmapped tiles per the chosen
	/// destination, open the converted map as a new tab, and persist the user
	/// pack when the extras were folded into the user tileset.
	pub fn wrl_finish(&mut self, dest: map_core::ExtrasDest) -> Outcome {
		let Some(mut run) = self.wrlimport.take() else { return Outcome::Ok };
		let Some(import) = run.result.take() else {
			// Finish only fires in the unmapped stage; keep the run if not.
			self.wrlimport = Some(run);
			return Outcome::Ok;
		};
		let (project, persist) = import.finish(dest);
		let outcome = self.add_doc(project, None, None);
		if let Some(name) = persist {
			if let Err(e) = self.persist_user_pack(&name) {
				self.console.push_line(format!("import-wrl: saving user pack '{name}' failed: {e}"));
			}
		}
		outcome
	}

	/// Step the Import WRL dialog back from the unmapped review to the pack
	/// picker (discarding the match, which is cheap to redo).
	pub fn wrl_back(&mut self) {
		if let Some(run) = self.wrlimport.as_mut() {
			run.result = None;
			run.rows.clear();
		}
	}

	/// Drop the parked WRL import (the dialog closed).
	pub fn wrl_cancel(&mut self) {
		self.wrlimport = None;
	}

	/// Whether the rasterize palette conversion is live (the shell keeps
	/// redrawing + stepping it while so).
	pub fn palette_converting(&self) -> bool {
		self.pconvert.as_ref().is_some_and(|m| m.running)
	}

	/// Begin the rasterize palette conversion with the dialog's (validated)
	/// options; the session itself is built on the first `palette_convert_tick`
	/// so a click on Convert paints the running state instantly. `threshold` is
	/// the relaxed similarity as a fraction (0..=1).
	pub fn palette_convert_start(&mut self, water: bool, relaxed: bool, threshold: f32) -> Outcome {
		if self.palette_converting() {
			return Outcome::Ok;
		}
		self.pconvert = Some(PaletteConvertRun {
			running: true,
			session: None,
			progress: 0.0,
			stage: "Rendering map".to_string(),
			elapsed: 0.0,
			water,
			relaxed,
			threshold,
		});
		Outcome::Redraw
	}

	/// Drop the palette-conversion state (the dialog closed; a live session is
	/// simply discarded — nothing was applied to the document yet).
	pub fn palette_convert_cancel(&mut self) {
		self.pconvert = None;
	}

	/// Step the live palette conversion a bounded slice; `elapsed` is wall-
	/// clock since Convert (display + ETA). On completion the document
	/// content swaps in (one undo unit) and the modal closes; `abort` stops
	/// the run and returns to the options.
	pub fn palette_convert_tick(&mut self, elapsed: f32, abort: bool) -> Outcome {
		let Some(mut m) = self.pconvert.take() else { return Outcome::Ok };
		let mut outcome = Outcome::Redraw;
		if m.running {
			m.elapsed = elapsed;
			if abort {
				m.running = false;
				m.session = None;
				m.stage = "Aborted".to_string();
			} else {
				if m.session.is_none() {
					let dedupe = if m.relaxed { map_core::Dedupe::Relaxed } else { map_core::Dedupe::Strict };
					m.session = Some(map_core::PaletteReimport::new(&self.project, m.water, dedupe, m.threshold));
				}
				if let Some(session) = m.session.as_mut() {
					// ~300k pixel-units/frame keeps a frame responsive; the
					// shell loops this while `palette_converting()`.
					session.step(&self.project, 300_000);
					m.progress = session.progress();
					m.stage = session.stage().to_string();
					if session.is_done() {
						m.running = false;
						match m.session.take().unwrap().finish() {
							Ok(wrl) => {
								let tile_count = self.project.apply_reimport(&wrl);
								self.refresh_palette();
								let line = format!(
									"palette converted by re-import: {tile_count} tiles rebuilt, water {} \
									 (lossy, undoable)",
									if m.water { "kept animated" } else { "flattened" },
								);
								eprintln!("{line}");
								self.console.push_line(line);
								// Modal done - drop it; the atlas must rebuild.
								return Outcome::DocReplaced;
							}
							Err(e) => {
								m.stage = format!("Failed: {e}");
								outcome = Outcome::Failed(format!("convert-palette: {e}"));
							}
						}
					}
				}
			}
		}
		self.pconvert = Some(m);
		outcome
	}

	/// Whether `path` is one of the shipped read-only maps
	/// (`resources/assets/maps/`). Those load path-less so a Save never
	/// overwrites them (Save → Save-As), same as an imported WRL.
	fn is_template(&self, path: &Path) -> bool {
		path.starts_with(self.resources_root.join("assets/maps"))
	}

	/// Is the open document a template-born map - opened from a shipped
	/// template (path-less, origin under `assets/maps`) and never saved? The
	/// first-save Map Metadata prompt then blanks date/version/author.
	pub fn doc_from_template(&self) -> bool {
		self.path.is_none() && self.origin.as_deref().is_some_and(|o| self.is_template(o))
	}

	/// Quick Load entries from the recent list (label = the file name).
	fn recent_map_entries(&self) -> Vec<crate::menu::MapEntry> {
		self.recent
			.iter()
			.map(|path| {
				let label = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
				crate::menu::MapEntry { label, note: None, path: path.clone() }
			})
			.collect()
	}

	/// Record `path` as a recently-opened map (most-recent first, deduped, ≤10)
	/// and refresh the Quick Load submenu. Templates are excluded - they live in
	/// the Template Maps submenu, not the user's history.
	/// Persist the `[Preferences]` section immediately (like Quick Load):
	/// small user options that shouldn't wait for the exit-time settings save.
	pub fn save_preferences(&self) {
		if let Some(path) = self.settings_path.as_deref() {
			let mut prefs = ini::INISection::new();
			let _ = prefs.set_entry("PalettePreview".to_string(), self.palette_preview);
			let _ = crate::settings_io::save_preferences(path, prefs);
		}
	}

	/// Apply Editor Preferences: set the M.A.X. / M.A.X. Port folder paths (blank
	/// → unset) and the "don't ask again" flag, persist them to `[Paths]`, and -
	/// if `MaxPath` changed - drop the unit/marker libraries so they reload from
	/// the new folder on next use.
	pub fn apply_preferences(&mut self, max_path: String, max_port_path: String, max_port_data: String, skip: bool) {
		let to_opt = |s: String| {
			let t = s.trim().to_string();
			(!t.is_empty()).then_some(PathBuf::from(t))
		};
		let new_max = to_opt(max_path);
		let max_changed = new_max != self.max_path;
		self.max_path = new_max;
		self.max_port_path = to_opt(max_port_path);
		let new_data = to_opt(max_port_data);
		let data_changed = new_data != self.max_port_data_path;
		self.max_port_data_path = new_data;
		self.skip_path_prompt = skip;
		self.paths_prompt_reason = None; // the paths were just provided
		if max_changed {
			// Force a reload of the game-data libraries against the new folder.
			self.units = None;
			self.units_loaded = false;
			self.markers = None;
			self.markers_loaded = false;
			// The armed unit is an index into the roster that is being dropped -
			// disarm it rather than let it point into whatever the new folder loads.
			self.active_unit = None;
		}
		if data_changed || (self.unit_stats.is_none() && max_changed) {
			self.reload_unit_stats();
		}
		self.save_paths();
	}

	/// Edit > Experimental > Edit Save Data: extract the embedded save's
	/// editable settings and the display context the dialog needs, refusing
	/// up front when no save is open or a settings region isn't modeled
	/// losslessly (editing it would corrupt unmodeled bytes on apply).
	fn open_edit_save_data(&mut self) -> Outcome {
		let Some(embedded) = self.project.save.as_ref() else {
			return Outcome::Failed("edit-save-data: no save open (open a `.DTA` first)".into());
		};
		if !embedded.file.settings_regions_lossless() {
			return Outcome::Failed(
				"edit-save-data: a settings region of this save did not decode losslessly - editing is disabled to \
				 protect it"
					.into(),
			);
		}
		let file = &embedded.file;
		let mut clan_names = vec!["Random".to_string()];
		match &self.unit_stats {
			Some(db) => clan_names.extend(db.clans.iter().map(|c| c.name.clone())),
			None => clan_names.extend(crate::savedata::CLAN_FALLBACK.iter().map(|s| s.to_string())),
		}
		let init = crate::savedata::SaveDataInit {
			settings: max_assets::save::SaveSettings::extract(file),
			world: file.header.world_file.map(str::to_string).unwrap_or_else(|| "custom world".into()),
			category: file.header.category.label().to_string(),
			game_state: file.game_state,
			clan_names,
			retype_supported: file.tail_follows_the_graph(),
		};
		Outcome::OpenDialog(DialogRequest::EditSaveData(Box::new(init)))
	}

	/// Apply the Edit Save Data dialog's settings block (shell-routed from
	/// [`crate::uikit_overlay::Outcome::ApplySaveData`]). One undoable step,
	/// labelled for the Undo History. Returns the console line to log.
	pub fn apply_save_data(&mut self, settings: &max_assets::save::SaveSettings) -> Result<String, String> {
		self.project.label_next_undo("Edit Save Data");
		self.project.apply_save_settings(settings).map_err(|e| format!("edit-save-data: {e}"))?;
		Ok("save data updated (undoable; File > Experimental > Export Save File writes it)".into())
	}

	/// (Re)loads the max-port unit database from `PATCHES.RES`, searching the
	/// configured folders in preference order. Logs the outcome to the console
	/// so a missing/misconfigured path is always explained, never silent.
	/// The `D_*` frame table for the fresh-body export path
	/// ([`max_assets::save::FreshBodyCtx`]) — `Some` only when both the unit
	/// database and the user's MAX.RES are at hand. Loaded per export; the
	/// table is a handful of 24-byte resources.
	fn fresh_body_frames(&self) -> Option<[Option<max_assets::attribs::FrameInfo>; max_assets::save::UNIT_END]> {
		let db = self.unit_stats.as_ref()?;
		let max_res = crate::units::find_max_res(self.max_path.as_ref()?)?;
		Some(max_assets::attribs::load_frame_infos(&max_res, &db.meta))
	}

	pub fn reload_unit_stats(&mut self) {
		let candidates: Vec<PathBuf> = [
			self.max_port_data_path.clone(),
			self.max_port_path.clone(),
			self.max_port_path.as_ref().map(|p| p.join("assets")),
			self.max_path.clone(),
		]
		.into_iter()
		.flatten()
		.collect();
		match max_assets::attribs::locate_patches_res(&candidates)
			.and_then(|path| max_assets::attribs::UnitStatsDb::load(&path))
		{
			Ok(db) => {
				self.console.push_line(format!("unit stats loaded: {}", db.source.display()));
				self.unit_stats = Some(db);
			}
			Err(e) => {
				self.unit_stats = None;
				self.console.push_line(format!("unit stats unavailable - {e}"));
				self.console
					.push_line("set the M.A.X. Port data folder (Editor Preferences) to enable stock unit stats");
			}
		}
	}

	/// Persist the `[Paths]` section (MaxPath / MaxPortPath / SkipPathPrompt) to
	/// the user settings file immediately (like [`save_preferences`](Self::save_preferences)).
	fn save_paths(&self) {
		let Some(path) = self.settings_path.as_deref() else { return };
		let disp = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
		let mut section = ini::INISection::new();
		let _ = section.set_entry("MaxPath".to_string(), disp(&self.max_path));
		let _ = section.set_entry("MaxPortPath".to_string(), disp(&self.max_port_path));
		let _ = section.set_entry("MaxPortDataPath".to_string(), disp(&self.max_port_data_path));
		let _ = section.set_entry("SkipPathPrompt".to_string(), self.skip_path_prompt);
		let _ = crate::settings_io::save_section(path, "Paths", section);
	}

	/// True when either game folder is unset — the trigger for the first-run
	/// Preferences prompt (unless the user chose "don't ask again").
	pub fn paths_incomplete(&self) -> bool {
		self.max_path.is_none() || self.max_port_path.is_none()
	}

	/// Open Editor Preferences because an action needs a folder that isn't set
	/// (`reason` explains which action). Marks the dialog "required" so a cancel
	/// leads to the Attention notice.
	pub fn prompt_paths(&mut self, reason: &str) -> Outcome {
		self.paths_prompt_reason = Some(reason.to_string());
		Outcome::OpenDialog(DialogRequest::EditorPreferences)
	}

	fn remember_recent(&mut self, path: &Path) {
		if self.is_template(path) {
			return;
		}
		self.recent.retain(|p| p != path);
		self.recent.insert(0, path.to_path_buf());
		self.recent.truncate(10);
		let entries = self.recent_map_entries();
		self.menu_tree.set_recent(&entries);
		self.rebuild_menu();
		// Persist the [QuickLoad] section right away (as soon as a map opens), so
		// the recent list survives even an unclean exit - not only `save-settings`.
		if let Some(path) = self.settings_path.as_deref() {
			let _ = crate::settings_io::save_quickload(path, &self.recent);
		}
	}

	/// Seed the recent-maps list from settings at startup, then sync the menu.
	pub fn load_recent(&mut self, paths: Vec<PathBuf>) {
		self.recent = paths;
		self.recent.truncate(10);
		let entries = self.recent_map_entries();
		self.menu_tree.set_recent(&entries);
		self.rebuild_menu();
	}

	/// Window title: `<map name>[*] - M.A.X. Map Editor`.
	/// Falls back to the project's own name (e.g. an imported WRL's stem) when
	/// there's no save path yet.
	pub fn title(&self) -> String {
		let name = self
			.path
			.as_deref()
			.and_then(|p| p.file_name())
			.map(|n| n.to_string_lossy().into_owned())
			.or_else(|| (!self.project.name.is_empty()).then(|| self.project.name.clone()))
			.unwrap_or_else(|| "untitled".into());
		let star = if self.dirty() { "*" } else { "" };
		// A save-editor session names its world so the window title is unambiguous.
		let world = Self::save_world(&self.project).map(|w| format!(" - {w}")).unwrap_or_default();
		format!("{name}{world}{star} - M.A.X. Map Editor")
	}

	pub fn uniforms(&self, tiles_per_row: u32) -> Uniforms {
		let (w, h) = self.map_size();
		Uniforms {
			screen_size: [self.screen.0 as f32, self.screen.1 as f32],
			pan: self.view.pan,
			map_size: [w as f32, h as f32],
			zoom: self.view.zoom,
			tiles_per_row,
		}
	}

	/// A native file dialog owned by the editor window. **Every `rfd` dialog
	/// must be built through here** - see `dialog_parent` for what an
	/// ownerless modal does to the app on Windows.
	fn native_dialog(&self) -> rfd::FileDialog {
		let dialog = rfd::FileDialog::new();
		match &self.dialog_parent {
			Some(window) => dialog.set_parent(window.as_ref()),
			None => dialog,
		}
	}

	/// Write the project `.json`, plus any synthetic pack (one built by
	/// `Project::from_wrl` for an imported WRL - absent from `assets_root`) to a
	/// sibling folder named after it, so the saved project reloads. Only the
	/// inferable assets are dumped (see `TilePack::dump`).
	fn write_project(&self, target: &Path) -> Result<(), String> {
		std::fs::write(target, self.project.save_string()).map_err(|e| write_error(target, &e))?;
		let dir = target.parent().unwrap_or_else(|| Path::new("."));
		for pack in &self.project.packs {
			// User packs persist to resources/user/tilepacks on edit; stock packs
			// live under assets_root. Only a synthetic (WRL-import) pack needs
			// dumping beside the project so it reloads.
			if !pack.user && !self.assets_root.join(&pack.name).is_dir() {
				pack.dump(&dir.join(&pack.name))?;
			}
		}
		Ok(())
	}

	/// The single mutator (the architectural invariant): every command - from
	/// input, `--script`, or the console - routes here. This dispatch is just
	/// the index; each grouped `exec_*` handler holds the logic. Handlers match
	/// only the variants routed to them, hence their `unreachable!` tail.
	pub fn execute(&mut self, command: Command) -> Outcome {
		use Command::*;
		// The selection mask tracks the document's dimensions; any command can
		// follow a resize/open/tab switch, so re-sync (cheap) before dispatch.
		self.sync_selection();
		// Label the undo patch this command commits (if any) for the Undo History
		// submenu; unlabelled patches derive a label from their contents.
		if let Some(label) = crate::command::undo_label(&command) {
			self.project.label_next_undo(label);
		}
		match command {
			c @ (Pan { .. } | PanTo { .. } | Zoom { .. } | ZoomAt { .. } | ZoomTo { .. } | Fit) => self.exec_nav(c),
			c @ (SetTile { .. }
			| SetPass { .. }
			| Place { .. }
			| SceneryList { .. }
			| SceneryPlace { .. }
			| SceneryPick { .. }
			| SceneryBlendMode { .. }
			| SceneryMove { .. }
			| SceneryRemove { .. }
			| SceneryClear
			| Erase { .. }
			| AssertCell { .. }
			| New { .. }
			| Tile { .. }
			| Paint { .. }
			| Fill { .. }
			| PaintMask { .. }
			| Randomize { .. }
			| BrushSize { .. }
			| BrushShape { .. }
			| AutoShore { .. }
			| ToolSelect { .. }
			| Layer { .. }
			| Mode { .. }
			| PassPick { .. }
			| PassPaint { .. }
			| TilePass { .. }
			| PassClear { .. }
			| ResetTilePass
			| TransformTile { .. }
			| Orient { .. }
			| Pick { .. }
			| Shore { .. }
			| Generate { .. }
			| Stroke { .. }) => self.exec_edit(c),
			c @ (Color { .. }
			| ColorTo { .. }
			| ColorToggle { .. }
			| SetColor { .. }
			| HslBlock { .. }
			| PaletteSave { .. }
			| PaletteLoad { .. }
			| PaletteSaveAs { .. }
			| PaletteRename { .. }
			| PaletteDelete { .. }
			| PaletteImport { .. }
			| PaletteSaveModal
			| PaletteRenameModal
			| PaletteDeleteModal
			| PaletteTab { .. }) => self.exec_palette(c),
			c @ (MinimapMode { .. }
			| PickerFilter { .. }
			| PickerSize { .. }
			| PickerScroll { .. }
			| PaletteScroll { .. }
			| MenuOpen { .. }
			| ContextMenu { .. }
			| NewMapModal
			| Window { .. }
			| DockTo { .. }
			| ResetLayout
			| UnitSelect { .. }
			| UnitTeam { .. }
			| UnitPlace { .. }
			| UnitErase { .. }
			| UnitClear
			| AutoConnect
			| ObjectSelect { .. }
			| ObjectPick { .. }
			| ObjectClone { .. }
			| ObjectEdit { .. }
			| ObjectValues { .. }
			| ResourceSet { .. }
			| ResourceBrush { .. }
			| ResourcePaint { .. }
			| ResourceAmountDialog
			| UnitsVisible { .. }
			| SaveSettings) => self.exec_panels(c),
			c @ (Undo
			| Redo
			| UndoTo { .. }
			| Open { .. }
			| OpenSave { .. }
			| OpenSaveWarn
			| OpenSaveAnyway
			| NewFromImage { .. }
			| ImportWrl { .. }
			| Convert
			| Save { .. }
			| SaveProject
			| SaveCopy { .. }
			| Tab { .. }
			| CloseProject { .. }
			| SaveAndClose
			| QuitRequest
			| SaveAndQuit
			| FileDialog { .. }
			| Resize { .. }
			| ResizeModal
			| AutoFixModal { .. }
			| GenerateModal
			| Export { .. }
			| ExportSave { .. }
			| NewSave { .. }
			| ExportSaveOnBase { .. }
			| EditSaveData
			| ConvertPalette { .. }
			| ConvertPaletteModal
			| MetadataModal
			| PreferencesModal
			| TilePaintNew
			| TilePaintClone
			| TilePaintEdit
			| TileDelete
			| TileCommit
			| TileExportPng { .. }
			| TileImportPng { .. }
			| SceneryNew
			| SceneryClone
			| SceneryEdit
			| SceneryImport { .. }
			| SceneryExport { .. }
			| SceneryHeightImport { .. }
			| SceneryHeightExport { .. }
			| SceneryCommit
			| SceneryDelete { .. }
			| SceneryRename { .. }
			| Bake
			| UpdateMap
			| MatchEditor
			| UiTests
			| MatchCombos { .. }
			| OpenUrl { .. }
			| HelpManual
			| About) => self.exec_io(c),
			c @ (Grid { .. }
			| StatusBar { .. }
			| PassOverlay { .. }
			| Resources { .. }
			| ShoreBugs { .. }
			| MatchProblems { .. }
			| ShowOnlyLayer { .. }
			| Animate { .. }
			| InGame { .. }
			| Crt { .. }
			| UiScale { .. }
			| MapPalette { .. }
			| Tick { .. }
			| Console { .. }
			| Screenshot { .. }) => self.exec_overlay(c),
			c @ (SelectOp { .. }
			| SelectCell { .. }
			| SelectRect { .. }
			| SelectMove { .. }
			| Copy
			| Cut
			| Delete
			| DeleteAll
			| Paste
			| Stamp { .. }
			| StampCancel
			| TemplateSave { .. }
			| TemplateDelete { .. }
			| TemplatePick { .. }
			| TemplateClone { .. }
			| TemplateImport { .. }
			| TemplateExport { .. }
			| TemplateExportPng { .. }
			| TemplateRename { .. }
			| TemplateDedupe
			| TemplateRenameModal
			| TemplateDeleteModal
			| TemplateDedupeModal
			| TemplateExplore) => self.exec_select(c),
			c @ (Hash | AssertTile { .. } | AssertHash { .. } | AssertDirty { .. } | Quit { .. }) => {
				self.exec_assert(c)
			}
		}
	}

	/// Recreate the selection mask when the document's dimensions changed
	/// (open / new / resize / tab switch) - a stale mask must never index
	/// out of the new map.
	fn sync_selection(&mut self) {
		if self.selection.size() != (self.project.width, self.project.height) {
			self.selection = Selection::new(self.project.width, self.project.height);
		}
	}

	/// Camera: pan / zoom / fit.
	fn exec_nav(&mut self, command: Command) -> Outcome {
		let (sw, sh) = (self.screen.0 as f32, self.screen.1 as f32);
		match command {
			Command::Pan { dx, dy } => {
				self.view.pan[0] += dx * TILE_PX as f32;
				self.view.pan[1] += dy * TILE_PX as f32;
				Outcome::Redraw
			}
			Command::PanTo { x, y } => {
				let half = TILE_PX as f32 / 2.0;
				self.view.pan = [
					x * TILE_PX as f32 + half - sw / (2.0 * self.view.zoom),
					y * TILE_PX as f32 + half - sh / (2.0 * self.view.zoom),
				];
				Outcome::Redraw
			}
			Command::Zoom { factor } => {
				self.view.zoom_at(sw / 2.0, sh / 2.0, factor);
				Outcome::Redraw
			}
			Command::ZoomAt { x, y, factor } => {
				self.view.zoom_at(x, y, factor);
				Outcome::Redraw
			}
			Command::ZoomTo { level } => {
				let factor = level / self.view.zoom;
				self.view.zoom_at(sw / 2.0, sh / 2.0, factor);
				Outcome::Redraw
			}
			Command::Fit => {
				self.view = self.fit_center(self.map_size());
				Outcome::Redraw
			}
			_ => unreachable!("non-nav command routed to exec_nav"),
		}
	}

	/// Map authoring: cell/tile edits, tool + mode selection, the eyedropper,
	/// shore passes, strokes, new-map, and the per-cell assert.
	fn exec_edit(&mut self, command: Command) -> Outcome {
		match command {
			Command::SetTile { x, y, tile } => {
				if self.project.set_base_tile(x, y, tile) {
					Outcome::Redraw
				} else {
					Outcome::Ok
				}
			}
			Command::SetPass { .. } => Outcome::Failed(
				"set-pass: per-tile pass editing is retired - edit per cell in the Pass Table Editor (pass-paint)"
					.into(),
			),
			Command::SceneryList { pack } => {
				let libs: Vec<&map_core::SceneryPack> = self
					.project
					.scenery_packs
					.iter()
					.filter(|l| pack.as_deref().is_none_or(|want| want == l.pack))
					.collect();
				if libs.is_empty() {
					return Outcome::Failed(match pack {
						Some(name) => format!("scenery-list: no scenery library for '{name}'"),
						None => "scenery-list: this project loaded no scenery libraries".into(),
					});
				}
				for lib in libs {
					self.console.push_line(format!("{} ({} objects)", lib.pack, lib.pieces.len()));
					for piece in &lib.pieces {
						self.console.push_line(format!(
							"  {:<16} {:>3}x{:<3} cells  {}",
							piece.id, piece.cells_w, piece.cells_h, piece.name
						));
					}
				}
				Outcome::Redraw
			}
			Command::SceneryPlace { pack, piece, x, y } => {
				let mut spot = map_core::ScenerySpot { pack, piece, x, y, blend: self.scenery_blend };
				let Some(piece) = self.project.scenery_piece(&spot) else {
					return Outcome::Failed(format!(
						"scenery-place: no piece '{}' in '{}' (try scenery-list)",
						spot.piece, spot.pack
					));
				};
				// The click point is the piece's centre of mass; the document
				// stores the footprint origin.
				(spot.x, spot.y) = piece.centered_at(x, y);
				let index = self.project.place_scenery(spot);
				self.console.push_line(format!("placed scenery {index} at ({x},{y})"));
				Outcome::Redraw
			}
			Command::SceneryPick { index } => {
				match index {
					Some(i) if i >= crate::scenery::piece_count(&self.project) => {
						return Outcome::Failed(format!("scenery-pick: no piece {i}"));
					}
					_ => {}
				}
				self.active_scenery = index;
				// Arming a piece arms the layer and the tool, the way picking a tile
				// arms the pencil - one click in the panel should be enough to place.
				// A template ghost armed before it would eat those clicks (it takes
				// the map click ahead of any tool), so it goes away too.
				if index.is_some() && self.mode == EditorMode::Map {
					self.active_layer = LAYER_SCENERY;
					self.tool = Tool::Scenery;
					self.disarm_stamp();
				}
				Outcome::Redraw
			}
			Command::SceneryBlendMode { index, mode } => {
				let Some(index) = index else {
					self.scenery_blend = mode;
					self.console.push_line(format!("new scenery blends {}", mode.name()));
					return Outcome::Redraw;
				};
				if index >= self.project.scenery.len() {
					return Outcome::Failed(format!("scenery-blend: no placement {index}"));
				}
				self.project.set_scenery_blend(index, mode);
				Outcome::Redraw
			}
			Command::SceneryMove { index, x, y } => {
				if self.project.move_scenery_to(index, x, y) {
					Outcome::Redraw
				} else {
					Outcome::Ok
				}
			}
			Command::SceneryRemove { index } => {
				if self.project.remove_scenery(index) {
					Outcome::Redraw
				} else {
					Outcome::Failed(format!("scenery-remove: no placement {index}"))
				}
			}
			Command::SceneryClear => {
				let mut removed = 0;
				while self.project.remove_scenery(self.project.scenery.len().saturating_sub(1)) {
					removed += 1;
				}
				self.console.push_line(format!("removed {removed} scenery placement(s)"));
				Outcome::Redraw
			}
			Command::Place { x, y, spec } => {
				let project = &mut self.project;
				match project.resolve_ref(&spec) {
					Ok((tile, layer)) => {
						if project.place(x, y, layer, Some(tile)) {
							Outcome::Redraw
						} else {
							Outcome::Ok
						}
					}
					Err(e) => Outcome::Failed(format!("place: {e}")),
				}
			}
			Command::Erase { x, y, layer } => {
				let explicit = match layer.as_deref() {
					Some("water") => Some(LAYER_WATER),
					Some("ground") => Some(LAYER_GROUND),
					Some(other) => {
						return Outcome::Failed(format!("erase: bad layer '{other}'"));
					}
					None => None,
				};
				// The eraser covers the brush footprint; each cell erases its
				// chosen layer (or its topmost present one when unspecified).
				let cells = self.brush_cells(x, y);
				let mut edits = Vec::with_capacity(cells.len());
				for (cx, cy) in cells {
					let layer = explicit.unwrap_or_else(|| match self.project.cell(cx, cy) {
						Some(stack) if stack[LAYER_GROUND].is_some() => LAYER_GROUND,
						_ => LAYER_WATER,
					});
					edits.push((cx, cy, layer, None));
				}
				if self.project.place_many(&edits) { Outcome::Redraw } else { Outcome::Ok }
			}
			Command::AssertCell { x, y, spec } => {
				let project = &self.project;
				let expected = if spec == "-" { String::new() } else { spec };
				match project.cell_spec(x, y) {
					Some(actual) if actual == expected => Outcome::Ok,
					Some(actual) => {
						Outcome::Failed(format!("assert-cell {x} {y}: expected '{expected}', got '{actual}'",))
					}
					None => Outcome::Failed(format!("assert-cell {x} {y}: out of range")),
				}
			}
			// New opens in a fresh tab - nothing is lost, so no dirty
			// guard; `force` is vestigial. Interactive default: a fresh seed;
			// scripts pass one for determinism. The seed is reported so any map
			// can be re-made.
			Command::New { width, height, packs, seed } => {
				let seed = seed.unwrap_or_else(roll_seed);
				match Project::new(width, height, &packs, &self.assets_root, seed) {
					Ok(project) => {
						let line = format!(
							"new map {width}x{height}, packs: {}, seed {seed}",
							project.uses.iter().map(|u| u.name.as_str()).collect::<Vec<_>>().join("+"),
						);
						eprintln!("{line}");
						self.console.push_line(line);
						self.add_doc(project, None, None)
					}
					Err(e) => Outcome::Failed(format!("new: {e}")),
				}
			}
			Command::Tile { spec } => {
				let project = &self.project;
				match spec.as_deref() {
					None => {
						let line = format!("active tile: {}", self.active_tile.as_deref().unwrap_or("none"),);
						println!("{line}");
						self.console.push_line(line);
						Outcome::Redraw
					}
					Some("-") => {
						self.active_tile = None;
						self.console.push_line("active tile: none");
						Outcome::Redraw
					}
					Some(s) => match project.resolve_ref(s) {
						Ok((_, layer)) => {
							let line = format!("active tile: {s} ({})", ["water", "ground"][layer]);
							self.console.push_line(line);
							self.active_tile = Some(s.to_string());
							// Picking a tile is a terrain edit: off the Scenery layer, onto
							// the one the tile belongs to (the layer choice the user made
							// there says nothing about tiles).
							self.leave_scenery_layer(layer);
							Outcome::Redraw
						}
						Err(e) => Outcome::Failed(format!("tile: {e}")),
					},
				}
			}
			Command::Paint { x, y } => {
				// The unit tool stamps a preview instead of painting tiles.
				if self.tool == Tool::Unit {
					let Some(unit) = self.active_unit else {
						return Outcome::Failed("unit: none selected (use the Units panel or `unit TAG`)".into());
					};
					return self.place_unit_preview(unit, x, y);
				}
				let Some(spec) = self.active_tile.clone() else {
					return Outcome::Failed("paint: no active tile (use `tile SPEC`)".into());
				};
				match self.project.resolve_ref(&spec) {
					// Paint onto the selected layer, not the tile's natural one;
					// the brush footprint covers a square of cells.
					Ok((tile, _)) => {
						let cells = self.brush_cells(x, y);
						let mut edits = Vec::with_capacity(cells.len());
						for (cx, cy) in cells {
							let t = if self.randomize {
								self.project.random_variant(tile, &mut self.paint_rng)
							} else {
								tile
							};
							edits.push((cx, cy, self.tile_layer(), Some(t)));
						}
						if self.project.place_many(&edits) { Outcome::Redraw } else { Outcome::Ok }
					}
					Err(e) => Outcome::Failed(format!("paint: {e}")),
				}
			}
			Command::Fill { x, y } => {
				let Some(spec) = self.active_tile.clone() else {
					return Outcome::Failed("fill: no active tile (use `tile SPEC`)".into());
				};
				match self.project.resolve_ref(&spec) {
					Ok((tile, _)) => {
						let (layer, randomize) = (self.tile_layer(), self.randomize);
						// An active selection confines the fill: every selected cell
						// gets the active tile as one undo unit (connectivity is
						// ignored). With no selection, it's the usual flood fill.
						let changed = if let Some((x0, y0, x1, y1)) = self.selection.bounds() {
							let mut edits = Vec::new();
							for cy in y0..=y1 {
								for cx in x0..=x1 {
									if self.selection.contains(cx, cy) {
										let t = if randomize {
											self.project.random_variant(tile, &mut self.paint_rng)
										} else {
											tile
										};
										edits.push((cx, cy, layer, Some(t)));
									}
								}
							}
							self.project.place_many(&edits)
						} else {
							self.project.fill(x, y, tile, layer, randomize, &mut self.paint_rng)
						};
						if changed { Outcome::Redraw } else { Outcome::Ok }
					}
					Err(e) => Outcome::Failed(format!("fill: {e}")),
				}
			}
			Command::PaintMask { x, y } => {
				// Free-hand terrain brush: lay flat land or water (the active
				// material) across the footprint, exactly the way the generator's
				// Apply phase fills its mask. No shoring here - the coast (beach +
				// animated coastal waves) grows over the whole stroke on release
				// (`take_mask_region` -> `Command::Shore`), so the drag stays cheap
				// and the whole stroke is one undo unit.
				let water = self.mask_water;
				let kind = if water { TileKind::Water } else { TileKind::Land };
				// Land follows the active tile's tileset; water stays universal.
				let preferred = if water { None } else { self.active_tile_ref().ok().map(|(pk, _)| pk) };
				let Some((pack_idx, family)) = self.project.variant_family_in(kind, preferred) else {
					let g = if water { "WATER" } else { "LAND" };
					return Outcome::Failed(format!(
						"terrain brush: no pack has a {g} variant group (tiles.props.json)"
					));
				};
				let tiles = self.project.packs[pack_idx].group_tiles(&family);
				if tiles.is_empty() {
					return Outcome::Failed(format!("terrain brush: '{family}' has no tiles"));
				}
				let cells = self.brush_cells(x, y);
				let mut edits = Vec::with_capacity(cells.len() * 2);
				for &(cx, cy) in &cells {
					let tile = tiles[self.paint_rng.below(tiles.len() as u32) as usize];
					let tref = TileRef { pack: pack_idx as u8, tile, transform: Transform::default() };
					if water {
						// Water shows through a cleared ground layer; the water-variant
						// tile (the animated coastal band) sits on the bottom layer
						// beneath, so erasing land later reveals waves.
						edits.push((cx, cy, LAYER_WATER, Some(tref)));
						edits.push((cx, cy, LAYER_GROUND, None));
					} else {
						edits.push((cx, cy, LAYER_GROUND, Some(tref)));
					}
					// Grow the stroke's painted bounds (consumed on release).
					self.mask_dirty = Some(match self.mask_dirty {
						Some((x0, y0, x1, y1)) => (x0.min(cx), y0.min(cy), x1.max(cx), y1.max(cy)),
						None => (cx, cy, cx, cy),
					});
				}
				if self.project.place_many(&edits) { Outcome::Redraw } else { Outcome::Ok }
			}
			Command::Randomize { on } => {
				self.randomize = on.unwrap_or(!self.randomize);
				self.console.push_line(format!("randomize variants: {}", if self.randomize { "on" } else { "off" }));
				Outcome::Redraw
			}
			Command::BrushSize { size } => {
				// Keep it odd so the square stays centred on the cursor cell.
				self.brush_size = size.clamp(1, 99) | 1;
				self.console.push_line(format!("brush size: {}", self.brush_size));
				Outcome::Redraw
			}
			Command::BrushShape { shape } => {
				self.brush_shape = match shape.as_str() {
					"square" => BrushShape::Square,
					"circle" => BrushShape::Circle,
					other => return Outcome::Failed(format!("brush-shape: unknown '{other}' (square|circle)")),
				};
				Outcome::Redraw
			}
			Command::AutoShore { mode } => {
				self.brush_shore = match mode.as_str() {
					"off" | "disabled" | "none" => BrushShore::Off,
					"sweep" => BrushShore::Sweep,
					"loop-walk" | "loop" | "alt" => BrushShore::LoopWalk,
					other => return Outcome::Failed(format!("auto-shore: unknown '{other}' (off|sweep|loop-walk)")),
				};
				self.console.push_line(format!("terrain brush auto-shore: {mode}"));
				Outcome::Redraw
			}
			Command::Layer { name } => {
				self.active_layer = match name.as_str() {
					"water" => LAYER_WATER,
					"ground" => LAYER_GROUND,
					"scenery" => LAYER_SCENERY,
					other => return Outcome::Failed(format!("layer: unknown '{other}' (water|ground|scenery)")),
				};
				// Switching layers re-points the tool rather than dropping it: the
				// pencil is still the pencil, it just draws on what is selected. Only
				// in Map mode - the pass editors own their own tool set.
				if self.mode == EditorMode::Map {
					self.tool = if self.on_scenery_layer() { scenery_twin(self.tool) } else { terrain_twin(self.tool) };
				}
				self.console.push_line(format!("layer: {name}"));
				Outcome::Redraw
			}
			Command::ToolSelect { name } => {
				// Set by the arms that need the resource overlay up; applied after
				// the tool lands, so the console reads "tool: ..." then the reveal.
				let mut reveal = false;
				self.tool = match name.as_str() {
					// The mode's own select tool - what a cancelled gesture reverts to,
					// so no call site has to know whether that is cells or objects.
					"default" => self.mode.default_tool(),
					"pencil" => Tool::Pencil,
					"picker" | "pick" => Tool::Picker,
					"eraser" | "erase" => Tool::Eraser,
					"fill" | "flood" => Tool::Fill,
					// The terrain brush, pre-set to a material: "land" / "water"
					// both select it (one click arms the tool and picks the colour).
					"paint-land" | "paint-mask" => {
						self.mask_water = false;
						Tool::PaintMask
					}
					"paint-water" => {
						self.mask_water = true;
						Tool::PaintMask
					}
					"unit" | "obj-place" => Tool::Unit,
					"unit-eraser" | "unit-erase" | "obj-delete" => Tool::UnitEraser,
					"obj-select" | "object-select" => Tool::ObjSelect,
					"obj-pick" | "object-pick" => Tool::ObjPick,
					"obj-move" | "object-move" => Tool::ObjMove,
					"obj-clone" | "object-clone" => Tool::ObjClone,
					"scenery" | "scenery-place" => Tool::Scenery,
					"scenery-move" => Tool::SceneryMove,
					"scenery-eraser" | "scenery-erase" | "scenery-delete" => Tool::SceneryEraser,
					"select" => Tool::Select,
					"select-rect" | "rect" => Tool::SelectRect,
					"resource-brush" | "resource" => {
						reveal = true;
						Tool::ResourceBrush
					}
					other => {
						return Outcome::Failed(format!(
							"tool: unknown '{other}' (default|pencil|picker|eraser|fill|paint-land|paint-water|unit|unit-eraser|obj-select|obj-pick|obj-move|scenery|scenery-move|scenery-eraser|select|select-rect|resource-brush)"
						));
					}
				};
				// The layer says what the three shared keys draw on: on the Scenery
				// layer the pencil drops a cut-out, the eraser removes one and the
				// arrow drags one (`scenery_twin`). Naming a scenery tool outright
				// implies the layer, so the menu tick, the toolbox key and the armed
				// tool can never disagree about which one is live.
				if self.mode == EditorMode::Map && self.on_scenery_layer() {
					self.tool = scenery_twin(self.tool);
				} else if matches!(self.tool, Tool::Scenery | Tool::SceneryMove | Tool::SceneryEraser) {
					self.active_layer = LAYER_SCENERY;
				}
				// Echo the tool, not the word: `tool default` resolves per mode, and the
				// console line has to say which tool that turned out to be.
				self.console.push_line(format!("tool: {}", self.tool.slug()));
				if reveal {
					self.reveal_resources();
				}
				Outcome::Redraw
			}
			Command::Mode { name } => {
				let new_mode = match name.as_str() {
					"map" => EditorMode::Map,
					"pass" => EditorMode::Pass,
					"localpass" => EditorMode::LocalPass,
					"save" => EditorMode::SaveEditor,
					other => {
						return Outcome::Failed(format!("mode: unknown '{other}' (map|pass|localpass|save)"));
					}
				};
				// Each mode carries its own dock layout (the two pass editors share
				// one): stash the outgoing layout and restore the incoming one.
				let (from, to) = (self.mode.layout_group(), new_mode.layout_group());
				self.switch_layout_group(from, to);
				self.mode = new_mode;
				// The pass overlay rides with either pass editor: it turns on so
				// painting is visible, and off again on returning to Map.
				self.show_pass_overlay = matches!(self.mode, EditorMode::Pass | EditorMode::LocalPass);
				// A tool the incoming mode does not offer would leave *no* tool
				// selected - nothing lit in any toolbox, and a map click doing
				// something this mode's UI never put on screen. Fall back to the
				// mode's own select tool.
				if !self.mode.owns_tool(self.tool) {
					self.tool = self.mode.default_tool();
				}
				// The pass swatches and the cell tally are the Pass Types Palette's, so
				// *arriving* in the pass group brings it up. Only on arrival: flipping
				// between the two pass editors is one layout group, and re-showing a
				// panel the user just closed would be the shell arguing with them.
				if to == LayoutGroup::Pass && from != to {
					let _ = self.workspace.show("passtools", Some(true));
				}
				self.console.push_line(format!("mode: {name}"));
				Outcome::Redraw
			}
			Command::PassPick { value } => {
				if let Some(o) = check_pass(value, "pass-pick") {
					return o;
				}
				self.active_pass = value;
				self.console.push_line(format!("pass: {}", PASS_LABELS[value as usize]));
				Outcome::Redraw
			}
			Command::PassPaint { x, y, value } => {
				if let Some(o) = check_pass(value, "pass-paint") {
					return o;
				}
				if self.project.set_pass(x, y, value) { Outcome::Redraw } else { Outcome::Ok }
			}
			Command::TilePass { x, y, value } => {
				if let Some(o) = check_pass(value, "tile-pass") {
					return o;
				}
				// The Pass Table Editor rewrites the cell's top tile's pack pass
				// table. Note which pack that is *before* the edit, so a `--dev` edit
				// of a stock pack is queued for Bake - only then does the new pass
				// reach the shipped `tiles.pass.json` (it was being lost otherwise).
				let pack =
					self.project.cell(x, y).and_then(|s| s[LAYER_GROUND].or(s[LAYER_WATER])).map(|t| t.pack as usize);
				if !self.project.set_tile_pass_at(x, y, value) {
					return Outcome::Ok;
				}
				if let Some(idx) = pack {
					if self.dev_mode && self.is_stock_pack(idx) {
						let name = self.project.packs[idx].name.clone();
						self.tile_ops.dirty_packs.insert(name);
					}
				}
				Outcome::Redraw
			}
			Command::PassClear { x, y } => {
				if self.project.set_pass_override(x, y, None) {
					Outcome::Redraw
				} else {
					Outcome::Ok
				}
			}
			Command::ResetTilePass => self.reset_tile_pass(),
			Command::Orient { rot, mirror } => self.orient_armed(map_core::Transform { rot, mirror }),
			Command::TransformTile { op } => {
				// An armed template stamp takes the transform tool: compose the op
				// onto its current orientation and re-derive from the base (the
				// single source of truth the 8-orientation grid shares).
				if self.stamp_base.is_some() {
					let Some(stamp_op) = map_core::StampOp::parse(&op) else {
						return Outcome::Failed(format!("transform: unknown '{op}' (flip-h|flip-v|cw|ccw)"));
					};
					let next = match stamp_op {
						map_core::StampOp::Cw => self.stamp_xform.rotated_cw(),
						map_core::StampOp::Ccw => self.stamp_xform.rotated_ccw(),
						map_core::StampOp::FlipH => self.stamp_xform.flipped_h(),
						map_core::StampOp::FlipV => self.stamp_xform.flipped_v(),
					};
					return self.orient_armed(next);
				}
				let Some(spec) = self.active_tile.clone() else {
					return Outcome::Failed("transform: no active tile (use `tile SPEC`)".into());
				};
				let (id, transform) = match spec.split_once(':') {
					Some((id, t)) => match map_core::Transform::parse(t) {
						Ok(tr) => (id, tr),
						Err(e) => return Outcome::Failed(format!("transform: {e}")),
					},
					None => (spec.as_str(), map_core::Transform::default()),
				};
				let transform = match op.as_str() {
					"cw" => transform.rotated_cw(),
					"ccw" => transform.rotated_ccw(),
					"flip-h" => transform.flipped_h(),
					"flip-v" => transform.flipped_v(),
					other => {
						return Outcome::Failed(format!("transform: unknown '{other}' (flip-h|flip-v|cw|ccw)",));
					}
				};
				let spec = format!("{id}{}", transform.suffix());
				let line = format!("active tile: {spec}");
				self.console.push_line(line);
				self.active_tile = Some(spec);
				Outcome::Redraw
			}
			Command::Pick { x, y } => {
				let project = &self.project;
				let Some(stack) = project.cell_spec(x, y) else {
					return Outcome::Failed(format!("pick: cell {x} {y} out of range"));
				};
				// The stack's top entry (transform included) becomes the brush.
				let Some(top) = stack.rsplit(',').next().filter(|s| !s.is_empty()) else {
					return Outcome::Failed(format!("pick: cell {x} {y} is empty"));
				};
				let line = format!("active tile: {top} (picked {x} {y})");
				self.console.push_line(line);
				self.active_tile = Some(top.to_string());
				// The eyedropper hands back to the pencil - pick, then paint - and
				// off the Scenery layer, onto the picked tile's own.
				let layer = self.project.resolve_ref(top).map_or(LAYER_GROUND, |(_, l)| l);
				self.leave_scenery_layer(layer);
				self.tool = Tool::Pencil;
				// Reveal the picked tile in the Tile Explorer (if it's open).
				self.reveal_active_tile_in_explorer();
				Outcome::Redraw
			}
			Command::Shore { region, mode } => {
				// With no explicit region, confine to the active selection (its
				// bounding rect); still whole-map when nothing is selected.
				let region = region.or_else(|| self.selection.bounds());
				let project = &mut self.project;
				if let Some((x0, y0, x1, y1)) = region {
					if x0.max(x1) >= project.width || y0.max(y1) >= project.height {
						return Outcome::Failed(format!(
							"shore: region exceeds the {}x{} map",
							project.width, project.height,
						));
					}
				}
				use map_core::FixStrength;
				// The `*Fix` / `Full` modes loop [place -> escalating fix -> re-check]
				// until the coast is clean (`shore_repair`, one undo unit) - Mangle for
				// the menu's "+ Fix", Destructive for Full. The console keeps these
				// synchronous so scripts complete deterministically; the menu routes
				// through the Fix Shore dialog instead so the UI never freezes.
				let (changed, unresolved, how) = match mode {
					ShoreMode::Sweep => {
						let (c, u) = project.auto_shore(region);
						(c, u, "auto-shore")
					}
					ShoreMode::LoopWalk => {
						let (c, u) = project.auto_shore_alt(region);
						(c, u, "auto-shore loop-walk")
					}
					ShoreMode::Fix => {
						let (c, u) = project.fix_shore(region);
						(c, u, "fix-shore")
					}
					ShoreMode::SweepFix => {
						let (c, u) = project.shore_repair(region, false, FixStrength::Mangle);
						(c, u, "shore sweep + fix")
					}
					ShoreMode::LoopFix => {
						let (c, u) = project.shore_repair(region, true, FixStrength::Mangle);
						(c, u, "shore loop-walk + fix")
					}
					ShoreMode::Full => {
						let (c, u) = project.shore_repair(region, false, FixStrength::Destructive);
						(c, u, "shore full")
					}
				};
				let line = match unresolved {
					0 => format!("{how}: {changed} cells"),
					n => format!(
						"{how}: {changed} cells ({n} seam{} the tileset cannot close)",
						if n == 1 { "" } else { "s" },
					),
				};
				self.console.push_line(line);
				Outcome::Redraw
			}
			Command::Stroke { begin } => {
				let project = &mut self.project;
				if begin {
					project.begin_stroke();
				} else {
					project.end_stroke();
				}
				Outcome::Ok
			}
			Command::Generate { mut params, explicit_seed } => {
				// No seed given: fresh randomness, reported below so the map
				// can be re-made (same convention as `new`).
				params.seed = explicit_seed.unwrap_or_else(roll_seed);
				let feats = self.feature_templates();
				match self.project.generate_terrain(&params, &feats) {
					Ok(s) => {
						self.console.push_line(generate_report(&params, &s));
						Outcome::Redraw
					}
					Err(e) => Outcome::Failed(format!("generate: {e}")),
				}
			}
			_ => unreachable!("non-edit command routed to exec_edit"),
		}
	}

	/// Palette: select a slot, set a dynamic color, re-tint a water block.
	fn exec_palette(&mut self, command: Command) -> Outcome {
		match command {
			Command::ColorTo { index } => {
				// Shift-click: extend the selection from the anchor to `index`
				// (or start a fresh single selection when there's no anchor yet).
				self.palettes.multi.clear();
				if self.active_color.is_none() {
					self.active_color = Some(index);
				}
				self.palettes.sel_end = Some(index);
				Outcome::Redraw
			}
			Command::ColorToggle { index } => {
				// Ctrl-click: toggle the slot in the multi-selection set; the
				// last-touched slot stays the active focus.
				if let Some(pos) = self.palettes.multi.iter().position(|&s| s == index) {
					self.palettes.multi.remove(pos);
				} else {
					self.palettes.multi.push(index);
				}
				self.active_color = Some(index);
				self.palettes.sel_end = None;
				Outcome::Redraw
			}
			Command::Color { index } => {
				self.active_color = Some(index);
				self.palettes.sel_end = None;
				self.palettes.multi.clear();
				let palette: Vec<u8> = self.project.palette.clone();
				let s = crate::palette_panel::section_of(index as u16);
				let at = index as usize * 3;
				let line = format!(
					"color {index}: #{:02x}{:02x}{:02x} - {}, {}{}",
					palette[at],
					palette[at + 1],
					palette[at + 2],
					s.label,
					if s.editable { "editable" } else { "fixed" },
					if crate::palette_panel::animated(index as u16) { ", cycled" } else { "" },
				);
				println!("{line}");
				self.console.push_line(line);
				Outcome::Redraw
			}
			Command::SetColor { slot, rgb } => {
				let project = &mut self.project;
				match project.set_color(slot, rgb) {
					Ok(changed) => {
						if changed {
							self.refresh_palette();
						}
						Outcome::Redraw
					}
					Err(e) => Outcome::Failed(format!("set-color: {e}")),
				}
			}
			Command::PaletteTab { saved } => {
				self.palettes.show_saved = saved;
				if saved {
					self.scan_palette_files();
				}
				Outcome::Redraw
			}
			Command::PaletteSave { path } => {
				let name = path.file_stem().map_or_else(|| "palette".into(), |s| s.to_string_lossy().into_owned());
				let json = map_core::write_palette(&self.project.palette, &name);
				match std::fs::write(&path, json) {
					Ok(()) => {
						self.console.push_line(format!("palette saved -> {}", path.display()));
						Outcome::Redraw
					}
					Err(e) => Outcome::Failed(format!("palette-save: {e}")),
				}
			}
			Command::PaletteLoad { path } => {
				let colors = match crate::palette_io::load(&path) {
					Ok(c) => c,
					Err(e) => return Outcome::Failed(format!("palette-load: {e}")),
				};
				match self.project.load_palette(&colors) {
					Ok(n) => {
						if n > 0 {
							self.refresh_palette();
						}
						self.console.push_line(format!("palette loaded ({n} slots) <- {}", path.display()));
						Outcome::Redraw
					}
					Err(e) => Outcome::Failed(format!("palette-load: {e}")),
				}
			}
			// The name becomes a file in `user/palettes`, so it has to be one
			// path component. The Save/Rename dialog checks this too, for the
			// nicer inline message - but the dialog is not the only caller: a
			// `--script` line or a console line reaches the command directly,
			// and `palette_io::save` runs `create_dir_all` on the parent. The
			// check belongs here, at the single mutator, not only at one door.
			Command::PaletteSaveAs { name } => {
				let name = name.trim();
				if let Err(e) = map_core::check_name_component("palette-save-as", name) {
					return Outcome::Failed(e);
				}
				let path = self.user_palettes_dir().join(format!("{name}.json"));
				match crate::palette_io::save(&path, &self.project.palette, name) {
					Ok(()) => self.palette_saved(format!("palette saved -> {}", path.display()), Some(path)),
					Err(e) => Outcome::Failed(format!("palette-save-as: {e}")),
				}
			}
			Command::PaletteRename { from, to } => {
				let to = to.trim();
				if let Err(e) = map_core::check_name_component("palette-rename", to) {
					return Outcome::Failed(e);
				}
				let target = self.user_palettes_dir().join(format!("{to}.json"));
				match crate::palette_io::rename(&from, &target) {
					Ok(()) => self.palette_saved(format!("palette renamed -> {}", target.display()), Some(target)),
					Err(e) => Outcome::Failed(format!("palette-rename: {e}")),
				}
			}
			Command::PaletteDelete { path } => match crate::palette_io::delete(&path) {
				Ok(()) => self.palette_saved(format!("palette deleted: {}", path.display()), None),
				Err(e) => Outcome::Failed(format!("palette-delete: {e}")),
			},
			Command::PaletteImport { path } => {
				let dir = self.user_palettes_dir();
				match crate::palette_io::import(&path, &dir) {
					Ok(dest) => self.palette_saved(format!("palette imported -> {}", dest.display()), Some(dest)),
					Err(e) => Outcome::Failed(format!("palette-import: {e}")),
				}
			}
			// These open wgpu-ui overlay dialogs - the shell routes the request
			// (`App::act_on`); headless/script runs have no overlay and drop it.
			Command::PaletteSaveModal => Outcome::OpenDialog(DialogRequest::PaletteSave),
			Command::PaletteRenameModal => Outcome::OpenDialog(DialogRequest::PaletteRename),
			Command::PaletteDeleteModal => Outcome::OpenDialog(DialogRequest::PaletteDelete),
			Command::HslBlock { slot, dh, ds, dl } => {
				let project = &mut self.project;
				// Percent points in the command, fractions in the core.
				match project.hsl_shift_block(slot, dh, ds / 100.0, dl / 100.0) {
					Ok(changed) => {
						if changed {
							self.refresh_palette();
						}
						Outcome::Redraw
					}
					Err(e) => Outcome::Failed(format!("hsl-block: {e}")),
				}
			}
			_ => unreachable!("non-palette command routed to exec_palette"),
		}
	}

	/// Panel + chrome state: minimap/picker/palette view options, the menu,
	/// the New Map modal opener, window show + dock.
	fn exec_panels(&mut self, command: Command) -> Outcome {
		match command {
			Command::MinimapMode { mode } => match minimap::Mode::parse(&mode) {
				Some(m) => {
					self.minimap_mode = m;
					self.console.push_line(format!("minimap: {}", m.name()));
					Outcome::Redraw
				}
				None => Outcome::Failed(format!("minimap: unknown '{mode}' (overworld|pass|minimap)",)),
			},
			Command::PickerFilter { name } => {
				let filter = if name == "next" {
					self.picker.filter.next()
				} else {
					match picker::Filter::parse(&name) {
						Some(f) => f,
						None => {
							return Outcome::Failed(format!(
								"picker filter: unknown '{name}' (all|used|unused|water|shore|land|blocked|next)",
							));
						}
					}
				};
				self.picker.filter = filter;
				self.picker.scroll_request = Some(picker::ScrollRequest::To(0.0));
				self.console.push_line(format!("picker filter: {}", filter.name()));
				Outcome::Redraw
			}
			Command::PickerSize { size } => {
				if size == "next" {
					self.picker.cycle_size();
				} else {
					match size.parse::<f32>() {
						Ok(px) if (8.0..=256.0).contains(&px) => self.picker.tile_px = px,
						_ => {
							return Outcome::Failed(format!("picker size: bad '{size}' (8..=256 px, or `next`)",));
						}
					}
				}
				self.console.push_line(format!("picker size: {} px", self.picker.tile_px as u32));
				Outcome::Redraw
			}
			Command::PickerScroll { to } => {
				self.picker.scroll_request = Some(picker::ScrollRequest::To(to.max(0.0)));
				Outcome::Redraw
			}
			Command::PaletteScroll { to } => {
				self.palettes.scroll_request = Some(to.max(0.0));
				Outcome::Redraw
			}
			Command::MenuOpen { name } => {
				if name == "off" {
					self.menu().close();
					return Outcome::Redraw;
				}
				// Case-insensitive against the model's titles, exact on the widget.
				match self.menu_tree.menus.iter().find(|m| m.title.eq_ignore_ascii_case(&name)) {
					Some(m) => {
						let title = m.title;
						self.menu().open_by_title(title);
						Outcome::Redraw
					}
					None => Outcome::Failed(format!(
						"menu: unknown menu '{name}' (have: {})",
						self.menu_tree.menus.iter().map(|m| m.title).collect::<Vec<_>>().join(" ").to_lowercase(),
					)),
				}
			}
			Command::ContextMenu { at } => {
				// `at` is a **physical** cursor point: the cell under it is read in
				// physical screen space (the map renders native), but the menu itself
				// is chrome - position it in logical space so it lays out + hit-tests
				// with the rest of the UI under the current scale.
				self.context_menu = at.map(|(x, y)| {
					let cell = self.cell_at(x, y);
					let pos = (x / self.ui_scale, y / self.ui_scale);
					menu::ContextMenu::new(self.context_menu_items(cell), pos)
				});
				self.menu().close();
				Outcome::Redraw
			}
			// Opens the wgpu-ui New Map overlay - the shell routes the request
			// (`App::act_on`); headless/script runs have no overlay and drop it.
			Command::NewMapModal => Outcome::OpenDialog(DialogRequest::NewMap { shape: None }),
			Command::Window { id, on } => match self.workspace.show_cmd(&id, on) {
				Ok(line) => {
					// A panel restores the place it was hidden from, which may predate
					// the current window size or UI scale - correct it before it draws.
					self.reclamp_workspace();
					// Opening the Units dock needs the M.A.X. folder (unit sprites live
					// in MAX.RES) — prompt for it if it isn't set yet.
					let prompt = id == "units" && line.ends_with("shown") && self.max_path.is_none();
					self.console.push_line(line);
					if prompt {
						return self.prompt_paths("The Units panel loads unit sprites from your M.A.X. folder.");
					}
					Outcome::Redraw
				}
				Err(e) => Outcome::Failed(format!("window: {e}")),
			},
			Command::DockTo { id, place, at } => match self.workspace.dock_cmd(&id, &place, at) {
				Ok(line) => {
					// `dock <id> float <x> <y>` names an arbitrary position - clamp it
					// like any other, so a typed y cannot hide the titlebar either.
					self.reclamp_workspace();
					self.console.push_line(line);
					Outcome::Redraw
				}
				Err(e) => Outcome::Failed(format!("dock: {e}")),
			},
			Command::ResetLayout => {
				self.workspace.reset();
				self.reclamp_workspace();
				self.console.push_line("layout reset to defaults");
				Outcome::Redraw
			}
			Command::UnitSelect { tag } => match tag {
				None => {
					self.active_unit = None;
					if self.tool == Tool::Unit {
						self.tool = Tool::Pencil;
					}
					self.console.push_line("unit: off");
					Outcome::Redraw
				}
				Some(tag) => {
					if let Err(e) = self.ensure_units() {
						return Outcome::Failed(e);
					}
					let lib = self.units.as_ref().expect("ensure_units");
					match lib.find(&tag) {
						Some(i) => {
							let tag = lib.units[i].tag.clone();
							self.active_unit = Some(i);
							self.tool = Tool::Unit;
							self.show_units = true;
							self.console.push_line(format!("unit: {tag} (click the map to place)"));
							Outcome::Redraw
						}
						None => Outcome::Failed(format!("unit: unknown tag '{tag}'")),
					}
				}
			},
			Command::UnitTeam { team } => match crate::units::parse_team(&team) {
				Some(t) => {
					self.unit_team = t;
					self.console.push_line(format!("unit team: {}", crate::units::TEAM_NAMES[t as usize]));
					Outcome::Redraw
				}
				None => Outcome::Failed(format!("unit-team: unknown '{team}' (red|green|blue|gray|yellow|0-4)")),
			},
			Command::UnitPlace { tag, x, y } => {
				if let Err(e) = self.ensure_units() {
					return Outcome::Failed(e);
				}
				let lib = self.units.as_ref().expect("ensure_units");
				let Some(unit) = lib.find(&tag) else {
					return Outcome::Failed(format!("unit-place: unknown tag '{tag}'"));
				};
				self.place_unit_preview(unit, x, y)
			}
			Command::UnitErase { x, y } => {
				if self.project.remove_object_at(x, y) {
					// Removal shifts the list, so a stored index would go stale.
					self.selected_object = None;
					Outcome::Redraw
				} else {
					Outcome::Ok
				}
			}
			Command::UnitClear => {
				let n = self.project.clear_objects();
				self.selected_object = None;
				self.console.push_line(format!("unit previews cleared ({n})"));
				Outcome::Redraw
			}
			Command::AutoConnect => {
				if self.project.auto_connect_buildings() {
					self.console.push_line("auto-connected adjacent same-team buildings");
					Outcome::Redraw
				} else {
					self.console.push_line("auto-connect: nothing to connect");
					Outcome::Ok
				}
			}
			Command::ObjectSelect { x, y } => {
				// Cycle through stacked objects when the same overlapped cell is
				// clicked again (item 7); a fresh cell picks its top-most.
				self.selected_object = self.object_at_cycling(x, y, self.selected_object);
				match self.selected_object {
					Some(i) => {
						let o = &self.project.objects[i];
						let name = max_assets::save::unit_type_name(o.unit_type).unwrap_or("object");
						self.console.push_line(format!("selected {name} at ({}, {})", o.x, o.y));
						// A Select-tool pick routes straight to editing: reveal the Unit
						// Properties panel (S4.3). A bare scripted `object-select` (the
						// tool not armed) leaves the layout alone.
						if self.tool == Tool::ObjSelect {
							let _ = self.workspace.show("unitprops", Some(true));
						}
					}
					None => self.console.push_line(format!("no object at ({x}, {y})")),
				}
				Outcome::Redraw
			}
			Command::ObjectPick { x, y } => {
				// Eyedropper: arm the object's type + team, then switch to Place.
				let Some(i) = self.object_at(x, y) else {
					return Outcome::Ok;
				};
				let o = &self.project.objects[i];
				let (unit_type, team) = (o.unit_type, o.team);
				let Some(tag) = max_assets::save::unit_type_name(unit_type) else {
					return Outcome::Failed(format!("object-pick: type {unit_type} has no sprite"));
				};
				if let Err(e) = self.ensure_units() {
					return Outcome::Failed(e);
				}
				let lib = self.units.as_ref().expect("ensure_units");
				let Some(idx) = lib.find(tag) else {
					return Outcome::Failed(format!("object-pick: '{tag}' not in the sprite roster"));
				};
				self.active_unit = Some(idx);
				self.unit_team = team;
				self.tool = Tool::Unit;
				self.show_units = true;
				let team_name = crate::units::TEAM_NAMES.get(team as usize).copied().unwrap_or("?");
				self.console.push_line(format!("picked {tag} (team {team_name})"));
				Outcome::Redraw
			}
			Command::ObjectClone { x, y } => {
				// Clone stamp. A cell holding an object re-sources; a bare cell
				// takes a copy of the source - every property included, which is
				// what separates this from the eyedropper.
				if let Some(i) = self.object_at(x, y) {
					let source = self.project.objects[i].clone();
					let tag = max_assets::save::unit_type_name(source.unit_type).unwrap_or("?");
					let team = crate::units::TEAM_NAMES.get(source.team as usize).copied().unwrap_or("?");
					self.console.push_line(format!("cloning {tag} (team {team}) - click a bare cell to stamp it"));
					self.clone_source = Some(source);
					self.show_units = true;
					return Outcome::Redraw;
				}
				let Some(source) = self.clone_source.clone() else {
					return Outcome::Failed("object-clone: click an object first to take it as the source".into());
				};
				let (w, h) = self.map_size();
				if x >= w || y >= h {
					return Outcome::Failed(format!("object-clone: ({x},{y}) is outside the {w}x{h} map"));
				}
				self.project.label_next_undo("Clone object");
				// The copy carries the source's properties; only where it stands
				// changes. A building lays its own slab here too. One exception:
				// a runtime order the source was caught in (moving, building, or
				// STORED - ORDER_IDLE, which the renderer culls on-map) is not a
				// property a fresh copy can hold; those reset to the type's
				// deploy order.
				let mut obj = map_core::MapObject { x, y, ..source };
				let valid = max_assets::save::resting_orders(obj.unit_type);
				if !valid.is_empty() && !valid.contains(&obj.props.orders) {
					obj.props.orders = max_assets::save::deploy_orders(obj.unit_type);
				}
				let slab = max_assets::save::slab_for_type(obj.unit_type).map(|slab_type| map_core::MapObject {
					unit_type: slab_type,
					x,
					y,
					team: obj.team,
					props: map_core::ObjectProps::default(),
				});
				if let Some(slab) = slab {
					self.project.place_object(slab);
				}
				self.project.place_object(obj);
				// A connector mask describes *neighbours*, so the copy's is
				// re-derived where it landed rather than carried over.
				if max_assets::save::is_connector_host_type(source.unit_type) {
					self.project.auto_connect_buildings();
				}
				self.selected_object = None;
				self.show_units = true;
				Outcome::Redraw
			}
			Command::ObjectEdit { field, value } => {
				let Some(index) = self.selected_object else {
					return Outcome::Failed("object-edit: no object selected".into());
				};
				let Some(obj) = self.project.objects.get(index) else {
					self.selected_object = None;
					return Outcome::Failed("object-edit: the selection is stale".into());
				};
				let mut team = obj.team;
				let mut props = obj.props.clone();
				let field = field.to_ascii_lowercase();
				// Parse the value into the target field. `angle` accepts the full
				// u8 range: for a mobile unit it is a 0-7 heading, but for ground
				// cover it indexes a decorative variant (the panel guards per type,
				// S4.5). `orders` takes a slug (`sentry`) or a raw byte.
				match field.as_str() {
					"team" => match crate::units::parse_team(&value) {
						Some(t) => team = t,
						None => return Outcome::Failed(format!("object-edit team: unknown team '{value}'")),
					},
					"name" => props.name = value.clone(),
					"angle" | "facing" => match value.parse::<u8>() {
						Ok(a) => props.angle = a,
						Err(_) => return Outcome::Failed("object-edit angle: expected a number 0-255".into()),
					},
					// Turret heading (0-7), independent of the body `angle` (S4.4); the
					// map overlay uses it for turret units and ignores it otherwise.
					"turret" => match value.parse::<u8>() {
						Ok(t) => props.turret_angle = t,
						Err(_) => return Outcome::Failed("object-edit turret: expected a number 0-255".into()),
					},
					"hits" => match value.parse::<u16>() {
						Ok(h) => props.hits = h,
						Err(_) => return Outcome::Failed("object-edit hits: expected a number 0-65535".into()),
					},
					"ammo" => match value.parse::<u8>() {
						Ok(a) => props.ammo = a,
						Err(_) => return Outcome::Failed("object-edit ammo: expected a number 0-255".into()),
					},
					// Cargo carried / experience accrued — signed (S4.4). Clamped to
					// per-type capacity later.
					"storage" => match value.parse::<i16>() {
						Ok(s) => props.storage = s,
						Err(_) => {
							return Outcome::Failed("object-edit storage: expected a number -32768..32767".into());
						}
					},
					// Connector adjacency bitmask, 8 half-edge bits (S4.4). The panel
					// XORs a single bit; a script may set the whole mask.
					"connectors" => match value.parse::<u16>() {
						Ok(c) => props.connectors = c,
						Err(_) => return Outcome::Failed("object-edit connectors: expected a number 0-65535".into()),
					},
					// Only an order this type can legitimately hold at rest is
					// accepted (`resting_orders`) - move/build orders carry runtime
					// state a placed unit does not have, and ORDER_IDLE marks a unit
					// stored in a container (the renderer culls an on-map IDLE unit).
					// Re-stating the unit's current order is always allowed.
					"orders" => match max_assets::save::order_id(&value).or_else(|| value.parse::<u8>().ok()) {
						Some(o) => {
							let cur = &self.project.objects[index];
							let valid = max_assets::save::resting_orders(cur.unit_type);
							if o != cur.props.orders && !valid.contains(&o) {
								let names: Vec<&str> =
									valid.iter().filter_map(|&v| max_assets::save::order_name(v)).collect();
								return Outcome::Failed(format!(
									"object-edit orders: '{value}' is not a resting order for this unit \
									 (valid: {})",
									names.join("|"),
								));
							}
							props.orders = o;
						}
						None => return Outcome::Failed(format!("object-edit orders: unknown order '{value}'")),
					},
					// Disable the unit for N turns (0 = not disabled). Setting turns > 0
					// also puts it on ORDER_DISABLE so the game treats it as disabled;
					// clearing to 0 lifts a disable back to await. Clamped to 0..=127
					// (the V70 disable byte is signed, so ≥128 would read back as 0).
					"disabled" => match value.parse::<u16>() {
						Ok(n) => {
							props.disabled_turns = n.min(127) as u8;
							if props.disabled_turns > 0 {
								props.orders = max_assets::save::ORDER_DISABLE;
							} else if props.orders == max_assets::save::ORDER_DISABLE {
								props.orders = 0; // ORDER_AWAIT
							}
						}
						Err(_) => return Outcome::Failed("object-edit disabled: expected a number 0-127".into()),
					},
					other => {
						return Outcome::Failed(format!(
							"object-edit: unknown field '{other}' \
							 (team|name|angle|turret|hits|ammo|storage|connectors|orders|disabled)"
						));
					}
				}
				// Nothing to do (and no label to leak) when the value is unchanged.
				let cur = &self.project.objects[index];
				if team == cur.team && props == cur.props {
					return Outcome::Ok;
				}
				// Per-field Undo History label (this edit definitely commits a patch).
				self.project.label_next_undo(format!("Set {field}"));
				self.project.set_object_state(index, team, props);
				self.console.push_line(format!("set {field} = {value}"));
				Outcome::Redraw
			}
			Command::ObjectValues { attr, value } => {
				let Some(index) = self.selected_object else {
					return Outcome::Failed("object-values: no object selected".into());
				};
				if self.project.objects.get(index).is_none() {
					self.selected_object = None;
					return Outcome::Failed("object-values: the selection is stale".into());
				}
				// The stats to edit: this unit's override if it has one, else the
				// save's shared seed, else the stock database seed — cloned here so
				// the first edit forks a per-unit copy (the engine's own
				// clone-on-edit, S4.5). No stats block anywhere → fail.
				let Some(mut values) = self.object_effective_values(index) else {
					return Outcome::Failed("object-values: this object has no unit-values block to edit".into());
				};
				let original = values.clone();
				let attr = attr.to_ascii_lowercase();
				let u16v = value.min(u16::MAX as u32) as u16;
				use max_assets::attribs::StatKind;
				let kind = match attr.as_str() {
					"hits" => StatKind::Hits,
					"attack" => StatKind::Attack,
					"armor" => StatKind::Armor,
					"range" => StatKind::Range,
					"speed" => StatKind::Speed,
					"scan" => StatKind::Scan,
					"rounds" | "shots" => StatKind::Rounds,
					"ammo" => StatKind::Ammo,
					"storage" => StatKind::Storage,
					"turns" => StatKind::Turns,
					"attack-radius" | "attack_radius" => StatKind::AttackRadius,
					"move-and-fire" | "move_and_fire" => StatKind::MoveAndFire,
					"agent-adjust" | "agent_adjust" => StatKind::AgentAdjust,
					"version" => StatKind::Version,
					other => {
						return Outcome::Failed(format!(
							"object-values: unknown attribute '{other}' (hits|attack|armor|range|speed|scan|rounds|ammo|storage|turns|attack-radius|move-and-fire|agent-adjust|version)"
						));
					}
				};
				// A stat the game ignores for this unit type is not editable (S7.5) —
				// e.g. attack on a radar, cargo capacity on a tank.
				if !self.object_stat_applicable(index, kind) {
					let type_name = self
						.project
						.objects
						.get(index)
						.and_then(|o| max_assets::save::unit_type_name(o.unit_type))
						.unwrap_or("this unit type");
					return Outcome::Failed(format!("object-values: '{attr}' is not applicable to {type_name}"));
				}
				match kind {
					StatKind::Hits => values.hits = u16v,
					StatKind::Attack => values.attack = u16v,
					StatKind::Armor => values.armor = u16v,
					StatKind::Range => values.range = u16v,
					StatKind::Speed => values.speed = u16v,
					StatKind::Scan => values.scan = u16v,
					StatKind::Rounds => values.rounds = u16v,
					StatKind::Ammo => values.ammo = u16v,
					StatKind::Storage => values.storage = u16v,
					StatKind::Turns => values.turns = u16v,
					StatKind::AttackRadius => values.attack_radius = u16v,
					StatKind::MoveAndFire => values.move_and_fire = value.min(u8::MAX as u32) as u8,
					StatKind::AgentAdjust => values.agent_adjust = u16v,
					StatKind::Version => values.version = u16v,
				}
				// Setting a stat to its current effective value changes nothing — don't
				// materialize a redundant override (and leak an undo label).
				if values == original {
					return Outcome::Ok;
				}
				let team = self.project.objects[index].team;
				let mut props = self.project.objects[index].props.clone();
				props.base_values = Some(values);
				self.project.label_next_undo(format!("Set max {attr}"));
				self.project.set_object_state(index, team, props);
				self.console.push_line(format!("set max {attr} = {value}"));
				Outcome::Redraw
			}
			Command::ResourceSet { x, y, material, amount } => {
				// Stage D: resources are placeable on ANY project — a save-less
				// map materializes its cargo map on first paint, and save
				// synthesis (`new-save`) carries it into a real `.DTA`.
				let material = material.to_ascii_lowercase();
				let mat = match material.as_str() {
					"none" | "clear" | "empty" => None,
					other => match max_assets::save::CargoMaterial::from_slug(other) {
						Some(m) => Some(m),
						None => {
							return Outcome::Failed(format!(
								"resource-set: unknown material '{other}' (raw|fuel|gold|none)"
							));
						}
					},
				};
				if x >= self.project.width || y >= self.project.height {
					return Outcome::Failed(format!("resource-set: ({x}, {y}) is outside the map"));
				}
				// A save-less project reads 0 until the first paint sizes the map.
				let cur = self.project.cargo_at(x, y).unwrap_or(0);
				// Painted resources are marked surveyed by all players (usable in-game,
				// S5.5); erasing (mat None) clears without adding survey bits.
				let value = max_assets::save::cargo_surveyed(max_assets::save::cargo_compose(cur, mat, amount as u16));
				let label = mat.map_or("Clear resource".to_string(), |m| format!("Paint {}", m.slug()));
				self.project.label_next_undo(label);
				if self.project.set_cargo(x, y, value) {
					self.console.push_line(format!("resource ({x}, {y}) = {material} {}", amount.min(31)));
					Outcome::Redraw
				} else {
					Outcome::Ok
				}
			}
			Command::ResourceBrush { field, value } => {
				match field.to_ascii_lowercase().as_str() {
					"material" | "mat" => {
						self.resource_material = match value.to_ascii_lowercase().as_str() {
							"none" | "clear" | "erase" | "empty" => None,
							other => match max_assets::save::CargoMaterial::from_slug(other) {
								Some(m) => Some(m),
								None => return Outcome::Failed(format!("resource-brush material: unknown '{other}'")),
							},
						};
					}
					"amount" | "amt" => match value.parse::<u8>() {
						Ok(a) => self.resource_amount = a.min(31),
						Err(_) => return Outcome::Failed("resource-brush amount: expected a number 0-31".into()),
					},
					"mode" => match ResourceMode::from_slug(&value.to_ascii_lowercase()) {
						Some(m) => self.resource_mode = m,
						None => {
							return Outcome::Failed(format!("resource-brush mode: unknown '{value}' (set|add|sub)"));
						}
					},
					other => {
						return Outcome::Failed(format!(
							"resource-brush: unknown field '{other}' (material|amount|mode)"
						));
					}
				}
				Outcome::Redraw
			}
			Command::ResourcePaint { x, y } => {
				// Per-cell brush apply during a Resource Brush drag; joins the open
				// stroke so the whole drag is one undo unit. Quiet (no console line).
				if x >= self.project.width || y >= self.project.height {
					return Outcome::Ok;
				}
				// A save-less project reads 0 until the first paint sizes the map.
				let cur = self.project.cargo_at(x, y).unwrap_or(0);
				let value = self.resource_mode.apply(cur, self.resource_material, self.resource_amount as u16);
				// Painting something you cannot see is never what was meant.
				self.reveal_resources();
				self.project.label_next_undo("Resource brush".to_string());
				if self.project.set_cargo(x, y, value) { Outcome::Redraw } else { Outcome::Ok }
			}
			Command::ResourceAmountDialog => Outcome::OpenDialog(DialogRequest::ResourceAmount),
			Command::UnitsVisible { on } => {
				self.show_units = on.unwrap_or(!self.show_units);
				self.console.push_line(format!("units: {}", if self.show_units { "shown" } else { "hidden" }));
				Outcome::Redraw
			}
			Command::SaveSettings => match &self.settings_path {
				None => {
					self.console.push_line("save-settings: no settings file (pass --settings PATH)");
					Outcome::Redraw
				}
				Some(path) => {
					// Flush the live layout into its group's slot, then emit every
					// group's section. Only the main [Workspace] carries the global
					// extras (UI scale + explorer preview sizes).
					self.saved_layouts[self.mode.layout_group() as usize] = self.workspace.save_layout();
					let mut main = Workspace::layout_to_ini(&self.saved_layouts[LayoutGroup::Main as usize]);
					let _ = main.set_entry("UiScale".to_string(), self.ui_scale.to_string());
					// Explorer thumbnail sizes (Tile Explorer + Templates Explorer) persist
					// alongside the layout, so the chosen preview size survives a restart.
					let _ = main.set_entry("TilesPreview".to_string(), self.picker.tile_px.to_string());
					let _ = main.set_entry("TemplatesPreview".to_string(), self.templates.cell.to_string());
					let _ = main.set_entry("SceneryPreview".to_string(), self.scenery_cell.to_string());
					let pass = Workspace::layout_to_ini(&self.saved_layouts[LayoutGroup::Pass as usize]);
					let save = Workspace::layout_to_ini(&self.saved_layouts[LayoutGroup::Save as usize]);
					// Recent maps live in their own [QuickLoad] section, persisted
					// immediately on every open (see `remember_recent`);
					// [Preferences] likewise persists on change and rides along here.
					self.save_preferences();
					let sections = vec![
						(LayoutGroup::Main.ini_section(), main),
						(LayoutGroup::Pass.ini_section(), pass),
						(LayoutGroup::Save.ini_section(), save),
					];
					match crate::settings_io::save_sections(path, sections) {
						Ok(()) => {
							self.console.push_line(format!("settings saved -> {}", path.display()));
							Outcome::Redraw
						}
						Err(e) => Outcome::Failed(format!("save-settings: {e}")),
					}
				}
			},
			_ => unreachable!("non-panel command routed to exec_panels"),
		}
	}

	/// Load the unit sprite library once (needs `MaxPath` → MAX.RES). A
	/// failed attempt doesn't retry - the cause lands in the console.
	pub fn ensure_units(&mut self) -> Result<(), String> {
		if self.units.is_some() {
			return Ok(());
		}
		if self.units_loaded {
			return Err("units: not available (see console)".into());
		}
		self.units_loaded = true;
		let Some(max_path) = self.max_path.clone() else {
			return Err("units: set MaxPath in resources/user/config/mme.ini first".into());
		};
		match crate::units::UnitLibrary::load(&max_path) {
			Ok(lib) => {
				self.console.push_line(format!("units: {} sprites loaded from MAX.RES", lib.units.len()));
				self.units = Some(lib);
				Ok(())
			}
			Err(e) => {
				self.console.push_line(format!("units: {e}"));
				Err(format!("units: {e}"))
			}
		}
	}

	/// Load the resource-marker sprite library once (needs `MaxPath` → MAX.RES),
	/// mirroring [`ensure_units`](Self::ensure_units). A failed attempt doesn't
	/// retry - the resource overlay then falls back to the flat material tint.
	pub fn ensure_markers(&mut self) -> Result<(), String> {
		if self.markers.is_some() {
			return Ok(());
		}
		if self.markers_loaded {
			return Err("markers: not available (see console)".into());
		}
		self.markers_loaded = true;
		let Some(max_path) = self.max_path.clone() else {
			return Err("markers: set MaxPath in resources/user/config/mme.ini first".into());
		};
		match crate::markers::MarkerLibrary::load(&max_path) {
			Ok(lib) => {
				self.markers = Some(lib);
				Ok(())
			}
			Err(e) => {
				self.console.push_line(format!("resource markers: {e} (overlay uses flat tint)"));
				Err(format!("markers: {e}"))
			}
		}
	}

	/// Turn the resource overlay on if it is off - what the Resource Brush works
	/// on is invisible otherwise, so arming it or painting with it reveals the
	/// cargo it is about to change. A view setting, so it is not undoable and it
	/// says so in the console like any other overlay flip.
	fn reveal_resources(&mut self) {
		if self.show_resources {
			return;
		}
		self.show_resources = true;
		self.console.push_line("resource overlay: on (the Resource Brush needs it)");
		// Load the marker sprites so the overlay draws them like the game; a
		// failure (no MaxPath) is non-fatal - the overlay falls back to tint.
		let _ = self.ensure_markers();
	}

	/// A compact readout of the resource at cell `(x, y)` for the status bar (S5.4)
	/// — `raw 15` / `fuel 8` / `empty`, or `None` unless the resource overlay is on
	/// or the Resource Brush is armed (so it never clutters ordinary map editing).
	/// A save-less project (no cargo map yet, Stage D) reads `empty` in-bounds.
	pub fn resource_readout(&self, x: u16, y: u16) -> Option<String> {
		if !(self.show_resources || self.tool == Tool::ResourceBrush) {
			return None;
		}
		if x >= self.project.width || y >= self.project.height {
			return None;
		}
		let value = self.project.cargo_at(x, y).unwrap_or(0);
		Some(match max_assets::save::cargo_material(value) {
			Some(m) => format!("{} {}", m.slug(), max_assets::save::cargo_amount(value)),
			None => "empty".to_string(),
		})
	}

	/// The maximum HP of object `index` — the cap the Unit Properties hits editor
	/// clamps to (S4.5). Reads the object's *effective* `base_values` (its per-unit
	/// override when edited, else the save's shared seed), so raising the max HP
	/// via `object-values` lifts the current-hits cap live. `None` for a fresh
	/// placement with no save (then current hits is unbounded).
	pub fn object_max_hits(&self, index: usize) -> Option<u16> {
		self.object_effective_values(index).map(|v| v.hits)
	}

	/// The maximum stats governing object `index`: its per-unit override, else
	/// the attached save's shared seed ([`Project::object_base_values`]), else —
	/// new to Stage B — the **stock base values** from the max-port unit
	/// database, so the stats editor works on save-less maps too. `None` only
	/// when the database is absent as well.
	pub fn object_effective_values(&self, index: usize) -> Option<max_assets::save::UnitValues> {
		self.project.object_base_values(index).or_else(|| {
			let unit_type = self.project.objects.get(index)?.unit_type;
			self.unit_stats.as_ref()?.base_for(unit_type).cloned()
		})
	}

	/// Whether a max-stat is *applicable* to object `index`'s unit type (S7.5):
	/// judged from the stock database (`attribs::stat_applicable` — no attack
	/// editor on a radar, no cargo editor on a tank). Permissive by design:
	/// without the database there is no basis to restrict, and a stat already
	/// carrying a nonzero value (edited/modded data) stays editable so nothing
	/// is ever trapped.
	pub fn object_stat_applicable(&self, index: usize, kind: max_assets::attribs::StatKind) -> bool {
		let Some(obj) = self.project.objects.get(index) else { return true };
		let Some(db) = &self.unit_stats else { return true };
		let (Some(meta), Some(base)) = (db.meta_for(obj.unit_type), db.base_for(obj.unit_type)) else {
			return true;
		};
		if max_assets::attribs::stat_applicable(kind, obj.unit_type, meta, base) {
			return true;
		}
		self.object_effective_values(index).is_some_and(|v| kind.get(&v) != 0)
	}

	/// The footprint (cells per side) of an object of `unit_type`, from its
	/// sprite (the single source of truth). `1` when the sprite library isn't
	/// loaded or the type has no sprite - hit-testing then treats it as 1×1.
	fn object_footprint(&self, unit_type: u16) -> u16 {
		self.units.as_ref().and_then(|lib| lib.find_type(unit_type).map(|i| lib.units[i].footprint)).unwrap_or(1) as u16
	}

	/// The footprint (cells per side) of object `index`; `1` when the index is
	/// out of range or the type has no sprite.
	pub fn object_footprint_of(&self, index: usize) -> u16 {
		self.project.objects.get(index).map(|o| self.object_footprint(o.unit_type)).unwrap_or(1)
	}

	/// Whether an object of `unit_type` has an independent turret — its sprite
	/// carries turret frames (`turret_count() > 0`). Gates the Unit Properties
	/// Turret dropdown (S4.4). `false` when the sprite library isn't loaded or the
	/// type has no turret.
	fn type_has_turret(&self, unit_type: u16) -> bool {
		self.units
			.as_ref()
			.and_then(|lib| lib.find_type(unit_type).map(|i| lib.units[i].turret_count() > 0))
			.unwrap_or(false)
	}

	/// Whether object `index` has an independent turret heading to edit; `false`
	/// when the index is out of range or the type has no turret sprite.
	pub fn object_has_turret(&self, index: usize) -> bool {
		self.project.objects.get(index).map(|o| self.type_has_turret(o.unit_type)).unwrap_or(false)
	}

	/// The topmost object whose footprint covers cell `(x, y)`, as an index into
	/// `project.objects` - `None` when the cell is empty. A 2×2 object at
	/// `(ox, oy)` covers `[ox, ox+2) × [oy, oy+2)`, so any of its four cells
	/// selects it.
	pub fn object_at(&self, x: u16, y: u16) -> Option<usize> {
		self.objects_at(x, y).first().copied()
	}

	/// Every object whose footprint covers cell `(x, y)`, **top-most first** —
	/// the reverse of the paint order ([`crate::units::draw_order`]), so a
	/// building always wins over the slab under it, and the stack a click cycles
	/// through (see [`Self::object_at_cycling`]) runs downwards from what is
	/// visibly on top. Empty when the cell is bare.
	pub fn objects_at(&self, x: u16, y: u16) -> Vec<usize> {
		crate::units::draw_order(&self.project.objects)
			.rev()
			.filter_map(|(i, o)| {
				let f = self.object_footprint(o.unit_type);
				(x >= o.x && x < o.x + f && y >= o.y && y < o.y + f).then_some(i)
			})
			.collect()
	}

	/// The object a click at `(x, y)` should select: the top-most, unless `current`
	/// already covers the cell — then the next object below it, wrapping. Repeated
	/// clicks on the same overlapped cell thus loop through every stacked object
	/// (item 7). `None` when the cell is bare.
	pub fn object_at_cycling(&self, x: u16, y: u16, current: Option<usize>) -> Option<usize> {
		let stack = self.objects_at(x, y);
		if let Some(cur) = current {
			if let Some(pos) = stack.iter().position(|&i| i == cur) {
				return Some(stack[(pos + 1) % stack.len()]);
			}
		}
		stack.first().copied()
	}

	/// Whether object `moving`, placed at `(x, y)`, would overlap another object
	/// that blocks it. Buildings (footprint ≥ 2) can't overlap anything and
	/// nothing can overlap them; footprint-1 objects (units, ground cover) stack
	/// freely (a tank on a slab), so an overlap only blocks when a 2×2 is on
	/// either side. Used by the Move tool to refuse a drop.
	pub fn object_collides(&self, moving: usize, x: u16, y: u16) -> bool {
		let Some(mo) = self.project.objects.get(moving) else { return false };
		let mf = self.object_footprint(mo.unit_type);
		self.project.objects.iter().enumerate().any(|(i, o)| {
			if i == moving {
				return false;
			}
			let f = self.object_footprint(o.unit_type);
			// AABB overlap of the two footprints, and at least one is a building.
			let overlap = x < o.x + f && o.x < x + mf && y < o.y + f && o.y < y + mf;
			overlap && (mf >= 2 || f >= 2)
		})
	}

	/// Stamp (or restamp) a unit preview on a cell as a first-class, undoable
	/// object (S2.1). Persists with the project and dirties it.
	fn place_unit_preview(&mut self, unit: usize, x: u16, y: u16) -> Outcome {
		let (w, h) = self.map_size();
		if x >= w || y >= h {
			return Outcome::Failed(format!("unit-place: ({x},{y}) is outside the {w}x{h} map"));
		}
		// `unit` indexes the unit library's roster, so the library has to be loaded
		// AND the index still in range. Both can lapse between arming a unit and
		// clicking: changing MaxPath drops the library for a reload against the new
		// folder, and the roster it comes back with need not be the same length.
		// The scripted `unit-place` calls `ensure_units` for us; the interactive
		// `Command::Paint` path does not, so ask here rather than trust the caller.
		if let Err(e) = self.ensure_units() {
			return Outcome::Failed(e);
		}
		let Some(entry) = self.units.as_ref().and_then(|lib| lib.units.get(unit)) else {
			return Outcome::Failed(format!("unit-place: no unit #{unit} in the library"));
		};
		let tag = entry.tag.clone();
		let Some(unit_type) = max_assets::save::unit_type_id(&tag) else {
			return Outcome::Failed(format!("unit-place: '{tag}' is not a placeable unit type"));
		};
		// A drag lays a stroke of units, and a continuation cell must not
		// overpaint: once the stroke has placed its first object, any cell whose
		// footprint would overlap an existing object's is skipped. The press
		// itself (nothing object-shaped in the stroke yet) keeps the
		// restamp-on-click semantics, so a deliberate click still replaces.
		if self.project.stroke_touched_objects() {
			let f = self.object_footprint(unit_type);
			let overlaps = self.project.objects.iter().any(|o| {
				let of = self.object_footprint(o.unit_type);
				x < o.x + of && o.x < x + f && y < o.y + of && o.y < y + f
			});
			if overlaps {
				return Outcome::Ok;
			}
		}
		// The unit tool reaches here via `Command::Paint` (labelled "Paint");
		// relabel so the Undo History reads meaningfully.
		self.project.label_next_undo("Place unit");
		// A building lays its foundation the way the game does - the slab first,
		// so it is under the structure in the draw order too. Same cell, same
		// owner; `place_object` keeps the two because they are different layers.
		let slab = max_assets::save::slab_for_type(unit_type).map(|slab_type| map_core::MapObject {
			unit_type: slab_type,
			x,
			y,
			team: self.unit_team,
			props: map_core::ObjectProps::default(),
		});
		// A fresh placement starts on its type's deploy order (a mining station
		// powers on and produces, a turret watches, a power plant idles off) —
		// the engine's own construction defaults; the Unit Properties panel
		// shows it and any user edit overrides it on export.
		let obj = map_core::MapObject {
			unit_type,
			x,
			y,
			team: self.unit_team,
			props: map_core::ObjectProps { orders: max_assets::save::deploy_orders(unit_type), ..Default::default() },
		};
		// Placing a building/connector auto-connects it to same-team neighbours —
		// place + connect commit as ONE undo step. During a drag the shell already
		// opened the stroke (the whole drag = one undo unit), so don't nest a fresh
		// begin/end here — that would split the drag; just place + connect inside it.
		let host = max_assets::save::is_connector_host_type(unit_type);
		let lay = |p: &mut map_core::Project| {
			if let Some(slab) = slab {
				p.place_object(slab);
			}
			p.place_object(obj);
		};
		if host && !self.project.in_stroke() {
			self.project.begin_stroke();
			lay(&mut self.project);
			self.project.auto_connect_buildings();
			self.project.end_stroke();
		} else {
			lay(&mut self.project);
			if host {
				self.project.auto_connect_buildings();
			}
		}
		// Replacing an object on a cell shifts the list, staling any selection.
		self.selected_object = None;
		self.show_units = true;
		Outcome::Redraw
	}

	/// Selection, clipboard, ghost stamps, and templates.
	fn exec_select(&mut self, command: Command) -> Outcome {
		match command {
			Command::SelectOp { op } => {
				match op.as_str() {
					"all" => self.selection.select_all(),
					"clear" => self.selection.clear(),
					"invert" => self.selection.invert(),
					"similar" => {
						// Fallback key when nothing is selected: the active brush.
						let fallback = self
							.active_tile
							.as_deref()
							.and_then(|spec| self.project.resolve_ref(spec).ok())
							.map(|(t, _)| (t.pack, t.tile));
						self.selection.select_similar(&self.project, fallback);
					}
					other => return Outcome::Failed(format!("select: unknown '{other}' (all|clear|invert|similar)")),
				}
				self.console.push_line(format!("select {op}: {} cells", self.selection.count()));
				Outcome::Redraw
			}
			Command::SelectCell { x, y, mode } => {
				self.selection.apply_cell(x, y, mode);
				Outcome::Redraw
			}
			Command::SelectRect { x0, y0, x1, y1, mode } => {
				if mode == SelectMode::Replace {
					self.selection.clear();
				}
				self.selection.apply_rect(x0, y0, x1, y1, mode);
				self.console.push_line(format!("select: {} cells", self.selection.count()));
				Outcome::Redraw
			}
			Command::SelectMove { dx, dy } => {
				if self.selection.translate(dx, dy) {
					Outcome::Redraw
				} else {
					Outcome::Ok
				}
			}
			Command::Copy => match Template::capture_clipboard(&self.project, &self.selection) {
				Ok(t) => {
					self.console.push_line(format!("copied {}x{} cells", t.width, t.height));
					self.clipboard = Some(t);
					Outcome::Redraw
				}
				Err(e) => Outcome::Failed(format!("copy: {e}")),
			},
			Command::Cut => match Template::capture_clipboard(&self.project, &self.selection) {
				Ok(t) => {
					// Cut keeps the water base, like the eraser - it lifts the ground.
					clear_selection_layer(&mut self.project, &self.selection, LAYER_GROUND);
					self.console.push_line(format!("cut {}x{} cells", t.width, t.height));
					self.clipboard = Some(t);
					Outcome::Redraw
				}
				Err(e) => Outcome::Failed(format!("cut: {e}")),
			},
			Command::Delete => {
				if self.selection.is_empty() {
					return Outcome::Failed("delete: empty selection (drag a select tool first)".into());
				}
				let n = self.selection.count();
				// Clear the active layer - deleting on the water layer drops water
				// exactly as deleting on ground drops ground (no land/water split).
				let layer = self.tile_layer();
				clear_selection_layer(&mut self.project, &self.selection, layer);
				self.console.push_line(format!("deleted {n} cells ({})", self.tile_layer_name()));
				Outcome::Redraw
			}
			Command::DeleteAll => {
				if self.selection.is_empty() {
					return Outcome::Failed("delete-all: empty selection (drag a select tool first)".into());
				}
				let n = self.selection.count();
				clear_selection(&mut self.project, &self.selection);
				self.console.push_line(format!("deleted {n} cells (all layers)"));
				Outcome::Redraw
			}
			Command::Paste => {
				let Some(t) = self.clipboard.clone() else {
					return Outcome::Failed("paste: the clipboard is empty (copy or cut first)".into());
				};
				self.arm_stamp(t);
				self.console.push_line("paste: click the map to place (Esc cancels)".to_string());
				Outcome::Redraw
			}
			Command::Stamp { x, y } => {
				let Some(stamp) = self.stamp.clone() else {
					return Outcome::Failed("stamp: nothing armed (paste or pick a template first)".into());
				};
				// Multi-cell chunks centre on the cursor cell, not top-left it.
				let (ox, oy) = stamp_origin(&stamp, x, y);
				match stamp.apply(&mut self.project, ox, oy) {
					// The stamp stays armed for repeat placing (forests!).
					Ok(_) => Outcome::Redraw,
					Err(e) => Outcome::Failed(format!("stamp: {e}")),
				}
			}
			Command::StampCancel => {
				self.disarm_stamp();
				Outcome::Redraw
			}
			Command::TemplateSave { name } => {
				let base = self.user_templates_dir();
				// Capture first so we know which pack subdir it belongs in.
				let mut template =
					match Template::capture(&self.project, &self.selection, name.as_deref().unwrap_or("template")) {
						Ok(t) => t,
						Err(e) => return Outcome::Failed(format!("template-save: {e}")),
					};
				let dir = base.join(template_pack(&template));
				// Display name = given (kept as typed) or an auto `template`/`-N`;
				// filename = its sanitized, collision-bumped stem.
				let display = name.clone().unwrap_or_else(|| free_stem_in(&dir, "template", None));
				let stem = free_stem_in(&dir, &sanitize_filename(&display), None);
				template.name = display.clone();
				let path = dir.join(format!("{stem}.json"));
				if let Err(e) = template.save(&path) {
					return Outcome::Failed(format!("template-save: {e}"));
				}
				self.template_saved(
					format!("template saved: {display} ({}x{})", template.width, template.height),
					&path,
				)
			}
			Command::TemplateDelete { name } => {
				let Some(i) = self.find_template(name.as_deref()) else {
					return Outcome::Failed("template-delete: no template selected".into());
				};
				if self.templates.entries[i].stock && !self.dev_mode {
					return Outcome::Failed(format!(
						"template-delete: '{}' is a stock template (clone it instead)",
						self.templates.entries[i].name
					));
				}
				let entry = &self.templates.entries[i];
				if let Err(e) = std::fs::remove_file(&entry.path) {
					return Outcome::Failed(format!("template-delete {}: {e}", entry.path.display()));
				}
				self.console.push_line(format!("template deleted: {}", entry.name));
				self.scan_templates();
				Outcome::Redraw
			}
			Command::TemplatePick { name } => {
				// Prefer the explorer's selection (it arms the exact entry the user
				// clicked); names can now repeat across tilesets, so a bare name lookup
				// is ambiguous. Fall back to the first match for the scripted path.
				let i = self
					.templates
					.sel
					.filter(|&s| self.templates.entries.get(s).is_some_and(|t| t.name == name))
					.or_else(|| self.find_template(Some(&name)));
				let Some(i) = i else {
					return Outcome::Failed(format!("template-pick: no template named '{name}'"));
				};
				let t = self.templates.entries[i].template.clone();
				if let Some(id) = t.missing_id(&self.project) {
					return Outcome::Failed(format!(
						"template-pick: '{name}' needs tile '{id}' - its pack isn't in this map"
					));
				}
				self.arm_stamp(t);
				// A stamp is terrain: leave the Scenery layer (a template spans both
				// tile layers, so ground - the one it is edited on) or the click that
				// places the ghost would look like a scenery click on the toolbox.
				self.leave_scenery_layer(LAYER_GROUND);
				self.templates.sel = Some(i);
				self.console.push_line(format!("template armed: {name} (click the map to place, Esc cancels)"));
				Outcome::Redraw
			}
			Command::TemplateClone { name } => {
				let Some(i) = self.find_template(name.as_deref()) else {
					return Outcome::Failed("template-clone: no template selected".into());
				};
				let base = self.user_templates_dir();
				let mut template = self.templates.entries[i].template.clone();
				let dir = base.join(template_pack(&template));
				let display = format!("{}-copy", self.templates.entries[i].name);
				let stem = free_stem_in(&dir, &sanitize_filename(&display), None);
				template.name = display.clone();
				let path = dir.join(format!("{stem}.json"));
				if let Err(e) = template.save(&path) {
					return Outcome::Failed(format!("template-clone: {e}"));
				}
				self.template_saved(format!("template cloned: {display}"), &path)
			}
			Command::TemplateImport { path } => {
				let template = match Template::load(&path) {
					Ok(t) => t,
					Err(e) => return Outcome::Failed(format!("template-import: {e}")),
				};
				let base = self.user_templates_dir();
				let dir = base.join(template_pack(&template));
				// Keep the imported display name; only the filename is sanitized
				// (and bumped on collision).
				let stem = free_stem_in(&dir, &sanitize_filename(&template.name), None);
				let dst = dir.join(format!("{stem}.json"));
				if let Err(e) = template.save(&dst) {
					return Outcome::Failed(format!("template-import: {e}"));
				}
				self.template_saved(
					format!("template imported: {} ({}x{})", template.name, template.width, template.height),
					&dst,
				)
			}
			Command::TemplateExport { path } => {
				let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or("template".into());
				match Template::capture(&self.project, &self.selection, &name) {
					Ok(t) => match t.save(&path) {
						Ok(()) => {
							self.console.push_line(format!("template exported: {}", path.display()));
							Outcome::Redraw
						}
						Err(e) => Outcome::Failed(format!("template-export: {e}")),
					},
					Err(e) => Outcome::Failed(format!("template-export: {e}")),
				}
			}
			Command::TemplateExportPng { path } => match path {
				// Bare: open the save dialog (the explorer's context menu route).
				None => {
					if self.templates.sel.is_none() {
						return Outcome::Failed("template-export-png: no template selected".into());
					}
					self.execute(Command::FileDialog { purpose: FilePurpose::ExportTemplatePng })
				}
				Some(path) => self.template_export_png(&path),
			},
			Command::TemplateRenameModal => {
				let Some(i) = self.templates.sel else {
					return Outcome::Failed("template-rename: no template selected".into());
				};
				if self.templates.entries[i].stock && !self.dev_mode {
					return Outcome::Failed(format!(
						"template-rename: '{}' is a stock template (clone it instead)",
						self.templates.entries[i].name
					));
				}
				// Other templates' display names **in the same tileset** (same pack
				// subdir) - renaming onto one is rejected so the user can correct it
				// (the modal alerts in place). The same name is allowed in other
				// tilesets, so only same-pack names collide.
				let pack = template_pack(&self.templates.entries[i].template);
				let existing: Vec<String> = self
					.templates
					.entries
					.iter()
					.enumerate()
					.filter(|(j, t)| *j != i && template_pack(&t.template) == pack)
					.map(|(_, t)| t.name.clone())
					.collect();
				let entry = &self.templates.entries[i];
				let from = entry.name.clone();
				let footprint = (entry.template.width, entry.template.height);
				let preview = crate::template_preview::compose(&self.project, &entry.template, self.cycler.rgba());
				Outcome::OpenDialog(DialogRequest::RenameTemplate { from, footprint, existing, preview })
			}
			Command::TemplateDeleteModal => {
				let Some(i) = self.templates.sel else {
					return Outcome::Failed("template-delete: no template selected".into());
				};
				if self.templates.entries[i].stock && !self.dev_mode {
					return Outcome::Failed(format!(
						"template-delete: '{}' is a stock template (clone it instead)",
						self.templates.entries[i].name
					));
				}
				let entry = &self.templates.entries[i];
				let name = entry.name.clone();
				let footprint = (entry.template.width, entry.template.height);
				let preview = crate::template_preview::compose(&self.project, &entry.template, self.cycler.rgba());
				Outcome::OpenDialog(DialogRequest::DeleteTemplate { name, footprint, preview })
			}
			Command::TemplateRename { from, to } => {
				// Prefer the selected template - the GUI always renames the selection,
				// and names can now repeat across tilesets so a bare name lookup is
				// ambiguous. Fall back to the first editable template with that name for
				// the scripted path. Stock templates are editable only under `--dev`.
				let editable = |t: &TemplateEntry| (self.dev_mode || !t.stock) && t.name == from;
				let i = self
					.templates
					.sel
					.filter(|&s| editable(&self.templates.entries[s]))
					.or_else(|| self.templates.entries.iter().position(editable));
				let Some(i) = i else {
					return Outcome::Failed(format!("template-rename: no editable template named '{from}'"));
				};
				let display = to.trim().to_string();
				if display.is_empty() {
					return Outcome::Failed("template-rename: the name is empty".into());
				}
				// Reject a name already used by another template **in the same
				// tileset** (same pack subdir); the same name is allowed across
				// different tilesets. The modal alerts in place; this also guards the
				// scripted path.
				let pack = template_pack(&self.templates.entries[i].template);
				if self
					.templates
					.entries
					.iter()
					.enumerate()
					.any(|(j, t)| j != i && t.name == display && template_pack(&t.template) == pack)
				{
					return Outcome::Failed(format!(
						"template-rename: a template named \"{display}\" already exists in this tileset"
					));
				}
				let src = self.templates.entries[i].path.clone();
				// Stay in the template's own pack subdir; the display name keeps the
				// user's text, the filename is sanitized and bumped on collision
				// (ignoring this file itself, so a display-only rename is fine).
				let dir = src.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| src.clone());
				let stem = free_stem_in(&dir, &sanitize_filename(&display), Some(&src));
				let dst = dir.join(format!("{stem}.json"));
				let mut template = self.templates.entries[i].template.clone();
				template.name = display.clone();
				if let Err(e) = template.save(&dst) {
					return Outcome::Failed(format!("template-rename: {e}"));
				}
				if dst != src {
					if let Err(e) = std::fs::remove_file(&src) {
						self.console.push_line(format!("template-rename: kept old {} ({e})", src.display()));
					}
				}
				self.template_saved(format!("template renamed: {from} -> {display}"), &dst)
			}
			Command::TemplateDedupeModal => {
				let dups = self.duplicate_template_indices();
				let names = dups.iter().map(|&i| self.templates.entries[i].name.clone()).collect();
				Outcome::OpenDialog(DialogRequest::DedupeTemplates { names })
			}
			Command::TemplateDedupe => {
				let dups = self.duplicate_template_indices();
				if dups.is_empty() {
					self.console.push_line("template-dedupe: no duplicates");
					return Outcome::Redraw;
				}
				let mut removed = 0;
				// Remove by descending index so earlier indices stay valid (we
				// rescan after, but the paths are captured up front regardless).
				let paths: Vec<PathBuf> = dups.iter().map(|&i| self.templates.entries[i].path.clone()).collect();
				for path in &paths {
					match std::fs::remove_file(path) {
						Ok(()) => removed += 1,
						Err(e) => self.console.push_line(format!("template-dedupe: {} {e}", path.display())),
					}
				}
				self.console.push_line(format!("template-dedupe: removed {removed} duplicate(s)"));
				self.scan_templates();
				Outcome::Redraw
			}
			Command::TemplateExplore => {
				let dir = self.user_templates_dir();
				// Created lazily on first save/import - make sure it exists so the
				// file manager has something to open.
				if let Err(e) = std::fs::create_dir_all(&dir) {
					return Outcome::Failed(format!("template-explore: create {}: {e}", dir.display()));
				}
				if self.headless {
					// No desktop to hand off to (screenshot/CI runs).
					self.console.push_line(format!("template-explore: {} (headless, not opened)", dir.display()));
					return Outcome::Redraw;
				}
				match open_in_file_manager(&dir) {
					Ok(()) => {
						self.console.push_line(format!("opened {}", dir.display()));
						Outcome::Redraw
					}
					Err(e) => Outcome::Failed(format!("template-explore: {e}")),
				}
			}
			_ => unreachable!("non-selection command routed to exec_select"),
		}
	}

	/// Indices into `templates` of removable exact-duplicate user templates
	/// among the **visible** (map-compatible) list - what the explorer shows.
	/// A template is a removable duplicate when its content (size + cells)
	/// exactly matches an *earlier* visible template and it is not stock
	/// (stock files can't be deleted). The first occurrence is always kept.
	fn duplicate_template_indices(&self) -> Vec<usize> {
		let visible = self.visible_templates();
		let mut dups = Vec::new();
		for (pos, &i) in visible.iter().enumerate() {
			let t = &self.templates.entries[i].template;
			let is_dup = visible[..pos].iter().any(|&j| {
				let o = &self.templates.entries[j].template;
				t.width == o.width && t.height == o.height && t.cells == o.cells
			});
			if is_dup && !self.templates.entries[i].stock {
				dups.push(i);
			}
		}
		dups
	}

	/// `resources/user/templates` - where the user's saved templates live, in
	/// per-pack subdirs (created on first save/import).
	fn user_templates_dir(&self) -> PathBuf {
		self.resources_root.join("user/templates")
	}

	/// `resources/assets/templates` - the shipped stamp templates (read-only),
	/// in per-pack subdirs.
	fn stock_templates_dir(&self) -> PathBuf {
		self.resources_root.join("assets/templates")
	}

	/// Re-read both template trees into `templates` - shipped
	/// (`assets/templates`) then user (`user/templates`). Templates live in
	/// per-pack subdirs named for the tileset(s) they use (`templates/<PACKS>/
	/// *.json`, e.g. `GREEN+WATER`); loose files directly under the base are
	/// tolerated too. Order: stock group then user group, packs in natural order,
	/// names natural-sorted within each. Unparseable files are skipped.
	pub fn scan_templates(&mut self) {
		let mut entries = Vec::new();
		for (base, stock) in [(self.stock_templates_dir(), true), (self.user_templates_dir(), false)] {
			// The base itself first (loose/legacy files), then each pack subdir.
			let mut packs: Vec<PathBuf> = match std::fs::read_dir(&base) {
				Ok(read) => read.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect(),
				Err(_) => Vec::new(),
			};
			packs.sort_by(|a, b| natural_cmp(stem(a), stem(b)));
			for dir in std::iter::once(base.clone()).chain(packs) {
				let Ok(read) = std::fs::read_dir(&dir) else { continue };
				let mut paths: Vec<PathBuf> =
					read.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
				// Natural order so numbers grow by value (3 < 20 < 100).
				paths.sort_by(|a, b| natural_cmp(stem(a), stem(b)));
				for path in paths {
					match Template::load(&path) {
						Ok(template) => {
							entries.push(TemplateEntry { name: template.name.clone(), path, stock, template })
						}
						Err(e) => self.console.push_line(format!("templates: skipped {e}")),
					}
				}
			}
		}
		self.templates.set_entries(entries);
		if self.templates.sel.is_some_and(|i| i >= self.templates.entries.len()) {
			self.templates.sel = None;
		}
	}

	/// Indices into `templates` that resolve against the open map - what the
	/// explorer shows (incompatible ones would only stamp errors), further
	/// narrowed to the selected tileset (by `template_pack` label) when one is
	/// chosen in the panel's tileset filter.
	pub fn visible_templates(&self) -> Vec<usize> {
		let ts = self.templates.tileset.as_deref();
		(0..self.templates.entries.len())
			.filter(|&i| {
				let e = &self.templates.entries[i];
				e.template.compatible(&self.project) && ts.is_none_or(|name| template_pack(&e.template) == name)
			})
			.collect()
	}

	/// The distinct tileset labels (`template_pack`) among the map-compatible
	/// templates, sorted - the option list for the explorer's tileset filter.
	/// Independent of the current selection so picking one never hides the rest.
	pub fn template_tilesets(&self) -> Vec<String> {
		let mut names: Vec<String> = self
			.templates
			.entries
			.iter()
			.filter(|e| e.template.compatible(&self.project))
			.map(|e| template_pack(&e.template))
			.collect();
		names.sort_unstable();
		names.dedup();
		names
	}

	/// Resolve a delete/clone target: an explicit name, else the explorer's
	/// selected entry.
	fn find_template(&self, name: Option<&str>) -> Option<usize> {
		match name {
			Some(n) => self.templates.entries.iter().position(|t| t.name == n),
			None => self.templates.sel,
		}
	}

	/// Document lifecycle: undo/redo, open/save/save-copy, file dialog, resize
	/// (+ its modal), the Auto Fix Shore modal, and WRL export.
	fn exec_io(&mut self, command: Command) -> Outcome {
		match command {
			Command::Undo => {
				let structure = self.project.structure_revision();
				if self.project.undo() {
					self.refresh_palette(); // the patch may have carried colors
					// A document-swap patch (palette conversion) replaced the
					// tile tables - the GPU atlas must rebuild.
					if self.project.structure_revision() != structure { Outcome::DocReplaced } else { Outcome::Redraw }
				} else {
					Outcome::Ok
				}
			}
			Command::UndoTo { steps } => {
				let structure = self.project.structure_revision();
				if self.project.undo_steps(steps) > 0 {
					self.refresh_palette();
					if self.project.structure_revision() != structure { Outcome::DocReplaced } else { Outcome::Redraw }
				} else {
					Outcome::Ok
				}
			}
			Command::Redo => {
				let structure = self.project.structure_revision();
				if self.project.redo() {
					self.refresh_palette();
					if self.project.structure_revision() != structure { Outcome::DocReplaced } else { Outcome::Redraw }
				} else {
					Outcome::Ok
				}
			}
			// Open adds a tab: no dirty guard (the current tab stays
			// open), and re-opening a path switches to its tab. `force` is now
			// vestigial.
			Command::Open { path } => {
				if path.extension().is_some_and(|e| e == "json") {
					// Layered map project.
					match Project::load(&path, &self.assets_root) {
						Ok(project) => {
							eprintln!(
								"opened {}: \"{}\" {}x{} cells, packs: {}",
								path.display(),
								project.name,
								project.width,
								project.height,
								project.uses.iter().map(|u| u.name.as_str()).collect::<Vec<_>>().join("+"),
							);
							// A read-only template loads path-less (Save → Save-As), but its
							// origin is kept so DEV ▸ Update Map can write back to it.
							let doc_path = if self.is_template(&path) { None } else { Some(path.clone()) };
							self.remember_recent(&path);
							self.add_doc(project, doc_path, Some(path))
						}
						Err(e) => Outcome::Failed(format!("open {}: {e}", path.display())),
					}
				} else {
					match read_wrl_file(&path) {
						Ok(wrl) => {
							let name = path
								.file_stem()
								.map(|s| s.to_string_lossy().into_owned())
								.unwrap_or_else(|| "map".into());
							let project = Project::from_wrl(&wrl, &name);
							eprintln!(
								"imported {}: {}x{} cells, {} tiles",
								path.display(),
								project.width,
								project.height,
								wrl.tile_count
							);
							// An imported WRL has no project file yet - `Save Project`
							// asks where to save (Save-As), never writes the WRL.
							self.remember_recent(&path);
							self.add_doc(project, None, None)
						}
						Err(e) => Outcome::Failed(format!("open {}: {e}", path.display())),
					}
				}
			}
			// Open Save File: open a M.A.X. saved game (`.DTA`) into a new tab -
			// resolve the world it references, load that pristine bundled stock
			// map, and embed the save's bytes (the byte-exact export anchor).
			// See SAVE-EDITOR.md S1.3. No `.DTA` is written here; Save writes a
			// project `.json` (Export Save File, a later stage, writes `.DTA`).
			Command::OpenSave { path } => {
				let bytes = match std::fs::read(&path) {
					Ok(b) => b,
					Err(e) => return Outcome::Failed(format!("open-save {}: {e}", path.display())),
				};
				// The header names the world by index (V70) or by content hash
				// resolved to a stock index (V71). The stored hash is the *slot's*
				// stock hash (max-port writes `TranslateWorldIndexToHashKey`), so it
				// identifies the slot but not swapped/custom content; a hash matching
				// no stock slot yields `None`.
				let header = match max_assets::save::read_save_header(&path) {
					Ok(h) => h,
					// A version we don't recognize at all (an old save, or not a save
					// file) → the same "incompatible file" dialog, not a bare fail.
					Err(max_assets::save::SaveError::UnsupportedVersion(_)) => {
						return Outcome::OpenDialog(DialogRequest::OpenSaveError {
							message: Self::incompatible_save_message(&path),
						});
					}
					Err(e) => return Outcome::Failed(format!("open-save {}: {e}", path.display())),
				};
				// The editor can only safely round-trip V71 (the format M.A.X. Port
				// v0.7.X writes). A V70 stock DOS save decodes, but editing it here
				// would not round-trip — refuse it with an explanation.
				if header.format != max_assets::save::SaveFormat::V71 {
					return Outcome::OpenDialog(DialogRequest::OpenSaveError {
						message: Self::incompatible_save_message(&path),
					});
				}
				let Some(world_index) = header.world_index else {
					// No resolvable slot → no map to load it onto. Abort-only notice.
					return Outcome::OpenDialog(DialogRequest::OpenSaveError {
						message: format!(
							"Can't open {}:\nit references a world with no matching stock slot (a fully custom map), \
							 so the editor can't tell which map slot it used.",
							path.display()
						),
					});
				};
				let world_file = max_assets::save::world_file_name(world_index)
					.expect("a header world_index is a valid stock index");
				let world_name = world_file.strip_suffix(".WRL").unwrap_or(world_file).to_string();
				// Preferred source: the map **installed at this slot** (`MaxPath/<file>`)
				// — the actual map the save references, which may have been swapped for a
				// custom map of different dimensions. It carries the save's true size, so
				// the body decodes. Fall back to the bundled pristine stock world.
				let installed = self.max_path.as_ref().map(|mp| mp.join(world_file)).filter(|p| p.is_file());
				if let Some(inst) = &installed {
					match Self::open_save_on_wrl(inst, &world_name, &bytes) {
						// The installed map decoded the save → open it directly.
						Ok(project) => self.commit_save_open(project, &header, &world_name),
						Err(installed_err) => {
							// The installed map didn't fit (usually its dimensions differ
							// from the save's). Try the pristine stock world: if that fits,
							// offer to open on it (dims match → Open Anyway); else the save
							// fits neither map → a real dimension mismatch (Abort only).
							match self.open_save_on_stock(&world_name, &bytes) {
								Ok(mut project) => {
									let stock_dims = (project.width, project.height);
									let inst_dims =
										read_wrl_header(inst).map(|h| (h.width, h.height)).unwrap_or((0, 0));
									let summary = Self::name_save_project(&mut project, &header, &world_name);
									self.pending_save_open = Some(PendingSaveOpen { project, summary });
									Outcome::OpenDialog(DialogRequest::ConfirmOpenSave {
										message: format!(
											"The {world_name} map installed in your M.A.X. folder is {}x{}, which \
											 doesn't fit this save.\nThe original {world_name} ({}x{}) does - its \
											 dimensions match.\n\nOpen the save on the original {world_name}?",
											inst_dims.0, inst_dims.1, stock_dims.0, stock_dims.1,
										),
									})
								}
								Err(_stock_err) => Outcome::OpenDialog(DialogRequest::OpenSaveError {
									message: format!(
										"Can't open {}:\nit was made on a {world_name} map whose dimensions match \
											 neither the installed map nor the original {world_name}.\n\n({installed_err})",
										path.display()
									),
								}),
							}
						}
					}
				} else {
					// No installed map (MaxPath unset, or the slot's file is missing).
					// A normal save on the unmodified stock world still opens with no
					// MaxPath. If even the stock world doesn't fit, the save is on a
					// swapped map we can't reach without MaxPath.
					match self.open_save_on_stock(&world_name, &bytes) {
						Ok(project) => self.commit_save_open(project, &header, &world_name),
						// The save doesn't fit the stock world. If MaxPath is unset we
						// can't reach the installed (swapped) map — prompt for it. If it
						// IS set, the slot's file is just missing/wrong → a plain notice.
						Err(_stock_err) if self.max_path.is_none() => self.prompt_paths(&format!(
							"This save was made on a modified {world_name}, which the editor loads from your \
							 M.A.X. folder."
						)),
						Err(_stock_err) => Outcome::OpenDialog(DialogRequest::OpenSaveError {
							message: format!(
								"Can't open {}:\nit was made on a modified {world_name}, but the editor couldn't \
								 load {world_file} from your M.A.X. folder.",
								path.display()
							),
						}),
					}
				}
			}
			// "Open Anyway" from the save-open confirm dialog: commit the project we
			// already built on the fallback (stock) world.
			Command::OpenSaveAnyway => {
				let Some(pending) = self.pending_save_open.take() else {
					return Outcome::Ok;
				};
				eprintln!("{}", pending.summary);
				self.console.push_line(pending.summary);
				self.add_doc(pending.project, None, None)
			}
			// New from Image: read only the PNG header (dimensions)
			// and open the settings modal - pixels are decoded later, at Convert.
			Command::NewFromImage { path } => {
				let (w, h) = match png_dimensions(&path) {
					Ok(v) => v,
					Err(e) => return Outcome::Failed(format!("new-from-image {}: {e}", path.display())),
				};
				let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into());
				// Seed the settings from the source dimensions (fit); the dialog
				// composes the widgets, the run's opts are set from them on Convert.
				let opts = map_core::ConvertOpts::fit_source(w, h);
				self.open_newimage(path, name, opts);
				self.menu().close();
				Outcome::OpenDialog(DialogRequest::NewFromImage)
			}
			// Import WRL: read the header and open the pack picker that matches
			// the WRL's tiles against existing tilesets (the heavy match runs on
			// the modal's Import press, in `wrl_match`).
			Command::ImportWrl { path } => {
				let header = match read_wrl_header(&path) {
					Ok(h) => h,
					Err(e) => return Outcome::Failed(format!("import-wrl {}: {e}", path.display())),
				};
				let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "map".into());
				self.wrlimport = Some(WrlImportRun {
					path,
					name,
					info: (header.width, header.height, header.tile_count),
					result: None,
					rows: Vec::new(),
					matched: 0,
					used: 0,
				});
				self.menu().close();
				Outcome::OpenDialog(DialogRequest::ImportWrl)
			}
			// Help ▸ Go to Website / Project GitHub - hand the URL to the OS browser.
			Command::OpenUrl { url } => {
				self.menu().close();
				// The menu only ever passes the two https links, but `open-url`
				// is a script command, so the argument is untrusted. Anything
				// else is a local path, and handing an arbitrary path to
				// `xdg-open` / `open` runs whatever handler the desktop has
				// registered for it - a `.desktop` file is an executable.
				if !(url.starts_with("https://") || url.starts_with("http://")) {
					return Outcome::Failed(format!("open-url: only http(s) URLs, got '{url}'"));
				}
				match crate::browser::open(&url) {
					Ok(()) => Outcome::Ok,
					Err(e) => Outcome::Failed(e),
				}
			}
			// Help ▸ User Manual - open the bundled HTML manual in the browser.
			Command::HelpManual => {
				self.menu().close();
				let manual = self.resources_root.join("manual/index.html");
				if !manual.is_file() {
					return Outcome::Failed(format!(
						"user manual not found at {} - run tools/build-manual.mjs to generate it",
						manual.display()
					));
				}
				match crate::browser::open(&manual.to_string_lossy()) {
					Ok(()) => Outcome::Ok,
					Err(e) => Outcome::Failed(e),
				}
			}
			// Help ▸ About - the credits / version dialog (wgpu-ui overlay,
			// shell-routed via `App::act_on`; dropped in headless runs).
			Command::About => Outcome::OpenDialog(DialogRequest::About),
			// Run the open image modal's conversion to completion synchronously
			// (scripts / headless). The interactive button uses the stepped path.
			Command::Convert => {
				let Some(m) = self.newimage.as_ref() else {
					return Outcome::Failed("convert: no image to convert (open File > New from Image)".into());
				};
				let (name, path, opts) = (m.name.clone(), m.path.clone(), m.opts);
				let result = build_convert_session(&path, opts).and_then(|mut s| {
					while !s.is_done() {
						s.step(usize::MAX);
					}
					s.finish()
				});
				match result {
					Ok(wrl) => {
						let project = Project::from_wrl(&wrl, &name);
						eprintln!(
							"imported image: {}x{} cells, {} tiles",
							project.width, project.height, wrl.tile_count
						);
						self.newimage = None;
						self.add_doc(project, None, None)
					}
					Err(e) => Outcome::Failed(format!("convert: {e}")),
				}
			}
			Command::Save { path } => {
				let Some(target) = path.or_else(|| self.path.clone()) else {
					return Outcome::Failed("save: no path (use `save PATH`)".into());
				};
				if target.extension().is_none_or(|e| e != "json") {
					return Outcome::Failed(format!(
						"save: a project saves as .json (got {}) - `export` writes the baked WRL",
						target.display(),
					));
				}
				match self.write_project(&target) {
					Ok(()) => {
						eprintln!("saved {}", target.display());
						self.project.mark_saved();
						self.path = Some(target);
						Outcome::Ok
					}
					Err(e) => Outcome::Failed(format!("save {}: {e}", target.display())),
				}
			}
			Command::SaveProject => {
				// Re-save to the current path, or open Save-As if never saved.
				if self.path.is_some() {
					self.execute(Command::Save { path: None })
				} else {
					self.execute(Command::FileDialog { purpose: FilePurpose::SaveAs })
				}
			}
			Command::Tab { index } => self.switch_to(index),
			Command::CloseProject { force } => {
				// A dirty tab prompts Save/Discard/Cancel instead of refusing;
				// a clean tab closes outright. Closing the last
				// project is allowed - it resets to a blank scratch.
				if !force && self.project.dirty() {
					Outcome::OpenDialog(DialogRequest::ConfirmClose {
						quit: false,
						prompt: format!("\"{}\" has unsaved changes.", self.name_at(self.tabs.active)),
					})
				} else {
					self.close_active(force)
				}
			}
			Command::SaveAndClose => {
				// Save the active tab, then close it - but only once it's clean.
				// A never-saved project routes to Save-As and stays open (the
				// user finishes the save, then closes).
				if self.path.is_some() {
					match self.execute(Command::Save { path: None }) {
						Outcome::Ok | Outcome::Redraw => self.close_active(true),
						other => other,
					}
				} else {
					self.execute(Command::FileDialog { purpose: FilePurpose::SaveAs })
				}
			}
			Command::QuitRequest => {
				// GUI quit: clean exits straight away; unsaved work raises the
				// Save/Discard/Cancel guard instead of losing it.
				if self.any_dirty() {
					Outcome::OpenDialog(DialogRequest::ConfirmClose { quit: true, prompt: self.dirty_summary() })
				} else {
					Outcome::Quit
				}
			}
			Command::SaveAndQuit => {
				// Save unsaved tabs one at a time (re-prompting after each), then
				// quit once everything is clean. A never-saved tab routes to
				// Save-As and stays open - the user finishes, then quits again.
				if !self.any_dirty() {
					return Outcome::Quit;
				}
				// Target the active tab if it's dirty, else the first dirty tab.
				if !self.project.dirty() {
					if let Some(i) = (0..self.tabs.slots.len()).find(|&i| self.dirty_at(i)) {
						self.switch_to(i);
					}
				}
				if self.path.is_none() {
					return self.execute(Command::FileDialog { purpose: FilePurpose::SaveAs });
				}
				match self.execute(Command::Save { path: None }) {
					Outcome::Ok | Outcome::Redraw => {
						if self.any_dirty() {
							// More unsaved tabs - show the guard again for the next.
							Outcome::OpenDialog(DialogRequest::ConfirmClose {
								quit: true,
								prompt: self.dirty_summary(),
							})
						} else {
							Outcome::Quit
						}
					}
					other => other,
				}
			}
			Command::SaveCopy { path } => {
				if path.extension().is_none_or(|e| e != "json") {
					return Outcome::Failed(format!("save-copy: a copy saves as .json (got {})", path.display()));
				}
				// A copy: the current path and dirty flag stay untouched.
				match self.write_project(&path) {
					Ok(()) => {
						let line = format!("saved copy {}", path.display());
						eprintln!("{line}");
						self.console.push_line(line);
						Outcome::Ok
					}
					Err(e) => Outcome::Failed(format!("save-copy {}: {e}", path.display())),
				}
			}
			Command::FileDialog { purpose } => {
				let user_templates = self.user_templates_dir();
				let start = dialog_default_dir(
					purpose,
					&self.resources_root,
					self.path.as_deref(),
					self.max_path.as_deref(),
					self.max_port_path.as_deref(),
					Some(user_templates.as_path()),
				);
				let suggested = dialog_suggested_name(purpose, self.path.as_deref(), &self.project.name);
				self.menu().close();

				if self.headless {
					return Outcome::Failed("file-dialog: not available in headless runs".into());
				}

				// Export to WRL and Save File (experimental combo): pick the WRL output and
				// export it, then run the Export Save File flow — both game files from one
				// menu click. The save half reuses ExportSave (which prompts for a base on a
				// normal map). Cancelling the WRL pick aborts before anything is written.
				if matches!(purpose, FilePurpose::ExportWrlAndSave) {
					let mut d = self
						.native_dialog()
						.set_directory(&start)
						.set_title("Export to WRL and Save File: choose the .WRL output")
						.add_filter("M.A.X. WRL maps", &["wrl", "WRL"]);
					if let Some(name) =
						dialog_suggested_name(FilePurpose::ExportWrl, self.path.as_deref(), &self.project.name)
					{
						d = d.set_file_name(name);
					}
					let Some(wrl_path) = d.save_file() else { return Outcome::Redraw };
					let wrl_outcome = self.execute(Command::Export { path: Some(wrl_export_path(wrl_path)) });
					if matches!(wrl_outcome, Outcome::Failed(_)) {
						return wrl_outcome;
					}
					// The save half reuses the full Export Save File dialog flow.
					return self.execute(Command::FileDialog { purpose: FilePurpose::ExportSave });
				}
				// Export Save File on a normal map (no save attached): there is nothing to
				// reconstitute, so build a `.DTA` from a user-picked base save plus the
				// placed units (`export-save-onto`). Two dialogs — the base to build on,
				// then the output. With a save open it falls through to the Save-As below.
				if matches!(purpose, FilePurpose::ExportSave) && self.project.save.is_none() {
					let base = self
						.native_dialog()
						.set_directory(&start)
						.set_title("Export as save: choose a base save to build on")
						.add_filter("M.A.X. saves", &["dta", "DTA"])
						.add_filter("all files", &["*"])
						.pick_file();
					let Some(base) = base else { return Outcome::Redraw };
					let mut save = self
						.native_dialog()
						.set_directory(&start)
						.set_title("Export as save: choose where to write the .DTA")
						.add_filter("M.A.X. saves", &["dta", "DTA"]);
					if let Some(name) = &suggested {
						save = save.set_file_name(name);
					}
					let Some(out) = save.save_file() else { return Outcome::Redraw };
					return self.execute(Command::ExportSaveOnBase { base, out });
				}
				// A never-saved map prompts Map Metadata before its first save;
				// the dialog's Save resumes this Save-As (via `first_save_meta`),
				// its Cancel abandons the save.
				if matches!(purpose, FilePurpose::SaveAs)
					&& self.path.is_none()
					&& !std::mem::take(&mut self.first_save_meta)
				{
					return Outcome::OpenDialog(DialogRequest::Metadata { save_after: true });
				}
				// Tile Painter PNG export/import: a `.png` dialog whose result a
				// command line *can* carry (just a path), so handle it up front -
				// it doesn't share the `.json` plumbing below.
				match purpose {
					FilePurpose::ExportTilePng => {
						let name = self
							.tilepaint
							.as_ref()
							.map(|r| r.id_text.trim())
							.filter(|s| !s.is_empty())
							.unwrap_or("tile");
						let picked = self
							.native_dialog()
							.set_directory(&start)
							.add_filter("PNG images", &["png"])
							.set_file_name(format!("{name}.png"))
							.save_file();
						return match picked {
							None => Outcome::Redraw,
							Some(path) => self.execute(Command::TileExportPng { path }),
						};
					}
					FilePurpose::ImportTilePng => {
						let picked = self
							.native_dialog()
							.set_directory(&start)
							.add_filter("PNG images", &["png"])
							.add_filter("all files", &["*"])
							.pick_file();
						return match picked {
							None => Outcome::Redraw,
							Some(path) => self.execute(Command::TileImportPng { path }),
						};
					}
					FilePurpose::ExportTemplatePng => {
						let Some(i) = self.templates.sel else {
							return Outcome::Failed("template-export-png: no template selected".into());
						};
						let name = sanitize_filename(&self.templates.entries[i].name);
						let picked = self
							.native_dialog()
							.set_directory(&start)
							.add_filter("PNG images", &["png"])
							.set_file_name(format!("{name}.png"))
							.save_file();
						return match picked {
							None => Outcome::Redraw,
							Some(path) => self.execute(Command::TemplateExportPng { path: Some(path) }),
						};
					}
					// Scenery in and out. Import offers both a finished piece and
					// an image to author one from in the same picker: the user
					// thinks "bring in that thing", not "which of my two import
					// verbs is this one".
					FilePurpose::ImportScenery | FilePurpose::ImportSceneryPng => {
						let d = self.native_dialog().set_directory(&start);
						let d = if purpose == FilePurpose::ImportSceneryPng {
							d.add_filter("PNG images", &["png"])
						} else {
							d.add_filter("scenery", &[map_core::SCN_EXT, "png"]).add_filter("PNG images", &["png"])
						};
						return match d.add_filter("all files", &["*"]).pick_file() {
							None => Outcome::Redraw,
							Some(path) => self.execute(Command::SceneryImport { path: Some(path) }),
						};
					}
					FilePurpose::ExportScenery => {
						let name = self.armed_scenery().map_or_else(|| "scenery".to_string(), |(_, id)| id);
						let picked = self
							.native_dialog()
							.set_directory(&start)
							.add_filter("scenery", &[map_core::SCN_EXT])
							.set_file_name(format!("{name}.{}", map_core::SCN_EXT))
							.save_file();
						return match picked {
							None => Outcome::Redraw,
							Some(path) => self.execute(Command::SceneryExport { path: Some(path) }),
						};
					}
					// A height map goes out and comes back as a plain greyscale
					// picture - the one thing every paint program can open.
					FilePurpose::ImportSceneryHeightPng => {
						let picked =
							self.native_dialog().set_directory(&start).add_filter("PNG images", &["png"]).pick_file();
						return match picked {
							None => Outcome::Redraw,
							Some(path) => self.execute(Command::SceneryHeightImport { path: Some(path) }),
						};
					}
					FilePurpose::ExportSceneryHeightPng => {
						let name = self
							.scenerypaint
							.as_ref()
							.map(|r| r.id_text.clone())
							.filter(|id| !id.is_empty())
							.unwrap_or_else(|| "scenery".to_string());
						let picked = self
							.native_dialog()
							.set_directory(&start)
							.add_filter("PNG images", &["png"])
							.set_file_name(format!("{name}.height.png"))
							.save_file();
						return match picked {
							None => Outcome::Redraw,
							Some(path) => self.execute(Command::SceneryHeightExport { path: Some(path) }),
						};
					}
					_ => {}
				}
				// Native dialog (rfd): blocks the event loop, which is fine -
				// the dialog is modal by nature. Cancel is a quiet no-op.
				let dialog = self.native_dialog().set_directory(&start);
				let picked = match purpose {
					FilePurpose::Load => dialog
						.add_filter("M.A.X. maps", &["json", "wrl", "WRL"])
						.add_filter("all files", &["*"])
						.pick_file(),
					FilePurpose::NewFromImage | FilePurpose::NewMapShape => {
						dialog.add_filter("PNG images", &["png"]).add_filter("all files", &["*"]).pick_file()
					}
					FilePurpose::ImportWrl => dialog
						.add_filter("M.A.X. WRL maps", &["wrl", "WRL"])
						.add_filter("all files", &["*"])
						.pick_file(),
					FilePurpose::OpenSave => dialog
						.add_filter(
							"M.A.X. saves",
							&["dta", "DTA", "cam", "CAM", "sce", "SCE", "tra", "TRA", "mps", "MPS", "dmo", "DMO"],
						)
						.add_filter("all files", &["*"])
						.pick_file(),
					FilePurpose::ExportWrl => {
						let mut d = dialog.add_filter("M.A.X. WRL maps", &["wrl", "WRL"]);
						if let Some(name) = &suggested {
							d = d.set_file_name(name);
						}
						d.save_file()
					}
					FilePurpose::ExportSave => {
						let mut d = dialog.add_filter("M.A.X. saves", &["dta", "DTA"]).add_filter("all files", &["*"]);
						if let Some(name) = &suggested {
							d = d.set_file_name(name);
						}
						d.save_file()
					}
					FilePurpose::LoadPalette | FilePurpose::ImportPalette => {
						dialog.add_filter("palettes", &["json"]).add_filter("all files", &["*"]).pick_file()
					}
					FilePurpose::ExportPalette => {
						let mut d = dialog.add_filter("palettes", &["json"]);
						if let Some(name) = &suggested {
							d = d.set_file_name(name);
						}
						d.save_file()
					}
					FilePurpose::SaveAs | FilePurpose::SaveCopy => {
						let mut d = dialog.add_filter("map projects", &["json"]);
						if let Some(name) = &suggested {
							d = d.set_file_name(name);
						}
						d.save_file()
					}
					FilePurpose::SavePalette => {
						let mut d = dialog.add_filter("palettes", &["json"]);
						if let Some(name) = &suggested {
							d = d.set_file_name(name);
						}
						d.save_file()
					}
					FilePurpose::ImportTemplate => {
						dialog.add_filter("templates", &["json"]).add_filter("all files", &["*"]).pick_file()
					}
					FilePurpose::ExportTemplate => {
						dialog.add_filter("templates", &["json"]).set_file_name("template.json").save_file()
					}
					FilePurpose::ExportTilePng
					| FilePurpose::ImportTilePng
					| FilePurpose::ExportTemplatePng
					| FilePurpose::ImportScenery
					| FilePurpose::ExportScenery
					| FilePurpose::ImportSceneryPng
					| FilePurpose::ImportSceneryHeightPng
					| FilePurpose::ExportSceneryHeightPng => {
						unreachable!("handled before the json dialog")
					}
					FilePurpose::ExportWrlAndSave => unreachable!("handled before the json dialog"),
				};
				match picked {
					None => Outcome::Redraw, // canceled
					Some(path) => match purpose {
						FilePurpose::Load => self.execute(Command::Open { path }),
						FilePurpose::SaveAs => self.execute(Command::Save { path: Some(path) }),
						FilePurpose::SaveCopy => self.execute(Command::SaveCopy { path }),
						FilePurpose::LoadPalette => self.execute(Command::PaletteLoad { path }),
						FilePurpose::SavePalette | FilePurpose::ExportPalette => {
							self.execute(Command::PaletteSave { path })
						}
						FilePurpose::ImportPalette => self.execute(Command::PaletteImport { path }),
						FilePurpose::NewFromImage => self.execute(Command::NewFromImage { path }),
						// File → New Terrain from Image: the picked PNG opens the
						// New Map form with the shape armed for the Create carve.
						FilePurpose::NewMapShape => Outcome::OpenDialog(DialogRequest::NewMap { shape: Some(path) }),
						FilePurpose::ImportWrl => self.execute(Command::ImportWrl { path }),
						FilePurpose::OpenSave => self.execute(Command::OpenSave { path }),
						FilePurpose::ExportSave => self.execute(Command::ExportSave { path }),
						FilePurpose::ExportWrl => self.execute(Command::Export { path: Some(wrl_export_path(path)) }),
						FilePurpose::ImportTemplate => self.execute(Command::TemplateImport { path }),
						FilePurpose::ExportTemplate => self.execute(Command::TemplateExport { path }),
						FilePurpose::ExportTilePng
						| FilePurpose::ImportTilePng
						| FilePurpose::ExportTemplatePng
						| FilePurpose::ImportScenery
						| FilePurpose::ExportScenery
						| FilePurpose::ImportSceneryPng
						| FilePurpose::ImportSceneryHeightPng
						| FilePurpose::ExportSceneryHeightPng => {
							unreachable!("handled before the json dialog")
						}
						FilePurpose::ExportWrlAndSave => unreachable!("handled before the json dialog"),
					},
				}
			}
			Command::Resize { width, height, off_x, off_y } => {
				let project = &mut self.project;
				match project.resize(width, height, off_x, off_y) {
					Ok(()) => {
						self.view = self.fit_center((width, height));
						let line = format!("resized to {width}x{height} (offset {off_x},{off_y})");
						eprintln!("{line}");
						self.console.push_line(line);
						// Dimensions changed - the renderer's textures rebuild.
						Outcome::DocReplaced
					}
					Err(e) => Outcome::Failed(format!("resize: {e}")),
				}
			}
			// Opens the wgpu-ui Resize overlay, routed by the shell (`App::run`);
			// a no-op in headless/script runs (no overlay to show).
			Command::ResizeModal => Outcome::OpenDialog(DialogRequest::Resize),
			Command::AutoFixModal { autostart } => {
				// Seed the run with the faithful (match.json) defect count and
				// stash the broken cells so they outline in red right away.
				self.open_autofix();
				self.menu().close();
				// `fix-shore-modal go` opens the window and starts the run right away
				// (live progress, Stop/Abort), stepping across frames.
				if autostart {
					self.autofix_start();
				}
				Outcome::OpenDialog(DialogRequest::AutoFix)
			}
			Command::GenerateModal => {
				self.open_generate();
				self.menu().close();
				Outcome::OpenDialog(DialogRequest::Generate)
			}
			Command::Export { path } => {
				let project = &self.project;
				// Default: the project's path with the extension swapped.
				let Some(target) = path.or_else(|| self.path.as_ref().map(|p| p.with_extension("wrl"))) else {
					return Outcome::Failed("export: no path (use `export PATH.wrl`)".into());
				};
				match map_core::bake(project).and_then(|wrl| {
					write_wrl_file(&wrl, &target).map_err(|e| e.to_string())?;
					// The Map Metadata rides along as a JSON tail after the
					// binary payload - the game (and `read_wrl_file`) reads the
					// WRL by its structured field sizes and ignores it.
					if let Some(meta) = project.info_json() {
						use std::io::Write;
						std::fs::OpenOptions::new()
							.append(true)
							.open(&target)
							.and_then(|mut f| writeln!(f, "\n{meta}"))
							.map_err(|e| format!("metadata tail: {e}"))?;
					}
					Ok(wrl.tile_count)
				}) {
					Ok(tile_count) => {
						let line = format!(
							"exported {} ({tile_count} baked tiles, budget {})",
							target.display(),
							map_core::MAX_BAKED_TILES,
						);
						eprintln!("{line}");
						self.console.push_line(line);
						// Free-placed scenery mints a unique composed tile per cell it
						// touches, so a heavily dressed map can walk into the u16 tile
						// ceiling - where the *next* export fails outright, with no
						// warning that it was close. Say so while there is still room
						// to act (SCENERY.md stage E).
						if tile_count as usize * 100 >= map_core::MAX_BAKED_TILES * BUDGET_WARN_PERCENT {
							let warn = format!(
								"warning: {}% of the {}-tile budget is used - scenery mints a tile per cell it covers",
								tile_count as usize * 100 / map_core::MAX_BAKED_TILES,
								map_core::MAX_BAKED_TILES,
							);
							eprintln!("{warn}");
							self.console.push_line(warn);
						}
						Outcome::Redraw
					}
					Err(e) => Outcome::Failed(format!("export {}: {e}", target.display())),
				}
			}
			// Export Save File (S6.1): reconstitute the opened `.DTA` with the
			// editor's scalar edits + resource map and write it to `path`, rotating a
			// backup first when overwriting (S6.5). The guided slot/overwrite modal is
			// deferred; scriptable via `export-save PATH`.
			Command::NewSave { name, world } => {
				// The menu form passes no name — default to the project's.
				let name = if name.is_empty() { self.project.name.clone() } else { name };
				let Some(db) = self.unit_stats.clone() else {
					return Outcome::Failed(
						"new-save: unit stats unavailable - set the M.A.X. Port data folder (Editor Preferences)"
							.into(),
					);
				};
				let Some(max_path) = self.max_path.clone() else {
					return Outcome::Failed("new-save: MaxPath not set (MAX.RES supplies unit frame data)".into());
				};
				let Some(max_res) = crate::units::find_max_res(&max_path) else {
					return Outcome::Failed(format!("new-save: MAX.RES not found in {}", max_path.display()));
				};
				// Stock slot the save claims (swapped-WRL workflow): explicit
				// world arg (SNOW_1…DESERT_6), else GREEN_1.
				let world_index = match &world {
					Some(w) => {
						let file = format!("{}.WRL", w.to_ascii_uppercase());
						match max_assets::save::WORLD_FILE_NAMES.iter().position(|&f| f == file) {
							Some(i) => i as u8,
							None => {
								return Outcome::Failed(format!(
									"new-save: unknown world '{w}' (use SNOW_1..DESERT_6)"
								));
							}
						}
					}
					None => 12, // GREEN_1
				};
				// Loadability check (swapped-WRL workflow): on load the game opens
				// the slot's INSTALLED map file and sizes the whole save stream by
				// its dimensions - the stored hash is a fixed per-slot string and
				// verifies nothing. If the installed map's dims differ from this
				// project's, the exported save desyncs mid-parse in the game; warn
				// now, at the moment the slot is chosen.
				let slot_warning = max_assets::save::world_file_name(world_index).and_then(|world_file| {
					let installed = max_path.join(world_file);
					match max_assets::wrl::read_wrl_header(&installed) {
						Ok(h) if (h.width, h.height) == (self.project.width, self.project.height) => None,
						Ok(h) => Some(format!(
							"warning: the game will size this save by the installed {world_file} ({}x{}), \
							 but this map is {}x{} - the save will NOT load until you install this map as {}",
							h.width,
							h.height,
							self.project.width,
							self.project.height,
							installed.display(),
						)),
						Err(_) => Some(format!(
							"warning: no readable map installed at {} - the game sizes the save by that \
							 file; install this map there (slot {}) before loading the save",
							installed.display(),
							world_file.trim_end_matches(".WRL"),
						)),
					}
				});
				let frames = max_assets::attribs::load_frame_infos(&max_res, &db.meta);
				let seed = std::time::SystemTime::now()
					.duration_since(std::time::UNIX_EPOCH)
					.map(|d| d.subsec_nanos() ^ d.as_secs() as u32)
					.unwrap_or(0x4d41585f); // "MAX_"
				let opts = map_core::SynthesizeSaveOptions {
					save_name: name.clone(),
					world_index,
					team_clans: [1, 2, 3, 4, 0],
					start_gold: 150,
					rng_seed: seed,
				};
				match self.project.synthesize_save(&opts, &db, &frames) {
					Ok(summary) => {
						let world_name = max_assets::save::world_file_name(world_index)
							.unwrap_or("?")
							.trim_end_matches(".WRL")
							.to_string();
						self.console.push_line(format!(
							"synthesized save \"{name}\" ({} units, {} player team{}, slot {world_name}, {} bytes) - attached; File \u{25b8} Export Save File writes the .DTA",
							summary.units,
							summary.teams,
							if summary.teams == 1 { "" } else { "s" },
							summary.bytes,
						));
						if let Some(warn) = slot_warning {
							eprintln!("{warn}");
							self.console.push_line(warn);
						}
						Outcome::Redraw
					}
					Err(e) => Outcome::Failed(format!("new-save: {e}")),
				}
			}
			Command::ExportSave { path } => {
				if self.project.save.is_none() {
					return Outcome::Failed("export-save: no save open (open a `.DTA` first)".into());
				}
				// Count corrupt-runtime-state units before export; `export_save` repairs
				// them (`save-editor-bug.md`), and we note how many below.
				let repaired = self.project.save_integrity_issues().len();
				// With the unit database + MAX.RES at hand, a placement whose type
				// the save lacks exports as a fresh from-scratch body instead of
				// being dropped (V71 saves).
				let frames = self.fresh_body_frames();
				let ctx = self
					.unit_stats
					.as_ref()
					.zip(frames.as_ref())
					.map(|(db, frames)| max_assets::save::FreshBodyCtx { db, frames });
				let (bytes, dropped) = match self.project.export_save_with(ctx.as_ref()) {
					Ok(v) => v,
					Err(e) => return Outcome::Failed(format!("export-save {}: {e}", path.display())),
				};
				// Never overwrite without a backup (S6.5): rotate the prior file to
				// `NAME.bak1`, shifting older backups up and dropping the 6th.
				let backed_up = match rotate_backups(&path, SAVE_BACKUP_KEEP) {
					Ok(b) => b,
					Err(e) => return Outcome::Failed(format!("export-save {}: backup failed: {e}", path.display())),
				};
				if let Err(e) = std::fs::write(&path, &bytes) {
					return Outcome::Failed(format!("export-save {}: {e}", path.display()));
				}
				// Surface any edits the export could not reflect so they aren't
				// silently dropped (house rule: no silent caps).
				let mut line = format!("exported save {} ({} bytes)", path.display(), bytes.len());
				if backed_up {
					line.push_str(&format!(" - prior version kept as {}.bak1", path.display()));
				}
				if !dropped.is_empty() {
					line.push_str(&format!(
						" - WARNING: {} placed unit(s) NOT exported (no same-type template in the save, and \
						 no unit database / MAX.RES to synthesize a body): {}",
						dropped.len(),
						dropped.join(", "),
					));
				}
				if repaired > 0 {
					line.push_str(&format!(" - repaired {repaired} unit(s) with corrupt runtime state"));
				}
				eprintln!("{line}");
				self.console.push_line(line);
				Outcome::Redraw
			}
			// Save a normal map (no attached save) as a `.DTA` by adding its placed
			// units onto a user-picked base save. The base carries the game-state
			// skeleton + terrain; placed types with no template in it are skipped.
			Command::ExportSaveOnBase { base, out } => {
				let base_raw = match std::fs::read(&base) {
					Ok(b) => b,
					Err(e) => {
						return Outcome::Failed(format!("export-save-onto: reading base {}: {e}", base.display()));
					}
				};
				// Same fresh-body context as `export-save`: a type absent from the
				// base synthesizes a from-scratch body when the runtime data exists.
				let frames = self.fresh_body_frames();
				let ctx = self
					.unit_stats
					.as_ref()
					.zip(frames.as_ref())
					.map(|(db, frames)| max_assets::save::FreshBodyCtx { db, frames });
				let (bytes, skipped) = match self.project.export_onto_base(&base_raw, ctx.as_ref()) {
					Ok(r) => r,
					Err(e) => return Outcome::Failed(format!("export-save-onto {}: {e}", out.display())),
				};
				// Never overwrite without a backup (S6.5), same as `export-save`.
				let backed_up = match rotate_backups(&out, SAVE_BACKUP_KEEP) {
					Ok(b) => b,
					Err(e) => {
						return Outcome::Failed(format!("export-save-onto {}: backup failed: {e}", out.display()));
					}
				};
				if let Err(e) = std::fs::write(&out, &bytes) {
					return Outcome::Failed(format!("export-save-onto {}: {e}", out.display()));
				}
				let added = self.project.objects.len().saturating_sub(skipped.len());
				let mut line = format!(
					"exported save {} ({} bytes) - {added} unit(s) added onto {}",
					out.display(),
					bytes.len(),
					base.display(),
				);
				if backed_up {
					line.push_str(&format!(" - prior version kept as {}.bak1", out.display()));
				}
				if !skipped.is_empty() {
					line.push_str(&format!(
						" - WARNING: {} placed unit(s) NOT exported (no same-type template in the base, and \
						 no unit database / MAX.RES to synthesize a body): {}",
						skipped.len(),
						skipped.join(", "),
					));
				}
				eprintln!("{line}");
				self.console.push_line(line);
				Outcome::Redraw
			}
			Command::ConvertPaletteModal => {
				if !self.project.is_wrl_import() {
					return Outcome::Failed(
						"convert-palette: only an opened WRL has an internal palette to convert".into(),
					);
				}
				self.menu().close();
				Outcome::OpenDialog(DialogRequest::ConvertPalette)
			}
			// Opens the wgpu-ui Map Metadata overlay, routed by the shell
			// (`App::run`); a no-op in headless/script runs (no overlay to show).
			Command::MetadataModal => Outcome::OpenDialog(DialogRequest::Metadata { save_after: false }),
			Command::PreferencesModal => {
				// A manual open (menu / Attention ▸ Continue) is never "required".
				self.paths_prompt_reason = None;
				Outcome::OpenDialog(DialogRequest::EditorPreferences)
			}
			// The Open Save File menu item / keybinding land here first: warn that
			// the save editor is experimental before the file picker. Confirming
			// the dialog runs `file-dialog open-save` (see the shell's dialog map).
			Command::OpenSaveWarn => {
				self.menu().close();
				Outcome::OpenDialog(DialogRequest::ConfirmExperimentalOpenSave)
			}
			Command::EditSaveData => {
				self.menu().close();
				self.open_edit_save_data()
			}
			Command::TilePaintNew => {
				self.menu().close();
				self.open_tile_new()
			}
			Command::TilePaintClone => {
				self.menu().close();
				self.open_tile_clone()
			}
			Command::TilePaintEdit => {
				self.menu().close();
				self.open_tile_edit()
			}
			// The script path commits with the run's defaults (a script can't
			// type into the dialog); the interactive path passes the widgets'
			// values through the shell instead.
			Command::TileCommit => match self.tilepaint.as_ref() {
				Some(run) => {
					let (typed, pass, pack) = (run.id_text.clone(), run.pass, run.target_pack().to_string());
					self.tile_paint_commit(typed, pass, pack)
				}
				None => Outcome::Ok,
			},
			Command::TileDelete => self.delete_active_tile(),
			Command::TileExportPng { path } => self.tile_export_png(&path),
			Command::TileImportPng { path } => self.tile_import_png(&path),
			Command::SceneryNew => {
				self.menu().close();
				self.open_scenery_new()
			}
			Command::SceneryClone => {
				self.menu().close();
				self.open_scenery_from_armed(crate::scenerypaint::Mode::Clone)
			}
			Command::SceneryEdit => {
				self.menu().close();
				self.open_scenery_from_armed(crate::scenerypaint::Mode::Edit)
			}
			Command::SceneryImport { path } => match path {
				Some(path) => self.scenery_import(&path),
				None => self.execute(Command::FileDialog { purpose: FilePurpose::ImportScenery }),
			},
			Command::SceneryExport { path } => match path {
				Some(path) => self.scenery_export(&path),
				None => self.execute(Command::FileDialog { purpose: FilePurpose::ExportScenery }),
			},
			// The script path commits with the run's defaults (a script cannot
			// type into the dialog); the interactive path passes the widgets'
			// values through the shell instead - the Tile Painter's split.
			Command::SceneryCommit => match self.scenerypaint.as_ref() {
				Some(run) => {
					let (pack, id, name) = (run.target_pack().to_string(), run.id_text.clone(), run.name_text.clone());
					// The art is whatever the run holds: a rasterized image, or the
					// piece a Clone/Edit opened on.
					let carried = run.piece.as_ref().and_then(|p| p.height.clone());
					let derived = match (run.uses_image(), run.piece.clone()) {
						(false, Some(p)) => Some((p.sprite, p.pass, p.cells_w, p.cells_h)),
						_ => self.rasterize_scenery_run(None),
					};
					match derived {
						Some((sprite, pass, w, h)) => {
							// A height map imported by script wins over one the source
							// piece carried, exactly as an imported image wins over its
							// art; `scenery_commit` drops either if it does not fit.
							let height = self.fit_scenery_height(&sprite, (w, h), None).or(carried);
							self.scenery_commit(pack, id, name, sprite, pass, (w, h), None, height)
						}
						None => Outcome::Failed("scenery-commit: nothing to commit - import an image first".into()),
					}
				}
				None => Outcome::Ok,
			},
			Command::SceneryHeightImport { path } => match path {
				Some(path) => self.scenery_height_import(&path),
				None => self.execute(Command::FileDialog { purpose: FilePurpose::ImportSceneryHeightPng }),
			},
			Command::SceneryHeightExport { path } => match path {
				Some(path) => self.scenery_height_export(&path),
				None => self.execute(Command::FileDialog { purpose: FilePurpose::ExportSceneryHeightPng }),
			},
			Command::SceneryDelete { force } => self.scenery_delete(force),
			Command::SceneryRename { name } => self.scenery_rename(name),
			Command::Bake => {
				self.menu().close();
				self.bake()
			}
			Command::MatchEditor => {
				self.menu().close();
				self.open_match_editor()
			}
			Command::UiTests => {
				self.menu().close();
				if !self.dev_mode {
					return Outcome::Failed("ui-tests: requires --dev".into());
				}
				Outcome::OpenDialog(DialogRequest::UiTests)
			}
			Command::MatchCombos { pack } => {
				self.menu().close();
				if !self.dev_mode {
					return Outcome::Failed("match-combos: requires --dev".into());
				}
				// The named pack, else the active map's palette-owning tileset.
				let Some(pack) = pack.or_else(|| self.project.uses.iter().find(|u| u.palette).map(|u| u.name.clone()))
				else {
					return Outcome::Failed("match-combos: no tileset (open a map or name a pack)".into());
				};
				match map_core::match_combos_map(&pack, &self.assets_root, roll_seed()) {
					Ok(project) => {
						let line = format!("match combos for {pack}: {}x{} map", project.width, project.height);
						eprintln!("{line}");
						self.console.push_line(line);
						self.add_doc(project, None, None)
					}
					Err(e) => Outcome::Failed(format!("match-combos: {e}")),
				}
			}
			Command::UpdateMap => {
				self.menu().close();
				if !self.dev_mode {
					return Outcome::Failed("update-map: requires --dev".into());
				}
				// Write back to the file this map was opened from (a shipped map
				// included), else its current save path. New / WRL / image imports
				// have no original map file - use Save / Save As instead.
				let Some(target) = self.origin.clone().or_else(|| self.path.clone()) else {
					return Outcome::Failed("update-map: this map has no original file (use Save As)".into());
				};
				match self.write_project(&target) {
					Ok(()) => {
						self.project.mark_saved();
						let line = format!("updated map {}", target.display());
						eprintln!("{line}");
						self.console.push_line(line);
						Outcome::Ok
					}
					Err(e) => Outcome::Failed(format!("update-map {}: {e}", target.display())),
				}
			}
			Command::ConvertPalette { rasterize, water, relaxed, threshold } => {
				// Tile pixels get rewritten - only a WRL import owns its tiles
				// (a .json project's packs are shared on disk).
				if !self.project.is_wrl_import() {
					return Outcome::Failed(
						"convert-palette: only an opened WRL has an internal palette to convert".into(),
					);
				}
				if rasterize {
					let dedupe = if relaxed { map_core::Dedupe::Relaxed } else { map_core::Dedupe::Strict };
					match self.project.convert_palette_by_reimport(water, dedupe, threshold) {
						Ok(tile_count) => {
							self.refresh_palette();
							let line = format!(
								"palette converted by re-import: {tile_count} tiles rebuilt, water {} \
								 (lossy, undoable)",
								if water { "kept animated" } else { "flattened" },
							);
							eprintln!("{line}");
							self.console.push_line(line);
							// The tile table changed - the GPU atlas must rebuild.
							Outcome::DocReplaced
						}
						Err(e) => Outcome::Failed(format!("convert-palette: {e}")),
					}
				} else {
					let opts = map_core::ConvertOptions { preserve_water: water };
					match self.project.convert_to_compatible_palette(opts) {
						None => {
							self.console.push_line("palette already MAX-compatible - nothing to convert");
							Outcome::Redraw
						}
						Some(r) => {
							self.refresh_palette();
							let line = format!(
								"palette converted: {} color(s) kept exactly, {} approximated, \
								 {} moved off animated slots (lossy, undoable)",
								r.exact, r.approximated, r.de_animated,
							);
							eprintln!("{line}");
							self.console.push_line(line);
							// Tile pixels changed - the GPU atlas must rebuild.
							Outcome::DocReplaced
						}
					}
				}
			}
			_ => unreachable!("non-io command routed to exec_io"),
		}
	}

	/// View overlays + clock + console + screenshot: grid, pass overlay,
	/// palette animation, the animation tick, the console, and capture.
	fn exec_overlay(&mut self, command: Command) -> Outcome {
		match command {
			Command::Grid { on } => {
				self.show_grid = on.unwrap_or(!self.show_grid);
				self.console.push_line(format!("grid: {}", if self.show_grid { "on" } else { "off" }));
				Outcome::Redraw
			}
			Command::StatusBar { on } => {
				self.status_bar = on.unwrap_or(!self.status_bar);
				// Reserve (or release) the bottom strip so docks reflow around it.
				self.workspace.bottom = if self.status_bar { crate::statusbar::BAR_H } else { 0.0 };
				Outcome::Redraw
			}
			Command::PassOverlay { on } => {
				self.show_pass_overlay = on.unwrap_or(!self.show_pass_overlay);
				self.console.push_line(format!("pass overlay: {}", if self.show_pass_overlay { "on" } else { "off" },));
				Outcome::Redraw
			}
			Command::Resources { on } => {
				self.show_resources = on.unwrap_or(!self.show_resources);
				self.console.push_line(format!("resource overlay: {}", if self.show_resources { "on" } else { "off" }));
				// Load the marker sprites so the overlay draws them like the game; a
				// failure (no MaxPath) is non-fatal - the overlay falls back to tint.
				if self.show_resources {
					let _ = self.ensure_markers();
				}
				Outcome::Redraw
			}
			Command::ShoreBugs { on } => {
				self.show_shore_bugs = on.unwrap_or(!self.show_shore_bugs);
				self.shore_bug_rev = u64::MAX; // force a recompute on the next frame
				self.console.push_line(format!("shore bugs: {}", if self.show_shore_bugs { "on" } else { "off" }));
				Outcome::Redraw
			}
			Command::MatchProblems { on } => {
				self.show_match_problems = on.unwrap_or(!self.show_match_problems);
				self.match_problem_rev = u64::MAX; // force a recompute on the next frame
				self.console
					.push_line(format!("match problems: {}", if self.show_match_problems { "on" } else { "off" }));
				Outcome::Redraw
			}
			Command::ShowOnlyLayer { on } => {
				self.show_only_layer = on.unwrap_or(!self.show_only_layer);
				self.console.push_line(format!(
					"show only {} layer: {}",
					self.active_layer_name(),
					if self.show_only_layer { "on" } else { "off" },
				));
				Outcome::Redraw
			}
			Command::Animate { on } => {
				self.animate = on.unwrap_or(!self.animate);
				// Static / Animated leave In-Game mode.
				self.ingame = false;
				self.cycler.set_ingame(false);
				Outcome::Redraw
			}
			Command::InGame { on } => {
				self.ingame = on.unwrap_or(!self.ingame);
				// In-Game implies the palette is cycling.
				if self.ingame {
					self.animate = true;
				}
				self.cycler.set_ingame(self.ingame);
				Outcome::Redraw
			}
			Command::Crt { on } => {
				self.crt = on.unwrap_or(!self.crt);
				Outcome::Redraw
			}
			Command::UiScale { scale } => {
				self.set_ui_scale(scale);
				self.console.push_line(format!("ui scale: {:.0}%", scale * 100.0));
				Outcome::Redraw
			}
			Command::MapPalette { on } => {
				self.debug_map_palette = on.unwrap_or(!self.debug_map_palette);
				self.refresh_palette();
				self.console.push_line(format!(
					"map palette render: {}",
					if self.debug_map_palette { "on (internal palette)" } else { "off (game palette)" },
				));
				Outcome::Redraw
			}
			Command::Tick { seconds } => {
				self.tick(seconds);
				Outcome::Redraw
			}
			Command::Console { on } => {
				let on = on.unwrap_or(!self.console.is_open());
				self.console.set_open(on);
				Outcome::Redraw
			}
			Command::Screenshot { path, crop, resize } => Outcome::Screenshot { path, crop, resize },
			_ => unreachable!("non-overlay command routed to exec_overlay"),
		}
	}

	/// Introspection + termination: hash, the test asserts, and quit.
	fn exec_assert(&mut self, command: Command) -> Outcome {
		match command {
			Command::Hash => {
				let hash = self.project.hash();
				let line = format!("hash: 0x{hash:016x}");
				println!("{line}");
				self.console.push_line(line);
				Outcome::Redraw
			}
			Command::AssertTile { x, y, tile } => {
				let actual = self.project.base_tile(x, y);
				if actual == Some(tile) {
					Outcome::Ok
				} else {
					Outcome::Failed(format!("assert-tile {x} {y}: expected {tile}, got {actual:?}",))
				}
			}
			Command::AssertHash { hash } => {
				let actual = self.project.hash();
				if actual == hash {
					Outcome::Ok
				} else {
					Outcome::Failed(format!("assert-hash: expected 0x{hash:016x}, got 0x{actual:016x}",))
				}
			}
			Command::AssertDirty { dirty } => {
				if self.dirty() == dirty {
					Outcome::Ok
				} else {
					Outcome::Failed(format!("assert-dirty: expected {dirty}, got {}", self.dirty(),))
				}
			}
			Command::Quit { force } => {
				if self.any_dirty() && !force {
					return Outcome::Failed("quit: unsaved changes - `save` first or use `quit!`".into());
				}
				Outcome::Quit
			}
			_ => unreachable!("non-assert command routed to exec_assert"),
		}
	}
}
