//! Main menu bar: the ten menus from the design
//! (`designs/features.drawio`, "Main menu" page - Layers was promoted out of
//! Mode into its own menu). Every leaf is either an
//! **Action** - a command line through the command parser, exactly like a
//! keybinding - a **Toggle** (checkbox), a **Todo** placeholder that echoes its
//! backlog ticket, or a **Disabled** row (drawn dim, inert) - the last two both
//! drawn dim, so the unbuilt surface area is visible but honest.
//!
//! Pure geometry/state - the shell routes presses here first (menus are
//! topmost); `menu NAME|off` drives it from scripts for screenshots.

use std::path::PathBuf;

pub const BAR_H: f32 = 24.0;

pub enum Item {
	/// Runs a command line (validated by a test against the parser).
	Action {
		label: String,
		command: String,
		/// Keyboard shortcut label (`"Ctrl+C"`) - drawn right-aligned, dim.
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
	/// Not built yet - echoes the backlog ticket when clicked.
	Todo {
		label: String,
		ticket: &'static str,
	},
	/// A permanently-disabled row: drawn dim and inert (a planned tool that is
	/// still a no-op - unlike [`Item::Todo`] it doesn't even echo a ticket).
	Disabled {
		label: String,
	},
	Sep,
	/// Opens a side submenu.
	Sub {
		label: String,
		items: Vec<Item>,
	},
	/// Opens a side submenu laid out as labelled columns (Template Maps). Each
	/// column is a header over a stack of [`Item::Action`] rows.
	Columns {
		label: String,
		columns: Vec<Column>,
	},
}

/// One column of an [`Item::Columns`] submenu.
pub struct Column {
	pub header: String,
	pub items: Vec<Item>,
}

fn act(label: &str, command: &str) -> Item {
	Item::Action { label: label.into(), command: command.into(), hint: None }
}

/// An action wired to a keybindable registry action by its [`ACTIONS`] id
/// ([`crate::input`]): the command line comes from the registry, the single
/// definition shared with the keyboard shortcut - so the menu item and its
/// shortcut run the same thing and can't drift. Use this (not [`act`]) for any
/// menu item that also has a keybinding.
fn act_id(label: &str, id: &str) -> Item {
	Item::Action { label: label.into(), command: crate::input::action_command(id).into(), hint: None }
}

/// A checkbox item: runs `command`, shows checked when `key` resolves true.
fn toggle(label: &str, command: &str, key: &'static str) -> Item {
	Item::Toggle { label: label.into(), command: command.into(), key, hint: None }
}

/// [`act_id`] for a checkbox item: the command line comes from the registry
/// (shared with the keyboard shortcut), `key` resolves the checkmark.
fn toggle_id(label: &str, id: &str, key: &'static str) -> Item {
	Item::Toggle { label: label.into(), command: crate::input::action_command(id).into(), key, hint: None }
}

fn todo(label: &str, ticket: &'static str) -> Item {
	Item::Todo { label: label.into(), ticket }
}

fn disabled(label: &str) -> Item {
	Item::Disabled { label: label.into() }
}

fn sub(label: &str, items: Vec<Item>) -> Item {
	Item::Sub { label: label.into(), items }
}

/// One Quick Load / Template Maps row: the label, an optional right-aligned
/// note (Template Maps puts the file name here), and the file it opens.
pub struct MapEntry {
	pub label: String,
	pub note: Option<String>,
	pub path: PathBuf,
}

/// A Quick Load-style submenu - one `open!` action per entry, or a single dim
/// placeholder when the list is empty. A `note` rides the right-aligned hint
/// column (the file name, for Template Maps).
fn quick_items(entries: &[MapEntry], empty: &'static str) -> Vec<Item> {
	if entries.is_empty() {
		return vec![Item::Todo { label: empty.into(), ticket: "" }];
	}
	entries
		.iter()
		.map(|e| Item::Action {
			label: e.label.clone(),
			command: format!("open! \"{}\"", e.path.display()),
			hint: e.note.clone(),
		})
		.collect()
}

/// The **Template Maps** File-menu item: a columnar submenu grouped by terrain
/// (Crater / Desert / Green / Snow) plus a Demo column for the `*_I` maps. Falls
/// back to a plain placeholder submenu when no maps are installed. Template-map
/// rows carry no right-aligned file name (`hint: None`) - the column header is
/// the context.
fn template_maps_item(entries: &[MapEntry]) -> Item {
	if entries.is_empty() {
		return sub("Template Maps", quick_items(entries, "(no template maps)"));
	}
	const HEADERS: [&str; 5] = ["Crater", "Desert", "Green", "Snow", "Demo"];
	let mut columns: Vec<Column> =
		HEADERS.iter().map(|h| Column { header: (*h).to_string(), items: Vec::new() }).collect();
	for e in entries {
		let stem = e.path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_ascii_uppercase();
		// `*_I` maps are demos regardless of terrain; the rest sort by prefix.
		let col = if stem.ends_with("_I") {
			4
		} else if stem.starts_with("CRATER") {
			0
		} else if stem.starts_with("DESERT") {
			1
		} else if stem.starts_with("GREEN") {
			2
		} else if stem.starts_with("SNOW") {
			3
		} else {
			continue;
		};
		columns[col].items.push(Item::Action {
			label: e.label.clone(),
			command: format!("open! \"{}\"", e.path.display()),
			hint: None,
		});
	}
	Item::Columns { label: "Template Maps".to_string(), columns }
}

pub struct Menu {
	pub title: &'static str,
	pub items: Vec<Item>,
}

/// The menu **model**: the item tree (labels, command lines, live toggle
/// keys, shortcut hints) the wgpu-ui `MenuBar` widget is built from via
/// [`MenuBar::build_bar`]. Open/hover state lives in the widget; this stays
/// the single source the editor mutates (dev menu, Quick Load, hints) —
/// rebuild the widget after a structure change.
pub struct MenuBar {
	pub menus: Vec<Menu>,
}

impl MenuBar {
	/// The full design tree. `maps_dir` feeds the Quick Load submenu.
	/// Add the developer-only **DEV** menu (last in the bar) when `--dev` is set.
	/// Called once at startup; idempotent.
	pub fn set_dev(&mut self, dev: bool, packs: &[String]) {
		if !dev || self.menus.iter().any(|m| m.title == "DEV") {
			return;
		}
		let mut items = vec![
			act("Bake to Asset Packs", "bake"),
			act("Update Map", "update-map"),
			act("Edit Match Data...", "match-editor"),
			act("UI Tests...", "ui-tests"),
		];
		// One entry per installed tileset: lay its whole tiles.match.json out as
		// match-editor crosses on a fresh map for visual inspection.
		if !packs.is_empty() {
			let combos = packs.iter().map(|p| act(p, &format!("match-combos {p}"))).collect();
			items.push(sub("Match Combinations Map", combos));
		}
		self.menus.push(Menu { title: "DEV", items });
	}

	/// Refresh the **Quick Load** submenu with the current recently-opened maps
	/// (called when a map opens). No-op if the File ▸ Quick Load sub is gone.
	pub fn set_recent(&mut self, recent: &[MapEntry]) {
		let items = quick_items(recent, "(no recent maps)");
		if let Some(file) = self.menus.iter_mut().find(|m| m.title == "File") {
			for item in &mut file.items {
				if let Item::Sub { label, items: sub_items } = item {
					if label == "Quick Load" {
						*sub_items = items;
						return;
					}
				}
			}
		}
	}

	/// Refresh the **Edit ▸ Undo History** submenu with the most recent undo
	/// actions (newest first); clicking one jumps back that many steps. Empty →
	/// a single dim placeholder. No-op if the Edit ▸ Undo History sub is gone.
	pub fn set_undo_history(&mut self, labels: &[String]) {
		let items: Vec<Item> = if labels.is_empty() {
			vec![Item::Disabled { label: "(nothing to undo)".into() }]
		} else {
			labels
				.iter()
				.enumerate()
				.map(|(i, l)| Item::Action {
					label: l.clone(),
					// Undo through this action (it + everything newer): i+1 steps.
					command: format!("undo-to {}", i + 1),
					hint: None,
				})
				.collect()
		};
		if let Some(edit) = self.menus.iter_mut().find(|m| m.title == "Edit") {
			for item in &mut edit.items {
				if let Item::Sub { label, items: sub_items } = item {
					if label == "Undo History" {
						*sub_items = items;
						return;
					}
				}
			}
		}
	}

	/// `templates` feeds the **Template Maps** submenu (stock starter maps);
	/// `recent` feeds **Quick Load** (the user's recently-opened maps, kept in
	/// sync afterwards via [`Self::set_recent`]).
	pub fn new(templates: &[MapEntry], recent: &[MapEntry]) -> Self {
		let menus = vec![
			Menu {
				title: "File",
				items: vec![
					act_id("New Map...", "NewMap"),
					act("New from Image...", "file-dialog new-from-image"),
					act("New Terrain from Image...", "file-dialog new-map-shape"),
					act_id("Load Map...", "FileDialogLoad"),
					sub("Quick Load", quick_items(recent, "(no recent maps)")),
					template_maps_item(templates),
					Item::Sep,
					// The save-editor tools live behind an "Experimental" gate (they can
					// break real saves — the Open action warns first). Single separators
					// fence the submenu off from Template Maps above and Save below.
					sub(
						"Experimental",
						vec![
							act_id("Open Save File...", "OpenSaveFile"),
							act_id("New Save From Map", "NewSaveFromMap"),
							act_id("Export Save File...", "ExportSaveFile"),
							// One click writes both game files (WRL + save) via two pickers.
							act("Export to WRL and Save File...", "file-dialog export-wrl-and-save"),
						],
					),
					Item::Sep,
					act_id("Save Project", "SaveProject"),
					act_id("Save Project As...", "FileDialogSaveAs"),
					act("Save Project Copy...", "file-dialog save-copy"),
					act_id("Close Project", "CloseProject"),
					Item::Sep,
					act_id("Export to WRL...", "Export"),
					act("Import WRL...", "file-dialog import-wrl"),
					Item::Sep,
					todo("Export as Image...", "IO-5"),
					Item::Sep,
					act("Exit", "quit-request"),
				],
			},
			Menu {
				title: "Edit",
				items: vec![
					act_id("Undo", "Undo"),
					act_id("Redo", "Redo"),
					// Rebuilt each frame from the undo stack (`set_undo_history`).
					sub("Undo History", vec![disabled("(nothing to undo)")]),
					Item::Sep,
					act_id("Cut", "Cut"),
					act_id("Copy", "Copy"),
					act_id("Paste", "Paste"),
					Item::Sep,
					act_id("Clear", "Delete"),
					act_id("Clear All Layers", "DeleteAll"),
					Item::Sep,
					act("Map Metadata...", "map-metadata"),
					Item::Sep,
					// Separator-fenced like File's Experimental block: save editing
					// can corrupt real saves, so it never sits flush against the
					// everyday items.
					sub("Experimental", vec![act_id("Edit Save Data...", "EditSaveData")]),
					Item::Sep,
					act("Editor Preferences...", "editor-preferences"),
				],
			},
			Menu {
				title: "View",
				items: vec![
					sub(
						"Overlays",
						vec![
							toggle_id("Grid", "GridToggle", "grid"),
							toggle_id("Passage", "PassOverlayToggle", "pass-overlay"),
							toggle_id("Units", "UnitsToggle", "show-units"),
							toggle_id("Resources", "ResourcesToggle", "resources"),
						],
					),
					Item::Sep,
					sub(
						"Map Zoom",
						vec![
							act_id("100%", "ZoomTo100"),
							act_id("50%", "ZoomTo50"),
							act_id("25%", "ZoomTo25"),
							act_id("Fit All", "Fit"),
							todo("Custom...", "UI-7"),
						],
					),
					Item::Sep,
					sub(
						"User Interface",
						vec![
							toggle("Small", "ui-scale small", "ui-scale:small"),
							toggle("Medium (125%)", "ui-scale medium", "ui-scale:medium"),
							toggle("Large (150%)", "ui-scale large", "ui-scale:large"),
						],
					),
					Item::Sep,
					toggle("Status Bar", "status-bar toggle", "status-bar"),
				],
			},
			Menu {
				title: "Mode",
				items: vec![
					toggle("Map Editor", "mode map", "mode:map"),
					toggle("Pass Table Editor", "mode pass", "mode:pass"),
					toggle("Local Pass Override Editor", "mode localpass", "mode:localpass"),
					// The save editor is experimental (it can break real saves), so it
					// sits behind its own gate - and, like the modes above, owns its
					// own dock layout.
					sub("Experimental", vec![toggle("Save Editor", "mode save", "mode:save")]),
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
			// Which layer the tools address. Its own menu rather than a Mode
			// submenu: it is a per-edit choice, not a mode switch, and the tick
			// that says what the pencil is pointed at is worth one click.
			Menu {
				title: "Layers",
				items: vec![
					toggle("Water", "layer water", "layer:water"),
					toggle("Ground", "layer ground", "layer:ground"),
					// The free-placed cut-outs (SCENERY.md). Not a tile layer -
					// picking it re-points the pencil, the eraser and the arrow
					// at the scenery list.
					toggle("Scenery", "layer scenery", "layer:scenery"),
					Item::Sep,
					toggle("Show Only Selected", "show-only-layer toggle", "layer:only-selected"),
					Item::Sep,
					// Unit-layer visibility (the same registry toggle as View ▸ Overlays ▸ Units).
					toggle_id("Units", "UnitsToggle", "show-units"),
				],
			},
			Menu {
				title: "Select",
				items: vec![
					act_id("Select All", "SelectAll"),
					act_id("Invert Selection", "SelectInvert"),
					act_id("Clear Selection", "SelectClear"),
					Item::Sep,
					// Add/subtract are drag modifiers: Shift+drag adds,
					// Ctrl+drag subtracts (with the select tools active).
					act_id("Select Tool", "ToolSelect"),
					act_id("Rect Select Tool", "ToolSelectRect"),
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
							act("Auto Fix...", "fix-shore-modal go"),
							toggle("Show Shore Bugs", "shore-bugs toggle", "shore-bugs"),
						],
					),
					sub("Validate", vec![toggle("Show Problems", "match-problems toggle", "match-problems")]),
					sub(
						"Passage Table",
						vec![disabled("Auto-Generate Passage"), act("Reset to Tileset Passage", "tile-pass-reset")],
					),
					sub(
						"Palette",
						vec![
							act("Convert to Compatible Palette...", "convert-palette-modal"),
							toggle("Render with Map Palette", "map-palette toggle", "debug:map-palette"),
						],
					),
					sub("Randomizers", vec![act("Generate Random Terrain...", "generate-modal")]),
					sub("Map", vec![act("Resize Map...", "resize-modal")]),
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
							toggle("WRL Internal Palette", "window wrlpalette", "win:wrlpalette"),
							toggle("Toolbox", "window toolbox", "win:toolbox"),
							toggle("Save Toolbox", "window savetools", "win:savetools"),
							toggle("Pass Types Palette", "window passtools", "win:passtools"),
							toggle("Unit Properties", "window unitprops", "win:unitprops"),
							toggle("Units", "window units", "win:units"),
							toggle("Templates Explorer", "window templates", "win:templates"),
							toggle("Scenery", "window scenery", "win:scenery"),
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
					act("User Manual", "help-manual"),
					Item::Sep,
					act("Go to Website", "open-url https://suns-echoes.github.io/max-map-editor/"),
					act("Go to Project GitHub", "open-url https://github.com/suns-echoes/max-map-editor"),
					Item::Sep,
					act("About...", "about"),
				],
			},
		];
		Self { menus }
	}

	/// Open a menu by title (case-insensitive) - the `menu` command.
	/// Stamp a shortcut hint onto every Action/Toggle whose command `resolve`s to
	/// one. `resolve` is the shell's single hint resolver (config binding, alias,
	/// or fixed shell shortcut), so no item is wired up by hand. Called once at
	/// startup after the bindings load.
	pub fn apply_shortcuts(&mut self, resolve: &dyn Fn(&str) -> Option<String>) {
		fn walk(items: &mut [Item], resolve: &dyn Fn(&str) -> Option<String>) {
			for item in items {
				match item {
					// Only overwrite when the command resolves to a chord, so a
					// pre-set hint (e.g. a Template Maps file name) survives.
					Item::Action { command, hint, .. } | Item::Toggle { command, hint, .. } => {
						if let Some(label) = resolve(command) {
							*hint = Some(label);
						}
					}
					Item::Sub { items, .. } => walk(items, resolve),
					_ => {}
				}
			}
		}
		for menu in &mut self.menus {
			walk(&mut menu.items, resolve);
		}
	}

	/// Build the wgpu-ui menu-bar widget from this model. Every leaf gets a
	/// sequential action id into the returned act table; toggle leaves also
	/// map their id to the live state key the shell re-syncs each frame
	/// (`wgpu_ui::MenuBar::set_checked`).
	pub fn build_bar(&self) -> (wgpu_ui::MenuBar, Vec<Act>, Vec<(u64, &'static str)>) {
		let mut acts = Vec::new();
		let mut toggles = Vec::new();
		// The bar fits the chrome strip the workspace reserves (BAR_H).
		let mut bar = wgpu_ui::MenuBar::new().bar_height(BAR_H);
		for m in &self.menus {
			bar = bar.menu(m.title, build_items(&m.items, &mut acts, &mut toggles));
		}
		(bar, acts, toggles)
	}
}

/// What a fired menu action runs, indexed by the widget item's action id.
pub enum Act {
	/// A command line (parsed + run by the shell).
	Run(String),
	/// Not built yet — echoes the backlog ticket to the console.
	Todo(String, &'static str),
}

fn build_items(items: &[Item], acts: &mut Vec<Act>, toggles: &mut Vec<(u64, &'static str)>) -> Vec<wgpu_ui::MenuItem> {
	items
		.iter()
		.map(|it| match it {
			Item::Action { label, command, hint } => {
				let id = acts.len() as u64;
				acts.push(Act::Run(command.clone()));
				let mut m = wgpu_ui::MenuItem::item(label.clone(), id);
				if let Some(h) = hint {
					m = m.shortcut(h.clone());
				}
				m
			}
			Item::Toggle { label, command, key, hint } => {
				let id = acts.len() as u64;
				acts.push(Act::Run(command.clone()));
				toggles.push((id, key));
				let mut m = wgpu_ui::MenuItem::item(label.clone(), id).checked(false);
				if let Some(h) = hint {
					m = m.shortcut(h.clone());
				}
				m
			}
			Item::Todo { label, ticket } => {
				let id = acts.len() as u64;
				acts.push(Act::Todo(label.clone(), ticket));
				wgpu_ui::MenuItem::item(label.clone(), id)
			}
			Item::Disabled { label } => {
				// Keeps an act slot (ids stay dense) but is inert - disabled rows
				// never fire, so the slot is never dispatched.
				let id = acts.len() as u64;
				acts.push(Act::Todo(label.clone(), "unavailable"));
				wgpu_ui::MenuItem::item(label.clone(), id).enabled(false)
			}
			Item::Sep => wgpu_ui::MenuItem::separator(),
			Item::Sub { label, items } => wgpu_ui::MenuItem::sub(label.clone(), build_items(items, acts, toggles)),
			Item::Columns { label, columns } => wgpu_ui::MenuItem::columns(
				label.clone(),
				columns.iter().map(|c| (c.header.clone(), build_items(&c.items, acts, toggles))).collect(),
			),
		})
		.collect()
}

/// The right-click context menu MODEL: an items snapshot (built from the
/// editor state at open time - `state::context_menu_items` /
/// `open_template_context_menu`) plus the anchor (logical px). The view is a
/// `wgpu_ui::ContextMenu` hosted shell-side in its own `Ui`; the shell syncs
/// it from this model and maps fired action ids back through
/// [`build_context`]'s act table.
pub struct ContextMenu {
	pub items: Vec<Item>,
	/// The click position (logical px; the widget clamps on-screen).
	pub pos: (f32, f32),
}

impl ContextMenu {
	pub fn new(items: Vec<Item>, pos: (f32, f32)) -> Self {
		Self { items, pos }
	}
}

/// Build the wgpu-ui item list + act table for a context-menu snapshot (the
/// same sequential-id scheme as [`MenuBar::build_bar`]).
pub fn build_context(items: &[Item]) -> (Vec<wgpu_ui::MenuItem>, Vec<Act>) {
	let mut acts = Vec::new();
	let mut toggles = Vec::new(); // context items carry no live toggles today
	let built = build_items(items, &mut acts, &mut toggles);
	(built, acts)
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::PathBuf;

	fn maps_dir() -> PathBuf {
		PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources/assets/maps")
	}

	/// The shipped maps as Template Maps entries (label = file stem; the real app
	/// reads each map's name, but the menu structure is all these tests check).
	fn template_entries() -> Vec<MapEntry> {
		let mut paths: Vec<PathBuf> = std::fs::read_dir(maps_dir())
			.unwrap()
			.filter_map(|e| e.ok())
			.map(|e| e.path())
			.filter(|p| p.extension().is_some_and(|x| x == "json"))
			.collect();
		paths.sort();
		paths
			.into_iter()
			.map(|path| {
				let label = path.file_stem().unwrap().to_string_lossy().into_owned();
				MapEntry { label, note: None, path }
			})
			.collect()
	}

	fn bar() -> MenuBar {
		MenuBar::new(&template_entries(), &[])
	}

	/// Every Action in the tree must parse - a typo'd menu command should
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
					Item::Columns { label, columns } => {
						for col in columns {
							check(&col.items, &format!("{path}/{label}/{}", col.header));
						}
					}
					_ => {}
				}
			}
		}
		let b = bar();
		assert_eq!(
			b.menus.iter().map(|m| m.title).collect::<Vec<_>>(),
			["File", "Edit", "View", "Mode", "Layers", "Select", "Templates", "Tools", "Windows", "Help"],
		);
		for m in &b.menus {
			check(&m.items, m.title);
		}
	}

	/// Both save-editor entries live under File ▸ Experimental (item 7), fenced by
	/// separators (items 6 and 8), and reach the registry actions by id.
	#[test]
	fn experimental_submenu_holds_the_save_editor_entries() {
		let b = bar();
		let file = &b.menus[0];
		assert!(matches!(&file.items[6], Item::Sep), "separator before Experimental");
		assert!(matches!(&file.items[8], Item::Sep), "separator after Experimental");
		let Item::Sub { label, items } = &file.items[7] else { panic!("Experimental submenu") };
		assert_eq!(label, "Experimental");
		let labels: Vec<&str> = items
			.iter()
			.filter_map(|it| match it {
				Item::Action { label, .. } => Some(label.as_str()),
				_ => None,
			})
			.collect();
		assert_eq!(
			labels,
			["Open Save File...", "New Save From Map", "Export Save File...", "Export to WRL and Save File..."]
		);
		assert!(
			matches!(&items[0], Item::Action { command, .. } if command == crate::input::action_command("OpenSaveFile"))
		);
		assert!(
			matches!(&items[1], Item::Action { command, .. } if command == crate::input::action_command("NewSaveFromMap"))
		);
		assert!(
			matches!(&items[2], Item::Action { command, .. } if command == crate::input::action_command("ExportSaveFile"))
		);
		// The combo item runs the two-picker file-dialog flow.
		assert!(matches!(&items[3], Item::Action { command, .. } if command == "file-dialog export-wrl-and-save"));
	}

	#[test]
	fn template_maps_columns_group_the_stock_maps() {
		let b = bar();
		let file = &b.menus[0];
		// items: New(0) NewImage(1) NewTerrain(2) Load(3) QuickLoad(4) TemplateMaps(5) Sep(6)
		// Experimental(7) ...
		let Item::Columns { label, columns } = &file.items[5] else { panic!("Template Maps columns") };
		assert_eq!(label, "Template Maps");
		assert_eq!(
			columns.iter().map(|c| c.header.as_str()).collect::<Vec<_>>(),
			["Crater", "Desert", "Green", "Snow", "Demo"],
		);
		let total: usize = columns.iter().map(|c| c.items.len()).sum();
		assert!(total >= 24, "the stock maps spread across the columns");
		// Every cell is an `open!` action.
		for col in columns {
			for it in &col.items {
				assert!(matches!(it, Item::Action { command, .. } if command.starts_with("open! ")));
			}
		}
		// The `*_I` maps land in the Demo column, not their terrain column.
		let demo = columns.iter().find(|c| c.header == "Demo").unwrap();
		assert!(demo.items.iter().any(|it| matches!(it, Item::Action { command, .. } if command.contains("_I"))));
		let crater = columns.iter().find(|c| c.header == "Crater").unwrap();
		assert!(
			crater.items.iter().all(|it| matches!(it, Item::Action { command, .. } if !command.contains("_I"))),
			"a demo map must not appear in the Crater column",
		);
	}

	/// With no installed maps the Template Maps item degrades to a plain
	/// submenu holding one dim placeholder (no empty five-column grid); a map
	/// whose stem matches no terrain prefix is silently left out of the grid.
	#[test]
	fn template_maps_handles_no_maps_and_unknown_stems() {
		let b = MenuBar::new(&[], &[]);
		let Item::Sub { label, items } = &b.menus[0].items[5] else { panic!("empty entries fall back to a Sub") };
		assert_eq!(label, "Template Maps");
		assert!(matches!(&items[0], Item::Todo { label, .. } if label == "(no template maps)"));

		// A stem outside CRATER/DESERT/GREEN/SNOW/_I lands in no column.
		let odd = MapEntry { label: "odd".into(), note: None, path: PathBuf::from("/maps/OTHER_1.json") };
		let b = MenuBar::new(&[odd], &[]);
		let Item::Columns { columns, .. } = &b.menus[0].items[5] else { panic!("non-empty entries build columns") };
		assert_eq!(columns.iter().map(|c| c.items.len()).sum::<usize>(), 0, "unknown stems are skipped");
	}

	/// `--dev` appends the DEV menu once (idempotent), with the per-tileset
	/// Match Combinations submenu only when packs are installed; without
	/// `--dev` the bar is untouched.
	#[test]
	fn set_dev_appends_the_dev_menu_once() {
		let mut b = MenuBar::new(&[], &[]);
		let plain = b.menus.len();
		b.set_dev(false, &["GREEN".into()]);
		assert_eq!(b.menus.len(), plain, "no --dev: no DEV menu");

		b.set_dev(true, &["GREEN".into(), "SNOW".into()]);
		let dev = b.menus.last().expect("DEV appended");
		assert_eq!(dev.title, "DEV");
		let Some(Item::Sub { label, items }) = dev.items.last() else { panic!("Match Combinations submenu") };
		assert_eq!(label, "Match Combinations Map");
		assert!(
			matches!(&items[0], Item::Action { command, .. } if command == "match-combos GREEN"),
			"one entry per installed tileset",
		);
		b.set_dev(true, &["GREEN".into()]);
		assert_eq!(b.menus.iter().filter(|m| m.title == "DEV").count(), 1, "set_dev is idempotent");

		// No installed packs: the DEV menu still appears, without the submenu.
		let mut b = MenuBar::new(&[], &[]);
		b.set_dev(true, &[]);
		let dev = b.menus.last().unwrap();
		assert!(!dev.items.iter().any(|i| matches!(i, Item::Sub { .. })), "no packs -> no combos submenu");
	}

	/// `set_recent` quietly does nothing on a bar without a File ▸ Quick Load
	/// submenu (it walks past other subs rather than clobbering them).
	#[test]
	fn set_recent_without_a_quick_load_sub_is_a_no_op() {
		let mut b = MenuBar { menus: vec![Menu { title: "File", items: vec![sub("Other", vec![act("A", "fit")])] }] };
		b.set_recent(&[MapEntry { label: "m.json".into(), note: None, path: PathBuf::from("/m.json") }]);
		let Item::Sub { label, items } = &b.menus[0].items[0] else { panic!("sub kept") };
		assert_eq!(label, "Other", "the non-Quick-Load sub is untouched");
		assert!(matches!(&items[0], Item::Action { label, .. } if label == "A"));
	}

	#[test]
	fn quick_load_starts_empty_then_set_recent_fills_it() {
		let mut b = bar();
		let Item::Sub { label, items } = &b.menus[0].items[4] else { panic!("Quick Load submenu") };
		assert_eq!(label, "Quick Load");
		assert!(matches!(&items[0], Item::Todo { .. }), "empty Quick Load shows a placeholder");

		b.set_recent(&[MapEntry { label: "my.json".into(), note: None, path: PathBuf::from("/maps/my.json") }]);
		let Item::Sub { items, .. } = &b.menus[0].items[4] else { panic!("Quick Load submenu") };
		assert!(matches!(&items[0], Item::Action { command, .. } if command == "open! \"/maps/my.json\""));
	}

	#[test]
	fn bound_menu_items_use_the_registry_command_line() {
		// Menu items that also have a keyboard shortcut pull their command line
		// from the shared registry (via `act_id`/`toggle_id`), so the menu item
		// and the shortcut can't drift. Pins the wiring for the ones that matter.
		fn find(items: &[Item], label: &str) -> Option<String> {
			for it in items {
				match it {
					Item::Action { label: l, command, .. } | Item::Toggle { label: l, command, .. } if l == label => {
						return Some(command.clone());
					}
					Item::Sub { items, .. } => {
						if let Some(c) = find(items, label) {
							return Some(c);
						}
					}
					_ => {}
				}
			}
			None
		}
		let b = bar();
		let menu_command = |label: &str| b.menus.iter().find_map(|m| find(&m.items, label));
		for (label, id) in [
			("Export to WRL...", "Export"),
			("Save Project", "SaveProject"),
			("Load Map...", "FileDialogLoad"),
			("Select All", "SelectAll"),
			("Fit All", "Fit"),
			("Grid", "GridToggle"),
		] {
			assert_eq!(
				menu_command(label).as_deref(),
				Some(crate::input::action_command(id)),
				"menu '{label}' drifted from registry action '{id}'",
			);
		}
		// The specific regression: Export runs the save picker, not pathless export.
		assert_eq!(menu_command("Export to WRL...").as_deref(), Some("file-dialog export-wrl"));
	}

	#[test]
	fn shortcuts_stamp_hints_into_the_tree() {
		let mut b = bar();
		b.apply_shortcuts(&|command| match command {
			"cut" => Some("Ctrl+X".into()),
			"zoom-to 1" => Some("1".into()),
			_ => None,
		});
		// Edit ▸ Cut gets its chord; unbound items stay clean.
		let Item::Action { hint, .. } = &b.menus[1].items[4] else { panic!("Edit/Cut") };
		assert_eq!(hint.as_deref(), Some("Ctrl+X"));
		let Item::Action { hint, .. } = &b.menus[1].items[5] else { panic!("Edit/Copy") };
		assert_eq!(hint.as_deref(), None);
		// Hints reach into submenus (View ▸ Map Zoom ▸ 100%).
		let Item::Sub { items, .. } = &b.menus[2].items[2] else { panic!("View/Map Zoom") };
		let Item::Action { hint, .. } = &items[0] else { panic!("Map Zoom/100%") };
		assert_eq!(hint.as_deref(), Some("1"));
		// (Row-width budgeting for hints lives in the wgpu-ui widget now -
		// covered by its panel-width test.)
	}

	#[test]
	fn context_snapshot_builds_widget_items_and_acts() {
		let items = vec![act("Select All", "select all"), Item::Sep, act("Fit Map", "fit")];
		let (built, acts) = build_context(&items);
		assert_eq!(built.len(), 3, "every row (incl. the separator) builds");
		assert_eq!(acts.len(), 2, "separators take no act slot");
		assert!(matches!(&acts[0], Act::Run(c) if c == "select all"));
		assert!(matches!(&acts[1], Act::Run(c) if c == "fit"));
	}
}
