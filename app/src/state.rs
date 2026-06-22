//! Editor state + the single command mutator.
//!
//! `EditorState` owns the document and the viewport; `execute` is the only
//! place either is mutated. GPU-side effects (screenshot, quit) are returned
//! as `Outcome`s for the shell (windowed or headless) to act on.

use std::path::{Path, PathBuf};

use map_core::{
	LAYER_GROUND, LAYER_WATER, Project, Rng, SelectMode, Selection, Template, clear_selection, clear_selection_layer,
};
use max_assets::wrl::{read_wrl_file, read_wrl_header, write_wrl_file};

use crate::command::{Command, FilePurpose, ShoreMode};
use crate::console::Console;
use crate::menu::{self, MenuBar};
use crate::minimap;
use crate::newmap::NewMap;
use crate::palette::PaletteCycler;
use crate::picker::{self, PickerState};
use crate::render::{TILE_PX, Uniforms};
use crate::workspace::Workspace;

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

/// Decode the modal's image and build a conversion session from its settings -
/// the conversion's first stage, shared by the stepped and synchronous
/// (`convert`) paths.
fn decode_and_build(m: &crate::newfromimage::NewFromImage) -> Result<map_core::ConvertSession, String> {
	let opts = m.opts()?;
	let (rgba, w, h) = decode_png_rgba(&m.path)?;
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
	/// Stamp a unit preview at the clicked cell (Units panel - palette aid).
	Unit,
	/// Remove the unit preview on the clicked cell.
	UnitEraser,
	/// Freehand cell selection: drag paints the mask (Shift adds,
	/// Ctrl subtracts, plain drag starts fresh).
	Select,
	/// Rectangle selection: drag spans a rect, applied on release (same
	/// modifier logic).
	SelectRect,
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
}

/// Brush footprint shape, paired with the brush size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrushShape {
	Square,
	Circle,
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
	format!(
		"generate {}: seed {} - {} water / {} land, {} obstruction / {} decoration cells, {} shore tiles{}",
		p.pattern.name(),
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
	let mut lines = vec![
		format!("{}: seed {}", p.pattern.name(), p.seed),
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
	Quit,
	Failed(String),
}

/// A fresh random seed from the wall clock (nanos since the epoch), 0 if the
/// clock is before the epoch. Used wherever a generate/new map needs a seed
/// the caller didn't pin (interactive default); scripts pass one explicitly.
fn roll_seed() -> u64 {
	std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
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
/// (Load) or `resources/maps` (Save). `doc_path` is the active document's path,
/// `max_path` the configured game directory, `user_templates` the user's saved-
/// templates dir.
fn dialog_default_dir(
	purpose: FilePurpose,
	resources_root: &Path,
	doc_path: Option<&Path>,
	max_path: Option<&Path>,
	user_templates: Option<&Path>,
) -> PathBuf {
	use FilePurpose::*;
	match purpose {
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
					let maps = resources_root.join("maps");
					let _ = std::fs::create_dir_all(&maps);
					Some(maps)
				}
			};
			doc_path.and_then(Path::parent).map(Path::to_path_buf).or(fallback).unwrap_or_else(|| PathBuf::from("."))
		}
	}
}

/// The pre-filled filename for a `purpose` dialog (`.json` ensured), or `None`
/// (purposes that don't pre-fill a name). Pure policy, no rfd.
fn dialog_suggested_name(purpose: FilePurpose, doc_path: Option<&Path>, project_name: &str) -> Option<String> {
	use FilePurpose::*;
	let raw = match purpose {
		SaveAs | SaveCopy => doc_path
			.and_then(Path::file_name)
			.map(|n| n.to_string_lossy().into_owned())
			.or_else(|| Some(project_name.to_string())),
		SavePalette | ExportPalette => Some(project_name.to_string()),
		_ => None,
	};
	raw.map(|n| if n.ends_with(".json") { n } else { format!("{n}.json") })
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
			// Map name on the left, file name right-aligned (the hint column);
			// a nameless map just shows its file name as the label.
			match read_map_name(&path) {
				Some(name) => crate::menu::MapEntry { label: name, note: Some(stem), path },
				None => crate::menu::MapEntry { label: stem, note: None, path },
			}
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
	pub entries: Vec<TemplateEntry>,
	/// Explorer scroll (px, clamped at draw time).
	pub scroll: f32,
	/// The explorer's selected template (index into `entries`).
	pub sel: Option<usize>,
	/// Thumbnail size (px), chosen from the panel's size dropdown (32..128).
	pub cell: f32,
	/// The preview-size dropdown's open state.
	pub dropdown_open: bool,
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
	/// Color Palette grid scroll (px, clamped at draw time).
	pub scroll: f32,
	/// WRL Internal Palette grid scroll (px, clamped at draw time).
	pub wrl_scroll: f32,
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
	/// Live pointer snapshot (cursor + held press) for widget hover/pressed
	/// rendering - written by the shell from winit events, read by the views.
	/// Stays inert (`Hot::NONE`) in headless runs, so captures are mouse-free.
	pub hot: crate::ui::Hot,
	/// Dockable panels around the map view.
	pub workspace: Workspace,
	/// The main menu bar.
	pub menu: MenuBar,
	/// The right-click context menu, when open - items snapshot the state
	/// at open time (selection, clipboard, stamp, the cell under the click).
	pub context_menu: Option<menu::ContextMenu>,
	/// Shortcut hints from the loaded bindings: normalized command line →
	/// chord label (`"copy"` → `"Ctrl+C"`). Set once by the shell; menus and
	/// the context menu annotate their items from it.
	shortcut_hints: Vec<(String, String)>,
	/// The single open modal (at most one at a time), behind a trait object.
	/// The concrete type is recovered by downcast - see `modal_as`,
	/// `modal_as_mut`, `take_modal_as`, and the `open` constructor.
	pub modal: Option<Box<dyn crate::modal::Modal>>,
	/// Headless run (`--headless`/`--screenshot`): native dialogs can't open.
	pub headless: bool,
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
	/// Selected unit in the Units panel (index into `units`). The placed
	/// previews themselves live in `project.units` (saved with the map).
	pub active_unit: Option<usize>,
	/// Show the placed unit previews on the map (View ▸ Show Units). Auto-
	/// enables when a unit is picked or stamped.
	pub show_units: bool,
	/// Team color for new previews (0..5 - red green blue gray yellow).
	pub unit_team: u8,
	/// Units panel scroll (px, clamped at draw time).
	pub units_scroll: f32,
	/// The selected-cell mask (editor state, never in the undo journal) -
	/// the select tools edit it; copy/cut and template capture read it.
	pub selection: Selection,
	/// A live rect-select drag's preview `(x0, y0, x1, y1)` in cells - set
	/// by the shell while dragging, drawn as a dashed-intent outline.
	pub select_preview: Option<(u16, u16, u16, u16)>,
	/// The copy/cut clipboard (a transient unnamed template).
	pub clipboard: Option<Template>,
	/// The armed ghost stamp riding under the cursor (paste or a picked
	/// template); a map click places it, Esc disarms.
	pub stamp: Option<Template>,
	/// Templates Explorer state (the known templates + scroll/selection/size).
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
	/// Ground (the detail layer over the water base).
	pub active_layer: usize,
	/// Brush/eraser footprint: an odd-sided square (`1` = single cell)
	/// centred on the cursor. Drives pencil paint and the eraser.
	pub brush_size: u16,
	/// Brush footprint shape (square or circle).
	pub brush_shape: BrushShape,
	/// Editor mode: tile painting vs pass-table painting.
	pub mode: EditorMode,
	/// Active pass value for the Pass Table Editor (0..3).
	pub active_pass: u8,
	/// Selected palette slot in the Color Palette panel - the anchor of
	/// a multi-select range.
	pub active_color: Option<u8>,
	/// Color Palette + WRL-palette panel state (selection range, scrolls, saved list).
	pub palettes: PaletteManager,
	/// Toolbox scroll (px, clamped at draw time) - the toolbox flows tall and
	/// scrolls when it doesn't fit.
	pub toolbox_scroll: f32,
	/// The toolbox brush-size dropdown's open state.
	pub brush_dropdown_open: bool,
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
		let mut workspace = Workspace::default();
		// The menu bar + project tab strip reserve the top strip; the status bar
		// reserves the bottom (shown by default).
		workspace.top = menu::BAR_H + crate::tabs::BAR_H;
		workspace.bottom = crate::statusbar::BAR_H;
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
			cycler,
			animate: false,
			ingame: false,
			crt: false,
			debug_map_palette: false,
			show_grid: false,
			show_pass_overlay: false,
			show_only_layer: false,
			status_bar: true,
			ui_scale: 1.0,
			console: Console::new(),
			hot: crate::ui::Hot::NONE,
			menu: MenuBar::new(&template_maps, &[]),
			context_menu: None,
			shortcut_hints: Vec::new(),
			modal: None,
			headless: false,
			units: None,
			units_loaded: false,
			active_unit: None,
			show_units: true,
			unit_team: 0,
			units_scroll: 0.0,
			selection: Selection::new(project_w, project_h),
			select_preview: None,
			clipboard: None,
			stamp: None,
			templates: TemplateLibrary { cell: 64.0, ..Default::default() },
			recent: Vec::new(),
			// (modal: None, set above - the 15 typed modal fields collapsed to one)
			dev_mode: false,
			tile_ops: TileOps::default(),
			workspace,
			picker: PickerState::default(),
			minimap_mode: minimap::Mode::Overworld,
			// one tab; the active live fields above are its state.
			tabs: TabSet { slots: vec![None], active: 0, replace_scratch: true },
			tool: Tool::Pencil,
			active_layer: LAYER_GROUND,
			brush_size: 1,
			brush_shape: BrushShape::Square,
			randomize: false,
			paint_rng: Rng::new(0x004d_4158_5f56_4152), // "MAX_VAR"
			mode: EditorMode::Map,
			active_pass: 1,
			active_color: None,
			palettes: PaletteManager::default(),
			toolbox_scroll: 0.0,
			brush_dropdown_open: false,
			active_tile: None,
			clock: 0.0,
		};
		s.scan_templates();
		s
	}

	/// Install the loaded bindings' shortcut hints: stamped onto the main
	/// menu items now, kept for context-menu builds later. Lines must match
	/// the menus' canonical command strings exactly.
	pub fn apply_shortcut_hints(&mut self, hints: Vec<(String, String)>) {
		self.menu.apply_shortcuts(&hints);
		self.shortcut_hints = hints;
	}

	/// The chord label bound to a command line, if any (`"copy"` → `"Ctrl+C"`).
	fn hint_for(&self, line: &str) -> Option<String> {
		self.shortcut_hints.iter().find(|(l, _)| l == line).map(|(_, label)| label.clone())
	}

	/// The right-click context menu for the current state. `cell` is the map
	/// cell under the click (`None` over chrome / outside the map); cell-bound
	/// entries bake it into their command line.
	fn context_menu_items(&self, cell: Option<(u16, u16)>) -> Vec<menu::Item> {
		let act = |label: &str, command: &str| menu::Item::Action {
			label: label.into(),
			hint: self.hint_for(command),
			command: command.into(),
		};
		// A focused text field in the open modal gets a text-edit menu (Cut/Copy/
		// Delete only with a selection, Select All only when non-empty) instead of
		// the map menu - the same conditional-inclusion idiom as below.
		if let Some(ec) = self.active_modal_ref().and_then(|m| m.edit_context()) {
			let mut items = Vec::new();
			if ec.has_selection {
				items.push(act("Cut", "edit-cut"));
				items.push(act("Copy", "edit-copy"));
				items.push(act("Delete", "edit-delete"));
				items.push(menu::Item::Sep);
			}
			items.push(act("Paste", "edit-paste"));
			if !ec.is_empty {
				items.push(act("Select All", "edit-select-all"));
			}
			return items;
		}
		let mut items = Vec::new();
		if self.stamp.is_some() {
			if let Some((x, y)) = cell {
				items.push(act("Place Here", &format!("stamp {x} {y}")));
			}
			items.push(menu::Item::Action {
				label: "Cancel Stamp".into(),
				hint: Some("Esc".into()),
				command: "stamp cancel".into(),
			});
			items.push(menu::Item::Sep);
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
			hint: self.hint_for(command),
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
		self.menu.close();
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

	/// The file stems of saved user palettes (for overwrite/duplicate checks).
	fn user_palette_names(&self) -> Vec<String> {
		let dir = self.user_palettes_dir();
		self.palettes
			.files
			.iter()
			.filter(|p| p.starts_with(&dir))
			.filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
			.collect()
	}

	/// Open the Save (`rename` false) or Rename (`rename` true) palette name
	/// modal. Rename targets the selected user palette.
	fn open_palette_name_modal(&mut self, rename: bool) {
		let existing = self.user_palette_names();
		if rename {
			let Some(path) = self.selected_palette().filter(|_| self.selected_palette_is_user()).cloned() else {
				self.console.push_line("select a saved palette to rename");
				return;
			};
			let from = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
			let others = existing.into_iter().filter(|n| n != &from).collect();
			self.open(crate::palettename::PaletteName::rename(&from, path, others));
		} else {
			// Suggest the selected user palette's name, so Save overwrites it.
			let suggested = self
				.selected_palette()
				.filter(|_| self.selected_palette_is_user())
				.and_then(|p| p.file_stem())
				.map_or_else(String::new, |s| s.to_string_lossy().into_owned());
			self.open(crate::palettename::PaletteName::save(existing, &suggested));
		}
	}

	/// Open the Delete-palette confirm for the selected user palette.
	fn open_palette_delete_modal(&mut self) {
		let Some(path) = self.selected_palette().filter(|_| self.selected_palette_is_user()).cloned() else {
			self.console.push_line("select a saved palette to delete");
			return;
		};
		let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
		self.open(crate::palettedelete::PaletteDelete::new(&name, path));
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

	/// Set the UI scale and mirror it into the font module's global (which label
	/// layout/measurement read), so fonts scale with the rest of the chrome from
	/// one source of truth.
	pub fn set_ui_scale(&mut self, scale: f32) {
		self.ui_scale = scale;
		crate::font::set_ui_scale(scale);
	}

	/// The active edit layer's name (`"water"`/`"ground"`) - for the eraser
	/// tool's `Erase` command and the toolbox highlight.
	pub fn active_layer_name(&self) -> &'static str {
		if self.active_layer == LAYER_WATER { "water" } else { "ground" }
	}

	/// Which layers the map view composites, as a bitmask (bit `n` = layer `n`).
	/// All layers normally; only the active layer when "show only selected" is
	/// on. Consumed by the project shader.
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
			EditorMode::Map => match self.tool {
				Tool::Pencil => "Pencil: drag to paint the active tile - pick one in the Tile Explorer",
				Tool::Eraser => "Eraser: drag to clear cells on the active layer",
				Tool::Picker => "Eyedropper: click a cell to make its tile the brush",
				Tool::Fill => "Flood Fill: click to fill a region - an active selection confines it",
				Tool::Select => "Select: drag to select cells (Shift adds, Ctrl subtracts); Del clears them",
				Tool::SelectRect => "Rect Select: drag a rectangle (Shift adds, Ctrl subtracts)",
				Tool::Unit => "Unit: click to stamp the active unit preview",
				Tool::UnitEraser => "Unit Eraser: click a unit preview to remove it",
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
		self.workspace.clamp_sizes(nw as f32, nh as f32);
		self.workspace.clamp_floating(nw as f32, nh as f32);
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

	/// The open modal, if any - the shell routes input through it (see
	/// `crate::modal`). They're mutually exclusive. Auto Fix Shore joins the
	/// rest here, but its Start/Stop drive a live run, not a command line.
	pub fn active_modal(&mut self) -> Option<&mut (dyn crate::modal::Modal + 'static)> {
		self.modal.as_deref_mut()
	}

	/// Read-only twin of [`Self::active_modal`]: the open modal, if any (same
	/// top-most-first order). Used by `&self` paths - the context-menu builder -
	/// that can't take the `&mut`.
	pub fn active_modal_ref(&self) -> Option<&dyn crate::modal::Modal> {
		self.modal.as_deref()
	}

	/// Open `m` as the single active modal (replacing any currently open).
	fn open(&mut self, m: impl crate::modal::Modal + 'static) {
		self.modal = Some(Box::new(m));
	}

	/// The open modal as a concrete `&T`, when it is a `T`.
	pub fn modal_as<T: crate::modal::Modal + 'static>(&self) -> Option<&T> {
		self.modal.as_ref()?.as_any().downcast_ref::<T>()
	}

	/// The open modal as a concrete `&mut T`, when it is a `T`.
	pub fn modal_as_mut<T: crate::modal::Modal + 'static>(&mut self) -> Option<&mut T> {
		self.modal.as_mut()?.as_any_mut().downcast_mut::<T>()
	}

	/// Take the open modal out as an owned `Box<T>` when it is a `T` (else leave
	/// it in place) - used by the stepped-run drivers, which park the modal
	/// between frames.
	fn take_modal_as<T: crate::modal::Modal + 'static>(&mut self) -> Option<Box<T>> {
		if self.modal.as_ref().is_some_and(|m| m.as_any().is::<T>()) {
			self.modal.take()?.into_any().downcast::<T>().ok()
		} else {
			None
		}
	}

	/// Dismiss whichever modal is open.
	pub fn close_modal(&mut self) {
		self.modal = None;
	}

	/// Commit the open Map Preferences modal to the document, then close it.
	pub fn apply_preferences(&mut self) {
		if let Some(prefs) = self.take_modal_as::<crate::preferences::Preferences>() {
			let (name, players, description, date, version, author) = prefs.values();
			self.project.set_info(name, players, description, date, version, author);
		}
	}

	/// True while the Tile Painter wants live palette cycling - the shell keeps
	/// ticking the cycler + redrawing so the preview (and swatches) shimmer.
	pub fn painter_animating(&self) -> bool {
		self.modal_as::<crate::tilepainter::TilePainter>().is_some_and(|p| p.animate)
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
		let has_clip = self.tile_ops.clipboard.is_some();
		let pack = &self.project.packs[pack_idx];
		self.open(crate::tilepainter::TilePainter::edit(
			pack.ids[tile as usize].clone(),
			pack.name.clone(),
			pack.tile_mask(tile),
			pack.tile_pixels(tile).to_vec(),
			pack.pass.as_ref().map_or(0, |p| p[tile as usize]),
			self.animate,
			has_clip,
		));
		Outcome::Redraw
	}

	/// Open the Tile Painter to clone the selected tile into a new one.
	fn open_tile_clone(&mut self) -> Outcome {
		let (pack_idx, tile) = match self.active_tile_ref() {
			Ok(v) => v,
			Err(e) => return Outcome::Failed(format!("clone tile: {e}")),
		};
		let has_clip = self.tile_ops.clipboard.is_some();
		let src_id = self.project.packs[pack_idx].ids[tile as usize].clone();
		// Suggest a fresh id in the source family for the editable id field.
		let family = map_core::family_of(&src_id).to_string();
		let width = src_id.len().saturating_sub(family.len()).max(3);
		let suggested = self.fresh_tile_id(&family, width);
		let pack = &self.project.packs[pack_idx];
		self.open(crate::tilepainter::TilePainter::clone_from(
			src_id.clone(),
			pack.name.clone(),
			pack.tile_mask(tile),
			pack.tile_pixels(tile).to_vec(),
			pack.pass.as_ref().map_or(0, |p| p[tile as usize]),
			self.animate,
			suggested,
			has_clip,
		));
		Outcome::Redraw
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

	/// Open the Tile Painter on a blank new tile (the target pack is chosen in
	/// the modal).
	fn open_tile_new(&mut self) -> Outcome {
		let packs: Vec<String> =
			self.project.packs.iter().filter(|p| p.name != "WATER").map(|p| p.name.clone()).collect();
		if packs.is_empty() {
			return Outcome::Failed("new tile: no editable pack loaded".into());
		}
		self.open(crate::tilepainter::TilePainter::new_tile(packs, self.animate, self.tile_ops.clipboard.is_some()));
		Outcome::Redraw
	}

	/// Commit the open Tile Painter. An Edit repaints the tile in its pack;
	/// New/Clone append a fresh tile to the per-source-name user pack under
	/// `resources/user/tilepacks/<NAME>/` (created on first use) and persist it.
	/// On success the modal closes and the atlas rebuilds (DocReplaced).
	pub fn tile_paint_commit(&mut self) -> Outcome {
		use crate::tilepainter::Mode;
		let Some(painter) = self.modal_as::<crate::tilepainter::TilePainter>() else { return Outcome::Ok };
		let (mode, pixels, pass) = (painter.mode, painter.pixels().to_vec(), painter.pass);
		let typed = painter.new_id().to_string();
		match mode {
			Mode::Edit => {
				self.commit_tile_edit(painter.pack_name.clone(), painter.tile_id.clone(), typed, &pixels, pass)
			}
			Mode::Clone => {
				// A clone defaults to a fresh id in the source family; the user
				// may have typed their own. Seed the new family's props from the
				// source so the clone renders like its origin (mask/kind).
				let src_family = map_core::family_of(&painter.tile_id).to_string();
				let width = painter.tile_id.len().saturating_sub(src_family.len()).max(3);
				let id = if typed.is_empty() { self.fresh_tile_id(&src_family, width) } else { typed };
				let pack_name = painter.pack_name.clone();
				let seed = self
					.project
					.packs
					.iter()
					.find(|p| p.name == pack_name)
					.and_then(|p| p.props.get(&src_family).cloned());
				self.commit_tile_new(pack_name, id, seed, &pixels, pass)
			}
			Mode::New => {
				let pack = painter.target_pack().to_string();
				// A typed id keeps its family; an empty one parks under "NEW".
				let id = if typed.is_empty() { self.fresh_tile_id("NEW", 3) } else { typed };
				self.commit_tile_new(pack, id, None, &pixels, pass)
			}
		}
	}

	/// Export the open painter's tile as a 64×64 RGBA PNG (palette colors → RGB;
	/// the family's mask color, if any, is written transparent so it round-trips).
	fn tile_export_png(&mut self, path: &Path) -> Outcome {
		let Some(painter) = self.modal_as::<crate::tilepainter::TilePainter>() else {
			return Outcome::Failed("tile-export: open a tile in the painter first".into());
		};
		let mask = painter.mask();
		let pal = &self.project.palette;
		let mut rgba = Vec::with_capacity(painter.pixels().len() * 4);
		for &i in painter.pixels() {
			let o = i as usize * 3;
			let a = if Some(i) == mask { 0 } else { 255 };
			rgba.extend_from_slice(&[pal[o], pal[o + 1], pal[o + 2], a]);
		}
		let tile = crate::tilepainter::TILE as u32;
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
				let line = format!("exported template to {} ({out_w}×{out_h})", path.display());
				eprintln!("{line}");
				self.console.push_line(line);
				Outcome::Redraw
			}
			Err(e) => Outcome::Failed(format!("template-export-png: {e}")),
		}
	}

	/// Load a PNG into the open painter, mapping each pixel to its visually
	/// closest palette color (nearest RGB). Non-64×64 images are nearest-sampled
	/// to the tile; transparent pixels become the family's mask color.
	fn tile_import_png(&mut self, path: &Path) -> Outcome {
		if self.modal_as::<crate::tilepainter::TilePainter>().is_none() {
			return Outcome::Failed("tile-import: open a tile in the painter first".into());
		}
		let (rgba, w, h) = match decode_png_rgba(path) {
			Ok(v) => v,
			Err(e) => return Outcome::Failed(format!("tile-import: {e}")),
		};
		if w == 0 || h == 0 {
			return Outcome::Failed("tile-import: empty image".into());
		}
		let tile = crate::tilepainter::TILE;
		let mask = self.modal_as::<crate::tilepainter::TilePainter>().unwrap().mask().unwrap_or(0);
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
		self.modal_as_mut::<crate::tilepainter::TilePainter>().unwrap().set_pixels(&indices);
		let line = format!("imported {} ({w}×{h}) into the tile painter", path.display());
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
		if !id.chars().all(crate::tilepainter::is_id_char) {
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
		self.modal = None;
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
		self.modal = None;
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

	/// Raise the error modal with `message` (the shell calls this on a failed
	/// command). Also mirrored to the console for the scrollback.
	pub fn raise_error(&mut self, message: &str) {
		self.console.push_line(format!("error: {message}"));
		self.open(crate::errormodal::ErrorModal::new(message));
	}

	/// Whether the Auto Fix Shore run is live (the shell keeps redrawing +
	/// ticking it while so).
	pub fn autofix_running(&self) -> bool {
		self.modal_as::<crate::autofix::AutoFix>().is_some_and(|a| a.running)
	}

	/// Begin an Auto Fix Shore run with the modal's chosen mode.
	pub fn autofix_start(&mut self) {
		let Some(strength) = self.modal_as::<crate::autofix::AutoFix>().map(|a| a.mode.strength()) else { return };
		let session = self.project.fix_session(None, strength);
		let found = session.found();
		if let Some(af) = self.modal_as_mut::<crate::autofix::AutoFix>() {
			af.found = found;
			af.fixed = 0;
			af.remaining = found;
			af.elapsed = 0.0;
			af.applied = None;
			af.running = true;
			af.session = Some(session);
		}
	}

	/// Step the live run by a bounded slice; `elapsed` is wall-clock since
	/// Start (for the Fast cap + the display). Applies as one undo unit when
	/// it finishes or the Fast budget elapses; `stop` forces a finish.
	pub fn autofix_tick(&mut self, elapsed: f32, stop: bool) -> Outcome {
		use crate::autofix::FixMode;
		let Some(mut af) = self.take_modal_as::<crate::autofix::AutoFix>() else { return Outcome::Ok };
		let mut outcome = Outcome::Redraw;
		if af.running {
			af.elapsed = elapsed;
			if let Some(session) = af.session.as_mut() {
				// ~200k nodes/frame keeps a frame well under a millisecond of
				// search while still closing fast.
				if !stop {
					session.step(200_000);
				}
				af.fixed = session.fixed();
				af.remaining = session.remaining();
				let times_up = matches!(af.mode, FixMode::Fast) && elapsed >= 1.0;
				if stop || times_up || session.is_done() {
					af.running = false;
					af.applied = Some(session.apply(&mut self.project));
					outcome = Outcome::Redraw;
				}
			}
		}
		self.modal = Some(af);
		outcome
	}

	/// Whether a terrain generation run is live (the shell keeps redrawing +
	/// stepping it while so).
	pub fn generate_running(&self) -> bool {
		self.modal_as::<crate::generator::Generator>().is_some_and(|g| g.running)
	}

	/// Begin a generation run from the modal's settings. An empty
	/// seed field rolls a fresh seed (reported, so the map can be re-made).
	pub fn generate_start(&mut self) -> Outcome {
		let Some(modal) = self.modal_as::<crate::generator::Generator>() else { return Outcome::Ok };
		let (mut params, seed) = match modal.params() {
			Ok(p) => p,
			Err(e) => return Outcome::Failed(format!("generate: {e}")),
		};
		params.seed = seed.unwrap_or_else(roll_seed);
		match map_core::GenSession::new(&self.project, params) {
			Ok(session) => {
				let modal = self.modal_as_mut::<crate::generator::Generator>().expect("generator modal checked above");
				modal.session = Some(session);
				modal.started = Some(params);
				modal.running = true;
				modal.status = vec![format!("seed {}", params.seed)];
				Outcome::Redraw
			}
			Err(e) => Outcome::Failed(format!("generate: {e}")),
		}
	}

	/// Step (or abort) the live generation run - the shell calls this per
	/// frame within a time budget. Completion reports to the console; an
	/// abort rolls the document back to before the run.
	pub fn generate_tick(&mut self, abort: bool) -> Outcome {
		let Some(mut modal) = self.take_modal_as::<crate::generator::Generator>() else { return Outcome::Ok };
		if modal.running {
			if let Some(mut session) = modal.session.take() {
				if abort {
					session.abort(&mut self.project);
					modal.running = false;
					modal.status = vec!["aborted".into()];
					self.console.push_line("generate: aborted, map rolled back");
				} else if session.step(&mut self.project) {
					let stats = session.stats().expect("stats set when done");
					let started = modal.started.as_ref().expect("started set on start");
					modal.status = generate_status_lines(started, stats);
					self.console.push_line(generate_report(started, stats));
					modal.running = false;
				} else {
					modal.session = Some(session);
				}
			} else {
				modal.running = false;
			}
		}
		self.modal = Some(modal);
		Outcome::Redraw
	}

	/// Whether the New-from-Image conversion is live (the shell keeps redrawing
	/// + stepping it while so).
	pub fn converting(&self) -> bool {
		self.modal_as::<crate::newfromimage::NewFromImage>().is_some_and(|m| m.running)
	}

	/// Begin the New-from-Image conversion. Validates the settings up front, but
	/// defers loading the image pixels to the first `convert_tick` (shown as the
	/// "Loading image" stage), so a click on Convert is instant.
	pub fn convert_start(&mut self) -> Outcome {
		let Some(m) = self.modal_as_mut::<crate::newfromimage::NewFromImage>() else { return Outcome::Ok };
		if m.running {
			return Outcome::Ok;
		}
		if let Err(e) = m.opts() {
			return Outcome::Failed(format!("convert: {e}"));
		}
		m.session = None;
		m.running = true;
		m.progress = 0.0;
		m.elapsed = 0.0;
		m.stage = "Loading image…".to_string();
		Outcome::Redraw
	}

	/// Step the live conversion a bounded slice; `elapsed` is wall-clock since
	/// Convert (for the display + ETA). On completion, opens the result as a new
	/// tab; `abort` stops the run and returns to the settings.
	pub fn convert_tick(&mut self, elapsed: f32, abort: bool) -> Outcome {
		let Some(mut m) = self.take_modal_as::<crate::newfromimage::NewFromImage>() else { return Outcome::Ok };
		let mut outcome = Outcome::Redraw;
		if m.running {
			m.elapsed = elapsed;
			if abort {
				m.running = false;
				m.session = None;
				m.stage = "Aborted".to_string();
			} else if m.session.is_none() {
				// First stage: load the image pixels and prepare the session.
				match decode_and_build(&m) {
					Ok(session) => {
						m.session = Some(session);
						m.stage = "Loading image…".to_string();
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
								"imported image: {}×{} cells, {} tiles",
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
		self.modal = Some(m);
		outcome
	}

	/// Run the open Import WRL modal's match against its selected packs. The
	/// match is fast (a hashmap over the packs), so it runs synchronously on the
	/// Import press: a clean match opens the converted map at once, otherwise the
	/// modal switches to its unmapped-review stage.
	pub fn wrl_match(&mut self) -> Outcome {
		let Some(m) = self.modal_as::<crate::importwrl::ImportWrl>() else { return Outcome::Ok };
		if !m.has_owner() {
			return Outcome::Failed("import-wrl: select at least one palette-owning tileset (e.g. GREEN)".into());
		}
		let path = m.path.clone();
		let packs = m.selected_packs();
		let owner = m.owner();
		let name = m.map_name().to_string();
		let wrl = match read_wrl_file(&path) {
			Ok(w) => w,
			Err(e) => return Outcome::Failed(format!("import-wrl {}: {e}", path.display())),
		};
		// Deterministic seed: the water fill beneath matched/dropped cells (and
		// thus the import) reproduces exactly for the same WRL + pack choice.
		let import = match map_core::WrlImport::new(wrl, &name, &owner, &packs, &self.assets_root, 0) {
			Ok(i) => i,
			Err(e) => return Outcome::Failed(format!("import-wrl: {e}")),
		};
		// A clean match: close the picker and open the converted map straight
		// away (the modal lives on the editor, not the tab, so close it first).
		if import.unmapped().is_empty() {
			self.close_modal();
			let (project, _) = import.finish(map_core::ExtrasDest::Ignore);
			return self.add_doc(project, None, None);
		}
		let Some(m) = self.modal_as_mut::<crate::importwrl::ImportWrl>() else { return Outcome::Ok };
		m.set_result(import);
		Outcome::Redraw
	}

	/// Commit the open Import WRL modal: place its unmapped tiles per the chosen
	/// destination, open the converted map as a new tab, and persist the user
	/// pack when the extras were folded into the user tileset.
	pub fn wrl_finish(&mut self) -> Outcome {
		let Some(mut m) = self.take_modal_as::<crate::importwrl::ImportWrl>() else { return Outcome::Ok };
		let Some((import, dest)) = m.take_result() else {
			// Finish only fires in the unmapped stage; keep the modal if not.
			self.modal = Some(m);
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

	/// Whether the rasterize palette conversion is live (the shell keeps
	/// redrawing + stepping it while so).
	pub fn palette_converting(&self) -> bool {
		self.modal_as::<crate::convertpalette::ConvertPalette>().is_some_and(|m| m.running)
	}

	/// Begin the rasterize palette conversion. Validates the options up
	/// front; the session itself is built on the first `palette_convert_tick`
	/// so a click on Convert paints the running state instantly.
	pub fn palette_convert_start(&mut self) -> Outcome {
		let Some(m) = self.modal_as_mut::<crate::convertpalette::ConvertPalette>() else { return Outcome::Ok };
		if m.running {
			return Outcome::Ok;
		}
		if let Err(e) = m.dedupe_opts() {
			return Outcome::Failed(format!("convert-palette: {e}"));
		}
		m.session = None;
		m.running = true;
		m.progress = 0.0;
		m.elapsed = 0.0;
		m.stage = "Rendering map".to_string();
		Outcome::Redraw
	}

	/// Step the live palette conversion a bounded slice; `elapsed` is wall-
	/// clock since Convert (display + ETA). On completion the document
	/// content swaps in (one undo unit) and the modal closes; `abort` stops
	/// the run and returns to the options.
	pub fn palette_convert_tick(&mut self, elapsed: f32, abort: bool) -> Outcome {
		let Some(mut m) = self.take_modal_as::<crate::convertpalette::ConvertPalette>() else { return Outcome::Ok };
		let mut outcome = Outcome::Redraw;
		if m.running {
			m.elapsed = elapsed;
			if abort {
				m.running = false;
				m.session = None;
				m.stage = "Aborted".to_string();
			} else {
				if m.session.is_none() {
					let (relaxed, threshold) = m.dedupe_opts().expect("validated at start");
					let dedupe = if relaxed { map_core::Dedupe::Relaxed } else { map_core::Dedupe::Strict };
					m.session = Some(map_core::PaletteReimport::new(&self.project, m.water, dedupe, threshold));
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
		self.modal = Some(m);
		outcome
	}

	/// Whether `path` is one of the shipped read-only maps
	/// (`resources/assets/maps/`). Those load path-less so a Save never
	/// overwrites them (Save → Save-As), same as an imported WRL.
	fn is_template(&self, path: &Path) -> bool {
		path.starts_with(self.resources_root.join("assets/maps"))
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
	fn remember_recent(&mut self, path: &Path) {
		if self.is_template(path) {
			return;
		}
		self.recent.retain(|p| p != path);
		self.recent.insert(0, path.to_path_buf());
		self.recent.truncate(10);
		let entries = self.recent_map_entries();
		self.menu.set_recent(&entries);
	}

	/// Seed the recent-maps list from settings at startup, then sync the menu.
	pub fn load_recent(&mut self, paths: Vec<PathBuf>) {
		self.recent = paths;
		self.recent.truncate(10);
		let entries = self.recent_map_entries();
		self.menu.set_recent(&entries);
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
		format!("{name}{star} - M.A.X. Map Editor")
	}

	// ----- multi-project tabs --------------------------------------

	/// The active tab index.
	pub fn active_tab(&self) -> usize {
		self.tabs.active
	}

	/// `(label, dirty)` for each open project, in tab order - the tab strip.
	pub fn tab_infos(&self) -> Vec<(String, bool)> {
		(0..self.tabs.slots.len()).map(|i| (self.name_at(i), self.dirty_at(i))).collect()
	}

	/// Whether tabs show a close `x`: false for the lone blank scratch (the
	/// "no project open" state - nothing to close).
	pub fn tabs_closable(&self) -> bool {
		!(self.tabs.replace_scratch && self.tabs.slots.len() == 1)
	}

	/// Any open project has unsaved changes - the quit guard.
	fn any_dirty(&self) -> bool {
		self.project.dirty() || self.tabs.slots.iter().flatten().any(|d| d.project.dirty())
	}

	/// A prompt summarizing the unsaved work for the quit confirm: names the one
	/// dirty map, or counts them when several tabs are unsaved.
	fn dirty_summary(&self) -> String {
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
	fn dirty_at(&self, i: usize) -> bool {
		if i == self.tabs.active {
			self.project.dirty()
		} else {
			self.tabs.slots[i].as_ref().is_some_and(|d| d.project.dirty())
		}
	}

	/// Tab `i`'s label: the save file name, else the project's own name.
	fn name_at(&self, i: usize) -> String {
		let (path, project_name) = if i == self.tabs.active {
			(self.path.as_deref(), self.project.name.as_str())
		} else {
			let d = self.tabs.slots[i].as_ref();
			(d.and_then(|d| d.path.as_deref()), d.map(|d| d.project.name.as_str()).unwrap_or(""))
		};
		path.and_then(|p| p.file_name())
			.map(|n| n.to_string_lossy().into_owned())
			.or_else(|| (!project_name.is_empty()).then(|| project_name.to_string()))
			.unwrap_or_else(|| "untitled".into())
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
	fn switch_to(&mut self, i: usize) -> Outcome {
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
	fn add_doc(&mut self, project: Project, path: Option<PathBuf>, origin: Option<PathBuf>) -> Outcome {
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

	/// Close the active tab. A dirty tab needs `force` (the confirm modal - see
	/// the `CloseProject` handler - gates this). Closing the **last** project
	/// is allowed: it resets to a blank scratch (the app stays open), which the
	/// next `open`/`new` replaces.
	fn close_active(&mut self, force: bool) -> Outcome {
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

	/// Write the project `.json`, plus any synthetic pack (one built by
	/// `Project::from_wrl` for an imported WRL - absent from `assets_root`) to a
	/// sibling folder named after it, so the saved project reloads. Only the
	/// inferable assets are dumped (see `TilePack::dump`).
	fn write_project(&self, target: &Path) -> Result<(), String> {
		std::fs::write(target, self.project.save_string()).map_err(|e| format!("{}: {e}", target.display()))?;
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
		match command {
			c @ (Pan { .. } | PanTo { .. } | Zoom { .. } | ZoomAt { .. } | ZoomTo { .. } | Fit) => self.exec_nav(c),
			c @ (SetTile { .. }
			| SetPass { .. }
			| Place { .. }
			| Erase { .. }
			| AssertCell { .. }
			| New { .. }
			| Tile { .. }
			| Paint { .. }
			| Fill { .. }
			| Randomize { .. }
			| BrushSize { .. }
			| BrushShape { .. }
			| ToolSelect { .. }
			| Layer { .. }
			| Mode { .. }
			| PassPick { .. }
			| PassPaint { .. }
			| TilePass { .. }
			| PassClear { .. }
			| ResetTilePass
			| TransformTile { .. }
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
			| NewMapModal { .. }
			| Window { .. }
			| DockTo { .. }
			| ResetLayout
			| UnitSelect { .. }
			| UnitTeam { .. }
			| UnitPlace { .. }
			| UnitErase { .. }
			| UnitClear
			| UnitsVisible { .. }
			| SaveSettings) => self.exec_panels(c),
			c @ (Undo
			| Redo
			| Open { .. }
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
			| AutoFixModal
			| GenerateModal
			| Export { .. }
			| ConvertPalette { .. }
			| ConvertPaletteModal
			| PreferencesModal
			| TilePaintNew
			| TilePaintClone
			| TilePaintEdit
			| TileDelete
			| TileCommit
			| TileExportPng { .. }
			| TileImportPng { .. }
			| Bake
			| UpdateMap
			| OpenUrl { .. }
			| HelpManual
			| About) => self.exec_io(c),
			c @ (Grid { .. }
			| StatusBar { .. }
			| PassOverlay { .. }
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
			// A text-field right-click edit, routed to the open modal's focused field.
			Edit(op) => {
				use crate::command::EditOp::*;
				use crate::modal::ModalKey;
				let key = match op {
					Cut => ModalKey::Cut,
					Copy => ModalKey::Copy,
					Paste => ModalKey::Paste,
					Delete => ModalKey::Delete,
					SelectAll => ModalKey::SelectAll,
				};
				if let Some(m) = self.active_modal() {
					m.on_key(key);
				}
				Outcome::Redraw
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
							"new map {width}×{height}, packs: {}, seed {seed}",
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
							edits.push((cx, cy, self.active_layer, Some(t)));
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
						let (layer, randomize) = (self.active_layer, self.randomize);
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
			Command::Layer { name } => {
				self.active_layer = match name.as_str() {
					"water" => LAYER_WATER,
					"ground" => LAYER_GROUND,
					other => return Outcome::Failed(format!("layer: unknown '{other}' (water|ground)")),
				};
				self.console.push_line(format!("layer: {name}"));
				Outcome::Redraw
			}
			Command::ToolSelect { name } => {
				self.tool = match name.as_str() {
					"pencil" => Tool::Pencil,
					"picker" | "pick" => Tool::Picker,
					"eraser" | "erase" => Tool::Eraser,
					"fill" | "flood" => Tool::Fill,
					"unit" => Tool::Unit,
					"unit-eraser" | "unit-erase" => Tool::UnitEraser,
					"select" => Tool::Select,
					"select-rect" | "rect" => Tool::SelectRect,
					other => {
						return Outcome::Failed(format!(
							"tool: unknown '{other}' (pencil|picker|eraser|fill|unit|unit-eraser|select|select-rect)"
						));
					}
				};
				self.console.push_line(format!("tool: {name}"));
				Outcome::Redraw
			}
			Command::Mode { name } => {
				self.mode = match name.as_str() {
					"map" => EditorMode::Map,
					"pass" => EditorMode::Pass,
					"localpass" => EditorMode::LocalPass,
					other => {
						return Outcome::Failed(format!("mode: unknown '{other}' (map|pass|localpass)"));
					}
				};
				// The pass overlay rides with either pass editor: it turns on so
				// painting is visible, and off again on returning to Map.
				self.show_pass_overlay = matches!(self.mode, EditorMode::Pass | EditorMode::LocalPass);
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
			Command::TransformTile { op } => {
				// An armed template stamp takes the transform tool: rotate/flip the
				// whole stamp (constrained by what its tiles allow) instead of the
				// brush. The ghost preview re-resolves the new template automatically.
				if let Some(stamp) = self.stamp.clone() {
					let Some(stamp_op) = map_core::StampOp::parse(&op) else {
						return Outcome::Failed(format!("transform: unknown '{op}' (flip-h|flip-v|cw|ccw)"));
					};
					return match stamp.transformed(&self.project, stamp_op) {
						Ok(t) => {
							self.console.push_line(format!("stamp {op} ({}x{})", t.width, t.height));
							self.stamp = Some(t);
							Outcome::Redraw
						}
						Err(e) => Outcome::Failed(format!("transform: {e}")),
					};
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
				// The eyedropper hands back to the pencil - pick, then paint.
				self.tool = Tool::Pencil;
				Outcome::Redraw
			}
			Command::Shore { region, mode } => {
				let project = &mut self.project;
				if let Some((x0, y0, x1, y1)) = region {
					if x0.max(x1) >= project.width || y0.max(y1) >= project.height {
						return Outcome::Failed(format!(
							"shore: region exceeds the {}x{} map",
							project.width, project.height,
						));
					}
				}
				let (changed, unresolved, how) = match mode {
					ShoreMode::Sweep => {
						let (c, u) = project.auto_shore(region);
						(c, u, "auto-shore")
					}
					ShoreMode::Alt => {
						let (c, u) = project.auto_shore_alt(region);
						(c, u, "auto-shore alt")
					}
					ShoreMode::Fix => {
						let (c, u) = project.fix_shore(region);
						(c, u, "fix-shore")
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
			Command::Generate { pattern, water, obstructions, decorations, seed, alt_shore } => {
				let pattern = match map_core::GenPattern::parse(&pattern) {
					Ok(p) => p,
					Err(e) => return Outcome::Failed(format!("generate: {e}")),
				};
				// No seed given: fresh randomness, reported below so the map
				// can be re-made (same convention as `new`).
				let seed = seed.unwrap_or_else(roll_seed);
				let params = map_core::GenParams { pattern, water, obstructions, decorations, seed, alt_shore };
				match self.project.generate_terrain(&params) {
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
						self.console.push_line(format!("palette saved → {}", path.display()));
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
						self.console.push_line(format!("palette loaded ({n} slots) ← {}", path.display()));
						Outcome::Redraw
					}
					Err(e) => Outcome::Failed(format!("palette-load: {e}")),
				}
			}
			Command::PaletteSaveAs { name } => {
				let path = self.user_palettes_dir().join(format!("{}.json", name.trim()));
				match crate::palette_io::save(&path, &self.project.palette, name.trim()) {
					Ok(()) => self.palette_saved(format!("palette saved → {}", path.display()), Some(path)),
					Err(e) => Outcome::Failed(format!("palette-save-as: {e}")),
				}
			}
			Command::PaletteRename { from, to } => {
				let target = self.user_palettes_dir().join(format!("{}.json", to.trim()));
				match crate::palette_io::rename(&from, &target) {
					Ok(()) => self.palette_saved(format!("palette renamed → {}", target.display()), Some(target)),
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
					Ok(dest) => self.palette_saved(format!("palette imported → {}", dest.display()), Some(dest)),
					Err(e) => Outcome::Failed(format!("palette-import: {e}")),
				}
			}
			Command::PaletteSaveModal => {
				self.open_palette_name_modal(false);
				Outcome::Redraw
			}
			Command::PaletteRenameModal => {
				self.open_palette_name_modal(true);
				Outcome::Redraw
			}
			Command::PaletteDeleteModal => {
				self.open_palette_delete_modal();
				Outcome::Redraw
			}
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
				self.picker.scroll = 0.0;
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
				self.picker.scroll = to.max(0.0);
				Outcome::Redraw
			}
			Command::PaletteScroll { to } => {
				self.palettes.scroll = to.max(0.0);
				Outcome::Redraw
			}
			Command::MenuOpen { name } => match self.menu.open_by_name(&name) {
				Ok(()) => Outcome::Redraw,
				Err(e) => Outcome::Failed(format!("menu: {e}")),
			},
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
				self.menu.close();
				Outcome::Redraw
			}
			Command::NewMapModal { picking } => {
				let mut modal = NewMap::new(&self.assets_root);
				modal.picking = picking;
				self.open(modal);
				self.menu.close();
				Outcome::Redraw
			}
			Command::Window { id, on } => match self.workspace.show(&id, on) {
				Ok(line) => {
					self.console.push_line(line);
					Outcome::Redraw
				}
				Err(e) => Outcome::Failed(format!("window: {e}")),
			},
			Command::DockTo { id, place, at } => match self.workspace.dock_to(&id, &place, at) {
				Ok(line) => {
					self.console.push_line(line);
					Outcome::Redraw
				}
				Err(e) => Outcome::Failed(format!("dock: {e}")),
			},
			Command::ResetLayout => {
				self.workspace.reset();
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
				if self.project.erase_unit_at(x, y) {
					Outcome::Redraw
				} else {
					Outcome::Ok
				}
			}
			Command::UnitClear => {
				let n = self.project.clear_units();
				self.console.push_line(format!("unit previews cleared ({n})"));
				Outcome::Redraw
			}
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
					let mut section = self.workspace.to_ini();
					let _ = section.set_entry("UiScale".to_string(), self.ui_scale.to_string());
					// Explorer thumbnail sizes (Tile Explorer + Templates Explorer) persist
					// alongside the layout, so the chosen preview size survives a restart.
					let _ = section.set_entry("TilesPreview".to_string(), self.picker.tile_px.to_string());
					let _ = section.set_entry("TemplatesPreview".to_string(), self.templates.cell.to_string());
					// Recent maps (File ▸ Quick Load), most-recent first.
					for (i, recent) in self.recent.iter().enumerate() {
						let _ = section.set_entry(format!("Recent{i}"), recent.display().to_string());
					}
					match crate::settings_io::save_workspace(path, section) {
						Ok(()) => {
							self.console.push_line(format!("settings saved → {}", path.display()));
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

	/// Stamp (or restamp) a unit preview on a cell. The note persists with
	/// the project (dirties it) but records no undo patch - annotations are
	/// metadata, not map edits.
	fn place_unit_preview(&mut self, unit: usize, x: u16, y: u16) -> Outcome {
		let (w, h) = self.map_size();
		if x >= w || y >= h {
			return Outcome::Failed(format!("unit-place: ({x},{y}) is outside the {w}×{h} map"));
		}
		let tag = self.units.as_ref().expect("units loaded before placing").units[unit].tag.clone();
		self.project.stamp_unit(map_core::UnitNote { tag, x, y, team: self.unit_team });
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
				clear_selection_layer(&mut self.project, &self.selection, self.active_layer);
				self.console.push_line(format!("deleted {n} cells ({})", self.active_layer_name()));
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
			Command::Paste => match &self.clipboard {
				Some(t) => {
					self.stamp = Some(t.clone());
					self.console.push_line("paste: click the map to place (Esc cancels)".to_string());
					Outcome::Redraw
				}
				None => Outcome::Failed("paste: the clipboard is empty (copy or cut first)".into()),
			},
			Command::Stamp { x, y } => {
				let Some(stamp) = self.stamp.clone() else {
					return Outcome::Failed("stamp: nothing armed (paste or pick a template first)".into());
				};
				match stamp.apply(&mut self.project, x, y) {
					// The stamp stays armed for repeat placing (forests!).
					Ok(_) => Outcome::Redraw,
					Err(e) => Outcome::Failed(format!("stamp: {e}")),
				}
			}
			Command::StampCancel => {
				self.stamp = None;
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
				let entry = &self.templates.entries[i];
				if let Some(id) = entry.template.missing_id(&self.project) {
					return Outcome::Failed(format!(
						"template-pick: '{name}' needs tile '{id}' - its pack isn't in this map"
					));
				}
				self.stamp = Some(entry.template.clone());
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
				self.open(crate::renametemplate::RenameTemplate::new(&entry.name, entry.template.clone(), existing));
				Outcome::Redraw
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
				self.open(crate::deletetemplate::DeleteTemplate::new(&entry.name, entry.template.clone()));
				Outcome::Redraw
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
				self.open(crate::dedupetemplates::DedupeTemplates::new(names));
				Outcome::Redraw
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
		self.templates.entries = entries;
		if self.templates.sel.is_some_and(|i| i >= self.templates.entries.len()) {
			self.templates.sel = None;
		}
	}

	/// Indices into `templates` that resolve against the open map - what the
	/// explorer shows (incompatible ones would only stamp errors).
	pub fn visible_templates(&self) -> Vec<usize> {
		(0..self.templates.entries.len())
			.filter(|&i| self.templates.entries[i].template.compatible(&self.project))
			.collect()
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
								"opened {}: \"{}\" {}×{} cells, packs: {}",
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
								"imported {}: {}×{} cells, {} tiles",
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
			// New from Image: read only the PNG header (dimensions)
			// and open the settings modal - pixels are decoded later, at Convert.
			Command::NewFromImage { path } => {
				let (w, h) = match png_dimensions(&path) {
					Ok(v) => v,
					Err(e) => return Outcome::Failed(format!("new-from-image {}: {e}", path.display())),
				};
				let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into());
				self.open(crate::newfromimage::NewFromImage::new(path, w, h, name));
				self.menu.close();
				Outcome::Redraw
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
				let modal = crate::importwrl::ImportWrl::new(
					path,
					name,
					header.width,
					header.height,
					header.tile_count,
					&self.assets_root,
				);
				self.open(modal);
				self.menu.close();
				Outcome::Redraw
			}
			// Help ▸ Go to Website / Project GitHub - hand the URL to the OS browser.
			Command::OpenUrl { url } => {
				self.menu.close();
				match crate::browser::open(&url) {
					Ok(()) => Outcome::Ok,
					Err(e) => Outcome::Failed(e),
				}
			}
			// Help ▸ User Manual - open the bundled HTML manual in the browser.
			Command::HelpManual => {
				self.menu.close();
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
			// Help ▸ About - the credits / version dialog.
			Command::About => {
				self.open(crate::about::About::new());
				self.menu.close();
				Outcome::Redraw
			}
			// Run the open image modal's conversion to completion synchronously
			// (scripts / headless). The interactive button uses the stepped path.
			Command::Convert => {
				let Some(m) = self.modal_as::<crate::newfromimage::NewFromImage>() else {
					return Outcome::Failed("convert: no image to convert (open File ▸ New from Image)".into());
				};
				let name = m.name.clone();
				let result = decode_and_build(m).and_then(|mut s| {
					while !s.is_done() {
						s.step(usize::MAX);
					}
					s.finish()
				});
				match result {
					Ok(wrl) => {
						let project = Project::from_wrl(&wrl, &name);
						eprintln!(
							"imported image: {}×{} cells, {} tiles",
							project.width, project.height, wrl.tile_count
						);
						self.modal = None;
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
					self.open(crate::confirm::ConfirmClose::new(self.name_at(self.tabs.active)));
					Outcome::Redraw
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
					self.open(crate::confirm::ConfirmClose::new_quit(self.dirty_summary()));
					Outcome::Redraw
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
							self.open(crate::confirm::ConfirmClose::new_quit(self.dirty_summary()));
							Outcome::Redraw
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
					Some(user_templates.as_path()),
				);
				let suggested = dialog_suggested_name(purpose, self.path.as_deref(), &self.project.name);
				self.menu.close();

				if self.headless {
					return Outcome::Failed("file-dialog: not available in headless runs".into());
				}
				// Tile Painter PNG export/import: a `.png` dialog whose result a
				// command line *can* carry (just a path), so handle it up front -
				// it doesn't share the `.json` plumbing below.
				match purpose {
					FilePurpose::ExportTilePng => {
						let name = self
							.modal_as::<crate::tilepainter::TilePainter>()
							.map(|p| p.new_id())
							.filter(|s| !s.is_empty())
							.unwrap_or("tile");
						let picked = rfd::FileDialog::new()
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
						let picked = rfd::FileDialog::new()
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
						let picked = rfd::FileDialog::new()
							.set_directory(&start)
							.add_filter("PNG images", &["png"])
							.set_file_name(format!("{name}.png"))
							.save_file();
						return match picked {
							None => Outcome::Redraw,
							Some(path) => self.execute(Command::TemplateExportPng { path: Some(path) }),
						};
					}
					_ => {}
				}
				// Native dialog (rfd): blocks the event loop, which is fine -
				// the dialog is modal by nature. Cancel is a quiet no-op.
				let dialog = rfd::FileDialog::new().set_directory(&start);
				let picked = match purpose {
					FilePurpose::Load => dialog
						.add_filter("M.A.X. maps", &["json", "wrl", "WRL"])
						.add_filter("all files", &["*"])
						.pick_file(),
					FilePurpose::NewFromImage => {
						dialog.add_filter("PNG images", &["png"]).add_filter("all files", &["*"]).pick_file()
					}
					FilePurpose::ImportWrl => dialog
						.add_filter("M.A.X. WRL maps", &["wrl", "WRL"])
						.add_filter("all files", &["*"])
						.pick_file(),
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
					FilePurpose::ExportTilePng | FilePurpose::ImportTilePng | FilePurpose::ExportTemplatePng => {
						unreachable!("handled before the json dialog")
					}
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
						FilePurpose::ImportWrl => self.execute(Command::ImportWrl { path }),
						FilePurpose::ImportTemplate => self.execute(Command::TemplateImport { path }),
						FilePurpose::ExportTemplate => self.execute(Command::TemplateExport { path }),
						FilePurpose::ExportTilePng | FilePurpose::ImportTilePng | FilePurpose::ExportTemplatePng => {
							unreachable!("handled before the json dialog")
						}
					},
				}
			}
			Command::Resize { width, height, off_x, off_y } => {
				let project = &mut self.project;
				match project.resize(width, height, off_x, off_y) {
					Ok(()) => {
						self.view = self.fit_center((width, height));
						let line = format!("resized to {width}×{height} (offset {off_x},{off_y})");
						eprintln!("{line}");
						self.console.push_line(line);
						// Dimensions changed - the renderer's textures rebuild.
						Outcome::DocReplaced
					}
					Err(e) => Outcome::Failed(format!("resize: {e}")),
				}
			}
			Command::ResizeModal => {
				let project = &self.project;
				self.open(crate::resize::Resize::new(project.width, project.height));
				self.menu.close();
				Outcome::Redraw
			}
			Command::AutoFixModal => {
				let project = &self.project;
				// A throwaway session counts the current broken seams to show.
				let found = project.fix_session(None, map_core::FixStrength::Shore).found();
				self.open(crate::autofix::AutoFix::new(found));
				self.menu.close();
				Outcome::Redraw
			}
			Command::GenerateModal => {
				self.open(crate::generator::Generator::new());
				self.menu.close();
				Outcome::Redraw
			}
			Command::Export { path } => {
				let project = &self.project;
				// Default: the project's path with the extension swapped.
				let Some(target) = path.or_else(|| self.path.as_ref().map(|p| p.with_extension("wrl"))) else {
					return Outcome::Failed("export: no path (use `export PATH.wrl`)".into());
				};
				match map_core::bake(project).and_then(|wrl| {
					write_wrl_file(&wrl, &target).map_err(|e| e.to_string())?;
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
						Outcome::Redraw
					}
					Err(e) => Outcome::Failed(format!("export {}: {e}", target.display())),
				}
			}
			Command::ConvertPaletteModal => {
				if !self.project.is_wrl_import() {
					return Outcome::Failed(
						"convert-palette: only an opened WRL has an internal palette to convert".into(),
					);
				}
				self.open(crate::convertpalette::ConvertPalette::new());
				self.menu.close();
				Outcome::Redraw
			}
			Command::PreferencesModal => {
				self.open(crate::preferences::Preferences::from_project(&self.project));
				self.menu.close();
				Outcome::Redraw
			}
			Command::TilePaintNew => {
				self.menu.close();
				self.open_tile_new()
			}
			Command::TilePaintClone => {
				self.menu.close();
				self.open_tile_clone()
			}
			Command::TilePaintEdit => {
				self.menu.close();
				self.open_tile_edit()
			}
			Command::TileCommit => self.tile_paint_commit(),
			Command::TileDelete => self.delete_active_tile(),
			Command::TileExportPng { path } => self.tile_export_png(&path),
			Command::TileImportPng { path } => self.tile_import_png(&path),
			Command::Bake => {
				self.menu.close();
				self.bake()
			}
			Command::UpdateMap => {
				self.menu.close();
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

#[cfg(test)]
mod tests {
	use super::*;

	fn resources() -> PathBuf {
		Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources")
	}

	#[test]
	fn nearest_palette_index_matches_closest_and_skips_slot_0() {
		// Palette: slot 0 black (transparent slot), 1 red, 2 green, 3 blue.
		let mut pal = vec![0u8; 768];
		pal[3..6].copy_from_slice(&[255, 0, 0]);
		pal[6..9].copy_from_slice(&[0, 255, 0]);
		pal[9..12].copy_from_slice(&[0, 0, 255]);
		assert_eq!(nearest_palette_index(&pal, 250, 10, 10), 1, "near-red → red");
		assert_eq!(nearest_palette_index(&pal, 10, 240, 5), 2, "near-green → green");
		// Pure black is closest to slot 0, but slot 0 is skipped, so it falls to
		// the next-nearest real color rather than mapping to "transparent".
		assert_ne!(nearest_palette_index(&pal, 0, 0, 0), 0);
	}

	#[test]
	fn template_map_entries_label_name_and_filename() {
		let entries = template_map_entries(&resources().join("assets/maps"));
		assert!(!entries.is_empty(), "the shipped stock maps");
		// Map name as the label, file name right-aligned in the note column -
		// GREEN_1.json is "New Luzon".
		let green = entries.iter().find(|e| e.path.file_stem().is_some_and(|s| s == "GREEN_1")).expect("GREEN_1");
		assert_eq!(green.label, "New Luzon");
		assert_eq!(green.note.as_deref(), Some("GREEN_1"));
	}

	fn editor() -> EditorState {
		let resources = resources();
		let project = Project::new(8, 8, &["GREEN".to_string()], &resources.join("assets/tilepacks"), 1).unwrap();
		EditorState::new(project, (800, 600), None, resources)
	}

	fn new_tab(e: &mut EditorState, seed: u64) -> Outcome {
		e.execute(Command::New { width: 8, height: 8, packs: vec!["GREEN".into()], seed: Some(seed) })
	}

	/// Routing safety net: every toolbox run-button command must parse AND
	/// execute without tripping an `unreachable!` (mis-routed-variant) panic.
	/// Toolbox commands are side-effect-free (tool/brush/shape/layer/transform/
	/// pass/select) - no IO or dialogs - so running them on a scratch editor is
	/// safe. `Act::Todo` buttons carry no command and are skipped.
	#[test]
	fn toolbox_commands_route_without_panicking() {
		for group in crate::toolbox::GROUPS {
			for button in group.buttons {
				let crate::toolbox::Act::Run(cmd) = button.act else { continue };
				let parsed = crate::command::parse_line(cmd)
					.unwrap_or_else(|e| panic!("{cmd}: parse error: {e}"))
					.unwrap_or_else(|| panic!("{cmd}: empty command"));
				// A mis-routed variant trips `unreachable!` in execute and fails here.
				let mut e = editor();
				let _ = e.execute(parsed);
			}
		}
	}

	#[test]
	fn filename_sanitization_lowercases_and_strips() {
		assert_eq!(sanitize_filename("My Cool Oasis"), "my-cool-oasis");
		assert_eq!(sanitize_filename("a/b:c*?"), "abc", "special chars dropped");
		assert_eq!(sanitize_filename("  spaced  out  "), "spaced-out", "edges trimmed, runs collapsed");
		assert_eq!(sanitize_filename("Lake-2"), "lake-2");
		assert_eq!(sanitize_filename("***"), "template", "empty result falls back");
		assert_eq!(sanitize_filename("под"), "template", "non-ascii dropped -> fallback");
	}

	#[test]
	fn natural_sort_orders_numbers_by_value() {
		use std::cmp::Ordering::Less;
		assert_eq!(natural_cmp("template-3", "template-20"), Less, "3 < 20");
		assert_eq!(natural_cmp("template-20", "template-100"), Less, "20 < 100");
		let mut v = ["template-100", "template-3", "template-20", "template-2", "template-1"];
		v.sort_by(|a, b| natural_cmp(a, b));
		assert_eq!(v, ["template-1", "template-2", "template-3", "template-20", "template-100"]);
		// Leading zeros tie by value; plain text is case-insensitive.
		assert_eq!(natural_cmp("a007", "a7"), std::cmp::Ordering::Equal);
		assert_eq!(natural_cmp("Crater", "desert"), Less);
	}

	#[test]
	fn dedupe_finds_only_removable_exact_duplicates() {
		let mut e = editor();
		// All-hole templates resolve in any project, so every one is "visible".
		let mk = |w: u16, h: u16| Template {
			name: String::new(),
			width: w,
			height: h,
			uses: Vec::new(),
			cells: vec![String::new(); (w * h) as usize],
		};
		let entry = |name: &str, stock: bool, t: Template| TemplateEntry {
			name: name.to_string(),
			path: PathBuf::from(format!("{name}.json")),
			stock,
			template: t,
		};
		// A stock template, two user copies of it, then a differently-sized one.
		e.templates.entries = vec![
			entry("stock", true, mk(2, 1)),
			entry("copy-a", false, mk(2, 1)),
			entry("copy-b", false, mk(2, 1)),
			entry("other", false, mk(1, 1)),
		];
		// Both user copies are removable duplicates of the (kept) earlier original;
		// the stock entry and the odd-sized one are left alone.
		assert_eq!(e.duplicate_template_indices(), vec![1, 2]);
	}

	#[test]
	fn context_menu_opens_with_state_dependent_items() {
		let mut e = editor();
		e.apply_shortcut_hints(vec![("copy".into(), "Ctrl+C".into())]);
		// Nothing selected, empty clipboard: the lean menu.
		assert!(matches!(e.execute(Command::ContextMenu { at: Some((400.0, 300.0)) }), Outcome::Redraw));
		let lean = e.context_menu.as_ref().expect("open").panel(800.0, 600.0);
		// Select something: the clipboard block appears, the panel grows.
		e.execute(Command::SelectRect { x0: 1, y0: 1, x1: 2, y1: 2, mode: SelectMode::Replace });
		e.execute(Command::ContextMenu { at: Some((400.0, 300.0)) });
		let full = e.context_menu.as_ref().expect("open").panel(800.0, 600.0);
		assert!(full.h > lean.h, "selection adds cut/copy/delete rows");
		// `off` closes.
		e.execute(Command::ContextMenu { at: None });
		assert!(e.context_menu.is_none());
	}

	#[test]
	fn focused_field_modal_yields_a_text_edit_menu() {
		use crate::modal::ModalKey;
		let action_cmds = |e: &EditorState| -> Vec<String> {
			e.context_menu_items(None)
				.into_iter()
				.filter_map(|i| match i {
					menu::Item::Action { command, .. } => Some(command),
					_ => None,
				})
				.collect()
		};
		let mut e = editor();
		// Map Preferences opens with its Name field focused → the edit menu, not
		// the map menu (Paste always; no selection yet, so no Cut/Copy/Delete).
		assert!(matches!(e.execute(Command::PreferencesModal), Outcome::Redraw));
		let cmds = action_cmds(&e);
		assert!(cmds.iter().any(|c| c == "edit-paste"), "paste offered: {cmds:?}");
		assert!(!cmds.iter().any(|c| c == "fit"), "not the map menu: {cmds:?}");
		assert!(!cmds.iter().any(|c| c == "edit-copy"), "no selection → no copy: {cmds:?}");
		// Type + select: Cut/Copy/Delete now appear (and Select All, non-empty).
		{
			let m = e.active_modal().unwrap();
			m.on_key(ModalKey::Char('X'));
			m.on_key(ModalKey::SelectAll);
		}
		let cmds = action_cmds(&e);
		for want in ["edit-cut", "edit-copy", "edit-delete", "edit-paste", "edit-select-all"] {
			assert!(cmds.iter().any(|c| c == want), "{want} offered with a selection: {cmds:?}");
		}
	}

	#[test]
	fn palette_manager_save_rename_delete_round_trip() {
		let mut e = editor();
		let dir = e.user_palettes_dir();
		let path = dir.join("__test_pal__.json");
		let renamed = dir.join("__test_pal2__.json");
		// Clean slate (a leftover from a previously-failed run must not confuse us).
		let _ = std::fs::remove_file(&path);
		let _ = std::fs::remove_file(&renamed);

		// Save the working palette under a name → file written, rescanned, selected.
		assert!(matches!(e.execute(Command::PaletteSaveAs { name: "__test_pal__".into() }), Outcome::Redraw));
		assert!(path.is_file(), "saved file exists");
		assert!(e.palettes.files.contains(&path), "rescanned + present");
		assert_eq!(e.selected_palette(), Some(&path), "the new palette is selected");
		assert!(e.selected_palette_is_user(), "a user palette is editable");

		// Rename it.
		e.execute(Command::PaletteRename { from: path.clone(), to: "__test_pal2__".into() });
		assert!(!path.is_file() && renamed.is_file(), "renamed on disk");
		assert_eq!(e.selected_palette(), Some(&renamed));

		// Delete it.
		e.execute(Command::PaletteDelete { path: renamed.clone() });
		assert!(!renamed.is_file(), "deleted on disk");
		assert!(e.palettes.sel.is_none(), "selection cleared");
	}

	#[test]
	fn map_palette_toggle_reseeds_the_cycler_from_the_internal_palette() {
		let mut e = editor();
		// A GREEN project: working slot 1 is the game red, the pack's own
		// (internal) byte there differs - that's exactly what the toggle shows.
		let game = [e.project.palette[3], e.project.palette[4], e.project.palette[5]];
		let internal = e.project.internal_palette();
		let raw = [internal[3], internal[4], internal[5]];
		assert_ne!(game, raw, "GREEN's palette.json slot 1 differs from the game palette");

		assert!(matches!(e.execute(Command::MapPalette { on: None }), Outcome::Redraw));
		assert!(e.debug_map_palette);
		assert_eq!(&e.cycler.rgba()[4..7], &raw, "cycler reseeded from the internal palette");
		e.execute(Command::MapPalette { on: Some(false) });
		assert!(!e.debug_map_palette);
		assert_eq!(&e.cycler.rgba()[4..7], &game, "back to the game-resolved palette");
		// A `window wrlpalette` toggle reaches the (hidden-by-default) panel.
		assert!(!e.workspace.is_visible("wrlpalette"));
		assert!(matches!(e.execute(Command::Window { id: "wrlpalette".into(), on: Some(true) }), Outcome::Redraw));
		assert!(e.workspace.is_visible("wrlpalette"));
	}

	#[test]
	fn map_preferences_modal_edits_and_applies_metadata() {
		use crate::modal::ModalKey;
		let mut e = editor();
		assert!(matches!(e.execute(Command::PreferencesModal), Outcome::Redraw));
		assert!(e.modal_as::<crate::preferences::Preferences>().is_some());
		{
			let m = e.active_modal().unwrap();
			// Name field is focused first; replace its default, then Tab past
			// Players to Description and type there too.
			m.on_key(ModalKey::SelectAll);
			for c in "Twin Peaks".chars() {
				m.on_key(ModalKey::Char(c));
			}
		}
		e.apply_preferences();
		assert!(e.modal_as::<crate::preferences::Preferences>().is_none(), "save closes the modal");
		assert_eq!(e.project.name, "Twin Peaks");
		assert!(e.dirty());
	}

	#[test]
	fn status_bar_toggles_and_reserves_the_bottom_strip() {
		let mut e = editor();
		assert!(e.status_bar);
		assert_eq!(e.workspace.bottom, crate::statusbar::BAR_H);
		e.execute(Command::StatusBar { on: Some(false) });
		assert!(!e.status_bar);
		assert_eq!(e.workspace.bottom, 0.0, "hidden bar releases the strip");
		e.execute(Command::StatusBar { on: None });
		assert!(e.status_bar);
		// The hint follows the active tool / mode.
		e.execute(Command::ToolSelect { name: "eraser".into() });
		assert!(e.status_hint().contains("Eraser"), "{}", e.status_hint());
		e.execute(Command::Mode { name: "localpass".into() });
		assert!(e.status_hint().contains("Override"), "{}", e.status_hint());
	}

	#[test]
	fn brush_size_paints_a_centered_square() {
		let mut e = editor(); // 8×8
		e.execute(Command::Tile { spec: Some("GLa000".into()) });
		e.execute(Command::BrushSize { size: 3 });
		assert_eq!(e.brush_size, 3);
		e.execute(Command::Paint { x: 4, y: 4 });
		let ground = |e: &EditorState, x, y| e.project.cell(x, y).unwrap()[LAYER_GROUND].is_some();
		for dy in -1..=1i32 {
			for dx in -1..=1i32 {
				assert!(ground(&e, (4 + dx) as u16, (4 + dy) as u16), "({},{}) painted", 4 + dx, 4 + dy);
			}
		}
		assert!(!ground(&e, 6, 4), "outside the 3×3 footprint untouched");
		// Even sizes snap odd so the square stays centred.
		e.execute(Command::BrushSize { size: 4 });
		assert_eq!(e.brush_size, 5);
	}

	#[test]
	fn circle_brush_drops_the_far_corners() {
		let mut e = editor(); // 8×8
		e.execute(Command::BrushSize { size: 5 });
		e.execute(Command::BrushShape { shape: "circle".into() });
		let cells = e.brush_cells(4, 4);
		assert!(!cells.contains(&(2, 2)) && !cells.contains(&(6, 6)), "circle drops the far corners");
		assert!(cells.contains(&(4, 2)) && cells.contains(&(2, 4)), "axis cells kept");
		e.execute(Command::BrushShape { shape: "square".into() });
		assert!(e.brush_cells(4, 4).contains(&(2, 2)), "square keeps corners");
	}

	#[test]
	fn fill_with_active_selection_fills_only_the_selection() {
		let mut e = editor();
		e.execute(Command::Tile { spec: Some("GLa000".into()) });
		e.execute(Command::SelectRect { x0: 1, y0: 1, x1: 2, y1: 2, mode: SelectMode::Replace });
		assert_eq!(e.selection.count(), 4);
		// Fill: the click cell (6,6) is ignored when a selection is active.
		assert!(matches!(e.execute(Command::Fill { x: 6, y: 6 }), Outcome::Redraw));
		let ground = |e: &EditorState, x, y| e.project.cell(x, y).unwrap()[LAYER_GROUND].map(|t| t.tile);
		let want = ground(&e, 1, 1);
		assert!(want.is_some(), "selected cell filled");
		assert_eq!(ground(&e, 2, 2), want, "whole selection filled");
		assert_eq!(ground(&e, 6, 6), None, "outside the selection untouched");
	}

	#[test]
	fn pass_editors_split_tile_passability_from_per_cell_overrides() {
		let mut e = editor();
		// Local Pass Override Editor: the pass overlay turns on; painting sets a
		// per-cell override, the eraser-driven clear lifts it.
		assert!(matches!(e.execute(Command::Mode { name: "localpass".into() }), Outcome::Redraw));
		assert_eq!(e.mode, EditorMode::LocalPass);
		assert!(e.show_pass_overlay);
		e.execute(Command::PassPaint { x: 1, y: 1, value: 3 });
		assert_eq!(e.project.pass_override(1, 1), Some(3));
		e.execute(Command::PassClear { x: 1, y: 1 });
		assert_eq!(e.project.pass_override(1, 1), None, "clear lifts the override");
		// Pass Table Editor: tile passability is tile-dependent - no per-cell
		// override is created, but the cell reads the new value.
		assert!(matches!(e.execute(Command::Mode { name: "pass".into() }), Outcome::Redraw));
		e.execute(Command::TilePass { x: 2, y: 2, value: 2 });
		assert_eq!(e.project.pass_override(2, 2), None, "tile pass is not a cell override");
		assert_eq!(e.project.pass_at(2, 2), Some(2), "the cell reads the tile's new pass");
	}

	#[test]
	fn pass_table_edit_queues_the_stock_pack_for_bake_only_in_dev() {
		let mut e = editor();
		// A stock GREEN tile under the cell, so the pass edit lands in GREEN's table.
		e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });

		// Without --dev the pass still edits in memory, but nothing is queued for
		// Bake (so it could never reach the shipped tiles.pass.json).
		e.execute(Command::TilePass { x: 1, y: 1, value: 3 });
		assert!(!e.tile_ops.dirty_packs.contains("GREEN"), "no --dev: pack not queued for bake");

		// With --dev, editing a stock tile's pass queues its pack - Bake then writes
		// tiles.pass.json (this was the missing link; the edit was lost before).
		e.dev_mode = true;
		assert!(matches!(e.execute(Command::TilePass { x: 1, y: 1, value: 1 }), Outcome::Redraw));
		assert!(e.tile_ops.dirty_packs.contains("GREEN"), "--dev: the affected pack is queued for bake");
		assert_eq!(e.project.pass_at(1, 1), Some(1), "the in-memory pass reflects the edit");

		// A no-op edit (same value) does not spuriously queue anything new.
		let mut e2 = editor();
		e2.dev_mode = true;
		e2.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });
		let current = e2.project.pass_at(1, 1).unwrap();
		e2.execute(Command::TilePass { x: 1, y: 1, value: current });
		assert!(!e2.tile_ops.dirty_packs.contains("GREEN"), "unchanged pass does not queue a bake");
	}

	#[test]
	fn reset_tile_pass_restores_the_tileset_values_and_undoes() {
		let mut e = editor();
		e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });
		// The tileset's canonical pass for GLa000 (a fresh load of GREEN).
		let fresh = map_core::TilePack::load(&e.assets_root, "GREEN").unwrap();
		let canonical = fresh.pass.as_ref().unwrap()[fresh.index_of["GLa000"] as usize];
		let edited = if canonical == 3 { 0 } else { 3 };

		// Edit the tile pass away from the tileset value.
		e.execute(Command::TilePass { x: 1, y: 1, value: edited });
		assert_eq!(e.project.pass_at(1, 1), Some(edited), "the edit took");

		// Reset restores the tileset value...
		assert!(matches!(e.execute(Command::ResetTilePass), Outcome::Redraw));
		assert_eq!(e.project.pass_at(1, 1), Some(canonical), "reset to the tileset pass");
		// ...as one undo unit (the edit comes back).
		assert!(e.project.undo(), "reset is undoable");
		assert_eq!(e.project.pass_at(1, 1), Some(edited), "undo restored the edit");

		// Resetting when already canonical is a quiet no-op.
		e.execute(Command::ResetTilePass);
		assert!(matches!(e.execute(Command::ResetTilePass), Outcome::Ok), "no-op when already at tileset");
		// Per-cell overrides are untouched by the reset.
		e.execute(Command::Mode { name: "localpass".into() });
		e.execute(Command::PassPaint { x: 1, y: 1, value: 2 });
		e.execute(Command::ResetTilePass);
		assert_eq!(e.project.pass_override(1, 1), Some(2), "reset leaves per-cell overrides alone");
	}

	#[test]
	fn opening_a_stock_map_keeps_its_origin_path_less() {
		let mut e = editor();
		let stock = e.resources_root.join("assets/maps/GREEN_1.json");
		assert!(stock.is_file(), "the shipped GREEN_1 map exists");
		e.execute(Command::Open { path: stock.clone() });
		// A shipped map loads path-less (so Save can't overwrite it) but keeps its
		// origin, so DEV ▸ Update Map can still write back to it.
		assert_eq!(e.path, None, "stock map is path-less (Save → Save As)");
		assert_eq!(e.origin.as_deref(), Some(stock.as_path()), "its origin is remembered");
	}

	#[test]
	fn update_map_overwrites_the_origin_only_in_dev() {
		let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/update-map-test");
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		let target = dir.join("stock.json");

		// Simulate a stock map: an origin to write back to, but path-less (Save off).
		let mut e = editor();
		e.origin = Some(target.clone());
		e.path = None;
		e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });

		// Without --dev it's refused and nothing is written.
		assert!(matches!(e.execute(Command::UpdateMap), Outcome::Failed(_)), "update-map needs --dev");
		assert!(!target.exists(), "nothing written without --dev");

		// With --dev it overwrites the origin and marks the project saved, without
		// adopting a save path (so plain Save stays protected).
		e.dev_mode = true;
		assert!(!matches!(e.execute(Command::UpdateMap), Outcome::Failed(_)), "update-map writes in --dev");
		assert!(target.is_file(), "the original file was written");
		assert!(!e.dirty(), "update-map marks the project saved");
		assert_eq!(e.path, None, "it does not adopt the path");

		// A map with no original file at all (New / WRL / image) is refused, even in --dev.
		let mut fresh = editor();
		fresh.dev_mode = true;
		assert!(matches!(fresh.execute(Command::UpdateMap), Outcome::Failed(_)), "no origin/path → refused");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn show_only_selected_layer_masks_the_view_to_the_active_layer() {
		let mut e = editor();
		// Off: every layer composites (all bits set).
		assert!(!e.show_only_layer);
		assert_eq!(e.layer_mask(), (1 << map_core::MAX_LAYERS) - 1);
		// On with the default active layer (ground) → ground only.
		assert!(matches!(e.execute(Command::ShowOnlyLayer { on: None }), Outcome::Redraw));
		assert!(e.show_only_layer);
		assert_eq!(e.layer_mask(), 1 << LAYER_GROUND);
		// Switching the active layer re-targets the filter, no extra toggle.
		e.execute(Command::Layer { name: "water".into() });
		assert_eq!(e.layer_mask(), 1 << LAYER_WATER);
		// Off restores the full mask.
		e.execute(Command::ShowOnlyLayer { on: Some(false) });
		assert_eq!(e.layer_mask(), (1 << map_core::MAX_LAYERS) - 1);
	}

	#[test]
	fn ctrl_click_builds_a_palette_multi_selection() {
		let mut e = editor();
		e.execute(Command::ColorToggle { index: 64 });
		e.execute(Command::ColorToggle { index: 70 });
		assert_eq!(e.palettes.multi, vec![64, 70]);
		assert_eq!(e.active_color, Some(70), "last toggled stays the focus");
		// Re-toggling removes a slot.
		e.execute(Command::ColorToggle { index: 64 });
		assert_eq!(e.palettes.multi, vec![70]);
		// A plain select clears the multi set; a shift-range too.
		e.execute(Command::Color { index: 100 });
		assert!(e.palettes.multi.is_empty());
		e.execute(Command::ColorToggle { index: 80 });
		e.execute(Command::ColorTo { index: 90 });
		assert!(e.palettes.multi.is_empty(), "shift-range clears multi");
	}

	#[test]
	fn convert_palette_guards_projects_and_converts_wrl_imports() {
		let convert = || Command::ConvertPalette { rasterize: false, water: true, relaxed: false, threshold: 0.05 };
		// A .json project doesn't own its tiles - loud refusal; the modal
		// opener refuses identically.
		let mut e = editor();
		assert!(matches!(e.execute(convert()), Outcome::Failed(_)));
		assert!(matches!(e.execute(Command::ConvertPaletteModal), Outcome::Failed(_)));

		// A WRL import with an off-spec static slot converts (DocReplaced -
		// the tile atlas must rebuild) and the cycler follows the new palette.
		let mut tiles = vec![0u8; max_assets::wrl::TILE_DATA_SIZE];
		tiles.fill(40);
		let mut palette = map_core::GAME_PALETTE.to_vec();
		palette[40 * 3..40 * 3 + 3].copy_from_slice(&[0xff, 0x00, 0xee]);
		let wrl = max_assets::wrl::WrlFile {
			header: vec![0; 5],
			width: 1,
			height: 1,
			minimap: vec![0],
			bigmap: vec![0],
			tile_count: 1,
			tiles,
			palette,
			pass_table: vec![0],
		};
		e.add_doc(Project::from_wrl(&wrl, "CONV"), None, None);
		assert!(matches!(e.execute(Command::ConvertPaletteModal), Outcome::Redraw));
		assert!(
			e.modal_as::<crate::convertpalette::ConvertPalette>().is_some(),
			"the options modal opens for WRL imports"
		);
		e.close_modal();
		assert!(matches!(e.execute(convert()), Outcome::DocReplaced));
		let to = e.project.packs[0].tiles[0] as usize;
		assert_eq!(&e.cycler.rgba()[to * 4..to * 4 + 3], &[0xff, 0x00, 0xee]);
		// Already compatible now - the second run is a no-op.
		assert!(matches!(e.execute(convert()), Outcome::Redraw));
		// Undo restores the document structurally (atlas rebuild) and the
		// cycler follows the restored (game-resolved) palette; redo too.
		assert!(matches!(e.execute(Command::Undo), Outcome::DocReplaced));
		assert!(e.project.packs[0].tiles.iter().all(|&b| b == 40));
		assert_eq!(&e.cycler.rgba()[40 * 4..40 * 4 + 3], &map_core::GAME_PALETTE[40 * 3..40 * 3 + 3]);
		assert!(matches!(e.execute(Command::Redo), Outcome::DocReplaced));
		assert_eq!(&e.cycler.rgba()[to * 4..to * 4 + 3], &[0xff, 0x00, 0xee]);
		// The rasterize method works through the same command (tiny map -
		// the synchronous re-import is instant here).
		let rast = Command::ConvertPalette { rasterize: true, water: true, relaxed: false, threshold: 0.05 };
		assert!(matches!(e.execute(rast), Outcome::DocReplaced));
		assert!(matches!(e.execute(Command::Undo), Outcome::DocReplaced));
	}

	#[test]
	fn rasterize_conversion_runs_stepped_with_progress_and_abort() {
		// The interactive path: modal → start → per-frame ticks → completion
		// swaps the document (DocReplaced) and closes the modal.
		let mut e = editor();
		let mut tiles = vec![0u8; max_assets::wrl::TILE_DATA_SIZE];
		tiles.fill(40);
		let mut palette = map_core::GAME_PALETTE.to_vec();
		palette[40 * 3..40 * 3 + 3].copy_from_slice(&[0xff, 0x00, 0xee]);
		let wrl = max_assets::wrl::WrlFile {
			header: vec![0; 5],
			width: 1,
			height: 1,
			minimap: vec![0],
			bigmap: vec![0],
			tile_count: 1,
			tiles,
			palette,
			pass_table: vec![0],
		};
		e.add_doc(Project::from_wrl(&wrl, "STEP"), None, None);
		e.execute(Command::ConvertPaletteModal);
		e.modal_as_mut::<crate::convertpalette::ConvertPalette>().unwrap().method =
			crate::convertpalette::Method::Rasterize;

		// An abort mid-run returns to the options with the session dropped.
		assert!(matches!(e.palette_convert_start(), Outcome::Redraw));
		assert!(e.palette_converting());
		assert!(matches!(e.palette_convert_tick(0.1, true), Outcome::Redraw));
		let m = e.modal_as::<crate::convertpalette::ConvertPalette>().unwrap();
		assert!(!m.running && m.session.is_none() && m.stage == "Aborted");
		assert!(e.project.packs[0].tiles.iter().all(|&b| b == 40), "abort leaves the document untouched");

		// A full run: bounded ticks make visible progress, completion swaps
		// the document as one undo unit and drops the modal.
		assert!(matches!(e.palette_convert_start(), Outcome::Redraw));
		let mut ticks = 0;
		let outcome = loop {
			ticks += 1;
			assert!(ticks < 10_000, "conversion never finished");
			match e.palette_convert_tick(ticks as f32 * 0.01, false) {
				Outcome::Redraw => continue,
				other => break other,
			}
		};
		assert!(matches!(outcome, Outcome::DocReplaced));
		assert!(e.modal_as::<crate::convertpalette::ConvertPalette>().is_none(), "the modal closes on completion");
		assert!(!e.project.packs[0].tiles.contains(&40), "pink re-quantized off the static slot");
		assert!(matches!(e.execute(Command::Undo), Outcome::DocReplaced), "one undo restores the document");
		assert!(e.project.packs[0].tiles.iter().all(|&b| b == 40));
	}

	#[test]
	fn delete_clears_selected_ground_without_clipboard() {
		let mut e = editor();
		e.execute(Command::Place { x: 1, y: 1, spec: "GSa000".into() });
		// Nothing selected → a loud no-op.
		assert!(matches!(e.execute(Command::Delete), Outcome::Failed(_)));
		e.execute(Command::SelectRect { x0: 1, y0: 1, x1: 1, y1: 1, mode: SelectMode::Replace });
		assert!(matches!(e.execute(Command::Delete), Outcome::Redraw));
		assert!(e.clipboard.is_none(), "delete is not cut");
		let spec = e.project.cell_spec(1, 1).unwrap_or_default();
		assert!(!spec.contains("GSa000"), "ground cleared: {spec}");
	}

	#[test]
	fn delete_clears_the_active_layer_delete_all_clears_both() {
		let mut e = editor();
		let has = |e: &EditorState, layer: usize| e.project.cell(1, 1).unwrap()[layer].is_some();
		// A cell with the water base + ground on top.
		e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });
		assert!(has(&e, LAYER_WATER) && has(&e, LAYER_GROUND), "starts with water + ground");
		e.execute(Command::SelectRect { x0: 1, y0: 1, x1: 1, y1: 1, mode: SelectMode::Replace });

		// On the ground layer (default), Delete lifts ground and keeps the water.
		assert!(matches!(e.execute(Command::Delete), Outcome::Redraw));
		assert!(!has(&e, LAYER_GROUND) && has(&e, LAYER_WATER), "ground gone, water base kept");

		// On the water layer, the same Delete drops the water - no land/water split.
		e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });
		e.execute(Command::Layer { name: "water".into() });
		assert!(matches!(e.execute(Command::Delete), Outcome::Redraw));
		assert!(has(&e, LAYER_GROUND) && !has(&e, LAYER_WATER), "water gone, ground kept");

		// Delete All empties every layer regardless of which one is active.
		assert!(matches!(e.execute(Command::DeleteAll), Outcome::Redraw));
		assert!(!has(&e, LAYER_GROUND) && !has(&e, LAYER_WATER), "all layers cleared → a true hole");

		// Both refuse an empty selection.
		e.execute(Command::SelectOp { op: "clear".into() });
		assert!(matches!(e.execute(Command::DeleteAll), Outcome::Failed(_)), "delete-all needs a selection");
	}

	#[test]
	fn quit_request_guards_unsaved_work() {
		let mut e = editor();
		// A fresh map is clean: a quit request goes straight through.
		assert!(!e.dirty());
		assert!(matches!(e.execute(Command::QuitRequest), Outcome::Quit));
		// Dirtying it makes the quit request raise the confirm instead of quitting.
		e.execute(Command::Place { x: 0, y: 0, spec: "GLa000".into() });
		assert!(e.dirty());
		assert!(matches!(e.execute(Command::QuitRequest), Outcome::Redraw));
		assert!(e.modal_as::<crate::confirm::ConfirmClose>().is_some(), "quit raises the Save/Discard/Cancel guard");
		// The quit confirm fires quit!/save-and-quit, not the tab-close commands.
		let c = crate::confirm::ConfirmClose::new_quit("x".into());
		assert_eq!((c.discard_line(), c.save_line()), ("quit!", "save-and-quit"));
	}

	#[test]
	fn tabs_stack_switch_and_close() {
		let mut e = editor();
		assert_eq!(e.tab_infos().len(), 1);
		// The first new replaces the bootstrap scratch tab (no stacking).
		new_tab(&mut e, 2);
		assert_eq!(e.tab_infos().len(), 1);
		// Subsequent new/open stack as tabs and activate the newest.
		new_tab(&mut e, 3);
		new_tab(&mut e, 4);
		assert_eq!(e.tab_infos().len(), 3);
		assert_eq!(e.active_tab(), 2);
		// Switching activates another tab; switching to the active one is a no-op.
		assert!(matches!(e.execute(Command::Tab { index: 0 }), Outcome::DocReplaced));
		assert_eq!(e.active_tab(), 0);
		assert!(matches!(e.execute(Command::Tab { index: 0 }), Outcome::Ok));
		// Closing drops a tab (these are clean new maps, so no prompt).
		e.execute(Command::CloseProject { force: false });
		assert_eq!(e.tab_infos().len(), 2);
		e.execute(Command::CloseProject { force: false });
		assert_eq!(e.tab_infos().len(), 1);
		// Closing the last project is allowed - it resets to a blank scratch
		// (one tab, replaceable by the next open/new), app stays open.
		assert!(matches!(e.execute(Command::CloseProject { force: false }), Outcome::DocReplaced));
		assert_eq!(e.tab_infos().len(), 1);
		assert!(e.tabs.replace_scratch);
	}

	#[test]
	fn nav_pan_and_zoom_move_the_view() {
		let mut e = editor();
		let pan0 = e.view.pan;
		e.execute(Command::Pan { dx: 3.0, dy: 2.0 });
		assert_eq!(e.view.pan[0] - pan0[0], 3.0 * TILE_PX as f32, "pan dx = 3 tiles");
		assert_eq!(e.view.pan[1] - pan0[1], 2.0 * TILE_PX as f32, "pan dy = 2 tiles");
		let z = e.view.zoom;
		e.execute(Command::Zoom { factor: 2.0 });
		assert!(e.view.zoom > z, "zoom in grows the zoom");
		e.execute(Command::Zoom { factor: 0.25 });
		assert!(e.view.zoom < 2.0 * z, "zoom out shrinks it back");
		e.execute(Command::Fit);
		assert!((ZOOM_MIN..=ZOOM_MAX).contains(&e.view.zoom), "fit stays in range");
	}

	#[test]
	fn overlay_flags_toggle_through_execute() {
		let mut e = editor();
		// on/off are explicit; a bare/None argument toggles (the unified flag rule).
		e.execute(Command::Grid { on: Some(true) });
		assert!(e.show_grid, "grid on");
		e.execute(Command::Grid { on: Some(false) });
		assert!(!e.show_grid, "grid off");
		e.execute(Command::Grid { on: None });
		assert!(e.show_grid, "grid toggle flips off -> on");
		let animate = e.animate;
		e.execute(Command::Animate { on: None });
		assert_eq!(e.animate, !animate, "animate toggles");
		e.execute(Command::Crt { on: Some(true) });
		assert!(e.crt, "crt on");
		e.execute(Command::PassOverlay { on: Some(true) });
		assert!(e.show_pass_overlay, "pass overlay on");
	}

	#[test]
	fn select_ops_set_the_mask() {
		let mut e = editor(); // 8x8 = 64 cells
		e.execute(Command::SelectOp { op: "all".into() });
		assert_eq!(e.selection.count(), 64, "select all");
		e.execute(Command::SelectOp { op: "clear".into() });
		assert_eq!(e.selection.count(), 0, "clear");
		e.execute(Command::SelectOp { op: "invert".into() });
		assert_eq!(e.selection.count(), 64, "invert of empty = all");
		e.execute(Command::SelectOp { op: "invert".into() });
		assert_eq!(e.selection.count(), 0, "invert of all = empty");
		assert!(matches!(e.execute(Command::SelectOp { op: "bogus".into() }), Outcome::Failed(_)), "unknown op fails");
	}

	#[test]
	fn set_color_writes_a_dynamic_slot_and_rejects_static() {
		let mut e = editor();
		assert!(matches!(e.execute(Command::SetColor { slot: 100, rgb: [0xaa, 0xbb, 0xcc] }), Outcome::Redraw));
		let at = 100 * 3;
		assert_eq!(&e.project.palette[at..at + 3], &[0xaa, 0xbb, 0xcc], "dynamic slot 100 written");
		// A game-static slot (outside the dynamic 64..=159 range) is refused.
		let out = e.execute(Command::SetColor { slot: 0, rgb: [1, 2, 3] });
		assert!(matches!(out, Outcome::Failed(_)), "static slot refused");
	}

	#[test]
	fn erase_clears_a_painted_ground_cell() {
		let mut e = editor();
		e.execute(Command::Tile { spec: Some("GLa000".into()) });
		e.execute(Command::Paint { x: 3, y: 3 });
		assert!(e.project.cell(3, 3).unwrap()[LAYER_GROUND].is_some(), "painted");
		e.execute(Command::Erase { x: 3, y: 3, layer: None });
		assert!(e.project.cell(3, 3).unwrap()[LAYER_GROUND].is_none(), "erased");
	}

	#[test]
	fn paint_fill_transform_and_hsl_drive_state() {
		let mut e = editor(); // 8×8 GREEN
		// Paint needs an active tile; with one it places onto the ground layer.
		assert!(matches!(e.execute(Command::Paint { x: 0, y: 0 }), Outcome::Failed(_)), "paint needs a tile");
		e.execute(Command::Tile { spec: Some("GLa000".into()) });
		assert!(matches!(e.execute(Command::Paint { x: 2, y: 2 }), Outcome::Redraw));
		assert!(e.project.cell(2, 2).unwrap()[LAYER_GROUND].is_some(), "paint placed the tile");

		// Fill floods the connected empty-ground region with the active tile.
		e.execute(Command::Fill { x: 0, y: 0 });
		let painted = (0..8u16)
			.flat_map(|y| (0..8u16).map(move |x| (x, y)))
			.filter(|&(x, y)| e.project.cell(x, y).unwrap()[LAYER_GROUND].is_some())
			.count();
		assert!(painted > 1, "fill spread to multiple cells (got {painted})");

		// Transform rotates the active paint tile; four cw turns are identity.
		e.execute(Command::Tile { spec: Some("GLa000".into()) });
		assert!(matches!(e.execute(Command::TransformTile { op: "cw".into() }), Outcome::Redraw));
		assert_ne!(e.active_tile.as_deref(), Some("GLa000"), "cw added a transform suffix");
		for _ in 0..3 {
			e.execute(Command::TransformTile { op: "cw".into() });
		}
		assert_eq!(e.active_tile.as_deref(), Some("GLa000"), "4× cw returns to identity");
		assert!(matches!(e.execute(Command::TransformTile { op: "bogus".into() }), Outcome::Failed(_)), "bad op fails");

		// HSL block shift darkens a dynamic slot; a game-static slot is refused.
		e.execute(Command::SetColor { slot: 100, rgb: [120, 120, 120] });
		let before = e.project.palette[100 * 3];
		assert!(matches!(e.execute(Command::HslBlock { slot: 100, dh: 0.0, ds: 0.0, dl: -40.0 }), Outcome::Redraw));
		assert!(e.project.palette[100 * 3] < before, "hsl-block -L darkened the slot");
		let static_shift = e.execute(Command::HslBlock { slot: 0, dh: 0.0, ds: 0.0, dl: 10.0 });
		assert!(matches!(static_shift, Outcome::Failed(_)), "static slot refused");
	}

	#[test]
	fn free_stem_in_bumps_on_collision() {
		let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/free-stem-test");
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		// An empty dir: the base name is free.
		assert_eq!(free_stem_in(&dir, "map", None), "map");
		// With `map.json` present it bumps to `map-2`, then `map-3`.
		std::fs::write(dir.join("map.json"), "{}").unwrap();
		assert_eq!(free_stem_in(&dir, "map", None), "map-2");
		std::fs::write(dir.join("map-2.json"), "{}").unwrap();
		assert_eq!(free_stem_in(&dir, "map", None), "map-3");
		// Excluding the colliding file (a rename keeping its own name) frees the base.
		assert_eq!(free_stem_in(&dir, "map", Some(&dir.join("map.json"))), "map");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn template_pack_joins_terrain_packs_and_excludes_water() {
		let pack = |uses: &str| {
			let json = format!(r#"{{"version":"1","name":"t","width":1,"height":1,"use":{uses},"map":[[""]]}}"#);
			template_pack(&Template::from_str(&json).unwrap())
		};
		assert_eq!(pack(r#"[{"name":"GREEN","version":"1"}]"#), "GREEN", "single pack → its name");
		// WATER is the universal base layer - excluded from the dir name.
		assert_eq!(pack(r#"[{"name":"WATER","version":"1"},{"name":"CRATER","version":"1"}]"#), "CRATER");
		// Multiple terrain packs: sorted, joined with `+` (regardless of order).
		assert_eq!(pack(r#"[{"name":"GREEN","version":"1"},{"name":"DESERT","version":"1"}]"#), "DESERT+GREEN");
		assert_eq!(pack(r#"[{"name":"WATER","version":"1"}]"#), "WATER", "only WATER → WATER");
		assert_eq!(pack("[]"), "MISC", "no packs → MISC");
	}

	#[test]
	fn template_rename_name_uniqueness_is_per_tileset() {
		let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/rename-tileset-test");
		let _ = std::fs::remove_dir_all(&dir);
		let (a, b) = (dir.join("A"), dir.join("B"));
		std::fs::create_dir_all(&a).unwrap();
		std::fs::create_dir_all(&b).unwrap();
		let mk = |dir: &Path, name: &str, pack: &str| {
			let t = Template {
				name: name.to_string(),
				width: 1,
				height: 1,
				uses: vec![(pack.to_string(), "1".to_string())],
				cells: vec![String::new()],
			};
			t.save(&dir.join(format!("{name}.json"))).unwrap();
			TemplateEntry { name: t.name.clone(), path: dir.join(format!("{name}.json")), stock: false, template: t }
		};
		let mut e = editor();
		// Tileset A holds "Shared" + "Taken"; tileset B holds another "Shared".
		e.templates.entries = vec![mk(&a, "Shared", "A"), mk(&b, "Shared", "B"), mk(&a, "Taken", "A")];

		// Renaming A's "Shared" onto "Taken" (same tileset) is rejected...
		e.templates.sel = Some(0);
		assert!(
			matches!(
				e.execute(Command::TemplateRename { from: "Shared".into(), to: "Taken".into() }),
				Outcome::Failed(_)
			),
			"same-tileset name collision is rejected",
		);

		// ...but the *selected* duplicate is the one renamed (not the first by name),
		// and a target that only exists in another tileset is allowed. Rename B's
		// "Shared" (index 1) → "Taken": B has no "Taken", so it succeeds and touches
		// B, leaving A's "Shared" alone.
		e.templates.sel = Some(1);
		assert!(
			!matches!(
				e.execute(Command::TemplateRename { from: "Shared".into(), to: "Taken".into() }),
				Outcome::Failed(_)
			),
			"a name used only in another tileset is allowed",
		);
		assert!(b.join("taken.json").exists(), "B's Shared was renamed (sanitized filename)");
		assert!(!b.join("Shared.json").exists(), "B's old file is gone");
		assert!(a.join("Shared.json").exists(), "A's Shared was untouched - the selected dup was renamed");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn template_pick_arms_the_selected_entry_not_the_first_by_name() {
		// Two templates share the display name "Shared" but belong to different
		// tilesets (packs A and B). Empty cells → both compatible with any map, so
		// resolution - not `missing_id` - is what's under test. The explorer arms
		// the exact entry clicked, so `template-pick` must honour the selection
		// rather than grabbing the first "Shared" by entry order.
		let mk = |name: &str, pack: &str| {
			let t = Template {
				name: name.to_string(),
				width: 1,
				height: 1,
				uses: vec![(pack.to_string(), "1".to_string())],
				cells: vec![String::new()],
			};
			TemplateEntry {
				name: t.name.clone(),
				path: PathBuf::from(format!("{pack}/{name}.json")),
				stock: false,
				template: t,
			}
		};
		let mut e = editor();
		e.templates.entries = vec![mk("Shared", "A"), mk("Shared", "B")];

		// Selecting B's "Shared" (index 1) arms B's template, not A's (index 0).
		e.templates.sel = Some(1);
		assert!(!matches!(e.execute(Command::TemplatePick { name: "Shared".into() }), Outcome::Failed(_)));
		assert_eq!(e.stamp.as_ref().unwrap().uses[0].0, "B", "the selected entry is armed");
		assert_eq!(e.templates.sel, Some(1), "selection stays on the picked entry");

		// With no matching selection, the scripted path falls back to first-by-name.
		e.stamp = None;
		e.templates.sel = None;
		assert!(!matches!(e.execute(Command::TemplatePick { name: "Shared".into() }), Outcome::Failed(_)));
		assert_eq!(e.stamp.as_ref().unwrap().uses[0].0, "A", "no selection → first match (scripted path)");
	}

	#[test]
	fn template_context_items_adapt_to_stock_vs_user() {
		let labels = |items: &[menu::Item]| -> Vec<String> {
			items
				.iter()
				.filter_map(|it| match it {
					menu::Item::Action { label, .. } => Some(label.clone()),
					_ => None,
				})
				.collect()
		};
		let mk = |name: &str, stock: bool| TemplateEntry {
			name: name.into(),
			path: PathBuf::from(format!("{name}.json")),
			stock,
			template: Template {
				name: name.into(),
				width: 1,
				height: 1,
				uses: vec![("GREEN".into(), String::new())],
				cells: vec!["GLa000".into()],
			},
		};
		let mut e = editor();
		e.templates.entries = vec![mk("mine", false), mk("shipped", true)];

		// A user template: Use, Rename, Duplicate, Delete, Export as PNG.
		e.templates.sel = Some(0);
		let user = labels(&e.template_context_items());
		for want in ["Use", "Rename", "Duplicate", "Delete", "Export as PNG"] {
			assert!(user.iter().any(|l| l == want), "user menu has {want}: {user:?}");
		}
		// A stock template is read-only: no Rename/Delete, but Duplicate + Export stay.
		e.templates.sel = Some(1);
		let stock = labels(&e.template_context_items());
		assert!(!stock.iter().any(|l| l == "Rename"), "stock can't be renamed");
		assert!(!stock.iter().any(|l| l == "Delete"), "stock can't be deleted");
		for want in ["Use", "Duplicate", "Export as PNG"] {
			assert!(stock.iter().any(|l| l == want), "stock menu has {want}: {stock:?}");
		}
		// --dev unlocks the stock template: Rename + Delete come back.
		e.dev_mode = true;
		let dev_stock = labels(&e.template_context_items());
		for want in ["Use", "Rename", "Duplicate", "Delete", "Export as PNG"] {
			assert!(dev_stock.iter().any(|l| l == want), "dev stock menu has {want}: {dev_stock:?}");
		}
	}

	#[test]
	fn dev_mode_unlocks_stock_template_rename_and_delete() {
		let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/dev-stock-template-test");
		let _ = std::fs::remove_dir_all(&dir);
		let pack_dir = dir.join("GREEN");
		std::fs::create_dir_all(&pack_dir).unwrap();
		let t = Template {
			name: "ridge".into(),
			width: 1,
			height: 1,
			uses: vec![("GREEN".into(), String::new())],
			cells: vec!["GLa000".into()],
		};
		let path = pack_dir.join("ridge.json");
		t.save(&path).unwrap();
		let entry = || TemplateEntry { name: "ridge".into(), path: path.clone(), stock: true, template: t.clone() };

		// Without --dev: the rename/delete modals and the delete itself are refused,
		// and the stock file is left on disk.
		let mut e = editor();
		e.templates.entries = vec![entry()];
		e.templates.sel = Some(0);
		assert!(matches!(e.execute(Command::TemplateRenameModal), Outcome::Failed(_)), "no --dev: rename refused");
		assert!(matches!(e.execute(Command::TemplateDeleteModal), Outcome::Failed(_)), "no --dev: delete refused");
		assert!(matches!(e.execute(Command::TemplateDelete { name: None }), Outcome::Failed(_)));
		assert!(path.exists(), "the stock file survives without --dev");

		// With --dev: the modal opener no longer refuses, and the delete removes the
		// stock file. (A fresh editor so the opened modal doesn't linger.)
		let mut e = editor();
		e.dev_mode = true;
		e.templates.entries = vec![entry()];
		e.templates.sel = Some(0);
		assert!(matches!(e.execute(Command::TemplateRenameModal), Outcome::Redraw), "--dev: rename opens");
		assert!(!matches!(e.execute(Command::TemplateDelete { name: None }), Outcome::Failed(_)), "--dev: delete runs");
		assert!(!path.exists(), "--dev removed the stock template file");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn template_export_png_writes_one_image_cell_per_template_cell() {
		let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/template-png-test");
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		let mut e = editor();
		// A real 2x1 template: ground over the project's actual water spec, then bare
		// ground - so every id resolves and the tiles rasterize.
		let water = e.project.cell_spec(0, 0).unwrap();
		let t = Template {
			name: "ridge".into(),
			width: 2,
			height: 1,
			uses: vec![("GREEN".into(), String::new()), ("WATER".into(), String::new())],
			cells: vec![format!("{water},GLa000"), "GLa001".into()],
		};
		e.templates.entries =
			vec![TemplateEntry { name: t.name.clone(), path: dir.join("ridge.json"), stock: false, template: t }];

		// No selection → refused; the bare command (no path) just opens the dialog,
		// which is unavailable headless.
		e.templates.sel = None;
		assert!(matches!(e.execute(Command::TemplateExportPng { path: None }), Outcome::Failed(_)));

		e.templates.sel = Some(0);
		let png = dir.join("ridge.png");
		assert!(matches!(e.execute(Command::TemplateExportPng { path: Some(png.clone()) }), Outcome::Redraw));
		let (rgba, w, h) = decode_png_rgba(&png).expect("decode the exported png");
		assert_eq!((w, h), (2 * 64, 64), "one 64px image cell per template cell");
		assert!(rgba.chunks_exact(4).any(|p| p[3] == 255), "the ground tiles rasterize opaque pixels");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn save_then_open_round_trips_the_project_on_disk() {
		let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/save-roundtrip-test");
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		let path = dir.join("m.json");

		let mut e = editor();
		e.execute(Command::Tile { spec: Some("GLa000".into()) });
		e.execute(Command::Paint { x: 1, y: 1 });
		e.execute(Command::SetColor { slot: 100, rgb: [0x12, 0x34, 0x56] }); // a palette override to carry
		let saved_hash = e.project.hash();
		assert!(matches!(e.execute(Command::Save { path: Some(path.clone()) }), Outcome::Ok | Outcome::Redraw));
		assert!(path.exists(), "the project file was written");
		assert!(!e.dirty(), "save cleared the dirty flag");

		// Reload into a fresh editor: the document hashes identically.
		let mut e2 = editor();
		e2.execute(Command::Open { path });
		assert_eq!(e2.project.hash(), saved_hash, "reloaded project matches what was saved");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn dirty_tab_guards_close_and_quit() {
		let mut e = editor();
		new_tab(&mut e, 2); // replaces scratch
		new_tab(&mut e, 3); // second tab, active
		// Dirty the active tab.
		e.execute(Command::Place { x: 0, y: 0, spec: "GSa000".into() });
		assert!(e.dirty());
		// Closing a dirty tab opens the Save/Discard/Cancel confirm modal.
		assert!(matches!(e.execute(Command::CloseProject { force: false }), Outcome::Redraw));
		assert!(e.modal_as::<crate::confirm::ConfirmClose>().is_some());
		e.close_modal();
		// Discard (`close-project!`) closes despite the unsaved changes.
		assert!(matches!(e.execute(Command::CloseProject { force: true }), Outcome::DocReplaced));
		assert!(e.modal_as::<crate::confirm::ConfirmClose>().is_none());
		// Quit guards on ANY open tab being dirty.
		e.execute(Command::Place { x: 1, y: 1, spec: "GSa000".into() });
		assert!(matches!(e.execute(Command::Quit { force: false }), Outcome::Failed(_)));
		assert!(matches!(e.execute(Command::Quit { force: true }), Outcome::Quit));
	}

	#[test]
	fn reopening_a_path_switches_instead_of_stacking() {
		let mut e = editor();
		// Two distinct in-memory tabs first (path-less new maps never dedup).
		new_tab(&mut e, 2);
		new_tab(&mut e, 3);
		assert_eq!(e.tab_infos().len(), 2);
		// Switching keeps per-tab state independent: dirty one, switch away, back.
		e.execute(Command::Place { x: 0, y: 0, spec: "GSa000".into() });
		assert!(e.dirty());
		e.execute(Command::Tab { index: 0 });
		assert!(!e.dirty(), "tab 0 is its own clean document");
		e.execute(Command::Tab { index: 1 });
		assert!(e.dirty(), "tab 1's edit survived the switch");
	}

	#[test]
	fn ui_scale_shrinks_the_logical_ui_size() {
		// `ui_screen` is what the chrome lays out in: physical / scale. (Set the
		// field directly rather than `set_ui_scale`, which also writes a process
		// global the parallel font tests read.)
		let mut e = editor();
		assert_eq!(e.ui_scale, 1.0);
		assert_eq!(e.ui_screen(), (800.0, 600.0)); // 1.0: logical == physical
		e.ui_scale = 1.25;
		assert_eq!(e.ui_screen(), (640.0, 480.0)); // 125% of an 800×600 target
		e.ui_scale = 1.5;
		let (lw, lh) = e.ui_screen();
		assert!((lw - 533.333).abs() < 0.01 && (lh - 400.0).abs() < 0.01, "150%: {lw}×{lh}");
	}

	#[test]
	fn dialog_path_policy_follows_purpose() {
		use crate::command::FilePurpose::*;
		let tmp = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/dialog-policy-test");
		let _ = std::fs::remove_dir_all(&tmp);
		let res = tmp.join("resources");
		let doc = PathBuf::from("/maps/proj/forest.json");
		let user_templates = res.join("user/templates");

		// Palette purposes always land in (and create) user/palettes.
		let pal = dialog_default_dir(SavePalette, &res, None, None, None);
		assert_eq!(pal, res.join("user/palettes"));
		assert!(pal.is_dir(), "palette dir is created on first use");
		// Templates land in (and create) the user templates dir.
		assert_eq!(dialog_default_dir(ImportTemplate, &res, None, None, Some(&user_templates)), user_templates);
		assert!(user_templates.is_dir());
		// Maps: the open doc's folder wins; with no doc, Load falls back to
		// assets/maps (not created), Save to resources/maps (created).
		assert_eq!(dialog_default_dir(Load, &res, Some(&doc), None, None), Path::new("/maps/proj"));
		assert_eq!(dialog_default_dir(Load, &res, None, None, None), res.join("assets/maps"));
		assert_eq!(dialog_default_dir(SaveAs, &res, None, None, None), res.join("maps"));
		assert!(res.join("maps").is_dir(), "save destination is created");

		// Suggested names ensure a `.json` extension; only save-style purposes pre-fill.
		assert_eq!(dialog_suggested_name(SaveAs, Some(&doc), "Untitled").as_deref(), Some("forest.json"));
		assert_eq!(dialog_suggested_name(SaveCopy, None, "My Map").as_deref(), Some("My Map.json"));
		assert_eq!(dialog_suggested_name(SavePalette, None, "swamp").as_deref(), Some("swamp.json"));
		assert_eq!(dialog_suggested_name(Load, Some(&doc), "x"), None);

		let _ = std::fs::remove_dir_all(&tmp);
	}
}
