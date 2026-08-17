//! Tile Explorer: the pickable-tile list (filters), the scrolling tile grid and
//! the header that drives them. The tile stills themselves are a **native GPU
//! pass** — an R8Uint index atlas through the palette shader
//! (`project_render::draw_picker`) — so this module hands that pass its quads
//! and scissor rather than drawing them. The map ghost stamp's GPU half stays in
//! `project_render::draw_picker` too.
//!
//! **The panel is a real `wgpu-ui` widget tree** (U5.6): a [`wgpu_ui::Linear`]
//! column of a header [`wgpu_ui::Wrap`] — the three hosted [`wgpu_ui::Select`]s
//! U3.5 gave it, four [`wgpu_ui::Button`] keys and the count
//! [`wgpu_ui::Label`] — over a [`PickerGrid`] **content widget** that owns the
//! [`crate::cellgrid`] geometry, its own [`Scroller`], the visible-window quads
//! the native pass draws, the selection/hover rings and the cell pick. There is
//! no hit oracle, no panel-wide `ArmFire` and no `Hot`: hover, arming and fire
//! are each key's own, and everything the panel produces comes back as an
//! **action tag** polled off `Ui::actions` — [`action_of`] maps it back to an
//! [`Action`] the shell turns into a command.

use std::collections::HashSet;
use wgpu_ui::Vec2;
use wgpu_ui::widget::{DrawCtx, EventCtx, LayoutCtx};

use map_core::Project;

use crate::theme;
use wgpu_ui::{
	ArmFire, Button, CrossAlign, DrawList, Event, Insets, Label, Length, Linear, PageKeys, Scroller, Select, Size,
	Widget, WidgetId, Wrap, descendant, descendant_mut,
};

use crate::ui::Rect;
use crate::uikit_theme::rgba;

/// Display sizes the size dropdown offers (the larger ones suit a wide panel
/// or close inspection of a single tile).
pub const SIZES: [f32; 7] = [16.0, 24.0, 32.0, 48.0, 64.0, 128.0, 256.0];
/// Height of one header control — a `Select::small` row, and the keys beside it.
const BTN_H: f32 = 18.0;
/// Inner padding of the tile grid.
const PAD: f32 = 4.0;
/// Gap between tiles, between rows, and between header controls.
const GAP: f32 = 2.0;
/// The header band's margins. A run is [`BTN_H`] + [`GAP`] = the 20px row the
/// hand-flowed header always drew, so 2px above the first run and a 4px gutter
/// below the last reproduce its exact height. That matters more here than in
/// any other panel: the grid below is a **native GPU pass**, and its scissor is
/// this band's complement — a band a pixel out crops a tile row rather than
/// merely moving one (U5.6).
const HDR_PAD: Insets = Insets { left: PAD, top: 2.0, right: PAD, bottom: 4.0 };

/// A scroll the command layer asked for, drained into the panel widget's own
/// [`Scroller`] on the next sync — `EditorState::execute` runs without the
/// panel `Ui` in reach, so it cannot set an offset directly (U2.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollRequest {
	/// An absolute offset (`picker scroll N`, or 0 after a filter change).
	To(f32),
	/// Bring item `index` into view, moving as little as possible — the
	/// just-picked tile after a map eyedrop.
	Reveal(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
	All,
	Used,
	Unused,
	Water,
	Shore,
	Land,
	Blocked,
}

impl Filter {
	pub const ALL: [Filter; 7] =
		[Filter::All, Filter::Used, Filter::Unused, Filter::Water, Filter::Shore, Filter::Land, Filter::Blocked];

	pub fn name(self) -> &'static str {
		match self {
			Filter::All => "all",
			Filter::Used => "used",
			Filter::Unused => "unused",
			Filter::Water => "water",
			Filter::Shore => "shore",
			Filter::Land => "land",
			Filter::Blocked => "blocked",
		}
	}

	pub fn next(self) -> Filter {
		let i = Self::ALL.iter().position(|&f| f == self).unwrap_or(0);
		Self::ALL[(i + 1) % Self::ALL.len()]
	}

	pub fn parse(s: &str) -> Option<Filter> {
		Self::ALL.iter().copied().find(|f| f.name() == s)
	}

	/// The pass value this filter selects (0 land / 1 water / 2 shore /
	/// 3 blocked) - `None` for the non-pass filters.
	fn pass(self) -> Option<u8> {
		match self {
			Filter::Land => Some(0),
			Filter::Water => Some(1),
			Filter::Shore => Some(2),
			Filter::Blocked => Some(3),
			_ => None,
		}
	}
}

pub struct PickerState {
	pub tile_px: f32,
	/// A scroll the command layer asked for, pending until the panel widget
	/// drains it (U2.4) — the offset itself lives in that widget's `Scroller`.
	pub scroll_request: Option<ScrollRequest>,
	pub filter: Filter,
	/// Restrict the grid to a single tile pack by **name** (None = all packs).
	/// Stored by name (not index) so it survives switching between open maps that
	/// load different packs - a name absent from the active map reads as "all".
	pub tileset: Option<String>,
}

impl Default for PickerState {
	fn default() -> Self {
		Self { tile_px: 32.0, scroll_request: None, filter: Filter::All, tileset: None }
	}
}

impl PickerState {
	pub fn cycle_size(&mut self) {
		let i = SIZES.iter().position(|&s| s == self.tile_px).unwrap_or(2);
		self.tile_px = SIZES[(i + 1) % SIZES.len()];
	}
}

/// One pickable tile.
pub struct Item<'a> {
	/// Pack/tile coordinates - tests pin the contract; the eyedropper and
	/// group filters (the custom group filter) are the future readers.
	#[allow(dead_code)]
	pub pack: usize,
	#[allow(dead_code)]
	pub tile: u16,
	pub id: &'a str,
	/// Global atlas index (`sum of preceding packs' tile counts + tile`) -
	/// the same contract `project_render::build_cell_data` uses.
	pub index: u32,
}

/// The index of the pack named `tileset` in `project`, or `None` (all packs)
/// when nothing is selected or the named pack isn't in this map (a stale
/// cross-map selection). The single resolver every `items` caller shares.
pub fn tileset_index(project: &Project, tileset: Option<&str>) -> Option<usize> {
	tileset.and_then(|name| project.packs.iter().position(|p| p.name == name))
}

/// The project's tiles under `filter`, restricted to pack `tileset` (its index,
/// `None` = every pack), in pack order. Atlas indices stay global (a skipped
/// pack still advances the running base) so `Item.index` matches the shader.
pub fn items(project: &Project, filter: Filter, tileset: Option<usize>) -> Vec<Item<'_>> {
	let used: Option<HashSet<(u8, u16)>> = match filter {
		Filter::Used | Filter::Unused => {
			Some(project.cells.iter().flat_map(|stack| stack.iter().flatten()).map(|t| (t.pack, t.tile)).collect())
		}
		_ => None,
	};

	let mut out = Vec::new();
	let mut base = 0u32;
	for (pack_index, pack) in project.packs.iter().enumerate() {
		let show = match tileset {
			Some(t) => t == pack_index,
			None => true,
		};
		if show {
			for tile in 0..pack.tile_count() {
				let keep = match filter {
					Filter::All => true,
					Filter::Used => used.as_ref().is_some_and(|u| u.contains(&(pack_index as u8, tile))),
					Filter::Unused => used.as_ref().is_some_and(|u| !u.contains(&(pack_index as u8, tile))),
					f => pack.pass.as_ref().is_some_and(|pass| Some(pass[tile as usize]) == f.pass()),
				};
				if keep {
					out.push(Item { pack: pack_index, tile, id: &pack.ids[tile as usize], index: base + tile as u32 });
				}
			}
		}
		base += pack.tile_count() as u32;
	}
	out
}

/// What a header key needs before it does anything (the shared header-key
/// convention, [`crate::panel_ui`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Need {
	/// Nothing — `new` authors from scratch.
	Always,
	/// A selected tile to act on.
	Tile,
}

/// The header's command keys, in flow order: label, the [`Action`] the key
/// fires, and what it needs. A key whose need fails reads **disabled-dead**
/// with the reason as its tooltip — the shared header-key convention
/// ([`crate::panel_ui`]; it supersedes G4's muted-but-live rule, which
/// predates tooltips). Deeper refusals — a shipped tile without `--dev` —
/// stay the command's, reported loudly in the console.
const COMMANDS: [(&str, Action, Need); 4] = [
	("new", Action::New, Need::Always),
	("clone", Action::Clone, Need::Tile),
	("edit", Action::Edit, Need::Tile),
	("delete", Action::Delete, Need::Tile),
];

/// The fixed width shared by the four command keys (sized to the longest
/// label). A [`Wrap`] gives every child its *measured* size, so a compact key
/// pins its own with `Button::sized` (G15) rather than leaving it to the
/// theme's dialog-button minimum. The three dropdowns carry no width: a
/// [`Select`] measures to its own widest option, which is why U3.5 made them
/// dropdowns in the first place.
///
/// `label_w` measures through the theme the header is drawn with — the same
/// shape [`crate::tabs::TabStrip`] resolves its tab widths in, and the reason
/// the editor no longer parses the font a second time to answer this (U7.1).
fn action_w(label_w: impl Fn(&str) -> f32) -> f32 {
	let longest = COMMANDS.iter().map(|&(label, ..)| label_w(label)).fold(0.0_f32, f32::max);
	longest + 12.0
}

/// What a fired action tag resolved to. `Pick` carries the index into the
/// filtered [`items`] list the view was built from; the shell resolves it
/// against live state (the same re-hit-at-release robustness the old
/// shell-armed path had).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
	/// Make grid item `i` (index into the filtered [`items`]) the active brush.
	Pick(usize),
	/// Pick tileset option `i` (0 = all packs, else pack `i-1`) — the hosted
	/// dropdown's commit; opening and dismissing are its own (U3.5).
	SetTileset(usize),
	/// Pick filter option `i` (index into [`Filter::ALL`]).
	SetFilter(usize),
	/// Pick size option `i` (index into [`SIZES`]).
	SetSize(usize),
	/// Open the Tile Painter on a blank new tile.
	New,
	/// Open the Tile Painter cloning the selected tile.
	Clone,
	/// Open the Tile Painter editing the selected tile.
	Edit,
	/// Delete the selected tile from its pack.
	Delete,
}

/// The tag space: a kind in the high bits over a 32-bit payload, so one
/// `Ui::actions` poll answers for the whole panel (U5.4's shape). Kind `0` is
/// deliberately unused — a stray zero tag resolves to nothing.
const KIND_SHIFT: u32 = 32;
/// A header command key: the payload is its row in [`COMMANDS`].
const KIND_CMD: u64 = 1;
/// A tileset-filter pick: the payload is `0` = all packs, else pack `i-1`.
const KIND_TILESET: u64 = 2;
/// A pass-filter pick: the payload is its index into [`Filter::ALL`].
const KIND_FILTER: u64 = 3;
/// A tile-size pick: the payload is its index into [`SIZES`].
const KIND_SIZE: u64 = 4;
/// A tile pick: the payload is its index into the filtered list.
const KIND_PICK: u64 = 5;

const fn tag(kind: u64, i: usize) -> u64 {
	(kind << KIND_SHIFT) | i as u64
}

/// The Tile Explorer action a fired tag stands for, or `None` if it is not one
/// of this panel's (the shell polls every tag its `Ui` collected).
pub fn action_of(tag: u64) -> Option<Action> {
	let i = (tag & 0xffff_ffff) as usize;
	match tag >> KIND_SHIFT {
		KIND_CMD => COMMANDS.get(i).map(|&(_, action, _)| action),
		// The pack list is per-map, so its range is the shell's to check (an
		// index past the packs reads as "all", exactly like a stale name).
		KIND_TILESET => Some(Action::SetTileset(i)),
		KIND_FILTER => (i < Filter::ALL.len()).then_some(Action::SetFilter(i)),
		KIND_SIZE => (i < SIZES.len()).then_some(Action::SetSize(i)),
		KIND_PICK => Some(Action::Pick(i)),
		_ => None,
	}
}

/// A tile quad for the GPU grid pass (`transform` = map-core bits; the
/// grid passes 0 = base art, the toolbox preview the active transform).
pub struct TileQuad {
	pub index: u32,
	pub transform: u32,
	pub rect: Rect,
}

/// The global atlas index of a tile ref - the same cumulative-pack-base
/// contract `project_render::build_cell_data` uses.
pub fn global_index(project: &Project, t: map_core::TileRef) -> u32 {
	let base: u32 = project.packs[..t.pack as usize].iter().map(|p| p.tile_count() as u32).sum();
	base + t.tile as u32
}

/// The picker-relevant state, snapshotted into the tree each frame so the
/// retained draw holds no `EditorState`/`Project` borrow: the filtered items
/// (id + atlas index, in display order - the grid draw and [`Action::Pick`]
/// share it) and the header's scalars.
#[derive(Clone)]
pub struct Snapshot {
	items: Vec<(String, u32)>,
	filter: Filter,
	/// The selected pack, resolved to an index into `tilesets` (`None` = all) -
	/// a stale cross-map name resolves to `None` and reads as "all".
	tileset: Option<usize>,
	/// The pack names, in project order - the tileset box label + option list.
	tilesets: Vec<String>,
	tile_px: f32,
	/// The active brush spec (transform suffix tolerated) - selection ring +
	/// the clone/edit/delete weight.
	active: Option<String>,
}

impl Snapshot {
	/// Snapshot the picker-relevant editor state for one frame's draw. The tile
	/// stills render from the index atlas ([`PickerGrid::tile_quads`]), so no
	/// composed-atlas handle is needed here.
	pub fn of(project: &Project, state: &PickerState, active: Option<&str>) -> Self {
		let tileset = tileset_index(project, state.tileset.as_deref());
		Self {
			items: items(project, state.filter, tileset).into_iter().map(|it| (it.id.to_string(), it.index)).collect(),
			filter: state.filter,
			tileset,
			tilesets: project.packs.iter().map(|p| p.name.clone()).collect(),
			tile_px: state.tile_px,
			active: active.map(str::to_string),
		}
	}

	fn empty() -> Self {
		Self {
			items: Vec::new(),
			filter: Filter::All,
			tileset: None,
			tilesets: Vec::new(),
			tile_px: 32.0,
			active: None,
		}
	}
}

/// The cell under `p` in `grid`, scrolled to `offset`, out of `count` tiles —
/// the grid's domain hit oracle. The padding, the gaps between cells and the
/// run past the last tile belong to nobody, exactly as the panel's old `hit_at`
/// oracle had them.
///
/// Free rather than a method so [`PickerGrid`] can hand it to its own `ArmFire`
/// without borrowing itself immutably and mutably at once.
fn cell_at(grid: &crate::cellgrid::Grid, offset: f32, count: usize, p: Vec2) -> Option<usize> {
	if !grid.body.contains(p) {
		return None;
	}
	let i = grid.index_at(p.x, p.y, offset)?;
	(i < count && grid.item_rect(i, offset).contains(p)).then_some(i)
}

/// Scroll offset that brings item `index` into `grid`'s visible window, moving
/// as little as possible from `scroll` (a no-op when it's already shown). Used
/// to reveal the just-picked tile (the map eyedropper).
fn scroll_to_reveal(grid: &crate::cellgrid::Grid, count: usize, index: usize, scroll: f32) -> f32 {
	let max = grid.max_scroll(count);
	let top = grid.item_rect(index, scroll).y; // current on-screen top
	let bot = top + grid.cell;
	let (win_top, win_bot) = (grid.body.y, grid.body.bottom());
	let s = if top < win_top {
		scroll - (win_top - top) // scroll up to bring the top into view
	} else if bot > win_bot {
		scroll + (bot - win_bot) // scroll down to bring the bottom into view
	} else {
		scroll // already visible
	};
	s.clamp(0.0, max)
}

/// The Tile Explorer's **content widget**: the scrolling tile grid.
///
/// It owns exactly what §5.2 allows a content widget to own — the
/// [`crate::cellgrid::Grid`] geometry, its own [`Scroller`], the domain cell
/// pick, the rings, and the `(quads, scissor)` the native index-atlas pass
/// renders the stills from — and no chrome: the three dropdowns, the four
/// command keys and the count are its **siblings** in the panel tree, never its
/// children.
///
/// Arranged straight *into* its viewport (not into a tall content rect), which
/// is what keeps G7 deferred: the widget clips its own draw, scrolls the rows
/// through it, and hands the GPU pass a scissor that is simply its own rect.
pub struct PickerGrid {
	id: WidgetId,
	snap: Snapshot,
	rect: Rect,
	scroller: Scroller,
	/// The theme's scrollbar metric, sampled at `arrange` — the gutter
	/// [`grid`](Self::grid) reserves, kept equal to the bar the `Scroller`
	/// paints.
	gutter: f32,
	/// Arm-on-press / fire-on-release-inside over the cells — the domain hit
	/// test a content widget keeps (the panel's chrome oracle is gone).
	clicks: ArmFire<usize>,
	/// The cell the pointer is over, tracked here because a *cell* is this
	/// widget's own domain — the `Ui` can only say whether the grid is hovered.
	/// The ring is drawn only while it agrees (see [`Self::draw`]).
	hover: Option<usize>,
	/// A [`ScrollRequest`] the command layer queued, applied at the next
	/// `arrange` (once the geometry it has to resolve against is known).
	pending_scroll: Option<ScrollRequest>,
}

impl PickerGrid {
	fn new() -> Self {
		Self {
			id: wgpu_ui::next_id(),
			snap: Snapshot::empty(),
			rect: Rect::ZERO,
			scroller: Scroller::new(),
			gutter: 8.0,
			clicks: ArmFire::new(),
			hover: None,
			pending_scroll: None,
		}
	}

	/// The cell geometry over this widget's own arranged rect. The grid *is* the
	/// viewport now, so it carries no header offset — the header band is the
	/// sibling above it.
	fn grid(&self) -> crate::cellgrid::Grid {
		crate::cellgrid::Grid {
			body: self.rect,
			cell: self.snap.tile_px,
			gap: GAP,
			pad: PAD,
			gutter: self.gutter,
			row_extra: 0.0,
		}
	}

	/// Tile `i`'s cell rect at the current scroll.
	fn item_rect(&self, i: usize) -> Rect {
		self.grid().item_rect(i, self.scroller.offset())
	}

	/// The cell under `p`, if any — the domain hit oracle.
	fn cell_at(&self, p: Vec2) -> Option<usize> {
		cell_at(&self.grid(), self.scroller.offset(), self.snap.items.len(), p)
	}

	/// The visible tile stills as index-atlas quads, plus the scissor to clip
	/// them to — the geometry `project_render::draw_picker` renders. Off-window
	/// rows are culled; the scissor is simply this widget's own rect, which is
	/// what makes the native pass and the chrome over it one layout by
	/// construction (U5.6 — before, both were recomputed from the panel body and
	/// a hand-flowed header height).
	fn tile_quads(&self) -> (Vec<TileQuad>, Rect) {
		let (g, scroll) = (self.grid(), self.scroller.offset());
		let clip = self.rect;
		let mut quads = Vec::new();
		for (i, (_, index)) in self.snap.items.iter().enumerate() {
			let r = g.item_rect(i, scroll);
			if r.bottom() < clip.y || r.y > clip.bottom() {
				continue;
			}
			quads.push(TileQuad { index: *index, transform: 0, rect: r });
		}
		(quads, clip)
	}
}

impl Widget for PickerGrid {
	fn measure(&mut self, avail: Size, _ctx: &mut LayoutCtx) -> Size {
		avail
	}

	fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
		self.rect = rect;
		self.gutter = ctx.theme.metrics().scrollbar;
		// The grid window is the viewport; the flowed rows are the content.
		// Re-clamps the offset when the filter or cell size changes.
		let count = self.snap.items.len();
		self.scroller.layout(ctx, rect, self.grid().content_height(count));
		// A queued command-layer scroll resolves here, where the geometry is known.
		match self.pending_scroll.take() {
			Some(ScrollRequest::To(v)) => self.scroller.set_offset(v),
			Some(ScrollRequest::Reveal(i)) => {
				let to = scroll_to_reveal(&self.grid(), count, i, self.scroller.offset());
				self.scroller.set_offset(to);
			}
			None => {}
		}
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		// The stills themselves are drawn straight from the R8Uint index atlas
		// through the palette shader (`project_render::draw_picker`, see
		// [`Self::tile_quads`]) — no per-frame RGBA compose, and they retint live
		// on palette edits. This `DrawList` carries only the rings over them.
		//
		// A cell rings dim under the pointer, gated on the `Ui` agreeing that this
		// widget is hovered at all — which an open header dropdown makes false,
		// because the `Ui` collapses hover to the popup's owner.
		let hovered = ctx.is_hovered(self.id).then_some(self.hover).flatten();
		let active_id = self.snap.active.as_deref().map(|s| s.split(':').next().unwrap_or(s));
		dl.push_clip(self.rect);
		for (i, (id, _)) in self.snap.items.iter().enumerate() {
			let r = self.item_rect(i);
			if r.bottom() < self.rect.y || r.y > self.rect.bottom() {
				continue;
			}
			let ring = Rect::new(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0);
			if active_id == Some(id.as_str()) {
				dl.stroke_rect(ring, 1.0, rgba(theme::INK));
			} else if hovered == Some(i) {
				// Hover ring on the cell under the cursor (dimmer than selection).
				dl.stroke_rect(ring, 1.0, rgba(theme::INK_DIM));
			}
		}
		dl.pop_clip();
		// The bar sits over the rows, outside their clip.
		self.scroller.draw(dl, ctx);
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		// Track the cell under the pointer while this widget is the one being
		// pointed at; anything else (a popup owning the pointer, the cursor
		// leaving) drops it.
		match ev {
			Event::PointerMoved { .. } | Event::PointerButton { .. } => {
				self.hover = ctx.is_target(self.id).then(|| self.cell_at(ctx.pointer)).flatten();
			}
			Event::PointerLeft | Event::Focus(false) => self.hover = None,
			_ => {}
		}
		let (grid, offset, count) = (self.grid(), self.scroller.offset(), self.snap.items.len());
		let handled = self.clicks.event(ev, ctx, self.id, |p| cell_at(&grid, offset, count, p));
		// The fire goes out as an action tag, so the shell polls one place for
		// the whole panel (the header keys' `Ui::actions`) instead of a second
		// channel per widget kind.
		if let Some(i) = self.clicks.take_outcome() {
			ctx.fire(self.id, Some(tag(KIND_PICK, i)));
		}
		if handled {
			return true;
		}
		// The cells keep first refusal; the wheel, the bar and the paging keys
		// fall to the scroller.
		self.scroller.event_with(ev, ctx, self.id, PageKeys::WhenHovered)
	}

	fn rect(&self) -> Rect {
		self.rect
	}

	fn id(&self) -> WidgetId {
		self.id
	}

	/// The cells and the scrollbar column claim the pointer; the padding and the
	/// gaps between them stay inert, exactly as the old `hit_at` oracle had them.
	/// The bar has to be claimed explicitly — [`Scroller`] only takes a press
	/// when its owner is the dispatch target.
	fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
		let bar = self.scroller.has_bar() && self.scroller.track_rect().contains(pos);
		(bar || self.cell_at(pos).is_some()).then_some(self.id)
	}
}

/// The Tile Explorer as a retained `wgpu_ui` [`Widget`]: a thin root over a
/// `Linear` column of the header flow and the [`PickerGrid`]. It exists to hold
/// the id tables and to push the per-frame [`Snapshot`] into them; everything
/// else — layout, paint, hover, arming, firing, scrolling, the dropdowns' popup
/// layer — is the tree's.
pub struct PickerContent {
	id: WidgetId,
	root: Linear,
	/// The three header dropdowns — tileset, pass filter, tile size (U3.5), in
	/// flow order. Each owns its open state, dismissal, keyboard and popup
	/// placement; their option lists are re-synced per frame, since the pack
	/// list is per-map.
	selects: [WidgetId; 3],
	/// The four command keys, in [`COMMANDS`] order — the last three take their
	/// weight from whether a tile is selected.
	keys: [WidgetId; 4],
	/// The visible-tile count readout.
	count: WidgetId,
	grid: WidgetId,
	rect: Rect,
}

impl Default for PickerContent {
	fn default() -> Self {
		Self::new()
	}
}

impl PickerContent {
	pub fn new() -> Self {
		// The header flows onto as many runs as it needs and the grid takes the
		// rest — `Length::Fit` measures the `Wrap` at the panel's width, so the
		// band is exactly as tall as the runs it produced.
		// `run_extent` is what makes a run a **row**: without it a run carrying
		// only the count `Label` would be that label's own height, and the band
		// — whose complement is the grid's rect, and so the native pass's
		// scissor — would lose a pixel at exactly the dock widths where the flow
		// packs that way.
		let mut header =
			Wrap::row().padding(HDR_PAD).spacing(GAP).run_spacing(GAP).run_extent(BTN_H).line_align(CrossAlign::Center);
		let mut selects = [WidgetId::NONE; 3];
		for slot in &mut selects {
			let sel = Select::new(Vec::<String>::new()).small();
			*slot = sel.id();
			header = header.push(sel);
		}
		// The keys' shared width is the widest label's, and only a `LayoutCtx`
		// knows how wide that is — `arrange` pins it (G29). The height is the
		// band's, which is this module's own constant either way.
		let mut keys = [WidgetId::NONE; 4];
		for (i, &(label, ..)) in COMMANDS.iter().enumerate() {
			let key = Button::new(label).small().sized(0.0, BTN_H).action(tag(KIND_CMD, i));
			keys[i] = key.id();
			header = header.push(key);
		}
		let count = Label::new("0").small().muted().with_id();
		let count_id = count.id();
		header = header.push(count);

		let grid = PickerGrid::new();
		let grid_id = grid.id();
		let root = Linear::column().child(header, Length::Fit).child(grid, Length::Flex(1.0));
		Self { id: wgpu_ui::next_id(), root, selects, keys, count: count_id, grid: grid_id, rect: Rect::ZERO }
	}

	/// Push one frame's state into the retained tree: the three dropdowns'
	/// options and values, the command keys' weight, the count, and the grid's
	/// visible tiles. The tileset list is per-map, so it is rebuilt each frame
	/// (an open list survives `Select::set_options`).
	pub fn sync(&mut self, snap: Snapshot) {
		if let Some(sel) = descendant_mut::<Select>(&mut self.root, self.selects[0]) {
			let mut tilesets = Vec::with_capacity(snap.tilesets.len() + 1);
			tilesets.push("all".to_string());
			tilesets.extend(snap.tilesets.iter().cloned());
			sel.set_options(tilesets);
			sel.set_selected(snap.tileset.map_or(0, |i| i + 1));
		}
		if let Some(sel) = descendant_mut::<Select>(&mut self.root, self.selects[1]) {
			sel.set_options(Filter::ALL.iter().map(|f| f.name()));
			sel.set_selected(Filter::ALL.iter().position(|&f| f == snap.filter).unwrap_or(0));
		}
		if let Some(sel) = descendant_mut::<Select>(&mut self.root, self.selects[2]) {
			sel.set_options(SIZES.iter().map(|s| format!("{} px", *s as u32)));
			sel.set_selected(SIZES.iter().position(|&s| s == snap.tile_px).unwrap_or(0));
		}
		// A key whose need fails greys out dead, with the reason as its tooltip
		// (the shared header-key convention, [`crate::panel_ui`]). The command
		// behind it still validates loudly - scripts reach it directly.
		let unmet = |need: Need| match need {
			Need::Always => None,
			Need::Tile => snap.active.is_none().then_some("needs a selected tile"),
		};
		for (i, &(_, _, need)) in COMMANDS.iter().enumerate() {
			if let Some(key) = descendant_mut::<Button>(&mut self.root, self.keys[i]) {
				crate::panel_ui::sync_header_key(key, unmet(need));
			}
		}
		if let Some(label) = descendant_mut::<Label>(&mut self.root, self.count) {
			label.set_text(snap.items.len().to_string());
		}
		if let Some(grid) = descendant_mut::<PickerGrid>(&mut self.root, self.grid) {
			grid.snap = snap;
		}
	}

	/// Queue a scroll the command layer asked for; it lands at the next
	/// `arrange`, resolved against the geometry the panel actually has.
	pub fn request_scroll(&mut self, req: ScrollRequest) {
		if let Some(grid) = descendant_mut::<PickerGrid>(&mut self.root, self.grid) {
			grid.pending_scroll = Some(req);
		}
	}

	/// The visible tile stills as index-atlas quads + their scissor — the shell
	/// draws them through `project_render::draw_picker`, under this panel's
	/// chrome. Read *after* `build`, which is what settles both the grid's rect
	/// and the scroll offset they hang off.
	///
	/// The widget hands the native pass its geometry rather than the pass
	/// recomputing it (the U5.3 invariant): `draw_picker` goes through the
	/// separately-borrowed `ProjectRenderer`, so there is one computation of the
	/// numbers and nothing to drift.
	pub fn visible_tile_quads(&self) -> (Vec<TileQuad>, Rect) {
		descendant::<PickerGrid>(&self.root, self.grid).map_or_else(|| (Vec::new(), Rect::ZERO), PickerGrid::tile_quads)
	}
}

impl Widget for PickerContent {
	fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
		self.rect = rect;
		// The command keys' shared width, measured through the theme the header
		// draws with — resolved *before* the measure below, or the flow would wrap
		// on last frame's widths (U7.1).
		let key_w = {
			let px = ctx.theme.font_px(wgpu_ui::TextRole::Small);
			let font = ctx.fonts.get(ctx.theme.font());
			action_w(|s| font.measure(s, px))
		};
		for &id in &self.keys {
			if let Some(key) = descendant_mut::<Button>(&mut self.root, id) {
				key.set_size(key_w, BTN_H);
			}
		}
		// Measure here as well as in `measure`: the header `Wrap` settles its run
		// count there, and a host that arranges without measuring first (the
		// snapshot harness) must still get a laid-out tree.
		self.root.measure(rect.size(), ctx);
		self.root.arrange(rect, ctx);
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		// The header's steel band, under the tree: only the root knows how tall
		// the flow wrapped this frame. Base pass only; the overlay pass carries an
		// open option list out to the shell's popup layer (U3.2).
		if ctx.is_base()
			&& let Some(band) = Widget::child(&self.root, 0).map(Widget::rect)
		{
			ctx.theme.header_band(dl, band);
		}
		self.root.draw(dl, ctx);
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		let handled = self.root.event(ev, ctx);
		crate::panel_ui::drain_selects(&mut self.root, &self.selects, ctx, |i, v| {
			Some(tag([KIND_TILESET, KIND_FILTER, KIND_SIZE][i], v))
		});
		handled
	}

	crate::panel_ui::thin_root_plumbing!();
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::uikit_menu::MenuChrome;
	use crate::uikit_theme::rgba;
	use std::path::Path;
	use wgpu_ui::Theme as _;
	use wgpu_ui::{DrawCmd, Modifiers, PointerButton, ScrollDelta, Ui, widget::DrawPass};

	fn project() -> Project {
		let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/assets/tilepacks");
		Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap()
	}

	/// The chrome fixture + the panel hosted in a `Ui`, laid out into `body`.
	/// A stock `Button` measures its own label, so this needs the real fonts
	/// (`Fonts::new()` + `Gunmetal` panics with "FontId(0) is not registered").
	fn hosted(body: Rect, snap: Snapshot) -> (MenuChrome, Ui, WidgetId) {
		let (_device, _queue, chrome) = crate::visual_test::chrome_fixture();
		let content = PickerContent::new();
		let id = content.id();
		let mut ui = Ui::new(content);
		ui.get_mut::<PickerContent>(id).expect("typed root").sync(snap);
		ui.layout_in(body, chrome.theme(), chrome.fonts());
		(chrome, ui, id)
	}

	/// A snapshot of the whole GREEN project at the default 32px cell.
	fn all_tiles(project: &Project, active: Option<&str>) -> Snapshot {
		Snapshot::of(project, &PickerState::default(), active)
	}

	fn press(pressed: bool, at: Vec2) -> Event {
		Event::PointerButton { button: PointerButton::Primary, pressed, pos: at, mods: Modifiers::NONE }
	}

	/// The grid child, borrowed typed off the hosted tree.
	fn grid_of(ui: &Ui, id: WidgetId) -> &PickerGrid {
		let content = ui.get::<PickerContent>(id).expect("typed root");
		descendant::<PickerGrid>(&content.root, content.grid).expect("the content widget")
	}

	/// The header flow (the root column's first child).
	fn header_of(ui: &Ui, id: WidgetId) -> &dyn Widget {
		Widget::child(&ui.get::<PickerContent>(id).expect("typed root").root, 0).expect("the header")
	}

	/// The base-pass draw of the hosted panel.
	fn drawn(chrome: &MenuChrome, ui: &Ui) -> DrawList {
		let mut dl = DrawList::new();
		ui.draw_pass(&mut dl, chrome.theme(), chrome.fonts(), DrawPass::Base);
		dl
	}

	#[test]
	fn filters_partition_the_packs() {
		let p = project();
		let total: usize = p.packs.iter().map(|k| k.tile_count() as usize).sum();
		assert_eq!(items(&p, Filter::All, None).len(), total);

		// Pass filters cover every tile exactly once (both packs ship pass data).
		let by_pass: usize = [Filter::Water, Filter::Shore, Filter::Land, Filter::Blocked]
			.iter()
			.map(|&f| items(&p, f, None).len())
			.sum();
		assert_eq!(by_pass, total);

		// A fresh map uses only water variants.
		let used = items(&p, Filter::Used, None);
		assert!(!used.is_empty() && used.len() <= 12);
		assert!(used.iter().all(|i| i.id.starts_with("WTR")));
		assert_eq!(items(&p, Filter::Unused, None).len(), total - used.len());

		// Atlas indices follow the pack_base contract (WATER first).
		let all = items(&p, Filter::All, None);
		assert_eq!((all[0].pack, all[0].tile, all[0].index), (0, 0, 0));
		let first_green = all.iter().find(|i| i.pack == 1).unwrap();
		assert_eq!(first_green.tile, 0);
		assert_eq!(first_green.index, p.packs[0].tile_count() as u32);
	}

	/// The tileset filter restricts the grid to one pack (by index) while keeping
	/// each tile's *global* atlas index; the pass filters still apply within it,
	/// and the name→index resolver ignores a pack that isn't in this map.
	#[test]
	fn tileset_filter_restricts_to_one_pack() {
		let p = project();
		let total: usize = p.packs.iter().map(|k| k.tile_count() as usize).sum();
		// WATER is pack 0, GREEN is pack 1.
		let water = items(&p, Filter::All, Some(0));
		let green = items(&p, Filter::All, Some(1));
		assert_eq!(water.len() + green.len(), total, "the two packs partition the grid");
		assert!(water.iter().all(|i| i.pack == 0) && green.iter().all(|i| i.pack == 1));
		// GREEN's tiles keep their global atlas index (offset past WATER).
		let base = p.packs[0].tile_count() as u32;
		assert_eq!(green[0].index, base, "a filtered pack keeps its global atlas index");
		// The pass filter still narrows within the chosen pack.
		let green_land = items(&p, Filter::Land, Some(1));
		assert!(green_land.len() < green.len() && green_land.iter().all(|i| i.pack == 1));

		// Name resolver: known pack → its index; unknown / none → all.
		assert_eq!(tileset_index(&p, Some("GREEN")), Some(1));
		assert_eq!(tileset_index(&p, Some("WATER")), Some(0));
		assert_eq!(tileset_index(&p, Some("DESERT")), None, "a pack not in this map reads as all");
		assert_eq!(tileset_index(&p, None), None);
	}

	/// The pass-value mapping behind the pass filters: land/water/shore/blocked
	/// select their pass byte; the non-pass filters (all/used/unused) map to
	/// none - they filter by usage, not passability. The filter cycle and the
	/// name round-trip are the `picker filter next` / `picker filter NAME` path.
	#[test]
	fn only_pass_filters_carry_a_pass_value() {
		assert_eq!(Filter::Land.pass(), Some(0));
		assert_eq!(Filter::Water.pass(), Some(1));
		assert_eq!(Filter::Shore.pass(), Some(2));
		assert_eq!(Filter::Blocked.pass(), Some(3));
		for f in [Filter::All, Filter::Used, Filter::Unused] {
			assert_eq!(f.pass(), None, "{} is not a pass filter", f.name());
		}
		assert_eq!(Filter::All.next(), Filter::Used);
		assert_eq!(Filter::Blocked.next(), Filter::All);
		assert_eq!(Filter::parse("shore"), Some(Filter::Shore));
		assert_eq!(Filter::parse("nope"), None);

		let mut s = PickerState::default();
		s.cycle_size();
		assert_eq!(s.tile_px, 48.0);
	}

	/// The global atlas index of a tile ref follows the cumulative-pack-base
	/// contract: pack 0 starts at 0; pack 1 starts after all of pack 0's tiles.
	#[test]
	fn global_index_follows_the_pack_base_contract() {
		let p = project();
		let t = |pack: u8, tile: u16| map_core::TileRef { pack, tile, transform: map_core::Transform::default() };
		assert_eq!(global_index(&p, t(0, 0)), 0);
		assert_eq!(global_index(&p, t(0, 5)), 5);
		let base = p.packs[0].tile_count() as u32;
		assert_eq!(global_index(&p, t(1, 3)), base + 3, "pack 1 sits after pack 0's tiles");
	}

	/// Every header command key fires **its own** table row on a press +
	/// release-inside. That is the whole click path now: no hit oracle, no
	/// panel-wide `ArmFire`, and no action written down twice. A tile is armed
	/// so every key's need holds - a dead key is the next test's subject.
	#[test]
	fn every_command_key_fires_its_own_row() {
		let p = project();
		let body = Rect::new(1000.0, 50.0, 278.0, 500.0);
		let (_chrome, mut ui, id) = hosted(body, all_tiles(&p, Some("WTR003")));

		for (i, &(label, action, _)) in COMMANDS.iter().enumerate() {
			let key_id = ui.get::<PickerContent>(id).expect("typed root").keys[i];
			let at = ui.rect_of(key_id).expect("the key is arranged").center();
			ui.dispatch(&[press(true, at)]);
			assert!(ui.actions().is_empty(), "{label}: a press only arms");
			ui.dispatch(&[press(false, at)]);
			assert_eq!(ui.actions().len(), 1, "{label}: one key, one action");
			assert_eq!(action_of(ui.actions()[0]), Some(action), "{label} resolves to its own row");
		}
	}

	/// Every tag resolves back to what built it, and a tag from nowhere resolves
	/// to nothing — the mapping the shell runs a fired action through.
	#[test]
	fn every_tag_resolves_to_its_own_action() {
		for (i, &(label, action, _)) in COMMANDS.iter().enumerate() {
			assert_eq!(action_of(tag(KIND_CMD, i)), Some(action), "{label}");
		}
		assert_eq!(action_of(tag(KIND_CMD, COMMANDS.len())), None, "a key past the table is nothing");
		assert_eq!(action_of(tag(KIND_TILESET, 0)), Some(Action::SetTileset(0)));
		assert_eq!(action_of(tag(KIND_FILTER, 2)), Some(Action::SetFilter(2)));
		assert_eq!(action_of(tag(KIND_FILTER, Filter::ALL.len())), None, "a filter past the list is nothing");
		assert_eq!(action_of(tag(KIND_SIZE, 3)), Some(Action::SetSize(3)));
		assert_eq!(action_of(tag(KIND_SIZE, SIZES.len())), None, "and so is a size past the list");
		assert_eq!(action_of(tag(KIND_PICK, 7)), Some(Action::Pick(7)));
		assert_eq!(action_of(0), None, "the unused kind resolves to nothing");
	}

	/// `clone` / `edit` / `delete` need a tile to work on. Without one they are
	/// **disabled-dead** with the reason as their tooltip - the shared
	/// header-key convention ([`crate::panel_ui`], superseding G4's
	/// muted-but-live rule). `new` never greys.
	#[test]
	fn the_three_keys_that_need_a_tile_grey_out_dead_without_one() {
		let p = project();
		let body = Rect::new(1000.0, 50.0, 278.0, 500.0);
		let dead = |ui: &Ui, id: WidgetId| {
			ui.get::<PickerContent>(id)
				.expect("typed root")
				.keys
				.map(|k| ui.get::<Button>(k).expect("a command key").is_disabled())
		};

		let (_chrome, mut ui, id) = hosted(body, all_tiles(&p, None));
		assert_eq!(dead(&ui, id), [false, true, true, true], "nothing armed: only `new` reads live");
		let clone_key = ui.get::<PickerContent>(id).expect("typed root").keys[1];
		assert_eq!(
			wgpu_ui::Widget::tooltip(ui.get::<Button>(clone_key).expect("a command key")),
			Some("needs a selected tile"),
			"a dead key says why on hover"
		);

		ui.get_mut::<PickerContent>(id).expect("typed root").sync(all_tiles(&p, Some("WTR003")));
		assert_eq!(dead(&ui, id), [false; 4], "with a tile armed every key reads live");
		assert_eq!(
			wgpu_ui::Widget::tooltip(ui.get::<Button>(clone_key).expect("a command key")),
			None,
			"a live key carries no tooltip"
		);

		// Disabled is dead, not decorative: the key swallows the click and
		// fires nothing (the command still refuses loudly for the script path -
		// state.rs owns that half).
		let (_chrome, mut ui, id) = hosted(body, all_tiles(&p, None));
		let key_id = ui.get::<PickerContent>(id).expect("typed root").keys[1];
		let at = ui.rect_of(key_id).expect("arranged").center();
		ui.dispatch(&[press(true, at)]);
		ui.dispatch(&[press(false, at)]);
		assert!(ui.actions().is_empty(), "a dead key fires nothing");
	}

	/// A tile arms on the press and fires its filtered-list index on the
	/// release; the gaps between cells belong to nobody — exactly as the old
	/// `hit_at` oracle had them.
	#[test]
	fn a_tile_picks_and_the_gaps_do_not() {
		let p = project();
		let body = Rect::new(1000.0, 50.0, 278.0, 500.0);
		let (_chrome, mut ui, id) = hosted(body, all_tiles(&p, None));

		for i in [0usize, 1, 7, 8] {
			let r = grid_of(&ui, id).item_rect(i);
			let at = Vec2::new(r.x + 5.0, r.y + 5.0);
			ui.dispatch(&[press(true, at)]);
			assert!(ui.actions().is_empty(), "tile {i}: a press only arms");
			ui.dispatch(&[press(false, at)]);
			assert_eq!(action_of(ui.actions()[0]), Some(Action::Pick(i)), "tile {i}");
		}

		// The gap between two cells in a row is inert, and consumes nothing.
		let r0 = grid_of(&ui, id).item_rect(0);
		let gap = Vec2::new(r0.right() + GAP / 2.0, r0.center().y);
		assert_eq!(grid_of(&ui, id).cell_at(gap), None, "the gap between cells picks nothing");
		let resp = ui.dispatch(&[press(true, gap), press(false, gap)]);
		assert!(!resp.wants_pointer(), "and consumes nothing");
		assert!(ui.actions().is_empty());
	}

	/// The three dropdowns are hosted `wgpu_ui::Select`s (U3.5), unchanged by the
	/// move into the tree: a press on a box opens *that* list, and picking a row
	/// fires the option's own action tag.
	#[test]
	fn the_dropdowns_open_pick_and_dismiss() {
		let p = project();
		let body = Rect::new(1000.0, 50.0, 278.0, 500.0);
		let (_chrome, mut ui, id) = hosted(body, all_tiles(&p, None));

		let wants = [
			Action::SetTileset(p.packs.len()),
			Action::SetFilter(Filter::ALL.len() - 1),
			Action::SetSize(SIZES.len() - 1),
		];
		for (k, want) in wants.into_iter().enumerate() {
			let sel_id = ui.get::<PickerContent>(id).expect("typed root").selects[k];
			let box_r = ui.rect_of(sel_id).expect("the box is arranged");
			ui.dispatch(&[press(true, box_r.center())]);
			assert!(ui.popup_open(), "box {k} opens its list");
			let all = ui.get::<PickerContent>(id).unwrap().selects;
			let open = all.iter().filter(|&&s| ui.get::<Select>(s).is_some_and(Select::is_open)).count();
			assert_eq!(open, 1, "one at a time");

			// Pick the last option: the shell gets it as this panel's action tag.
			let popup = ui.get::<Select>(sel_id).expect("typed").popup_rect();
			ui.dispatch(&[press(true, Vec2::new(popup.x + 2.0, popup.bottom() - 2.0))]);
			assert_eq!(action_of(ui.actions()[0]), Some(want), "box {k} picked its last option");
			assert!(!ui.popup_open(), "picking closes the list");

			// Re-open, then press well clear of it: dismissed, nothing picked.
			ui.dispatch(&[press(true, box_r.center())]);
			ui.dispatch(&[press(true, Vec2::new(body.x + 4.0, body.bottom() - 4.0))]);
			assert!(!ui.popup_open(), "an outside press dismisses");
			assert!(ui.actions().is_empty(), "and picks nothing");
		}
	}

	/// **A pick is reported on the press, and only for that dispatch.** A
	/// `Select` commits the row under the *press* (a command key waits for its
	/// release), and `Ui::dispatch` clears `actions` on the way in — so the
	/// release that follows would wipe a pick nobody had read yet. That is why
	/// the shell drains this panel after the press dispatch as well as after the
	/// release (`App::drain_picker`); this test is what says the drain cannot be
	/// moved to the release alone.
	#[test]
	fn a_dropdown_pick_lives_only_for_the_dispatch_that_made_it() {
		let p = project();
		let body = Rect::new(1000.0, 50.0, 278.0, 500.0);
		let (_chrome, mut ui, id) = hosted(body, all_tiles(&p, None));
		let sel_id = ui.get::<PickerContent>(id).expect("typed root").selects[1];
		let box_r = ui.rect_of(sel_id).expect("the box is arranged");

		ui.dispatch(&[press(true, box_r.center())]);
		let popup = ui.get::<Select>(sel_id).expect("typed").popup_rect();
		let row = Vec2::new(popup.x + 2.0, popup.y + 2.0);
		ui.dispatch(&[press(true, row)]);
		assert_eq!(action_of(ui.actions()[0]), Some(Action::SetFilter(0)), "the press itself picks");

		// The release of that same click — the dispatch the shell used to poll.
		ui.dispatch(&[press(false, row)]);
		assert!(ui.actions().is_empty(), "and the release clears it before anyone could read it");
	}

	/// **The check that G7's deferral held.** The tiles are a native GPU pass, so
	/// the content widget's whole job is to report the visible window: only the
	/// tiles that touch it, in list order, carrying their global atlas indices —
	/// and a scissor that is exactly the grid's own rect, which the header band
	/// and the panel body bracket.
	#[test]
	fn the_grid_reports_only_the_window_and_a_scissor_that_is_its_own_rect() {
		let p = project();
		let body = Rect::new(1000.0, 50.0, 278.0, 300.0);
		let snap = all_tiles(&p, Some("WTR003:!N"));
		let total = snap.items.len();
		let (_chrome, ui, id) = hosted(body, snap.clone());

		let (quads, clip) = ui.get::<PickerContent>(id).expect("typed root").visible_tile_quads();
		let grid = grid_of(&ui, id);
		assert_eq!(clip, grid.rect, "the scissor is the grid's viewport");
		assert_eq!(clip.bottom(), body.bottom(), "which reaches the panel's bottom");
		assert_eq!(clip.y, header_of(&ui, id).rect().bottom(), "and starts below the header band");
		assert!(quads.len() < total, "off-window rows are culled");
		assert!(!quads.is_empty());
		for q in &quads {
			assert!(q.rect.bottom() >= clip.y && q.rect.y <= clip.bottom(), "each quad touches the window");
			assert_eq!(q.transform, 0, "the explorer shows base art");
		}
		assert_eq!(quads[0].index, snap.items[0].1, "quads carry the tiles' global atlas indices, in order");
	}

	/// The grid scrolls itself: the wheel over it moves the rows, and the
	/// command layer's requests land in its `Scroller` at the next layout —
	/// `execute` cannot reach the panel `Ui`, so `picker scroll N` and
	/// reveal-the-active-tile travel as requests (U2.4).
	#[test]
	fn the_grid_scrolls_itself_and_applies_scroll_requests() {
		let p = project();
		let body = Rect::new(0.0, 0.0, 280.0, 300.0);
		let (chrome, mut ui, id) = hosted(body, all_tiles(&p, None));
		let count = grid_of(&ui, id).snap.items.len();

		let wheel = Event::Scroll {
			delta: ScrollDelta::Lines(Vec2::new(0.0, 1.0)),
			pos: Vec2::new(140.0, 200.0),
			mods: Modifiers::NONE,
		};
		assert!(ui.dispatch(&[wheel]).wants_pointer(), "the grid takes the wheel");
		assert_eq!(grid_of(&ui, id).scroller.offset(), 48.0, "one wheel notch");

		// `picker scroll 120`, applied at the next arrange.
		ui.get_mut::<PickerContent>(id).unwrap().request_scroll(ScrollRequest::To(120.0));
		ui.layout_in(body, chrome.theme(), chrome.fonts());
		assert_eq!(grid_of(&ui, id).scroller.offset(), 120.0);

		// One-shot: a later layout must not re-apply it.
		ui.layout_in(body, chrome.theme(), chrome.fonts());
		assert_eq!(grid_of(&ui, id).scroller.offset(), 120.0, "the request fired once");

		// Reveal brings a scrolled-out tile back into the grid window — the map
		// eyedropper jumping to the tile it just picked up.
		ui.get_mut::<PickerContent>(id).unwrap().request_scroll(ScrollRequest::Reveal(count - 1));
		ui.layout_in(body, chrome.theme(), chrome.fonts());
		let (win, r) = (grid_of(&ui, id).rect, grid_of(&ui, id).item_rect(count - 1));
		assert!(
			r.y >= win.y - 0.5 && r.bottom() <= win.bottom() + 0.5,
			"the revealed tile sits in the window: {r:?} in {win:?}"
		);

		// …and an already-visible tile does not move the scroll at all.
		ui.get_mut::<PickerContent>(id).unwrap().request_scroll(ScrollRequest::To(0.0));
		ui.layout_in(body, chrome.theme(), chrome.fonts());
		ui.get_mut::<PickerContent>(id).unwrap().request_scroll(ScrollRequest::Reveal(0));
		ui.layout_in(body, chrome.theme(), chrome.fonts());
		assert_eq!(grid_of(&ui, id).scroller.offset(), 0.0, "the top tile needs no scroll");
	}

	/// A press in the scrollbar column pages — the bar is chrome the grid claims
	/// in its own `hit_test`, since a `Scroller` only takes a press aimed at its
	/// owner (U5.5). A list that fits never scrolls at all.
	#[test]
	fn the_grid_claims_its_own_scrollbar_column() {
		let p = project();
		let body = Rect::new(0.0, 0.0, 280.0, 300.0);
		let (_chrome, mut ui, _id) = hosted(body, all_tiles(&p, None));
		let bar = Vec2::new(body.right() - 4.0, body.bottom() - 4.0);
		assert!(ui.dispatch(&[press(true, bar)]).wants_pointer(), "the bar takes the press");

		let mut short = all_tiles(&p, None);
		short.items.truncate(4);
		let (_chrome, mut ui, id) = hosted(body, short);
		let wheel =
			Event::Scroll { delta: ScrollDelta::Lines(Vec2::new(0.0, 1.0)), pos: body.center(), mods: Modifiers::NONE };
		ui.dispatch(&[wheel]);
		assert_eq!(grid_of(&ui, id).scroller.offset(), 0.0, "one row never scrolls");
	}

	/// The selection ring hugs the armed tile and a hovered cell rings dim — and
	/// that dim ring goes out under an open header dropdown, because the `Ui`
	/// collapses hover to the popup's owner. Both rings are drawn over the native
	/// pass, and they are all the `DrawList` the grid layer carries.
	#[test]
	fn the_grid_rings_the_armed_tile_and_the_hovered_one() {
		let p = project();
		let body = Rect::new(0.0, 0.0, 280.0, 300.0);
		let (chrome, mut ui, id) = hosted(body, all_tiles(&p, Some("WTR000")));
		let ringed = |ui: &Ui, cell: Rect, color| {
			drawn(&chrome, ui).cmds.iter().any(|c| match c {
				DrawCmd::Solid { rect, color: c } => {
					rect.y == cell.y - 1.0 && rect.w == cell.w + 2.0 && *c == rgba(color)
				}
				_ => false,
			})
		};
		let cell0 = grid_of(&ui, id).item_rect(0);
		let cell1 = grid_of(&ui, id).item_rect(1);
		assert!(ringed(&ui, cell0, theme::INK), "the armed tile keeps its selection ring");
		assert!(!ringed(&ui, cell1, theme::INK_DIM), "nothing is hovered at rest");

		ui.dispatch(&[Event::PointerMoved { pos: cell1.center() }]);
		assert_eq!(grid_of(&ui, id).hover, Some(1), "the grid knows which cell it is");
		assert!(ringed(&ui, cell1, theme::INK_DIM), "the hovered tile rings dimly");

		// Open the tileset dropdown: the `Ui` hands it the pointer, so the grid's
		// ring goes out even though the cursor never moved.
		let sel_id = ui.get::<PickerContent>(id).expect("typed root").selects[0];
		let box_r = ui.rect_of(sel_id).expect("the box is arranged");
		ui.dispatch(&[press(true, box_r.center())]);
		assert!(ui.popup_open(), "the box opened its list");
		assert!(!ringed(&ui, cell1, theme::INK_DIM), "an open dropdown inerts the grid under it");
		ui.dispatch(&[Event::PointerMoved { pos: cell1.center() }]);
		assert_eq!(grid_of(&ui, id).hover, None, "the grid is not the pointer's target");
	}

	/// The header flows onto one run when the dock is wide and wraps when it is
	/// narrow — the shape the hand-rolled `header_flow` had — and the band it
	/// produces is exactly as tall: 2px above the runs, each of them a 20px row,
	/// and a 4px gutter below the last. **That is the invariant with teeth
	/// here**: the grid's rect is this band's complement and it *is* the native
	/// pass's scissor, so a band a pixel out crops a tile row.
	#[test]
	fn the_header_wraps_to_the_band_height_the_flow_always_had() {
		let p = project();
		let mut counts = Vec::new();
		for w in [700.0, 278.0, 180.0] {
			let body = Rect::new(0.0, 0.0, w, 400.0);
			let (_chrome, ui, id) = hosted(body, all_tiles(&p, None));
			let header = header_of(&ui, id);
			// How many runs the flow actually produced, read off the arranged
			// children rather than assumed from the width.
			let mut tops: Vec<u32> =
				(0..header.child_count()).filter_map(|i| header.child(i)).map(|c| c.rect().y as u32).collect();
			tops.dedup();
			let runs = tops.len() as f32;
			let (band, grid) = (header.rect(), grid_of(&ui, id).rect);
			assert_eq!(
				band.h,
				HDR_PAD.top + runs * (BTN_H + GAP) - GAP + HDR_PAD.bottom,
				"a {w}px dock flowed {runs} run(s)"
			);
			assert_eq!(grid.y, band.bottom(), "the grid starts below the band");
			assert_eq!(grid.bottom(), body.bottom(), "and reaches the body bottom");
			counts.push(runs);
		}
		assert_eq!(counts[0], 1.0, "a wide dock keeps every control on one run");
		assert_eq!(counts[1], 2.0, "the shipped dock width flows two runs, as it always has");
		assert!(counts[2] > counts[1], "and a narrower one wraps further: {counts:?}");
	}

	/// The header controls keep a fixed width whatever the dock does: the four
	/// keys are pinned to one measured width (G15/G29), and each dropdown
	/// measures to its own widest option — "SNOW_DARK", "blocked", "256 px" — so
	/// neither the closed box nor its popup rows ellipsize.
	///
	/// Both claims are checked against the **chrome the panel was laid out with**
	/// (U7.1): the editor has one font stack, so a test that wants to know how
	/// wide a label is asks the same theme the header asked.
	#[test]
	fn the_header_controls_are_fixed_width() {
		let p = project();
		let mut snap = all_tiles(&p, None);
		snap.tilesets = vec!["SNOW_DARK".to_string()];
		let laid = |w: f32| {
			let (chrome, ui, id) = hosted(Rect::new(0.0, 0.0, w, 400.0), snap.clone());
			let header = header_of(&ui, id);
			let widths =
				(0..header.child_count()).filter_map(|i| header.child(i)).map(|c| c.rect().w).collect::<Vec<_>>();
			(chrome, widths)
		};
		let ((chrome, narrow), (_, wide)) = (laid(200.0), laid(700.0));
		assert_eq!(narrow.len(), 8, "three dropdowns, four keys and the count");
		assert_eq!(narrow, wide, "every control keeps its width whatever the dock does");

		let px = chrome.theme().font_px(wgpu_ui::TextRole::Small);
		let font = chrome.fonts().get(chrome.theme().font());
		let label_w = |s: &str| font.measure(s, px);
		for (i, w) in narrow[..3].iter().enumerate() {
			let longest = ["SNOW_DARK", "blocked", "256 px"][i];
			assert!(*w >= label_w(longest), "box {i} fits {longest}");
		}
		for w in &narrow[3..7] {
			assert_eq!(*w, action_w(label_w), "the four keys share one pinned width");
		}
	}

	/// Tab reaches a panel's dropdowns, and cycles them — the §6d orphan, closed
	/// by U5's trees rather than by any of U7's work. A panel root reports its
	/// subtree through `child`/`child_count`, a `Select` accepts focus, and
	/// `Ui::focus_step` walks exactly that, so the three header boxes are ordinary
	/// tab stops in flow order and the fourth Tab wraps to the first. This is what
	/// "a widget behaves the same wherever it is mounted" means for a dropdown:
	/// the panel does nothing a dialog does not.
	#[test]
	fn tab_cycles_the_header_dropdowns() {
		let p = project();
		let (_chrome, mut ui, id) = hosted(Rect::new(0.0, 0.0, 700.0, 400.0), all_tiles(&p, None));
		let selects = ui.get::<PickerContent>(id).expect("typed root").selects;
		let tab = Event::Key { key: wgpu_ui::Key::Tab, pressed: true, repeat: false, mods: Modifiers::NONE };
		for (i, &want) in [selects[0], selects[1], selects[2], selects[0]].iter().enumerate() {
			ui.dispatch(std::slice::from_ref(&tab));
			assert_eq!(ui.focused(), want, "Tab #{} lands on the box the flow shows next", i + 1);
		}
	}

	/// The count readout is the visible-tile total, and it tracks the filter.
	#[test]
	fn the_count_reports_the_visible_tiles() {
		let p = project();
		let body = Rect::new(0.0, 0.0, 278.0, 400.0);
		let text = |ui: &Ui, id: WidgetId| {
			let content = ui.get::<PickerContent>(id).expect("typed root");
			descendant::<Label>(&content.root, content.count).expect("the count").text().to_string()
		};
		let (_chrome, ui, id) = hosted(body, all_tiles(&p, None));
		assert_eq!(text(&ui, id), items(&p, Filter::All, None).len().to_string());

		let water = Snapshot::of(&p, &PickerState { filter: Filter::Water, ..Default::default() }, None);
		let n = water.items.len();
		let (_chrome, ui, id) = hosted(body, water);
		assert_eq!(text(&ui, id), n.to_string(), "a filter re-counts");
	}
}
