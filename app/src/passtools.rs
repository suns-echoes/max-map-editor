//! Pass Types Palette dockable (id `passtools`, design `features.drawio`
//! Windows ▸ Dockable Dialogs, backlog TOOL-6): the four pass-type swatches the
//! two pass editors paint with, plus a live tally of what the map currently
//! reads as.
//!
//! The swatches used to sit in the terrain [`crate::toolbox`], where they were
//! dead weight in the mode that owns eight other groups and *invisible* in the
//! mode that actually paints with them. They live here now, beside the numbers
//! that say what the painting did — which is the whole reason a pass editor
//! wants a panel of its own.
//!
//! **The panel is a real `wgpu-ui` widget tree** (the `panel_ui.rs` recipe): a
//! [`wgpu_ui::ScrollArea`] over a [`wgpu_ui::Wrap`] of two blocks — the swatch
//! keys as square [`wgpu_ui::ColorButton`] palette chips (2×2, the pass name on
//! each chip's tooltip), the tally as rows of stock
//! [`wgpu_ui::Label`]s rewritten each frame through `sync`. There is no hit
//! oracle and no panel-wide `ArmFire`: hover, arming and fire are each key's
//! own, and a fired key comes back as an **action tag** [`command_of`] maps
//! through [`GROUPS`], so no command line is ever re-typed (memory
//! `menu-kb-action-registry`).

use wgpu_ui::widget::Widget;
use wgpu_ui::{
	ColorButton, CrossAlign, Insets, Label, Length, Linear, ScrollArea, TextAlign, WidgetId, Wrap, descendant_mut,
};

use crate::state::{EditorState, PASS_COLORS, PASS_LABELS};
use crate::toolbox::{Button, Group, KEY, Kind};
use crate::ui::Rect;
use crate::uikit_theme::rgba;

const PAD: f32 = 6.0;
const GAP: f32 = 2.0; // between keys within a group
const GROUP_GAP: f32 = 10.0; // between group blocks on a row
const ROW_GAP: f32 = 8.0; // between wrapped rows
const GROUP_LABEL_H: f32 = 14.0;
/// One tally row — text-height, denser than the square swatch cells beside it.
const STAT_ROW_H: f32 = 18.0;
/// The tally's three columns: name, count, share of the map.
const STAT_NAME_W: f32 = 46.0;
const STAT_COUNT_W: f32 = 52.0;
const STAT_PCT_W: f32 = 40.0;

/// A colored swatch button (the pass types, drawn in their overlay colours;
/// the name rides the tooltip — a square swatch has no room for a caption).
const fn sw(label: &'static str, cmd: &'static str, fill: [f32; 4]) -> Button {
	Button { label, cmd, fill: Some(fill), icon: None }
}

/// The panel's button groups — one, all [`Kind::Buttons`], so the widget needs
/// no popup layer. The tally rows are labels, not keys, so they are not in the
/// table: nothing about them is clickable.
pub const GROUPS: &[Group] = &[Group {
	label: "pass type",
	cols: 2,
	kind: Kind::Buttons,
	rows: &[],
	buttons: &[
		sw("land", "pass-pick 0", PASS_COLORS[0]),
		sw("water", "pass-pick 1", PASS_COLORS[1]),
		sw("shore", "pass-pick 2", PASS_COLORS[2]),
		sw("block", "pass-pick 3", PASS_COLORS[3]),
	],
}];

/// The action tag a key carries: its `(group, button)` coordinates in
/// [`GROUPS`], packed — the same scheme [`crate::savetools`] uses, so a key that
/// moves in the table moves its tag with it.
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

/// The pass-editor state this panel draws, snapshotted per frame: which swatch
/// is armed, and how the map's cells currently tally by pass value.
#[derive(Clone)]
pub struct Snapshot {
	active_pass: u8,
	/// Cells reading as each pass value, and how many carry an explicit
	/// per-cell override (`map_core::Project::pass_counts`).
	counts: [u32; 4],
	overrides: u32,
}

impl Snapshot {
	pub fn of(editor: &EditorState) -> Self {
		let (counts, overrides) = editor.project.pass_counts();
		Self { active_pass: editor.active_pass, counts, overrides }
	}

	#[cfg(test)]
	fn empty() -> Self {
		Self { active_pass: 0, counts: [0; 4], overrides: 0 }
	}

	/// Whether the key running `cmd` reflects the current state (lit key).
	fn active(&self, cmd: &str) -> bool {
		cmd.strip_prefix("pass-pick ").and_then(|v| v.parse::<u8>().ok()) == Some(self.active_pass)
	}

	/// The tallied cells across all four pass values — the denominator the
	/// shares are of, and 0 for a map whose pack ships no pass table.
	fn total(&self) -> u32 {
		self.counts.iter().sum()
	}

	/// One tally row's `(count, share)` text. The share is of the tallied cells,
	/// so the four rows always add up to 100% (bar rounding).
	fn stat_texts(&self, i: usize) -> (String, String) {
		let n = self.counts[i];
		let total = self.total();
		let pct = if total == 0 { 0.0 } else { n as f32 * 100.0 / total as f32 };
		(n.to_string(), format!("{pct:.1}%"))
	}
}

/// One key in the built tree: which widget it is, and which command line it
/// stands for (so [`Snapshot::active`] can light it).
struct Key {
	id: WidgetId,
	cmd: &'static str,
}

/// The tally's per-row label ids, in [`PASS_LABELS`] order.
struct StatRow {
	count: WidgetId,
	pct: WidgetId,
}

/// One tally row: a fixed-width name, then the count and the share right-aligned
/// in fixed columns so the digits line up down the block (a proportional font
/// gives padding-by-spaces nothing to align on).
fn stat_row(name: &str) -> (Linear, StatRow) {
	let count = Label::new("0").small().align(TextAlign::Right).with_id();
	let pct = Label::new("0.0%").small().muted().align(TextAlign::Right).with_id();
	let ids = StatRow { count: count.id(), pct: pct.id() };
	let row = Linear::row()
		.spacing(GAP)
		.cross_align(CrossAlign::Stretch)
		.child(Label::new(name).small().muted(), Length::Fixed(STAT_NAME_W))
		.child(count, Length::Fixed(STAT_COUNT_W))
		.child(pct, Length::Fixed(STAT_PCT_W));
	(row, ids)
}

/// Build the panel's tree once: a `ScrollArea` over a flow of the swatch block
/// and the tally block.
///
/// The flow is a [`Wrap`] like the two sibling toolboxes': a wide bottom dock
/// keeps both blocks on one run, a narrow one stacks them. A block is a column
/// of [`Length::Fixed`] rows rather than a [`wgpu_ui::Grid`] for the same reason
/// they are — a `Grid` measures to the width it is *given*, which inside a flow
/// is the whole run, so each block would claim the entire row.
fn build() -> (ScrollArea, Vec<Key>, Vec<StatRow>, WidgetId) {
	let mut keys = Vec::new();
	let mut flow = Wrap::row().padding(Insets::all(PAD)).spacing(GROUP_GAP).run_spacing(ROW_GAP);

	for (g, group) in GROUPS.iter().enumerate() {
		let mut block = Linear::column().spacing(GAP);
		block = block.child(Label::new(group.label).small().muted(), Length::Fixed(GROUP_LABEL_H));
		for (base, chunk) in group.key_rows() {
			let mut row = Linear::row().spacing(GAP).cross_align(CrossAlign::Stretch);
			for (c, button) in chunk.iter().enumerate() {
				let tag = tag(g, base + c);
				let cmd = button.cmd;
				// A pass-type key is a colour *key*: the semantic pass colour is the
				// whole square face — a palette chip — and the name rides the
				// tooltip (a 24px cell has no room for a caption).
				let fill = button.fill.unwrap_or(PASS_COLORS[0]);
				// 3px inset (1 up from the default): a wider face ring, so the
				// active chip reads at a glance.
				let key = ColorButton::new(rgba(fill), KEY, KEY).inset(3.0).tooltip(button.label).action(tag);
				keys.push(Key { id: key.id(), cmd });
				row = row.child(key, Length::Fixed(KEY));
			}
			block = block.child(row, Length::Fixed(KEY));
		}
		flow = flow.push(block);
	}

	// The tally: one row per pass value, then the override count under them.
	let mut stats = Vec::new();
	let mut block = Linear::column().spacing(GAP);
	block = block.child(Label::new("cells").small().muted(), Length::Fixed(GROUP_LABEL_H));
	for name in PASS_LABELS {
		let (row, ids) = stat_row(name);
		stats.push(ids);
		block = block.child(row, Length::Fixed(STAT_ROW_H));
	}
	let overrides = Label::new("0").small().align(TextAlign::Right).with_id();
	let overrides_id = overrides.id();
	block = block.child(
		Linear::row()
			.spacing(GAP)
			.cross_align(CrossAlign::Stretch)
			.child(Label::new("local").small().muted(), Length::Fixed(STAT_NAME_W))
			.child(overrides, Length::Fixed(STAT_COUNT_W)),
		Length::Fixed(STAT_ROW_H),
	);
	flow = flow.push(block);

	(ScrollArea::new(flow), keys, stats, overrides_id)
}

/// The Pass Types Palette as a retained `wgpu_ui` [`Widget`]: a thin root over
/// the built tree, holding the id tables (the swatch keys, the tally's labels)
/// and pushing the per-frame [`Snapshot`] into them. Everything else — layout,
/// paint, hover, arming, firing, scrolling — is the tree's.
pub struct PassToolsContent {
	id: WidgetId,
	root: ScrollArea,
	keys: Vec<Key>,
	/// One entry per pass value, in [`PASS_LABELS`] order.
	stats: Vec<StatRow>,
	/// The per-cell override count's label.
	overrides: WidgetId,
	rect: Rect,
}

impl Default for PassToolsContent {
	fn default() -> Self {
		Self::new()
	}
}

impl PassToolsContent {
	pub fn new() -> Self {
		let (root, keys, stats, overrides) = build();
		Self { id: wgpu_ui::next_id(), root, keys, stats, overrides, rect: Rect::ZERO }
	}

	/// Push one frame's editor state into the retained tree: which swatch is
	/// armed, and what the tally reads.
	pub fn sync(&mut self, snap: Snapshot) {
		for key in &self.keys {
			let on = snap.active(key.cmd);
			if let Some(swatch) = descendant_mut::<ColorButton>(&mut self.root, key.id) {
				swatch.set_selected(on);
			}
		}
		for (i, row) in self.stats.iter().enumerate() {
			let (count, pct) = snap.stat_texts(i);
			if let Some(label) = descendant_mut::<Label>(&mut self.root, row.count) {
				label.set_text(count);
			}
			if let Some(label) = descendant_mut::<Label>(&mut self.root, row.pct) {
				label.set_text(pct);
			}
		}
		if let Some(label) = descendant_mut::<Label>(&mut self.root, self.overrides) {
			label.set_text(snap.overrides.to_string());
		}
	}
}

impl Widget for PassToolsContent {
	crate::panel_ui::thin_root_plumbing!(arrange, draw, event);
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::uikit_menu::MenuChrome;
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

	/// Every key's tag round-trips to its own command line — the mapping the
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

	/// The four swatches are the four pass values, in order and in their own
	/// overlay colours — the panel is the *palette* for what the pass overlay
	/// paints, so a drift between the two would be a lie about the map.
	#[test]
	fn the_swatches_are_the_pass_values_in_their_own_colours() {
		let keys = GROUPS[0].buttons;
		assert_eq!(keys.len(), PASS_COLORS.len(), "one key per pass value");
		for (i, button) in keys.iter().enumerate() {
			assert_eq!(button.fill, Some(PASS_COLORS[i]), "key {i} wears its pass colour");
			assert_eq!(button.cmd, format!("pass-pick {i}"), "and picks its own value");
		}
	}

	/// The chrome fixture + a panel hosted in a `Ui`, laid out into `body`.
	fn hosted(body: Rect) -> (MenuChrome, Ui, WidgetId) {
		let (_device, _queue, chrome) = crate::visual_test::chrome_fixture();
		let content = PassToolsContent::new();
		let id = content.id();
		let mut ui = Ui::new(content);
		ui.get_mut::<PassToolsContent>(id).expect("typed root").sync(Snapshot::empty());
		ui.layout_in(body, chrome.theme(), chrome.fonts());
		(chrome, ui, id)
	}

	/// The centre of the key running `cmd`, after layout.
	fn key_centre(ui: &Ui, cmd: &str) -> Vec2 {
		let content = ui.get::<PassToolsContent>(ui.root().id()).expect("the panel is the root");
		let key = content.keys.iter().find(|k| k.cmd == cmd).unwrap_or_else(|| panic!("no key runs {cmd}"));
		ui.rect_of(key.id).unwrap_or_else(|| panic!("{cmd} was never arranged")).center()
	}

	fn press(pressed: bool, at: Vec2) -> Event {
		Event::PointerButton { button: PointerButton::Primary, pressed, pos: at, mods: Modifiers::NONE }
	}

	/// A press-and-release on a swatch fires exactly one action, and it is that
	/// swatch's; a release elsewhere fires nothing (the release-inside commit
	/// policy every command key shares).
	#[test]
	fn a_click_on_a_swatch_picks_its_own_pass_value() {
		let body = Rect::new(0.0, 600.0, 720.0, 160.0);
		let (_chrome, mut ui, _id) = hosted(body);
		let at = key_centre(&ui, "pass-pick 2");

		ui.dispatch(&[press(true, at)]);
		assert!(ui.actions().is_empty(), "a press only arms");
		ui.dispatch(&[press(false, at)]);
		assert_eq!(ui.actions().len(), 1, "one key, one action");
		assert_eq!(command_of(ui.actions()[0]), Some("pass-pick 2"));

		let elsewhere = key_centre(&ui, "pass-pick 0");
		ui.dispatch(&[press(true, at), press(false, elsewhere)]);
		assert!(ui.actions().is_empty(), "the armed key never saw its release");
	}

	/// The armed pass value's swatch reads selected, and only it.
	#[test]
	fn the_armed_pass_value_reads_selected() {
		let body = Rect::new(0.0, 600.0, 720.0, 160.0);
		let (_chrome, mut ui, id) = hosted(body);
		let lit = |ui: &Ui| -> Vec<&'static str> {
			let content = ui.get::<PassToolsContent>(id).expect("typed root");
			content
				.keys
				.iter()
				.filter(|k| ui.get::<ColorButton>(k.id).is_some_and(|s| s.selected()))
				.map(|k| k.cmd)
				.collect()
		};
		assert_eq!(lit(&ui), vec!["pass-pick 0"], "an empty snapshot arms land");

		let mut snap = Snapshot::empty();
		snap.active_pass = 3;
		ui.get_mut::<PassToolsContent>(id).expect("typed root").sync(snap);
		assert_eq!(lit(&ui), vec!["pass-pick 3"], "and the picked value takes it over");
	}

	/// The tally rewrites the retained labels rather than rebuilding the block:
	/// counts as typed, shares of the tallied cells, and the override row.
	#[test]
	fn the_tally_rewrites_its_labels_from_the_snapshot() {
		let body = Rect::new(0.0, 600.0, 720.0, 200.0);
		let (_chrome, mut ui, id) = hosted(body);
		let text = |ui: &Ui, w: WidgetId| ui.get::<Label>(w).expect("an id'd label").text().to_string();

		let (rows, overrides) = {
			let c = ui.get::<PassToolsContent>(id).expect("typed root");
			(c.stats.iter().map(|r| (r.count, r.pct)).collect::<Vec<_>>(), c.overrides)
		};
		assert_eq!(rows.len(), PASS_LABELS.len(), "one row per pass value");
		assert_eq!(text(&ui, rows[0].1), "0.0%", "an empty map divides by nothing rather than panicking");

		let mut snap = Snapshot::empty();
		snap.counts = [50, 30, 15, 5];
		snap.overrides = 7;
		let ids: Vec<WidgetId> = rows.iter().map(|r| r.0).collect();
		ui.get_mut::<PassToolsContent>(id).expect("typed root").sync(snap);
		assert_eq!(ids.iter().map(|&w| text(&ui, w)).collect::<Vec<_>>(), ["50", "30", "15", "5"]);
		assert_eq!(text(&ui, rows[0].1), "50.0%", "the share is of the tallied cells");
		assert_eq!(text(&ui, rows[3].1), "5.0%");
		assert_eq!(text(&ui, overrides), "7", "and the override row counts the per-cell ones");

		// The ids are the *same* labels - a rebuilt block would mint new ones and
		// lose every id the panel holds (the `panel_ui.rs` re-sync rule).
		let after: Vec<WidgetId> =
			ui.get::<PassToolsContent>(id).expect("typed root").stats.iter().map(|r| r.count).collect();
		assert_eq!(after, ids, "sync rewrites the retained labels, it does not rebuild them");
	}
}
