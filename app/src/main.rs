//! M.A.X. Map Editor - Rust + WGPU.
//!
//! All mutation flows through `Command`s (see `command.rs`) executed by
//! `EditorState::execute` - interactive input, `--script` files, key bindings
//! and the in-app console all share that one path.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod blit;
mod browser;
mod capture;
mod cellgrid;
mod clipboard;
mod command;
mod console;
mod console_view;
mod crt;
mod fontprobe;
mod genform;
mod gpu;
mod grid;
mod input;
mod markers;
mod markers_render;
mod matcheditor;
mod matchview;
mod menu;
mod minimap;
mod newmap;
mod packlist;
mod palette;
mod palette_io;
mod palette_panel;
mod panel_ui;
mod passtools;
mod picker;
mod project_render;
mod render;
mod savedata;
mod savetools;
mod scenery;
mod scenery_render;
mod scenerypaint;
mod settings_io;
mod skin;
mod state;
mod statusbar;
mod tabs;
mod template_preview;
mod templates_panel;
mod theme;
mod tile_atlas;
mod tilepaint;
mod toolbox;
mod ui;
mod ui_router;
mod uikit_menu;
mod uikit_overlay;
mod uikit_theme;
mod unitprops;
mod units;
mod units_render;
#[cfg(test)]
mod visual_dialogs;
#[cfg(test)]
mod visual_panels;
#[cfg(test)]
mod visual_test;
mod workspace;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use wgpu_ui::Vec2;
/// The toolkit's own event — what the router translates a `winit::WindowEvent`
/// into, and the only kind any UI host is ever fed.
use wgpu_ui::{BlurCause, Event, PointerButton};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use ini::INI;
use map_core::Project;

use crate::blit::BlitPass;
use crate::command::Command;
use crate::minimap::MinimapPass;
use crate::panel_ui::{PanelInput, PanelUi};
use crate::project_render::ProjectRenderer;
use crate::state::{EditorState, Outcome};
use crate::ui_router::{Layer, Over};
use crate::uikit_theme::rgba;
use wgpu_ui::DrawList;

/// The body of the experimental-feature warning shown before the Open Save File
/// picker (item: the save editor is experimental and can break real saves).
/// Confirming it runs `file-dialog open-save`.
const EXPERIMENTAL_SAVE_WARNING: &str = "The Save File editor is EXPERIMENTAL. It may not work, and it can corrupt or destroy real saved games.\n\n\
	 Back up your save files manually before you touch anything here.\n\n\
	 Misuse may result in unforeseen consequences, like: world destruction or -kzzt- your -kzzt- cat.";

/// The red warning line under [`EXPERIMENTAL_SAVE_WARNING`]: a modified save is
/// not the game's fault, so don't file its bugs against the game.
const EXPERIMENTAL_SAVE_BUG_WARNING: &str = "/!\\ DO NOT REPORT GAME BUGS IF YOU PLAY ON MODIFIED SAVE FILES";

/// The shared draw passes (one set per GPU device).
struct Passes {
	blit: BlitPass,
	minimap: MinimapPass,
	grid: grid::GridPass,
	/// CRT post-process + its offscreen scene target (lazily sized).
	crt: crt::CrtPass,
	scene: Option<crt::SceneTarget>,
	/// Unit-preview pass - built lazily on the first frame after the unit
	/// library loads (needs the sprite data for its atlas).
	units: Option<units_render::UnitsGpu>,
	/// Resource-marker overlay pass (View ▸ Resources) - built lazily on the
	/// first frame after the marker library loads.
	markers: Option<markers_render::MarkersGpu>,
	/// Scenery pass - built lazily from the open project's cut-out libraries,
	/// and rebuilt when those change (a project on another tile pack).
	scenery: Option<scenery_render::SceneryGpu>,
	/// The wgpu-ui renderer for the main menu bar + context menu (`None` only if
	/// the menu font fails to parse). In `Passes` so `render_frame` reaches it in
	/// every path, headless screenshots included.
	menu_chrome: Option<uikit_menu::MenuChrome>,
	/// The console: one hosted widget — plate, scrollback, prompt and a
	/// monospace `wgpu_ui::TextInput` — laid out and drawn like any other panel
	/// (U4.5). It was the last surface with a bespoke font stack and no pointer
	/// path at all.
	console_view: panel_ui::PanelHost<console_view::ConsoleView>,
	/// The retained-mode status bar (a `wgpu_ui::Ui` of labels), drawn through
	/// `menu_chrome`. First panel converted off immediate mode.
	status: statusbar::StatusBar,
	/// The retained-mode project tab strip; input runs through its `Ui` dispatch.
	tabs_strip: panel_ui::PanelHost<tabs::TabStrip>,
	/// The retained-mode minimap chrome (mode radios + view rect); fully
	/// retained — drawing + input both.
	minimap_overlay: panel_ui::PanelHost<minimap::MinimapOverlay>,
	/// The retained-mode toolbox (flowed command groups, preview box, brush
	/// dropdown, scrollbar); fully retained — drawing + input both.
	toolbox_content: panel_ui::PanelHost<toolbox::ToolboxContent>,
	/// The retained-mode Save Toolbox (object tools + ground cover + team keys);
	/// fully retained — drawing + input both.
	savetools_content: panel_ui::PanelHost<savetools::SaveToolsContent>,
	/// The retained-mode Pass Types Palette (the four pass swatches + the cell
	/// tally); fully retained — drawing + input both.
	passtools_content: panel_ui::PanelHost<passtools::PassToolsContent>,
	/// The Unit Properties inspector/editor for the selected save object; team
	/// swatches + facing/orders steppers fire `object-edit` (S4.2). Fully
	/// retained — drawing + input both.
	unitprops_content: panel_ui::PanelHost<unitprops::UnitPropsContent>,
	/// The retained-mode Tile Explorer (filter/size dropdowns, action buttons,
	/// the static-still tile grid); fully retained — drawing + input both.
	picker_content: panel_ui::PanelHost<picker::PickerContent>,
	/// The retained-mode units chrome (team swatches, eraser, active tag, grid
	/// rings, scrollbar); fully retained — drawing + input both.
	units_content: panel_ui::PanelHost<units::UnitsContent>,
	/// The Scenery panel: the cut-out libraries as a picker grid over a fixed
	/// header band, the same shape the Units panel uses (U5.7).
	scenery_content: panel_ui::PanelHost<scenery::SceneryContent>,
	/// The retained-mode Templates Explorer (command keys, the tileset + preview
	/// dropdowns, the count and the thumbnail grid); fully retained — drawing +
	/// input both.
	templates_content: panel_ui::PanelHost<templates_panel::TemplatesContent>,
	/// The retained-mode Color Palette panel (swatch grid + tab bar + editor
	/// strip); fully retained — drawing + input both.
	palette_content: panel_ui::PanelHost<palette_panel::PaletteContent>,
	/// The retained-mode WRL Internal Palette panel — the same widget synced
	/// `bare`, but its **own** host. One instance served both panels until U1.6,
	/// synced per frame, so they could not hold independent retained state
	/// (arming, and later focus / scroll / capture).
	wrlpalette_content: panel_ui::PanelHost<palette_panel::PaletteContent>,
	/// The right-click context menu: a `wgpu_ui::ContextMenu` hosted alone in
	/// its own `Ui`, synced each frame from the editor's model snapshot
	/// (`EditorState::context_menu`); fired ids resolve through `context_acts`.
	context_menu: panel_ui::PanelHost<wgpu_ui::ContextMenu>,
	/// The open context menu's act table + the snapshot key it was built for
	/// (anchor + item count), so a replaced model re-syncs the widget.
	context_acts: Vec<menu::Act>,
	context_synced: Option<(f32, f32, usize)>,
	/// The static tile atlas the retained panels draw tiles from (rest palette,
	/// no cycling — see [`tile_atlas`]); recomposed by [`refresh_tile_atlas`].
	tile_atlas: Option<TileAtlasState>,
	/// The composed template-thumbnail atlas the Templates panel draws from
	/// (rest palette); recomposed by [`refresh_template_atlas`].
	template_atlas: Option<TemplateAtlasState>,
	format: wgpu::TextureFormat,
}

/// The retained [`PanelUi`] behind a [`Layer`], by **shared** reference — the
/// read-only twin of [`App::panel_input`], for the focus / IME questions the
/// shell asks without dispatching. A free fn rather than a method because
/// `App::redraw` asks them with `self.win` already mutably borrowed; taking
/// `passes` and `editor` separately keeps the borrows disjoint.
///
/// A **hidden** panel answers `None`: it is neither laid out nor drawn, so
/// whatever its `Ui` still calls focused is stale — it must not claim the
/// keyboard or the OS IME on the strength of a click from before it was closed.
/// [`Layer::Overlay`] is likewise absent: a dialog is arbitrated ahead of the
/// layer stack, not inside it.
fn layer_panel<'a>(passes: &'a Passes, editor: &'a EditorState, layer: Layer) -> Option<&'a PanelUi> {
	if let Layer::Panel(id) = layer
		&& !editor.workspace.is_visible(id)
	{
		return None;
	}
	Some(match layer {
		Layer::MenuBar => &editor.menu_panel,
		Layer::ContextMenu => &passes.context_menu.panel,
		Layer::Tabs => &passes.tabs_strip.panel,
		Layer::Console => &passes.console_view.panel,
		Layer::Panel("tiles") => &passes.picker_content.panel,
		Layer::Panel("units") => &passes.units_content.panel,
		Layer::Panel("scenery") => &passes.scenery_content.panel,
		Layer::Panel("minimap") => &passes.minimap_overlay.panel,
		Layer::Panel("toolbox") => &passes.toolbox_content.panel,
		Layer::Panel("savetools") => &passes.savetools_content.panel,
		Layer::Panel("passtools") => &passes.passtools_content.panel,
		Layer::Panel("unitprops") => &passes.unitprops_content.panel,
		Layer::Panel("templates") => &passes.templates_content.panel,
		Layer::Panel("palette") => &passes.palette_content.panel,
		Layer::Panel("wrlpalette") => &passes.wrlpalette_content.panel,
		_ => return None,
	})
}

/// The docked panel hosted under workspace id `id`, type-erased — the mutable
/// twin of [`layer_panel`], keyed by the id rather than the [`Layer`] so the
/// render loop (which walks `Workspace::layout`, whose ids are borrowed, not
/// `'static`) can reach a host too.
fn panel_host<'a>(passes: &'a mut Passes, id: &str) -> Option<&'a mut dyn PanelInput> {
	Some(match id {
		"tiles" => &mut passes.picker_content,
		"units" => &mut passes.units_content,
		"scenery" => &mut passes.scenery_content,
		"minimap" => &mut passes.minimap_overlay,
		"toolbox" => &mut passes.toolbox_content,
		"savetools" => &mut passes.savetools_content,
		"passtools" => &mut passes.passtools_content,
		"unitprops" => &mut passes.unitprops_content,
		"templates" => &mut passes.templates_content,
		"palette" => &mut passes.palette_content,
		"wrlpalette" => &mut passes.wrlpalette_content,
		_ => return None,
	})
}

/// The docked panel holding an open dropdown, if one is — the press-modal claim
/// [`over_at`] takes above every panel rect. Widget state, read back from the
/// hosted `Ui`s rather than mirrored onto [`EditorState`]; at most one can be
/// open, because a press anywhere else routes to the owner and dismisses it —
/// the routing rides on the pointer grab (an open popup reports
/// `Response::capturing`, so the router holds the layer and the press cascade's
/// capture branch feeds it the press); this answer is the *render* half, for
/// hover, ghosts and the wheel.
fn popup_layer(passes: &Passes, editor: &EditorState) -> Option<Layer> {
	Layer::HOSTED
		.into_iter()
		.filter(|l| matches!(l, Layer::Panel(_)))
		.find(|&l| layer_panel(passes, editor, l).is_some_and(PanelUi::popup_open))
}

/// The layer that owns the keyboard right now.
///
/// The open console is the one *context* that outranks the router's press-driven
/// focus: it is an app accelerator mode you enter with a key, not a panel you
/// clicked into, so while it is up it holds the keyboard whatever was focused
/// before. Everything else is the router's answer.
///
/// One resolver, so the keyboard routing, the keymap gate, the Escape cascade and
/// the IME arbiter can never disagree — before U1.4 those were four separate
/// ad-hoc tests. A free fn for the same reason as [`layer_panel`]: `App::redraw`
/// asks with `self.win` already mutably borrowed.
fn focus_layer(editor: &EditorState, router: &ui_router::UiRouter) -> Option<Layer> {
	if editor.console.is_open() {
		return Some(Layer::Console);
	}
	router.focus()
}

/// What is under the pointer at logical (chrome-space) `lcx`/`lcy` — the shell's
/// **one** pointer hit test, resolved top-down through the z-order (U1.5).
///
/// The map is the fallthrough, [`Over::Map`], and *every* map-side gate now asks
/// this and nothing else: the tools' press guard, the pan/context-menu guard, the
/// wheel's zoom-vs-panel routing, the move arm's redraw test, and the four render
/// gates (brush outline, stamp ghost, tile ghost, unit ghost) plus the status
/// bar's cell readout. Seven ad-hoc conjunctions collapse into this one — see
/// [`Over`] for what they disagreed about.
///
/// Three layers are *press-modal*: an open menu cascade, an open context menu and
/// a panel with an open dropdown each take the next press wherever it lands (a
/// row fires, anything else dismisses), so they cover the whole window rather than
/// their own rect — which is also how `render_frame` already dims shell hover
/// while one is up. The third arrives as `popup`, the panel whose hosted `Ui`
/// reports a popup open ([`popup_layer`]); it is widget state, so the shell reads
/// it back rather than mirroring it onto [`EditorState`] (the U2 rule).
///
/// [`Layer::Overlay`] is absent for the same reason as in [`layer_panel`]: a
/// dialog is arbitrated ahead of the layer stack, at the top of `window_event`,
/// and a non-blocking float gates on its own `wants_pointer_at`.
fn over_at(editor: &EditorState, popup: Option<Layer>, lcx: f32, lcy: f32) -> Over {
	if editor.context_menu.is_some() {
		return Over::Ui(Layer::ContextMenu);
	}
	if let Some(layer) = popup {
		return Over::Ui(layer);
	}
	// The open console is drawn over every other layer, so it owns presses over
	// its band - the menu bar and tab strip *underneath* it must not take them
	// (U4.5). Below the band the map and panels are live as usual.
	if editor.console.is_open() {
		let (sw, sh) = editor.ui_screen();
		if console_view::console_rect(sw, sh).contains(wgpu_ui::Vec2::new(lcx, lcy)) {
			return Over::Ui(Layer::Console);
		}
	}
	if editor.menu_ref().is_open() || lcy < menu::BAR_H {
		return Over::Ui(Layer::MenuBar);
	}
	if lcy < menu::BAR_H + tabs::BAR_H {
		return Over::Ui(Layer::Tabs);
	}
	let (sw, sh) = editor.ui_screen();
	// A panel rect covers its titlebar as well as its body, so this answers for
	// the whole window; `Workspace` routes the two apart itself on a press.
	if let Some((id, _)) = editor.workspace.body_at(lcx, lcy, sw, sh) {
		return Over::Ui(Layer::Panel(id));
	}
	if editor.workspace.over_ui(lcx, lcy, sw, sh) {
		return Over::Chrome;
	}
	Over::Map
}

/// Whether the pointer, resolved to `over`, is on the **workspace frame** — the
/// chrome the `Workspace` model paints, lights and puts a resize cursor over. A
/// panel rect covers its titlebar, so that is the panel layer plus the bare
/// chrome between panels; anything else (a cascade, a popup, a dialog, the map)
/// means the frame must be told the pointer left, since nothing dispatches to it
/// (U6.2).
///
/// **`popup` is not redundant with `over`.** A panel with an open dropdown is
/// press-modal, so [`over_at`] reports it for the *whole window* — as
/// `Over::Ui(Layer::Panel(id))`, the very same answer it gives when the pointer
/// is simply inside that panel's rect. The two cases are opposite (one means the
/// frame owns the point, the other that a list is floating over it) and `over`
/// alone cannot tell them apart, so the popup is passed in and settles it.
fn over_frame(over: Over, popup: Option<Layer>) -> bool {
	popup.is_none() && matches!(over, Over::Ui(Layer::Panel(_)) | Over::Chrome)
}

/// Whether `events` contain an Escape **press** — the one key the shell wants
/// back from a focused field (see rule 2 in [`App::route_keyboard`]).
fn escape_press(events: &[Event]) -> bool {
	events.iter().any(|e| matches!(e, Event::Key { key: wgpu_ui::Key::Escape, pressed: true, .. }))
}

/// Thumbnail cell side in the template atlas (the largest preview size).
const TEMPLATE_THUMB: u32 = 128;

/// The composed template-thumbnail atlas + the keys it was composed from.
struct TemplateAtlasState {
	tex: wgpu_ui::TextureId,
	cols: u32,
	rows: u32,
	/// Per-entry thumb size as a fraction of its cell (aspect-fit, top-left).
	fracs: Vec<(f32, f32)>,
	palette: Vec<u8>,
	/// The entry-list key ([`TemplateLibrary::set_entries`]'s stamp — no more
	/// re-joining every name into a `String` each frame); the thumb art is
	/// keyed by the palette + a `DocReplaced` invalidation (tile-art edits),
	/// like the tile atlas.
	revision: u64,
}

/// Recompose the template-thumbnail atlas when the template list or stored
/// palette changes (or it was invalidated by a `DocReplaced` tile-art edit) —
/// cheap when clean, and *not* recomposed by a map edit. Incompatible templates
/// (filtered out of the visible grid) get a blank cell.
fn refresh_template_atlas(editor: &EditorState, passes: &mut Passes) {
	let revision = editor.templates.revision();
	let stale =
		passes.template_atlas.as_ref().is_none_or(|a| a.palette != editor.project.palette || a.revision != revision);
	if !stale {
		return;
	}
	let Some(mc) = passes.menu_chrome.as_mut() else { return };
	let lut = tile_atlas::rest_lut(&editor.project.palette);
	let cols = 8u32;
	let rows = (editor.templates.entries.len().max(1) as u32).div_ceil(cols);
	let (aw, ah) = (cols * TEMPLATE_THUMB, rows * TEMPLATE_THUMB);
	let mut rgba = vec![0u8; (aw as usize) * (ah as usize) * 4];
	let mut fracs = Vec::with_capacity(editor.templates.entries.len());
	for (i, e) in editor.templates.entries.iter().enumerate() {
		if !e.template.compatible(&editor.project) {
			fracs.push((0.0, 0.0));
			continue;
		}
		let (thumb, tw, th) = template_preview::thumb(&editor.project, &e.template, &lut, TEMPLATE_THUMB);
		let (cx, cy) = ((i as u32 % cols) * TEMPLATE_THUMB, (i as u32 / cols) * TEMPLATE_THUMB);
		for y in 0..th as usize {
			let d = ((cy as usize + y) * aw as usize + cx as usize) * 4;
			let s = y * tw as usize * 4;
			rgba[d..d + tw as usize * 4].copy_from_slice(&thumb[s..s + tw as usize * 4]);
		}
		fracs.push((tw as f32 / TEMPLATE_THUMB as f32, th as f32 / TEMPLATE_THUMB as f32));
	}
	let tex = match passes.template_atlas.as_ref() {
		Some(a) => {
			mc.replace_texture(a.tex, &rgba, aw, ah);
			a.tex
		}
		None => mc.register_texture(&rgba, aw, ah),
	};
	passes.template_atlas =
		Some(TemplateAtlasState { tex, cols, rows, fracs, palette: editor.project.palette.clone(), revision });
}

/// The composed RGBA tile atlas the **Match editor** samples (the Tile Explorer
/// renders straight from the index atlas via `draw_picker`, so it no longer
/// needs this). A function of the packs' tile *art* + the stored palette only,
/// never the map: a tile-art edit returns `DocReplaced` (which sets this to
/// `None`), a palette edit is caught by the `palette` key. Only recomposed while
/// the match dialog is open (see [`refresh_tile_atlas`]).
struct TileAtlasState {
	tex: wgpu_ui::TextureId,
	count: u32,
	palette: Vec<u8>,
}

/// Recompose the RGBA tile atlas the **Match editor** samples — its only
/// consumer now that the Tile Explorer renders straight from the index atlas.
/// `needed` (the match-edit dialog is open) gates it: otherwise skipped, so
/// painting / tab switches / palette drags never pay the whole-project compose
/// (60–318 ms). When open, recomposes on a palette change (or a `DocReplaced`
/// invalidation); cheap when clean.
fn refresh_tile_atlas(editor: &EditorState, passes: &mut Passes, needed: bool) {
	if !needed {
		return;
	}
	let stale = passes.tile_atlas.as_ref().is_none_or(|a| a.palette != editor.project.palette);
	if !stale {
		return;
	}
	let Some(mc) = passes.menu_chrome.as_mut() else { return };
	let lut = tile_atlas::rest_lut(&editor.project.palette);
	let (rgba, w, h, count) = tile_atlas::compose(&editor.project, &lut);
	let tex = match passes.tile_atlas.as_ref() {
		Some(a) => {
			mc.replace_texture(a.tex, &rgba, w, h);
			a.tex
		}
		None => mc.register_texture(&rgba, w, h),
	};
	passes.tile_atlas = Some(TileAtlasState { tex, count, palette: editor.project.palette.clone() });
}

impl Passes {
	fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
		// The UI skin (brushed-steel sheet) is loaded once per device; a missing
		// asset falls back to flat gray inside `skin`.
		let steel = skin::load_steel(&resources_dir());
		let menu_chrome = uikit_menu::MenuChrome::new(device, queue, format, &steel);
		Self {
			blit: BlitPass::new(device, format),
			minimap: MinimapPass::new(),
			grid: grid::GridPass::new(device, format),
			crt: crt::CrtPass::new(device, format),
			scene: None,
			units: None,
			markers: None,
			scenery: None,
			menu_chrome,
			console_view: panel_ui::PanelHost::new(console_view::ConsoleView::new()),
			status: statusbar::StatusBar::new(),
			tabs_strip: panel_ui::PanelHost::new(tabs::TabStrip::new()),
			minimap_overlay: panel_ui::PanelHost::new(minimap::MinimapOverlay::new()),
			toolbox_content: panel_ui::PanelHost::new(toolbox::ToolboxContent::new()),
			savetools_content: panel_ui::PanelHost::new(savetools::SaveToolsContent::new()),
			passtools_content: panel_ui::PanelHost::new(passtools::PassToolsContent::new()),
			unitprops_content: panel_ui::PanelHost::new(unitprops::UnitPropsContent::new()),
			picker_content: panel_ui::PanelHost::new(picker::PickerContent::new()),
			units_content: panel_ui::PanelHost::new(units::UnitsContent::new()),
			scenery_content: panel_ui::PanelHost::new(scenery::SceneryContent::new()),
			templates_content: panel_ui::PanelHost::new(templates_panel::TemplatesContent::new()),
			tile_atlas: None,
			template_atlas: None,
			palette_content: panel_ui::PanelHost::new(palette_panel::PaletteContent::new()),
			wrlpalette_content: panel_ui::PanelHost::new(palette_panel::PaletteContent::new()),
			context_menu: panel_ui::PanelHost::new(wgpu_ui::ContextMenu::new(
				Vec::new(),
				wgpu_ui::layout::Spacer::new(),
			)),
			context_acts: Vec::new(),
			context_synced: None,
			format,
		}
	}

	/// Whether any hosted panel's tooltip is **arming** — a hover resting
	/// toward its delay, which is a redraw the event-driven frame loop cannot
	/// otherwise see (time passes, no event arrives). Asked uniformly of every
	/// host, like `set_viewport`, so a panel that grows a tooltip later
	/// inherits the rule; `redraw`'s tail schedules the next frame while any
	/// answers true, and the first *due* frame — requested by the last arming
	/// one — is the frame that draws the tip.
	/// Every hosted panel's `Ui`, in one place. The two questions asked of all of
	/// them - [`Self::tooltip_arming`] and [`Self::hovered_hint`] - read this
	/// list rather than each repeating it, so a new panel joins both by joining
	/// one array. (The console view and the tab strip are hosts too, though the
	/// workspace does not model them as docked panels.)
	fn panel_uis(&self) -> [&wgpu_ui::Ui; 13] {
		[
			&self.console_view.panel.ui,
			&self.tabs_strip.panel.ui,
			&self.minimap_overlay.panel.ui,
			&self.toolbox_content.panel.ui,
			&self.savetools_content.panel.ui,
			&self.passtools_content.panel.ui,
			&self.unitprops_content.panel.ui,
			&self.picker_content.panel.ui,
			&self.units_content.panel.ui,
			&self.scenery_content.panel.ui,
			&self.templates_content.panel.ui,
			&self.palette_content.panel.ui,
			&self.wrlpalette_content.panel.ui,
		]
	}

	fn tooltip_arming(&self) -> bool {
		self.panel_uis().iter().any(|ui| ui.tooltip_arming())
	}

	/// The tooltip text under the pointer in any hosted panel, the moment hover
	/// lands — no rest delay, unlike the floating plate. The status bar mirrors
	/// it into its hint slot while the hover holds, so a key's name is readable
	/// at once and without covering anything; the plate still arrives for the
	/// pointer-parked reader. Asked uniformly of every host, like
	/// `tooltip_arming`, so a panel that grows a tooltip later inherits the rule.
	/// Owned, not borrowed: the caller reads it beside a `&mut` borrow of a
	/// sibling field (the status bar itself).
	fn hovered_hint(&self) -> Option<String> {
		self.panel_uis().into_iter().find_map(|ui| ui.hovered_tooltip().map(str::to_string))
	}
}

/// The document opened when none is passed - the GREEN starter project,
/// resolved relative to `resources/` (see [`resources_dir`]).
fn default_map() -> PathBuf {
	resources_dir().join("assets/maps/GREEN_1.json")
}

struct Args {
	map: PathBuf,
	script: Vec<Command>,
	headless: bool,
	size: (u32, u32),
	/// `--settings PATH`: load/persist all settings from this file.
	settings: Option<PathBuf>,
	/// `--dev`: unlock editing shipped assets + the Bake menu.
	dev: bool,
}

/// Load an INI file, tolerating absence (`None`) but reporting the parse error
/// of a file that does exist.
fn read_ini(path: &Path) -> Option<INI> {
	match INI::from_file(path) {
		Ok(ini) => Some(ini),
		Err(e) => {
			if path.exists() {
				eprintln!("settings: {e}");
			}
			None
		}
	}
}

fn die(message: &str) -> ! {
	eprintln!("error: {message}");
	eprintln!();
	eprintln!("usage: max-map-editor [MAP.WRL] [options]");
	eprintln!();
	eprintln!("options:");
	eprintln!("  --script FILE       run commands from FILE (one per line, # comments)");
	eprintln!("  --screenshot OUT    shorthand: append 'screenshot OUT' and run headless");
	eprintln!("  --crop x,y,w,h      crop the --screenshot to a region (render-res px)");
	eprintln!("  --resize WxH        resize the --screenshot (nearest-neighbour) after cropping");
	eprintln!("  --headless          run the script without a window, then exit");
	eprintln!("  --size WxH          render-target size (default 1280x800)");
	eprintln!("  --settings FILE     load/persist all settings from FILE (an alternate mme.ini)");
	eprintln!("  --dev               developer mode: edit shipped tiles + enable Bake");
	std::process::exit(2);
}

fn parse_args() -> Args {
	let mut map = None;
	let mut script = Vec::new();
	let mut screenshot = None;
	let mut crop = None;
	let mut resize = None;
	let mut headless = false;
	let mut size = (1280u32, 800u32);
	let mut settings = None;
	let mut dev = false;

	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		match arg.as_str() {
			"--script" => {
				let Some(path) = args.next() else { die("--script needs a path") };
				let text = std::fs::read_to_string(&path).unwrap_or_else(|e| die(&format!("cannot read {path}: {e}")));
				script = command::parse_script(&text).unwrap_or_else(|e| die(&format!("{path}: {e}")));
			}
			"--screenshot" => {
				let Some(path) = args.next() else { die("--screenshot needs a path") };
				screenshot = Some(PathBuf::from(path));
			}
			"--crop" => {
				let Some(value) = args.next() else { die("--crop needs x,y,w,h") };
				crop = Some(command::parse_crop(&value).unwrap_or_else(|| die("--crop format is x,y,w,h")));
			}
			"--resize" => {
				let Some(value) = args.next() else { die("--resize needs WxH") };
				resize = Some(command::parse_dims(&value).unwrap_or_else(|| die("--resize format is WxH")));
			}
			"--headless" => headless = true,
			"--size" => {
				let Some(value) = args.next() else { die("--size needs WxH") };
				size = command::parse_dims(&value).unwrap_or_else(|| die("--size format is WxH"));
			}
			"--settings" => {
				let Some(path) = args.next() else { die("--settings needs a path") };
				settings = Some(PathBuf::from(path));
			}
			"--dev" => dev = true,
			"-h" | "--help" => die("help"),
			_ if map.is_none() => map = Some(PathBuf::from(arg)),
			_ => die(&format!("unknown argument: {arg}")),
		}
	}

	if let Some(path) = screenshot {
		script.push(Command::Screenshot { path, crop, resize });
		headless = true;
	}

	Args { map: map.unwrap_or_else(default_map), script, headless, size, settings, dev }
}

/// Locate `resources/`, in order: `./resources` (cargo-run from the
/// workspace root - cwd wins so a stray copy under `target/` can't shadow
/// the live data), exe-adjacent (the portable zip layout), or exe-relative
/// `../../../resources` (a `target/…` build launched from elsewhere).
fn resources_dir() -> PathBuf {
	let cwd = PathBuf::from("resources");
	if cwd.is_dir() {
		return cwd;
	}
	if let Some(beside_exe) = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join("resources"))) {
		if beside_exe.is_dir() {
			return beside_exe;
		}
	}
	std::env::current_exe()
		.ok()
		.and_then(|exe| Some(exe.parent()?.parent()?.parent()?.join("resources")))
		.filter(|p| p.is_dir())
		.unwrap_or(cwd)
}

/// Re-upload the active document's edited cell/pass sub-rectangles (drained
/// from the project's dirty region, so a stroke uploads only what it touched).
fn refresh_renderer(renderer: &ProjectRenderer, queue: &wgpu::Queue, editor: &mut EditorState) {
	let dirty = editor.project.take_render_dirty();
	renderer.update_cells(queue, &editor.project, &dirty);
}

/// Build the renderer matching the open document kind. `core` is the shared
/// device-lifetime pipeline set ([`project_render::RenderCore`]) — reusing it
/// is what keeps a tab switch from paying the ~16 ms pipeline rebuild.
fn make_renderer(
	device: &wgpu::Device,
	queue: &wgpu::Queue,
	editor: &EditorState,
	core: &Rc<project_render::RenderCore>,
) -> ProjectRenderer {
	ProjectRenderer::new(device, queue, &editor.project, core)
}

/// Whether this docked panel draws a content widget tree of its own.
///
/// A panel that says `false` gets the placeholder hint drawn in its body
/// instead - so a panel missing from this list renders empty however complete
/// its widget tree is. Named (rather than spelled inline in `render_frame`) so
/// the panel-routing drift test can hold it to the workspace's panel list.
fn panel_has_content(id: &str) -> bool {
	matches!(
		id,
		"tiles"
			| "minimap"
			| "palette"
			| "wrlpalette"
			| "toolbox"
			| "units" | "scenery"
			| "templates"
			| "savetools"
			| "passtools"
			| "unitprops"
	)
}

/// Push a fresh palette to every pass that samples one.
///
/// The live redraw and both screenshot paths (windowed and headless) all do
/// this, so the list of palette-consuming passes lives here and only here - a
/// new one is wired in once instead of in three places, where missing a copy
/// shows up only as a stale texture in a screenshot.
fn sync_palette(rgba: &[u8], renderer: &ProjectRenderer, passes: &Passes, queue: &wgpu::Queue) {
	renderer.update_palette(queue, rgba);
	if let Some(units) = &passes.units {
		units.update_palette(queue, rgba);
	}
	if let Some(markers) = &passes.markers {
		markers.update_palette(queue, rgba);
	}
	if let Some(scenery) = &passes.scenery {
		scenery.update_palette(queue, rgba);
	}
}

/// Everything an `Outcome::DocReplaced` invalidates: the document is a
/// different one, so the renderer is rebuilt against it, the minimap and both
/// composed atlases are dropped (tile art / the pack set may have changed -
/// only `DocReplaced` touches them), and the dirty-region bookkeeping resets
/// because the fresh renderer already holds everything.
///
/// Shared by the windowed and headless paths for the same reason as
/// [`sync_palette`]: it was two copies, and they have to agree.
fn adopt_new_document(
	renderer: &mut ProjectRenderer,
	passes: &mut Passes,
	editor: &mut EditorState,
	uploaded_revision: &mut u64,
	device: &wgpu::Device,
	queue: &wgpu::Queue,
	core: &Rc<project_render::RenderCore>,
) {
	*renderer = make_renderer(device, queue, editor, core);
	passes.minimap.invalidate();
	passes.tile_atlas = None;
	passes.template_atlas = None;
	editor.project.clear_render_dirty();
	*uploaded_revision = editor.revision();
}

/// A map cell's rect in screen px for the current view (pan in world px,
/// then zoom) - the same math the unit previews use.
fn map_cell_rect(editor: &EditorState, x: u16, y: u16) -> ui::Rect {
	let zoom = editor.view.zoom;
	let ts = render::TILE_PX as f32;
	ui::Rect::new(
		(x as f32 * ts - editor.view.pan[0]) * zoom,
		(y as f32 * ts - editor.view.pan[1]) * zoom,
		ts * zoom,
		ts * zoom,
	)
}

/// The armed tile/stamp rendered into the 8 orientation-preview cells: a quad
/// per **enabled** cell (a single tile → one quad at that orientation; a stamp →
/// its cached oriented template scaled to fit, one quad per cell layer).
/// Disabled (greyed) orientations render nothing. Index-atlas quads for
/// `project_render::draw_picker`.
fn toolbox_preview_quads(editor: &EditorState, cells: &[ui::Rect; 8]) -> Vec<picker::TileQuad> {
	let mut quads = Vec::new();
	let project = &editor.project;
	if editor.stamp_base.is_some() {
		// A stamp: its cached oriented template scaled (aspect-fit) into each cell.
		for (i, cell) in cells.iter().enumerate() {
			let Some(tpl) = &editor.stamp_orients[i] else { continue };
			let (tw, th) = (tpl.width.max(1) as f32, tpl.height.max(1) as f32);
			let inset = ui::Rect::new(cell.x + 1.0, cell.y + 1.0, cell.w - 2.0, cell.h - 2.0);
			let s = (inset.w / tw).min(inset.h / th);
			let (ox, oy) = (inset.x + (inset.w - tw * s) / 2.0, inset.y + (inset.h - th * s) / 2.0);
			for cy in 0..tpl.height {
				for cx in 0..tpl.width {
					let spec = &tpl.cells[cy as usize * tpl.width as usize + cx as usize];
					let rect = ui::Rect::new(ox + cx as f32 * s, oy + cy as f32 * s, s, s);
					// Bottom-up (water then ground) so a masked ground shows water.
					for part in spec.split(',').filter(|p| !p.is_empty()) {
						if let Ok((tref, _)) = project.resolve_ref(part) {
							let index = picker::global_index(project, tref);
							quads.push(picker::TileQuad { index, transform: tref.transform.bits(), rect });
						}
					}
				}
			}
		}
	} else if let Some(spec) = editor.active_tile() {
		// A single tile: the same tile at each allowed orientation.
		if let Ok((tile, _)) = project.resolve_ref(spec) {
			let index = picker::global_index(project, tile);
			for (i, cell) in cells.iter().enumerate() {
				let t = toolbox::orient_transform(i);
				if !editor.orient_allowed(t) {
					continue;
				}
				let inset = ui::Rect::new(cell.x + 1.0, cell.y + 1.0, cell.w - 2.0, cell.h - 2.0);
				quads.push(picker::TileQuad { index, transform: t.bits(), rect: inset });
			}
		}
	}
	quads
}

/// The selection's thick outline (every selected-region boundary edge,
/// viewport-culled) plus the live rect-drag preview.
fn selection_overlay(editor: &EditorState, w: f32, h: f32) -> DrawList {
	let mut dl = DrawList::new();
	let zoom = editor.view.zoom;
	let ts = render::TILE_PX as f32;
	if !editor.selection.is_empty() && zoom > 0.0 {
		// The visible cell window - the boundary walk never touches
		// off-screen cells, however large the map or selection.
		let (mw, mh) = (editor.project.width, editor.project.height);
		let x0 = (editor.view.pan[0] / ts).floor().max(0.0) as u16;
		let y0 = (editor.view.pan[1] / ts).floor().max(0.0) as u16;
		let x1 = (((editor.view.pan[0] + w / zoom) / ts).ceil().max(0.0) as u16).min(mw.saturating_sub(1));
		let y1 = (((editor.view.pan[1] + h / zoom) / ts).ceil().max(0.0) as u16).min(mh.saturating_sub(1));
		if x0 <= x1 && y0 <= y1 {
			const T: f32 = 2.0; // outline thickness (screen px)
			for (cx, cy, edge) in editor.selection.boundary_edges(x0, y0, x1, y1) {
				let r = map_cell_rect(editor, cx, cy);
				// Segments overhang 1px past the corners so the outline
				// reads as one continuous band around each region.
				let seg = match edge {
					map_core::Edge::Top => ui::Rect::new(r.x - 1.0, r.y - 1.0, r.w + 2.0, T),
					map_core::Edge::Bottom => ui::Rect::new(r.x - 1.0, r.y + r.h - 1.0, r.w + 2.0, T),
					map_core::Edge::Left => ui::Rect::new(r.x - 1.0, r.y - 1.0, T, r.h + 2.0),
					map_core::Edge::Right => ui::Rect::new(r.x + r.w - 1.0, r.y - 1.0, T, r.h + 2.0),
				};
				dl.fill_rect(seg, rgba(theme::ACCENT));
			}
		}
	}
	// The rect tool's live drag: a hairline preview of the intended span.
	if let Some((ax, ay, bx, by)) = editor.select_preview {
		let a = map_cell_rect(editor, ax.min(bx), ay.min(by));
		let b = map_cell_rect(editor, ax.max(bx), ay.max(by));
		dl.stroke_rect(ui::Rect::new(a.x, a.y, b.x + b.w - a.x, b.y + b.h - a.y), 1.0, rgba(theme::ACCENT));
	}
	dl
}

/// A `color` 2-px box around each of `cells`, viewport-culled (so a map-wide set
/// never costs more than the on-screen cells). The shared primitive behind the
/// Fix Shore defect outlines and the Show-Shore-Bugs / Show-Problems overlays.
fn cell_ring_overlay(editor: &EditorState, cells: &[(u16, u16)], color: [f32; 4], w: f32, h: f32) -> DrawList {
	let mut dl = DrawList::new();
	let zoom = editor.view.zoom;
	if cells.is_empty() || zoom <= 0.0 {
		return dl;
	}
	let ts = render::TILE_PX as f32;
	let (mw, mh) = (editor.project.width, editor.project.height);
	let x0 = (editor.view.pan[0] / ts).floor().max(0.0) as u16;
	let y0 = (editor.view.pan[1] / ts).floor().max(0.0) as u16;
	let x1 = (((editor.view.pan[0] + w / zoom) / ts).ceil().max(0.0) as u16).min(mw.saturating_sub(1));
	let y1 = (((editor.view.pan[1] + h / zoom) / ts).ceil().max(0.0) as u16).min(mh.saturating_sub(1));
	const T: f32 = 2.0; // outline thickness (screen px)
	for &(cx, cy) in cells {
		if cx < x0 || cx > x1 || cy < y0 || cy > y1 {
			continue;
		}
		let r = map_cell_rect(editor, cx, cy);
		// Four fills overhanging the corners by 1px so the box reads as one band.
		dl.fill_rect(ui::Rect::new(r.x - 1.0, r.y - 1.0, r.w + 2.0, T), rgba(color));
		dl.fill_rect(ui::Rect::new(r.x - 1.0, r.y + r.h - 1.0, r.w + 2.0, T), rgba(color));
		dl.fill_rect(ui::Rect::new(r.x - 1.0, r.y - 1.0, T, r.h + 2.0), rgba(color));
		dl.fill_rect(ui::Rect::new(r.x + r.w - 1.0, r.y - 1.0, T, r.h + 2.0), rgba(color));
	}
	dl
}

/// A `T`-px band around `r`, its corners overhanging so the four fills read as
/// one continuous frame. Grown outward from the rect, so a thicker band never
/// eats into the sprite it surrounds.
fn frame_rect(dl: &mut DrawList, r: ui::Rect, t: f32, color: [f32; 4]) {
	let (o, span) = (t * 0.5, t); // half outside, half over the footprint edge
	dl.fill_rect(ui::Rect::new(r.x - o, r.y - o, r.w + span, t), rgba(color));
	dl.fill_rect(ui::Rect::new(r.x - o, r.y + r.h - o, r.w + span, t), rgba(color));
	dl.fill_rect(ui::Rect::new(r.x - o, r.y - o, t, r.h + span), rgba(color));
	dl.fill_rect(ui::Rect::new(r.x + r.w - o, r.y - o, t, r.h + span), rgba(color));
}

/// A frame around **every** placed object's whole footprint — one box for a 2×2,
/// not a per-cell grid — drawn in its owner team's colour. The frames say at a
/// glance what is placed and whose it is, on terrain the sprites can otherwise
/// disappear into: a hairline `FRAME_T` band for every object, and a `SEL_T` one
/// for the selected object, which has to read as picked from across the map.
///
/// Viewport-culled, so a save with hundreds of objects costs no more than the
/// on-screen ones. The unselected frames follow **View ▸ Overlays ▸ Units**
/// (framing a sprite that isn't drawn would be pointing at nothing); the
/// selection's own frame does not — it is selection chrome, and the Unit
/// Properties panel edits that object whether or not the sprites are shown.
fn object_frames(editor: &EditorState, w: f32, h: f32) -> DrawList {
	/// Every placed object's hairline; the selected one's band.
	const FRAME_T: f32 = 1.0;
	const SEL_T: f32 = 3.0;

	let mut dl = DrawList::new();
	let zoom = editor.view.zoom;
	if zoom <= 0.0 {
		return dl;
	}
	// The footprint's screen rect, or `None` when it is entirely off-screen.
	let box_of = |i: usize| -> Option<(ui::Rect, [f32; 4])> {
		let o = editor.project.objects.get(i)?;
		let f = editor.object_footprint_of(i) as f32;
		let tl = map_cell_rect(editor, o.x, o.y);
		let side = render::TILE_PX as f32 * zoom * f;
		let r = ui::Rect::new(tl.x, tl.y, side, side);
		if r.x + r.w < 0.0 || r.y + r.h < 0.0 || r.x > w || r.y > h {
			return None; // fully off-screen
		}
		Some((r, crate::units::TEAM_SWATCH.get(o.team as usize).copied().unwrap_or(theme::ACCENT)))
	};

	if editor.show_units {
		for i in 0..editor.project.objects.len() {
			if Some(i) == editor.selected_object {
				continue; // its own band is drawn last, over any neighbour's hairline
			}
			if let Some((r, color)) = box_of(i) {
				frame_rect(&mut dl, r, FRAME_T, color);
			}
		}
	}
	if let Some(idx) = editor.selected_object
		&& let Some((r, color)) = box_of(idx)
	{
		frame_rect(&mut dl, r, SEL_T, color);
	}
	dl
}

/// A red box around every coast cell the Fix Shore tool currently judges broken
/// (against `tiles.match.json`). Drawn whenever the Fix Shore modal is open - the
/// outlines update live as a run clears defects. Empty otherwise.
fn shore_defect_overlay(editor: &EditorState, w: f32, h: f32) -> DrawList {
	if !editor.autofix_open() {
		return DrawList::new();
	}
	cell_ring_overlay(editor, &editor.autofix_defects, theme::DEFECT, w, h)
}

/// The colour the game's survey marker uses for each material's dial — raw is
/// green, gold yellow, fuel white (sampled from `RAWMSK`/`GOLDMK`/`FUELMK`). The
/// flat-tint fallback overlay uses these so it speaks the same colour language as
/// the sprite markers.
fn resource_rgb(material: max_assets::save::CargoMaterial) -> [f32; 3] {
	use max_assets::save::CargoMaterial::*;
	match material {
		Raw => [0.122, 0.800, 0.243],
		Fuel => [0.965, 0.973, 0.976],
		Gold => [0.988, 0.988, 0.0],
	}
}

/// Resource-distribution overlay (View ▸ Resources, S5): a filled tint per
/// surveyed cargo cell, coloured by material with opacity scaled by the amount
/// (0-31). Viewport-culled like the other cell overlays; empty when no save is
/// open (the cargo map is then empty).
fn resource_overlay(editor: &EditorState, w: f32, h: f32) -> DrawList {
	let mut dl = DrawList::new();
	let zoom = editor.view.zoom;
	let cargo = editor.project.cargo_map();
	if cargo.is_empty() || zoom <= 0.0 {
		return dl;
	}
	let ts = render::TILE_PX as f32;
	let (mw, mh) = (editor.project.width, editor.project.height);
	let x0 = (editor.view.pan[0] / ts).floor().max(0.0) as u16;
	let y0 = (editor.view.pan[1] / ts).floor().max(0.0) as u16;
	let x1 = (((editor.view.pan[0] + w / zoom) / ts).ceil().max(0.0) as u16).min(mw.saturating_sub(1));
	let y1 = (((editor.view.pan[1] + h / zoom) / ts).ceil().max(0.0) as u16).min(mh.saturating_sub(1));
	for cy in y0..=y1 {
		for cx in x0..=x1 {
			let value = cargo[cy as usize * mw as usize + cx as usize];
			let Some(material) = max_assets::save::cargo_material(value) else { continue };
			let amount = max_assets::save::cargo_amount(value);
			let [r, g, b] = resource_rgb(material);
			// Even an amount-0 cell reads faintly; a full 31 tints strongly.
			let alpha = 0.20 + (amount as f32 / 31.0) * 0.55;
			dl.fill_rect(map_cell_rect(editor, cx, cy), rgba([r, g, b, alpha]));
		}
	}
	dl
}

/// A green outline around the brush footprint under the cursor (pencil/eraser
/// in Map mode, multi-cell brush only) so a wide brush shows where it lands.
/// `None` when not applicable, or the cursor is off the map / over UI chrome.
fn brush_overlay(editor: &EditorState, popup: Option<Layer>) -> Option<DrawList> {
	if editor.brush_size <= 1 || editor.mode != state::EditorMode::Map {
		return None;
	}
	if !matches!(editor.tool, state::Tool::Pencil | state::Tool::Eraser | state::Tool::PaintMask) {
		return None;
	}
	// `hot.cursor` is logical (UI space): gate against the logical UI rects with
	// the shell's one hit test, but read the map cell in physical screen px (the
	// map renders native, so `cell_at` expects physical = logical × scale).
	let (cx, cy) = editor.cursor?;
	if !over_at(editor, popup, cx, cy).is_map() {
		return None;
	}
	let (x, y) = editor.cell_at(cx * editor.ui_scale, cy * editor.ui_scale)?;
	// Tint each footprint cell so the brush shape (square or circle) reads.
	let tint = [theme::ACCENT[0], theme::ACCENT[1], theme::ACCENT[2], 0.22];
	let mut dl = DrawList::new();
	for (bx, by) in editor.brush_cells(x, y) {
		dl.fill_rect(map_cell_rect(editor, bx, by), rgba(tint));
	}
	Some(dl)
}

/// The armed ghost stamp's tile quads at the cell under the cursor, plus
/// its footprint rect - `None` when nothing is armed, the cursor is off
/// the map, or it hovers UI chrome (panels, menu, tabs).
fn ghost_quads(
	editor: &EditorState,
	popup: Option<Layer>,
	w: f32,
	h: f32,
) -> Option<(ui::Rect, Vec<picker::TileQuad>)> {
	let stamp = editor.stamp.as_ref()?;
	// See `brush_overlay`: logical cursor for the UI gate, physical for `cell_at`.
	let (cx, cy) = editor.cursor?;
	if !over_at(editor, popup, cx, cy).is_map() {
		return None;
	}
	// The footprint is centred on the hovered cell, exactly as `Command::Stamp`
	// places it.
	let (hx, hy) = editor.cell_at(cx * editor.ui_scale, cy * editor.ui_scale)?;
	let (ox, oy) = state::stamp_origin(stamp, hx, hy);
	let mut entries = stamp.resolve(&editor.project).ok()?;
	// Water under ground, exactly like the map composes.
	entries.sort_by_key(|&(.., layer, _)| layer);
	let quads: Vec<picker::TileQuad> = entries
		.into_iter()
		.filter_map(|(dx, dy, _, tile)| {
			let (x, y) = (ox.checked_add(dx)?, oy.checked_add(dy)?);
			(x < editor.project.width && y < editor.project.height).then(|| picker::TileQuad {
				index: picker::global_index(&editor.project, tile),
				transform: tile.transform.bits(),
				rect: map_cell_rect(editor, x, y),
			})
		})
		.collect();
	let a = map_cell_rect(editor, ox, oy);
	let fw = (stamp.width as f32 * a.w).min(w * 4.0);
	let fh = (stamp.height as f32 * a.h).min(h * 4.0);
	Some((ui::Rect::new(a.x, a.y, fw, fh), quads))
}

/// The paint tool's tile ghost: the active tile (its transform applied) shown
/// half-transparent over the cells a click would place it on, so you preview a
/// stroke before committing. `Pencil` previews the whole brush footprint;
/// `Fill` previews the single hovered cell (the flood extent is unknown). `None`
/// unless a paint tool is active with a resolved tile and the cursor is over the
/// map (not UI); never while a stamp is armed - its own ghost takes over.
fn paint_ghost_quads(editor: &EditorState, popup: Option<Layer>) -> Option<Vec<picker::TileQuad>> {
	if editor.mode != state::EditorMode::Map || editor.stamp.is_some() {
		return None;
	}
	let footprint = match editor.tool {
		state::Tool::Pencil => true,
		state::Tool::Fill => false,
		_ => return None,
	};
	let (tile, _) = editor.project.resolve_ref(editor.active_tile()?).ok()?;
	// See `brush_overlay`: logical cursor for the UI gate, physical for `cell_at`.
	let (cx, cy) = editor.cursor?;
	if !over_at(editor, popup, cx, cy).is_map() {
		return None;
	}
	let (x, y) = editor.cell_at(cx * editor.ui_scale, cy * editor.ui_scale)?;
	let index = picker::global_index(&editor.project, tile);
	let cells = if footprint { editor.brush_cells(x, y) } else { vec![(x, y)] };
	let quads = cells
		.into_iter()
		.map(|(bx, by)| picker::TileQuad {
			index,
			transform: tile.transform.bits(),
			rect: map_cell_rect(editor, bx, by),
		})
		.collect();
	Some(quads)
}

/// What the placement ghost would preview: `(unit_type, x, y)` for the active
/// unit on the cell under the cursor, gated exactly like the tool's click — the
/// Place tool armed, a unit selected, and the cursor over the map (not UI / a
/// context menu / while a template stamp owns its own ghost). `None` when no
/// ghost should show, or the selected unit isn't a placeable type. Split from the
/// quad build so the gating is unit-testable without a GPU atlas.
fn unit_ghost_placement(editor: &EditorState, popup: Option<Layer>) -> Option<(u16, u16, u16)> {
	if editor.mode != state::EditorMode::Map || editor.tool != state::Tool::Unit || editor.stamp.is_some() {
		return None;
	}
	let active = editor.active_unit?;
	// See `brush_overlay`: logical cursor for the UI gate, physical for `cell_at`.
	let (cx, cy) = editor.cursor?;
	if !over_at(editor, popup, cx, cy).is_map() {
		return None;
	}
	let (x, y) = editor.cell_at(cx * editor.ui_scale, cy * editor.ui_scale)?;
	let unit_type = max_assets::save::unit_type_id(&editor.units.as_ref()?.units.get(active)?.tag)?;
	Some((unit_type, x, y))
}

/// The placement tool's unit ghost: the active unit (body + struts + turret, no
/// shadow) composited on the cell under the cursor, for the units GPU pass to
/// draw half-transparent — the unit analogue of the template-stamp ghost. Empty
/// unless [`unit_ghost_placement`] resolves a cell + type, or the type has no
/// sprite in `lib`.
fn unit_ghost_quads(
	editor: &EditorState,
	lib: &units::UnitLibrary,
	slots: &units_render::AtlasSlots,
	popup: Option<Layer>,
) -> Vec<units::UnitQuad> {
	let Some((unit_type, x, y)) = unit_ghost_placement(editor, popup) else { return Vec::new() };
	let obj = map_core::MapObject { unit_type, x, y, team: editor.unit_team, props: map_core::ObjectProps::default() };
	// Reuse the map object layout, then drop the shadow (a translucent shadow
	// reads as grime under the ghost, not a preview).
	units::object_quads(std::slice::from_ref(&obj), lib, slots, editor.view.pan, editor.view.zoom)
		.into_iter()
		.filter(|q| !q.shadow)
		.collect()
}

/// The place tool's scenery ghost: the armed cut-out anchored where a click
/// would drop it. `None` unless the map tool is armed with a resolvable piece
/// and the cursor is over the map rather than a panel or an open popup - the
/// same gate [`unit_ghost_placement`] applies.
fn scenery_ghost(
	editor: &EditorState,
	gpu: &scenery_render::SceneryGpu,
	popup: Option<Layer>,
) -> Option<scenery_render::SceneryQuad> {
	let active = scenery_ghost_armed(editor)?;
	// See `brush_overlay`: logical cursor for the UI gate, physical for the map.
	let (cx, cy) = editor.cursor?;
	if !over_at(editor, popup, cx, cy).is_map() {
		return None;
	}
	let (px, py) = editor.world_at(cx * editor.ui_scale, cy * editor.ui_scale);
	scenery_render::ghost_quad(
		&editor.project,
		gpu,
		active,
		px,
		py,
		editor.view.pan,
		editor.view.zoom,
		editor.scenery_blend,
	)
}

/// The piece [`scenery_ghost`] would preview, ignoring where the cursor is: the
/// place tool armed with a resolvable index, no stamp in the way. Split out
/// because the `CursorMoved` redraw gate needs the same answer without a GPU -
/// the scenery ghost is anchored in map **pixels**, so unlike every other hover
/// preview it has to redraw on sub-cell moves or it visibly snaps to the grid.
fn scenery_ghost_armed(editor: &EditorState) -> Option<usize> {
	if editor.mode != state::EditorMode::Map || editor.tool != state::Tool::Scenery || editor.stamp.is_some() {
		return None;
	}
	editor.active_scenery
}

/// Compose one full frame: map, workspace background, each panel's chrome +
/// content **in z-order**, dock peeks, menu bar, modal, console overlay.
/// Shared by the live window and the screenshot path, so captures are
/// always faithful.
fn render_frame(
	device: &wgpu::Device,
	queue: &wgpu::Queue,
	encoder: &mut wgpu::CommandEncoder,
	target: &wgpu::TextureView,
	editor: &mut EditorState,
	renderer: &ProjectRenderer,
	passes: &mut Passes,
) {
	// Recompute the problem-overlay cell sets (Show Shore Bugs / Show Problems)
	// if their toggle is on and the map changed - before the immutable draws.
	editor.refresh_problem_overlays();
	// Rebuild the Edit ▸ Undo History submenu when the undo stack changed.
	editor.sync_undo_history();
	let (w, h) = editor.screen;
	let (pw, ph) = (w as f32, h as f32); // physical framebuffer: the map scene + GPU scissors
	// UI scale: the chrome + fonts lay out in **logical** px (physical / scale) and
	// the projection scales them up to fill the framebuffer; the map itself stays
	// native. At scale 1.0, logical == physical and every draw below is unchanged.
	let scale = editor.ui_scale;
	let (wf, hf) = editor.ui_screen(); // logical UI size (chrome + panels + modals)
	// CRT: when on, render the whole frame into an offscreen scene (sized
	// to the viewport) and post-process it onto `target` at the end; otherwise
	// draw straight to `target`.
	// The panel holding an open dropdown, if any: press-modal, so the map-side
	// gates below (ghosts, brush outline, the cell readout) treat it as covering
	// the window — the same answer the press path gets (U3.2).
	let popup = popup_layer(passes, editor);
	// Every hosted panel places its popups against the **window**, not its own
	// body, so a dropdown near a panel's bottom edge flips up at the screen edge
	// like a dialog's rather than at the panel's (U3.2). Set here, before the
	// panel loop, because from there on `target` holds a borrow of `passes`.
	for panel in &editor.workspace.panels {
		if let Some(host) = panel_host(passes, &panel.id) {
			host.set_viewport(ui::Rect::new(0.0, 0.0, wf, hf));
		}
	}
	// CRT: when on, render the whole frame into an offscreen scene (sized
	// to the viewport) and post-process it onto `target` at the end; otherwise
	// draw straight to `target`.
	let crt_on = editor.crt;
	if crt_on && passes.scene.as_ref().map(|s| s.size) != Some((w, h)) {
		passes.scene = Some(passes.crt.make_target(device, (w, h)));
	}
	let final_target = target;
	let target: &wgpu::TextureView =
		if crt_on { &passes.scene.as_ref().expect("scene built when crt_on").view } else { final_target };
	// App background: the raw steel sheet stretched across the viewport,
	// drawn first (covering every pixel) so the map's out-of-bounds fragments -
	// which now discard - reveal it instead of a flat void colour. Dimmed to
	// 50% exposure so it recedes behind the map and the chrome (panels, windows,
	// and modals draw their own opaque steel on top, so they stay full bright).
	if let Some(mc) = passes.menu_chrome.as_mut() {
		// The whole map-space chrome composites at scale 1.0 (it rides the native
		// map): the steel grain samples the full physical viewport (origin plate).
		mc.prepare((w, h), 1.0);
		let mut dl = DrawList::new();
		mc.theme().steel_fill(&mut dl, ui::Rect::new(0.0, 0.0, pw, ph), [0.5, 0.5, 0.5, 1.0]);
		mc.render_list(encoder, target, (w, h), &dl);
	}
	renderer.draw(queue, encoder, target, editor.uniforms(0), editor.show_pass_overlay, editor.layer_mask());
	// Scenery stands on the terrain it was cut from, so it draws straight after
	// the map - under the grid, under the units. The pass rebuilds whenever the
	// open project's libraries change (a project on another tile pack).
	if !editor.project.scenery_packs.is_empty() {
		let want = scenery_render::signature(&editor.project.scenery_packs);
		if passes.scenery.as_ref().is_none_or(|g| g.signature() != want) {
			passes.scenery = scenery_render::SceneryGpu::new(
				device,
				queue,
				&editor.project.scenery_packs,
				passes.format,
				editor.cycler.rgba(),
			);
		}
		if let Some(sgpu) = passes.scenery.as_mut() {
			let mut quads = scenery_render::map_quads(&editor.project, sgpu, editor.view.pan, editor.view.zoom);
			// The place tool's ghost: the armed cut-out under the cursor, drawn
			// half-transparent so you see where a click would drop it - the
			// scenery analogue of the template-stamp and unit ghosts. It goes in
			// the same call as the placements, so its shadow merges with theirs
			// instead of doubling over one it crosses.
			let ghost = scenery_ghost(editor, sgpu, popup);
			let has_ghost = ghost.is_some();
			quads.extend(ghost);
			sgpu.draw(device, encoder, target, &quads, has_ghost, None, (w, h), 1.0);
		}
	}
	// Grid overlay sits on the map, beneath the panels.
	if editor.show_grid {
		passes.grid.draw(queue, encoder, target, editor.uniforms(0), grid::GRID_STRENGTH);
	}
	// Unit previews (palette aid) stand on the terrain, above the grid.
	if let Some(lib) = &editor.units {
		if passes.units.is_none() {
			passes.units = Some(units_render::UnitsGpu::new(device, queue, lib, passes.format, editor.cycler.rgba()));
		}
		if editor.show_units {
			let ugpu = passes.units.as_ref().expect("units pass built above");
			// One unified object list: preview annotations on an ordinary map, or
			// the opened save's units / slabs / rubble (seeded on open, S2.1).
			let quads =
				units::object_quads(&editor.project.objects, lib, &ugpu.slots, editor.view.pan, editor.view.zoom);
			ugpu.draw(device, encoder, target, &quads, None, (w, h), 1.0, false);
		}
		// The placement tool's unit ghost under the cursor: the active unit
		// half-transparent on the hovered cell, so you preview what a click/drag
		// will place — the unit analogue of the template-stamp ghost (item 3).
		if let Some(ugpu) = passes.units.as_ref() {
			let ghost = unit_ghost_quads(editor, lib, &ugpu.slots, popup);
			if !ghost.is_empty() {
				ugpu.draw(device, encoder, target, &ghost, None, (w, h), 1.0, true);
			}
		}
	}
	// Selection chrome rides on the map, beneath the panels: the thick
	// outline around selected regions and a live rect-drag preview. These ride
	// the physical map (they're positioned via `editor.view`), so they project
	// at native size (`pw`/`ph`), not the logical UI size.
	if let Some(mc) = passes.menu_chrome.as_mut() {
		mc.prepare((w, h), 1.0);
		// Resource-distribution tint (View ▸ Resources, S5): each surveyed cargo
		// cell coloured by material, beneath the selection / object chrome. Only
		// the fallback for when the sprite markers can't load (no MaxPath); with
		// the marker library present the sprite pass above draws instead.
		if editor.show_resources && editor.markers.is_none() {
			let ov = resource_overlay(editor, pw, ph);
			if !ov.cmds.is_empty() {
				mc.render_list(encoder, target, (w, h), &ov);
			}
		}
		let sel_overlay = selection_overlay(editor, pw, ph);
		if !sel_overlay.cmds.is_empty() {
			mc.render_list(encoder, target, (w, h), &sel_overlay);
		}
		// Red boxes around the coast cells Fix Shore still judges broken.
		let defect_overlay = shore_defect_overlay(editor, pw, ph);
		if !defect_overlay.cmds.is_empty() {
			mc.render_list(encoder, target, (w, h), &defect_overlay);
		}
		// Show Shore Bugs: broken / missing shore outlined in red (view toggle).
		if editor.show_shore_bugs {
			let ov = cell_ring_overlay(editor, &editor.shore_bug_cells, theme::DEFECT, pw, ph);
			if !ov.cmds.is_empty() {
				mc.render_list(encoder, target, (w, h), &ov);
			}
		}
		// Show Problems: every tile that violates its match rules, in red.
		if editor.show_match_problems {
			let ov = cell_ring_overlay(editor, &editor.match_problem_cells, theme::DEFECT, pw, ph);
			if !ov.cmds.is_empty() {
				mc.render_list(encoder, target, (w, h), &ov);
			}
		}
		// Brush footprint outline (wide pencil/eraser).
		if let Some(bo) = brush_overlay(editor, popup) {
			mc.render_list(encoder, target, (w, h), &bo);
		}
		// Every placed object's footprint outlined in its team's colour — a
		// hairline each, a 3 px band on the picked one (S2.3).
		let frames = object_frames(editor, pw, ph);
		if !frames.cmds.is_empty() {
			mc.render_list(encoder, target, (w, h), &frames);
		}
	}
	// The armed ghost stamp under the cursor: half-transparent tiles snapped
	// to the cell grid, framed so the footprint reads (hidden over UI).
	if let Some((origin, quads)) = ghost_quads(editor, popup, pw, ph) {
		renderer.draw_picker(device, encoder, target, &quads, ui::Rect::new(0.0, 0.0, pw, ph), (w, h), 1.0, 0.55);
		if let Some(mc) = passes.menu_chrome.as_mut() {
			mc.prepare((w, h), 1.0);
			let mut dl = DrawList::new();
			dl.stroke_rect(origin, 1.0, rgba(theme::ACCENT));
			mc.render_list(encoder, target, (w, h), &dl);
		}
	}
	// The paint tool's tile ghost under the cursor (mutually exclusive with the
	// stamp ghost above: `paint_ghost_quads` bails while a stamp is armed).
	if let Some(quads) = paint_ghost_quads(editor, popup) {
		renderer.draw_picker(device, encoder, target, &quads, ui::Rect::new(0.0, 0.0, pw, ph), (w, h), 1.0, 0.55);
	}
	// Resource-marker sprites (View ▸ Resources): the game's RAW/FUEL/GOLD markers
	// on each surveyed cargo cell. Drawn last of the map-space passes — above the
	// units and every map overlay — so a unit or selection ring never hides a
	// resource dial; still beneath the panels (they're drawn after this). Built
	// lazily once the marker library loads; when it can't (no MaxPath) the
	// flat-tint fallback above draws instead.
	if editor.show_resources {
		if let Some(lib) = &editor.markers {
			if passes.markers.is_none() {
				passes.markers =
					Some(markers_render::MarkersGpu::new(device, queue, lib, passes.format, editor.cycler.rgba()));
			}
			let mgpu = passes.markers.as_ref().expect("markers pass built above");
			let quads = markers::marker_quads(
				editor.project.cargo_map(),
				(editor.project.width, editor.project.height),
				lib,
				&mgpu.slots,
				editor.view.pan,
				editor.view.zoom,
				(pw, ph),
			);
			mgpu.draw(device, encoder, target, &quads, (w, h));
		}
	}
	// Dock splitters/edges and drop-target peeks: flat chrome through the wgpu-ui
	// renderer (origin plate). Peeks sit on the map, *below* the windows (a docked
	// panel that stays put must stay readable while another drags near its edge).
	if let Some(mc) = passes.menu_chrome.as_mut() {
		mc.prepare((w, h), scale);
		mc.render_list(encoder, target, (w, h), &editor.workspace.draw_background(wf, hf));
		mc.render_list(encoder, target, (w, h), &editor.workspace.draw_peeks(wf, hf));
	}
	// Every panel's *overlay* pass — its open dropdown — collected across the
	// loop and composited after it (U3.2). A panel's base chrome belongs at its
	// own depth, clipped to its body; a popup belongs above every panel, on the
	// origin steel plate like the menu cascade.
	let mut panel_popups = DrawList::new();
	for (i, r) in editor.workspace.layout(wf, hf).panels {
		let id = editor.workspace.panels[i].id.as_str();
		let has_content = panel_has_content(id);
		// A floating panel's chrome AND content share one anchored steel crop;
		// docked panels share the stretched viewport sheet. The chrome draws
		// through the steel theme on the *same* mapping (`prepare_panel`).
		let map = editor.workspace.steel_map(i, r);
		if let Some(mc) = passes.menu_chrome.as_mut() {
			mc.prepare_panel((w, h), scale, map);
			let chrome = editor.workspace.draw_panel(mc.theme(), mc.fonts(), i, r, !has_content);
			mc.render_list(encoder, target, (w, h), &chrome);
		}
		let body = editor.workspace.body_of(i, r);
		if id == "toolbox" {
			// The Tile Editing Toolbox is a real widget tree (U5.4): a `ScrollArea`
			// over a flow of the preview content widget and the eight group blocks,
			// which clip and scroll themselves. `sync` only pushes per-frame state
			// (lit keys, dropdown values, the preview) - hover and press are each
			// widget's own, so no pointer state reaches it.
			if let Some(mc) = passes.menu_chrome.as_mut() {
				mc.prepare_panel((w, h), scale, map);
				if let Some(c) = passes.toolbox_content.root_mut() {
					c.sync(toolbox::Snapshot::of(editor));
				}
				let mut dl = DrawList::new();
				passes.toolbox_content.build(mc, body, scale, &[], &mut dl, &mut panel_popups);
				mc.render_list(encoder, target, (w, h), &dl);
			}
			// The 8-orientation preview grid: render the armed tile/stamp into each
			// enabled cell straight from the R8Uint index atlas (draw_picker) - it
			// retints live on palette edits with no per-frame RGBA compose, over the
			// widget's field wells / grey veils / rings.
			// The cells come from the content widget itself, read back after `build`
			// settled the layout they hang off - one computation of the geometry, so
			// the native quads and the chrome under them cannot drift (U5.3's
			// invariant; here the pass can simply be *handed* the rects, because
			// `draw_picker` goes through the separately-borrowed `renderer` rather
			// than through `passes`).
			if let Some(cells) = passes.toolbox_content.root().map(toolbox::ToolboxContent::preview_cells) {
				let quads = toolbox_preview_quads(editor, &cells);
				if !quads.is_empty() {
					let clip = ui::Rect::new(body.x, body.y, body.w.max(0.0), body.h.max(0.0));
					renderer.draw_picker(device, encoder, target, &quads, clip, (w, h), scale, 1.0);
				}
			}
		} else if id == "savetools" {
			// The Save Toolbox is a real widget tree (U5.2): a `ScrollArea` over a
			// flow of key blocks, which clip and scroll themselves. `sync` only
			// pushes which keys are lit - hover and press are each key's own, so no
			// pointer state reaches it.
			if let Some(mc) = passes.menu_chrome.as_mut() {
				mc.prepare_panel((w, h), scale, map);
				if let Some(c) = passes.savetools_content.root_mut() {
					c.sync(savetools::Snapshot::of(editor));
				}
				let mut dl = DrawList::new();
				passes.savetools_content.build(mc, body, scale, &[], &mut dl, &mut panel_popups);
				mc.render_list(encoder, target, (w, h), &dl);
			}
		} else if id == "passtools" {
			// The Pass Types Palette is a real widget tree: a `ScrollArea` over the
			// swatch block and the cell tally. `sync` only pushes which swatch is lit
			// and what the tally reads - hover and press are each key's own, so no
			// pointer state reaches it.
			if let Some(mc) = passes.menu_chrome.as_mut() {
				mc.prepare_panel((w, h), scale, map);
				if let Some(c) = passes.passtools_content.root_mut() {
					c.sync(passtools::Snapshot::of(editor));
				}
				let mut dl = DrawList::new();
				passes.passtools_content.build(mc, body, scale, &[], &mut dl, &mut panel_popups);
				mc.render_list(encoder, target, (w, h), &dl);
			}
		} else if id == "unitprops" {
			// The Unit Properties inspector is a real widget tree (U5.8): a
			// `ScrollArea` over a column of form sections, whose optional blocks
			// (turret row, values section, connector grid) are `Reveal` slots.
			// `sync` only pushes the selection's values - hover, arming, focus and
			// scrolling are each widget's own, so no pointer state reaches it.
			let snap = unitprops::Snapshot::of(editor);
			if let Some(mc) = passes.menu_chrome.as_mut() {
				mc.prepare_panel((w, h), scale, map);
				if let Some(c) = passes.unitprops_content.root_mut() {
					c.sync(snap);
				}
				let mut dl = DrawList::new();
				// `build`'s arrange re-clamps the tree's own scroll for the current
				// selection (it shrinks when a shorter object is picked), so the
				// wells read back below are the ones the panel actually drew.
				passes.unitprops_content.build(mc, body, scale, &[], &mut dl, &mut panel_popups);
				mc.render_list(encoder, target, (w, h), &dl);
			}
			// Native units-pass sprites over the panel (scissored to its body): the
			// header band's live preview and the connector-grid footprint thumbnail.
			// Both rects come from the content widgets that reserved them, read back
			// after `build` settled the layout they hang off - one computation of the
			// geometry, so the native quads and the chrome under them cannot drift
			// (U5.3's invariant; here `passes.units` is separately borrowed, so no
			// recompute is needed).
			if let (Some(idx), Some(lib), Some(ugpu), Some(content)) =
				(editor.selected_object, editor.units.as_ref(), passes.units.as_ref(), passes.unitprops_content.root())
			{
				if let Some(obj) = editor.project.objects.get(idx) {
					// Top header live preview (item 11): the full composited sprite —
					// body + connector struts + turret (item 4b) — over the header well.
					let preview = content
						.preview_rect()
						.map(|r| units::object_preview_quads(obj, r, lib, &ugpu.slots))
						.unwrap_or_default();
					if !preview.is_empty() {
						ugpu.draw(device, encoder, target, &preview, Some(body), (w, h), scale, false);
					}
					// The connector-grid footprint thumbnail (host buildings only —
					// and the widget only reserves the rect for those).
					if let Some(q) =
						content.connector_rect().and_then(|fp| units::object_sprite_quad(obj, fp, lib, &ugpu.slots))
					{
						ugpu.draw(device, encoder, target, &[q], Some(body), (w, h), scale, false);
					}
				}
			}
		} else if id == "tiles" {
			// The Tile Explorer is a retained `PickerContent` widget: the tile
			// stills render straight from the R8Uint index atlas through the
			// palette shader (`draw_picker`, no per-frame RGBA compose - retints
			// live on palette edits), with the header + rings chrome drawn over.
			if let Some(c) = passes.picker_content.root_mut() {
				c.sync(picker::Snapshot::of(&editor.project, &editor.picker, editor.active_tile()));
				// A `picker scroll N` / reveal-the-active-tile the command layer
				// queued: `execute` cannot reach the panel `Ui`, so it leaves a
				// request the widget resolves against its own geometry (U2.4).
				if let Some(req) = editor.picker.scroll_request.take() {
					c.request_scroll(req);
				}
			}
			if let Some(mc) = passes.menu_chrome.as_mut() {
				mc.prepare_panel((w, h), scale, map);
				let mut dl = DrawList::new();
				// Build first: `arrange` settles the grid widget's rect *and* its
				// scroll (applying any queued request), and those are exactly what
				// the stills' quads and scissor hang off — so the native pass and
				// the chrome over it are one layout. They still *draw* first, under
				// the chrome.
				passes.picker_content.build(mc, body, scale, &[], &mut dl, &mut panel_popups);
				if let Some(c) = passes.picker_content.root() {
					let (quads, clip) = c.visible_tile_quads();
					renderer.draw_picker(device, encoder, target, &quads, clip, (w, h), scale, 1.0);
				}
				mc.render_list(encoder, target, (w, h), &dl);
			}
		} else if id == "minimap" {
			passes.minimap.draw(device, queue, encoder, target, &passes.blit, editor, body, (w, h), scale);
			let mode = editor.minimap_mode;
			let view = minimap::view_rect(editor, body);
			let map_size = editor.map_size();
			if let Some(mc) = passes.menu_chrome.as_mut() {
				mc.prepare_panel((w, h), scale, map);
				// Header steel band shell-side (the theme's header band); the
				// retained tree draws the mode keys over it, and its content
				// widget the camera outline over the (native) texture.
				let mut dl = DrawList::new();
				wgpu_ui::Theme::header_band(mc.theme(), &mut dl, crate::ui::strip_top(body, minimap::HEADER_H));
				if let Some(o) = passes.minimap_overlay.root_mut() {
					o.sync(mode, map_size, view);
				}
				passes.minimap_overlay.build(mc, body, scale, &[], &mut dl, &mut panel_popups);
				mc.render_list(encoder, target, (w, h), &dl);
			}
		} else if id == "palette" {
			let base: Vec<u8> = editor.project.palette.clone();
			// While cycling, swatches show the live working palette.
			let display: Vec<u8> = if editor.animate {
				editor.cycler.rgba().chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]]).collect()
			} else {
				base.clone()
			};
			let names = editor.palette_file_names();
			let multi: Vec<u16> = editor.palettes.multi.iter().map(|&s| s as u16).collect();
			// The whole palette is a retained `PaletteContent` widget tree (U5.9) —
			// it has no native pass, so every layer draws into one list. Hover, press
			// and scrolling are each widget's own, so no pointer state reaches it.
			if let Some(mc) = passes.menu_chrome.as_mut() {
				mc.prepare_panel((w, h), scale, map);
				if let Some(c) = passes.palette_content.root_mut() {
					c.sync(palette_panel::Snapshot::of(
						&display,
						&base,
						editor.active_color.map(u16::from),
						editor.palettes.sel_end.map(u16::from),
						&multi,
						editor.animate,
						true,
						editor.palettes.show_saved,
						&names,
						editor.palettes.sel,
						editor.selected_palette_is_user(),
					));
					// `palette scroll N` from the command layer, which cannot reach
					// this `Ui`; the widget applies it at its next layout (U2.5).
					if let Some(to) = editor.palettes.scroll_request.take() {
						c.request_scroll(to);
					}
				}
				let mut dl = DrawList::new();
				passes.palette_content.build(mc, body, scale, &[], &mut dl, &mut panel_popups);
				mc.render_list(encoder, target, (w, h), &dl);
			}
		} else if id == "wrlpalette" {
			// The document's internal palette - the file's bytes, not the
			// game-resolved working palette. Cycled swatches only when the
			// cycler is actually seeded from it (the Debug ▸ map-palette mode).
			let base: Vec<u8> = editor.project.internal_palette();
			let display: Vec<u8> = if editor.animate && editor.debug_map_palette {
				editor.cycler.rgba().chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]]).collect()
			} else {
				base.clone()
			};
			if let Some(mc) = passes.menu_chrome.as_mut() {
				mc.prepare_panel((w, h), scale, map);
				if let Some(c) = passes.wrlpalette_content.root_mut() {
					c.sync(palette_panel::Snapshot::of_bare(
						&display,
						&base,
						editor.active_color.map(u16::from),
						editor.palettes.sel_end.map(u16::from),
						editor.animate,
					));
				}
				let mut dl = DrawList::new();
				passes.wrlpalette_content.build(mc, body, scale, &[], &mut dl, &mut panel_popups);
				mc.render_list(encoder, target, (w, h), &dl);
			}
		} else if id == "units" {
			// The units panel is a real widget tree (U5.7): a fixed-height header
			// row — five team swatches, the eraser toggle and the active tag — over
			// a `UnitsGrid` content widget. Its `build` runs first, because
			// `arrange` settles the grid's rect and its own scroll; the wells and
			// the native sprite pass below both come from the window that settles
			// (`visible_cells`), and the chrome list it produces is composited last,
			// on top. Hover and press are each widget's own.
			let mut chrome = DrawList::new();
			if let Some(mc) = passes.menu_chrome.as_mut() {
				mc.prepare_panel((w, h), scale, map);
				if let Some(c) = passes.units_content.root_mut() {
					c.sync(units::Snapshot::of(
						editor.units.as_ref(),
						passes.units.as_ref().map(|g| &g.slots),
						editor.active_unit,
						editor.unit_team,
						editor.tool == state::Tool::UnitEraser,
					));
				}
				passes.units_content.build(mc, body, scale, &[], &mut chrome, &mut panel_popups);
			}
			let (cells, clip) =
				passes.units_content.root().map_or_else(|| (Vec::new(), ui::Rect::ZERO), |c| c.visible_cells());
			// Per-cell black wells behind the sprites (units paint on black, per cell
			// — not the steel panel). Drawn first so the native sprite pass and the
			// chrome rings both composite on top.
			if let Some(mc) = passes.menu_chrome.as_mut() {
				mc.prepare_panel((w, h), scale, map);
				let mut bg = DrawList::new();
				units::cell_backgrounds(&mut bg, &cells, clip);
				mc.render_list(encoder, target, (w, h), &bg);
			}
			let quads =
				units::quads(editor.units.as_ref(), passes.units.as_ref().map(|g| &g.slots), editor.unit_team, &cells);
			if let Some(g) = &passes.units {
				g.draw(device, encoder, target, &quads, Some(clip), (w, h), scale, false);
			}
			if let Some(mc) = passes.menu_chrome.as_mut() {
				mc.prepare_panel((w, h), scale, map);
				mc.render_list(encoder, target, (w, h), &chrome);
			}
		} else if id == "scenery" {
			// The Scenery panel is the Templates Explorer's shape (U5.5): a header
			// flow - the pack filter, the preview-size dropdown and the count - over
			// a `SceneryGrid`. `build` runs first, because `arrange` settles the
			// grid's rect and its scroll; the wells and the native thumbnail pass
			// both come from the window that settles, and the chrome composites last.
			let mut chrome = DrawList::new();
			if let Some(mc) = passes.menu_chrome.as_mut() {
				mc.prepare_panel((w, h), scale, map);
				if let Some(c) = passes.scenery_content.root_mut() {
					c.sync(
						scenery::Snapshot::of(
							&editor.project,
							editor.active_scenery,
							editor.scenery_cell,
							editor.scenery_pack.as_deref(),
							editor.dev_mode,
						)
						.with_blend(editor.scenery_blend),
					);
				}
				passes.scenery_content.build(mc, body, scale, &[], &mut chrome, &mut panel_popups);
			}
			let (cells, clip) =
				passes.scenery_content.root().map_or_else(|| (Vec::new(), ui::Rect::ZERO), |c| c.visible_cells());
			// Per-cell black wells: a cut-out's palette has to read against a
			// neutral ground, not the steel panel.
			if let Some(mc) = passes.menu_chrome.as_mut() {
				mc.prepare_panel((w, h), scale, map);
				let mut bg = DrawList::new();
				scenery::cell_backgrounds(&mut bg, &cells, clip);
				mc.render_list(encoder, target, (w, h), &bg);
			}
			if let Some(g) = passes.scenery.as_mut() {
				let quads = scenery_render::thumb_quads(&editor.project, g, &cells);
				g.draw(device, encoder, target, &quads, false, Some(clip), (w, h), scale);
			}
			if let Some(mc) = passes.menu_chrome.as_mut() {
				mc.prepare_panel((w, h), scale, map);
				mc.render_list(encoder, target, (w, h), &chrome);
			}
		} else if id == "templates" {
			// The Templates Explorer is a real widget tree (U5.5): a flowed header
			// of command keys + the two dropdowns + the count, over a
			// `TemplatesGrid` content widget that draws the thumbnails itself -
			// they are uv'd from the composed atlas through the same `DrawList`,
			// so this panel has no native pass at all. `sync` only pushes
			// per-frame state; hover and press are each widget's own, so no pointer
			// state reaches it.
			let visible = editor.visible_templates();
			let entries: Vec<&state::TemplateEntry> = visible.iter().map(|&g| &editor.templates.entries[g]).collect();
			// The explorer's selection, mapped into the visible list.
			let selected = editor.templates.sel.and_then(|g| visible.iter().position(|&v| v == g));
			// The tileset filter options + the current selection resolved to an
			// index into them (a label absent from this map -> None = all).
			let tilesets = editor.template_tilesets();
			let tileset_sel = editor.templates.tileset.as_deref().and_then(|l| tilesets.iter().position(|x| x == l));
			// A template's global index is also its cell in the thumbnail atlas,
			// so the visible list doubles as the slot list.
			let thumbs = passes.template_atlas.as_ref().map(|a| templates_panel::ThumbAtlas {
				tex: a.tex,
				cols: a.cols,
				rows: a.rows,
				fracs: &a.fracs,
			});
			let snap = templates_panel::Snapshot::of(
				&entries,
				&visible,
				thumbs.as_ref(),
				selected,
				editor.templates.cell,
				tileset_sel,
				tilesets,
			);
			if let Some(mc) = passes.menu_chrome.as_mut() {
				mc.prepare_panel((w, h), scale, map);
				if let Some(c) = passes.templates_content.root_mut() {
					c.sync(snap);
				}
				let mut dl = DrawList::new();
				passes.templates_content.build(mc, body, scale, &[], &mut dl, &mut panel_popups);
				mc.render_list(encoder, target, (w, h), &dl);
			}
		}
	}
	// Every panel's open dropdown, composited after the whole loop on the origin
	// steel plate (like the menu / context menu) so it floats over every panel
	// instead of being clipped by its own and painted over by the next (U3.2).
	if !panel_popups.is_empty()
		&& let Some(mc) = passes.menu_chrome.as_mut()
	{
		mc.prepare((w, h), scale);
		mc.render_list(encoder, target, (w, h), &panel_popups);
	}
	// Project tab strip below the menu bar; the menu (with its dropdowns) draws
	// last so it stays topmost.
	let tab_infos = editor.tab_infos();
	// Bottom status bar (View ▸ Status Bar): drawn over the docks, under the
	// menus / modals / console chrome that follows.
	if editor.status_bar {
		let cursor_cell = editor.cursor.and_then(|(cx, cy)| {
			// Logical cursor for the UI gate; physical (× scale) for the map cell.
			over_at(editor, popup, cx, cy).is_map().then(|| editor.cell_at(cx * scale, cy * scale)).flatten()
		});
		// A hovered key's tooltip text mirrors into the hint slot while the
		// hover holds; leaving restores the tool hint (recomputed every frame,
		// so there is nothing to put back by hand).
		let hover_hint = passes.hovered_hint();
		if let Some(mc) = passes.menu_chrome.as_mut() {
			mc.prepare((w, h), scale);
			let dl = passes.status.build(mc, editor, hover_hint.as_deref(), cursor_cell, wf, hf, scale);
			mc.render_list(encoder, target, (w, h), &dl);
		}
	}
	let tabs_closable = editor.tabs_closable();
	if let Some(mc) = passes.menu_chrome.as_mut() {
		mc.prepare((w, h), scale);
		// The steel band + lit bottom seam are chrome the theme owns (drawn
		// shell-side, like the status strip); the retained `TabStrip` widget
		// draws the tabs over it.
		let strip = crate::ui::Rect::new(0.0, menu::BAR_H, wf, tabs::BAR_H);
		let mut dl = DrawList::new();
		mc.theme().material(&mut dl, strip, theme::PANEL);
		mc.theme().seam(&mut dl, strip, crate::uikit_theme::Edge::Bottom);
		if let Some(s) = passes.tabs_strip.root_mut() {
			s.sync(tab_infos.clone(), editor.active_tab(), tabs_closable);
		}
		passes.tabs_strip.build(mc, strip, scale, &[], &mut dl, &mut DrawList::new());
		mc.render_list(encoder, target, (w, h), &dl);
	}
	// Resolve a menu toggle's `key` against live editor state for its checkbox.
	let checked = |key: &str| -> bool {
		match key {
			"grid" => editor.show_grid,
			"status-bar" => editor.status_bar,
			"pass-overlay" => editor.show_pass_overlay,
			"resources" => editor.show_resources,
			"shore-bugs" => editor.show_shore_bugs,
			"match-problems" => editor.show_match_problems,
			"show-units" => editor.show_units,
			"mode:map" => editor.mode == state::EditorMode::Map,
			"mode:pass" => editor.mode == state::EditorMode::Pass,
			"mode:localpass" => editor.mode == state::EditorMode::LocalPass,
			"mode:save" => editor.mode == state::EditorMode::SaveEditor,
			"layer:water" => editor.active_layer == map_core::LAYER_WATER,
			"layer:ground" => editor.active_layer == map_core::LAYER_GROUND,
			"layer:scenery" => editor.on_scenery_layer(),
			"layer:only-selected" => editor.show_only_layer,
			"anim:off" => !editor.animate && !editor.ingame,
			"anim:on" => editor.animate && !editor.ingame,
			"anim:ingame" => editor.ingame,
			"crt" => editor.crt,
			"ui-scale:small" => (editor.ui_scale - 1.0).abs() < 0.01,
			"ui-scale:medium" => (editor.ui_scale - 1.25).abs() < 0.01,
			"ui-scale:large" => (editor.ui_scale - 1.5).abs() < 0.01,
			"debug:map-palette" => editor.debug_map_palette,
			_ => key.strip_prefix("win:").is_some_and(|id| editor.workspace.is_visible(id)),
		}
	};
	// The menu bar is a retained `wgpu_ui::MenuBar` (input + draw both) hosted
	// in the editor's `menu_panel` Ui: re-sync the live toggle checkmarks, then
	// draw it last (over the docks; overlay dialogs composite after the frame).
	let marks: Vec<(u64, bool)> = editor.menu_toggles.iter().map(|&(id, key)| (id, checked(key))).collect();
	if let Some(mc) = passes.menu_chrome.as_mut() {
		mc.prepare((w, h), scale);
		if let Some(bar) = editor.menu_panel.ui.get_mut::<wgpu_ui::MenuBar>(editor.menu_id) {
			for (id, on) in marks {
				bar.set_checked(id, on);
			}
		}
		let mut dl = DrawList::new();
		// The open cascade draws in the overlay pass — into its own list, like
		// every hosted Ui. The bar is the top chrome layer at this point in the
		// frame, so its popups composite right after its base.
		let mut popups = DrawList::new();
		editor.menu_panel.build(mc, ui::Rect::new(0.0, 0.0, wf, hf), scale, &[], &mut dl, &mut popups);
		mc.render_list(encoder, target, (w, h), &dl);
		if !popups.is_empty() {
			mc.render_list(encoder, target, (w, h), &popups);
		}
	}
	// The right-click context menu floats over panels and the menu bar -
	// drawn last. The widget (hosted alone in its own Ui) syncs from the
	// editor's model snapshot: opened at the model's anchor with its baked
	// items, closed when the model clears (Esc / wheel / a press resolved).
	if let Some(mc) = passes.menu_chrome.as_mut() {
		match &editor.context_menu {
			Some(cm) => {
				let key = (cm.pos.0, cm.pos.1, cm.items.len());
				if passes.context_synced != Some(key) {
					passes.context_synced = Some(key);
					let (items, acts) = menu::build_context(&cm.items);
					passes.context_acts = acts;
					if let Some(w) = passes.context_menu.root_mut() {
						w.set_items(items);
						w.open_at(wgpu_ui::Vec2::new(cm.pos.0, cm.pos.1));
					}
				}
				mc.prepare((w, h), scale);
				// The menu itself draws in the overlay pass (the base pass is
				// the widget's wrapped content, none here).
				let mut dl = DrawList::new();
				let mut popups = DrawList::new();
				passes.context_menu.build(mc, ui::Rect::new(0.0, 0.0, wf, hf), scale, &[], &mut dl, &mut popups);
				mc.render_list(encoder, target, (w, h), &dl);
				if !popups.is_empty() {
					mc.render_list(encoder, target, (w, h), &popups);
				}
			}
			None => {
				if passes.context_synced.take().is_some() {
					if let Some(cmw) = passes.context_menu.root_mut() {
						cmw.close();
					}
				}
			}
		}
	}
	if editor.console.is_open() {
		// The widget owns the geometry, so how many rows fit is *its* answer:
		// read it back to clamp the model's paging (the U2 "build, then read"
		// rule), and hand it exactly the window it will draw.
		let rows = passes.console_view.root().map_or(10, console_view::ConsoleView::rows);
		editor.console.set_view_rows(rows);
		let lines = editor.console.visible_lines(rows);
		if let Some(c) = passes.console_view.root_mut() {
			c.sync(lines);
		}
		if let Some(mc) = passes.menu_chrome.as_mut() {
			// Logical px like every other panel now — the console honors UI Scale
			// instead of riding the framebuffer at 1.0.
			mc.prepare((w, h), scale);
			let mut dl = DrawList::new();
			let body = console_view::console_rect(wf, hf);
			passes.console_view.build(mc, body, scale, &[], &mut dl, &mut DrawList::new());
			mc.render_list(encoder, target, (w, h), &dl);
		}
	}
	// CRT: post-process the offscreen scene onto the real target.
	if crt_on {
		let scene = passes.scene.as_ref().expect("scene built when crt_on");
		passes.crt.draw(encoder, final_target, &scene.bind_group);
	}
}

/// Compose the match editor's strip texture: the main tile at rest plus the
/// candidate at all 8 orientations - 9 cells of 64×64 RGBA side by side,
/// through the rest-palette LUT (static art, like the tile atlas).
fn compose_match_strip(editor: &EditorState, pack: usize, main: u16, cand: u16) -> Vec<u8> {
	const T: usize = 64;
	let lut = tile_atlas::rest_lut(&editor.project.palette);
	let mut cells: Vec<Vec<u8>> = Vec::with_capacity(9);
	let tile = |tile, transform| map_core::TileRef { pack: pack as u8, tile, transform };
	cells.push(tile_atlas::compose_tile(&editor.project, tile(main, map_core::Transform::default()), &lut));
	for k in 0..8u32 {
		let transform = map_core::Transform { rot: (k & 3) as u8, mirror: k & 4 != 0 };
		cells.push(tile_atlas::compose_tile(&editor.project, tile(cand, transform), &lut));
	}
	// Row-major assembly: each output row concatenates the 9 cells' rows.
	let mut out = vec![0u8; 9 * T * T * 4];
	for (ci, cell) in cells.iter().enumerate() {
		for row in 0..T {
			let src = &cell[row * T * 4..(row + 1) * T * 4];
			let off = (row * 9 * T + ci * T) * 4;
			out[off..off + T * 4].copy_from_slice(src);
		}
	}
	out
}

/// Sink for the `log` facade - the copied decoders in `max-assets` report
/// real failures (RLE decode, malformed res.ini) through `log::error!`/
/// `warn!`; without an installed logger those messages vanish.
struct StderrLogger;

static LOGGER: StderrLogger = StderrLogger;

impl log::Log for StderrLogger {
	fn enabled(&self, metadata: &log::Metadata) -> bool {
		metadata.level() <= log::Level::Warn
	}

	fn log(&self, record: &log::Record) {
		if self.enabled(record.metadata()) {
			eprintln!("{}: {}", record.level().as_str().to_ascii_lowercase(), record.args());
		}
	}

	fn flush(&self) {}
}

fn main() {
	let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(log::LevelFilter::Warn));
	let args = parse_args();

	// Initial load goes through the same `open` path as the command -
	// it sniffs .json (project) vs .WRL and sets up view/palette/cycler.
	let mut editor = EditorState::new(Project::empty(), args.size, None, resources_dir());
	if let Outcome::Failed(message) = editor.execute(Command::Open { path: args.map.clone() }) {
		eprintln!("{message}");
		std::process::exit(1);
	}

	// Settings: one `mme.ini` carries everything - paths, bindings,
	// mouse, UI layout. `--settings` always wins; a windowed run falls back
	// to the config default; headless without the flag stays off (keeps the
	// script suite from touching any file). Restore now if present.
	editor.headless = args.headless;
	editor.dev_mode = args.dev;
	// Installed tilesets feed the DEV ▸ Match Combinations Map submenu (WATER has
	// no tile pairings, so it's left out).
	let dev_packs: Vec<String> = if args.dev {
		crate::packlist::scan(&editor.assets_root).into_iter().map(|p| p.name).filter(|n| n != "WATER").collect()
	} else {
		Vec::new()
	};
	editor.menu_set_dev(args.dev, &dev_packs);
	// Settings layering: `--settings PATH` is a single self-contained file (load
	// + save there). Otherwise the shipped defaults (`resources/config/mme.ini`)
	// are overlaid by the user's overrides (`resources/user/config/mme.ini`, where
	// the app saves). Headless without `--settings` keeps persistence off.
	let (settings, save_path) = if let Some(path) = args.settings.clone() {
		(read_ini(&path), Some(path))
	} else if args.headless {
		(None, None)
	} else {
		let resources = resources_dir();
		let shipped = read_ini(&resources.join("config/mme.ini"));
		let user_path = resources.join("user/config/mme.ini");
		let merged = match (shipped, read_ini(&user_path)) {
			(Some(mut base), user) => {
				if let Some(over) = user {
					base.overlay(over);
				}
				Some(base)
			}
			(None, user) => user,
		};
		(merged, Some(user_path))
	};
	editor.settings_path = save_path;
	if let Some(ini) = &settings {
		if let Some(section) = ini.get_section("Workspace") {
			let (w, h) = editor.screen;
			editor.workspace.apply_ini(section, w as f32, h as f32);
			// The INI parser types bare numbers, so read numerics as Integer or
			// Float (a `String` fetch would miss `UiScale=1.25`, `TilesPreview=64`).
			let num = |key: &str| -> Option<f32> {
				section
					.get_entry::<i64>(key)
					.map(|n| n as f32)
					.or_else(|| section.get_entry::<f64>(key).map(|f| f as f32))
			};
			// UI scale (View ▸ UI Scale): snap a (possibly hand-edited) value to the
			// nearest supported level so the menu radio + render stay consistent.
			if let Some(scale) = num("UiScale") {
				let snapped = state::UI_SCALES
					.into_iter()
					.min_by(|a, b| (a - scale).abs().total_cmp(&(b - scale).abs()))
					.unwrap_or(1.0);
				editor.set_ui_scale(snapped);
			}
			// Explorer preview sizes (honoured only when they name a real option;
			// a stray value just keeps the default).
			if let Some(px) = num("TilesPreview").filter(|px| picker::SIZES.contains(px)) {
				editor.picker.tile_px = px;
			}
			if let Some(px) =
				num("TemplatesPreview").filter(|px| templates_panel::PREVIEW_SIZES.iter().any(|&(s, _)| s == *px))
			{
				editor.templates.cell = px;
			}
			if let Some(px) = num("SceneryPreview").filter(|px| scenery::PREVIEW_SIZES.iter().any(|&(s, _)| s == *px)) {
				editor.scenery_cell = px;
			}
			// Recent maps (File ▸ Quick Load): the [QuickLoad] section, keys 0..n,
			// most-recent first. Falls back to the legacy [Workspace] Recent0.. keys
			// (a one-time migration from older settings files).
			let recent: Vec<PathBuf> = ini
				.get_section("QuickLoad")
				.map(|qs| (0..10).map_while(|i| qs.get_entry::<String>(&i.to_string())).map(PathBuf::from).collect())
				.filter(|v: &Vec<PathBuf>| !v.is_empty())
				.unwrap_or_else(|| {
					(0..10)
						.map_while(|i| section.get_entry::<String>(&format!("Recent{i}")))
						.map(PathBuf::from)
						.collect()
				});
			if !recent.is_empty() {
				editor.load_recent(recent);
			}
		}
		// Seed each mode's dock-layout slot: Main from the (just-applied) live
		// workspace, Pass/Save from their [Workspace.*] section or - when absent
		// - the current main layout.
		let (w, h) = editor.screen;
		editor.seed_mode_layouts(ini, w as f32, h as f32);
		editor.max_path =
			ini.get_entry::<String>("Paths", "MaxPath").filter(|p| !p.trim().is_empty()).map(PathBuf::from);
		editor.max_port_path =
			ini.get_entry::<String>("Paths", "MaxPortPath").filter(|p| !p.trim().is_empty()).map(PathBuf::from);
		editor.max_port_data_path =
			ini.get_entry::<String>("Paths", "MaxPortDataPath").filter(|p| !p.trim().is_empty()).map(PathBuf::from);
		// "Don't ask again" for the first-run paths prompt (Editor Preferences).
		editor.skip_path_prompt = ini.get_entry::<bool>("Paths", "SkipPathPrompt").unwrap_or(false);
		// [Preferences]: small user options (the New Map palette-preview toggle).
		editor.palette_preview = ini.get_entry::<bool>("Preferences", "PalettePreview").unwrap_or(false);
	}
	match &editor.max_path {
		Some(path) if !path.is_dir() => {
			editor.console.push_line(format!("MaxPath is set but not a directory: {}", path.display()));
		}
		None => editor
			.console
			.push_line("MaxPath not set - point it at your M.A.X. directory in resources/user/config/mme.ini"),
		Some(_) => {}
	}

	// Load the unit library up front when MaxPath is set - the Units panel
	// is then populated on first open, and headless screenshots render the
	// project's unit previews. Without MaxPath this is a no-op.
	if editor.max_path.is_some() {
		let _ = editor.ensure_units();
	}

	// The max-port unit database (stock UnitValues + clan advantages +
	// applicability metadata out of PATCHES.RES). Explains itself on the
	// console when no configured folder holds the archive.
	editor.reload_unit_stats();

	// Bindings load before the headless branch so menu shortcut hints (and
	// the context menu's) render identically windowed and headless.
	let bindings = input::Bindings::load(settings.as_ref());
	editor.apply_shortcut_hints(bindings.hint_table());

	if args.headless {
		std::process::exit(run_headless(editor, args.script));
	}

	let event_loop = EventLoop::new().expect("create event loop");
	let mut app = App::new(editor, bindings, args.script);
	event_loop.run_app(&mut app).expect("run event loop");
}

/// Run the script without a window; returns the process exit code.
fn run_headless(mut editor: EditorState, script: Vec<Command>) -> i32 {
	let (device, queue) = pollster::block_on(gpu::headless());
	let render_core = project_render::RenderCore::new(&device, capture::FORMAT);
	let mut renderer = make_renderer(&device, &queue, &editor, &render_core);
	let mut passes = Passes::new(&device, &queue, capture::FORMAT);
	let mut uploaded_revision = editor.revision();

	for command in script {
		match editor.execute(command) {
			Outcome::DocReplaced => {
				adopt_new_document(
					&mut renderer,
					&mut passes,
					&mut editor,
					&mut uploaded_revision,
					&device,
					&queue,
					&render_core,
				);
			}
			Outcome::Screenshot { path, crop, resize } => {
				if editor.revision() != uploaded_revision {
					refresh_renderer(&renderer, &queue, &mut editor);
					uploaded_revision = editor.revision();
				}
				if let Some(rgba) = editor.cycler.take_if_dirty() {
					sync_palette(rgba, &renderer, &passes, &queue);
				}
				// The match-edit dialog can't open headless, so its atlas is never needed.
				refresh_tile_atlas(&editor, &mut passes, false);
				refresh_template_atlas(&editor, &mut passes);
				let (w, h) = editor.screen;
				let passes = &mut passes;
				capture::render_to_png(&device, &queue, w, h, &path, crop, resize, |encoder, view| {
					render_frame(&device, &queue, encoder, view, &mut editor, &renderer, passes);
				});
			}
			Outcome::Failed(message) => {
				eprintln!("FAILED: {message}");
				return 1;
			}
			Outcome::Quit => break,
			// No window, no overlay: a requested dialog has nowhere to show.
			Outcome::OpenDialog(_) => {}
			Outcome::Ok | Outcome::Redraw => {}
		}
	}
	// Persist the (possibly --settings-driven) layout before exiting.
	if editor.settings_path.is_some() {
		editor.execute(Command::SaveSettings);
	}
	0
}

struct WindowState {
	window: Arc<Window>,
	gpu: gpu::WindowGpu,
	/// Shared device-lifetime pipelines; every `make_renderer` reuses it.
	render_core: Rc<project_render::RenderCore>,
	renderer: ProjectRenderer,
	passes: Passes,
	uploaded_revision: u64,
	title: String,
	/// Proof-of-concept `wgpu-ui` overlay (toggle with F1); `None` if its font
	/// failed to load.
	overlay: Option<uikit_overlay::Overlay>,
}

/// A deferred Import WRL dialog verb ([`Deferred::Wrl`]).
enum WrlAct {
	Match { packs: Vec<String>, owner: String },
	Finish { dest: map_core::ExtrasDest },
}

/// A deferred Tile Painter commit ([`Deferred::Tile`]): the dialog's
/// typed id, chosen passability, and target pack.
struct TileCommitAct {
	id: String,
	pass: u8,
	pack: String,
}

/// A deferred New Scenery commit ([`Deferred::Scenery`]): the derived
/// piece plus where it is going. The planes can't ride a command line, and a
/// success is `DocReplaced` (the scenery atlas rebuilds), so it takes the same
/// next-redraw path a Tile Painter commit does.
struct SceneryCommitAct {
	pack: String,
	id: String,
	name: String,
	sprite: map_core::Sprite,
	pass: Vec<u8>,
	cells: (u16, u16),
	relief: Option<(u8, bool)>,
	/// The Heightmap tab's drawn relief, or `None` to infer it from the art.
	height: Option<Vec<u8>>,
}

/// A verb an overlay outcome deferred past the frame: it needs the whole
/// `&mut self` (act_on: DocReplaced → renderer rebuild) or the event loop,
/// neither of which the overlay-outcome match can take mid-`win` borrow. All
/// six kinds ride one FIFO ([`App::deferred`]) drained at a single point,
/// right after `redraw` returns — so a multi-step queue (New Map's create,
/// then its palette, then its shape carve) runs in the order it was enqueued.
enum Deferred {
	/// A command an overlay asked to run (e.g. New Map's Create).
	Command(Command),
	/// Load a custom palette right after the `Command` that created the map
	/// (the New Map palette selector's non-default choice).
	Palette(std::path::PathBuf),
	/// Carve a New Map shape image into the fresh all-water map and open Fix
	/// Shore ("Shape from image...", queued after the create + palette).
	Shape(std::path::PathBuf),
	/// An Import WRL dialog verb (match / finish).
	Wrl(WrlAct),
	/// A Tile Painter commit (a success is DocReplaced → renderer rebuild; a
	/// failure lands back in the open dialog).
	Tile(TileCommitAct),
	/// A New Scenery commit (same shape and the same reasons).
	Scenery(SceneryCommitAct),
}

struct App {
	editor: EditorState,
	bindings: input::Bindings,
	startup_script: Vec<Command>,
	win: Option<WindowState>,
	/// Overlay verbs deferred past the frame, drained FIFO at the one drain
	/// point (after `redraw` returns, where the event loop is in scope).
	deferred: VecDeque<Deferred>,
	/// The match-editor strip texture's cache key: (pack, main tile, cand
	/// tile, document revision) — recomposed when any of them moves.
	match_strip_key: Option<(usize, u16, u16, u64)>,
	/// The shell's one winit→toolkit event translator: every UI host is fed from
	/// it, so cursor / modifier / scale state cannot diverge between them.
	router: ui_router::UiRouter,
	cursor: (f32, f32),
	/// The OS mouse cursor last applied to the window (only changes reach it).
	cursor_icon: wgpu_ui::CursorIcon,
	/// OS IME state last applied: enabled + candidate-window anchor
	/// (physical px), so only changes reach winit.
	ime_on: bool,
	ime_rect: Option<(f32, f32, f32, f32)>,
	/// Cursor position at the last drag step, while a pan-drag is active.
	drag: Option<(f32, f32)>,
	/// A right press's origin, while held over the map - a release within a
	/// few px is a *click* (context menu), farther away it was a pan-drag.
	rclick: Option<(f32, f32)>,
	/// A right press on a Templates Explorer thumbnail: the global template
	/// index + press origin. A release within a few px opens that item's menu.
	rclick_template: Option<(usize, (f32, f32))>,
	/// Last painted cell, while a paint-drag (stroke) is active.
	paint: Option<(u16, u16)>,
	/// A freehand select-drag: the mode plus the last applied cell.
	select_paint: Option<(map_core::SelectMode, (u16, u16))>,
	/// A rect select-drag's anchor cell + mode (applied on release).
	select_anchor: Option<(u16, u16, map_core::SelectMode)>,
	/// An Alt+drag selection-move: the last cell the cursor passed over (the
	/// marquee translates by each cell delta; terrain is untouched).
	select_move: Option<(u16, u16)>,
	/// An object Move-tool drag: `(object index, grab offset x, grab offset y)`.
	/// The object's origin follows the cursor minus the offset so the grabbed
	/// cell stays under the pointer; the whole drag is one undo unit.
	obj_drag: Option<(usize, u16, u16)>,
	/// A scenery Move-tool drag: `(placement index, grab offset in map px)`.
	/// The footprint origin follows the cursor minus the offset, so the pixel
	/// the user grabbed stays under the pointer; the whole drag is one undo
	/// unit.
	scenery_drag: Option<(usize, i32, i32)>,
	modifiers: ModifiersState,
	last_frame: std::time::Instant,
	/// Wall-clock start of the live Auto Fix Shore run.
	autofix_clock: Option<std::time::Instant>,
	/// Wall-clock start of the live New-from-Image conversion.
	convert_clock: Option<std::time::Instant>,
	/// Wall clock of the live rasterize palette conversion.
	pconvert_clock: Option<std::time::Instant>,
	/// False until the conversion's "Loading image…" state has been painted
	/// once, so the heavy first-stage decode starts only *after* the user sees
	/// it began; otherwise the decode blocks the very frame meant to show it.
	convert_primed: bool,
	/// The map cell the cursor was over at the last move-driven redraw (`None`
	/// off the map). A move over the bare map that stays in the same cell changes
	/// nothing on-screen (the ghost/brush/status readout are cell-granular), so it
	/// skips the full-frame redraw; a cell change or any move over chrome redraws.
	hover_redraw_cell: Option<(u16, u16)>,
}

impl App {
	fn new(editor: EditorState, bindings: input::Bindings, startup_script: Vec<Command>) -> Self {
		let router = ui_router::UiRouter::new(editor.ui_scale);
		Self {
			editor,
			bindings,
			startup_script,
			router,
			win: None,
			deferred: VecDeque::new(),
			match_strip_key: None,
			cursor: (0.0, 0.0),
			cursor_icon: wgpu_ui::CursorIcon::Default,
			ime_on: false,
			ime_rect: None,
			drag: None,
			rclick: None,
			rclick_template: None,
			paint: None,
			select_paint: None,
			select_anchor: None,
			select_move: None,
			obj_drag: None,
			scenery_drag: None,
			autofix_clock: None,
			convert_clock: None,
			pconvert_clock: None,
			convert_primed: false,
			hover_redraw_cell: None,
			modifiers: ModifiersState::empty(),
			last_frame: std::time::Instant::now(),
		}
	}

	/// The stroke command for a cell under the current mode + tool: tile
	/// passability in Pass Table Editor, a per-cell override (eraser clears) in
	/// Local Pass Override Editor; otherwise erase (Eraser tool) or tile paint.
	/// Drives both the initial press and the drag continuation.
	fn paint_command(&self, x: u16, y: u16) -> Command {
		match self.editor.mode {
			state::EditorMode::Pass => Command::TilePass { x, y, value: self.editor.active_pass },
			state::EditorMode::LocalPass => match self.editor.tool {
				// The eraser lifts a local override back to the tile's value.
				state::Tool::Eraser => Command::PassClear { x, y },
				_ => Command::PassPaint { x, y, value: self.editor.active_pass },
			},
			// The save editor edits units/resources via clicks; its map surface
			// paints exactly like Map (tools unchanged).
			state::EditorMode::Map | state::EditorMode::SaveEditor => match self.editor.tool {
				// Erase only the selected layer, not the topmost present.
				state::Tool::Eraser => Command::Erase { x, y, layer: Some(self.editor.tile_layer_name().to_string()) },
				// The terrain brush paints a land/water mask (its own command).
				state::Tool::PaintMask => Command::PaintMask { x, y },
				// The resource brush paints into the cargo map (its own command, S5.3).
				state::Tool::ResourceBrush => Command::ResourcePaint { x, y },
				// The unit eraser removes the object under each dragged cell.
				state::Tool::UnitEraser => Command::UnitErase { x, y },
				_ => Command::Paint { x, y },
			},
		}
	}

	/// Seconds since the live Auto Fix Shore run started.
	fn autofix_elapsed(&self) -> f32 {
		self.autofix_clock.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0)
	}

	/// Seconds since the live New-from-Image conversion started.
	fn convert_elapsed(&self) -> f32 {
		self.convert_clock.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0)
	}

	/// Seconds since the live rasterize palette conversion started.
	fn pconvert_elapsed(&self) -> f32 {
		self.pconvert_clock.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0)
	}

	fn run(&mut self, command: Command, event_loop: &ActiveEventLoop) {
		let toggles_console = matches!(command, Command::Console { .. });
		let covered = self.covered();
		let outcome = self.editor.execute(command);
		self.act_on(outcome, event_loop);
		// `menu file` / `context-menu` open (and close) a press-modal layer over
		// the panels without the pointer moving - from the keyboard, a script line
		// or an accelerator - so the z-order shifted under a stationary cursor and
		// nothing else in the shell would notice (U5.2).
		if self.covered() != covered {
			self.resync_hover_layer();
		}
		if toggles_console {
			self.sync_console_focus();
		}
	}

	/// Whether something press-modal covers the panels: an open menu cascade or
	/// context menu.
	fn covered(&self) -> bool {
		self.editor.menu_ref().is_open() || self.editor.context_menu.is_some()
	}

	/// Re-resolve which layer the pointer is over and tell the layers that
	/// changed - for the moments the **z-order** moves under a pointer that did
	/// not. `CursorMoved` does this on every move; this is the same step for a
	/// menu or context menu that opens or closes without one.
	///
	/// It matters from U5.2 on. A panel's hover used to be the shell's:
	/// `render_frame` handed every panel `Hot::NONE` while anything press-modal
	/// was open, so nothing underneath could stay lit. A converted panel's hover
	/// is its own `Ui`'s, and a `Ui` clears `hovered` on a `PointerLeft` or a
	/// move it can hit-test - and on nothing else. Without this the key under
	/// the cursor keeps its highlight behind an open cascade, and stays dark
	/// after it closes.
	fn resync_hover_layer(&mut self) {
		// A layer mid-drag owns the whole pointer cascade and its hover is
		// deliberately frozen for the duration (the `CursorMoved` arm makes the
		// same exception).
		if self.router.capture().is_some() {
			return;
		}
		let (lcx, lcy) = self.lcursor();
		let popup = self.popup_layer();
		let over = over_at(&self.editor, popup, lcx, lcy);
		let panel = match over {
			Over::Ui(layer @ Layer::Panel(_)) => Some(layer),
			_ => None,
		};
		if let Some(left) = self.router.retarget(panel) {
			self.dispatch_layer(left, &[Event::PointerLeft]);
		}
		// The other direction: a `Ui` only refreshes `hovered` on a move it can
		// hit-test, so the panel a closing cascade uncovers needs the position it
		// already has restated before it lights again.
		if let Some(target) = panel {
			self.dispatch_layer(target, &[Event::PointerMoved { pos: Vec2::new(lcx, lcy) }]);
		}
		// The workspace frame is not a hosted `Ui` and takes neither dispatch, so
		// its own chrome hover (the titlebar close `x`) gets the same two signals
		// by hand (U6.2).
		let (sw, sh) = self.editor.ui_screen();
		if over_frame(over, popup) {
			self.editor.workspace.on_move(lcx, lcy, sw, sh);
		} else {
			self.editor.workspace.on_pointer_left();
		}
	}

	/// Follow the console's open state with the keyboard: opening it focuses its
	/// input line, closing it hands the keyboard back. The console is an
	/// accelerator *mode* — you enter it with a key, a menu item or a script line,
	/// not by clicking a field — so nothing else would ever give it focus, and
	/// without this the caret only appears once you start typing (U4.5).
	fn sync_console_focus(&mut self) {
		if self.editor.console.is_open() {
			if let Some(lost) = self.router.refocus(None) {
				self.blur_layer(lost, BlurCause::Moved);
			}
			if let Some(win) = self.win.as_mut() {
				win.passes.console_view.panel.ui.focus_first();
			}
		} else {
			// A *click* in the band leaves the router pointing at the console
			// (focus follows the press, U1.4 - `dispatch_layer` sets it), so closing
			// has to hand the keyboard back as well: otherwise `focus_layer` keeps
			// naming a layer that is no longer drawn and the next keystroke is
			// swallowed by an invisible field instead of running the map bindings.
			if self.router.focus() == Some(Layer::Console) {
				self.router.refocus(None);
			}
			if let Some(win) = self.win.as_mut() {
				// Closing is not a commit: the half-typed line stays in the field.
				win.passes.console_view.panel.blur(BlurCause::Cancelled);
			}
		}
	}

	/// True while the wgpu-ui About dialog is shown.
	fn about_open(&self) -> bool {
		self.win.as_ref().and_then(|w| w.overlay.as_ref()).is_some_and(|o| o.visible())
	}

	/// Opens the wgpu-ui About dialog, populated with live editor facts.
	/// DEV > UI Tests: the font/raster probe, at the UI scale in force.
	fn open_ui_tests(&mut self) {
		let scale = self.editor.ui_scale as f64;
		if let Some(win) = self.win.as_mut() {
			if let Some(overlay) = &mut win.overlay {
				overlay.set_scale(scale);
				overlay.open_ui_tests();
			}
			win.window.request_redraw();
		}
	}

	fn open_about(&mut self) {
		let scale = self.editor.ui_scale as f64;
		if let Some(win) = self.win.as_mut() {
			if let Some(overlay) = &mut win.overlay {
				overlay.set_scale(scale);
				overlay.open_about();
			}
			win.window.request_redraw();
		}
	}

	/// Opens the wgpu-ui Map Metadata form, populated from the project.
	/// `save_after` = the first-save prompt (Save resumes Save-As); a
	/// template-born map then starts with date/version/author blanked - the
	/// template's name, players and description carry over, its provenance
	/// fields don't.
	fn open_metadata(&mut self, save_after: bool) {
		let vals = {
			let p = &self.editor.project;
			let blank = save_after && self.editor.doc_from_template();
			uikit_overlay::MetadataValues {
				name: p.name.clone(),
				players: p.players,
				description: p.description.clone(),
				date: if blank { String::new() } else { p.date.clone() },
				version: if blank { String::new() } else { p.map_version.clone() },
				author: if blank { String::new() } else { p.author.clone() },
			}
		};
		let scale = self.editor.ui_scale as f64;
		if let Some(win) = self.win.as_mut() {
			if let Some(overlay) = &mut win.overlay {
				overlay.set_scale(scale);
				overlay.open_metadata(vals, save_after);
			}
			win.window.request_redraw();
		}
	}

	/// Opens Editor Preferences (the M.A.X. / M.A.X. Port folder paths), seeded
	/// from the current settings.
	fn open_preferences(&mut self) {
		let disp = |p: &Option<std::path::PathBuf>| p.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
		let (max_path, max_port, max_port_data, skip) = (
			disp(&self.editor.max_path),
			disp(&self.editor.max_port_path),
			disp(&self.editor.max_port_data_path),
			self.editor.skip_path_prompt,
		);
		// "Required" when a missing-path action opened it (cancel → Attention).
		let required = self.editor.paths_prompt_reason.is_some();
		let scale = self.editor.ui_scale as f64;
		if let Some(win) = self.win.as_mut() {
			if let Some(overlay) = &mut win.overlay {
				overlay.set_scale(scale);
				overlay.open_preferences(&max_path, &max_port, &max_port_data, skip, required);
			}
			win.window.request_redraw();
		}
	}

	/// Opens the wgpu-ui New Map form (scans the installed tile packs and the
	/// palette choices). `shape` is the land/water PNG when the form opens via
	/// File → New Terrain from Image.
	fn open_newmap(&mut self, shape: Option<std::path::PathBuf>) {
		let packs = crate::packlist::scan(&self.editor.assets_root);
		let (palettes, tilesets) =
			crate::newmap::palette_choices(&packs, &self.editor.assets_root, &self.editor.user_palettes_dir());
		let scale = self.editor.ui_scale as f64;
		let preview = self.editor.palette_preview;
		if let Some(win) = self.win.as_mut() {
			if let (Some(overlay), Some(chrome)) = (win.overlay.as_mut(), win.passes.menu_chrome.as_mut()) {
				overlay.set_scale(scale);
				overlay.open_newmap(chrome, packs, &self.editor.assets_root, palettes, tilesets, preview, shape);
			}
			win.window.request_redraw();
		}
	}

	/// Opens the wgpu-ui Resize Map form, seeded with the current map size.
	fn open_resize(&mut self) {
		let (w, h) = (self.editor.project.width, self.editor.project.height);
		let scale = self.editor.ui_scale as f64;
		if let Some(win) = self.win.as_mut() {
			if let Some(overlay) = &mut win.overlay {
				overlay.set_scale(scale);
				overlay.open_resize(w, h);
			}
			win.window.request_redraw();
		}
	}

	/// User saved-palette names (file stems under the user palettes dir) - the
	/// clash list for the Save/Rename name dialogs. Mirrors the bespoke
	/// `user_palette_names`, using the public dir + palette file list.
	fn user_palette_names(&self) -> Vec<String> {
		let dir = self.editor.user_palettes_dir();
		self.editor
			.palettes
			.files
			.iter()
			.filter(|p| p.starts_with(&dir))
			.filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
			.collect()
	}

	/// Opens the wgpu-ui Save-palette name dialog (suggests the selected user
	/// palette's name, so Save overwrites it); confirming runs `palette-save-as`.
	fn open_palette_save(&mut self) {
		let existing = self.user_palette_names();
		let suggested = self
			.editor
			.selected_palette()
			.filter(|_| self.editor.selected_palette_is_user())
			.and_then(|p| p.file_stem())
			.map_or_else(String::new, |s| s.to_string_lossy().into_owned());
		self.open_palette_name_dialog("Save Palette", &suggested, None, existing);
	}

	/// Opens the wgpu-ui Rename-palette name dialog for the selected user palette;
	/// confirming runs `palette-rename "<path>" "<name>"`.
	fn open_palette_rename(&mut self) {
		let Some(path) = self.editor.selected_palette().filter(|_| self.editor.selected_palette_is_user()).cloned()
		else {
			self.editor.console.push_line("select a saved palette to rename");
			return;
		};
		let from = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
		let existing: Vec<String> = self.user_palette_names().into_iter().filter(|n| n != &from).collect();
		self.open_palette_name_dialog("Rename Palette", &from, Some((from.clone(), path)), existing);
	}

	/// Routes a Save/Rename palette name dialog to the overlay at the editor scale.
	fn open_palette_name_dialog(
		&mut self,
		title: &str,
		initial: &str,
		from: Option<(String, std::path::PathBuf)>,
		existing: Vec<String>,
	) {
		let scale = self.editor.ui_scale as f64;
		if let Some(win) = self.win.as_mut() {
			if let Some(overlay) = &mut win.overlay {
				overlay.set_scale(scale);
				overlay.open_palette_name(title, initial, from, existing);
			}
			win.window.request_redraw();
		}
	}

	/// Opens the Remove Duplicate Templates confirm (a wgpu-ui overlay).
	fn open_dedupe(&mut self, names: &[String]) {
		let scale = self.editor.ui_scale as f64;
		if let Some(win) = self.win.as_mut() {
			if let Some(overlay) = &mut win.overlay {
				overlay.set_scale(scale);
				overlay.open_dedupe(names);
			}
			win.window.request_redraw();
		}
	}

	/// Opens the Delete Template confirm (a wgpu-ui overlay); the composed
	/// thumbnail is registered into the shared preview slot via the chrome.
	fn open_delete_template(&mut self, name: &str, footprint: (u16, u16), preview: &(Vec<u8>, u32, u32)) {
		let scale = self.editor.ui_scale as f64;
		if let Some(win) = self.win.as_mut() {
			if let (Some(overlay), Some(chrome)) = (win.overlay.as_mut(), win.passes.menu_chrome.as_mut()) {
				overlay.set_scale(scale);
				let (rgba, tw, th) = preview;
				overlay.open_delete_template(chrome, name, footprint, rgba, *tw, *th);
			}
			win.window.request_redraw();
		}
	}

	/// Opens the Rename Template dialog (a wgpu-ui overlay).
	fn open_rename_template(
		&mut self,
		from: &str,
		footprint: (u16, u16),
		existing: Vec<String>,
		preview: &(Vec<u8>, u32, u32),
	) {
		let scale = self.editor.ui_scale as f64;
		if let Some(win) = self.win.as_mut() {
			if let (Some(overlay), Some(chrome)) = (win.overlay.as_mut(), win.passes.menu_chrome.as_mut()) {
				overlay.set_scale(scale);
				let (rgba, tw, th) = preview;
				overlay.open_rename_template(chrome, from, footprint, existing, rgba, *tw, *th);
			}
			win.window.request_redraw();
		}
	}

	/// Opens the non-blocking Fix Shore window (a wgpu-ui overlay float); the
	/// run state already lives on the editor (`fix-shore-modal go` may have
	/// started it in `execute`, so the clock starts here too).
	fn open_autofix_dialog(&mut self) {
		let found = self.editor.autofix.as_ref().map_or(0, |a| a.found);
		let scale = self.editor.ui_scale as f64;
		if self.editor.autofix_running() {
			self.autofix_clock = Some(std::time::Instant::now());
		}
		if let Some(win) = self.win.as_mut() {
			if let Some(overlay) = &mut win.overlay {
				overlay.set_scale(scale);
				overlay.open_autofix(found);
			}
			win.window.request_redraw();
		}
	}

	/// Opens the Convert to Compatible Palette dialog (a wgpu-ui overlay); the
	/// rasterize run lives on the editor and re-syncs the dialog per frame.
	fn open_convert_palette_dialog(&mut self) {
		let scale = self.editor.ui_scale as f64;
		if let Some(win) = self.win.as_mut() {
			if let Some(overlay) = &mut win.overlay {
				overlay.set_scale(scale);
				overlay.open_convert_palette();
			}
			win.window.request_redraw();
		}
	}

	/// Route a press into the menu bar's retained Ui: the widget opens
	/// a dropdown, navigates the open cascade, or fires a leaf — whose action
	/// id runs its command line / ticket echo via `menu_acts`. Returns whether
	/// the menu consumed the press (the world must not also see it).
	fn menu_press(&mut self, events: &[Event], event_loop: &ActiveEventLoop) -> bool {
		let covered = self.covered();
		let consumed = self.dispatch_layer(Layer::MenuBar, events).wants_pointer();
		let fired: Vec<u64> = self.editor.menu_panel.ui.actions().to_vec();
		// The widget opened or closed its own cascade: the z-order moved, so the
		// panel under the pointer has a hover to drop (or to take back). U5.2.
		if self.covered() != covered {
			self.resync_hover_layer();
		}
		for a in fired {
			match self.editor.menu_acts.get(a as usize) {
				Some(menu::Act::Run(line)) => {
					let line = line.clone();
					match command::parse_line(&line) {
						Ok(Some(cmd)) => self.run(cmd, event_loop),
						Ok(None) => {}
						Err(e) => eprintln!("menu: {e}"),
					}
				}
				Some(menu::Act::Todo(label, ticket)) => {
					let msg = format!("{label}: not implemented yet - backlog {ticket}");
					eprintln!("{msg}");
					self.editor.console.push_line(msg);
				}
				None => {}
			}
		}
		consumed
	}

	/// Route a press into the open context menu's widget: a fired row
	/// resolves through the act table (exactly like the menu bar), an outside
	/// press dismisses. The editor model follows the widget's open state.
	fn context_press(&mut self, events: &[Event], event_loop: &ActiveEventLoop) {
		let covered = self.covered();
		self.dispatch_layer(Layer::ContextMenu, events);
		let (fired, open) = match self.win.as_mut() {
			Some(win) => {
				let host = &mut win.passes.context_menu;
				let fired: Vec<u64> = host.panel.ui.actions().to_vec();
				let open = host.root_mut().is_some_and(|cm| cm.is_open());
				(fired, open)
			}
			None => (Vec::new(), false),
		};
		if !open {
			self.editor.context_menu = None;
		}
		// A dismissed context menu uncovers the panel under the pointer, which
		// has been holding a stale (dark) hover since it opened. U5.2.
		if self.covered() != covered {
			self.resync_hover_layer();
		}
		for a in fired {
			let act = self.win.as_ref().and_then(|win| win.passes.context_acts.get(a as usize));
			match act {
				Some(menu::Act::Run(line)) => {
					let line = line.clone();
					match command::parse_line(&line) {
						Ok(Some(cmd)) => self.run(cmd, event_loop),
						Ok(None) => {}
						Err(e) => eprintln!("context menu: {e}"),
					}
				}
				Some(menu::Act::Todo(label, ticket)) => {
					let msg = format!("{label}: not implemented yet - backlog {ticket}");
					eprintln!("{msg}");
					self.editor.console.push_line(msg);
				}
				None => {}
			}
		}
	}

	/// Opens the Import WRL dialog (a wgpu-ui overlay) at its pack-picker stage;
	/// the parked import lives on the editor (`Command::ImportWrl` set it up).
	fn open_import_wrl_dialog(&mut self) {
		let Some((name, info)) = self.editor.wrlimport.as_ref().map(|r| (r.name.clone(), r.info)) else { return };
		let packs = crate::packlist::scan(&self.editor.assets_root);
		let scale = self.editor.ui_scale as f64;
		let assets_root = self.editor.assets_root.clone();
		if let Some(win) = self.win.as_mut() {
			if let (Some(overlay), Some(chrome)) = (win.overlay.as_mut(), win.passes.menu_chrome.as_mut()) {
				overlay.set_scale(scale);
				overlay.open_import_wrl(chrome, packs, &assets_root, &name, info);
			}
			win.window.request_redraw();
		}
	}

	/// Opens the Generate Random Terrain dialog (a non-blocking wgpu-ui float);
	/// the run lives on the editor, the per-generator settings come from the
	/// session memory, and Surprise Me scales to the map size.
	fn open_generate_dialog(&mut self) {
		let mem = self.editor.gen_memory.clone();
		let (mw, mh) = (self.editor.project.width as usize, self.editor.project.height as usize);
		let scale = self.editor.ui_scale as f64;
		if let Some(win) = self.win.as_mut() {
			if let Some(overlay) = &mut win.overlay {
				overlay.set_scale(scale);
				overlay.open_generate(&mem, mw, mh);
			}
			win.window.request_redraw();
		}
	}

	/// Opens the New from Image dialog (a wgpu-ui overlay); the conversion run
	/// lives on the editor (its settings were seeded by `open_newimage`).
	fn open_new_image_dialog(&mut self) {
		let (w, h) = self.editor.newimage.as_ref().map(|m| (m.opts.width_tiles, m.opts.height_tiles)).unwrap_or((1, 1));
		let scale = self.editor.ui_scale as f64;
		if let Some(win) = self.win.as_mut() {
			if let Some(overlay) = &mut win.overlay {
				overlay.set_scale(scale);
				overlay.open_new_image(w, h);
			}
			win.window.request_redraw();
		}
	}

	/// Opens the error dialog (a wgpu-ui overlay): the failed command's message
	/// in front of the user instead of buried in the console.
	fn open_error(&mut self, message: &str) {
		let scale = self.editor.ui_scale as f64;
		if let Some(win) = self.win.as_mut() {
			if let Some(overlay) = &mut win.overlay {
				overlay.set_scale(scale);
				overlay.open_error(message);
			}
			win.window.request_redraw();
		}
	}

	/// Opens the Save / Discard / Cancel unsaved-changes guard (a wgpu-ui
	/// overlay dialog). `quit` picks the quit vs close-tab command pair.
	fn open_confirm_close(&mut self, quit: bool, prompt: &str) {
		let (save_cmd, discard_cmd) =
			if quit { ("save-and-quit", "quit!") } else { ("save-and-close", "close-project!") };
		let scale = self.editor.ui_scale as f64;
		if let Some(win) = self.win.as_mut() {
			if let Some(overlay) = &mut win.overlay {
				overlay.set_scale(scale);
				overlay.open_confirm_save(
					"Unsaved Changes",
					prompt,
					"Save",
					save_cmd.into(),
					"Discard",
					discard_cmd.into(),
				);
			}
			win.window.request_redraw();
		}
	}

	/// Opens the wgpu-ui Delete-palette confirm for the selected user palette;
	/// confirming runs the same `palette-delete "<path>"` line through the
	/// command path.
	fn open_palette_delete(&mut self) {
		let Some(path) = self.editor.selected_palette().filter(|_| self.editor.selected_palette_is_user()).cloned()
		else {
			self.editor.console.push_line("select a saved palette to delete");
			return;
		};
		let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
		let command = format!("palette-delete \"{}\"", path.display());
		let scale = self.editor.ui_scale as f64;
		if let Some(win) = self.win.as_mut() {
			if let Some(overlay) = &mut win.overlay {
				overlay.set_scale(scale);
				overlay.open_confirm(
					"Delete Palette",
					&format!("Delete \"{name}\"?"),
					"This cannot be undone.",
					"Delete",
					command,
				);
			}
			win.window.request_redraw();
		}
	}

	/// Toggles the About dialog (the F1 shortcut).
	fn toggle_about(&mut self) {
		if self.about_open() {
			if let Some(win) = self.win.as_mut() {
				if let Some(overlay) = &mut win.overlay {
					overlay.hide();
				}
				win.window.request_redraw();
			}
		} else {
			self.open_about();
		}
	}

	/// Act on an [`Outcome`] - from `execute`, or from a stepped job (autofix /
	/// New-from-Image) that mutates outside the command path.
	fn act_on(&mut self, outcome: Outcome, event_loop: &ActiveEventLoop) {
		match outcome {
			Outcome::Redraw => {
				if let Some(win) = self.win.as_ref() {
					win.window.request_redraw();
				}
			}
			Outcome::DocReplaced => {
				if let Some(win) = self.win.as_mut() {
					adopt_new_document(
						&mut win.renderer,
						&mut win.passes,
						&mut self.editor,
						&mut win.uploaded_revision,
						&win.gpu.device,
						&win.gpu.queue,
						&win.render_core,
					);
					win.window.request_redraw();
				}
			}
			Outcome::Screenshot { path, crop, resize } => {
				if let Some(win) = self.win.as_mut() {
					if self.editor.revision() != win.uploaded_revision {
						refresh_renderer(&win.renderer, &win.gpu.queue, &mut self.editor);
						win.uploaded_revision = self.editor.revision();
					}
					if let Some(rgba) = self.editor.cycler.take_if_dirty() {
						sync_palette(rgba, &win.renderer, &win.passes, &win.gpu.queue);
					}
					let (w, h) = self.editor.screen;
					let editor = &mut self.editor;
					let passes = &mut win.passes;
					capture::render_to_png(
						&win.gpu.device,
						&win.gpu.queue,
						w,
						h,
						&path,
						crop,
						resize,
						|encoder, view| {
							render_frame(&win.gpu.device, &win.gpu.queue, encoder, view, editor, &win.renderer, passes);
						},
					);
				}
			}
			// The wgpu-ui overlay dialogs (embedded, not editor modals): commands
			// resolve to a request in `execute`, so menus, the console, and
			// scripts all open them through one path.
			Outcome::OpenDialog(req) => {
				use crate::state::DialogRequest as D;
				// Only one dialog is ever up, so anything but New Scenery itself
				// replaces it - and its run has to go with it, or a later
				// `scenery-import` would quietly load into a dialog that closed.
				if !matches!(req, D::SceneryNew) {
					self.editor.scenerypaint = None;
				}
				match req {
					D::About => self.open_about(),
					D::Metadata { save_after } => self.open_metadata(save_after),
					D::NewMap { shape } => self.open_newmap(shape),
					D::Resize => self.open_resize(),
					D::PaletteSave => self.open_palette_save(),
					D::PaletteRename => self.open_palette_rename(),
					D::PaletteDelete => self.open_palette_delete(),
					D::ConfirmClose { quit, prompt } => self.open_confirm_close(quit, &prompt),
					D::DedupeTemplates { names } => self.open_dedupe(&names),
					D::DeleteTemplate { name, footprint, preview } => {
						self.open_delete_template(&name, footprint, &preview)
					}
					D::RenameTemplate { from, footprint, existing, preview } => {
						self.open_rename_template(&from, footprint, existing, &preview)
					}
					D::AutoFix => self.open_autofix_dialog(),
					D::ConvertPalette => self.open_convert_palette_dialog(),
					D::NewFromImage => self.open_new_image_dialog(),
					D::Generate => self.open_generate_dialog(),
					D::ImportWrl => self.open_import_wrl_dialog(),
					D::TilePaint => self.open_tile_paint_dialog(),
					D::SceneryNew => self.open_scenery_new_dialog(),
					D::DeleteScenery { pack, id, name, placed } => {
						if let Some(overlay) = self.win.as_mut().and_then(|w| w.overlay.as_mut()) {
							overlay.open_delete_scenery(&pack, &id, &name, placed);
						}
					}
					D::RenameScenery { pack, id, from } => {
						if let Some(overlay) = self.win.as_mut().and_then(|w| w.overlay.as_mut()) {
							overlay.open_rename_scenery(&pack, &id, &from);
						}
					}
					D::MatchEdit => self.open_match_edit_dialog(),
					D::UiTests => self.open_ui_tests(),
					D::ResourceAmount => self.open_resource_amount_modal(),
					D::ConfirmExperimentalOpenSave => {
						if let Some(overlay) = self.win.as_mut().and_then(|w| w.overlay.as_mut()) {
							overlay.open_confirm_warned(
								"Experimental Feature",
								EXPERIMENTAL_SAVE_WARNING,
								EXPERIMENTAL_SAVE_BUG_WARNING,
								"Cancel",
								"I Understand",
								"file-dialog open-save".into(),
							);
						}
					}
					D::ConfirmOpenSave { message } => {
						if let Some(overlay) = self.win.as_mut().and_then(|w| w.overlay.as_mut()) {
							overlay.open_confirm_labeled(
								"Open Save",
								&message,
								"Abort",
								"Open Anyway",
								"open-save-anyway".into(),
							);
						}
					}
					D::OpenSaveError { message } => {
						self.editor.console.push_line(message.clone());
						if let Some(overlay) = self.win.as_mut().and_then(|w| w.overlay.as_mut()) {
							overlay.open_notice("Open Save", "Abort", &message);
						}
					}
					D::EditorPreferences => self.open_preferences(),
					D::EditSaveData(init) => {
						if let Some(overlay) = self.win.as_mut().and_then(|w| w.overlay.as_mut()) {
							overlay.open_save_data(*init);
						}
					}
				}
			}
			Outcome::Failed(message) => {
				eprintln!("FAILED: {message}");
				self.editor.raise_error(&message);
				self.open_error(&message);
				if let Some(win) = self.win.as_ref() {
					win.window.request_redraw();
				}
			}
			Outcome::Quit => event_loop.exit(),
			Outcome::Ok => {}
		}
	}

	/// Request a redraw if the window exists (no-op while headless / pre-init).
	fn redraw_win(&self) {
		if let Some(win) = self.win.as_ref() {
			win.window.request_redraw();
		}
	}

	/// The cursor in **logical** UI px (physical / scale): the space the chrome
	/// (menu, panels, modals, scrollbars) lays out + hit-tests in. Map-space
	/// reads (`cell_at`, pan/zoom at the cursor) use the raw physical
	/// [`cursor`](Self::cursor) instead, since the map renders at native size.
	fn lcursor(&self) -> (f32, f32) {
		let s = self.editor.ui_scale;
		(self.cursor.0 / s, self.cursor.1 / s)
	}

	/// The mouse cursor for the current pointer position: an open dialog's
	/// widgets while the overlay owns the pointer there (I-beam over its text
	/// fields), the workspace chrome affordances (splitter/edge/grip arrows,
	/// the grabbing hand mid-titlebar-drag) otherwise.
	///
	/// **The chrome affordances are gated on the z-order, not just on geometry.**
	/// A splitter, a dock edge and a float's resize grip are all *under* an open
	/// menu cascade, context menu or panel dropdown, and those layers are
	/// press-modal - they cover the whole window. Without the gate the pointer
	/// picked up a resize arrow from whatever happened to lie beneath the menu
	/// item it was resting on, which reads as if the menu were not there.
	/// [`over_frame`] is the same answer the frame's own hover uses (U6.2), so
	/// the cursor and the chrome highlight can no longer disagree.
	///
	/// A live workspace gesture is the exception, and it comes first: mid-drag
	/// the pointer belongs to the workspace wherever it has wandered to, so the
	/// grabbing hand must survive being dragged out over the map.
	fn desired_cursor(&self) -> wgpu_ui::CursorIcon {
		let (lcx, lcy) = self.lcursor();
		if let Some(overlay) = self.win.as_ref().and_then(|w| w.overlay.as_ref())
			&& overlay.visible()
			&& (overlay.blocking() || overlay.wants_pointer_at(Vec2::new(lcx, lcy)))
		{
			return overlay.cursor_icon();
		}
		let (sw, sh) = self.editor.ui_screen();
		let popup = self.popup_layer();
		if !self.editor.workspace.dragging() && !over_frame(over_at(&self.editor, popup, lcx, lcy), popup) {
			return wgpu_ui::CursorIcon::Default;
		}
		self.editor.workspace.cursor_at(lcx, lcy, sw, sh)
	}

	/// Applies [`desired_cursor`](Self::desired_cursor) to the OS window —
	/// call after pointer moves; only changes reach winit.
	fn apply_cursor(&mut self) {
		let icon = self.desired_cursor();
		if icon != self.cursor_icon {
			self.cursor_icon = icon;
			if let Some(win) = self.win.as_ref() {
				win.window.set_cursor(wgpu_ui::winit::map_cursor(icon));
			}
		}
	}

	/// The bound command for a pressed key whose *context* applies:
	/// context-specific matches (pass-value picks in the Pass Table Editor,
	/// tool switches in the map editor) beat context-free ones sharing the
	/// chord - table order never decides between contexts.
	fn bound_command(&self, key: &Key) -> Option<Command> {
		let (mut generic, mut specific) = (None, None);
		for cmd in self.bindings.lookup_all(self.modifiers, key) {
			let context = match &cmd {
				Command::PassPick { .. } | Command::PassPaint { .. } => {
					Some(matches!(self.editor.mode, state::EditorMode::Pass | state::EditorMode::LocalPass))
				}
				Command::ToolSelect { .. } => Some(self.editor.mode == state::EditorMode::Map),
				// F2 renames only when a template is selected in the explorer.
				Command::TemplateRenameModal => Some(self.editor.templates.sel.is_some()),
				_ => None,
			};
			match context {
				Some(true) if specific.is_none() => specific = Some(cmd),
				None if generic.is_none() => generic = Some(cmd),
				_ => {}
			}
		}
		specific.or(generic)
	}

	/// The select-gesture mode from the live modifiers: Shift adds, Ctrl
	/// subtracts, plain starts fresh.
	fn select_modifier(&self) -> map_core::SelectMode {
		if self.modifiers.shift_key() {
			map_core::SelectMode::Add
		} else if self.modifiers.control_key() {
			map_core::SelectMode::Subtract
		} else {
			map_core::SelectMode::Replace
		}
	}

	/// True while the open Tile Painter wants live palette cycling.
	fn painter_animating(&self) -> bool {
		self.win.as_ref().and_then(|w| w.overlay.as_ref()).is_some_and(|o| o.tile_paint_animating())
	}

	/// Opens the Edit Tile Match Data dialog over the staged model (parked in
	/// [`EditorState::matchedit_stage`] by the `match-editor` command). The
	/// dialog owns the model; the shell keeps its strip texture + atlas synced.
	fn open_match_edit_dialog(&mut self) {
		let scale = self.editor.ui_scale as f64;
		let Some(me) = self.editor.matchedit_stage.take() else { return };
		let Some(win) = self.win.as_mut() else { return };
		// The atlas is only kept fresh while this dialog is open; compose it now so
		// the strip has it on the opening frame.
		refresh_tile_atlas(&self.editor, &mut win.passes, true);
		let (Some(overlay), Some(chrome)) = (win.overlay.as_mut(), win.passes.menu_chrome.as_mut()) else {
			return;
		};
		let Some(atlas) = win.passes.tile_atlas.as_ref() else { return };
		let (pack, main, cand) = {
			let pd = me.pd();
			(pd.pack, pd.main_tile, pd.cand_tile)
		};
		let strip = compose_match_strip(&self.editor, pack, main, cand);
		let base = crate::picker::global_index(
			&self.editor.project,
			map_core::TileRef { pack: pack as u8, tile: 0, transform: map_core::Transform::default() },
		);
		overlay.set_scale(scale);
		overlay.open_match_edit(chrome, *me, &strip, (atlas.tex, atlas.count, base));
		self.match_strip_key = Some((pack, main, cand, self.editor.revision()));
		win.window.request_redraw();
	}

	/// Opens the Tile Painter dialog over [`EditorState::tilepaint`] (set by the
	/// `tile-new`/`tile-clone`/`tile-edit` commands before they request it).
	fn open_tile_paint_dialog(&mut self) {
		let scale = self.editor.ui_scale as f64;
		let Some(run) = self.editor.tilepaint.as_ref() else { return };
		if let Some(win) = self.win.as_mut() {
			if let (Some(overlay), Some(chrome)) = (win.overlay.as_mut(), win.passes.menu_chrome.as_mut()) {
				overlay.set_scale(scale);
				overlay.open_tile_paint(
					chrome,
					run,
					self.editor.cycler.rgba(),
					self.editor.animate,
					self.editor.tile_ops.clipboard.as_deref(),
				);
			}
			win.window.request_redraw();
		}
	}

	/// Opens New Scenery over [`EditorState::scenerypaint`] (set by the
	/// `scenery-new` / `scenery-import` commands before they request it).
	fn open_scenery_new_dialog(&mut self) {
		let scale = self.editor.ui_scale as f64;
		let Some(run) = self.editor.scenerypaint.as_ref() else { return };
		if let Some(win) = self.win.as_mut() {
			if let (Some(overlay), Some(chrome)) = (win.overlay.as_mut(), win.passes.menu_chrome.as_mut()) {
				overlay.set_scale(scale);
				overlay.open_scenery_new(chrome, run, &self.editor.project.palette, self.editor.cycler.rgba());
			}
			win.window.request_redraw();
		}
	}

	/// The global template index under a logical-space point, when it lands on a
	/// thumbnail in the Templates Explorer (for the right-click item menu).
	///
	/// The one question a fired action tag cannot answer — a right-click fires
	/// nothing — so it goes to the grid content widget's own domain hit test,
	/// which is the same one a pick runs through (U5.5). The panel's chrome
	/// oracle is gone; this is all that is left of `templates_click`.
	fn template_at(&self, lcx: f32, lcy: f32, sw: f32, sh: f32) -> Option<usize> {
		let (id, _) = self.editor.workspace.body_at(lcx, lcy, sw, sh)?;
		if id != "templates" {
			return None;
		}
		let i = self.win.as_ref()?.passes.templates_content.root()?.template_at(wgpu_ui::Vec2::new(lcx, lcy))?;
		self.editor.visible_templates().get(i).copied()
	}

	/// The hosted panel behind a [`Layer`], type-erased. See also the shared-borrow
	/// twin [`layer_panel`], which the focus / IME questions use — they are asked
	/// from inside `redraw`, where `win` is already mutably borrowed.
	///
	/// Every `PanelHost<W>` is a different type, so this one lookup is what lets
	/// the shell keep a single dispatch path instead of a helper per panel. The
	/// overlay and the menu bar are absent because they are not `PanelHost`s at
	/// all — [`App::dispatch_layer`] handles those two directly.
	fn panel_input(&mut self, layer: Layer) -> Option<&mut dyn PanelInput> {
		let passes = &mut self.win.as_mut()?.passes;
		Some(match layer {
			Layer::ContextMenu => &mut passes.context_menu,
			Layer::Tabs => &mut passes.tabs_strip,
			Layer::Console => &mut passes.console_view,
			Layer::Panel(id) => return panel_host(passes, id),
			_ => return None,
		})
	}

	/// The layer that owns the keyboard — see [`focus_layer`].
	fn focus_layer(&self) -> Option<Layer> {
		focus_layer(&self.editor, &self.router)
	}

	/// The panel holding an open dropdown — see [`popup_layer`]. `None` before
	/// the window exists (no panel is hosted yet, so nothing can be open).
	fn popup_layer(&self) -> Option<Layer> {
		popup_layer(&self.win.as_ref()?.passes, &self.editor)
	}

	/// Whether `layer`'s focused widget is a text field (so typing belongs to it,
	/// not to the map bindings).
	fn layer_wants_text(&self, layer: Layer) -> bool {
		let Some(win) = self.win.as_ref() else { return false };
		layer_panel(&win.passes, &self.editor, layer).is_some_and(PanelUi::wants_text_input)
	}

	/// Whether any widget in `layer` holds keyboard focus — a superset of
	/// [`layer_wants_text`](Self::layer_wants_text) (a focused list or dropdown
	/// wants keys but no typing), and what decides who owns the keyboard after a
	/// press.
	fn layer_has_focus(&self, layer: Layer) -> bool {
		let Some(win) = self.win.as_ref() else { return false };
		layer_panel(&win.passes, &self.editor, layer).is_some_and(PanelUi::has_focus)
	}

	/// Drop `layer`'s keyboard focus, for `cause`. Called when a press moves focus
	/// elsewhere (two `Ui`s must never both believe they hold the keyboard) and
	/// when Escape leaves a field.
	///
	/// The cause is what a hosted text field reads its commit off (U4.1): focus
	/// **moving** on means the edit stands, Escape means it is abandoned. Passing
	/// `Moved` here is how a click on the map or in another panel — a press this
	/// panel's `Ui` never sees — still applies what the user typed.
	fn blur_layer(&mut self, layer: Layer, cause: BlurCause) {
		match layer {
			Layer::MenuBar => self.editor.menu_panel.blur(cause),
			other => {
				if let Some(panel) = self.panel_input(other) {
					panel.blur(cause);
				}
			}
		}
	}

	/// Dispatch router-translated `events` into one UI [`Layer`] — the single
	/// dispatch path (U1.2) that replaced the eight `App::*_dispatch` helpers and
	/// their synthesized primary-only, `Modifiers::NONE`, move-less presses. The
	/// `Response` is the toolkit's own verdict: `wants_pointer` / `wants_keyboard`
	/// say whether the shell must withhold the event from the layers under this
	/// one (and from the map).
	fn dispatch_layer(&mut self, layer: Layer, events: &[Event]) -> wgpu_ui::Response {
		if events.is_empty() {
			return wgpu_ui::Response::default();
		}
		let response = match layer {
			// The overlay buffers its events and drains them at render time (it
			// owns a `Ui` per dialog), so it reports no `Response` here at all -
			// there is nothing to read a capture off. It needs none: a modal
			// already swallows every event via the intercept above, and the
			// non-blocking float gates on `wants_pointer_at`. The caller has
			// decided it takes this one, so say so.
			Layer::Overlay => {
				if let Some(overlay) = self.win.as_mut().and_then(|w| w.overlay.as_mut()) {
					overlay.dispatch_events(events);
				}
				// The caller has already decided the dialog takes this one, so say
				// so on its behalf.
				return wgpu_ui::Response { pointer: true, keyboard: true, ..wgpu_ui::Response::default() };
			}
			// The menu bar lives on `EditorState` (it outlives the window).
			Layer::MenuBar => self.editor.menu_panel.dispatch_events(events),
			other => match self.panel_input(other) {
				Some(panel) => panel.dispatch(events),
				None => return wgpu_ui::Response::default(),
			},
		};
		// A widget that begins a drag captures the pointer, and its `Ui` reports it
		// here (U1.3). The router then feeds *this* layer every later pointer event
		// until it lets go - which is what makes a drag survive leaving the widget,
		// and the panel.
		self.router.set_capture(layer, response.capturing);
		// Focus follows the primary press (U1.4): whichever layer a press lands in
		// owns the keyboard afterwards, and a press that focuses nothing gives it
		// up. The layer losing it is blurred, so two `Ui`s never both believe they
		// hold the keyboard.
		if events
			.iter()
			.any(|e| matches!(e, Event::PointerButton { button: PointerButton::Primary, pressed: true, .. }))
		{
			let now = self.layer_has_focus(layer).then_some(layer);
			if let Some(lost) = self.router.refocus(now) {
				self.blur_layer(lost, BlurCause::Moved);
			}
		}
		response
	}

	/// Poll the tab strip for a fired select/close after a release dispatch —
	/// the action tag its `Ui` collected, decoded by the strip (U6.1).
	fn tabs_outcome(&self) -> Option<tabs::TabAct> {
		let ui = &self.win.as_ref()?.passes.tabs_strip.panel.ui;
		ui.actions().iter().copied().find_map(tabs::act_of)
	}

	/// Poll the minimap for a fired mode change after a release dispatch — the
	/// action tag its `Ui` collected, mapped back through `Mode::ALL` (U5.3).
	fn minimap_outcome(&self) -> Option<minimap::Mode> {
		let ui = &self.win.as_ref()?.passes.minimap_overlay.panel.ui;
		ui.actions().iter().copied().find_map(minimap::MinimapOverlay::mode_of)
	}

	/// The outcomes a panel can fire on the *press*, drained right after a press
	/// dispatch — from the body-press arm and from the capture branch (a press
	/// routed to the panel holding the pointer grab, e.g. its own open
	/// dropdown). A palette swatch selects and an HSL bar starts its drag on the
	/// press (`CommitPolicy::PressFire`), the minimap's press both pans and
	/// opens a drag capture, and a hosted `Select` picks the row under the
	/// press. The last one matters because `Ui::actions` lives for exactly one
	/// dispatch — leaving a pick for the release poll would let the release
	/// dispatch clear it first.
	fn drain_panel_press(&mut self, id: &'static str, event_loop: &ActiveEventLoop) {
		self.drain_palette(id, event_loop);
		match id {
			"minimap" => self.drain_minimap_pan(event_loop),
			"toolbox" => self.drain_toolbox(event_loop),
			"templates" => self.drain_templates(event_loop),
			// The Tile Explorer's three dropdowns need this exactly as much as
			// the templates' two do — U5.6 wrote `drain_picker` to be called
			// after both dispatches and then only called it after the release,
			// so a picked tileset / filter / size was cleared before anyone
			// read it. (The units panel needs no arm here: nothing in its tree
			// fires on a press.)
			"tiles" => self.drain_picker(event_loop),
			// The Scenery panel's pack / preview-size dropdowns, same rule.
			"scenery" => self.drain_scenery(event_loop),
			// Unit Properties has three of them (facing / orders / turret), so
			// it needs the same arm — U5.8's tree does fire on a press.
			"unitprops" => {
				self.drain_unitprops(event_loop);
				self.drain_unitprops_commits(event_loop);
			}
			_ => {}
		}
	}

	/// Drain the minimap content widget's pan drag into a `PanTo`. Polled after
	/// every dispatch that could have moved it: the press that starts the drag,
	/// and each move while it holds the pointer capture.
	fn drain_minimap_pan(&mut self, event_loop: &ActiveEventLoop) {
		let pan = self.win.as_mut().and_then(|w| w.passes.minimap_overlay.root_mut()).and_then(|o| o.take_pan());
		if let Some((x, y)) = pan {
			self.run(Command::PanTo { x, y }, event_loop);
		}
	}

	/// Poll the toolbox for a fired hit after a release dispatch. Since U5.4
	/// every part of the panel — the group keys, both dropdowns' picked options
	/// and the orientation cells — comes back as an **action tag** its `Ui`
	/// collected; `hit_of` maps it through `GROUPS`, so the shell reads one
	/// channel and re-types no command line.
	fn toolbox_outcome(&self) -> Option<toolbox::Hit> {
		let ui = &self.win.as_ref()?.passes.toolbox_content.panel.ui;
		ui.actions().iter().copied().find_map(toolbox::hit_of)
	}

	/// Poll the Save Toolbox for a fired key's command line after a release
	/// dispatch. Since U5.2 the keys are stock `wgpu_ui::Button`s, so what comes
	/// back is the **action tag** its `Ui` collected; `command_of` maps it
	/// through `GROUPS` to the line that key was built from.
	fn savetools_outcome(&self) -> Option<&'static str> {
		let ui = &self.win.as_ref()?.passes.savetools_content.panel.ui;
		ui.actions().iter().copied().find_map(savetools::command_of)
	}

	/// The command line the Pass Types Palette fired, if any — one poll for the
	/// whole panel, mapped back through its own `GROUPS` (the tally rows are
	/// labels, so only the four swatches can fire).
	fn passtools_outcome(&self) -> Option<&'static str> {
		let ui = &self.win.as_ref()?.passes.passtools_content.panel.ui;
		ui.actions().iter().copied().find_map(passtools::command_of)
	}

	/// Poll the Unit Properties panel for a fired [`unitprops::Action`]. Since
	/// U5.8 the panel is a real widget tree, so what comes back is the **action
	/// tag** its `Ui` collected; `action_of` maps it through the panel's one tag
	/// space.
	fn unitprops_outcome(&self) -> Option<unitprops::Action> {
		let ui = &self.win.as_ref()?.passes.unitprops_content.panel.ui;
		ui.actions().iter().copied().find_map(unitprops::action_of)
	}

	/// Run whatever the Unit Properties panel fired, if anything.
	///
	/// Called after **both** dispatches: its three dropdowns are hosted
	/// `Select`s, which commit on the **press**, and `Ui::actions` lives for
	/// exactly one dispatch — leaving a pick for the release poll would let the
	/// release dispatch clear it first (the U5.6 bug U5.7 found). Everything else
	/// in the tree fires on release-inside, so the second call is the one that
	/// sees those.
	fn drain_unitprops(&mut self, event_loop: &ActiveEventLoop) {
		let Some(action) = self.unitprops_outcome() else { return };
		match action {
			// Toggle the values section's advanced (static) rows — panel state,
			// no command.
			unitprops::Action::ToggleAdvanced => {
				self.editor.unitprops_advanced = !self.editor.unitprops_advanced;
				self.redraw_win();
			}
			other => {
				if let Some(line) = self.unitprops_command(other) {
					if let Ok(Some(cmd)) = command::parse_line(&line) {
						self.run(cmd, event_loop);
					}
				}
			}
		}
	}

	/// Resolve a fired Unit Properties [`unitprops::Action`] to an `object-edit`
	/// command line against the *current* selection (read live, so it stays correct
	/// after an intervening edit/undo). `SelectToggle` (opens a dropdown) and
	/// `Edit` (opens a modal) run no command — the caller handles those.
	fn unitprops_command(&self, action: unitprops::Action) -> Option<String> {
		let o = self.editor.project.objects.get(self.editor.selected_object?)?;
		Some(match action {
			unitprops::Action::Team(t) => format!("object-edit team {t}"),
			// Toggle the half-edge bit against the live mask (robust to intervening edits).
			unitprops::Action::ConnectorToggle(bit) => {
				format!("object-edit connectors {}", o.props.connectors ^ bit)
			}
			// A dropdown pick writes the field it belongs to (U3.4).
			unitprops::Action::SelectPick(kind, v) => {
				let field = match kind {
					unitprops::SelectKind::Facing => "facing",
					unitprops::SelectKind::Orders => "orders",
					unitprops::SelectKind::Turret => "turret",
				};
				format!("object-edit {field} {v}")
			}
			// The advanced toggle is panel state, handled by the caller (it runs
			// no command); text edits arrive as a `Commit`.
			unitprops::Action::ToggleAdvanced => return None,
		})
	}

	/// The `object-edit` / `object-values` command line for an in-place text
	/// [`Commit`] (item 8), against the live selection. `None` when nothing is
	/// selected; the command handler validates + clamps the value.
	fn unitprops_commit_command(&self, commit: unitprops::Commit) -> Option<String> {
		self.editor.selected_object?;
		Some(match commit {
			unitprops::Commit::Field(field, text) => {
				let t = text.trim();
				match field {
					// Quote the name (spaces kept; embedded quotes stripped so the
					// tokenizer sees one argument), matching the old modal.
					unitprops::Field::Name => format!("object-edit name \"{}\"", t.replace('"', "")),
					unitprops::Field::Hits => format!("object-edit hits {t}"),
					unitprops::Field::Ammo => format!("object-edit ammo {t}"),
					unitprops::Field::Storage => format!("object-edit storage {t}"),
					unitprops::Field::Disabled => format!("object-edit disabled {t}"),
				}
			}
			unitprops::Commit::Value(attr, text) => format!("object-values {attr} {}", text.trim()),
		})
	}

	/// Drain the Unit Properties panel's queued in-place edit commits (Enter /
	/// focus-out) into `object-edit` / `object-values` commands.
	fn drain_unitprops_commits(&mut self, event_loop: &ActiveEventLoop) {
		while let Some(commit) = self.win.as_mut().and_then(|w| w.passes.unitprops_content.root_mut()?.take_commit()) {
			if let Some(line) = self.unitprops_commit_command(commit) {
				if let Ok(Some(cmd)) = command::parse_line(&line) {
					self.run(cmd, event_loop);
				}
			}
		}
	}

	/// Open the exact-entry editor for the resource brush's amount (S5.4), seeded
	/// with the current brush amount; OK runs `resource-brush amount N`.
	fn open_resource_amount_modal(&mut self) {
		let initial = self.editor.resource_amount.to_string();
		let scale = self.editor.ui_scale as f64;
		if let Some(win) = self.win.as_mut() {
			if let Some(overlay) = win.overlay.as_mut() {
				overlay.set_scale(scale);
				overlay.open_resource_amount(&initial);
			}
			win.window.request_redraw();
		}
	}

	/// Poll the Templates Explorer for a fired action. Since U5.5 every part of
	/// the panel — the command keys, both dropdowns' picked options and the
	/// thumbnail grid — comes back as an **action tag** its `Ui` collected;
	/// `action_of` maps it back, so the shell reads one channel for the whole
	/// panel.
	fn templates_outcome(&self) -> Option<templates_panel::Action> {
		let ui = &self.win.as_ref()?.passes.templates_content.panel.ui;
		ui.actions().iter().copied().find_map(templates_panel::action_of)
	}

	/// Run whatever the Templates Explorer fired, if anything.
	///
	/// Called after **both** dispatches, like [`App::drain_palette`]: the command
	/// keys and the thumbnail grid commit on release-inside, but a hosted
	/// `Select` picks the row under the **press** — and `Ui::actions` lives only
	/// for the dispatch that produced it, so a pick left for the release poll
	/// would be cleared by the release dispatch itself before anyone read it.
	fn drain_templates(&mut self, event_loop: &ActiveEventLoop) {
		let Some(action) = self.templates_outcome() else { return };
		match action {
			templates_panel::Action::Pick(i) => {
				let visible = self.editor.visible_templates();
				if let Some(&g) = visible.get(i) {
					// Select the exact entry clicked: names can repeat across
					// tilesets, so `template-pick` resolves by selection, not a bare
					// (maybe ambiguous) name.
					self.editor.templates.sel = Some(g);
					let name = self.editor.templates.entries[g].name.clone();
					self.run(Command::TemplatePick { name }, event_loop);
				}
			}
			templates_panel::Action::Save => self.run(Command::TemplateSave { name: None }, event_loop),
			templates_panel::Action::Import => {
				self.run(Command::FileDialog { purpose: command::FilePurpose::ImportTemplate }, event_loop);
			}
			templates_panel::Action::Delete => self.run(Command::TemplateDeleteModal, event_loop),
			templates_panel::Action::Dedupe => self.run(Command::TemplateDedupeModal, event_loop),
			templates_panel::Action::Rename => self.run(Command::TemplateRenameModal, event_loop),
			templates_panel::Action::Explore => self.run(Command::TemplateExplore, event_loop),
			// The two header dropdowns report only their pick (U3.6).
			templates_panel::Action::SizeOption(i) => {
				if let Some(&(px, _)) = templates_panel::PREVIEW_SIZES.get(i) {
					self.editor.templates.cell = px;
				}
				self.redraw_win();
			}
			templates_panel::Action::TilesetOption(i) => {
				// Option 0 = all; 1.. = label `i-1`. Store by label.
				let labels = self.editor.template_tilesets();
				self.editor.templates.tileset = i.checked_sub(1).and_then(|li| labels.get(li).cloned());
				// A different pack re-lists the grid: back to the top.
				if let Some(c) = self.win.as_mut().and_then(|w| w.passes.templates_content.root_mut()) {
					c.scroll_to_top();
				}
				self.redraw_win();
			}
		}
	}

	/// Run whatever the toolbox fired, if anything — the same two-dispatch rule
	/// as [`App::drain_templates`]: its group keys and orientation cells commit
	/// on release-inside, its two dropdowns on the press.
	fn drain_toolbox(&mut self, event_loop: &ActiveEventLoop) {
		let Some(hit) = self.toolbox_outcome() else { return };
		match hit {
			// A group key, or a picked dropdown option: the same `GROUPS` row
			// either way. Opening, dismissing and closing a list are the widget's
			// own since U3.3.
			toolbox::Hit::Key(button) => {
				if let Ok(Some(cmd)) = command::parse_line(button.cmd) {
					self.run(cmd, event_loop);
				}
			}
			toolbox::Hit::Orient(i) => {
				// Re-orient the armed tile/stamp; a greyed (disallowed) cell is a
				// no-op.
				let t = toolbox::orient_transform(i);
				if self.editor.orient_allowed(t) {
					self.run(Command::Orient { rot: t.rot, mirror: t.mirror }, event_loop);
				}
			}
		}
	}

	/// Poll the Tile Explorer for a fired action. Since U5.6 every part of the
	/// panel — the four command keys, all three dropdowns' picked options and
	/// the tile grid — comes back as an **action tag** its `Ui` collected;
	/// `action_of` maps it back, so the shell reads one channel for the whole
	/// panel.
	fn picker_outcome(&self) -> Option<picker::Action> {
		let ui = &self.win.as_ref()?.passes.picker_content.panel.ui;
		ui.actions().iter().copied().find_map(picker::action_of)
	}

	/// Run whatever the Tile Explorer fired, if anything.
	///
	/// Called after **both** dispatches, like [`App::drain_templates`]: the
	/// command keys and the tile grid commit on release-inside, but a hosted
	/// `Select` picks the row under the **press** — and `Ui::actions` lives only
	/// for the dispatch that produced it, so a pick left for the release poll
	/// would be cleared by the release dispatch itself before anyone read it.
	fn drain_picker(&mut self, event_loop: &ActiveEventLoop) {
		let Some(action) = self.picker_outcome() else { return };
		match action {
			// An index into the filtered list, resolved against live state here
			// rather than at press time - the same re-hit-at-release robustness the
			// old shell-armed path had.
			picker::Action::Pick(i) => {
				let ts = picker::tileset_index(&self.editor.project, self.editor.picker.tileset.as_deref());
				let id = picker::items(&self.editor.project, self.editor.picker.filter, ts)
					.get(i)
					.map(|it| it.id.to_string());
				if let Some(id) = id {
					// Carry the current transform (the `:suffix`) onto the newly picked
					// tile, so selecting another tile keeps the rotation/flip the transform
					// tool applied instead of snapping back to identity. Any single tile
					// accepts any transform, so this is always valid.
					let spec = match self.editor.active_tile().and_then(|t| t.split_once(':')) {
						Some((_, xf)) => format!("{id}:{xf}"),
						None => id,
					};
					self.run(Command::Tile { spec: Some(spec) }, event_loop);
					// Choosing a tile arms the pencil (which cancels the select / other
					// tools) and drops any armed template stamp, so the click unambiguously
					// means "paint this tile".
					self.run(Command::StampCancel, event_loop);
					self.run(Command::ToolSelect { name: "pencil".into() }, event_loop);
				}
			}
			// The three header dropdowns report only their pick; opening, dismissing
			// and "one open at a time" are the widgets' own (U3.5).
			picker::Action::SetTileset(i) => {
				// Option 0 = all packs; 1.. = pack `i-1` by name (stored by name so it
				// survives switching between open maps).
				let name = i.checked_sub(1).and_then(|pi| self.editor.project.packs.get(pi).map(|pk| pk.name.clone()));
				let p = &mut self.editor.picker;
				p.tileset = name;
				// A different pack re-lists the grid: back to the top.
				p.scroll_request = Some(picker::ScrollRequest::To(0.0));
				self.redraw_win();
			}
			picker::Action::SetFilter(i) => {
				if let Some(f) = picker::Filter::ALL.get(i) {
					self.run(Command::PickerFilter { name: f.name().into() }, event_loop);
				}
			}
			picker::Action::SetSize(i) => {
				let px = picker::SIZES[i] as u32;
				self.run(Command::PickerSize { size: px.to_string() }, event_loop);
			}
			picker::Action::New => self.run(Command::TilePaintNew, event_loop),
			picker::Action::Clone => self.run(Command::TilePaintClone, event_loop),
			picker::Action::Edit => self.run(Command::TilePaintEdit, event_loop),
			picker::Action::Delete => self.run(Command::TileDelete, event_loop),
		}
	}

	/// Poll the units panel for a fired action. Since U5.7 every part of it —
	/// the five team swatches, the eraser toggle and the sprite grid — comes
	/// back as an **action tag** its `Ui` collected; `action_of` maps it back,
	/// so the shell reads one channel for the whole panel. Everything here
	/// commits on release-inside (there is no `Select` in this tree), so unlike
	/// the tiles / templates explorers one poll after the release is enough.
	fn units_outcome(&self) -> Option<units::Action> {
		let ui = &self.win.as_ref()?.passes.units_content.panel.ui;
		ui.actions().iter().copied().find_map(units::action_of)
	}

	/// The Scenery panel's fired action, if any.
	fn scenery_outcome(&self) -> Option<scenery::Action> {
		let ui = &self.win.as_ref()?.passes.scenery_content.panel.ui;
		ui.actions().iter().copied().find_map(scenery::action_of)
	}

	/// Run whatever the Scenery panel fired, if anything.
	///
	/// Called after **both** dispatches, for the same reason as
	/// [`App::drain_templates`]: the thumbnail grid commits on release-inside,
	/// but a hosted `Select` picks the row under the **press**, and `Ui::actions`
	/// lives only for the dispatch that produced it.
	fn drain_scenery(&mut self, event_loop: &ActiveEventLoop) {
		let Some(action) = self.scenery_outcome() else { return };
		match action {
			// The grid indexes the *listed* pieces; the document and the tool
			// speak flat indices, so the mapping is resolved here, against the
			// live filter, rather than baked into the tag.
			scenery::Action::Pick(i) => {
				let visible = scenery::visible_pieces(&self.editor.project, self.editor.scenery_pack.as_deref());
				if let Some(&flat) = visible.get(i) {
					self.run(Command::SceneryPick { index: Some(flat) }, event_loop);
				}
			}
			scenery::Action::New => self.run(Command::SceneryNew, event_loop),
			scenery::Action::Import => self.run(Command::SceneryImport { path: None }, event_loop),
			scenery::Action::Clone => self.run(Command::SceneryClone, event_loop),
			scenery::Action::Edit => self.run(Command::SceneryEdit, event_loop),
			scenery::Action::Export => self.run(Command::SceneryExport { path: None }, event_loop),
			scenery::Action::Delete => self.run(Command::SceneryDelete { force: false }, event_loop),
			scenery::Action::Rename => self.run(Command::SceneryRename { name: None }, event_loop),
			scenery::Action::SizeOption(i) => {
				if let Some(&(px, _)) = scenery::PREVIEW_SIZES.get(i) {
					self.editor.scenery_cell = px;
				}
				self.redraw_win();
			}
			// Option 0 = all; 1.. = library `i-1`. Stored by name.
			scenery::Action::BlendOption(i) => {
				if let Some(&mode) = map_core::SceneryBlend::ALL.get(i) {
					self.run(Command::SceneryBlendMode { index: None, mode }, event_loop);
				}
			}
			scenery::Action::PackOption(i) => {
				let packs = scenery::pack_names(&self.editor.project);
				self.editor.scenery_pack = i.checked_sub(1).and_then(|li| packs.get(li).cloned());
				// A different pack re-lists the grid: back to the top.
				if let Some(c) = self.win.as_mut().and_then(|w| w.passes.scenery_content.root_mut()) {
					c.scroll_to_top();
				}
				self.redraw_win();
			}
		}
	}

	/// Run whatever the palette panels fired, if anything.
	///
	/// Called after **both** dispatches: a swatch selects on the *press*
	/// (`CommitPolicy::PressFire`) and a saved row is picked there too, while the
	/// toolbar keys arm and fire on release-inside — and `Ui::actions` lives for
	/// exactly one dispatch, so a pick left for the release poll would be cleared
	/// by the release dispatch itself.
	///
	/// The **edit gestures** are a second, ordered channel: they carry colours,
	/// which no `u64` action tag can. Drained in a loop, because one dispatch can
	/// produce a whole `Begin` → `Colors` → `End` sequence (a fast click).
	fn drain_palette(&mut self, id: &str, event_loop: &ActiveEventLoop) {
		match id {
			"palette" => {
				if let Some(action) = self.palette_outcome() {
					self.palette_action(action, event_loop);
				}
				self.drain_palette_edits(event_loop);
			}
			"wrlpalette" => {
				if let Some(action) = self.wrlpalette_outcome() {
					self.wrlpalette_action(action, event_loop);
				}
			}
			_ => {}
		}
	}

	/// Poll the Color Palette panel for a fired action — the action tag its `Ui`
	/// collected, mapped back through `action_of`, so the shell reads one channel
	/// for everything discrete the panel produces (U5.9).
	fn palette_outcome(&self) -> Option<palette_panel::Action> {
		let ui = &self.win.as_ref()?.passes.palette_content.panel.ui;
		ui.actions().iter().copied().find_map(palette_panel::action_of)
	}

	/// Poll the WRL Internal Palette panel for a fired action.
	fn wrlpalette_outcome(&self) -> Option<palette_panel::Action> {
		let ui = &self.win.as_ref()?.passes.wrlpalette_content.panel.ui;
		ui.actions().iter().copied().find_map(palette_panel::action_of)
	}

	/// Apply the Color Palette's queued colour edits. The panel resolves each
	/// gesture against the selection itself — the slots, and the absolute colours
	/// to write — so the whole of the shell's former drag lifecycle
	/// (its drag enum, its baseline capture, its `CursorMoved` tail) is one
	/// bracketing rule: `Begin` opens exactly one undo stroke and `End` closes it.
	fn drain_palette_edits(&mut self, event_loop: &ActiveEventLoop) {
		loop {
			let Some(edit) = self.win.as_mut().and_then(|w| w.passes.palette_content.root_mut()?.take_edit()) else {
				return;
			};
			match edit {
				palette_panel::Edit::Begin => self.run(Command::Stroke { begin: true }, event_loop),
				palette_panel::Edit::Colors(colors) => {
					for (slot, rgb) in colors {
						self.run(Command::SetColor { slot, rgb }, event_loop);
					}
				}
				palette_panel::Edit::End => self.run(Command::Stroke { begin: false }, event_loop),
			}
		}
	}

	/// Run one fired Color Palette action.
	///
	/// A swatch reports only *which* slot: a stock `ColorButton` carries no
	/// modifier state, and the shell is where the live one lives. So the three
	/// selection gestures are resolved here — Ctrl toggles the slot into the
	/// multi set, Shift extends the range, a plain click replaces both.
	fn palette_action(&mut self, action: palette_panel::Action, event_loop: &ActiveEventLoop) {
		match action {
			palette_panel::Action::Select(slot) => self.palette_select(slot, event_loop),
			palette_panel::Action::ShowSaved(saved) => self.run(Command::PaletteTab { saved }, event_loop),
			palette_panel::Action::Save => self.run(Command::PaletteSaveModal, event_loop),
			palette_panel::Action::Edit => self.run(Command::PaletteRenameModal, event_loop),
			palette_panel::Action::Delete => self.run(Command::PaletteDeleteModal, event_loop),
			palette_panel::Action::Import => {
				self.run(Command::FileDialog { purpose: command::FilePurpose::ImportPalette }, event_loop);
			}
			palette_panel::Action::Export => {
				self.run(Command::FileDialog { purpose: command::FilePurpose::ExportPalette }, event_loop);
			}
			palette_panel::Action::LoadSaved(i) => {
				if let Some(path) = self.editor.palettes.files.get(i).cloned() {
					self.editor.palettes.sel = Some(i);
					self.run(Command::PaletteLoad { path }, event_loop);
				}
			}
			palette_panel::Action::Cycle(on) => self.run(Command::Animate { on: Some(on) }, event_loop),
			palette_panel::Action::CycleToggle => self.run(Command::Animate { on: None }, event_loop),
		}
	}

	/// A swatch click, under whichever modifier is held.
	fn palette_select(&mut self, slot: u16, event_loop: &ActiveEventLoop) {
		let index = slot as u8;
		let command = if self.modifiers.shift_key() && self.editor.active_color.is_some() {
			Command::ColorTo { index }
		} else if self.modifiers.control_key() {
			Command::ColorToggle { index }
		} else {
			Command::Color { index }
		};
		self.run(command, event_loop);
	}

	/// Run one fired WRL Internal Palette action. The bare panel is read-only:
	/// selection (no Ctrl multi-select — that edits) and its cycle/static header
	/// keys, nothing else. Its tree has no toolbar, no saved list and no editing
	/// rows, so nothing else can reach here.
	fn wrlpalette_action(&mut self, action: palette_panel::Action, event_loop: &ActiveEventLoop) {
		match action {
			palette_panel::Action::Select(slot) => {
				let index = slot as u8;
				let command = if self.modifiers.shift_key() && self.editor.active_color.is_some() {
					Command::ColorTo { index }
				} else {
					Command::Color { index }
				};
				self.run(command, event_loop);
			}
			palette_panel::Action::Cycle(on) => self.run(Command::Animate { on: Some(on) }, event_loop),
			_ => {}
		}
	}

	/// Route a keyboard / text / IME batch to the layer that owns the keyboard,
	/// returning whether the shell must **stop** there — `true` means the
	/// keystroke belonged to the focused layer and the app keymap must not also
	/// see it (U1.4).
	///
	/// This is the whole keyboard path now. Before U1.4 there were two, each with
	/// its own gate — `console_key` (on `console.is_open()`) and
	/// `route_unitprops_key` (on `unitprops_wants_text_input`) — so *no other*
	/// panel widget's keyboard behavior existed: `List` Up/Down, `Select` keys,
	/// and Tab between fields were unreachable by construction.
	fn route_keyboard(&mut self, event: &WindowEvent, events: &[Event], event_loop: &ActiveEventLoop) -> bool {
		// Console-level chords (close / submit / history / scrollback) are app
		// accelerators, not text, and are answered before the hosted field sees
		// anything. They stay host-side by design: they are bindings that happen to
		// apply only in this context, not widget behavior.
		if self.editor.console.is_open()
			&& let WindowEvent::KeyboardInput { event: key, .. } = event
			&& key.state.is_pressed()
			&& self.console_chord(&key.logical_key, event_loop)
		{
			return true;
		}
		let Some(focus) = self.focus_layer() else { return false };
		if focus == Layer::Console {
			// Opening the console takes the keyboard from whatever held it, which
			// has to let go visually too - two fields must never look focused at
			// once. Opening it by key focuses nothing on its own, so seed the
			// field here, on the way in (a click into the console focuses it too,
			// since U4.5).
			//
			// Never the console itself, though: a click in the band makes it the
			// router's focus as well (focus follows the press, U1.4), and blurring
			// it here would drop its live IME preedit on every keystroke and then
			// re-seed the field from `focus_first` below.
			if let Some(lost) = self.router.refocus(None).filter(|&l| l != Layer::Console) {
				self.blur_layer(lost, BlurCause::Moved);
			}
			if !self.layer_wants_text(focus)
				&& let Some(win) = self.win.as_mut()
			{
				win.passes.console_view.panel.ui.focus_first();
			}
		}
		let response = self.dispatch_layer(focus, events);
		// Enter (or a focus move) may have queued an in-place edit commit (item 8).
		self.drain_unitprops_commits(event_loop);
		// …and Enter in the console field is a submitted command line (U4.5).
		if focus == Layer::Console {
			self.console_submit(event_loop);
		}
		let typing = self.layer_wants_text(focus);

		// RULE 2. The shell's Escape cascade (menu → context menu → stamp → tool →
		// selection) runs only **after** the focused layer declines Escape - and
		// `TextInput` declines it always, because a single-line field has no
		// editing meaning for Escape. So without this rule, Esc pressed with the
		// caret blinking in a Unit Properties box would fall straight through and
		// cancel an armed stamp. It leaves the field instead; the cascade gets the
		// *next* Esc.
		if escape_press(events) && !response.wants_keyboard() {
			if typing {
				// Escape abandons the edit - the one blur that is not a commit.
				self.blur_layer(focus, BlurCause::Cancelled);
				self.router.refocus(None);
				return true;
			}
			return false;
		}

		// RULE 1. Gate the app keymap on the focused layer's `wants_text_input()`,
		// **not** on `Response::keyboard`: `TextInput` deliberately does not consume
		// `Key::Character` (wgpu-ui/src/textedit.rs - the chord arms match specific
		// keys and fall through otherwise), so a bare `p` typed into a unit name
		// would otherwise also reach the bindings and arm the pencil. A key the
		// layer *did* consume never reaches them either, hence the `||`.
		response.wants_keyboard() || typing
	}

	/// A console-level key chord (not text editing); returns whether it matched.
	fn console_chord(&mut self, key: &Key, event_loop: &ActiveEventLoop) -> bool {
		match key {
			// Close on the same keys that open it (Esc / F1 / backtick).
			Key::Named(NamedKey::Escape) | Key::Named(NamedKey::F1) => {
				self.run(Command::Console { on: Some(false) }, event_loop);
			}
			Key::Character(c) if c.as_str() == "`" => {
				self.run(Command::Console { on: Some(false) }, event_loop);
			}
			// Enter is *not* here: the hosted field consumes it and reports a
			// commit, which `console_submit` polls after the dispatch (U4.5) —
			// exactly how a dialog's submit-on-Enter works. Up/Down recall
			// history into the field.
			Key::Named(NamedKey::ArrowUp) => {
				if let Some(text) = self.editor.console.history_prev() {
					self.console_set_input(text);
				}
			}
			Key::Named(NamedKey::ArrowDown) => {
				if let Some(text) = self.editor.console.history_next() {
					self.console_set_input(text);
				}
			}
			// PgUp/PgDn scroll the scrollback; Ctrl+Home/End jump to its ends.
			Key::Named(NamedKey::PageUp) => self.editor.console.scroll_lines(5),
			Key::Named(NamedKey::PageDown) => self.editor.console.scroll_lines(-5),
			Key::Named(NamedKey::Home) if self.modifiers.control_key() => self.editor.console.scroll_lines(i32::MAX),
			Key::Named(NamedKey::End) if self.modifiers.control_key() => self.editor.console.scroll_lines(i32::MIN),
			_ => return false,
		}
		true
	}

	/// Poll the console for a line Enter submitted (the field cleared itself),
	/// echo + record it, then run the parsed command (errors land in the
	/// scrollback). Called after every dispatch to the console layer.
	fn console_submit(&mut self, event_loop: &ActiveEventLoop) {
		let Some(line) = self.win.as_mut().and_then(|w| w.passes.console_view.root_mut()?.take_submit()) else {
			return;
		};
		if let Some(line) = self.editor.console.submit(&line) {
			match command::parse_line(&line) {
				Ok(Some(cmd)) => self.run(cmd, event_loop),
				Ok(None) => {}
				Err(e) => self.editor.console.push_line(format!("error: {e}")),
			}
		}
	}

	/// Push recalled history text into the hosted input field (caret at the end).
	fn console_set_input(&mut self, text: String) {
		if let Some(c) = self.win.as_mut().and_then(|w| w.passes.console_view.root_mut()) {
			c.set_input(text);
		}
	}

	fn redraw(&mut self, event_loop: &ActiveEventLoop) {
		// Animation: advance the working palette by real frame time.
		if self.editor.animate || self.painter_animating() {
			let dt = self.last_frame.elapsed().as_secs_f32().min(0.25);
			self.editor.tick(dt);
		}
		self.last_frame = std::time::Instant::now();

		self.step_background_jobs(event_loop);
		self.render_and_present(event_loop);
	}

	/// A deferred Tile Painter commit: a success clears the run + closes the
	/// dialog and rebuilds the atlas (DocReplaced); a failure lands back in
	/// the still-open dialog as an inline line (the edits survive).
	fn run_tile_commit(&mut self, act: TileCommitAct, event_loop: &ActiveEventLoop) {
		let outcome = self.editor.tile_paint_commit(act.id, act.pass, act.pack);
		match outcome {
			Outcome::Failed(message) => {
				self.editor.console.push_line(message.clone());
				if let Some(overlay) = self.win.as_mut().and_then(|w| w.overlay.as_mut()) {
					overlay.tile_paint_error(&message);
				}
			}
			outcome => {
				if let Some(overlay) = self.win.as_mut().and_then(|w| w.overlay.as_mut()) {
					overlay.hide();
				}
				self.act_on(outcome, event_loop);
			}
		}
	}

	/// A deferred New Scenery commit, on exactly the Tile Painter's terms.
	fn run_scenery_commit(&mut self, act: SceneryCommitAct, event_loop: &ActiveEventLoop) {
		let outcome = self
			.editor
			.scenery_commit(act.pack, act.id, act.name, act.sprite, act.pass, act.cells, act.relief, act.height);
		match outcome {
			Outcome::Failed(message) => {
				self.editor.console.push_line(message.clone());
				if let Some(overlay) = self.win.as_mut().and_then(|w| w.overlay.as_mut()) {
					overlay.scenery_new_error(&message);
				}
			}
			outcome => {
				if let Some(overlay) = self.win.as_mut().and_then(|w| w.overlay.as_mut()) {
					overlay.hide();
				}
				self.act_on(outcome, event_loop);
			}
		}
	}

	/// A deferred Import WRL verb: run the match/finish, then reflect the
	/// parked run into the dialog — a review switches it to the unmapped
	/// stage, a closed run (clean match / finish / failure) hides it.
	fn run_wrl_act(&mut self, act: WrlAct, event_loop: &ActiveEventLoop) {
		let outcome = match act {
			WrlAct::Match { packs, owner } => self.editor.wrl_match(packs, owner),
			WrlAct::Finish { dest } => self.editor.wrl_finish(dest),
		};
		if let Some(overlay) = self.win.as_mut().and_then(|w| w.overlay.as_mut()) {
			match self.editor.wrlimport.as_ref() {
				Some(run) if run.result.is_some() => {
					overlay.show_wrl_unmapped(run.matched, run.used, &run.rows);
				}
				Some(_) => {}
				None => overlay.hide(),
			}
		}
		// Last, so a failure's error dialog lands over the hidden picker.
		self.act_on(outcome, event_loop);
	}

	/// Step every live background job a per-frame time slice: Fix Shore,
	/// Generate Random Terrain, New from Image, and the rasterize palette
	/// conversion. Each keeps the frame responsive by budgeting a few
	/// milliseconds and letting the redraw loop carry it forward.
	fn step_background_jobs(&mut self, event_loop: &ActiveEventLoop) {
		// Fix Shore: step the live run a slice per frame. The clock starts on the
		// first running frame (covers both the Start button and the menu's
		// auto-started runs) and resets while idle so each run times fresh; the
		// modal keeps its own final `elapsed` for the display after a run ends.
		if self.editor.autofix_running() {
			self.autofix_clock.get_or_insert_with(std::time::Instant::now);
			// Step the run in small slices within a per-frame wall-clock budget
			// (like Generate): the map keeps redrawing - and, since each tick
			// applies its tiles live, keeps *updating* - between slices, so the
			// UI stays responsive and the coast is seen resolving as it goes.
			let frame = std::time::Instant::now();
			loop {
				self.editor.autofix_tick(self.autofix_elapsed(), false);
				if !self.editor.autofix_running() || frame.elapsed() >= std::time::Duration::from_millis(6) {
					break;
				}
			}
		} else {
			self.autofix_clock = None;
		}

		// Generate Random Terrain: step the live run within a
		// per-frame time budget - the progress bar fills, the UI stays live.
		if self.editor.generate_running() {
			let frame = std::time::Instant::now();
			let mut outcome = Outcome::Redraw;
			while self.editor.generate_running() && frame.elapsed() < std::time::Duration::from_millis(7) {
				outcome = self.editor.generate_tick(false);
			}
			self.act_on(outcome, event_loop);
		}

		// New from Image: step the conversion within a per-frame time
		// budget (keeps the frame responsive); completion opens a new tab. The
		// first frame after Convert only *paints* the "Loading image…" state -
		// the demanding decode begins next frame, so the user sees it started.
		if self.editor.converting() {
			if !self.convert_primed {
				self.convert_primed = true; // paint this frame; decode/step from next
			} else {
				let frame = std::time::Instant::now();
				let mut outcome = Outcome::Redraw;
				while self.editor.converting() && frame.elapsed() < std::time::Duration::from_millis(7) {
					outcome = self.editor.convert_tick(self.convert_elapsed(), false);
				}
				// Completion opens the new tab (DocReplaced) and closes the dialog
				// (the editor dropped `newimage` on finish).
				if matches!(outcome, Outcome::DocReplaced) {
					if let Some(overlay) = self.win.as_mut().and_then(|w| w.overlay.as_mut()) {
						overlay.hide();
					}
				}
				self.act_on(outcome, event_loop);
			}
		}

		// Rasterize palette conversion: same per-frame time budget; completion
		// swaps the document content (DocReplaced → atlas rebuild) and closes
		// the dialog (the editor drops `pconvert` on finish).
		if self.editor.palette_converting() {
			let frame = std::time::Instant::now();
			let mut outcome = Outcome::Redraw;
			while self.editor.palette_converting() && frame.elapsed() < std::time::Duration::from_millis(7) {
				outcome = self.editor.palette_convert_tick(self.pconvert_elapsed(), false);
			}
			if matches!(outcome, Outcome::DocReplaced) {
				if let Some(overlay) = self.win.as_mut().and_then(|w| w.overlay.as_mut()) {
					overlay.hide();
				}
			}
			self.act_on(outcome, event_loop);
		}
	}

	/// The render half of a redraw: refresh the per-document GPU state, draw
	/// the frame + the overlay, present, then apply what the overlay reported
	/// ([`Self::apply_overlay_outcome`]) and schedule the follow-up frame any
	/// live job or animation needs.
	fn render_and_present(&mut self, event_loop: &ActiveEventLoop) {
		let Some(win) = self.win.as_mut() else { return };

		if self.editor.revision() != win.uploaded_revision {
			refresh_renderer(&win.renderer, &win.gpu.queue, &mut self.editor);
			win.uploaded_revision = self.editor.revision();
		}
		if let Some(rgba) = self.editor.cycler.take_if_dirty() {
			sync_palette(rgba, &win.renderer, &win.passes, &win.gpu.queue);
		}
		// The RGBA tile atlas is only needed while the match-edit dialog is open;
		// otherwise its whole-project compose is skipped entirely.
		let match_open = win.overlay.as_ref().is_some_and(|o| o.visible() && o.match_strip_key().is_some());
		refresh_tile_atlas(&self.editor, &mut win.passes, match_open);
		refresh_template_atlas(&self.editor, &mut win.passes);

		let title = self.editor.title();
		if title != win.title {
			win.window.set_title(&title);
			win.title = title;
		}

		// wgpu 30 reports the acquire as a status, not a `Result`. `Suboptimal`
		// still hands over a usable texture (it only asks for a reconfigure
		// eventually), so it draws like `Success`.
		use wgpu::CurrentSurfaceTexture as Acquired;
		let frame = match win.gpu.surface.get_current_texture() {
			Acquired::Success(frame) | Acquired::Suboptimal(frame) => frame,
			Acquired::Lost | Acquired::Outdated => {
				win.gpu.surface.configure(&win.gpu.device, &win.gpu.config);
				win.window.request_redraw();
				return;
			}
			// Transient: a timed-out frame, or a window that is minimized or
			// fully covered, has nothing to present. Skip the frame - taking
			// the editor down over a minimize would lose unsaved work.
			Acquired::Timeout | Acquired::Occluded => {
				win.window.request_redraw();
				return;
			}
			Acquired::Validation => {
				eprintln!("fatal surface error: the surface acquire failed validation");
				event_loop.exit();
				return;
			}
		};

		let target = frame.texture.create_view(&Default::default());
		let mut encoder = win.gpu.device.create_command_encoder(&Default::default());
		// Reset the minimap follow-up flag; `render_frame` re-sets it only if the
		// (throttled) minimap draws stale content this frame.
		win.passes.minimap.clear_followup();
		render_frame(
			&win.gpu.device,
			&win.gpu.queue,
			&mut encoder,
			&target,
			&mut self.editor,
			&win.renderer,
			&mut win.passes,
		);
		// Composite the wgpu-ui overlay on top of the editor's frame, then act on
		// what it reports (e.g. Map Metadata applied on Save).
		let mut overlay_outcome = uikit_overlay::Outcome::Idle;
		if let (Some(overlay), Some(chrome)) = (win.overlay.as_mut(), win.passes.menu_chrome.as_mut()) {
			if overlay.visible() {
				// Fix Shore: push the live run numbers into its window before
				// this frame's dispatch/draw (the editor owns the run).
				if let Some(af) = self.editor.autofix.as_ref() {
					let elapsed = if af.running || af.applied.is_some() {
						format!("{:.1}s", af.elapsed)
					} else {
						"-".to_string()
					};
					overlay.sync_autofix(af.running, af.found, af.fixed, af.remaining, &elapsed, af.applied);
				}
				// Convert Palette: same — push the live rasterize numbers in.
				if let Some(pc) = self.editor.pconvert.as_ref() {
					let mut time = format!("{:.0}%   elapsed {:.1}s", pc.progress * 100.0, pc.elapsed);
					if pc.running && pc.progress > 0.02 {
						let eta = pc.elapsed * (1.0 - pc.progress) / pc.progress;
						time.push_str(&format!("   ~{eta:.1}s left"));
					}
					overlay.sync_convert_palette(pc.running, pc.progress, &pc.stage, &time);
				}
				// New from Image: same.
				if let Some(ni) = self.editor.newimage.as_ref() {
					let mut time = format!("{:.0}%   elapsed {:.1}s", ni.progress * 100.0, ni.elapsed);
					if ni.running && ni.progress > 0.02 {
						let eta = ni.elapsed * (1.0 - ni.progress) / ni.progress;
						time.push_str(&format!("   ~{eta:.1}s left"));
					}
					overlay.sync_new_image(ni.running, ni.progress, &ni.stage, &time);
				}
				// Generate: progress while stepping, the report lines after.
				if let Some(gr) = self.editor.genrun.as_ref() {
					let progress = gr.session.as_ref().map(|s| s.progress());
					let reported = gr.started.as_ref().map(|p| p.seed);
					overlay.sync_generate(gr.running, progress, &gr.status, reported);
				}
				// Tile Painter: push the live palette table (the preview cycles
				// with it) and any editor-side canvas write (PNG import).
				if let Some(run) = self.editor.tilepaint.as_ref() {
					overlay.sync_tile_paint(self.editor.cycler.rgba(), Some((&run.canvas, run.canvas_rev)));
				}
				// New Scenery: the live palette table, plus any editor-side
				// source write (a PNG picked in the native dialog).
				if let Some(run) = self.editor.scenerypaint.as_ref() {
					overlay.sync_scenery_new(chrome, run, &self.editor.project.palette, self.editor.cycler.rgba());
				}
				// Match editor: recompose the orientation strip when the
				// selection moves (or the document changed), re-sync the atlas.
				if let Some((pack, main, cand)) = overlay.match_strip_key() {
					let key = (pack, main, cand, self.editor.revision());
					if self.match_strip_key != Some(key) {
						self.match_strip_key = Some(key);
						let strip = compose_match_strip(&self.editor, pack, main, cand);
						overlay.update_match_strip(chrome, &strip);
					}
					if let Some(atlas) = win.passes.tile_atlas.as_ref() {
						let base = crate::picker::global_index(
							&self.editor.project,
							map_core::TileRef { pack: pack as u8, tile: 0, transform: map_core::Transform::default() },
						);
						overlay.sync_match_atlas(atlas.tex, atlas.count, base);
					}
				}
				let size = (win.gpu.config.width, win.gpu.config.height);
				overlay_outcome = overlay.render(&mut encoder, &target, size, chrome);
				// Mirror a "Palette preview" toggle into the persisted
				// preference (written immediately, like Quick Load).
				if let Some(on) = overlay.take_palette_preview_change() {
					self.editor.palette_preview = on;
					self.editor.save_preferences();
				}
				// Mirror an edited working canvas back into the run, so command
				// paths (commit / PNG export) read current pixels.
				if let Some(pixels) = overlay.tile_canvas_if_edited() {
					if let Some(run) = self.editor.tilepaint.as_mut() {
						if run.canvas.len() == pixels.len() {
							run.canvas.copy_from_slice(pixels);
						}
					}
				}
			}
		}
		// Mirror focused-text-field state into the OS IME through ONE arbiter, so
		// no two hosts fight over `set_ime_allowed`. A visible dialog outranks
		// everything (it is modal, or a float that intercepts keys); otherwise the
		// answer comes from the router's focused layer — the same layer the
		// keystrokes themselves go to, which is what makes the IME work in modal
		// fields, panel fields and the console from one code path (U1.4; before it,
		// only dialogs and Unit Properties were wired, each testing its own state,
		// and the console's field could never receive a composition at all). The
		// candidate window anchors at the focused field's caret (logical → physical
		// via the UI scale; only changes reach winit). Defocusing turns it back off.
		let focus = focus_layer(&self.editor, &self.router);
		let overlay_ime = win.overlay.as_ref().filter(|o| o.visible() && o.wants_text_input());
		let focused_field =
			focus.and_then(|l| layer_panel(&win.passes, &self.editor, l)).filter(|p| p.wants_text_input());
		let want_ime = overlay_ime.is_some() || focused_field.is_some();
		let caret = match (overlay_ime, focused_field) {
			(Some(o), _) => o.ime_rect(),
			(None, Some(p)) => p.ime_rect(),
			_ => None,
		};
		if want_ime != self.ime_on {
			self.ime_on = want_ime;
			self.ime_rect = None;
			win.window.set_ime_allowed(want_ime);
		}
		if want_ime && let Some(r) = caret {
			let s = self.editor.ui_scale;
			let phys = (r.x * s, r.y * s, r.w * s, r.h * s);
			if self.ime_rect != Some(phys) {
				self.ime_rect = Some(phys);
				win.window.set_ime_cursor_area(
					winit::dpi::PhysicalPosition::new(phys.0 as f64, phys.1 as f64),
					winit::dpi::PhysicalSize::new(phys.2 as f64, phys.3 as f64),
				);
			}
		}
		win.gpu.queue.submit([encoder.finish()]);
		// Presenting moved onto the queue in wgpu 30, and must follow the submit.
		win.gpu.queue.present(frame);
		self.apply_overlay_outcome(overlay_outcome);

		let Some(win) = self.win.as_mut() else { return };
		if self.editor.animate
			|| self.editor.autofix_running()
			|| self.editor.converting()
			|| self.editor.palette_converting()
			|| self.editor.generate_running()
			|| win.overlay.as_ref().is_some_and(|o| o.tile_paint_animating())
			// The minimap throttled its rebuild this frame (a live edit stroke);
			// keep redrawing so it catches up once the edits settle.
			|| win.passes.minimap.needs_followup()
			// A hover resting toward a tooltip: time alone will change the
			// pixels, and no event will arrive to say so.
			|| win.passes.tooltip_arming()
		{
			win.window.request_redraw();
		}
	}

	/// Act on what a frame's overlay reported: dialog outcomes that mutate
	/// editor state directly run here, and the verbs that need `act_on` or the
	/// event loop (which this method cannot take mid-`win` borrow) go onto
	/// [`Self::deferred`] for the drain point after `redraw` returns.
	fn apply_overlay_outcome(&mut self, outcome: uikit_overlay::Outcome) {
		let Some(win) = self.win.as_mut() else { return };
		match outcome {
			uikit_overlay::Outcome::ApplyMetadata { vals: v, save_after } => {
				self.editor.project.set_info(v.name, v.players, v.description, v.date, v.version, v.author);
				if save_after {
					// First-save flow: resume the Save-As this prompt
					// interrupted; the one-shot skips re-prompting.
					self.editor.first_save_meta = true;
					self.deferred.push_back(Deferred::Command(Command::FileDialog {
						purpose: crate::command::FilePurpose::SaveAs,
					}));
				}
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::ApplyPreferences { max_path, max_port_path, max_port_data_path, skip_prompt } => {
				self.editor.apply_preferences(max_path, max_port_path, max_port_data_path, skip_prompt);
				self.editor.console.push_line("editor preferences saved");
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::ApplySaveData(settings) => {
				// The dialog validated the block; apply it as one undoable step.
				match self.editor.apply_save_data(&settings) {
					Ok(line) => self.editor.console.push_line(line),
					Err(message) => {
						self.editor.console.push_line(message.clone());
						if let Some(overlay) = &mut win.overlay {
							overlay.open_error(&message);
						}
					}
				}
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::PrefsCancelledRequired => {
				// The user backed out of a required paths prompt: if a folder is
				// still missing, explain why it's needed (Abort / Continue → reopen).
				if self.editor.paths_incomplete() {
					let reason = self
						.editor
						.paths_prompt_reason
						.clone()
						.unwrap_or_else(|| "The editor needs your M.A.X. folders.".into());
					if let Some(overlay) = &mut win.overlay {
						overlay.open_confirm_labeled(
							"Attention",
							&format!(
								"{reason}\n\nThe editor needs your M.A.X. and M.A.X. Port folders for this. \
									 Set them now?"
							),
							"Abort",
							"Continue",
							"editor-preferences".into(),
						);
					}
				}
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::CreateMap(v) => {
				// Run after the frame (act_on needs the event loop): same path as
				// the `new` script command. A custom palette choice loads right
				// after, via the queued `Deferred::Palette`.
				self.deferred.push_back(Deferred::Command(Command::New {
					width: v.width,
					height: v.height,
					packs: v.packs,
					seed: None,
				}));
				if let Some(path) = v.palette {
					self.deferred.push_back(Deferred::Palette(path));
				}
			}
			uikit_overlay::Outcome::NewMapPreview { palette, water, key } => {
				// The picker's palette / water choice changed: compose the preview
				// atlas with those choices and poke it back (the dialog stays
				// open). An unreadable palette file falls back to pack colours.
				let assets_root = self.editor.assets_root.clone();
				let override_pal = palette.and_then(|p| crate::palette_io::load(&p).ok());
				if let (Some(overlay), Some(chrome)) = (win.overlay.as_mut(), win.passes.menu_chrome.as_mut()) {
					let (rgba, _rows) = crate::newmap::build_rgba(
						overlay.pack_entries(),
						&assets_root,
						override_pal.as_deref(),
						&water,
					);
					let rows = (overlay.pack_entries().len().max(1) as u32) * 64;
					let tex = chrome.register_texture(&rgba, (crate::newmap::PREVIEW_TILES * 64) as u32, rows);
					overlay.provide_preview_tex(key, tex);
				}
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::CreateShapedMap { width, height, packs, palette, image } => {
				// Create the all-water map after the frame, then carve the shape in
				// (the carve + Fix Shore run via `pending_shape`).
				self.deferred.push_back(Deferred::Command(Command::New { width, height, packs, seed: None }));
				if let Some(path) = palette {
					self.deferred.push_back(Deferred::Palette(path));
				}
				self.deferred.push_back(Deferred::Shape(image));
			}
			uikit_overlay::Outcome::ResizeMap(line) => {
				// Run the validated `resize …` line after the frame (same path
				// scripts and the old modal use).
				if let Some(command) = crate::command::parse_line(&line).ok().flatten() {
					self.deferred.push_back(Deferred::Command(command));
				}
			}
			uikit_overlay::Outcome::RunCommand(line) => {
				// A confirm dialog's primary fired: run the line it carried (e.g.
				// `palette-delete "<path>"`) after the frame, via the command path.
				if let Some(command) = crate::command::parse_line(&line).ok().flatten() {
					self.deferred.push_back(Deferred::Command(command));
				}
			}
			uikit_overlay::Outcome::OpenUrl(url) => {
				let _ = crate::browser::open(&url);
			}
			// Fix Shore verbs: the run lives on the editor; the window is a view.
			uikit_overlay::Outcome::FixStart => {
				self.editor.autofix_start();
				self.autofix_clock = Some(std::time::Instant::now());
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::FixStop => {
				let elapsed = self.autofix_clock.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
				self.editor.autofix_tick(elapsed, true);
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::FixAbort => {
				self.editor.autofix_abort();
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::FixClose => {
				self.editor.autofix_close();
				win.window.request_redraw();
			}
			// Convert Palette rasterize verbs: the run lives on the editor.
			uikit_overlay::Outcome::PaletteConvertStart { water, relaxed, threshold } => {
				self.editor.palette_convert_start(water, relaxed, threshold);
				self.pconvert_clock = Some(std::time::Instant::now());
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::PaletteConvertAbort => {
				let elapsed = self.pconvert_clock.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
				self.editor.palette_convert_tick(elapsed, true);
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::PaletteConvertCancel => {
				self.editor.palette_convert_cancel();
				win.window.request_redraw();
			}
			// New from Image verbs: the run lives on the editor.
			uikit_overlay::Outcome::NewImageStart(opts) => {
				self.editor.convert_start(opts);
				self.convert_clock = Some(std::time::Instant::now());
				self.convert_primed = false;
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::NewImageAbort => {
				let elapsed = self.convert_clock.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
				self.editor.convert_tick(elapsed, true);
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::NewImageCancel => {
				self.editor.newimage_cancel();
				win.window.request_redraw();
			}
			// Generate verbs: the run lives on the editor; the dialog stays open
			// across runs (Close hands the session memory back).
			uikit_overlay::Outcome::GenerateStart { params, seed } => {
				if let Outcome::Failed(e) = self.editor.generate_start(params, seed) {
					eprintln!("FAILED: {e}");
					self.editor.console.push_line(e);
				}
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::GenerateAbort => {
				self.editor.generate_tick(true);
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::GenerateClose(mem) => {
				self.editor.gen_memory = mem;
				self.editor.generate_close();
				win.window.request_redraw();
			}
			// Import WRL verbs: match/finish defer to the next redraw (they
			// need act_on); back/cancel just adjust the parked run.
			uikit_overlay::Outcome::WrlMatch { packs, owner } => {
				self.deferred.push_back(Deferred::Wrl(WrlAct::Match { packs, owner }));
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::WrlFinish { dest } => {
				self.deferred.push_back(Deferred::Wrl(WrlAct::Finish { dest }));
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::WrlBack => {
				self.editor.wrl_back();
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::WrlCancel => {
				self.editor.wrl_cancel();
				win.window.request_redraw();
			}
			// Tile Painter verbs. The commit defers to the next redraw (a
			// success needs act_on: DocReplaced → renderer rebuild); the rest
			// act on editor state directly.
			uikit_overlay::Outcome::TileCommit { id, pass, pack, pixels } => {
				// The freshest canvas rides the outcome (the same-frame edit
				// may not have been mirrored yet).
				if let Some(run) = self.editor.tilepaint.as_mut() {
					if run.canvas.len() == pixels.len() {
						run.canvas = pixels;
					}
				}
				self.deferred.push_back(Deferred::Tile(TileCommitAct { id, pass, pack }));
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::TileCopy(pixels) => {
				self.editor.tile_ops.clipboard = Some(pixels);
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::TileExportPng { id } => {
				// Stash the typed id so the save dialog suggests `<id>.png`.
				if let Some(run) = self.editor.tilepaint.as_mut() {
					run.id_text = id;
				}
				self.deferred
					.push_back(Deferred::Command(Command::FileDialog { purpose: command::FilePurpose::ExportTilePng }));
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::TileImportPng => {
				self.deferred
					.push_back(Deferred::Command(Command::FileDialog { purpose: command::FilePurpose::ImportTilePng }));
				win.window.request_redraw();
			}
			// New Scenery verbs, on the Tile Painter's terms: a threshold change
			// re-derives from the source the editor owns, the commit defers to
			// the next redraw (DocReplaced -> the scenery atlas rebuilds).
			uikit_overlay::Outcome::SceneryRederive => win.window.request_redraw(),
			uikit_overlay::Outcome::SceneryImportPng => {
				self.deferred.push_back(Deferred::Command(Command::FileDialog {
					purpose: command::FilePurpose::ImportSceneryPng,
				}));
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::SceneryImportHeightPng => {
				self.deferred.push_back(Deferred::Command(Command::FileDialog {
					purpose: command::FilePurpose::ImportSceneryHeightPng,
				}));
				win.window.request_redraw();
			}
			// The picture is the dialog's to make and the file the editor's to
			// write, so it is parked on the run and the picker follows it.
			uikit_overlay::Outcome::SceneryExportHeightPng { grey, w, h } => {
				if let Some(run) = self.editor.scenerypaint.as_mut() {
					run.hgt_out = grey;
					run.hgt_out_w = w;
					run.hgt_out_h = h;
				}
				self.deferred.push_back(Deferred::Command(Command::FileDialog {
					purpose: command::FilePurpose::ExportSceneryHeightPng,
				}));
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::SceneryCommit { pack, id, name, sprite, pass, cells, relief, height } => {
				self.deferred.push_back(Deferred::Scenery(SceneryCommitAct {
					pack,
					id,
					name,
					sprite,
					pass,
					cells,
					relief,
					height,
				}));
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::SceneryNewClose => {
				self.editor.scenerypaint = None;
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::TilePaintClose => {
				self.editor.tilepaint = None;
				win.window.request_redraw();
			}
			// Match editor: apply + persist the staged commits; the dialog stays
			// open with its baseline advanced (success) or the error inline.
			uikit_overlay::Outcome::MatchSave(commits) => {
				if let Some(overlay) = &mut win.overlay {
					match self.editor.match_editor_save(commits) {
						Ok(()) => overlay.match_saved(),
						Err(message) => {
							self.editor.console.push_line(message.clone());
							overlay.match_error(&message);
						}
					}
				}
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::MatchClose => {
				win.window.request_redraw();
			}
			uikit_overlay::Outcome::Idle => {}
		}
	}
}

impl ApplicationHandler for App {
	fn resumed(&mut self, event_loop: &ActiveEventLoop) {
		if self.win.is_some() {
			return;
		}

		let title = self.editor.title();
		let window = Arc::new(
			event_loop
				.create_window(
					Window::default_attributes()
						.with_title(&title)
						.with_inner_size(LogicalSize::new(self.editor.screen.0, self.editor.screen.1)),
				)
				.expect("create window"),
		);

		let gpu = pollster::block_on(gpu::WindowGpu::new(window.clone()));
		let render_core = project_render::RenderCore::new(&gpu.device, gpu.config.format);
		let renderer = make_renderer(&gpu.device, &gpu.queue, &self.editor, &render_core);
		self.editor.project.clear_render_dirty(); // the fresh renderer already has everything
		let passes = Passes::new(&gpu.device, &gpu.queue, gpu.config.format);
		// The overlay shares `passes.menu_chrome`'s renderer/steel theme/fonts, so
		// it just tracks the editor's UI scale; controls size like the chrome.
		let overlay = Some(uikit_overlay::Overlay::new(self.editor.ui_scale as f64));

		self.editor.screen = (gpu.config.width, gpu.config.height);
		let _ = self.editor.execute(Command::Fit);
		let uploaded_revision = self.editor.revision();

		window.request_redraw();
		self.win = Some(WindowState { window, gpu, render_core, renderer, passes, uploaded_revision, title, overlay });

		// Startup script runs through the exact same path as live input.
		for command in std::mem::take(&mut self.startup_script) {
			self.run(command, event_loop);
		}

		// First run: if a game folder is still unset, prompt for it once (unless
		// the user ticked "don't ask again"), so the Units panel + save editor work
		// out of the box. Skipped when a startup dialog is already up.
		if self.editor.paths_incomplete()
			&& !self.editor.skip_path_prompt
			&& !self.win.as_ref().and_then(|w| w.overlay.as_ref()).is_some_and(|o| o.visible())
		{
			self.open_preferences();
		}
	}

	fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
		// Track the physical cursor before anything else: the dialog intercepts
		// below early-return, so a move over a non-blocking float never reached
		// the main CursorMoved arm — and the float's click gate
		// (`wants_pointer_at(lc)`) then tested the position from *before* the
		// pointer entered the float, dropping its button clicks onto the map.
		if let WindowEvent::CursorMoved { position, .. } = &event {
			self.cursor = (position.x as f32, position.y as f32);
		}
		// Translate ONCE, here, for every UI host below - the dialog intercept, the
		// console / Unit Properties keyboard routing, and the pointer arms all
		// borrow this one `Vec` (U1.2). Two things depend on it being exactly once:
		// a second `translate` of the same event would re-read the OS clipboard on
		// a Ctrl+V chord, and `ModifiersChanged` (which no host consumes, but which
		// stamps every later pointer/key event) must never be skipped - that is
		// what makes panel text fields see real chords at all.
		let events = self.router.translate(&event, self.editor.ui_scale);
		// --- wgpu-ui dialogs: F1 toggles About; a blocking dialog is modal, a
		// non-blocking one (Fix Shore) floats and shares input with the map. --
		{
			let mut toggle = false;
			let mut intercept = false;
			// Whether the dialog takes this event. Decided under a *shared* borrow;
			// the gates below only read the last frame's layout, so deciding here
			// and dispatching after is equivalent to the old feed-then-test order.
			let mut feed = false;
			let lc = self.lcursor();
			let scale = self.editor.ui_scale;
			if let Some(overlay) = self.win.as_ref().and_then(|w| w.overlay.as_ref()) {
				let f1 = matches!(
					&event,
					WindowEvent::KeyboardInput { event: ke, .. }
						if ke.state.is_pressed()
							&& ke.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::F1)
				);
				if f1 {
					toggle = true;
				} else if overlay.visible() && overlay.blocking() {
					feed = true;
					// Swallow input while modal; let structural events fall through.
					intercept = !matches!(
						event,
						WindowEvent::RedrawRequested
							| WindowEvent::Resized(_)
							| WindowEvent::ScaleFactorChanged { .. }
							| WindowEvent::CloseRequested
					);
				} else if overlay.visible() {
					// Non-blocking float (Fix Shore): keys go to the window;
					// pointer input belongs to it only over the window (or
					// mid-drag) — everything else falls through to the live map.
					match &event {
						WindowEvent::KeyboardInput { .. } | WindowEvent::Ime(_) => {
							feed = true;
							intercept = true;
						}
						WindowEvent::CursorMoved { position, .. } => {
							feed = true;
							let p = Vec2::new(position.x as f32 / scale, position.y as f32 / scale);
							intercept = overlay.wants_pointer_at(p);
						}
						WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. } => {
							feed = overlay.wants_pointer_at(Vec2::new(lc.0, lc.1));
							intercept = feed;
						}
						_ => {}
					}
				}
			}
			if toggle {
				self.toggle_about();
				return;
			}
			if feed {
				self.dispatch_layer(Layer::Overlay, &events);
			}
			if intercept {
				if matches!(event, WindowEvent::CursorMoved { .. }) {
					self.apply_cursor();
				}
				self.redraw_win();
				return;
			}
		}

		// Keys, text and IME go to the layer that owns the keyboard first; only
		// what it declines reaches the map bindings below (U1.4).
		if matches!(event, WindowEvent::KeyboardInput { .. } | WindowEvent::Ime(_))
			&& self.route_keyboard(&event, &events, event_loop)
		{
			self.redraw_win();
			return;
		}

		match event {
			// The OS close button is a quit request: clean exits, unsaved work
			// raises the Save/Discard/Cancel guard instead of being lost.
			WindowEvent::CloseRequested => self.run(Command::QuitRequest, event_loop),

			WindowEvent::ModifiersChanged(modifiers) => {
				self.modifiers = modifiers.state();
			}

			WindowEvent::KeyboardInput { event, .. } if event.state.is_pressed() => {
				// Esc closes an open menu before it can mean `quit`.
				if self.editor.menu_ref().is_open() && event.logical_key == Key::Named(NamedKey::Escape) {
					self.editor.menu().close();
					if let Some(win) = self.win.as_ref() {
						win.window.request_redraw();
					}
					return;
				}
				// Esc next closes the context menu, then disarms a ghost
				// stamp, then drops the selection - only an idle Esc reaches
				// the bindings (where it can be quit).
				if event.logical_key == Key::Named(NamedKey::Escape) {
					if self.editor.context_menu.is_some() {
						self.run(Command::ContextMenu { at: None }, event_loop);
						return;
					}
					if self.editor.stamp.is_some() {
						self.run(Command::StampCancel, event_loop);
						return;
					}
					// The unit place / erase tools stay armed for repeat use (like a
					// stamp); Esc cancels them back to the mode's own select tool,
					// matching the context-menu Cancel entry.
					if matches!(self.editor.tool, state::Tool::Unit | state::Tool::UnitEraser) {
						self.run(Command::ToolSelect { name: "default".into() }, event_loop);
						return;
					}
					if !self.editor.selection.is_empty() {
						self.run(Command::SelectOp { op: "clear".into() }, event_loop);
						return;
					}
				}
				// PageUp/Down/Home/End scroll the docked panel under the cursor.
				// Targeting is the shell's (it matches how the wheel picks a panel,
				// and keyboard *focus* reaches only Unit Properties before U5);
				// the scrolling itself is the panel's own `Scroller`, which pages on
				// `PageKeys::WhenHovered` (U2).
				let (sw, sh) = self.editor.ui_screen();
				let (lcx, lcy) = self.lcursor();
				if let Some((id, _)) = self.editor.workspace.body_at(lcx, lcy, sw, sh)
					&& matches!(
						&event.logical_key,
						Key::Named(NamedKey::PageUp | NamedKey::PageDown | NamedKey::Home | NamedKey::End)
					) && self.dispatch_layer(Layer::Panel(id), &events).wants_keyboard()
				{
					self.redraw_win();
					return;
				}
				if let Some(cmd) = self.bound_command(&event.logical_key) {
					self.run(cmd, event_loop);
				}
			}

			WindowEvent::Resized(size) => {
				if let Some(win) = self.win.as_mut() {
					win.gpu.resize(size.width, size.height);
					// Keep the on-screen map centre centred across the resize.
					self.editor.on_resize(size.width, size.height);
					win.window.request_redraw();
				}
			}

			WindowEvent::CursorLeft { .. } => {
				// No cursor → no hover; otherwise the last hovered widget would
				// stay lit after the mouse leaves the window. Every hosted `Ui`
				// gets the real `PointerLeft` the router translated this into -
				// there is no single target to route it to, because *whichever*
				// layer was hovered is now stale.
				self.editor.cursor = None;
				// The workspace frame paints its own chrome hover and is not a
				// hosted `Ui`, so it is told by hand (U6.2).
				self.editor.workspace.on_pointer_left();
				self.hover_redraw_cell = None; // re-entry must redraw even onto the same cell
				self.router.clear_hover();
				for layer in Layer::HOSTED {
					self.dispatch_layer(layer, &events);
				}
				if let Some(win) = self.win.as_ref() {
					win.window.request_redraw();
				}
			}

			// Window focus loss makes every layer's hover stale and strands any
			// armed control (its release will never arrive), so this is the second
			// broadcast: `ArmFire` disarms on `Focus(false)` and each `Ui` drops its
			// hover + capture.
			WindowEvent::Focused(_) => {
				for layer in Layer::HOSTED {
					self.dispatch_layer(layer, &events);
				}
				self.redraw_win();
			}

			WindowEvent::CursorMoved { .. } => {
				// `self.cursor` was already updated at the top of `window_event`
				// (before the dialog intercepts).
				// UI hit-tests use the **logical** cursor + logical screen (the chrome
				// layout space); the map reads below keep the raw physical cursor.
				let (sw, sh) = self.editor.ui_screen();
				let (lcx, lcy) = self.lcursor();
				// Feed the pointer snapshot for hover/pressed widget states.
				self.editor.cursor = Some((lcx, lcy));
				// Redraw on move only when it can change something visible: over
				// chrome (panel/menu/tab hover, or an open menu/context), or - over
				// the bare map - when the cursor's *cell* changes (the tile ghost,
				// brush outline, and status-bar readout are all cell-granular). A
				// sub-cell move over the map changes nothing on-screen, so it skips
				// the full-frame redraw; active drags redraw via their branches below.
				// The one exception is the scenery place ghost: it hangs from the
				// cursor by map *pixels*, so cell-granular redraws would make it
				// jump a tile at a time when the placement itself is free.
				let over = over_at(&self.editor, self.popup_layer(), lcx, lcy);
				let cell = self.editor.cell_at(self.cursor.0, self.cursor.1);
				let pixel_ghost = scenery_ghost_armed(&self.editor).is_some();
				if !over.is_map() || cell != self.hover_redraw_cell || pixel_ghost {
					self.redraw_win();
				}
				self.hover_redraw_cell = cell;
				// A layer mid-drag owns the whole pointer cascade (U1.3): the move
				// goes to it and to nothing else, wherever the cursor now is. Hover
				// is deliberately not retargeted - the drag has not left anything, it
				// is still going on. The stream is handed back on the release, and -
				// the safety valve, since a release can go missing - on window focus
				// loss, which broadcasts `Focus(false)` and makes every `Ui` drop its
				// capture.
				if let Some(held) = self.router.capture() {
					self.dispatch_layer(held, &events);
					// A drag the capturing widget converts into commands as it goes:
					// the minimap's pan, and the palette's slider / block-bar tracks
					// (U5.9). Polled here because this branch returns before the
					// ordinary per-panel outcome polls.
					match held {
						Layer::Panel("minimap") => self.drain_minimap_pan(event_loop),
						Layer::Panel("palette") => self.drain_palette_edits(event_loop),
						_ => {}
					}
					self.apply_cursor();
					self.redraw_win();
					return;
				}
				// Menu-bar hover / open-cascade tracking (the retained widget
				// owns it). Redraw on consumed moves, and over the bar strip so
				// the closed-state header hover stays live.
				if self.dispatch_layer(Layer::MenuBar, &events).wants_pointer() || lcy < menu::BAR_H {
					if let Some(win) = self.win.as_ref() {
						win.window.request_redraw();
					}
				}
				// Context-menu hover tracking (the widget owns it).
				if self.editor.context_menu.is_some()
					&& self.dispatch_layer(Layer::ContextMenu, &events).wants_pointer()
				{
					if let Some(win) = self.win.as_ref() {
						win.window.request_redraw();
					}
				}
				// The panel under the cursor gets the move too (U1.2). A panel `Ui`
				// never saw one before - every `build` call was handed `&[]` - so
				// anything motion-driven inside a panel was dead by construction:
				// drag-select in a Unit Properties field extends the selection *only*
				// on `PointerMoved` while capturing. Retargeting hands the panel being
				// left a `PointerLeft` first, or its hover stays lit behind us; that
				// one is derived rather than translated, because "the pointer left
				// *this layer*" is a fact about the z-order, not about the window.
				// The target comes off the same `over_at` the redraw test used (U1.5),
				// so a panel under an open cascade no longer lights up while something
				// press-modal covers it - `render_frame` already blanked its hover.
				let panel = match over {
					Over::Ui(layer @ Layer::Panel(_)) => Some(layer),
					_ => None,
				};
				if let Some(left) = self.router.retarget(panel) {
					self.dispatch_layer(left, &[Event::PointerLeft]);
				}
				if let Some(target) = panel {
					self.dispatch_layer(target, &events);
				}
				if self.editor.workspace.on_move(lcx, lcy, sw, sh) {
					if let Some(win) = self.win.as_ref() {
						win.window.request_redraw();
					}
				}
				self.apply_cursor();
				if let Some(last) = self.drag {
					let zoom = self.editor.view.zoom;
					let dx = (last.0 - self.cursor.0) / zoom / render::TILE_PX as f32;
					let dy = (last.1 - self.cursor.1) / zoom / render::TILE_PX as f32;
					self.drag = Some(self.cursor);
					self.run(Command::Pan { dx, dy }, event_loop);
				}
				if self.paint.is_some() {
					if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
						if self.paint != Some((x, y)) {
							self.paint = Some((x, y));
							self.run(self.paint_command(x, y), event_loop);
						}
					}
				}
				// Freehand select-drag: extend the mask cell by cell.
				if let Some((mode, last)) = self.select_paint {
					if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
						if last != (x, y) {
							self.select_paint = Some((mode, (x, y)));
							self.run(Command::SelectCell { x, y, mode }, event_loop);
						}
					}
				}
				// Rect select-drag: stretch the live preview to the cursor.
				if let Some((ax, ay, _)) = self.select_anchor {
					if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
						self.editor.select_preview = Some((ax, ay, x, y));
					}
				}
				// Alt+drag selection-move: translate the marquee by the cell delta.
				if let Some(last) = self.select_move {
					if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
						if last != (x, y) {
							let (dx, dy) = (x as i32 - last.0 as i32, y as i32 - last.1 as i32);
							self.select_move = Some((x, y));
							self.run(Command::SelectMove { dx, dy }, event_loop);
						}
					}
				}
				// Scenery Move drag: slide the grabbed object to the cursor, minus
				// the grab offset. No collision rule - scenery overlaps freely, and
				// the draw order (placement order) already says what covers what.
				if let Some((i, ox, oy)) = self.scenery_drag {
					let (px, py) = self.editor.world_at(self.cursor.0, self.cursor.1);
					self.run(Command::SceneryMove { index: i, x: px - ox, y: py - oy }, event_loop);
				}
				// Object Move drag: slide the grabbed object to the cursor cell
				// (minus the grab offset), unless a building blocks the target.
				if let Some((i, ox, oy)) = self.obj_drag {
					if let Some((cx, cy)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
						let (mw, mh) = self.editor.map_size();
						let f = self.editor.object_footprint_of(i);
						let tx = cx.saturating_sub(ox).min(mw.saturating_sub(f));
						let ty = cy.saturating_sub(oy).min(mh.saturating_sub(f));
						if !self.editor.object_collides(i, tx, ty) {
							self.editor.project.move_object_to(i, tx, ty);
						}
					}
				}
			}

			WindowEvent::MouseInput { state, button, .. } if self.bindings.is_paint_button(button) => {
				// UI hit-tests below use the logical cursor + logical screen (chrome
				// space); map paints (`cell_at`) keep the raw physical cursor.
				let (sw, sh) = self.editor.ui_screen();
				let (lcx, lcy) = self.lcursor();
				match state {
					ElementState::Pressed => {
						// A layer mid-drag owns the pointer (U1.3), so this press
						// reaches it and nothing else. This is also what makes the
						// blocks at the tail of `CursorMoved` and of the release arm
						// provably inert while a panel captures: every one of them is
						// gated on shell drag state (`paint`, `drag`, `select_*`,
						// `obj_drag`, `scroll_drag`) that
						// only a press reaching *this* arm can set.
						//
						// Since the toolkit reports an open popup as that same
						// capture (an open dropdown or cascade is a pointer grab),
						// this branch is also what routes the press *into* the
						// popup's layer wherever it lands — the owner picks or
						// dismisses, and the press never reaches the map. The two
						// menu layers route through their act-mapping wrappers, or
						// a fired item's command would never run.
						if let Some(held) = self.router.capture() {
							match held {
								Layer::MenuBar => {
									self.menu_press(&events, event_loop);
								}
								Layer::ContextMenu => self.context_press(&events, event_loop),
								held => {
									self.dispatch_layer(held, &events);
									// What this press fired — a dropdown pick, a palette
									// swatch — dies with the next dispatch: drain now,
									// exactly as the body-press arm below does.
									if let Layer::Panel(id) = held {
										self.drain_panel_press(id, event_loop);
									}
								}
							}
							self.redraw_win();
							return;
						}
						// Focus follows the primary press (U1.4): a press landing
						// outside the layer that owns the keyboard takes it away, so
						// typing after clicking the map runs the map bindings instead
						// of dribbling into the field you left behind. Only presses
						// that will *not* reach that layer blur it here -
						// `dispatch_layer` re-resolves focus for the ones that do.
						// This is the case an app-side focus-out test could never
						// see (the press goes to the map, or to another panel's
						// `Ui`), and the `Moved` blur is what commits it (U4.1).
						// The layer this press *will* reach: a docked panel, or the
						// console band while it is up (it covers them, and a press
						// inside it must not blur its own field).
						let under = match over_at(&self.editor, self.popup_layer(), lcx, lcy) {
							Over::Ui(Layer::Console) => Some(Layer::Console),
							_ => self.editor.workspace.body_at(lcx, lcy, sw, sh).map(|(id, _)| Layer::Panel(id)),
						};
						if let Some(held) = self.router.focus()
							&& Some(held) != under
						{
							self.blur_layer(held, BlurCause::Moved);
							self.router.refocus(None);
							// Apply what that blur committed *now*, while the
							// selection it was typed against is still the live one -
							// this press is about to change it.
							self.drain_unitprops_commits(event_loop);
						}
						// An open context menu is topmost: the press routes into
						// its widget - a row fires its act (run via the command
						// path, like the menu bar), an outside press dismisses;
						// the model follows the widget's open state.
						if self.editor.context_menu.is_some() {
							self.context_press(&events, event_loop);
							self.redraw_win();
							return;
						}
						// The open console covers the top band and is drawn over
						// everything: a press inside it belongs to its field (caret,
						// drag-select — which then captures the pointer, so the move
						// and release arms follow it above), never to the menu bar or
						// tab strip hidden underneath (U4.5).
						if matches!(over_at(&self.editor, self.popup_layer(), lcx, lcy), Over::Ui(Layer::Console)) {
							self.dispatch_layer(Layer::Console, &events);
							self.apply_cursor();
							self.redraw_win();
							return;
						}
						// The menu bar is next: it sees the press first (the
						// retained widget opens/navigates/dismisses; a fired
						// leaf's action id maps through `menu_acts`).
						if self.menu_press(&events, event_loop) {
							if let Some(win) = self.win.as_ref() {
								win.window.request_redraw();
							}
							return;
						}
						// Project tab strip next: the retained `TabStrip` widget arms
						// the hit on this press (and fires on release-inside, handled
						// in the Released arm). It consumes the press only when over
						// a tab, so a press on the empty strip area falls through.
						if self.dispatch_layer(Layer::Tabs, &events).wants_pointer() {
							self.redraw_win();
							return;
						}
						// The workspace is the last UI layer above the map: its own
						// chrome (titlebar / close / splitter / dock edge) is consumed
						// inside `Workspace`, a body press routes into that panel, and
						// `Press::None` is the fallthrough - handled after the match, so
						// the map's guard is "no layer above took this press" and
						// nothing else (U1.5).
						match self.editor.workspace.on_press(lcx, lcy, sw, sh) {
							workspace::Press::Chrome => {
								if let Some(win) = self.win.as_ref() {
									win.window.request_redraw();
								}
								return;
							}
							workspace::Press::Body { id, .. } => {
								// A press in a scrollbar gutter used to be intercepted here,
								// before the panel saw it. Since U2 the panel's own
								// `Scroller` takes it, along with the wheel and the paging
								// keys — so a body press goes straight to the panel below.
								//
								// Every fully-retained panel arms the hit under the press
								// through its own `Ui` and fires on release-inside - one
								// dispatch, keyed by panel id. Since U1.6 that is *every*
								// panel: the two palettes were the last shell-driven pair,
								// and U5.3 folded the minimap's hand-run pan in here too.
								self.dispatch_layer(Layer::Panel(id), &events);
								// Not every panel commits on release-inside — see
								// `drain_panel_press` for what a press can fire and why
								// it must be drained before the next dispatch clears it.
								self.drain_panel_press(id, event_loop);
								if let Some(win) = self.win.as_ref() {
									win.window.request_redraw();
								}
								return;
							}
							// The map is below every layer, so `Press::None` is not an arm to
							// handle here - it is the cascade running out. Fall through.
							workspace::Press::None => {}
						}

						// --- The map: the fallthrough layer (U1.5) -------------------------
						// Everything below runs on one condition - that no layer above the
						// map took this press. That condition is the cascade itself: every
						// step from the capture check down to `Workspace::on_press` returns
						// when it consumes, so reaching this line *is* `Over::Map`. There is
						// deliberately no second hit test here to disagree with it.
						//
						// Painting is silently inert without a project + active tile, so bare
						// map clicks don't spam errors.
						if matches!(self.editor.mode, state::EditorMode::Pass | state::EditorMode::LocalPass) {
							// Pass editors: LMB paints (tile passability, or a per-cell
							// override), drag = one undo stroke.
							if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
								self.paint = Some((x, y));
								self.run(Command::Stroke { begin: true }, event_loop);
								self.run(self.paint_command(x, y), event_loop);
							}
							return;
						}
						// An armed ghost stamp takes the click: place it at
						// the cell under the cursor (it stays armed for
						// repeat stamping; Esc disarms).
						if self.editor.stamp.is_some() {
							if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
								self.run(Command::Stamp { x, y }, event_loop);
							}
							return;
						}
						// Alt+drag with a select tool moves the selection
						// marquee (not the terrain); the drag translates it
						// cell by cell. Falls through to the tool otherwise.
						if self.modifiers.alt_key()
							&& matches!(self.editor.tool, state::Tool::Select | state::Tool::SelectRect)
							&& !self.editor.selection.is_empty()
						{
							self.select_move = self.editor.cell_at(self.cursor.0, self.cursor.1);
							return;
						}
						// LMB on the map: the active tool decides.
						match self.editor.tool {
							state::Tool::Picker => {
								if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
									self.run(Command::Pick { x, y }, event_loop);
								}
							}
							// Pencil paints, Eraser erases - both stroke
							// (press + drag = one undo unit).
							state::Tool::Pencil | state::Tool::Eraser => {
								let erasing = self.editor.tool == state::Tool::Eraser;
								if erasing || self.editor.can_paint() {
									if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
										self.paint = Some((x, y));
										self.run(Command::Stroke { begin: true }, event_loop);
										self.run(self.paint_command(x, y), event_loop);
									}
								}
							}
							// Terrain brush: strokes like the pencil (press +
							// drag = one undo unit), but needs no active tile -
							// it paints the land/water mask. The coast grows on
							// release (see the release handler below).
							state::Tool::PaintMask => {
								if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
									self.paint = Some((x, y));
									self.run(Command::Stroke { begin: true }, event_loop);
									self.run(self.paint_command(x, y), event_loop);
								}
							}
							// Resource brush: strokes like the terrain brush (press +
							// drag = one undo unit), painting the current material /
							// amount into the cargo map (S5.3). No-op without a save.
							state::Tool::ResourceBrush => {
								if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
									self.paint = Some((x, y));
									self.run(Command::Stroke { begin: true }, event_loop);
									self.run(self.paint_command(x, y), event_loop);
								}
							}
							// Flood fill: a single click fills the region
							// (its own undo unit - no drag).
							state::Tool::Fill => {
								if self.editor.can_paint() {
									if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
										self.run(Command::Fill { x, y }, event_loop);
									}
								}
							}
							// Place tool: press + drag lays a stroke of units/objects
							// as one undo unit (one per cell). The tool stays armed
							// for repeat placement until cancelled (Esc / context
							// menu), like a template stamp — it never reverts to the
							// pencil on its own.
							state::Tool::Unit => {
								if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
									self.paint = Some((x, y));
									self.run(Command::Stroke { begin: true }, event_loop);
									self.run(Command::Paint { x, y }, event_loop);
								}
							}
							// Unit eraser: press + drag removes every object under the
							// stroke as one undo unit, and stays armed for repeat
							// erasing until cancelled (Esc / context menu).
							state::Tool::UnitEraser => {
								if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
									self.paint = Some((x, y));
									self.run(Command::Stroke { begin: true }, event_loop);
									self.run(Command::UnitErase { x, y }, event_loop);
								}
							}
							// Object Select / Pick: click the topmost object at
							// the cell (the tool stays armed for repeat picks).
							state::Tool::ObjSelect => {
								if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
									self.run(Command::ObjectSelect { x, y }, event_loop);
								}
							}
							state::Tool::ObjPick => {
								if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
									self.run(Command::ObjectPick { x, y }, event_loop);
								}
							}
							// Clone stamp: an object under the cursor becomes the
							// source, a bare cell takes a copy of it.
							state::Tool::ObjClone => {
								if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
									self.run(Command::ObjectClone { x, y }, event_loop);
								}
							}
							// Object Move: grab the object under the cursor and
							// begin a drag (one undo unit); origin follows the
							// cursor minus the grab offset. Motion + release below.
							state::Tool::ObjMove => {
								if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
									if let Some(i) = self.editor.object_at(x, y) {
										let o = &self.editor.project.objects[i];
										self.obj_drag = Some((i, x - o.x, y - o.y));
										self.editor.selected_object = Some(i);
										self.editor.project.begin_stroke();
									}
								}
							}
							// Scenery place: drop the armed cut-out with its
							// footprint origin under the cursor. Free-positioned,
							// so this is a map *pixel*, not a cell. The tool stays
							// armed for repeat placement until cancelled.
							state::Tool::Scenery => {
								if let Some((pack, piece)) = self.editor.armed_scenery() {
									let (x, y) = self.editor.world_at(self.cursor.0, self.cursor.1);
									self.run(Command::SceneryPlace { pack, piece, x, y }, event_loop);
								}
							}
							// Scenery move: grab the object under the cursor and
							// begin a drag (one undo unit). Motion + release below.
							state::Tool::SceneryMove => {
								let (px, py) = self.editor.world_at(self.cursor.0, self.cursor.1);
								if let Some(i) = self.editor.project.scenery_at(px, py) {
									let spot = &self.editor.project.scenery[i];
									self.scenery_drag = Some((i, px - spot.x, py - spot.y));
									// Ring the grabbed piece in the panel, the way the
									// object Move tool selects what it picks up.
									self.editor.active_scenery =
										scenery::index_of(&self.editor.project, &spot.pack, &spot.piece);
									self.editor.project.begin_stroke();
								}
							}
							// Scenery delete: remove the topmost object under the
							// cursor; the tool stays armed for repeat deletes.
							state::Tool::SceneryEraser => {
								let (px, py) = self.editor.world_at(self.cursor.0, self.cursor.1);
								if let Some(index) = self.editor.project.scenery_at(px, py) {
									self.run(Command::SceneryRemove { index }, event_loop);
								}
							}
							// Freehand select: drag paints the mask. Shift
							// adds, Ctrl subtracts; a plain drag starts a
							// fresh selection.
							state::Tool::Select => {
								if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
									let mode = self.select_modifier();
									if mode == map_core::SelectMode::Replace {
										self.run(Command::SelectOp { op: "clear".into() }, event_loop);
									}
									// The stroke continues in Add (or Subtract).
									let mode = match mode {
										map_core::SelectMode::Subtract => map_core::SelectMode::Subtract,
										_ => map_core::SelectMode::Add,
									};
									self.select_paint = Some((mode, (x, y)));
									self.run(Command::SelectCell { x, y, mode }, event_loop);
								}
							}
							// Rect select: anchor on press, preview while
							// dragging, applied on release.
							state::Tool::SelectRect => {
								if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
									self.select_anchor = Some((x, y, self.select_modifier()));
									self.editor.select_preview = Some((x, y, x, y));
									self.redraw_win();
								}
							}
						}
					}
					ElementState::Released => {
						// Repaint the lift: a key that armed on the press draws sunken
						// until something asks for the frame that shows it back up.
						if let Some(win) = self.win.as_ref() {
							win.window.request_redraw();
						}
						// The release is *broadcast* to every layer below, not routed
						// to the one under the cursor: a widget armed on press has to
						// see the release even when the pointer has since left it, or
						// a release-outside (which must cancel the fire) would leave it
						// armed forever. Each dispatch is immediately followed by its
						// outcome poll, so a command one panel runs is visible to the
						// next. U1.3's capture arbitration replaces the broadcast with
						// "the capturing layer, and only it".
						//
						// The console first - it is the topmost of these layers, and it
						// is the one that *must* be here even though it has no outcome
						// poll (its only outcome, a submitted line, comes off Enter). A
						// click in its input line starts a drag-select, and a
						// drag-select captures the pointer; a capture whose owning `Ui`
						// never sees the matching release is never dropped (`Ui` clears
						// `captured` on that release or on `Focus(false)`, and on
						// nothing else). Leave it out and the router keeps routing
						// *every* later pointer event to the console and to nothing
						// else - the rest of the UI goes dead, and closing the console
						// does not help, because a closed console is not rebuilt and so
						// never clears it either.
						self.dispatch_layer(Layer::Console, &events);
						// The retained tab strip's keys fire on release-inside:
						// dispatch the release to its `Ui`, then run whatever the
						// fired tag asked for (harmless when no tab was armed).
						self.dispatch_layer(Layer::Tabs, &events);
						if let Some(act) = self.tabs_outcome() {
							match act {
								tabs::TabAct::Select(i) => self.run(Command::Tab { index: i }, event_loop),
								tabs::TabAct::Close(i) => {
									// Close-x makes the tab active first, then closes -
									// its unsaved-changes guard applies.
									self.run(Command::Tab { index: i }, event_loop);
									self.run(Command::CloseProject { force: false }, event_loop);
								}
							}
							self.redraw_win();
						}
						// The minimap overlay's mode radios fire on release-inside.
						self.dispatch_layer(Layer::Panel("minimap"), &events);
						if let Some(mode) = self.minimap_outcome() {
							self.run(Command::MinimapMode { mode: mode.name().into() }, event_loop);
							self.redraw_win();
						}
						// The toolbox keys and its orientation cells fire on
						// release-inside; its two dropdowns commit on the **press**, so
						// the drain runs after both dispatches (see `drain_toolbox`).
						self.dispatch_layer(Layer::Panel("toolbox"), &events);
						self.drain_toolbox(event_loop);
						// The Save Toolbox's keys fire on release-inside: run the key's
						// command line (all `Act::Run`, validated by a test).
						self.dispatch_layer(Layer::Panel("savetools"), &events);
						if let Some(line) = self.savetools_outcome() {
							if let Ok(Some(cmd)) = command::parse_line(line) {
								self.run(cmd, event_loop);
							}
						}
						// The Pass Types Palette's swatches fire on release-inside, same as
						// the Save Toolbox's keys.
						self.dispatch_layer(Layer::Panel("passtools"), &events);
						if let Some(line) = self.passtools_outcome() {
							if let Ok(Some(cmd)) = command::parse_line(line) {
								self.run(cmd, event_loop);
							}
						}
						// The Unit Properties swatches, connector checkboxes and advanced
						// toggle fire on release-inside; its three dropdowns commit on the
						// **press**, so the drain runs after both dispatches (see
						// `drain_unitprops`). Text-box edits arrive separately as commits
						// (item 8).
						self.dispatch_layer(Layer::Panel("unitprops"), &events);
						self.drain_unitprops(event_loop);
						// A click may have committed the field it moved focus out of (item 8).
						self.drain_unitprops_commits(event_loop);
						// The tile explorer's command keys and its tile grid fire on
						// release-inside; its three dropdowns commit on the **press**, so
						// the drain runs after both dispatches (see `drain_picker`).
						self.dispatch_layer(Layer::Panel("tiles"), &events);
						self.drain_picker(event_loop);
						// The templates command keys and its thumbnail grid fire on
						// release-inside; its two dropdowns commit on the **press**, so
						// the drain runs after both dispatches (see `drain_templates`).
						self.dispatch_layer(Layer::Panel("templates"), &events);
						self.drain_templates(event_loop);
						// The units panel's team swatches, eraser and sprite grid all fire
						// on release-inside, so one poll after this dispatch reads
						// everything the panel produced (see `units_outcome`).
						self.dispatch_layer(Layer::Panel("units"), &events);
						if let Some(action) = self.units_outcome() {
							match action {
								units::Action::Pick(i) => {
									// An index into the live roster, resolved here rather than at
									// press time — the library can be reloaded between the two.
									let tag =
										self.editor.units.as_ref().and_then(|l| l.units.get(i)).map(|u| u.tag.clone());
									if let Some(tag) = tag {
										self.run(Command::UnitSelect { tag: Some(tag) }, event_loop);
									}
								}
								units::Action::Team(t) => {
									self.run(Command::UnitTeam { team: t.to_string() }, event_loop)
								}
								units::Action::Eraser => {
									let name = if self.editor.tool == state::Tool::UnitEraser {
										"pencil"
									} else {
										"unit-eraser"
									};
									self.run(Command::ToolSelect { name: name.into() }, event_loop);
								}
							}
						}
						// The Scenery panel: a grid pick arms a piece (and the Scenery
						// layer's place tool with it). Its thumbnails fire on
						// release-inside; its two dropdowns commit on the **press**, so
						// the drain runs after both dispatches (see `drain_scenery`).
						self.dispatch_layer(Layer::Panel("scenery"), &events);
						self.drain_scenery(event_loop);
						// The palette toolbar / tabs / saved rows and the WRL panel's
						// cycle keys fire on release-inside, the same as every other
						// panel button (U1.6 - this was `fire_armed`, the shell's own
						// re-hit-test at release). Their selections and drag starts
						// already fired at the press.
						self.dispatch_layer(Layer::Panel("palette"), &events);
						self.drain_palette("palette", event_loop);
						self.dispatch_layer(Layer::Panel("wrlpalette"), &events);
						self.drain_palette("wrlpalette", event_loop);
						// A select drag ends: freehand just stops; the rect
						// applies anchor → release cell in one command.
						self.select_paint = None;
						self.select_move = None;
						// An object Move drag ends: commit the whole drag as one undo
						// unit (the stroke opened at press; empty drags close cleanly).
						if self.obj_drag.take().is_some() {
							self.run(Command::Stroke { begin: false }, event_loop);
						}
						// A scenery Move drag ends the same way: the stroke opened at
						// press, so the whole drag commits as one undo unit.
						if self.scenery_drag.take().is_some() {
							self.run(Command::Stroke { begin: false }, event_loop);
						}
						if let Some((ax, ay, mode)) = self.select_anchor.take() {
							self.editor.select_preview = None;
							let (x, y) = self.editor.cell_at(self.cursor.0, self.cursor.1).unwrap_or((ax, ay));
							self.run(Command::SelectRect { x0: ax, y0: ay, x1: x, y1: y, mode }, event_loop);
						}
						if self.editor.workspace.on_release(lcx, lcy, sw, sh) {
							if let Some(win) = self.win.as_ref() {
								win.window.request_redraw();
							}
						} else if self.paint.is_some() {
							self.paint = None;
							// Terrain brush: grow the coast (beach + animated coastal
							// waves) over everything the stroke painted, inside the same
							// undo unit, before the stroke closes - the toolbox "auto
							// shore" select chooses the placement (or off).
							if self.editor.tool == state::Tool::PaintMask {
								if let Some(region) = self.editor.take_mask_region() {
									let mode = match self.editor.brush_shore {
										state::BrushShore::Off => None,
										state::BrushShore::Sweep => Some(command::ShoreMode::Sweep),
										state::BrushShore::LoopWalk => Some(command::ShoreMode::LoopWalk),
									};
									if let Some(mode) = mode {
										self.run(Command::Shore { region: Some(region), mode }, event_loop);
									}
								}
							}
							self.run(Command::Stroke { begin: false }, event_loop);
						}
					}
				}
			}

			WindowEvent::MouseInput { state, button, .. }
				if self.bindings.is_pan_button(button) || button == MouseButton::Right =>
			{
				let (sw, sh) = self.editor.ui_screen();
				let (lcx, lcy) = self.lcursor();
				// The map is the fallthrough layer (U1.5): pan-drag and the context
				// menu are its business only where nothing covers it. The hit test is
				// a UI question (logical space); the pan / context-menu points
				// captured below stay physical (the map).
				let over_map = over_at(&self.editor, self.popup_layer(), lcx, lcy).is_map();
				// A layer mid-drag owns the pointer (U1.3): a stray second button
				// during a drag must not open a context menu or start a pan behind
				// the drag's back.
				if let Some(held) = self.router.capture() {
					self.dispatch_layer(held, &events);
					self.redraw_win();
					return;
				}
				// The panel under the cursor sees secondary/middle buttons too (U1.2):
				// only the primary button reached a panel `Ui` before, so a widget
				// could not tell a right-click from nothing at all. Nothing claims
				// them yet - `ArmFire` and `TextInput` both guard on Primary - so the
				// verdict is deliberately not consulted and the shell's right-click
				// handling below is unchanged.
				if let Some((id, _)) = self.editor.workspace.body_at(lcx, lcy, sw, sh) {
					self.dispatch_layer(Layer::Panel(id), &events);
				}
				match state {
					ElementState::Pressed => {
						self.drag = (over_map && self.bindings.is_pan_button(button)).then_some(self.cursor);
						// A right press might be a click (context menu) or a
						// pan-drag - decided by how far the release lands.
						let right = button == MouseButton::Right && self.editor.context_menu.is_none();
						// A right press on a Templates Explorer thumbnail opens that
						// item's menu (resolved on release-inside) - takes precedence
						// over the map menu, which only fires over the map.
						let on_template = right.then(|| self.template_at(lcx, lcy, sw, sh)).flatten();
						self.rclick_template = on_template.map(|g| (g, self.cursor));
						self.rclick = (right && on_template.is_none() && over_map).then_some(self.cursor);
					}
					ElementState::Released => {
						self.drag = None;
						if button == MouseButton::Right {
							if let Some((g, (px, py))) = self.rclick_template.take() {
								let moved = (self.cursor.0 - px).abs().max((self.cursor.1 - py).abs());
								if moved < 4.0 {
									// Select the right-clicked entry, then open its menu in
									// logical (chrome) space under the cursor.
									self.editor.templates.sel = Some(g);
									let (lcx, lcy) = self.lcursor();
									self.editor.open_template_context_menu((lcx, lcy));
									self.redraw_win();
								}
							} else if let Some((px, py)) = self.rclick.take() {
								let moved = (self.cursor.0 - px).abs().max((self.cursor.1 - py).abs());
								if moved < 4.0 {
									self.run(Command::ContextMenu { at: Some(self.cursor) }, event_loop);
								}
							}
						}
					}
				}
			}

			WindowEvent::MouseWheel { delta, .. } => {
				// A layer mid-drag owns the pointer (U1.3), wheel included - the map
				// must not zoom out from under a drag in progress.
				if let Some(held) = self.router.capture() {
					self.dispatch_layer(held, &events);
					self.redraw_win();
					return;
				}
				// The context menu baked the clicked cell into its items -
				// close it rather than let the view scroll out from under it.
				if self.editor.context_menu.is_some() {
					self.run(Command::ContextMenu { at: None }, event_loop);
				}
				let steps = match delta {
					MouseScrollDelta::LineDelta(_, y) => y,
					MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 60.0,
				};
				// The open console takes the wheel over its own band for the
				// scrollback (no visible bar) - through its widget, like any other
				// hosted layer. Below the band the map is still live, which the
				// old blanket "console is open, swallow the wheel" branch was not.
				if self.editor.console.is_open() {
					let consumed = self.dispatch_layer(Layer::Console, &events).wants_pointer();
					let lines =
						self.win.as_mut().and_then(|w| w.passes.console_view.root_mut()).map_or(0, |c| c.take_scroll());
					if lines != 0 {
						self.editor.console.scroll_lines(lines);
					}
					if consumed {
						self.redraw_win();
						return;
					}
				}
				// Wheel over a panel belongs to the panel (picker scroll, minimap
				// zoom); over the map it zooms at the cursor - and *only* over the
				// map (U1.5). It used to zoom over the menu bar, the tab strip and
				// workspace chrome too, none of which are the map. The hit test is
				// logical; `ZoomAt` below uses the physical cursor (map).
				let (sw, sh) = self.editor.ui_screen();
				let (lcx, lcy) = self.lcursor();
				if over_at(&self.editor, self.popup_layer(), lcx, lcy).is_map() {
					self.run(
						Command::ZoomAt {
							x: self.cursor.0,
							y: self.cursor.1,
							factor: self.bindings.zoom_step().powf(steps),
						},
						event_loop,
					);
				} else if let Some((id, _)) = self.editor.workspace.body_at(lcx, lcy, sw, sh) {
					if id == "minimap" {
						self.run(Command::Zoom { factor: self.bindings.zoom_step().powf(steps) }, event_loop);
					} else if self.dispatch_layer(Layer::Panel(id), &events).wants_pointer() {
						// The panel's own `Ui` takes the wheel (U1.2 routed it here,
						// U2 gave every panel a `Scroller` to consume it). A panel
						// that doesn't scroll simply declines, and the wheel dies
						// here rather than zooming the map behind it.
						self.redraw_win();
					}
				}
			}

			WindowEvent::RedrawRequested => {
				self.redraw(event_loop);
				// THE drain point: every verb an overlay deferred runs here, in
				// enqueue order, with the event loop in scope. New Map's shaped
				// create is the multi-step case - create, then the chosen
				// palette (so Fix Shore previews the final colours), then the
				// shape carve (same two-step the bespoke modal's CreateShaped
				// did).
				while let Some(deferred) = self.deferred.pop_front() {
					match deferred {
						Deferred::Command(command) => self.run(command, event_loop),
						Deferred::Palette(path) => {
							let outcome = self.editor.execute(Command::PaletteLoad { path });
							self.act_on(outcome, event_loop);
						}
						Deferred::Shape(image) => {
							let outcome = self.editor.apply_shape_image(&image);
							self.act_on(outcome, event_loop);
						}
						Deferred::Wrl(act) => self.run_wrl_act(act, event_loop),
						Deferred::Tile(act) => self.run_tile_commit(act, event_loop),
						Deferred::Scenery(act) => self.run_scenery_commit(act, event_loop),
					}
				}
			}

			_ => {}
		}
	}

	fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
		// Persist the UI layout on the way out.
		if self.editor.settings_path.is_some() {
			self.editor.execute(Command::SaveSettings);
		}
		// Drop the surface/device/window while the display connection is
		// still alive. `run_app` consumes the event loop, so anything left
		// in `self.win` would otherwise be destroyed *after* the Wayland/X11
		// connection closes - vkDestroySurfaceKHR then segfaults.
		self.win = None;
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use map_core::Project;
	use std::path::Path;

	fn editor() -> EditorState {
		let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources");
		let project = Project::new(16, 16, &["GREEN".to_string()], &resources.join("assets/tilepacks"), 42).unwrap();
		let mut e = EditorState::new(project, (800, 600), None, resources);
		e.ui_scale = 1.0;
		e
	}

	/// The keyboard owner (U1.4): the open console outranks the router's
	/// press-driven focus, because it is an accelerator *mode* you enter with a
	/// key rather than a panel you clicked into. One resolver, so the keyboard
	/// routing, the keymap gate, the Escape cascade and the IME arbiter cannot
	/// disagree about who holds the keyboard.
	#[test]
	fn the_open_console_outranks_press_driven_focus() {
		let mut e = editor();
		let mut router = ui_router::UiRouter::new(1.0);
		assert_eq!(focus_layer(&e, &router), None, "idle: the map bindings get every key");

		router.refocus(Some(Layer::Panel("unitprops")));
		assert_eq!(focus_layer(&e, &router), Some(Layer::Panel("unitprops")), "a focused field owns the keyboard");

		e.console.set_open(true);
		assert_eq!(focus_layer(&e, &router), Some(Layer::Console), "the open console takes it, whatever was focused");
		e.console.set_open(false);
		assert_eq!(focus_layer(&e, &router), Some(Layer::Panel("unitprops")), "and gives it back on close");
	}

	/// The shell's one pointer hit test (U1.5): the z-order resolved top-down,
	/// with the map at the bottom. Every map-side gate - the tools' press guard,
	/// the pan / context-menu guard, the wheel's zoom, the move arm's redraw test
	/// and the four render ghosts - asks this and nothing else, so none of them
	/// can decide the pointer is over the map while another says it is over UI.
	#[test]
	fn over_at_resolves_the_z_order_with_the_map_underneath() {
		let mut e = editor();
		let (sw, sh) = e.ui_screen();
		// The viewport centre is map space (clear of the docked panels).
		let (mx, my) = (sw / 2.0, sh / 2.0);
		assert_eq!(over_at(&e, None, mx, my), Over::Map, "nothing covers the middle of the map");

		// The two top strips, by height - the menu bar over the tab strip.
		assert_eq!(over_at(&e, None, mx, 1.0), Over::Ui(Layer::MenuBar));
		assert_eq!(over_at(&e, None, mx, menu::BAR_H + 1.0), Over::Ui(Layer::Tabs));

		// A panel answers for its whole rect, titlebar included: `Workspace`
		// routes chrome and body apart itself once the press reaches it.
		let layout = e.workspace.layout(sw, sh);
		let &(i, r) = layout.panels.first().expect("the default layout docks panels");
		let id = e.workspace.panels[i].id.clone();
		match over_at(&e, None, r.x + r.w / 2.0, r.y + 1.0) {
			Over::Ui(Layer::Panel(got)) => assert_eq!(got, id, "the panel whose rect the point is in"),
			other => panic!("a point on a panel titlebar resolved to {other:?}"),
		}

		// Splitters and dock edges host no `Ui` of their own - but they are not
		// the map either, and a press on one must not paint.
		let edge = layout.edges.iter().flatten().next().copied().expect("docked panels have a dock edge");
		assert_eq!(over_at(&e, None, edge.x + edge.w / 2.0, edge.y + edge.h / 2.0), Over::Chrome);

		// The press-modal layers cover the *window*, not their own rect: the next
		// press dismisses them wherever it lands, so nothing beneath may act on
		// it. This is what four of the seven old gates got wrong. (The panel with
		// an open dropdown is the third; it arrives as `popup`, asserted below.)
		assert!(!matches!(e.execute(Command::MenuOpen { name: "file".into() }), Outcome::Failed(_)));
		assert_eq!(over_at(&e, None, mx, my), Over::Ui(Layer::MenuBar), "an open cascade covers the map under it");
		e.menu().close();

		e.execute(Command::ContextMenu { at: Some((mx * e.ui_scale, my * e.ui_scale)) });
		assert_eq!(over_at(&e, None, mx, my), Over::Ui(Layer::ContextMenu));
		assert_eq!(over_at(&e, None, 1.0, 1.0), Over::Ui(Layer::ContextMenu), "topmost: even over the menu bar");
		e.context_menu = None;
		assert_eq!(over_at(&e, None, mx, my), Over::Map, "dismissed - the map is back");

		// A panel whose hosted `Ui` reports an open popup is press-modal the same
		// way (U3.2) - and it wins *everywhere*, because its list can hang well
		// outside the panel that owns it.
		let tiles = Layer::Panel("tiles");
		assert_eq!(over_at(&e, Some(tiles), mx, my), Over::Ui(tiles), "over the map");
		assert_eq!(over_at(&e, Some(tiles), 1.0, 1.0), Over::Ui(tiles), "and over the menu bar");
		let &(_, pr) = layout.panels.first().expect("the default layout docks panels");
		assert_eq!(over_at(&e, Some(tiles), pr.x + 1.0, pr.y + 1.0), Over::Ui(tiles), "and over another panel");
		// Only the context menu outranks it (it is dismissed by the same press).
		e.execute(Command::ContextMenu { at: Some((mx * e.ui_scale, my * e.ui_scale)) });
		assert_eq!(over_at(&e, Some(tiles), mx, my), Over::Ui(Layer::ContextMenu));
		e.context_menu = None;
	}

	/// **Every visible panel is reachable through all three routing tables.**
	///
	/// A hosted panel is addressed by id in three places, and they are written
	/// out by hand: [`workspace::PANEL_IDS`] (which `body_at` / `on_press` map a
	/// model id through), [`panel_host`] (the mutable dispatch target) and
	/// [`layer_panel`] (the shared read for focus / IME / popup state). Miss one
	/// and the panel still lays out, draws and animates - it just never receives
	/// a press, a wheel notch or a paging key, which is how the Scenery panel
	/// shipped: a full grid of thumbnails, two dropdowns, a scrollbar, and not
	/// one of them answering the mouse.
	///
	/// `Layer::HOSTED` is the fourth, and it is what the two shell-wide
	/// broadcasts (pointer-left, focus-loss) and [`popup_layer`] walk - so a
	/// panel missing from it strands its hover and its open dropdown is never
	/// recognised as press-modal.
	#[test]
	fn every_panel_is_reachable_through_every_routing_table() {
		let mut e = editor();
		let (device, queue, _serial) = crate::visual_test::test_gpu();
		let mut passes = Passes::new(&device, &queue, crate::capture::FORMAT);
		let ids: Vec<String> = e.workspace.panels.iter().map(|p| p.id.clone()).collect();
		assert!(ids.iter().any(|id| id == "scenery"), "the fixture holds the panel this test was written for");

		for id in &ids {
			// `body_at` is the id every pointer path routes on, so it must survive
			// the model->'static mapping: show the panel, aim at its body.
			e.workspace.show(id, Some(true)).expect("a known panel");
			let (sw, sh) = e.ui_screen();
			let i = e.workspace.find(id).expect("a known panel");
			let r = e
				.workspace
				.layout(sw, sh)
				.panels
				.iter()
				.find(|(p, _)| *p == i)
				.map(|(_, r)| *r)
				.expect("a shown panel is in the layout");
			let body = e.workspace.body_of(i, r);
			let hit = e.workspace.body_at(body.x + body.w / 2.0, body.y + body.h / 2.0, sw, sh);
			assert_eq!(hit.map(|(got, _)| got), Some(id.as_str()), "{id}: body_at loses the id");

			let layer = Layer::Panel(hit.expect("hit").0);
			assert!(panel_host(&mut passes, id).is_some(), "{id}: no panel_host arm - every press is dropped");
			assert!(layer_panel(&passes, &e, layer).is_some(), "{id}: no layer_panel arm - focus / IME are blind");
			assert!(Layer::HOSTED.contains(&layer), "{id}: not in Layer::HOSTED - hover and popups go stale");
			assert!(panel_has_content(id), "{id}: not in panel_has_content - its body draws the placeholder hint");
		}
		assert_eq!(
			Layer::HOSTED.iter().filter(|l| matches!(l, Layer::Panel(_))).count(),
			ids.len(),
			"and HOSTED names no panel the workspace does not have"
		);
	}

	/// **The chrome cursor is gated on the z-order, not just on geometry.**
	///
	/// `Workspace::cursor_at` answers about *its* model and nothing else, so over
	/// a dock edge it reports a resize arrow whether or not an open menu cascade
	/// is drawn on top of that dock edge. `desired_cursor` composes it with
	/// `over_at` for exactly that reason: moving the pointer down an open
	/// submenu used to flip the cursor to a resize arrow wherever the item
	/// happened to sit over a splitter, which reads as though the menu were not
	/// there at all.
	///
	/// The pieces are asserted here (the composition itself needs a live window):
	/// the workspace still claims the arrow, and `over_frame(over_at(..))` - the
	/// gate - stops agreeing the moment anything press-modal is up.
	#[test]
	fn an_open_popup_suppresses_the_workspace_cursor_beneath_it() {
		let mut e = editor();
		let (sw, sh) = e.ui_screen();
		let edge = e
			.workspace
			.layout(sw, sh)
			.edges
			.iter()
			.flatten()
			.next()
			.copied()
			.expect("the default layout docks panels, so it has a dock edge");
		let (ex, ey) = (edge.x + edge.w / 2.0, edge.y + edge.h / 2.0);
		// The gate exactly as `desired_cursor` and `resync_hover_layer` compose it.
		let frame = |e: &EditorState, popup, x, y| over_frame(over_at(e, popup, x, y), popup);

		// Uncovered: the frame owns this point, and the arrow is right.
		assert!(frame(&e, None, ex, ey), "an uncovered dock edge is the frame's");
		let arrow = e.workspace.cursor_at(ex, ey, sw, sh);
		assert_ne!(arrow, wgpu_ui::CursorIcon::Default, "the workspace does claim a resize arrow here");

		// An open cascade covers it. The workspace has not changed its mind - it
		// cannot, it knows nothing about menus - so the gate is what must.
		assert!(!matches!(e.execute(Command::MenuOpen { name: "file".into() }), Outcome::Failed(_)));
		assert_eq!(e.workspace.cursor_at(ex, ey, sw, sh), arrow, "the workspace still says resize");
		assert!(!frame(&e, None, ex, ey), "but the gate no longer hands it the pointer");
		e.menu().close();

		// Same for a panel's open dropdown - the case `over` alone cannot see,
		// since a press-modal panel and a panel under the pointer are the same
		// `Over` value. Its list can hang anywhere, including over this edge.
		assert!(!frame(&e, Some(Layer::Panel("tiles")), ex, ey), "an open dropdown covers it too");
		// And for the context menu.
		e.execute(Command::ContextMenu { at: Some((ex * e.ui_scale, ey * e.ui_scale)) });
		assert!(!frame(&e, None, ex, ey));
		e.context_menu = None;
		assert!(frame(&e, None, ex, ey), "dismissed - the frame has it back");

		// The exception the gate must not swallow, and why it is checked *first*:
		// mid-gesture the workspace owns the pointer wherever it has wandered to.
		// Drag this dock edge up into the reserved top strip - the pointer is over
		// the menu bar, which the gate rightly refuses to call the frame, while
		// the resize is still live and must keep its arrow.
		let (mx, top) = (sw / 2.0, 2.0);
		e.workspace.on_press(ex, ey, sw, sh);
		e.workspace.on_move(mx, top, sw, sh);
		assert!(e.workspace.dragging(), "a live dock-edge drag");
		assert_eq!(over_at(&e, None, mx, top), Over::Ui(Layer::MenuBar), "dragged up under the menu bar");
		assert!(!frame(&e, None, mx, top), "so the gate alone would drop the resize arrow mid-resize");
		assert_eq!(e.workspace.cursor_at(mx, top, sw, sh), arrow, "which is why `dragging()` is checked first");
		e.workspace.on_release(mx, top, sw, sh);
		assert!(!e.workspace.dragging(), "and the release hands the pointer back");
	}

	/// The hover retarget a menu opening owes the panel beneath it (U5.2). Until
	/// U5 the shell simply handed every panel `Hot::NONE` while anything
	/// press-modal was open, so a converted panel — whose hover is its own `Ui`'s
	/// — would keep its highlight lit behind an open cascade. Opening a menu from
	/// the keyboard, a script line or an accelerator moves no pointer, so the
	/// `CursorMoved` arm never runs and only `resync_hover_layer` closes the gap.
	///
	/// This is the composition `resync_hover_layer` performs, on its two pieces:
	/// `over_at` stops naming the panel, so `retarget` names it as the layer left
	/// — the one that must be sent `Event::PointerLeft`.
	#[test]
	fn opening_a_menu_hands_the_panel_under_the_pointer_its_leave() {
		let mut e = editor();
		let mut router = ui_router::UiRouter::new(1.0);
		let (sw, sh) = e.ui_screen();
		let layout = e.workspace.layout(sw, sh);
		let &(i, r) = layout.panels.first().expect("the default layout docks panels");
		let id = e.workspace.panels[i].id.clone();
		// The pointer rests inside a panel body; nothing covers it.
		let (px, py) = (r.x + r.w / 2.0, r.y + r.h - 2.0);

		let target = |e: &EditorState| match over_at(e, None, px, py) {
			Over::Ui(layer @ Layer::Panel(_)) => Some(layer),
			_ => None,
		};

		let panel = target(&e).expect("the pointer is over a panel");
		assert_eq!(router.retarget(panel.into()), None, "entering it leaves nothing behind");

		// The keyboard/script path: the cascade opens without a pointer event.
		assert!(!matches!(e.execute(Command::MenuOpen { name: "file".into() }), Outcome::Failed(_)));
		assert_eq!(target(&e), None, "the open cascade covers the panel, wherever the pointer is");
		assert_eq!(
			router.retarget(None),
			Some(panel),
			"so the panel is named as the layer left - the shell owes it a PointerLeft"
		);

		// And closing it hands the panel back, so it can light again.
		e.menu().close();
		assert_eq!(target(&e), Some(panel), "dismissed - the panel answers for its body again");
		assert_eq!(router.retarget(target(&e)), None, "nothing else was holding the hover");
		assert_eq!(id, e.workspace.panels[i].id, "the panel under test never moved");
	}

	/// The same debt to the **tab strip** (U6.1). Until this ticket the strip
	/// took the shell's `shell_hot`, which went inert whenever a menu was open,
	/// so no tab could stay lit behind a cascade. Its hover is now its own `Ui`'s,
	/// which makes it just another hosted layer — and the retarget the panels
	/// already rely on is what de-lights it.
	#[test]
	fn opening_a_menu_hands_the_tab_strip_its_leave() {
		let mut e = editor();
		let mut router = ui_router::UiRouter::new(1.0);
		// A point inside the strip, below the menu bar.
		let (sx, sy) = (40.0, menu::BAR_H + tabs::BAR_H / 2.0);
		let target = |e: &EditorState| match over_at(e, None, sx, sy) {
			Over::Ui(Layer::Tabs) => Some(Layer::Tabs),
			_ => None,
		};

		assert_eq!(target(&e), Some(Layer::Tabs), "the pointer rests on the strip");
		assert_eq!(router.retarget(Some(Layer::Tabs)), None, "entering it leaves nothing behind");

		assert!(!matches!(e.execute(Command::MenuOpen { name: "file".into() }), Outcome::Failed(_)));
		assert_eq!(target(&e), None, "the open cascade covers the strip");
		assert_eq!(router.retarget(None), Some(Layer::Tabs), "so the strip is owed its PointerLeft");

		e.menu().close();
		assert_eq!(target(&e), Some(Layer::Tabs), "dismissed - the strip answers for its band again");
	}

	/// And the same debt to the **workspace frame** (U6.2), which is not a hosted
	/// `Ui` at all: nothing dispatches to it, so `resync_hover_layer` reads
	/// `over_frame` and hands the model its own leave. Until this ticket the
	/// frame's close `x` lit off the shell's `Hot`, which went inert with
	/// everything else while a cascade was open.
	#[test]
	fn opening_a_menu_hands_the_workspace_frame_its_leave() {
		let mut e = editor();
		let (sw, sh) = e.ui_screen();
		let layout = e.workspace.layout(sw, sh);
		let &(i, r) = layout.panels.first().expect("the default layout docks panels");
		let close = e.workspace.close_of(i, r);
		let (px, py) = (close.x + 2.0, close.y + 2.0);

		// The pointer rests on a panel's close `x`: the frame is the thing under it.
		assert!(over_frame(over_at(&e, None, px, py), None), "a panel rect covers its titlebar");
		e.workspace.on_move(px, py, sw, sh);
		assert!(e.workspace.close_hovered(i, r), "so the x lights");

		// The keyboard/script path: the cascade opens without a pointer event.
		assert!(!matches!(e.execute(Command::MenuOpen { name: "file".into() }), Outcome::Failed(_)));
		assert!(!over_frame(over_at(&e, None, px, py), None), "the open cascade covers the frame");
		e.workspace.on_pointer_left();
		assert!(!e.workspace.close_hovered(i, r), "and the leave puts the x out");

		// A panel's open dropdown covers the frame the same way - and this is the
		// case `over` alone cannot see, since a press-modal panel and the panel
		// under the pointer are one `Over` value. Without the popup argument the
		// close `x` under an open dropdown stayed lit and clickable.
		e.menu().close();
		let popup = Some(Layer::Panel("tiles"));
		assert!(!over_frame(over_at(&e, popup, px, py), popup), "an open dropdown covers it too");
	}

	/// Escape is the one key the shell wants back from a focused field, so it is
	/// matched on the *press* only - a release must not blur the field a second
	/// time, nor let the shell's cascade fire twice off one keystroke.
	#[test]
	fn escape_press_matches_the_press_and_nothing_else() {
		let esc =
			|pressed| Event::Key { key: wgpu_ui::Key::Escape, pressed, repeat: false, mods: wgpu_ui::Modifiers::NONE };
		assert!(escape_press(&[esc(true)]));
		assert!(escape_press(&[Event::PointerLeft, esc(true)]), "found among other events");
		assert!(!escape_press(&[esc(false)]), "the release is not a second Escape");
		assert!(!escape_press(&[]), "no events");
		assert!(
			!escape_press(&[Event::Key {
				key: wgpu_ui::Key::Enter,
				pressed: true,
				repeat: false,
				mods: wgpu_ui::Modifiers::NONE,
			}]),
			"Enter commits a field; it is not the cascade key"
		);
	}

	/// Every placed object is framed in its team's colour, the selected one in a
	/// visibly thicker band. The unselected hairlines follow the Units overlay
	/// toggle; the selection's own band does not - it is selection chrome.
	#[test]
	fn every_object_is_framed_and_the_selected_one_thicker() {
		let mut e = editor(); // 16×16
		let obj = |tag: &str, x, y, team| map_core::MapObject {
			unit_type: max_assets::save::unit_type_id(tag).unwrap(),
			x,
			y,
			team,
			props: map_core::ObjectProps::default(),
		};
		e.project.place_object(obj("TANK", 2, 2, 0));
		e.project.place_object(obj("TANK", 5, 2, 2));
		e.project.place_object(obj("COMMTWR", 8, 8, 2)); // 2×2

		// A frame is four fills, so the command count is the object count × 4.
		let bands = |e: &EditorState| -> Vec<(ui::Rect, wgpu_ui::Rgba)> {
			object_frames(e, 800.0, 600.0)
				.cmds
				.iter()
				.filter_map(|c| match c {
					wgpu_ui::DrawCmd::Solid { rect, color } => Some((*rect, *color)),
					_ => None,
				})
				.collect()
		};
		let all = bands(&e);
		assert_eq!(all.len(), 3 * 4, "one four-fill frame per placed object");
		let red = rgba(crate::units::TEAM_SWATCH[0]);
		let blue = rgba(crate::units::TEAM_SWATCH[2]);
		assert_eq!(all.iter().filter(|(_, c)| *c == red).count(), 4, "the red tank's frame is red");
		assert_eq!(all.iter().filter(|(_, c)| *c == blue).count(), 8, "and the two blue objects' are blue");
		let thin = all.iter().map(|(r, _)| r.h.min(r.w)).fold(f32::MAX, f32::min);
		assert_eq!(thin, 1.0, "an unselected object gets a hairline");

		// The 2×2 building's frame spans its whole footprint, not one cell.
		let side = |i: usize| render::TILE_PX as f32 * e.view.zoom * e.object_footprint_of(i) as f32;
		let widest = all.iter().map(|(r, _)| r.w).fold(0.0_f32, f32::max);
		assert!(widest > side(0), "the 2x2 is framed as one box, wider than a single cell's");

		// Selecting one thickens its band and leaves the others alone.
		e.selected_object = Some(0);
		let sel = bands(&e);
		assert_eq!(sel.len(), 3 * 4, "still one frame each");
		let thickest = sel.iter().map(|(r, _)| r.h.min(r.w)).fold(0.0_f32, f32::max);
		assert_eq!(thickest, 3.0, "the picked object reads as picked from across the map");
		assert_eq!(sel.iter().filter(|(r, _)| r.h.min(r.w) == 3.0).count(), 4, "and only it");

		// Hiding the units hides their hairlines - but not the selection's band,
		// which the properties panel is editing whether sprites show or not.
		e.show_units = false;
		let hidden = bands(&e);
		assert_eq!(hidden.len(), 4, "only the selected object's frame survives");
		assert!(hidden.iter().all(|(r, _)| r.h.min(r.w) == 3.0), "and it is still the thick one");
		e.selected_object = None;
		assert!(object_frames(&e, 800.0, 600.0).cmds.is_empty(), "nothing selected, units hidden -> nothing drawn");
	}

	/// The paint tool's tile ghost previews the active tile over the cells a
	/// click would place it on - and only for the tile-placing tools.
	#[test]
	fn paint_ghost_previews_the_active_tile_for_paint_tools_only() {
		let mut e = editor();
		e.tool = state::Tool::Pencil;
		// The pencil is armed but no tile is picked yet → nothing to preview.
		assert!(paint_ghost_quads(&e, None).is_none(), "no ghost without an active tile");

		// Pick a tile and hover the map's centre cell (its screen centre is over
		// the map, clear of the side docks).
		assert!(!matches!(e.execute(Command::Tile { spec: Some("GLa000".into()) }), Outcome::Failed(_)));
		let (mx, my) = (e.project.width / 2, e.project.height / 2);
		let r = map_cell_rect(&e, mx, my);
		e.cursor = Some((r.x + r.w / 2.0, r.y + r.h / 2.0));

		// Pencil, brush size 1 → one ghost quad over the hovered cell, showing
		// the resolved tile's art + transform.
		let (tile, _) = e.project.resolve_ref("GLa000").unwrap();
		let quads = paint_ghost_quads(&e, None).expect("pencil + tile + over map -> ghost");
		assert_eq!(quads.len(), 1);
		assert_eq!(quads[0].index, picker::global_index(&e.project, tile));
		assert_eq!(quads[0].transform, tile.transform.bits());
		assert_eq!(quads[0].rect, map_cell_rect(&e, mx, my), "the ghost sits on the hovered cell");

		// Pencil follows the brush footprint; Fill previews just the hovered cell.
		e.brush_size = 3;
		assert_eq!(
			paint_ghost_quads(&e, None).unwrap().len(),
			e.brush_cells(mx, my).len(),
			"pencil ghosts the footprint"
		);
		e.tool = state::Tool::Fill;
		assert_eq!(paint_ghost_quads(&e, None).unwrap().len(), 1, "fill previews only the hovered cell");
		e.brush_size = 1;
		e.tool = state::Tool::Pencil;

		// Suppressed for non-placing tools, in pass mode, off the map, and while a
		// stamp is armed (the stamp shows its own ghost).
		e.tool = state::Tool::Eraser;
		assert!(paint_ghost_quads(&e, None).is_none(), "the eraser has no tile ghost");
		e.tool = state::Tool::Pencil;
		e.mode = state::EditorMode::Pass;
		assert!(paint_ghost_quads(&e, None).is_none(), "no tile ghost in pass mode");
		e.mode = state::EditorMode::Map;
		e.cursor = Some((r.x + r.w / 2.0, r.y + r.h / 2.0));
		assert!(paint_ghost_quads(&e, None).is_some(), "restored: pencil over the map ghosts again");
		e.cursor = None;
		assert!(paint_ghost_quads(&e, None).is_none(), "no cursor -> no ghost");
	}

	/// The placement ghost previews the active unit on the hovered cell, and only
	/// while the Place tool is armed over the map (not another tool / off the map).
	#[test]
	fn unit_ghost_placement_gates_on_tool_selection_and_cursor() {
		use crate::units::{UnitEntry, UnitLibrary};
		let mut e = editor();
		e.units = Some(UnitLibrary::new(vec![UnitEntry {
			tag: "TANK".into(),
			frames: vec![],
			shadow: vec![],
			data: Default::default(),
			footprint: 1,
		}]));
		e.tool = state::Tool::Unit;
		e.active_unit = Some(0);
		// Hover the centre cell (clear of the side docks).
		let (mx, my) = (e.project.width / 2, e.project.height / 2);
		let r = map_cell_rect(&e, mx, my);
		e.cursor = Some((r.x + r.w / 2.0, r.y + r.h / 2.0));

		let tank = max_assets::save::unit_type_id("TANK").unwrap();
		assert_eq!(unit_ghost_placement(&e, None), Some((tank, mx, my)), "the active unit ghosts on the hovered cell");

		// No unit selected → no ghost.
		e.active_unit = None;
		assert!(unit_ghost_placement(&e, None).is_none(), "no unit selected -> no ghost");
		e.active_unit = Some(0);

		// Only the Place tool ghosts (the eraser doesn't preview a placement).
		e.tool = state::Tool::UnitEraser;
		assert!(unit_ghost_placement(&e, None).is_none(), "the eraser has no placement ghost");
		e.tool = state::Tool::Unit;
		assert!(unit_ghost_placement(&e, None).is_some(), "restored under the Place tool");

		// An armed template stamp owns its own ghost; off the map → nothing.
		e.cursor = None;
		assert!(unit_ghost_placement(&e, None).is_none(), "off the map -> no ghost");
	}
}
