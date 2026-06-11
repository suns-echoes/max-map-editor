//! M.A.X. Map Editor — Rust + WGPU.
//!
//! All mutation flows through `Command`s (see `command.rs`) executed by
//! `EditorState::execute` — interactive input, `--script` files, and the
//! future console all share that one path.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autofix;
mod blit;
mod capture;
mod command;
mod confirm;
mod console;
mod console_font;
mod crt;
mod errormodal;
mod font;
mod generator;
mod gpu;
mod grid;
mod input;
mod max_font;
mod menu;
mod minimap;
mod modal;
mod newfromimage;
mod newmap;
mod palette;
mod palette_panel;
mod picker;
mod project_render;
mod render;
mod resize;
mod skin;
mod state;
mod tabs;
mod templates_panel;
mod text;
mod theme;
mod toolbox;
mod ui;
mod units;
mod units_render;
mod workspace;

use std::path::PathBuf;
use std::sync::Arc;

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
use crate::modal::{ModalAction, ModalKey};
use crate::project_render::ProjectRenderer;
use crate::state::{EditorState, Outcome};
use crate::text::TextPass;

/// The shared draw passes (one set per GPU device).
struct Passes {
	text: TextPass,
	blit: BlitPass,
	minimap: MinimapPass,
	previews: newmap::Previews,
	grid: grid::GridPass,
	/// CRT post-process + its offscreen scene target (lazily sized).
	crt: crt::CrtPass,
	scene: Option<crt::SceneTarget>,
	/// Unit-preview pass — built lazily on the first frame after the unit
	/// library loads (needs the sprite data for its atlas).
	units: Option<units_render::UnitsGpu>,
	format: wgpu::TextureFormat,
}

impl Passes {
	fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
		// The UI skin (brushed-steel sheet) is loaded once per device; a missing
		// asset falls back to flat gray inside `skin`.
		let steel = skin::load_steel(&resources_dir());
		Self {
			text: TextPass::new(device, queue, format, &steel),
			blit: BlitPass::new(device, format),
			minimap: MinimapPass::new(),
			previews: newmap::Previews::new(),
			grid: grid::GridPass::new(device, format),
			crt: crt::CrtPass::new(device, format),
			scene: None,
			units: None,
			format,
		}
	}
}

/// The document opened when none is passed — the GREEN starter project,
/// resolved relative to `resources/` (see [`resources_dir`]).
fn default_map() -> PathBuf {
	resources_dir().join("templates/GREEN_1.json")
}

struct Args {
	map: PathBuf,
	script: Vec<Command>,
	headless: bool,
	size: (u32, u32),
	/// `--settings PATH`: load/persist all settings from this file.
	settings: Option<PathBuf>,
}

/// Default settings file: `mme.ini` in the config directory (beside the
/// portable binary, or `config/` at the workspace root during development).
fn default_settings_path() -> Option<PathBuf> {
	input::config_dir().map(|dir| dir.join("mme.ini"))
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
			"-h" | "--help" => die("help"),
			_ if map.is_none() => map = Some(PathBuf::from(arg)),
			_ => die(&format!("unknown argument: {arg}")),
		}
	}

	if let Some(path) = screenshot {
		script.push(Command::Screenshot { path, crop, resize });
		headless = true;
	}

	Args { map: map.unwrap_or_else(default_map), script, headless, size, settings }
}

/// Locate `resources/`, in order: `./resources` (cargo-run from the
/// workspace root — cwd wins so a stray copy under `target/` can't shadow
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

/// Re-upload the active document's cell data after edits.
fn refresh_renderer(renderer: &ProjectRenderer, queue: &wgpu::Queue, editor: &EditorState) {
	renderer.update_cells(queue, &editor.project);
}

/// Build the renderer matching the open document kind.
fn make_renderer(
	device: &wgpu::Device,
	queue: &wgpu::Queue,
	editor: &EditorState,
	format: wgpu::TextureFormat,
) -> ProjectRenderer {
	ProjectRenderer::new(device, queue, &editor.project, format)
}

/// A map cell's rect in screen px for the current view (pan in world px,
/// then zoom) — the same math the unit previews use.
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

/// The selection's thick outline (every selected-region boundary edge,
/// viewport-culled) plus the live rect-drag preview.
fn selection_overlay(editor: &EditorState, w: f32, h: f32) -> ui::UiQuads {
	let mut q = ui::UiQuads::default();
	let zoom = editor.view.zoom;
	let ts = render::TILE_PX as f32;
	if !editor.selection.is_empty() && zoom > 0.0 {
		// The visible cell window — the boundary walk never touches
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
				q.rect(seg, w, h, theme::ACCENT);
			}
		}
	}
	// The rect tool's live drag: a hairline preview of the intended span.
	if let Some((ax, ay, bx, by)) = editor.select_preview {
		let a = map_cell_rect(editor, ax.min(bx), ay.min(by));
		let b = map_cell_rect(editor, ax.max(bx), ay.max(by));
		q.border(ui::Rect::new(a.x, a.y, b.x + b.w - a.x, b.y + b.h - a.y), w, h, theme::ACCENT);
	}
	q
}

/// The armed ghost stamp's tile quads at the cell under the cursor, plus
/// its footprint rect — `None` when nothing is armed, the cursor is off
/// the map, or it hovers UI chrome (panels, menu, tabs).
fn ghost_quads(editor: &EditorState, w: f32, h: f32) -> Option<(ui::Rect, Vec<picker::TileQuad>)> {
	let stamp = editor.stamp.as_ref()?;
	let (cx, cy) = editor.hot.cursor?;
	if cy < menu::BAR_H + tabs::BAR_H || editor.workspace.over_ui(cx, cy, w, h) || editor.context_menu.is_some() {
		return None;
	}
	let (ox, oy) = editor.cell_at(cx, cy)?;
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
	let (w, h) = editor.screen;
	let (wf, hf) = (w as f32, h as f32);
	// Pointer state for hover/pressed widget rendering. Covered surfaces get
	// an inert pointer: panels/tabs don't highlight under an open modal or
	// menu dropdown, and only the topmost modal (the one input routes to —
	// the error modal beats whatever raised it) reacts to the cursor.
	// Headless runs leave `editor.hot` at `Hot::NONE`, so captures are stable.
	let hot = editor.hot;
	let modal_open = editor.active_modal().is_some();
	let covered = modal_open || editor.menu.open.is_some() || editor.context_menu.is_some();
	let shell_hot = if covered { ui::Hot::NONE } else { hot };
	let menu_hot = if modal_open { ui::Hot::NONE } else { hot };
	let modal_hot = if editor.error.is_some() { ui::Hot::NONE } else { hot };
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
	let text_pass = &passes.text;
	// App background: the raw steel sheet stretched across the viewport,
	// drawn first (covering every pixel) so the map's out-of-bounds fragments —
	// which now discard — reveal it instead of a flat void colour.
	let mut bg = ui::UiQuads::default();
	bg.steel(ui::Rect::new(0.0, 0.0, wf, hf), wf, hf, [1.0, 1.0, 1.0, 1.0]);
	text_pass.draw_ui(device, encoder, target, &bg);
	renderer.draw(queue, encoder, target, editor.uniforms(0), editor.show_pass_overlay);
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
			let quads = units::map_quads(&editor.project.units, lib, &ugpu.slots, editor.view.pan, editor.view.zoom);
			ugpu.draw(device, encoder, target, &quads, None, (w, h));
		}
	}
	// Selection chrome rides on the map, beneath the panels: the thick
	// outline around selected regions and a live rect-drag preview.
	let sel_overlay = selection_overlay(editor, wf, hf);
	if !sel_overlay.verts.is_empty() {
		text_pass.draw_ui(device, encoder, target, &sel_overlay);
	}
	// The armed ghost stamp under the cursor: half-transparent tiles snapped
	// to the cell grid, framed so the footprint reads (hidden over UI).
	if let Some((origin, quads)) = ghost_quads(editor, wf, hf) {
		renderer.draw_picker(device, encoder, target, &quads, ui::Rect::new(0.0, 0.0, wf, hf), (w, h), 0.55);
		let mut frame = ui::UiQuads::default();
		frame.border(origin, wf, hf, theme::ACCENT);
		text_pass.draw_ui(device, encoder, target, &frame);
	}
	text_pass.draw_ui(device, encoder, target, &editor.workspace.draw_background(wf, hf));
	// Drop-target peek sits on the map, *below* the windows (a docked panel that
	// stays put must remain readable while another is dragged near its edge).
	text_pass.draw_ui(device, encoder, target, &editor.workspace.draw_peeks(wf, hf));
	for (i, r) in editor.workspace.layout(wf, hf).panels {
		let id = editor.workspace.panels[i].id;
		let has_content = id == "tiles"
			|| id == "minimap"
			|| id == "palette"
			|| id == "toolbox"
			|| id == "units"
			|| id == "templates";
		// A floating panel's chrome AND content share one anchored steel crop;
		// docked panels share the stretched viewport sheet.
		let map = editor.workspace.steel_map(i, r);
		let chrome = editor.workspace.draw_panel(i, r, wf, hf, !has_content, shell_hot);
		text_pass.draw_ui(device, encoder, target, &chrome);
		let body = editor.workspace.body_of(i, r);
		if id == "toolbox" {
			let view = toolbox::view(editor, body, editor.toolbox_scroll, wf, hf, map, shell_hot);
			// Clip the (scrolling) toolbox content to its body so it crops.
			text_pass.draw_ui_clipped(device, encoder, target, &view.chrome, body, (w, h));
			if let Some(quad) = view.preview {
				let r = quad.rect;
				let (x0, y0) = (r.x.max(body.x), r.y.max(body.y));
				let (x1, y1) = ((r.x + r.w).min(body.x + body.w), (r.y + r.h).min(body.y + body.h));
				let clip = ui::Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0));
				if clip.w > 0.0 && clip.h > 0.0 {
					renderer.draw_picker(device, encoder, target, &[quad], clip, (w, h), 1.0);
				}
			}
		} else if id == "tiles" {
			let pr = renderer;
			let view =
				picker::view(&editor.project, &editor.picker, editor.active_tile(), body, wf, hf, map, shell_hot);
			pr.draw_picker(device, encoder, target, &view.tiles, view.scissor, (w, h), 1.0);
			text_pass.draw_ui(device, encoder, target, &view.overlay);
		} else if id == "minimap" {
			passes.minimap.draw(device, queue, encoder, target, &passes.blit, editor, body, (w, h));
			text_pass.draw_ui(device, encoder, target, &minimap::overlay(editor, body, wf, hf, map, shell_hot));
		} else if id == "palette" {
			let base: Vec<u8> = editor.project.palette.clone();
			// While cycling, swatches show the live working palette.
			let display: Vec<u8> = if editor.animate {
				editor.cycler.rgba().chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]]).collect()
			} else {
				base.clone()
			};
			let names = editor.palette_file_names();
			let view = palette_panel::view(
				&display,
				&base,
				editor.active_color.map(u16::from),
				editor.palette_sel_end.map(u16::from),
				editor.palette_scroll,
				editor.animate,
				true,
				editor.palette_show_saved,
				&names,
				body,
				wf,
				hf,
				map,
				shell_hot,
			);
			text_pass.draw_ui_clipped(device, encoder, target, &view.grid, view.scissor, (w, h));
			text_pass.draw_ui(device, encoder, target, &view.chrome);
		} else if id == "units" {
			let view = units::view(
				editor.units.as_ref(),
				passes.units.as_ref().map(|g| &g.slots),
				editor.active_unit,
				editor.unit_team,
				editor.tool == state::Tool::UnitEraser,
				editor.units_scroll,
				body,
				wf,
				hf,
				map,
				shell_hot,
			);
			if let Some(g) = &passes.units {
				g.draw(device, encoder, target, &view.quads, Some(view.scissor), (w, h));
			}
			text_pass.draw_ui(device, encoder, target, &view.overlay);
		} else if id == "templates" {
			let visible = editor.visible_templates();
			let entries: Vec<&state::TemplateEntry> = visible.iter().map(|&g| &editor.templates[g]).collect();
			// The explorer's selection, mapped into the visible list.
			let selected = editor.template_sel.and_then(|g| visible.iter().position(|&v| v == g));
			let view = templates_panel::view(
				&editor.project,
				&entries,
				selected,
				editor.templates_scroll,
				body,
				wf,
				hf,
				map,
				shell_hot,
			);
			text_pass.draw_ui(device, encoder, target, &view.underlay);
			renderer.draw_picker(device, encoder, target, &view.tiles, view.scissor, (w, h), 1.0);
			text_pass.draw_ui(device, encoder, target, &view.overlay);
		}
	}
	// Project tab strip below the menu bar; the menu (with its dropdowns) draws
	// last so it stays topmost.
	let tab_infos = editor.tab_infos();
	let tabs_closable = editor.tabs_closable();
	text_pass.draw_ui(
		device,
		encoder,
		target,
		&tabs::draw(&tab_infos, editor.active_tab(), tabs_closable, menu::BAR_H, wf, hf, shell_hot),
	);
	// Resolve a menu toggle's `key` against live editor state for its checkbox.
	let checked = |key: &str| -> bool {
		match key {
			"grid" => editor.show_grid,
			"pass-overlay" => editor.show_pass_overlay,
			"show-units" => editor.show_units,
			"mode:map" => editor.mode == state::EditorMode::Map,
			"mode:pass" => editor.mode == state::EditorMode::Pass,
			"layer:water" => editor.active_layer == map_core::LAYER_WATER,
			"layer:ground" => editor.active_layer == map_core::LAYER_GROUND,
			"anim:off" => !editor.animate && !editor.ingame,
			"anim:on" => editor.animate && !editor.ingame,
			"anim:ingame" => editor.ingame,
			"crt" => editor.crt,
			_ => key.strip_prefix("win:").is_some_and(|id| editor.workspace.is_visible(id)),
		}
	};
	text_pass.draw_ui(device, encoder, target, &editor.menu.draw(wf, hf, &checked, menu_hot));
	// The right-click context menu floats over panels and the menu bar.
	if let Some(cm) = &editor.context_menu {
		text_pass.draw_ui(device, encoder, target, &cm.draw(wf, hf, &checked));
	}
	if let Some(modal) = &editor.newmap {
		text_pass.draw_ui(device, encoder, target, &modal.view(wf, hf, modal_hot));
		if modal.picking {
			passes.previews.draw(device, queue, encoder, target, &passes.blit, modal, &editor.assets_root, (w, h));
		}
	}
	if let Some(modal) = &editor.resize {
		text_pass.draw_ui(device, encoder, target, &modal.view(wf, hf, modal_hot));
	}
	if let Some(modal) = &editor.autofix {
		text_pass.draw_ui(device, encoder, target, &modal.view(wf, hf, modal_hot));
	}
	if let Some(modal) = &editor.generator {
		text_pass.draw_ui(device, encoder, target, &modal.view(wf, hf, modal_hot));
	}
	if let Some(modal) = &editor.confirm {
		text_pass.draw_ui(device, encoder, target, &modal.view(wf, hf, modal_hot));
	}
	if let Some(modal) = &editor.newfromimage {
		text_pass.draw_ui(device, encoder, target, &modal.view(wf, hf, modal_hot));
	}
	// The error modal draws last so it sits on top of whatever raised it.
	if let Some(modal) = &editor.error {
		text_pass.draw_ui(device, encoder, target, &modal.view(wf, hf, hot));
	}
	if editor.console.is_open() {
		editor.console.set_view_rows(text::rows_for(h));
		let quads = text::console_quads(&editor.console, w, h);
		text_pass.draw(device, encoder, target, &quads);
	}
	// CRT: post-process the offscreen scene onto the real target.
	if crt_on {
		let scene = passes.scene.as_ref().expect("scene built when crt_on");
		passes.crt.draw(encoder, final_target, &scene.bind_group);
	}
}

/// Sink for the `log` facade — the copied decoders in `max-assets` report
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

	// Initial load goes through the same `open` path as the command —
	// it sniffs .json (project) vs .WRL and sets up view/palette/cycler.
	let mut editor = EditorState::new(Project::empty(), args.size, None, resources_dir().join("assets"));
	if let Outcome::Failed(message) = editor.execute(Command::Open { path: args.map.clone(), force: true }) {
		eprintln!("{message}");
		std::process::exit(1);
	}

	// Settings: one `mme.ini` carries everything — paths, bindings,
	// mouse, UI layout. `--settings` always wins; a windowed run falls back
	// to the config default; headless without the flag stays off (keeps the
	// script suite from touching any file). Restore now if present.
	editor.headless = args.headless;
	editor.settings_path = args.settings.clone().or_else(|| (!args.headless).then(default_settings_path).flatten());
	let settings = editor.settings_path.as_deref().and_then(|path| match INI::from_file(path) {
		Ok(ini) => Some(ini),
		Err(e) => {
			if path.exists() {
				eprintln!("settings: {e}");
			}
			None
		}
	});
	if let Some(ini) = &settings {
		if let Some(section) = ini.get_section("Workspace") {
			let (w, h) = editor.screen;
			editor.workspace.apply_ini(section, w as f32, h as f32);
		}
		editor.max_path =
			ini.get_entry::<String>("Paths", "MaxPath").filter(|p| !p.trim().is_empty()).map(PathBuf::from);
	}
	match &editor.max_path {
		Some(path) if !path.is_dir() => {
			editor.console.push_line(format!("MaxPath is set but not a directory: {}", path.display()));
		}
		None => editor.console.push_line("MaxPath not set — point it at your M.A.X. directory in config/mme.ini"),
		Some(_) => {}
	}

	// Load the unit library up front when MaxPath is set — the Units panel
	// is then populated on first open, and headless screenshots render the
	// project's unit previews. Without MaxPath this is a no-op.
	if editor.max_path.is_some() {
		let _ = editor.ensure_units();
	}

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
	let mut renderer = make_renderer(&device, &queue, &editor, capture::FORMAT);
	let mut passes = Passes::new(&device, &queue, capture::FORMAT);
	let mut uploaded_revision = editor.revision();

	for command in script {
		match editor.execute(command) {
			Outcome::DocReplaced => {
				renderer = make_renderer(&device, &queue, &editor, capture::FORMAT);
				passes.minimap.invalidate();
				uploaded_revision = editor.revision();
			}
			Outcome::Screenshot { path, crop, resize } => {
				if editor.revision() != uploaded_revision {
					refresh_renderer(&renderer, &queue, &editor);
					uploaded_revision = editor.revision();
				}
				if let Some(rgba) = editor.cycler.take_if_dirty() {
					renderer.update_palette(&queue, rgba);
					if let Some(units) = &passes.units {
						units.update_palette(&queue, rgba);
					}
				}
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
	renderer: ProjectRenderer,
	passes: Passes,
	uploaded_revision: u64,
	title: String,
}

struct App {
	editor: EditorState,
	bindings: input::Bindings,
	startup_script: Vec<Command>,
	win: Option<WindowState>,
	cursor: (f32, f32),
	/// Cursor position at the last drag step, while a pan-drag is active.
	drag: Option<(f32, f32)>,
	/// A right press's origin, while held over the map — a release within a
	/// few px is a *click* (context menu), farther away it was a pan-drag.
	rclick: Option<(f32, f32)>,
	/// Last painted cell, while a paint-drag (stroke) is active.
	paint: Option<(u16, u16)>,
	/// A freehand select-drag: the mode plus the last applied cell.
	select_paint: Option<(map_core::SelectMode, (u16, u16))>,
	/// A rect select-drag's anchor cell + mode (applied on release).
	select_anchor: Option<(u16, u16, map_core::SelectMode)>,
	/// The minimap body being drag-panned, while one is.
	minipan: Option<crate::ui::Rect>,
	/// An HSL bar drag in the palette panel, while one is active.
	palette_drag: Option<PaletteDrag>,
	/// A scrollbar thumb drag in a scrollable panel, while one is active.
	scroll_drag: Option<ScrollDrag>,
	/// A button press waiting for its release: the action
	/// computed at press fires only if the release re-hits the same control —
	/// dragging off cancels. Selections, focus, and drag-starts stay
	/// press-fired (modals arm their own buttons via `Modal::on_release`).
	armed: Option<Armed>,
	/// True while the open modal is being dragged by its titlebar.
	modal_drag: bool,
	modifiers: ModifiersState,
	last_frame: std::time::Instant,
	/// Wall-clock start of the live Auto Fix Shore run.
	autofix_clock: Option<std::time::Instant>,
	/// Wall-clock start of the live New-from-Image conversion.
	convert_clock: Option<std::time::Instant>,
	/// False until the conversion's "Loading image…" state has been painted once
	/// — so the heavy first-stage decode starts only *after* the user sees it
	/// began; otherwise the decode blocks the very frame meant to show it.
	convert_primed: bool,
}

/// A palette-editor drag: colors captured at press time + which control;
/// each move re-derives absolute colors from the baseline (no drift), and
/// the whole press-to-release is one undo stroke.
enum PaletteDrag {
	/// Absolute slider on the selected color (0..=2 R/G/B, 3..=5 H/S/L):
	/// the value is wherever the cursor sits on the track.
	Slider { channel: usize, track: crate::ui::Rect, baseline: (u8, [u8; 3]) },
	/// Relative HSL shift of a whole water cycle block.
	Block { channel: usize, start_x: f32, baseline: Vec<(u8, [u8; 3])> },
}

/// A scrollbar drag: which panel, the grab offset within the thumb, and
/// the track/content metrics captured at press (constant for the drag).
#[derive(Clone, Copy)]
struct ScrollDrag {
	id: &'static str,
	grab: f32,
	track: crate::ui::Rect,
	content_h: f32,
	max: f32,
}

/// A deferred button click: the action resolved at press
/// time, plus what's needed to re-hit-test at release — firing only when the
/// release lands on the same control.
enum Armed {
	/// A tab-strip hit (select / close).
	Tab(tabs::Hit),
	/// A Tile Explorer header/grid action within `body`.
	Picker { body: crate::ui::Rect, action: picker::Action },
	/// A Units panel action within `body`.
	Units { body: crate::ui::Rect, action: units::Action },
	/// A minimap header radio within `body` (map pans stay press-fired).
	MinimapMode { body: crate::ui::Rect, mode: minimap::Mode },
	/// A toolbox key within `body`, identified by its (unique) label.
	Toolbox { body: crate::ui::Rect, label: &'static str },
	/// A palette toolbar/header button within `body` (slot selection and
	/// slider/bar drags stay press-fired).
	Palette { body: crate::ui::Rect, action: palette_panel::Action },
	/// A Templates Explorer action within `body` (`Pick` indexes the
	/// visible, pack-compatible list).
	Templates { body: crate::ui::Rect, action: templates_panel::Action },
}

impl App {
	fn new(editor: EditorState, bindings: input::Bindings, startup_script: Vec<Command>) -> Self {
		Self {
			editor,
			bindings,
			startup_script,
			win: None,
			cursor: (0.0, 0.0),
			drag: None,
			rclick: None,
			paint: None,
			select_paint: None,
			select_anchor: None,
			minipan: None,
			modal_drag: false,
			palette_drag: None,
			scroll_drag: None,
			armed: None,
			autofix_clock: None,
			convert_clock: None,
			convert_primed: false,
			modifiers: ModifiersState::empty(),
			last_frame: std::time::Instant::now(),
		}
	}

	/// The stroke command for a cell under the current mode + tool: pass-value
	/// paint in Pass Table Editor mode; otherwise erase (Eraser tool) or tile
	/// paint. Drives both the initial press and the drag continuation.
	fn paint_command(&self, x: u16, y: u16) -> Command {
		match self.editor.mode {
			state::EditorMode::Pass => Command::PassPaint { x, y, value: self.editor.active_pass },
			state::EditorMode::Map => match self.editor.tool {
				// Erase only the selected layer, not the topmost present.
				state::Tool::Eraser => {
					Command::Erase { x, y, layer: Some(self.editor.active_layer_name().to_string()) }
				}
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

	fn run(&mut self, command: Command, event_loop: &ActiveEventLoop) {
		let outcome = self.editor.execute(command);
		self.act_on(outcome, event_loop);
	}

	/// Act on an [`Outcome`] — from `execute`, or from a stepped job (autofix /
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
					win.renderer = make_renderer(&win.gpu.device, &win.gpu.queue, &self.editor, win.gpu.config.format);
					win.passes.minimap.invalidate();
					win.uploaded_revision = self.editor.revision();
					win.window.request_redraw();
				}
			}
			Outcome::Screenshot { path, crop, resize } => {
				if let Some(win) = self.win.as_mut() {
					if self.editor.revision() != win.uploaded_revision {
						refresh_renderer(&win.renderer, &win.gpu.queue, &self.editor);
						win.uploaded_revision = self.editor.revision();
					}
					if let Some(rgba) = self.editor.cycler.take_if_dirty() {
						win.renderer.update_palette(&win.gpu.queue, rgba);
						if let Some(units) = &win.passes.units {
							units.update_palette(&win.gpu.queue, rgba);
						}
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
			Outcome::Failed(message) => {
				eprintln!("FAILED: {message}");
				self.editor.raise_error(&message);
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

	/// The bound command for a pressed key whose *context* applies:
	/// context-specific matches (pass-value picks in the Pass Table Editor,
	/// tool switches in the map editor) beat context-free ones sharing the
	/// chord — table order never decides between contexts.
	fn bound_command(&self, key: &Key) -> Option<Command> {
		let (mut generic, mut specific) = (None, None);
		for cmd in self.bindings.lookup_all(self.modifiers, key) {
			let context = match &cmd {
				Command::PassPick { .. } | Command::PassPaint { .. } => {
					Some(self.editor.mode == state::EditorMode::Pass)
				}
				Command::ToolSelect { .. } => Some(self.editor.mode == state::EditorMode::Map),
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

	/// Act on a [`ModalAction`] returned by the open modal.
	fn apply_modal_action(&mut self, action: ModalAction, event_loop: &ActiveEventLoop) {
		match action {
			ModalAction::Consumed => {}
			ModalAction::Close => self.editor.close_modal(),
			ModalAction::Run(line) => {
				self.editor.close_modal();
				match command::parse_line(&line) {
					Ok(Some(cmd)) => self.run(cmd, event_loop),
					Ok(None) => {}
					Err(e) => self.editor.console.push_line(format!("modal: {e}")),
				}
			}
			// Generate Random Terrain: a stepped run the shell drives
			// per frame (see `redraw`); the modal stays open showing progress.
			ModalAction::StartGenerate => {
				let outcome = self.editor.generate_start();
				self.act_on(outcome, event_loop);
			}
			ModalAction::AbortGenerate => {
				let outcome = self.editor.generate_tick(true);
				self.act_on(outcome, event_loop);
			}
			ModalAction::Error(message) => self.editor.console.push_line(message),
			// Auto Fix Shore live run: the shell owns the wall clock and steps
			// the session per frame (see `redraw`); the modal stays open.
			ModalAction::StartFix => {
				self.editor.autofix_start();
				self.autofix_clock = Some(std::time::Instant::now());
			}
			ModalAction::StopFix => {
				self.editor.autofix_tick(self.autofix_elapsed(), true);
			}
			// New-from-Image live conversion: the shell owns the wall clock
			// and steps the session per frame (see `redraw`); the modal stays open.
			ModalAction::StartConvert => {
				let outcome = self.editor.convert_start();
				self.convert_clock = Some(std::time::Instant::now());
				// Paint the "Loading image…" state before the heavy decode starts.
				self.convert_primed = false;
				self.act_on(outcome, event_loop);
			}
			ModalAction::AbortConvert => {
				let outcome = self.editor.convert_tick(self.convert_elapsed(), true);
				self.act_on(outcome, event_loop);
			}
		}
	}

	/// Fire a deferred button click: re-hit-test the
	/// release position with the same pure `click` functions and run the
	/// action only when it matches what was armed at press — a release on a
	/// different control (or off every control) cancels the click.
	fn fire_armed(&mut self, armed: Armed, event_loop: &ActiveEventLoop) {
		let (cx, cy) = self.cursor;
		match armed {
			Armed::Tab(hit) => {
				let tab_infos = self.editor.tab_infos();
				let closable = self.editor.tabs_closable();
				let sw = self.editor.screen.0 as f32;
				if tabs::hit(&tab_infos, closable, menu::BAR_H, cx, cy, sw) != hit {
					return;
				}
				match hit {
					tabs::Hit::Select(i) => self.run(Command::Tab { index: i }, event_loop),
					tabs::Hit::Close(i) => {
						// Close-x makes the tab active first, then closes — its
						// unsaved-changes guard applies.
						self.run(Command::Tab { index: i }, event_loop);
						self.run(Command::CloseProject { force: false }, event_loop);
					}
					tabs::Hit::None => {}
				}
				self.redraw_win();
			}
			Armed::Picker { body, action } => {
				if picker::click(&self.editor.project, &self.editor.picker, body, cx, cy) != Some(action.clone()) {
					return;
				}
				match action {
					picker::Action::Pick(tile) => self.run(Command::Tile { spec: Some(tile) }, event_loop),
					picker::Action::CycleFilter => {
						self.run(Command::PickerFilter { name: "next".into() }, event_loop);
					}
					picker::Action::CycleSize => self.run(Command::PickerSize { size: "next".into() }, event_loop),
				}
			}
			Armed::Units { body, action } => {
				if units::click(self.editor.units.as_ref(), body, self.editor.units_scroll, cx, cy) != Some(action) {
					return;
				}
				match action {
					units::Action::Pick(i) => {
						if let Some(lib) = &self.editor.units {
							let tag = lib.units[i].tag.clone();
							self.run(Command::UnitSelect { tag: Some(tag) }, event_loop);
						}
					}
					units::Action::Team(t) => self.run(Command::UnitTeam { team: t.to_string() }, event_loop),
					units::Action::Eraser => {
						let name = if self.editor.tool == state::Tool::UnitEraser { "pencil" } else { "unit-eraser" };
						self.run(Command::ToolSelect { name: name.into() }, event_loop);
					}
				}
			}
			Armed::MinimapMode { body, mode } => {
				if minimap::click(self.editor.map_size(), body, cx, cy) != Some(minimap::Click::Mode(mode)) {
					return;
				}
				self.run(Command::MinimapMode { mode: mode.name().into() }, event_loop);
			}
			Armed::Toolbox { body, label } => {
				let hit = toolbox::click(body, cx, cy, self.editor.toolbox_scroll);
				let Some(button) = hit.filter(|b| b.label == label) else { return };
				match &button.act {
					toolbox::Act::Run(line) => {
						if let Ok(Some(cmd)) = command::parse_line(line) {
							self.run(cmd, event_loop);
						}
					}
					toolbox::Act::Todo(ticket) => {
						let msg = format!("{}: not implemented yet — backlog {ticket}", button.label);
						eprintln!("{msg}");
						self.editor.console.push_line(msg);
					}
				}
			}
			Armed::Palette { body, action } => {
				let hit = palette_panel::click(
					body,
					self.editor.active_color.map(u16::from),
					self.editor.palette_sel_end.map(u16::from),
					true,
					self.editor.palette_scroll,
					cx,
					cy,
					self.modifiers.shift_key(),
					self.editor.palette_show_saved,
					self.editor.palette_files.len(),
				);
				if hit != Some(action) {
					return;
				}
				match action {
					palette_panel::Action::ShowSaved(saved) => self.run(Command::PaletteTab { saved }, event_loop),
					palette_panel::Action::Save => {
						self.run(Command::FileDialog { purpose: command::FilePurpose::SavePalette }, event_loop);
					}
					palette_panel::Action::Load => {
						self.run(Command::FileDialog { purpose: command::FilePurpose::LoadPalette }, event_loop);
					}
					palette_panel::Action::LoadSaved(i) => {
						if let Some(path) = self.editor.palette_files.get(i).cloned() {
							self.run(Command::PaletteLoad { path }, event_loop);
						}
					}
					palette_panel::Action::Cycle(on) => self.run(Command::Animate { on: Some(on) }, event_loop),
					// Selections and drags never arm.
					_ => {}
				}
			}
			Armed::Templates { body, action } => {
				let visible = self.editor.visible_templates();
				if templates_panel::click(visible.len(), body, self.editor.templates_scroll, cx, cy) != Some(action) {
					return;
				}
				match action {
					templates_panel::Action::Pick(i) => {
						if let Some(&g) = visible.get(i) {
							let name = self.editor.templates[g].name.clone();
							self.run(Command::TemplatePick { name }, event_loop);
						}
					}
					templates_panel::Action::Save => self.run(Command::TemplateSave { name: None }, event_loop),
					templates_panel::Action::Import => {
						self.run(Command::FileDialog { purpose: command::FilePurpose::ImportTemplate }, event_loop);
					}
					templates_panel::Action::Delete => self.run(Command::TemplateDelete { name: None }, event_loop),
				}
			}
		}
	}

	/// Scrollbar geometry for a scrollable panel body: `(track rect, content
	/// height, max scroll, current scroll)`, or `None` when it doesn't scroll.
	/// `content_h = max + region.h` matches what [`ui::UiQuads::scrollbar`] draws.
	fn scrollbar_of(&self, id: &str, body: crate::ui::Rect) -> Option<(crate::ui::Rect, f32, f32, f32)> {
		let (region, max, scroll) = match id {
			"palette" => (palette_panel::grid_area(body), palette_panel::max_scroll(body), self.editor.palette_scroll),
			"tiles" => {
				let count = picker::items(&self.editor.project, self.editor.picker.filter).len();
				let max = picker::max_scroll(count, body, self.editor.picker.tile_px);
				(picker::scissor(body), max, self.editor.picker.scroll)
			}
			"toolbox" => (body, toolbox::max_scroll(body), self.editor.toolbox_scroll),
			"units" => {
				let count = self.editor.units.as_ref().map(|l| l.units.len()).unwrap_or(0);
				(units::scissor(body), units::max_scroll(count, body), self.editor.units_scroll)
			}
			"templates" => {
				let count = self.editor.visible_templates().len();
				(templates_panel::scissor(body), templates_panel::max_scroll(count, body), self.editor.templates_scroll)
			}
			_ => return None,
		};
		if max <= 0.0 || region.h <= 0.0 {
			return None;
		}
		let track = crate::ui::Rect::new(region.x + region.w - ui::SCROLLBAR_W, region.y, ui::SCROLLBAR_W, region.h);
		Some((track, max + region.h, max, scroll))
	}

	/// Re-derive the active scrollbar drag's offset from the cursor (grab-relative
	/// thumb) and write it to the panel's scroll field.
	fn update_scroll_drag(&mut self) {
		let Some(d) = self.scroll_drag else { return };
		let thumb_h = (d.track.h * d.track.h / d.content_h).clamp(16.0f32.min(d.track.h), d.track.h);
		let span = (d.track.h - thumb_h).max(1.0);
		let t = ((self.cursor.1 - d.grab - d.track.y) / span).clamp(0.0, 1.0);
		let scroll = t * d.max;
		match d.id {
			"palette" => self.editor.palette_scroll = scroll,
			"tiles" => self.editor.picker.scroll = scroll,
			"toolbox" => self.editor.toolbox_scroll = scroll,
			"units" => self.editor.units_scroll = scroll,
			"templates" => self.editor.templates_scroll = scroll,
			_ => {}
		}
		if let Some(win) = self.win.as_ref() {
			win.window.request_redraw();
		}
	}

	/// Re-derive colors from the active palette drag's baseline + cursor and
	/// apply them (absolute — repeated HSL round trips would drift).
	fn apply_palette_drag(&mut self, event_loop: &ActiveEventLoop) {
		let edits: Vec<Command> = match &self.palette_drag {
			None => return,
			Some(PaletteDrag::Slider { channel, track, baseline: (slot, rgb) }) => {
				let t = ((self.cursor.0 - track.x) / track.w).clamp(0.0, 1.0);
				let rgb = match channel {
					0..=2 => {
						let mut out = *rgb;
						out[*channel] = (t * 255.0).round() as u8;
						out
					}
					_ => {
						let (hue, sat, light) = map_core::rgb_to_hsl(*rgb);
						match channel {
							3 => map_core::hsl_to_rgb(t * 360.0, sat, light),
							4 => map_core::hsl_to_rgb(hue, t, light),
							_ => map_core::hsl_to_rgb(hue, sat, t),
						}
					}
				};
				vec![Command::SetColor { slot: *slot, rgb }]
			}
			Some(PaletteDrag::Block { channel, start_x, baseline }) => {
				let dx = self.cursor.0 - start_x;
				let (dh, ds, dl) = match channel {
					0 => (dx * palette_panel::HUE_PER_PX, 0.0, 0.0),
					1 => (0.0, dx * palette_panel::SL_PER_PX, 0.0),
					_ => (0.0, 0.0, dx * palette_panel::SL_PER_PX),
				};
				baseline
					.iter()
					.map(|&(slot, rgb)| {
						let (hue, sat, light) = map_core::rgb_to_hsl(rgb);
						Command::SetColor { slot, rgb: map_core::hsl_to_rgb(hue + dh, sat + ds, light + dl) }
					})
					.collect()
			}
		};
		for command in edits {
			self.run(command, event_loop);
		}
	}

	/// Keyboard routing while the console is open.
	fn console_key(&mut self, event: &winit::event::KeyEvent, event_loop: &ActiveEventLoop) {
		match &event.logical_key {
			Key::Named(NamedKey::Escape) | Key::Named(NamedKey::F1) => {
				self.run(Command::Console { on: Some(false) }, event_loop);
				return;
			}
			Key::Character(c) if c.as_str() == "`" => {
				self.run(Command::Console { on: Some(false) }, event_loop);
				return;
			}
			Key::Named(NamedKey::Enter) => {
				if let Some(line) = self.editor.console.submit() {
					match command::parse_line(&line) {
						Ok(Some(cmd)) => self.run(cmd, event_loop),
						Ok(None) => {}
						Err(e) => self.editor.console.push_line(format!("error: {e}")),
					}
				}
			}
			Key::Named(NamedKey::Backspace) => self.editor.console.backspace(),
			Key::Named(NamedKey::ArrowUp) => self.editor.console.history_prev(),
			Key::Named(NamedKey::ArrowDown) => self.editor.console.history_next(),
			Key::Named(NamedKey::PageUp) => self.editor.console.scroll_lines(5),
			Key::Named(NamedKey::PageDown) => self.editor.console.scroll_lines(-5),
			_ => {
				if let Some(text) = &event.text {
					self.editor.console.insert(text);
				}
			}
		}
		if let Some(win) = self.win.as_ref() {
			win.window.request_redraw();
		}
	}

	fn redraw(&mut self, event_loop: &ActiveEventLoop) {
		// Animation: advance the working palette by real frame time.
		if self.editor.animate {
			let dt = self.last_frame.elapsed().as_secs_f32().min(0.25);
			self.editor.tick(dt);
		}
		self.last_frame = std::time::Instant::now();

		// Auto Fix Shore: step the live run a slice per frame.
		if self.editor.autofix_running() {
			self.editor.autofix_tick(self.autofix_elapsed(), false);
		}

		// Generate Random Terrain: step the live run within a
		// per-frame time budget — the progress bar fills, the UI stays live.
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
		// first frame after Convert only *paints* the "Loading image…" state —
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
				self.act_on(outcome, event_loop);
			}
		}

		let Some(win) = self.win.as_mut() else { return };

		if self.editor.revision() != win.uploaded_revision {
			refresh_renderer(&win.renderer, &win.gpu.queue, &self.editor);
			win.uploaded_revision = self.editor.revision();
		}
		if let Some(rgba) = self.editor.cycler.take_if_dirty() {
			win.renderer.update_palette(&win.gpu.queue, rgba);
			if let Some(units) = &win.passes.units {
				units.update_palette(&win.gpu.queue, rgba);
			}
		}

		let title = self.editor.title();
		if title != win.title {
			win.window.set_title(&title);
			win.title = title;
		}

		let frame = match win.gpu.surface.get_current_texture() {
			Ok(frame) => frame,
			Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
				win.gpu.surface.configure(&win.gpu.device, &win.gpu.config);
				win.window.request_redraw();
				return;
			}
			Err(e) => {
				eprintln!("fatal surface error: {e}");
				event_loop.exit();
				return;
			}
		};

		let target = frame.texture.create_view(&Default::default());
		let mut encoder = win.gpu.device.create_command_encoder(&Default::default());
		render_frame(
			&win.gpu.device,
			&win.gpu.queue,
			&mut encoder,
			&target,
			&mut self.editor,
			&win.renderer,
			&mut win.passes,
		);
		win.gpu.queue.submit([encoder.finish()]);
		frame.present();

		if self.editor.animate
			|| self.editor.autofix_running()
			|| self.editor.converting()
			|| self.editor.generate_running()
		{
			win.window.request_redraw();
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
		let renderer = make_renderer(&gpu.device, &gpu.queue, &self.editor, gpu.config.format);
		let passes = Passes::new(&gpu.device, &gpu.queue, gpu.config.format);

		self.editor.screen = (gpu.config.width, gpu.config.height);
		let _ = self.editor.execute(Command::Fit);
		let uploaded_revision = self.editor.revision();

		window.request_redraw();
		self.win = Some(WindowState { window, gpu, renderer, passes, uploaded_revision, title });

		// Startup script runs through the exact same path as live input.
		for command in std::mem::take(&mut self.startup_script) {
			self.run(command, event_loop);
		}
	}

	fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
		match event {
			WindowEvent::CloseRequested => event_loop.exit(),

			WindowEvent::ModifiersChanged(modifiers) => {
				self.modifiers = modifiers.state();
			}

			WindowEvent::KeyboardInput { event, .. } if event.state.is_pressed() => {
				// Console captures the keyboard while open.
				if self.editor.console.is_open() {
					self.console_key(&event, event_loop);
					return;
				}
				// An open text-field modal captures the keyboard next.
				if self.editor.active_modal().is_some() {
					let key = match &event.logical_key {
						Key::Named(NamedKey::Enter) => Some(ModalKey::Enter),
						Key::Named(NamedKey::Escape) => Some(ModalKey::Escape),
						Key::Named(NamedKey::Backspace) => Some(ModalKey::Backspace),
						Key::Named(NamedKey::Tab) => Some(ModalKey::Tab),
						_ => None,
					};
					if let Some(key) = key {
						let action = self.editor.active_modal().unwrap().on_key(key);
						self.apply_modal_action(action, event_loop);
					} else if let Some(text) = event.text.clone() {
						for c in text.chars() {
							self.editor.active_modal().unwrap().on_key(ModalKey::Char(c));
						}
					}
					self.redraw_win();
					return;
				}
				// Esc closes an open menu before it can mean `quit`.
				if self.editor.menu.open.is_some() && event.logical_key == Key::Named(NamedKey::Escape) {
					self.editor.menu.close();
					if let Some(win) = self.win.as_ref() {
						win.window.request_redraw();
					}
					return;
				}
				// Esc next closes the context menu, then disarms a ghost
				// stamp, then drops the selection — only an idle Esc reaches
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
					if !self.editor.selection.is_empty() {
						self.run(Command::SelectOp { op: "clear".into() }, event_loop);
						return;
					}
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
				// stay lit after the mouse leaves the window.
				self.editor.hot.cursor = None;
				if let Some(win) = self.win.as_ref() {
					win.window.request_redraw();
				}
			}

			WindowEvent::CursorMoved { position, .. } => {
				let prev = self.cursor;
				self.cursor = (position.x as f32, position.y as f32);
				let (sw, sh) = (self.editor.screen.0 as f32, self.editor.screen.1 as f32);
				// Feed the pointer snapshot for hover/pressed widget states; any
				// move can change what's under the cursor, so always redraw.
				self.editor.hot.cursor = Some(self.cursor);
				if let Some(win) = self.win.as_ref() {
					win.window.request_redraw();
				}
				// Dragging a modal by its titlebar takes the whole move.
				if self.modal_drag {
					let (dx, dy) = (self.cursor.0 - prev.0, self.cursor.1 - prev.1);
					if let Some(modal) = self.editor.active_modal() {
						modal.drag(dx, dy);
					}
					self.redraw_win();
					return;
				}
				if self.editor.menu.on_move(self.cursor.0, self.cursor.1, sw) {
					if let Some(win) = self.win.as_ref() {
						win.window.request_redraw();
					}
				}
				if let Some(cm) = &mut self.editor.context_menu {
					if cm.on_move(self.cursor.0, self.cursor.1, sw, sh) {
						if let Some(win) = self.win.as_ref() {
							win.window.request_redraw();
						}
					}
				}
				if self.editor.workspace.on_move(self.cursor.0, self.cursor.1, sw, sh) {
					if let Some(win) = self.win.as_ref() {
						win.window.request_redraw();
					}
				}
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
				if let Some(body) = self.minipan {
					let (x, y) = minimap::pan_target(self.editor.map_size(), body, self.cursor.0, self.cursor.1);
					self.run(Command::PanTo { x, y }, event_loop);
				}
				if self.palette_drag.is_some() {
					self.apply_palette_drag(event_loop);
				}
				if self.scroll_drag.is_some() {
					self.update_scroll_drag();
				}
			}

			WindowEvent::MouseInput { state, button, .. } if self.bindings.is_paint_button(button) => {
				let (sw, sh) = (self.editor.screen.0 as f32, self.editor.screen.1 as f32);
				match state {
					ElementState::Pressed => {
						// Arm the pressed-widget visual: the press origin lives until
						// release, so buttons render sunken while held.
						self.editor.hot.down = Some(self.cursor);
						// A stale deferred click (its release never arrived — e.g.
						// it landed outside the window) must not fire later.
						self.armed = None;
						// An open context menu is topmost: an item press runs
						// its command; any other press just closes it.
						if let Some(cm) = self.editor.context_menu.take() {
							let press = cm.on_press(self.cursor.0, self.cursor.1, sw, sh);
							match press {
								menu::Press::Run(line) => match command::parse_line(&line) {
									Ok(Some(cmd)) => self.run(cmd, event_loop),
									Ok(None) => {}
									Err(e) => eprintln!("context menu: {e}"),
								},
								menu::Press::Todo(label, ticket) => {
									let msg = format!("{label}: not implemented yet — backlog {ticket}");
									eprintln!("{msg}");
									self.editor.console.push_line(msg);
								}
								_ => {}
							}
							self.redraw_win();
							return;
						}
						// An open text-field modal swallows every press; a press on
						// its titlebar starts a drag, otherwise it routes in.
						if self.editor.active_modal().is_some() {
							let (cx, cy) = self.cursor;
							if self.editor.active_modal().unwrap().titlebar(sw, sh).contains(cx, cy) {
								self.modal_drag = true;
							} else {
								let action = self.editor.active_modal().unwrap().on_press(cx, cy, sw, sh);
								self.apply_modal_action(action, event_loop);
							}
							self.redraw_win();
							return;
						}
						// The menu bar is next: it sees the press first.
						match self.editor.menu.on_press(self.cursor.0, self.cursor.1, sw) {
							menu::Press::None => {}
							press => {
								match press {
									menu::Press::Run(line) => match command::parse_line(&line) {
										Ok(Some(cmd)) => self.run(cmd, event_loop),
										Ok(None) => {}
										Err(e) => eprintln!("menu: {e}"),
									},
									menu::Press::Todo(label, ticket) => {
										let msg = format!("{label}: not implemented yet — backlog {ticket}",);
										eprintln!("{msg}");
										self.editor.console.push_line(msg);
									}
									_ => {}
								}
								if let Some(win) = self.win.as_ref() {
									win.window.request_redraw();
								}
								return;
							}
						}
						// Project tab strip next: switch / close arm and fire on
						// release-inside.
						let tab_infos = self.editor.tab_infos();
						let closable = self.editor.tabs_closable();
						match tabs::hit(&tab_infos, closable, menu::BAR_H, self.cursor.0, self.cursor.1, sw) {
							tabs::Hit::None => {}
							hit => {
								self.armed = Some(Armed::Tab(hit));
								self.redraw_win();
								return;
							}
						}
						// Workspace next (panels swallow clicks); otherwise
						// paint — silently inert without a project + active
						// tile, so bare map clicks don't spam errors.
						match self.editor.workspace.on_press(self.cursor.0, self.cursor.1, sw, sh) {
							workspace::Press::Chrome => {
								if let Some(win) = self.win.as_ref() {
									win.window.request_redraw();
								}
							}
							workspace::Press::Body { id, body } => {
								let (cx, cy) = self.cursor;
								// A press in the scrollbar gutter starts a thumb drag (jumping
								// the page when the track, not the thumb, is hit).
								let on_bar = self.scrollbar_of(id, body).filter(|(track, ..)| track.contains(cx, cy));
								if let Some((track, content_h, max, scroll)) = on_bar {
									let thumb_h = (track.h * track.h / content_h).clamp(16.0f32.min(track.h), track.h);
									let thumb_y = track.y + (scroll / max).clamp(0.0, 1.0) * (track.h - thumb_h);
									let grab = if cy >= thumb_y && cy <= thumb_y + thumb_h {
										cy - thumb_y // grabbed the thumb: keep the grab point under the cursor
									} else {
										thumb_h / 2.0 // clicked the track: centre the thumb on the cursor
									};
									self.scroll_drag = Some(ScrollDrag { id, grab, track, content_h, max });
									self.update_scroll_drag();
								} else if id == "tiles" {
									// Tile picks + header buttons arm; they fire on
									// release-inside.
									if let Some(action) = picker::click(
										&self.editor.project,
										&self.editor.picker,
										body,
										self.cursor.0,
										self.cursor.1,
									) {
										self.armed = Some(Armed::Picker { body, action });
									}
								} else if id == "units" {
									if let Some(action) = units::click(
										self.editor.units.as_ref(),
										body,
										self.editor.units_scroll,
										self.cursor.0,
										self.cursor.1,
									) {
										self.armed = Some(Armed::Units { body, action });
									}
								} else if id == "minimap" {
									match minimap::click(self.editor.map_size(), body, self.cursor.0, self.cursor.1) {
										Some(minimap::Click::Mode(m)) => {
											self.armed = Some(Armed::MinimapMode { body, mode: m });
										}
										Some(minimap::Click::Pan(x, y)) => {
											// Click pans; holding drags the view (a drag
											// start — stays press-fired).
											self.minipan = Some(body);
											self.run(Command::PanTo { x, y }, event_loop);
										}
										None => {}
									}
								} else if id == "toolbox" {
									if let Some(button) =
										toolbox::click(body, self.cursor.0, self.cursor.1, self.editor.toolbox_scroll)
									{
										self.armed = Some(Armed::Toolbox { body, label: button.label });
									}
								} else if id == "templates" {
									let count = self.editor.visible_templates().len();
									if let Some(action) = templates_panel::click(
										count,
										body,
										self.editor.templates_scroll,
										self.cursor.0,
										self.cursor.1,
									) {
										self.armed = Some(Armed::Templates { body, action });
									}
								} else if id == "palette" {
									match palette_panel::click(
										body,
										self.editor.active_color.map(u16::from),
										self.editor.palette_sel_end.map(u16::from),
										true,
										self.editor.palette_scroll,
										self.cursor.0,
										self.cursor.1,
										self.modifiers.shift_key(),
										self.editor.palette_show_saved,
										self.editor.palette_files.len(),
									) {
										// Selections are immediate; toolbar/header buttons
										// arm and fire on release.
										Some(palette_panel::Action::Select(slot)) => {
											self.run(Command::Color { index: slot as u8 }, event_loop);
										}
										Some(
											action @ (palette_panel::Action::ShowSaved(_)
											| palette_panel::Action::Save
											| palette_panel::Action::Load
											| palette_panel::Action::LoadSaved(_)
											| palette_panel::Action::Cycle(_)),
										) => {
											self.armed = Some(Armed::Palette { body, action });
										}
										Some(palette_panel::Action::SelectTo(slot)) => {
											self.run(Command::ColorTo { index: slot as u8 }, event_loop);
										}
										Some(palette_panel::Action::Slider { channel, track }) => {
											if let Some(slot) = self.editor.active_color {
												let project = &self.editor.project;
												let at = slot as usize * 3;
												let p = &project.palette;
												self.palette_drag = Some(PaletteDrag::Slider {
													channel,
													track,
													baseline: (slot, [p[at], p[at + 1], p[at + 2]]),
												});
												self.run(Command::Stroke { begin: true }, event_loop);
												// Click-to-set: apply immediately.
												self.apply_palette_drag(event_loop);
											}
										}
										Some(palette_panel::Action::BlockBar { channel }) => {
											// Capture baseline colors; the drag re-derives from
											// them (one stroke). A multi-select range shifts its
											// editable slots; a single water slot shifts its block.
											let sel = palette_panel::selection(
												self.editor.active_color.map(u16::from),
												self.editor.palette_sel_end.map(u16::from),
											);
											let slots: Vec<u8> = match sel {
												Some((lo, hi)) if lo != hi => palette_panel::editable_in(lo, hi)
													.iter()
													.map(|&s| s as u8)
													.collect(),
												Some((lo, _)) => palette_panel::water_block(lo)
													.map_or_else(Vec::new, |(s, e)| (s..=e).collect()),
												None => Vec::new(),
											};
											if !slots.is_empty() {
												let project = &self.editor.project;
												let baseline = slots
													.iter()
													.map(|&s| {
														let at = s as usize * 3;
														let p = &project.palette;
														(s, [p[at], p[at + 1], p[at + 2]])
													})
													.collect();
												self.palette_drag = Some(PaletteDrag::Block {
													channel,
													start_x: self.cursor.0,
													baseline,
												});
												self.run(Command::Stroke { begin: true }, event_loop);
											}
										}
										None => {}
									}
								}
								if let Some(win) = self.win.as_ref() {
									win.window.request_redraw();
								}
							}
							workspace::Press::None if self.editor.mode == state::EditorMode::Pass => {
								// Pass Table Editor: LMB paints the active pass
								// value (drag = one undo stroke).
								if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
									self.paint = Some((x, y));
									self.run(Command::Stroke { begin: true }, event_loop);
									self.run(self.paint_command(x, y), event_loop);
								}
							}
							workspace::Press::None => {
								// An armed ghost stamp takes the click: place it at
								// the cell under the cursor (it stays armed for
								// repeat stamping; Esc disarms).
								if self.editor.stamp.is_some() {
									if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
										self.run(Command::Stamp { x, y }, event_loop);
									}
									return;
								}
								// LMB on the map: the active tool decides.
								match self.editor.tool {
									state::Tool::Picker => {
										if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
											self.run(Command::Pick { x, y }, event_loop);
										}
									}
									// Pencil paints, Eraser erases — both stroke
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
									// Flood fill: a single click fills the region
									// (its own undo unit — no drag).
									state::Tool::Fill => {
										if self.editor.can_paint() {
											if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
												self.run(Command::Fill { x, y }, event_loop);
											}
										}
									}
									// Unit stamp: one click = one preview (no
									// stroke). Shift keeps stamping; a plain
									// click drops back to the pencil.
									state::Tool::Unit => {
										if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
											self.run(Command::Paint { x, y }, event_loop);
											if !self.modifiers.shift_key() {
												self.run(Command::ToolSelect { name: "pencil".into() }, event_loop);
											}
										}
									}
									state::Tool::UnitEraser => {
										if let Some((x, y)) = self.editor.cell_at(self.cursor.0, self.cursor.1) {
											self.run(Command::UnitErase { x, y }, event_loop);
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
						}
					}
					ElementState::Released => {
						// Disarm the pressed-widget visual (and repaint the lift).
						self.editor.hot.down = None;
						if let Some(win) = self.win.as_ref() {
							win.window.request_redraw();
						}
						// A modal's armed command button fires on release-inside;
						// a titlebar drag just ends.
						if self.editor.active_modal().is_some() && !self.modal_drag {
							let (cx, cy) = self.cursor;
							let action = self.editor.active_modal().unwrap().on_release(cx, cy, sw, sh);
							self.apply_modal_action(action, event_loop);
						}
						// Panel/tab buttons armed at press fire the same way.
						if let Some(armed) = self.armed.take() {
							self.fire_armed(armed, event_loop);
						}
						// A select drag ends: freehand just stops; the rect
						// applies anchor → release cell in one command.
						self.select_paint = None;
						if let Some((ax, ay, mode)) = self.select_anchor.take() {
							self.editor.select_preview = None;
							let (x, y) = self.editor.cell_at(self.cursor.0, self.cursor.1).unwrap_or((ax, ay));
							self.run(Command::SelectRect { x0: ax, y0: ay, x1: x, y1: y, mode }, event_loop);
						}
						self.minipan = None;
						self.modal_drag = false;
						self.scroll_drag = None;
						if self.palette_drag.take().is_some() {
							self.run(Command::Stroke { begin: false }, event_loop);
						}
						if self.editor.workspace.on_release(self.cursor.0, self.cursor.1, sw, sh) {
							if let Some(win) = self.win.as_ref() {
								win.window.request_redraw();
							}
						} else if self.paint.is_some() {
							self.paint = None;
							self.run(Command::Stroke { begin: false }, event_loop);
						}
					}
				}
			}

			WindowEvent::MouseInput { state, button, .. }
				if self.bindings.is_pan_button(button) || button == MouseButton::Right =>
			{
				let (sw, sh) = (self.editor.screen.0 as f32, self.editor.screen.1 as f32);
				// Not from the menu bar, tab strip, panels, or under a modal.
				let over_map = self.cursor.1 >= menu::BAR_H + tabs::BAR_H
					&& self.editor.menu.open.is_none()
					&& self.editor.active_modal().is_none()
					&& !self.editor.workspace.over_ui(self.cursor.0, self.cursor.1, sw, sh);
				match state {
					ElementState::Pressed => {
						self.drag = (over_map && self.bindings.is_pan_button(button)).then_some(self.cursor);
						// A right press might be a click (context menu) or a
						// pan-drag — decided by how far the release lands.
						self.rclick = (button == MouseButton::Right && over_map && self.editor.context_menu.is_none())
							.then_some(self.cursor);
					}
					ElementState::Released => {
						self.drag = None;
						if button == MouseButton::Right {
							if let Some((px, py)) = self.rclick.take() {
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
				// The context menu baked the clicked cell into its items —
				// close it rather than let the view scroll out from under it.
				if self.editor.context_menu.is_some() {
					self.run(Command::ContextMenu { at: None }, event_loop);
				}
				let steps = match delta {
					MouseScrollDelta::LineDelta(_, y) => y,
					MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 60.0,
				};
				// An open modal takes the wheel — the file dialog scrolls its
				// list; the others just swallow it.
				if let Some(modal) = self.editor.active_modal() {
					modal.on_wheel(steps);
					self.redraw_win();
					return;
				}
				let (sw, sh) = (self.editor.screen.0 as f32, self.editor.screen.1 as f32);
				// Wheel over a panel belongs to the panel (picker scroll,
				// minimap zoom); over the map it zooms at the cursor.
				if let Some((id, body)) = self.editor.workspace.body_at(self.cursor.0, self.cursor.1, sw, sh) {
					if id == "tiles" {
						{
							let project = &self.editor.project;
							let count = picker::items(project, self.editor.picker.filter).len();
							let max = picker::max_scroll(count, body, self.editor.picker.tile_px);
							self.editor.picker.scroll =
								(self.editor.picker.scroll - steps * picker::WHEEL_STEP).clamp(0.0, max);
							if let Some(win) = self.win.as_ref() {
								win.window.request_redraw();
							}
						}
					} else if id == "minimap" {
						self.run(Command::Zoom { factor: self.bindings.zoom_step().powf(steps) }, event_loop);
					} else if id == "palette" {
						let max = palette_panel::max_scroll(body);
						self.editor.palette_scroll =
							(self.editor.palette_scroll - steps * picker::WHEEL_STEP).clamp(0.0, max);
						if let Some(win) = self.win.as_ref() {
							win.window.request_redraw();
						}
					} else if id == "toolbox" {
						let max = toolbox::max_scroll(body);
						self.editor.toolbox_scroll =
							(self.editor.toolbox_scroll - steps * picker::WHEEL_STEP).clamp(0.0, max);
						if let Some(win) = self.win.as_ref() {
							win.window.request_redraw();
						}
					} else if id == "units" {
						let count = self.editor.units.as_ref().map(|l| l.units.len()).unwrap_or(0);
						let max = units::max_scroll(count, body);
						self.editor.units_scroll =
							(self.editor.units_scroll - steps * units::WHEEL_STEP).clamp(0.0, max);
						if let Some(win) = self.win.as_ref() {
							win.window.request_redraw();
						}
					} else if id == "templates" {
						let count = self.editor.visible_templates().len();
						let max = templates_panel::max_scroll(count, body);
						self.editor.templates_scroll =
							(self.editor.templates_scroll - steps * templates_panel::WHEEL_STEP).clamp(0.0, max);
						if let Some(win) = self.win.as_ref() {
							win.window.request_redraw();
						}
					}
				} else {
					self.run(
						Command::ZoomAt {
							x: self.cursor.0,
							y: self.cursor.1,
							factor: self.bindings.zoom_step().powf(steps),
						},
						event_loop,
					);
				}
			}

			WindowEvent::RedrawRequested => self.redraw(event_loop),

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
		// connection closes — vkDestroySurfaceKHR then segfaults.
		self.win = None;
	}
}
