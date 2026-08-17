//! Color Palette dockable, laid out by the palette slot
//! contract (`docs/design/tileset-contract.md` §1): labeled sections (the
//! label ink + an amber tick mark the editable dynamic slots, 64–159),
//! animated classes dotted, duplicate colors in the dynamic range flagged.
//! Swatches are always opaque - they show the true palette colour.
//!
//! Single click selects (`color N`); the editor strip below the grid edits
//! the selected **dynamic** slot in HSL - and for water cycle slots a
//! second bar row re-tints the whole animated block. Edits land as project
//! palette overrides (map-specific colors), undoable.
//!
//! # Shape (U5.9)
//!
//! One retained tree, hosting **both** panels — the full Color Palette and the
//! bare WRL Internal Palette. The chrome each has is a [`Reveal`] slot, so the
//! `bare` flag in the synced [`Snapshot`] picks a panel by *showing* parts of one
//! tree rather than by branching two draw paths:
//!
//! ```text
//! Linear::column
//!   [0] Reveal  toolbar        tabs + animate, then the wrapped action keys
//!   [1] Reveal  cycle/static   the bare panel's header band
//!   [2] Stack   body           SwatchGrid xor ScrollArea<List>  (Flex)
//!   [3] Separator              the rule over the editor strip
//!   [4] Linear  editor strip   info line, then four Reveal'd track rows
//! ```
//!
//! The 256 swatches are **retained** [`ColorButton`]s owned by [`SwatchGrid`],
//! re-synced (`set_color` / `set_selected`) every frame and never rebuilt: a
//! rebuilt tree mints new ids each frame, and hover, arming and capture all hang
//! off the id. The grid arranges them itself rather than through a `Grid`,
//! because the cell size is the panel's width divided by [`COLS`] — a size no
//! stock container measures to.
//!
//! Everything discrete the panel produces (a swatch, a tab, a key, a saved row,
//! the cycle toggles) comes back as an action tag through [`action_of`]. The
//! edit *gestures* — six absolute [`Slider`] tracks and three relative
//! [`BlockBar`]s — carry colours, which do not fit a `u64` tag, so they queue as
//! ordered [`Edit`]s the shell drains with [`PaletteContent::take_edit`]. The
//! panel resolves them against the selection itself, so no drag lifecycle and no
//! baseline is left shell-side.

use map_core::{WATER_CYCLES, hsl_to_rgb, rgb_to_hsl};

use wgpu_ui::widget::{DrawCtx, EventCtx, LayoutCtx, Widget};
use wgpu_ui::{
	Button, ColorButton, CommitPolicy, CrossAlign, DragPhase, DrawList, Emboss, Event, Insets, Label, Length, Linear,
	List, PageKeys, PointerButton, Reveal, Rgba, ScrollArea, Scroller, Separator, Size, Slider, Spacer, Stack,
	TextRole, Vec2, Well, WidgetId, WidgetState, Wrap, descendant, descendant_mut,
};

use crate::theme;
use crate::ui::Rect;
use crate::uikit_theme::rgba;

const COLS: u16 = 8; // 8 swatches per line - a water-cycle block reads as one row
const PAD: f32 = 4.0;
const LABEL_H: f32 = 13.0;
const GAP: f32 = 1.0;
/// The bare panel's cycle/static header band.
pub const HEADER_H: f32 = 22.0;
/// The info line at the strip's top; the track rows stack below it.
const INFO_H: f32 = 22.0;
/// A track row's slot: the track itself plus the gap under it.
const BAR_H: f32 = 13.0;
const BAR_GAP: f32 = 6.0;
const BAR_ROW_H: f32 = BAR_H + BAR_GAP;
/// The editor strip at the panel bottom, **by construction**: the info line over
/// the deepest stack of track rows the strip can show (RGB + HSL + block).
///
/// Declared as that sum rather than hand-tuned to it, because the strip is a
/// `Length::Fixed` slot and a `Linear` does not clip: one pixel short and the
/// block row's last scanline paints over the panel frame — which is exactly what
/// the pre-U5.9 78px strip did, silently swallowing the row whenever a water
/// slot was selected.
pub const EDITOR_H: f32 = INFO_H + 3.0 * BAR_ROW_H;
/// The row-prefix column ("rgb" / "hsl" / "block") and the per-track letter.
const PREFIX_W: f32 = 36.0;
const LETTER_W: f32 = 11.0;
/// Slack after each track, so three cells tile the strip with a gap between.
const TRACK_TAIL: f32 = 4.0;
/// Block-bar drag sensitivity: degrees of hue per px; S/L fraction per px.
pub const HUE_PER_PX: f32 = 1.0;
pub const SL_PER_PX: f32 = 0.005;

/// Toolbar row height (one button row), and the action keys' width.
pub const TAB_H: f32 = 20.0;
const ABW: f32 = 36.0;
/// The `animate` toggle's width (toolbar row 1, right-aligned).
const ANIM_W: f32 = 76.0;
/// The grid / saved tabs, and the bare panel's cycle / static keys.
const GRID_TAB_W: f32 = 44.0;
const SAVED_TAB_W: f32 = 46.0;
const CYCLE_W: f32 = 56.0;
/// Toolbar / header inset, and the gap between keys on a row.
const TB_PAD: f32 = 2.0;
/// The rule between the body and the editor strip.
const RULE_H: f32 = 1.0;

/// The root column's slots, by index — [`Widget::child`] reads the arranged
/// rects back off them for the two bands the tree does not paint itself.
const SLOT_TOOLBAR: usize = 0;
const SLOT_HEADER: usize = 1;
const SLOT_STRIP: usize = 4;

/// One contract range (`end` inclusive).
pub struct Section {
	pub label: &'static str,
	pub start: u16,
	pub end: u16,
	pub editable: bool,
	pub animated: bool,
}

/// The palette slot contract, §1. Dynamic slots 64–159 belong to the
/// tileset; everything else is the game's. Animated = color-cycled in game
/// (9–31 system sparkle/sea, 96–127 the per-planet water colors). Each
/// water cycle block gets its own labeled line - one block = one gradient,
/// reading it as a row is the point.
pub const SECTIONS: [Section; 11] = [
	Section { label: "system 0-8", start: 0, end: 8, editable: false, animated: false },
	Section { label: "game animated 9-31", start: 9, end: 31, editable: false, animated: true },
	Section { label: "game ramps 32-63", start: 32, end: 63, editable: false, animated: false },
	Section { label: "map tiles 64-95", start: 64, end: 95, editable: true, animated: false },
	Section { label: "water cycle 96-102", start: 96, end: 102, editable: true, animated: true },
	Section { label: "water cycle 103-109", start: 103, end: 109, editable: true, animated: true },
	Section { label: "water cycle 110-116", start: 110, end: 116, editable: true, animated: true },
	Section { label: "water cycle 117-122", start: 117, end: 122, editable: true, animated: true },
	Section { label: "water cycle 123-127", start: 123, end: 127, editable: true, animated: true },
	Section { label: "map tiles 128-159", start: 128, end: 159, editable: true, animated: false },
	Section { label: "game ramps 160-255", start: 160, end: 255, editable: false, animated: false },
];

/// Is a slot tileset-editable (dynamic)?
pub fn editable(index: u16) -> bool {
	(64..=159).contains(&index)
}

/// Is a slot color-cycled by the game?
pub fn animated(index: u16) -> bool {
	(9..=31).contains(&index) || (96..=127).contains(&index)
}

/// The section a slot belongs to.
pub fn section_of(index: u16) -> &'static Section {
	SECTIONS.iter().find(|s| index >= s.start && index <= s.end).expect("0-255 covered")
}

/// The water cycle block containing a slot, if any.
pub fn water_block(index: u16) -> Option<(u8, u8)> {
	u8::try_from(index).ok().and_then(|i| WATER_CYCLES.iter().copied().find(|&(s, e)| (s..=e).contains(&i)))
}

fn rows(section: &Section) -> u16 {
	(section.end - section.start + 1).div_ceil(COLS)
}

/// Swatch box size: sized to the grid's width (the `COLS` columns fill it),
/// with `gutter` (the theme's scrollbar metric, sampled at `arrange`)
/// reserved so a swatch never sits under the bar.
fn box_px(grid: Rect, gutter: f32) -> f32 {
	let by_w = (grid.w - gutter - 2.0 * PAD - (COLS - 1) as f32 * GAP) / COLS as f32;
	by_w.clamp(4.0, 28.0)
}

/// The selected slot range `(lo, hi)` from the anchor + optional shift-end;
/// `lo == hi` is a single slot. `None` when nothing is selected.
pub fn selection(active: Option<u16>, sel_end: Option<u16>) -> Option<(u16, u16)> {
	let a = active?;
	Some(sel_end.map_or((a, a), |e| (a.min(e), a.max(e))))
}

/// Editable (dynamic, tileset-owned) slots inside an inclusive range.
pub fn editable_in(lo: u16, hi: u16) -> Vec<u16> {
	(lo..=hi).filter(|&i| editable(i)).collect()
}

/// Slot colors that repeat within the **dynamic** range (64–159) - wasted
/// editable slots, flagged per the design.
pub fn dynamic_duplicates(palette: &[u8]) -> Vec<u16> {
	let mut out = Vec::new();
	for i in 64..=159u16 {
		let a = &palette[i as usize * 3..i as usize * 3 + 3];
		let dup = (64..=159u16).any(|j| j != i && &palette[j as usize * 3..j as usize * 3 + 3] == a);
		if dup {
			out.push(i);
		}
	}
	out
}

/// The `[slot*3..]` colour of a palette slot.
fn rgb_at(palette: &[u8], slot: u16) -> [u8; 3] {
	let at = slot as usize * 3;
	[palette[at], palette[at + 1], palette[at + 2]]
}

// --- what the panel produces --------------------------------------------------

/// A discrete thing the panel fired: everything that is a click on a *control*.
/// The value-carrying edit gestures are [`Edit`]s instead.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
	/// A swatch. Whether that selects, toggles or extends is the *shell's* call —
	/// it holds the live modifier state, and a stock `ColorButton` carries none.
	Select(u16),
	/// The bare panel's header keys: turn palette cycling on/off.
	Cycle(bool),
	/// The full panel's `animate` toggle.
	CycleToggle,
	/// Switch the panel's tab: false = the grid, true = the saved-palettes list.
	ShowSaved(bool),
	/// Toolbar keys: Save (name modal), Edit (rename the selected saved
	/// palette), Delete it, Import a file, Export the working palette.
	Save,
	Edit,
	Delete,
	Import,
	Export,
	/// A row in the saved-palettes list - load + select it.
	LoadSaved(usize),
}

/// What a manager key needs before it does anything (the shared header-key
/// convention, [`crate::panel_ui`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Need {
	/// Nothing - save/import/export always apply.
	Always,
	/// A saved *user* palette selected - `edit` renames it, `del` deletes it.
	UserPalette,
}

/// The five manager keys of toolbar row 2, in the order they are laid out:
/// label, the [`Action`] the key fires, and what it needs.
const COMMANDS: [(&str, Action, Need); 5] = [
	("save", Action::Save, Need::Always),
	("edit", Action::Edit, Need::UserPalette),
	("del", Action::Delete, Need::UserPalette),
	("imp", Action::Import, Need::Always),
	("exp", Action::Export, Need::Always),
];

/// One tag space for the whole panel: `kind << 32 | payload`. Kind 0 is left
/// unused, so a stray zero resolves to nothing.
const KIND_SHIFT: u32 = 32;
/// A swatch: the payload is its slot.
const KIND_SLOT: u64 = 1;
/// A tab: the payload is 1 for the saved list, 0 for the grid.
const KIND_TAB: u64 = 2;
/// A manager key: the payload indexes [`COMMANDS`].
const KIND_CMD: u64 = 3;
/// The `animate` toggle (no payload).
const KIND_ANIMATE: u64 = 4;
/// A bare-panel header key: 1 = cycle, 0 = static.
const KIND_CYCLE: u64 = 5;
/// A saved-list row: the payload is its index.
const KIND_SAVED: u64 = 6;

const fn tag(kind: u64, payload: u64) -> u64 {
	(kind << KIND_SHIFT) | payload
}

/// The palette action a fired tag stands for, or `None` if it is not one of
/// this panel's.
pub fn action_of(t: u64) -> Option<Action> {
	let payload = t & 0xffff_ffff;
	match t >> KIND_SHIFT {
		KIND_SLOT => (payload < 256).then_some(Action::Select(payload as u16)),
		KIND_TAB => Some(Action::ShowSaved(payload != 0)),
		KIND_CMD => COMMANDS.get(payload as usize).map(|&(_, a, _)| a),
		KIND_ANIMATE => Some(Action::CycleToggle),
		KIND_CYCLE => Some(Action::Cycle(payload != 0)),
		KIND_SAVED => Some(Action::LoadSaved(payload as usize)),
		_ => None,
	}
}

/// One step of a colour-editing gesture, queued in the order the panel produced
/// it. A drag is `Begin`, then a `Colors` per move, then `End` — so the shell's
/// whole job is bracketing them in one undo stroke.
///
/// The colours are **absolute and already resolved against the selection**: the
/// panel knows the base palette, the range and the Ctrl-built multi set, so a
/// relative block shift is re-derived from the baseline it captured at `Begin`
/// rather than compounded frame by frame. That is what lets the shell hold no
/// drag state at all.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
	/// A track drag began — open exactly one undo stroke.
	Begin,
	/// Slots to write, `(slot, rgb)`.
	Colors(Vec<(u8, [u8; 3])>),
	/// The drag ended — close the stroke.
	End,
}

// --- the swatch grid ----------------------------------------------------------

/// The scrolling swatch grid: 256 retained [`ColorButton`]s over its own
/// [`Scroller`], plus the section decoration (label, editable tick) and the
/// per-swatch adornments (animated dot, duplicate corner, range ring) drawn as
/// an overlay over its own cells.
///
/// It arranges the swatches itself instead of handing them to a `Grid`, because
/// the cell is the panel width divided by [`COLS`] — the grid is the viewport,
/// the flowed sections are the content, and both change with the dock width.
/// Off-window swatches keep their true (scrolled-away) rects, so [`hit_test`]
/// gates on the viewport first and [`draw`] culls to it.
///
/// [`hit_test`]: Widget::hit_test
/// [`draw`]: Widget::draw
pub struct SwatchGrid {
	id: WidgetId,
	/// One swatch per slot, in slot order. Re-synced, **never rebuilt**.
	boxes: Vec<ColorButton>,
	/// The dynamic-range duplicates, flagged with a corner mark.
	dups: Vec<u16>,
	/// The current range; a plain range rings in ink, while the Ctrl-built multi
	/// set rides the swatch's own `selected` face (the theme's accent ring).
	range: Option<(u16, u16)>,
	rect: Rect,
	scroller: Scroller,
	/// The theme's scrollbar metric, sampled at `arrange` — the gutter
	/// [`box_px`] reserves, kept equal to the bar the `Scroller` paints.
	gutter: f32,
	/// An offset `palette scroll N` asked for, applied at the next `arrange`.
	pending_scroll: Option<f32>,
}

impl SwatchGrid {
	fn new() -> Self {
		let boxes = (0..256u16)
			.map(|i| {
				// A swatch commits on the *press*: a selection is immediate
				// feedback, not a confirmable command.
				ColorButton::new(Rgba::rgb(0, 0, 0), 8.0, 8.0)
					.inset(1.0)
					.commit(CommitPolicy::PressFire)
					.action(tag(KIND_SLOT, u64::from(i)))
			})
			.collect();
		Self {
			id: wgpu_ui::next_id(),
			boxes,
			dups: Vec::new(),
			range: None,
			rect: Rect::ZERO,
			scroller: Scroller::new(),
			gutter: 8.0,
			pending_scroll: None,
		}
	}

	/// The scrolled content's full height (labels + swatch rows).
	fn content_height(&self) -> f32 {
		let b = box_px(self.rect, self.gutter);
		let total: u16 = SECTIONS.iter().map(rows).sum();
		2.0 * PAD + SECTIONS.len() as f32 * LABEL_H + total as f32 * (b + GAP)
	}

	/// Slot `index`'s cell at the current scroll — the swatch's own arranged
	/// rect, so geometry is read back off the tree rather than recomputed.
	#[cfg(test)]
	fn slot_rect(&self, index: u16) -> Rect {
		self.boxes.get(index as usize).map_or(Rect::ZERO, Widget::rect)
	}

	/// Whether slot `index` is inside the visible window.
	fn on_window(&self, r: Rect) -> bool {
		r.bottom() >= self.rect.y && r.y <= self.rect.bottom()
	}
}

impl Widget for SwatchGrid {
	fn measure(&mut self, avail: Size, _ctx: &mut LayoutCtx) -> Size {
		avail
	}

	fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
		self.rect = rect;
		self.gutter = ctx.theme.metrics().scrollbar;
		self.scroller.layout(ctx, rect, self.content_height());
		if let Some(to) = self.pending_scroll.take() {
			self.scroller.set_offset(to);
		}
		// The cell size follows the dock width, so every swatch is re-arranged
		// each layout — the ids stay, only the rects move.
		let (b, off) = (box_px(rect, self.gutter), self.scroller.offset());
		let mut y = rect.y + PAD - off;
		for s in &SECTIONS {
			y += LABEL_H;
			for index in s.start..=s.end {
				let i = index - s.start;
				let cell =
					Rect::new(rect.x + PAD + (i % COLS) as f32 * (b + GAP), y + (i / COLS) as f32 * (b + GAP), b, b);
				self.boxes[index as usize].arrange(cell, ctx);
			}
			y += rows(s) as f32 * (b + GAP);
		}
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		dl.push_clip(self.rect);
		let b = box_px(self.rect, self.gutter);
		let mut y = self.rect.y + PAD - self.scroller.offset();
		for s in &SECTIONS {
			let section_h = LABEL_H + rows(s) as f32 * (b + GAP);
			// Cull sections fully outside the visible window.
			if y + section_h < self.rect.y || y > self.rect.bottom() {
				y += section_h;
				continue;
			}
			let ink = if s.editable { theme::INK } else { theme::INK_DIM };
			ctx.theme.text_top(
				dl,
				ctx.fonts,
				Vec2::new(self.rect.x + PAD, y),
				s.label,
				TextRole::Small,
				Emboss::Engraved,
				rgba(ink),
			);
			if s.editable {
				// Editable sections carry an amber tick before the label line.
				dl.fill_rect(Rect::new(self.rect.x + 1.0, y + 2.0, 2.0, LABEL_H - 4.0), rgba(theme::INK));
			}
			y += LABEL_H;
			for index in s.start..=s.end {
				let sw = &self.boxes[index as usize];
				let r = sw.rect();
				// A swatch scrolled out of the window keeps its rect (so the
				// arrange stays one pass) but must not paint over the chrome.
				if !self.on_window(r) {
					continue;
				}
				sw.draw(dl, ctx);
				if s.animated {
					let dot = if s.editable { theme::INK } else { theme::INK_DIM };
					dl.fill_rect(Rect::new(r.x + 1.0, r.bottom() - 3.0, 2.0, 2.0), rgba(dot));
				}
				if s.editable && self.dups.contains(&index) {
					dl.fill_rect(Rect::new(r.right() - 3.0, r.y + 1.0, 2.0, 2.0), rgba(theme::CLOSE_INK));
				}
				// The Ctrl-built multi set is the swatch's own `selected` face;
				// a plain range is a domain flavour the theme has no word for, so
				// it stays an overlay ring.
				if !sw.selected() && self.range.is_some_and(|(lo, hi)| (lo..=hi).contains(&index)) {
					dl.stroke_rect(Rect::new(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0), 1.0, rgba(theme::INK));
				}
			}
			y += rows(s) as f32 * (b + GAP);
		}
		dl.pop_clip();
		// The bar sits over the cells, outside their clip.
		self.scroller.draw(dl, ctx);
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		let mut handled = false;
		for sw in &mut self.boxes {
			handled |= sw.event(ev, ctx);
		}
		if handled {
			return true;
		}
		// The swatches keep first refusal; the wheel, the bar and the paging keys
		// fall to the scroller.
		self.scroller.event_with(ev, ctx, self.id, PageKeys::WhenHovered)
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

	/// The window gates everything: a scrolled-away swatch still *has* a rect,
	/// and without this it would answer for a point over the toolbar above. The
	/// bar has to be claimed explicitly — a [`Scroller`] only takes a press aimed
	/// at its owner (U5.5).
	fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
		if !self.rect.contains(pos) {
			return None;
		}
		if self.scroller.has_bar() && self.scroller.track_rect().contains(pos) {
			return Some(self.id);
		}
		self.boxes.iter().find_map(|b| b.hit_test(pos))
	}
}

// --- the relative block bar ---------------------------------------------------

/// One relative HSL bar: a well with a centre notch at the rest position, which
/// re-tints a whole block by however far the pointer has travelled since the
/// press. Not a [`Slider`] — a slider has a value, and this has only a delta,
/// which is why it is a content widget rather than a stock track.
///
/// It reports the same drag edges a `Slider` does, so one gesture is one undo
/// stroke by the same rule (G9).
pub struct BlockBar {
	id: WidgetId,
	rect: Rect,
	dragging: bool,
	/// Where the press landed; the shift is measured from here every move.
	start_x: f32,
	dx: f32,
	drags: Vec<DragPhase>,
}

impl BlockBar {
	fn new() -> Self {
		Self { id: wgpu_ui::next_id(), rect: Rect::ZERO, dragging: false, start_x: 0.0, dx: 0.0, drags: Vec::new() }
	}

	fn id(&self) -> WidgetId {
		self.id
	}

	/// Pops the oldest un-reported drag edge — poll in a loop, since a press and
	/// its release can land in the same dispatch batch.
	fn take_drag(&mut self) -> Option<DragPhase> {
		(!self.drags.is_empty()).then(|| self.drags.remove(0))
	}

	fn dragging(&self) -> bool {
		self.dragging
	}

	/// Pointer travel since the press, in logical px.
	fn dx(&self) -> f32 {
		self.dx
	}
}

impl Widget for BlockBar {
	fn measure(&mut self, avail: Size, _ctx: &mut LayoutCtx) -> Size {
		Size::new(avail.w, BAR_H)
	}

	fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
		self.rect = rect;
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		ctx.theme.well(dl, self.rect, WidgetState::default());
		// Relative bars: a centre notch marks the rest position.
		dl.fill_rect(Rect::new(self.rect.x + self.rect.w / 2.0 - 1.0, self.rect.y, 2.0, self.rect.h), rgba(theme::INK));
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		match ev {
			Event::PointerButton { button: PointerButton::Primary, pressed: true, .. } if ctx.is_target(self.id) => {
				ctx.consume_pointer();
				self.dragging = true;
				self.start_x = ctx.pointer.x;
				self.dx = 0.0;
				self.drags.push(DragPhase::Begin);
				ctx.capture(self.id);
				true
			}
			Event::PointerMoved { .. } if self.dragging && ctx.is_target(self.id) => {
				self.dx = ctx.pointer.x - self.start_x;
				ctx.fire(self.id, None);
				ctx.consume_pointer();
				true
			}
			Event::PointerButton { button: PointerButton::Primary, pressed: false, .. } if self.dragging => {
				self.dragging = false;
				self.drags.push(DragPhase::End);
				ctx.consume_pointer();
				true
			}
			// Window focus loss: the release will never arrive, so end the drag
			// here or the host's undo stroke stays open forever.
			Event::Focus(false) if self.dragging => {
				self.dragging = false;
				self.drags.push(DragPhase::End);
				false
			}
			_ => false,
		}
	}

	fn rect(&self) -> Rect {
		self.rect
	}

	fn id(&self) -> WidgetId {
		self.id
	}
}

// --- the tree -----------------------------------------------------------------

/// A muted caption, the ink every readout in this panel starts at.
fn caption(text: &str) -> Label {
	Label::new(text).small().muted()
}

/// One track cell: the channel letter over the track it names, with the slack
/// that separates it from the next cell.
fn track_cell(letter: &str, track: impl Widget + 'static) -> Linear {
	Linear::row()
		.cross_align(CrossAlign::Stretch)
		.child(caption(letter), Length::Fixed(LETTER_W))
		.child(track, Length::Flex(1.0))
		.child(Spacer::new(), Length::Fixed(TRACK_TAIL))
}

/// One row of three equal track cells behind a prefix column. The row's own
/// bottom padding is the gap to the next row, so the slot is a whole
/// [`BAR_ROW_H`] and a hidden one takes the row with it.
fn track_row(prefix: Label, cells: [Linear; 3]) -> Linear {
	let mut row = Linear::row()
		.padding(Insets { left: PAD, top: 0.0, right: PAD, bottom: BAR_GAP })
		.cross_align(CrossAlign::Stretch)
		.child(prefix, Length::Fixed(PREFIX_W));
	for cell in cells {
		row = row.child(cell, Length::Flex(1.0));
	}
	row
}

/// The ids the panel reaches its retained children by. Everything the tree can
/// show or hide is a [`Reveal`]; everything it can rewrite is a `Label`,
/// `Button`, `List`, `ColorButton` or track.
struct Ids {
	/// The two chrome bands — exactly one shows, chosen by `Snapshot::bare`.
	toolbar: WidgetId,
	header: WidgetId,
	/// The grid / saved tabs, and the `animate` toggle.
	tabs: [WidgetId; 2],
	animate: WidgetId,
	/// The five manager keys, in [`COMMANDS`] order.
	cmds: [WidgetId; COMMANDS.len()],
	/// The bare panel's cycle / static keys.
	cycle: [WidgetId; 2],
	/// The body's two alternatives, and what they hold.
	grid_slot: WidgetId,
	saved_slot: WidgetId,
	grid: WidgetId,
	list_slot: WidgetId,
	list: WidgetId,
	empty_note: WidgetId,
	/// The editor strip's info line: the readout, and the selected colour's chip.
	info: WidgetId,
	chip_slot: WidgetId,
	chip: WidgetId,
	/// The four optional track rows, top to bottom.
	note_row: WidgetId,
	rgb_row: WidgetId,
	hsl_row: WidgetId,
	block_row: WidgetId,
	/// The row prefix that reads "block" for a single water slot and nothing for
	/// a multi-slot range (whose bars are the only row there is).
	block_prefix: WidgetId,
	/// The six absolute tracks: R/G/B then H/S/L.
	sliders: [WidgetId; 6],
	/// The three relative tracks: H/S/L.
	blocks: [WidgetId; 3],
}

/// Build the panel's tree once. Both panels come out of this one call — the
/// `bare` flag in the synced snapshot picks which chrome slots show.
fn build() -> (Linear, Ids) {
	// --- toolbar row 1: the tabs, then the animate toggle at the right edge ---
	let mut tabs = [WidgetId::NONE; 2];
	let grid_tab = Button::new("grid").small().sized(GRID_TAB_W, TAB_H - 2.0 * TB_PAD).action(tag(KIND_TAB, 0));
	tabs[0] = grid_tab.id();
	let saved_tab = Button::new("saved").small().sized(SAVED_TAB_W, TAB_H - 2.0 * TB_PAD).action(tag(KIND_TAB, 1));
	tabs[1] = saved_tab.id();
	let animate = Button::new("animate").small().sized(ANIM_W, TAB_H - 2.0 * TB_PAD).action(tag(KIND_ANIMATE, 0));
	let animate_id = animate.id();
	let row1 = Linear::row()
		.padding(Insets::all(TB_PAD))
		.spacing(TB_PAD)
		.cross_align(CrossAlign::Stretch)
		.child(grid_tab, Length::Fixed(GRID_TAB_W))
		.child(saved_tab, Length::Fixed(SAVED_TAB_W))
		.child(Spacer::new(), Length::Flex(1.0))
		.child(animate, Length::Fixed(ANIM_W));

	// --- toolbar row 2: the manager keys, flowed ------------------------------
	// A `Wrap`, not a `Linear`: five fixed keys re-pack onto more runs in a
	// narrow dock, and `run_extent` is what makes each run a *row* rather than
	// the height of whatever happened to land on it.
	let mut row2 = Wrap::row()
		.padding(Insets::all(TB_PAD))
		.spacing(1.0)
		.run_spacing(2.0 * TB_PAD)
		.run_extent(TAB_H - 2.0 * TB_PAD)
		.line_align(CrossAlign::Center);
	let mut cmds = [WidgetId::NONE; COMMANDS.len()];
	for (i, (label, ..)) in COMMANDS.iter().enumerate() {
		let key = Button::new(*label).small().sized(ABW, TAB_H - 2.0 * TB_PAD).action(tag(KIND_CMD, i as u64));
		cmds[i] = key.id();
		row2 = row2.push(key);
	}
	let toolbar = Reveal::new(
		Linear::column().cross_align(CrossAlign::Stretch).child(row1, Length::Fixed(TAB_H)).child(row2, Length::Fit),
	);
	let toolbar_id = toolbar.id();

	// --- the bare panel's cycle/static band -----------------------------------
	let mut cycle = [WidgetId::NONE; 2];
	let cycle_on = Button::new("cycle").small().sized(CYCLE_W, HEADER_H - 2.0 * TB_PAD).action(tag(KIND_CYCLE, 1));
	cycle[0] = cycle_on.id();
	let cycle_off = Button::new("static").small().sized(CYCLE_W, HEADER_H - 2.0 * TB_PAD).action(tag(KIND_CYCLE, 0));
	cycle[1] = cycle_off.id();
	let header = Reveal::new(
		Linear::row()
			.padding(Insets::all(TB_PAD))
			.spacing(TB_PAD)
			.cross_align(CrossAlign::Stretch)
			.child(cycle_on, Length::Fixed(CYCLE_W))
			.child(cycle_off, Length::Fixed(CYCLE_W))
			.child(Spacer::new(), Length::Flex(1.0)),
	)
	.height(HEADER_H)
	.with_shown(false);
	let header_id = header.id();

	// --- the body: the swatch grid, or the saved list over it -----------------
	let grid = SwatchGrid::new();
	let grid_id = grid.id();
	let grid_slot = Reveal::new(grid);
	let grid_slot_id = grid_slot.id();

	let empty_note = Reveal::new(caption("no saved palettes found"));
	let empty_note_id = empty_note.id();
	let list = List::new();
	let list_id = list.id();
	let list_slot = Reveal::new(list).with_shown(false);
	let list_slot_id = list_slot.id();
	// A list box: a `Well` filling the body, the rows scrolling inside it. The
	// well is what makes the row highlight read as a crop of the list's own
	// material (`Theme::accent_well_row`), and the `ScrollArea` is what keeps a
	// long list off the editor strip — the old hand-drawn rows simply stopped
	// being emitted at the panel's bottom edge, so the tail was unreachable.
	let saved_slot = Reveal::new(
		Well::new(
			ScrollArea::new(
				Linear::column()
					.cross_align(CrossAlign::Stretch)
					.child(empty_note, Length::Fit)
					.child(list_slot, Length::Fit),
			)
			.page_keys(PageKeys::WhenHovered),
		)
		.padding(Insets::all(PAD)),
	)
	.with_shown(false);
	let saved_slot_id = saved_slot.id();
	// A `Stack` gives both alternatives the *whole* body rect, which is what a
	// `Length::Flex` slot cannot do for two `Reveal`s: hiding one flex child
	// hands its share to nobody (U5.8).
	let body = Stack::new().push(grid_slot).push(saved_slot);

	// --- the editor strip -----------------------------------------------------
	let info = Label::new("").small().muted().ellipsize().with_id();
	let info_id = info.id();
	// The selected colour at full strength. Inert: it is a readout, so it takes
	// no click and wears the theme's disabled face around the true colour.
	let chip = ColorButton::new(Rgba::rgb(0, 0, 0), 16.0, 16.0).inset(1.0).disabled(true);
	let chip_id = chip.id();
	let chip_slot = Reveal::new(chip).with_shown(false);
	let chip_slot_id = chip_slot.id();
	let info_row = Linear::row()
		.padding(Insets { left: PAD, top: 0.0, right: 6.0, bottom: 0.0 })
		.cross_align(CrossAlign::Center)
		.child(info, Length::Flex(1.0))
		.child(chip_slot, Length::Fit);

	let note_row = Reveal::new(
		Linear::row()
			.padding(Insets { left: PAD, top: 0.0, right: PAD, bottom: BAR_GAP })
			.child(caption("read-only - open a project (.json) to edit").ellipsize(), Length::Flex(1.0)),
	)
	.height(BAR_ROW_H)
	.with_shown(false);
	let note_row_id = note_row.id();

	let mut sliders = [WidgetId::NONE; 6];
	let mut slider_cell = |i: usize, letter: &str, max: f32, step: f32| -> Linear {
		let track = Slider::new(0.0, max, 0.0).step(step);
		sliders[i] = track.id();
		track_cell(letter, track)
	};
	// R/G/B are byte channels; H is degrees and S/L fractions.
	let rgb_row = Reveal::new(track_row(
		caption("rgb"),
		[slider_cell(0, "R", 255.0, 1.0), slider_cell(1, "G", 255.0, 1.0), slider_cell(2, "B", 255.0, 1.0)],
	))
	.height(BAR_ROW_H)
	.with_shown(false);
	let rgb_row_id = rgb_row.id();
	let hsl_row = Reveal::new(track_row(
		caption("hsl"),
		[slider_cell(3, "H", 360.0, 0.0), slider_cell(4, "S", 1.0, 0.0), slider_cell(5, "L", 1.0, 0.0)],
	))
	.height(BAR_ROW_H)
	.with_shown(false);
	let hsl_row_id = hsl_row.id();

	let mut blocks = [WidgetId::NONE; 3];
	let mut block_cell = |i: usize, letter: &str| -> Linear {
		let bar = BlockBar::new();
		blocks[i] = bar.id();
		track_cell(letter, bar)
	};
	let block_prefix = caption("").with_id();
	let block_prefix_id = block_prefix.id();
	let block_row = Reveal::new(track_row(block_prefix, [block_cell(0, "H"), block_cell(1, "S"), block_cell(2, "L")]))
		.height(BAR_ROW_H)
		.with_shown(false);
	let block_row_id = block_row.id();

	let strip = Linear::column()
		.cross_align(CrossAlign::Stretch)
		.child(info_row, Length::Fixed(INFO_H))
		.child(note_row, Length::Fit)
		.child(rgb_row, Length::Fit)
		.child(hsl_row, Length::Fit)
		.child(block_row, Length::Fit);

	// `Stretch` is what gives every band the panel's full width: a `Linear`
	// measures to its *content*, so without it the toolbar would be as wide as
	// its keys and the right-aligned toggle would stop wherever they ended.
	let root = Linear::column()
		.cross_align(CrossAlign::Stretch)
		.child(toolbar, Length::Fit)
		.child(header, Length::Fit)
		.child(body, Length::Flex(1.0))
		.child(Separator::new(), Length::Fixed(RULE_H))
		.child(strip, Length::Fixed(EDITOR_H));

	let ids = Ids {
		toolbar: toolbar_id,
		header: header_id,
		tabs,
		animate: animate_id,
		cmds,
		cycle,
		grid_slot: grid_slot_id,
		saved_slot: saved_slot_id,
		grid: grid_id,
		list_slot: list_slot_id,
		list: list_id,
		empty_note: empty_note_id,
		info: info_id,
		chip_slot: chip_slot_id,
		chip: chip_id,
		note_row: note_row_id,
		rgb_row: rgb_row_id,
		hsl_row: hsl_row_id,
		block_row: block_row_id,
		block_prefix: block_prefix_id,
		sliders,
		blocks,
	};
	(root, ids)
}

/// The palette-panel state, snapshotted each frame so the retained tree holds no
/// borrow. `bare` selects the WRL-internal variant, which is the same tree with
/// the toolbar hidden, the cycle/static band shown and every editing row off.
#[derive(Clone)]
pub struct Snapshot {
	display: Vec<u8>,
	base: Vec<u8>,
	active: Option<u16>,
	sel_end: Option<u16>,
	multi: Vec<u16>,
	cycling: bool,
	can_edit: bool,
	show_saved: bool,
	saved: Vec<String>,
	sel: Option<usize>,
	sel_is_user: bool,
	bare: bool,
}

impl Snapshot {
	/// Snapshot the full Color Palette panel.
	pub fn of(
		display: &[u8],
		base: &[u8],
		active: Option<u16>,
		sel_end: Option<u16>,
		multi: &[u16],
		cycling: bool,
		can_edit: bool,
		show_saved: bool,
		saved: &[String],
		sel: Option<usize>,
		sel_is_user: bool,
	) -> Self {
		Self {
			display: display.to_vec(),
			base: base.to_vec(),
			active,
			sel_end,
			multi: multi.to_vec(),
			cycling,
			can_edit,
			show_saved,
			saved: saved.to_vec(),
			sel,
			sel_is_user,
			bare: false,
		}
	}

	/// Snapshot the bare WRL Internal Palette panel (no toolbar, read-only).
	pub fn of_bare(display: &[u8], base: &[u8], active: Option<u16>, sel_end: Option<u16>, cycling: bool) -> Self {
		Self {
			display: display.to_vec(),
			base: base.to_vec(),
			active,
			sel_end,
			multi: Vec::new(),
			cycling,
			can_edit: false,
			show_saved: false,
			saved: Vec::new(),
			sel: None,
			sel_is_user: false,
			bare: true,
		}
	}

	fn empty() -> Self {
		Self::of(&[0; 768], &[0; 768], None, None, &[], false, false, false, &[], None, false)
	}

	/// The current range, and whether it is a multi-slot one.
	fn range(&self) -> Option<(u16, u16)> {
		selection(self.active, self.sel_end)
	}

	/// The single selected slot, when the range is one slot wide.
	fn single(&self) -> Option<u16> {
		self.range().filter(|&(lo, hi)| lo == hi).map(|(lo, _)| lo)
	}

	/// The slots a relative block shift moves, with the colours it starts from:
	/// every editable slot of a multi-slot range, or the whole water block of a
	/// single one.
	fn block_baseline(&self) -> Vec<(u8, [u8; 3])> {
		let slots: Vec<u8> = match self.range() {
			Some((lo, hi)) if lo != hi => editable_in(lo, hi).iter().map(|&s| s as u8).collect(),
			Some((lo, _)) => water_block(lo).map_or_else(Vec::new, |(s, e)| (s..=e).collect()),
			None => Vec::new(),
		};
		slots.iter().map(|&s| (s, rgb_at(&self.base, u16::from(s)))).collect()
	}

	/// The slots an *absolute* track writes: the Ctrl-built multi set if there is
	/// one, otherwise just the active slot.
	fn absolute_slots(&self) -> Vec<u8> {
		if self.multi.is_empty() {
			self.single().and_then(|s| u8::try_from(s).ok()).into_iter().collect()
		} else {
			self.multi.iter().filter_map(|&s| u8::try_from(s).ok()).collect()
		}
	}

	/// The editor strip's info line for this selection, and whether edits can
	/// land on it.
	fn info_line(&self) -> (String, bool) {
		match self.range() {
			Some((lo, hi)) if lo != hi => {
				let n = editable_in(lo, hi).len();
				// ASCII only - the MAX atlas has no em-dash.
				(format!("{lo}-{hi} selected - {n} editable, drag to shift HSL"), self.can_edit && n > 0)
			}
			Some((lo, _)) => {
				let s = section_of(lo);
				let rgb = rgb_at(&self.base, lo);
				let note = if s.editable { "" } else { "  (fixed)" };
				let text = format!("{lo}  #{:02x}{:02x}{:02x}  {}{note}", rgb[0], rgb[1], rgb[2], s.label);
				(text, s.editable && self.can_edit)
			}
			None => ("click a color to inspect/edit".to_string(), false),
		}
	}
}

/// The whole Color Palette panel as a retained `wgpu_ui` [`Widget`]: a thin root
/// over the built tree, holding the id tables, the per-frame snapshot and the
/// edit queue. Everything else — layout, paint, hover, arming, firing,
/// scrolling — is the tree's.
///
/// One instance hosts *one* panel; the two are separate widgets with separate
/// retained state (selection, scroll, capture).
pub struct PaletteContent {
	id: WidgetId,
	root: Linear,
	ids: Ids,
	snap: Snapshot,
	/// Ordered colour-edit gestures waiting for the shell.
	edits: Vec<Edit>,
	/// The last value each absolute track reported, so a dispatch that did not
	/// move one queues nothing.
	last_slider: [f32; 6],
	/// Ditto for the relative bars, plus the baseline their shifts start from —
	/// captured once, at the press.
	last_block: [f32; 3],
	block_base: Vec<(u8, [u8; 3])>,
	/// The saved row the tree was last synced to, so a `List` fire (which
	/// carries no tag of its own) is recognised as a *change*.
	saved_sel: Option<usize>,
	/// An offset `palette scroll N` asked for, waiting for a grid to hand it to.
	pending_scroll: Option<f32>,
	rect: Rect,
}

impl Default for PaletteContent {
	fn default() -> Self {
		Self::new()
	}
}

impl PaletteContent {
	pub fn new() -> Self {
		let (root, ids) = build();
		Self {
			id: wgpu_ui::next_id(),
			root,
			ids,
			snap: Snapshot::empty(),
			edits: Vec::new(),
			last_slider: [f32::NAN; 6],
			last_block: [0.0; 3],
			block_base: Vec::new(),
			saved_sel: None,
			pending_scroll: None,
			rect: Rect::ZERO,
		}
	}

	/// Push one frame's state into the retained tree — **top-down**: a `Reveal`
	/// hides its whole subtree from every tree walk, so each slot is shown or
	/// hidden *before* anything inside it is reached.
	pub fn sync(&mut self, snap: Snapshot) {
		self.snap = snap;
		let bare = self.snap.bare;

		// --- the two chrome bands ---------------------------------------------
		self.set_shown(self.ids.toolbar, !bare);
		self.set_shown(self.ids.header, bare);
		if bare {
			let (cycle, cycling) = (self.ids.cycle, self.snap.cycling);
			for (i, &id) in cycle.iter().enumerate() {
				self.set_selected(id, (i == 0) == cycling);
			}
		} else {
			let (tabs, show_saved, cycling) = (self.ids.tabs, self.snap.show_saved, self.snap.cycling);
			for (i, &id) in tabs.iter().enumerate() {
				self.set_selected(id, (i == 1) == show_saved);
			}
			self.set_selected(self.ids.animate, cycling);
			// A key whose need fails greys out dead, with the reason as its
			// tooltip (the shared header-key convention, [`crate::panel_ui`]).
			// The shell still validates behind it and reports into the console.
			let idle = !self.snap.sel_is_user;
			for (i, &id) in self.ids.cmds.iter().enumerate() {
				if let Some(key) = descendant_mut::<Button>(&mut self.root, id) {
					let unmet = match COMMANDS[i].2 {
						Need::Always => None,
						Need::UserPalette => (idle).then_some("needs a saved palette selected"),
					};
					crate::panel_ui::sync_header_key(key, unmet);
				}
			}
		}

		// --- the body ---------------------------------------------------------
		let show_saved = self.snap.show_saved;
		self.set_shown(self.ids.grid_slot, !show_saved);
		self.set_shown(self.ids.saved_slot, show_saved);
		if show_saved {
			self.set_shown(self.ids.empty_note, self.snap.saved.is_empty());
			self.set_shown(self.ids.list_slot, !self.snap.saved.is_empty());
			let (saved, sel) = (self.snap.saved.clone(), self.snap.sel);
			if let Some(list) = descendant_mut::<List>(&mut self.root, self.ids.list) {
				list.set_items(saved);
				match sel {
					Some(i) => list.select(i),
					None => list.clear_selection(),
				}
				self.saved_sel = list.selected();
			}
		} else {
			let (display, range) = (self.snap.display.clone(), self.snap.range());
			let (dups, multi) = (dynamic_duplicates(&self.snap.base), self.snap.multi.clone());
			if let Some(grid) = descendant_mut::<SwatchGrid>(&mut self.root, self.ids.grid) {
				for (i, sw) in grid.boxes.iter_mut().enumerate() {
					let at = i * 3;
					sw.set_color(Rgba::rgb(display[at], display[at + 1], display[at + 2]));
					sw.set_selected(multi.contains(&(i as u16)));
				}
				grid.dups = dups;
				grid.range = range;
			}
		}

		// --- the editor strip -------------------------------------------------
		let (text, live) = self.snap.info_line();
		self.set_text(self.ids.info, &text);
		if let Some(label) = descendant_mut::<Label>(&mut self.root, self.ids.info) {
			label.set_muted(!live);
		}
		let single = self.snap.single();
		self.set_shown(self.ids.chip_slot, single.is_some());
		if let Some(slot) = single {
			let rgb = rgb_at(&self.snap.base, slot);
			if let Some(chip) = descendant_mut::<ColorButton>(&mut self.root, self.ids.chip) {
				chip.set_color(Rgba::rgb(rgb[0], rgb[1], rgb[2]));
			}
		}
		// A slot the project could edit but this document cannot: say why there
		// are no tracks. The bare panel is read-only by design, so it stays silent.
		let note = !bare && !live && single.is_some_and(|s| section_of(s).editable);
		self.set_shown(self.ids.note_row, note);
		// The rows: absolute tracks for one editable slot, relative ones for its
		// water block or for a whole multi-slot range.
		let absolute = live && single.is_some();
		let block = if single.is_some() { absolute && single.is_some_and(|s| water_block(s).is_some()) } else { live };
		self.set_shown(self.ids.rgb_row, absolute);
		self.set_shown(self.ids.hsl_row, absolute);
		self.set_shown(self.ids.block_row, block);
		self.set_text(self.ids.block_prefix, if absolute { "block" } else { "" });
		if absolute && !self.tracks_live() {
			// Reseeding a track the user is dragging would fight the drag; a track
			// they are not is the palette's own value. The row is composed from all
			// three tracks, which is also what keeps repeated HSL round trips from
			// drifting the two channels the drag is not touching.
			let rgb = rgb_at(&self.snap.base, single.expect("absolute implies a single slot"));
			let (h, s, l) = rgb_to_hsl(rgb);
			let values = [f32::from(rgb[0]), f32::from(rgb[1]), f32::from(rgb[2]), h, s, l];
			for (i, &id) in self.ids.sliders.iter().enumerate() {
				if let Some(track) = descendant_mut::<Slider>(&mut self.root, id) {
					track.set_value(values[i]);
				}
				self.last_slider[i] = values[i];
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

	/// Latch one key on.
	fn set_selected(&mut self, id: WidgetId, on: bool) {
		if let Some(key) = descendant_mut::<Button>(&mut self.root, id) {
			key.set_selected(on);
		}
	}

	/// Is any track mid-drag? A live gesture owns the values until it ends.
	fn tracks_live(&self) -> bool {
		self.ids.sliders.iter().any(|&id| descendant::<Slider>(&self.root, id).is_some_and(Slider::dragging))
			|| self.ids.blocks.iter().any(|&id| descendant::<BlockBar>(&self.root, id).is_some_and(BlockBar::dragging))
	}

	/// Queue the offset `palette scroll N` asked for; it lands at the next
	/// `arrange`, clamped against the geometry the panel actually has.
	/// `EditorState::execute` cannot reach the panel `Ui`, so the command leaves
	/// a request the shell hands over here (U2.5).
	pub fn request_scroll(&mut self, to: f32) {
		self.pending_scroll = Some(to);
	}

	/// Hand a queued scroll request to the grid, if the grid is *in the tree* — a
	/// hidden `Reveal` reports no children, so while the saved tab is up there is
	/// nothing to hand it to and the request waits for the grid to come back.
	fn forward_scroll(&mut self) {
		let Some(to) = self.pending_scroll.take() else { return };
		match descendant_mut::<SwatchGrid>(&mut self.root, self.ids.grid) {
			Some(grid) => grid.pending_scroll = Some(to),
			None => self.pending_scroll = Some(to),
		}
	}

	/// The next queued colour-edit step, oldest first. Poll in a **loop**, after
	/// both dispatches *and* inside the pointer-capture branch — a drag produces
	/// its moves while the capture holds, which is a path that returns before the
	/// ordinary per-panel polls.
	pub fn take_edit(&mut self) -> Option<Edit> {
		(!self.edits.is_empty()).then(|| self.edits.remove(0))
	}

	/// The colour the three absolute tracks of `row` currently compose: the RGB
	/// row is its three bytes, the HSL row its three components converted back.
	fn composed(&self, row: usize) -> [u8; 3] {
		let at = row * 3;
		let v = |i: usize| descendant::<Slider>(&self.root, self.ids.sliders[at + i]).map_or(0.0, Slider::value);
		if row == 0 {
			[v(0).round() as u8, v(1).round() as u8, v(2).round() as u8]
		} else {
			hsl_to_rgb(v(0), v(1), v(2))
		}
	}

	/// Turn this dispatch's track movement into queued [`Edit`]s. Called after
	/// the tree has seen the event, so the drag edges and values are this event's.
	fn drain_tracks(&mut self) {
		for row in 0..2 {
			for i in 0..3 {
				let slot = row * 3 + i;
				let Some(track) = descendant_mut::<Slider>(&mut self.root, self.ids.sliders[slot]) else { continue };
				let (mut begin, mut end) = (false, false);
				while let Some(phase) = track.take_drag() {
					match phase {
						DragPhase::Begin => begin = true,
						DragPhase::End => end = true,
					}
				}
				let (live, value) = (track.dragging(), track.value());
				if begin {
					self.edits.push(Edit::Begin);
				}
				// A press sets the value before the first move, so click-to-set
				// applies immediately — the same frame the stroke opened.
				if (begin || live) && value != self.last_slider[slot] {
					let rgb = self.composed(row);
					self.edits.push(Edit::Colors(self.snap.absolute_slots().into_iter().map(|s| (s, rgb)).collect()));
					self.last_slider[slot] = value;
				}
				if end {
					self.edits.push(Edit::End);
				}
			}
		}
		for i in 0..3 {
			let Some(bar) = descendant_mut::<BlockBar>(&mut self.root, self.ids.blocks[i]) else { continue };
			let (mut begin, mut end) = (false, false);
			while let Some(phase) = bar.take_drag() {
				match phase {
					DragPhase::Begin => begin = true,
					DragPhase::End => end = true,
				}
			}
			let (live, dx) = (bar.dragging(), bar.dx());
			if begin {
				// The baseline is captured once, here: every move re-derives the
				// shift from it, so a long drag cannot compound its own rounding.
				self.block_base = self.snap.block_baseline();
				self.last_block[i] = 0.0;
				self.edits.push(Edit::Begin);
			}
			if (begin || live) && dx != self.last_block[i] {
				let (dh, ds, dl) = match i {
					0 => (dx * HUE_PER_PX, 0.0, 0.0),
					1 => (0.0, dx * SL_PER_PX, 0.0),
					_ => (0.0, 0.0, dx * SL_PER_PX),
				};
				let shifted = self
					.block_base
					.iter()
					.map(|&(slot, rgb)| {
						let (h, s, l) = rgb_to_hsl(rgb);
						(slot, hsl_to_rgb(h + dh, s + ds, l + dl))
					})
					.collect();
				self.edits.push(Edit::Colors(shifted));
				self.last_block[i] = dx;
			}
			if end {
				self.edits.push(Edit::End);
			}
		}
	}

	/// Re-emit a saved-list pick as an action tag, so the shell polls one place
	/// for everything discrete this panel produces. A `List` fires without a tag
	/// of its own, so the panel reads the pick two ways, and needs both:
	///
	/// - **a press that landed on the list** — clicking the row that is *already*
	///   selected has always reloaded that palette, and a selection watch alone
	///   would silently stop doing that;
	/// - **a change in the selection** — which is how the arrow keys reach it,
	///   the list being Tab-focusable.
	///
	/// Either way the pick lands on the **press**, which is why the shell drains
	/// this panel there as well as after the release.
	fn drain_list(&mut self, ev: &Event, ctx: &mut EventCtx) {
		let id = self.ids.list;
		let Some(list) = descendant::<List>(&self.root, id) else { return };
		let picked = list.selected();
		let clicked = matches!(ev, Event::PointerButton { button: PointerButton::Primary, pressed: true, .. })
			&& list.rect().contains(ctx.pointer);
		if picked == self.saved_sel && !clicked {
			return;
		}
		self.saved_sel = picked;
		if let Some(i) = picked {
			ctx.fire(id, Some(tag(KIND_SAVED, i as u64)));
		}
	}

	/// The swatch grid's arranged window and scroll offset — test-only geometry,
	/// read back off the tree rather than recomputed.
	#[cfg(test)]
	fn grid(&self) -> &SwatchGrid {
		descendant::<SwatchGrid>(&self.root, self.ids.grid).expect("the grid is in the tree")
	}

	/// The arranged rect of one root band, by slot index.
	fn band(&self, slot: usize) -> Rect {
		Widget::child(&self.root, slot).map_or(Rect::ZERO, Widget::rect)
	}
}

impl Widget for PaletteContent {
	fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
		self.rect = rect;
		self.forward_scroll();
		// Measure here as well as in `measure`: a host that arranges without
		// measuring first (the snapshot harness) must still get a laid-out tree.
		self.root.measure(rect.size(), ctx);
		self.root.arrange(rect, ctx);
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if ctx.is_base() {
			// The two materials the tree's own widgets do not carry: the steel band
			// behind whichever chrome row is showing, and the strip's plate.
			for slot in [SLOT_TOOLBAR, SLOT_HEADER] {
				let band = self.band(slot);
				if !band.is_empty() {
					ctx.theme.header_band(dl, band);
				}
			}
			ctx.theme.surface(dl, self.band(SLOT_STRIP));
		}
		self.root.draw(dl, ctx);
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		let handled = self.root.event(ev, ctx);
		self.drain_list(ev, ctx);
		self.drain_tracks();
		handled
	}

	crate::panel_ui::thin_root_plumbing!();
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::uikit_menu::MenuChrome;
	use crate::visual_test::chrome_fixture;
	use wgpu_ui::Ui;
	use wgpu_ui::event::{Modifiers, ScrollDelta};

	/// The real chrome fixture, not `Fonts::new()` + a bare theme: a stock
	/// `Button` measures its label, so an unregistered `FontId` panics.
	fn skin() -> MenuChrome {
		chrome_fixture().2
	}

	/// A hosted panel laid out into `body`, ready to be fed events.
	fn host(snap: Snapshot, body: Rect) -> (Ui, WidgetId) {
		let chrome = skin();
		let content = PaletteContent::new();
		let id = content.id();
		let mut ui = Ui::new(content);
		ui.get_mut::<PaletteContent>(id).unwrap().sync(snap);
		ui.layout_in(body, chrome.theme(), chrome.fonts());
		(ui, id)
	}

	/// Re-layout after a state change the geometry depends on.
	fn relayout(ui: &mut Ui, body: Rect) {
		let chrome = skin();
		ui.layout_in(body, chrome.theme(), chrome.fonts());
	}

	/// A primary press or release at `p`.
	fn press(p: Vec2, pressed: bool) -> Event {
		Event::PointerButton { button: PointerButton::Primary, pressed, pos: p, mods: Modifiers::NONE }
	}

	fn gradient() -> Vec<u8> {
		(0..256u16).flat_map(|i| [i as u8, (i * 3) as u8, (255 - i) as u8]).collect()
	}

	fn full(active: Option<u16>, sel_end: Option<u16>) -> Snapshot {
		let p = gradient();
		Snapshot::of(&p, &p, active, sel_end, &[], false, true, false, &[], None, false)
	}

	#[test]
	fn sections_cover_the_palette_exactly() {
		let mut next = 0u16;
		for s in &SECTIONS {
			assert_eq!(s.start, next, "no gap/overlap before '{}'", s.label);
			assert!(s.end >= s.start);
			next = s.end + 1;
		}
		assert_eq!(next, 256);
		// Contract classes (tileset-contract.md §1).
		assert!(editable(64) && editable(159) && !editable(63) && !editable(160));
		assert!(animated(9) && animated(31) && animated(96) && animated(127));
		assert!(!animated(8) && !animated(32) && !animated(95) && !animated(128));
		assert_eq!(water_block(110), Some((110, 116)));
		assert_eq!(water_block(70), None);
		assert_eq!(section_of(100).label, "water cycle 96-102");
		assert_eq!(section_of(125).label, "water cycle 123-127");
	}

	#[test]
	fn selection_ranges_and_duplicates() {
		assert_eq!(selection(Some(100), None), Some((100, 100)));
		assert_eq!(selection(Some(120), Some(100)), Some((100, 120)), "ordered low..high");
		assert_eq!(selection(None, Some(50)), None);
		assert_eq!(editable_in(60, 66), vec![64, 65, 66], "only 64.. are editable");

		let mut palette = vec![0u8; 768];
		for i in 0..256usize {
			palette[i * 3] = i as u8; // unique reds
		}
		assert!(dynamic_duplicates(&palette).is_empty());
		// Same color at 70 and 130 (both dynamic) → both flagged.
		palette[70 * 3] = 7;
		palette[130 * 3] = 7;
		palette[70 * 3 + 1] = 9;
		palette[130 * 3 + 1] = 9;
		assert_eq!(dynamic_duplicates(&palette), vec![70, 130]);
		// A static slot repeating a dynamic color is not a warning.
		palette[130 * 3 + 1] = 10;
		palette[5 * 3] = 7;
		palette[5 * 3 + 1] = 9;
		assert!(dynamic_duplicates(&palette).is_empty());
	}

	/// Kind 0 is unused, so a stray zero resolves to nothing.
	#[test]
	fn tags_round_trip_to_actions() {
		assert_eq!(action_of(tag(KIND_SLOT, 100)), Some(Action::Select(100)));
		assert_eq!(action_of(tag(KIND_SLOT, 256)), None, "past the palette");
		assert_eq!(action_of(tag(KIND_TAB, 1)), Some(Action::ShowSaved(true)));
		assert_eq!(action_of(tag(KIND_CMD, 2)), Some(Action::Delete));
		assert_eq!(action_of(tag(KIND_CMD, 5)), None, "no such key");
		assert_eq!(action_of(tag(KIND_ANIMATE, 0)), Some(Action::CycleToggle));
		assert_eq!(action_of(tag(KIND_CYCLE, 0)), Some(Action::Cycle(false)));
		assert_eq!(action_of(tag(KIND_SAVED, 3)), Some(Action::LoadSaved(3)));
		assert_eq!(action_of(0), None);
	}

	/// The grid is the viewport and the flowed sections are the content: a wider
	/// dock grows the cells, a short one scrolls, and the swatches land on the
	/// geometry the section flow describes.
	#[test]
	fn the_grid_fills_its_window_and_scrolls() {
		let narrow = Rect::new(0.0, 0.0, 200.0, 300.0);
		let wide = Rect::new(0.0, 0.0, 420.0, 300.0);
		assert!(box_px(wide, 8.0) > box_px(narrow, 8.0), "a wider window -> bigger swatches");

		let (ui, id) = host(full(None, None), narrow);
		let content = ui.get::<PaletteContent>(id).unwrap();
		let grid = content.grid();
		assert!(grid.scroller.has_bar(), "the 8-per-line grid can't fit a 300px window");
		// Slot 0 sits a pad + the first section label below the window top.
		assert_eq!(grid.slot_rect(0).y, grid.rect.y + PAD + LABEL_H);
		// The window is the complement of the chrome above and below it.
		assert!(grid.rect.y >= content.band(SLOT_TOOLBAR).bottom());
		assert!(grid.rect.bottom() <= content.band(SLOT_STRIP).y);

		let tall = Rect::new(0.0, 0.0, 200.0, 1400.0);
		let (ui, id) = host(full(None, None), tall);
		assert!(!ui.get::<PaletteContent>(id).unwrap().grid().scroller.has_bar(), "a tall dock needs no scroll");
	}

	/// A swatch selects on the **press** — a selection is feedback, not a
	/// confirmable command — and a scrolled-away one is not clickable at all,
	/// because the window gates the hit test before any swatch rect answers.
	#[test]
	fn a_swatch_fires_on_the_press_and_only_inside_the_window() {
		let body = Rect::new(0.0, 0.0, 260.0, 460.0);
		let (mut ui, id) = host(full(None, None), body);
		let cell = ui.get::<PaletteContent>(id).unwrap().grid().slot_rect(10);
		let at = Vec2::new(cell.x + cell.w / 2.0, cell.y + cell.h / 2.0);
		ui.dispatch(&[press(at, true)]);
		assert_eq!(ui.actions().iter().copied().find_map(action_of), Some(Action::Select(10)), "the press selects");
		ui.dispatch(&[press(at, false)]);
		assert_eq!(ui.actions().iter().copied().find_map(action_of), None, "and not again on the release");

		// Scroll the top sections away: slot 0's rect now overlaps the toolbar,
		// and must answer nothing.
		ui.get_mut::<PaletteContent>(id).unwrap().request_scroll(200.0);
		relayout(&mut ui, body);
		let content = ui.get::<PaletteContent>(id).unwrap();
		let gone = content.grid().slot_rect(0);
		assert!(gone.y < content.grid().rect.y, "slot 0 scrolled above the window");
		assert_eq!(content.hit_test(Vec2::new(gone.x + 1.0, gone.y + 1.0)), None, "and is not clickable there");
	}

	/// The two panels scroll independently (U2.5) — one shared widget could never
	/// have held two offsets — and `palette scroll N` lands through the request
	/// channel, since `execute` cannot reach a panel's `Ui`.
	#[test]
	fn the_two_palettes_scroll_independently() {
		let body = Rect::new(0.0, 0.0, 260.0, 300.0);
		let p = gradient();
		let (mut a, aid) = host(full(None, None), body);
		let (b, bid) = host(Snapshot::of_bare(&p, &p, None, None, false), body);

		let wheel = Event::Scroll {
			delta: ScrollDelta::Lines(Vec2::new(0.0, 1.0)),
			pos: Vec2::new(130.0, 150.0),
			mods: Modifiers::NONE,
		};
		assert!(a.dispatch(&[wheel]).wants_pointer(), "the panel takes the wheel");
		assert!(a.get::<PaletteContent>(aid).unwrap().grid().scroller.offset() > 0.0, "one wheel notch");
		assert_eq!(b.get::<PaletteContent>(bid).unwrap().grid().scroller.offset(), 0.0, "the other did not move");

		a.get_mut::<PaletteContent>(aid).unwrap().request_scroll(42.0);
		relayout(&mut a, body);
		assert_eq!(a.get::<PaletteContent>(aid).unwrap().grid().scroller.offset(), 42.0);
	}

	/// The strip's rows are `Reveal` slots, so hiding the absolute ones moves the
	/// relative row up into their place: a multi-slot range gets exactly one row
	/// of bars, at the top of the strip, which is where the old hand-placed
	/// geometry put it.
	#[test]
	fn the_strip_shows_the_rows_the_selection_has() {
		let body = Rect::new(0.0, 0.0, 280.0, 460.0);
		let shown = |snap: Snapshot| {
			let (ui, id) = host(snap, body);
			let c = ui.get::<PaletteContent>(id).unwrap();
			let rect = |slot: WidgetId| descendant::<Reveal>(&c.root, slot).map_or(Rect::ZERO, Widget::rect);
			(
				!rect(c.ids.rgb_row).is_empty(),
				!rect(c.ids.hsl_row).is_empty(),
				!rect(c.ids.block_row).is_empty(),
				!rect(c.ids.note_row).is_empty(),
				rect(c.ids.block_row).y,
				c.band(SLOT_STRIP).y,
			)
		};

		// A single editable, non-water slot: the two absolute rows, nothing else.
		let (rgb, hsl, block, note, _, _) = shown(full(Some(70), None));
		assert_eq!((rgb, hsl, block, note), (true, true, false, false));
		// A water slot adds the block row, third — and the strip is exactly deep
		// enough for it. A `Linear` does not clip, so a strip one pixel short
		// would paint that row over the panel frame instead of dropping it.
		let (rgb, hsl, block, note, block_y, strip_y) = shown(full(Some(100), None));
		assert_eq!((rgb, hsl, block, note), (true, true, true, false));
		assert_eq!(block_y, strip_y + INFO_H + 2.0 * BAR_ROW_H, "the block row is the third");
		assert_eq!(block_y + BAR_ROW_H, strip_y + EDITOR_H, "and the last one fits");
		// A multi-slot range: one relative row, and it sits where row 0 does.
		let (rgb, hsl, block, note, block_y, strip_y) = shown(full(Some(64), Some(95)));
		assert_eq!((rgb, hsl, block, note), (false, false, true, false));
		assert_eq!(block_y, strip_y + INFO_H, "the only row is the first");
		// A fixed slot: no tracks at all, and no read-only note either (nothing
		// about the document is stopping it).
		let (rgb, hsl, block, note, _, _) = shown(full(Some(32), None));
		assert_eq!((rgb, hsl, block, note), (false, false, false, false));
		// An editable slot with no project: the note explains the missing tracks.
		let p = gradient();
		let no_project = Snapshot::of(&p, &p, Some(70), None, &[], false, false, false, &[], None, false);
		let (rgb, hsl, block, note, _, _) = shown(no_project);
		assert_eq!((rgb, hsl, block, note), (false, false, false, true));
	}

	/// The six absolute tracks read the selected slot: R/G/B in bytes, H in
	/// degrees, S/L as fractions — and a track the user is *dragging* is not
	/// reseeded, or the sync would fight the drag (and round-trip the two
	/// channels it is not touching through HSL every frame).
	#[test]
	fn the_tracks_seed_from_the_selected_slot() {
		let body = Rect::new(0.0, 0.0, 280.0, 460.0);
		let (mut ui, id) = host(full(Some(70), None), body);
		let values = |ui: &Ui| {
			let c = ui.get::<PaletteContent>(id).unwrap();
			c.ids.sliders.map(|s| descendant::<Slider>(&c.root, s).map_or(f32::NAN, Slider::value))
		};
		let rgb = rgb_at(&gradient(), 70);
		let (h, s, l) = rgb_to_hsl(rgb);
		let got = values(&ui);
		assert_eq!(got[0..3], [f32::from(rgb[0]), f32::from(rgb[1]), f32::from(rgb[2])], "R/G/B are bytes");
		assert!((got[3] - h).abs() < 0.5, "H is degrees: {} vs {h}", got[3]);
		assert!((got[4] - s).abs() < 0.01 && (got[5] - l).abs() < 0.01, "S/L are fractions");
		// The row composes back to the colour it came from, both ways round.
		let c = ui.get::<PaletteContent>(id).unwrap();
		assert_eq!(c.composed(0), rgb, "the RGB row round-trips");
		assert_eq!(c.composed(1), rgb, "and so does the HSL row");

		// Mid-drag, a re-sync leaves the tracks alone.
		let track = {
			let c = ui.get::<PaletteContent>(id).unwrap();
			descendant::<Slider>(&c.root, c.ids.sliders[0]).unwrap().rect()
		};
		ui.dispatch(&[press(Vec2::new(track.right() - 1.0, track.center().y), true)]);
		let dragging = values(&ui);
		assert!(dragging[0] > got[0], "the press moved R to the track end");
		ui.get_mut::<PaletteContent>(id).unwrap().sync(full(Some(70), None));
		assert_eq!(values(&ui), dragging, "a sync mid-drag reseeds nothing");
		ui.dispatch(&[press(Vec2::new(track.right() - 1.0, track.center().y), false)]);
		ui.get_mut::<PaletteContent>(id).unwrap().sync(full(Some(70), None));
		assert_eq!(values(&ui), got, "and the released tracks take the palette's word again");
	}

	/// The bare panel is the same tree with the toolbar off, the cycle/static
	/// band on, and every editing row hidden — read-only by construction rather
	/// than by a second oracle.
	#[test]
	fn the_bare_panel_has_no_toolbar_and_never_edits() {
		let body = Rect::new(0.0, 0.0, 320.0, 460.0);
		let p = gradient();
		// Slot 100 is editable *and* a water slot: the full panel would show all
		// three track rows for it.
		let (ui, id) = host(Snapshot::of_bare(&p, &p, Some(100), None, false), body);
		let c = ui.get::<PaletteContent>(id).unwrap();
		assert!(c.band(SLOT_TOOLBAR).is_empty(), "no toolbar");
		assert_eq!(c.band(SLOT_HEADER).h, HEADER_H, "the cycle/static band instead");
		for slot in [c.ids.rgb_row, c.ids.hsl_row, c.ids.block_row, c.ids.note_row] {
			assert!(descendant::<Reveal>(&c.root, slot).map_or(Rect::ZERO, Widget::rect).is_empty(), "no tracks");
		}
		// Its grid starts right below its own band.
		assert!(c.grid().rect.y >= c.band(SLOT_HEADER).bottom());

		// The full panel is the complement: a toolbar, no cycle/static band.
		let (ui, id) = host(full(Some(100), None), body);
		let c = ui.get::<PaletteContent>(id).unwrap();
		assert!(c.band(SLOT_HEADER).is_empty(), "the full panel has no cycle band");
		assert!(c.band(SLOT_TOOLBAR).h >= 2.0 * TAB_H, "tabs row + at least one key run");
	}

	/// A narrow dock re-packs the manager keys onto more runs, and the toolbar
	/// band grows to match — the band is the *complement* of the body, so a run
	/// that did not lengthen it would put keys over the swatches.
	#[test]
	fn the_key_run_wraps_and_the_band_follows() {
		let wide = Rect::new(0.0, 0.0, 300.0, 460.0);
		let narrow = Rect::new(0.0, 0.0, 120.0, 460.0);
		let band = |body: Rect| {
			let (ui, id) = host(full(None, None), body);
			let c = ui.get::<PaletteContent>(id).unwrap();
			(c.band(SLOT_TOOLBAR).h, c.grid().rect.y)
		};
		let (wide_h, wide_top) = band(wide);
		let (narrow_h, narrow_top) = band(narrow);
		assert_eq!(wide_h, 2.0 * TAB_H, "five keys fit one run at 300px");
		assert!(narrow_h > wide_h, "and re-pack onto more at 120px");
		assert_eq!(wide_top, wide_h, "the grid starts where the band ends");
		assert_eq!(narrow_top, narrow_h);
	}

	/// One drag is one undo stroke: the press queues `Begin` and the value it set
	/// (click-to-set), every move that changes the value queues another, and the
	/// release queues exactly one `End`. A doubled `Begin` would split the drag
	/// into two undo entries — silent until someone hits Ctrl+Z.
	#[test]
	fn a_slider_drag_is_one_stroke() {
		let body = Rect::new(0.0, 0.0, 280.0, 460.0);
		let (mut ui, id) = host(full(Some(70), None), body);
		let track = {
			let c = ui.get::<PaletteContent>(id).unwrap();
			descendant::<Slider>(&c.root, c.ids.sliders[0]).expect("the R track is in the tree").rect()
		};
		let drain = |ui: &mut Ui| {
			let mut out = Vec::new();
			while let Some(e) = ui.get_mut::<PaletteContent>(id).unwrap().take_edit() {
				out.push(e);
			}
			out
		};
		let at = |x: f32| Vec2::new(x, track.center().y);

		ui.dispatch(&[press(at(track.x + track.w * 0.25), true)]);
		let opened = drain(&mut ui);
		assert_eq!(opened.iter().filter(|e| **e == Edit::Begin).count(), 1, "exactly one stroke opens");
		assert!(matches!(opened.last(), Some(Edit::Colors(_))), "click-to-set applies at the press");

		ui.dispatch(&[Event::PointerMoved { pos: at(track.x + track.w * 0.75) }]);
		let moved = drain(&mut ui);
		assert!(moved.iter().all(|e| matches!(e, Edit::Colors(_))), "a move only re-colours: {moved:?}");
		assert!(!moved.is_empty(), "and it did move");
		// The R channel rose and the other two are untouched.
		let (Some(Edit::Colors(first)), Some(Edit::Colors(last))) = (opened.last().cloned(), moved.last().cloned())
		else {
			panic!("both ends are colour edits")
		};
		assert_eq!(first.len(), 1, "one slot selected -> one slot written");
		assert!(last[0].1[0] > first[0].1[0], "dragging right raises R");
		assert_eq!(last[0].1[1..], first[0].1[1..], "G and B are untouched");

		ui.dispatch(&[press(at(track.x + track.w * 0.75), false)]);
		assert_eq!(drain(&mut ui), vec![Edit::End], "the release closes it once");
	}

	/// A block bar shifts the whole water block from the baseline it captured at
	/// the press, so a long drag cannot compound its own rounding — and it is one
	/// stroke by the same rule the sliders follow.
	#[test]
	fn a_block_drag_shifts_the_whole_block_from_one_baseline() {
		let body = Rect::new(0.0, 0.0, 280.0, 460.0);
		let (mut ui, id) = host(full(Some(100), None), body);
		let bar = {
			let c = ui.get::<PaletteContent>(id).unwrap();
			descendant::<BlockBar>(&c.root, c.ids.blocks[0]).expect("the H bar is in the tree").rect()
		};
		let at = |x: f32| Vec2::new(x, bar.center().y);
		let drain = |ui: &mut Ui| {
			let mut out = Vec::new();
			while let Some(e) = ui.get_mut::<PaletteContent>(id).unwrap().take_edit() {
				out.push(e);
			}
			out
		};

		ui.dispatch(&[press(at(bar.center().x), true)]);
		assert_eq!(drain(&mut ui), vec![Edit::Begin], "the press only opens the stroke - dx is still 0");

		ui.dispatch(&[Event::PointerMoved { pos: at(bar.center().x + 10.0) }]);
		let moved = drain(&mut ui);
		let Some(Edit::Colors(shifted)) = moved.last().cloned() else { panic!("a move re-colours: {moved:?}") };
		// Slot 100's block is 96..=102 - the whole gradient moves, not one slot.
		assert_eq!(shifted.iter().map(|&(s, _)| s).collect::<Vec<_>>(), (96..=102).collect::<Vec<_>>());

		// Twice as far is twice the hue shift off the *same* baseline.
		ui.dispatch(&[Event::PointerMoved { pos: at(bar.center().x + 20.0) }]);
		let Some(Edit::Colors(further)) = drain(&mut ui).last().cloned() else { panic!("still re-colouring") };
		let p = gradient();
		let hue = |rgb: [u8; 3]| rgb_to_hsl(rgb).0;
		let base = hue(rgb_at(&p, 96));
		let one = (hue(shifted[0].1) - base).rem_euclid(360.0);
		let two = (hue(further[0].1) - base).rem_euclid(360.0);
		assert!((two - 2.0 * one).abs() < 2.0, "10px -> {one}°, 20px -> {two}° (no compounding)");

		ui.dispatch(&[press(at(bar.center().x + 20.0), false)]);
		assert_eq!(drain(&mut ui), vec![Edit::End]);
	}

	/// A Ctrl-built multi selection makes an absolute track write *every* slot in
	/// the set, and the swatches in it wear the theme's own selected face.
	#[test]
	fn an_absolute_track_writes_the_whole_multi_set() {
		let body = Rect::new(0.0, 0.0, 280.0, 460.0);
		let p = gradient();
		let snap = Snapshot::of(&p, &p, Some(70), None, &[70, 80, 90], false, true, false, &[], None, false);
		let (mut ui, id) = host(snap, body);
		{
			let c = ui.get::<PaletteContent>(id).unwrap();
			let grid = c.grid();
			assert!(grid.boxes[70].selected() && grid.boxes[90].selected(), "the multi set reads chosen");
			assert!(!grid.boxes[71].selected(), "and only it");
		}
		let track = {
			let c = ui.get::<PaletteContent>(id).unwrap();
			descendant::<Slider>(&c.root, c.ids.sliders[0]).unwrap().rect()
		};
		ui.dispatch(&[press(Vec2::new(track.x + track.w * 0.9, track.center().y), true)]);
		let mut written = Vec::new();
		while let Some(e) = ui.get_mut::<PaletteContent>(id).unwrap().take_edit() {
			if let Edit::Colors(v) = e {
				written = v;
			}
		}
		assert_eq!(written.iter().map(|&(s, _)| s).collect::<Vec<_>>(), vec![70, 80, 90]);
		assert!(written.windows(2).all(|w| w[0].1 == w[1].1), "all set to the same colour");
	}

	/// The saved tab replaces the grid, each row loads its palette, and an empty
	/// list explains itself instead of showing rows.
	#[test]
	fn the_saved_tab_swaps_the_body_and_loads_a_row() {
		let body = Rect::new(0.0, 0.0, 300.0, 460.0);
		let p = gradient();
		let names: Vec<String> = (0..4).map(|i| format!("pal-{i}")).collect();
		let (mut ui, id) = host(Snapshot::of(&p, &p, None, None, &[], false, true, true, &names, Some(0), true), body);
		{
			let c = ui.get::<PaletteContent>(id).unwrap();
			assert!(
				descendant::<SwatchGrid>(&c.root, c.ids.grid).is_none(),
				"the saved tab takes the grid out of the tree entirely"
			);
			let list = descendant::<List>(&c.root, c.ids.list).expect("the list is shown");
			assert_eq!(list.len(), 4);
			assert!(!list.rect().is_empty());
		}
		// A press on row 2 loads it - on the press, like every list pick.
		let row = {
			let c = ui.get::<PaletteContent>(id).unwrap();
			let list = descendant::<List>(&c.root, c.ids.list).unwrap().rect();
			Vec2::new(list.x + 4.0, list.y + list.h * 5.0 / 8.0)
		};
		ui.dispatch(&[press(row, true)]);
		assert_eq!(ui.actions().iter().copied().find_map(action_of), Some(Action::LoadSaved(2)));
		// Clicking the row that is *already* selected loads it again — a selection
		// watch alone would go quiet here, and re-loading a palette you have
		// wandered away from is the whole point of the list.
		ui.dispatch(&[press(row, false)]);
		ui.dispatch(&[press(row, true)]);
		assert_eq!(
			ui.actions().iter().copied().find_map(action_of),
			Some(Action::LoadSaved(2)),
			"the same row reloads"
		);

		// No saved palettes: the note, no list.
		let (ui, id) = host(Snapshot::of(&p, &p, None, None, &[], false, true, true, &[], None, false), body);
		let c = ui.get::<PaletteContent>(id).unwrap();
		assert!(descendant::<List>(&c.root, c.ids.list).is_none(), "an empty list is out of the tree");
		assert!(!descendant::<Reveal>(&c.root, c.ids.empty_note).unwrap().rect().is_empty(), "the note shows");
	}

	/// Edit and Delete need a selected *user* palette; without one they are
	/// **disabled-dead** with the reason as their tooltip - the shared
	/// header-key convention ([`crate::panel_ui`], superseding G4's
	/// muted-but-live rule). The tabs and the animate toggle latch on the state
	/// they mean.
	#[test]
	fn the_toolbar_reads_the_state_it_means() {
		let body = Rect::new(0.0, 0.0, 300.0, 460.0);
		let p = gradient();
		let weights = |sel_is_user: bool, show_saved: bool, cycling: bool| {
			let snap = Snapshot::of(&p, &p, None, None, &[], cycling, true, show_saved, &[], None, sel_is_user);
			let (ui, id) = host(snap, body);
			let c = ui.get::<PaletteContent>(id).unwrap();
			let key = |i: usize| descendant::<Button>(&c.root, c.ids.cmds[i]).unwrap().is_disabled();
			let tip = |i: usize| {
				wgpu_ui::Widget::tooltip(descendant::<Button>(&c.root, c.ids.cmds[i]).unwrap()).map(str::to_string)
			};
			let tab = |i: usize| descendant::<Button>(&c.root, c.ids.tabs[i]).unwrap().selected();
			let anim = descendant::<Button>(&c.root, c.ids.animate).unwrap().selected();
			([key(0), key(1), key(2), key(3), key(4)], tip(1), [tab(0), tab(1)], anim)
		};
		let (dead, tip, tabs, anim) = weights(false, false, false);
		assert_eq!(dead, [false, true, true, false, false], "only edit + del need a user palette");
		assert_eq!(tip.as_deref(), Some("needs a saved palette selected"), "a dead key says why on hover");
		assert_eq!(tabs, [true, false], "the grid tab is the live one");
		assert!(!anim);
		let (dead, tip, tabs, anim) = weights(true, true, true);
		assert_eq!(dead, [false; 5], "a user palette lights them all");
		assert_eq!(tip, None, "a live key carries no tooltip");
		assert_eq!(tabs, [false, true]);
		assert!(anim);
	}

	/// The bare panel's cycle/static pair is a two-key segmented band: exactly
	/// one reads selected, and each fires its own side of `animate on|off`.
	#[test]
	fn the_bare_header_keys_toggle_cycling() {
		let body = Rect::new(0.0, 0.0, 320.0, 460.0);
		let p = gradient();
		for cycling in [false, true] {
			let (mut ui, id) = host(Snapshot::of_bare(&p, &p, None, None, cycling), body);
			let (on, off) = {
				let c = ui.get::<PaletteContent>(id).unwrap();
				let key = |i: usize| descendant::<Button>(&c.root, c.ids.cycle[i]).unwrap();
				assert_eq!(key(0).selected(), cycling);
				assert_eq!(key(1).selected(), !cycling);
				(key(0).rect(), key(1).rect())
			};
			// The keys are ordinary chrome: they arm on the press and fire on a
			// release inside.
			for (r, want) in [(on, Action::Cycle(true)), (off, Action::Cycle(false))] {
				let p = r.center();
				ui.dispatch(&[press(p, true)]);
				assert_eq!(ui.actions().iter().copied().find_map(action_of), None, "a key only arms at the press");
				ui.dispatch(&[press(p, false)]);
				assert_eq!(ui.actions().iter().copied().find_map(action_of), Some(want));
			}
			// A release somewhere else cancels the arm — and does not select the
			// swatch it landed on either, because press-fire commits at the press.
			let away = {
				let c = ui.get::<PaletteContent>(id).unwrap();
				c.grid().slot_rect(10).center()
			};
			ui.dispatch(&[press(on.center(), true)]);
			ui.dispatch(&[press(away, false)]);
			assert_eq!(ui.actions().iter().copied().find_map(action_of), None, "dragging off a key cancels the click");
		}
	}

	/// The swatches are **re-synced, not rebuilt**: their ids survive a frame in
	/// which every colour and the whole selection changed. A rebuilt grid would
	/// mint new ids and hover would die with them.
	#[test]
	fn the_swatches_keep_their_ids_across_a_resync() {
		let body = Rect::new(0.0, 0.0, 260.0, 460.0);
		let (mut ui, id) = host(full(None, None), body);
		let before: Vec<WidgetId> =
			ui.get::<PaletteContent>(id).unwrap().grid().boxes.iter().map(ColorButton::id).collect();
		let mut other = gradient();
		other.rotate_left(3);
		let snap = Snapshot::of(&other, &other, Some(64), Some(95), &[70], true, true, false, &[], None, false);
		ui.get_mut::<PaletteContent>(id).unwrap().sync(snap);
		let after: Vec<WidgetId> =
			ui.get::<PaletteContent>(id).unwrap().grid().boxes.iter().map(ColorButton::id).collect();
		assert_eq!(before, after, "the ids are the same widgets");
		let grid = ui.get::<PaletteContent>(id).unwrap().grid();
		assert_eq!(grid.boxes[0].color(), Rgba::rgb(other[0], other[1], other[2]), "the colours followed");
		assert_eq!(grid.range, Some((64, 95)));
	}

	/// The panel draws its two materials — the chrome band and the strip plate —
	/// under a tree that carries neither, and draws nothing at all on the overlay
	/// pass (it is base-pass chrome).
	#[test]
	fn the_panel_draws_its_bands_and_nothing_on_the_overlay() {
		use wgpu_ui::widget::DrawPass;
		let body = Rect::new(0.0, 0.0, 260.0, 460.0);
		let (ui, id) = host(full(Some(64), None), body);
		let chrome = skin();
		let mut base = DrawList::new();
		ui.draw(&mut base, chrome.theme(), chrome.fonts());
		assert!(!base.cmds.is_empty());
		let content = ui.get::<PaletteContent>(id).unwrap();
		let ctx = DrawCtx {
			fonts: chrome.fonts(),
			theme: chrome.theme(),
			scale: 1.0,
			hovered: WidgetId::NONE,
			focused: WidgetId::NONE,
			pass: DrawPass::Overlay,
		};
		let mut over = DrawList::new();
		content.draw(&mut over, &ctx);
		assert!(over.cmds.is_empty(), "no overlay-pass drawing");
	}
}
