//! Save Toolbox dockable (id `savetools`, SAVE-EDITOR.md §3a): the object-
//! editing tools + ground-cover quick-picks + team selector for save editing.
//! A lean, button-only sibling of the terrain [`crate::toolbox`] - it borrows
//! that module's `Group`/`Button`/`Kind` data model. Every button runs a
//! command line (the menu's pattern), so the tools stay defined once (in
//! `state.rs` / `input.rs`).
//!
//! **This panel is a real `wgpu-ui` widget tree** (U5.2, the stage U5 pilot):
//! a [`wgpu_ui::ScrollArea`] over a [`wgpu_ui::Wrap`] of group blocks, each a
//! [`wgpu_ui::Label`] over a tight grid of **square tool keys** — stencil-faced
//! [`wgpu_ui::Button`]s and square [`wgpu_ui::ColorButton`] team swatches, the
//! name on each key's tooltip (the graphics-app toolbox costume; only the
//! numeric presets keep text faces). There is no hit oracle, no `ArmFire` and
//! no `Hot`: hover, arming and fire are each key's own, and a fired key emits
//! an **action tag** that [`command_of`] maps back through [`GROUPS`] — the
//! command line is never re-typed (memory `menu-kb-action-registry`).

use wgpu_ui::widget::Widget;
use wgpu_ui::{
	ColorButton, CrossAlign, Insets, Label, Length, Linear, ScrollArea, WidgetId, Wrap, descendant_mut, icon,
};

use crate::state::{EditorState, Tool};
use crate::toolbox::{Button, Group, KEY, Kind, b, ik, tooltip_text};
use crate::ui::Rect;
use crate::uikit_theme::rgba;
use crate::units::TEAM_SWATCH;

const PAD: f32 = 6.0;
const GAP: f32 = 2.0; // between keys within a group
const GROUP_GAP: f32 = 10.0; // between group blocks on a row
const ROW_GAP: f32 = 8.0; // between wrapped rows
const GROUP_LABEL_H: f32 = 14.0;

/// A team swatch button (its player colour fills the key; the name rides the
/// tooltip — a square swatch has no room for a caption).
const fn sw(label: &'static str, cmd: &'static str, fill: [f32; 4]) -> Button {
	Button { label, cmd, fill: Some(fill), icon: None }
}

/// The Save Toolbox's button groups. All [`Kind::Buttons`] - no dropdowns, so
/// the widget needs no popup layer. The object tools reuse the existing
/// place/delete tools via the `obj-place`/`obj-delete` aliases; the ground-cover
/// keys arm a paving type (`unit TAG`); the team swatches set the owner.
pub const GROUPS: &[Group] = &[
	Group {
		label: "object",
		cols: 3,
		kind: Kind::Buttons,
		rows: &[],
		buttons: &[
			ik("select object", "tool obj-select", icon::CURSOR),
			ik("place object", "tool obj-place", icon::CROSSHAIR),
			ik("move object", "tool obj-move", icon::MOVE),
			ik("delete object", "tool obj-delete", icon::TRASH),
			ik("pick object", "tool obj-pick", icon::DROPPER),
			ik("clone object", "tool obj-clone", icon::STAMP),
		],
	},
	// The two overlays a save editor flips constantly while placing objects and
	// painting cargo. Each key runs the same `toggle` line the View menu and the
	// U / R accelerators do, and reads lit while its overlay is on.
	Group {
		label: "show",
		cols: 2,
		kind: Kind::Buttons,
		rows: &[],
		buttons: &[ik("show units", "units toggle", icon::EYE), ik("show resources", "resources toggle", icon::FLAG)],
	},
	Group {
		label: "ground cover",
		cols: 3,
		kind: Kind::Buttons,
		rows: &[],
		buttons: &[
			ik("small slab", "unit SMLSLAB", icon::SLAB),
			ik("large slab", "unit LRGSLAB", icon::SLABS),
			ik("small rubble", "unit SMLRUBLE", icon::DEBRIS),
			ik("large rubble", "unit LRGRUBLE", icon::DEBRIS_LARGE),
			ik("road", "unit ROAD", icon::ROAD),
			ik("cones", "unit SMLCONES", icon::CONE),
		],
	},
	Group {
		label: "team",
		cols: 3,
		kind: Kind::Buttons,
		rows: &[],
		buttons: &[
			sw("red", "unit-team red", TEAM_SWATCH[0]),
			sw("green", "unit-team green", TEAM_SWATCH[1]),
			sw("blue", "unit-team blue", TEAM_SWATCH[2]),
			sw("gray", "unit-team gray", TEAM_SWATCH[3]),
			sw("alien", "unit-team yellow", TEAM_SWATCH[4]),
		],
	},
	// Resource brush (S5.3): arm the tool, pick a material (or erase), and set the
	// combine mode + amount. Painting drags over the cargo map (one undo/stroke).
	// Ragged rows by design: the two *actions* (paint, erase) over the three
	// *material* keys that select what the brush lays.
	Group {
		label: "resource",
		cols: 3,
		kind: Kind::Buttons,
		rows: &[2, 3],
		buttons: &[
			ik("resource brush", "tool resource-brush", icon::BRUSH),
			ik("erase resources", "resource-brush material none", icon::FORBID),
			ik("raw materials", "resource-brush material raw", icon::ORE),
			ik("fuel", "resource-brush material fuel", icon::DROP),
			ik("gold", "resource-brush material gold", icon::INGOT),
		],
	},
	Group {
		label: "res mode",
		cols: 3,
		kind: Kind::Buttons,
		rows: &[],
		buttons: &[
			ik("set exactly", "resource-brush mode set", icon::EQUALS),
			ik("add", "resource-brush mode add", icon::PLUS),
			ik("subtract", "resource-brush mode sub", icon::MINUS),
		],
	},
	// One key per surveyable amount: the game shows a marker frame per unit of
	// cargo up to 16, so 1-16 is the whole useful range and a number is its own
	// best icon. Two rows of eight, with the type-an-exact-value key last.
	Group {
		label: "res amount",
		cols: 9,
		kind: Kind::Buttons,
		rows: &[8, 9],
		buttons: &[
			b("1", "resource-brush amount 1"),
			b("2", "resource-brush amount 2"),
			b("3", "resource-brush amount 3"),
			b("4", "resource-brush amount 4"),
			b("5", "resource-brush amount 5"),
			b("6", "resource-brush amount 6"),
			b("7", "resource-brush amount 7"),
			b("8", "resource-brush amount 8"),
			b("9", "resource-brush amount 9"),
			b("10", "resource-brush amount 10"),
			b("11", "resource-brush amount 11"),
			b("12", "resource-brush amount 12"),
			b("13", "resource-brush amount 13"),
			b("14", "resource-brush amount 14"),
			b("15", "resource-brush amount 15"),
			b("16", "resource-brush amount 16"),
			// Opens a one-field modal to type an exact 0-31 amount (S5.4); the shell
			// intercepts this pseudo-command rather than running it.
			b("...", "resource-amount-dialog"),
		],
	},
];

/// The action tag a key carries: its `(group, button)` coordinates in
/// [`GROUPS`], packed. The tag is what the fired [`wgpu_ui::Ui`] hands back, so
/// the panel resolves a click to a command line by *looking it up in the same
/// table the key was built from* - no command line is ever re-typed, and a
/// button that moves in the table moves its tag with it.
const fn tag(group: usize, button: usize) -> u64 {
	((group as u64) << 32) | button as u64
}

/// The command line a fired action tag stands for, or `None` if it is not one
/// of this panel's (the shell polls every tag its `Ui` collected).
pub fn command_of(tag: u64) -> Option<&'static str> {
	let group = GROUPS.get((tag >> 32) as usize)?;
	let button = group.buttons.get((tag & 0xffff_ffff) as usize)?;
	Some(button.cmd)
}

/// The Save Toolbox state that decides which keys light, snapshotted per frame.
#[derive(Clone)]
pub struct Snapshot {
	tool: Tool,
	team: u8,
	armed_tag: Option<String>,
	/// Resource brush state (S5.3), so its material / mode / amount keys light.
	res_material: Option<max_assets::save::CargoMaterial>,
	res_amount: u8,
	res_mode: crate::state::ResourceMode,
	/// The two overlay toggles the "show" keys mirror.
	show_units: bool,
	show_resources: bool,
}

impl Snapshot {
	pub fn of(editor: &EditorState) -> Self {
		let armed_tag =
			editor.active_unit.and_then(|i| editor.units.as_ref().and_then(|l| l.units.get(i)).map(|u| u.tag.clone()));
		Self {
			tool: editor.tool,
			team: editor.unit_team,
			armed_tag,
			res_material: editor.resource_material,
			res_amount: editor.resource_amount,
			res_mode: editor.resource_mode,
			show_units: editor.show_units,
			show_resources: editor.show_resources,
		}
	}

	#[cfg(test)]
	fn empty() -> Self {
		Self {
			tool: Tool::Pencil,
			team: 0,
			armed_tag: None,
			res_material: None,
			res_amount: 0,
			res_mode: crate::state::ResourceMode::Set,
			show_units: false,
			show_resources: false,
		}
	}

	/// Whether the button running `cmd` reflects the current state (lit key).
	fn active(&self, cmd: &str) -> bool {
		match cmd {
			"tool obj-select" => self.tool == Tool::ObjSelect,
			"tool obj-place" => self.tool == Tool::Unit,
			"tool obj-move" => self.tool == Tool::ObjMove,
			"tool obj-delete" => self.tool == Tool::UnitEraser,
			"tool obj-pick" => self.tool == Tool::ObjPick,
			"tool obj-clone" => self.tool == Tool::ObjClone,
			"tool resource-brush" => self.tool == Tool::ResourceBrush,
			"units toggle" => self.show_units,
			"resources toggle" => self.show_resources,
			_ => {
				if let Some(tag) = cmd.strip_prefix("unit ") {
					self.armed_tag.as_deref() == Some(tag)
				} else if let Some(team) = cmd.strip_prefix("unit-team ") {
					crate::units::parse_team(team) == Some(self.team)
				} else if let Some(mat) = cmd.strip_prefix("resource-brush material ") {
					self.res_material.map_or("none", |m| m.slug()) == mat
				} else if let Some(mode) = cmd.strip_prefix("resource-brush mode ") {
					self.res_mode.slug() == mode
				} else if let Some(amt) = cmd.strip_prefix("resource-brush amount ") {
					amt.parse::<u8>().ok() == Some(self.res_amount)
				} else {
					false
				}
			}
		}
	}
}

/// One key in the built tree: which widget it is, and which command line it
/// stands for (so [`Snapshot::active`] can light it).
struct Key {
	id: WidgetId,
	cmd: &'static str,
}

/// Build the panel's tree once: a `ScrollArea` over a flow of group blocks.
///
/// The flow is a [`Wrap`], not a plain column, because that is what this panel
/// has always done - a wide bottom dock keeps the six blocks on one row and a
/// narrow one stacks them, so the dock's own aspect decides the shape. A block
/// is a column of [`Length::Fixed`] rows rather than a [`wgpu_ui::Grid`]: a
/// `Grid` measures to the width it is *given*, which inside a flow is the whole
/// run, so every block would claim the full row. Fixed rows keep each block's
/// natural width (`cols` keys wide) and the keys uniform at `BTN_W`×`BTN_H`.
fn build() -> (ScrollArea, Vec<Key>) {
	let mut keys = Vec::new();
	let mut flow = Wrap::row().padding(Insets::all(PAD)).spacing(GROUP_GAP).run_spacing(ROW_GAP);
	for (g, group) in GROUPS.iter().enumerate() {
		let mut block = Linear::column().spacing(GAP);
		block = block.child(Label::new(group.label).small().muted(), Length::Fixed(GROUP_LABEL_H));
		for (base, chunk) in group.key_rows() {
			// `Stretch` sizes each key to the row's fixed height; `Fixed(KEY)`
			// keeps the square cells aligned in tight grid columns.
			let mut row = Linear::row().spacing(GAP).cross_align(CrossAlign::Stretch);
			for (c, button) in chunk.iter().enumerate() {
				let tag = tag(g, base + c);
				let cmd = button.cmd;
				match (button.fill, button.icon) {
					// A team swatch is a colour *key*: the player colour is the whole
					// square face, the team's name rides the tooltip (a 24px cell has
					// no room for a caption).
					(Some(fill), _) => {
						// The 3px inset (1 up from the widget's default) leaves the
						// active face a wider ring around the fill, so a selected
						// swatch reads at a glance.
						let key = ColorButton::new(rgba(fill), KEY, KEY).inset(3.0).tooltip(button.label).action(tag);
						keys.push(Key { id: key.id(), cmd });
						row = row.child(key, Length::Fixed(KEY));
					}
					// A square icon key: the stencil is the face, the name the tooltip.
					(None, Some(ic)) => {
						let key = wgpu_ui::Button::new(button.label).icon(ic).tooltip(tooltip_text(button)).action(tag);
						keys.push(Key { id: key.id(), cmd });
						row = row.child(key, Length::Fixed(KEY));
					}
					// A numeric preset stays a text key — a number is its own best
					// icon — squared to the same cell so the grid stays a grid.
					(None, None) => {
						let key = wgpu_ui::Button::new(button.label).small().action(tag);
						keys.push(Key { id: key.id(), cmd });
						row = row.child(key, Length::Fixed(KEY));
					}
				}
			}
			block = block.child(row, Length::Fixed(KEY));
		}
		flow = flow.push(block);
	}
	(ScrollArea::new(flow), keys)
}

/// The Save Toolbox as a retained `wgpu_ui` [`Widget`]: a thin root over the
/// built tree, which exists to hold the key table (`WidgetId` → command line)
/// and to push the per-frame [`Snapshot`] into the keys' `selected` state.
/// Everything else - layout, paint, hover, arming, firing, scrolling - is the
/// tree's.
pub struct SaveToolsContent {
	id: WidgetId,
	root: ScrollArea,
	keys: Vec<Key>,
	rect: Rect,
}

impl Default for SaveToolsContent {
	fn default() -> Self {
		Self::new()
	}
}

impl SaveToolsContent {
	pub fn new() -> Self {
		let (root, keys) = build();
		Self { id: wgpu_ui::next_id(), root, keys, rect: Rect::ZERO }
	}

	/// Light the keys the current editor state selects — the per-frame snapshot
	/// pushed into the retained tree, one key at a time. A key is a `Button` or
	/// (team swatch) a `ColorButton`, so each id is asked for both.
	pub fn sync(&mut self, snap: Snapshot) {
		for key in &self.keys {
			let on = snap.active(key.cmd);
			if let Some(button) = descendant_mut::<wgpu_ui::Button>(&mut self.root, key.id) {
				button.set_selected(on);
			} else if let Some(swatch) = descendant_mut::<ColorButton>(&mut self.root, key.id) {
				swatch.set_selected(on);
			}
		}
	}
}

impl Widget for SaveToolsContent {
	crate::panel_ui::thin_root_plumbing!(arrange, draw, event);
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::uikit_menu::MenuChrome;
	use std::path::Path;
	use wgpu_ui::{Event, Modifiers, PointerButton, Ui, Vec2};

	/// Every button's command parses (the menu's contract).
	#[test]
	fn every_button_parses() {
		for group in GROUPS {
			for button in group.buttons {
				crate::command::parse_line(button.cmd)
					.unwrap_or_else(|e| panic!("{}/{}: {e}", group.label, button.label))
					.unwrap_or_else(|| panic!("{}/{}: empty", group.label, button.label));
			}
		}
	}

	/// Every key's tag round-trips to its own command line - the mapping the
	/// shell resolves a fired action through, so no command is ever re-typed.
	#[test]
	fn every_tag_resolves_to_its_own_command() {
		for (g, group) in GROUPS.iter().enumerate() {
			for (i, button) in group.buttons.iter().enumerate() {
				assert_eq!(command_of(tag(g, i)), Some(button.cmd), "{}/{}", group.label, button.label);
			}
		}
		assert_eq!(command_of(tag(GROUPS.len(), 0)), None, "a tag past the table resolves to nothing");
		assert_eq!(command_of(tag(0, GROUPS[0].buttons.len())), None, "and so does one past a group's keys");
	}

	/// The chrome fixture + a panel hosted in a `Ui`, laid out into `body`.
	fn hosted(body: Rect) -> (MenuChrome, Ui, WidgetId) {
		let (device, queue, _serial) = crate::visual_test::test_gpu();
		let res = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources");
		let steel = crate::skin::load_steel(&res);
		let chrome = MenuChrome::new(&device, &queue, crate::capture::FORMAT, &steel).expect("chrome");
		let content = SaveToolsContent::new();
		let id = content.id();
		let mut ui = Ui::new(content);
		ui.layout_in(body, chrome.theme(), chrome.fonts());
		(chrome, ui, id)
	}

	/// The centre of the key running `cmd`, after layout.
	fn key_centre(ui: &Ui, cmd: &str) -> Vec2 {
		let content = ui.get::<SaveToolsContent>(ui.root().id()).expect("the panel is the root");
		let key = content.keys.iter().find(|k| k.cmd == cmd).unwrap_or_else(|| panic!("no key runs {cmd}"));
		ui.rect_of(key.id).unwrap_or_else(|| panic!("{cmd} was never arranged")).center()
	}

	fn press(pressed: bool, at: Vec2) -> Event {
		Event::PointerButton { button: PointerButton::Primary, pressed, pos: at, mods: Modifiers::NONE }
	}

	/// A press-and-release on a key fires exactly one action, and it is that
	/// key's. This is the whole click path now - no hit oracle, no `ArmFire`.
	#[test]
	fn a_click_on_a_key_fires_exactly_one_action() {
		let body = Rect::new(0.0, 600.0, 1280.0, 160.0);
		let (_chrome, mut ui, _id) = hosted(body);
		let at = key_centre(&ui, "tool obj-move");

		ui.dispatch(&[press(true, at)]);
		assert!(ui.actions().is_empty(), "a press only arms - a command runs on the release");

		ui.dispatch(&[press(false, at)]);
		assert_eq!(ui.actions().len(), 1, "one key, one action");
		assert_eq!(command_of(ui.actions()[0]), Some("tool obj-move"));
	}

	/// A press that releases somewhere else fires nothing - the release-inside
	/// commit policy every command button shares.
	#[test]
	fn a_release_outside_the_key_fires_nothing() {
		let body = Rect::new(0.0, 600.0, 1280.0, 160.0);
		let (_chrome, mut ui, _id) = hosted(body);
		let at = key_centre(&ui, "tool obj-move");
		let elsewhere = key_centre(&ui, "tool obj-pick");

		ui.dispatch(&[press(true, at), press(false, elsewhere)]);
		assert!(ui.actions().is_empty(), "the armed key never saw its release");
	}

	/// The active tool's key reads selected, and only it - the per-frame
	/// `Snapshot` pushed into the tree, read back off the widgets.
	#[test]
	fn the_active_tools_key_reads_selected() {
		let body = Rect::new(0.0, 600.0, 1280.0, 160.0);
		let (_chrome, mut ui, id) = hosted(body);

		let selected = |ui: &Ui| -> Vec<&'static str> {
			let content = ui.get::<SaveToolsContent>(id).expect("typed root");
			content
				.keys
				.iter()
				.filter(|k| match ui.get::<wgpu_ui::Button>(k.id) {
					Some(b) => b.selected(),
					None => ui.get::<ColorButton>(k.id).is_some_and(|s| s.selected()),
				})
				.map(|k| k.cmd)
				.collect()
		};

		let mut snap = Snapshot::empty();
		snap.tool = Tool::ObjMove;
		snap.team = 2;
		ui.get_mut::<SaveToolsContent>(id).expect("typed root").sync(snap);
		assert_eq!(
			selected(&ui),
			vec!["tool obj-move", "unit-team blue", "resource-brush material none", "resource-brush mode set"],
			"the armed tool, the owning team and the brush's resting material/mode"
		);

		let mut snap = Snapshot::empty();
		snap.tool = Tool::ResourceBrush;
		snap.res_amount = 16;
		ui.get_mut::<SaveToolsContent>(id).expect("typed root").sync(snap);
		assert!(selected(&ui).contains(&"resource-brush amount 16"), "the amount key follows the snapshot");
		assert!(!selected(&ui).contains(&"tool obj-move"), "and the key that was lit goes dark");
	}

	/// Hover is the `Ui`'s now, not a shell-fed `Hot`: a move lights exactly one
	/// key, and the `PointerLeft` the router derives when a menu opens over the
	/// panel unlights it. Before U5.2 that leave had no effect on a panel -
	/// `render_frame` blanked the whole panel's hover instead.
	#[test]
	fn a_move_lights_one_key_and_a_leave_unlights_it() {
		let body = Rect::new(0.0, 600.0, 1280.0, 160.0);
		let (_chrome, mut ui, _id) = hosted(body);
		let at = key_centre(&ui, "tool obj-pick");

		ui.dispatch(&[Event::PointerMoved { pos: at }]);
		let hovered = ui.hovered();
		assert_ne!(hovered, WidgetId::NONE, "the key under the pointer lights");
		assert_eq!(hovered, ui.root().hit_test(at).expect("a key is hit there"), "and it is that key");

		ui.dispatch(&[Event::PointerLeft]);
		assert_eq!(ui.hovered(), WidgetId::NONE, "an open menu's derived leave puts it out");
	}
}
