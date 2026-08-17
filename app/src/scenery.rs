//! The Scenery panel (`SCENERY.md` stage D): the open project's cut-out
//! libraries as a picker-style grid, plus the flat index the Scenery layer's
//! tools arm from.
//!
//! Pure logic - the GPU half (atlas + quad pass) is `scenery_render.rs`, input
//! routing `main.rs`. The panel is the Templates Explorer's shape (U5.5): a
//! header [`Wrap`] of the pack filter, the preview-size dropdown and the count,
//! over a [`SceneryGrid`] **content widget** whose thumbnails are a native pass
//! and whose rows carry a name strip.
//!
//! It holds **no tools**. Placing, moving and deleting a cut-out are the
//! Scenery *layer's* pencil, eraser and arrow (`state::scenery_twin`), which
//! live on the toolbox with every other tool - a panel that grew its own
//! `move` / `del` keys was a second place to look for them.
//!
//! ## Two index spaces, deliberately
//!
//! * A **flat** index runs over every piece in every loaded library, in library
//!   order ([`piece_at`]). That is what `EditorState::active_scenery` holds and
//!   what `scenery-pick` takes, because it does not move when the header's
//!   filter changes.
//! * A **visible** index runs over the rows the grid is currently showing
//!   ([`visible_pieces`]). That is what a fired [`Action::Pick`] carries, and
//!   the shell resolves it to a flat one before anything else sees it.
//!
//! Neither reaches the document: a placement names its pack and piece by
//! string, so re-baking or reordering a library cannot silently move an object
//! already on the map.

use wgpu_ui::widget::{DrawCtx, EventCtx, LayoutCtx, Widget};
use wgpu_ui::{
	ArmFire, CrossAlign, DrawList, Emboss, Event, Insets, Label, Length, Linear, PageKeys, Scroller, Select, Size,
	TextRole, Vec2, WidgetId, Wrap, descendant, descendant_mut,
};

use crate::theme;
use crate::ui::Rect;
use crate::uikit_theme::rgba;
use map_core::{Project, SceneryBlend, SceneryPiece};

/// Every piece the project's libraries hold, flattened in library order - the
/// index space [`EditorState::active_scenery`] and the `scenery-pick` command
/// share.
///
/// [`EditorState::active_scenery`]: crate::state::EditorState::active_scenery
pub fn piece_at(project: &Project, flat: usize) -> Option<(&str, &SceneryPiece)> {
	let mut seen = 0usize;
	for lib in &project.scenery_packs {
		if flat < seen + lib.pieces.len() {
			return Some((&lib.pack, &lib.pieces[flat - seen]));
		}
		seen += lib.pieces.len();
	}
	None
}

/// How many pieces [`piece_at`] indexes.
pub fn piece_count(project: &Project) -> usize {
	project.scenery_packs.iter().map(|l| l.pieces.len()).sum()
}

/// The flat index of `(pack, id)`, for arming the tool from a placement the user
/// grabbed rather than from the grid.
pub fn index_of(project: &Project, pack: &str, id: &str) -> Option<usize> {
	let mut seen = 0usize;
	for lib in &project.scenery_packs {
		if lib.pack == pack {
			if let Some(i) = lib.pieces.iter().position(|p| p.id == id) {
				return Some(seen + i);
			}
		}
		seen += lib.pieces.len();
	}
	None
}

/// The libraries a project loaded, by pack name - the header filter's options.
pub fn pack_names(project: &Project) -> Vec<String> {
	project.scenery_packs.iter().map(|l| l.pack.clone()).collect()
}

/// Order two display names the way a person reads them: a run of digits
/// compares as a **number**, so "Mountain 2" comes before "Mountain 10" rather
/// than after it.
///
/// The bake writes its manifest in plain ASCII order, which interleaves a
/// library's numbering into `1, 10, 11, ... 2, 20, ...` - unreadable in a grid
/// of 25 near-identical green silhouettes, which is the one place the number is
/// how you tell them apart.
///
/// A digit run compares by value with leading zeros ignored (longer run of
/// significant digits wins, then digit by digit), and only as a tiebreak by the
/// run's written width - so "Trees 07" and "Trees 7" land adjacent instead of
/// either sorting far apart or comparing equal. Everything else compares
/// case-insensitively, with the exact string settling an otherwise perfect tie
/// so the order is total and stable.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
	use std::cmp::Ordering;
	/// The leading digit run, consumed off `it`.
	fn digits(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
		let mut run = String::new();
		while it.peek().is_some_and(char::is_ascii_digit) {
			run.push(it.next().unwrap_or_default());
		}
		run
	}
	let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
	loop {
		let ord = match (x.peek().copied(), y.peek().copied()) {
			// Every run matched: the exact strings settle it (case, width, "" vs "").
			(None, None) => return a.cmp(b),
			(None, Some(_)) => return Ordering::Less,
			(Some(_), None) => return Ordering::Greater,
			(Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
				// Compared as text with leading zeros stripped, not parsed: a run of
				// any length is exact, and nothing can overflow.
				let (da, db) = (digits(&mut x), digits(&mut y));
				let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
				ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)).then_with(|| da.len().cmp(&db.len()))
			}
			(Some(ca), Some(cb)) => {
				x.next();
				y.next();
				ca.to_lowercase().cmp(cb.to_lowercase())
			}
		};
		if ord != Ordering::Equal {
			return ord;
		}
	}
}

/// The flat indices the grid lists: every piece, or one pack's when the header
/// filter names one (a name no library answers to lists nothing, the same way a
/// stale tileset filter does in the Templates Explorer).
///
/// Listed by **name**, [`natural_cmp`], not in library order - the flat index is
/// an identity, not a position, so the grid is free to sort. Ties break on the
/// flat index, which makes the order total: two packs may each hold a
/// "Mountain 3", and with no tiebreak `sort_by` would leave which one comes
/// first up to the sort's internals.
pub fn visible_pieces(project: &Project, pack: Option<&str>) -> Vec<usize> {
	let mut out: Vec<usize> = (0..piece_count(project))
		.filter(|&i| match pack {
			Some(want) => piece_at(project, i).is_some_and(|(p, _)| p == want),
			None => true,
		})
		.collect();
	let name = |i: usize| piece_at(project, i).map_or("", |(_, p)| p.name.as_str());
	out.sort_by(|&a, &b| natural_cmp(name(a), name(b)).then(a.cmp(&b)));
	out
}

// --- panel (a header flow over the sprite grid) -------------------------------

/// Height of one header control.
const BTN_H: f32 = 18.0;
/// Gap between header controls, between its runs, and below the last run.
const HDR_GAP: f32 = 4.0;
/// The header band's margins - the Templates Explorer's, so the two explorers'
/// bands line up when they are docked side by side.
const HDR_PAD: Insets = Insets { left: 2.0, top: 2.0, right: 2.0, bottom: HDR_GAP };
/// Inner padding of the thumbnail grid.
const PAD: f32 = 4.0;
/// Gap between cells and between rows.
const GAP: f32 = 4.0;
/// The name strip under each thumbnail. Without it a picker of 48 lumpy green
/// silhouettes is unusable - "Trees 14" and "Rock 3" look alike at any size.
const NAME_H: f32 = 14.0;

/// Thumbnail sizes the preview-size dropdown offers, with their labels. Larger
/// than the Templates Explorer's at the top end: a scenery piece is a landscape
/// feature, and a mountain range at 48px reads as a smudge.
pub const PREVIEW_SIZES: [(f32, &str); 5] =
	[(48.0, "very small"), (64.0, "small"), (96.0, "medium"), (128.0, "large"), (192.0, "very large")];

/// The preview size a fresh editor uses. Big enough that a name strip holds a
/// whole piece name ("Mountain 10") and that a mountain range is more than a
/// green smudge; the dropdown goes both ways from there.
pub const DEFAULT_PREVIEW: f32 = 96.0;

/// What a fired action tag resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
	/// Arm a piece for placing; the payload is its **visible** index.
	Pick(usize),
	/// Author a new cut-out from an image (opens the New Scenery dialog).
	New,
	/// Bring in a `.scn` or a `.png` (opens the file picker).
	Import,
	/// Author a copy of the armed piece under a fresh id - what you do with a
	/// shipped cut-out, which is read-only.
	Clone,
	/// Re-author the armed piece in place (user pieces, or `--dev`).
	Edit,
	/// Write the armed piece out as a shareable `.scn`.
	Export,
	/// Delete the armed piece (opens the confirmation).
	Delete,
	/// Rename the armed piece (opens the rename dialog).
	Rename,
	/// Pick preview-size option `i` (an index into [`PREVIEW_SIZES`]).
	SizeOption(usize),
	/// Pick pack-filter option `i` (0 = all, else pack `i-1`).
	PackOption(usize),
	/// Pick blend-mode option `i` (an index into `SceneryBlend::ALL`) - the mode
	/// the *next* placement takes.
	BlendOption(usize),
}

/// What a header key needs before it does anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Need {
	/// Nothing - authoring from scratch is always available.
	Always,
	/// A piece armed in the grid.
	Armed,
	/// An armed piece this install may rewrite: the user's own, or anything at
	/// all under `--dev`. **Shipped cut-outs are read-only** - the shipped bake
	/// is what every stock map's placements resolve through - so `edit`,
	/// `rename` and `delete` grey out on one and `clone` is the way in.
	Editable,
}

/// The header's command keys, in flow order: label, the [`Action`] the key
/// fires and what it needs. A [`Wrap`] sizes every child to what it
/// *measures*, so each key pins its own compact width with `Button::sized`
/// rather than leaving it to the theme's dialog-button minimum - measured from
/// its label at `arrange` (picker's G29/U7.1 rule; these were hand-pinned
/// magic numbers before 2026-08-11).
const COMMANDS: [(&str, Action, Need); 7] = [
	("new", Action::New, Need::Always),
	("import", Action::Import, Need::Always),
	("clone", Action::Clone, Need::Armed),
	("edit", Action::Edit, Need::Editable),
	("rename", Action::Rename, Need::Editable),
	("export", Action::Export, Need::Armed),
	("delete", Action::Delete, Need::Editable),
];

/// The tag space: a kind in the high bits over a 32-bit payload, so one
/// `Ui::actions` poll answers for the whole panel. Kind `0` is deliberately
/// unused - a stray zero tag resolves to nothing.
const KIND_SHIFT: u32 = 32;
const KIND_PICK: u64 = 1;
const KIND_SIZE: u64 = 2;
const KIND_PACK: u64 = 3;
/// A header command key: the payload is its row in [`COMMANDS`].
const KIND_CMD: u64 = 4;
const KIND_BLEND: u64 = 5;

const fn tag(kind: u64, i: usize) -> u64 {
	(kind << KIND_SHIFT) | i as u64
}

/// The scenery action a fired tag stands for, or `None` if it is not one of
/// this panel's.
pub fn action_of(tag: u64) -> Option<Action> {
	let i = (tag & 0xffff_ffff) as usize;
	match tag >> KIND_SHIFT {
		KIND_PICK => Some(Action::Pick(i)),
		KIND_SIZE => (i < PREVIEW_SIZES.len()).then_some(Action::SizeOption(i)),
		// The pack list is per-map, so its range is the shell's to check (an
		// index past the libraries reads as "all", like a stale name).
		KIND_PACK => Some(Action::PackOption(i)),
		KIND_CMD => COMMANDS.get(i).map(|&(_, action, _)| action),
		KIND_BLEND => (i < SceneryBlend::ALL.len()).then_some(Action::BlendOption(i)),
		_ => None,
	}
}

/// The cell under `p` - the grid's domain hit oracle. A thumbnail's target
/// includes its name strip; the padding and the gaps belong to nobody. Free
/// rather than a method so [`SceneryGrid`] can hand it to its own `ArmFire`
/// without borrowing itself twice.
fn cell_at(grid: &crate::cellgrid::Grid, offset: f32, count: usize, p: Vec2) -> Option<usize> {
	if !grid.body.contains(p) {
		return None;
	}
	let i = grid.index_at(p.x, p.y, offset)?;
	let r = grid.item_rect(i, offset);
	(i < count && Rect::new(r.x, r.y, r.w, r.h + NAME_H).contains(p)).then_some(i)
}

/// One listed piece, as the grid draws it.
#[derive(Clone)]
struct Item {
	/// Its **flat** index - what the native thumbnail pass resolves the sprite
	/// through, and what a pick turns into.
	flat: usize,
	name: String,
}

/// The panel state the chrome reflects, snapshotted each frame so the retained
/// tree holds no document borrow.
#[derive(Clone)]
pub struct Snapshot {
	/// Preview cell px (drives the size dropdown's value and the grid's pitch).
	cell: f32,
	/// The pack filter resolved to an index into `packs` (`None` = all).
	pack_sel: Option<usize>,
	/// The pack filter's option labels.
	packs: Vec<String>,
	/// The listed pieces, in grid order.
	items: Vec<Item>,
	/// The armed piece as a **visible** index, or `None` when nothing is armed
	/// or the filter hides it.
	active: Option<usize>,
	/// Something is armed - even if the filter hides it, the verbs that act on
	/// the armed piece still apply.
	armed: bool,
	/// The armed piece may be rewritten (it is the user's, or this is a `--dev`
	/// build) - see [`Need::Editable`].
	editable: bool,
	/// The blend mode a new placement takes ([`EditorState::scenery_blend`]).
	/// Set with [`Snapshot::with_blend`] rather than through `of`: it is the
	/// editor's state, not the document's, and every other caller wants the
	/// default.
	blend: SceneryBlend,
}

impl Snapshot {
	/// Snapshot one frame. `active` is the armed **flat** index; it rings only
	/// while the current filter actually lists it.
	pub fn of(project: &Project, active: Option<usize>, cell: f32, pack: Option<&str>, dev: bool) -> Self {
		let packs = pack_names(project);
		let visible = visible_pieces(project, pack);
		let items = visible
			.iter()
			.map(|&flat| Item { flat, name: piece_at(project, flat).map_or_else(String::new, |(_, p)| p.name.clone()) })
			.collect();
		let armed_piece = active.and_then(|flat| piece_at(project, flat));
		Self {
			cell,
			pack_sel: pack.and_then(|want| packs.iter().position(|p| p == want)),
			packs,
			items,
			active: active.and_then(|flat| visible.iter().position(|&v| v == flat)),
			armed: armed_piece.is_some(),
			editable: armed_piece.is_some_and(|(_, p)| p.user || dev),
			blend: SceneryBlend::default(),
		}
	}

	/// The blend mode the header's dropdown shows.
	pub fn with_blend(mut self, blend: SceneryBlend) -> Self {
		self.blend = blend;
		self
	}

	fn empty() -> Self {
		Self {
			cell: DEFAULT_PREVIEW,
			pack_sel: None,
			packs: Vec::new(),
			items: Vec::new(),
			active: None,
			armed: false,
			editable: false,
			blend: SceneryBlend::default(),
		}
	}

	/// The reason the key at `i` in [`COMMANDS`] is dead this frame, or `None`
	/// when its [`Need`] holds - what `sync` turns into the disabled state and
	/// the tooltip (the shared header-key convention, [`crate::panel_ui`]).
	fn key_unmet(&self, i: usize) -> Option<&'static str> {
		match COMMANDS.get(i).map(|&(_, _, need)| need) {
			Some(Need::Always) | None => None,
			Some(Need::Armed) if self.armed => None,
			Some(Need::Editable) if self.editable => None,
			Some(Need::Editable) if self.armed => Some("shipped cut-outs are read-only - clone it"),
			Some(Need::Armed | Need::Editable) => Some("needs an armed piece"),
		}
	}
}

/// The panel's **content widget**: the scrolling thumbnail grid. It owns its
/// grid geometry, its `Scroller`, the domain cell pick, the name strips and the
/// selection / hover rings - and no chrome; the two dropdowns and the count are
/// its siblings in the tree.
pub struct SceneryGrid {
	id: WidgetId,
	snap: Snapshot,
	rect: Rect,
	scroller: Scroller,
	gutter: f32,
	clicks: ArmFire<usize>,
	hover: Option<usize>,
}

impl SceneryGrid {
	fn new() -> Self {
		Self {
			id: wgpu_ui::next_id(),
			snap: Snapshot::empty(),
			rect: Rect::ZERO,
			scroller: Scroller::new(),
			gutter: 8.0,
			clicks: ArmFire::new(),
			hover: None,
		}
	}

	fn grid(&self) -> crate::cellgrid::Grid {
		crate::cellgrid::Grid {
			body: self.rect,
			cell: self.snap.cell,
			gap: GAP,
			pad: PAD,
			gutter: self.gutter,
			row_extra: NAME_H,
		}
	}

	fn item_rect(&self, i: usize) -> Rect {
		self.grid().item_rect(i, self.scroller.offset())
	}

	fn cell_at(&self, p: Vec2) -> Option<usize> {
		cell_at(&self.grid(), self.scroller.offset(), self.snap.items.len(), p)
	}

	/// Every cell touching the window, as `(flat index, thumbnail rect)` - the
	/// one list the wells, the native thumbnails and the rings are all laid out
	/// from. The index is the **flat** one, so `scenery_render::thumb_quads` can
	/// resolve a sprite without knowing about the filter.
	fn visible_cells(&self) -> Vec<(usize, Rect)> {
		self.snap
			.items
			.iter()
			.enumerate()
			.map(|(i, item)| (item.flat, self.item_rect(i)))
			.filter(|(_, r)| r.bottom() + NAME_H >= self.rect.y && r.y <= self.rect.bottom())
			.collect()
	}
}

impl Widget for SceneryGrid {
	fn measure(&mut self, avail: Size, _ctx: &mut LayoutCtx) -> Size {
		avail
	}

	fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
		self.rect = rect;
		self.gutter = ctx.theme.metrics().scrollbar;
		self.scroller.layout(ctx, rect, self.grid().content_height(self.snap.items.len()));
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		dl.push_clip(self.rect);
		if self.snap.items.is_empty() {
			let note = if self.snap.packs.is_empty() {
				"this map's tile packs ship no scenery"
			} else {
				"no scenery in this pack - clear the filter to see the rest"
			};
			ctx.theme.text_wrapped(
				dl,
				ctx.fonts,
				Rect::new(self.rect.x, self.rect.y + 4.0, self.rect.w, self.rect.h),
				PAD,
				note,
				TextRole::Small,
				Emboss::Engraved,
				rgba(theme::INK_DIM),
			);
			dl.pop_clip();
			return;
		}
		// The thumbnails are the native pass; this list carries the rings and the
		// name strips, and composites over it.
		let hovered = ctx.is_hovered(self.id).then_some(self.hover).flatten();
		for (i, item) in self.snap.items.iter().enumerate() {
			let r = self.item_rect(i);
			if r.bottom() + NAME_H < self.rect.y || r.y > self.rect.bottom() {
				continue;
			}
			let ring = Rect::new(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0);
			if self.snap.active == Some(i) {
				dl.stroke_rect(ring, 1.0, rgba(theme::ACCENT));
			} else if hovered == Some(i) {
				dl.stroke_rect(ring, 1.0, rgba(theme::INK_DIM));
			}
			ctx.theme.text_fit(
				dl,
				ctx.fonts,
				Rect::new(r.x, r.y + r.h, r.w, NAME_H),
				1.0,
				&item.name,
				TextRole::Small,
				Emboss::Engraved,
				rgba(theme::INK),
			);
		}
		dl.pop_clip();
		self.scroller.draw(dl, ctx);
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		match ev {
			Event::PointerMoved { .. } | Event::PointerButton { .. } => {
				self.hover = ctx.is_target(self.id).then(|| self.cell_at(ctx.pointer)).flatten();
			}
			Event::PointerLeft | Event::Focus(false) => self.hover = None,
			_ => {}
		}
		let (grid, offset, count) = (self.grid(), self.scroller.offset(), self.snap.items.len());
		let handled = self.clicks.event(ev, ctx, self.id, |p| cell_at(&grid, offset, count, p));
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
	/// gaps between them stay inert. The bar has to be claimed explicitly -
	/// [`Scroller`] only takes a press when its owner is the dispatch target.
	fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
		let bar = self.scroller.has_bar() && self.scroller.track_rect().contains(pos);
		(bar || self.cell_at(pos).is_some()).then_some(self.id)
	}
}

/// The Scenery panel as a retained widget: a thin root over a column of the
/// header flow and the [`SceneryGrid`].
pub struct SceneryContent {
	id: WidgetId,
	root: Linear,
	/// The header's command keys, in [`COMMANDS`] order - re-enabled per frame
	/// against what is armed.
	keys: [WidgetId; COMMANDS.len()],
	/// The pack-filter and preview-size dropdowns, in flow order. Each owns its
	/// open state, dismissal, keyboard and popup placement.
	selects: [WidgetId; 3],
	/// The listed-piece count readout.
	count: WidgetId,
	grid: WidgetId,
	rect: Rect,
}

impl Default for SceneryContent {
	fn default() -> Self {
		Self::new()
	}
}

impl SceneryContent {
	pub fn new() -> Self {
		// The header flows onto as many runs as it needs and the grid takes the
		// rest - `Length::Fit` measures the `Wrap` at the panel's width, so the
		// band is exactly as tall as the runs it produced. `run_extent` is what
		// makes a run a row rather than its tallest child's height.
		let mut header = Wrap::row()
			.padding(HDR_PAD)
			.spacing(HDR_GAP)
			.run_spacing(HDR_GAP)
			.run_extent(BTN_H)
			.line_align(CrossAlign::Center);
		// The keys' widths are their labels', and only a `LayoutCtx` knows how
		// wide those are - `arrange` pins them (G29).
		let mut keys = [WidgetId::NONE; COMMANDS.len()];
		for (i, &(label, ..)) in COMMANDS.iter().enumerate() {
			let key = wgpu_ui::Button::new(label).small().sized(0.0, BTN_H).action(tag(KIND_CMD, i));
			keys[i] = key.id();
			header = header.push(key);
		}
		let select = || Select::new(Vec::<String>::new()).small();
		let (pack, size, blend) = (select(), select(), select());
		let selects = [pack.id(), size.id(), blend.id()];
		let count = Label::new("0").small().muted().with_id();
		let count_id = count.id();
		header = header.push(pack).push(size).push(blend).push(count);

		let grid = SceneryGrid::new();
		let grid_id = grid.id();
		let root = Linear::column().child(header, Length::Fit).child(grid, Length::Flex(1.0));
		Self { id: wgpu_ui::next_id(), root, keys, selects, count: count_id, grid: grid_id, rect: Rect::ZERO }
	}

	/// Push one frame's state into the retained tree: the two dropdowns' options
	/// and values, the count, and the grid's listed pieces. The pack list is
	/// per-map, so it is rebuilt each frame (an open list survives
	/// `Select::set_options`).
	pub fn sync(&mut self, snap: Snapshot) {
		// A verb whose precondition fails greys out dead, with the reason as its
		// tooltip (the shared header-key convention, [`crate::panel_ui`]): what
		// a shipped cut-out will and will not let you do is visible before you
		// click, and hovering the grey key says why.
		for (i, &id) in self.keys.iter().enumerate() {
			if let Some(key) = descendant_mut::<wgpu_ui::Button>(&mut self.root, id) {
				crate::panel_ui::sync_header_key(key, snap.key_unmet(i));
			}
		}
		if let Some(sel) = descendant_mut::<Select>(&mut self.root, self.selects[0]) {
			let mut packs = Vec::with_capacity(snap.packs.len() + 1);
			packs.push("all".to_string());
			packs.extend(snap.packs.iter().cloned());
			sel.set_options(packs);
			sel.set_selected(snap.pack_sel.map_or(0, |i| i + 1));
		}
		if let Some(sel) = descendant_mut::<Select>(&mut self.root, self.selects[1]) {
			sel.set_options(PREVIEW_SIZES.iter().map(|&(_, name)| name));
			sel.set_selected(PREVIEW_SIZES.iter().position(|&(px, _)| px == snap.cell).unwrap_or(0));
		}
		if let Some(sel) = descendant_mut::<Select>(&mut self.root, self.selects[2]) {
			sel.set_options(SceneryBlend::ALL.iter().map(|m| m.name()));
			sel.set_selected(SceneryBlend::ALL.iter().position(|&m| m == snap.blend).unwrap_or(0));
		}
		if let Some(label) = descendant_mut::<Label>(&mut self.root, self.count) {
			label.set_text(snap.items.len().to_string());
		}
		if let Some(grid) = descendant_mut::<SceneryGrid>(&mut self.root, self.grid) {
			grid.snap = snap;
		}
	}

	/// Back to the top - the pack filter re-lists the grid, so the old offset
	/// would land on unrelated thumbnails.
	pub fn scroll_to_top(&mut self) {
		if let Some(grid) = descendant_mut::<SceneryGrid>(&mut self.root, self.grid) {
			grid.scroller.set_offset(0.0);
		}
	}

	/// The visible cells plus the scissor to clip them to. Read *after* `build`,
	/// which settles both the grid's rect and its scroll offset.
	pub fn visible_cells(&self) -> (Vec<(usize, Rect)>, Rect) {
		descendant::<SceneryGrid>(&self.root, self.grid)
			.map_or_else(|| (Vec::new(), Rect::ZERO), |g| (g.visible_cells(), g.rect))
	}
}

/// Black wells behind each visible cell, so a cut-out's palette reads against a
/// neutral ground instead of the steel panel. Drawn **before** the native pass
/// and clamped to `clip` - the grid's own rect, which is also the scissor.
pub fn cell_backgrounds(dl: &mut DrawList, cells: &[(usize, Rect)], clip: Rect) {
	for &(_, r) in cells {
		let top = r.y.max(clip.y);
		let bot = (r.y + r.h).min(clip.y + clip.h);
		if bot <= top {
			continue;
		}
		dl.fill_rect(Rect::new(r.x, top, r.w, bot - top), rgba(theme::SPRITE_WELL));
	}
}

impl Widget for SceneryContent {
	fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
		self.rect = rect;
		// Each key's width is its label's, measured through the theme the header
		// draws with - resolved *before* the measure below, or the flow would
		// wrap on last frame's widths (U7.1).
		let px = ctx.theme.font_px(wgpu_ui::TextRole::Small);
		let font = ctx.fonts.get(ctx.theme.font());
		for (i, &(label, ..)) in COMMANDS.iter().enumerate() {
			if let Some(key) = descendant_mut::<wgpu_ui::Button>(&mut self.root, self.keys[i]) {
				key.set_size(font.measure(label, px) + 12.0, BTN_H);
			}
		}
		// Measure here as well as in `measure`: the header `Wrap` settles its run
		// count there, and a host that arranges without measuring first (the
		// snapshot harness) must still get a laid-out tree.
		self.root.measure(rect.size(), ctx);
		self.root.arrange(rect, ctx);
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		// Only the root knows how tall the flow wrapped this frame, so - unlike a
		// fixed band - the shell cannot draw it. Base pass only; the overlay pass
		// carries an open option list out to the shell's popup layer (U3.2).
		if ctx.is_base()
			&& let Some(band) = Widget::child(&self.root, 0).map(Widget::rect)
		{
			ctx.theme.header_band(dl, band);
		}
		self.root.draw(dl, ctx);
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		let handled = self.root.event(ev, ctx);
		// One kind per dropdown, in `self.selects` order - a catch-all `else` here
		// silently filed the blend dropdown's picks under the preview size.
		crate::panel_ui::drain_selects(&mut self.root, &self.selects, ctx, |i, v| match i {
			0 => Some(tag(KIND_PACK, v)),
			1 => Some(tag(KIND_SIZE, v)),
			2 => Some(tag(KIND_BLEND, v)),
			_ => None,
		});
		handled
	}

	crate::panel_ui::thin_root_plumbing!();
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::uikit_menu::MenuChrome;
	use wgpu_ui::{Modifiers, PointerButton, ScrollDelta, Ui};

	fn green() -> Project {
		let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/assets/tilepacks");
		Project::new(8, 8, &["GREEN".to_string()], &root, 1).expect("GREEN project")
	}

	/// The chrome fixture + the panel hosted in a `Ui`, laid out into `body`.
	fn hosted(body: Rect, snap: Snapshot) -> (MenuChrome, Ui, WidgetId) {
		let (_device, _queue, chrome) = crate::visual_test::chrome_fixture();
		let content = SceneryContent::new();
		let id = content.id();
		let mut ui = Ui::new(content);
		ui.get_mut::<SceneryContent>(id).expect("typed root").sync(snap);
		ui.layout_in(body, chrome.theme(), chrome.fonts());
		(chrome, ui, id)
	}

	fn grid_of(ui: &Ui, id: WidgetId) -> &SceneryGrid {
		let content = ui.get::<SceneryContent>(id).expect("typed root");
		descendant::<SceneryGrid>(&content.root, content.grid).expect("the content widget")
	}

	fn press(pressed: bool, at: Vec2) -> Event {
		Event::PointerButton { button: PointerButton::Primary, pressed, pos: at, mods: Modifiers::NONE }
	}

	/// The flat index space is dense, ordered by library then by piece, and
	/// `index_of` is its exact inverse - the panel's ring, the armed tool and a
	/// grabbed placement all have to agree on which piece a number means.
	#[test]
	fn the_flat_index_round_trips_through_index_of() {
		let p = green();
		let n = piece_count(&p);
		assert!(n > 0, "the GREEN project loads a scenery library");
		assert!(piece_at(&p, n).is_none(), "one past the end resolves to nothing");
		for i in 0..n {
			let (pack, piece) = piece_at(&p, i).expect("every index in range resolves");
			assert_eq!(index_of(&p, pack, &piece.id), Some(i), "piece {} round-trips", piece.id);
		}
		assert_eq!(index_of(&p, "GREEN", "no-such-piece"), None);
		assert_eq!(index_of(&p, "NOPACK", "mountain-1"), None);
	}

	/// A tag decodes to the action that made it, and a tag from another panel
	/// (or a stray zero) decodes to nothing.
	#[test]
	fn action_tags_decode_to_themselves() {
		assert_eq!(action_of(tag(KIND_PICK, 17)), Some(Action::Pick(17)));
		assert_eq!(action_of(tag(KIND_SIZE, 2)), Some(Action::SizeOption(2)));
		assert_eq!(action_of(tag(KIND_SIZE, PREVIEW_SIZES.len())), None, "an option past the table is nobody's");
		assert_eq!(action_of(tag(KIND_PACK, 0)), Some(Action::PackOption(0)));
		assert_eq!(action_of(0), None, "a stray zero tag is nobody's");
		assert_eq!(action_of(tag(9, 0)), None, "another panel's kind is nobody's");
		for (i, &(label, action, _)) in COMMANDS.iter().enumerate() {
			assert_eq!(action_of(tag(KIND_CMD, i)), Some(action), "{label}");
		}
		assert_eq!(action_of(tag(KIND_CMD, COMMANDS.len())), None, "a key past the table is nothing");
	}

	/// Every header key is a distinct verb, and the row the tag indexes is the
	/// row the button was built from - an off-by-one here would silently wire
	/// "delete" to rename.
	#[test]
	fn the_header_keys_are_distinct_and_in_build_order() {
		let labels: Vec<&str> = COMMANDS.iter().map(|&(l, ..)| l).collect();
		assert_eq!(labels, ["new", "import", "clone", "edit", "rename", "export", "delete"]);
		let actions: Vec<Action> = COMMANDS.iter().map(|&(_, a, _)| a).collect();
		assert_eq!(
			actions,
			[Action::New, Action::Import, Action::Clone, Action::Edit, Action::Rename, Action::Export, Action::Delete],
			"the tag payload is the row index, so build order is the contract",
		);
	}

	/// **The read-only rule, as the panel shows it.** With nothing armed only
	/// the two authoring keys are live; a shipped piece adds `clone` and
	/// `export` but not `edit` / `rename` / `delete`; the user's own adds all of
	/// them; and `--dev` unlocks a shipped one entirely.
	#[test]
	fn a_shipped_piece_offers_clone_but_not_edit() {
		let p = green();
		let live = |snap: &Snapshot| -> Vec<&str> {
			COMMANDS.iter().enumerate().filter(|(i, _)| snap.key_unmet(*i).is_none()).map(|(_, &(l, ..))| l).collect()
		};
		let nothing = Snapshot::of(&p, None, DEFAULT_PREVIEW, None, false);
		assert_eq!(live(&nothing), ["new", "import"], "nothing armed, nothing to act on");
		assert_eq!(nothing.key_unmet(3), Some("needs an armed piece"), "the reason rides the dead key as a tooltip");

		// Every shipped piece is read-only, and the GREEN library is all shipped.
		let shipped = Snapshot::of(&p, Some(0), DEFAULT_PREVIEW, None, false);
		assert_eq!(live(&shipped), ["new", "import", "clone", "export"]);
		assert_eq!(shipped.key_unmet(3), Some("shipped cut-outs are read-only - clone it"), "armed but not editable");
		assert_eq!(shipped.key_unmet(2), None, "clone is live, so no tooltip");
		let dev = Snapshot::of(&p, Some(0), DEFAULT_PREVIEW, None, true);
		assert_eq!(live(&dev), ["new", "import", "clone", "edit", "rename", "export", "delete"], "--dev unlocks it");

		// The user's own piece is editable without --dev. Faked by flipping the
		// flag the merge derives from where a library loaded.
		let mut mine = p;
		mine.scenery_packs[0].pieces[0].user = true;
		let snap = Snapshot::of(&mine, Some(0), DEFAULT_PREVIEW, None, false);
		assert_eq!(live(&snap), ["new", "import", "clone", "edit", "rename", "export", "delete"]);

		// And the greying is real, not decorative: a press on a key that needs an
		// editable piece produces nothing at all.
		let body = Rect::new(0.0, 0.0, 700.0, 300.0);
		let (_chrome, mut ui, id) = hosted(body, shipped);
		let edit_key = ui.get::<SceneryContent>(id).expect("typed root").keys[3];
		assert_eq!(
			wgpu_ui::Widget::tooltip(ui.get::<wgpu_ui::Button>(edit_key).expect("a command key")),
			Some("shipped cut-outs are read-only - clone it"),
			"the dead key carries the reason as its tooltip"
		);
		let keys = ui.get::<SceneryContent>(id).expect("typed root").keys;
		for (i, &(label, action, need)) in COMMANDS.iter().enumerate() {
			let at = ui.rect_of(keys[i]).expect("the key is arranged").center();
			ui.dispatch(&[press(true, at), press(false, at)]);
			let fired = ui.actions().iter().copied().find_map(action_of);
			match need {
				Need::Editable => assert_eq!(fired, None, "{label}: dead on a shipped piece"),
				_ => assert_eq!(fired, Some(action), "{label}: live"),
			}
		}
	}

	/// The header filter narrows the **visible** list without touching the flat
	/// one, and the ring follows the armed flat index into it - or goes out when
	/// the filter hides the armed piece.
	#[test]
	fn the_pack_filter_maps_between_the_two_index_spaces() {
		let p = green();
		let n = piece_count(&p);
		assert_eq!(visible_pieces(&p, None).len(), n, "no filter lists everything");
		let one = visible_pieces(&p, Some("GREEN"));
		assert_eq!(one.len(), n, "one library, so it is all of it");
		assert_eq!(one.iter().copied().collect::<std::collections::BTreeSet<_>>().len(), n, "each flat index once");
		assert!(visible_pieces(&p, Some("NOPACK")).is_empty(), "a pack no library answers to lists nothing");

		let all = Snapshot::of(&p, Some(3), DEFAULT_PREVIEW, None, false);
		let at3 = all.items[3].flat;
		assert_eq!(
			all.active,
			visible_pieces(&p, None).iter().position(|&v| v == 3),
			"the armed piece rings at its visible position"
		);
		assert_eq!(all.items.len(), n);
		assert_eq!(all.pack_sel, None);
		assert_eq!(all.items[3].name, piece_at(&p, at3).expect("the flat piece row 3 lists").1.name);

		let hidden = Snapshot::of(&p, Some(3), DEFAULT_PREVIEW, Some("NOPACK"), false);
		assert_eq!(hidden.active, None, "a filter that hides the armed piece rings nothing");
		let stale = Snapshot::of(&p, Some(usize::MAX), DEFAULT_PREVIEW, None, false);
		assert_eq!(stale.active, None, "an index past the library rings nothing");
	}

	/// Names sort the way a person reads them: the number in "Mountain 2" is a
	/// number, not two characters that happen to be `< '1','0'`.
	#[test]
	fn names_sort_number_aware() {
		use std::cmp::Ordering;
		let mut v = ["Mountain 10", "Mountain 2", "Trees 1", "Mountain 1", "Trees 10", "Mountain 20"];
		v.sort_by(|a, b| natural_cmp(a, b));
		assert_eq!(v, ["Mountain 1", "Mountain 2", "Mountain 10", "Mountain 20", "Trees 1", "Trees 10"]);

		assert_eq!(natural_cmp("Rock 9", "Rock 10"), Ordering::Less, "9 < 10, the whole point");
		assert_eq!(natural_cmp("Rock 2", "Rock 2"), Ordering::Equal);
		// Leading zeros do not change the value, but they do break the tie, so
		// equal numbers written differently land adjacent rather than either far
		// apart or (worse) comparing equal and sorting arbitrarily.
		assert_eq!(natural_cmp("Rock 07", "Rock 8"), Ordering::Less);
		assert_eq!(natural_cmp("Rock 7", "Rock 07"), Ordering::Less, "narrower first");
		// A number run of any length is exact - nothing is parsed, so nothing
		// overflows.
		assert_eq!(natural_cmp("a99999999999999999999999", "a99999999999999999999998"), Ordering::Greater);
		// Case is secondary, and only the exact string can settle a full tie.
		assert_eq!(natural_cmp("rock 3", "Rock 4"), Ordering::Less, "case does not outrank the number");
		assert_ne!(natural_cmp("Rock", "rock"), Ordering::Equal, "but a total order all the same");
		// Digit vs non-digit, and a prefix, both fall through to a plain compare.
		assert_eq!(natural_cmp("Rock 3", "Rock x"), Ordering::Less);
		assert_eq!(natural_cmp("Rock", "Rock 1"), Ordering::Less);
	}

	/// The grid lists by name, not in the order the bake happened to write the
	/// manifest in - and every flat index still appears exactly once.
	#[test]
	fn the_grid_lists_pieces_in_natural_name_order() {
		let p = green();
		let visible = visible_pieces(&p, None);
		let names: Vec<&str> = visible.iter().map(|&i| piece_at(&p, i).expect("in range").1.name.as_str()).collect();
		assert!(names.len() > 10, "the GREEN library is worth sorting");
		assert!(
			names.windows(2).all(|w| natural_cmp(w[0], w[1]) != std::cmp::Ordering::Greater),
			"listed in natural name order: {names:?}"
		);
		assert!(
			names.windows(2).any(|w| w[0].cmp(w[1]) == std::cmp::Ordering::Greater),
			"and that is genuinely not the ASCII order the manifest is written in"
		);
	}

	/// **The scroll range is the thing the panel lives or dies by.** A cell now
	/// carries a name strip and the size is the user's, so a library that used
	/// to fit in three fixed rows scrolls properly at every dock shape - and the
	/// wheel moves it, notch by notch, all the way to the end.
	#[test]
	fn the_grid_scrolls_through_the_whole_library_at_every_size() {
		let p = green();
		// A wide, short dock - the shape that made the old fixed 68px cell
		// degenerate into ~1.5 wheel notches of travel.
		let wide = Rect::new(0.0, 0.0, 1280.0, 171.0);
		let (chrome, mut ui, id) = hosted(wide, Snapshot::of(&p, None, DEFAULT_PREVIEW, None, false));
		assert!(grid_of(&ui, id).scroller.has_bar(), "the library overflows a 171px dock");

		let wheel =
			|at: Vec2| Event::Scroll { delta: ScrollDelta::Lines(Vec2::new(0.0, 1.0)), pos: at, mods: Modifiers::NONE };
		let at = grid_of(&ui, id).rect.center();
		let mut last = grid_of(&ui, id).scroller.offset();
		let mut notches = 0;
		while notches < 200 {
			ui.dispatch(&[wheel(at)]);
			// The panel re-lays out every frame; the offset has to survive it.
			ui.layout_in(wide, chrome.theme(), chrome.fonts());
			let now = grid_of(&ui, id).scroller.offset();
			if now == last {
				break;
			}
			last = now;
			notches += 1;
		}
		let g = grid_of(&ui, id);
		assert!(notches >= 3, "the wheel has real travel, not one notch: {notches}");
		assert_eq!(last, g.scroller.max_offset(), "and it reaches the end");
		// The last row is fully on screen there.
		let last_row = g.item_rect(g.snap.items.len() - 1);
		assert!(last_row.bottom() + NAME_H <= g.rect.bottom() + 0.5, "the last row lands inside the window");

		// Bigger cells, more rows, more travel - and it still terminates at the
		// end rather than running past it.
		let (_chrome, mut ui, id) = hosted(wide, Snapshot::of(&p, None, 192.0, None, false));
		let at = grid_of(&ui, id).rect.center();
		for _ in 0..500 {
			ui.dispatch(&[wheel(at)]);
		}
		let g = grid_of(&ui, id);
		assert_eq!(g.scroller.offset(), g.scroller.max_offset());
		assert!(g.scroller.max_offset() > 0.0, "192px cells overflow a 171px dock by a lot");
		drop(chrome);
	}

	/// **Each dropdown reports its own kind.** They are drained by position in
	/// `selects`, so a catch-all mapping files one box's picks under another's
	/// verb - which is exactly what the blend dropdown did when it arrived (its
	/// picks resized the thumbnails).
	#[test]
	fn each_dropdown_reports_its_own_kind() {
		let p = green();
		let body = Rect::new(0.0, 0.0, 700.0, 300.0);
		let (_chrome, mut ui, id) = hosted(body, Snapshot::of(&p, None, DEFAULT_PREVIEW, None, false));
		// The last option of each box, so the payload is unambiguous too.
		let wants = [
			Action::PackOption(p.scenery_packs.len()),
			Action::SizeOption(PREVIEW_SIZES.len() - 1),
			Action::BlendOption(SceneryBlend::ALL.len() - 1),
		];
		for (k, want) in wants.into_iter().enumerate() {
			let sel_id = ui.get::<SceneryContent>(id).expect("typed root").selects[k];
			let box_r = ui.rect_of(sel_id).expect("the box is arranged");
			ui.dispatch(&[press(true, box_r.center())]);
			assert!(ui.popup_open(), "box {k} opens its list");
			let popup = ui.get::<Select>(sel_id).expect("typed").popup_rect();
			ui.dispatch(&[press(true, Vec2::new(popup.x + 2.0, popup.bottom() - 2.0))]);
			assert_eq!(action_of(ui.actions()[0]), Some(want), "box {k} picked its own last option");
		}
	}

	/// Every header key fires its own verb on a real press/release, at the
	/// position the `Wrap` actually put it - the one thing the tag-table test
	/// above cannot check, because it never lays the band out.
	#[test]
	fn each_header_key_fires_its_own_verb() {
		let mut p = green();
		// Armed, and the user's own - otherwise the keys that act on a piece are
		// greyed out and a press on one is not a press on anything.
		p.scenery_packs[0].pieces[0].user = true;
		// Wide enough that the whole command run fits on the first line.
		let body = Rect::new(0.0, 0.0, 700.0, 300.0);
		let (_chrome, mut ui, id) = hosted(body, Snapshot::of(&p, Some(0), DEFAULT_PREVIEW, None, false));
		let keys = ui.get::<SceneryContent>(id).expect("typed root").keys;
		for (i, &(label, action, _)) in COMMANDS.iter().enumerate() {
			let at = ui.rect_of(keys[i]).expect("the key is arranged").center();
			ui.dispatch(&[press(true, at)]);
			assert!(ui.actions().iter().copied().find_map(action_of).is_none(), "{label}: a press only arms");
			ui.dispatch(&[press(false, at)]);
			assert_eq!(ui.actions().iter().copied().find_map(action_of), Some(action), "{label}");
		}
	}

	/// A cell arms on the press and fires its **visible** index on the release,
	/// with the name strip part of its target.
	#[test]
	fn a_cell_fires_its_visible_index_on_release_inside() {
		let p = green();
		// Tall enough that row three is on screen under a header the seven keys
		// wrap onto several runs at this width.
		let body = Rect::new(0.0, 0.0, 300.0, 600.0);
		let (_chrome, mut ui, id) = hosted(body, Snapshot::of(&p, None, DEFAULT_PREVIEW, None, false));
		for i in [0usize, 1, 5] {
			let r = grid_of(&ui, id).item_rect(i);
			// Aim at the name strip: it is part of the cell's target.
			let at = Vec2::new(r.x + r.w * 0.5, r.y + r.h + NAME_H * 0.5);
			ui.dispatch(&[press(true, at)]);
			assert!(ui.actions().is_empty(), "cell {i}: a press only arms");
			ui.dispatch(&[press(false, at)]);
			assert_eq!(ui.actions().iter().copied().find_map(action_of), Some(Action::Pick(i)), "cell {i}");
		}
	}
}
