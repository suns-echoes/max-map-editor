//! Dockable workspace: four edge docks around the map view plus a floating
//! layer of in-app windows. The **docking model** — geometry, hit-testing,
//! drag/resize/dock/float input, visibility, and a serializable layout — now
//! lives in the toolkit ([`wgpu_ui::workspace`]); this module is the editor's
//! thin wrapper: it delegates the model through [`Deref`], adds the editor's
//! steel-material chrome ([`draw_panel`](Workspace::draw_panel)/
//! [`draw_background`](Workspace::draw_background)/[`draw_peeks`](Workspace::draw_peeks)/
//! [`steel_map`](Workspace::steel_map)), its default panel set, and `[Workspace]`
//! INI persistence. Chrome + persistence stay here because they couple to the
//! `SteelTheme` (anchored per-panel grain) and the `mme.ini` format.

use ini::INISection;
use wgpu_ui::workspace::{self as ws, PanelSpec};
use wgpu_ui::{DrawList, Emboss, Fonts, Rect as WRect, TextRole, Theme, Vec2};

use crate::theme;
use crate::ui::{self, Rect, SteelMap};
use crate::uikit_theme::{SteelTheme, rgba};

/// Dock side indices + placement + the serializable layout snapshot,
/// re-exported from the toolkit model.
pub use wgpu_ui::workspace::{BOTTOM, LEFT, Place, RIGHT, TOP, WorkspaceLayout};

/// The floating-window resize-handle square (mirrors the model's, for drawing
/// the grip; the hit area lives in the toolkit).
const HANDLE: f32 = 14.0;

/// An embossed chrome label clipped to `r`, left-aligned at `r.x + pad` and
/// vertically centred - the panel-title / close-glyph drawing. Titlebar text is
/// `Raised` (hilite + shadow).
fn label(
	dl: &mut DrawList,
	skin: &SteelTheme,
	fonts: &Fonts,
	r: Rect,
	pad: f32,
	s: &str,
	color: [f32; 4],
	emboss: Emboss,
) {
	let px = skin.font_px(TextRole::Body);
	dl.push_clip(r);
	let baseline = Vec2::new(r.x + pad, r.y + r.h * 0.5 + px * 0.34);
	skin.emboss_text(dl, fonts, baseline, s, px, rgba(color), emboss);
	dl.pop_clip();
}

/// A word-wrapped engraved hint filling `r` from the top-left (padded by `pad`) -
/// the placeholder shown in a content-less panel, measured in the wgpu-ui font so
/// the breaks land where they're drawn.
fn hint(dl: &mut DrawList, skin: &SteelTheme, fonts: &Fonts, r: Rect, pad: f32, s: &str, color: [f32; 4]) {
	let px = skin.font_px(TextRole::Small);
	let max_w = (r.w - 2.0 * pad).max(0.0);
	let line_h = px + 4.0;
	let mut y = r.y + pad + px * 0.8; // top → first baseline (≈ ascent)
	let mut cur = String::new();
	for word in s.split_whitespace() {
		let trial = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
		if cur.is_empty() || fonts.get(skin.font()).measure(&trial, px) <= max_w {
			cur = trial;
		} else {
			skin.emboss_text(dl, fonts, Vec2::new(r.x + pad, y), &cur, px, rgba(color), Emboss::Engraved);
			y += line_h;
			cur = word.to_string();
		}
	}
	if !cur.is_empty() {
		skin.emboss_text(dl, fonts, Vec2::new(r.x + pad, y), &cur, px, rgba(color), Emboss::Engraved);
	}
}

/// What a primary-button press hit. Editor-side (`id` is a static panel key,
/// mapped back from the model's owned id) so the shell's routing stays on
/// `&'static str` matches.
#[derive(Debug, Clone, PartialEq)]
pub enum Press {
	None,
	/// Titlebar / close / splitter / resizer - handled internally by the model.
	Chrome,
	/// A panel body - the shell routes content interaction (picker, …).
	Body {
		id: &'static str,
		body: Rect,
	},
}

/// The editor's known panel ids (also the map from a model `String` id back to
/// the `&'static str` the shell routes on).
///
/// **Every panel with a hosted `Ui` must be listed here.** A missing id maps to
/// `""`, which no [`panel_host`] arm answers to - so the panel is drawn, but
/// every press, wheel notch and paging key routed by id is silently dropped and
/// the panel looks alive and is completely dead. That is exactly what the
/// Scenery panel did until it was added here.
///
/// [`panel_host`]: crate::panel_host
const PANEL_IDS: [&str; 11] = [
	"minimap",
	"tiles",
	"palette",
	"wrlpalette",
	"toolbox",
	"units",
	"templates",
	"scenery",
	"savetools",
	"unitprops",
	"passtools",
];

/// Map a model panel id back to its `&'static str` (all editor panels are known).
fn static_id(id: &str) -> &'static str {
	PANEL_IDS.iter().copied().find(|&s| s == id).unwrap_or("")
}

/// An independent dock layout the user arranges and the editor persists. Each
/// [`crate::state::EditorMode`] maps to exactly one group (see
/// [`EditorMode::layout_group`](crate::state::EditorMode::layout_group)): the
/// Map editor keeps the main layout, the two pass editors share one, and the
/// save editor has its own. The discriminants index the slot array on
/// [`EditorState`](crate::state::EditorState).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutGroup {
	/// Map editor - the main `[Workspace]` section.
	Main = 0,
	/// Pass Table + Local Pass Override editors (shared) - `[Workspace.Pass]`.
	Pass = 1,
	/// Save editor (experimental) - `[Workspace.Save]`.
	Save = 2,
}

impl LayoutGroup {
	/// The INI section this group's layout persists into. Non-main groups use a
	/// dotted suffix so a hand-editor can tell them apart from `[Workspace]`.
	pub fn ini_section(self) -> &'static str {
		match self {
			LayoutGroup::Main => "Workspace",
			LayoutGroup::Pass => "Workspace.Pass",
			LayoutGroup::Save => "Workspace.Save",
		}
	}
}

/// The editor's dockable workspace: the toolkit docking model (via [`Deref`])
/// plus the editor's steel chrome and INI persistence.
pub struct Workspace(ws::Workspace);

impl std::ops::Deref for Workspace {
	type Target = ws::Workspace;
	fn deref(&self) -> &ws::Workspace {
		&self.0
	}
}

impl std::ops::DerefMut for Workspace {
	fn deref_mut(&mut self) -> &mut ws::Workspace {
		&mut self.0
	}
}

impl Workspace {
	/// Console verb `window ID [on|off]`: sets/toggles visibility, formatted for
	/// the console (the toolkit API is typed; the strings live host-side).
	pub fn show_cmd(&mut self, id: &str, on: Option<bool>) -> Result<String, String> {
		match self.0.show(id, on) {
			Ok(visible) => Ok(format!("window {id}: {}", if visible { "shown" } else { "hidden" })),
			Err(e) => Err(describe(e)),
		}
	}

	/// Console verb `dock ID PLACE [x y]`: parses the place word, docks/floats,
	/// and formats the console line.
	pub fn dock_cmd(&mut self, id: &str, place: &str, at: Option<(f32, f32)>) -> Result<String, String> {
		let place = match place {
			"left" => Place::Docked(LEFT),
			"right" => Place::Docked(RIGHT),
			"top" => Place::Docked(TOP),
			"bottom" => Place::Docked(BOTTOM),
			"float" => {
				let (x, y) = at.unwrap_or((80.0, 80.0));
				Place::Floating(x, y)
			}
			other => return Err(format!("dock: bad place '{other}' (left|right|top|bottom|float)")),
		};
		match self.0.dock_to(id, place) {
			Ok(()) => Ok(format!("window {id}: {place:?}")),
			Err(e) => Err(describe(e)),
		}
	}
}

/// The console text for a toolkit workspace error (same wording the model
/// produced before the API went typed, so script/console output is stable).
fn describe(e: ws::WorkspaceError) -> String {
	match e {
		ws::WorkspaceError::UnknownPanel { id, known } => {
			format!("unknown window '{id}' (have: {})", known.join(" "))
		}
	}
}

impl Default for Workspace {
	fn default() -> Self {
		let dock = |side| Place::Docked(side);
		let inner = ws::Workspace::new()
			.panel(
				PanelSpec::new("minimap", "Minimap")
					.hint("the whole map at a glance - click or drag to jump the view")
					.place(dock(LEFT))
					.size(260.0, 220.0)
					.extent(220.0)
					.bounds((150.0, 150.0), (480.0, 480.0)),
			)
			.panel(
				PanelSpec::new("tiles", "Tile Explorer")
					.hint("every tile in the open tilesets - pick one to paint with")
					.place(dock(RIGHT))
					.size(300.0, 320.0)
					.extent(320.0)
					.bounds((170.0, 140.0), (560.0, 900.0)),
			)
			.panel(
				PanelSpec::new("palette", "Color Palette")
					.hint("this map's colour palette - edit, save, import and export it")
					.place(dock(RIGHT))
					.size(300.0, 220.0)
					.extent(220.0)
					// Max width = 8 max-size swatches + gaps + padding + scrollbar.
					.bounds((180.0, 170.0), (251.0, 640.0)),
			)
			.panel(
				// Hidden by default - a debugging aid; `prev` points at a real
				// dock so `window wrlpalette on` restores somewhere sensible.
				PanelSpec::new("wrlpalette", "WRL Internal Palette")
					.hint("the opened WRL's palette as stored in the file")
					.place(Place::Hidden)
					.prev(dock(RIGHT))
					.size(300.0, 220.0)
					.extent(220.0)
					.bounds((180.0, 170.0), (251.0, 640.0)),
			)
			.panel(
				PanelSpec::new("toolbox", "Toolbox")
					.hint("brushes, shapes and the terrain tools")
					.place(dock(BOTTOM))
					.size(360.0, 160.0)
					.extent(360.0)
					// Height intentionally unbounded for now - the toolbox scrolls.
					// The floor fits the widest block (the tile preview) — the
					// icon-grid keys pack far narrower than the old text keys did.
					.bounds((150.0, 120.0), (1200.0, 4096.0)),
			)
			.panel(
				// Hidden by default - needs MaxPath/MAX.RES.
				PanelSpec::new("units", "Units")
					.hint("unit previews for palette tuning")
					.place(Place::Hidden)
					.prev(dock(RIGHT))
					.size(300.0, 320.0)
					.extent(320.0)
					.bounds((170.0, 140.0), (560.0, 900.0)),
			)
			.panel(
				// Hidden by default - Windows menu / `window templates` shows it.
				PanelSpec::new("templates", "Templates")
					.hint("select tiles, save them, stamp them anywhere")
					.place(Place::Hidden)
					.prev(dock(RIGHT))
					.size(300.0, 320.0)
					.extent(320.0)
					.bounds((170.0, 140.0), (560.0, 900.0)),
			)
			.panel(
				// Hidden by default - Windows menu / `window scenery` shows it.
				PanelSpec::new("scenery", "Scenery")
					.hint("drop trees, mountains and cliffs anywhere - any pixel, no grid")
					.place(Place::Hidden)
					.prev(dock(RIGHT))
					.size(300.0, 320.0)
					.extent(320.0)
					.bounds((170.0, 140.0), (560.0, 900.0)),
			)
			.panel(
				// Hidden by default - the save editor's object tools (Windows menu
				// / `window savetools`). Docks bottom like the terrain toolbox.
				PanelSpec::new("savetools", "Save Toolbox")
					.hint("place / paint / move / delete a save's objects")
					.place(Place::Hidden)
					.prev(dock(BOTTOM))
					.size(360.0, 130.0)
					.extent(300.0)
					// The floor fits the widest block (the five amount presets).
					.bounds((160.0, 100.0), (1200.0, 4096.0)),
			)
			.panel(
				// Hidden by default - the pass editors' swatches + cell tally
				// (Windows menu / `window passtools`). Entering either pass mode shows
				// it, and that mode's own layout group remembers where it was put.
				// Docks bottom like the other two toolboxes.
				PanelSpec::new("passtools", "Pass Types Palette")
					.hint("pick a pass type to paint, and see how the map tallies")
					.place(Place::Hidden)
					.prev(dock(BOTTOM))
					.size(360.0, 160.0)
					.extent(300.0)
					// The floor fits the widest block (the cell tally's columns).
					.bounds((170.0, 100.0), (1200.0, 4096.0)),
			)
			.panel(
				// Hidden by default - the save editor's selected-object inspector
				// (Windows menu / `window unitprops`). Opens FLOATING (not docked):
				// its 380px + 32px connector grid clips when squeezed into a full
				// right column beside Tile Explorer + Color Palette, and a float is
				// how the user actually uses it. Clamped on-screen by clamp_floating.
				PanelSpec::new("unitprops", "Unit Properties")
					.hint("inspect / edit the selected object's properties")
					.place(Place::Hidden)
					.prev(Place::Floating(880.0, 70.0))
					// Tall enough for the appended max-values section (S4.5) on a
					// typical unit; buildings + advanced mode overflow → resize (up to
					// the 900px bound) or scroll the float.
					.size(280.0, 500.0)
					.extent(500.0)
					.bounds((200.0, 150.0), (480.0, 900.0)),
			);
		Self(inner)
	}
}

impl Workspace {
	/// Pointer press, mapping the model outcome to the editor's `&'static str`
	/// [`Press`] (the shell routes body interaction on the static id).
	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		match self.0.on_press(x, y, w, h) {
			ws::Press::None => Press::None,
			ws::Press::Chrome => Press::Chrome,
			ws::Press::Body { id, body } => Press::Body { id: static_id(&id), body },
		}
	}

	/// The topmost panel under the cursor (static id + body rect) - wheel routing.
	pub fn body_at(&self, x: f32, y: f32, w: f32, h: f32) -> Option<(&'static str, Rect)> {
		self.0.body_at(x, y, w, h).map(|(id, r)| (static_id(id), r))
	}

	/// Reset the whole layout to defaults (Windows ▸ Reset Dialogs), keeping the
	/// reserved top strip (the menu/tab bar height).
	pub fn reset(&mut self) {
		let top = self.0.top;
		*self = Self::default();
		self.0.top = top;
	}

	/// The steel sampling for panel `i` at `r`: floating panels anchor a stable
	/// crop to themselves (no swimming as they move); docked panels share the
	/// stretched viewport sheet.
	pub fn steel_map(&self, i: usize, r: Rect) -> SteelMap {
		if self.0.is_floating(i) { SteelMap::anchored(r) } else { SteelMap::Stretch }
	}

	/// Frame chrome below the panels: dock-edge resizers + splitters.
	pub fn draw_background(&self, w: f32, h: f32) -> DrawList {
		let mut dl = DrawList::new();
		let layout = self.0.layout(w, h);
		let c = rgba(theme::SPLITTER);
		for r in layout.edges.iter().flatten() {
			dl.fill_rect(*r, c);
		}
		for (_, _, r) in &layout.splitters {
			dl.fill_rect(*r, c);
		}
		dl
	}

	/// Peeked empty docks: drop-target highlights, drawn on the map below the windows.
	pub fn draw_peeks(&self, w: f32, h: f32) -> DrawList {
		let mut dl = DrawList::new();
		let c = rgba(theme::DOCK_PEEK);
		for r in self.0.peeked_docks(w, h).into_iter().flatten() {
			dl.fill_rect(r, c);
		}
		dl
	}

	/// One panel's chrome (+ its placeholder hint while `show_hint`), drawn through
	/// the steel `skin`. The caller sets the panel's steel mapping on `skin` first
	/// (`MenuChrome::prepare_panel` with [`steel_map`](Self::steel_map)), so the
	/// chrome grain matches the panel's native content - one coherent surface for
	/// both docked (stretched plate) and floating (anchored crop).
	pub fn draw_panel(&self, skin: &SteelTheme, fonts: &Fonts, i: usize, r: Rect, show_hint: bool) -> DrawList {
		let p = &self.0.panels[i];
		let mut dl = DrawList::new();
		let dragging = self.0.is_dragging(i);
		// A thin 2-px border ring that also margins the content.
		let frame = self.0.frame_of(i);

		// Body fill, titlebar band, 1px recessed seam, raised bevel ring - the
		// same composition as `ui::panel`, through the steel theme.
		skin.material(&mut dl, r, theme::PANEL);
		let bar = ui::titlebar_band(r, frame);
		// The shared rusted titlebar (brighter rust while dragged) + a 1px rust seam.
		skin.material(&mut dl, bar, if dragging { theme::RUST_TITLE_DRAG } else { theme::RUST_TITLE });
		skin.material(&mut dl, WRect::new(bar.x, bar.y + ui::TITLEBAR_H - 1.0, bar.w, 1.0), theme::RUST_EDGE);
		skin.bevel(&mut dl, r, frame, true);

		// Title (amber, raised, 12px left pad); the close "x" washed when hot.
		label(
			&mut dl,
			skin,
			fonts,
			ui::titlebar_rect(r, frame),
			ui::TITLE_PAD,
			&p.title,
			theme::TITLE_INK,
			Emboss::Raised,
		);
		// The close `x` lights off the pointer the **model** tracks (U6.2): the
		// same rect its press resolves, so the affordance and the click cannot
		// disagree. There is no held state to draw - a close acts on the press.
		let close = ui::close_rect(r, frame);
		if self.0.close_hovered(i, r) {
			dl.fill_rect(close, rgba(theme::HOVER));
		}
		label(&mut dl, skin, fonts, close, 6.0, "x", theme::CLOSE_INK, Emboss::Raised);

		if show_hint {
			let body = ui::body_rect(r, frame);
			hint(&mut dl, skin, fonts, crate::ui::strip_top(body, 24.0), 8.0, &p.hint, theme::INK_DIM);
		}
		if matches!(p.place, Place::Floating(..)) {
			// A dark grip triangle in the bottom-right corner (the hit area is the
			// corner square - see the model's `on_press`). No triangle primitive in
			// the DrawList, so it's stepped 1px rows like the menu's submenu arrow.
			let (x1, y1) = (r.x + r.w, r.y + r.h);
			let c = rgba(theme::RESIZE_HANDLE);
			for k in 0..HANDLE as i32 {
				let row_w = HANDLE - k as f32;
				dl.fill_rect(WRect::new(x1 - row_w, y1 - 1.0 - k as f32, row_w, 1.0), c);
			}
		}
		dl
	}

	// ----- layout persistence -------------------------------------

	/// Serialize a layout snapshot as a `[Workspace*]` section of `mme.ini`:
	/// `Docks = left right top bottom`, plus one key per panel -
	/// `Place X Y W H Extent` (`X`/`Y` only meaningful for `Float`). Taking a
	/// snapshot (rather than `&self`) lets the non-active layout groups - whose
	/// layouts live in stored snapshots, not the live workspace - persist
	/// through the same format; serialize the live workspace with
	/// `layout_to_ini(&ws.save_layout())`.
	pub fn layout_to_ini(saved: &WorkspaceLayout) -> INISection {
		const NAMES: [&str; 4] = ["Left", "Right", "Top", "Bottom"];
		// Sizes persist as whole pixels (rounded) so the file stays clean and a
		// restored layout lands on exact pixel boundaries.
		let px = |v: f32| v.round() as i32;
		let mut section = INISection::new();
		let d = saved.dock_size;
		let _ = section.set_entry("Docks".to_string(), format!("{} {} {} {}", px(d[0]), px(d[1]), px(d[2]), px(d[3])));
		for p in &saved.panels {
			let (place, x, y) = match p.place {
				Place::Docked(side) => (NAMES[side.min(3)], 0.0, 0.0),
				Place::Floating(fx, fy) => ("Float", fx, fy),
				Place::Hidden => ("Hidden", 0.0, 0.0),
			};
			let _ = section.set_entry(
				camel(&p.id),
				format!("{place} {} {} {} {} {}", px(x), px(y), px(p.w), px(p.h), px(p.extent)),
			);
		}
		section
	}

	/// Apply a `[Workspace]` section: set dock sizes + each known panel's
	/// place/size, then clamp into the `w`×`h` screen. Unknown keys and malformed
	/// fields are skipped (keeps defaults), so it's forward-compatible with
	/// hand-edited files.
	pub fn apply_ini(&mut self, section: &INISection, w: f32, h: f32) {
		let mut dock_size = self.0.dock_size();
		if let Some(docks) = section.get_entry::<String>("Docks") {
			for (i, text) in docks.split_whitespace().take(4).enumerate() {
				if let Ok(n) = text.parse::<f32>() {
					dock_size[i] = n.round(); // whole pixels, even from a hand-edited file
				}
			}
		}
		let mut panels = Vec::new();
		for (key, value) in section {
			if key == "Docks" {
				continue;
			}
			let id = key.to_lowercase();
			let Some(idx) = self.0.find(&id) else {
				continue;
			};
			// Missing size fields keep the panel's current value.
			let (cw, ch, ce) = {
				let p = &self.0.panels[idx];
				(p.w, p.h, p.extent)
			};
			let text = value.to_string();
			let mut parts = text.split_whitespace();
			let Some(place_word) = parts.next() else {
				continue;
			};
			// Round loaded sizes/positions to whole pixels.
			let mut num = || parts.next().and_then(|t| t.parse::<f32>().ok()).map(f32::round);
			let (x, y, wv, hv, ev) = (num(), num(), num(), num(), num());
			let place = match place_word.to_ascii_lowercase().as_str() {
				"left" => Place::Docked(LEFT),
				"right" => Place::Docked(RIGHT),
				"top" => Place::Docked(TOP),
				"bottom" => Place::Docked(BOTTOM),
				"float" => Place::Floating(x.unwrap_or(80.0), y.unwrap_or(80.0)),
				"hidden" => Place::Hidden,
				_ => self.0.panels[idx].place,
			};
			panels.push(ws::PanelLayout {
				id,
				place,
				w: wv.unwrap_or(cw),
				h: hv.unwrap_or(ch),
				extent: ev.unwrap_or(ce),
			});
		}
		// `load_layout` sets dock sizes + each known panel's place/size, preserves
		// a hidden panel's `prev`, then clamps size + position into the screen.
		self.0.load_layout(&WorkspaceLayout { dock_size, panels }, w, h);
	}

	/// Parse a `[Workspace*]` section into a standalone layout snapshot, filling
	/// in a fresh default panel roster for any panel the section omits. Lets a
	/// non-active mode's saved layout load at startup without disturbing the
	/// live workspace.
	pub fn layout_from_ini(section: &INISection, w: f32, h: f32) -> WorkspaceLayout {
		let mut tmp = Workspace::default();
		tmp.apply_ini(section, w, h);
		tmp.0.save_layout()
	}
}

/// Panel ids are single lowercase words (`"palette"`); their `[Workspace]` keys
/// follow the CamelCase INI convention (`Palette`). `wrlpalette` is a compound
/// (WRL + palette) with no separator to split on, so its key is spelled out.
/// (`apply_ini` matches keys case-insensitively, so old `Wrlpalette` files load.)
fn camel(id: &str) -> String {
	if id == "wrlpalette" {
		return "WrlPalette".to_string();
	}
	let mut chars = id.chars();
	match chars.next() {
		Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
		None => String::new(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const W: f32 = 1280.0;
	const H: f32 = 800.0;

	fn ws() -> Workspace {
		Workspace::default()
	}

	#[test]
	fn ini_round_trip_preserves_layout() {
		// Serialize a customized layout, re-apply onto a fresh default, and every
		// panel's place + the dock sizes come back identical.
		let mut a = ws();
		a.dock_to("palette", Place::Docked(LEFT)).unwrap();
		a.dock_to("minimap", Place::Floating(120.0, 90.0)).unwrap();
		a.show("toolbox", Some(false)).unwrap();
		a.set_dock_size([333.0, 280.0, 130.0, 150.0]);
		let section = Workspace::layout_to_ini(&a.save_layout());
		let mut b = ws();
		b.apply_ini(&section, W, H);
		for id in ["palette", "minimap", "toolbox", "tiles"] {
			let (pa, pb) = (a.find(id).unwrap(), b.find(id).unwrap());
			assert_eq!(a.panels[pa].place, b.panels[pb].place, "{id} place round-trips");
		}
		assert_eq!(a.dock_size(), b.dock_size(), "dock sizes round-trip");
	}

	#[test]
	fn reset_restores_defaults_keeping_top() {
		let mut w = ws();
		w.top = 24.0;
		w.dock_to("palette", Place::Floating(10.0, 10.0)).unwrap();
		w.show("toolbox", Some(false)).unwrap();
		w.reset();
		let def = ws();
		for id in ["palette", "toolbox", "minimap", "tiles"] {
			assert_eq!(w.panels[w.find(id).unwrap()].place, def.panels[def.find(id).unwrap()].place, "{id} reset");
		}
		assert_eq!(w.top, 24.0, "top strip preserved across reset");
	}

	#[test]
	fn on_press_maps_body_to_static_id() {
		let mut w = ws();
		let l = w.layout(W, H);
		let ti = w.find("tiles").unwrap();
		let r = l.panels.iter().find(|(i, _)| *i == ti).unwrap().1;
		match w.on_press(r.x + 50.0, r.y + 100.0, W, H) {
			Press::Body { id, body } => {
				assert_eq!(id, "tiles");
				assert_eq!(body, w.body_of(ti, r));
			}
			other => panic!("expected Body, got {other:?}"),
		}
	}

	/// **Every panel the workspace holds must be in [`PANEL_IDS`].**
	///
	/// `static_id` maps an unknown id to `""`, and `""` is the id no `panel_host`
	/// arm answers to - so a panel left out of the table is laid out and drawn
	/// exactly like the others while every press, wheel notch and paging key
	/// routed by id is dropped on the floor. It looks alive and is completely
	/// dead, with nothing in the build or the goldens to say so. That is what the
	/// Scenery panel shipped as, so the table gets a test rather than a comment.
	#[test]
	fn every_panel_has_a_static_id() {
		let w = ws();
		for p in &w.panels {
			assert_eq!(static_id(&p.id), p.id, "panel '{}' is missing from PANEL_IDS - its input is dead", p.id);
		}
		assert_eq!(w.panels.len(), PANEL_IDS.len(), "and PANEL_IDS names no panel that does not exist");
		assert_eq!(static_id("nope"), "", "an id the workspace never had still resolves to nothing");
	}

	/// The arranged rect of panel `id` in the default layout.
	fn panel_rect(w: &Workspace, id: &str) -> Rect {
		let i = w.find(id).unwrap();
		w.layout(W, H).panels.iter().find(|(p, _)| *p == i).map(|(_, r)| *r).unwrap()
	}

	/// A headless steel theme + fonts for inspecting produced `DrawList`s (a
	/// dummy steel texture id - the list only records commands).
	fn skin() -> (SteelTheme, Fonts) {
		let mut fonts = Fonts::new();
		let font = fonts.add(include_bytes!("../assets/MAX_Redesign_Square.ttf").to_vec()).unwrap();
		let em = fonts.get(font).units_per_em();
		(SteelTheme::new(font, wgpu_ui::TextureId::ATLAS, em), fonts)
	}

	/// The console verbs format success and failure the console way: `window`
	/// reports shown/hidden (toggling without an explicit state), `dock` parses
	/// its place word, and both spell out an unknown id with the known list.
	#[test]
	fn console_verbs_format_success_and_errors() {
		let mut w = ws();
		assert_eq!(w.show_cmd("palette", Some(false)).unwrap(), "window palette: hidden");
		assert_eq!(w.show_cmd("palette", None).unwrap(), "window palette: shown", "None toggles back on");
		let err = w.show_cmd("nope", None).unwrap_err();
		assert!(err.starts_with("unknown window 'nope' (have: "), "unknown id names itself: {err}");
		assert!(err.contains("palette") && err.contains("toolbox"), "the known list is spelled out: {err}");

		assert_eq!(w.dock_cmd("palette", "float", Some((50.0, 60.0))).unwrap(), "window palette: Floating(50.0, 60.0)");
		assert_eq!(w.dock_cmd("palette", "left", None).unwrap(), format!("window palette: {:?}", Place::Docked(LEFT)));
		assert_eq!(
			w.dock_cmd("palette", "diagonal", None).unwrap_err(),
			"dock: bad place 'diagonal' (left|right|top|bottom|float)"
		);
		let err = w.dock_cmd("ghost", "top", None).unwrap_err();
		assert!(err.starts_with("unknown window 'ghost'"), "dock reports unknown ids too: {err}");
	}

	/// Press routing beyond the body case: empty map space is `None`, a
	/// titlebar is `Chrome`; `body_at` resolves the panel under a point to its
	/// static id (and nothing over the map).
	#[test]
	fn press_and_body_at_route_chrome_and_map_space() {
		let mut w = ws();
		// The map center: inside no panel → None (and no drag starts).
		assert_eq!(w.on_press(W / 2.0, H / 2.0, W, H), Press::None);
		assert!(w.body_at(W / 2.0, H / 2.0, W, H).is_none(), "map space has no panel body");
		// A titlebar press is chrome (handled inside the model).
		let r = panel_rect(&w, "minimap");
		let mi = w.find("minimap").unwrap();
		let tb = w.titlebar_of(mi, r);
		assert_eq!(w.on_press(tb.x + 30.0, tb.y + tb.h / 2.0, W, H), Press::Chrome);
		w.on_release(tb.x + 30.0, tb.y + tb.h / 2.0, W, H);
		// body_at maps back to the editor's static id + body rect.
		let (id, body) = w.body_at(r.x + 50.0, r.y + 100.0, W, H).expect("minimap under the point");
		assert_eq!(id, "minimap");
		assert_eq!(body, w.body_of(mi, r));
	}

	/// While a titlebar drag hovers near an empty dock edge, `draw_peeks`
	/// paints that dock's drop-target highlight; idle, it paints nothing.
	#[test]
	fn draw_peeks_highlights_an_empty_dock_during_a_drag() {
		let mut w = ws();
		assert!(w.draw_peeks(W, H).cmds.is_empty(), "no peeks while idle");
		// Drag the minimap by its titlebar toward the (empty) top dock.
		let r = panel_rect(&w, "minimap");
		let mi = w.find("minimap").unwrap();
		let tb = w.titlebar_of(mi, r);
		assert_eq!(w.on_press(tb.x + 30.0, tb.y + tb.h / 2.0, W, H), Press::Chrome);
		w.on_move(W / 2.0, 2.0, W, H); // well past the drag threshold, at the top edge
		let dl = w.draw_peeks(W, H);
		assert!(!dl.cmds.is_empty(), "the empty top dock peeks as a drop target");
		w.on_release(W / 2.0, 2.0, W, H);
	}

	/// The wrapped placeholder hint: words flow to a new line when the next
	/// one won't fit the panel width, so a narrow rect yields multiple
	/// baselines spanning at least one line height.
	#[test]
	fn hint_wraps_words_to_the_rect_width() {
		let (skin, fonts) = skin();
		let px = skin.font_px(TextRole::Small);
		let baselines = |w: f32, text: &str| -> Vec<f32> {
			let mut dl = DrawList::new();
			hint(&mut dl, &skin, &fonts, Rect::new(0.0, 0.0, w, 300.0), 8.0, text, theme::INK_DIM);
			let mut ys: Vec<f32> = dl
				.cmds
				.iter()
				.filter_map(|c| match c {
					wgpu_ui::DrawCmd::Glyph { pen, .. } => Some(pen.y),
					_ => None,
				})
				.collect();
			ys.sort_by(f32::total_cmp);
			ys.dedup();
			ys
		};
		let text = "interactive minimap lands with UI-11";
		// Wide: everything on one baseline cluster (emboss layers sit within 2px).
		let wide = baselines(600.0, text);
		assert!(wide.last().unwrap() - wide.first().unwrap() < px + 4.0, "one line when wide: {wide:?}");
		// Narrow: the words wrap - baselines span at least one line height.
		let narrow = baselines(120.0, text);
		assert!(narrow.last().unwrap() - narrow.first().unwrap() >= px + 4.0, "wraps when narrow: {narrow:?}");
	}

	/// Panel chrome: the hint only draws when asked, the close `x` washes under
	/// the pointer the model tracks (and goes dark when it leaves - the rule a
	/// cascade opening over the frame relies on), and a floating panel gets its
	/// stepped resize-grip rows in the bottom-right corner.
	#[test]
	fn draw_panel_paints_hint_close_wash_and_float_grip() {
		let (skin, fonts) = skin();
		let mut w = ws();
		let r = Rect::new(100.0, 100.0, 160.0, 160.0);
		let mi = w.find("minimap").unwrap();
		let frame = w.frame_of(mi);

		let plain = w.draw_panel(&skin, &fonts, mi, r, false);
		let hinted = w.draw_panel(&skin, &fonts, mi, r, true);
		assert!(hinted.cmds.len() > plain.cmds.len(), "show_hint adds the engraved placeholder text");

		// The close `x` washes while the pointer is on it.
		let close = ui::close_rect(r, frame);
		let washed = |dl: &DrawList| {
			dl.cmds.iter().any(|c| match c {
				wgpu_ui::DrawCmd::Solid { rect, color } => *rect == close && *color == rgba(theme::HOVER),
				_ => false,
			})
		};
		assert!(!washed(&plain), "no pointer, no wash");
		w.0.on_move(close.x + 2.0, close.y + 2.0, 1280.0, 800.0);
		assert!(washed(&w.draw_panel(&skin, &fonts, mi, r, false)), "the x washes under the pointer");
		// A menu opening over the frame moves no pointer, so the leave is the only
		// thing that can put it out (U6.2).
		w.0.on_pointer_left();
		assert!(!washed(&w.draw_panel(&skin, &fonts, mi, r, false)), "and the leave puts it out");

		// Floating: the same panel gains the stepped grip rows (docked has none).
		w.dock_to("minimap", Place::Floating(100.0, 100.0)).unwrap();
		let mi = w.find("minimap").unwrap();
		let floating = w.draw_panel(&skin, &fonts, mi, r, false);
		let grip = |dl: &DrawList| {
			dl.cmds
				.iter()
				.filter(|c| match c {
					wgpu_ui::DrawCmd::Solid { rect, color } => {
						rect.h == 1.0 && rect.y > r.y + r.h - 15.0 && *color == rgba(theme::RESIZE_HANDLE)
					}
					_ => false,
				})
				.count()
		};
		assert_eq!(grip(&floating), 14, "one 1px row per grip step");
		assert_eq!(grip(&plain), 0, "docked panels draw no grip");
	}

	/// `apply_ini` is forgiving with hand-edited files: non-numeric dock sizes
	/// are skipped (the rest still apply), unknown panel keys are ignored, an
	/// empty value keeps the panel untouched, and an unknown place word keeps
	/// the panel's current place while its sizes still load.
	#[test]
	fn apply_ini_skips_malformed_fields_and_unknown_keys() {
		let mut w = ws();
		let default_docks = w.dock_size();
		let palette_place = w.panels[w.find("palette").unwrap()].place;
		let tiles_place = w.panels[w.find("tiles").unwrap()].place;

		let mut section = INISection::new();
		let _ = section.set_entry("Docks".to_string(), "abc 280".to_string());
		let _ = section.set_entry("Bogus".to_string(), "Left 0 0 100 100 100".to_string());
		let _ = section.set_entry("Palette".to_string(), "".to_string());
		let _ = section.set_entry("Tiles".to_string(), "diagonal 0 0 300 320 320".to_string());
		w.apply_ini(&section, W, H);

		assert_eq!(w.dock_size()[0], default_docks[0], "the unparsable dock size keeps its default");
		assert_eq!(w.dock_size()[1], 280.0, "the numeric dock size still applies");
		assert!(w.find("bogus").is_none(), "unknown keys add no panel");
		assert_eq!(w.panels[w.find("palette").unwrap()].place, palette_place, "an empty value keeps the panel");
		assert_eq!(w.panels[w.find("tiles").unwrap()].place, tiles_place, "a bad place word keeps the place");
	}

	/// Panel-id → INI-key casing: plain ids capitalize their first letter, the
	/// compound `wrlpalette` is spelled `WrlPalette`, and the empty id (never
	/// produced, but total) maps to the empty key.
	#[test]
	fn camel_cases_panel_ids_for_ini_keys() {
		assert_eq!(camel("palette"), "Palette");
		assert_eq!(camel("wrlpalette"), "WrlPalette");
		assert_eq!(camel(""), "");
	}
}
