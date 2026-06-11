//! Editor state + the single command mutator.
//!
//! `EditorState` owns the document and the viewport; `execute` is the only
//! place either is mutated. GPU-side effects (screenshot, quit) are returned
//! as `Outcome`s for the shell (windowed or headless) to act on.

use std::path::{Path, PathBuf};

use map_core::{LAYER_GROUND, LAYER_WATER, Project, Rng, SelectMode, Selection, Template, clear_selection_ground};
use max_assets::wrl::{read_wrl_file, write_wrl_file};

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

/// Read a PNG's dimensions from its header only — no pixel decode (the
/// New-from-Image modal opens instantly; pixels load at Convert).
fn png_dimensions(path: &Path) -> Result<(u32, u32), String> {
	let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
	let reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().map_err(|e| e.to_string())?;
	let info = reader.info();
	Ok((info.width, info.height))
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
		return Err(format!("{:?} PNG unsupported — re-export as 8-bit", info.bit_depth));
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

/// Decode the modal's image and build a conversion session from its settings —
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

	/// Fit the map into a screen-space rect (the workspace's center area —
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

/// The active map tool — what LMB does on the map.
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
	/// Stamp a unit preview at the clicked cell (Units panel — palette aid).
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

/// Editor mode (Mode menu) — what the map surface edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
	/// Tile painting — pencil/picker/transform/shore.
	Map,
	/// Pass Table editing — LMB paints the active pass value.
	Pass,
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

/// One console line for a finished generation run — shared by the
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

/// The same report split into short lines for the Generate modal — a single
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
	/// The document was replaced (`open`) — renderer must be rebuilt.
	DocReplaced,
	Screenshot {
		path: PathBuf,
		crop: Option<(u32, u32, u32, u32)>,
		resize: Option<(u32, u32)>,
	},
	Quit,
	Failed(String),
}

/// One open project's per-tab state. The **active** document's state
/// lives directly on [`EditorState`] (`project`/`path`/`view`/…) so the editor
/// body needn't thread an index through every access; inactive tabs are parked
/// here and swapped in on a tab switch. The cycler is re-derived from the
/// project's palette on restore, so it isn't stored.
struct Document {
	project: Project,
	path: Option<PathBuf>,
	view: View,
	active_tile: Option<String>,
	active_color: Option<u8>,
}

pub struct EditorState {
	/// The **active** in-memory document. A `.json` loads directly; a
	/// `.WRL` is imported via `Project::from_wrl` (a synthetic in-memory
	/// pack). Everything — render, edit, save, export — goes through it.
	/// Other open projects are parked in `tabs`.
	pub project: Project,
	pub view: View,
	/// Render-target size in px (window inner size / `--size` headless).
	pub screen: (u32, u32),
	/// Where the document came from / was last saved to.
	pub path: Option<PathBuf>,
	/// Where tile packs live (`resources/assets`).
	pub assets_root: PathBuf,
	/// Where all settings persist (`--settings`, or `config/mme.ini`); `None`
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
	/// Cell grid overlay on?
	pub show_grid: bool,
	/// Pass-value overlay on? — auto-on in Pass Table Editor mode.
	pub show_pass_overlay: bool,
	pub console: Console,
	/// Live pointer snapshot (cursor + held press) for widget hover/pressed
	/// rendering — written by the shell from winit events, read by the views.
	/// Stays inert (`Hot::NONE`) in headless runs, so captures are mouse-free.
	pub hot: crate::ui::Hot,
	/// Dockable panels around the map view.
	pub workspace: Workspace,
	/// The main menu bar.
	pub menu: MenuBar,
	/// The right-click context menu, when open — items snapshot the state
	/// at open time (selection, clipboard, stamp, the cell under the click).
	pub context_menu: Option<menu::ContextMenu>,
	/// Shortcut hints from the loaded bindings: normalized command line →
	/// chord label (`"copy"` → `"Ctrl+C"`). Set once by the shell; menus and
	/// the context menu annotate their items from it.
	shortcut_hints: Vec<(String, String)>,
	/// The Create New Map modal, when open.
	pub newmap: Option<NewMap>,
	/// Headless run (`--headless`/`--screenshot`): native dialogs can't open.
	pub headless: bool,
	/// The Resize Map modal, when open.
	pub resize: Option<crate::resize::Resize>,
	/// The Auto Fix Shore modal, when open.
	pub autofix: Option<crate::autofix::AutoFix>,
	/// The Generate Random Terrain modal, when open.
	pub generator: Option<crate::generator::Generator>,
	/// The close-project Save/Discard/Cancel confirm modal.
	pub confirm: Option<crate::confirm::ConfirmClose>,
	/// The error modal — raised by the shell when a command fails so the
	/// reason is in front of the user, not buried in the console.
	pub error: Option<crate::errormodal::ErrorModal>,
	/// The New-from-Image modal, when open.
	pub newfromimage: Option<crate::newfromimage::NewFromImage>,
	/// Tile Explorer state: filter / display size / scroll.
	pub picker: PickerState,
	/// Minimap source: overworld / pass / in-game minimap.
	pub minimap_mode: minimap::Mode,
	/// Unit sprite library from the user's MAX.RES (`None` until loaded —
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
	/// Team color for new previews (0..5 — red green blue gray yellow).
	pub unit_team: u8,
	/// Units panel scroll (px, clamped at draw time).
	pub units_scroll: f32,
	/// The selected-cell mask (editor state, never in the undo journal) —
	/// the select tools edit it; copy/cut and template capture read it.
	pub selection: Selection,
	/// A live rect-select drag's preview `(x0, y0, x1, y1)` in cells — set
	/// by the shell while dragging, drawn as a dashed-intent outline.
	pub select_preview: Option<(u16, u16, u16, u16)>,
	/// The copy/cut clipboard (a transient unnamed template).
	pub clipboard: Option<Template>,
	/// The armed ghost stamp riding under the cursor (paste or a picked
	/// template); a map click places it, Esc disarms.
	pub stamp: Option<Template>,
	/// Templates known to the explorer (stock + user), rescanned on changes.
	pub templates: Vec<TemplateEntry>,
	/// Templates Explorer scroll (px, clamped at draw time).
	pub templates_scroll: f32,
	/// The explorer's selected template (index into `templates`).
	pub template_sel: Option<usize>,
	/// Open projects, in tab order. The active tab's slot is `None`
	/// (its live state is the fields above); every other slot is `Some`.
	tabs: Vec<Option<Document>>,
	/// Index into `tabs` of the active document.
	active: usize,
	/// The bootstrap (empty) document is replaced by the first `open`/`new`
	/// rather than stacked — so the editor starts with one real tab, not two.
	replace_scratch: bool,
	/// The active map tool: pencil paints, picker eyedrops.
	pub tool: Tool,
	/// Randomize-variants toggle: when on, painting/filling places a
	/// random sibling from the tile's `tiles.variants.json` group.
	pub randomize: bool,
	/// RNG for the randomize toggle — fixed-seeded so a replayed script paints
	/// the same "random" sequence (scripts/tests stay reproducible).
	paint_rng: Rng,
	/// Active edit layer: paint + erase act only on it. Default
	/// Ground (the detail layer over the water base).
	pub active_layer: usize,
	/// Editor mode: tile painting vs pass-table painting.
	pub mode: EditorMode,
	/// Active pass value for the Pass Table Editor (0..3).
	pub active_pass: u8,
	/// Selected palette slot in the Color Palette panel — the anchor of
	/// a multi-select range.
	pub active_color: Option<u8>,
	/// The far end of a shift-click palette selection range; `None` = a
	/// single slot. The selection is `active_color..=palette_sel_end` (ordered).
	pub palette_sel_end: Option<u8>,
	/// Color Palette grid scroll (px, clamped at draw time).
	pub palette_scroll: f32,
	/// Toolbox scroll (px, clamped at draw time) — the toolbox flows tall and
	/// scrolls when it doesn't fit.
	pub toolbox_scroll: f32,
	/// Color Palette panel tab: false = the grid, true = the saved-palettes
	/// list.
	pub palette_show_saved: bool,
	/// Saved/installed palette files for the "saved" tab — scanned on
	/// switching to it.
	pub palette_files: Vec<PathBuf>,
	/// The tile spec `paint` stamps — set by the `tile` command or
	/// a Tile Explorer click. Resolved per paint, so it re-validates
	/// after document switches.
	active_tile: Option<String>,
	clock: f32,
}

/// One template known to the explorer: where it lives and the parsed file.
/// Stock entries (shipped under `resources/stock/templates`) can be picked
/// and cloned but never deleted; user entries live in
/// `resources/user/templates`.
pub struct TemplateEntry {
	pub name: String,
	pub path: PathBuf,
	pub stock: bool,
	pub template: Template,
}

impl EditorState {
	pub fn new(project: Project, screen: (u32, u32), path: Option<PathBuf>, assets_root: PathBuf) -> Self {
		let (project_w, project_h) = (project.width, project.height);
		let view = View::fit((project.width, project.height), screen.0 as f32, screen.1 as f32);
		let cycler = PaletteCycler::from_rgb(&project.palette);
		// Quick Load lists the shipped read-only templates, not the user dir.
		let templates_dir = assets_root.parent().map(|p| p.join("templates")).unwrap_or_default();
		let mut workspace = Workspace::default();
		// The menu bar + project tab strip reserve the top strip.
		workspace.top = menu::BAR_H + crate::tabs::BAR_H;
		let mut s = Self {
			project,
			view,
			screen,
			path,
			assets_root,
			settings_path: None,
			max_path: None,
			cycler,
			animate: false,
			ingame: false,
			crt: false,
			show_grid: false,
			show_pass_overlay: false,
			console: Console::new(),
			hot: crate::ui::Hot::NONE,
			menu: MenuBar::new(&templates_dir),
			context_menu: None,
			shortcut_hints: Vec::new(),
			newmap: None,
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
			templates: Vec::new(),
			templates_scroll: 0.0,
			template_sel: None,
			resize: None,
			autofix: None,
			generator: None,
			confirm: None,
			error: None,
			newfromimage: None,
			workspace,
			picker: PickerState::default(),
			minimap_mode: minimap::Mode::Overworld,
			tabs: vec![None], // one tab; the active live fields above are its state
			active: 0,
			replace_scratch: true,
			tool: Tool::Pencil,
			active_layer: LAYER_GROUND,
			randomize: false,
			paint_rng: Rng::new(0x004d_4158_5f56_4152), // "MAX_VAR"
			mode: EditorMode::Map,
			active_pass: 1,
			active_color: None,
			palette_sel_end: None,
			palette_scroll: 0.0,
			toolbox_scroll: 0.0,
			palette_show_saved: false,
			palette_files: Vec::new(),
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

	/// Re-seed the cycling palette after a project palette edit (or its
	/// undo/redo) so the working palette + GPU upload follow.
	fn refresh_palette(&mut self) {
		self.cycler = PaletteCycler::from_rgb(&self.project.palette);
		self.cycler.set_ingame(self.ingame);
	}

	/// Scan the installed-tileset (`resources/assets/*/palette.json`) and
	/// user (`resources/palettes/*.json`) palettes for the "saved" tab.
	fn scan_palette_files(&mut self) {
		let mut files = Vec::new();
		if let Ok(rd) = std::fs::read_dir(&self.assets_root) {
			let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
			dirs.sort();
			files.extend(dirs.into_iter().map(|d| d.join("palette.json")).filter(|p| p.is_file()));
		}
		if let Some(pal_dir) = self.assets_root.parent().map(|p| p.join("palettes")) {
			if let Ok(rd) = std::fs::read_dir(&pal_dir) {
				let mut jsons: Vec<PathBuf> = rd
					.flatten()
					.map(|e| e.path())
					.filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")))
					.collect();
				jsons.sort();
				files.extend(jsons);
			}
		}
		self.palette_files = files;
	}

	/// Display names for the saved-palette list: a tileset `palette.json` shows
	/// its tileset (parent) name; a user palette shows its file stem.
	pub fn palette_file_names(&self) -> Vec<String> {
		self.palette_files
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

	/// The active edit layer's name (`"water"`/`"ground"`) — for the eraser
	/// tool's `Erase` command and the toolbox highlight.
	pub fn active_layer_name(&self) -> &'static str {
		if self.active_layer == LAYER_WATER { "water" } else { "ground" }
	}

	/// Resize the render target, keeping the world point under the old
	/// viewport centre still centred — so a window resize doesn't drift the
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
	/// `tick` when headless — same code path, deterministic under scripts).
	pub fn tick(&mut self, dt: f32) {
		self.clock += dt;
		self.cycler.tick(self.clock);
	}

	/// The open modal, if any — the shell routes input through it (see
	/// `crate::modal`). They're mutually exclusive. Auto Fix Shore joins the
	/// rest here, but its Start/Stop drive a live run, not a command line.
	pub fn active_modal(&mut self) -> Option<&mut dyn crate::modal::Modal> {
		// The error modal sits on top of whatever raised it.
		if let Some(m) = self.error.as_mut() {
			return Some(m);
		}
		if let Some(m) = self.newmap.as_mut() {
			return Some(m);
		}
		if let Some(m) = self.resize.as_mut() {
			return Some(m);
		}
		if let Some(m) = self.autofix.as_mut() {
			return Some(m);
		}
		if let Some(m) = self.generator.as_mut() {
			return Some(m);
		}
		if let Some(m) = self.confirm.as_mut() {
			return Some(m);
		}
		if let Some(m) = self.newfromimage.as_mut() {
			return Some(m);
		}
		None
	}

	/// Dismiss whichever modal is open.
	pub fn close_modal(&mut self) {
		self.newmap = None;
		self.resize = None;
		self.autofix = None;
		self.generator = None;
		self.confirm = None;
		self.error = None;
		self.newfromimage = None;
	}

	/// Raise the error modal with `message` (the shell calls this on a failed
	/// command). Also mirrored to the console for the scrollback.
	pub fn raise_error(&mut self, message: &str) {
		self.console.push_line(format!("error: {message}"));
		self.error = Some(crate::errormodal::ErrorModal::new(message));
	}

	/// Whether the Auto Fix Shore run is live (the shell keeps redrawing +
	/// ticking it while so).
	pub fn autofix_running(&self) -> bool {
		self.autofix.as_ref().is_some_and(|a| a.running)
	}

	/// Begin an Auto Fix Shore run with the modal's chosen mode.
	pub fn autofix_start(&mut self) {
		let Some(strength) = self.autofix.as_ref().map(|a| a.mode.strength()) else { return };
		let session = self.project.fix_session(None, strength);
		let found = session.found();
		if let Some(af) = self.autofix.as_mut() {
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
		let Some(mut af) = self.autofix.take() else { return Outcome::Ok };
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
		self.autofix = Some(af);
		outcome
	}

	/// Whether a terrain generation run is live (the shell keeps redrawing +
	/// stepping it while so).
	pub fn generate_running(&self) -> bool {
		self.generator.as_ref().is_some_and(|g| g.running)
	}

	/// Begin a generation run from the modal's settings. An empty
	/// seed field rolls a fresh seed (reported, so the map can be re-made).
	pub fn generate_start(&mut self) -> Outcome {
		let Some(modal) = self.generator.as_ref() else { return Outcome::Ok };
		let (mut params, seed) = match modal.params() {
			Ok(p) => p,
			Err(e) => return Outcome::Failed(format!("generate: {e}")),
		};
		params.seed = seed.unwrap_or_else(|| {
			std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
		});
		match map_core::GenSession::new(&self.project, params) {
			Ok(session) => {
				let modal = self.generator.as_mut().expect("generator modal checked above");
				modal.session = Some(session);
				modal.started = Some(params);
				modal.running = true;
				modal.status = vec![format!("seed {}", params.seed)];
				Outcome::Redraw
			}
			Err(e) => Outcome::Failed(format!("generate: {e}")),
		}
	}

	/// Step (or abort) the live generation run — the shell calls this per
	/// frame within a time budget. Completion reports to the console; an
	/// abort rolls the document back to before the run.
	pub fn generate_tick(&mut self, abort: bool) -> Outcome {
		let Some(mut modal) = self.generator.take() else { return Outcome::Ok };
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
		self.generator = Some(modal);
		Outcome::Redraw
	}

	/// Whether the New-from-Image conversion is live (the shell keeps redrawing
	/// + stepping it while so).
	pub fn converting(&self) -> bool {
		self.newfromimage.as_ref().is_some_and(|m| m.running)
	}

	/// Begin the New-from-Image conversion. Validates the settings up front, but
	/// defers loading the image pixels to the first `convert_tick` (shown as the
	/// "Loading image" stage), so a click on Convert is instant.
	pub fn convert_start(&mut self) -> Outcome {
		let Some(m) = self.newfromimage.as_mut() else { return Outcome::Ok };
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
		let Some(mut m) = self.newfromimage.take() else { return Outcome::Ok };
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
							// Modal done — open the new tab (drops `m`).
							return self.add_doc(project, None);
						}
						Err(e) => {
							m.stage = format!("Failed: {e}");
							outcome = Outcome::Failed(format!("convert: {e}"));
						}
					}
				}
			}
		}
		self.newfromimage = Some(m);
		outcome
	}

	/// Whether `path` is one of the shipped read-only template maps
	/// (`resources/templates/`). Those load path-less so a Save never
	/// overwrites them (Save → Save-As), same as an imported WRL.
	fn is_template(&self, path: &Path) -> bool {
		self.assets_root.parent().map(|p| p.join("templates")).is_some_and(|t| path.starts_with(t))
	}

	/// Window title: `<map name>[*] — M.A.X. Map Editor`.
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
		format!("{name}{star} — M.A.X. Map Editor")
	}

	// ----- multi-project tabs --------------------------------------

	/// The active tab index.
	pub fn active_tab(&self) -> usize {
		self.active
	}

	/// `(label, dirty)` for each open project, in tab order — the tab strip.
	pub fn tab_infos(&self) -> Vec<(String, bool)> {
		(0..self.tabs.len()).map(|i| (self.name_at(i), self.dirty_at(i))).collect()
	}

	/// Whether tabs show a close `x`: false for the lone blank scratch (the
	/// "no project open" state — nothing to close).
	pub fn tabs_closable(&self) -> bool {
		!(self.replace_scratch && self.tabs.len() == 1)
	}

	/// Any open project has unsaved changes — the quit guard.
	fn any_dirty(&self) -> bool {
		self.project.dirty() || self.tabs.iter().flatten().any(|d| d.project.dirty())
	}

	/// The save path of tab `i` (the active tab reads the live field).
	fn path_at(&self, i: usize) -> Option<&Path> {
		if i == self.active { self.path.as_deref() } else { self.tabs[i].as_ref()?.path.as_deref() }
	}

	/// The dirty flag of tab `i`.
	fn dirty_at(&self, i: usize) -> bool {
		if i == self.active { self.project.dirty() } else { self.tabs[i].as_ref().is_some_and(|d| d.project.dirty()) }
	}

	/// Tab `i`'s label: the save file name, else the project's own name.
	fn name_at(&self, i: usize) -> String {
		let (path, project_name) = if i == self.active {
			(self.path.as_deref(), self.project.name.as_str())
		} else {
			let d = self.tabs[i].as_ref();
			(d.and_then(|d| d.path.as_deref()), d.map(|d| d.project.name.as_str()).unwrap_or(""))
		};
		path.and_then(|p| p.file_name())
			.map(|n| n.to_string_lossy().into_owned())
			.or_else(|| (!project_name.is_empty()).then(|| project_name.to_string()))
			.unwrap_or_else(|| "untitled".into())
	}

	/// The tab already showing `path`, if any (re-opening switches, not stacks).
	fn tab_index_of(&self, path: &Path) -> Option<usize> {
		(0..self.tabs.len()).find(|&i| self.path_at(i) == Some(path))
	}

	/// Snapshot the live (active) fields into a parked [`Document`].
	fn capture_doc(&mut self) -> Document {
		Document {
			project: std::mem::replace(&mut self.project, Project::empty()),
			path: self.path.take(),
			view: std::mem::replace(&mut self.view, View { pan: [0.0, 0.0], zoom: 1.0 }),
			active_tile: self.active_tile.take(),
			active_color: self.active_color.take(),
		}
	}

	/// Load a parked [`Document`] into the live fields; re-derives the cycler.
	fn restore_doc(&mut self, d: Document) {
		self.project = d.project;
		self.path = d.path;
		self.view = d.view;
		self.active_tile = d.active_tile;
		self.active_color = d.active_color;
		self.palette_sel_end = None;
		self.refresh_palette();
	}

	/// Switch the active tab. `Ok` (no redraw) when already active / out of range.
	fn switch_to(&mut self, i: usize) -> Outcome {
		if i == self.active || i >= self.tabs.len() {
			return Outcome::Ok;
		}
		let parked = self.capture_doc();
		self.tabs[self.active] = Some(parked);
		let d = self.tabs[i].take().expect("an inactive tab is parked");
		self.active = i;
		self.restore_doc(d);
		Outcome::DocReplaced
	}

	/// Open `project` (loaded from `path`) and make it active: switch to an
	/// already-open tab with the same path, replace the bootstrap scratch tab,
	/// or push a new tab.
	fn add_doc(&mut self, project: Project, path: Option<PathBuf>) -> Outcome {
		if let Some(p) = path.as_deref() {
			if let Some(i) = self.tab_index_of(p) {
				return self.switch_to(i);
			}
		}
		let view = self.fit_center((project.width, project.height));
		let doc = Document { project, path, view, active_tile: None, active_color: None };
		if self.replace_scratch {
			self.replace_scratch = false;
			self.restore_doc(doc);
		} else {
			let parked = self.capture_doc();
			self.tabs[self.active] = Some(parked);
			self.tabs.push(None);
			self.active = self.tabs.len() - 1;
			self.restore_doc(doc);
		}
		Outcome::DocReplaced
	}

	/// Close the active tab. A dirty tab needs `force` (the confirm modal — see
	/// the `CloseProject` handler — gates this). Closing the **last** project
	/// is allowed: it resets to a blank scratch (the app stays open), which the
	/// next `open`/`new` replaces.
	fn close_active(&mut self, force: bool) -> Outcome {
		if self.project.dirty() && !force {
			return Outcome::Failed("close-project: unsaved changes — `save` first or use `close-project!`".into());
		}
		if self.tabs.len() <= 1 {
			let view = self.fit_center((1, 1));
			let blank = Document { project: Project::empty(), path: None, view, active_tile: None, active_color: None };
			self.tabs = vec![None];
			self.active = 0;
			self.replace_scratch = true;
			self.restore_doc(blank);
			return Outcome::DocReplaced;
		}
		// Drop the active doc (its `None` slot), then activate a neighbour.
		self.tabs.remove(self.active);
		let i = self.active.min(self.tabs.len() - 1);
		let d = self.tabs[i].take().expect("a neighbour tab is parked");
		self.active = i;
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
	/// `Project::from_wrl` for an imported WRL — absent from `assets_root`) to a
	/// sibling folder named after it, so the saved project reloads. Only the
	/// inferable assets are dumped (see `TilePack::dump`).
	fn write_project(&self, target: &Path) -> Result<(), String> {
		std::fs::write(target, self.project.save_string()).map_err(|e| format!("{}: {e}", target.display()))?;
		let dir = target.parent().unwrap_or_else(|| Path::new("."));
		for pack in &self.project.packs {
			if !self.assets_root.join(&pack.name).is_dir() {
				pack.dump(&dir.join(&pack.name))?;
			}
		}
		Ok(())
	}

	/// The single mutator (the architectural invariant): every command — from
	/// input, `--script`, or the console — routes here. This dispatch is just
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
			| ToolSelect { .. }
			| Layer { .. }
			| Mode { .. }
			| PassPick { .. }
			| PassPaint { .. }
			| TransformTile { .. }
			| Pick { .. }
			| Shore { .. }
			| Generate { .. }
			| Stroke { .. }) => self.exec_edit(c),
			c @ (Color { .. }
			| ColorTo { .. }
			| SetColor { .. }
			| HslBlock { .. }
			| PaletteSave { .. }
			| PaletteLoad { .. }
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
			| Convert
			| Save { .. }
			| SaveProject
			| SaveCopy { .. }
			| Tab { .. }
			| CloseProject { .. }
			| SaveAndClose
			| FileDialog { .. }
			| Resize { .. }
			| ResizeModal
			| AutoFixModal
			| GenerateModal
			| Export { .. }) => self.exec_io(c),
			c @ (Grid { .. }
			| PassOverlay { .. }
			| Animate { .. }
			| InGame { .. }
			| Crt { .. }
			| Tick { .. }
			| Console { .. }
			| Screenshot { .. }) => self.exec_overlay(c),
			c @ (SelectOp { .. }
			| SelectCell { .. }
			| SelectRect { .. }
			| Copy
			| Cut
			| Delete
			| Paste
			| Stamp { .. }
			| StampCancel
			| TemplateSave { .. }
			| TemplateDelete { .. }
			| TemplatePick { .. }
			| TemplateClone { .. }
			| TemplateImport { .. }
			| TemplateExport { .. }) => self.exec_select(c),
			c @ (Hash | AssertTile { .. } | AssertHash { .. } | AssertDirty { .. } | Quit { .. }) => {
				self.exec_assert(c)
			}
		}
	}

	/// Recreate the selection mask when the document's dimensions changed
	/// (open / new / resize / tab switch) — a stale mask must never index
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
				"set-pass: per-tile pass editing is retired — edit per cell in the Pass Table Editor (pass-paint)"
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
				let project = &mut self.project;
				let layer = match layer.as_deref() {
					Some("water") => LAYER_WATER,
					Some("ground") => LAYER_GROUND,
					Some(other) => {
						return Outcome::Failed(format!("erase: bad layer '{other}'"));
					}
					// Default: topmost present layer.
					None => match project.cell(x, y) {
						Some(stack) if stack[LAYER_GROUND].is_some() => LAYER_GROUND,
						_ => LAYER_WATER,
					},
				};
				if project.place(x, y, layer, None) { Outcome::Redraw } else { Outcome::Ok }
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
			// New opens in a fresh tab — nothing is lost, so no dirty
			// guard; `force` is vestigial. Interactive default: a fresh seed;
			// scripts pass one for determinism. The seed is reported so any map
			// can be re-made.
			Command::New { width, height, packs, seed, force: _ } => {
				let seed = seed.unwrap_or_else(|| {
					std::time::SystemTime::now()
						.duration_since(std::time::UNIX_EPOCH)
						.map(|d| d.as_nanos() as u64)
						.unwrap_or(0)
				});
				match Project::new(width, height, &packs, &self.assets_root, seed) {
					Ok(project) => {
						let line = format!(
							"new map {width}×{height}, packs: {}, seed {seed}",
							project.uses.iter().map(|u| u.name.as_str()).collect::<Vec<_>>().join("+"),
						);
						eprintln!("{line}");
						self.console.push_line(line);
						self.add_doc(project, None)
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
					// Paint onto the selected layer, not the tile's natural one.
					Ok((mut tile, _)) => {
						if self.randomize {
							tile = self.project.random_variant(tile, &mut self.paint_rng);
						}
						if self.project.place(x, y, self.active_layer, Some(tile)) {
							Outcome::Redraw
						} else {
							Outcome::Ok
						}
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
						if self.project.fill(x, y, tile, layer, randomize, &mut self.paint_rng) {
							Outcome::Redraw
						} else {
							Outcome::Ok
						}
					}
					Err(e) => Outcome::Failed(format!("fill: {e}")),
				}
			}
			Command::Randomize { on } => {
				self.randomize = on.unwrap_or(!self.randomize);
				self.console.push_line(format!("randomize variants: {}", if self.randomize { "on" } else { "off" }));
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
					other => {
						return Outcome::Failed(format!("mode: unknown '{other}' (map|pass)"));
					}
				};
				// The pass overlay rides with the Pass Table Editor: on entering
				// Pass it turns on (so painting is visible), on returning to Map
				// it turns off.
				self.show_pass_overlay = self.mode == EditorMode::Pass;
				self.console.push_line(format!("mode: {name}"));
				Outcome::Redraw
			}
			Command::PassPick { value } => {
				if value > 3 {
					return Outcome::Failed("pass-pick: value is 0..3".into());
				}
				self.active_pass = value;
				self.console.push_line(format!("pass: {}", PASS_LABELS[value as usize]));
				Outcome::Redraw
			}
			Command::PassPaint { x, y, value } => {
				let project = &mut self.project;
				if value > 3 {
					return Outcome::Failed("pass-paint: value is 0..3".into());
				}
				if project.set_pass(x, y, value) { Outcome::Redraw } else { Outcome::Ok }
			}
			Command::TransformTile { op } => {
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
				// The eyedropper hands back to the pencil — pick, then paint.
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
				let seed = seed.unwrap_or_else(|| {
					std::time::SystemTime::now()
						.duration_since(std::time::UNIX_EPOCH)
						.map(|d| d.as_nanos() as u64)
						.unwrap_or(0)
				});
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
				if self.active_color.is_none() {
					self.active_color = Some(index);
				}
				self.palette_sel_end = Some(index);
				Outcome::Redraw
			}
			Command::Color { index } => {
				self.active_color = Some(index);
				self.palette_sel_end = None;
				let palette: Vec<u8> = self.project.palette.clone();
				let s = crate::palette_panel::section_of(index as u16);
				let at = index as usize * 3;
				let line = format!(
					"color {index}: #{:02x}{:02x}{:02x} — {}, {}{}",
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
				self.palette_show_saved = saved;
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
				let text = match std::fs::read_to_string(&path) {
					Ok(t) => t,
					Err(e) => return Outcome::Failed(format!("palette-load: {e}")),
				};
				let colors = match map_core::parse_palette(&text) {
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
						Ok(px) if (8.0..=128.0).contains(&px) => self.picker.tile_px = px,
						_ => {
							return Outcome::Failed(format!("picker size: bad '{size}' (8..=128 px, or `next`)",));
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
				self.palette_scroll = to.max(0.0);
				Outcome::Redraw
			}
			Command::MenuOpen { name } => match self.menu.open_by_name(&name) {
				Ok(()) => Outcome::Redraw,
				Err(e) => Outcome::Failed(format!("menu: {e}")),
			},
			Command::ContextMenu { at } => {
				self.context_menu = at.map(|(x, y)| {
					let cell = self.cell_at(x, y);
					menu::ContextMenu::new(self.context_menu_items(cell), (x, y))
				});
				self.menu.close();
				Outcome::Redraw
			}
			Command::NewMapModal { picking } => {
				let mut modal = NewMap::new(&self.assets_root);
				modal.picking = picking;
				self.newmap = Some(modal);
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
					// Re-read the file so concurrent hand edits (bindings,
					// MaxPath) survive — only [Workspace] is machine-owned.
					// NOTE: the INI writer re-emits the whole file sorted;
					// comments are not preserved (documented in MANUAL.md).
					let mut ini = ini::INI::from_file(path).unwrap_or_else(|_| ini::INI::new());
					ini.insert_section("Workspace".to_string(), self.workspace.to_ini());
					let parent_ok =
						path.parent().is_none_or(|p| p.as_os_str().is_empty() || std::fs::create_dir_all(p).is_ok());
					match parent_ok.then(|| ini.to_file(path)) {
						Some(Ok(())) => {
							self.console.push_line(format!("settings saved → {}", path.display()));
							Outcome::Redraw
						}
						_ => Outcome::Failed(format!("save-settings: cannot write {}", path.display())),
					}
				}
			},
			_ => unreachable!("non-panel command routed to exec_panels"),
		}
	}

	/// Load the unit sprite library once (needs `MaxPath` → MAX.RES). A
	/// failed attempt doesn't retry — the cause lands in the console.
	pub fn ensure_units(&mut self) -> Result<(), String> {
		if self.units.is_some() {
			return Ok(());
		}
		if self.units_loaded {
			return Err("units: not available (see console)".into());
		}
		self.units_loaded = true;
		let Some(max_path) = self.max_path.clone() else {
			return Err("units: set MaxPath in config/mme.ini first".into());
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
	/// the project (dirties it) but records no undo patch — annotations are
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
					clear_selection_ground(&mut self.project, &self.selection);
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
				clear_selection_ground(&mut self.project, &self.selection);
				self.console.push_line(format!("deleted {n} cells"));
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
				let Some(dir) = self.user_templates_dir() else {
					return Outcome::Failed("template-save: no resources dir".into());
				};
				let name = match name {
					Some(n) => n,
					None => self.free_template_name(),
				};
				let template = match Template::capture(&self.project, &self.selection, &name) {
					Ok(t) => t,
					Err(e) => return Outcome::Failed(format!("template-save: {e}")),
				};
				let path = dir.join(format!("{name}.json"));
				if let Err(e) = template.save(&path) {
					return Outcome::Failed(format!("template-save: {e}"));
				}
				self.console.push_line(format!("template saved: {name} ({}x{})", template.width, template.height));
				self.scan_templates();
				self.template_sel = self.templates.iter().position(|t| !t.stock && t.name == name);
				Outcome::Redraw
			}
			Command::TemplateDelete { name } => {
				let Some(i) = self.find_template(name.as_deref()) else {
					return Outcome::Failed("template-delete: no template selected".into());
				};
				if self.templates[i].stock {
					return Outcome::Failed(format!(
						"template-delete: '{}' is a stock template (clone it instead)",
						self.templates[i].name
					));
				}
				let entry = &self.templates[i];
				if let Err(e) = std::fs::remove_file(&entry.path) {
					return Outcome::Failed(format!("template-delete {}: {e}", entry.path.display()));
				}
				self.console.push_line(format!("template deleted: {}", entry.name));
				self.scan_templates();
				Outcome::Redraw
			}
			Command::TemplatePick { name } => {
				let Some(i) = self.find_template(Some(&name)) else {
					return Outcome::Failed(format!("template-pick: no template named '{name}'"));
				};
				let entry = &self.templates[i];
				if let Some(id) = entry.template.missing_id(&self.project) {
					return Outcome::Failed(format!(
						"template-pick: '{name}' needs tile '{id}' — its pack isn't in this map"
					));
				}
				self.stamp = Some(entry.template.clone());
				self.template_sel = Some(i);
				self.console.push_line(format!("template armed: {name} (click the map to place, Esc cancels)"));
				Outcome::Redraw
			}
			Command::TemplateClone { name } => {
				let Some(i) = self.find_template(name.as_deref()) else {
					return Outcome::Failed("template-clone: no template selected".into());
				};
				let Some(dir) = self.user_templates_dir() else {
					return Outcome::Failed("template-clone: no resources dir".into());
				};
				let mut template = self.templates[i].template.clone();
				let base = format!("{}-copy", self.templates[i].name);
				let mut name = base.clone();
				let mut n = 2;
				while dir.join(format!("{name}.json")).exists() {
					name = format!("{base}-{n}");
					n += 1;
				}
				template.name = name.clone();
				if let Err(e) = template.save(&dir.join(format!("{name}.json"))) {
					return Outcome::Failed(format!("template-clone: {e}"));
				}
				self.console.push_line(format!("template cloned: {name}"));
				self.scan_templates();
				self.template_sel = self.templates.iter().position(|t| !t.stock && t.name == name);
				Outcome::Redraw
			}
			Command::TemplateImport { path } => {
				let template = match Template::load(&path) {
					Ok(t) => t,
					Err(e) => return Outcome::Failed(format!("template-import: {e}")),
				};
				let Some(dir) = self.user_templates_dir() else {
					return Outcome::Failed("template-import: no resources dir".into());
				};
				let mut name = template.name.clone();
				let mut n = 2;
				while dir.join(format!("{name}.json")).exists() {
					name = format!("{}-{n}", template.name);
					n += 1;
				}
				if let Err(e) = template.save(&dir.join(format!("{name}.json"))) {
					return Outcome::Failed(format!("template-import: {e}"));
				}
				self.console.push_line(format!("template imported: {name} ({}x{})", template.width, template.height));
				self.scan_templates();
				self.template_sel = self.templates.iter().position(|t| !t.stock && t.name == name);
				Outcome::Redraw
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
			_ => unreachable!("non-selection command routed to exec_select"),
		}
	}

	/// `resources/user/templates` (created on first save/import).
	fn user_templates_dir(&self) -> Option<PathBuf> {
		self.assets_root.parent().map(|p| p.join("user/templates"))
	}

	/// `resources/stock/templates` (shipped, read-only).
	fn stock_templates_dir(&self) -> Option<PathBuf> {
		self.assets_root.parent().map(|p| p.join("stock/templates"))
	}

	/// Re-read both template dirs into `templates` (stock first, names sorted
	/// within each group). Unparseable files are skipped with a console note.
	pub fn scan_templates(&mut self) {
		let mut entries = Vec::new();
		for (dir, stock) in [(self.stock_templates_dir(), true), (self.user_templates_dir(), false)] {
			let Some(dir) = dir else { continue };
			let Ok(read) = std::fs::read_dir(&dir) else { continue };
			let mut paths: Vec<PathBuf> =
				read.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
			paths.sort();
			for path in paths {
				match Template::load(&path) {
					Ok(template) => {
						entries.push(TemplateEntry { name: template.name.clone(), path, stock, template });
					}
					Err(e) => self.console.push_line(format!("templates: skipped {e}")),
				}
			}
		}
		self.templates = entries;
		if self.template_sel.is_some_and(|i| i >= self.templates.len()) {
			self.template_sel = None;
		}
	}

	/// Indices into `templates` that resolve against the open map — what the
	/// explorer shows (incompatible ones would only stamp errors).
	pub fn visible_templates(&self) -> Vec<usize> {
		(0..self.templates.len()).filter(|&i| self.templates[i].template.compatible(&self.project)).collect()
	}

	/// Resolve a delete/clone target: an explicit name, else the explorer's
	/// selected entry.
	fn find_template(&self, name: Option<&str>) -> Option<usize> {
		match name {
			Some(n) => self.templates.iter().position(|t| t.name == n),
			None => self.template_sel,
		}
	}

	/// First free `template-N` name in the user dir.
	fn free_template_name(&self) -> String {
		let dir = self.user_templates_dir();
		for n in 1.. {
			let name = format!("template-{n}");
			let taken = dir.as_ref().is_some_and(|d| d.join(format!("{name}.json")).exists());
			if !taken {
				return name;
			}
		}
		unreachable!("an unbounded counter finds a free name")
	}

	/// Document lifecycle: undo/redo, open/save/save-copy, file dialog, resize
	/// (+ its modal), the Auto Fix Shore modal, and WRL export.
	fn exec_io(&mut self, command: Command) -> Outcome {
		match command {
			Command::Undo => {
				let undone = self.project.undo();
				if undone {
					self.refresh_palette(); // the patch may have carried colors
					Outcome::Redraw
				} else {
					Outcome::Ok
				}
			}
			Command::Redo => {
				let redone = self.project.redo();
				if redone {
					self.refresh_palette();
					Outcome::Redraw
				} else {
					Outcome::Ok
				}
			}
			// Open adds a tab: no dirty guard (the current tab stays
			// open), and re-opening a path switches to its tab. `force` is now
			// vestigial.
			Command::Open { path, force: _ } => {
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
							// A read-only template loads path-less (Save → Save-As).
							let doc_path = if self.is_template(&path) { None } else { Some(path) };
							self.add_doc(project, doc_path)
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
							// An imported WRL has no project file yet — `Save Project`
							// asks where to save (Save-As), never writes the WRL.
							self.add_doc(project, None)
						}
						Err(e) => Outcome::Failed(format!("open {}: {e}", path.display())),
					}
				}
			}
			// New from Image: read only the PNG header (dimensions)
			// and open the settings modal — pixels are decoded later, at Convert.
			Command::NewFromImage { path } => {
				let (w, h) = match png_dimensions(&path) {
					Ok(v) => v,
					Err(e) => return Outcome::Failed(format!("new-from-image {}: {e}", path.display())),
				};
				let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into());
				self.newfromimage = Some(crate::newfromimage::NewFromImage::new(path, w, h, name));
				self.menu.close();
				Outcome::Redraw
			}
			// Run the open image modal's conversion to completion synchronously
			// (scripts / headless). The interactive button uses the stepped path.
			Command::Convert => {
				let Some(m) = self.newfromimage.as_ref() else {
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
						self.newfromimage = None;
						self.add_doc(project, None)
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
						"save: a project saves as .json (got {}) — `export` writes the baked WRL",
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
				// project is allowed — it resets to a blank scratch.
				if !force && self.project.dirty() {
					self.confirm = Some(crate::confirm::ConfirmClose::new(self.name_at(self.active)));
					Outcome::Redraw
				} else {
					self.close_active(force)
				}
			}
			Command::SaveAndClose => {
				// Save the active tab, then close it — but only once it's clean.
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
				// Default dir: palettes open in `resources/palettes`; maps in the
				// current document's folder, else MaxPath/templates (Load) or
				// maps (Save). `resources/maps` isn't shipped — create it on
				// first use so Save dialogs always have somewhere to land.
				let resources = self.assets_root.parent();
				let start = match purpose {
					FilePurpose::LoadPalette | FilePurpose::SavePalette => {
						resources.map(|p| p.join("palettes")).unwrap_or_else(|| PathBuf::from("."))
					}
					FilePurpose::ImportTemplate | FilePurpose::ExportTemplate => self
						.user_templates_dir()
						.inspect(|d| {
							let _ = std::fs::create_dir_all(d);
						})
						.unwrap_or_else(|| PathBuf::from(".")),
					_ => {
						let fallback = match purpose {
							FilePurpose::Load => self
								.max_path
								.clone()
								.filter(|p| p.is_dir())
								.or_else(|| resources.map(|p| p.join("templates"))),
							_ => resources.map(|p| {
								let maps = p.join("maps");
								let _ = std::fs::create_dir_all(&maps);
								maps
							}),
						};
						self.path
							.as_ref()
							.and_then(|p| p.parent())
							.map(Path::to_path_buf)
							.or(fallback)
							.unwrap_or_else(|| PathBuf::from("."))
					}
				};
				let suggested = match purpose {
					FilePurpose::SaveAs | FilePurpose::SaveCopy => self
						.path
						.as_ref()
						.and_then(|p| p.file_name())
						.map(|n| n.to_string_lossy().into_owned())
						.or_else(|| Some(self.project.name.clone())),
					FilePurpose::SavePalette => Some(self.project.name.clone()),
					_ => None,
				};
				// Suggested names from the project title may lack an extension.
				let suggested = suggested.map(|n| if n.ends_with(".json") { n } else { format!("{n}.json") });
				self.menu.close();

				if self.headless {
					return Outcome::Failed("file-dialog: not available in headless runs".into());
				}
				// Native dialog (rfd): blocks the event loop, which is fine —
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
					FilePurpose::LoadPalette => {
						dialog.add_filter("palettes", &["json"]).add_filter("all files", &["*"]).pick_file()
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
				};
				match picked {
					None => Outcome::Redraw, // canceled
					Some(path) => match purpose {
						FilePurpose::Load => self.execute(Command::Open { path, force: true }),
						FilePurpose::SaveAs => self.execute(Command::Save { path: Some(path) }),
						FilePurpose::SaveCopy => self.execute(Command::SaveCopy { path }),
						FilePurpose::LoadPalette => self.execute(Command::PaletteLoad { path }),
						FilePurpose::SavePalette => self.execute(Command::PaletteSave { path }),
						FilePurpose::NewFromImage => self.execute(Command::NewFromImage { path }),
						FilePurpose::ImportTemplate => self.execute(Command::TemplateImport { path }),
						FilePurpose::ExportTemplate => self.execute(Command::TemplateExport { path }),
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
						// Dimensions changed — the renderer's textures rebuild.
						Outcome::DocReplaced
					}
					Err(e) => Outcome::Failed(format!("resize: {e}")),
				}
			}
			Command::ResizeModal => {
				let project = &self.project;
				self.resize = Some(crate::resize::Resize::new(project.width, project.height));
				self.menu.close();
				Outcome::Redraw
			}
			Command::AutoFixModal => {
				let project = &self.project;
				// A throwaway session counts the current broken seams to show.
				let found = project.fix_session(None, map_core::FixStrength::Shore).found();
				self.autofix = Some(crate::autofix::AutoFix::new(found));
				self.menu.close();
				Outcome::Redraw
			}
			Command::GenerateModal => {
				self.generator = Some(crate::generator::Generator::new());
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
			Command::PassOverlay { on } => {
				self.show_pass_overlay = on.unwrap_or(!self.show_pass_overlay);
				self.console.push_line(format!("pass overlay: {}", if self.show_pass_overlay { "on" } else { "off" },));
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
					return Outcome::Failed("quit: unsaved changes — `save` first or use `quit!`".into());
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

	fn assets() -> PathBuf {
		Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/assets")
	}

	fn editor() -> EditorState {
		let root = assets();
		let project = Project::new(8, 8, &["GREEN".to_string()], &root, 1).unwrap();
		EditorState::new(project, (800, 600), None, root)
	}

	fn new_tab(e: &mut EditorState, seed: u64) -> Outcome {
		e.execute(Command::New { width: 8, height: 8, packs: vec!["GREEN".into()], seed: Some(seed), force: false })
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
		// Closing the last project is allowed — it resets to a blank scratch
		// (one tab, replaceable by the next open/new), app stays open.
		assert!(matches!(e.execute(Command::CloseProject { force: false }), Outcome::DocReplaced));
		assert_eq!(e.tab_infos().len(), 1);
		assert!(e.replace_scratch);
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
		assert!(e.confirm.is_some());
		e.close_modal();
		// Discard (`close-project!`) closes despite the unsaved changes.
		assert!(matches!(e.execute(Command::CloseProject { force: true }), Outcome::DocReplaced));
		assert!(e.confirm.is_none());
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
}
