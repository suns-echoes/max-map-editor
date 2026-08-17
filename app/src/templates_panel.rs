//! Templates Explorer dockable: a picker-style grid of tile templates -
//! shipped stock ones and the user's own - shown as static composed
//! thumbnails (rest palette) uv'd from the shell's template atlas. Clicking a
//! template arms it as the ghost stamp under the cursor. The header holds Save
//! (selection → template), Import, Delete, Duplicates (remove exact duplicates),
//! Rename, Explore, a tileset filter and a preview-size dropdown, plus the
//! visible-template count.
//!
//! **The panel is a real `wgpu-ui` widget tree** (U5.5): a [`wgpu_ui::Linear`]
//! column of a header [`wgpu_ui::Wrap`] — six [`wgpu_ui::Button`] keys, the two
//! hosted [`wgpu_ui::Select`]s U3.6 gave it, and the count [`wgpu_ui::Label`] —
//! over a [`TemplatesGrid`] **content widget** that owns the thumbnail geometry,
//! its own [`Scroller`], the atlas uv draw, the name strips and the cell pick.
//! There is no hit oracle, no panel-wide `ArmFire` and no `Hot`: hover, arming
//! and fire are each key's own, and everything the panel produces comes back as
//! an **action tag** polled off `Ui::actions` — [`action_of`] maps it back to an
//! [`Action`] the shell turns into a command.

use wgpu_ui::widget::{DrawCtx, EventCtx, LayoutCtx, Widget};
use wgpu_ui::{
	ArmFire, CrossAlign, DrawList, Emboss, Event, Insets, Label, Length, Linear, PageKeys, Scroller, Select, Size,
	TextRole, Vec2, WidgetId, WidgetState, Wrap, descendant, descendant_mut,
};

use crate::state::TemplateEntry;
use crate::theme;
use crate::ui::Rect;
use crate::uikit_theme::rgba;

/// Height of one header key.
const BTN_H: f32 = 18.0;
/// Gap between header controls, between its runs, and below the last run.
const HDR_GAP: f32 = 4.0;
/// The header band's margins. A run is [`BTN_H`] + [`HDR_GAP`] = the 22px row
/// the hand-flowed header always drew, so 2px above the first run and a
/// [`HDR_GAP`] gutter below the last reproduce its exact height — and the grid
/// below it does not move by so much as a pixel (U5.5).
const HDR_PAD: Insets = Insets { left: 2.0, top: 2.0, right: 2.0, bottom: HDR_GAP };
/// Inner padding of the thumbnail grid.
const PAD: f32 = 4.0;
/// Gap between thumbnails (and rows).
const GAP: f32 = 4.0;
/// The name strip under each thumbnail.
const NAME_H: f32 = 14.0;
/// Below this preview size the WxH badge has no room to stay legible.
const BADGE_MIN_CELL: f32 = 64.0;

/// Thumbnail sizes the preview-size dropdown offers, with their labels (very
/// small 32 .. very large 128). The shell stores the chosen px in
/// `EditorState::templates_cell`.
pub const PREVIEW_SIZES: [(f32, &str); 5] =
	[(32.0, "very small"), (48.0, "small"), (64.0, "medium"), (96.0, "large"), (128.0, "very large")];

/// What a fired action tag resolved to. `Pick` carries the index into the
/// **visible** (pack-compatible) list the view was built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
	Pick(usize),
	Save,
	Import,
	Delete,
	/// Remove duplicates (opens the modal).
	Dedupe,
	/// Rename the selected template (opens the modal).
	Rename,
	/// Open the user-templates folder in the OS file manager.
	Explore,
	/// Pick preview-size option `i` (index into [`PREVIEW_SIZES`]) — the hosted
	/// dropdown's commit; opening and dismissing are its own (U3.6).
	SizeOption(usize),
	/// Pick tileset option `i` (0 = all, else label `i-1`).
	TilesetOption(usize),
}

/// What a header key needs before it does anything (the shared header-key
/// convention, [`crate::panel_ui`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Need {
	/// Nothing - the key acts on the map selection, the whole list, or the
	/// filesystem, none of which this panel's selection gates.
	Always,
	/// A selected template to act on.
	Selected,
}

/// The header's command keys, in flow order: label, the [`Action`] the key
/// fires and what it needs. A [`Wrap`] sizes every child to what it
/// *measures*, so each key pins its own compact width with `Button::sized`
/// rather than leaving it to the theme's dialog-button minimum — measured from
/// its label at `arrange` (picker's G29/U7.1 rule; these were hand-pinned
/// magic numbers before 2026-08-11). The two dropdowns carry none: a `Select`
/// measures to its own widest option, which is why U3.6 made them dropdowns in
/// the first place. A key whose need fails reads disabled-dead with the reason
/// as its tooltip; the stock-template refusal stays the command's, reported
/// loudly in the console.
const COMMANDS: [(&str, Action, Need); 6] = [
	("save", Action::Save, Need::Always),
	("import", Action::Import, Need::Always),
	("delete", Action::Delete, Need::Selected),
	("duplicates", Action::Dedupe, Need::Always),
	("rename", Action::Rename, Need::Selected),
	("explore", Action::Explore, Need::Always),
];

/// The tag space: a kind in the high bits over a 32-bit payload, so one
/// `Ui::actions` poll answers for the whole panel (U5.4's shape). Kind `0` is
/// deliberately unused — a stray zero tag resolves to nothing.
const KIND_SHIFT: u32 = 32;
/// A header command key: the payload is its row in [`COMMANDS`].
const KIND_CMD: u64 = 1;
/// A preview-size pick: the payload is its index into [`PREVIEW_SIZES`].
const KIND_SIZE: u64 = 2;
/// A tileset-filter pick: the payload is `0` = all, else label `i-1`.
const KIND_TILESET: u64 = 3;
/// A thumbnail pick: the payload is its index into the **visible** list.
const KIND_PICK: u64 = 4;

const fn tag(kind: u64, i: usize) -> u64 {
	(kind << KIND_SHIFT) | i as u64
}

/// The templates action a fired tag stands for, or `None` if it is not one of
/// this panel's (the shell polls every tag its `Ui` collected).
pub fn action_of(tag: u64) -> Option<Action> {
	let i = (tag & 0xffff_ffff) as usize;
	match tag >> KIND_SHIFT {
		KIND_CMD => COMMANDS.get(i).map(|&(_, action, _)| action),
		KIND_SIZE => (i < PREVIEW_SIZES.len()).then_some(Action::SizeOption(i)),
		// The tileset list is per-map, so its range is the shell's to check
		// (an index past the labels reads as "all", exactly like a stale name).
		KIND_TILESET => Some(Action::TilesetOption(i)),
		KIND_PICK => Some(Action::Pick(i)),
		_ => None,
	}
}

/// Per-template thumbnail geometry in the shell's template atlas: the
/// texture, the atlas grid (cells per row + total rows), and each entry's
/// thumb size as a fraction of its cell (aspect-fit, anchored top-left).
pub struct ThumbAtlas<'a> {
	pub tex: wgpu_ui::TextureId,
	pub cols: u32,
	pub rows: u32,
	pub fracs: &'a [(f32, f32)],
}

/// The atlas handle a [`Snapshot`] carries — [`ThumbAtlas`] minus the borrowed
/// fracs, which are resolved per entry into [`Item::frac`].
#[derive(Clone, Copy)]
struct Atlas {
	tex: wgpu_ui::TextureId,
	cols: u32,
	rows: u32,
}

/// One visible template, as the grid draws it.
#[derive(Clone)]
struct Item {
	name: String,
	/// A stock (shipped) entry dims its name — it cannot be deleted.
	stock: bool,
	/// The template's footprint, for the WxH badge.
	dims: (u16, u16),
	/// The entry's cell in the shell's thumbnail atlas...
	slot: usize,
	/// ...and its thumb size as a fraction of that cell.
	frac: (f32, f32),
}

/// The panel state one frame reflects: the header's three scalars and the
/// grid's visible-entry list. Pushed into the retained tree by
/// [`TemplatesContent::sync`].
#[derive(Clone)]
pub struct Snapshot {
	/// Preview cell px (drives the size-dropdown label + selected option).
	cell: f32,
	/// The selected tileset, resolved to an index into `tilesets` (None = all).
	tileset_sel: Option<usize>,
	/// The tileset filter option labels (the box value + popup rows).
	tilesets: Vec<String>,
	/// The visible (pack-compatible) entries, in grid order.
	items: Vec<Item>,
	/// The composed thumbnail atlas — `None` before the shell has built one, in
	/// which case the wells draw empty.
	atlas: Option<Atlas>,
	/// The selected entry, as an index into `items`.
	selected: Option<usize>,
}

impl Snapshot {
	/// Snapshot the templates-relevant editor state for one frame's draw.
	/// `entries` is the visible list and `slots` the matching global template
	/// indices — which are also the entries' cells in the thumbnail atlas.
	pub fn of(
		entries: &[&TemplateEntry],
		slots: &[usize],
		atlas: Option<&ThumbAtlas>,
		selected: Option<usize>,
		cell: f32,
		tileset_sel: Option<usize>,
		tilesets: Vec<String>,
	) -> Self {
		let items = entries
			.iter()
			.zip(slots)
			.map(|(e, &slot)| Item {
				name: e.name.clone(),
				stock: e.stock,
				dims: (e.template.width, e.template.height),
				slot,
				frac: atlas.and_then(|a| a.fracs.get(slot).copied()).unwrap_or((0.0, 0.0)),
			})
			.collect();
		Self {
			cell,
			tileset_sel,
			tilesets,
			items,
			atlas: atlas.map(|a| Atlas { tex: a.tex, cols: a.cols, rows: a.rows }),
			selected,
		}
	}

	fn empty() -> Self {
		Self { cell: 64.0, tileset_sel: None, tilesets: Vec::new(), items: Vec::new(), atlas: None, selected: None }
	}
}

/// The cell under `p` in `grid`, scrolled to `offset`, out of `count` entries —
/// the grid's domain hit oracle. A thumbnail's target includes its name strip;
/// the padding, the gaps between cells and the run past the last entry belong to
/// nobody, exactly as the panel's old `click` oracle had them.
///
/// Free rather than a method so [`TemplatesGrid`] can hand it to its own
/// `ArmFire` without borrowing itself immutably and mutably at once.
fn cell_at(grid: &crate::cellgrid::Grid, offset: f32, count: usize, p: Vec2) -> Option<usize> {
	if !grid.body.contains(p) {
		return None;
	}
	let i = grid.index_at(p.x, p.y, offset)?;
	let r = grid.item_rect(i, offset);
	(i < count && Rect::new(r.x, r.y, r.w, r.h + NAME_H).contains(p)).then_some(i)
}

/// The templates explorer's **content widget**: the scrolling thumbnail grid.
///
/// It owns exactly what §5.2 allows a content widget to own — the
/// [`crate::cellgrid::Grid`] geometry, the atlas uv draw, its own [`Scroller`]
/// and the domain cell pick — and no chrome: the command keys, both dropdowns
/// and the count are its **siblings** in the panel tree, never its children.
///
/// Arranged straight *into* its viewport (not into a tall content rect), which
/// is what keeps G7 deferred: the widget clips its own draw and scrolls the
/// rows through it.
pub struct TemplatesGrid {
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
}

impl TemplatesGrid {
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

	/// The cell geometry over this widget's own arranged rect. The grid *is* the
	/// viewport now, so it carries no header offset — the header band is the
	/// sibling above it.
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

	/// Slot `i`'s thumbnail box at the current scroll (the name strip hangs
	/// below it).
	fn item_rect(&self, i: usize) -> Rect {
		self.grid().item_rect(i, self.scroller.offset())
	}

	/// The cell under `p`, if any — the domain hit oracle.
	fn cell_at(&self, p: Vec2) -> Option<usize> {
		cell_at(&self.grid(), self.scroller.offset(), self.snap.items.len(), p)
	}
}

impl Widget for TemplatesGrid {
	fn measure(&mut self, avail: Size, _ctx: &mut LayoutCtx) -> Size {
		avail
	}

	fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
		self.rect = rect;
		self.gutter = ctx.theme.metrics().scrollbar;
		let content = self.grid().content_height(self.snap.items.len());
		self.scroller.layout(ctx, rect, content);
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		let (skin, fonts) = (ctx.theme, ctx.fonts);
		// Everything scrolls with the grid; clip it all to its own rect.
		dl.push_clip(self.rect);
		if self.snap.items.is_empty() {
			// The "no templates" note lives in the clipped grid layer so it can't
			// spill past a short panel.
			skin.text_wrapped(
				dl,
				fonts,
				Rect::new(self.rect.x, self.rect.y + 4.0, self.rect.w, self.rect.h),
				PAD,
				"no templates match this map's tile packs - select tiles and press save",
				TextRole::Small,
				Emboss::Engraved,
				rgba(theme::INK_DIM),
			);
			dl.pop_clip();
			return;
		}
		// A cell rings dim under the pointer, gated on the `Ui` agreeing that this
		// widget is hovered at all. That gate is what makes an open header
		// dropdown inert the grid *for free* — the `Ui` collapses hover to the
		// popup's owner — so the shell no longer reaches in to do it.
		let hovered = ctx.is_hovered(self.id).then_some(self.hover).flatten();
		let cell = self.snap.cell;
		for (i, item) in self.snap.items.iter().enumerate() {
			let r = self.item_rect(i);
			if r.bottom() + NAME_H < self.rect.y || r.y > self.rect.bottom() {
				continue;
			}
			// The thumbnail well, then the composed still aspect-fit into it
			// (uv'd from the entry's atlas cell — same centring the tile pass had).
			skin.well(dl, r, WidgetState::default());
			if let Some(atlas) = self.snap.atlas {
				let (fw, fh) = item.frac;
				let span = fw.max(fh).max(f32::MIN_POSITIVE);
				let (tw, th) = ((cell - 4.0) * fw / span, (cell - 4.0) * fh / span);
				let target =
					Rect::new(r.x + 2.0 + (cell - 4.0 - tw) / 2.0, r.y + 2.0 + (cell - 4.0 - th) / 2.0, tw, th);
				let slot = item.slot as u32;
				let (cx, cy) = ((slot % atlas.cols) as f32, (slot / atlas.cols) as f32);
				let (fc, fr) = (atlas.cols as f32, atlas.rows.max(1) as f32);
				let uv = wgpu_ui::TexRect::new(cx / fc, cy / fr, (cx + fw) / fc, (cy + fh) / fr);
				dl.image(atlas.tex, target, uv, wgpu_ui::Rgba::WHITE);
			}
			// Selection / hover ring + name (clipped to the grid).
			let ring = Rect::new(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0);
			if self.snap.selected == Some(i) {
				dl.stroke_rect(ring, 1.0, rgba(theme::ACCENT));
			} else if hovered == Some(i) {
				dl.stroke_rect(ring, 1.0, rgba(theme::INK_DIM));
			}
			let ink = if item.stock { theme::INK_DIM } else { theme::INK };
			let name_rect = Rect::new(r.x, r.y + r.h, r.w, NAME_H);
			skin.text_fit(dl, fonts, name_rect, 1.0, &item.name, TextRole::Small, Emboss::Engraved, rgba(ink));
			// A small WxH badge in the thumbnail's top-right - only when the preview
			// is medium or larger, so it stays legible (and has room).
			if cell >= BADGE_MIN_CELL {
				let (tw, th) = item.dims;
				let span = tw.max(th).max(1) as f32;
				let px = (cell - 4.0) / span;
				let dims = format!("{tw}x{th}");
				let w = fonts.get(skin.font()).measure(&dims, px);
				skin.text_top(
					dl,
					fonts,
					Vec2::new(r.x + r.w - w - 3.0, r.y + 3.0),
					&dims,
					TextRole::Small,
					Emboss::Raised,
					rgba(theme::INK),
				);
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

	/// The thumbnails and the scrollbar column claim the pointer; the padding
	/// and the gaps between cells stay inert, exactly as the old `click` oracle
	/// had them. The bar has to be claimed explicitly — [`Scroller`] only takes
	/// a press when its owner is the dispatch target.
	fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
		let bar = self.scroller.has_bar() && self.scroller.track_rect().contains(pos);
		(bar || self.cell_at(pos).is_some()).then_some(self.id)
	}
}

/// The Templates Explorer as a retained `wgpu_ui` [`Widget`]: a thin root over a
/// `Linear` column of the header flow and the [`TemplatesGrid`]. It exists to
/// hold the id tables and to push the per-frame [`Snapshot`] into them;
/// everything else — layout, paint, hover, arming, firing, scrolling, the
/// dropdowns' popup layer — is the tree's.
pub struct TemplatesContent {
	id: WidgetId,
	root: Linear,
	/// The header's command keys, in [`COMMANDS`] order - re-synced per frame
	/// against the selection (the shared header-key convention).
	keys: [WidgetId; COMMANDS.len()],
	/// The tileset-filter and preview-size dropdowns, in flow order (U3.6).
	/// Each owns its open state, dismissal, keyboard and popup placement.
	selects: [WidgetId; 2],
	/// The visible-template count readout.
	count: WidgetId,
	grid: WidgetId,
	rect: Rect,
}

impl Default for TemplatesContent {
	fn default() -> Self {
		Self::new()
	}
}

impl TemplatesContent {
	pub fn new() -> Self {
		// The header flows onto as many runs as it needs and the grid takes the
		// rest — `Length::Fit` measures the `Wrap` at the panel's width, so the
		// band is exactly as tall as the runs it produced.
		// `run_extent` is what makes a run a **row**: without it a run carrying
		// only the count `Label` would be that label's own height, and the band
		// — which the whole grid hangs off — would lose a pixel at exactly the
		// dock widths where the flow packs that way.
		let mut header = Wrap::row()
			.padding(HDR_PAD)
			.spacing(HDR_GAP)
			.run_spacing(HDR_GAP)
			.run_extent(BTN_H)
			.line_align(CrossAlign::Center);
		// The keys' widths are their labels', and only a `LayoutCtx` knows how
		// wide those are — `arrange` pins them (G29).
		let mut keys = [WidgetId::NONE; COMMANDS.len()];
		for (i, &(label, ..)) in COMMANDS.iter().enumerate() {
			let key = wgpu_ui::Button::new(label).small().sized(0.0, BTN_H).action(tag(KIND_CMD, i));
			keys[i] = key.id();
			header = header.push(key);
		}
		let (tileset, size) = (Select::new(Vec::<String>::new()).small(), Select::new(Vec::<String>::new()).small());
		let selects = [tileset.id(), size.id()];
		let count = Label::new("0").small().muted().with_id();
		let count_id = count.id();
		header = header.push(tileset).push(size).push(count);

		let grid = TemplatesGrid::new();
		let grid_id = grid.id();
		let root = Linear::column().child(header, Length::Fit).child(grid, Length::Flex(1.0));
		Self { id: wgpu_ui::next_id(), root, keys, selects, count: count_id, grid: grid_id, rect: Rect::ZERO }
	}

	/// Push one frame's state into the retained tree: the two dropdowns' options
	/// and values, the count, and the grid's visible entries. The tileset list is
	/// per-map, so it is rebuilt each frame (an open list survives
	/// `Select::set_options`).
	pub fn sync(&mut self, snap: Snapshot) {
		// A key whose need fails greys out dead, with the reason as its tooltip
		// (the shared header-key convention, [`crate::panel_ui`]). The command
		// behind it still validates loudly - scripts reach it directly.
		let unmet = |need: Need| match need {
			Need::Always => None,
			Need::Selected => snap.selected.is_none().then_some("needs a selected template"),
		};
		for (i, &(_, _, need)) in COMMANDS.iter().enumerate() {
			if let Some(key) = descendant_mut::<wgpu_ui::Button>(&mut self.root, self.keys[i]) {
				crate::panel_ui::sync_header_key(key, unmet(need));
			}
		}
		if let Some(sel) = descendant_mut::<Select>(&mut self.root, self.selects[0]) {
			let mut tilesets = Vec::with_capacity(snap.tilesets.len() + 1);
			tilesets.push("all".to_string());
			tilesets.extend(snap.tilesets.iter().cloned());
			sel.set_options(tilesets);
			sel.set_selected(snap.tileset_sel.map_or(0, |i| i + 1));
		}
		if let Some(sel) = descendant_mut::<Select>(&mut self.root, self.selects[1]) {
			sel.set_options(PREVIEW_SIZES.iter().map(|&(_, name)| name));
			sel.set_selected(PREVIEW_SIZES.iter().position(|&(px, _)| px == snap.cell).unwrap_or(0));
		}
		if let Some(label) = descendant_mut::<Label>(&mut self.root, self.count) {
			label.set_text(snap.items.len().to_string());
		}
		if let Some(grid) = descendant_mut::<TemplatesGrid>(&mut self.root, self.grid) {
			grid.snap = snap;
		}
	}

	/// Back to the top — the tileset filter re-lists the grid, so the old offset
	/// would land on unrelated thumbnails.
	pub fn scroll_to_top(&mut self) {
		if let Some(grid) = descendant_mut::<TemplatesGrid>(&mut self.root, self.grid) {
			grid.scroller.set_offset(0.0);
		}
	}

	/// The visible-list index of the thumbnail under `pos`, if any — the grid's
	/// own domain hit test, which the shell asks for the right-click item menu
	/// (the one question a fired action tag cannot answer, because a right-click
	/// fires nothing).
	pub fn template_at(&self, pos: Vec2) -> Option<usize> {
		descendant::<TemplatesGrid>(&self.root, self.grid).and_then(|g| g.cell_at(pos))
	}
}

impl Widget for TemplatesContent {
	fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
		self.rect = rect;
		// Each key's width is its label's, measured through the theme the header
		// draws with — resolved *before* the measure below, or the flow would
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
		// The header's steel band, under the tree: only the root knows how tall
		// the flow wrapped this frame, so — unlike the minimap's fixed band — the
		// shell cannot draw it. Base pass only; the overlay pass carries an open
		// option list out to the shell's popup layer (U3.2).
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
			Some(tag(if i == 0 { KIND_TILESET } else { KIND_SIZE }, v))
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
	use wgpu_ui::{DrawCmd, Modifiers, PointerButton, ScrollDelta, Ui, widget::DrawPass};

	const CELL: f32 = 64.0;

	/// A visible-list entry for the grid tests (`stock` dims the name ink).
	fn entry(name: &str, stock: bool, w: u16, h: u16) -> TemplateEntry {
		TemplateEntry {
			name: name.to_string(),
			path: std::path::PathBuf::new(),
			stock,
			template: map_core::Template {
				name: name.to_string(),
				width: w,
				height: h,
				uses: Vec::new(),
				cells: Vec::new(),
			},
		}
	}

	/// `n` identical 2x2 entries + a full-cell atlas — the populated grid the
	/// pick / scroll / cull tests read.
	fn filled(n: usize, cell: f32, selected: Option<usize>) -> Snapshot {
		let e = entry("t", false, 2, 2);
		let entries: Vec<&TemplateEntry> = (0..n).map(|_| &e).collect();
		let slots: Vec<usize> = (0..n).collect();
		let fracs: Vec<(f32, f32)> = (0..n).map(|_| (1.0, 1.0)).collect();
		let atlas = ThumbAtlas { tex: wgpu_ui::TextureId::ATLAS, cols: 8, rows: 5, fracs: &fracs };
		Snapshot::of(&entries, &slots, Some(&atlas), selected, cell, None, Vec::new())
	}

	/// The chrome fixture + the panel hosted in a `Ui`, laid out into `body`.
	/// A stock `Button` measures its own label, so this needs the real fonts
	/// (`Fonts::new()` + `Gunmetal` panics with "FontId(0) is not registered").
	fn hosted(body: Rect, snap: Snapshot) -> (MenuChrome, Ui, WidgetId) {
		let (_device, _queue, chrome) = crate::visual_test::chrome_fixture();
		let content = TemplatesContent::new();
		let id = content.id();
		let mut ui = Ui::new(content);
		ui.get_mut::<TemplatesContent>(id).expect("typed root").sync(snap);
		ui.layout_in(body, chrome.theme(), chrome.fonts());
		(chrome, ui, id)
	}

	fn press(pressed: bool, at: Vec2) -> Event {
		Event::PointerButton { button: PointerButton::Primary, pressed, pos: at, mods: Modifiers::NONE }
	}

	/// The grid child, borrowed typed off the hosted tree.
	fn grid_of(ui: &Ui, id: WidgetId) -> &TemplatesGrid {
		let content = ui.get::<TemplatesContent>(id).expect("typed root");
		descendant::<TemplatesGrid>(&content.root, content.grid).expect("the content widget")
	}

	/// The header flow (the root column's first child).
	fn header_of(ui: &Ui, id: WidgetId) -> &dyn Widget {
		Widget::child(&ui.get::<TemplatesContent>(id).expect("typed root").root, 0).expect("the header")
	}

	/// The arranged rect of command key `i` — the header flow's `i`th child, in
	/// [`COMMANDS`] order. No id table: the tree's own shape is the mapping.
	fn key_rect(ui: &Ui, id: WidgetId, i: usize) -> Rect {
		Widget::child(header_of(ui, id), i).expect("a command key").rect()
	}

	/// Every header command key fires **its own** table row on a press +
	/// release-inside. That is the whole click path now: no hit oracle, no
	/// panel-wide `ArmFire`, and no action written down twice. A template is
	/// selected so every key's need holds - a dead key is the next test's
	/// subject.
	#[test]
	fn every_command_key_fires_its_own_row() {
		let body = Rect::new(100.0, 50.0, 300.0, 400.0);
		let (_chrome, mut ui, id) = hosted(body, filled(5, CELL, Some(2)));

		for (i, &(label, action, _)) in COMMANDS.iter().enumerate() {
			let at = key_rect(&ui, id, i).center();
			ui.dispatch(&[press(true, at)]);
			assert!(ui.actions().is_empty(), "{label}: a press only arms");
			ui.dispatch(&[press(false, at)]);
			assert_eq!(ui.actions().len(), 1, "{label}: one key, one action");
			assert_eq!(action_of(ui.actions()[0]), Some(action), "{label} resolves to its own row");
		}
	}

	/// `delete` / `rename` need a selected template. Without one they are
	/// **disabled-dead** with the reason as their tooltip - the shared
	/// header-key convention ([`crate::panel_ui`]); this panel's keys used to
	/// be always-hot. The other four keys never grey: what they act on is not
	/// this panel's selection.
	#[test]
	fn the_two_keys_that_need_a_selection_grey_out_dead_without_one() {
		let body = Rect::new(100.0, 50.0, 300.0, 400.0);
		let dead = |ui: &Ui, id: WidgetId| {
			ui.get::<TemplatesContent>(id)
				.expect("typed root")
				.keys
				.map(|k| ui.get::<wgpu_ui::Button>(k).expect("a command key").is_disabled())
		};

		let (_chrome, mut ui, id) = hosted(body, filled(5, CELL, None));
		assert_eq!(dead(&ui, id), [false, false, true, false, true, false], "only delete + rename grey");
		let del_key = ui.get::<TemplatesContent>(id).expect("typed root").keys[2];
		assert_eq!(
			wgpu_ui::Widget::tooltip(ui.get::<wgpu_ui::Button>(del_key).expect("a command key")),
			Some("needs a selected template"),
			"a dead key says why on hover"
		);

		ui.get_mut::<TemplatesContent>(id).expect("typed root").sync(filled(5, CELL, Some(0)));
		assert_eq!(dead(&ui, id), [false; 6], "a selection lights them all");
		assert_eq!(
			wgpu_ui::Widget::tooltip(ui.get::<wgpu_ui::Button>(del_key).expect("a command key")),
			None,
			"a live key carries no tooltip"
		);

		// Disabled is dead, not decorative: the key swallows the click and
		// fires nothing (the command still refuses loudly for the script path -
		// state.rs owns that half).
		let (_chrome, mut ui, id) = hosted(body, filled(5, CELL, None));
		let at = key_rect(&ui, id, 2).center();
		ui.dispatch(&[press(true, at)]);
		ui.dispatch(&[press(false, at)]);
		assert!(ui.actions().is_empty(), "a dead key fires nothing");
	}

	/// Every tag resolves back to what built it, and a tag from nowhere resolves
	/// to nothing — the mapping the shell runs a fired action through.
	#[test]
	fn every_tag_resolves_to_its_own_action() {
		for (i, &(label, action, _)) in COMMANDS.iter().enumerate() {
			assert_eq!(action_of(tag(KIND_CMD, i)), Some(action), "{label}");
		}
		assert_eq!(action_of(tag(KIND_CMD, COMMANDS.len())), None, "a key past the table is nothing");
		assert_eq!(action_of(tag(KIND_SIZE, 2)), Some(Action::SizeOption(2)));
		assert_eq!(action_of(tag(KIND_SIZE, PREVIEW_SIZES.len())), None, "and so is a size past the list");
		assert_eq!(action_of(tag(KIND_TILESET, 0)), Some(Action::TilesetOption(0)));
		assert_eq!(action_of(tag(KIND_PICK, 7)), Some(Action::Pick(7)));
		assert_eq!(action_of(0), None, "the unused kind resolves to nothing");
	}

	/// A thumbnail arms on the press and fires its visible-list index on the
	/// release; its name strip is part of the same target, and the padding
	/// between cells belongs to nobody — exactly as the old oracle had it.
	#[test]
	fn a_thumbnail_and_its_name_strip_pick_and_the_gaps_do_not() {
		let body = Rect::new(100.0, 50.0, 300.0, 400.0);
		let (_chrome, mut ui, id) = hosted(body, filled(5, CELL, None));

		for i in 0..5 {
			let r = grid_of(&ui, id).item_rect(i);
			for (what, at) in
				[("thumb", Vec2::new(r.x + 2.0, r.y + 2.0)), ("name", Vec2::new(r.x + 2.0, r.bottom() + 2.0))]
			{
				ui.dispatch(&[press(true, at)]);
				assert!(ui.actions().is_empty(), "{what} {i}: a press only arms");
				ui.dispatch(&[press(false, at)]);
				assert_eq!(action_of(ui.actions()[0]), Some(Action::Pick(i)), "{what} {i}");
			}
		}

		// The gap between two cells in a row, and the row of thumbnails past the
		// end of the list: both inert.
		let r0 = grid_of(&ui, id).item_rect(0);
		let gap = Vec2::new(r0.right() + GAP / 2.0, r0.center().y);
		let past = grid_of(&ui, id).item_rect(5).center();
		for at in [gap, past] {
			let resp = ui.dispatch(&[press(true, at), press(false, at)]);
			assert!(!resp.wants_pointer(), "{at:?} consumes nothing");
			assert!(ui.actions().is_empty(), "{at:?} picks nothing");
		}
	}

	/// The empty grid explains itself instead of drawing thumbnails.
	#[test]
	fn an_empty_grid_explains_itself() {
		let body = Rect::new(0.0, 0.0, 280.0, 300.0);
		let (chrome, ui, id) = hosted(body, Snapshot::empty());
		let dl = drawn(&chrome, &ui);
		assert_eq!(stills(&dl), 0, "nothing to thumbnail");
		assert!(dl.cmds.iter().any(|c| matches!(c, DrawCmd::Glyph { .. })), "the empty panel explains itself");
		// …and there is nothing to pick, anywhere in the body.
		assert_eq!(grid_of(&ui, id).cell_at(body.center()), None);
	}

	/// The composed stills, the rings and the WxH badges — the grid layer's
	/// paint, now the content widget's own.
	#[test]
	fn the_grid_draws_stills_rings_and_badges() {
		let body = Rect::new(100.0, 50.0, 300.0, 400.0);
		let (a, b) = (entry("stocky", true, 8, 6), entry("mine", false, 3, 3));
		let entries = [&a, &b];
		let fracs = [(1.0f32, 0.75), (0.5, 0.5)];
		let atlas = ThumbAtlas { tex: wgpu_ui::TextureId::ATLAS, cols: 2, rows: 1, fracs: &fracs };
		let snap = |selected, cell| Snapshot::of(&entries, &[0, 1], Some(&atlas), selected, cell, None, Vec::new());

		let (chrome, ui, id) = hosted(body, snap(Some(0), CELL));
		let selected = drawn(&chrome, &ui);
		// One still per entry, aspect-fit inside its cell. The stills are the
		// untinted (WHITE) images; the well's own grain fill behind each is a
		// tinted image, so match on the tint to count just the composed stills.
		let stills: Vec<Rect> = selected
			.cmds
			.iter()
			.filter_map(|c| match c {
				DrawCmd::Image { rect, color, .. } if *color == wgpu_ui::Rgba::WHITE => Some(*rect),
				_ => None,
			})
			.collect();
		assert_eq!(stills.len(), 2, "one composed still per entry");
		for (i, still) in stills.iter().enumerate() {
			let cell = grid_of(&ui, id).item_rect(i);
			assert!(
				still.x >= cell.x && still.y >= cell.y && still.right() <= cell.right() + 0.5,
				"still {i} stays inside its thumbnail box"
			);
		}
		// The selection ring hugs the selected thumbnail.
		let r0 = grid_of(&ui, id).item_rect(0);
		let ring = Rect::new(r0.x - 1.0, r0.y - 1.0, r0.w + 2.0, r0.h + 2.0);
		let ringed = selected.cmds.iter().any(|c| match c {
			DrawCmd::Solid { rect, color } => rect.y == ring.y && rect.w == ring.w && *color == rgba(theme::ACCENT),
			_ => false,
		});
		assert!(ringed, "the selected entry draws its accent ring");

		let (chrome, ui, _id) = hosted(body, snap(None, CELL));
		let plain = drawn(&chrome, &ui);
		assert!(plain.cmds.len() < selected.cmds.len(), "no selection -> no ring");

		// The WxH badge needs a medium-or-larger preview.
		let (chrome, ui, _id) = hosted(body, snap(None, 48.0));
		assert!(drawn(&chrome, &ui).cmds.len() < plain.cmds.len(), "no 8x6/3x3 badges below 64px previews");
	}

	/// A hover ring is the grid's own domain state — and it is drawn only while
	/// the `Ui` agrees the grid is hovered. **That is what deleted the shell's
	/// `grid_hot`**: opening a header dropdown collapses the `Ui`'s hover to the
	/// popup's owner, so the cells go dark without anyone reaching into the panel
	/// to inert them.
	#[test]
	fn a_cell_rings_under_the_pointer_and_goes_dark_under_an_open_dropdown() {
		let body = Rect::new(100.0, 50.0, 300.0, 400.0);
		let (chrome, mut ui, id) = hosted(body, filled(5, CELL, None));
		// Cell 1's dim ring, matched by its top edge — the same shape the accent
		// ring is asserted with, so a stray dim solid elsewhere can't stand in.
		let cell = grid_of(&ui, id).item_rect(1);
		let ringed = |ui: &Ui| {
			drawn(&chrome, ui).cmds.iter().any(|c| match c {
				DrawCmd::Solid { rect, color } => {
					rect.y == cell.y - 1.0 && rect.w == cell.w + 2.0 && *color == rgba(theme::INK_DIM)
				}
				_ => false,
			})
		};

		assert!(!ringed(&ui), "nothing is hovered at rest");
		ui.dispatch(&[Event::PointerMoved { pos: cell.center() }]);
		assert_eq!(grid_of(&ui, id).hover, Some(1), "the grid knows which cell it is");
		assert!(ringed(&ui), "the hovered entry rings dimly");

		// Open the tileset dropdown: the `Ui` hands it the pointer, so the grid's
		// ring goes out even though the cursor never moved.
		let sel_id = ui.get::<TemplatesContent>(id).expect("typed root").selects[0];
		let box_r = ui.rect_of(sel_id).expect("the box is arranged");
		ui.dispatch(&[press(true, box_r.center())]);
		assert!(ui.popup_open(), "the box opened its list");
		assert!(!ringed(&ui), "an open dropdown inerts the grid under it");

		// …and it stays dark as the pointer moves back over the cells, because
		// the `Ui` routes every pointer event to the popup's owner while the list
		// is up. This is the whole of what `main.rs`'s `grid_hot` used to do.
		ui.dispatch(&[Event::PointerMoved { pos: cell.center() }]);
		assert_eq!(grid_of(&ui, id).hover, None, "the grid is not the pointer's target");
		assert!(!ringed(&ui), "so a move under an open list rings nothing");
	}

	/// The grid scrolls itself: the wheel over it moves the rows, off-window rows
	/// are culled, and a short list never scrolls at all.
	#[test]
	fn the_grid_scrolls_itself_and_culls_off_window_rows() {
		let body = Rect::new(0.0, 0.0, 280.0, 300.0);
		let (chrome, mut ui, id) = hosted(body, filled(40, CELL, None));
		let shown = stills(&drawn(&chrome, &ui));
		assert!(shown > 0 && shown < 40, "only the visible rows draw: {shown}");

		let wheel = || Event::Scroll {
			delta: ScrollDelta::Lines(Vec2::new(0.0, 1.0)),
			pos: body.center(),
			mods: Modifiers::NONE,
		};
		assert!(ui.dispatch(&[wheel()]).wants_pointer(), "the grid takes the wheel");
		assert!(grid_of(&ui, id).scroller.offset() > 0.0, "40 entries outgrow a 300px panel");

		// A press in the bar column pages — the bar is chrome the grid claims in
		// its own `hit_test`, since a `Scroller` only takes a press aimed at its
		// owner.
		let bar = Vec2::new(body.right() - 4.0, body.bottom() - 4.0);
		assert!(ui.dispatch(&[press(true, bar)]).wants_pointer(), "the bar takes the press");

		let (_chrome, mut ui, id) = hosted(body, filled(2, CELL, None));
		ui.dispatch(&[wheel()]);
		assert_eq!(grid_of(&ui, id).scroller.offset(), 0.0, "one row never scrolls");
	}

	/// Both dropdowns are hosted `wgpu_ui::Select`s (U3.6), unchanged by the move
	/// into the tree: a press on a box opens *that* list, and picking a row fires
	/// the option's own action tag.
	#[test]
	fn the_dropdowns_open_pick_and_dismiss() {
		let body = Rect::new(100.0, 50.0, 300.0, 400.0);
		let mut snap = filled(5, CELL, None);
		snap.tilesets = vec!["GREEN".to_string(), "DESERT".to_string()];
		let (_chrome, mut ui, id) = hosted(body, snap);

		for (k, want) in [(0usize, Action::TilesetOption(2)), (1, Action::SizeOption(PREVIEW_SIZES.len() - 1))] {
			let sel_id = ui.get::<TemplatesContent>(id).expect("typed root").selects[k];
			let box_r = ui.rect_of(sel_id).expect("the box is arranged");
			ui.dispatch(&[press(true, box_r.center())]);
			assert!(ui.popup_open(), "box {k} opens its list");
			let all = ui.get::<TemplatesContent>(id).unwrap().selects;
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
	/// release (`App::drain_templates`); this test is what says the drain cannot
	/// be moved to the release alone.
	#[test]
	fn a_dropdown_pick_lives_only_for_the_dispatch_that_made_it() {
		let body = Rect::new(100.0, 50.0, 300.0, 400.0);
		let (_chrome, mut ui, id) = hosted(body, filled(5, CELL, None));
		let sel_id = ui.get::<TemplatesContent>(id).expect("typed root").selects[1];
		let box_r = ui.rect_of(sel_id).expect("the box is arranged");

		ui.dispatch(&[press(true, box_r.center())]);
		let popup = ui.get::<Select>(sel_id).expect("typed").popup_rect();
		let row = Vec2::new(popup.x + 2.0, popup.y + 2.0);
		ui.dispatch(&[press(true, row)]);
		assert_eq!(action_of(ui.actions()[0]), Some(Action::SizeOption(0)), "the press itself picks");

		// The release of that same click — the dispatch the shell used to poll.
		ui.dispatch(&[press(false, row)]);
		assert!(ui.actions().is_empty(), "and the release clears it before anyone could read it");
	}

	/// The header flows onto one run when the dock is wide and wraps when it is
	/// narrow — the shape the hand-rolled `flow_header` had — and the band it
	/// produces is exactly as tall: 2px above the runs, each of them a 22px row.
	/// **That is what keeps the grid from moving**: the thumbnails below hang off
	/// this height, so a band a pixel out would redraw the whole panel.
	#[test]
	fn the_header_wraps_to_the_band_height_the_flow_always_had() {
		let mut counts = Vec::new();
		for w in [700.0, 300.0, 180.0] {
			let body = Rect::new(0.0, 0.0, w, 400.0);
			let (_chrome, ui, id) = hosted(body, filled(5, CELL, None));
			let content = ui.get::<TemplatesContent>(id).expect("typed root");
			let header = Widget::child(&content.root, 0).expect("the header");
			// How many runs the flow actually produced, read off the arranged
			// children rather than assumed from the width.
			let mut tops: Vec<u32> =
				(0..header.child_count()).filter_map(|i| header.child(i)).map(|c| c.rect().y as u32).collect();
			tops.dedup();
			let runs = tops.len() as f32;
			let (band, grid) = (header.rect(), grid_of(&ui, id).rect);
			assert_eq!(band.h, 2.0 + runs * (BTN_H + HDR_GAP), "a {w}px dock flowed {runs} run(s)");
			assert_eq!(grid.y, band.bottom(), "the grid starts below the band");
			assert_eq!(grid.bottom(), body.bottom(), "and reaches the body bottom");
			counts.push(runs);
		}
		assert_eq!(counts[0], 1.0, "a wide dock keeps every control on one run");
		assert!(counts[1] > 1.0 && counts[2] > counts[1], "and narrower ones wrap further: {counts:?}");
	}

	/// The right-click item menu's one question: which template is under this
	/// point. It is the grid's own domain hit test — the same one the pick uses,
	/// so the menu can never open on a different entry than a click would arm.
	#[test]
	fn the_right_click_menu_asks_the_grid_which_template_is_under_the_pointer() {
		let body = Rect::new(100.0, 50.0, 300.0, 400.0);
		let (_chrome, ui, id) = hosted(body, filled(5, CELL, None));
		let content = ui.get::<TemplatesContent>(id).expect("typed root");
		for i in 0..5 {
			assert_eq!(content.template_at(grid_of(&ui, id).item_rect(i).center()), Some(i));
		}
		assert_eq!(content.template_at(Vec2::new(body.x + 2.0, body.y + 2.0)), None, "the header is not a thumbnail");
	}

	/// The base-pass draw of the hosted panel.
	fn drawn(chrome: &MenuChrome, ui: &Ui) -> DrawList {
		let mut dl = DrawList::new();
		ui.draw_pass(&mut dl, chrome.theme(), chrome.fonts(), DrawPass::Base);
		dl
	}

	/// The composed thumbnails in `dl`. They are the **untinted** (WHITE) images;
	/// every other image the panel draws — the steel band, the key faces, each
	/// well's grain — carries a colour tint, so the tint is what tells them apart.
	fn stills(dl: &DrawList) -> usize {
		dl.cmds.iter().filter(|c| matches!(c, DrawCmd::Image { color, .. } if *color == wgpu_ui::Rgba::WHITE)).count()
	}
}
