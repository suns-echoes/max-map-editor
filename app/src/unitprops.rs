//! Unit Properties dockable (id `unitprops`, SAVE-EDITOR.md §3b): the
//! inspector + editor for the object picked by the Select tool
//! (`EditorState.selected_object`, S2.3). It reads out the object's fields and
//! edits them: team (swatches), facing / orders / turret (dropdowns), the
//! connector mask (a footprint-shaped checkbox grid), and the free-value fields
//! name / hits / ammo / storage / disabled — all firing the `object-edit`
//! command (S4.2/S4.4). The turret row shows only for units whose sprite carries
//! turret frames (`object_has_turret`).
//!
//! **This panel is a real `wgpu-ui` widget tree** (U5.8, the stage's biggest
//! ticket): a [`wgpu_ui::ScrollArea`] over a [`wgpu_ui::Linear`] column of
//! sections — a header band ([`SpriteWell`] + the unit's name), the `object`
//! rows, the max-values section and the connector grid. There is no hand-placed
//! `Layout`, no hit oracle, no `ArmFire` and no `Hot`: every row is a
//! [`wgpu_ui::Label`] beside a [`TextInput`] / [`Select`] / swatch /
//! [`Checkbox`], hover and arming are each child's own, and everything the panel
//! produces comes back as an **action tag** off `Ui::actions` that [`action_of`]
//! maps to an [`Action`] (U5.4's one-tag-space shape).
//!
//! **Optional sections are [`wgpu_ui::Reveal`] slots, not a rebuilt tree.** The
//! Turret row, the whole values section, each stat row and the connector grid
//! come and go with the selection; collapsing them keeps every [`TextInput`]'s
//! text, caret and focus alive across the change, which rebuilding could not.
//! A hidden `Reveal` reports no children, so [`sync`](UnitPropsContent::sync)
//! walks the tree **top-down** — show the slot, *then* reach inside it.
//!
//! The two native sprite quads (the header preview and the connector
//! footprint) are drawn by the shell into rects it reads back off the tree after
//! `build` — [`UnitPropsContent::preview_rect`] and
//! [`UnitPropsContent::connector_rect`] — so the chrome under them cannot drift
//! (the U5.3 invariant).

use max_assets::attribs::StatKind;
use max_assets::save::UnitValues;
use wgpu_ui::widget::{DrawCtx, EventCtx, LayoutCtx, Widget};
use wgpu_ui::{
	Charset, Checkbox, ColorButton, CrossAlign, DrawList, Event, Insets, Label, Length, Linear, PageKeys, Reveal,
	ScrollArea, Select, Separator, Size, TextAlign, TextInput, Vec2, WidgetId, WidgetState, descendant, descendant_mut,
};

use crate::state::EditorState;
use crate::theme;
use crate::ui::Rect;
use crate::uikit_theme::rgba;
use crate::units::{TEAM_NAMES, TEAM_SWATCH};

const PAD: f32 = 8.0;
const ROW_H: f32 = 18.0;
const GROUP_LABEL_H: f32 = 14.0;
const SW: f32 = 14.0; // team swatch size
const SWGAP: f32 = 3.0; // gap between swatches
/// Blank space inserted before each new section header (object → values →
/// connector), so the groups read as distinct blocks (item 10).
const SECTION_GAP: f32 = 8.0;
/// Height of the top preview/name header band (item 11). The live sprite preview
/// + the unit's name sit here, above the "object" section.
const HEADER_H: f32 = 56.0;
/// Side of the square live-preview well in the header band.
const PREVIEW: f32 = 44.0;
// Connector grid cell (square). A 1-cell footprint previews the unit's 64px
// (1-cell) sprite at this size, so 32 = half-scale; a 2×2 footprint spans 2 cells.
const CGRID_CELL: f32 = 32.0;
/// Gap between the "Connect" caption and the grid on the line below it.
const CONN_CAPTION_GAP: f32 = 4.0;
/// The slot a [`Separator`] rule sits in — a section underline, or the rule
/// under the header band.
const RULE_H: f32 = 2.0;
/// A section heading's whole band: the label plus its rule.
const HEAD_H: f32 = GROUP_LABEL_H + RULE_H;
/// The "advanced" caption's slot in the values header row.
const ADVANCED_W: f32 = 78.0;
/// The bare checkbox's slot (the toolkit box is 16px square).
const CHECK_W: f32 = 16.0;

/// Heading names for a mobile unit's `angle` (0-7 = N..NW, `enums.hpp`
/// `UNIT_ANGLE_*`). Ground cover stores a decorative variant index in the same
/// field, which can exceed 7 — shown as `variant N` instead.
const HEADINGS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];

/// The fixed rows' labels, in display order — the single list the tree is built
/// from *and* [`rows`] reads out, so a row cannot be labelled two ways.
const ROW_LABELS: [&str; ROW_COUNT] =
	["Type", "Id", "Team", "Name", "Facing", "Hits", "Ammo", "Storage", "Orders", "Disabled"];

// Row indices into [`ROW_LABELS`], shared by the tree build and the readout.
const TYPE_ROW: usize = 0;
const ID_ROW: usize = 1;
const TEAM_ROW: usize = 2;
const NAME_ROW: usize = 3;
const FACING_ROW: usize = 4;
const HITS_ROW: usize = 5;
const AMMO_ROW: usize = 6;
const STORAGE_ROW: usize = 7;
const ORDERS_ROW: usize = 8;
/// Disable countdown (turns): a free-value box; 0 = not disabled. Editing it also
/// flips the order to/from `ORDER_DISABLE` (shell-side, `object-edit disabled`).
const DISABLED_ROW: usize = 9;
const ROW_COUNT: usize = 10;

/// One editable maximum-stat row (S4.5): its `object-values` attr slug, the panel
/// label, whether it's an *advanced* (static/structural) stat shown only in
/// advanced mode, and a reader for its current value off a [`UnitValues`]. The
/// dynamic combat stats come first (always shown); the advanced tail is the
/// structural/internal fields the game doesn't upgrade in play.
pub struct ValueStat {
	pub attr: &'static str,
	pub label: &'static str,
	pub advanced: bool,
	pub kind: StatKind,
}

/// The max-stats the panel edits, in display order — nine always-shown dynamic
/// combat stats, then five advanced ones. Public so the shell resolves a row's
/// label / current value when opening the edit modal. Each row reads/edits the
/// [`UnitValues`] field its [`StatKind`] names.
pub const VALUE_STATS: [ValueStat; 14] = [
	ValueStat { attr: "hits", label: "Max HP", advanced: false, kind: StatKind::Hits },
	ValueStat { attr: "attack", label: "Attack", advanced: false, kind: StatKind::Attack },
	ValueStat { attr: "armor", label: "Armor", advanced: false, kind: StatKind::Armor },
	ValueStat { attr: "range", label: "Range", advanced: false, kind: StatKind::Range },
	ValueStat { attr: "speed", label: "Speed", advanced: false, kind: StatKind::Speed },
	ValueStat { attr: "scan", label: "Scan", advanced: false, kind: StatKind::Scan },
	ValueStat { attr: "rounds", label: "Shots", advanced: false, kind: StatKind::Rounds },
	ValueStat { attr: "ammo", label: "Ammo max", advanced: false, kind: StatKind::Ammo },
	ValueStat { attr: "storage", label: "Cargo max", advanced: false, kind: StatKind::Storage },
	ValueStat { attr: "turns", label: "Build turns", advanced: true, kind: StatKind::Turns },
	ValueStat { attr: "attack-radius", label: "Atk radius", advanced: true, kind: StatKind::AttackRadius },
	ValueStat { attr: "move-and-fire", label: "Move+fire", advanced: true, kind: StatKind::MoveAndFire },
	ValueStat { attr: "agent-adjust", label: "Agent adj", advanced: true, kind: StatKind::AgentAdjust },
	ValueStat { attr: "version", label: "Version", advanced: true, kind: StatKind::Version },
];

/// Visibility of the max-values section for the current selection: whether a
/// stats block exists at all, the advanced toggle, and the per-stat
/// applicability mask (S7.5) — stats the game ignores for this unit type
/// (attack on a radar, cargo on a tank) drop their rows entirely.
#[derive(Clone, Copy, PartialEq)]
pub struct StatsVis {
	pub present: bool,
	pub advanced: bool,
	pub mask: [bool; VALUE_STATS.len()],
}

impl StatsVis {
	/// All stats applicable — the pre-mask behavior (the section's own tests).
	#[cfg(test)]
	pub const fn all(present: bool, advanced: bool) -> StatsVis {
		StatsVis { present, advanced, mask: [true; VALUE_STATS.len()] }
	}

	/// Whether the i-th [`VALUE_STATS`] row is shown.
	fn shown(&self, i: usize) -> bool {
		self.present && self.mask[i] && (self.advanced || !VALUE_STATS[i].advanced)
	}

	/// How many stat rows are shown.
	#[cfg(test)]
	fn count(&self) -> usize {
		(0..VALUE_STATS.len()).filter(|&i| self.shown(i)).count()
	}
}

/// Per-side base bit (`enums.hpp`) for the first half-edge of each building side,
/// in `connector_bit_at`'s `N, E, S, W` order. The second half-edge of a side
/// (2×2 footprints) is `base << 1`: N→NL 0x01 / NR 0x02, E→ET 0x04 / EB 0x08,
/// S→SL 0x10 / SR 0x20, W→WT 0x40 / WB 0x80. A 1×1 footprint uses only the base
/// bits (NL|ET|SL|WT = 0x55), matching the mask those units actually store.
const SIDE_BASE: [u16; 4] = [0x01, 0x04, 0x10, 0x40];

/// Every half-edge bit, in the order [`ConnectorGrid`] holds its checkboxes:
/// NL, NR, ET, EB, SL, SR, WT, WB.
const BITS: [u16; 8] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80];

/// The connector mask bit a grid cell `(row, col)` toggles, or `None` for a
/// corner / the centre footprint / an off-grid cell. The grid is `(f+2)²` cells
/// for a footprint `f` (1 or 2): the inner `f×f` block is the building, the four
/// corners are inert, and each remaining perimeter cell is one half-edge
/// checkbox. `k` (0 or 1) is the cell's offset along its side → `base << k`.
fn connector_bit_at(footprint: u16, row: usize, col: usize) -> Option<u16> {
	let f = footprint.clamp(1, 2) as usize;
	let n = f + 2;
	if row >= n || col >= n {
		return None;
	}
	let on_perimeter = row == 0 || row == n - 1 || col == 0 || col == n - 1;
	let corner = (row == 0 || row == n - 1) && (col == 0 || col == n - 1);
	if !on_perimeter || corner {
		return None; // centre footprint or a corner
	}
	// A perimeter, non-corner cell: which side, and its offset `k` along it.
	let (side, k) = if row == 0 {
		(0, col - 1) // north
	} else if col == n - 1 {
		(1, row - 1) // east
	} else if row == n - 1 {
		(2, col - 1) // south
	} else {
		(3, row - 1) // west
	};
	Some(SIDE_BASE[side] << k)
}

/// The inverse of [`connector_bit_at`]: which cell carries `bit` at this
/// footprint, or `None` when the footprint has no such half-edge (a 1×1 host
/// uses only the four base bits). Derived by scanning the same oracle, so the
/// two can never disagree.
fn cell_of_bit(footprint: u16, bit: u16) -> Option<(usize, usize)> {
	let n = grid_side(footprint);
	(0..n).flat_map(|r| (0..n).map(move |c| (r, c))).find(|&(r, c)| connector_bit_at(footprint, r, c) == Some(bit))
}

/// Cells per side of the connector grid for a footprint (`f + 2`).
fn grid_side(footprint: u16) -> usize {
	footprint.clamp(1, 2) as usize + 2
}

/// Whether cell `(row, col)` is part of the centre footprint block (the `f×f`
/// core the building sprite fills).
fn is_footprint_cell(footprint: u16, row: usize, col: usize) -> bool {
	let f = footprint.clamp(1, 2) as usize;
	(1..=f).contains(&row) && (1..=f).contains(&col)
}

/// The `(row, col)` cell rect in a connector grid at `origin`.
fn connector_cell(origin: Vec2, row: usize, col: usize) -> Rect {
	Rect::new(origin.x + col as f32 * CGRID_CELL, origin.y + row as f32 * CGRID_CELL, CGRID_CELL, CGRID_CELL)
}

/// The centre footprint rect (the `f×f` block the sprite fills) in the grid.
fn footprint_rect(origin: Vec2, footprint: u16) -> Rect {
	let f = footprint.clamp(1, 2) as f32;
	Rect::new(origin.x + CGRID_CELL, origin.y + CGRID_CELL, f * CGRID_CELL, f * CGRID_CELL)
}

/// A free-value field the panel edits in place (item 8) — the values are
/// unbounded (name) or too wide for a stepper (hits/ammo).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
	Name,
	Hits,
	Ammo,
	/// Cargo carried / experience accrued — signed (`ObjectProps::storage`, S4.4).
	Storage,
	/// Turns the unit stays disabled (`ObjectProps::disabled_turns`); 0 = not
	/// disabled. The shell couples it to `ORDER_DISABLE`.
	Disabled,
}

/// The enumerable fields edited through a hosted [`Select`] rather than typed:
/// facing (8 headings), orders (the `UnitOrderType` slugs), and — for units with
/// an independent turret — the turret heading (8 headings, S4.4). The open list
/// rides the panel's overlay pass out to the shell's popup layer (U3.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectKind {
	Facing,
	Orders,
	/// The turret's own heading, independent of the body's `Facing` — shown only
	/// for units whose sprite carries turret frames (`object_has_turret`).
	Turret,
}

/// What a fired action tag stands for. The shell resolves each against the
/// *current* selection: [`Action::Team`]/[`Action::ConnectorToggle`]/
/// [`Action::SelectPick`] run an `object-edit` command;
/// [`Action::ToggleAdvanced`] is panel state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
	/// Set the owner team (0-4).
	Team(u8),
	/// A value picked from the facing / orders / turret dropdown — the hosted
	/// [`Select`]'s commit (U3.4). Opening, dismissing and the keyboard are the
	/// widget's; only the chosen value reaches the shell.
	SelectPick(SelectKind, u8),
	/// Toggle one connector half-edge bit (XOR into `connectors`, S4.4).
	ConnectorToggle(u16),
	/// Toggle the values section's *advanced* rows (structural stats) on/off.
	ToggleAdvanced,
}

/// The tag space: a kind in the high bits over a 32-bit payload, so one
/// `Ui::actions` poll answers for the whole panel (U5.4's shape). Kind `0` is
/// deliberately unused — a stray zero tag resolves to nothing.
const KIND_SHIFT: u32 = 32;
/// A team swatch: the payload is its index into [`TEAM_NAMES`].
const KIND_TEAM: u64 = 1;
/// A dropdown pick: the payload is `(dropdown index << 8) | value`.
const KIND_PICK: u64 = 2;
/// A connector checkbox: the payload is its half-edge bit.
const KIND_CONNECTOR: u64 = 3;
/// The advanced toggle (no payload).
const KIND_ADVANCED: u64 = 4;

const fn tag(kind: u64, payload: u64) -> u64 {
	(kind << KIND_SHIFT) | payload
}

/// The panel's dropdowns, in the order [`UnitPropsContent`] holds them — the
/// index a [`KIND_PICK`] payload carries.
const SELECT_KINDS: [SelectKind; 3] = [SelectKind::Facing, SelectKind::Orders, SelectKind::Turret];

/// The Unit Properties action a fired tag stands for, or `None` if it is not one
/// of this panel's (the shell polls every tag its `Ui` collected).
pub fn action_of(t: u64) -> Option<Action> {
	let payload = t & 0xffff_ffff;
	match t >> KIND_SHIFT {
		KIND_TEAM => (payload < TEAM_NAMES.len() as u64).then_some(Action::Team(payload as u8)),
		KIND_PICK => {
			let kind = *SELECT_KINDS.get((payload >> 8) as usize)?;
			Some(Action::SelectPick(kind, (payload & 0xff) as u8))
		}
		KIND_CONNECTOR => BITS.contains(&(payload as u16)).then_some(Action::ConnectorToggle(payload as u16)),
		KIND_ADVANCED => Some(Action::ToggleAdvanced),
		_ => None,
	}
}

/// A committed in-place text edit (item 8), polled separately from [`Action`]
/// because it carries owned text (so it can't ride the `Copy` tag channel).
/// Fired when a field box loses focus or takes Enter; the shell turns it into an
/// `object-edit` / `object-values` command against the live selection.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Commit {
	/// A free-value field (name / hits / ammo / storage) → `object-edit <field>`.
	Field(Field, String),
	/// A max-stat field → `object-values <attr>`; the payload is its attr slug.
	Value(&'static str, String),
}

/// The option labels for a dropdown, in value order (index = the field value
/// written). Facing/turret = the 8 compass headings *with their raw value*
/// ([`heading_label`]) — which is what the closed box always showed, while the
/// list showed the bare name: one control, two labellings, the drift U3 exists
/// to end. Orders = every `UnitOrderType` slug.
pub fn select_labels(kind: SelectKind) -> Vec<String> {
	match kind {
		SelectKind::Facing | SelectKind::Turret => (0..HEADINGS.len() as u8).map(heading_label).collect(),
		SelectKind::Orders => max_assets::save::ORDER_NAMES.iter().map(|s| s.to_string()).collect(),
	}
}

/// The display fields of the selected object, snapshotted per frame. `None`
/// (nothing selected) shows the empty-state prompt.
#[derive(Clone, Default)]
pub struct Snapshot {
	sel: Option<Sel>,
	/// The selected object's index in `project.objects` — the identity `sync`
	/// compares to know a *selection change* from a plain redraw. Without it a
	/// new selection whose value equals the old seed never reseeds its box, and
	/// the stale text can later commit onto the wrong unit.
	index: Option<usize>,
	/// Whether the max-values section shows its advanced (static) rows
	/// (`EditorState::unitprops_advanced`, S4.5).
	advanced: bool,
}

#[derive(Clone)]
struct Sel {
	/// The `ResourceID` — drives per-type applicability (which orders the
	/// Orders dropdown offers, via `resting_orders`).
	unit_type: u16,
	type_name: String,
	source_id: Option<u16>,
	team: u8,
	name: String,
	angle: u8,
	/// The turret's own heading (`ObjectProps::turret_angle`) — an independent 0-7
	/// direction for genuine turret units, editable via the Turret dropdown, shown
	/// only when `has_turret` (S4.4).
	turret_angle: u8,
	/// Whether this type carries turret frames (`object_has_turret`) — gates the
	/// Turret row in.
	has_turret: bool,
	hits: u16,
	/// The unit's max HP from the save, when known — shown as `cur / max` and the
	/// hits-editor clamp (S4.5).
	max_hits: Option<u16>,
	ammo: u8,
	orders: u8,
	/// Turns the unit stays disabled (`ObjectProps::disabled_turns`); a free-value
	/// box, 0 = not disabled.
	disabled_turns: u8,
	/// Cargo carried / experience accrued (`ObjectProps::storage`, signed). Edited
	/// in place like name/hits/ammo (S4.4).
	storage: i16,
	/// Connector adjacency bitmask (8 half-edge bits). Editable via the connector
	/// grid, shown only for connector-host types (S4.4).
	connectors: u16,
	/// Whether the connector grid applies to this type (`is_connector_host_type`);
	/// non-hosts hide it entirely.
	connector_host: bool,
	/// Building footprint (cells per side, 1 or 2) — sizes the connector grid
	/// (3×3 vs 4×4) and the centre sprite. `1` when the sprite library isn't loaded.
	footprint: u16,
	/// Ground cover stores a decorative *variant* in `angle` (not a heading) and
	/// has no orders, so those two rows show read-only (S4.5).
	ground_cover: bool,
	/// Per-stat applicability for this unit type
	/// (`EditorState::object_stat_applicable`, S7.5) — inapplicable stats drop
	/// their rows. All-true when the unit database is unavailable.
	applicable: [bool; VALUE_STATS.len()],
	/// The object's effective maximum stats (`EditorState::object_effective_values`):
	/// its per-unit override when edited, else the save's shared seed, else the
	/// stock unit-database seed (Stage B — works save-less). `None` (no stats
	/// anywhere) hides the whole max-values section (S4.5).
	values: Option<UnitValues>,
}

impl Snapshot {
	/// Read the selected object's display fields off the editor (empty when the
	/// selection is clear or stale).
	pub fn of(editor: &EditorState) -> Self {
		let picked = editor.selected_object.and_then(|i| editor.project.objects.get(i).map(|o| (i, o)));
		let index = picked.map(|(i, _)| i);
		let sel = picked.map(|(i, o)| Sel {
			unit_type: o.unit_type,
			type_name: max_assets::save::unit_type_name(o.unit_type).unwrap_or("object").to_string(),
			source_id: o.props.source_id,
			team: o.team,
			name: o.props.name.clone(),
			angle: o.props.angle,
			turret_angle: o.props.turret_angle,
			has_turret: editor.object_has_turret(i),
			hits: o.props.hits,
			max_hits: editor.object_max_hits(i),
			ammo: o.props.ammo,
			orders: o.props.orders,
			disabled_turns: o.props.disabled_turns,
			storage: o.props.storage,
			connectors: o.props.connectors,
			connector_host: max_assets::save::is_connector_host_type(o.unit_type),
			footprint: editor.object_footprint_of(i),
			ground_cover: max_assets::save::is_ground_cover_type(o.unit_type),
			applicable: applicability_mask(editor, i),
			values: editor.object_effective_values(i),
		});
		Self { sel, index, advanced: editor.unitprops_advanced }
	}

	/// The values-section visibility for a selection: presence of a stats block,
	/// the advanced toggle, and the applicability mask.
	fn vis(&self, sel: &Sel) -> StatsVis {
		StatsVis { present: sel.values.is_some(), advanced: self.advanced, mask: sel.applicable }
	}
}

/// Per-stat applicability of [`VALUE_STATS`] for object `idx` — the mask the
/// panel and shell share. All-true without the unit database (no data ⇒ no
/// restriction); a nonzero value in an inapplicable slot stays shown so
/// nothing an edited/modded save carries is ever hidden.
pub fn applicability_mask(editor: &EditorState, idx: usize) -> [bool; VALUE_STATS.len()] {
	std::array::from_fn(|i| editor.object_stat_applicable(idx, VALUE_STATS[i].kind))
}

/// An order byte's dropdown label — its `UnitOrderType` slug, or `#N` for a
/// byte outside the known enum.
fn order_label(order: u8) -> String {
	max_assets::save::order_name(order).map(str::to_string).unwrap_or_else(|| format!("#{order}"))
}

/// A heading value as `N  (name)`, or `variant N` when out of the 0-7 compass
/// range. Shared by the Facing row and the Turret dropdown (a turret angle is a
/// heading in the same `UNIT_ANGLE_*` space).
fn heading_label(angle: u8) -> String {
	match HEADINGS.get(angle as usize) {
		Some(name) => format!("{angle}  ({name})"),
		None => format!("variant {angle}"),
	}
}

/// The name shown beside the sprite preview in the header band: the unit's
/// custom name, or — unnamed — its proper in-game name ("Tank"). The
/// technical tag stays on the `type` row.
fn header_name(sel: &Sel) -> &str {
	if sel.name.is_empty() {
		max_assets::save::unit_display_name(&sel.type_name).unwrap_or(&sel.type_name)
	} else {
		&sel.name
	}
}

/// The `label : value` rows for a selected object, in display order. The rows
/// whose value is a live control (team swatches, the typed fields, the two
/// dropdowns) carry the string only for the read-only cases and the tests; the
/// tree draws whichever of the pair applies.
fn rows(sel: &Sel) -> [(&'static str, String); ROW_COUNT] {
	// The spatial-hash id as a hex value (the engine/save think in hex); placed
	// units carry no id until export.
	let id = sel.source_id.map(|i| format!("0x{i:04X}")).unwrap_or_else(|| "(new)".to_string());
	let name = if sel.name.is_empty() { "(default)".to_string() } else { sel.name.clone() };
	// Ground cover's `angle` is a decorative variant, not a heading; a mobile
	// unit at 0-7 names its direction (an out-of-range heading also reads as a
	// variant).
	let facing = if sel.ground_cover { format!("variant {}", sel.angle) } else { heading_label(sel.angle) };
	let hits = match sel.max_hits {
		Some(max) => format!("{} / {max}", sel.hits),
		None => sel.hits.to_string(),
	};
	let orders =
		max_assets::save::order_name(sel.orders).map(str::to_string).unwrap_or_else(|| format!("#{}", sel.orders));
	let team = TEAM_NAMES.get(sel.team as usize).copied().unwrap_or("?").to_string();
	let values = [
		sel.type_name.clone(),
		id,
		team,
		name,
		facing,
		hits,
		sel.ammo.to_string(),
		sel.storage.to_string(),
		orders,
		sel.disabled_turns.to_string(),
	];
	std::array::from_fn(|i| (ROW_LABELS[i], values[i].clone()))
}

/// The identity of an in-place editable box (item 8): a free-value field or the
/// i-th max-stat (index into [`VALUE_STATS`]). Each maps to a persistent
/// [`TextInput`] the tree hosts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldKey {
	Free(Field),
	Stat(usize),
}

impl FieldKey {
	/// The character set the box accepts.
	fn charset(self) -> Charset {
		match self {
			FieldKey::Free(Field::Name) => Charset::Text,
			FieldKey::Free(Field::Storage) => Charset::SignedInt, // signed cargo / experience
			_ => Charset::Digits,                                 // hits / ammo / every stat
		}
	}

	/// The max typed length (a name vs a 16-bit / signed-16 number).
	fn max_len(self) -> usize {
		match self {
			FieldKey::Free(Field::Name) => 40,
			_ => 6,
		}
	}

	/// The [`Commit`] this box emits, carrying the box's `text`.
	fn commit(self, text: String) -> Commit {
		match self {
			FieldKey::Free(f) => Commit::Field(f, text),
			FieldKey::Stat(i) => Commit::Value(VALUE_STATS[i].attr, text),
		}
	}

	/// The box's current value string, off the selection.
	fn value(self, sel: &Sel) -> String {
		match self {
			FieldKey::Free(Field::Name) => sel.name.clone(),
			FieldKey::Free(Field::Hits) => sel.hits.to_string(),
			FieldKey::Free(Field::Ammo) => sel.ammo.to_string(),
			FieldKey::Free(Field::Storage) => sel.storage.to_string(),
			FieldKey::Free(Field::Disabled) => sel.disabled_turns.to_string(),
			FieldKey::Stat(i) => {
				sel.values.as_ref().map(|v| VALUE_STATS[i].kind.get(v).to_string()).unwrap_or_default()
			}
		}
	}

	/// Whether the box shows for this selection (the free fields always; a stat
	/// only when the object carries a stats block and the stat is currently shown).
	fn visible(self, vis: StatsVis) -> bool {
		match self {
			FieldKey::Free(_) => true,
			FieldKey::Stat(i) => vis.shown(i),
		}
	}
}

/// The free-value fields, in top-to-bottom order — the boxes that show for every
/// selection. The stat boxes follow them in [`VALUE_STATS`] order.
const FREE_FIELDS: [Field; 5] = [Field::Name, Field::Hits, Field::Ammo, Field::Storage, Field::Disabled];

/// Every editable box in top-to-bottom order.
fn all_field_keys() -> Vec<FieldKey> {
	let mut keys: Vec<FieldKey> = FREE_FIELDS.iter().copied().map(FieldKey::Free).collect();
	keys.extend((0..VALUE_STATS.len()).map(FieldKey::Stat));
	keys
}

// --- content widgets ---------------------------------------------------------

/// A recessed square well the shell composites a native unit sprite into — the
/// header band's live preview (item 11). A **content widget** in the §7 sense:
/// it owns no behavior at all, only the rect the GPU pass needs, which the shell
/// reads back off the tree after `build`.
struct SpriteWell {
	id: WidgetId,
	side: f32,
	rect: Rect,
}

impl SpriteWell {
	fn new(side: f32) -> Self {
		Self { id: wgpu_ui::next_id(), side, rect: Rect::ZERO }
	}
}

impl Widget for SpriteWell {
	fn measure(&mut self, _avail: Size, _ctx: &mut LayoutCtx) -> Size {
		Size::new(self.side, self.side)
	}

	fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
		self.rect = rect;
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if ctx.is_base() {
			ctx.theme.well(dl, self.rect, WidgetState::default());
		}
	}

	fn rect(&self) -> Rect {
		self.rect
	}

	fn id(&self) -> WidgetId {
		self.id
	}

	/// Inert: the sprite well is a readout, never a pointer target.
	fn hit_test(&self, _pos: Vec2) -> Option<WidgetId> {
		None
	}
}

/// The connector-mask editor (S4.4): an `(f+2)²` grid whose inner `f×f` block is
/// the building (a well the shell composites the sprite into), whose four
/// corners are inert, and whose other perimeter cells are a **bare**
/// [`Checkbox`] toggling one half-edge bit — checked = connected.
///
/// A content widget rather than a [`wgpu_ui::Grid`]: the centre block *spans*
/// `f×f` cells and the corners are decoration, neither of which a uniform grid
/// of equal cells expresses. What it owns is the footprint geometry; the boxes
/// themselves are stock children, one per bit, and each fires its own action tag.
pub struct ConnectorGrid {
	id: WidgetId,
	/// One box per half-edge bit, in [`BITS`] order. A 1×1 footprint arranges
	/// only the four base bits; the rest sit collapsed at the origin.
	boxes: Vec<Checkbox>,
	footprint: u16,
	rect: Rect,
}

impl ConnectorGrid {
	fn new() -> Self {
		Self {
			id: wgpu_ui::next_id(),
			boxes: BITS.iter().map(|&b| Checkbox::new("").action(tag(KIND_CONNECTOR, u64::from(b)))).collect(),
			footprint: 1,
			rect: Rect::ZERO,
		}
	}

	fn set_footprint(&mut self, footprint: u16) {
		self.footprint = footprint.clamp(1, 2);
	}

	/// Push the object's live mask into the boxes (a click toggles optimistically;
	/// the round trip through `object-edit` is what confirms it).
	fn set_mask(&mut self, mask: u16) {
		for (b, bit) in self.boxes.iter_mut().zip(BITS) {
			b.set_checked(mask & bit != 0);
		}
	}

	/// The grid's side in px.
	fn side(&self) -> f32 {
		grid_side(self.footprint) as f32 * CGRID_CELL
	}

	/// The centre footprint rect — where the shell composites the building
	/// sprite thumbnail.
	fn footprint_rect(&self) -> Rect {
		footprint_rect(self.rect.min(), self.footprint)
	}
}

impl Widget for ConnectorGrid {
	fn measure(&mut self, _avail: Size, _ctx: &mut LayoutCtx) -> Size {
		Size::new(self.side(), self.side())
	}

	fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
		// The grid claims only the square it draws, at the top-left of whatever
		// slot it was given — the value column, so it lines up under the wells.
		self.rect = Rect::new(rect.x, rect.y, self.side(), self.side());
		let origin = self.rect.min();
		let footprint = self.footprint;
		for (b, bit) in self.boxes.iter_mut().zip(BITS) {
			match cell_of_bit(footprint, bit) {
				Some((row, col)) => b.arrange(connector_cell(origin, row, col), ctx),
				// This footprint has no such half-edge: collapse the box away.
				None => b.arrange(Rect::new(origin.x, origin.y, 0.0, 0.0), ctx),
			}
		}
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		let origin = self.rect.min();
		let n = grid_side(self.footprint);
		for row in 0..n {
			for col in 0..n {
				// The merged centre is drawn once, below; a checkbox cell is the
				// box's own; what is left is an inert corner.
				if is_footprint_cell(self.footprint, row, col) || connector_bit_at(self.footprint, row, col).is_some() {
					continue;
				}
				connector_corner(dl, connector_cell(origin, row, col));
			}
		}
		// The footprint block: a recessed well the shell composites the building
		// sprite over, ringed so it reads as the building even without sprites.
		let fp = self.footprint_rect();
		ctx.theme.well(dl, fp, WidgetState::default());
		dl.stroke_rect(fp, 1.0, rgba(theme::PANEL_BORDER));
		// Only the boxes this footprint has a cell for: a collapsed one is parked
		// at the origin, and a `Checkbox` centres its box in whatever rect it was
		// given — so drawing it would paint a stray well off the grid's corner.
		for (b, bit) in self.boxes.iter().zip(BITS) {
			if cell_of_bit(self.footprint, bit).is_some() {
				b.draw(dl, ctx);
			}
		}
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		let mut handled = false;
		for b in &mut self.boxes {
			handled |= b.event(ev, ctx);
		}
		handled
	}

	fn rect(&self) -> Rect {
		self.rect
	}

	fn id(&self) -> WidgetId {
		self.id
	}

	fn child_count(&self) -> usize {
		self.boxes.len()
	}

	fn child(&self, i: usize) -> Option<&dyn Widget> {
		self.boxes.get(i).map(|b| b as &dyn Widget)
	}

	fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
		self.boxes.get_mut(i).map(|b| b as &mut dyn Widget)
	}

	/// Only the checkbox cells claim the pointer — the corners, the footprint
	/// well and the slack beside the grid stay inert, exactly as the old `click`
	/// oracle had them.
	fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
		self.boxes.iter().find_map(|b| b.hit_test(pos))
	}
}

/// An inert grid corner: a dim, un-clickable box (no side connects here).
fn connector_corner(dl: &mut DrawList, cell: Rect) {
	let s = 14.0;
	let bx = Rect::new(cell.x + (cell.w - s) * 0.5, cell.y + (cell.h - s) * 0.5, s, s);
	dl.fill_rect(bx, rgba(theme::PRESS));
	dl.stroke_rect(bx, 1.0, rgba(theme::BEVEL.bottom));
}

// --- the tree ----------------------------------------------------------------

/// A muted caption in the label column.
fn caption(text: &str) -> Label {
	Label::new(text).small().muted().ellipsize()
}

/// A read-only value in the value column, with a stable id so `sync` can rewrite
/// it each frame.
fn readout() -> Label {
	Label::new("").small().ellipsize().with_id()
}

/// One `label : value` form row — the 50/50 split the panel has always had
/// (item 9), expressed as two flex halves rather than a computed column width.
fn form_row(label: &str, value: impl Widget + 'static) -> Linear {
	Linear::row()
		.cross_align(CrossAlign::Stretch)
		.child(caption(label), Length::Flex(1.0))
		.child(value, Length::Flex(1.0))
}

/// A section heading: a muted caption over the theme's rule, matching the Save
/// Toolbox groups.
fn section_head(label: &str) -> Linear {
	Linear::column()
		.cross_align(CrossAlign::Stretch)
		.child(Label::new(label).small().muted(), Length::Flex(1.0))
		.child(Separator::new(), Length::Fixed(RULE_H))
}

/// The ids the panel reaches its retained children by. Everything the tree can
/// show or hide is a [`Reveal`]; everything it can rewrite is a `Label`,
/// `TextInput`, `Select`, `ColorButton` or `Checkbox`.
struct Ids {
	/// The "nothing selected" prompt, and the whole form — exactly one is shown.
	empty: WidgetId,
	form: WidgetId,
	preview: WidgetId,
	/// The unit's name in the header band.
	name: WidgetId,
	/// The Type and Id readouts.
	type_value: WidgetId,
	id_value: WidgetId,
	swatches: [WidgetId; TEAM_NAMES.len()],
	/// The three dropdowns, in [`SELECT_KINDS`] order.
	selects: [WidgetId; SELECT_KINDS.len()],
	/// Facing / Orders: the dropdown's slot, and the read-only label's slot +
	/// label (ground cover has neither a heading nor orders).
	dropdown_slot: [WidgetId; 2],
	readonly_slot: [WidgetId; 2],
	readonly: [WidgetId; 2],
	/// The Turret row's slot.
	turret_row: WidgetId,
	/// One [`TextInput`] per [`all_field_keys`] entry.
	fields: Vec<WidgetId>,
	/// The values section: its leading gap, its header row, the advanced
	/// checkbox, and one slot per [`VALUE_STATS`] row.
	values_gap: WidgetId,
	values_head: WidgetId,
	advanced: WidgetId,
	stat_rows: Vec<WidgetId>,
	/// The connector section: its leading gap, its row, and the grid.
	conn_gap: WidgetId,
	conn_row: WidgetId,
	grid: WidgetId,
}

/// Build the panel's tree once: a `ScrollArea` over a column of sections.
///
/// The column is a plain `Linear`, not a `Wrap`: this panel is a **form**, so its
/// rows stack at every dock width (the flowed key banks of the toolboxes are the
/// other shape). Every row is a `Length::Fixed(ROW_H)` slot, or — where the row
/// is optional — a `Reveal` whose own `height` names the same row, because a
/// `Fit` parent is what lets it collapse and a `Fixed` one would hold the gap
/// open.
fn build() -> (ScrollArea, Ids) {
	let keys = all_field_keys();
	let mut fields = Vec::with_capacity(keys.len());

	// --- the header band: the live preview well beside the unit's name --------
	let preview = SpriteWell::new(PREVIEW);
	let preview_id = preview.id();
	let name = Label::new("").title().ellipsize().with_id();
	let name_id = name.id();
	let header = Linear::column()
		.cross_align(CrossAlign::Stretch)
		.child(
			Linear::row()
				.spacing(PAD)
				.cross_align(CrossAlign::Center)
				.child(preview, Length::Fixed(PREVIEW))
				.child(name, Length::Flex(1.0)),
			Length::Flex(1.0),
		)
		.child(Separator::new(), Length::Fixed(RULE_H));

	// --- the `object` section -------------------------------------------------
	let type_value = readout();
	let type_value_id = type_value.id();
	let id_value = readout();
	let id_value_id = id_value.id();

	let mut swatch_bank = Linear::row().spacing(SWGAP).cross_align(CrossAlign::Center);
	let mut swatches = [WidgetId::NONE; TEAM_NAMES.len()];
	for (t, slot) in swatches.iter_mut().enumerate() {
		// 3px inset (1 up from the default): a wider face ring, so the owning
		// team's key reads at a glance.
		let key = ColorButton::new(rgba(TEAM_SWATCH[t]), SW, SW).inset(3.0).action(tag(KIND_TEAM, t as u64));
		*slot = key.id();
		swatch_bank = swatch_bank.child(key, Length::Fixed(SW));
	}

	// The two enumerable rows carry *both* controls: a hosted `Select` for a
	// real unit, a read-only label for ground cover (whose `angle` is a
	// decorative variant and which has no orders at all). Exactly one shows.
	let mut selects = [WidgetId::NONE; SELECT_KINDS.len()];
	let mut dropdown_slot = [WidgetId::NONE; 2];
	let mut readonly_slot = [WidgetId::NONE; 2];
	let mut readonly = [WidgetId::NONE; 2];
	let mut enum_cell = |i: usize| -> Linear {
		let select = Select::new(select_labels(SELECT_KINDS[i])).small();
		selects[i] = select.id();
		let drop = Reveal::new(select).height(ROW_H);
		dropdown_slot[i] = drop.id();
		let label = readout();
		readonly[i] = label.id();
		let plain = Reveal::new(label).height(ROW_H).with_shown(false);
		readonly_slot[i] = plain.id();
		Linear::column().cross_align(CrossAlign::Stretch).child(drop, Length::Fit).child(plain, Length::Fit)
	};
	let facing_cell = enum_cell(0);
	let orders_cell = enum_cell(1);

	// One persistent box per editable field, in `all_field_keys` order.
	let mut field_box = |key: FieldKey| -> TextInput {
		let input = TextInput::new().charset(key.charset()).max_len(key.max_len());
		fields.push(input.id());
		input
	};
	let name_box = field_box(FieldKey::Free(Field::Name));
	let hits_box = field_box(FieldKey::Free(Field::Hits));
	let ammo_box = field_box(FieldKey::Free(Field::Ammo));
	let storage_box = field_box(FieldKey::Free(Field::Storage));
	let disabled_box = field_box(FieldKey::Free(Field::Disabled));

	let mut form = Linear::column()
		.cross_align(CrossAlign::Stretch)
		.child(header, Length::Fixed(HEADER_H))
		.child(wgpu_ui::Spacer::new(), Length::Fixed(PAD))
		.child(section_head("object"), Length::Fixed(HEAD_H))
		.child(form_row(ROW_LABELS[TYPE_ROW], type_value), Length::Fixed(ROW_H))
		.child(form_row(ROW_LABELS[ID_ROW], id_value), Length::Fixed(ROW_H))
		.child(form_row(ROW_LABELS[TEAM_ROW], swatch_bank), Length::Fixed(ROW_H))
		.child(form_row(ROW_LABELS[NAME_ROW], name_box), Length::Fixed(ROW_H))
		.child(form_row(ROW_LABELS[FACING_ROW], facing_cell), Length::Fixed(ROW_H))
		.child(form_row(ROW_LABELS[HITS_ROW], hits_box), Length::Fixed(ROW_H))
		.child(form_row(ROW_LABELS[AMMO_ROW], ammo_box), Length::Fixed(ROW_H))
		.child(form_row(ROW_LABELS[STORAGE_ROW], storage_box), Length::Fixed(ROW_H))
		.child(form_row(ROW_LABELS[ORDERS_ROW], orders_cell), Length::Fixed(ROW_H))
		.child(form_row(ROW_LABELS[DISABLED_ROW], disabled_box), Length::Fixed(ROW_H));

	// --- the Turret row (units with an independent turret only) --------------
	let turret = Select::new(select_labels(SelectKind::Turret)).small();
	selects[2] = turret.id();
	let turret_row = Reveal::new(form_row("Turret", turret)).height(ROW_H).with_shown(false);
	let turret_row_id = turret_row.id();
	form = form.child(turret_row, Length::Fit);

	// --- the max-values section (objects with a stats block only) -------------
	let values_gap = Reveal::new(wgpu_ui::Spacer::new()).height(SECTION_GAP).with_shown(false);
	let values_gap_id = values_gap.id();
	let advanced = Checkbox::new("").action(tag(KIND_ADVANCED, 0));
	let advanced_id = advanced.id();
	// The values heading doubles as the advanced toggle's row: the caption takes
	// the leftover width, the toggle sits at the right edge, and the rule runs
	// the full width under both.
	let values_head = Reveal::new(
		Linear::column()
			.cross_align(CrossAlign::Stretch)
			.child(
				Linear::row()
					.cross_align(CrossAlign::Stretch)
					.child(Label::new("values").small().muted(), Length::Flex(1.0))
					.child(Label::new("advanced").small().muted().align(TextAlign::Right), Length::Fixed(ADVANCED_W))
					.child(advanced, Length::Fixed(CHECK_W)),
				Length::Flex(1.0),
			)
			.child(Separator::new(), Length::Fixed(RULE_H)),
	)
	.height(ROW_H)
	.with_shown(false);
	let values_head_id = values_head.id();
	form = form.child(values_gap, Length::Fit).child(values_head, Length::Fit);

	let mut stat_rows = Vec::with_capacity(VALUE_STATS.len());
	for (i, stat) in VALUE_STATS.iter().enumerate() {
		let row = Reveal::new(form_row(stat.label, field_box(FieldKey::Stat(i)))).height(ROW_H).with_shown(false);
		stat_rows.push(row.id());
		form = form.child(row, Length::Fit);
	}

	// --- the connector grid (connector-host types only) ----------------------
	let conn_gap = Reveal::new(wgpu_ui::Spacer::new()).height(SECTION_GAP).with_shown(false);
	let conn_gap_id = conn_gap.id();
	let grid = ConnectorGrid::new();
	let grid_id = grid.id();
	// The grid is a *picture* of the building and its edges, not a field: it
	// gets its own line under the caption, centred across the panel, rather
	// than sitting in the value column beside a label.
	let conn_row = Reveal::new(
		Linear::column()
			.spacing(CONN_CAPTION_GAP)
			.cross_align(CrossAlign::Stretch)
			.child(caption("Connect"), Length::Fixed(ROW_H))
			.child(
				Linear::row()
					.child(wgpu_ui::Spacer::new(), Length::Flex(1.0))
					.child(grid, Length::Fit)
					.child(wgpu_ui::Spacer::new(), Length::Flex(1.0)),
				Length::Fit,
			),
	)
	.with_shown(false);
	let conn_row_id = conn_row.id();
	form = form.child(conn_gap, Length::Fit).child(conn_row, Length::Fit);

	// --- the two top-level states --------------------------------------------
	let empty =
		Reveal::new(Linear::column().padding(Insets::symmetric(0.0, PAD)).cross_align(CrossAlign::Stretch).child(
			Label::new("No object selected. Pick one with the Select tool (V).").small().muted().wrap(),
			Length::Fit,
		));
	let empty_id = empty.id();
	let form_slot = Reveal::new(form).with_shown(false);
	let form_id = form_slot.id();

	let body = Linear::column()
		.padding(Insets { left: PAD, top: 0.0, right: PAD, bottom: PAD })
		.cross_align(CrossAlign::Stretch)
		.child(empty, Length::Fit)
		.child(form_slot, Length::Fit);

	// `WhenHovered` is the panel accelerator rule — and, here, the promise that a
	// press on the form's inert chrome will not blur (and so commit) the field
	// the user is still typing in.
	let root = ScrollArea::new(body).page_keys(PageKeys::WhenHovered);
	let ids = Ids {
		empty: empty_id,
		form: form_id,
		preview: preview_id,
		name: name_id,
		type_value: type_value_id,
		id_value: id_value_id,
		swatches,
		selects,
		dropdown_slot,
		readonly_slot,
		readonly,
		turret_row: turret_row_id,
		fields,
		values_gap: values_gap_id,
		values_head: values_head_id,
		advanced: advanced_id,
		stat_rows,
		conn_gap: conn_gap_id,
		conn_row: conn_row_id,
		grid: grid_id,
	};
	(root, ids)
}

/// The Unit Properties panel as a retained `wgpu_ui` [`Widget`]: a thin root over
/// the built tree, holding the id tables, the field→key mapping and the commit
/// queue. Everything else — layout, paint, hover, arming, firing, scrolling,
/// focus — is the tree's.
pub struct UnitPropsContent {
	id: WidgetId,
	root: ScrollArea,
	ids: Ids,
	/// Parallel to `ids.fields`: which value each box edits.
	keys: Vec<FieldKey>,
	/// The last value pushed into each box (`None` = must reseed), so `sync`
	/// reseeds only on a real change — never clobbering what the user is
	/// mid-typing. Every entry drops to `None` when the selection moves or the
	/// box hides (a hidden `Reveal` is unreachable, so its box can go stale
	/// invisibly).
	seeded: Vec<Option<String>>,
	/// Which object index the boxes were seeded for; a different index in the
	/// next snapshot forces a full reseed (see [`Snapshot::index`]).
	seeded_for: Option<usize>,
	/// Indices (into `ids.fields`) shown for the current selection, top to bottom.
	visible: Vec<usize>,
	/// The order byte behind each row of the (per-type filtered) Orders
	/// dropdown — the pick index translates through this before firing, so the
	/// action always carries the order byte itself.
	orders_options: Vec<u8>,
	/// Committed edits queued for the shell (Enter / focus-out).
	commits: Vec<Commit>,
	rect: Rect,
}

impl Default for UnitPropsContent {
	fn default() -> Self {
		Self::new()
	}
}

impl UnitPropsContent {
	pub fn new() -> Self {
		let (root, ids) = build();
		let keys = all_field_keys();
		let seeded = vec![None; keys.len()];
		// Nothing is selected until the first `sync`, so only the free fields are
		// in the tree's shown set.
		let visible = (0..FREE_FIELDS.len()).collect();
		Self {
			id: wgpu_ui::next_id(),
			root,
			ids,
			keys,
			seeded,
			seeded_for: None,
			visible,
			orders_options: Vec::new(),
			commits: Vec::new(),
			rect: Rect::ZERO,
		}
	}

	/// Push one frame's state into the retained tree — **top-down**: a `Reveal`
	/// hides its whole subtree from every tree walk, so each slot is shown or
	/// hidden *before* anything inside it is reached.
	pub fn sync(&mut self, snap: Snapshot) {
		// A selection change invalidates every box: text still sitting in one
		// belongs to the previous object (the shell drained the real commits
		// before the selection moved), and a new value that happens to equal the
		// old seed must still be pushed. Forget the seeds and drop any commit
		// the selection change would otherwise misdeliver.
		if self.seeded_for != snap.index {
			self.seeded_for = snap.index;
			self.seeded.iter_mut().for_each(|s| *s = None);
			self.commits.clear();
		}
		let Some(sel) = snap.sel.clone() else {
			self.set_shown(self.ids.empty, true);
			self.set_shown(self.ids.form, false);
			self.visible.clear();
			return;
		};
		let vis = snap.vis(&sel);
		self.set_shown(self.ids.empty, false);
		self.set_shown(self.ids.form, true);

		// The header band.
		let row_values = rows(&sel);
		self.set_text(self.ids.name, header_name(&sel));
		self.set_text(self.ids.type_value, &row_values[TYPE_ROW].1);
		self.set_text(self.ids.id_value, &row_values[ID_ROW].1);

		// The team swatches: the owner's key reads selected.
		for (t, &id) in self.ids.swatches.iter().enumerate() {
			if let Some(key) = descendant_mut::<ColorButton>(&mut self.root, id) {
				key.set_selected(t as u8 == sel.team);
			}
		}

		// Facing / Orders: a live dropdown, or the read-only readout for ground
		// cover. Show the slot first, then write into what it holds.
		for (i, row) in [FACING_ROW, ORDERS_ROW].into_iter().enumerate() {
			self.set_shown(self.ids.dropdown_slot[i], !sel.ground_cover);
			self.set_shown(self.ids.readonly_slot[i], sel.ground_cover);
			if sel.ground_cover {
				self.set_text(self.ids.readonly[i], &row_values[row].1);
			}
		}
		if !sel.ground_cover {
			if let Some(s) = descendant_mut::<Select>(&mut self.root, self.ids.selects[0]) {
				s.set_selected(usize::from(sel.angle));
			}
			// The Orders dropdown offers only the orders this type can hold at
			// rest (`resting_orders`) — plus the current value when it is
			// outside that set (an in-game runtime order still displays; picking
			// it back is a no-op edit). The pick is translated through this
			// list, so the fired value is always the order BYTE.
			self.orders_options = max_assets::save::resting_orders(sel.unit_type).to_vec();
			if !self.orders_options.contains(&sel.orders) {
				self.orders_options.push(sel.orders);
			}
			if let Some(s) = descendant_mut::<Select>(&mut self.root, self.ids.selects[1]) {
				let labels = self.orders_options.iter().map(|&o| order_label(o));
				s.set_options(labels.collect::<Vec<_>>());
				let pos = self.orders_options.iter().position(|&o| o == sel.orders).unwrap_or(0);
				s.set_selected(pos);
			}
		}

		// The Turret row.
		self.set_shown(self.ids.turret_row, sel.has_turret);
		if sel.has_turret
			&& let Some(s) = descendant_mut::<Select>(&mut self.root, self.ids.selects[2])
		{
			s.set_selected(usize::from(sel.turret_angle));
		}

		// The max-values section, then one slot per stat row.
		self.set_shown(self.ids.values_gap, vis.present);
		self.set_shown(self.ids.values_head, vis.present);
		if vis.present
			&& let Some(cb) = descendant_mut::<Checkbox>(&mut self.root, self.ids.advanced)
		{
			cb.set_checked(vis.advanced);
		}
		for i in 0..VALUE_STATS.len() {
			self.set_shown(self.ids.stat_rows[i], vis.shown(i));
		}

		// The connector grid.
		self.set_shown(self.ids.conn_gap, sel.connector_host);
		self.set_shown(self.ids.conn_row, sel.connector_host);
		if sel.connector_host
			&& let Some(grid) = descendant_mut::<ConnectorGrid>(&mut self.root, self.ids.grid)
		{
			grid.set_footprint(sel.footprint);
			grid.set_mask(sel.connectors);
		}

		// The editable boxes: record the shown order, and reseed only what really
		// changed (a box the user is typing in keeps its text).
		self.visible.clear();
		for i in 0..self.keys.len() {
			let key = self.keys[i];
			if !key.visible(vis) {
				// Unreachable while hidden (its `Reveal` drops it from every
				// walk) — forget the seed so re-showing always reseeds.
				self.seeded[i] = None;
				continue;
			}
			self.visible.push(i);
			let value = key.value(&sel);
			if self.seeded[i].as_deref() != Some(value.as_str()) {
				if let Some(input) = descendant_mut::<TextInput>(&mut self.root, self.ids.fields[i]) {
					input.set_text(value.clone());
					// Inside the `if let` on purpose: a failed lookup must not
					// mark the box seeded.
					self.seeded[i] = Some(value);
				}
			}
		}
	}

	/// Show or hide one [`Reveal`] slot.
	fn set_shown(&mut self, id: WidgetId, shown: bool) {
		if let Some(slot) = descendant_mut::<Reveal>(&mut self.root, id) {
			slot.set_shown(shown);
		}
	}

	/// Rewrite one readout `Label`.
	fn set_text(&mut self, id: WidgetId, text: &str) {
		if let Some(label) = descendant_mut::<Label>(&mut self.root, id) {
			label.set_text(text);
		}
	}

	/// The next queued in-place edit [`Commit`] (Enter / focus-out), or `None`.
	/// The shell drains these each dispatch into `object-edit` / `object-values`.
	pub fn take_commit(&mut self) -> Option<Commit> {
		(!self.commits.is_empty()).then(|| self.commits.remove(0))
	}

	/// The live scroll offset (px) — read back after `build`, which is what
	/// settles it. Test-only: the shell reads the *rects* it needs off the tree
	/// (see [`Self::preview_rect`]), never the offset they hang off.
	#[cfg(test)]
	pub fn scroll(&self) -> f32 {
		self.root.offset()
	}

	/// The header band's live-preview well, for the native units pass. Read
	/// *after* `build` — that is what settles the scroll offset it hangs off
	/// (the U5.3 invariant; here the borrow allows reading the geometry back
	/// rather than recomputing it).
	pub fn preview_rect(&self) -> Option<Rect> {
		let well = descendant::<SpriteWell>(&self.root, self.ids.preview)?;
		(!well.rect.is_empty()).then_some(well.rect)
	}

	/// The connector grid's centre footprint well, for the same pass. `None`
	/// when the selection has no connector grid.
	pub fn connector_rect(&self) -> Option<Rect> {
		let grid = descendant::<ConnectorGrid>(&self.root, self.ids.grid)?;
		(!grid.rect.is_empty()).then(|| grid.footprint_rect())
	}

	/// The arranged rect of each visible field box, in top-to-bottom order (Name,
	/// Hits, Ammo, Storage, Disabled, then the shown stats) — for the focus/commit
	/// tests.
	#[cfg(test)]
	pub fn field_rects_for_test(&self) -> Vec<Rect> {
		self.visible
			.iter()
			.filter_map(|&i| descendant::<TextInput>(&self.root, self.ids.fields[i]).map(Widget::rect))
			.collect()
	}

	/// The caret byte offset in the `i`-th visible field — parallel to
	/// [`Self::field_rects_for_test`]. `TextInput` exposes no selection getter, so
	/// the drag-select test reads the caret: extending a selection moves it.
	#[cfg(test)]
	pub fn field_caret_for_test(&self, i: usize) -> usize {
		descendant::<TextInput>(&self.root, self.ids.fields[self.visible[i]]).map_or(0, TextInput::caret)
	}

	/// The arranged closed-box rect of one dropdown — where a test aims a click
	/// to open it (the tree owns the placement now).
	#[cfg(test)]
	pub fn select_rect_for_test(&self, kind: SelectKind) -> Rect {
		self.select_for_test(kind).rect()
	}

	/// The arranged option-list rect of one dropdown — the geometry a test aims
	/// a pick at.
	#[cfg(test)]
	pub fn select_popup_for_test(&self, kind: SelectKind) -> Rect {
		self.select_for_test(kind).popup_rect()
	}

	#[cfg(test)]
	fn select_for_test(&self, kind: SelectKind) -> &Select {
		let i = SELECT_KINDS.iter().position(|&k| k == kind).expect("a known dropdown");
		descendant::<Select>(&self.root, self.ids.selects[i]).expect("the dropdown is in the tree")
	}

	/// Drain the boxes' own commit signals into the panel's queue — Enter, or
	/// focus leaving a box for anywhere at all (U4.1). Called after each event
	/// reaches the fields.
	///
	/// This replaced a hand-rolled `pending_commit` that read the *pre-event*
	/// focus and asked "is this press outside the focused box?". That could only
	/// ever see presses routed to this panel, so Tab (which the `Ui` consumes
	/// before any widget sees it), a click in another panel, and a click on the
	/// map each dropped the typed edit.
	fn drain_field_commits(&mut self) {
		for k in 0..self.visible.len() {
			let i = self.visible[k];
			let Some(input) = descendant_mut::<TextInput>(&mut self.root, self.ids.fields[i]) else { continue };
			if input.take_commit().is_some() {
				let text = input.text().to_string();
				self.commits.push(self.keys[i].commit(text));
			}
		}
	}
}

impl Widget for UnitPropsContent {
	crate::panel_ui::thin_root_plumbing!(arrange, draw);

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		let handled = self.root.event(ev, ctx);
		// A `Select` commits on the press, which is why the shell drains this
		// panel after the press dispatch too. The Orders list is per-type
		// filtered, so its pick index is translated to the order byte it stands
		// for; Facing/Turret rows are the value itself.
		let orders_options = &self.orders_options;
		crate::panel_ui::drain_selects(&mut self.root, &self.ids.selects, ctx, |i, picked| {
			let v =
				if SELECT_KINDS[i] == SelectKind::Orders { usize::from(*orders_options.get(picked)?) } else { picked };
			(v <= u8::MAX as usize).then(|| tag(KIND_PICK, ((i as u64) << 8) | v as u64))
		});
		self.drain_field_commits();
		handled
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::uikit_menu::MenuChrome;
	use wgpu_ui::{Modifiers, PointerButton, Ui};

	fn sel(angle: u8, orders: u8) -> Sel {
		Sel {
			unit_type: 0x33, // TANK
			type_name: "TANK".into(),
			source_id: Some(7),
			team: 2,
			name: String::new(),
			angle,
			turret_angle: 0,
			has_turret: false,
			hits: 20,
			max_hits: Some(40),
			ammo: 4,
			orders,
			disabled_turns: 0,
			storage: 8,
			connectors: 0,
			connector_host: false,
			footprint: 1,
			ground_cover: false,
			values: None,
			applicable: [true; VALUE_STATS.len()],
		}
	}

	fn sel_ground_cover(angle: u8) -> Sel {
		Sel { ground_cover: true, max_hits: None, unit_type: 0x11, type_name: "LRGSLAB".into(), ..sel(angle, 0) }
	}

	/// A dummy stats block for exercising the max-values section.
	fn sample_values() -> UnitValues {
		UnitValues {
			turns: 3,
			hits: 40,
			armor: 8,
			attack: 16,
			speed: 16,
			range: 6,
			rounds: 1,
			move_and_fire: 0,
			scan: 6,
			storage: 0,
			ammo: 8,
			attack_radius: 0,
			agent_adjust: 0,
			version: 2,
			in_use: true,
		}
	}

	fn snapshot(sel: Sel, advanced: bool) -> Snapshot {
		Snapshot { sel: Some(sel), index: Some(0), advanced }
	}

	/// Like [`snapshot`], for a selection at a specific object index.
	fn snapshot_at(index: usize, sel: Sel, advanced: bool) -> Snapshot {
		Snapshot { sel: Some(sel), index: Some(index), advanced }
	}

	/// The panel hosted in a real `Ui` on the chrome fixture, synced and laid out
	/// into `body`. A stock `Button`/`Label` measures its text, so this needs the
	/// real fonts, not a bare `Fonts::new()`.
	fn hosted(snap: Snapshot, body: Rect) -> (MenuChrome, Ui, WidgetId) {
		let (_device, _queue, chrome) = crate::visual_test::chrome_fixture();
		let content = UnitPropsContent::new();
		let id = content.id();
		let mut ui = Ui::new(content);
		ui.get_mut::<UnitPropsContent>(id).expect("typed root").sync(snap);
		ui.set_viewport(Rect::new(0.0, 0.0, 1280.0, 800.0));
		ui.layout_in(body, chrome.theme(), chrome.fonts());
		(chrome, ui, id)
	}

	fn press(pressed: bool, at: Vec2) -> Event {
		Event::PointerButton { button: PointerButton::Primary, pressed, pos: at, mods: Modifiers::NONE }
	}

	/// Press + release at `at`, then the actions that fired.
	fn click(ui: &mut Ui, at: Vec2) -> Vec<Action> {
		ui.dispatch(&[press(true, at)]);
		ui.dispatch(&[press(false, at)]);
		ui.actions().iter().copied().filter_map(action_of).collect()
	}

	fn panel(ui: &Ui, id: WidgetId) -> &UnitPropsContent {
		ui.get::<UnitPropsContent>(id).expect("typed root")
	}

	#[test]
	fn rows_cover_the_editable_fields_in_order() {
		let r = rows(&sel(2, 0x0C));
		let labels: Vec<&str> = r.iter().map(|(l, _)| *l).collect();
		assert_eq!(labels, ["Type", "Id", "Team", "Name", "Facing", "Hits", "Ammo", "Storage", "Orders", "Disabled"]);
		assert_eq!(r[1].1, "0x0007", "the id is shown as a 4-digit hex value");
		assert_eq!(r[TEAM_ROW].0, "Team");
		assert_eq!(r[STORAGE_ROW].0, "Storage");
		assert_eq!(r[DISABLED_ROW].0, "Disabled");
	}

	#[test]
	fn empty_name_shows_default_and_orders_resolve() {
		let r = rows(&sel(2, 0x0C));
		assert_eq!(r[3].1, "(default)", "row 3 (Name): an empty custom name reads as the type default");
		assert_eq!(r[ORDERS_ROW].1, "sentry", "order 0x0C is sentry");
	}

	#[test]
	fn header_prefers_the_custom_name_then_the_in_game_name() {
		let mut s = sel(2, 0);
		assert_eq!(header_name(&s), "Tank", "unnamed: the proper in-game name, not the TANK tag");
		s.name = "Grid Alpha".into();
		assert_eq!(header_name(&s), "Grid Alpha");
		s.name.clear();
		s.type_name = "LRGRUBLE".into();
		assert_eq!(header_name(&s), "LRGRUBLE", "a type without an in-game name falls back to its tag");
	}

	#[test]
	fn facing_names_headings_variant_and_hits_shows_cap() {
		assert_eq!(rows(&sel(2, 0))[FACING_ROW].1, "2  (E)", "a mobile heading names its direction");
		assert_eq!(rows(&sel(11, 0))[FACING_ROW].1, "variant 11", "an out-of-range angle is a variant");
		assert_eq!(rows(&sel(0, 200))[ORDERS_ROW].1, "#200", "an unknown order shows its raw byte");
		// Ground cover's angle is always a variant, even at a heading-range value.
		assert_eq!(rows(&sel_ground_cover(2))[FACING_ROW].1, "variant 2", "ground cover angle is a variant");
		// Hits shows the cap when known (S4.5), else bare.
		assert_eq!(rows(&sel(2, 0))[HITS_ROW].1, "20 / 40", "current / max when the cap is known");
		assert_eq!(rows(&sel_ground_cover(0))[HITS_ROW].1, "20", "bare hits when no cap is known");
		// Storage is a free-value field showing its signed integer verbatim.
		assert_eq!(rows(&sel(2, 0))[STORAGE_ROW].1, "8", "storage shows its raw value");
	}

	/// The Orders dropdown offers only the orders the selected type can hold
	/// at rest; an out-of-set runtime order the save carries is appended so it
	/// still displays, and the pick path translates indices through this list.
	#[test]
	fn orders_dropdown_offers_only_resting_orders() {
		use max_assets::save::{ORDER_AWAIT, ORDER_DISABLE, ORDER_SENTRY};
		let mut content = UnitPropsContent::new();
		content.sync(snapshot(sel(2, ORDER_SENTRY), false)); // a TANK on sentry
		assert_eq!(content.orders_options, vec![ORDER_AWAIT, ORDER_SENTRY, ORDER_DISABLE], "mobile resting set");
		// A runtime order (here 0x01 = move) still displays, appended.
		content.sync(snapshot(sel(2, 0x01), false));
		assert_eq!(content.orders_options, vec![ORDER_AWAIT, ORDER_SENTRY, ORDER_DISABLE, 0x01]);
	}

	/// Regression (2026-08-11): editing a box and then selecting a DIFFERENT
	/// object whose value equals the old seed must reseed the box — the stale
	/// text used to survive the switch (and could later commit the first unit's
	/// cargo onto the second). A selection change must also drop any commit
	/// still queued for the old object.
	#[test]
	fn selection_change_reseeds_boxes_and_drops_stale_commits() {
		let storage = all_field_keys().iter().position(|&k| k == FieldKey::Free(Field::Storage)).expect("storage box");
		let mut content = UnitPropsContent::new();
		let mut a = sel(2, 0);
		a.storage = 0;
		content.sync(snapshot_at(0, a, false));
		let id = content.ids.fields[storage];
		let box_text =
			|c: &UnitPropsContent| descendant::<TextInput>(&c.root, id).expect("storage box").text().to_string();
		assert_eq!(box_text(&content), "0", "seeded from object 0");

		// The user types a new cargo amount (uncommitted), and a stale commit is
		// still queued when the selection moves.
		descendant_mut::<TextInput>(&mut content.root, id).expect("storage box").set_text("5");
		content.commits.push(FieldKey::Free(Field::Storage).commit("5".into()));

		// Object 1 has the same storage value as object 0's old seed.
		let mut b = sel(2, 0);
		b.storage = 0;
		content.sync(snapshot_at(1, b, false));
		assert_eq!(box_text(&content), "0", "a new selection reseeds even when its value equals the old seed");
		assert!(content.take_commit().is_none(), "the queued commit for object 0 is dropped, never delivered to 1");

		// Same index again = a plain redraw: the user's in-progress text stays.
		descendant_mut::<TextInput>(&mut content.root, id).expect("storage box").set_text("7");
		let mut b2 = sel(2, 0);
		b2.storage = 0;
		content.sync(snapshot_at(1, b2, false));
		assert_eq!(box_text(&content), "7", "a redraw of the same selection never clobbers mid-typing text");
	}

	/// `connector_bit_at` matches the engine's half-edge layout: a 1×1 footprint
	/// uses the four base bits (`0x55`); a 2×2 uses all eight, two per side. And
	/// [`cell_of_bit`] is its exact inverse — the mapping the grid arranges by.
	#[test]
	fn connector_bit_layout() {
		// 1×1 (3×3 grid): N/E/S/W midpoints → NL/ET/SL/WT; corners + centre None.
		assert_eq!(connector_bit_at(1, 0, 1), Some(0x01), "north -> NL");
		assert_eq!(connector_bit_at(1, 1, 2), Some(0x04), "east -> ET");
		assert_eq!(connector_bit_at(1, 2, 1), Some(0x10), "south -> SL");
		assert_eq!(connector_bit_at(1, 1, 0), Some(0x40), "west -> WT");
		let one_by_one: u16 =
			(0..3).flat_map(|r| (0..3).map(move |c| (r, c))).filter_map(|(r, c)| connector_bit_at(1, r, c)).sum();
		assert_eq!(one_by_one, 0x55, "a 1x1 host's cells cover exactly NL|ET|SL|WT");
		for (r, c) in [(0, 0), (0, 2), (2, 0), (2, 2), (1, 1)] {
			assert_eq!(connector_bit_at(1, r, c), None, "corner/centre ({r},{c}) has no bit");
		}
		// 2×2 (4×4 grid): each side has two half-edges, `base << k`.
		assert_eq!(connector_bit_at(2, 0, 1), Some(0x01), "NL");
		assert_eq!(connector_bit_at(2, 0, 2), Some(0x02), "NR");
		assert_eq!(connector_bit_at(2, 1, 3), Some(0x04), "ET");
		assert_eq!(connector_bit_at(2, 2, 3), Some(0x08), "EB");
		assert_eq!(connector_bit_at(2, 3, 1), Some(0x10), "SL");
		assert_eq!(connector_bit_at(2, 3, 2), Some(0x20), "SR");
		assert_eq!(connector_bit_at(2, 1, 0), Some(0x40), "WT");
		assert_eq!(connector_bit_at(2, 2, 0), Some(0x80), "WB");
		let two_by_two: u16 =
			(0..4).flat_map(|r| (0..4).map(move |c| (r, c))).filter_map(|(r, c)| connector_bit_at(2, r, c)).sum();
		assert_eq!(two_by_two, 0xFF, "a 2x2 host's cells cover all eight half-edges");
		for (r, c) in [(0, 0), (0, 3), (3, 0), (3, 3), (1, 1), (2, 2)] {
			assert_eq!(connector_bit_at(2, r, c), None, "corner/centre ({r},{c}) has no bit");
		}
		// The inverse agrees, and a 1×1 host simply has no second half-edges.
		for f in [1u16, 2] {
			for bit in BITS {
				match cell_of_bit(f, bit) {
					Some((r, c)) => assert_eq!(connector_bit_at(f, r, c), Some(bit), "f={f} bit={bit:#x}"),
					None => assert_eq!(f, 1, "only a 1x1 host drops a bit ({bit:#x})"),
				}
			}
		}
		assert_eq!(cell_of_bit(1, 0x02), None, "a 1x1 host has no NR half-edge");
	}

	/// A tag round-trips to the action it stands for, and a stray one resolves to
	/// nothing — including the zero tag, which is what kind 0 is reserved for.
	#[test]
	fn every_tag_resolves_to_its_own_action() {
		assert_eq!(action_of(tag(KIND_TEAM, 3)), Some(Action::Team(3)));
		assert_eq!(action_of(tag(KIND_TEAM, TEAM_NAMES.len() as u64)), None, "past the roster");
		assert_eq!(action_of(tag(KIND_PICK, (1 << 8) | 5)), Some(Action::SelectPick(SelectKind::Orders, 5)));
		assert_eq!(action_of(tag(KIND_PICK, (9 << 8) | 5)), None, "no such dropdown");
		assert_eq!(action_of(tag(KIND_CONNECTOR, 0x08)), Some(Action::ConnectorToggle(0x08)));
		assert_eq!(action_of(tag(KIND_CONNECTOR, 0x03)), None, "0x03 is not a half-edge bit");
		assert_eq!(action_of(tag(KIND_ADVANCED, 0)), Some(Action::ToggleAdvanced));
		assert_eq!(action_of(0), None, "kind 0 is unused, so a stray zero means nothing");
	}

	/// The team swatches are the panel's one always-present command bank: a click
	/// on one fires its team, and the owner's key reads selected. Nothing selected
	/// ⇒ the whole form is collapsed, so the same point fires nothing.
	#[test]
	fn a_click_on_a_team_swatch_fires_that_team() {
		let body = Rect::new(0.0, 0.0, 280.0, 640.0);
		let (_chrome, mut ui, id) = hosted(snapshot(sel(2, 0), false), body);
		let at = ui.rect_of(panel(&ui, id).ids.swatches[3]).expect("the swatch was arranged").center();
		assert_eq!(click(&mut ui, at), vec![Action::Team(3)], "one swatch, one action");
		let owner = ui.get::<ColorButton>(panel(&ui, id).ids.swatches[2]).expect("a swatch");
		assert!(owner.selected(), "the owning team's key reads selected");

		// The empty state: the form is a collapsed `Reveal`, so nothing is there.
		let (_chrome, mut ui, _id) = hosted(Snapshot::default(), body);
		assert!(click(&mut ui, at).is_empty(), "no selection -> the form is not in the tree");
	}

	/// Every connector cell maps to its half-edge bit for a 2×2 footprint (the
	/// acceptance case), the corners and the centre are inert, and a non-host
	/// never shows the grid at all.
	#[test]
	fn the_connector_grid_maps_every_cell_to_its_bit() {
		let body = Rect::new(0.0, 0.0, 280.0, 900.0);
		let mut host = sel(2, 0);
		host.connector_host = true;
		host.footprint = 2;
		let (_chrome, mut ui, id) = hosted(snapshot(host, false), body);
		let origin = descendant::<ConnectorGrid>(&panel(&ui, id).root, panel(&ui, id).ids.grid)
			.expect("the grid is in the tree")
			.rect
			.min();
		for row in 0..4 {
			for col in 0..4 {
				let at = connector_cell(origin, row, col).center();
				let want: Vec<Action> =
					connector_bit_at(2, row, col).map(Action::ConnectorToggle).into_iter().collect();
				assert_eq!(click(&mut ui, at), want, "cell ({row},{col})");
			}
		}
		// A non-host collapses the whole section: the same points fire nothing.
		let (_chrome, mut ui, _id) = hosted(snapshot(sel(2, 0), false), body);
		let at = connector_cell(origin, 0, 1).center();
		assert!(click(&mut ui, at).is_empty(), "a non-host has no connector grid");
	}

	/// The stats rows show and hide per [`StatsVis`] — a dynamic stat always, an
	/// advanced one only in advanced mode, an inapplicable one never — and the
	/// advanced checkbox fires the toggle.
	#[test]
	fn the_stats_rows_show_and_hide_per_visibility() {
		let body = Rect::new(0.0, 0.0, 280.0, 900.0);
		let mut valued = sel(2, 0);
		valued.values = Some(sample_values());
		let adv = VALUE_STATS.iter().position(|s| s.advanced).expect("an advanced stat");

		let shown = |ui: &Ui, id: WidgetId| -> Vec<bool> {
			let p = panel(ui, id);
			p.ids
				.stat_rows
				.iter()
				.map(|&r| descendant::<Reveal>(&p.root, r).expect("the slot is in the tree").is_shown())
				.collect()
		};

		let (_chrome, ui, id) = hosted(snapshot(valued.clone(), false), body);
		assert!(shown(&ui, id)[0], "a dynamic stat shows");
		assert!(!shown(&ui, id)[adv], "an advanced one does not, collapsed");
		assert_eq!(panel(&ui, id).visible.len(), FREE_FIELDS.len() + 9, "five free boxes + nine dynamic stats");

		let (_chrome, mut ui, id) = hosted(snapshot(valued.clone(), true), body);
		assert!(shown(&ui, id)[adv], "...and does in advanced mode");
		// The advanced checkbox fires the toggle from its own cell.
		let at = ui.rect_of(panel(&ui, id).ids.advanced).expect("the checkbox was arranged").center();
		assert_eq!(click(&mut ui, at), vec![Action::ToggleAdvanced]);

		// An inapplicable stat drops its row even in advanced mode.
		let mut masked = valued;
		masked.applicable[0] = false;
		let (_chrome, ui, id) = hosted(snapshot(masked, true), body);
		assert!(!shown(&ui, id)[0], "an inapplicable stat is not shown");

		// No stats block at all: the section header is gone with the rows.
		let (_chrome, ui, id) = hosted(snapshot(sel(2, 0), true), body);
		let p = panel(&ui, id);
		assert!(
			!descendant::<Reveal>(&p.root, p.ids.values_head).expect("the slot").is_shown(),
			"no stats block -> no values section",
		);
	}

	/// Ground cover swaps the two enumerable rows for read-only readouts: its
	/// `angle` is a decorative variant and it has no orders, so neither is a
	/// dropdown, and clicking where the box was opens nothing.
	#[test]
	fn ground_cover_shows_its_enumerable_rows_read_only() {
		let body = Rect::new(0.0, 0.0, 280.0, 640.0);
		let (_chrome, ui, id) = hosted(snapshot(sel(2, 0), false), body);
		let facing_box = panel(&ui, id).select_rect_for_test(SelectKind::Facing);
		assert!(!facing_box.is_empty(), "a real unit gets a live dropdown");

		let (_chrome, mut ui, id) = hosted(snapshot(sel_ground_cover(2), false), body);
		let p = panel(&ui, id);
		assert!(
			!descendant::<Reveal>(&p.root, p.ids.dropdown_slot[0]).expect("the slot").is_shown(),
			"ground cover has no facing dropdown",
		);
		assert_eq!(
			descendant::<Label>(&p.root, p.ids.readonly[0]).expect("the readout").text(),
			"variant 2",
			"...it reads out the variant instead",
		);
		ui.dispatch(&[press(true, facing_box.center())]);
		assert!(!ui.popup_open(), "and pressing where the box was opens nothing");
	}

	/// The Turret row is in the tree only for a unit with an independent turret,
	/// and it pushes everything below it down by exactly one row.
	#[test]
	fn the_turret_row_appears_only_for_a_turret_unit() {
		let body = Rect::new(0.0, 0.0, 280.0, 640.0);
		let orders_y = |has_turret: bool| -> f32 {
			let mut s = sel(2, 0);
			s.has_turret = has_turret;
			let (_chrome, ui, id) = hosted(snapshot(s, false), body);
			let p = panel(&ui, id);
			assert_eq!(
				descendant::<Reveal>(&p.root, p.ids.turret_row).expect("the slot").is_shown(),
				has_turret,
				"the turret row follows the unit",
			);
			// The last free field (Disabled) is the row above the turret row.
			p.field_rects_for_test()[FREE_FIELDS.len() - 1].y
				+ descendant::<Reveal>(&p.root, p.ids.turret_row).expect("the slot").rect().h
		};
		assert_eq!(orders_y(true) - orders_y(false), ROW_H, "a turret adds exactly one row");
	}

	/// The section stack grows as each optional block is revealed — the content
	/// height the `ScrollArea` sizes its bar from.
	#[test]
	fn the_form_grows_with_each_revealed_section() {
		// A dock tall enough that nothing scrolls, so the form's own bottom is
		// the measure.
		let body = Rect::new(0.0, 0.0, 280.0, 2000.0);
		let bottom = |s: Sel, advanced: bool| -> f32 {
			let (_chrome, ui, id) = hosted(snapshot(s, advanced), body);
			let p = panel(&ui, id);
			descendant::<Reveal>(&p.root, p.ids.form).expect("the form slot").rect().bottom()
		};
		let base = bottom(sel(2, 0), false);
		let mut valued = sel(2, 0);
		valued.values = Some(sample_values());
		let with_values = bottom(valued.clone(), false);
		assert!(with_values > base, "the values section adds height");
		assert!(bottom(valued, true) > with_values, "advanced mode adds the static-stat rows on top");
		let mut host = sel(2, 0);
		host.connector_host = true;
		host.footprint = 2;
		assert!(bottom(host, false) > base, "a connector host adds the footprint grid");
	}

	/// The scroll range is zero when nothing is selected or the content fits, and
	/// positive once a valued object outgrows a short dock — the `ScrollArea`'s
	/// own, fed by the column it measures.
	#[test]
	fn scroll_range_zero_when_fits_positive_when_overflowing() {
		let mut valued = sel(2, 0);
		valued.values = Some(sample_values());
		let overflows = |snap: Snapshot, body: Rect| -> bool {
			let (_chrome, mut ui, id) = hosted(snap, body);
			// End pages to the bottom; a form that fits has nowhere to go.
			ui.dispatch(&[Event::PointerMoved { pos: body.center() }]);
			ui.dispatch(&[Event::Key { key: wgpu_ui::Key::End, pressed: true, repeat: false, mods: Modifiers::NONE }]);
			panel(&ui, id).scroll() > 0.0
		};
		assert!(!overflows(Snapshot::default(), Rect::new(0.0, 0.0, 280.0, 400.0)), "no selection never scrolls");
		assert!(!overflows(snapshot(valued.clone(), false), Rect::new(0.0, 0.0, 280.0, 1000.0)), "a tall dock fits");
		assert!(overflows(snapshot(valued, false), Rect::new(0.0, 0.0, 280.0, 120.0)), "a short dock must scroll");
	}

	/// The two native sprite wells are real rects in the tree — the header
	/// preview always, the connector footprint only for a host — so the shell can
	/// read them back instead of recomputing the geometry.
	#[test]
	fn the_native_wells_are_read_back_off_the_tree() {
		let body = Rect::new(0.0, 0.0, 280.0, 900.0);
		let mut host = sel(2, 0);
		host.connector_host = true;
		host.footprint = 2;
		let (_chrome, ui, id) = hosted(snapshot(host, false), body);
		let p = panel(&ui, id);
		let preview = p.preview_rect().expect("the header preview well");
		assert_eq!((preview.w, preview.h), (PREVIEW, PREVIEW), "the preview well is square");
		assert!(body.contains(preview.center()), "and inside the panel body");
		let fp = p.connector_rect().expect("the connector footprint well");
		assert_eq!((fp.w, fp.h), (2.0 * CGRID_CELL, 2.0 * CGRID_CELL), "a 2x2 footprint spans four cells");

		// A non-host has no connector well at all.
		let (_chrome, ui, id) = hosted(snapshot(sel(2, 0), false), body);
		assert!(panel(&ui, id).connector_rect().is_none(), "a non-host reserves no footprint rect");
	}

	/// The value-stat table reads the right `UnitValues` fields and the
	/// dynamic/advanced split is what the section counts on.
	#[test]
	fn value_stats_table_reads_fields_and_counts_rows() {
		let v = sample_values();
		let by = |attr: &str| VALUE_STATS.iter().find(|s| s.attr == attr).map(|s| s.kind.get(&v));
		assert_eq!(by("hits"), Some(40), "Max HP reads UnitValues::hits");
		assert_eq!(by("attack"), Some(16));
		assert_eq!(by("ammo"), Some(8));
		assert_eq!(by("turns"), Some(3), "an advanced stat still reads its field");
		// Nine dynamic (always-shown) stats, five advanced.
		let dynamic = VALUE_STATS.iter().filter(|s| !s.advanced).count();
		let advanced = VALUE_STATS.iter().filter(|s| s.advanced).count();
		assert_eq!((dynamic, advanced), (9, 5), "dynamic vs advanced split");
		assert_eq!(StatsVis::all(false, false).count(), 0, "no values -> no rows");
		assert_eq!(StatsVis::all(true, false).count(), dynamic, "the dynamic rows");
		assert_eq!(StatsVis::all(true, true).count(), dynamic + advanced, "all of them");
	}

	/// `FieldKey` maps each editable box to its value, visibility and commit.
	#[test]
	fn field_keys_map_values_and_commits() {
		let mut s = sel(2, 0);
		s.name = "Grid Alpha".into();
		s.values = Some(sample_values());
		let adv_idx = VALUE_STATS.iter().position(|st| st.advanced).unwrap();
		// Free fields always visible; a stat only with a values block + when shown.
		assert!(FieldKey::Free(Field::Name).visible(StatsVis::all(true, false)));
		assert!(FieldKey::Stat(0).visible(StatsVis::all(true, false)), "a dynamic stat shows");
		assert!(!FieldKey::Stat(adv_idx).visible(StatsVis::all(true, false)), "advanced stat hidden when collapsed");
		assert!(FieldKey::Stat(adv_idx).visible(StatsVis::all(true, true)), "...shown in advanced mode");
		assert!(!FieldKey::Stat(0).visible(StatsVis::all(false, false)), "no stats block -> no stat boxes");
		let mut masked = StatsVis::all(true, true);
		masked.mask[0] = false;
		assert!(!FieldKey::Stat(0).visible(masked), "an inapplicable stat hides its box");
		// Values read off the selection.
		assert_eq!(FieldKey::Free(Field::Name).value(&s), "Grid Alpha");
		assert_eq!(FieldKey::Free(Field::Hits).value(&s), "20");
		assert_eq!(FieldKey::Stat(0).value(&s), VALUE_STATS[0].kind.get(&sample_values()).to_string());
		// Commits carry the field identity + the typed text.
		assert_eq!(FieldKey::Free(Field::Hits).commit("99".into()), Commit::Field(Field::Hits, "99".into()));
		assert_eq!(FieldKey::Stat(0).commit("7".into()), Commit::Value(VALUE_STATS[0].attr, "7".into()));
	}
}
