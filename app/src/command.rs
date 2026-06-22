//! Command vocabulary + script parser.
//!
//! Every mutation - interactive, scripted, or (later) console-typed - is a
//! `Command` executed by `state::EditorState::execute`. One text line maps
//! to one command; `#` starts a comment.

use std::path::PathBuf;

/// What the file dialog is for - decides the command it resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilePurpose {
	/// Load a project: resolves to `open! PATH`.
	Load,
	/// Save As: resolves to `save PATH` (becomes the project's path).
	SaveAs,
	/// Save a Copy: resolves to `save-copy PATH` (path/dirty untouched).
	SaveCopy,
	/// Load a palette JSON: resolves to `palette-load PATH`.
	LoadPalette,
	/// Save the palette as JSON: resolves to `palette-save PATH`.
	SavePalette,
	/// Pick a palette JSON to copy into `user/palettes`: resolves to
	/// `palette-import PATH`.
	ImportPalette,
	/// Write the working palette to a chosen location: resolves to
	/// `palette-save PATH`.
	ExportPalette,
	/// Pick a template JSON to copy into the user templates dir: resolves to
	/// `template-import PATH`.
	ImportTemplate,
	/// Save the selection as a template at a chosen path: resolves to
	/// `template-export PATH`.
	ExportTemplate,
	/// New from Image: resolves to `new-from-image PATH`.
	NewFromImage,
	/// Import a WRL onto chosen tilesets: resolves to `import-wrl PATH`.
	ImportWrl,
	/// Export the open Tile Painter's tile to a PNG: resolves to `tile-export PATH`.
	ExportTilePng,
	/// Load a PNG into the open Tile Painter (nearest palette match): resolves
	/// to `tile-import PATH`.
	ImportTilePng,
	/// Render the explorer's selected template to a PNG: resolves to
	/// `template-export-png PATH`.
	ExportTemplatePng,
}

/// Which shore pass `shore` runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShoreMode {
	/// Sweep optimizer - uniform, deterministic coastline.
	Sweep,
	/// Loop-walk - varied coastline (may leave the odd discontinuity).
	Alt,
	/// Fix existing shore - re-tile to close discontinuities. A deliberate,
	/// bounded pass; never chained onto the auto passes.
	Fix,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
	/// Pan by a delta in tiles.
	Pan {
		dx: f32,
		dy: f32,
	},
	/// Center the view on a tile coordinate.
	PanTo {
		x: f32,
		y: f32,
	},
	/// Multiply zoom, anchored at the screen center.
	Zoom {
		factor: f32,
	},
	/// Multiply zoom, anchored at a screen-px point (interactive wheel).
	ZoomAt {
		x: f32,
		y: f32,
		factor: f32,
	},
	/// Set an absolute zoom level, anchored at the screen center.
	ZoomTo {
		level: f32,
	},
	/// Fit the whole map in the viewport.
	Fit,
	SetTile {
		x: u16,
		y: u16,
		tile: u16,
	},
	SetPass {
		tile: u16,
		value: u8,
	},
	/// Place a pack tile (project documents): layer derives from the pack.
	Place {
		x: u16,
		y: u16,
		spec: String,
	},
	/// Erase a cell layer (project documents); `None` = topmost present.
	Erase {
		x: u16,
		y: u16,
		layer: Option<String>,
	},
	/// Assert a cell's stack spec (`"WATR05,GSd004:!N"`, `-` = empty).
	AssertCell {
		x: u16,
		y: u16,
		spec: String,
	},
	/// Create a blank project: bottom layer = seeded random water (`None` seed =
	/// fresh random). Opens as a new tab, so there is no dirty guard to skip -
	/// `new` and `new!` are aliases.
	New {
		width: u16,
		height: u16,
		packs: Vec<String>,
		seed: Option<u64>,
	},
	/// Set the active paint tile; `"-"` clears, `None` shows.
	Tile {
		spec: Option<String>,
	},
	/// Paint the active tile at a cell.
	Paint {
		x: u16,
		y: u16,
	},
	/// Flood-fill the connected same-tile region from a cell with the active
	/// tile.
	Fill {
		x: u16,
		y: u16,
	},
	/// Toggle the randomize-variants paint mode (`None` = toggle).
	Randomize {
		on: Option<bool>,
	},
	/// Set the brush/eraser footprint size (odd-sided square; 1 = single cell).
	BrushSize {
		size: u16,
	},
	/// Set the brush footprint shape: square | circle.
	BrushShape {
		shape: String,
	},
	/// Switch the editor mode: map | pass.
	Mode {
		name: String,
	},
	/// Select the active pass value for the Pass Table Editor (0..3).
	PassPick {
		value: u8,
	},
	/// Paint a per-cell pass *override* into a cell (Local Pass Override
	/// Editor). This is document state - it changes the hash and bakes through.
	PassPaint {
		x: u16,
		y: u16,
		value: u8,
	},
	/// Set the passability of the *tile* under a cell (Pass Table Editor):
	/// retints every cell sharing that tile id.
	TilePass {
		x: u16,
		y: u16,
		value: u8,
	},
	/// Clear a cell's local pass override (back to the tile-derived value).
	PassClear {
		x: u16,
		y: u16,
	},
	/// Reset every tile's passability to the tileset's shipped values (Tools ▸
	/// Reset Pass Table to Tileset) - reverts Pass Table Editor edits / a loaded
	/// map's `tilepass` block. Per-cell overrides are left alone.
	ResetTilePass,
	/// Switch the map tool: pencil | picker.
	ToolSelect {
		name: String,
	},
	/// Whole-selection operations: all | clear | invert | similar.
	SelectOp {
		op: String,
	},
	/// Add/subtract one cell to/from the selection (the freehand select
	/// tool's stroke unit).
	SelectCell {
		x: u16,
		y: u16,
		mode: map_core::SelectMode,
	},
	/// Apply a rectangle to the selection (the rect select tool's release).
	SelectRect {
		x0: u16,
		y0: u16,
		x1: u16,
		y1: u16,
		mode: map_core::SelectMode,
	},
	/// Translate the selection mask by a cell delta (Alt+drag move — marquee
	/// only, never the terrain). Cells shifted off the map are dropped.
	SelectMove {
		dx: i32,
		dy: i32,
	},
	/// Copy the selected cells to the clipboard.
	Copy,
	/// Copy, then clear the selected cells' ground layer (one undo unit).
	Cut,
	/// Clear the selected cells' **active layer** without touching the
	/// clipboard (Edit ▸ Clear, the Delete key) - water on the water layer,
	/// ground on the ground layer.
	Delete,
	/// Clear **every** layer of the selected cells (Shift+Delete) - the cells
	/// become true holes, water and ground both gone.
	DeleteAll,
	/// Arm the clipboard as the ghost stamp under the cursor.
	Paste,
	/// Place the armed ghost stamp (paste or a picked template) with its
	/// top-left at a cell. The stamp stays armed for repeat placing.
	Stamp {
		x: u16,
		y: u16,
	},
	/// Disarm the ghost stamp.
	StampCancel,
	/// Save the selection as a user template (auto-named when no name).
	TemplateSave {
		name: Option<String>,
	},
	/// Delete a user template (`None` = the explorer's selected one).
	TemplateDelete {
		name: Option<String>,
	},
	/// Rename a user template's file (`from` -> `to`).
	TemplateRename {
		from: String,
		to: String,
	},
	/// Remove exact-duplicate user templates among the explorer's visible list.
	TemplateDedupe,
	/// Open the user-templates folder in the OS file manager.
	TemplateExplore,
	/// Open the Rename Template modal for the explorer's selected template.
	TemplateRenameModal,
	/// Open the Delete Template confirmation modal for the selected template.
	TemplateDeleteModal,
	/// Open the Remove Duplicates modal (reports how many would be removed).
	TemplateDedupeModal,
	/// Arm a template as the ghost stamp (the explorer click).
	TemplatePick {
		name: String,
	},
	/// Duplicate a template into the user dir (`None` = explorer selection).
	TemplateClone {
		name: Option<String>,
	},
	/// Copy an external template file into the user templates dir.
	TemplateImport {
		path: PathBuf,
	},
	/// Save the selection as a template at an explicit path (Export As).
	TemplateExport {
		path: PathBuf,
	},
	/// Render the explorer's selected template to a PNG. `None` opens the save
	/// dialog (the explorer's context menu); `Some` writes that path (scripted).
	TemplateExportPng {
		path: Option<PathBuf>,
	},
	/// Select a unit sprite for stamping previews (Units panel), or `None`
	/// (`unit off`) to deselect and return to the pencil.
	UnitSelect {
		tag: Option<String>,
	},
	/// Active team color for new unit previews: red|green|blue|gray|yellow|0-4.
	UnitTeam {
		team: String,
	},
	/// Stamp a unit preview directly (scriptable form of panel + map click).
	UnitPlace {
		tag: String,
		x: u16,
		y: u16,
	},
	/// Remove the unit preview on a cell (the unit-eraser tool's click).
	UnitErase {
		x: u16,
		y: u16,
	},
	/// Remove all unit previews.
	UnitClear,
	/// Show/hide all unit previews on the map (`None` = toggle).
	UnitsVisible {
		on: Option<bool>,
	},
	/// Select the active edit layer: water | ground. Paint + erase
	/// act only on it.
	Layer {
		name: String,
	},
	/// View filter: show only the active layer, hiding the others
	/// (`None` = toggle). Layers stay editable - this changes nothing in the
	/// document, only what the map view composites.
	ShowOnlyLayer {
		on: Option<bool>,
	},
	/// Apply a transform op to the active tile: flip-h | flip-v | cw | ccw.
	TransformTile {
		op: String,
	},
	/// Eyedropper: make a cell's top tile (transform included) the brush.
	Pick {
		x: u16,
		y: u16,
	},
	/// Shore the land/water boundary: whole map, or a cell
	/// rectangle (inclusive corners, expanded by one). `mode` picks the
	/// pass: sweep optimizer, loop-walk (`alt`), or fix-existing (`fix`).
	Shore {
		region: Option<(u16, u16, u16, u16)>,
		mode: ShoreMode,
	},
	/// Generate random terrain over the whole map: a pattern word
	/// plus percent knobs. `None` seed = fresh random (reported, so any
	/// generated map can be re-made); `alt_shore` picks the loop-walk
	/// auto-shore pass over the sweep optimizer.
	Generate {
		pattern: String,
		water: u8,
		obstructions: u8,
		decorations: u8,
		seed: Option<u64>,
		alt_shore: bool,
	},
	/// Open the Generate Random Terrain modal.
	GenerateModal,
	/// Open/commit a stroke - edits in between undo as one unit.
	Stroke {
		begin: bool,
	},
	/// Minimap source: overworld | pass | minimap.
	MinimapMode {
		mode: String,
	},
	/// Select a palette slot in the Color Palette panel.
	Color {
		index: u8,
	},
	/// Extend the palette selection to `index` (shift-click range).
	ColorTo {
		index: u8,
	},
	/// Toggle `index` in the palette multi-selection (ctrl-click).
	ColorToggle {
		index: u8,
	},
	/// Set a dynamic palette slot (the map's color override) - undoable.
	SetColor {
		slot: u8,
		rgb: [u8; 3],
	},
	/// Write the current palette to a JSON file.
	PaletteSave {
		path: PathBuf,
	},
	/// Load a palette JSON file into the editable slots - undoable.
	PaletteLoad {
		path: PathBuf,
	},
	/// Save the working palette into `user/palettes/<name>.json` (the manager's
	/// Save), then rescan + select it.
	PaletteSaveAs {
		name: String,
	},
	/// Rename a saved user palette file's stem (the manager's Edit).
	PaletteRename {
		from: PathBuf,
		to: String,
	},
	/// Delete a saved user palette file.
	PaletteDelete {
		path: PathBuf,
	},
	/// Copy/validate an external palette JSON into `user/palettes`.
	PaletteImport {
		path: PathBuf,
	},
	/// Open the Save-palette name modal (working palette → `user/palettes`).
	PaletteSaveModal,
	/// Open the Edit (rename) modal for the selected saved palette.
	PaletteRenameModal,
	/// Open the Delete-confirm modal for the selected saved palette.
	PaletteDeleteModal,
	/// Switch the Color Palette panel tab: the grid, or the saved-palettes list
	/// (`saved` true scans the palette dirs).
	PaletteTab {
		saved: bool,
	},
	/// Re-tint a whole water cycle block in HSL: degrees, percent, percent.
	HslBlock {
		slot: u8,
		dh: f32,
		ds: f32,
		dl: f32,
	},
	/// Color Palette grid scroll position (px, clamped at draw time).
	PaletteScroll {
		to: f32,
	},
	/// Tile Explorer filter: a name or `next`.
	PickerFilter {
		name: String,
	},
	/// Tile Explorer display size: px or `next`.
	PickerSize {
		size: String,
	},
	/// Tile Explorer scroll position (px, clamped at draw time).
	PickerScroll {
		to: f32,
	},
	/// Open a main-menu dropdown by title, or `off`.
	MenuOpen {
		name: String,
	},
	/// Open the right-click context menu at a screen-px point, or close it
	/// (`None`). Items reflect the state at open time.
	ContextMenu {
		at: Option<(f32, f32)>,
	},
	/// Open the Create New Map modal; `picking` jumps to the
	/// tile-set selection stage (screenshots).
	NewMapModal {
		picking: bool,
	},
	/// Show/hide a workspace panel (`None` = toggle).
	Window {
		id: String,
		on: Option<bool>,
	},
	/// Move a workspace panel to a dock side or float it.
	DockTo {
		id: String,
		place: String,
		at: Option<(f32, f32)>,
	},
	/// Reset the whole dockable layout to defaults (Windows ▸ Reset Dialogs).
	ResetLayout,
	/// Write the current UI layout to the settings file.
	SaveSettings,
	Undo,
	Redo,
	/// Load a project/WRL into a tab. `open` and `open!` are aliases (the load
	/// opens its own tab, so there is no dirty guard to skip).
	Open {
		path: PathBuf,
	},
	/// New from Image: decode a PNG and open the settings modal.
	NewFromImage {
		path: PathBuf,
	},
	/// Import WRL: read the WRL header and open the pack-picker modal that
	/// matches its tiles against existing tilesets.
	ImportWrl {
		path: PathBuf,
	},
	/// Run the open New-from-Image modal's conversion to completion and open the
	/// result as a new tab (the scriptable/headless counterpart to the modal's
	/// stepped Convert button).
	Convert,
	/// Open a URL in the system browser (Help ▸ Go to Website / Project GitHub).
	OpenUrl {
		url: String,
	},
	/// Open the bundled HTML user manual in the system browser (Help ▸ User Manual).
	HelpManual,
	/// Open the About dialog (Help ▸ About).
	About,
	/// Save the document in its native format (project → .json, flat doc →
	/// WRL); `None` re-saves to the loaded path.
	Save {
		path: Option<PathBuf>,
	},
	/// Save Project (menu): re-save to the current path, or open the file
	/// dialog in Save-As mode if the project was never saved.
	SaveProject,
	/// Switch the active open project to tab `index` (0-based).
	Tab {
		index: usize,
	},
	/// Close the active project tab; `force` skips the unsaved-changes guard
	/// (`close-project!`). The last tab stays open.
	CloseProject {
		force: bool,
	},
	/// Save the active project then close its tab (the confirm modal's "Save");
	/// a never-saved project routes to Save-As and stays open.
	SaveAndClose,
	/// GUI quit entry point (window close / menu Exit): quits when clean, else
	/// opens the Save/Discard/Cancel confirm. Distinct from `quit` so scripts
	/// keep the hard `quit` / `quit!` guard.
	QuitRequest,
	/// Save the next unsaved tab, then quit once all are clean (the quit
	/// confirm's "Save"); a never-saved tab routes to Save-As and stays open.
	SaveAndQuit,
	/// Write a copy of the project to PATH without changing the current
	/// path or clearing the dirty flag.
	SaveCopy {
		path: PathBuf,
	},
	/// Open the hand-rolled file dialog for a load/save purpose.
	FileDialog {
		purpose: FilePurpose,
	},
	/// Resize the map canvas: new W×H, existing map placed at
	/// `(off_x, off_y)` (negative crops); new area fills with water.
	Resize {
		width: u16,
		height: u16,
		off_x: i32,
		off_y: i32,
	},
	/// Open the Resize Map modal.
	ResizeModal,
	/// Open the Auto Fix Shore modal.
	AutoFixModal,
	/// Bake a project to a game-ready WRL; `None` = project path as .wrl.
	Export {
		path: Option<PathBuf>,
	},
	/// Show/hide the cell grid overlay (`None` = toggle).
	Grid {
		on: Option<bool>,
	},
	/// Show/hide the bottom status bar (`None` = toggle).
	StatusBar {
		on: Option<bool>,
	},
	/// Show/hide the pass-value overlay (`None` = toggle).
	PassOverlay {
		on: Option<bool>,
	},
	/// Enable/disable palette-cycling animation (`None` = toggle).
	Animate {
		on: Option<bool>,
	},
	/// In-Game render mode: cycling + 6-bit colour quantization (`None` =
	/// toggle).
	InGame {
		on: Option<bool>,
	},
	/// CRT post-process effect over the whole app (`None` = toggle).
	Crt {
		on: Option<bool>,
	},
	/// Set the UI scale factor (View ▸ UI Scale): 1.0 (small) / 1.25 (medium) /
	/// 1.5 (large). Scales all chrome + fonts; the map stays native.
	UiScale {
		scale: f32,
	},
	/// Debug: render with the document's internal (map/WRL) palette instead
	/// of the game-resolved one (`None` = toggle).
	MapPalette {
		on: Option<bool>,
	},
	/// Remap the document's internal palette onto a MAX-compatible one
	/// (Tools ▸ Palette) - lossy but undoable, WRL imports only. `rasterize`
	/// re-imports the composed map like New-from-Image instead of remapping
	/// slots; `water` keeps the water cycle blocks animated; `relaxed` +
	/// `threshold` (fraction) tune the rasterize dedupe.
	ConvertPalette {
		rasterize: bool,
		water: bool,
		relaxed: bool,
		threshold: f32,
	},
	/// Open the Convert to Compatible Palette modal.
	ConvertPaletteModal,
	/// Open the Map Preferences modal.
	PreferencesModal,
	/// Open the Tile Painter on a blank new tile.
	TilePaintNew,
	/// Open the Tile Painter cloning the selected tile.
	TilePaintClone,
	/// Open the Tile Painter editing the selected tile in place (stock tiles
	/// need `--dev`).
	TilePaintEdit,
	/// Commit the open Tile Painter (the Save action; also scriptable).
	TileCommit,
	/// Delete the selected tile from its pack (stock tiles need `--dev`; user
	/// tiles delete in normal mode). Refused if the tile is painted on the map.
	TileDelete,
	/// Export the open Tile Painter's tile as a PNG (palette colors → RGB).
	TileExportPng {
		path: PathBuf,
	},
	/// Load a PNG into the open Tile Painter, mapping each pixel to its visually
	/// closest palette color.
	TileImportPng {
		path: PathBuf,
	},
	/// Bake edited tiles back to their shipped asset packs (`--dev` only).
	Bake,
	/// Overwrite the map's original file in place - even a shipped (stock) map
	/// that normally loads read-only (DEV ▸ Update Map, `--dev` only).
	UpdateMap,
	/// A text-edit action from a focused field's right-click menu, routed to the
	/// open modal's focused text field.
	Edit(EditOp),
	/// Advance the animation clock (deterministic time for scripts).
	Tick {
		seconds: f32,
	},
	/// Show/hide the in-app console (`None` = toggle).
	Console {
		on: Option<bool>,
	},
	Screenshot {
		path: PathBuf,
		/// Optional `crop=x,y,w,h` sub-rect (render-resolution px).
		crop: Option<(u32, u32, u32, u32)>,
		/// Optional `resize=WxH` (nearest-neighbour, applied after crop).
		resize: Option<(u32, u32)>,
	},
	/// Print the document hash to stdout.
	Hash,
	AssertTile {
		x: u16,
		y: u16,
		tile: u16,
	},
	AssertHash {
		hash: u64,
	},
	AssertDirty {
		dirty: bool,
	},
	/// `force` skips the dirty guard.
	Quit {
		force: bool,
	},
}

/// Split a line into tokens, honoring `"…"` quoting so an argument with
/// spaces or `#` (e.g. a path under `M.A.X. Projects/`) survives intact, and
/// treating an *unquoted* `#` as a line comment. `None` for a blank or
/// comment-only line. Quotes concatenate (`a" "b` → `a b`); an unterminated
/// quote runs to end-of-line.
fn tokenize(line: &str) -> Option<Vec<String>> {
	let mut tokens = Vec::new();
	let mut cur = String::new();
	let mut started = false;
	let mut chars = line.chars();
	while let Some(c) = chars.next() {
		match c {
			'"' => {
				started = true;
				for q in chars.by_ref() {
					if q == '"' {
						break;
					}
					cur.push(q);
				}
			}
			'#' => break, // comment to end of line (outside quotes)
			c if c.is_whitespace() => {
				if started {
					tokens.push(std::mem::take(&mut cur));
					started = false;
				}
			}
			c => {
				started = true;
				cur.push(c);
			}
		}
	}
	if started {
		tokens.push(cur);
	}
	(!tokens.is_empty()).then_some(tokens)
}

/// Parse `x,y,w,h` (four `u32`s) - a screenshot crop rect.
pub fn parse_crop(s: &str) -> Option<(u32, u32, u32, u32)> {
	let p: Vec<&str> = s.split(',').collect();
	match p[..] {
		[x, y, w, h] => Some((x.parse().ok()?, y.parse().ok()?, w.parse().ok()?, h.parse().ok()?)),
		_ => None,
	}
}

/// Parse `WxH` (two `u32`s) - a screenshot resize target or `--size`.
pub fn parse_dims(s: &str) -> Option<(u32, u32)> {
	let (w, h) = s.split_once(['x', 'X'])?;
	Some((w.parse().ok()?, h.parse().ok()?))
}

/// Parse one script line. `Ok(None)` for blank lines and comments.
/// A text-edit action from a focused field's right-click menu (mapped to a
/// [`crate::modal::ModalKey`] and routed to the open modal's focused field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditOp {
	Cut,
	Copy,
	Paste,
	Delete,
	SelectAll,
}

pub fn parse_line(line: &str) -> Result<Option<Command>, String> {
	let Some(tokens) = tokenize(line) else {
		return Ok(None);
	};
	let verb = tokens[0].as_str();
	let args: Vec<&str> = tokens[1..].iter().map(String::as_str).collect();

	fn num<T: std::str::FromStr>(args: &[&str], i: usize, verb: &str) -> Result<T, String> {
		args.get(i)
			.ok_or_else(|| format!("{verb}: missing argument {}", i + 1))?
			.parse()
			.map_err(|_| format!("{verb}: bad argument '{}'", args[i]))
	}

	/// A required positional string argument; `msg` is the user-facing error
	/// when it is absent (call sites carry their own `"verb: …"` messages).
	fn req_str(args: &[&str], i: usize, msg: &str) -> Result<String, String> {
		Ok(args.get(i).ok_or_else(|| msg.to_string())?.to_string())
	}

	/// An optional positional string argument (absent → `None`).
	fn opt_str(args: &[&str], i: usize) -> Option<String> {
		args.get(i).map(|s| s.to_string())
	}

	/// Parse a UI scale level word into its factor: `small`=1.0, `medium`=1.25,
	/// `large`=1.5 (percentages also accepted, for scriptability).
	fn parse_ui_scale(word: &str) -> Result<f32, String> {
		match word {
			"small" | "1" | "100" | "100%" => Ok(1.0),
			"medium" | "125" | "125%" => Ok(1.25),
			"large" | "150" | "150%" => Ok(1.5),
			other => Err(format!("ui-scale: expected small|medium|large, got '{other}'")),
		}
	}

	/// Parse an `on` / `off` / `toggle` flag (a bare/absent argument = toggle)
	/// into an `Option<bool>`: `Some(true/false)`, or `None` to toggle.
	fn on_off_toggle(args: &[&str], verb: &str) -> Result<Option<bool>, String> {
		match args.first() {
			Some(&"on") => Ok(Some(true)),
			Some(&"off") => Ok(Some(false)),
			None | Some(&"toggle") => Ok(None),
			_ => Err(format!("{verb}: expected on|off|toggle")),
		}
	}

	let command = match verb {
		"pan" => Command::Pan { dx: num(&args, 0, verb)?, dy: num(&args, 1, verb)? },
		"pan-to" => Command::PanTo { x: num(&args, 0, verb)?, y: num(&args, 1, verb)? },
		"zoom" => Command::Zoom { factor: num(&args, 0, verb)? },
		"zoom-at" => Command::ZoomAt { x: num(&args, 0, verb)?, y: num(&args, 1, verb)?, factor: num(&args, 2, verb)? },
		"zoom-to" => Command::ZoomTo { level: num(&args, 0, verb)? },
		"fit" => Command::Fit,
		"set-tile" => Command::SetTile { x: num(&args, 0, verb)?, y: num(&args, 1, verb)?, tile: num(&args, 2, verb)? },
		"set-pass" => Command::SetPass { tile: num(&args, 0, verb)?, value: num(&args, 1, verb)? },
		"place" => Command::Place {
			x: num(&args, 0, verb)?,
			y: num(&args, 1, verb)?,
			spec: req_str(&args, 2, "place: missing tile id")?,
		},
		"erase" => Command::Erase { x: num(&args, 0, verb)?, y: num(&args, 1, verb)?, layer: opt_str(&args, 2) },
		"assert-cell" => Command::AssertCell {
			x: num(&args, 0, verb)?,
			y: num(&args, 1, verb)?,
			spec: req_str(&args, 2, "assert-cell: missing spec")?,
		},
		"new" | "new!" => {
			let width = num(&args, 0, verb)?;
			let height = num(&args, 1, verb)?;
			// Remaining args: pack names; a bare integer is the seed.
			let mut packs = Vec::new();
			let mut seed = None;
			for arg in &args[2..] {
				match arg.parse::<u64>() {
					Ok(n) => seed = Some(n),
					Err(_) => packs.push(arg.to_string()),
				}
			}
			Command::New { width, height, packs, seed }
		}
		"tile" => Command::Tile { spec: opt_str(&args, 0) },
		"paint" => Command::Paint { x: num(&args, 0, verb)?, y: num(&args, 1, verb)? },
		"fill" => Command::Fill { x: num(&args, 0, verb)?, y: num(&args, 1, verb)? },
		"randomize" => Command::Randomize { on: on_off_toggle(&args, verb)? },
		"brush-size" => Command::BrushSize { size: num(&args, 0, verb)? },
		"brush-shape" => Command::BrushShape { shape: req_str(&args, 0, "brush-shape: expected square|circle")? },
		"mode" => Command::Mode { name: req_str(&args, 0, "mode: expected map|pass|localpass")? },
		"pass-pick" => Command::PassPick { value: num(&args, 0, verb)? },
		"pass-paint" => {
			Command::PassPaint { x: num(&args, 0, verb)?, y: num(&args, 1, verb)?, value: num(&args, 2, verb)? }
		}
		"tile-pass" => {
			Command::TilePass { x: num(&args, 0, verb)?, y: num(&args, 1, verb)?, value: num(&args, 2, verb)? }
		}
		"pass-clear" => Command::PassClear { x: num(&args, 0, verb)?, y: num(&args, 1, verb)? },
		"tile-pass-reset" => Command::ResetTilePass,
		"tool" => Command::ToolSelect {
			name: req_str(&args, 0, "tool: expected pencil|picker|eraser|fill|unit|select|select-rect")?,
		},
		"select" => Command::SelectOp { op: req_str(&args, 0, "select: expected all|clear|invert|similar")? },
		"select-cell" => Command::SelectCell {
			x: num(&args, 0, verb)?,
			y: num(&args, 1, verb)?,
			mode: map_core::SelectMode::parse(args.get(2).copied().unwrap_or("add"))?,
		},
		"select-rect" => Command::SelectRect {
			x0: num(&args, 0, verb)?,
			y0: num(&args, 1, verb)?,
			x1: num(&args, 2, verb)?,
			y1: num(&args, 3, verb)?,
			mode: map_core::SelectMode::parse(args.get(4).copied().unwrap_or("replace"))?,
		},
		"select-move" => Command::SelectMove { dx: num(&args, 0, verb)?, dy: num(&args, 1, verb)? },
		"copy" => Command::Copy,
		"cut" => Command::Cut,
		"paste" => Command::Paste,
		"delete" => Command::Delete,
		"delete-all" => Command::DeleteAll,
		"stamp" => match args.first() {
			Some(&"cancel") => Command::StampCancel,
			_ => Command::Stamp { x: num(&args, 0, verb)?, y: num(&args, 1, verb)? },
		},
		"template-save" => Command::TemplateSave { name: opt_str(&args, 0) },
		// No args opens the confirmation modal for the selection; a name deletes
		// it directly (scripted). `template-delete!` deletes the selection now.
		"template-delete" => match args.first() {
			Some(name) => Command::TemplateDelete { name: Some(name.to_string()) },
			None => Command::TemplateDeleteModal,
		},
		"template-delete!" => Command::TemplateDelete { name: None },
		// `template-rename` with no args opens the modal; with `FROM TO` it renames.
		"template-rename" => match (args.first(), args.get(1)) {
			(Some(from), Some(to)) => Command::TemplateRename { from: from.to_string(), to: to.to_string() },
			(None, _) => Command::TemplateRenameModal,
			(Some(_), None) => return Err("template-rename: expected `FROM TO` (or no args to open the dialog)".into()),
		},
		// `template-dedupe` opens the modal; `template-dedupe!` performs the removal.
		"template-dedupe" => Command::TemplateDedupeModal,
		"template-dedupe!" => Command::TemplateDedupe,
		"template-explore" => Command::TemplateExplore,
		"template-pick" => {
			Command::TemplatePick { name: req_str(&args, 0, "template-pick: expected a template name")? }
		}
		"template-clone" => Command::TemplateClone { name: opt_str(&args, 0) },
		"template-import" => {
			Command::TemplateImport { path: PathBuf::from(req_str(&args, 0, "template-import: expected a path")?) }
		}
		"template-export" => {
			Command::TemplateExport { path: PathBuf::from(req_str(&args, 0, "template-export: expected a path")?) }
		}
		// No args opens the save dialog for the selection; a path writes it (scripted).
		"template-export-png" => Command::TemplateExportPng { path: opt_str(&args, 0).map(PathBuf::from) },
		"unit" => match args.first() {
			None => return Err("unit: expected a unit tag or `off`".into()),
			Some(&"off") => Command::UnitSelect { tag: None },
			Some(tag) => Command::UnitSelect { tag: Some(tag.to_string()) },
		},
		"unit-team" => {
			Command::UnitTeam { team: req_str(&args, 0, "unit-team: expected red|green|blue|gray|yellow|0-4")? }
		}
		"unit-place" => Command::UnitPlace {
			tag: req_str(&args, 0, "unit-place: expected TAG X Y")?,
			x: num(&args, 1, verb)?,
			y: num(&args, 2, verb)?,
		},
		"unit-erase" => Command::UnitErase { x: num(&args, 0, verb)?, y: num(&args, 1, verb)? },
		"unit-clear" => Command::UnitClear,
		"units" => Command::UnitsVisible { on: on_off_toggle(&args, verb)? },
		"layer" => Command::Layer { name: req_str(&args, 0, "layer: expected water|ground")? },
		"show-only-layer" => Command::ShowOnlyLayer { on: on_off_toggle(&args, verb)? },
		"transform" => Command::TransformTile { op: req_str(&args, 0, "transform: expected flip-h|flip-v|cw|ccw")? },
		"pick" => Command::Pick { x: num(&args, 0, verb)?, y: num(&args, 1, verb)? },
		"shore" => {
			// An optional leading mode word: `alt` (loop-walk) or `fix`.
			let (mode, rest) = match args.first() {
				Some(&"alt") => (ShoreMode::Alt, &args[1..]),
				Some(&"fix") => (ShoreMode::Fix, &args[1..]),
				_ => (ShoreMode::Sweep, &args[..]),
			};
			let region = match rest.len() {
				0 => None,
				4 => Some((num(rest, 0, verb)?, num(rest, 1, verb)?, num(rest, 2, verb)?, num(rest, 3, verb)?)),
				_ => {
					return Err("shore: expected [alt|fix] with no region (whole map) or X0 Y0 X1 Y1".into());
				}
			};
			Command::Shore { region, mode }
		}
		"generate" => {
			let pattern = req_str(&args, 0, "generate: missing pattern (islands|continent|land-mass|river-raid)")?;
			// Order-independent options; percentages default to a middle-of-
			// the-road map (45% water, 10% obstructions, 5% decorations).
			let (mut water, mut obstructions, mut decorations, mut seed, mut alt_shore) =
				(45u8, 10u8, 5u8, None, false);
			for a in &args[1..] {
				if let Some(v) = a.strip_prefix("water=") {
					water = v.parse().map_err(|_| format!("generate: bad water '{v}'"))?;
				} else if let Some(v) = a.strip_prefix("obstructions=") {
					obstructions = v.parse().map_err(|_| format!("generate: bad obstructions '{v}'"))?;
				} else if let Some(v) = a.strip_prefix("decorations=") {
					decorations = v.parse().map_err(|_| format!("generate: bad decorations '{v}'"))?;
				} else if let Some(v) = a.strip_prefix("seed=") {
					seed = Some(v.parse().map_err(|_| format!("generate: bad seed '{v}'"))?);
				} else if let Some(v) = a.strip_prefix("shore=") {
					alt_shore = match v {
						"alt" => true,
						"sweep" => false,
						_ => return Err(format!("generate: bad shore '{v}' (sweep|alt)")),
					};
				} else {
					return Err(format!(
						"generate: unexpected '{a}' (water=N obstructions=N decorations=N seed=N shore=sweep|alt)",
					));
				}
			}
			if water > 100 || obstructions > 100 || decorations > 100 {
				return Err("generate: water/obstructions/decorations are percentages (0..=100)".into());
			}
			Command::Generate { pattern, water, obstructions, decorations, seed, alt_shore }
		}
		"generate-modal" => Command::GenerateModal,
		"stroke" => match args.first() {
			Some(&"begin") => Command::Stroke { begin: true },
			Some(&"end") => Command::Stroke { begin: false },
			_ => return Err("stroke: expected begin|end".into()),
		},
		"minimap" => Command::MinimapMode { mode: req_str(&args, 0, "minimap: expected overworld|pass|minimap")? },
		"color" => Command::Color { index: num(&args, 0, verb)? },
		"color-to" => Command::ColorTo { index: num(&args, 0, verb)? },
		"color-toggle" => Command::ColorToggle { index: num(&args, 0, verb)? },
		"palette-save" => {
			Command::PaletteSave { path: PathBuf::from(req_str(&args, 0, "palette-save: missing path")?) }
		}
		"palette-load" => {
			Command::PaletteLoad { path: PathBuf::from(req_str(&args, 0, "palette-load: missing path")?) }
		}
		"palette-save-as" => match args.first() {
			Some(name) => Command::PaletteSaveAs { name: name.to_string() },
			None => Command::PaletteSaveModal,
		},
		"palette-rename" => match (args.first(), args.get(1)) {
			(Some(from), Some(to)) => Command::PaletteRename { from: PathBuf::from(from), to: to.to_string() },
			(None, _) => Command::PaletteRenameModal,
			_ => return Err("palette-rename: expected FROM TO".into()),
		},
		"palette-delete" => match args.first() {
			Some(path) => Command::PaletteDelete { path: PathBuf::from(path) },
			None => Command::PaletteDeleteModal,
		},
		"palette-import" => {
			Command::PaletteImport { path: PathBuf::from(req_str(&args, 0, "palette-import: missing path")?) }
		}
		"palette-tab" => Command::PaletteTab {
			saved: match args.first() {
				Some(&"saved") => true,
				Some(&"grid") | Some(&"current") => false,
				_ => return Err("palette-tab: expected grid|saved".into()),
			},
		},
		"set-color" => {
			let slot = num(&args, 0, verb)?;
			// Bare hex - `#` starts a script comment, so `#rrggbb` would be
			// stripped before it ever reached the parser.
			let hex = args.get(1).ok_or("set-color: missing color (rrggbb - no '#', it starts a comment)")?;
			if hex.len() != 6 {
				return Err(format!("set-color: bad color '{hex}' (want rrggbb)"));
			}
			let mut rgb = [0u8; 3];
			for i in 0..3 {
				rgb[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
					.map_err(|_| format!("set-color: bad hex '{hex}'"))?;
			}
			Command::SetColor { slot, rgb }
		}
		"hsl-block" => Command::HslBlock {
			slot: num(&args, 0, verb)?,
			dh: num(&args, 1, verb)?,
			ds: num(&args, 2, verb)?,
			dl: num(&args, 3, verb)?,
		},
		"palette" => match args.first() {
			Some(&"scroll") => Command::PaletteScroll { to: num(&args, 1, verb)? },
			_ => return Err("palette: expected scroll".into()),
		},
		"picker" => match args.first() {
			Some(&"filter") => {
				Command::PickerFilter { name: req_str(&args, 1, "picker filter: missing name (or `next`)")? }
			}
			Some(&"size") => Command::PickerSize { size: req_str(&args, 1, "picker size: missing px (or `next`)")? },
			Some(&"scroll") => Command::PickerScroll { to: num(&args, 1, verb)? },
			_ => return Err("picker: expected filter|size|scroll".into()),
		},
		"menu" => Command::MenuOpen { name: req_str(&args, 0, "menu: expected a menu title or `off`")? },
		"context-menu" => match args.first() {
			Some(&"off") => Command::ContextMenu { at: None },
			_ => Command::ContextMenu { at: Some((num(&args, 0, verb)?, num(&args, 1, verb)?)) },
		},
		"new-map" => match args.first() {
			None => Command::NewMapModal { picking: false },
			Some(&"packs") => Command::NewMapModal { picking: true },
			Some(other) => return Err(format!("new-map: unexpected '{other}' (or `packs`)")),
		},
		"window" => {
			let id = req_str(&args, 0, "window: missing id")?;
			let on = match args.get(1) {
				None | Some(&"toggle") => None,
				Some(&"on") => Some(true),
				Some(&"off") => Some(false),
				Some(other) => return Err(format!("window: expected on|off|toggle, got '{other}'")),
			};
			Command::Window { id, on }
		}
		"dock" => {
			let id = req_str(&args, 0, "dock: missing id")?;
			let place = req_str(&args, 1, "dock: missing place (left|right|top|bottom|float)")?;
			let at = match (args.get(2), args.get(3)) {
				(Some(x), Some(y)) => Some((
					x.parse().map_err(|_| format!("dock: bad x '{x}'"))?,
					y.parse().map_err(|_| format!("dock: bad y '{y}'"))?,
				)),
				_ => None,
			};
			Command::DockTo { id, place, at }
		}
		"reset-layout" | "reset-dialogs" => Command::ResetLayout,
		"save-settings" => Command::SaveSettings,
		"undo" => Command::Undo,
		"redo" => Command::Redo,
		"open" | "open!" => {
			let path = args.first().ok_or("open: missing path")?;
			Command::Open { path: PathBuf::from(path) }
		}
		"new-from-image" => {
			Command::NewFromImage { path: PathBuf::from(req_str(&args, 0, "new-from-image: missing path")?) }
		}
		"import-wrl" => Command::ImportWrl { path: PathBuf::from(req_str(&args, 0, "import-wrl: missing path")?) },
		"convert" => Command::Convert,
		"open-url" => Command::OpenUrl { url: req_str(&args, 0, "open-url: missing url")?.to_string() },
		"help-manual" => Command::HelpManual,
		"about" => Command::About,
		"save" => Command::Save { path: args.first().map(PathBuf::from) },
		"save-project" => Command::SaveProject,
		"tab" => Command::Tab { index: num(&args, 0, verb)? },
		"close-project" | "close-project!" => Command::CloseProject { force: verb.ends_with('!') },
		"save-and-close" => Command::SaveAndClose,
		"quit-request" => Command::QuitRequest,
		"save-and-quit" => Command::SaveAndQuit,
		"save-copy" => Command::SaveCopy { path: PathBuf::from(req_str(&args, 0, "save-copy: missing path")?) },
		"file-dialog" => Command::FileDialog {
			purpose: match args.first() {
				Some(&"load") => FilePurpose::Load,
				Some(&"save-as") => FilePurpose::SaveAs,
				Some(&"save-copy") => FilePurpose::SaveCopy,
				Some(&"load-palette") => FilePurpose::LoadPalette,
				Some(&"save-palette") => FilePurpose::SavePalette,
				Some(&"import-palette") => FilePurpose::ImportPalette,
				Some(&"export-palette") => FilePurpose::ExportPalette,
				Some(&"new-from-image") => FilePurpose::NewFromImage,
				Some(&"import-wrl") => FilePurpose::ImportWrl,
				Some(&"import-template") => FilePurpose::ImportTemplate,
				Some(&"export-template") => FilePurpose::ExportTemplate,
				Some(&"export-tile-png") => FilePurpose::ExportTilePng,
				Some(&"import-tile-png") => FilePurpose::ImportTilePng,
				Some(&"export-template-png") => FilePurpose::ExportTemplatePng,
				_ => {
					return Err("file-dialog: expected load|save-as|save-copy|load-palette|save-palette|\
						 new-from-image|import-wrl|import-template|export-template|export-tile-png|import-tile-png|\
						 export-template-png"
						.into());
				}
			},
		},
		"resize" => Command::Resize {
			width: num(&args, 0, verb)?,
			height: num(&args, 1, verb)?,
			off_x: if args.len() > 2 { num(&args, 2, verb)? } else { 0 },
			off_y: if args.len() > 3 { num(&args, 3, verb)? } else { 0 },
		},
		"resize-modal" => Command::ResizeModal,
		"fix-shore-modal" => Command::AutoFixModal,
		"export" => Command::Export { path: args.first().map(PathBuf::from) },
		"grid" => Command::Grid { on: on_off_toggle(&args, verb)? },
		"status-bar" => Command::StatusBar { on: on_off_toggle(&args, verb)? },
		"pass-overlay" => Command::PassOverlay { on: on_off_toggle(&args, verb)? },
		"animate" => Command::Animate { on: on_off_toggle(&args, verb)? },
		"ingame" => Command::InGame { on: on_off_toggle(&args, verb)? },
		"crt" => Command::Crt { on: on_off_toggle(&args, verb)? },
		"ui-scale" => {
			Command::UiScale { scale: parse_ui_scale(&req_str(&args, 0, "ui-scale: expected small|medium|large")?)? }
		}
		"map-palette" => Command::MapPalette { on: on_off_toggle(&args, verb)? },
		"convert-palette" => {
			let mut rasterize = false;
			let mut water = true;
			let mut relaxed = false;
			let mut threshold = 0.05f32;
			for a in &args {
				match *a {
					"match" => rasterize = false,
					"rasterize" => rasterize = true,
					"water=keep" => water = true,
					"water=drop" => water = false,
					"dedupe=strict" => relaxed = false,
					"dedupe=relaxed" => relaxed = true,
					_ => {
						if let Some(v) = a.strip_prefix("threshold=") {
							let pct: f32 = v.parse().map_err(|_| format!("convert-palette: bad threshold '{v}'"))?;
							if !(0.0..=100.0).contains(&pct) {
								return Err(format!("convert-palette: threshold {pct}% (0..=100)"));
							}
							threshold = pct / 100.0;
						} else {
							return Err(format!(
								"convert-palette: unexpected '{a}' (match|rasterize, water=keep|drop, \
								 dedupe=strict|relaxed, threshold=PCT)"
							));
						}
					}
				}
			}
			Command::ConvertPalette { rasterize, water, relaxed, threshold }
		}
		"convert-palette-modal" => Command::ConvertPaletteModal,
		"map-preferences" => Command::PreferencesModal,
		"tile-new" => Command::TilePaintNew,
		"tile-clone" => Command::TilePaintClone,
		"tile-edit" => Command::TilePaintEdit,
		"tile-delete" => Command::TileDelete,
		"tile-commit" => Command::TileCommit,
		"tile-export" => Command::TileExportPng { path: args.first().ok_or("tile-export: missing PATH.png")?.into() },
		"tile-import" => Command::TileImportPng { path: args.first().ok_or("tile-import: missing PATH.png")?.into() },
		"bake" => Command::Bake,
		"update-map" => Command::UpdateMap,
		"edit-cut" => Command::Edit(EditOp::Cut),
		"edit-copy" => Command::Edit(EditOp::Copy),
		"edit-paste" => Command::Edit(EditOp::Paste),
		"edit-delete" => Command::Edit(EditOp::Delete),
		"edit-select-all" => Command::Edit(EditOp::SelectAll),
		"tick" => Command::Tick { seconds: num(&args, 0, verb)? },
		"console" => Command::Console { on: on_off_toggle(&args, verb)? },
		"screenshot" => {
			let path = args.first().ok_or("screenshot: missing path")?;
			// Optional, order-independent: `crop=x,y,w,h`, `resize=WxH`.
			let mut crop = None;
			let mut resize = None;
			for a in &args[1..] {
				if let Some(v) = a.strip_prefix("crop=") {
					crop = Some(parse_crop(v).ok_or("screenshot: crop=x,y,w,h (four numbers)")?);
				} else if let Some(v) = a.strip_prefix("resize=") {
					resize = Some(parse_dims(v).ok_or("screenshot: resize=WxH")?);
				} else {
					return Err(format!("screenshot: unexpected '{a}' (use crop=x,y,w,h, resize=WxH)"));
				}
			}
			Command::Screenshot { path: PathBuf::from(path), crop, resize }
		}
		"hash" => Command::Hash,
		"assert-tile" => {
			Command::AssertTile { x: num(&args, 0, verb)?, y: num(&args, 1, verb)?, tile: num(&args, 2, verb)? }
		}
		"assert-hash" => {
			let raw = args.first().ok_or("assert-hash: missing hash")?;
			let raw = raw.strip_prefix("0x").unwrap_or(raw);
			let hash = u64::from_str_radix(raw, 16).map_err(|_| format!("assert-hash: bad hash '{raw}'"))?;
			Command::AssertHash { hash }
		}
		"assert-dirty" => Command::AssertDirty { dirty: num::<bool>(&args, 0, verb)? },
		"quit" | "quit!" => Command::Quit { force: verb.ends_with('!') },
		_ => return Err(format!("unknown command: {verb}")),
	};
	Ok(Some(command))
}

/// Parse a whole script; errors carry 1-based line numbers.
pub fn parse_script(text: &str) -> Result<Vec<Command>, String> {
	let mut commands = Vec::new();
	for (i, line) in text.lines().enumerate() {
		match parse_line(line) {
			Ok(Some(command)) => commands.push(command),
			Ok(None) => {}
			Err(e) => return Err(format!("line {}: {e}", i + 1)),
		}
	}
	Ok(commands)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_verbs_comments_and_blanks() {
		let script = "
			# paint a tile and check it
			set-tile 5 7 42
			assert-tile 5 7 42   # trailing comment
			zoom 1.5
			pan -2 3.5
			screenshot temp/x.png
			assert-hash 0xdeadbeef
			assert-dirty true
			quit
		";
		let commands = parse_script(script).unwrap();
		assert_eq!(commands.len(), 8);
		assert_eq!(commands[0], Command::SetTile { x: 5, y: 7, tile: 42 });
		assert_eq!(commands[3], Command::Pan { dx: -2.0, dy: 3.5 });
		assert_eq!(commands[5], Command::AssertHash { hash: 0xdead_beef });
		assert_eq!(commands[7], Command::Quit { force: false });
	}

	#[test]
	fn bang_variants_force() {
		let commands = parse_script("open! a.wrl\nquit!").unwrap();
		assert_eq!(commands[0], Command::Open { path: "a.wrl".into() });
		assert_eq!(commands[1], Command::Quit { force: true });
	}

	#[test]
	fn quoted_paths_survive_spaces_and_hashes() {
		// A space in the path (the project lives under "M.A.X. Projects/").
		let cmds = parse_script("open! \"/a/My Map.json\"").unwrap();
		assert_eq!(cmds[0], Command::Open { path: "/a/My Map.json".into() });
		// save + screenshot paths quote too.
		let cmds = parse_script("save \"out dir/map.json\"\nscreenshot \"temp/a b.png\"").unwrap();
		assert_eq!(cmds[0], Command::Save { path: Some("out dir/map.json".into()) });
		assert_eq!(cmds[1], Command::Screenshot { path: "temp/a b.png".into(), crop: None, resize: None });
		// A `#` inside quotes is literal, not a comment.
		let cmds = parse_script("open! \"a#b.json\"").unwrap();
		assert_eq!(cmds[0], Command::Open { path: "a#b.json".into() });
		// An unquoted trailing comment is still stripped.
		let cmds = parse_script("open plain.wrl   # a note").unwrap();
		assert_eq!(cmds[0], Command::Open { path: "plain.wrl".into() });
	}

	#[test]
	fn screenshot_crop_and_resize_parse() {
		// Bare path: no crop/resize.
		assert_eq!(
			parse_line("screenshot temp/a.png").unwrap().unwrap(),
			Command::Screenshot { path: "temp/a.png".into(), crop: None, resize: None }
		);
		// Both options, order-independent (resize before crop here).
		assert_eq!(
			parse_line("screenshot temp/a.png resize=800x600 crop=10,24,400,300").unwrap().unwrap(),
			Command::Screenshot { path: "temp/a.png".into(), crop: Some((10, 24, 400, 300)), resize: Some((800, 600)) }
		);
		// Malformed values and unknown args are rejected.
		assert!(parse_line("screenshot a.png crop=1,2,3").is_err());
		assert!(parse_line("screenshot a.png resize=800").is_err());
		assert!(parse_line("screenshot a.png 800x600").is_err());
	}

	#[test]
	fn new_tile_paint_stroke_parse() {
		let commands = parse_script(
			"new 64 48 GREEN 42\nnew! 8 8 WATER DESERT\ntile GLa000:!N\ntile -\ntile\npaint 3 7\nstroke begin\nstroke end",
		)
		.unwrap();
		assert_eq!(commands[0], Command::New { width: 64, height: 48, packs: vec!["GREEN".into()], seed: Some(42) });
		assert_eq!(
			commands[1],
			Command::New { width: 8, height: 8, packs: vec!["WATER".into(), "DESERT".into()], seed: None }
		);
		assert_eq!(commands[2], Command::Tile { spec: Some("GLa000:!N".into()) });
		assert_eq!(commands[3], Command::Tile { spec: Some("-".into()) });
		assert_eq!(commands[4], Command::Tile { spec: None });
		assert_eq!(commands[5], Command::Paint { x: 3, y: 7 });
		assert_eq!(commands[6], Command::Stroke { begin: true });
		assert_eq!(commands[7], Command::Stroke { begin: false });

		assert!(parse_script("stroke maybe").is_err());
		assert!(parse_script("paint 1").is_err());
	}

	#[test]
	fn ui_scale_parses_levels_and_rejects_garbage() {
		assert_eq!(parse_line("ui-scale small").unwrap().unwrap(), Command::UiScale { scale: 1.0 });
		assert_eq!(parse_line("ui-scale medium").unwrap().unwrap(), Command::UiScale { scale: 1.25 });
		assert_eq!(parse_line("ui-scale large").unwrap().unwrap(), Command::UiScale { scale: 1.5 });
		// Percentages are accepted too (scriptability).
		assert_eq!(parse_line("ui-scale 125%").unwrap().unwrap(), Command::UiScale { scale: 1.25 });
		assert_eq!(parse_line("ui-scale 150").unwrap().unwrap(), Command::UiScale { scale: 1.5 });
		// A missing or unknown level is an error, not a silent default.
		assert!(parse_line("ui-scale").is_err());
		assert!(parse_line("ui-scale huge").is_err());
	}

	#[test]
	fn fill_randomize_and_tools_parse() {
		assert_eq!(parse_line("fill 3 7").unwrap().unwrap(), Command::Fill { x: 3, y: 7 });
		assert!(parse_line("fill 1").is_err());
		assert_eq!(parse_line("randomize on").unwrap().unwrap(), Command::Randomize { on: Some(true) });
		assert_eq!(parse_line("randomize off").unwrap().unwrap(), Command::Randomize { on: Some(false) });
		assert_eq!(parse_line("randomize").unwrap().unwrap(), Command::Randomize { on: None });
		assert!(parse_line("randomize maybe").is_err());
		// The new tool names parse (state.rs validates the value).
		for name in ["pencil", "picker", "eraser", "fill"] {
			assert_eq!(
				parse_line(&format!("tool {name}")).unwrap().unwrap(),
				Command::ToolSelect { name: name.into() }
			);
		}
	}

	#[test]
	fn tab_and_close_project_parse() {
		assert_eq!(parse_line("tab 2").unwrap().unwrap(), Command::Tab { index: 2 });
		assert!(parse_line("tab").is_err());
		assert_eq!(parse_line("close-project").unwrap().unwrap(), Command::CloseProject { force: false });
		assert_eq!(parse_line("close-project!").unwrap().unwrap(), Command::CloseProject { force: true });
	}

	#[test]
	fn mode_and_pass_parse() {
		let cmds = parse_script("mode map\nmode pass\npass-pick 2\npass-paint 4 5 3").unwrap();
		assert_eq!(cmds[0], Command::Mode { name: "map".into() });
		assert_eq!(cmds[1], Command::Mode { name: "pass".into() });
		assert_eq!(cmds[2], Command::PassPick { value: 2 });
		assert_eq!(cmds[3], Command::PassPaint { x: 4, y: 5, value: 3 });
		assert!(parse_script("mode").is_err());
		assert!(parse_script("pass-paint 1 2").is_err());
	}

	#[test]
	fn file_ops_parse() {
		let cmds = parse_script(
			"save-project\nsave-copy temp/c.json\nfile-dialog load\nfile-dialog save-as\nfile-dialog save-copy",
		)
		.unwrap();
		assert_eq!(cmds[0], Command::SaveProject);
		assert_eq!(cmds[1], Command::SaveCopy { path: "temp/c.json".into() });
		assert_eq!(cmds[2], Command::FileDialog { purpose: FilePurpose::Load });
		assert_eq!(cmds[3], Command::FileDialog { purpose: FilePurpose::SaveAs });
		assert_eq!(cmds[4], Command::FileDialog { purpose: FilePurpose::SaveCopy });
		assert!(parse_script("save-copy").is_err());
		assert!(parse_script("file-dialog nonsense").is_err());
	}

	#[test]
	fn grid_and_pass_overlay_parse() {
		let cmds = parse_script("grid\ngrid on\ngrid off\npass-overlay toggle\npass-overlay on").unwrap();
		assert_eq!(cmds[0], Command::Grid { on: None });
		assert_eq!(cmds[1], Command::Grid { on: Some(true) });
		assert_eq!(cmds[2], Command::Grid { on: Some(false) });
		assert_eq!(cmds[3], Command::PassOverlay { on: None });
		assert_eq!(cmds[4], Command::PassOverlay { on: Some(true) });
		assert!(parse_script("grid nonsense").is_err());
	}

	#[test]
	fn show_only_layer_parses() {
		let cmds =
			parse_script("show-only-layer\nshow-only-layer toggle\nshow-only-layer on\nshow-only-layer off").unwrap();
		assert_eq!(cmds[0], Command::ShowOnlyLayer { on: None });
		assert_eq!(cmds[1], Command::ShowOnlyLayer { on: None });
		assert_eq!(cmds[2], Command::ShowOnlyLayer { on: Some(true) });
		assert_eq!(cmds[3], Command::ShowOnlyLayer { on: Some(false) });
		assert!(parse_script("show-only-layer nonsense").is_err());
	}

	#[test]
	fn delete_and_delete_all_parse() {
		assert_eq!(parse_line("delete").unwrap(), Some(Command::Delete));
		assert_eq!(parse_line("delete-all").unwrap(), Some(Command::DeleteAll));
	}

	#[test]
	fn template_rename_and_dedupe_parse() {
		// No args opens the modal; FROM TO renames (quotes carry spaces).
		assert_eq!(parse_line("template-rename").unwrap().unwrap(), Command::TemplateRenameModal);
		assert_eq!(
			parse_line("template-rename \"old name\" newname").unwrap().unwrap(),
			Command::TemplateRename { from: "old name".into(), to: "newname".into() }
		);
		assert!(parse_line("template-rename onlyone").is_err());
		// Dedupe: bare opens the modal, `!` performs the removal.
		assert_eq!(parse_line("template-dedupe").unwrap().unwrap(), Command::TemplateDedupeModal);
		assert_eq!(parse_line("template-dedupe!").unwrap().unwrap(), Command::TemplateDedupe);
		// Delete: bare opens the confirm modal, `!` deletes the selection, a name
		// deletes that template directly.
		assert_eq!(parse_line("template-delete").unwrap().unwrap(), Command::TemplateDeleteModal);
		assert_eq!(parse_line("template-delete!").unwrap().unwrap(), Command::TemplateDelete { name: None });
		assert_eq!(
			parse_line("template-delete \"my map\"").unwrap().unwrap(),
			Command::TemplateDelete { name: Some("my map".into()) }
		);
	}

	#[test]
	fn generate_parses() {
		assert_eq!(
			parse_line("generate islands").unwrap().unwrap(),
			Command::Generate {
				pattern: "islands".into(),
				water: 45,
				obstructions: 10,
				decorations: 5,
				seed: None,
				alt_shore: false,
			}
		);
		assert_eq!(
			parse_line("generate river-raid seed=42 water=30 obstructions=0 decorations=2 shore=alt").unwrap().unwrap(),
			Command::Generate {
				pattern: "river-raid".into(),
				water: 30,
				obstructions: 0,
				decorations: 2,
				seed: Some(42),
				alt_shore: true,
			}
		);
		assert_eq!(parse_line("generate-modal").unwrap().unwrap(), Command::GenerateModal);
		assert!(parse_line("generate").is_err());
		assert!(parse_line("generate islands water=101").is_err());
		assert!(parse_line("generate islands 42").is_err());
		assert!(parse_line("generate islands seed=x").is_err());
		assert!(parse_line("generate islands shore=loop").is_err());
	}

	#[test]
	fn shore_parses() {
		let commands =
			parse_script("shore\nshore 2 3 10 12\nshore alt\nshore alt 2 3 10 12\nshore fix\nshore fix 2 3 10 12")
				.unwrap();
		assert_eq!(commands[0], Command::Shore { region: None, mode: ShoreMode::Sweep });
		assert_eq!(commands[1], Command::Shore { region: Some((2, 3, 10, 12)), mode: ShoreMode::Sweep });
		assert_eq!(commands[2], Command::Shore { region: None, mode: ShoreMode::Alt });
		assert_eq!(commands[3], Command::Shore { region: Some((2, 3, 10, 12)), mode: ShoreMode::Alt });
		assert_eq!(commands[4], Command::Shore { region: None, mode: ShoreMode::Fix });
		assert_eq!(commands[5], Command::Shore { region: Some((2, 3, 10, 12)), mode: ShoreMode::Fix });
		assert!(parse_script("shore 1 2").is_err());
		assert!(parse_script("shore alt 1 2").is_err());
		assert!(parse_script("shore fix 1 2").is_err());
	}

	#[test]
	fn window_and_dock_parse() {
		let commands =
			parse_script("window tiles\nwindow tiles off\ndock minimap right\ndock minimap float 40 60").unwrap();
		assert_eq!(commands[0], Command::Window { id: "tiles".into(), on: None });
		assert_eq!(commands[1], Command::Window { id: "tiles".into(), on: Some(false) });
		assert_eq!(commands[2], Command::DockTo { id: "minimap".into(), place: "right".into(), at: None });
		assert_eq!(
			commands[3],
			Command::DockTo { id: "minimap".into(), place: "float".into(), at: Some((40.0, 60.0)) }
		);
		assert!(parse_script("window tiles maybe").is_err());
		assert!(parse_script("dock minimap").is_err());
	}

	#[test]
	fn picker_parses() {
		let commands =
			parse_script("picker filter water\npicker filter next\npicker size 48\npicker scroll 120").unwrap();
		assert_eq!(commands[0], Command::PickerFilter { name: "water".into() });
		assert_eq!(commands[1], Command::PickerFilter { name: "next".into() });
		assert_eq!(commands[2], Command::PickerSize { size: "48".into() });
		assert_eq!(commands[3], Command::PickerScroll { to: 120.0 });
		assert!(parse_script("picker").is_err());
		assert!(parse_script("picker scroll").is_err());

		let commands = parse_script("minimap pass").unwrap();
		assert_eq!(commands[0], Command::MinimapMode { mode: "pass".into() });
		assert!(parse_script("minimap").is_err());
	}

	#[test]
	fn palette_commands_parse() {
		let commands =
			parse_script("color 100\nset-color 100 aabbcc\nset-color 64 0a141e\nhsl-block 110 40 0 -10").unwrap();
		assert_eq!(commands[0], Command::Color { index: 100 });
		assert_eq!(commands[1], Command::SetColor { slot: 100, rgb: [0xaa, 0xbb, 0xcc] });
		assert_eq!(commands[2], Command::SetColor { slot: 64, rgb: [0x0a, 0x14, 0x1e] });
		assert_eq!(commands[3], Command::HslBlock { slot: 110, dh: 40.0, ds: 0.0, dl: -10.0 });
		assert!(parse_script("set-color 100 abc").is_err());
		// `#` starts a comment - the color argument vanishes, loudly.
		assert!(parse_script("set-color 100 #aabbcc").is_err());
		assert!(parse_script("hsl-block 110 40").is_err());
	}

	#[test]
	fn errors_carry_line_numbers() {
		let err = parse_script("fit\nnonsense 1 2").unwrap_err();
		assert!(err.starts_with("line 2:"), "{err}");

		let err = parse_script("set-tile 1 oops 3").unwrap_err();
		assert!(err.contains("bad argument 'oops'"), "{err}");

		let err = parse_script("set-tile 1").unwrap_err();
		assert!(err.contains("missing argument"), "{err}");
	}

	#[test]
	fn parses_text_edit_commands() {
		assert!(matches!(parse_line("edit-cut").unwrap(), Some(Command::Edit(EditOp::Cut))));
		assert!(matches!(parse_line("edit-copy").unwrap(), Some(Command::Edit(EditOp::Copy))));
		assert!(matches!(parse_line("edit-paste").unwrap(), Some(Command::Edit(EditOp::Paste))));
		assert!(matches!(parse_line("edit-delete").unwrap(), Some(Command::Edit(EditOp::Delete))));
		assert!(matches!(parse_line("edit-select-all").unwrap(), Some(Command::Edit(EditOp::SelectAll))));
	}

	#[test]
	fn parses_palette_manager_commands() {
		// A quoted name survives the tokenizer with its spaces.
		assert!(
			matches!(parse_line("palette-save-as \"My Pal\"").unwrap(), Some(Command::PaletteSaveAs { name }) if name == "My Pal")
		);
		// The arg-less forms open the modals.
		assert!(matches!(parse_line("palette-save-as").unwrap(), Some(Command::PaletteSaveModal)));
		assert!(matches!(parse_line("palette-rename").unwrap(), Some(Command::PaletteRenameModal)));
		assert!(matches!(parse_line("palette-delete").unwrap(), Some(Command::PaletteDeleteModal)));
		assert!(
			matches!(parse_line("palette-rename \"/u/a.json\" \"b\"").unwrap(), Some(Command::PaletteRename { to, .. }) if to == "b")
		);
		assert!(matches!(parse_line("palette-delete /u/p.json").unwrap(), Some(Command::PaletteDelete { .. })));
		assert!(matches!(parse_line("palette-import /tmp/x.json").unwrap(), Some(Command::PaletteImport { .. })));
	}

	#[test]
	fn tokenize_handles_quotes_concat_eol_and_comments() {
		// Quotes group spaces; bare-adjacent quotes concatenate into one token.
		assert_eq!(tokenize(r#"place "a b" 1"#).unwrap(), ["place", "a b", "1"]);
		assert_eq!(tokenize(r#"a"b"c"#).unwrap(), ["abc"], "quote splices mid-token");
		// An unterminated quote runs to end-of-line (lenient, no error).
		assert_eq!(tokenize(r#"open "x y"#).unwrap(), ["open", "x y"]);
		// An empty quote is a real (empty) token.
		assert_eq!(tokenize(r#"x """#).unwrap(), ["x", ""]);
		// `#` outside quotes starts a comment; a pure comment / blank line is None.
		assert_eq!(tokenize("cmd arg # tail").unwrap(), ["cmd", "arg"]);
		assert_eq!(tokenize("   # only a comment"), None);
		assert_eq!(tokenize("    "), None);
	}

	#[test]
	fn file_dialog_accepts_every_purpose_word() {
		use FilePurpose::*;
		let words = [
			("load", Load),
			("save-as", SaveAs),
			("save-copy", SaveCopy),
			("load-palette", LoadPalette),
			("save-palette", SavePalette),
			("import-palette", ImportPalette),
			("export-palette", ExportPalette),
			("new-from-image", NewFromImage),
			("import-wrl", ImportWrl),
			("import-template", ImportTemplate),
			("export-template", ExportTemplate),
			("export-tile-png", ExportTilePng),
			("import-tile-png", ImportTilePng),
			("export-template-png", ExportTemplatePng),
		];
		for (word, purpose) in words {
			assert_eq!(
				parse_line(&format!("file-dialog {word}")).unwrap(),
				Some(Command::FileDialog { purpose }),
				"{word}"
			);
		}
		assert!(parse_line("file-dialog bogus").is_err(), "unknown purpose word rejected");
	}

	#[test]
	fn template_export_png_parses_with_and_without_a_path() {
		// Bare opens the save dialog (the explorer context menu); a path is scripted.
		assert_eq!(parse_line("template-export-png").unwrap(), Some(Command::TemplateExportPng { path: None }));
		assert_eq!(
			parse_line("template-export-png \"temp/a b.png\"").unwrap(),
			Some(Command::TemplateExportPng { path: Some(PathBuf::from("temp/a b.png")) })
		);
	}

	#[test]
	fn help_menu_commands_parse() {
		assert_eq!(parse_line("help-manual").unwrap(), Some(Command::HelpManual));
		assert_eq!(parse_line("about").unwrap(), Some(Command::About));
		assert_eq!(
			parse_line("open-url https://example.com/x").unwrap(),
			Some(Command::OpenUrl { url: "https://example.com/x".into() })
		);
		assert!(parse_line("open-url").unwrap_err().contains("missing url"));
	}

	#[test]
	fn parse_argument_edges_report_errors() {
		// set-color: 6 non-hex chars pass the length gate but fail from_str_radix.
		assert!(parse_line("set-color 64 gggggg").unwrap_err().contains("bad hex"));
		assert!(parse_line("set-color 64 fff").unwrap_err().contains("want rrggbb"), "wrong length");
		// convert-palette threshold=: non-numeric and out-of-range.
		assert!(parse_line("convert-palette match threshold=abc").unwrap_err().contains("bad threshold"));
		assert!(parse_line("convert-palette match threshold=150").unwrap_err().contains("0..=100"));
		// dock: non-numeric float coordinates.
		assert!(parse_line("dock palette float zz 4").unwrap_err().contains("bad x"));
		assert!(parse_line("dock palette float 4 zz").unwrap_err().contains("bad y"));
	}
}
