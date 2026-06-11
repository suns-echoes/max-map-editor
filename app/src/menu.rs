//! Main menu bar: the ten menus from the design
//! (`designs/features.drawio`, "Main menu" page). Every leaf is either an
//! **Action** — a command line through the command parser, exactly like a
//! keybinding — or a **Todo** placeholder that echoes its backlog ticket
//! (drawn dim, so the unbuilt surface area is visible but honest).
//!
//! Pure geometry/state — the shell routes presses here first (menus are
//! topmost); `menu NAME|off` drives it from scripts for screenshots.

use std::path::Path;

use crate::text;
use crate::theme;
use crate::ui::{Hot, Rect, UiQuads};

pub const BAR_H: f32 = 24.0;
// 16px label + 4px top + 4px bottom padding.
const ITEM_H: f32 = 24.0;
const SEP_H: f32 = 7.0;
const FONT: f32 = crate::ui::FONT_BODY; // menu is primary nav → the 16px tier
const PAD_X: f32 = 10.0;
/// Left gutter reserved on every dropdown row — holds the toggle checkbox and
/// keeps all labels in one aligned column.
const CHECK_W: f32 = 22.0;

pub enum Item {
	/// Runs a command line (validated by a test against the parser).
	Action {
		label: String,
		command: String,
		/// Keyboard shortcut label (`"Ctrl+C"`) — drawn right-aligned, dim.
		/// Resolved from the loaded bindings via [`MenuBar::apply_shortcuts`].
		hint: Option<String>,
	},
	/// Like [`Item::Action`], but reflects on/off state with a checkbox; `key`
	/// is resolved against live editor state at draw time.
	Toggle {
		label: String,
		command: String,
		key: &'static str,
		/// Keyboard shortcut label, as on [`Item::Action`].
		hint: Option<String>,
	},
	/// Not built yet — echoes the backlog ticket when clicked.
	Todo {
		label: String,
		ticket: &'static str,
	},
	Sep,
	/// Opens a side submenu.
	Sub {
		label: String,
		items: Vec<Item>,
	},
}

fn act(label: &str, command: &str) -> Item {
	Item::Action { label: label.into(), command: command.into(), hint: None }
}

/// A checkbox item: runs `command`, shows checked when `key` resolves true.
fn toggle(label: &str, command: &str, key: &'static str) -> Item {
	Item::Toggle { label: label.into(), command: command.into(), key, hint: None }
}

fn todo(label: &str, ticket: &'static str) -> Item {
	Item::Todo { label: label.into(), ticket }
}

fn sub(label: &str, items: Vec<Item>) -> Item {
	Item::Sub { label: label.into(), items }
}

pub struct Menu {
	pub title: &'static str,
	pub items: Vec<Item>,
}

pub struct MenuBar {
	pub menus: Vec<Menu>,
	/// Open dropdown (menu index).
	pub open: Option<usize>,
	/// Open submenu (item index within the open dropdown).
	sub_open: Option<usize>,
	hover: Option<(usize, bool)>, // (item index, inside submenu)
}

/// What a press did — the shell acts on `Run`/`Todo`.
#[derive(Debug, PartialEq)]
pub enum Press {
	/// Not on the menu (and nothing was open) — fall through.
	None,
	/// Swallowed (opened/closed/ignored).
	Consumed,
	Run(String),
	Todo(String, &'static str),
}

fn items_height(items: &[Item]) -> f32 {
	items.iter().map(|it| if matches!(it, Item::Sep) { SEP_H } else { ITEM_H }).sum::<f32>() + 8.0
}

/// Gap between a label and its right-aligned shortcut hint.
const HINT_GAP: f32 = 18.0;

fn items_width(items: &[Item]) -> f32 {
	items
		.iter()
		.filter_map(|it| match it {
			Item::Action { label, hint, .. } | Item::Toggle { label, hint, .. } => {
				let hint_w = hint.as_ref().map_or(0.0, |c| text::label_width(c, FONT) + HINT_GAP);
				Some(text::label_width(label, FONT) + hint_w)
			}
			Item::Todo { label, .. } | Item::Sub { label, .. } => Some(text::label_width(label, FONT)),
			Item::Sep => None,
		})
		.fold(120.0_f32, f32::max)
		+ CHECK_W // left gutter (checkbox column)
		+ PAD_X
		+ 16.0 // submenu arrow column
}

/// Rect of item `i` inside a panel that starts at `(px, py)`.
fn item_rect(items: &[Item], px: f32, py: f32, w: f32, i: usize) -> Rect {
	let mut y = py + 4.0;
	for (k, it) in items.iter().enumerate() {
		let h = if matches!(it, Item::Sep) { SEP_H } else { ITEM_H };
		if k == i {
			return Rect::new(px, y, w, h);
		}
		y += h;
	}
	unreachable!("item index in range");
}

fn item_at(items: &[Item], panel: Rect, x: f32, y: f32) -> Option<usize> {
	if !panel.contains(x, y) {
		return None;
	}
	(0..items.len())
		.find(|&i| !matches!(items[i], Item::Sep) && item_rect(items, panel.x, panel.y, panel.w, i).contains(x, y))
}

impl MenuBar {
	/// The full design tree. `maps_dir` feeds the Quick Load submenu.
	pub fn new(maps_dir: &Path) -> Self {
		let mut quick = Vec::new();
		if let Ok(entries) = std::fs::read_dir(maps_dir) {
			let mut names: Vec<String> = entries
				.filter_map(|e| e.ok())
				.filter_map(|e| {
					let name = e.file_name().to_string_lossy().into_owned();
					name.ends_with(".json").then_some(name)
				})
				.collect();
			names.sort();
			for name in names.into_iter().take(24) {
				quick.push(act(
					name.trim_end_matches(".json"),
					&format!("open! \"{}\"", maps_dir.join(&name).display()),
				));
			}
		}
		if quick.is_empty() {
			quick.push(todo("(no projects found)", "IO-7"));
		}

		let menus = vec![
			Menu {
				title: "File",
				items: vec![
					act("New Map...", "new-map"),
					act("New from Image...", "file-dialog new-from-image"),
					act("Load Map...", "file-dialog load"),
					sub("Quick Load", quick),
					todo("Load Previous", "SHELL-4 (recent maps)"),
					Item::Sep,
					act("Save Project", "save-project"),
					act("Save Project As...", "file-dialog save-as"),
					act("Save Project Copy...", "file-dialog save-copy"),
					act("Close Project", "close-project"),
					Item::Sep,
					act("Export to WRL", "export"),
					todo("Import WRL...", "IO-9"),
					Item::Sep,
					todo("Export as Image...", "IO-5"),
					Item::Sep,
					act("Exit", "quit"),
				],
			},
			Menu {
				title: "Edit",
				items: vec![
					act("Undo", "undo"),
					act("Redo", "redo"),
					todo("Undo History", "CORE-15"),
					Item::Sep,
					act("Cut", "cut"),
					act("Copy", "copy"),
					act("Paste", "paste"),
					act("Clear", "delete"),
					Item::Sep,
					todo("Preferences...", "SHELL-4"),
				],
			},
			Menu {
				title: "View",
				items: vec![
					sub(
						"Zoom",
						vec![
							act("100%", "zoom-to 1"),
							act("50%", "zoom-to 0.5"),
							act("25%", "zoom-to 0.25"),
							act("Fit All", "fit"),
							todo("Custom...", "UI-7"),
						],
					),
					toggle("Show Grid", "grid toggle", "grid"),
					toggle("Show Pass Overlay", "pass-overlay toggle", "pass-overlay"),
					toggle("Show Units", "units toggle", "show-units"),
					todo("Fullscreen", "SHELL-7"),
					todo("Immersive Mode", "SHELL-7"),
				],
			},
			Menu {
				title: "Mode",
				items: vec![
					toggle("Map Editor", "mode map", "mode:map"),
					toggle("Pass Table Editor", "mode pass", "mode:pass"),
					todo("Tile Pixel Editor", "SHELL-8 / TOOL-7"),
					Item::Sep,
					sub(
						"Tile Layer",
						vec![
							toggle("Water", "layer water", "layer:water"),
							toggle("Ground", "layer ground", "layer:ground"),
							todo("Objects", "SHELL-8 (layer is a v2 format concern)"),
						],
					),
					Item::Sep,
					sub(
						"Render Mode",
						vec![
							toggle("Static", "animate off", "anim:off"),
							toggle("Animated", "animate on", "anim:on"),
							toggle("In-Game", "ingame on", "anim:ingame"),
							Item::Sep,
							toggle("CRT", "crt toggle", "crt"),
						],
					),
				],
			},
			Menu {
				title: "Snapshot",
				items: vec![
					todo("Take Snapshot", "CORE-14"),
					todo("Revert to Snapshot", "CORE-14"),
					todo("Show All Snapshots", "CORE-14"),
					todo("Clear Snapshots", "CORE-14"),
				],
			},
			Menu {
				title: "Select",
				items: vec![
					act("Select All", "select all"),
					act("Invert Selection", "select invert"),
					act("Clear Selection", "select clear"),
					Item::Sep,
					// Add/subtract are drag modifiers: Shift+drag adds,
					// Ctrl+drag subtracts (with the select tools active).
					act("Select Tool", "tool select"),
					act("Rect Select Tool", "tool select-rect"),
					Item::Sep,
					act("Select Similar", "select similar"),
				],
			},
			Menu {
				title: "Templates",
				items: vec![
					act("Open Template Explorer...", "window templates on"),
					act("Create New Template", "template-save"),
					act("Create Template from Selection", "template-save"),
					act("Export Selection as Template...", "file-dialog export-template"),
					Item::Sep,
					act("Import Template...", "file-dialog import-template"),
					Item::Sep,
					act("Clone Selected Template", "template-clone"),
					act("Delete Selected Template", "template-delete"),
				],
			},
			Menu {
				title: "Tools",
				items: vec![
					sub(
						"Shore",
						vec![
							act("Auto Shore", "shore"),
							act("Auto Shore ALT", "shore alt"),
							act("Auto Fix Shore...", "fix-shore-modal"),
							todo("Find Shore Bugs...", "TOOL-13"),
						],
					),
					Item::Sep,
					todo("Auto Generate Pass Table...", "TOOL-6"),
					Item::Sep,
					act("Generate Random Terrain...", "generate-modal"),
					Item::Sep,
					act("Resize Map...", "resize-modal"),
				],
			},
			Menu {
				title: "Windows",
				items: vec![
					todo("Open Projects", "SHELL-9"),
					sub(
						"Dockable Dialogs",
						vec![
							toggle("Minimap", "window minimap", "win:minimap"),
							toggle("Tile Explorer", "window tiles", "win:tiles"),
							toggle("Color Palette", "window palette", "win:palette"),
							toggle("Toolbox", "window toolbox", "win:toolbox"),
							toggle("Units", "window units", "win:units"),
							toggle("Templates Explorer", "window templates", "win:templates"),
							todo("Pass Types Palette", "TOOL-6"),
							todo("Tile Packs Manager", "IO-4"),
						],
					),
					Item::Sep,
					act("Reset Dialogs", "reset-layout"),
					todo("Show Docks", "UI-3"),
					Item::Sep,
					todo("Tabs Positions", "SHELL-9"),
				],
			},
			Menu {
				title: "Help",
				items: vec![
					todo("Help...", "UI-7"),
					todo("User Manual", "UI-7"),
					Item::Sep,
					todo("Go to Website", "UI-7"),
					Item::Sep,
					todo("Check for Newer Version", "UI-7"),
					todo("About...", "UI-7"),
				],
			},
		];
		Self { menus, open: None, sub_open: None, hover: None }
	}

	/// Open a menu by title (case-insensitive) — the `menu` command.
	pub fn open_by_name(&mut self, name: &str) -> Result<(), String> {
		if name == "off" {
			self.close();
			return Ok(());
		}
		match self.menus.iter().position(|m| m.title.eq_ignore_ascii_case(name)) {
			Some(i) => {
				self.open = Some(i);
				self.sub_open = None;
				self.hover = None;
				Ok(())
			}
			None => Err(format!(
				"unknown menu '{name}' (have: {})",
				self.menus.iter().map(|m| m.title).collect::<Vec<_>>().join(" ").to_lowercase(),
			)),
		}
	}

	pub fn close(&mut self) {
		self.open = None;
		self.sub_open = None;
		self.hover = None;
	}

	/// Stamp shortcut hints onto every Action/Toggle whose command line has a
	/// binding (`hints`: normalized command line → chord label). Called once
	/// at startup after the bindings load.
	pub fn apply_shortcuts(&mut self, hints: &[(String, String)]) {
		fn walk(items: &mut [Item], hints: &[(String, String)]) {
			for item in items {
				match item {
					Item::Action { command, hint, .. } | Item::Toggle { command, hint, .. } => {
						*hint = hints.iter().find(|(line, _)| line == command).map(|(_, label)| label.clone());
					}
					Item::Sub { items, .. } => walk(items, hints),
					_ => {}
				}
			}
		}
		for menu in &mut self.menus {
			walk(&mut menu.items, hints);
		}
	}

	// ----- geometry ----------------------------------------------------------

	fn title_cell(&self, i: usize) -> Rect {
		let mut x = 4.0;
		for (k, m) in self.menus.iter().enumerate() {
			let w = text::label_width(m.title, FONT) + 2.0 * PAD_X;
			if k == i {
				return Rect::new(x, 0.0, w, BAR_H);
			}
			x += w;
		}
		unreachable!("menu index in range");
	}

	/// The open dropdown's panel rect, kept on-screen: the width caps at the
	/// viewport and the panel slides left rather than hanging off the right
	/// edge (long Quick Load names).
	fn dropdown_rect(&self, menu: usize, vw: f32) -> Rect {
		let t = self.title_cell(menu);
		let items = &self.menus[menu].items;
		let w = items_width(items).min((vw - 8.0).max(60.0));
		let x = t.x.min((vw - w - 4.0).max(0.0));
		Rect::new(x, BAR_H, w, items_height(items))
	}

	/// The open submenu's panel rect (beside its parent item) — flipped to the
	/// parent's left when the right side would leave the viewport.
	fn submenu_rect(&self, menu: usize, item: usize, vw: f32) -> Option<Rect> {
		let Item::Sub { items, .. } = &self.menus[menu].items[item] else { return None };
		let d = self.dropdown_rect(menu, vw);
		let parent = item_rect(&self.menus[menu].items, d.x, d.y, d.w, item);
		let w = items_width(items).min((vw - 8.0).max(60.0));
		let mut x = d.x + d.w - 2.0;
		if x + w > vw {
			x = (d.x - w + 2.0).max(0.0);
		}
		Some(Rect::new(x, parent.y, w, items_height(items)))
	}

	// ----- events -------------------------------------------------------------

	fn leaf_press(item: &Item) -> Press {
		match item {
			Item::Action { command, .. } | Item::Toggle { command, .. } => Press::Run(command.clone()),
			Item::Todo { label, ticket } => Press::Todo(label.clone(), ticket),
			_ => Press::Consumed,
		}
	}

	pub fn on_press(&mut self, x: f32, y: f32, vw: f32) -> Press {
		// Bar titles: toggle a dropdown.
		if y < BAR_H {
			for i in 0..self.menus.len() {
				if self.title_cell(i).contains(x, y) {
					if self.open == Some(i) {
						self.close();
					} else {
						self.open = Some(i);
						self.sub_open = None;
					}
					return Press::Consumed;
				}
			}
			let was_open = self.open.is_some();
			self.close();
			return if was_open { Press::Consumed } else { Press::None };
		}
		let Some(menu) = self.open else { return Press::None };

		// Submenu first (it overlaps the dropdown's right edge).
		if let Some(si) = self.sub_open {
			if let (Some(panel), Item::Sub { items, .. }) =
				(self.submenu_rect(menu, si, vw), &self.menus[menu].items[si])
			{
				if let Some(i) = item_at(items, panel, x, y) {
					let press = Self::leaf_press(&items[i]);
					if press != Press::Consumed {
						self.close();
					}
					return press;
				}
			}
		}
		let d = self.dropdown_rect(menu, vw);
		if let Some(i) = item_at(&self.menus[menu].items, d, x, y) {
			if matches!(self.menus[menu].items[i], Item::Sub { .. }) {
				self.sub_open = if self.sub_open == Some(i) { None } else { Some(i) };
				return Press::Consumed;
			}
			let press = Self::leaf_press(&self.menus[menu].items[i]);
			if press != Press::Consumed {
				self.close();
			}
			return press;
		}
		if d.contains(x, y) {
			return Press::Consumed; // a separator / panel padding
		}
		// Anywhere else closes the menu and swallows the click.
		self.close();
		Press::Consumed
	}

	/// Hover tracking: highlights, switching menus along the bar, opening
	/// submenus. Returns true when a redraw is needed.
	pub fn on_move(&mut self, x: f32, y: f32, vw: f32) -> bool {
		let Some(menu) = self.open else { return false };
		let mut changed = false;

		// Sliding along the bar switches the open menu.
		if y < BAR_H {
			for i in 0..self.menus.len() {
				if self.title_cell(i).contains(x, y) && self.open != Some(i) {
					self.open = Some(i);
					self.sub_open = None;
					self.hover = None;
					return true;
				}
			}
		}

		let mut hover = None;
		if let Some(si) = self.sub_open {
			if let (Some(panel), Item::Sub { items, .. }) =
				(self.submenu_rect(menu, si, vw), &self.menus[menu].items[si])
			{
				if let Some(i) = item_at(items, panel, x, y) {
					hover = Some((i, true));
				}
			}
		}
		if hover.is_none() {
			let d = self.dropdown_rect(menu, vw);
			if let Some(i) = item_at(&self.menus[menu].items, d, x, y) {
				hover = Some((i, false));
				// Hovering a submenu parent opens it; hovering a plain item
				// closes any open submenu.
				let is_sub = matches!(self.menus[menu].items[i], Item::Sub { .. });
				let want = is_sub.then_some(i);
				if self.sub_open != want {
					self.sub_open = want;
					changed = true;
				}
			}
		}
		if self.hover != hover {
			self.hover = hover;
			changed = true;
		}
		changed
	}

	// ----- drawing -------------------------------------------------------------

	/// Draw the bar + any open dropdown. `checked` resolves a [`Item::Toggle`]'s
	/// `key` against live editor state (so the checkboxes reflect the session).
	pub fn draw(&self, w: f32, h: f32, checked: &dyn Fn(&str) -> bool, hot: Hot) -> UiQuads {
		let mut q = UiQuads::default();
		// Steel menu band with a shaded seam under it.
		let bar = Rect::new(0.0, 0.0, w, BAR_H);
		q.material(bar, w, h, theme::TITLE);
		q.rect(Rect::new(0.0, BAR_H - 1.0, w, 1.0), w, h, theme::BEVEL.bottom);
		for (i, m) in self.menus.iter().enumerate() {
			let cell = self.title_cell(i);
			if self.open == Some(i) {
				q.rect(cell, w, h, theme::SELECTION);
			} else if hot.hover(cell) {
				q.rect(cell, w, h, theme::HOVER);
			}
			q.label_in(m.title, cell, PAD_X, FONT, w, h, theme::INK);
		}
		let Some(menu) = self.open else { return q };

		draw_panel(
			&mut q,
			&self.menus[menu].items,
			self.dropdown_rect(menu, w),
			self.hover.filter(|(_, in_sub)| !in_sub).map(|(i, _)| i),
			self.sub_open,
			checked,
			w,
			h,
		);
		if let Some(si) = self.sub_open {
			if let (Some(panel), Item::Sub { items, .. }) =
				(self.submenu_rect(menu, si, w), &self.menus[menu].items[si])
			{
				let hov = self.hover.filter(|(_, in_sub)| *in_sub).map(|(i, _)| i);
				draw_panel(&mut q, items, panel, hov, None, checked, w, h);
			}
		}
		q
	}
}

/// A small checkbox in a row's left gutter — an inset well, filled with the
/// accent when `on`.
fn checkbox(q: &mut UiQuads, r: Rect, on: bool, w: f32, h: f32) {
	let bx = Rect::new(r.x + 6.0, r.y + (r.h - 11.0) / 2.0, 11.0, 11.0);
	q.field(bx, w, h);
	if on {
		q.rect(Rect::new(bx.x + 2.0, bx.y + 2.0, bx.w - 4.0, bx.h - 4.0), w, h, theme::ACCENT);
	}
}

/// A row's shortcut hint, right-aligned and dim. The label's `label_fit`
/// already leaves room — `items_width` budgets label + gap + hint.
fn draw_hint(q: &mut UiQuads, hint: &Option<String>, r: Rect, w: f32, h: f32) {
	if let Some(hint) = hint {
		let hw = text::label_width(hint, FONT);
		q.label_in(hint, Rect::new(r.x + r.w - PAD_X - hw, r.y, hw, r.h), 0.0, FONT, w, h, theme::INK_DIM);
	}
}

/// One dropdown panel — shared by the menu bar's dropdowns/submenus and the
/// right-click context menu. `sub_open` keeps an open submenu's parent row lit.
#[allow(clippy::too_many_arguments)]
fn draw_panel(
	q: &mut UiQuads,
	items: &[Item],
	panel: Rect,
	hover: Option<usize>,
	sub_open: Option<usize>,
	checked: &dyn Fn(&str) -> bool,
	w: f32,
	h: f32,
) {
	q.raised(panel, w, h, theme::PANEL, 2.0);
	for (i, item) in items.iter().enumerate() {
		let r = item_rect(items, panel.x, panel.y, panel.w, i);
		match item {
			Item::Sep => {
				q.rect(Rect::new(r.x + 6.0, r.y + r.h / 2.0, r.w - 12.0, 1.0), w, h, theme::SPLITTER);
			}
			Item::Action { label, hint, .. } => {
				if hover == Some(i) {
					q.rect(r, w, h, theme::SELECTION);
				}
				q.label_fit(label, r, CHECK_W, FONT, w, h, theme::INK);
				draw_hint(q, hint, r, w, h);
			}
			Item::Toggle { label, key, hint, .. } => {
				if hover == Some(i) {
					q.rect(r, w, h, theme::SELECTION);
				}
				checkbox(q, r, checked(key), w, h);
				q.label_fit(label, r, CHECK_W, FONT, w, h, theme::INK);
				draw_hint(q, hint, r, w, h);
			}
			Item::Todo { label, .. } => {
				// Placeholders are dim — visible surface, honest state.
				if hover == Some(i) {
					q.rect(r, w, h, theme::HOVER);
				}
				q.label_fit(label, r, CHECK_W, FONT, w, h, theme::INK_DIM);
			}
			Item::Sub { label, .. } => {
				if hover == Some(i) || sub_open == Some(i) {
					q.rect(r, w, h, theme::SELECTION);
				}
				// Keep the label clear of the submenu-arrow column.
				let avail = Rect::new(r.x, r.y, r.w - 16.0, r.h);
				q.label_fit(label, avail, CHECK_W, FONT, w, h, theme::INK);
				q.label_in(">", Rect::new(r.x + r.w - 16.0, r.y, 16.0, r.h), 0.0, FONT, w, h, theme::INK_DIM);
			}
		}
	}
}

/// The right-click context menu: a single dropdown panel anchored at the
/// click, clamped to the viewport. Items are plain [`Item`]s built from the
/// editor state at open time (`state::context_menu_items`); the shell routes
/// presses here before everything else while it's open.
pub struct ContextMenu {
	items: Vec<Item>,
	/// The click position (the panel's preferred top-left).
	pos: (f32, f32),
	hover: Option<usize>,
}

impl ContextMenu {
	pub fn new(items: Vec<Item>, pos: (f32, f32)) -> Self {
		Self { items, pos, hover: None }
	}

	/// The panel rect: at the click, slid left/up as needed to stay on-screen.
	pub fn panel(&self, vw: f32, vh: f32) -> Rect {
		let w = items_width(&self.items).min((vw - 8.0).max(60.0));
		let h = items_height(&self.items);
		let x = self.pos.0.min((vw - w - 4.0).max(0.0));
		// Flip above the cursor when there's no room below (but never off-top).
		let y = if self.pos.1 + h > vh { (self.pos.1 - h).max(0.0) } else { self.pos.1 };
		Rect::new(x, y, w, h)
	}

	/// A press: `Run`/`Todo` for a leaf, `Consumed` for panel padding,
	/// `None` off the panel (the shell closes it either way).
	pub fn on_press(&self, x: f32, y: f32, vw: f32, vh: f32) -> Press {
		let panel = self.panel(vw, vh);
		if let Some(i) = item_at(&self.items, panel, x, y) {
			return MenuBar::leaf_press(&self.items[i]);
		}
		if panel.contains(x, y) { Press::Consumed } else { Press::None }
	}

	/// Hover tracking; true when a redraw is needed.
	pub fn on_move(&mut self, x: f32, y: f32, vw: f32, vh: f32) -> bool {
		let hover = item_at(&self.items, self.panel(vw, vh), x, y);
		let changed = self.hover != hover;
		self.hover = hover;
		changed
	}

	pub fn draw(&self, w: f32, h: f32, checked: &dyn Fn(&str) -> bool) -> UiQuads {
		let mut q = UiQuads::default();
		draw_panel(&mut q, &self.items, self.panel(w, h), self.hover, None, checked, w, h);
		q
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::PathBuf;

	fn maps_dir() -> PathBuf {
		Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/templates")
	}

	fn bar() -> MenuBar {
		MenuBar::new(&maps_dir())
	}

	/// Every Action in the tree must parse — a typo'd menu command should
	/// fail this test, not a click at runtime (same rule as keybindings).
	#[test]
	fn every_action_parses() {
		fn check(items: &[Item], path: &str) {
			for it in items {
				match it {
					Item::Action { label, command, .. } | Item::Toggle { label, command, .. } => {
						crate::command::parse_line(command)
							.unwrap_or_else(|e| panic!("{path}/{label}: {e}"))
							.unwrap_or_else(|| panic!("{path}/{label}: empty command"));
					}
					Item::Sub { label, items } => check(items, &format!("{path}/{label}")),
					_ => {}
				}
			}
		}
		let b = bar();
		assert_eq!(b.menus.len(), 10, "the design's ten menus");
		for m in &b.menus {
			check(&m.items, m.title);
		}
	}

	#[test]
	fn quick_load_lists_projects() {
		let b = bar();
		let file = &b.menus[0];
		let Item::Sub { items, .. } = &file.items[3] else { panic!("Quick Load submenu") };
		assert!(items.len() >= 24, "the 24 converted stock maps");
		assert!(matches!(&items[0], Item::Action { command, .. } if command.starts_with("open! ")));
	}

	#[test]
	fn press_flow_open_run_close() {
		let mut b = bar();
		// Click "Edit" in the bar → opens.
		let t = b.title_cell(1);
		assert_eq!(b.on_press(t.x + 2.0, t.y + 2.0, 1280.0), Press::Consumed);
		assert_eq!(b.open, Some(1));
		// Click "Undo" (first item) → Run + closes.
		let d = b.dropdown_rect(1, 1280.0);
		let r = item_rect(&b.menus[1].items, d.x, d.y, d.w, 0);
		assert_eq!(b.on_press(r.x + 4.0, r.y + 4.0, 1280.0), Press::Run("undo".into()));
		assert_eq!(b.open, None);
		// Re-open, click a Todo → Todo with its ticket + closes.
		b.open_by_name("edit").unwrap();
		let r = item_rect(&b.menus[1].items, d.x, d.y, d.w, 2);
		match b.on_press(r.x + 4.0, r.y + 4.0, 1280.0) {
			Press::Todo(label, ticket) => {
				assert_eq!(label, "Undo History");
				assert_eq!(ticket, "CORE-15");
			}
			other => panic!("expected Todo, got {other:?}"),
		}
		assert_eq!(b.open, None);
		// Open, then click far away → closed + swallowed.
		b.open_by_name("file").unwrap();
		assert_eq!(b.on_press(900.0, 500.0, 1280.0), Press::Consumed);
		assert_eq!(b.open, None);
		// Nothing open: a click below the bar falls through.
		assert_eq!(b.on_press(900.0, 500.0, 1280.0), Press::None);
		// A bar click outside every title is swallowed only when open.
		assert_eq!(b.on_press(5000.0, 10.0, 1280.0), Press::None);
	}

	#[test]
	fn submenu_opens_and_runs() {
		let mut b = bar();
		b.open_by_name("view").unwrap();
		// Click the "Zoom" submenu parent → opens beside.
		let d = b.dropdown_rect(2, 1280.0);
		let zoom = item_rect(&b.menus[2].items, d.x, d.y, d.w, 0);
		assert_eq!(b.on_press(zoom.x + 4.0, zoom.y + 4.0, 1280.0), Press::Consumed);
		assert_eq!(b.sub_open, Some(0));
		let panel = b.submenu_rect(2, 0, 1280.0).unwrap();
		let Item::Sub { items, .. } = &b.menus[2].items[0] else { unreachable!() };
		let fit = item_rect(items, panel.x, panel.y, panel.w, 3);
		assert_eq!(b.on_press(fit.x + 4.0, fit.y + 4.0, 1280.0), Press::Run("fit".into()));
		assert_eq!(b.open, None, "running a leaf closes everything");
	}

	#[test]
	fn hover_switches_menus_and_opens_submenus() {
		let mut b = bar();
		b.open_by_name("file").unwrap();
		// Sliding to "Edit" on the bar switches.
		let t = b.title_cell(1);
		assert!(b.on_move(t.x + 2.0, t.y + 2.0, 1280.0));
		assert_eq!(b.open, Some(1));
		// Hovering a submenu parent opens it (View/Zoom).
		b.open_by_name("view").unwrap();
		let d = b.dropdown_rect(2, 1280.0);
		let zoom = item_rect(&b.menus[2].items, d.x, d.y, d.w, 0);
		assert!(b.on_move(zoom.x + 4.0, zoom.y + 4.0, 1280.0));
		assert_eq!(b.sub_open, Some(0));
		// Hovering a plain item closes the submenu again.
		let grid = item_rect(&b.menus[2].items, d.x, d.y, d.w, 1);
		assert!(b.on_move(grid.x + 4.0, grid.y + 4.0, 1280.0));
		assert_eq!(b.sub_open, None);
	}

	#[test]
	fn dropdowns_stay_inside_the_viewport() {
		// A narrow window: the widest dropdown must cap its width and slide
		// left rather than hang off the right edge.
		let b = bar();
		for (i, m) in b.menus.iter().enumerate() {
			for vw in [200.0f32, 480.0, 1280.0] {
				let d = b.dropdown_rect(i, vw);
				assert!(d.x >= 0.0, "{}: x {} < 0 at vw {vw}", m.title, d.x);
				assert!(d.x + d.w <= vw, "{}: right edge {} > vw {vw}", m.title, d.x + d.w);
			}
		}
		// Submenus too (File ▸ Quick Load is the widest).
		let mut b = bar();
		b.open_by_name("file").unwrap();
		let panel = b.submenu_rect(0, 3, 480.0).expect("Quick Load submenu");
		assert!(panel.x >= 0.0 && panel.x + panel.w <= 480.0, "submenu off-screen: {panel:?}");
	}

	#[test]
	fn shortcuts_stamp_hints_and_widen_rows() {
		let mut b = bar();
		b.apply_shortcuts(&[("cut".into(), "Ctrl+X".into()), ("zoom-to 1".into(), "1".into())]);
		// Edit ▸ Cut gets its chord; unbound items stay clean.
		let Item::Action { hint, .. } = &b.menus[1].items[4] else { panic!("Edit/Cut") };
		assert_eq!(hint.as_deref(), Some("Ctrl+X"));
		let Item::Action { hint, .. } = &b.menus[1].items[5] else { panic!("Edit/Copy") };
		assert_eq!(hint.as_deref(), None);
		// Hints reach into submenus (View ▸ Zoom ▸ 100%).
		let Item::Sub { items, .. } = &b.menus[2].items[0] else { panic!("View/Zoom") };
		let Item::Action { hint, .. } = &items[0] else { panic!("Zoom/100%") };
		assert_eq!(hint.as_deref(), Some("1"));
		// The dropdown budgets label + gap + hint (past the 120px floor).
		let plain = items_width(&[act("Undo History Browser", "undo")]);
		let hinted = items_width(&[Item::Action {
			label: "Undo History Browser".into(),
			command: "undo".into(),
			hint: Some("Ctrl+Shift+Z".into()),
		}]);
		assert!(hinted > plain, "hint widens the panel: {hinted} vs {plain}");
	}

	#[test]
	fn context_menu_resolves_presses_and_stays_on_screen() {
		let items = vec![act("Select All", "select all"), Item::Sep, act("Fit Map", "fit")];
		let cm = ContextMenu::new(items, (100.0, 100.0));
		let panel = cm.panel(1280.0, 800.0);
		assert_eq!((panel.x, panel.y), (100.0, 100.0), "fits: opens at the click");
		let r = item_rect(&cm.items, panel.x, panel.y, panel.w, 0);
		assert_eq!(cm.on_press(r.x + 4.0, r.y + 4.0, 1280.0, 800.0), Press::Run("select all".into()));
		// Panel padding is consumed; off the panel falls out (the shell
		// closes the menu either way).
		assert_eq!(cm.on_press(panel.x + 2.0, panel.y + 1.0, 1280.0, 800.0), Press::Consumed);
		assert_eq!(cm.on_press(900.0, 700.0, 1280.0, 800.0), Press::None);
		// Near the bottom-right corner it slides left and flips above.
		let cm = ContextMenu::new(vec![act("Fit Map", "fit")], (1275.0, 795.0));
		let p = cm.panel(1280.0, 800.0);
		assert!(p.x + p.w <= 1280.0 && p.y + p.h <= 800.0, "off-screen: {p:?}");
	}

	#[test]
	fn open_by_name_validates() {
		let mut b = bar();
		assert!(b.open_by_name("tools").is_ok());
		assert_eq!(b.open, Some(7));
		assert!(b.open_by_name("off").is_ok());
		assert_eq!(b.open, None);
		assert!(b.open_by_name("nonsense").is_err());
	}
}
