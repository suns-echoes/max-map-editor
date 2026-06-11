//! Dockable workspace: four edge docks around the map view
//! plus a floating layer — in-app windows, not OS windows. Behavior ports the
//! finished Tauri prototype (workspace.component.ts, archived on the
//! `bak/rust-1` branch under `_old-app/front/src/ui/main-window/workspace/`):
//! drag a titlebar >3 px to undock into floating; dragging near an edge peeks
//! that dock open as a drop target; drop inside it to dock at the
//! midpoint-based insert position; splitters between dock windows, resizer
//! strips between each dock and the center; empty docks auto-hide; the close
//! glyph hides a panel (the `window` command re-shows it).
//!
//! Pure layout/state logic — headless-testable; rendering happens in [`draw`]
//! from the computed [`Layout`], so the rects you click are the rects drawn.

use ini::INISection;

use crate::theme;
use crate::ui::{self, Rect, SteelMap, UiQuads};

pub const LEFT: usize = 0;
pub const RIGHT: usize = 1;
pub const TOP: usize = 2;
pub const BOTTOM: usize = 3;

const SPLIT: f32 = 6.0; // splitter / dock-edge resizer thickness
const PEEK_DIST: f32 = 32.0; // edge proximity that peeks a dock during a drag
const DRAG_START: f32 = 3.0; // titlebar movement before a drag undocks
const MIN_PANEL: f32 = 50.0; // min extent of a docked panel
const MIN_DOCK: f32 = 120.0;
const MAX_DOCK: f32 = 520.0;
const HANDLE: f32 = 14.0; // floating resize-handle square
const FRAME_FLOAT: f32 = 2.0; // floating-window border ring (also content margin)
const FRAME_DOCK: f32 = 2.0; // docked-panel border ring
const MIN_VISIBLE: f32 = 32.0; // a floating window must keep this much on-screen

/// A panel's min size along a dock's stacking axis (height for L/R, width T/B).
fn along_min(min: (f32, f32), side: usize) -> f32 {
	if side == LEFT || side == RIGHT { min.1 } else { min.0 }
}

/// A panel's min size across a dock's axis (width for L/R, height for T/B).
fn cross_min(min: (f32, f32), side: usize) -> f32 {
	if side == LEFT || side == RIGHT { min.0 } else { min.1 }
}

/// Where a panel lives. `Floating` holds its top-left; size is per-panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
	Docked(usize),
	Floating(f32, f32),
	Hidden,
}

pub struct Panel {
	pub id: &'static str,
	pub title: &'static str,
	/// Body placeholder until the panel grows real content.
	pub hint: &'static str,
	pub place: Place,
	/// Where `window ID on` restores a hidden panel to.
	prev: Place,
	/// Floating size.
	pub w: f32,
	pub h: f32,
	/// Docked extent along the dock's stacking axis.
	pub extent: f32,
	/// Sensible size bounds (w, h) so content can't overflow a too-small window
	/// nor a window grow absurdly large.
	pub min: (f32, f32),
	pub max: (f32, f32),
}

#[derive(Clone, Copy)]
enum Drag {
	None,
	/// Titlebar drag: grab offset within the panel, undocked yet?
	Move {
		panel: usize,
		grab: (f32, f32),
		start: (f32, f32),
		moved: bool,
	},
	DockEdge {
		side: usize,
	},
	/// Resize the `nth` docked panel of `side` via the splitter below/right.
	Splitter {
		side: usize,
		nth: usize,
	},
	FloatResize {
		panel: usize,
	},
}

pub struct Workspace {
	pub panels: Vec<Panel>,
	/// Reserved strip above the docks (the main menu bar). 0 in unit tests;
	/// the editor sets it to `menu::BAR_H`.
	pub top: f32,
	dock_size: [f32; 4],
	drag: Drag,
	cursor: (f32, f32),
}

/// What a primary-button press hit.
#[derive(Debug, Clone, PartialEq)]
pub enum Press {
	None,
	/// Titlebar / close / splitter / resizer — handled internally.
	Chrome,
	/// A panel body — the shell routes content interaction (picker, …).
	Body {
		id: &'static str,
		body: Rect,
	},
}

/// One frame's computed geometry (also the hit-test source).
pub struct Layout {
	pub center: Rect,
	/// Visible dock areas (including drag-peeked empty ones).
	pub docks: [Option<Rect>; 4],
	/// Docked panels first (vec order), floating after — also draw order.
	pub panels: Vec<(usize, Rect)>,
	/// `(side, nth docked panel it resizes, rect)`.
	pub splitters: Vec<(usize, usize, Rect)>,
	/// Dock-edge resizer strips.
	pub edges: [Option<Rect>; 4],
}

impl Default for Workspace {
	fn default() -> Self {
		#[allow(clippy::too_many_arguments)]
		let panel = |id, title, hint, place, w, h, extent, min, max| Panel {
			id,
			title,
			hint,
			place,
			prev: place,
			w,
			h,
			extent,
			min,
			max,
		};
		Self {
			panels: vec![
				panel(
					"minimap",
					"Minimap",
					"interactive minimap lands with UI-11",
					Place::Docked(LEFT),
					260.0,
					220.0,
					220.0,
					(150.0, 150.0),
					(480.0, 480.0),
				),
				panel(
					"tiles",
					"Tile Explorer",
					"tile picker lands with UI-5",
					Place::Docked(RIGHT),
					300.0,
					320.0,
					320.0,
					(170.0, 140.0),
					(560.0, 900.0),
				),
				panel(
					"palette",
					"Color Palette",
					"palette grid lands with UI-12",
					Place::Docked(RIGHT),
					300.0,
					220.0,
					220.0,
					(180.0, 170.0),
					// Max width = 8 max-size swatches + gaps + padding + scrollbar.
					(251.0, 640.0),
				),
				panel(
					"toolbox",
					"Toolbox",
					"tool buttons land with UI-13",
					Place::Docked(BOTTOM),
					360.0,
					160.0,
					360.0,
					(300.0, 140.0),
					// Height intentionally unbounded for now — the toolbox scrolls.
					(1200.0, 4096.0),
				),
				// Hidden by default — needs MaxPath/MAX.RES; Windows menu shows
				// it. `prev` points at a real dock so `window units on` has
				// somewhere sensible to restore to.
				Panel {
					prev: Place::Docked(RIGHT),
					..panel(
						"units",
						"Units",
						"unit previews for palette tuning",
						Place::Hidden,
						300.0,
						320.0,
						320.0,
						(170.0, 140.0),
						(560.0, 900.0),
					)
				},
				// Hidden by default — Windows menu / `window templates` shows it.
				Panel {
					prev: Place::Docked(RIGHT),
					..panel(
						"templates",
						"Templates",
						"select tiles, save them, stamp them anywhere",
						Place::Hidden,
						300.0,
						320.0,
						320.0,
						(170.0, 140.0),
						(560.0, 900.0),
					)
				},
			],
			top: 0.0,
			dock_size: [240.0, 280.0, 130.0, 150.0],
			drag: Drag::None,
			cursor: (0.0, 0.0),
		}
	}
}

impl Workspace {
	pub fn find(&self, id: &str) -> Option<usize> {
		self.panels.iter().position(|p| p.id == id)
	}

	/// Is panel `id` currently on screen (not hidden)? Drives the Windows menu
	/// checkboxes.
	pub fn is_visible(&self, id: &str) -> bool {
		self.find(id).is_some_and(|i| self.panels[i].place != Place::Hidden)
	}

	/// Show/hide a panel (`None` toggles). `Ok(line)` describes the result.
	pub fn show(&mut self, id: &str, on: Option<bool>) -> Result<String, String> {
		let Some(i) = self.find(id) else {
			return Err(format!("unknown window '{id}' (have: {})", self.ids().join(" ")));
		};
		let p = &mut self.panels[i];
		let visible = p.place != Place::Hidden;
		let want = on.unwrap_or(!visible);
		if want && !visible {
			p.place = p.prev;
		} else if !want && visible {
			p.prev = p.place;
			p.place = Place::Hidden;
		}
		Ok(format!("window {id}: {}", if want { "shown" } else { "hidden" }))
	}

	/// Dock a panel to a side, or float it (optionally at a position).
	pub fn dock_to(&mut self, id: &str, place: &str, at: Option<(f32, f32)>) -> Result<String, String> {
		let Some(i) = self.find(id) else {
			return Err(format!("unknown window '{id}' (have: {})", self.ids().join(" ")));
		};
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
		self.panels[i].place = place;
		self.panels[i].prev = place;
		Ok(format!("window {id}: {place:?}"))
	}

	fn ids(&self) -> Vec<&'static str> {
		self.panels.iter().map(|p| p.id).collect()
	}

	/// Panels docked to `side`, in stacking order.
	fn docked(&self, side: usize) -> Vec<usize> {
		self.panels.iter().enumerate().filter(|(_, p)| p.place == Place::Docked(side)).map(|(i, _)| i).collect()
	}

	/// A dock is force-shown while a titlebar drag hovers near its edge.
	fn peek(&self, w: f32, h: f32) -> [bool; 4] {
		let Drag::Move { moved: true, .. } = self.drag else { return [false; 4] };
		let (cx, cy) = self.cursor;
		[cx <= PEEK_DIST, w - cx <= PEEK_DIST, cy <= self.top + PEEK_DIST, h - cy <= PEEK_DIST]
	}

	/// Compute the frame's geometry for a `w`×`h` screen.
	pub fn layout(&self, w: f32, h: f32) -> Layout {
		let peek = self.peek(w, h);
		let occupied: [Vec<usize>; 4] = [self.docked(0), self.docked(1), self.docked(2), self.docked(3)];
		let visible = [
			!occupied[0].is_empty() || peek[0],
			!occupied[1].is_empty() || peek[1],
			!occupied[2].is_empty() || peek[2],
			!occupied[3].is_empty() || peek[3],
		];
		let size = |side: usize| {
			if visible[side] {
				self.dock_size[side].min(match side {
					TOP | BOTTOM => (h - MIN_DOCK).max(MIN_DOCK),
					_ => (w - MIN_DOCK).max(MIN_DOCK),
				})
			} else {
				0.0
			}
		};
		let (lw, rw, th, bh) = (size(LEFT), size(RIGHT), size(TOP), size(BOTTOM));

		let mut docks = [None; 4];
		let mut edges = [None; 4];
		// Top/bottom span the full width; left/right fill the middle band.
		// Everything sits below the reserved `top` strip (the menu bar).
		if visible[TOP] {
			docks[TOP] = Some(Rect::new(0.0, self.top, w, th));
			edges[TOP] = Some(Rect::new(0.0, self.top + th, w, SPLIT));
		}
		if visible[BOTTOM] {
			docks[BOTTOM] = Some(Rect::new(0.0, h - bh, w, bh));
			edges[BOTTOM] = Some(Rect::new(0.0, h - bh - SPLIT, w, SPLIT));
		}
		let y0 = self.top + th + if visible[TOP] { SPLIT } else { 0.0 };
		let y1 = h - bh - if visible[BOTTOM] { SPLIT } else { 0.0 };
		if visible[LEFT] {
			docks[LEFT] = Some(Rect::new(0.0, y0, lw, y1 - y0));
			edges[LEFT] = Some(Rect::new(lw, y0, SPLIT, y1 - y0));
		}
		if visible[RIGHT] {
			docks[RIGHT] = Some(Rect::new(w - rw, y0, rw, y1 - y0));
			edges[RIGHT] = Some(Rect::new(w - rw - SPLIT, y0, SPLIT, y1 - y0));
		}
		let x0 = lw + if visible[LEFT] { SPLIT } else { 0.0 };
		let x1 = w - rw - if visible[RIGHT] { SPLIT } else { 0.0 };
		let center = Rect::new(x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0));

		// Stack each dock's panels: all but the last keep their extent, the
		// last takes the remainder; splitters between.
		let mut panels = Vec::new();
		let mut splitters = Vec::new();
		for side in 0..4 {
			let Some(dock) = docks[side] else { continue };
			let ids = &occupied[side];
			if ids.is_empty() {
				continue;
			}
			let vertical = side == LEFT || side == RIGHT;
			let total = if vertical { dock.h } else { dock.w };
			let gaps = SPLIT * (ids.len() - 1) as f32;
			let mut used = 0.0;
			for (nth, &i) in ids.iter().enumerate() {
				let last = nth == ids.len() - 1;
				let ext =
					if last { (total - gaps - used).max(MIN_PANEL) } else { self.panels[i].extent.max(MIN_PANEL) };
				let r = if vertical {
					Rect::new(dock.x, dock.y + used + SPLIT * nth as f32, dock.w, ext)
				} else {
					Rect::new(dock.x + used + SPLIT * nth as f32, dock.y, ext, dock.h)
				};
				panels.push((i, r));
				if !last {
					let s = if vertical {
						Rect::new(dock.x, r.y + r.h, dock.w, SPLIT)
					} else {
						Rect::new(r.x + r.w, dock.y, SPLIT, dock.h)
					};
					splitters.push((side, nth, s));
				}
				used += ext;
			}
		}
		// Floating panels draw after (= above) docked ones, in vec order.
		for (i, p) in self.panels.iter().enumerate() {
			if let Place::Floating(x, y) = p.place {
				panels.push((i, Rect::new(x, y, p.w, p.h)));
			}
		}

		Layout { center, docks, panels, splitters, edges }
	}

	/// Is the cursor over any workspace chrome (panel, splitter, edge)?
	/// Map input (paint/pan) should be suppressed when this is true.
	pub fn over_ui(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
		let l = self.layout(w, h);
		l.panels.iter().any(|(_, r)| r.contains(x, y))
			|| l.splitters.iter().any(|(_, _, r)| r.contains(x, y))
			|| l.edges.iter().flatten().any(|r| r.contains(x, y))
	}

	/// The topmost panel under the cursor (id + body rect) — wheel routing.
	pub fn body_at(&self, x: f32, y: f32, w: f32, h: f32) -> Option<(&'static str, Rect)> {
		let l = self.layout(w, h);
		l.panels.iter().rev().find(|(_, r)| r.contains(x, y)).map(|&(i, r)| (self.panels[i].id, self.body_of(i, r)))
	}

	/// Pointer press (the paint/primary button).
	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		self.cursor = (x, y);
		let layout = self.layout(w, h);

		// Topmost first: floating panels are at the tail of `layout.panels`.
		for &(i, r) in layout.panels.iter().rev() {
			let frame = self.frame_of(i);
			if ui::close_rect(r, frame).contains(x, y) {
				self.panels[i].prev = self.panels[i].place;
				self.panels[i].place = Place::Hidden;
				return Press::Chrome;
			}
			if ui::titlebar_rect(r, frame).contains(x, y) {
				// `raise` reorders the vec — re-resolve the index by id.
				let id = self.panels[i].id;
				self.raise(i);
				self.drag = Drag::Move {
					panel: self.find(id).unwrap_or(i),
					grab: (x - r.x, y - r.y),
					start: (x, y),
					moved: false,
				};
				return Press::Chrome;
			}
			let floating = matches!(self.panels[i].place, Place::Floating(..));
			let handle = Rect::new(r.x + r.w - HANDLE, r.y + r.h - HANDLE, HANDLE, HANDLE);
			if floating && handle.contains(x, y) {
				let id = self.panels[i].id;
				self.raise(i);
				self.drag = Drag::FloatResize { panel: self.find(id).unwrap_or(i) };
				return Press::Chrome;
			}
			if r.contains(x, y) {
				let id = self.panels[i].id;
				if floating {
					self.raise(i);
				}
				return Press::Body { id, body: ui::body_rect(r, frame) };
			}
		}
		for &(side, nth, r) in &layout.splitters {
			if r.contains(x, y) {
				self.drag = Drag::Splitter { side, nth };
				return Press::Chrome;
			}
		}
		for side in 0..4 {
			if layout.edges[side].is_some_and(|r| r.contains(x, y)) {
				self.drag = Drag::DockEdge { side };
				return Press::Chrome;
			}
		}
		Press::None
	}

	/// `raise` moves a floating panel to the end of the vec (topmost) —
	/// indices in an active drag are resolved by id afterwards.
	fn raise(&mut self, i: usize) {
		if matches!(self.panels[i].place, Place::Floating(..)) && i + 1 != self.panels.len() {
			let p = self.panels.remove(i);
			self.panels.push(p);
		}
	}

	/// Pointer move. Returns true when the workspace wants a redraw.
	pub fn on_move(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
		self.cursor = (x, y);
		match self.drag {
			Drag::None => false,
			Drag::Move { panel, mut grab, start, moved } => {
				let mut i = panel;
				if !moved {
					if (x - start.0).abs() < DRAG_START && (y - start.1).abs() < DRAG_START {
						return false;
					}
					// Undock: become floating at the cursor, keeping the grab
					// point inside the (possibly narrower) floating titlebar —
					// and take the top z-index immediately (`raise` reorders
					// the vec, so re-resolve the dragged index by id).
					grab.0 = grab.0.min(self.panels[i].w - ui::TITLEBAR_H);
					self.panels[i].place = Place::Floating(x - grab.0, y - grab.1);
					let id = self.panels[i].id;
					self.raise(i);
					i = self.find(id).unwrap_or(i);
					self.drag = Drag::Move { panel: i, grab, start, moved: true };
				}
				self.panels[i].place = Place::Floating(x - grab.0, y - grab.1);
				true
			}
			Drag::DockEdge { side } => {
				let v = match side {
					LEFT => x,
					RIGHT => w - x,
					TOP => y - self.top,
					_ => h - y,
				};
				let lo = self.dock_cross_min(side);
				self.dock_size[side] = v.clamp(lo, MAX_DOCK.max(lo));
				true
			}
			Drag::Splitter { side, nth } => {
				let ids = self.docked(side);
				let layout = self.layout(w, h);
				if let Some(&i) = ids.get(nth) {
					let along = if side == LEFT || side == RIGHT { y } else { x };
					let origin = layout
						.panels
						.iter()
						.find(|(p, _)| *p == i)
						.map(|(_, r)| if side == LEFT || side == RIGHT { r.y } else { r.x })
						.unwrap_or(0.0);
					self.panels[i].extent = (along - origin).max(along_min(self.panels[i].min, side));
				}
				true
			}
			Drag::FloatResize { panel } => {
				let i = panel;
				if let Place::Floating(px, py) = self.panels[i].place {
					let (min, max) = (self.panels[i].min, self.panels[i].max);
					self.panels[i].w = (x - px).clamp(min.0, max.0);
					self.panels[i].h = (y - py).clamp(min.1, max.1);
				}
				true
			}
		}
	}

	/// Pointer release. Returns true when a drag was finished.
	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
		self.cursor = (x, y);
		// Compute the layout while the drag is still live: a peeked-empty
		// dock's drop rect only exists during the drag, and clearing
		// `self.drag` first would make the drop miss it.
		let layout = self.layout(w, h);
		match std::mem::replace(&mut self.drag, Drag::None) {
			Drag::None => false,
			Drag::Move { panel, moved, .. } => {
				if moved {
					self.drop_at(panel, x, y, &layout);
				}
				true
			}
			_ => true,
		}
	}

	/// Drop a dragged panel: into the dock under the cursor (insert position
	/// by midpoint along the dock axis, per the prototype), or stay floating.
	/// `layout` must be computed while the drag is live (peeks included).
	fn drop_at(&mut self, i: usize, x: f32, y: f32, layout: &Layout) {
		let Some(side) = (0..4).find(|&s| layout.docks[s].is_some_and(|r| r.contains(x, y))) else {
			let p = &mut self.panels[i];
			p.prev = p.place;
			return;
		};
		self.panels[i].place = Place::Docked(side);
		self.panels[i].prev = Place::Docked(side);
		// Insert before the first dock-mate whose midpoint is past the cursor.
		let vertical = side == LEFT || side == RIGHT;
		let target = layout
			.panels
			.iter()
			.filter(|(p, _)| *p != i && self.panels[*p].place == Place::Docked(side))
			.find(|(_, r)| if vertical { y < r.y + r.h / 2.0 } else { x < r.x + r.w / 2.0 })
			.map(|(p, _)| *p);
		let moved = self.panels.remove(i);
		match target {
			Some(t) => {
				let t = if t > i { t - 1 } else { t };
				self.panels.insert(t, moved);
			}
			None => self.panels.push(moved),
		}
	}

	/// Frame chrome below the panels: dock-edge resizers + splitters.
	/// The shell composes a frame as: background → peeks → per-panel chrome +
	/// content (in `layout().panels` order — that IS the z-order).
	pub fn draw_background(&self, w: f32, h: f32) -> UiQuads {
		let mut q = UiQuads::default();
		let layout = self.layout(w, h);
		for r in layout.edges.iter().flatten() {
			q.rect(*r, w, h, theme::SPLITTER);
		}
		for (_, _, r) in &layout.splitters {
			q.rect(*r, w, h, theme::SPLITTER);
		}
		q
	}

	/// The steel sampling for panel `i` at `r`: floating panels anchor a stable
	/// crop to themselves (no swimming as they move); docked panels share the
	/// stretched viewport sheet. The shell uses this for a panel's content
	/// quads too, so chrome + content stay one coherent surface.
	pub fn steel_map(&self, i: usize, r: Rect) -> SteelMap {
		if matches!(self.panels[i].place, Place::Floating(..)) { SteelMap::anchored(r) } else { SteelMap::Stretch }
	}

	/// Panel `i`'s border-ring width (a thin 2-px ring). The ring is also the
	/// content margin (borders-as-margin), so this drives both the chrome draw
	/// and the body/titlebar/close hit geometry.
	pub fn frame_of(&self, i: usize) -> f32 {
		if matches!(self.panels[i].place, Place::Floating(..)) { FRAME_FLOAT } else { FRAME_DOCK }
	}

	/// Panel `i`'s content area for rect `r` (inside its border-as-margin).
	pub fn body_of(&self, i: usize, r: Rect) -> Rect {
		ui::body_rect(r, self.frame_of(i))
	}

	/// One panel's chrome (+ its placeholder hint while `show_hint`).
	pub fn draw_panel(&self, i: usize, r: Rect, w: f32, h: f32, show_hint: bool, hot: crate::ui::Hot) -> UiQuads {
		let p = &self.panels[i];
		// Floating panels carry their own stable steel crop; docked ones share
		// the stretched viewport sheet.
		let mut q = UiQuads::with_steel_map(self.steel_map(i, r));
		let dragging = matches!(self.drag, Drag::Move { panel, moved: true, .. } if panel == i);
		// A thin 2-px border ring that also margins the content.
		let frame = self.frame_of(i);
		ui::panel(&mut q, r, p.title, dragging, frame, w, h, hot);
		if show_hint {
			let body = ui::body_rect(r, frame);
			q.label_wrapped(p.hint, body.strip_top(24.0), 8.0, ui::FONT_SMALL, w, h, theme::INK_DIM);
		}
		if matches!(p.place, Place::Floating(..)) {
			// A dark grip triangle in the bottom-right corner (the hit area is
			// still the corner square — see `on_press`).
			let (x1, y1) = (r.x + r.w, r.y + r.h);
			q.tri((x1 - HANDLE, y1), (x1, y1 - HANDLE), (x1, y1), w, h, theme::RESIZE_HANDLE);
		}
		q
	}

	/// Peeked empty docks: black 50 % drop-target highlights, drawn on the map
	/// below the windows.
	pub fn draw_peeks(&self, w: f32, h: f32) -> UiQuads {
		let mut q = UiQuads::default();
		let layout = self.layout(w, h);
		let peek = self.peek(w, h);
		for side in 0..4 {
			if peek[side] && self.docked(side).is_empty() {
				if let Some(r) = layout.docks[side] {
					q.rect(r, w, h, theme::DOCK_PEEK);
				}
			}
		}
		q
	}

	// ----- layout persistence -------------------------------------

	/// Reset the whole layout to defaults (Windows ▸ Reset Dialogs),
	/// keeping the reserved top strip (the menu/tab bar height).
	pub fn reset(&mut self) {
		let top = self.top;
		*self = Self::default();
		self.top = top;
	}

	/// Clamp floating panels so at least [`MIN_VISIBLE`] px stays on-screen and
	/// the titlebar never hides above the top strip. Applied on load and
	/// on every window resize, so a window can't be lost off the edge.
	pub fn clamp_floating(&mut self, w: f32, h: f32) {
		for p in &mut self.panels {
			if let Place::Floating(fx, fy) = &mut p.place {
				let x_lo = MIN_VISIBLE - p.w;
				*fx = fx.clamp(x_lo, (w - MIN_VISIBLE).max(x_lo));
				*fy = fy.clamp(self.top, (h - MIN_VISIBLE).max(self.top));
			}
		}
	}

	/// The smallest a dock's cross-axis may be to fit its widest panel's content
	/// (never below [`MIN_DOCK`]).
	fn dock_cross_min(&self, side: usize) -> f32 {
		self.docked(side).iter().fold(MIN_DOCK, |m, &i| m.max(cross_min(self.panels[i].min, side)))
	}

	/// Clamp every panel into its sensible size range so content can't overflow a
	/// too-small window, nor a window grow past its max / the viewport. The
	/// companion to [`clamp_floating`] (position); both run on load + resize.
	pub fn clamp_sizes(&mut self, w: f32, h: f32) {
		let avail_h = (h - self.top).max(MIN_VISIBLE);
		for p in &mut self.panels {
			match p.place {
				Place::Floating(..) => {
					p.w = p.w.clamp(p.min.0, p.max.0.min(w).max(p.min.0));
					p.h = p.h.clamp(p.min.1, p.max.1.min(avail_h).max(p.min.1));
				}
				Place::Docked(side) => p.extent = p.extent.max(along_min(p.min, side)),
				Place::Hidden => {}
			}
		}
		for side in 0..4 {
			let lo = self.dock_cross_min(side);
			self.dock_size[side] = self.dock_size[side].clamp(lo, MAX_DOCK.max(lo));
		}
	}

	/// Serialize the layout as the `[Workspace]` section of `mme.ini`:
	/// `Docks = left right top bottom`, plus one key per panel —
	/// `Place X Y W H Extent` (`X`/`Y` only meaningful for `Float`).
	pub fn to_ini(&self) -> INISection {
		const NAMES: [&str; 4] = ["Left", "Right", "Top", "Bottom"];
		let mut section = INISection::new();
		let d = self.dock_size;
		let _ = section.set_entry("Docks".to_string(), format!("{} {} {} {}", d[0], d[1], d[2], d[3]));
		for p in &self.panels {
			let (place, x, y) = match p.place {
				Place::Docked(side) => (NAMES[side.min(3)], 0.0, 0.0),
				Place::Floating(fx, fy) => ("Float", fx, fy),
				Place::Hidden => ("Hidden", 0.0, 0.0),
			};
			let _ = section.set_entry(camel(p.id), format!("{place} {x} {y} {} {} {}", p.w, p.h, p.extent));
		}
		section
	}

	/// Apply a `[Workspace]` section: set dock sizes + each known
	/// panel's place/size, then clamp floats into the `w`×`h` screen. Unknown
	/// keys and malformed fields are skipped (keeps defaults), so it's
	/// forward-compatible with hand-edited files.
	pub fn apply_ini(&mut self, section: &INISection, w: f32, h: f32) {
		if let Some(docks) = section.get_entry::<String>("Docks") {
			for (i, text) in docks.split_whitespace().take(4).enumerate() {
				if let Ok(n) = text.parse::<f32>() {
					self.dock_size[i] = n;
				}
			}
		}
		for (key, value) in section {
			if key == "Docks" {
				continue;
			}
			let Some(idx) = self.find(&key.to_lowercase()) else {
				continue;
			};
			let text = value.to_string();
			let mut parts = text.split_whitespace();
			let Some(place_word) = parts.next() else {
				continue;
			};
			let mut num = || parts.next().and_then(|t| t.parse::<f32>().ok());
			let (x, y, wv, hv, ev) = (num(), num(), num(), num(), num());
			if let Some(v) = wv {
				self.panels[idx].w = v;
			}
			if let Some(v) = hv {
				self.panels[idx].h = v;
			}
			if let Some(v) = ev {
				self.panels[idx].extent = v;
			}
			let place = match place_word.to_ascii_lowercase().as_str() {
				"left" => Place::Docked(LEFT),
				"right" => Place::Docked(RIGHT),
				"top" => Place::Docked(TOP),
				"bottom" => Place::Docked(BOTTOM),
				"float" => Place::Floating(x.unwrap_or(80.0), y.unwrap_or(80.0)),
				"hidden" => Place::Hidden,
				_ => self.panels[idx].place,
			};
			self.panels[idx].place = place;
			// A hidden panel keeps its default `prev` so `window … on` restores
			// it to a sensible dock; otherwise track the loaded place.
			if !matches!(place, Place::Hidden) {
				self.panels[idx].prev = place;
			}
		}
		// Loaded sizes are untrusted — clamp them into range before clamping
		// position, so a stale/hand-edited file can't overflow content off-screen.
		self.clamp_sizes(w, h);
		self.clamp_floating(w, h);
	}
}

/// Panel ids are single lowercase words (`"palette"`); their `[Workspace]`
/// keys follow the CamelCase INI convention (`Palette`).
fn camel(id: &str) -> String {
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
	fn settings_round_trip_preserves_layout() {
		// serialize a customized layout, re-apply onto a fresh default,
		// and every panel's place + the dock sizes come back identical.
		let mut a = ws();
		a.dock_to("palette", "left", None).unwrap();
		a.dock_to("minimap", "float", Some((120.0, 90.0))).unwrap();
		a.show("toolbox", Some(false)).unwrap();
		a.dock_size[LEFT] = 333.0;
		let section = a.to_ini();
		let mut b = ws();
		b.apply_ini(&section, W, H);
		for id in ["palette", "minimap", "toolbox", "tiles"] {
			let (pa, pb) = (a.find(id).unwrap(), b.find(id).unwrap());
			assert_eq!(a.panels[pa].place, b.panels[pb].place, "{id} place round-trips");
		}
		assert_eq!(a.dock_size, b.dock_size, "dock sizes round-trip");
	}

	#[test]
	fn clamp_keeps_floats_on_screen() {
		// a window dragged/loaded far off-screen is pulled back so ≥32px
		// stays visible and its titlebar can't hide above the top strip.
		let mut w = ws();
		w.top = 24.0;
		w.dock_to("minimap", "float", Some((5000.0, -500.0))).unwrap();
		w.clamp_floating(W, H);
		let p = &w.panels[w.find("minimap").unwrap()];
		let Place::Floating(x, y) = p.place else { panic!("still floating") };
		assert!(x <= W - MIN_VISIBLE && x + p.w >= MIN_VISIBLE, "≥32px visible horizontally");
		assert!(y >= w.top && y <= H - MIN_VISIBLE, "titlebar below the top strip, ≥32px visible");
	}

	#[test]
	fn clamp_sizes_bounds_floating_and_docks() {
		let mut w = ws();
		let i = w.find("minimap").unwrap();
		let (min, max) = (w.panels[i].min, w.panels[i].max);
		w.panels[i].place = Place::Floating(10.0, 50.0);
		// Too small → grows to the per-panel min.
		w.panels[i].w = 5.0;
		w.panels[i].h = 5.0;
		w.clamp_sizes(W, H);
		assert_eq!((w.panels[i].w, w.panels[i].h), min, "min enforced");
		// Too large → capped at the per-panel max (which is below the screen).
		w.panels[i].w = 5000.0;
		w.panels[i].h = 5000.0;
		w.clamp_sizes(W, H);
		assert_eq!((w.panels[i].w, w.panels[i].h), max, "max enforced");
		// A max past the viewport still can't exceed the screen.
		w.panels[i].max = (9999.0, 9999.0);
		w.panels[i].w = 9999.0;
		w.clamp_sizes(W, H);
		assert_eq!(w.panels[i].w, W, "floating width capped at the viewport");
		// A dock too thin for its widest panel's content is widened (fresh
		// workspace — minimap docked LEFT by default).
		let mut d = ws();
		d.dock_size[LEFT] = 10.0;
		d.clamp_sizes(W, H);
		let mm = d.panels[d.find("minimap").unwrap()].min;
		assert!(d.dock_size[LEFT] >= cross_min(mm, LEFT), "dock fits content min");
	}

	#[test]
	fn reset_restores_defaults_keeping_top() {
		// Reset Dialogs returns every panel to its default place, but keeps
		// the reserved top strip the editor set.
		let mut w = ws();
		w.top = 24.0;
		w.dock_to("palette", "float", Some((10.0, 10.0))).unwrap();
		w.show("toolbox", Some(false)).unwrap();
		w.reset();
		let def = ws();
		for id in ["palette", "toolbox", "minimap", "tiles"] {
			assert_eq!(w.panels[w.find(id).unwrap()].place, def.panels[def.find(id).unwrap()].place, "{id} reset");
		}
		assert_eq!(w.top, 24.0, "top strip preserved across reset");
	}

	#[test]
	fn default_layout_partitions_the_screen() {
		let l = ws().layout(W, H);
		// All four default panels laid out; center inset on three sides
		// (no top dock by default → center touches the screen top).
		assert_eq!(l.panels.len(), 4);
		assert!(l.center.x > 0.0);
		assert!(l.center.x + l.center.w < W);
		assert!(l.center.y + l.center.h < H);
		assert!(l.docks[TOP].is_none());
		assert_eq!(l.center.y, 0.0);
		assert_eq!(l.center.y, l.docks[LEFT].unwrap().y);
		// Right dock stacks two panels with one splitter between them.
		assert_eq!(l.splitters.len(), 1);
		assert_eq!(l.splitters[0].0, RIGHT);
	}

	#[test]
	fn hiding_a_dock_panel_grows_the_center() {
		let mut w = ws();
		let before = w.layout(W, H).center;
		w.show("minimap", Some(false)).unwrap();
		let after = w.layout(W, H).center;
		assert!(after.x < before.x, "left dock auto-hides when emptied");
		assert!(after.w > before.w);
		// And re-showing restores the docked place.
		w.show("minimap", Some(true)).unwrap();
		assert_eq!(w.panels[w.find("minimap").unwrap()].place, Place::Docked(LEFT));
	}

	#[test]
	fn window_toggle_round_trips() {
		let mut w = ws();
		w.show("toolbox", None).unwrap();
		assert_eq!(w.panels[w.find("toolbox").unwrap()].place, Place::Hidden);
		w.show("toolbox", None).unwrap();
		assert_eq!(w.panels[w.find("toolbox").unwrap()].place, Place::Docked(BOTTOM));
		assert!(w.show("nonsense", None).is_err());
	}

	#[test]
	fn dock_command_moves_between_sides_and_float() {
		let mut w = ws();
		w.dock_to("palette", "left", None).unwrap();
		assert_eq!(w.docked(LEFT).len(), 2);
		assert_eq!(w.docked(RIGHT).len(), 1);
		w.dock_to("palette", "float", Some((50.0, 60.0))).unwrap();
		let i = w.find("palette").unwrap();
		assert_eq!(w.panels[i].place, Place::Floating(50.0, 60.0));
		assert!(w.dock_to("palette", "diagonal", None).is_err());
	}

	#[test]
	fn titlebar_drag_undocks_then_drops_into_a_dock() {
		let mut w = ws();
		let l = w.layout(W, H);
		// Find the minimap's titlebar.
		let mm = w.find("minimap").unwrap();
		let r = l.panels.iter().find(|(i, _)| *i == mm).unwrap().1;
		let (px, py) = (r.x + 40.0, r.y + 8.0);
		assert_eq!(Press::Chrome, w.on_press(px, py, W, H));
		// A 2-px wiggle does not undock.
		w.on_move(px + 2.0, py, W, H);
		assert_eq!(w.panels[w.find("minimap").unwrap()].place, Place::Docked(LEFT));
		// Crossing the threshold floats it at the cursor.
		w.on_move(640.0, 400.0, W, H);
		assert!(matches!(w.panels[w.find("minimap").unwrap()].place, Place::Floating(..)));
		// Releasing over the right dock docks it there (after `tiles`+`palette`).
		let right = w.layout(W, H).docks[RIGHT].unwrap();
		w.on_move(right.x + right.w / 2.0, right.y + right.h - 10.0, W, H);
		assert!(w.on_release(right.x + right.w / 2.0, right.y + right.h - 10.0, W, H));
		assert_eq!(w.panels[w.find("minimap").unwrap()].place, Place::Docked(RIGHT));
		assert_eq!(w.docked(RIGHT).len(), 3);
		// Dropped at the bottom → it stacks last.
		assert_eq!(*w.docked(RIGHT).last().unwrap(), w.find("minimap").unwrap());
		// The left dock emptied and auto-hides.
		assert!(w.layout(W, H).docks[LEFT].is_none());
	}

	#[test]
	fn undocking_takes_top_z_immediately() {
		let mut w = ws();
		// An existing floating panel that would otherwise cover the drag.
		w.dock_to("tiles", "float", Some((400.0, 200.0))).unwrap();
		let l = w.layout(W, H);
		let pi = w.find("palette").unwrap();
		let r = l.panels.iter().find(|(i, _)| *i == pi).unwrap().1;
		assert_eq!(Press::Chrome, w.on_press(r.x + 30.0, r.y + 8.0, W, H));
		// Crossing the drag threshold undocks AND raises in the same move.
		w.on_move(640.0, 300.0, W, H);
		assert_eq!(w.panels.last().unwrap().id, "palette", "undocked panel is topmost");
		// The drag keeps tracking the panel across the reorder.
		w.on_move(700.0, 350.0, W, H);
		let p = w.panels.last().unwrap();
		assert_eq!(p.id, "palette");
		assert!(matches!(p.place, Place::Floating(px, _) if px > 600.0));
	}

	#[test]
	fn re_docks_into_an_emptied_peeked_dock() {
		let mut w = ws();
		// Drag the only left panel out — the left dock auto-hides...
		let l = w.layout(W, H);
		let mm = w.find("minimap").unwrap();
		let r = l.panels.iter().find(|(i, _)| *i == mm).unwrap().1;
		assert_eq!(Press::Chrome, w.on_press(r.x + 40.0, r.y + 8.0, W, H));
		w.on_move(600.0, 400.0, W, H);
		assert!(w.layout(W, H).docks[LEFT].is_none(), "emptied dock auto-hides");
		// ...dragging back near the edge peeks it open as a drop target...
		w.on_move(30.0, 400.0, W, H);
		assert!(w.layout(W, H).docks[LEFT].is_some(), "peeked open during the drag");
		// ...and releasing inside the peeked rect docks the panel.
		assert!(w.on_release(30.0, 400.0, W, H));
		assert_eq!(w.panels[w.find("minimap").unwrap()].place, Place::Docked(LEFT));
	}

	#[test]
	fn drop_outside_any_dock_stays_floating() {
		let mut w = ws();
		let l = w.layout(W, H);
		let mm = w.find("minimap").unwrap();
		let r = l.panels.iter().find(|(i, _)| *i == mm).unwrap().1;
		assert_eq!(Press::Chrome, w.on_press(r.x + 40.0, r.y + 8.0, W, H));
		w.on_move(600.0, 300.0, W, H);
		w.on_release(600.0, 300.0, W, H);
		let p = &w.panels[w.find("minimap").unwrap()];
		assert!(matches!(p.place, Place::Floating(..)));
		assert_eq!(p.prev, p.place, "floating place survives a later hide/show");
	}

	#[test]
	fn close_glyph_hides_and_window_command_restores() {
		let mut w = ws();
		let l = w.layout(W, H);
		let tb = w.find("toolbox").unwrap();
		let r = l.panels.iter().find(|(i, _)| *i == tb).unwrap().1;
		let close = crate::ui::close_rect(r, FRAME_DOCK);
		assert_eq!(Press::Chrome, w.on_press(close.x + 4.0, close.y + 4.0, W, H));
		assert_eq!(w.panels[w.find("toolbox").unwrap()].place, Place::Hidden);
		assert!(w.layout(W, H).docks[BOTTOM].is_none());
		w.show("toolbox", Some(true)).unwrap();
		assert_eq!(w.panels[w.find("toolbox").unwrap()].place, Place::Docked(BOTTOM));
	}

	#[test]
	fn dock_edge_drag_resizes_and_clamps() {
		let mut w = ws();
		let l = w.layout(W, H);
		let edge = l.edges[LEFT].unwrap();
		assert_eq!(Press::Chrome, w.on_press(edge.x + 2.0, edge.y + 50.0, W, H));
		w.on_move(400.0, edge.y + 50.0, W, H);
		w.on_release(400.0, edge.y + 50.0, W, H);
		assert_eq!(w.layout(W, H).docks[LEFT].unwrap().w, 400.0);
		// Clamped on both ends.
		let edge = w.layout(W, H).edges[LEFT].unwrap();
		assert_eq!(Press::Chrome, w.on_press(edge.x + 2.0, edge.y + 50.0, W, H));
		w.on_move(5000.0, edge.y + 50.0, W, H);
		w.on_release(5000.0, edge.y + 50.0, W, H);
		assert_eq!(w.layout(W, H).docks[LEFT].unwrap().w, MAX_DOCK);
	}

	#[test]
	fn splitter_drag_resizes_the_dock_mate_above() {
		let mut w = ws();
		let l = w.layout(W, H);
		let (side, nth, s) = l.splitters[0];
		assert_eq!((side, nth), (RIGHT, 0));
		assert_eq!(Press::Chrome, w.on_press(s.x + 2.0, s.y + 2.0, W, H));
		let dock_top = l.docks[RIGHT].unwrap().y;
		w.on_move(s.x + 2.0, dock_top + 150.0, W, H);
		w.on_release(s.x + 2.0, dock_top + 150.0, W, H);
		let i = w.docked(RIGHT)[0];
		assert_eq!(w.panels[i].extent, 150.0);
	}

	#[test]
	fn body_press_reports_panel_and_rect() {
		let mut w = ws();
		let l = w.layout(W, H);
		let ti = w.find("tiles").unwrap();
		let r = l.panels.iter().find(|(i, _)| *i == ti).unwrap().1;
		match w.on_press(r.x + 50.0, r.y + 100.0, W, H) {
			Press::Body { id, body } => {
				assert_eq!(id, "tiles");
				assert_eq!(body, ui::body_rect(r, FRAME_DOCK));
			}
			other => panic!("expected Body, got {other:?}"),
		}
		// body_at finds the same panel (wheel routing); the center is free.
		assert_eq!(w.body_at(r.x + 50.0, r.y + 100.0, W, H).unwrap().0, "tiles");
		let c = w.layout(W, H).center;
		assert!(w.body_at(c.x + c.w / 2.0, c.y + c.h / 2.0, W, H).is_none());
	}

	#[test]
	fn over_ui_separates_chrome_from_map() {
		let w = ws();
		let l = w.layout(W, H);
		let c = l.center;
		assert!(!w.over_ui(c.x + c.w / 2.0, c.y + c.h / 2.0, W, H));
		let r = l.panels[0].1;
		assert!(w.over_ui(r.x + 5.0, r.y + 5.0, W, H));
	}

	#[test]
	fn floating_resize_respects_minimums() {
		let mut w = ws();
		w.dock_to("minimap", "float", Some((100.0, 100.0))).unwrap();
		let i = w.find("minimap").unwrap();
		let (pw, ph) = (w.panels[i].w, w.panels[i].h);
		let handle = (100.0 + pw - 4.0, 100.0 + ph - 4.0);
		assert_eq!(Press::Chrome, w.on_press(handle.0, handle.1, W, H));
		w.on_move(100.0 + 10.0, 100.0 + 10.0, W, H);
		w.on_release(100.0 + 10.0, 100.0 + 10.0, W, H);
		let i = w.find("minimap").unwrap();
		assert_eq!((w.panels[i].w, w.panels[i].h), w.panels[i].min);
	}
}
