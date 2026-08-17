//! Visual-regression snapshots for the editor's panels / components / windows.
//! Each test builds the component's `DrawList` with the chrome's steel theme +
//! fonts and renders it through `MenuChrome::render_list` (see
//! [`crate::visual_test`]). Regenerate with `UPDATE_SNAPSHOTS=1`.
//!
//! Panels whose *pixels* come from a native GPU pass are captured in their
//! chrome/geometry state, with the native layer noted per test: the minimap map
//! texture (a blit), the toolbox active-tile preview quad, the picker/templates
//! tile stills (here uv'd from the steel atlas so they stay deterministic), and
//! the units sprite thumbnails (the no-library state is snapshotted instead —
//! sprites need retail M.A.X. data). Pure-logic modules with no draw entry are
//! not snapshotted here: `cellgrid` (shared grid geometry), `genform` (the
//! Generate form's data logic — its widgets live in a modal dialog), and
//! `packlist` (the tilepack-selection model — its UI is the New Map modal).

use map_core::Project;
use wgpu_ui::widget::{DrawCtx, DrawPass, LayoutCtx, Widget};
use wgpu_ui::{DrawList, TexRect, TextureId, Theme, WidgetId};

use crate::state::EditorState;
use crate::ui::Rect;
use crate::uikit_menu::MenuChrome;
use crate::visual_test::{BACKDROP, chrome_fixture, snapshot_list};

/// The editor's `resources/` root (tilepacks, skin).
fn resources() -> std::path::PathBuf {
	std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources")
}

/// A fresh 8×8 GREEN project + editor - the standard populated document the
/// panel snapshots read (seeded, so the tiles/palette are deterministic).
fn green_editor() -> EditorState {
	let res = resources();
	let project = Project::new(8, 8, &["GREEN".to_string()], &res.join("assets/tilepacks"), 42).unwrap();
	EditorState::new(project, (800, 600), None, res)
}

/// A deterministic 256-colour gradient palette (768 bytes) for the palette panel
/// snapshots - unique colours, so the duplicate-warning path stays quiet.
fn gradient_palette() -> Vec<u8> {
	let mut p = vec![0u8; 768];
	for i in 0..256usize {
		p[i * 3] = (i as u8).wrapping_mul(3);
		p[i * 3 + 1] = (i as u8).wrapping_mul(5);
		p[i * 3 + 2] = (i as u8).wrapping_mul(7);
	}
	p
}

/// Arrange a retained panel `widget` into `body` and draw its base pass through
/// the chrome's steel theme + fonts, returning the composited `DrawList` (the
/// same base-pass paint the shell composites each frame). Immutable-borrows the
/// chrome, so it drops before the `&mut chrome` snapshot call.
fn render_widget<W: Widget>(chrome: &MenuChrome, widget: &mut W, body: Rect) -> DrawList {
	let theme: &dyn Theme = chrome.theme();
	let fonts = chrome.fonts();
	let mut lctx = LayoutCtx { fonts, theme, scale: 1.0, viewport: wgpu_ui::Rect::ZERO };
	widget.arrange(body, &mut lctx);
	let ctx =
		DrawCtx { fonts, theme, scale: 1.0, hovered: WidgetId::NONE, focused: WidgetId::NONE, pass: DrawPass::Base };
	let mut dl = DrawList::new();
	widget.draw(&mut dl, &ctx);
	dl
}

/// The palette panel (full, editable): a 256-swatch gradient with slot 64
/// selected, so the editor strip shows its channel sliders under the grid.
#[test]
fn panel_palette() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (260u32, 460u32);
	let pal = gradient_palette();
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let mut content = crate::palette_panel::PaletteContent::new();
	content.sync(crate::palette_panel::Snapshot::of(
		&pal,
		&pal,
		Some(64),
		None,
		&[],
		false,
		true,
		false,
		&[],
		None,
		false,
	));
	let dl = render_widget(&chrome, &mut content, body);
	snapshot_list(&device, &queue, &mut chrome, "panel_palette", w, h, BACKDROP, &dl);
}

/// The palette panel's saved-palettes tab: the toolbar switched to the saved
/// list, six named palettes, row 1 selected (accent well).
#[test]
fn panel_palette_saved() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (260u32, 460u32);
	let pal = gradient_palette();
	let saved: Vec<String> = (0..6).map(|i| format!("palette-{i}")).collect();
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let mut content = crate::palette_panel::PaletteContent::new();
	content.sync(crate::palette_panel::Snapshot::of(
		&pal,
		&pal,
		None,
		None,
		&[],
		false,
		true,
		true,
		&saved,
		Some(1),
		true,
	));
	let dl = render_widget(&chrome, &mut content, body);
	snapshot_list(&device, &queue, &mut chrome, "panel_palette_saved", w, h, BACKDROP, &dl);
}

/// The bare (WRL Internal Palette) panel: the same swatch grid + editor strip as
/// the full panel, but header-only chrome and read-only (no toolbar, no sliders).
#[test]
fn panel_palette_bare() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (260u32, 460u32);
	let pal = gradient_palette();
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let mut content = crate::palette_panel::PaletteContent::new();
	content.sync(crate::palette_panel::Snapshot::of_bare(&pal, &pal, Some(100), None, false));
	let dl = render_widget(&chrome, &mut content, body);
	snapshot_list(&device, &queue, &mut chrome, "panel_palette_bare", w, h, BACKDROP, &dl);
}

/// The Templates Explorer: the flowed header — six command keys, the tileset and
/// preview-size dropdowns and the count — over three composed-thumbnail entries
/// (uv'd from the steel atlas; the live template atlas is a GPU-composed one),
/// entry 1 selected, WxH badges at the 64px preview size (stock entries dim
/// their name ink). A real widget tree since U5.5, so every key here is a stock
/// `Button` pinned with `sized`, each dropdown a `Select` sized to its own
/// widest option, and the grid a `TemplatesGrid` content widget.
#[test]
fn panel_templates_grid() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (280u32, 400u32);
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let mk = |name: &str, stock: bool, tw: u16, th: u16| crate::state::TemplateEntry {
		name: name.to_string(),
		path: std::path::PathBuf::new(),
		stock,
		template: map_core::Template {
			name: name.to_string(),
			width: tw,
			height: th,
			uses: Vec::new(),
			cells: Vec::new(),
		},
	};
	let (a, b, c) = (mk("crossing", true, 8, 6), mk("bunker", false, 4, 4), mk("ridge", true, 6, 3));
	let entries = [&a, &b, &c];
	let fracs = [(1.0f32, 0.75), (1.0, 1.0), (1.0, 0.5)];
	let atlas = crate::templates_panel::ThumbAtlas { tex: TextureId::ATLAS, cols: 4, rows: 2, fracs: &fracs };
	let mut content = crate::templates_panel::TemplatesContent::new();
	content.sync(crate::templates_panel::Snapshot::of(
		&entries,
		&[0, 1, 2],
		Some(&atlas),
		Some(1),
		64.0,
		None,
		vec!["GREEN".to_string()],
	));
	let dl = render_widget(&chrome, &mut content, body);
	snapshot_list(&device, &queue, &mut chrome, "panel_templates_grid", w, h, BACKDROP, &dl);
}

/// The Templates Explorer with no matching templates: the header still flows,
/// and the clipped grid draws its explanatory note instead of thumbnails.
#[test]
fn panel_templates_empty() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (280u32, 160u32);
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let mut content = crate::templates_panel::TemplatesContent::new();
	content.sync(crate::templates_panel::Snapshot::of(&[], &[], None, None, 64.0, None, Vec::new()));
	let dl = render_widget(&chrome, &mut content, body);
	snapshot_list(&device, &queue, &mut chrome, "panel_templates_empty", w, h, BACKDROP, &dl);
}

/// The Tile Editing toolbox docked narrow: the flowed groups wrap into stacked
/// runs (tile+draw / brush+shape+shore / layer+selection+advanced), the "tile"
/// preview well empty ("none" — the native preview quad is a GPU pass), default
/// tool state. Since the icon-grid re-costume every command key is a **square
/// stencil-faced `Button`** (its name on the tooltip), each dropdown a `Select`
/// sized to its own widest option, and the `advanced` placeholders read
/// **muted** (G4) rather than disabled. Tall enough to hold the whole flow — a
/// shorter body scrolls (that is `a_narrow_dock_wraps_its_runs_and_scrolls`),
/// and a golden that clipped its last group would stop watching it.
#[test]
fn panel_toolbox() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (280u32, 270u32);
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let editor = green_editor();
	let mut content = crate::toolbox::ToolboxContent::new();
	content.sync(crate::toolbox::Snapshot::of(&editor));
	let dl = render_widget(&chrome, &mut content, body);
	snapshot_list(&device, &queue, &mut chrome, "panel_toolbox", w, h, BACKDROP, &dl);
}

/// The same toolbox docked wide (its default, along the bottom): every block on
/// one run — the horizontal orientation of the icon grid, watched separately so
/// a reflow regression in either aspect shows up in its own golden.
#[test]
fn panel_toolbox_wide() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (800u32, 130u32);
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let editor = green_editor();
	let mut content = crate::toolbox::ToolboxContent::new();
	content.sync(crate::toolbox::Snapshot::of(&editor));
	let dl = render_widget(&chrome, &mut content, body);
	snapshot_list(&device, &queue, &mut chrome, "panel_toolbox_wide", w, h, BACKDROP, &dl);
}

/// The Pass Types Palette docked wide (its default, along the bottom): the four
/// pass swatches as a 2×2 block of square palette chips with `land` armed (its
/// accent ring), beside the cell tally of an 8×8 GREEN map — counts and shares
/// as the panel reads them off `Project::pass_counts`, plus the per-cell
/// override row. A real widget tree: the chips are stock `ColorButton`s (names
/// on their tooltips), the tally stock right-aligned `Label`s that `sync`
/// rewrites in place.
#[test]
fn panel_passtools() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (420u32, 140u32);
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let editor = green_editor();
	let mut content = crate::passtools::PassToolsContent::new();
	content.sync(crate::passtools::Snapshot::of(&editor));
	let dl = render_widget(&chrome, &mut content, body);
	snapshot_list(&device, &queue, &mut chrome, "panel_passtools", w, h, BACKDROP, &dl);
}

/// The Save Toolbox docked wide (its default, along the bottom): the six key
/// blocks on one run — square icon keys, square team-swatch chips and the
/// text-faced amount presets — with the object tool, the red team and the
/// brush's resting material+mode lit. The first panel converted to a real
/// widget tree (U5.2), and since the icon-grid re-costume every command key is
/// a stencil-faced `Button` (name on the tooltip) in a `Wrap` in a
/// `ScrollArea`.
#[test]
fn panel_savetools() {
	let (device, queue, mut chrome) = chrome_fixture();
	// Wide enough for the seven blocks to sit on one run, the way the real
	// bottom dock hosts them.
	let (w, h) = (800u32, 110u32);
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let mut editor = green_editor();
	editor.tool = crate::state::Tool::ObjSelect;
	let mut content = crate::savetools::SaveToolsContent::new();
	content.sync(crate::savetools::Snapshot::of(&editor));
	let dl = render_widget(&chrome, &mut content, body);
	snapshot_list(&device, &queue, &mut chrome, "panel_savetools", w, h, BACKDROP, &dl);
}

/// The bottom status bar: the tool hint on the left, the cursor-cell readout on
/// the right, over the steel strip + lit top seam.
#[test]
fn panel_statusbar() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (460u32, 22u32);
	let editor = green_editor();
	let dl = {
		let mut bar = crate::statusbar::StatusBar::new();
		bar.build(&chrome, &editor, None, Some((12, 5)), w as f32, h as f32, 1.0)
	};
	snapshot_list(&device, &queue, &mut chrome, "panel_statusbar", w, h, BACKDROP, &dl);
}

/// The status bar while a tooltip-carrying key is hovered: the key's hint text
/// mirrors into the left slot in place of the tool hint (and leaving restores
/// the tool hint by plain recomputation — the `None` case above).
#[test]
fn panel_statusbar_hover_hint() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (460u32, 22u32);
	let editor = green_editor();
	let dl = {
		let mut bar = crate::statusbar::StatusBar::new();
		bar.build(&chrome, &editor, Some("erase resources"), Some((12, 5)), w as f32, h as f32, 1.0)
	};
	snapshot_list(&device, &queue, &mut chrome, "panel_statusbar_hover_hint", w, h, BACKDROP, &dl);
}

/// A dropdown whose option list cannot show all twenty options at once (the
/// `max_visible` cap the Edit Save Data unit picker uses): the popup grows the
/// editor's scrollbar down its right-hand column, the rows stop short of it,
/// and two wheel notches have walked the window to the middle of the list -
/// where the thumb rides its track. The bar is the theme's own `scrollbar`, the
/// one every panel body and text area paints.
#[test]
fn select_popup_scrollbar() {
	use wgpu_ui::event::{Event, Modifiers, PointerButton, ScrollDelta};
	use wgpu_ui::{Select, Ui, Vec2};

	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (220u32, 236u32);
	let opts: Vec<String> = (1..=20).map(|i| format!("Option {i}")).collect();
	let mut ui = Ui::new(Select::new(opts).max_visible(8));
	let dl = {
		let theme: &dyn Theme = chrome.theme();
		let fonts = chrome.fonts();
		ui.layout_in(wgpu_ui::Rect::new(10.0, 10.0, 200.0, 24.0), theme, fonts);
		ui.dispatch(&[Event::PointerButton {
			button: PointerButton::Primary,
			pressed: true,
			pos: Vec2::new(110.0, 22.0),
			mods: Modifiers::NONE,
		}]);
		// Two notches (three rows each) down the twenty-row list, with the
		// pointer left over a row so its hover wash shows.
		ui.dispatch(&[Event::Scroll {
			delta: ScrollDelta::Lines(Vec2::new(0.0, 2.0)),
			pos: Vec2::new(110.0, 100.0),
			mods: Modifiers::NONE,
		}]);
		let mut dl = DrawList::new();
		ui.draw(&mut dl, theme, fonts);
		dl
	};
	snapshot_list(&device, &queue, &mut chrome, "select_popup_scrollbar", w, h, BACKDROP, &dl);
}

/// The project tab strip over the steel band: three open projects (the second
/// dirty, marked `*`), the first active (raised + amber), each with a close `x`.
#[test]
fn panel_tabs() {
	tab_strip_snapshot(0, "panel_tabs");
}

/// The tab strip with the second (dirty) project active instead of the first.
#[test]
fn panel_tabs_selected() {
	tab_strip_snapshot(1, "panel_tabs_selected");
}

/// The tab strip with a save-editor session open (the middle tab): warning-red
/// ink + a leading `/!\` so a modified game save can't be mistaken for a map.
#[test]
fn panel_tabs_save_file() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (480u32, 22u32);
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let mut strip = crate::tabs::TabStrip::new();
	let tabs = vec![
		("mars_arena.wrl".to_string(), false, false),
		("SAVE7.DAT - SNOW_1".to_string(), true, true),
		("scratch.wrl".to_string(), false, false),
	];
	strip.sync(tabs, 0, true);
	let dl = {
		let mut dl = DrawList::new();
		chrome.theme().header_band(&mut dl, body);
		let strip_dl = render_widget(&chrome, &mut strip, body);
		dl.cmds.extend(strip_dl.cmds);
		dl
	};
	snapshot_list(&device, &queue, &mut chrome, "panel_tabs_save_file", w, h, BACKDROP, &dl);
}

fn tab_strip_snapshot(active: usize, name: &str) {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (480u32, 22u32);
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let mut strip = crate::tabs::TabStrip::new();
	let tabs = vec![
		("mars_arena.wrl".to_string(), false, false),
		("green_valley.wrl".to_string(), true, false),
		("scratch.wrl".to_string(), false, false),
	];
	strip.sync(tabs, active, true);
	let dl = {
		// The steel band is drawn shell-side (like the status bar); the strip
		// draws its tab faces on top.
		let mut dl = DrawList::new();
		chrome.theme().header_band(&mut dl, body);
		let strip_dl = render_widget(&chrome, &mut strip, body);
		dl.cmds.extend(strip_dl.cmds);
		dl
	};
	snapshot_list(&device, &queue, &mut chrome, name, w, h, BACKDROP, &dl);
}

/// The minimap dockable header: the three source keys (over / pass / mini,
/// "overworld" active) over the steel band, with the camera-viewport outline in
/// the map area (the map texture itself is a GPU blit pass, not captured here).
/// A real widget tree since U5.3 — stock `Button`s over a `MinimapView`.
#[test]
fn panel_minimap() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (220u32, 240u32);
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let mut overlay = crate::minimap::MinimapOverlay::new();
	overlay.sync(crate::minimap::Mode::Overworld, (8, 6), Some(Rect::new(30.0, 60.0, 120.0, 90.0)));
	let dl = {
		let mut dl = DrawList::new();
		chrome.theme().header_band(&mut dl, Rect::new(0.0, 0.0, w as f32, crate::minimap::HEADER_H));
		let over_dl = render_widget(&chrome, &mut overlay, body);
		dl.cmds.extend(over_dl.cmds);
		dl
	};
	snapshot_list(&device, &queue, &mut chrome, "panel_minimap", w, h, BACKDROP, &dl);
}

/// The units panel with no library loaded (the acceptable no-retail state): the
/// five team swatches + the eraser toggle in the header, and the "set MaxPath…"
/// note below (loading sprites needs retail M.A.X. data + a GPU atlas). A real
/// widget tree since U5.7, so the swatches are `ColorButton` keys (G8) and the
/// eraser a stock `Button`, over a `UnitsGrid` content widget.
#[test]
fn panel_units() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (240u32, 360u32);
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let mut content = crate::units::UnitsContent::new();
	content.sync(crate::units::Snapshot::of(None, None, None, 0, false));
	let dl = render_widget(&chrome, &mut content, body);
	snapshot_list(&device, &queue, &mut chrome, "panel_units", w, h, BACKDROP, &dl);
}

/// The Scenery panel over a real GREEN project, with a piece armed: the pack
/// filter, the preview-size dropdown and the count in the header band, and the
/// named, ringed rows of the thumbnail grid below it. The thumbnails themselves
/// are a native GPU pass (`scenery_render`), so what this pins is the chrome -
/// the band, the two dropdown boxes, the name strips, the selection ring and
/// the scrollbar.
#[test]
fn panel_scenery() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (240u32, 360u32);
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let editor = green_editor();
	let mut content = crate::scenery::SceneryContent::new();
	// Arm the second piece the grid *lists*, not the second one the library
	// holds: the grid sorts by name number-aware, so those are different pieces
	// and only the listed one is on screen for the ring to be pinned on.
	let armed = crate::scenery::visible_pieces(&editor.project, None)[1];
	content.sync(crate::scenery::Snapshot::of(
		&editor.project,
		Some(armed),
		crate::scenery::DEFAULT_PREVIEW,
		None,
		false,
	));
	let dl = render_widget(&chrome, &mut content, body);
	snapshot_list(&device, &queue, &mut chrome, "panel_scenery", w, h, BACKDROP, &dl);
}

/// The same panel with no libraries loaded (a project on a pack that ships no
/// scenery): the empty note replaces the grid, and the header stays live.
#[test]
fn panel_scenery_empty() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (240u32, 160u32);
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let mut content = crate::scenery::SceneryContent::new();
	// Every *shipped* terrain pack has a cut-out library, and a project with no
	// palette-owning pack does not load at all - so the honest way to reach this
	// state is a real project whose libraries came back empty. It must still be
	// synced, not left at the widget's default, or the golden would pin two
	// dropdowns that were never given their options.
	let mut empty = green_editor().project;
	empty.scenery_packs.clear();
	content.sync(crate::scenery::Snapshot::of(&empty, None, crate::scenery::DEFAULT_PREVIEW, None, false));
	let dl = render_widget(&chrome, &mut content, body);
	snapshot_list(&device, &queue, &mut chrome, "panel_scenery_empty", w, h, BACKDROP, &dl);
}

/// The Unit Properties inspector over a selected connector-host building
/// (POWERSTN, team green): the type/id readout, team swatches (green ringed),
/// the name/hits/ammo/storage field wells, the facing + orders steppers, and the
/// appended connector grid (S4.4). No sprite library here, so the footprint
/// falls back to 1 → a 3×3 grid (the 2×2 layout is covered by unit tests); the
/// mask `NL|ET|SL` lights three of the four edge checkboxes. Exercises every
/// v1 + v2 control in one frame; the centre sprite is a separate GPU pass.
#[test]
fn panel_unitprops() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (280u32, 380u32);
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let mut editor = green_editor();
	editor.project.place_object(map_core::MapObject {
		unit_type: max_assets::save::unit_type_id("POWERSTN").unwrap(),
		x: 3,
		y: 3,
		team: 1,
		props: map_core::ObjectProps {
			name: "Grid Alpha".into(),
			hits: 50,
			ammo: 0,
			storage: 4,
			connectors: 0x15, // NL | ET | SL (coherent for a 1×1 grid)
			..Default::default()
		},
	});
	editor.selected_object = Some(editor.project.objects.len() - 1);
	let mut content = crate::unitprops::UnitPropsContent::new();
	content.sync(crate::unitprops::Snapshot::of(&editor));
	let dl = render_widget(&chrome, &mut content, body);
	snapshot_list(&device, &queue, &mut chrome, "panel_unitprops", w, h, BACKDROP, &dl);
}

/// Item 8 end to end, through the real panel `Ui`: clicking a value box focuses
/// it (the panel then wants keyboard), and typing + Enter fires a `Commit` for
/// that field — the signal the shell turns into an `object-edit` command.
#[test]
fn unitprops_inline_edit_focuses_and_commits() {
	use crate::panel_ui::PanelHost;
	use crate::unitprops::{Commit, Field, Snapshot, UnitPropsContent};
	use wgpu_ui::{Event, Key, Modifiers, PointerButton, Vec2};

	let (_device, _queue, chrome) = chrome_fixture();
	let body = Rect::new(0.0, 0.0, 280.0, 640.0);
	let mut editor = green_editor();
	editor.project.place_object(map_core::MapObject {
		unit_type: max_assets::save::unit_type_id("TANK").unwrap(),
		x: 3,
		y: 3,
		team: 1,
		props: map_core::ObjectProps { hits: 50, ..Default::default() },
	});
	editor.selected_object = Some(editor.project.objects.len() - 1);

	let mut host = PanelHost::new(UnitPropsContent::new());
	host.root_mut().unwrap().sync(Snapshot::of(&editor));
	// First build lays the boxes out (no events).
	host.build(&chrome, body, 1.0, &[], &mut DrawList::new(), &mut DrawList::new());
	// The Hits box is the 2nd visible field (Name, Hits, Ammo, Storage).
	let hits = host.root_mut().unwrap().field_rects_for_test()[1];
	let (cx, cy) = (hits.x + hits.w / 2.0, hits.y + hits.h / 2.0);
	let press = |p: bool| Event::PointerButton {
		button: PointerButton::Primary,
		pressed: p,
		pos: Vec2::new(cx, cy),
		mods: Modifiers::NONE,
	};
	// A click on the box focuses it → the panel now wants keyboard input.
	host.build(&chrome, body, 1.0, &[press(true), press(false)], &mut DrawList::new(), &mut DrawList::new());
	assert!(host.panel.ui.wants_text_input(), "clicking a value box focuses it");
	// Type a digit, then Enter commits the field.
	host.build(&chrome, body, 1.0, &[Event::Text("7".into())], &mut DrawList::new(), &mut DrawList::new());
	let enter = Event::Key { key: Key::Enter, pressed: true, repeat: false, mods: Modifiers::NONE };
	host.build(&chrome, body, 1.0, &[enter], &mut DrawList::new(), &mut DrawList::new());
	let commit = host.root_mut().unwrap().take_commit();
	assert!(
		matches!(commit, Some(Commit::Field(Field::Hits, _))),
		"Enter commits the focused Hits field, got {commit:?}",
	);
	assert_eq!(host.root_mut().unwrap().take_commit(), None, "and only once");
}

/// U4.3: the three ways focus leaves a box that the old app-side `pending_commit`
/// could not see — Tab, and the shell blurring the whole panel because a press
/// landed somewhere this `Ui` never hears about (the map, another panel). Escape
/// is the one that must *not* commit.
#[test]
fn unitprops_commits_on_tab_and_on_a_moved_blur_but_not_on_escape() {
	use crate::panel_ui::PanelHost;
	use crate::unitprops::{Commit, Field, Snapshot, UnitPropsContent};
	use wgpu_ui::{BlurCause, Event, Key, Modifiers, PointerButton, Vec2};

	let (_device, _queue, chrome) = chrome_fixture();
	let body = Rect::new(0.0, 0.0, 280.0, 640.0);
	let mut editor = green_editor();
	editor.project.place_object(map_core::MapObject {
		unit_type: max_assets::save::unit_type_id("TANK").unwrap(),
		x: 3,
		y: 3,
		team: 1,
		props: map_core::ObjectProps { hits: 50, ..Default::default() },
	});
	editor.selected_object = Some(editor.project.objects.len() - 1);

	let mut host = PanelHost::new(UnitPropsContent::new());
	host.root_mut().unwrap().sync(Snapshot::of(&editor));
	host.build(&chrome, body, 1.0, &[], &mut DrawList::new(), &mut DrawList::new());

	// Focus the Hits box (the 2nd visible field) with a click, and type into it.
	let hits = host.root_mut().unwrap().field_rects_for_test()[1];
	let (cx, cy) = (hits.x + hits.w / 2.0, hits.y + hits.h / 2.0);
	// Focusing Hits *from another box* commits that box (it lost focus, exactly
	// as designed), so each case starts from a drained queue.
	let click = |host: &mut PanelHost<UnitPropsContent>, chrome: &MenuChrome| {
		let ev = |p: bool| Event::PointerButton {
			button: PointerButton::Primary,
			pressed: p,
			pos: Vec2::new(cx, cy),
			mods: Modifiers::NONE,
		};
		host.build(chrome, body, 1.0, &[ev(true), ev(false)], &mut DrawList::new(), &mut DrawList::new());
		while host.root_mut().unwrap().take_commit().is_some() {}
	};
	let typed = |host: &mut PanelHost<UnitPropsContent>, chrome: &MenuChrome| {
		host.build(chrome, body, 1.0, &[Event::Text("7".into())], &mut DrawList::new(), &mut DrawList::new());
	};

	// 1. Tab out of the box. The `Ui` consumes Tab before any widget sees the
	//    event, so nothing app-side could ever have caught this one.
	click(&mut host, &chrome);
	typed(&mut host, &chrome);
	let tab = Event::Key { key: Key::Tab, pressed: true, repeat: false, mods: Modifiers::NONE };
	host.build(&chrome, body, 1.0, &[tab], &mut DrawList::new(), &mut DrawList::new());
	assert!(
		matches!(host.root_mut().unwrap().take_commit(), Some(Commit::Field(Field::Hits, _))),
		"Tab commits the box it leaves",
	);

	// 2. The shell hands the keyboard to another layer (a click on the map or in
	//    another panel): `PanelUi::blur(Moved)`, and the edit stands.
	click(&mut host, &chrome);
	typed(&mut host, &chrome);
	host.panel.blur(BlurCause::Moved);
	assert!(
		matches!(host.root_mut().unwrap().take_commit(), Some(Commit::Field(Field::Hits, _))),
		"a press this panel never sees still commits, via the blur",
	);

	// 3. Escape: the shell blurs with `Cancelled` and the edit is abandoned.
	click(&mut host, &chrome);
	typed(&mut host, &chrome);
	host.panel.blur(BlurCause::Cancelled);
	assert_eq!(host.root_mut().unwrap().take_commit(), None, "Escape commits nothing");

	// 4. A press on the panel's own inert chrome does **not** commit: it moves
	//    no focus, so the caret stays in the box and the edit is still live.
	//    This is the one case that changed with U4.3 — the old rect test
	//    committed on any press outside the box — and it is the change that
	//    makes a panel field behave like a dialog field, where clicking the
	//    dialog background has never committed anything.
	click(&mut host, &chrome);
	typed(&mut host, &chrome);
	let away = |p: bool| Event::PointerButton {
		button: PointerButton::Primary,
		pressed: p,
		pos: Vec2::new(cx, hits.y + 200.0),
		mods: Modifiers::NONE,
	};
	host.build(&chrome, body, 1.0, &[away(true), away(false)], &mut DrawList::new(), &mut DrawList::new());
	assert_eq!(host.root_mut().unwrap().take_commit(), None, "inert chrome takes no focus, so nothing commits");
	assert!(host.panel.ui.wants_text_input(), "the caret is still in the box");
	// …and the edit is not lost: it lands when focus actually leaves.
	host.panel.blur(BlurCause::Moved);
	assert!(
		matches!(host.root_mut().unwrap().take_commit(), Some(Commit::Field(Field::Hits, _))),
		"the pending edit commits when focus does leave",
	);
	assert_eq!(host.root_mut().unwrap().take_commit(), None, "exactly once");
}

/// U3.4 through the real panel `Ui`: the facing box is a hosted `Select`, so a
/// click opens its list *into the panel's popup layer* (not its base list),
/// picking a row hands the shell an `object-edit`-shaped `SelectPick`, and the
/// panel becomes press-modal while it is open.
#[test]
fn unitprops_dropdown_opens_into_the_popup_layer_and_picks() {
	use crate::panel_ui::PanelHost;
	use crate::unitprops::{Action, SelectKind, Snapshot, UnitPropsContent};
	use wgpu_ui::{Event, Modifiers, PointerButton, Vec2};

	let (_device, _queue, chrome) = chrome_fixture();
	let body = Rect::new(0.0, 0.0, 280.0, 640.0);
	let mut editor = green_editor();
	editor.project.place_object(map_core::MapObject {
		unit_type: max_assets::save::unit_type_id("TANK").unwrap(),
		x: 3,
		y: 3,
		team: 1,
		props: map_core::ObjectProps { hits: 50, ..Default::default() },
	});
	editor.selected_object = Some(editor.project.objects.len() - 1);

	let mut host = PanelHost::new(UnitPropsContent::new());
	host.root_mut().unwrap().sync(Snapshot::of(&editor));
	host.panel.set_viewport(Rect::new(0.0, 0.0, 1280.0, 800.0));
	host.build(&chrome, body, 1.0, &[], &mut DrawList::new(), &mut DrawList::new());

	// The box's rect comes off the tree that arranged it — there is no shell-side
	// layout model left to derive it from (U5.8).
	let facing = host.root_mut().unwrap().select_rect_for_test(SelectKind::Facing);
	let press = |x: f32, y: f32| Event::PointerButton {
		button: PointerButton::Primary,
		pressed: true,
		pos: Vec2::new(x, y),
		mods: Modifiers::NONE,
	};

	// Open: the list lands in the popup list, and nothing of it in the base one.
	let (mut base, mut popups) = (DrawList::new(), DrawList::new());
	host.build(&chrome, body, 1.0, &[press(facing.center().x, facing.center().y)], &mut base, &mut popups);
	assert!(host.panel.popup_open(), "the facing box opens its list");
	assert!(!popups.is_empty(), "the open list draws into the panel's popup layer");

	// Pick row 3: the shell gets the value, as an `object-edit`-shaped action.
	let row = wgpu_ui::SelectSize::SMALL_ROW_H;
	let popup = host.root_mut().unwrap().select_popup_for_test(SelectKind::Facing);
	host.build(
		&chrome,
		body,
		1.0,
		&[press(popup.x + 2.0, popup.y + 3.0 * row + row / 2.0)],
		&mut DrawList::new(),
		&mut DrawList::new(),
	);
	assert!(!host.panel.popup_open(), "picking closes the list");
	// The pick comes back as an action tag on the dispatch that made it — a
	// `Select` commits on the **press**, so this is the poll the shell's
	// `Press::Body` arm makes (U5.8; the U5.6 bug U5.7 found).
	let picked: Vec<Action> = host.panel.ui.actions().iter().copied().filter_map(crate::unitprops::action_of).collect();
	assert_eq!(picked, vec![Action::SelectPick(SelectKind::Facing, 3)]);
	// …and it lives for exactly that dispatch: the next one clears it.
	host.build(&chrome, body, 1.0, &[press(0.0, 0.0)], &mut DrawList::new(), &mut DrawList::new());
	assert!(host.panel.ui.actions().is_empty(), "a pick is reported once");
}

/// The dispatch half of press-modality (`over_at`'s half is asserted in
/// `main.rs`): an open dropdown is a pointer *grab* — the panel's dispatch
/// reports `Response::capturing`, which is what the shell's capture branch keys
/// on to route the next press here wherever it lands. An outside press then
/// dismisses without picking and releases the grab in the same dispatch, so it
/// can never fall through to the map.
#[test]
fn an_open_panel_dropdown_grabs_the_pointer_until_dismissed() {
	use crate::panel_ui::PanelHost;
	use crate::unitprops::{SelectKind, Snapshot, UnitPropsContent};
	use wgpu_ui::{Event, Modifiers, PointerButton, Vec2};

	let (_device, _queue, chrome) = chrome_fixture();
	let body = Rect::new(0.0, 0.0, 280.0, 640.0);
	let mut editor = green_editor();
	editor.project.place_object(map_core::MapObject {
		unit_type: max_assets::save::unit_type_id("TANK").unwrap(),
		x: 3,
		y: 3,
		team: 1,
		props: map_core::ObjectProps { hits: 50, ..Default::default() },
	});
	editor.selected_object = Some(editor.project.objects.len() - 1);

	let mut host = PanelHost::new(UnitPropsContent::new());
	host.root_mut().unwrap().sync(Snapshot::of(&editor));
	host.panel.set_viewport(Rect::new(0.0, 0.0, 1280.0, 800.0));
	host.build(&chrome, body, 1.0, &[], &mut DrawList::new(), &mut DrawList::new());

	let facing = host.root_mut().unwrap().select_rect_for_test(SelectKind::Facing);
	let press = |x: f32, y: f32| Event::PointerButton {
		button: PointerButton::Primary,
		pressed: true,
		pos: Vec2::new(x, y),
		mods: Modifiers::NONE,
	};

	let r = host.panel.dispatch_events(&[press(facing.center().x, facing.center().y)]);
	assert!(host.panel.popup_open(), "the facing box opens its list");
	assert!(r.capturing, "an open dropdown is a pointer grab the router will hold");

	// A press far outside the panel body — the map, in shell terms. The grab
	// routed it here; the owner dismisses without picking and lets go.
	let r = host.panel.dispatch_events(&[press(1000.0, 700.0)]);
	assert!(!host.panel.popup_open(), "an outside press dismisses the list");
	assert!(!r.capturing, "...and releases the grab in the same dispatch");
	assert!(
		host.panel.ui.actions().iter().copied().filter_map(crate::unitprops::action_of).next().is_none(),
		"nothing was picked"
	);
}

/// U1.2's headline regression: **press → move → release inside a Unit Properties
/// value box drags a selection.**
///
/// It could not, before: every panel `build` call in `render_frame` passed `&[]`,
/// so a panel `Ui` never saw a `PointerMoved` at all — and `TextInput` extends a
/// selection *only* on a move while capturing (its release arm deliberately
/// leaves the caret alone). Drag-select in a docked panel was dead by
/// construction, no matter what the widget did. The router now routes moves to
/// the panel under the cursor, so the caret follows the drag.
#[test]
fn unitprops_drag_selects_inside_a_value_box() {
	use crate::panel_ui::PanelHost;
	use crate::unitprops::{Snapshot, UnitPropsContent};
	use wgpu_ui::{Event, Modifiers, PointerButton, Vec2};

	let (_device, _queue, chrome) = chrome_fixture();
	let body = Rect::new(0.0, 0.0, 280.0, 640.0);
	let mut editor = green_editor();
	editor.project.place_object(map_core::MapObject {
		unit_type: max_assets::save::unit_type_id("TANK").unwrap(),
		x: 3,
		y: 3,
		team: 1,
		// A wide value, so a drag across the box crosses several characters.
		props: map_core::ObjectProps { name: "Alpha Company".into(), hits: 50, ..Default::default() },
	});
	editor.selected_object = Some(editor.project.objects.len() - 1);

	let mut host = PanelHost::new(UnitPropsContent::new());
	host.root_mut().unwrap().sync(Snapshot::of(&editor));
	host.build(&chrome, body, 1.0, &[], &mut DrawList::new(), &mut DrawList::new());
	// The Name box is the first visible field.
	let name = host.root_mut().unwrap().field_rects_for_test()[0];
	let y = name.y + name.h / 2.0;
	let at = |x: f32| Vec2::new(x, y);

	let button = |pressed: bool, x: f32| Event::PointerButton {
		button: PointerButton::Primary,
		pressed,
		pos: at(x),
		mods: Modifiers::NONE,
	};
	let (right, left) = (name.x + name.w - 2.0, name.x + 1.0);

	// Press at the right end of the text: the caret lands past the first glyphs.
	host.build(&chrome, body, 1.0, &[button(true, right)], &mut DrawList::new(), &mut DrawList::new());
	let anchored = host.root_mut().unwrap().field_caret_for_test(0);
	assert!(anchored > 0, "the press put the caret at the click, not at 0 (got {anchored})");

	// Release at the far left *without* a move in between — the shape of every
	// panel click before U1.2. The caret does not budge: `TextInput`'s release
	// arm only ends the capture, so this alone can never select anything.
	host.build(&chrome, body, 1.0, &[button(false, left)], &mut DrawList::new(), &mut DrawList::new());
	assert_eq!(host.root_mut().unwrap().field_caret_for_test(0), anchored, "a moveless press+release selects nothing");

	// Now the same gesture *with* the move the router feeds: the caret walks
	// left with the pointer, extending the selection across what it crossed.
	host.build(&chrome, body, 1.0, &[button(true, right)], &mut DrawList::new(), &mut DrawList::new());
	let moved = Event::PointerMoved { pos: at(left) };
	host.build(&chrome, body, 1.0, &[moved, button(false, left)], &mut DrawList::new(), &mut DrawList::new());
	let dragged = host.root_mut().unwrap().field_caret_for_test(0);
	assert!(dragged < anchored, "the drag walked the caret back from {anchored} (got {dragged})");
}

/// U2.3: the Unit Properties panel scrolls **itself**, and the fields still keep
/// first refusal. Before U2 the shell intercepted every press in the bar column
/// before the panel saw it, and kept the offset on `EditorState`; now the panel's
/// own `Scroller` owns the wheel, the bar and the paging keys — and a text drag
/// that starts hard against the gutter is still the box's.
#[test]
fn unitprops_scrolls_itself_and_fields_keep_first_refusal() {
	use crate::panel_ui::{PanelHost, PanelInput};
	use crate::unitprops::{Snapshot, UnitPropsContent};
	use wgpu_ui::{Event, Key, Modifiers, PointerButton, ScrollDelta, Vec2};

	let (_device, _queue, chrome) = chrome_fixture();
	// Short enough that the inspector outgrows the dock and shows a bar.
	let body = Rect::new(0.0, 0.0, 280.0, 200.0);
	let mut editor = green_editor();
	editor.project.place_object(map_core::MapObject {
		unit_type: max_assets::save::unit_type_id("TANK").unwrap(),
		x: 3,
		y: 3,
		team: 1,
		props: map_core::ObjectProps { name: "Alpha Company".into(), hits: 50, ..Default::default() },
	});
	editor.selected_object = Some(editor.project.objects.len() - 1);

	let mut host = PanelHost::new(UnitPropsContent::new());
	host.root_mut().unwrap().sync(Snapshot::of(&editor));
	host.build(&chrome, body, 1.0, &[], &mut DrawList::new(), &mut DrawList::new());
	let offset = |h: &mut PanelHost<UnitPropsContent>| h.root_mut().unwrap().scroll();

	// The wheel over the body scrolls the panel (and is consumed, so it never
	// reaches the map behind it).
	let wheel = Event::Scroll {
		delta: ScrollDelta::Lines(Vec2::new(0.0, 1.0)),
		pos: Vec2::new(140.0, 60.0),
		mods: Modifiers::NONE,
	};
	assert!(host.dispatch(&[wheel]).wants_pointer(), "the panel takes the wheel");
	assert_eq!(offset(&mut host), 48.0, "one wheel notch");

	// Paging works with nothing focused — the hover-targeted accelerator rule.
	let key = |k| Event::Key { key: k, pressed: true, repeat: false, mods: Modifiers::NONE };
	assert!(host.dispatch(&[key(Key::End)]).wants_keyboard(), "End is consumed");
	assert!(offset(&mut host) > 48.0, "End reaches the bottom");
	host.dispatch(&[key(Key::Home)]);
	assert_eq!(offset(&mut host), 0.0, "Home returns to the top");

	// A press in the bar column pages: the panel's own click oracle stops at the
	// gutter (its PAD equals the bar width), so nothing else claims it.
	let bar = Event::PointerButton {
		button: PointerButton::Primary,
		pressed: true,
		pos: Vec2::new(body.x + body.w - 4.0, body.y + body.h - 4.0),
		mods: Modifiers::NONE,
	};
	assert!(host.dispatch(&[bar]).wants_pointer(), "the bar takes the press");
	assert!(offset(&mut host) > 0.0, "a track click below the thumb pages down");
	host.dispatch(&[key(Key::Home)]);

	// Back at the top: a press at the Name box's right edge is still a text drag,
	// not a scroll — the fields ran first.
	host.build(&chrome, body, 1.0, &[], &mut DrawList::new(), &mut DrawList::new());
	let name = host.root_mut().unwrap().field_rects_for_test()[0];
	let y = name.y + name.h / 2.0;
	let press = Event::PointerButton {
		button: PointerButton::Primary,
		pressed: true,
		pos: Vec2::new(name.x + name.w - 1.0, y),
		mods: Modifiers::NONE,
	};
	assert!(host.dispatch(&[press]).capturing, "the field, not the bar, took the press");
	assert_eq!(offset(&mut host), 0.0, "and nothing scrolled");
}

/// U1.3: **a drag that leaves the widget — and the whole panel — keeps going.**
///
/// `TextInput` takes `ctx.capture(id)` on press, and a captured `Ui` routes every
/// later pointer event to the holder regardless of where it lands. The shell used
/// to drop that signal, sending moves to whatever was under the cursor instead,
/// so a drag died at the widget's edge; the router now feeds the capturing layer
/// until it lets go. Here the pointer runs off the *left of the window* and the
/// selection still tracks it, then the release ends the capture.
#[test]
fn unitprops_drag_survives_leaving_the_panel() {
	use crate::panel_ui::{PanelHost, PanelInput};
	use crate::unitprops::{Snapshot, UnitPropsContent};
	use wgpu_ui::{Event, Modifiers, PointerButton, Vec2};

	let (_device, _queue, chrome) = chrome_fixture();
	let body = Rect::new(40.0, 0.0, 240.0, 640.0);
	let mut editor = green_editor();
	editor.project.place_object(map_core::MapObject {
		unit_type: max_assets::save::unit_type_id("TANK").unwrap(),
		x: 3,
		y: 3,
		team: 1,
		props: map_core::ObjectProps { name: "Alpha Company".into(), hits: 50, ..Default::default() },
	});
	editor.selected_object = Some(editor.project.objects.len() - 1);

	let mut host = PanelHost::new(UnitPropsContent::new());
	host.root_mut().unwrap().sync(Snapshot::of(&editor));
	host.build(&chrome, body, 1.0, &[], &mut DrawList::new(), &mut DrawList::new());
	let name = host.root_mut().unwrap().field_rects_for_test()[0];
	let y = name.y + name.h / 2.0;

	// Press at the right end of the Name box: the field captures the pointer.
	let press = Event::PointerButton {
		button: PointerButton::Primary,
		pressed: true,
		pos: Vec2::new(name.x + name.w - 2.0, y),
		mods: Modifiers::NONE,
	};
	assert!(host.dispatch(&[press]).capturing, "pressing a text field captures the pointer for the drag");
	let anchored = host.root_mut().unwrap().field_caret_for_test(0);

	// Drag well past the panel's own left edge — off the UI entirely. The router
	// keeps feeding this layer, and the captured field keeps extending.
	let outside = Vec2::new(-50.0, y + 400.0);
	assert!(host.dispatch(&[Event::PointerMoved { pos: outside }]).capturing, "the drag is still live off-panel");
	let dragged = host.root_mut().unwrap().field_caret_for_test(0);
	assert!(dragged < anchored, "the out-of-panel move still extended the selection ({anchored} -> {dragged})");

	// The real release ends it, wherever it lands.
	let release =
		Event::PointerButton { button: PointerButton::Primary, pressed: false, pos: outside, mods: Modifiers::NONE };
	assert!(!host.dispatch(&[release]).capturing, "the release hands the pointer back");
}

/// The Tile Explorer populated with the GREEN pack's tiles: the flowed header —
/// the tileset / filter / size dropdowns, the new/clone/edit/delete keys and the
/// count — over the tile grid, with the first tile selected (its ring). Converted
/// to a real widget tree in U5.6, so every key here is a stock `Button::sized`
/// and each dropdown a `Select` measured to its own widest option; the tiles
/// themselves are a native GPU pass, so what this captures is the header band
/// plus the grid's rings and scrollbar.
#[test]
fn panel_tilepicker() {
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (280u32, 460u32);
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let res = resources();
	let project = Project::new(8, 6, &["GREEN".to_string()], &res.join("assets/tilepacks"), 42).unwrap();
	let state = crate::picker::PickerState::default();
	let list = crate::picker::items(&project, crate::picker::Filter::All, None);
	let active = list.first().map(|it| it.id.to_string());
	// The tile stills render from the index atlas via a GPU pass (draw_picker);
	// this DrawList snapshot captures the panel chrome (header, dropdowns, keys,
	// count, selection ring) over the grid.
	let snap = crate::picker::Snapshot::of(&project, &state, active.as_deref());
	let mut content = crate::picker::PickerContent::new();
	content.sync(snap);
	let dl = render_widget(&chrome, &mut content, body);
	snapshot_list(&device, &queue, &mut chrome, "panel_tilepicker", w, h, BACKDROP, &dl);
}

/// The match-editor's grouped tile list (a `RowList`): a group header row and
/// its member tiles with thumbnails (uv'd from the steel atlas — the live
/// rest-palette tile atlas is a GPU pass), a rule/warn toned pair, one selected.
#[test]
fn panel_matchview() {
	use crate::matcheditor::RowTone;
	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (200u32, 160u32);
	let body = Rect::new(0.0, 0.0, w as f32, h as f32);
	let row = |label: &str, tag: &str, header: bool, tone: RowTone, selected: bool| crate::matchview::ListRow {
		label: label.to_string(),
		tag: tag.to_string(),
		thumb: (!header).then_some(TexRect::FULL),
		header,
		collapsed: false,
		tone,
		selected,
	};
	let rows = vec![
		row("[ground]", "", true, RowTone::Plain, false),
		row("grass_a", "", false, RowTone::Plain, false),
		row("grass_b", "NES", false, RowTone::Rule, false),
		row("shore_nw", "W", false, RowTone::Warn, true),
		row("cliff_e", "", false, RowTone::Select, false),
	];
	let mut list = crate::matchview::RowList::new(TextureId::ATLAS, w as f32);
	list.set_rows(rows);
	let dl = render_widget(&chrome, &mut list, body);
	snapshot_list(&device, &queue, &mut chrome, "panel_matchview", w, h, BACKDROP, &dl);
}

/// The console (U4.5): one widget — plate, border, scrollback in each ink class,
/// the `] ` prompt and the hosted monospace field with the caret in it. This is
/// the golden that records what replaced the baked bitmap atlas; open
/// `panel_console.diff.png` and look at the glyphs before re-baselining it.
#[test]
fn panel_console() {
	use crate::console_view::{ConsoleView, console_rect};
	use crate::panel_ui::PanelHost;

	let (device, queue, mut chrome) = chrome_fixture();
	let (w, h) = (640u32, 360u32);
	let body = console_rect(w as f32, h as f32);

	let mut host = PanelHost::new(ConsoleView::new());
	{
		let c = host.root_mut().expect("typed root");
		c.sync(vec![
			"M.A.X. Map Editor console - Enter runs, Up/Down history".to_string(),
			"] fit".to_string(),
			"error: unknown verb `fti`".to_string(),
			"auto-shore: 24 cells".to_string(),
		]);
		c.set_input("shore fix");
	}
	// A focused field draws its caret — the state the console is always in.
	host.panel.ui.focus_first();

	let mut dl = DrawList::new();
	host.build(&chrome, body, 1.0, &[], &mut dl, &mut DrawList::new());
	snapshot_list(&device, &queue, &mut chrome, "panel_console", w, h, BACKDROP, &dl);
}

/// The console's input line is an ordinary field now: a click inside it places
/// the caret (it had no pointer path at all before U4.5 — no rect, no hit), and
/// Enter reports the submitted line and clears the field.
#[test]
fn console_click_places_the_caret_and_enter_submits() {
	use crate::console_view::{ConsoleView, console_rect};
	use crate::panel_ui::PanelHost;
	use wgpu_ui::{Event, Key, Modifiers, PointerButton, Vec2};

	let (_device, _queue, chrome) = chrome_fixture();
	let (w, h) = (640.0, 360.0);
	let body = console_rect(w, h);

	let mut host = PanelHost::new(ConsoleView::new());
	host.root_mut().unwrap().set_input("shore fix");
	let build = |host: &mut PanelHost<ConsoleView>, events: &[Event]| {
		host.build(&chrome, body, 1.0, events, &mut DrawList::new(), &mut DrawList::new());
	};
	build(&mut host, &[]);

	// A press a few characters into the field puts the caret there, not at the
	// end where `set_input` left it.
	let field = host.root_mut().unwrap().input_rect();
	let (px, py) = (field.x + 3.0, field.y + field.h / 2.0);
	let press = |p: bool| Event::PointerButton {
		button: PointerButton::Primary,
		pressed: p,
		pos: Vec2::new(px, py),
		mods: Modifiers::NONE,
	};
	build(&mut host, &[Event::PointerMoved { pos: Vec2::new(px, py) }, press(true), press(false)]);
	assert!(host.panel.ui.wants_text_input(), "the click focused the field");
	assert!(host.root_mut().unwrap().take_submit().is_none(), "a click submits nothing");

	// Typing lands in the field; Enter submits it and clears the line.
	build(&mut host, &[Event::Text("!".into())]);
	assert_ne!(host.root_mut().unwrap().input_text(), "shore fix", "the caret was inside the text");
	let line = host.root_mut().unwrap().input_text().to_string();
	build(&mut host, &[Event::Key { key: Key::Enter, pressed: true, repeat: false, mods: Modifiers::NONE }]);
	assert_eq!(host.root_mut().unwrap().take_submit().as_deref(), Some(line.as_str()), "Enter submits the line");
	assert_eq!(host.root_mut().unwrap().input_text(), "", "and clears the prompt");
	assert_eq!(host.root_mut().unwrap().take_submit(), None, "reported once");
}

/// Why the shell has to dispatch the pointer **release** to the console layer:
/// a click in the input line starts a drag-select, and a drag-select captures
/// the pointer. `Ui` drops that capture on the matching release and on window
/// focus loss, and on nothing else — so a release the console never sees strands
/// the grab, the router keeps feeding it every later pointer event, and the rest
/// of the UI goes dead (closing the console does not help: a closed console is
/// not rebuilt, so nothing clears it either).
#[test]
fn console_click_captures_the_pointer_until_the_release() {
	use crate::console_view::{ConsoleView, console_rect};
	use crate::panel_ui::PanelHost;
	use wgpu_ui::{Event, Modifiers, PointerButton, Vec2};

	let (_device, _queue, chrome) = chrome_fixture();
	let (w, h) = (640.0, 360.0);
	let body = console_rect(w, h);

	let mut host = PanelHost::new(ConsoleView::new());
	host.build(&chrome, body, 1.0, &[], &mut DrawList::new(), &mut DrawList::new());

	let field = host.root_mut().unwrap().input_rect();
	let (px, py) = (field.x + field.w / 2.0, field.y + field.h / 2.0);
	let press = |p: bool| Event::PointerButton {
		button: PointerButton::Primary,
		pressed: p,
		pos: Vec2::new(px, py),
		mods: Modifiers::NONE,
	};

	let down = host.panel.dispatch_events(&[Event::PointerMoved { pos: Vec2::new(px, py) }, press(true)]);
	assert!(down.capturing, "a press in the input line grabs the pointer for the drag-select");

	let up = host.panel.dispatch_events(&[press(false)]);
	assert!(!up.capturing, "and only the release hands it back - the shell owes the console this dispatch");
}

/// The wheel over the console band scrolls the scrollback (the model's own
/// line-based paging), and only over the band — below it the map is still live,
/// which the shell's old blanket "console is open" branch was not.
#[test]
fn console_wheel_scrolls_only_over_its_band() {
	use crate::console_view::{ConsoleView, console_rect};
	use crate::panel_ui::PanelHost;
	use wgpu_ui::{Event, Modifiers, ScrollDelta, Vec2};

	let (_device, _queue, chrome) = chrome_fixture();
	let (w, h) = (640.0, 360.0);
	let body = console_rect(w, h);
	let mut host = PanelHost::new(ConsoleView::new());
	let wheel = |x: f32, y: f32, dy: f32| Event::Scroll {
		delta: ScrollDelta::Lines(Vec2::new(0.0, dy)),
		pos: Vec2::new(x, y),
		mods: Modifiers::NONE,
	};
	host.build(&chrome, body, 1.0, &[], &mut DrawList::new(), &mut DrawList::new());

	// Up (negative y) walks back through the scrollback.
	host.build(&chrome, body, 1.0, &[wheel(100.0, body.h / 2.0, -1.0)], &mut DrawList::new(), &mut DrawList::new());
	assert!(host.root_mut().unwrap().take_scroll() > 0, "wheel up scrolls back in time");
	assert_eq!(host.root_mut().unwrap().take_scroll(), 0, "the request is drained once");

	// Below the band: not the console's.
	host.build(&chrome, body, 1.0, &[wheel(100.0, h - 10.0, -1.0)], &mut DrawList::new(), &mut DrawList::new());
	assert_eq!(host.root_mut().unwrap().take_scroll(), 0, "a wheel over the map is not the console's");
}
