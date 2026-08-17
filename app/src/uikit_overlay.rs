//! Dialogs rendered by the first-party `wgpu-ui` toolkit, composited over the
//! editor frame by sharing this editor's `wgpu::Device`/`Queue` (no second
//! surface). They use the MAX UI font (`assets/MAX_Redesign_Square.ttf`) and a brushed-
//! steel [`SteelTheme`](crate::uikit_theme) (the editor's own steel sheet), so
//! they sit native over the chrome, and lay out at the editor's `ui_scale` so
//! their controls are sized like the editor's own.
//!
//! Four dialogs so far, all retained widget trees:
//! - **About** (Help -> About, or F1): name/version, credits, and Website/GitHub/
//!   Close - matching the bespoke modal's content.
//! - **Map Metadata** (Map menu): name/players/description/date/version/
//!   author, read from the project on open, written via `Project::set_info`.
//! - **New Map** (File menu): size preset, a W/H line, the palette selector,
//!   and the scrolling tile-set / water-tiles pickers with live preview
//!   strips - creating the map through the same `Command::New` path scripts use.
//! - **Resize Map** (Map menu): size preset + W/H + a 3x3 anchor radio grid +
//!   a live offset note, running the same `resize …` command line.
//!
//! The editor's frame loop touches this in two places only: an event intercept
//! in `window_event` and a `render_into` after `render_frame`.

use std::path::PathBuf;

use wgpu_ui::widget::Length;
use wgpu_ui::widget::{CrossAlign, MainAlign};
use wgpu_ui::{
	Button, Charset, Checkbox, Constrained, DrawList, Event, Fill, Image, Insets, Key, Label, Linear, ProgressBar,
	Radio, Rgba, ScrollArea, Select, Size, Spacer, Stack, Tabs, TexRect, TextArea, TextInput, TextureId, Ui, Vec2,
	Widget, WidgetId, Window,
};

use crate::fontprobe::FontProbe;
use crate::genform;
use crate::newmap::{PREVIEW_TILES, PaletteChoice, SIZE_PRESETS};
use crate::packlist::{self, PackEntry};
use crate::savedata::{self, SaveDataForm, SaveDataInit};
use crate::uikit_menu::MenuChrome;
use crate::uikit_theme::{InsetFrame, Well};

mod autofix;
mod confirm;
mod convert_palette;
mod generate;
mod match_edit;
mod new_image;
mod new_map;
mod prompts;
mod resize;
mod save_data;
mod scenery_new;
mod tile_paint;

use autofix::AutoFixIds;
use confirm::ConfirmIds;
use convert_palette::ConvertPaletteIds;
use generate::GenerateIds;
use match_edit::MatchEditIds;
use new_image::NewImageIds;
use new_map::{ImportWrlIds, NewMapIds};
use prompts::{NamePromptIds, ObjectFieldIds, PaletteNameIds, RenameTemplateIds};
use resize::ResizeIds;
use save_data::SaveDataIssuesIds;
use scenery_new::SceneryNewIds;
use tile_paint::TilePaintIds;

/// The red ink for a dialog warning line, matching the editor's defect red
/// (`theme::DEFECT` = 0.95/0.16/0.13 in sRGB) so the alert reads the same
/// everywhere.
const WARNING_INK: Rgba = Rgba::rgb(242, 41, 33);

/// The very-dark-red ground behind a boxed warning line (an inset [`Well`] wash),
/// so the bright [`WARNING_INK`] text reads as a pronounced alert block.
const WARNING_GROUND: Rgba = Rgba::rgb(48, 8, 8);

/// The two links the About dialog opens (Help menu uses the same `open-url`).
const WEBSITE: &str = "https://suns-echoes.github.io/max-map-editor/";
const GITHUB: &str = "https://github.com/suns-echoes/max-map-editor";

/// New Map preview tiles at their native 64px, each in an inset frame.
const STRIP_TILE: f32 = 64.0;
/// A picker item's inner padding (its own inset well).
const PICK_PAD: f32 = 3.0;
/// One tile-set picker item: a padded well holding the name row (control
/// height) above its tile strip.
const PICK_ITEM_H: f32 = PICK_PAD + 24.0 + 2.0 + STRIP_TILE + PICK_PAD;
/// Gap between picker items.
const PICK_SPACING: f32 = 6.0;
/// The pick lists' well padding (the darkened inset container).
const LIST_PAD: f32 = 4.0;
/// The land list shows 3 items at once, the water list 2; more scroll.
const LAND_LIST_H: f32 = 3.0 * PICK_ITEM_H + 2.0 * PICK_SPACING + 2.0 * LIST_PAD;
const WATER_LIST_H: f32 = 2.0 * PICK_ITEM_H + PICK_SPACING + 2.0 * LIST_PAD;
/// Gap between the picker items and the lists' scrollbar.
const LIST_GAP: f32 = 4.0;
/// The New Map dialog's content width (374px window minus the window pads;
/// the extra 4px over the original 370 buys [`LIST_GAP`]).
const NEWMAP_W: f32 = 358.0;

/// UI Tests: the probe's viewport height. The sheet is taller than this at
/// every scale (it is deliberately exhaustive), so it scrolls; this is the
/// height that keeps the window on a 1080p screen even at 150%.
const UITEST_H: f32 = 500.0;

/// Map Metadata field values - read from the project on open, returned on Save.
#[derive(Default)]
pub struct MetadataValues {
	pub name: String,
	pub players: Option<u8>,
	pub description: String,
	pub date: String,
	pub version: String,
	pub author: String,
}

/// New Map field values, returned on Create (ordered pack names for `new`).
pub struct NewMapValues {
	pub width: u16,
	pub height: u16,
	pub packs: Vec<String>,
	/// A custom palette to load right after creation (the selector's
	/// non-default choice); `None` = "from selected tileset" (the owner
	/// radio's pack palette, adopted by `Project::new`).
	pub palette: Option<PathBuf>,
}

/// What the host should do after a `render` (polled once per frame).
pub enum Outcome {
	Idle,
	/// Map Metadata saved: apply via `Project::set_info`. `save_after` = the
	/// dialog ran as a first-save prompt, so resume the Save-As file dialog
	/// (setting [`EditorState::first_save_meta`] to skip re-prompting).
	ApplyMetadata {
		vals: MetadataValues,
		save_after: bool,
	},
	/// Editor Preferences Save: the (possibly blank) M.A.X. / M.A.X. Port folder
	/// paths and the "don't ask again" flag, to persist + apply.
	ApplyPreferences {
		max_path: String,
		max_port_path: String,
		max_port_data_path: String,
		skip_prompt: bool,
	},
	/// Edit Save Data OK with a valid form: the settings block to apply as one
	/// undoable step (`EditorState::apply_save_data`). Validation already ran —
	/// an invalid form never reaches this.
	ApplySaveData(Box<max_assets::save::SaveSettings>),
	/// The user cancelled a Preferences dialog that a missing-path action opened
	/// ("required") — the shell shows the Attention notice if paths are still set.
	PrefsCancelledRequired,
	CreateMap(NewMapValues),
	/// The New Map / Import WRL preview strips need a rebuild (the palette /
	/// water choice changed to an uncached combination): the host composes the
	/// atlas (`newmap::build_rgba` with these choices), registers it, and pokes
	/// it back via [`Overlay::provide_preview_tex`]. The dialog stays open.
	NewMapPreview {
		palette: Option<PathBuf>,
		water: String,
		key: (usize, usize),
	},
	/// Create an all-water map of this size + packs, then carve the chosen PNG's
	/// land/water shape in and open Fix Shore (New Map opened with a shape
	/// image via File → New Terrain from Image).
	CreateShapedMap {
		width: u16,
		height: u16,
		packs: Vec<String>,
		palette: Option<PathBuf>,
		image: PathBuf,
	},
	/// A validated `resize W H OFFX OFFY` command line (run via the command path).
	ResizeMap(String),
	/// A command line to run (a confirm dialog's primary button emits the same
	/// line the bespoke modal did, e.g. `palette-delete "<path>"`).
	RunCommand(String),
	OpenUrl(String),
	/// Fix Shore verbs (the run itself lives on the editor; the shell drives
	/// start/step/abort and re-syncs this dialog each frame).
	FixStart,
	FixStop,
	FixAbort,
	/// The Fix Shore window closed (Close / Escape while idle) — the shell
	/// clears the run state (and the red defect outlines with it).
	FixClose,
	/// Convert Palette: begin the stepped rasterize run with the dialog's
	/// validated options (`threshold` is a 0..=1 fraction).
	PaletteConvertStart {
		water: bool,
		relaxed: bool,
		threshold: f32,
	},
	/// Abort the running rasterize (back to the options stage).
	PaletteConvertAbort,
	/// The Convert Palette dialog closed — drop any parked run state.
	PaletteConvertCancel,
	/// New from Image: begin the stepped conversion with the validated options.
	NewImageStart(map_core::ConvertOpts),
	/// Abort the running conversion (back to settings).
	NewImageAbort,
	/// The New-from-Image dialog closed — drop the run.
	NewImageCancel,
	/// Generate: begin a run with the validated settings (`None` seed rolls
	/// fresh); the dialog stays open showing progress, then the report.
	GenerateStart {
		params: map_core::GenParams,
		seed: Option<u64>,
	},
	/// Abort the running generation, rolling the document back.
	GenerateAbort,
	/// The Generate dialog closed — the shell stores the session memory
	/// (per-generator last-used settings) and drops the run state.
	GenerateClose(crate::genform::GenMemory),
	/// Import WRL: run the match against the picker's selected packs (WATER
	/// first, owner next — [`packlist::selected`] order).
	WrlMatch {
		packs: Vec<String>,
		owner: String,
	},
	/// Commit the parked import with this destination for the unmapped tiles.
	WrlFinish {
		dest: map_core::ExtrasDest,
	},
	/// The review stage stepped back to the picker — drop the parked match.
	WrlBack,
	/// The Import WRL dialog closed — drop the parked import.
	WrlCancel,
	/// Tile Painter: commit the painted canvas (the dialog's typed id, chosen
	/// passability, and target pack ride along - a command line can't carry
	/// 4 KiB of pixels). The dialog stays open; the shell hides it on success
	/// or pushes the failure back in via [`Overlay::tile_paint_error`].
	TileCommit {
		id: String,
		pass: u8,
		pack: String,
		pixels: Vec<u8>,
	},
	/// Copy the painter's canvas to the shell's tile clipboard (so it survives
	/// closing the dialog; the dialog keeps its own working copy for Paste).
	TileCopy(Vec<u8>),
	/// Export the painter's canvas to a PNG (the shell runs the native save
	/// dialog; `id` suggests the file name).
	TileExportPng {
		id: String,
	},
	/// Import a PNG over the painter's canvas (native open dialog; the editor
	/// writes the mapped pixels into the run and bumps its revision).
	TileImportPng,
	/// The Tile Painter closed (Cancel / Escape) — drop the run context.
	TilePaintClose,
	/// New Scenery: a threshold moved, so the source has to be rasterized
	/// again. The dialog keeps the new thresholds and does the work itself in
	/// the next `sync` (only the shell has the source image in hand there);
	/// this exists so that frame is actually asked for.
	SceneryRederive,
	/// New Scenery: pick a PNG to author from (native open dialog; the editor
	/// writes it into the run and bumps its revision).
	SceneryImportPng,
	/// New Scenery, Heightmap tab: pick a painted height map to read the
	/// piece's relief off, on exactly the same terms.
	SceneryImportHeightPng,
	/// New Scenery, Heightmap tab: write the relief out as a greyscale picture
	/// to paint on. The bytes ride along because the dialog is what holds the
	/// derived field; the shell parks them on the run and opens the save
	/// picker.
	SceneryExportHeightPng {
		grey: Vec<u8>,
		w: u32,
		h: u32,
	},
	/// New Scenery: file the derived piece (the planes can't ride a command
	/// line). The dialog stays open; the shell hides it on success or pushes
	/// the failure back in via [`Overlay::scenery_new_error`].
	SceneryCommit {
		pack: String,
		id: String,
		name: String,
		sprite: map_core::Sprite,
		pass: Vec<u8>,
		cells: (u16, u16),
		/// The `Stands:` choice - see `scenery_new::RELIEFS`.
		relief: Option<(u8, bool)>,
		/// The Heightmap tab's drawn relief in the sprite's frame, or `None` to
		/// leave the whole field inferred from the art.
		height: Option<Vec<u8>>,
	},
	/// New Scenery closed (Cancel / Escape) — drop the run context.
	SceneryNewClose,
	/// Match editor: apply + write the staged pack commits (symmetrized). The
	/// dialog stays open; the shell acks success via [`Overlay::match_saved`]
	/// or failure via [`Overlay::match_error`].
	MatchSave(Vec<crate::matcheditor::PackCommit>),
	/// The match editor closed (Close / Escape) — staged edits discarded.
	MatchClose,
}

#[derive(Clone, Copy)]
struct AboutIds {
	website: WidgetId,
	github: WidgetId,
	close: WidgetId,
}

#[derive(Clone, Copy)]
struct MetadataIds {
	save: WidgetId,
	cancel: WidgetId,
	name: WidgetId,
	players: WidgetId,
	description: WidgetId,
	date: WidgetId,
	version: WidgetId,
	author: WidgetId,
}

#[derive(Clone, Copy)]
struct PrefsIds {
	save: WidgetId,
	cancel: WidgetId,
	max_path: WidgetId,
	max_path_browse: WidgetId,
	max_port_path: WidgetId,
	max_port_browse: WidgetId,
	max_port_data: WidgetId,
	max_port_data_browse: WidgetId,
	skip: WidgetId,
}

#[derive(Clone, Copy)]
enum Dialog {
	None,
	About(AboutIds),
	Metadata(MetadataIds),
	Preferences(PrefsIds),
	NewMap(NewMapIds),
	Resize(ResizeIds),
	Confirm(ConfirmIds),
	PaletteName(PaletteNameIds),
	RenameTemplate(RenameTemplateIds),
	/// A one-field name prompt (Rename Scenery); Save emits `<verb> "typed"`.
	NamePrompt(NamePromptIds),
	/// The Unit Properties one-field editor (name / hits / ammo); on OK it emits
	/// `object-edit <field> <value>` (S4.2b).
	ObjectField(ObjectFieldIds),
	/// The non-blocking Fix Shore window (live run state lives on the editor).
	AutoFix(AutoFixIds),
	/// Convert to Compatible Palette (options stage ↔ running stage; the run
	/// lives on the editor).
	ConvertPalette(ConvertPaletteIds),
	/// New from Image (settings stage ↔ running stage; the run lives on the
	/// editor).
	NewImage(NewImageIds),
	/// Generate Random Terrain (a non-blocking float; the run lives on the
	/// editor, the per-generator form is rebuilt on switch).
	Generate(GenerateIds),
	/// Import WRL (pack picker stage ↔ unmapped-review stage; the parked
	/// import lives on the editor).
	ImportWrl(ImportWrlIds),
	/// The Tile Painter (the working canvas + tool state live on the overlay;
	/// [`crate::state::EditorState::tilepaint`] mirrors the canvas for command
	/// paths).
	TilePaint(TilePaintIds),
	/// New Scenery: the derived piece, the recolour map and the sub-palette
	/// selection live on the overlay; [`crate::state::EditorState::scenerypaint`]
	/// holds the *source image* the whole thing is re-derived from.
	SceneryNew(SceneryNewIds),
	/// The Edit Tile Match Data editor (DEV); the staged model
	/// ([`crate::matcheditor::MatchEditor`]) lives on the overlay, and Save
	/// hands self-contained pack commits to the shell.
	MatchEdit(MatchEditIds),
	/// A dismiss-only error acknowledgement; the id is its OK button.
	Error(WidgetId),
	/// **UI Tests** (DEV): the font/raster probe. Dismiss-only - the id is its
	/// Close button; everything it shows is derived from the theme and the UI
	/// scale in force, so it carries no state of its own.
	UiTests(WidgetId),
	/// Edit Save Data (S7.2): the tabbed non-map settings form. The canonical
	/// values live in [`Overlay::sd_form`]; the visible tab's widgets are
	/// captured into it every frame, and a tab/team switch rebuilds the tree
	/// from it. Its (many) widget ids live in [`Overlay::sd_ids`] so this
	/// variant doesn't dwarf the enum.
	SaveData,
	/// The Edit Save Data validation list: every invalid field with how to fix
	/// it, plus Back (fix by hand) / Auto Fix (nearest valid values). The form
	/// state stays parked in [`Overlay::sd_form`] while this shows.
	SaveDataIssues(SaveDataIssuesIds),
}

pub struct Overlay {
	ui: Ui,
	/// The editor's `ui_scale`, cached from `set_scale`. The shared renderer +
	/// steel theme (owned by [`MenuChrome`]) are scaled to it at render time.
	scale: f32,
	/// Router-translated events buffered since the last render, drained by
	/// [`Overlay::render`] (dispatch runs there so hit-testing uses the geometry
	/// the user actually saw).
	events: Vec<Event>,
	visible: bool,
	dialog: Dialog,
	/// The current dialog's window, so the steel grain can be anchored to it
	/// (fixed-to-window background) after layout each frame.
	win_id: Option<WidgetId>,
	// The New Map / Import WRL pack-picker machinery below is the one dialog
	// cluster deliberately NOT folded into a per-dialog state struct: two
	// dialogs share it, `preview_cache` lives for the session, and
	// `nm_preview_changed` must survive `hide` (see its comment there) - a
	// wholesale-replace lifecycle would be wrong for half of it.
	/// New Map tile-set state: the scanned packs and their picker control ids
	/// (parallel: a checkbox for land packs, the water-choice radio for water
	/// packs).
	packs: Vec<PackEntry>,
	pack_ids: Vec<WidgetId>,
	/// New Map palette-owner radio per pack (parallel to `packs`; `None` for
	/// water / palette-less rows). One group: selecting one clears the others.
	palette_ids: Vec<Option<WidgetId>>,
	/// Per-pack preview-strip `Image` ids (parallel to `packs`), so a palette /
	/// water change swaps the atlas in place.
	preview_img_ids: Vec<WidgetId>,
	/// The preview atlases by (palette choice, water pack index) - built on
	/// demand by the host ([`Outcome::NewMapPreview`]) and kept for the session
	/// (the pack set is fixed; the palette list clears it when it changes).
	preview_cache: std::collections::HashMap<(usize, usize), TextureId>,
	/// The combination requested from the host and not yet provided.
	preview_want: Option<(usize, usize)>,
	/// The New Map palette selector's rows (index-aligned with its options).
	nm_palettes: Vec<PaletteChoice>,
	/// The palette selector's widget (NONE while Import WRL / closed - its
	/// pickers have no palette row).
	nm_palette_sel: WidgetId,
	/// The "Palette preview" checkbox (NONE while Import WRL / closed).
	nm_preview: WidgetId,
	/// A toggled preview value not yet taken by the shell (which mirrors it
	/// into the persisted `[Preferences]`).
	nm_preview_changed: Option<bool>,
	/// New Map's optional land/water shape image (File → New Terrain from
	/// Image opened the dialog with it); `Create` then carves it in.
	newmap_shape: Option<PathBuf>,
	/// Map Metadata opened for a first save: its Save reports
	/// `save_after: true` so the shell resumes the Save-As file dialog.
	meta_save_after: bool,
	/// The open Preferences dialog was opened by a missing-path action ("required"),
	/// so a cancel emits [`Outcome::PrefsCancelledRequired`] (→ Attention notice).
	prefs_required: bool,
	/// Resize: the 9 anchor radio ids (row-major, 0..9) and the original map size.
	anchor_ids: Vec<WidgetId>,
	resize_old: (u16, u16),
	/// Confirm dialog state - the armed command lines, reset by [`Overlay::hide`].
	confirm: confirm::ConfirmState,
	/// Object-field prompt state ([`Dialog::ObjectField`]), reset by [`Overlay::hide`].
	of: prompts::ObjectFieldState,
	/// One reusable texture slot for a modal's composed template thumbnail
	/// (Delete / Rename); replaced (not re-registered) per open, so it never
	/// leaks a slot per dialog.
	template_tex: Option<TextureId>,
	/// Palette Save/Rename prompt state, reset by [`Overlay::hide`].
	pn: prompts::PaletteNameState,
	/// Rename Template prompt state, reset by [`Overlay::hide`].
	rt: prompts::RenameTemplateState,
	/// Name-prompt state, reset by [`Overlay::hide`].
	np: prompts::NamePromptState,
	/// New Scenery dialog state - one struct, replaced wholesale by
	/// [`Overlay::open_scenery_new`] (its struct literal is the complete
	/// reseed, so a new field cannot be forgotten there; `hide` leaves it).
	sn: scenery_new::SceneryNewState,
	/// The registered New Scenery preview/swatch texture slots, kept across
	/// opens (replaced in place, never re-registered).
	sn_preview_tex: Option<TextureId>,
	sn_swatch_tex: Option<TextureId>,
	/// The Heightmap tab.s relief picture.
	sn_height_tex: Option<TextureId>,
	/// Whether the open dialog dims + blocks the editor beneath (default) or
	/// floats over the live map (Fix Shore: pointer input outside its window
	/// falls through, no scrim).
	blocking: bool,
	/// Convert Palette dialog state - replaced wholesale by
	/// [`Overlay::open_convert_palette`] (`hide` leaves it; open is the reset).
	cp: convert_palette::ConvertPaletteState,
	/// New-from-Image dialog state - replaced wholesale by
	/// [`Overlay::open_new_image`] (`hide` leaves it; open is the reset).
	ni: new_image::NewImageState,
	/// Generate dialog state - replaced wholesale by [`Overlay::open_generate`]
	/// and reset by [`Overlay::hide`].
	generate: generate::GenerateState,
	/// The picker's explicit palette-owner choice (`None` = first selected
	/// palette pack), preserved across a stage rebuild (New Map / Import WRL).
	owner_choice: Option<String>,
	/// Import WRL: the WRL's base name + header info, the effective owner label
	/// (the user-tileset destination), and whether the review stage is showing.
	wrl_name: String,
	wrl_info: (u16, u16, u16),
	wrl_owner: String,
	wrl_unmapped: bool,
	/// Fix Shore: the run's live/idle state, synced by the shell each frame
	/// (flips the button labels + what Escape means).
	af_running: bool,
	/// Tile Painter dialog state - one struct, replaced wholesale by
	/// [`Overlay::open_tile_paint`] and reset by [`Overlay::hide`].
	tp: tile_paint::TilePaintState,
	/// Reusable texture slots for the composed canvas (64×64) and palette grid
	/// (16×16) - registered once, updated in place per change.
	tp_canvas_tex: Option<TextureId>,
	tp_swatch_tex: Option<TextureId>,
	/// Match editor: the staged model (dialog-owned; Save emits self-contained
	/// commits), the reusable strip-texture slot (main tile + candidate at all
	/// 8 orientations, composed by the shell), and the shared tile atlas the
	/// list thumbnails uv (`(tex, total count, active pack's base index)`).
	me: Option<crate::matcheditor::MatchEditor>,
	me_strip_tex: Option<TextureId>,
	me_atlas: Option<(TextureId, u32, u32)>,
	/// Edit Save Data dialog state - one struct, replaced wholesale by
	/// [`Overlay::open_save_data`] and reset by [`Overlay::hide`].
	sd: save_data::SaveDataState,
	/// The last dispatch's pointer-capture state — a non-blocking window being
	/// dragged keeps owning pointer events even outside its rect.
	capturing: bool,
}

impl Overlay {
	/// Builds the overlay. It shares the menu's [`MenuChrome`] renderer / steel
	/// theme / fonts (passed at `open_newmap`/`render` time), so the MAX font is
	/// parsed once and the steel sheet registered once for the whole wgpu-ui side.
	pub fn new(scale: f64) -> Self {
		Self {
			ui: Ui::new(Label::new("")),
			scale: scale as f32,
			events: Vec::new(),
			visible: false,
			dialog: Dialog::None,
			win_id: None,
			packs: Vec::new(),
			pack_ids: Vec::new(),
			palette_ids: Vec::new(),
			preview_img_ids: Vec::new(),
			preview_cache: std::collections::HashMap::new(),
			preview_want: None,
			nm_palettes: Vec::new(),
			nm_palette_sel: WidgetId::NONE,
			nm_preview: WidgetId::NONE,
			nm_preview_changed: None,
			newmap_shape: None,
			meta_save_after: false,
			prefs_required: false,
			anchor_ids: Vec::new(),
			resize_old: (0, 0),
			confirm: Default::default(),
			of: Default::default(),
			template_tex: None,
			pn: Default::default(),
			rt: Default::default(),
			np: Default::default(),
			sn: Default::default(),
			sn_preview_tex: None,
			sn_swatch_tex: None,
			sn_height_tex: None,
			blocking: true,
			cp: Default::default(),
			ni: Default::default(),
			generate: Default::default(),
			owner_choice: None,
			wrl_name: String::new(),
			wrl_info: (0, 0, 0),
			wrl_owner: String::new(),
			wrl_unmapped: false,
			af_running: false,
			tp: Default::default(),
			tp_canvas_tex: None,
			tp_swatch_tex: None,
			me: None,
			me_strip_tex: None,
			me_atlas: None,
			sd: Default::default(),
			capturing: false,
		}
	}

	/// Whether the open dialog dims + blocks everything beneath it. A
	/// non-blocking window (Fix Shore) takes only the pointer events over
	/// itself — see [`wants_pointer_at`](Self::wants_pointer_at).
	pub fn blocking(&self) -> bool {
		self.blocking
	}

	/// Whether a pointer event at `logical` (px) belongs to the non-blocking
	/// dialog: over its window, or while its Ui captures a drag (titlebar
	/// moves keep routing here even when the cursor leaves the window).
	pub fn wants_pointer_at(&self, logical: Vec2) -> bool {
		self.capturing
			|| self
				.win_id
				.and_then(|id| self.ui.get::<Window>(id))
				.is_some_and(|w| wgpu_ui::Widget::rect(w).contains(logical))
	}

	pub fn visible(&self) -> bool {
		self.visible
	}

	/// The mouse cursor the dialog's widgets want at the current pointer
	/// (I-beam over text fields, resize arrows on a window grip) — the shell
	/// applies it while the overlay owns the pointer.
	pub fn cursor_icon(&self) -> wgpu_ui::CursorIcon {
		self.ui.cursor_icon()
	}

	/// True while the dialog's focused widget is a text field — the shell
	/// mirrors this into the OS IME (`set_ime_allowed`).
	pub fn wants_text_input(&self) -> bool {
		self.ui.wants_text_input()
	}

	/// The focused text field's caret rect (logical px), anchoring the OS IME
	/// candidate window (`set_ime_cursor_area`).
	pub fn ime_rect(&self) -> Option<wgpu_ui::Rect> {
		self.ui.ime_rect()
	}

	/// Matches the editor's `ui_scale` so controls are sized like the chrome.
	/// The shared renderer/theme are scaled to this at render time; the router
	/// scales incoming events to it.
	pub fn set_scale(&mut self, scale: f64) {
		self.scale = scale as f32;
	}

	pub fn hide(&mut self) {
		self.visible = false;
		self.dialog = Dialog::None;
		self.win_id = None;
		self.packs.clear();
		self.pack_ids.clear();
		self.palette_ids.clear();
		self.preview_img_ids.clear();
		self.preview_want = None;
		self.nm_palette_sel = WidgetId::NONE;
		self.nm_preview = WidgetId::NONE;
		// `nm_preview_changed` is deliberately NOT cleared: a toggle in the
		// same event batch as a close must still reach the shell's take.
		self.anchor_ids.clear();
		self.confirm = Default::default();
		self.pn = Default::default();
		self.rt = Default::default();
		self.np = Default::default();
		self.of = Default::default();
		self.blocking = true;
		self.af_running = false;
		self.generate = Default::default();
		self.wrl_unmapped = false;
		self.tp = Default::default();
		self.me = None;
		self.me_atlas = None;
		self.sd = Default::default();
		self.capturing = false;
		self.events.clear();
		// `preview_cache` is kept: the pack set is fixed for the session, so
		// each (palette, water) atlas is built at most once. The painter
		// texture slots (`tp_canvas_tex`/`tp_swatch_tex`) are kept too - fixed
		// sizes, reused per open.
	}

	/// Opens the About dialog: the editor's name/version, tagline, copyright, and
	/// Website / GitHub / Close buttons - matching the bespoke modal's content.
	pub fn open_about(&mut self) {
		let website = Button::new("Website");
		let github = Button::new("GitHub");
		let close = Button::new("Close").primary();
		let ids = AboutIds { website: website.id(), github: github.id(), close: close.id() };
		// Credits in a darker inset well, like the bespoke dialog. Content at the
		// body size (16px); blank lines stay small, just as paragraph spacing.
		let credits = column()
			.push(Label::new(format!("M.A.X. Map Editor v{}", env!("CARGO_PKG_VERSION"))))
			.push(Label::new(" ").small())
			.push(Label::new("The map `Utility` for M.A.X.:"))
			.push(Label::new("Mechanized Assault & Exploration"))
			.push(Label::new(" ").small())
			.push(Label::new("by MAX Commander for MAX Commanders"))
			.push(Label::new(" ").small())
			.push(Label::new("(c) Aneta Suns"));
		// Website / GitHub at the left, Close at the right (a flex spacer between
		// them no longer inflates the dialog - it sizes to the credits).
		let buttons = Linear::row()
			.spacing(6.0)
			.cross_align(CrossAlign::Center)
			.child(website, Length::Fit)
			.child(github, Length::Fit)
			.child(Label::new(""), Length::Flex(1.0))
			.child(close, Length::Fit);
		let content = column().push(Well::new(credits)).push(buttons);
		let win = dialog("About", content);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.dialog = Dialog::About(ids);
		self.events.clear();
		self.visible = true;
	}

	/// Opens **UI Tests** (DEV > UI Tests): the font/raster probe. Every chrome
	/// role rendered raised, engraved and flat, then a ladder of physical em
	/// sizes, white on black - see [`crate::fontprobe`] for what each band
	/// answers. Dismiss-only; the probe derives everything it draws from the
	/// theme and the UI scale, so re-opening it after `ui-scale` shows the new
	/// numbers with nothing to carry over.
	pub fn open_ui_tests(&mut self) {
		let close = Button::new("Close").primary();
		let ids = close.id();
		let content =
			column().child(Well::new(ScrollArea::new(FontProbe::new())).padding(2.0), Length::Fixed(UITEST_H)).push(
				Linear::row()
					.spacing(6.0)
					.cross_align(CrossAlign::Center)
					.child(Label::new("").small(), Length::Flex(1.0))
					.child(close, Length::Fit),
			);
		let win = dialog("UI Tests: Font Raster", content);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.dialog = Dialog::UiTests(ids);
		self.events.clear();
		self.visible = true;
	}

	/// Opens the Map Metadata form, populated from the current project.
	/// `save_after` = the first-save flow: Save reports it back so the shell
	/// resumes the interrupted Save-As.
	pub fn open_metadata(&mut self, vals: MetadataValues, save_after: bool) {
		let name = TextInput::with_text(&vals.name);
		// Player counts read as ranges from the 2-player floor ("2-4" = up to
		// four); only the max is stored (`Project::players`).
		let players = Select::new(["Unspecified", "2-2", "2-3", "2-4"]).with_selected(match vals.players {
			Some(2) => 1,
			Some(3) => 2,
			Some(4) => 3,
			_ => 0,
		});
		let description = TextArea::with_text(&vals.description);
		let date = TextInput::with_text(&vals.date);
		let version = TextInput::with_text(&vals.version);
		let author = TextInput::with_text(&vals.author);
		let save = Button::new("Save").primary();
		let cancel = Button::new("Cancel").secondary();
		let ids = MetadataIds {
			save: save.id(),
			cancel: cancel.id(),
			name: name.id(),
			players: players.id(),
			description: description.id(),
			date: date.id(),
			version: version.id(),
			author: author.id(),
		};
		let content = column()
			.push(width_strut(340.0))
			.push(field_row("Name", name))
			.push(field_row("Players", players))
			.push(Label::new("Description").small())
			.child(description, Length::Fixed(96.0))
			.push(field_row("Date", date))
			.push(field_row("Version", version))
			.push(field_row("Author", author))
			.push(buttons(cancel, save));
		let win = dialog("Map Metadata", content);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.ui.focus_first();
		self.dialog = Dialog::Metadata(ids);
		self.meta_save_after = save_after;
		self.events.clear();
		self.visible = true;
	}

	/// Opens Editor Preferences: the M.A.X. and M.A.X. Port folder paths (each an
	/// editable field with a Browse picker) and a "Don't ask again" toggle. Save
	/// emits [`Outcome::ApplyPreferences`].
	pub fn open_preferences(
		&mut self,
		max_path: &str,
		max_port_path: &str,
		max_port_data_path: &str,
		skip: bool,
		required: bool,
	) {
		let max_field = TextInput::with_text(max_path);
		let max_browse = Button::new("Browse...").secondary();
		let port_field = TextInput::with_text(max_port_path);
		let port_browse = Button::new("Browse...").secondary();
		let data_field = TextInput::with_text(max_port_data_path);
		let data_browse = Button::new("Browse...").secondary();
		let skip_cb = Checkbox::new("Don't ask again").with_checked(skip);
		let save = Button::new("Save").primary();
		let cancel = Button::new("Cancel").secondary();
		let ids = PrefsIds {
			save: save.id(),
			cancel: cancel.id(),
			max_path: max_field.id(),
			max_path_browse: max_browse.id(),
			max_port_path: port_field.id(),
			max_port_browse: port_browse.id(),
			max_port_data: data_field.id(),
			max_port_data_browse: data_browse.id(),
			skip: skip_cb.id(),
		};
		// A label + editable path field + Browse button on one line.
		let path_row = |label: &str, field: TextInput, browse: Button| {
			Linear::row()
				.spacing(8.0)
				.cross_align(CrossAlign::Center)
				.child(Label::new(label).small(), Length::Fixed(128.0))
				.child(field, Length::Flex(1.0))
				.child(browse, Length::Fixed(88.0))
		};
		let content = column()
			.push(width_strut(480.0))
			.push(Label::new("Point the editor at your M.A.X. game folders so it can load unit").small())
			.push(Label::new("sprites, resource markers, and saves made on installed maps.").small())
			.push(path_row("M.A.X. folder", max_field, max_browse))
			.push(Label::new("Holds MAX.RES and the installed .WRL maps.").small().muted())
			.push(path_row("M.A.X. Port folder", port_field, port_browse))
			.push(Label::new("Holds your saved games (.DTA).").small().muted())
			.push(path_row("M.A.X. Port data", data_field, data_browse))
			.push(
				Label::new("Holds PATCHES.RES (the install/assets folder) - enables stock unit stats.").small().muted(),
			)
			.push(skip_cb)
			.push(buttons(cancel, save));
		let win = dialog("Editor Preferences", content);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.ui.focus_first();
		self.dialog = Dialog::Preferences(ids);
		self.prefs_required = required;
		self.events.clear();
		self.visible = true;
	}

	/// Radio-group behaviour over one group of ids: if one fired during the
	/// last dispatch, select it and clear the rest, returning its index. An
	/// absent member is `WidgetId::NONE` (never fires, never found).
	fn radio_group(&mut self, ids: &[WidgetId]) -> Option<usize> {
		let sel = ids.iter().position(|id| self.ui.fired(*id))?;
		self.radio_select(ids, Some(sel));
		Some(sel)
	}

	/// Sets exactly `sel` selected in a radio group (`None` clears them all).
	fn radio_select(&mut self, ids: &[WidgetId], sel: Option<usize>) {
		for (k, id) in ids.iter().enumerate() {
			if let Some(rb) = self.ui.get_mut::<Radio>(*id) {
				rb.set_selected(Some(k) == sel);
			}
		}
	}

	/// Buffers router-translated events for the dialog (only call while visible);
	/// [`render`](Self::render) drains them. Translation and the OS-clipboard read
	/// live in [`UiRouter`](crate::ui_router::UiRouter) — the one place either
	/// happens.
	pub fn dispatch_events(&mut self, events: &[Event]) {
		self.events.extend_from_slice(events);
	}

	/// Dispatches buffered input, composites the dialog over the rendered editor
	/// frame, and returns what the host should do (apply Map Metadata, create a
	/// map, etc).
	pub fn render(
		&mut self,
		encoder: &mut wgpu::CommandEncoder,
		view: &wgpu::TextureView,
		size: (u32, u32),
		chrome: &mut MenuChrome,
	) -> Outcome {
		let events = std::mem::take(&mut self.events);
		if events.iter().any(|e| matches!(e, Event::Key { key: Key::Escape, pressed: true, .. })) {
			// Fix Shore: Esc aborts a live run (the window stays open, idle);
			// idle Esc closes it like any dialog, telling the shell to clear
			// the run state.
			if let Dialog::AutoFix(_) = self.dialog {
				if self.af_running {
					return Outcome::FixAbort;
				}
				self.hide();
				return Outcome::FixClose;
			}
			// Convert Palette: Esc aborts a live rasterize (back to options);
			// idle Esc closes, dropping any parked run state.
			if let Dialog::ConvertPalette(_) = self.dialog {
				if self.cp.running {
					return Outcome::PaletteConvertAbort;
				}
				self.hide();
				return Outcome::PaletteConvertCancel;
			}
			// New from Image: same shape.
			if let Dialog::NewImage(_) = self.dialog {
				if self.ni.running {
					return Outcome::NewImageAbort;
				}
				self.hide();
				return Outcome::NewImageCancel;
			}
			// Generate: Esc aborts a live run (the window stays open); idle Esc
			// closes, handing the session memory back to the shell.
			if let Dialog::Generate(_) = self.dialog {
				if self.generate.running {
					return Outcome::GenerateAbort;
				}
				return self.generate_close_outcome();
			}
			// Tile Painter: Esc cancels, dropping the edits (like Cancel).
			if let Dialog::TilePaint(_) = self.dialog {
				self.hide();
				return Outcome::TilePaintClose;
			}
			// New Scenery: Esc cancels, dropping the derived piece (like Cancel).
			if let Dialog::SceneryNew(_) = self.dialog {
				self.hide();
				return Outcome::SceneryNewClose;
			}
			// Match editor: Esc closes, discarding staged edits (like Close).
			if let Dialog::MatchEdit(_) = self.dialog {
				self.hide();
				return Outcome::MatchClose;
			}
			// Edit Save Data: from the Issues list, Esc steps back to the form
			// (the edits stay parked in `sd_form`); on the form it closes,
			// discarding the edits like Cancel.
			if let Dialog::SaveDataIssues(_) = self.dialog {
				self.build_save_data();
				return Outcome::Idle;
			}
			// Import WRL: Esc steps the review back to the picker (discarding
			// the match, cheap to redo); at the picker it closes.
			if let Dialog::ImportWrl(_) = self.dialog {
				if self.wrl_unmapped {
					if let Some(&tex) = self.preview_cache.get(&(0, self.default_water())) {
						self.build_wrl_picker(tex);
					}
					return Outcome::WrlBack;
				}
				self.hide();
				return Outcome::WrlCancel;
			}
			self.hide();
			return Outcome::Idle;
		}
		let resp = self.ui.dispatch(&events);
		self.capturing = resp.capturing;
		// A text field's Ctrl+C/X handed its selection out: write it through.
		if let Some(copied) = self.ui.take_clipboard() {
			crate::clipboard::set(&copied);
		}
		// The Generate hover hint, from the pointer the dispatch above just
		// resolved hover against (a no-op for every other dialog).
		self.sync_generate_hint();

		let mut outcome = Outcome::Idle;
		match self.dialog {
			Dialog::UiTests(close) => {
				if self.ui.fired(close) {
					self.hide();
				}
			}
			Dialog::About(ids) => {
				if self.ui.fired(ids.close) {
					self.hide();
				} else if self.ui.fired(ids.website) {
					outcome = Outcome::OpenUrl(WEBSITE.to_string());
				} else if self.ui.fired(ids.github) {
					outcome = Outcome::OpenUrl(GITHUB.to_string());
				}
			}
			Dialog::Metadata(ids) => {
				if self.ui.fired(ids.cancel) {
					self.hide();
				} else if self.ui.fired(ids.save) {
					outcome =
						Outcome::ApplyMetadata { vals: self.collect_metadata(&ids), save_after: self.meta_save_after };
					self.hide();
				}
			}
			Dialog::Preferences(ids) => {
				// Browse opens a native folder picker and fills the field, keeping the
				// dialog up (rfd blocks the loop, which is fine for a modal).
				if self.ui.fired(ids.cancel) {
					// A "required" open (a missing-path action opened it) escalates to
					// the Attention notice; a plain open just closes.
					if self.prefs_required {
						outcome = Outcome::PrefsCancelledRequired;
					}
					self.hide();
				} else if self.ui.fired(ids.max_path_browse) {
					if let Some(dir) = rfd::FileDialog::new().pick_folder() {
						self.set_text(ids.max_path, &dir.display().to_string());
					}
				} else if self.ui.fired(ids.max_port_browse) {
					if let Some(dir) = rfd::FileDialog::new().pick_folder() {
						self.set_text(ids.max_port_path, &dir.display().to_string());
					}
				} else if self.ui.fired(ids.max_port_data_browse) {
					if let Some(dir) = rfd::FileDialog::new().pick_folder() {
						self.set_text(ids.max_port_data, &dir.display().to_string());
					}
				} else if self.ui.fired(ids.save) {
					outcome = Outcome::ApplyPreferences {
						max_path: self.text(ids.max_path),
						max_port_path: self.text(ids.max_port_path),
						max_port_data_path: self.text(ids.max_port_data),
						skip_prompt: self.ui.get::<Checkbox>(ids.skip).is_some_and(Checkbox::checked),
					};
					self.hide();
				}
			}
			Dialog::NewMap(ids) => outcome = self.dispatch_new_map(ids),
			Dialog::Resize(ids) => outcome = self.dispatch_resize(ids),
			Dialog::Confirm(ids) => outcome = self.dispatch_confirm(ids),
			Dialog::AutoFix(ids) => outcome = self.dispatch_autofix(ids),
			Dialog::ConvertPalette(ids) => outcome = self.dispatch_convert_palette(ids),
			Dialog::NewImage(ids) => outcome = self.dispatch_new_image(ids),
			Dialog::ImportWrl(ids) => outcome = self.dispatch_import_wrl(ids),
			Dialog::Generate(ids) => outcome = self.dispatch_generate(ids),
			Dialog::TilePaint(ids) => outcome = self.dispatch_tile_paint(ids, chrome),
			Dialog::MatchEdit(ids) => outcome = self.dispatch_match_edit(ids),
			Dialog::Error(ok) => {
				if self.ui.fired(ok) {
					self.hide();
				}
			}
			Dialog::SaveData => outcome = self.dispatch_save_data(),
			Dialog::SaveDataIssues(ids) => outcome = self.dispatch_save_data_issues(ids),
			Dialog::PaletteName(ids) => outcome = self.dispatch_palette_name(ids),
			Dialog::RenameTemplate(ids) => outcome = self.dispatch_rename_template(ids),
			Dialog::SceneryNew(ids) => outcome = self.dispatch_scenery_new(ids),
			Dialog::NamePrompt(ids) => outcome = self.dispatch_name_prompt(ids),
			Dialog::ObjectField(ids) => outcome = self.dispatch_object_field(ids),
			Dialog::None => {}
		}
		if !self.visible {
			return outcome;
		}

		let scale = self.scale;
		let logical = Size::new(size.0 as f32 / scale, size.1 as f32 / scale);
		// Scale the shared renderer + theme to the editor's ui_scale (the emboss
		// snaps to whole physical px at this scale for crisp engraving).
		chrome.set_scale(scale);
		// Steel grain scale spans the logical viewport (one continuous sheet).
		chrome.theme_mut().set_viewport(logical.w, logical.h);
		// Layout at the real scale so the window pixel-locks to the device grid
		// (no sub-pixel drift between elements when it is dragged).
		self.ui.set_scale(scale);
		self.ui.layout(logical, chrome.theme(), chrome.fonts());
		// Anchor the grain to the (possibly dragged) window so the background is
		// fixed to the window rather than scrolling under it.
		let origin =
			self.win_id.and_then(|id| self.ui.get::<Window>(id)).map(|w| w.position()).unwrap_or(Vec2::new(0.0, 0.0));
		chrome.theme_mut().set_origin(origin);
		let mut dl = DrawList::new();
		self.ui.draw(&mut dl, chrome.theme(), chrome.fonts());
		chrome.render_list(encoder, view, size, &dl);
		outcome
	}

	fn collect_metadata(&self, ids: &MetadataIds) -> MetadataValues {
		let players = match self.ui.get::<Select>(ids.players).map(Select::selected).unwrap_or(0) {
			1 => Some(2),
			2 => Some(3),
			3 => Some(4),
			_ => None,
		};
		MetadataValues {
			name: self.text(ids.name),
			players,
			description: self.ui.get::<TextArea>(ids.description).map(|t| t.text().to_string()).unwrap_or_default(),
			date: self.text(ids.date),
			version: self.text(ids.version),
			author: self.text(ids.author),
		}
	}

	fn text(&self, id: WidgetId) -> String {
		self.ui.get::<TextInput>(id).map(|t| t.text().to_string()).unwrap_or_default()
	}

	fn set_text(&mut self, id: WidgetId, text: &str) {
		if let Some(t) = self.ui.get_mut::<TextInput>(id) {
			t.set_text(text);
		}
	}

	fn set_label(&mut self, id: WidgetId, text: &str) {
		if let Some(l) = self.ui.get_mut::<Label>(id) {
			l.set_text(text);
		}
	}

	/// The on-screen position to (re)build a dialog window at, or `None` to
	/// centre it. Returns `Some(pos)` only when `same` is set *and* `win_id`
	/// still points at a live window — i.e. this is a rebuild of the dialog
	/// already showing (an option switched, a run started/finished), so the
	/// window keeps its place instead of snapping back to centre. [`Self::hide`]
	/// drops `win_id`, so the next fresh open re-centres.
	fn hold_pos(&self, same: bool) -> Option<Vec2> {
		self.win_id.filter(|_| same).and_then(|id| self.ui.get::<Window>(id)).map(|w| w.position())
	}

	/// A [`dialog`] window that stays put across rebuilds: centred on a fresh
	/// open, pinned to its current spot when `same` marks a rebuild (see
	/// [`Self::hold_pos`]). Auto-sized, so it grows/shrinks from a fixed
	/// top-left instead of re-centring on every content change.
	fn dialog_kept(&self, title: &str, content: Linear, same: bool) -> Window {
		match self.hold_pos(same) {
			Some(pos) => Window::new(title, content).auto_size().resizable(false).pos(pos),
			None => dialog(title, content),
		}
	}
}

/// Parses a map dimension: a whole number in the engine's `1..=1024`, or the
/// reason it is not. New Map and Resize report the reason inline
/// ([`status_slot`]) instead of silently coercing - the callers prefix the
/// field name.
fn parse_dim(s: &str) -> Result<u16, String> {
	let t = s.trim();
	let n = t.parse::<u16>().map_err(|_| format!("'{t}' is not a number in 1..=1024"))?;
	if !(1..=1024).contains(&n) {
		return Err(format!("{n} is out of 1..=1024"));
	}
	Ok(n)
}

/// The Save/Rename palette name rules (the bespoke `PaletteName::try_confirm`):
/// reject empty / slashed / unchanged names; a first clash with an `existing`
/// name arms overwrite (alert + `Err`, recorded in `armed`) and the same name
/// confirmed again resolves. `from` is the rename source `(name, file)` (`None`
/// for Save) and decides the command (`palette-save-as` / `palette-rename`).
/// Validate a template rename `to` against its source `from` and the sibling
/// `existing` names: on success the `template-rename "from" "to"` command line,
/// else the inline alert to show. Mirrors the legacy modal's `validate`.
fn resolve_template_rename(to: &str, from: &str, existing: &[String]) -> Result<String, String> {
	let to = to.trim();
	if to.is_empty() {
		return Err("the name is empty".into());
	}
	if to == from {
		return Err("the name is unchanged".into());
	}
	if existing.iter().any(|n| n == to) {
		return Err(format!("a template named \"{to}\" already exists"));
	}
	Ok(format!("template-rename \"{from}\" \"{to}\""))
}

fn resolve_palette_name(
	name: &str,
	from: &Option<(String, PathBuf)>,
	existing: &[String],
	armed: &mut Option<String>,
) -> Result<String, String> {
	let name = name.trim().to_string();
	if name.is_empty() {
		return Err("the name is empty".into());
	}
	if name.contains(['/', '\\']) {
		return Err("no slashes in the name".into());
	}
	if from.as_ref().map(|(n, _)| n.as_str()) == Some(name.as_str()) {
		return Err("the name is unchanged".into());
	}
	if existing.iter().any(|n| n == &name) && armed.as_deref() != Some(name.as_str()) {
		*armed = Some(name.clone());
		return Err(format!("\"{name}\" exists - confirm again to overwrite"));
	}
	Ok(match from {
		None => format!("palette-save-as \"{name}\""),
		Some((_, path)) => format!("palette-rename \"{}\" \"{name}\"", path.display()),
	})
}

/// A stretch column with the dialog's standard spacing.
fn column() -> Linear {
	Linear::column().spacing(6.0).cross_align(CrossAlign::Stretch)
}

/// A zero-height transparent strut that fixes a column's width (the rows then
/// stretch to match), so text fields have room.
fn width_strut(w: f32) -> Fill {
	Fill::new(Rgba::rgba(0, 0, 0, 0), Size::new(w, 0.0))
}

/// Pushes `label` as a *changing* status line (hover hint, validation error,
/// live note): wrapped at `w` (the dialog's strut width) inside a reserved
/// `lines`-line slot, so text changes never resize the auto-sized dialog —
/// a plain label would re-measure wider or taller and wobble the window.
/// The slot's height is the label's own (`Label::fixed_lines`, measured off
/// the font at its role) — the last app-side line-height constant retired
/// with U7.1's rule: nothing app-side has an opinion about how tall text is.
fn status_slot(col: Linear, label: Label, w: f32, lines: usize) -> Linear {
	col.child(label.wrap_at(w).fixed_lines(lines), Length::Fit)
}

/// A `label : control` row (fixed label column, flexible control).
fn field_row(label: &str, control: impl Widget + 'static) -> Linear {
	Linear::row()
		.spacing(8.0)
		.cross_align(CrossAlign::Center)
		.child(Label::new(label).small(), Length::Fixed(78.0))
		.child(control, Length::Flex(1.0))
}

/// A right-aligned `secondary  primary` button row.
fn buttons(secondary: Button, primary: Button) -> Linear {
	Linear::row().spacing(8.0).main_align(MainAlign::End).push(secondary).push(primary)
}

/// A right-aligned numeric entry seeded with `text` (Edit Save Data fields).
fn digits(text: &str, max_len: usize) -> TextInput {
	TextInput::with_text(text).charset(Charset::Digits).max_len(max_len).align(wgpu_ui::TextAlign::Right)
}

/// A dimming scrim behind a centred, movable dialog window.
fn modal(window: Window) -> impl Widget + 'static {
	Stack::new().push(Fill::new(Rgba::rgba(0, 0, 0, 150), Size::ZERO)).push(window)
}

/// Wraps dialog content in a movable, auto-sized, centred window titled `title`.
fn dialog(title: &str, content: Linear) -> Window {
	Window::new(title, content).auto_size().centered().resizable(false)
}

#[cfg(test)]
mod logic_tests {
	use super::*;

	// Parity with the bespoke `PaletteName` rules (palettename.rs tests).
	#[test]
	fn palette_name_save_resolves_and_arms_overwrite() {
		let existing = vec!["forest".to_string()];
		let mut armed = None;
		assert!(resolve_palette_name("", &None, &existing, &mut armed).is_err(), "empty refused");
		assert_eq!(resolve_palette_name("swamp", &None, &existing, &mut armed).unwrap(), "palette-save-as \"swamp\"");
		// A clash arms overwrite first, then commits on the next confirm.
		assert!(resolve_palette_name("forest", &None, &existing, &mut armed).is_err(), "first confirm arms");
		assert_eq!(armed.as_deref(), Some("forest"));
		assert_eq!(resolve_palette_name("forest", &None, &existing, &mut armed).unwrap(), "palette-save-as \"forest\"");
	}

	#[test]
	fn palette_name_rename_refuses_unchanged_and_quotes_path() {
		let from = Some(("old".to_string(), PathBuf::from("/u/old.json")));
		let mut armed = None;
		assert!(resolve_palette_name("old", &from, &[], &mut armed).unwrap_err().contains("unchanged"));
		assert!(resolve_palette_name("a/b", &from, &[], &mut armed).is_err(), "slashes refused");
		assert_eq!(
			resolve_palette_name("new", &from, &[], &mut armed).unwrap(),
			"palette-rename \"/u/old.json\" \"new\""
		);
	}
}

#[cfg(test)]
mod render_tests {
	use super::*;
	use std::path::Path;
	use std::sync::mpsc::channel;

	/// Render the currently-open dialog offscreen at `w`×`h` and read the frame
	/// back as RGBA8. The overlay is invisible to the `--screenshot` path (which
	/// composites `render_frame` only), so this is the dialogs' sole visual cover.
	fn frame(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		overlay: &mut Overlay,
		chrome: &mut MenuChrome,
		w: u32,
		h: u32,
	) -> Vec<u8> {
		let texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("dialog.target"),
			size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: crate::capture::FORMAT,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
			view_formats: &[],
		});
		let view = texture.create_view(&Default::default());
		let mut encoder = device.create_command_encoder(&Default::default());
		// A dark backdrop so the scrim + steel read (mimics the map behind).
		{
			let _rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("dialog.clear"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: &view,
					resolve_target: None,
					depth_slice: None,
					ops: wgpu::Operations {
						load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.10, g: 0.10, b: 0.10, a: 1.0 }),
						store: wgpu::StoreOp::Store,
					},
				})],
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
		}
		overlay.render(&mut encoder, &view, (w, h), chrome);

		let unpadded = w * 4;
		let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
		let padded = unpadded.div_ceil(align) * align;
		let buffer = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("dialog.readback"),
			size: (padded * h) as u64,
			usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
			mapped_at_creation: false,
		});
		encoder.copy_texture_to_buffer(
			texture.as_image_copy(),
			wgpu::TexelCopyBufferInfo {
				buffer: &buffer,
				layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) },
			},
			wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
		);
		queue.submit([encoder.finish()]);

		let slice = buffer.slice(..);
		let (tx, rx) = channel();
		slice.map_async(wgpu::MapMode::Read, move |r| {
			let _ = tx.send(r);
		});
		device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).expect("poll");
		rx.recv().expect("map channel").expect("map");
		let data = slice.get_mapped_range().expect("buffer mapped for read");
		let mut rgba = Vec::with_capacity((unpadded * h) as usize);
		for row in 0..h as usize {
			let off = row * padded as usize;
			rgba.extend_from_slice(&data[off..off + unpadded as usize]);
		}
		drop(data);
		buffer.unmap();
		rgba
	}

	/// Count pixels whose (r, g, b) satisfy `f` (alpha ignored).
	fn count(rgba: &[u8], f: impl Fn(i32, i32, i32) -> bool) -> usize {
		rgba.chunks_exact(4).filter(|p| f(p[0] as i32, p[1] as i32, p[2] as i32)).count()
	}

	#[test]
	fn dialogs_render_with_content() {
		let (device, queue, _serial) = crate::visual_test::test_gpu();
		let res = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources");
		let steel = crate::skin::load_steel(&res);
		// The overlay shares the menu chrome's renderer/steel theme/fonts.
		let mut chrome = MenuChrome::new(&device, &queue, crate::capture::FORMAT, &steel).expect("chrome");
		let mut overlay = Overlay::new(1.0);
		let (w, h) = (620u32, 800u32);

		// The steel dialog body is a large mid-grey region (low channel spread);
		// the dark backdrop/scrim is below it. Its presence proves the dialog
		// painted - a blank or panicking dialog leaves only the flat clear.
		let body =
			|buf: &[u8]| count(buf, |r, g, b| (r - g).abs() < 24 && (g - b).abs() < 24 && (60..210).contains(&r));

		overlay.open_about();
		let about = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&about) > 3000, "About body did not render ({})", body(&about));

		// The description overflows the 96px area and carries a word wider
		// than the field - the dump shows the wrap + scrollbar.
		overlay.open_metadata(
			MetadataValues {
				name: "New Luzon".into(),
				players: Some(4),
				description: "A test map set among the islands of a drowned range. \
					Landings are contested from the first turn; hold the straits and the \
					supersupersupercalifragilisticexpialidocious-long-token wraps mid-word. \
					Expect naval skirmishes, tight beachheads, and late-game air superiority \
					fights over the central atoll."
					.into(),
				date: "1996".into(),
				version: "1.0".into(),
				author: "MAX".into(),
			},
			false,
		);
		let metadata = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&metadata) > 3000, "Metadata body did not render ({})", body(&metadata));

		overlay.open_resize(112, 112);
		let resize = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&resize) > 3000, "Resize body did not render ({})", body(&resize));

		// Edit Save Data: all three tab pages render (the tab switch rebuilds
		// from the canonical form), and the Issues list renders its rows.
		overlay.open_save_data(crate::savedata::tests::init());
		let setup = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&setup) > 3000, "Save Data / Game Setup did not render ({})", body(&setup));
		for tab in [1usize, 2] {
			overlay.sd.tab = tab;
			overlay.build_save_data();
			let page = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
			assert!(body(&page) > 3000, "Save Data tab {tab} did not render ({})", body(&page));
		}
		overlay.sd.issues = vec![savedata::Issue {
			field: "Game Setup / Start gold".into(),
			message: "is 700000 - enter 0 to 9999".into(),
			target: savedata::Target::StartGold,
			fixed: "9999".into(),
		}];
		overlay.build_save_data_issues();
		let issues = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&issues) > 3000, "Save Data issues list did not render ({})", body(&issues));
		overlay.hide();

		overlay.open_confirm(
			"Delete Palette",
			"Delete \"swamp\"?",
			"This cannot be undone.",
			"Delete",
			"palette-delete \"/u/swamp.json\"".into(),
		);
		let confirm = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&confirm) > 3000, "Confirm body did not render ({})", body(&confirm));

		overlay.open_confirm_save(
			"Unsaved Changes",
			"\"scratch\" has unsaved changes.",
			"Save",
			"save-and-close".into(),
			"Discard",
			"close-project!".into(),
		);
		let guard = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&guard) > 3000, "Save/Discard/Cancel guard did not render ({})", body(&guard));

		// Delete Template: a synthetic 2×1-cell magenta preview stands in for the
		// composed thumbnail; the dialog must show it (strong magenta pixels the
		// steel/green UI never makes) plus its confirm body.
		let (tw, th) = (128u32, 64u32);
		let preview = vec![[255u8, 0, 255, 255]; (tw * th) as usize].concat();
		overlay.open_delete_template(&mut chrome, "Ridge", (2, 1), &preview, tw, th);
		let del = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&del) > 3000, "Delete Template body did not render ({})", body(&del));
		let magenta = count(&del, |r, g, b| r > 180 && g < 80 && b > 180);
		assert!(magenta > 400, "template preview image missing (only {magenta} magenta px)");

		// Rename Template: same synthetic preview + a name field.
		overlay.open_rename_template(&mut chrome, "Ridge", (2, 1), vec!["Bluff".into()], &preview, tw, th);
		let ren = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&ren) > 3000, "Rename Template body did not render ({})", body(&ren));
		let magenta = count(&ren, |r, g, b| r > 180 && g < 80 && b > 180);
		assert!(magenta > 400, "rename preview image missing (only {magenta} magenta px)");

		// Fix Shore: the non-blocking float renders its stat rows, and a
		// running sync (flipping Start->Stop etc.) still renders.
		overlay.open_autofix(42);
		assert!(!overlay.blocking(), "Fix Shore floats (no modal block)");
		let fix = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&fix) > 3000, "Fix Shore body did not render ({})", body(&fix));
		overlay.sync_autofix(true, 42, 7, 35, "1.2s", None);
		let fix2 = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&fix2) > 3000, "running Fix Shore did not render ({})", body(&fix2));

		// Convert Palette: the options stage renders, and a running sync flips
		// to the stage/progress view (which must also render).
		overlay.open_convert_palette();
		let cp = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&cp) > 3000, "Convert Palette options did not render ({})", body(&cp));
		overlay.sync_convert_palette(true, 0.4, "Re-importing", "40%   elapsed 0.8s");
		let cp2 = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&cp2) > 3000, "running Convert Palette did not render ({})", body(&cp2));

		// New from Image: settings stage + a running sync.
		overlay.open_new_image(64, 48);
		let ni = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&ni) > 3000, "New from Image settings did not render ({})", body(&ni));
		overlay.sync_new_image(true, 0.3, "Quantizing", "30%   elapsed 0.5s");
		let ni2 = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&ni2) > 3000, "running New from Image did not render ({})", body(&ni2));

		// Generate: the (non-blocking) knob form for every generator renders,
		// and the running sync (progress bar + frozen form) still renders.
		overlay.open_generate(&crate::genform::GenMemory::default(), 64, 64);
		assert!(!overlay.blocking(), "Generate floats (no modal block)");
		let g1 = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&g1) > 3000, "Generate form did not render ({})", body(&g1));
		overlay.sync_generate(true, Some(("carving rivers", 0.4)), &[], None);
		let g2 = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&g2) > 3000, "running Generate did not render ({})", body(&g2));
		let report = ["seed 42".to_string(), "12 features".to_string()];
		overlay.sync_generate(false, None, &report, Some(42));
		let g3 = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&g3) > 3000, "Generate report stage did not render ({})", body(&g3));

		overlay.open_palette_name("Save Palette", "forest", None, vec!["swamp".into()]);
		let palname = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&palname) > 3000, "Palette name body did not render ({})", body(&palname));

		let assets = res.join("assets/tilepacks");
		let packs = crate::packlist::scan(&assets);
		let (palettes, tilesets) = crate::newmap::palette_choices(&packs, &assets, Path::new("/nonexistent"));
		overlay.open_newmap(&mut chrome, packs, &assets, palettes.clone(), tilesets, true, None);
		let newmap = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&newmap) > 3000, "New Map body did not render ({})", body(&newmap));
		// The per-pack tile previews introduce strong blue (water) and warm
		// (crater / desert) pixels the steel + green UI never produces, so their
		// presence proves the preview strips rendered - the feature this guards.
		let tiles = count(&newmap, |r, _g, b| (b > r + 30 && b > 90) || (r > b + 50 && r > 110));
		assert!(tiles > 150, "New Map tile previews missing (only {tiles} tile-coloured px)");

		// Create refuses a bad W/H inline - the reason lands in the status
		// slot instead of silently coercing to 112 or clamping (audit item 5) -
		// and a corrected pair collects normally.
		{
			let Dialog::NewMap(ids) = overlay.dialog else { unreachable!("New Map is open") };
			overlay.set_text(ids.width, "2000");
			assert!(matches!(overlay.collect_newmap(&ids), Err(e) if e == "width 2000 is out of 1..=1024"));
			overlay.set_text(ids.width, "256");
			overlay.set_text(ids.height, "");
			assert!(matches!(overlay.collect_newmap(&ids), Err(e) if e == "height '' is not a number in 1..=1024"));
			overlay.set_text(ids.height, "64");
			match overlay.collect_newmap(&ids) {
				Ok(v) => assert_eq!((v.width, v.height), (256, 64), "in-range fields collect"),
				Err(e) => panic!("in-range fields refused: {e}"),
			}
			overlay.set_text(ids.width, "112");
			overlay.set_text(ids.height, "112");
		}

		// Interaction reaches through the list well + scroll area + item well:
		// a click through the real event path toggles a land checkbox (the
		// chrome wrappers must forward events, or the lists go dead).
		{
			let land = overlay.packs.iter().position(|p| !p.water).expect("a land pack");
			let id = overlay.pack_ids[land];
			let was = overlay.ui.get::<Checkbox>(id).expect("land checkbox").checked();
			let r = overlay.ui.rect_of(id).expect("checkbox laid out");
			let c = Vec2::new(r.x + 8.0, r.y + r.h / 2.0);
			let mods = wgpu_ui::Modifiers::default();
			for pressed in [true, false] {
				overlay.events.push(Event::PointerButton {
					button: wgpu_ui::PointerButton::Primary,
					pressed,
					pos: c,
					mods,
				});
			}
			let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
			assert_eq!(
				overlay.ui.get::<Checkbox>(id).expect("land checkbox").checked(),
				!was,
				"a click inside the pick list reaches its checkbox"
			);
		}

		// The palette selector drives the previews: an uncached (palette,
		// water) combination asks the host for an atlas; providing it swaps
		// the strips and caches, so the same choice later applies directly.
		{
			let pal_id = match overlay.dialog {
				Dialog::NewMap(ids) => ids.palette,
				_ => unreachable!("New Map is open"),
			};
			if let Some(sel) = overlay.ui.get_mut::<Select>(pal_id) {
				sel.set_selected(1);
			}
			let out = overlay.request_previews();
			let (key, water) = match out {
				Outcome::NewMapPreview { key, water, palette } => {
					assert_eq!(key.0, 1, "choice 1 = the first tileset palette");
					assert!(palette.as_ref().is_some_and(|p| p.ends_with("palette.json")), "carries its file");
					(key, water)
				}
				_ => panic!("an uncached combination requests a host build"),
			};
			let (rgba, _rows) = crate::newmap::build_rgba(overlay.pack_entries(), &assets, None, &water);
			let t2 = chrome.register_texture(
				&rgba,
				(PREVIEW_TILES * 64) as u32,
				(overlay.pack_entries().len().max(1) as u32) * 64,
			);
			overlay.provide_preview_tex(key, t2);
			assert!(
				matches!(overlay.request_previews(), Outcome::Idle),
				"a provided combination is cached and applies without the host"
			);
		}

		// File → New Terrain from Image opens the form with the shape armed for
		// the carve-on-Create path; the dialog notes it and still renders.
		assert!(overlay.newmap_shape.is_none(), "no shape on a plain New Map");
		overlay.hide();
		let packs = crate::packlist::scan(&assets);
		overlay.open_newmap(
			&mut chrome,
			packs,
			&assets,
			palettes,
			tilesets,
			true,
			Some(PathBuf::from("/maps/atoll.png")),
		);
		assert_eq!(overlay.newmap_shape.as_deref(), Some(Path::new("/maps/atoll.png")));
		let shaped = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&shaped) > 3000, "New Map still renders with a shape set");

		// Import WRL: the pack picker renders (same rows), and switching to the
		// unmapped review (scrolling list + destination radios) still renders.
		overlay.open_import_wrl(&mut chrome, crate::packlist::scan(&assets), &assets, "SNOW1", (112, 112, 400));
		let iw = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&iw) > 3000, "Import WRL picker did not render ({})", body(&iw));
		let rows: Vec<String> = (0..12).map(|i| format!("{i:03}   land   {i} cells")).collect();
		overlay.show_wrl_unmapped(380, 400, &rows);
		let iw2 = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(body(&iw2) > 3000, "Import WRL review did not render ({})", body(&iw2));

		// Tile Painter: the composed canvas + swatch textures render (a magenta
		// palette slot filling the canvas makes pixels the steel UI never
		// produces), and a synthetic press through the real event path paints
		// the working canvas (the edit then mirrors out to the shell).
		let mut pal: Vec<u8> = (0..256u16).flat_map(|i| [i as u8, i as u8, i as u8, 255]).collect();
		pal[5 * 4..5 * 4 + 4].copy_from_slice(&[255, 0, 255, 255]); // slot 5 = magenta
		let run = crate::tilepaint::TilePaintRun {
			mode: crate::tilepaint::Mode::Edit,
			tile_id: "GLa000".into(),
			pack_name: "GREEN".into(),
			mask: None,
			canvas: vec![5u8; crate::tilepaint::TILE * crate::tilepaint::TILE],
			canvas_rev: 0,
			pass: 1,
			id_text: "GLa000".into(),
			packs: Vec::new(),
		};
		overlay.open_tile_paint(&mut chrome, &run, &pal, false, None);
		assert!(overlay.blocking(), "the painter is a blocking modal");
		let (tw2, th2) = (900u32, 700u32);
		let tp = frame(&device, &queue, &mut overlay, &mut chrome, tw2, th2);
		assert!(body(&tp) > 3000, "Tile Painter body did not render ({})", body(&tp));
		let magenta = count(&tp, |r, g, b| r > 180 && g < 80 && b > 180);
		assert!(magenta > 20_000, "the composed canvas is missing (only {magenta} magenta px)");
		// Press the canvas center: paints palette index 1 (the default color)
		// at pixel (32, 32) and reports the edit for the shell's mirror.
		let canvas_id = match overlay.dialog {
			Dialog::TilePaint(ids) => ids.canvas,
			_ => unreachable!("the painter is open"),
		};
		let r = overlay.ui.rect_of(canvas_id).expect("canvas laid out");
		let center = Vec2::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
		let mods = wgpu_ui::Modifiers::default();
		overlay.events.push(Event::PointerButton {
			button: wgpu_ui::PointerButton::Primary,
			pressed: true,
			pos: center,
			mods,
		});
		overlay.events.push(Event::PointerButton {
			button: wgpu_ui::PointerButton::Primary,
			pressed: false,
			pos: center,
			mods,
		});
		let tp2 = frame(&device, &queue, &mut overlay, &mut chrome, tw2, th2);
		assert!(body(&tp2) > 3000, "painted Tile Painter did not render ({})", body(&tp2));
		let edited = overlay.tile_canvas_if_edited().expect("the press edited the canvas").to_vec();
		let mid = 32 * crate::tilepaint::TILE + 32;
		assert_eq!(edited[mid], 1, "the canvas center now holds the paint color");
		assert!(overlay.tile_canvas_if_edited().is_none(), "the edit is taken once");
		overlay.hide();

		// Match editor: the model over a real GREEN project, the shared tile
		// atlas for list thumbnails, and a magenta strip standing in for the
		// composed cross/orientation art (pixels the steel UI never makes).
		let project = map_core::Project::new(16, 16, &["GREEN".to_string()], &assets, 7).expect("GREEN project");
		let me = crate::matcheditor::MatchEditor::new(&project, None).expect("match rules");
		let was_dirty = me.dirty();
		assert!(!was_dirty, "a fresh model starts clean");
		let lut = crate::tile_atlas::rest_lut(&project.palette);
		let (argba, aw, ah, acount) = crate::tile_atlas::compose(&project, &lut);
		let atlas_tex = chrome.register_texture(&argba, aw, ah);
		let strip = vec![[255u8, 0, 255, 255]; 9 * 64 * 64].concat();
		overlay.open_match_edit(&mut chrome, me, &strip, (atlas_tex, acount, 0));
		assert!(overlay.blocking(), "the match editor is a blocking modal");
		let med = frame(&device, &queue, &mut overlay, &mut chrome, tw2, th2);
		assert!(body(&med) > 3000, "match editor body did not render ({})", body(&med));
		let magenta = count(&med, |r, g, b| r > 180 && g < 80 && b > 180);
		assert!(magenta > 3000, "cross/orientation strip art missing (only {magenta} magenta px)");
		// Select a known main/cand pair (the model tests' GSa/GSh), then click
		// the cross's south side through the real event path: it toggles the
		// match between the two groups → the model dirties.
		{
			let me = overlay.me.as_mut().expect("model open");
			let find = |me: &crate::matcheditor::MatchEditor, id: &str| {
				(0..me.pd().tile_count).find(|&t| me.pd().effective_id(t) == id).expect("tile id present")
			};
			let (gsa, gsh) = (find(me, "GSa000"), find(me, "GSh000"));
			me.pd_mut().main_tile = gsa;
			me.pd_mut().cand_tile = gsh;
			assert!(me.pd().wildcard(2).is_none(), "GSa's south side is toggleable");
		}
		let cross_id = match overlay.dialog {
			Dialog::MatchEdit(ids) => ids.cross,
			_ => unreachable!("the match editor is open"),
		};
		let r = overlay.ui.rect_of(cross_id).expect("cross laid out");
		let cell = r.w / 3.0;
		let south = Vec2::new(r.x + 1.5 * cell, r.y + 2.5 * cell);
		for pressed in [true, false] {
			overlay.events.push(Event::PointerButton {
				button: wgpu_ui::PointerButton::Primary,
				pressed,
				pos: south,
				mods,
			});
		}
		let med2 = frame(&device, &queue, &mut overlay, &mut chrome, tw2, th2);
		assert!(body(&med2) > 3000, "match editor did not re-render ({})", body(&med2));
		assert!(
			overlay.me.as_ref().is_some_and(crate::matcheditor::MatchEditor::dirty),
			"the cross click staged a match edit"
		);
		overlay.hide();

		// Drop PNGs for human inspection (best-effort; the asserts are the gate).
		let dir = std::env::temp_dir().join("max-map-editor-dialogs");
		let _ = std::fs::create_dir_all(&dir);
		for (name, buf) in [
			("about", &about),
			("metadata", &metadata),
			("resize", &resize),
			("confirm", &confirm),
			("palname", &palname),
			("newmap", &newmap),
			("newmap-shaped", &shaped),
		] {
			write_png(&dir.join(format!("{name}.png")), buf, w, h);
		}
		write_png(&dir.join("tilepaint.png"), &tp2, tw2, th2);
		write_png(&dir.join("matchedit.png"), &med2, tw2, th2);
	}

	/// Changing status text (hover hints, validation errors, live notes) must
	/// not resize an auto-sized dialog: the changing labels wrap at the
	/// dialog's strut width inside reserved fixed-height slots
	/// ([`status_slot`]), so the measured window size stays put while the
	/// text flips between empty and its longest form.
	#[test]
	fn changing_status_text_does_not_resize_dialogs() {
		let (device, queue, _serial) = crate::visual_test::test_gpu();
		let res = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources");
		let steel = crate::skin::load_steel(&res);
		let mut chrome = MenuChrome::new(&device, &queue, crate::capture::FORMAT, &steel).expect("chrome");
		let mut overlay = Overlay::new(1.0);
		let (w, h) = (620u32, 800u32);
		let win = |o: &Overlay| o.ui.rect_of(o.win_id.expect("dialog window")).expect("laid out");

		// Resize: the live offset note flips empty -> a full line.
		overlay.open_resize(112, 112);
		let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		let before = win(&overlay);
		let Dialog::Resize(ids) = overlay.dialog else { unreachable!() };
		overlay.set_label(ids.note, "tile 104x104 inside new bounds, offset -104, -104");
		let shot = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert_eq!(win(&overlay).size(), before.size(), "Resize resized with its note");

		// Generate: the longest hover hint plus a validation error at once —
		// the hint wraps to multiple lines inside its reserved slot.
		overlay.open_generate(&crate::genform::GenMemory::default(), 112, 112);
		let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		let before = win(&overlay);
		let Dialog::Generate(ids) = overlay.dialog else { unreachable!() };
		overlay.set_label(ids.hint, crate::genform::ACCESS_HINT);
		overlay.set_label(ids.error, "accessibility is not a number");
		let gshot = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert_eq!(win(&overlay).size(), before.size(), "Generate resized with hint/error");

		// Drop PNGs for human inspection (best-effort; the asserts are the gate).
		let dir = std::env::temp_dir().join("max-map-editor-dialogs");
		let _ = std::fs::create_dir_all(&dir);
		write_png(&dir.join("resize-note.png"), &shot, w, h);
		write_png(&dir.join("generate-hint.png"), &gshot, w, h);
	}

	/// A Generate rebuild (a run starting, a generator switch, a report landing)
	/// keeps the window where it sits instead of snapping back to centre — the
	/// guard behind `dialog_kept` / `hold_pos`. Without it, every content change
	/// re-centres the auto-sized window, so it jumps as the form grows/shrinks.
	#[test]
	fn generate_dialog_keeps_its_place_across_rebuilds() {
		let (device, queue, _serial) = crate::visual_test::test_gpu();
		let res = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources");
		let steel = crate::skin::load_steel(&res);
		let mut chrome = MenuChrome::new(&device, &queue, crate::capture::FORMAT, &steel).expect("chrome");
		let mut overlay = Overlay::new(1.0);
		let (w, h) = (620u32, 800u32);
		let win = |o: &Overlay| o.ui.rect_of(o.win_id.expect("dialog window")).expect("laid out");

		// Open + render once so the fresh form centres.
		overlay.open_generate(&crate::genform::GenMemory::default(), 64, 64);
		let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		let idle = win(&overlay);

		// Start a run: the form is rebuilt at a different height (a progress bar
		// replaces the three status lines). It must hold its top-left.
		overlay.sync_generate(true, Some(("carving rivers", 0.4)), &[], None);
		let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		let running = win(&overlay);
		assert_ne!(idle.size().h, running.size().h, "the running form must change height (else the guard is vacuous)");
		assert_eq!((running.x, running.y), (idle.x, idle.y), "a rebuild re-centred the window instead of holding it");

		// Finishing the run rebuilds again (Copy Seed appears) — still no jump.
		overlay.sync_generate(false, None, &["seed 42".into()], Some(42));
		let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		let done = win(&overlay);
		assert_eq!((done.x, done.y), (idle.x, idle.y), "the report stage re-centred instead of holding");
	}

	/// [`frame`] without the pixel readback, returning the dialog's outcome —
	/// for asserting what a clicked button reported to the shell.
	fn frame_outcome(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		overlay: &mut Overlay,
		chrome: &mut MenuChrome,
		w: u32,
		h: u32,
	) -> Outcome {
		let texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("dialog.target"),
			size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: crate::capture::FORMAT,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			view_formats: &[],
		});
		let view = texture.create_view(&Default::default());
		let mut encoder = device.create_command_encoder(&Default::default());
		let outcome = overlay.render(&mut encoder, &view, (w, h), chrome);
		queue.submit([encoder.finish()]);
		outcome
	}

	/// A stock dialog button must fire on a synthetic press + release through
	/// the real event path (regression guard: "buttons in modals don't work").
	#[test]
	fn dialog_button_fires_on_click() {
		let (device, queue, _serial) = crate::visual_test::test_gpu();
		let res = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources");
		let steel = crate::skin::load_steel(&res);
		let mut chrome = MenuChrome::new(&device, &queue, crate::capture::FORMAT, &steel).expect("chrome");
		let mut overlay = Overlay::new(1.0);
		let (w, h) = (620u32, 800u32);

		overlay.open_about();
		assert!(overlay.visible());
		// One frame so the tree is laid out and the button rect is valid.
		let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		let close = match overlay.dialog {
			Dialog::About(ids) => ids.close,
			_ => unreachable!("About is open"),
		};
		let r = overlay.ui.rect_of(close).expect("Close laid out");
		let center = Vec2::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
		let mods = wgpu_ui::Modifiers::default();
		overlay.events.push(Event::PointerButton {
			button: wgpu_ui::PointerButton::Primary,
			pressed: true,
			pos: center,
			mods,
		});
		overlay.events.push(Event::PointerButton {
			button: wgpu_ui::PointerButton::Primary,
			pressed: false,
			pos: center,
			mods,
		});
		let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(!overlay.visible(), "About's Close button did not fire");

		// A dialog WITH focused text fields (Resize calls focus_first): its
		// Cancel must still fire — covers the focus/IME interplay.
		overlay.open_resize(112, 112);
		let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		let cancel = match overlay.dialog {
			Dialog::Resize(ids) => ids.cancel,
			_ => unreachable!("Resize is open"),
		};
		let r = overlay.ui.rect_of(cancel).expect("Cancel laid out");
		let center = Vec2::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
		for pressed in [true, false] {
			overlay.events.push(Event::PointerButton {
				button: wgpu_ui::PointerButton::Primary,
				pressed,
				pos: center,
				mods,
			});
		}
		let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(!overlay.visible(), "Resize's Cancel button did not fire");

		// Outcome-carrying buttons: About's website link must report OpenUrl.
		overlay.open_about();
		let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		let website = match overlay.dialog {
			Dialog::About(ids) => ids.website,
			_ => unreachable!("About is open"),
		};
		let r = overlay.ui.rect_of(website).expect("website laid out");
		let center = Vec2::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
		for pressed in [true, false] {
			overlay.events.push(Event::PointerButton {
				button: wgpu_ui::PointerButton::Primary,
				pressed,
				pos: center,
				mods,
			});
		}
		let outcome = frame_outcome(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(
			matches!(&outcome, Outcome::OpenUrl(url) if url == WEBSITE),
			"About's website button did not report OpenUrl"
		);
		assert!(overlay.visible(), "About stays open after a link click");
	}

	/// Every player slot takes every type, the out-of-game ones included — a
	/// slot that takes no part is exactly the one the user needs to be able to
	/// assign. The pick is driven through the real select and read back off the
	/// canonical form, so the option-index-to-`TEAM_TYPE_*` mapping is what is
	/// under test. Only the alien slot is fixed (the game reads four teams).
	#[test]
	fn every_player_slot_takes_every_type() {
		let (device, queue, _serial) = crate::visual_test::test_gpu();
		let res = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources");
		let steel = crate::skin::load_steel(&res);
		let mut chrome = MenuChrome::new(&device, &queue, crate::capture::FORMAT, &steel).expect("chrome");
		let mut overlay = Overlay::new(1.0);
		let (w, h) = (800u32, 800u32);

		let init = crate::savedata::tests::init();
		let alien = init.settings.team_types[4];
		overlay.open_save_data(init);
		let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);

		// Blue (slot 2) starts out of the game; walk it through all five.
		for (option, &code) in crate::savedata::TYPE_CHOICES.iter().enumerate() {
			let ids = overlay.sd.ids.clone().expect("the form stored its ids");
			overlay.ui.get_mut::<Select>(ids.team_type[2]).expect("Blue's type select").set_selected(option);
			let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
			assert_eq!(overlay.sd.form.team_type[2], code, "option {option} is {}", savedata::team_type_label(code));
		}

		// The alien slot's select is a disabled single-option stub: whatever it
		// reads, the form keeps the save's own code.
		let ids = overlay.sd.ids.clone().expect("the form stored its ids");
		overlay.ui.get_mut::<Select>(ids.team_type[4]).expect("Alien's stub").set_selected(4);
		let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert_eq!(overlay.sd.form.team_type[4], alien, "the alien slot never moves");
	}

	/// Edit Save Data's whole guard path through real clicks: OK on an invalid
	/// form opens the Issues list (nothing applied), Auto Fix repairs the form
	/// in place, and the next OK reports the corrected settings block.
	#[test]
	fn save_data_ok_validates_auto_fix_repairs_then_applies() {
		let (device, queue, _serial) = crate::visual_test::test_gpu();
		let res = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources");
		let steel = crate::skin::load_steel(&res);
		let mut chrome = MenuChrome::new(&device, &queue, crate::capture::FORMAT, &steel).expect("chrome");
		let mut overlay = Overlay::new(1.0);
		let (w, h) = (800u32, 800u32);
		let mods = wgpu_ui::Modifiers::default();
		let click = |overlay: &mut Overlay, chrome: &mut MenuChrome, id: WidgetId| {
			let r = overlay.ui.rect_of(id).expect("target laid out");
			let center = Vec2::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
			for pressed in [true, false] {
				overlay.events.push(Event::PointerButton {
					button: wgpu_ui::PointerButton::Primary,
					pressed,
					pos: center,
					mods,
				});
			}
			frame_outcome(&device, &queue, overlay, chrome, w, h)
		};

		overlay.open_save_data(crate::savedata::tests::init());
		let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert!(matches!(overlay.dialog, Dialog::SaveData), "Save Data is open");
		let ids = overlay.sd.ids.clone().expect("the form stored its ids");
		// An emptied save name is invalid; OK must divert to the Issues list.
		overlay.set_text(ids.save_name, "");
		let out = click(&mut overlay, &mut chrome, ids.ok);
		assert!(matches!(out, Outcome::Idle), "an invalid form applies nothing");
		let issue_ids = match overlay.dialog {
			Dialog::SaveDataIssues(ids) => ids,
			_ => unreachable!("the Issues list is showing"),
		};
		assert_eq!(overlay.sd.issues.len(), 1, "one invalid field is listed");

		// Auto Fix writes the nearest valid value and returns to the form.
		let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		let out = click(&mut overlay, &mut chrome, issue_ids.fix);
		assert!(matches!(out, Outcome::Idle));
		assert!(matches!(overlay.dialog, Dialog::SaveData), "Auto Fix returns to the form");
		let ids = overlay.sd.ids.clone().expect("the rebuilt form stored its ids");
		assert_eq!(overlay.sd.form.save_name, "UNTITLED", "the fix reached the canonical form");
		let _ = frame(&device, &queue, &mut overlay, &mut chrome, w, h);
		assert_eq!(overlay.text(ids.save_name), "UNTITLED", "the fix reached the rebuilt widget");
		assert!(!overlay.sd.note.is_empty(), "the status note reports the fix");

		// OK on the now-valid form applies the corrected block and closes.
		let out = click(&mut overlay, &mut chrome, ids.ok);
		match out {
			Outcome::ApplySaveData(settings) => {
				assert_eq!(settings.save_name, "UNTITLED");
				let expect = {
					let mut s = crate::savedata::tests::init().settings;
					s.save_name = "UNTITLED".into();
					s
				};
				assert_eq!(*settings, expect, "only the fixed field differs from the opened settings");
			}
			_ => panic!("a valid OK reports ApplySaveData"),
		}
		assert!(!overlay.visible(), "the dialog closed on apply");
	}

	fn write_png(path: &Path, rgba: &[u8], w: u32, h: u32) {
		let Ok(file) = std::fs::File::create(path) else { return };
		let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
		enc.set_color(png::ColorType::Rgba);
		enc.set_depth(png::BitDepth::Eight);
		if let Ok(mut wr) = enc.write_header() {
			let _ = wr.write_image_data(rgba);
		}
		eprintln!("wrote {}", path.display());
	}
}

#[cfg(test)]
mod rename_tests {
	use super::resolve_template_rename;

	#[test]
	fn template_rename_validation_matches_the_legacy_rules() {
		let existing = vec!["Bluff".to_string()];
		assert!(resolve_template_rename("", "Ridge", &existing).is_err(), "empty refused");
		assert!(resolve_template_rename("Ridge", "Ridge", &existing).is_err(), "unchanged refused");
		assert!(resolve_template_rename("Bluff", "Ridge", &existing).is_err(), "collision refused");
		assert_eq!(
			resolve_template_rename("  Brook  ", "Ridge", &existing).unwrap(),
			"template-rename \"Ridge\" \"Brook\"",
			"trims + quotes both names"
		);
	}
}

/// Headless state-machine cover for the GPU-free dialogs: the value mappings,
/// validations, and stage flips a dialog runs on its own, driven by opening it
/// and calling its logic directly (no `wgpu::Device`/`render` needed — those
/// paths are the `render_tests` module's beat). Dialogs whose open registers a
/// preview/thumbnail texture (New Map, Import WRL, Rename/Delete Template, Tile
/// Painter, Match editor) need a device and live in `render_tests`.
#[cfg(test)]
mod state_tests {
	use super::*;
	use crate::uikit_theme::SteelTheme;
	use wgpu_ui::Fonts;

	/// A map dimension parses to an in-range `u16` or names the reason it does
	/// not - New Map and Resize show that reason inline instead of silently
	/// coercing (the audit's `12a0` -> 112 case, item 5).
	#[test]
	fn parse_dim_names_the_reason_it_refuses() {
		assert_eq!(parse_dim("256"), Ok(256));
		assert_eq!(parse_dim("  64 "), Ok(64), "surrounding whitespace is trimmed");
		assert_eq!(parse_dim("0").unwrap_err(), "0 is out of 1..=1024", "the floor refuses, not clamps");
		assert_eq!(parse_dim("2000").unwrap_err(), "2000 is out of 1..=1024", "the ceiling refuses, not clamps");
		assert_eq!(parse_dim("abc").unwrap_err(), "'abc' is not a number in 1..=1024");
		assert_eq!(parse_dim("").unwrap_err(), "'' is not a number in 1..=1024", "empty refuses, no 112 default");
		assert_eq!(parse_dim("99999").unwrap_err(), "'99999' is not a number in 1..=1024", "a value past u16");
	}

	/// Map Metadata maps the project's player count to the players Select on
	/// open (the 2-player floor: `Some(2)`→"2-2", `Some(3)`→"2-3", `Some(4)`→
	/// "2-4", `None` and any other count → "Unspecified"), and Save collects the
	/// selected row back to the same count.
	#[test]
	fn metadata_players_map_both_ways() {
		for (players, index, back) in
			[(None, 0, None), (Some(2), 1, Some(2)), (Some(3), 2, Some(3)), (Some(4), 3, Some(4)), (Some(9), 0, None)]
		{
			let mut o = Overlay::new(1.0);
			o.open_metadata(MetadataValues { players, ..Default::default() }, false);
			let Dialog::Metadata(ids) = o.dialog else { unreachable!("Metadata is open") };
			assert_eq!(
				o.ui.get::<Select>(ids.players).map(Select::selected),
				Some(index),
				"{players:?} -> row {index}"
			);
			assert_eq!(o.collect_metadata(&ids).players, back, "row {index} collects back to {back:?}");
		}
	}

	/// Save collects the edited fields verbatim into the returned values (the
	/// description from the TextArea, the rest from the TextInputs).
	#[test]
	fn metadata_collects_edited_fields() {
		let mut o = Overlay::new(1.0);
		o.open_metadata(
			MetadataValues {
				name: "New Luzon".into(),
				players: Some(4),
				description: "drowned range".into(),
				date: "1996".into(),
				version: "1.0".into(),
				author: "MAX".into(),
			},
			false,
		);
		let Dialog::Metadata(ids) = o.dialog else { unreachable!("Metadata is open") };
		let vals = o.collect_metadata(&ids);
		assert_eq!(vals.name, "New Luzon");
		assert_eq!(vals.players, Some(4));
		assert_eq!(vals.description, "drowned range");
		assert_eq!(vals.date, "1996");
		assert_eq!(vals.version, "1.0");
		assert_eq!(vals.author, "MAX");
	}

	/// Selects the resize anchor radio at `idx` (row-major 0..9), clearing the
	/// rest — the one-group behaviour the dispatch loop applies at runtime.
	fn set_anchor(o: &mut Overlay, idx: usize) {
		let ids = o.anchor_ids.clone();
		o.radio_select(&ids, Some(idx));
	}

	/// Resize derives the old map's offset inside the new bounds from the 3×3
	/// anchor: column/row 0 pins the top/left edge (offset 0), 1 centres (half
	/// the size delta), 2 pins the bottom/right edge (the full delta). The
	/// command line is `resize W H OFFX OFFY`.
	#[test]
	fn resize_offsets_follow_the_anchor() {
		let mut o = Overlay::new(1.0);
		o.open_resize(112, 112);
		let Dialog::Resize(ids) = o.dialog else { unreachable!("Resize is open") };
		o.set_text(ids.width, "224");
		o.set_text(ids.height, "224");
		// Centre (the open-time default) → half the +112 delta on each axis.
		assert_eq!(o.resize_command(&ids).as_deref(), Some("resize 224 224 56 56"));
		set_anchor(&mut o, 0); // top-left corner: no offset
		assert_eq!(o.resize_command(&ids).as_deref(), Some("resize 224 224 0 0"));
		set_anchor(&mut o, 8); // bottom-right corner: the full delta
		assert_eq!(o.resize_command(&ids).as_deref(), Some("resize 224 224 112 112"));
	}

	/// The live resize note names the operation from the size change (grow both
	/// axes = enlarge/fill, shrink both = crop, one of each = fill-and-crop) and
	/// echoes the source size + derived offset; an out-of-range W/H clears both
	/// the note and the command.
	#[test]
	fn resize_note_reads_the_size_change() {
		let mut o = Overlay::new(1.0);
		o.open_resize(112, 112);
		let Dialog::Resize(ids) = o.dialog else { unreachable!("Resize is open") };
		// Centre anchor throughout; grow both axes.
		o.set_text(ids.width, "224");
		o.set_text(ids.height, "224");
		assert_eq!(o.resize_note(&ids), "Enlarge - fills with water   from 112 x 112, at 56, 56");
		// Shrink both axes.
		o.set_text(ids.width, "64");
		o.set_text(ids.height, "64");
		assert_eq!(o.resize_note(&ids), "Shrink - crops to the anchor   from 112 x 112, at -24, -24");
		// Grow one axis, shrink the other.
		o.set_text(ids.width, "224");
		o.set_text(ids.height, "64");
		assert_eq!(o.resize_note(&ids), "Resize - fills and crops   from 112 x 112, at 56, -24");
		// A bad field: the note says why (live, in the same slot) and the
		// command refuses - Resize no longer no-ops silently (audit item 5).
		o.set_text(ids.width, "0");
		assert_eq!(o.resize_note(&ids), "width 0 is out of 1..=1024");
		assert!(o.resize_command(&ids).is_none(), "an invalid size yields no command line");
		o.set_text(ids.width, "224");
		o.set_text(ids.height, "");
		assert_eq!(o.resize_note(&ids), "height '' is not a number in 1..=1024");
	}

	/// Convert Palette / best match confirms straight to a plain command line
	/// (closing the dialog), carrying the water flag as `water=keep|drop`.
	#[test]
	fn convert_palette_best_match_runs_command() {
		let mut o = Overlay::new(1.0);
		o.open_convert_palette();
		let Dialog::ConvertPalette(ids) = o.dialog else { unreachable!("Convert Palette is open") };
		match o.convert_palette_confirm(ids) {
			Outcome::RunCommand(s) => assert_eq!(s, "convert-palette match water=keep"),
			_ => panic!("best match runs the plain command"),
		}
		assert!(!o.visible(), "best match confirms and closes");

		let mut o = Overlay::new(1.0);
		o.open_convert_palette();
		o.cp.water = false;
		let Dialog::ConvertPalette(ids) = o.dialog else { unreachable!("Convert Palette is open") };
		match o.convert_palette_confirm(ids) {
			Outcome::RunCommand(s) => assert_eq!(s, "convert-palette match water=drop"),
			_ => panic!("the water flag flips to drop"),
		}
	}

	/// Convert Palette / rasterize hands the run to the shell: strict dedupe
	/// starts at threshold 0; relaxed parses the percent field into a 0..1
	/// fraction; a non-numeric or out-of-range percent refuses inline (Idle,
	/// dialog kept open with an alert).
	#[test]
	fn convert_palette_rasterize_validates_threshold() {
		let start = |o: &mut Overlay, relaxed: bool, threshold: &str| {
			o.cp.rasterize = true;
			o.cp.relaxed = relaxed;
			o.cp.threshold = threshold.to_string();
			o.build_convert_palette();
			let Dialog::ConvertPalette(ids) = o.dialog else { unreachable!("Convert Palette is open") };
			o.convert_palette_confirm(ids)
		};
		let mut o = Overlay::new(1.0);
		o.open_convert_palette();
		// Strict: no threshold field, the run starts at 0.
		match start(&mut o, false, "") {
			Outcome::PaletteConvertStart { relaxed, threshold, .. } => {
				assert!(!relaxed, "strict dedupe");
				assert!(threshold == 0.0, "strict starts at threshold 0");
			}
			_ => panic!("rasterize + strict starts the run"),
		}
		// Relaxed 5% → 0.05, water flag carried.
		match start(&mut o, true, "5") {
			Outcome::PaletteConvertStart { relaxed, threshold, water } => {
				assert!(relaxed);
				assert!((threshold - 0.05).abs() < 1e-6, "5% is 0.05");
				assert!(water, "the water flag rides along");
			}
			_ => panic!("relaxed with a valid percent starts the run"),
		}
		// Out of range and non-numeric both refuse inline.
		assert!(matches!(start(&mut o, true, "150"), Outcome::Idle));
		let Dialog::ConvertPalette(ids) = o.dialog else { unreachable!("Convert Palette is open") };
		assert!(o.ui.get::<Label>(ids.error).unwrap().text().contains("0..=100"), "the range is named");
		assert!(matches!(start(&mut o, true, "abc"), Outcome::Idle));
		let Dialog::ConvertPalette(ids) = o.dialog else { unreachable!("Convert Palette is open") };
		assert!(o.ui.get::<Label>(ids.error).unwrap().text().contains("not a number"));
	}

	/// Convert Palette's per-frame sync flips the options stage to the running
	/// stage view and drives its progress bar / stage / time widgets.
	#[test]
	fn convert_palette_sync_flips_to_running_stage() {
		let mut o = Overlay::new(1.0);
		o.open_convert_palette();
		assert!(!o.cp.running, "opens on the options stage");
		o.sync_convert_palette(true, 0.4, "Re-importing", "40%   elapsed 0.8s");
		assert!(o.cp.running, "the sync flipped to the running stage");
		let Dialog::ConvertPalette(ids) = o.dialog else { unreachable!("Convert Palette is open") };
		assert!((o.ui.get::<ProgressBar>(ids.bar).unwrap().fraction() - 0.4).abs() < 1e-6);
		assert_eq!(o.ui.get::<Label>(ids.stage).unwrap().text(), "Re-importing");
		assert_eq!(o.ui.get::<Label>(ids.time).unwrap().text(), "40%   elapsed 0.8s");
	}

	/// New from Image validates the settings into `ConvertOpts`: an in-range
	/// size, the coverage index → enum, offsets (empty = 0), and dedupe (strict
	/// = 0 threshold).
	#[test]
	fn new_image_builds_convert_opts() {
		let mut o = Overlay::new(1.0);
		o.open_new_image(64, 48);
		o.ni.off_x = "3".into();
		o.ni.off_y = "-2".into();
		o.ni.coverage = 2; // Fill
		let Dialog::NewImage(ids) = o.dialog else { unreachable!("New from Image is open") };
		match o.new_image_confirm(ids) {
			Outcome::NewImageStart(opts) => {
				assert_eq!((opts.width_tiles, opts.height_tiles), (64, 48));
				assert_eq!((opts.off_x, opts.off_y), (3, -2));
				assert!(matches!(opts.coverage, map_core::Coverage::Fill), "coverage index 2 is Fill");
				assert!(matches!(opts.dedupe, map_core::Dedupe::Strict));
				assert!(opts.threshold == 0.0, "strict dedupe ignores the threshold");
			}
			_ => panic!("valid settings start the conversion"),
		}
	}

	/// New from Image / relaxed dedupe converts the percent field to a clamped
	/// 0..1 fraction; a non-numeric width and an over-ceiling size both refuse
	/// inline (Idle) with an alert naming the offending field.
	#[test]
	fn new_image_relaxed_and_rejections() {
		let mut o = Overlay::new(1.0);
		o.open_new_image(64, 48);
		o.ni.relaxed = true;
		o.ni.threshold = "50".into();
		let Dialog::NewImage(ids) = o.dialog else { unreachable!("New from Image is open") };
		match o.new_image_confirm(ids) {
			Outcome::NewImageStart(opts) => {
				assert!(matches!(opts.dedupe, map_core::Dedupe::Relaxed));
				assert!((opts.threshold - 0.5).abs() < 1e-6, "50% is 0.5");
			}
			_ => panic!("relaxed with a valid percent starts the conversion"),
		}
		o.ni.width = "abc".into();
		assert!(matches!(o.new_image_confirm(ids), Outcome::Idle));
		assert!(o.ui.get::<Label>(ids.error).unwrap().text().contains("width"), "the bad field is named");
		o.ni.width = "2000".into();
		assert!(matches!(o.new_image_confirm(ids), Outcome::Idle));
		assert!(o.ui.get::<Label>(ids.error).unwrap().text().contains("1..=1024"), "the range is named");
	}

	/// New from Image captures its in-progress field text into the canonical
	/// copies before a rebuild, so flipping a row never drops what was typed.
	#[test]
	fn new_image_capture_preserves_typing() {
		let mut o = Overlay::new(1.0);
		o.open_new_image(64, 48);
		let Dialog::NewImage(ids) = o.dialog else { unreachable!("New from Image is open") };
		o.set_text(ids.width, "100");
		o.set_text(ids.off_x, "7");
		o.capture_new_image(ids);
		assert_eq!(o.ni.width, "100");
		assert_eq!(o.ni.off_x, "7");
	}

	/// Generate validates every visible knob field and the seed, then hands the
	/// settings to the shell: an empty seed rolls fresh (`None`), a numeric seed
	/// is carried; a knob field that overflows `u8` or a non-numeric seed refuse
	/// inline (Idle).
	#[test]
	fn generate_confirm_validates_and_carries_seed() {
		let mut o = Overlay::new(1.0);
		o.open_generate(&genform::GenMemory::default(), 64, 64);
		let Dialog::Generate(ids) = o.dialog else { unreachable!("Generate is open") };
		// Default fields are valid; an empty seed rolls fresh.
		assert!(matches!(o.generate_confirm(ids), Outcome::GenerateStart { seed: None, .. }));
		// A typed seed is parsed and carried.
		o.set_text(ids.seed, "42");
		assert!(matches!(o.generate_confirm(ids), Outcome::GenerateStart { seed: Some(42), .. }));
		// A knob field that overflows u8 (three digits fit the field) refuses.
		let fid =
			o.generate.rows.iter().flat_map(|(_, _, c)| c.iter().copied()).find(|&i| i != WidgetId::NONE).unwrap();
		o.set_text(fid, "999");
		assert!(matches!(o.generate_confirm(ids), Outcome::Idle));
		assert!(o.ui.get::<Label>(ids.error).unwrap().text().contains("not a number"));
		// With the knob fixed, a non-numeric seed still refuses.
		o.set_text(fid, "1");
		o.set_text(ids.seed, "notaseed");
		assert!(matches!(o.generate_confirm(ids), Outcome::Idle));
		assert!(o.ui.get::<Label>(ids.error).unwrap().text().contains("seed"));
	}

	/// The one-field dialog submits on Enter: the focused field fires a commit
	/// (the toolkit's TextInput Enter-fire), which the ObjectField arm treats like
	/// OK — and `object_field_confirm` validates the entry into its command.
	#[test]
	fn one_field_dialog_submits_on_enter() {
		use wgpu_ui::{Event, Key, Modifiers};
		let mut o = Overlay::new(1.0);
		o.open_resource_amount("8"); // amount editor: `resource-brush amount N`, 0-31
		let Dialog::ObjectField(ids) = o.dialog else { unreachable!("ObjectField is open") };
		o.set_text(ids.value, "20");
		// Enter in the focused field fires it (open_field_with_verb focus_first's it).
		o.ui.dispatch(&[Event::Key { key: Key::Enter, pressed: true, repeat: false, mods: Modifiers::NONE }]);
		assert!(o.ui.fired(ids.value), "Enter fires the field (treated as OK by the dialog)");
		assert!(matches!(o.object_field_confirm(ids), Outcome::RunCommand(s) if s == "resource-brush amount 20"));
	}

	/// Closing Generate captures the form into per-generator session memory,
	/// hands it back, and hides; the dialog is a non-blocking float.
	#[test]
	fn generate_close_returns_memory_and_hides() {
		let mut o = Overlay::new(1.0);
		o.open_generate(&genform::GenMemory::default(), 64, 64);
		assert!(!o.blocking(), "Generate floats over the live map");
		assert!(matches!(o.generate_close_outcome(), Outcome::GenerateClose(_)));
		assert!(!o.visible(), "closing hides the dialog");
	}

	/// Fix Shore's per-frame sync pushes the live counts into the stat labels
	/// and tracks the run flag; the applied line only shows once a result lands.
	#[test]
	fn autofix_sync_updates_stats_and_run_flag() {
		let mut o = Overlay::new(1.0);
		o.open_autofix(42);
		assert!(!o.blocking(), "Fix Shore floats (no scrim)");
		let Dialog::AutoFix(ids) = o.dialog else { unreachable!("Fix Shore is open") };
		let text = |o: &Overlay, id| o.ui.get::<Label>(id).unwrap().text().to_string();
		o.sync_autofix(true, 42, 7, 35, "1.2s", None);
		assert!(o.af_running, "the run flag follows the sync");
		assert_eq!(text(&o, ids.fixed), "7");
		assert_eq!(text(&o, ids.remaining), "35");
		assert_eq!(text(&o, ids.elapsed), "1.2s");
		assert_eq!(text(&o, ids.applied), "", "no applied line until a result lands");
		o.sync_autofix(false, 42, 42, 0, "2.0s", Some(9));
		assert!(!o.af_running);
		assert_eq!(text(&o, ids.applied), "applied - 9 cells changed");
	}

	/// A confirm dialog stores the command line its primary will emit; the
	/// three-way save guard additionally arms a discard command on its middle
	/// button (the plain confirm has none).
	#[test]
	fn confirm_dialogs_store_their_commands() {
		let mut o = Overlay::new(1.0);
		o.open_confirm(
			"Delete Palette",
			"Delete \"swamp\"?",
			"cannot undo",
			"Delete",
			"palette-delete \"swamp\"".into(),
		);
		assert!(matches!(o.dialog, Dialog::Confirm(ids) if ids.discard.is_none()), "a plain confirm has no discard");
		assert_eq!(o.confirm.cmd, "palette-delete \"swamp\"");

		o.open_confirm_save("Unsaved", "unsaved changes", "Save", "save-and-close".into(), "Discard", "close!".into());
		assert!(matches!(o.dialog, Dialog::Confirm(ids) if ids.discard.is_some()), "the guard arms a discard button");
		assert_eq!(o.confirm.cmd, "save-and-close");
		assert_eq!(o.confirm.discard_cmd, "close!");
	}

	/// Remove Duplicate Templates is a danger confirm running `template-dedupe!`
	/// when duplicates exist, and a dismiss-only notice when none do.
	#[test]
	fn dedupe_dialog_switches_between_confirm_and_notice() {
		let mut o = Overlay::new(1.0);
		o.open_dedupe(&["Ridge".to_string(), "Bluff".to_string()]);
		assert!(matches!(o.dialog, Dialog::Confirm(_)), "duplicates present -> a confirm");
		assert_eq!(o.confirm.cmd, "template-dedupe!");

		o.open_dedupe(&[]);
		assert!(matches!(o.dialog, Dialog::Error(_)), "nothing duplicated -> a dismiss-only notice");
	}

	/// Hiding a dialog clears the visible/dialog state and the per-dialog
	/// bookkeeping (here the Resize anchor ids + window) so nothing leaks into
	/// the next open.
	#[test]
	fn hide_resets_dialog_state() {
		let mut o = Overlay::new(1.0);
		o.open_resize(112, 112);
		assert!(o.visible());
		assert!(!o.anchor_ids.is_empty());
		o.hide();
		assert!(!o.visible());
		assert!(matches!(o.dialog, Dialog::None));
		assert!(o.anchor_ids.is_empty(), "the anchor ids are cleared");
		assert!(o.win_id.is_none(), "the window handle is dropped");
	}

	/// A headless steel theme + fonts for laying out and drawing a dialog tree
	/// into a `DrawList` (a dummy steel texture id — the list only records
	/// commands, so no GPU is needed). Mirrors the panel modules' `skin()`.
	fn skin() -> (SteelTheme, Fonts) {
		let mut fonts = Fonts::new();
		let font = fonts.add(include_bytes!("../assets/MAX_Redesign_Square.ttf").to_vec()).unwrap();
		let em = fonts.get(font).units_per_em();
		(SteelTheme::new(font, TextureId::ATLAS, em), fonts)
	}

	/// A GPU-free dialog lays out and draws real content: the About tree emits
	/// glyph commands (its credits/labels) and solid fills (the steel body /
	/// dimming scrim) into the DrawList.
	#[test]
	fn about_dialog_draws_glyphs_and_fills() {
		use wgpu_ui::DrawCmd;
		let (steel, fonts) = skin();
		let mut o = Overlay::new(1.0);
		o.open_about();
		o.ui.layout(Size::new(620.0, 800.0), &steel, &fonts);
		let mut dl = DrawList::new();
		o.ui.draw(&mut dl, &steel, &fonts);
		let glyphs = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::Glyph { .. })).count();
		let solids = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::Solid { .. })).count();
		assert!(glyphs > 0, "the About text draws glyphs ({glyphs})");
		assert!(solids > 0, "the steel body / scrim draw solid fills ({solids})");
	}

	/// UI Tests: the probe rasterizes at the size its ladder claims, so the
	/// *set of glyph px* it emits must widen with the UI scale rather than
	/// repeat one size. A regression that pinned every run to one em (the very
	/// fault this dialog exists to rule out) would collapse this set.
	#[test]
	fn ui_tests_probe_draws_every_ladder_size() {
		use wgpu_ui::DrawCmd;
		let (mut steel, fonts) = skin();
		for scale in [1.0f32, 1.5] {
			steel.set_scale(scale);
			let mut o = Overlay::new(scale as f64);
			o.open_ui_tests();
			o.ui.set_scale(scale);
			o.ui.layout(Size::new(700.0, 900.0), &steel, &fonts);
			let mut dl = DrawList::new();
			o.ui.draw(&mut dl, &steel, &fonts);
			// The physical raster sizes the frame actually asked the cache for.
			let mut buckets: Vec<u32> = dl
				.cmds
				.iter()
				.filter_map(|c| match c {
					DrawCmd::Glyph { px, .. } => Some((px * scale).round().max(1.0) as u32),
					_ => None,
				})
				.collect();
			buckets.sort_unstable();
			buckets.dedup();
			assert!(
				buckets.len() >= 8,
				"{scale}x: the probe drew only {} distinct raster sizes ({buckets:?})",
				buckets.len()
			);
			// Both ends of the ladder are present, whatever the scale - the
			// probe asks for `physical / scale`, so its sweep is scale-proof.
			for end in [16u32, 32] {
				assert!(buckets.contains(&end), "{scale}x: no {end}px row ({buckets:?})");
			}
		}
	}
}
