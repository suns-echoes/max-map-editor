//! Tile Editing Toolbox dockable (design: features.drawio
//! "Dockables"): command-bound button groups, no logic of its own - every
//! button runs a command line (the menu's pattern). The left edge previews the
//! active tile **with its transform** (the flip/rotate feedback).
//!
//! **The panel is a real `wgpu-ui` widget tree** (U5.4, the ticket that joins
//! U5.2's and U5.3's halves): a [`wgpu_ui::ScrollArea`] over a
//! [`wgpu_ui::Wrap`] of the [`PreviewView`] **content widget** and the six
//! group blocks — [`Kind::Buttons`] as a [`wgpu_ui::Label`] over rows of
//! [`wgpu_ui::Button`]s / captioned [`wgpu_ui::ColorButton`]s, [`Kind::Select`]
//! as the hosted [`wgpu_ui::Select`] U3.3 gave it. There is no hit oracle, no
//! panel-wide `ArmFire` and no `Hot`: hover, arming and fire are each key's own,
//! and everything the panel produces comes back as an **action tag** polled off
//! `Ui::actions` — [`hit_of`] maps it through [`GROUPS`], so no command line is
//! ever re-typed (memory `menu-kb-action-registry`).

use wgpu_ui::widget::{DrawCtx, EventCtx, LayoutCtx, Widget};
use wgpu_ui::{
	ArmFire, ColorButton, CrossAlign, DrawList, Emboss, Event, Icon, Insets, Label, Length, Linear, ScrollArea, Select,
	Size, TextRole, Vec2, WidgetId, WidgetState, Wrap, descendant, descendant_mut, icon,
};

use crate::state::{EditorState, Tool};
use crate::theme;
use crate::ui::Rect;
use crate::uikit_theme::rgba;

const PAD: f32 = 6.0;
/// The 8-orientation preview grid: `PREVIEW_COLS`×`PREVIEW_ROWS` cells of
/// `PREVIEW_CELL` px, and `PREVIEW_W` reserved before the first tool group.
const PREVIEW_COLS: usize = 4;
const PREVIEW_ROWS: usize = 2;
const PREVIEW_CELL: f32 = 28.0;
const PREVIEW_GAP: f32 = 2.0;
/// Height under the grid for the spec / stamp-size readout.
const PREVIEW_SPEC_H: f32 = 14.0;
const PREVIEW_W: f32 = 122.0;
/// The square icon-key side — the graphics-app tool cell all three toolboxes
/// share (a stencil face + tooltip instead of a label-wide bar).
pub(crate) const KEY: f32 = 24.0;
/// A hosted dropdown's row height (the one control still sized by its words).
const SELECT_H: f32 = 18.0;
const GAP: f32 = 2.0; // between keys within a group
const GROUP_GAP: f32 = 10.0; // between group blocks on a row
const ROW_GAP: f32 = 8.0; // between wrapped rows
const GROUP_LABEL_H: f32 = 14.0;

pub struct Button {
	pub label: &'static str,
	/// The command line the key runs (validated against the parser by a test).
	pub cmd: &'static str,
	/// Optional swatch fill (pass-type buttons use the pass colors).
	pub fill: Option<[f32; 4]>,
	/// Optional stencil face — the key builds as a square icon key with the
	/// label as its tooltip. `None` keeps a text face (numeric presets).
	pub icon: Option<Icon>,
}

/// How a group renders: a block of buttons, or a single dropdown whose
/// `buttons` are its options (a hosted [`wgpu_ui::Select`], which owns its own
/// open state — see [`ToolboxContent::selects`]).
#[derive(PartialEq, Eq)]
pub enum Kind {
	Buttons,
	Select,
}

pub struct Group {
	pub label: &'static str,
	pub cols: usize,
	pub kind: Kind,
	pub buttons: &'static [Button],
	/// Explicit key-row lengths, in order, for a block whose rows are *ragged on
	/// purpose* (the Save Toolbox's resource group: paint + erase over the three
	/// material keys). Empty (the norm) chunks `buttons` uniformly by [`cols`]
	/// (Self::cols); when set, the lengths must sum to `buttons.len()`
	/// ([`key_rows`](Self::key_rows) asserts it, and a test walks every table).
	pub rows: &'static [usize],
}

impl Group {
	/// The block's key rows: `(flat index of the row's first button, the row)`.
	/// One definition for all three toolboxes, so a button's action tag — its
	/// flat index in [`Group::buttons`] — never depends on how the rows split.
	pub(crate) fn key_rows(&self) -> Vec<(usize, &'static [Button])> {
		if self.rows.is_empty() {
			return self
				.buttons
				.chunks(self.cols)
				.scan(0, |base, row| {
					let out = (*base, row);
					*base += row.len();
					Some(out)
				})
				.collect();
		}
		let mut out = Vec::with_capacity(self.rows.len());
		let mut base = 0;
		for &len in self.rows {
			out.push((base, &self.buttons[base..base + len]));
			base += len;
		}
		assert_eq!(base, self.buttons.len(), "{}: row lengths must cover the group exactly", self.label);
		out
	}
}

/// A plain command button.
pub(crate) const fn b(label: &'static str, cmd: &'static str) -> Button {
	Button { label, cmd, fill: None, icon: None }
}

/// A square icon key: a stencil face, the label as its tooltip.
pub(crate) const fn ik(label: &'static str, cmd: &'static str, ic: Icon) -> Button {
	Button { label, cmd, fill: None, icon: Some(ic) }
}

pub const GROUPS: &[Group] = &[
	Group {
		label: "draw",
		cols: 4,
		kind: Kind::Buttons,
		rows: &[],
		buttons: &[
			ik("pencil", "tool pencil", icon::PENCIL),
			ik("pick tile", "tool picker", icon::DROPPER),
			ik("erase", "tool eraser", icon::ERASER),
			ik("flood fill", "tool fill", icon::BUCKET),
			ik("paint land", "tool paint-land", icon::TERRAIN),
			ik("paint water", "tool paint-water", icon::WAVES),
			ik("randomize", "randomize toggle", icon::DICE),
		],
	},
	// Brush size is a dropdown (the options are wider than the old 1/3/5/7
	// key row allowed); each option runs its `brush-size N`.
	Group {
		label: "brush",
		cols: 1,
		kind: Kind::Select,
		rows: &[],
		buttons: &[
			b("1 cells", "brush-size 1"),
			b("2 cells", "brush-size 2"),
			b("3 cells", "brush-size 3"),
			b("5 cells", "brush-size 5"),
			b("7 cells", "brush-size 7"),
			b("9 cells", "brush-size 9"),
			b("13 cells", "brush-size 13"),
		],
	},
	Group {
		label: "shape",
		cols: 2,
		kind: Kind::Buttons,
		rows: &[],
		buttons: &[
			ik("square brush", "brush-shape square", icon::SQUARE),
			ik("round brush", "brush-shape circle", icon::CIRCLE),
		],
	},
	// The terrain brush's coast-on-release behaviour (Tool::PaintMask).
	Group {
		label: "auto shore",
		cols: 1,
		kind: Kind::Select,
		rows: &[],
		buttons: &[
			b("disabled", "auto-shore off"),
			b("sweep", "auto-shore sweep"),
			b("loop-walk", "auto-shore loop-walk"),
		],
	},
	// What the pencil, the eraser and the arrow act on. The first two are the
	// tile stack's layers; Scenery is the free-placed cut-out list, which is not
	// a tile layer at all - picking it re-points those three tools at it.
	Group {
		label: "layer",
		cols: 3,
		kind: Kind::Buttons,
		rows: &[],
		buttons: &[
			ik("water layer", "layer water", icon::WAVES),
			ik("ground layer", "layer ground", icon::SOIL),
			ik("scenery layer", "layer scenery", icon::TREE),
		],
	},
	// The four pass-type swatches used to sit here; they are the Pass Types
	// Palette's now ([`crate::passtools`]), beside the cell tally that says what
	// painting them did — this panel is the *terrain* toolbox.
	Group {
		label: "selection",
		cols: 4,
		kind: Kind::Buttons,
		rows: &[],
		buttons: &[
			ik("select", "tool select", icon::CURSOR),
			ik("select rect", "tool select-rect", icon::MARQUEE),
			ik("clear selection", "select clear", icon::MARQUEE_OFF),
			ik("select similar", "select similar", icon::WAND),
		],
	},
];

/// The 8-orientation grid's transform for cell index `i` (0..8): row 0 (0-3) =
/// no mirror, row 1 (4-7) = mirror; the column is the clockwise quarter-turn
/// count. The grid lays cells row-major, so cell `i` shows `orient_transform(i)`.
pub fn orient_transform(i: usize) -> map_core::Transform {
	map_core::Transform { rot: (i % 4) as u8, mirror: i >= 4 }
}

/// The grid cell index for orientation `t` - the inverse of [`orient_transform`].
pub fn orient_index(t: map_core::Transform) -> usize {
	(t.mirror as usize) * 4 + t.rot as usize
}

/// The 8 orientation-cell rects laid row-major from the grid's top-left `at`
/// (matches [`orient_transform`]). Pure, and the **one** definition of the
/// preview's cell geometry: [`PreviewView`] draws, hit-tests and reports its
/// cells through this, and the shell renders the armed tile/stamp into exactly
/// the rects it hands back ([`ToolboxContent::preview_cells`]).
fn preview_cells(at: Vec2) -> [Rect; 8] {
	std::array::from_fn(|i| {
		let (col, row) = (i % PREVIEW_COLS, i / PREVIEW_COLS);
		Rect::new(
			at.x + col as f32 * (PREVIEW_CELL + PREVIEW_GAP),
			at.y + row as f32 * (PREVIEW_CELL + PREVIEW_GAP),
			PREVIEW_CELL,
			PREVIEW_CELL,
		)
	})
}

/// The action tag a key carries: its `(group, button)` coordinates in
/// [`GROUPS`], packed — for a [`Kind::Buttons`] key *and* for a
/// [`Kind::Select`] option, which are the same table row. An orientation cell
/// carries [`ORIENT_TAG`] plus its index instead, so one `Ui::actions` poll
/// answers for the whole panel.
const fn tag(group: usize, button: usize) -> u64 {
	((group as u64) << 32) | button as u64
}

/// The tag space the 8 orientation cells live in (above every [`tag`]).
const ORIENT_TAG: u64 = 1 << 63;

/// What a fired toolbox action tag stands for.
#[derive(Clone, Copy)]
pub enum Hit {
	/// A group key or a picked dropdown option - its row in [`GROUPS`].
	Key(&'static Button),
	/// An orientation-grid cell (0-7) - re-orient the armed tile/stamp.
	Orient(usize),
}

/// The toolbox action a fired tag stands for, or `None` if it is not one of
/// this panel's (the shell polls every tag its `Ui` collected). Resolving a
/// click means *looking it up in the same table the key was built from*, so a
/// button that moves in [`GROUPS`] moves its tag with it.
pub fn hit_of(tag: u64) -> Option<Hit> {
	if tag & ORIENT_TAG != 0 {
		let i = (tag & !ORIENT_TAG) as usize;
		return (i < 8).then_some(Hit::Orient(i));
	}
	let group = GROUPS.get((tag >> 32) as usize)?;
	group.buttons.get((tag & 0xffff_ffff) as usize).map(Hit::Key)
}

/// The [`GROUPS`] row a key tag stands for (`None` for an orientation tag).
fn button_of(tag: u64) -> Option<&'static Button> {
	match hit_of(tag) {
		Some(Hit::Key(button)) => Some(button),
		_ => None,
	}
}

/// Every [`Kind::Select`] group's index, in flow order — one hosted
/// [`wgpu_ui::Select`] each (there are two: brush size and auto shore).
fn select_groups() -> impl Iterator<Item = usize> {
	GROUPS.iter().enumerate().filter(|(_, g)| g.kind == Kind::Select).map(|(i, _)| i)
}

/// The toolbox-relevant editor state, snapshotted into [`ToolboxContent`] each
/// frame: which keys light, which dropdown option each box shows, and what the
/// preview draws. The native preview quads are computed separately
/// (`main::toolbox_preview_quads`) because they need the project.
#[derive(Clone)]
pub struct Snapshot {
	tool: Tool,
	mask_water: bool,
	randomize: bool,
	brush_size: u16,
	brush_shape: crate::state::BrushShape,
	brush_shore: crate::state::BrushShore,
	layer: &'static str,
	tile: Option<String>,
	/// The 8 orientation cells' enabled state (the `false`s draw greyed), the
	/// current orientation's cell index (the selection ring), and the armed
	/// stamp's footprint (the readout shows `WxH` for a stamp, else the spec).
	orient_enabled: [bool; 8],
	orient_current: Option<usize>,
	stamp_dims: Option<(u16, u16)>,
}

impl Snapshot {
	/// Snapshot the toolbox-relevant editor state for one frame's draw.
	pub fn of(editor: &EditorState) -> Self {
		// The armed thing drives the orientation grid: a stamp (its cached 8
		// orientations) or the single active tile (its family's permissions).
		let (orient_enabled, orient_current, stamp_dims) = if editor.stamp_base.is_some() {
			let enabled = std::array::from_fn(|i| editor.stamp_orients[i].is_some());
			let dims = editor.stamp.as_ref().map(|s| (s.width, s.height));
			(enabled, Some(orient_index(editor.stamp_xform)), dims)
		} else if let Some(spec) = editor.active_tile() {
			let tref = editor.project.resolve_ref(spec).ok();
			let enabled = std::array::from_fn(|i| {
				tref.is_some_and(|(t, _)| editor.project.tile_allows(t.pack, t.tile, orient_transform(i)))
			});
			(enabled, tref.map(|(t, _)| orient_index(t.transform)), None)
		} else {
			([false; 8], None, None)
		};
		Self {
			tool: editor.tool,
			mask_water: editor.mask_water,
			randomize: editor.randomize,
			brush_size: editor.brush_size,
			brush_shape: editor.brush_shape,
			brush_shore: editor.brush_shore,
			layer: editor.active_layer_name(),
			tile: editor.active_tile().map(str::to_string),
			orient_enabled,
			orient_current,
			stamp_dims,
		}
	}

	fn empty() -> Self {
		Self {
			tool: Tool::Pencil,
			mask_water: false,
			randomize: false,
			brush_size: 1,
			brush_shape: crate::state::BrushShape::Square,
			brush_shore: crate::state::BrushShore::Off,
			layer: "ground",
			tile: None,
			orient_enabled: [false; 8],
			orient_current: None,
			stamp_dims: None,
		}
	}

	/// The preview block's heading: what the 8 cells are orientations *of*.
	fn preview_heading(&self) -> &'static str {
		if self.stamp_dims.is_some() { "stamp" } else { "tile" }
	}

	/// The readout under the grid: the stamp footprint, the tile spec, or "none".
	fn preview_readout(&self) -> (String, [f32; 4]) {
		match (self.stamp_dims, self.tile.as_deref()) {
			(Some((w, h)), _) => (format!("stamp {w}x{h}"), theme::INK),
			(None, Some(spec)) => (spec.to_string(), theme::INK),
			(None, None) => ("none".to_string(), theme::INK_DIM),
		}
	}
}

/// The toolbox's **content widget**: the 4×2 orientation grid the native
/// `draw_picker` pass renders the armed tile/stamp into, plus the spec readout
/// under it. It reserves the cells, draws their wells / grey veils / selection
/// ring and the readout, and owns the cell pick.
///
/// It is a content widget, so it holds no chrome (§5.2): the group labels and
/// every key are its siblings in the flow, never its children. The readout
/// stays inside it precisely *because* it is not chrome — a stock `Label` would
/// measure to the tile spec's width, and the whole flow would then reflow every
/// time a different tile is armed.
pub struct PreviewView {
	id: WidgetId,
	snap: Snapshot,
	rect: Rect,
	/// Arm-on-press / fire-on-release-inside over the cells - the domain hit
	/// test a content widget keeps (the panel's chrome oracle is gone).
	clicks: ArmFire<usize>,
}

impl PreviewView {
	fn new() -> Self {
		Self { id: wgpu_ui::next_id(), snap: Snapshot::empty(), rect: Rect::ZERO, clicks: ArmFire::new() }
	}

	/// The 8 orientation-cell rects, in the panel's current layout — what the
	/// shell renders the armed tile/stamp into. Row-major, matching
	/// [`orient_transform`].
	fn cells(&self) -> [Rect; 8] {
		preview_cells(Vec2::new(self.rect.x, self.rect.y))
	}

	/// The cell under `p`, if any - the domain hit oracle.
	fn cell_at(&self, p: Vec2) -> Option<usize> {
		self.cells().iter().position(|c| c.contains(p))
	}
}

impl Widget for PreviewView {
	fn measure(&mut self, _avail: Size, _ctx: &mut LayoutCtx) -> Size {
		// The reserved block: the cell grid, plus the readout row under it. Wider
		// than the cells by design (`PREVIEW_W`), so the readout has room and the
		// first tool group starts where it always has.
		let grid_h = PREVIEW_ROWS as f32 * PREVIEW_CELL + (PREVIEW_ROWS - 1) as f32 * PREVIEW_GAP;
		Size::new(PREVIEW_W, grid_h + PREVIEW_SPEC_H)
	}

	fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
		self.rect = rect;
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		// A field well per cell, a dim veil over the orientations the tiles
		// forbid, and a ring on the current one. The armed tile/stamp pixels are
		// drawn over this by the shell's native pass.
		let cells = self.cells();
		for (i, cell) in cells.iter().enumerate() {
			ctx.theme.well(dl, *cell, WidgetState::default());
			if !self.snap.orient_enabled[i] {
				// Disallowed orientation: a dim veil (the shell draws no tile here).
				dl.fill_rect(*cell, rgba(theme::VEIL));
			}
			if self.snap.orient_current == Some(i) {
				dl.stroke_rect(
					Rect::new(cell.x - 1.0, cell.y - 1.0, cell.w + 2.0, cell.h + 2.0),
					1.0,
					rgba(theme::ACCENT),
				);
			}
		}
		// The readout, clipped to this widget so a long tile spec is cut at the
		// block's edge instead of painting across the group beside it.
		let (text, ink) = self.snap.preview_readout();
		let below = cells[cells.len() - 1].bottom();
		dl.push_clip(self.rect);
		ctx.theme.text_top(
			dl,
			ctx.fonts,
			Vec2::new(self.rect.x, below + 2.0),
			&text,
			TextRole::Small,
			Emboss::Engraved,
			rgba(ink),
		);
		dl.pop_clip();
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		let cells = self.cells();
		let handled = self.clicks.event(ev, ctx, self.id, |p| cells.iter().position(|c| c.contains(p)));
		// The fire goes out as an action tag, so the shell polls one place for
		// the whole panel (the keys' `Ui::actions`) instead of a second channel.
		if let Some(i) = self.clicks.take_outcome() {
			ctx.fire(self.id, Some(ORIENT_TAG | i as u64));
		}
		handled
	}

	fn rect(&self) -> Rect {
		self.rect
	}

	fn id(&self) -> WidgetId {
		self.id
	}

	/// Only the 8 cells claim the pointer: the readout row and the slack beside
	/// the grid stay inert, exactly as the old `click` oracle had them.
	fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
		self.cell_at(pos).map(|_| self.id)
	}
}

/// One key in the built tree: which widget it is, and which [`GROUPS`] row it
/// stands for. The tag is the *only* link back to the table — the same one the
/// fired `Ui` hands the shell — so a key can never light off one row and run
/// another's command.
struct Key {
	id: WidgetId,
	tag: u64,
}

/// Build the panel's tree once: a `ScrollArea` over a flow of the preview block
/// and the eight group blocks.
///
/// The flow is a [`Wrap`], not a plain column, because that is what this panel
/// has always done — a wide bottom dock keeps the blocks on one row and a narrow
/// one stacks them. A block is a column of [`Length::Fixed`] rows rather than a
/// [`wgpu_ui::Grid`]: a `Grid` measures to the width it is *given*, which inside
/// a flow is the whole run, so every block would claim the full row (U5.2).
fn build() -> (ScrollArea, Vec<Key>, WidgetId, WidgetId, Vec<WidgetId>) {
	let mut keys = Vec::new();
	let mut selects = Vec::new();
	let mut flow = Wrap::row().padding(Insets::all(PAD)).spacing(GROUP_GAP).run_spacing(ROW_GAP);

	// The preview block leads the flow, like every other block: a heading label
	// over the content widget. Both are id'd - the heading's text and the cells'
	// state are per-frame `Snapshot` state pushed in through `sync`.
	let heading = Label::new("tile").small().muted().with_id();
	let heading_id = heading.id();
	let view = PreviewView::new();
	let view_id = view.id();
	flow = flow.push(
		Linear::column()
			.child(heading, Length::Fixed(GROUP_LABEL_H))
			.child(view, Length::Fixed(PREVIEW_ROWS as f32 * PREVIEW_CELL + PREVIEW_GAP + PREVIEW_SPEC_H)),
	);

	for (g, group) in GROUPS.iter().enumerate() {
		let mut block = Linear::column().spacing(GAP);
		block = block.child(Label::new(group.label).small().muted(), Length::Fixed(GROUP_LABEL_H));
		if group.kind == Kind::Select {
			// A dropdown group is one hosted `Select` (U3.3): its options are the
			// group's own buttons, and it owns open/close/dismiss/pick/keyboard.
			// It sizes itself to its widest option — the reason it stopped being a
			// key row in the first place.
			let sel = Select::new(group.buttons.iter().map(|b| b.label)).small();
			selects.push(sel.id());
			block = block.child(sel, Length::Fixed(SELECT_H));
			flow = flow.push(block);
			continue;
		}
		for (base, chunk) in group.key_rows() {
			// `Stretch` sizes each key to the row's fixed height; `Fixed(KEY)`
			// keeps the square cells aligned in tight grid columns.
			let mut row = Linear::row().spacing(GAP).cross_align(CrossAlign::Stretch);
			for (c, button) in chunk.iter().enumerate() {
				let tag = tag(g, base + c);
				// A square icon key: the stencil is the face, the name rides the
				// tooltip.
				let key = wgpu_ui::Button::new(button.label)
					.icon(button.icon.expect("every non-select toolbox key wears an icon"))
					.tooltip(tooltip_text(button))
					.action(tag);
				keys.push(Key { id: key.id(), tag });
				row = row.child(key, Length::Fixed(KEY));
			}
			block = block.child(row, Length::Fixed(KEY));
		}
		flow = flow.push(block);
	}
	(ScrollArea::new(flow), keys, heading_id, view_id, selects)
}

/// An icon key's hover caption: the command's name — the tooltip carries what
/// the label used to say.
pub(crate) fn tooltip_text(button: &Button) -> String {
	button.label.to_string()
}

/// The Tile Editing Toolbox as a retained `wgpu_ui` [`Widget`]: a thin root over
/// the built tree, which exists to hold the id tables (keys, the preview, the
/// two dropdowns) and to push the per-frame [`Snapshot`] into them. Everything
/// else — layout, paint, hover, arming, firing, scrolling, the dropdowns' popup
/// layer — is the tree's.
pub struct ToolboxContent {
	id: WidgetId,
	root: ScrollArea,
	keys: Vec<Key>,
	/// The preview block's heading label (`tile` / `stamp`).
	heading: WidgetId,
	/// The [`PreviewView`] content widget.
	view: WidgetId,
	/// One hosted dropdown per [`Kind::Select`] group, in [`select_groups`]
	/// order (U3.3). Open/close/dismiss/pick and the keyboard are the widget's;
	/// nothing about them lives on `EditorState`.
	selects: Vec<WidgetId>,
	rect: Rect,
}

impl Default for ToolboxContent {
	fn default() -> Self {
		Self::new()
	}
}

impl ToolboxContent {
	pub fn new() -> Self {
		let (root, keys, heading, view, selects) = build();
		Self { id: wgpu_ui::next_id(), root, keys, heading, view, selects, rect: Rect::ZERO }
	}

	/// Push one frame's editor state into the retained tree: which keys light,
	/// which option each dropdown shows, and what the preview draws.
	pub fn sync(&mut self, snap: Snapshot) {
		for key in &self.keys {
			let on = button_of(key.tag).is_some_and(|b| cmd_active(b.cmd, &snap));
			if let Some(button) = descendant_mut::<wgpu_ui::Button>(&mut self.root, key.id) {
				button.set_selected(on);
			} else if let Some(swatch) = descendant_mut::<ColorButton>(&mut self.root, key.id) {
				swatch.set_selected(on);
			}
		}
		// Point each dropdown at the option its state is currently on. A value no
		// option carries (`brush-size 4`, reachable from the console) is shown as
		// an extra option rather than silently reading as some other size.
		for (&id, g) in self.selects.iter().zip(select_groups()) {
			let Some(sel) = descendant_mut::<Select>(&mut self.root, id) else { continue };
			let group = &GROUPS[g];
			match select_active(group, &snap) {
				Some(i) => {
					sel.set_options(group.buttons.iter().map(|b| b.label));
					sel.set_selected(i);
				}
				None => {
					let odd = format!("{} cells", snap.brush_size);
					let labels: Vec<String> = group.buttons.iter().map(|b| b.label.to_string()).chain([odd]).collect();
					let last = labels.len() - 1;
					sel.set_options(labels);
					sel.set_selected(last);
				}
			}
		}
		if let Some(label) = descendant_mut::<Label>(&mut self.root, self.heading) {
			label.set_text(snap.preview_heading());
		}
		if let Some(view) = descendant_mut::<PreviewView>(&mut self.root, self.view) {
			view.snap = snap;
		}
	}

	/// The 8 orientation-preview cell rects, in the layout the panel last drew —
	/// the rects the shell renders the armed tile/stamp into. Read *after*
	/// `build`, which is what settles the scroll offset they hang off.
	///
	/// The widget hands the native pass its geometry rather than the pass
	/// recomputing it (the U5.3 invariant, which the minimap could not do —
	/// `render_frame` holds `passes.menu_chrome` mutably while the tree lays out,
	/// and the blit needs `&mut passes.minimap`). `draw_picker` goes through the
	/// separately-borrowed `ProjectRenderer`, so here there is one computation of
	/// the number and nothing to drift.
	pub fn preview_cells(&self) -> [Rect; 8] {
		descendant::<PreviewView>(&self.root, self.view).map_or([Rect::ZERO; 8], PreviewView::cells)
	}
}

impl Widget for ToolboxContent {
	crate::panel_ui::thin_root_plumbing!(arrange, draw);

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		let handled = self.root.event(ev, ctx);
		// The k-th hosted box stands for the k-th `Select` group; a pick past
		// the group's option count is dropped.
		crate::panel_ui::drain_selects(&mut self.root, &self.selects, ctx, |k, i| {
			let g = select_groups().nth(k)?;
			(i < GROUPS[g].buttons.len()).then(|| tag(g, i))
		});
		handled
	}
}

/// Whether running `cmd` would be a no-op because its effect *is* the current
/// state - what lights a toggle key, and what picks a select group's active
/// option. One matcher for both, so the two can't disagree (they did: the
/// select's used to test only `brush-size`, which is why the "auto shore" box
/// read "1 px" and never lit its own value).
///
/// Three keys are shared with the Scenery layer, which re-points them at the
/// cut-out list rather than adding keys of its own (`state::scenery_twin`), so
/// each lights for its own tool *or* its scenery twin. There is no ambiguity to
/// resolve: a scenery tool can only be armed while that layer is active.
fn cmd_active(cmd: &str, s: &Snapshot) -> bool {
	(cmd == "tool pencil" && matches!(s.tool, Tool::Pencil | Tool::Scenery))
		|| (cmd == "tool picker" && s.tool == Tool::Picker)
		|| (cmd == "tool eraser" && matches!(s.tool, Tool::Eraser | Tool::SceneryEraser))
		|| (cmd == "tool fill" && s.tool == Tool::Fill)
		|| (cmd == "tool paint-land" && s.tool == Tool::PaintMask && !s.mask_water)
		|| (cmd == "tool paint-water" && s.tool == Tool::PaintMask && s.mask_water)
		|| (cmd == "tool select" && matches!(s.tool, Tool::Select | Tool::SceneryMove))
		|| (cmd == "tool select-rect" && s.tool == Tool::SelectRect)
		|| (cmd == "randomize toggle" && s.randomize)
		|| (cmd.strip_prefix("brush-size ").is_some_and(|n| n.parse() == Ok(s.brush_size)))
		|| (cmd == "brush-shape square" && s.brush_shape == crate::state::BrushShape::Square)
		|| (cmd == "brush-shape circle" && s.brush_shape == crate::state::BrushShape::Circle)
		|| (cmd == "auto-shore off" && s.brush_shore == crate::state::BrushShore::Off)
		|| (cmd == "auto-shore sweep" && s.brush_shore == crate::state::BrushShore::Sweep)
		|| (cmd == "auto-shore loop-walk" && s.brush_shore == crate::state::BrushShore::LoopWalk)
		|| (cmd == "layer water" && s.layer == "water")
		|| (cmd == "layer ground" && s.layer == "ground")
		|| (cmd == "layer scenery" && s.layer == "scenery")
}

/// A select group's active option index - the one whose command line reflects
/// the current state.
fn select_active(group: &Group, editor: &Snapshot) -> Option<usize> {
	group.buttons.iter().position(|b| cmd_active(b.cmd, editor))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::uikit_menu::MenuChrome;
	use wgpu_ui::{DrawCmd, Modifiers, PointerButton, ScrollDelta, Ui, widget::DrawPass};

	/// Every button's command must parse (the menu's contract).
	#[test]
	fn every_run_button_parses() {
		for group in GROUPS {
			for button in group.buttons {
				crate::command::parse_line(button.cmd)
					.unwrap_or_else(|e| panic!("{}/{}: {e}", group.label, button.label))
					.unwrap_or_else(|| panic!("{}/{}: empty", group.label, button.label));
			}
		}
	}

	/// What a table row *says* — its label and the command line it carries. Two
	/// uses of a `const` slice need not share an address, so identity here is
	/// the content, not the pointer.
	fn row_of(button: &Button) -> (&'static str, &'static str) {
		(button.label, button.cmd)
	}

	/// Every table row's tag resolves back to *that* row — the mapping the shell
	/// runs a fired action through, so no command line is ever re-typed. Both
	/// group kinds share the tag space: a dropdown option is the same table row
	/// a key would be. The orientation cells live above them all.
	#[test]
	fn every_tag_resolves_to_its_own_table_row() {
		for (g, group) in GROUPS.iter().enumerate() {
			for (i, button) in group.buttons.iter().enumerate() {
				let found = button_of(tag(g, i))
					.unwrap_or_else(|| panic!("{}/{}: tag resolves to no key", group.label, button.label));
				assert_eq!(row_of(found), row_of(button), "{}/{}", group.label, button.label);
			}
		}
		assert!(button_of(tag(GROUPS.len(), 0)).is_none(), "a tag past the table resolves to nothing");
		assert!(button_of(tag(0, GROUPS[0].buttons.len())).is_none(), "and so does one past a group's keys");
		for i in 0..8 {
			assert!(matches!(hit_of(ORIENT_TAG | i as u64), Some(Hit::Orient(j)) if j == i));
		}
		assert!(hit_of(ORIENT_TAG | 8).is_none(), "there is no ninth orientation");
	}

	/// [`Group::key_rows`] covers every table exactly once, in order, whatever
	/// the row shape: a uniform group chunks by `cols`, a ragged one follows its
	/// declared `rows`, and either way the flat indices — the action tags — walk
	/// `buttons` start to end. All three toolboxes' tables are walked, so a
	/// declared `rows` that fails to sum to its group would panic right here.
	#[test]
	fn key_rows_cover_every_table_in_flat_order() {
		let tables = [GROUPS, crate::savetools::GROUPS, crate::passtools::GROUPS];
		for group in tables.iter().flat_map(|t| t.iter()) {
			let mut next = 0;
			for (base, row) in group.key_rows() {
				assert_eq!(base, next, "{}: rows are contiguous", group.label);
				assert!(!row.is_empty(), "{}: no empty rows", group.label);
				if group.rows.is_empty() {
					assert!(row.len() <= group.cols, "{}: a uniform row stays within cols", group.label);
				}
				next += row.len();
			}
			assert_eq!(next, group.buttons.len(), "{}: every button sits in exactly one row", group.label);
		}
		// The one ragged group so far: paint + erase over the material selectors.
		let resource = crate::savetools::GROUPS.iter().find(|g| g.label == "resource").expect("a resource group");
		let rows: Vec<usize> = resource.key_rows().iter().map(|(_, r)| r.len()).collect();
		assert_eq!(rows, [2, 3], "the resource block is two actions over three materials");
	}

	/// Every key in this panel is a plain command key: the swatch faces moved out
	/// with the pass-type group (they are [`crate::passtools`]'s now).
	#[test]
	fn every_toolbox_key_is_a_plain_command_key() {
		assert!(GROUPS.iter().flat_map(|g| g.buttons).all(|b| b.fill.is_none()), "no swatch faces here");
		assert!(b("probe", "tool pencil").fill.is_none(), "and `b` is what builds them");
	}

	/// The chrome fixture + the panel hosted in a `Ui`, laid out into `body`.
	/// A stock `Button` measures its own label, so this needs the real fonts
	/// (`Fonts::new()` + `Gunmetal` panics with "FontId(0) is not registered").
	fn hosted(body: Rect) -> (MenuChrome, Ui, WidgetId) {
		let (_device, _queue, chrome) = crate::visual_test::chrome_fixture();
		let content = ToolboxContent::new();
		let id = content.id();
		let mut ui = Ui::new(content);
		ui.get_mut::<ToolboxContent>(id).expect("typed root").sync(Snapshot::empty());
		ui.layout_in(body, chrome.theme(), chrome.fonts());
		(chrome, ui, id)
	}

	fn press(pressed: bool, at: Vec2) -> Event {
		Event::PointerButton { button: PointerButton::Primary, pressed, pos: at, mods: Modifiers::NONE }
	}

	/// The arranged centre of the key carrying `tag`.
	fn key_centre(ui: &Ui, want: u64) -> Vec2 {
		let content = ui.get::<ToolboxContent>(ui.root().id()).expect("the panel is the root");
		let key = content.keys.iter().find(|k| k.tag == want).expect("a built key carries this tag");
		ui.rect_of(key.id).expect("the key was arranged").center()
	}

	/// Every key in every [`Kind::Buttons`] group fires **its own** table row on
	/// a press + release-inside. That is the whole click path now: no hit oracle,
	/// no panel-wide `ArmFire`, and no command line written down twice.
	#[test]
	fn every_group_key_fires_its_own_row() {
		let body = Rect::new(0.0, 600.0, 1280.0, 220.0);
		let (_chrome, mut ui, _id) = hosted(body);

		for (g, group) in GROUPS.iter().enumerate().filter(|(_, g)| g.kind == Kind::Buttons) {
			for (i, button) in group.buttons.iter().enumerate() {
				let at = key_centre(&ui, tag(g, i));
				ui.dispatch(&[press(true, at)]);
				assert!(ui.actions().is_empty(), "{}: a press only arms", button.label);
				ui.dispatch(&[press(false, at)]);
				assert_eq!(ui.actions(), [tag(g, i)], "{}/{}: one key, one action", group.label, button.label);
			}
		}
	}

	/// A press that releases somewhere else fires nothing - the release-inside
	/// commit policy every command button shares.
	#[test]
	fn a_release_outside_the_key_fires_nothing() {
		let body = Rect::new(0.0, 600.0, 1280.0, 220.0);
		let (_chrome, mut ui, _id) = hosted(body);
		let at = key_centre(&ui, tag(0, 0));
		let elsewhere = key_centre(&ui, tag(0, 2));

		ui.dispatch(&[press(true, at), press(false, elsewhere)]);
		assert!(ui.actions().is_empty(), "the armed key never saw its release");
	}

	/// The active tool / brush shape / layer / pass keys read selected, and only
	/// they — the per-frame `Snapshot` pushed into the tree, read back off the
	/// widgets rather than off a shell-side mirror.
	#[test]
	fn the_active_states_keys_read_selected() {
		let body = Rect::new(0.0, 600.0, 1280.0, 220.0);
		let (_chrome, mut ui, id) = hosted(body);

		let lit = |ui: &Ui| -> Vec<&'static str> {
			let content = ui.get::<ToolboxContent>(id).expect("typed root");
			content
				.keys
				.iter()
				.filter(|k| match ui.get::<wgpu_ui::Button>(k.id) {
					Some(b) => b.selected(),
					None => ui.get::<ColorButton>(k.id).is_some_and(|s| s.selected()),
				})
				.filter_map(|k| button_of(k.tag))
				.map(|b| b.cmd)
				.collect()
		};

		let mut snap = Snapshot::empty();
		snap.tool = Tool::Eraser;
		ui.get_mut::<ToolboxContent>(id).expect("typed root").sync(snap);
		assert_eq!(
			lit(&ui),
			vec!["tool eraser", "brush-shape square", "layer ground"],
			"the armed tool, the brush shape and the active layer"
		);

		let mut snap = Snapshot::empty();
		snap.tool = Tool::PaintMask;
		snap.mask_water = true;
		ui.get_mut::<ToolboxContent>(id).expect("typed root").sync(snap);
		let on = lit(&ui);
		assert!(on.contains(&"tool paint-water"), "the water brush lights its own key, got {on:?}");
		assert!(!on.contains(&"tool eraser"), "and the key that was lit goes dark");
	}

	/// **The U5.4 content-widget invariant.** The cells the shell renders the
	/// armed tile/stamp into are the widget's own arranged cells — one
	/// computation, so the native `draw_picker` quads and the wells / veils /
	/// ring drawn under them cannot drift apart. Only those 8 cells are live:
	/// the readout row beneath them belongs to nobody, exactly as the old
	/// `click` oracle had it.
	#[test]
	fn the_preview_owns_its_cells_and_only_they_are_live() {
		let body = Rect::new(0.0, 600.0, 1280.0, 220.0);
		let (_chrome, mut ui, id) = hosted(body);

		let cells = ui.get::<ToolboxContent>(id).expect("typed root").preview_cells();
		let view = ui.rect_of(ui.get::<ToolboxContent>(id).unwrap().view).expect("the content widget is arranged");
		assert_eq!(cells[0].min(), view.min(), "the cells start at the content widget's own origin");
		assert!(cells.iter().all(|c| view.contains(c.center())), "and every cell sits inside it");
		assert_eq!(cells[4].y - cells[0].y, PREVIEW_CELL + PREVIEW_GAP, "row 1 is the mirrored row");

		// Cell 5 = mirrored, one quarter-turn (row 1, column 1).
		assert_eq!(orient_transform(5), map_core::Transform { rot: 1, mirror: true });
		let at = cells[5].center();
		ui.dispatch(&[press(true, at)]);
		assert!(ui.actions().is_empty(), "a press only arms");
		ui.dispatch(&[press(false, at)]);
		assert!(matches!(hit_of(ui.actions()[0]), Some(Hit::Orient(5))), "the release fires that cell");

		// The readout row under the cells is inert.
		let dead = Vec2::new(cells[4].x + 2.0, cells[4].bottom() + 4.0);
		assert!(view.contains(dead), "the probe is inside the block");
		ui.dispatch(&[press(true, dead), press(false, dead)]);
		assert!(ui.actions().is_empty(), "the readout row fires nothing");
	}

	/// The preview's heading tracks what the 8 cells are orientations *of* — the
	/// one piece of the block that is a stock `Label`, rewritten each frame
	/// through `descendant_mut` rather than redrawn by hand.
	#[test]
	fn the_preview_heading_follows_what_is_armed() {
		let body = Rect::new(0.0, 600.0, 1280.0, 220.0);
		let (_chrome, mut ui, id) = hosted(body);
		let heading = ui.get::<ToolboxContent>(id).unwrap().heading;
		assert_eq!(ui.get::<Label>(heading).expect("an id'd label").text(), "tile");

		let mut snap = Snapshot::empty();
		snap.stamp_dims = Some((3, 2));
		ui.get_mut::<ToolboxContent>(id).unwrap().sync(snap);
		assert_eq!(ui.get::<Label>(heading).expect("an id'd label").text(), "stamp");
	}

	/// A narrow dock wraps the flowed blocks onto further runs, and the
	/// `ScrollArea` scrolls exactly when the flow outgrows the panel — the shape
	/// this panel has always had, now the `Wrap`'s doing rather than a hand-rolled
	/// flow.
	#[test]
	fn a_narrow_dock_wraps_its_runs_and_scrolls() {
		let wide = Rect::new(0.0, 0.0, 1280.0, 124.0);
		let narrow = Rect::new(0.0, 0.0, 200.0, 124.0);

		// The first key of the last group: on one run with the rest in a wide
		// dock, pushed well down the flow in a narrow one.
		let last = GROUPS.len() - 1;
		let top_of_last = |body: Rect| {
			let (_chrome, ui, _id) = hosted(body);
			key_centre(&ui, tag(last, 0)).y - body.y
		};
		assert!(top_of_last(wide) < 100.0, "a wide dock keeps every block on one run");
		assert!(top_of_last(narrow) > 100.0, "a narrow one stacks them");

		let offset = |ui: &Ui| ui.get::<ToolboxContent>(ui.root().id()).expect("typed root").root.offset();
		let wheel = |ui: &mut Ui, body: Rect| {
			ui.dispatch(&[Event::Scroll {
				delta: ScrollDelta::Lines(Vec2::new(0.0, 1.0)),
				pos: body.center(),
				mods: Modifiers::NONE,
			}]);
		};

		let (_chrome, mut ui, _id) = hosted(narrow);
		wheel(&mut ui, narrow);
		assert!(offset(&ui) > 0.0, "the wrapped flow outgrows a 124px panel");

		let tall = Rect::new(0.0, 0.0, 200.0, 4000.0);
		let (_chrome, mut ui, _id) = hosted(tall);
		wheel(&mut ui, tall);
		assert_eq!(offset(&ui), 0.0, "a tall panel needs no scroll");
	}

	/// Both select groups are hosted `wgpu_ui::Select`s (U3.3), unchanged by the
	/// move into the tree: a press on a box opens *that* list (and only that
	/// one), a press on a row picks it and fires the option's own table row, and
	/// a press elsewhere just dismisses.
	#[test]
	fn select_groups_open_pick_and_dismiss() {
		// Wide enough that the flow fits on one run and nothing wraps.
		let body = Rect::new(0.0, 0.0, 1280.0, 124.0);
		let (_chrome, mut ui, id) = hosted(body);

		for (k, g) in select_groups().enumerate() {
			let sel_id = ui.get::<ToolboxContent>(id).expect("typed root").selects[k];
			let box_r = ui.rect_of(sel_id).expect("the box is arranged");
			ui.dispatch(&[press(true, box_r.center())]);
			assert!(ui.popup_open(), "{}: the box opens its list", GROUPS[g].label);
			assert!(ui.get::<Select>(sel_id).expect("typed").is_open(), "and it is this group's list");
			let all = ui.get::<ToolboxContent>(id).unwrap().selects.clone();
			let open = all.iter().filter(|&&s| ui.get::<Select>(s).is_some_and(Select::is_open)).count();
			assert_eq!(open, 1, "one at a time");

			// Pick the last option: the shell gets its table row, once.
			let popup = ui.get::<Select>(sel_id).expect("typed").popup_rect();
			ui.dispatch(&[press(true, Vec2::new(popup.x + 2.0, popup.bottom() - 2.0))]);
			let last = GROUPS[g].buttons.len() - 1;
			assert_eq!(ui.actions(), [tag(g, last)], "{}: picked its last option", GROUPS[g].label);
			assert!(!ui.popup_open(), "picking closes the list");

			// Re-open, then press well clear of it: dismissed, nothing picked.
			ui.dispatch(&[press(true, box_r.center())]);
			ui.dispatch(&[press(true, Vec2::new(body.x + 4.0, body.bottom() - 4.0))]);
			assert!(!ui.popup_open(), "an outside press dismisses");
			assert!(ui.actions().is_empty(), "and picks nothing");
		}

		// The "auto shore" box shows its *own* value, not the brush size — the bug
		// one shared open-flag and a brush-size-only matcher used to produce.
		let shore = select_groups().position(|g| GROUPS[g].label == "auto shore").expect("an auto-shore group");
		ui.get_mut::<ToolboxContent>(id).unwrap().sync(Snapshot::empty());
		let sel_id = ui.get::<ToolboxContent>(id).unwrap().selects[shore];
		assert_eq!(
			Select::selected_text(ui.get::<Select>(sel_id).expect("typed")),
			"disabled",
			"BrushShore::Off reads as its own label"
		);
	}

	/// An open dropdown's list reaches the shell's popup layer **uncropped**,
	/// even though the flow it lives in sits inside a `ScrollArea` — the
	/// container's viewport crop belongs to the base pass alone (toolkit
	/// `72d92ab`). This is the first panel to put a `Select` inside a scrolling
	/// flow, and in its default bottom dock the list flips *up*, clear of the
	/// panel body, so a crop would have hidden it outright.
	#[test]
	fn an_open_list_escapes_the_scroll_areas_crop() {
		let body = Rect::new(0.0, 660.0, 1280.0, 124.0);
		let (chrome, mut ui, id) = hosted(body);
		ui.set_viewport(wgpu_ui::Rect::new(0.0, 0.0, 1280.0, 800.0));
		ui.layout_in(body, chrome.theme(), chrome.fonts());

		let sel_id = ui.get::<ToolboxContent>(id).expect("typed root").selects[0];
		let box_r = ui.rect_of(sel_id).expect("arranged");
		ui.dispatch(&[press(true, box_r.center())]);
		assert!(ui.popup_open(), "the box opened");

		let popup = ui.get::<Select>(sel_id).expect("typed").popup_rect();
		assert!(popup.y < body.y, "a bottom dock flips the list up, out of the panel body");

		let mut overlay = DrawList::new();
		ui.draw_pass(&mut overlay, chrome.theme(), chrome.fonts(), DrawPass::Overlay);
		assert!(!overlay.cmds.is_empty(), "the list reaches the overlay pass");
		for cmd in &overlay.cmds {
			if let DrawCmd::PushClip(r) = cmd {
				let inside =
					r.x >= popup.x && r.y >= popup.y && r.right() <= popup.right() && r.bottom() <= popup.bottom();
				assert!(inside, "a clip outside the popup crops it: {r:?} vs {popup:?}");
			}
		}
	}
}
