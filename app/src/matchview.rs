//! Match-editor view widgets: the grouped tile lists, the adjacency cross,
//! and the orientation picker — dumb wgpu-ui bricks over the model in
//! [`crate::matcheditor`].
//!
//! Each widget draws from data the dialog re-syncs every frame (row
//! view-models, uv rects into the shared rest-palette tile atlas, and a small
//! strip texture holding the main tile + the candidate at all 8 orientations)
//! and reports pointer gestures through drained queues — the dialog applies
//! them to the model, exactly like the Tile Painter's canvas.

use wgpu_ui::widget::{DrawCtx, EventCtx, LayoutCtx};
use wgpu_ui::{
	DrawList, Emboss, Event, PointerButton, Rect, Rgba, Size, TexRect, TextRole, TextureId, Vec2, Widget, WidgetId,
};

use crate::matcheditor::RowTone;
use crate::theme;
use crate::uikit_theme::rgba;

pub(crate) const ROW_H: f32 = 20.0;
pub(crate) const LIST_W: f32 = 168.0;
const THUMB: f32 = 16.0;
/// Left gutter for the group fold glyph; member tiles nest a little further in.
const FOLD_W: f32 = 12.0;
/// Right-side reserve for the candidate-list match tag (e.g. `"NES"`).
const TAG_W: f32 = 34.0;
/// Orientation-preview cell (a mini cross is 3×3 of these).
const MINI: f32 = 16.0;

fn tone_color(tone: RowTone) -> Rgba {
	match tone {
		RowTone::Select => rgba(theme::ACCENT),
		RowTone::Warn => rgba(theme::MATCH_WARN),
		RowTone::Rule => rgba(theme::MATCH_RULE),
		RowTone::Plain => rgba(theme::INK),
	}
}

/// The strip texture's uv for cell `i` of `n` (main tile at 0, the candidate's
/// 8 orientations at 1..=8).
pub(crate) fn strip_uv(i: usize, n: usize) -> TexRect {
	let w = 1.0 / n as f32;
	TexRect { u0: i as f32 * w, v0: 0.0, u1: (i + 1) as f32 * w, v1: 1.0 }
}

// ----- RowList ------------------------------------------------------------------

/// One display row of a [`RowList`], rebuilt from the model each frame.
pub(crate) struct ListRow {
	pub label: String,
	/// Right-aligned matched-direction tag (candidate list only; "" = none).
	pub tag: String,
	/// Thumbnail uv into the shared tile atlas.
	pub thumb: Option<TexRect>,
	/// Header rows ([ungrouped] / a group) carry the fold glyph.
	pub header: bool,
	pub collapsed: bool,
	pub tone: RowTone,
	pub selected: bool,
}

/// A grouped tile list (or the groups panel): full-height rows inside a
/// [`wgpu_ui::ScrollArea`]. A press reports `(row, in_fold_gutter)`;
/// the dialog maps it to select / collapse on the model.
pub(crate) struct RowList {
	id: WidgetId,
	tex: TextureId,
	rows: Vec<ListRow>,
	width: f32,
	clicked: Option<(usize, bool)>,
	rect: Rect,
}

impl RowList {
	pub fn new(tex: TextureId, width: f32) -> Self {
		Self { id: wgpu_ui::interact::next_id(), tex, rows: Vec::new(), width, clicked: None, rect: Rect::ZERO }
	}

	pub fn id(&self) -> WidgetId {
		self.id
	}

	pub fn set_rows(&mut self, rows: Vec<ListRow>) {
		self.rows = rows;
	}

	/// The last press: `(row index, hit the header fold gutter)`.
	pub fn take_clicked(&mut self) -> Option<(usize, bool)> {
		self.clicked.take()
	}
}

impl Widget for RowList {
	fn measure(&mut self, _avail: Size, _ctx: &mut LayoutCtx) -> Size {
		Size::new(self.width, self.rows.len() as f32 * ROW_H)
	}

	fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
		self.rect = rect;
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		for (i, row) in self.rows.iter().enumerate() {
			let r = Rect::new(self.rect.x, self.rect.y + i as f32 * ROW_H, self.rect.w, ROW_H);
			if row.selected {
				ctx.theme.accent_row(dl, r, 0.40);
			}
			let mut x = r.x + 3.0;
			if row.header {
				let glyph = if row.collapsed { "+" } else { "-" };
				ctx.theme.text_colored(
					dl,
					ctx.fonts,
					Vec2::new(x, r.y + 14.0),
					glyph,
					TextRole::Small,
					Emboss::Engraved,
					rgba(theme::INK_DIM),
				);
			}
			x = r.x + FOLD_W + if row.header { 2.0 } else { 12.0 };
			if let Some(uv) = row.thumb {
				dl.image(self.tex, Rect::new(x, r.y + 2.0, THUMB, THUMB), uv, Rgba::WHITE);
				x += THUMB + 4.0;
			} else {
				x += 4.0;
			}
			let mut reserve = 2.0;
			if !row.tag.is_empty() {
				ctx.theme.text_colored(
					dl,
					ctx.fonts,
					Vec2::new(r.x + r.w - TAG_W, r.y + 14.0),
					&row.tag,
					TextRole::Small,
					Emboss::Engraved,
					rgba(theme::ACCENT),
				);
				reserve = TAG_W;
			}
			dl.push_clip(Rect::new(x, r.y, (r.x + r.w - reserve - x).max(0.0), r.h));
			ctx.theme.text_colored(
				dl,
				ctx.fonts,
				Vec2::new(x, r.y + 14.0),
				&row.label,
				TextRole::Small,
				Emboss::Engraved,
				tone_color(row.tone),
			);
			dl.pop_clip();
		}
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		if let Event::PointerButton { button: PointerButton::Primary, pressed: true, pos, .. } = ev {
			if ctx.is_target(self.id) {
				let i = ((pos.y - self.rect.y) / ROW_H).floor();
				if i >= 0.0 && (i as usize) < self.rows.len() {
					let i = i as usize;
					let fold = self.rows[i].header && pos.x < self.rect.x + FOLD_W + 2.0;
					self.clicked = Some((i, fold));
					ctx.fire(self.id, None);
				}
				ctx.consume_pointer();
				return true;
			}
		}
		false
	}

	fn rect(&self) -> Rect {
		self.rect
	}

	fn id(&self) -> WidgetId {
		self.id
	}
}

// ----- CrossView ----------------------------------------------------------------

/// One cross side's display state, synced from the model each frame.
#[derive(Clone, Copy, Default)]
pub(crate) struct SideView {
	/// `Some(true)`=water, `Some(false)`=land wildcard fill; `None` = tile.
	pub wildcard: Option<bool>,
	/// Draw the matched external highlight.
	pub matched: bool,
}

/// A press on the cross: which side, and whether it was the primary button
/// (toggle match) or secondary (cycle the wildcard).
pub(crate) struct CrossPress {
	pub dir: usize,
	pub primary: bool,
}

/// The borderless adjacency cross: main tile centre, candidate on the four
/// sides (at the picked orientation), wildcard sides as water/land fills.
pub(crate) struct CrossView {
	id: WidgetId,
	tex: TextureId,
	/// Screen px per cell (16 × the dialog's cross size).
	cell: f32,
	sides: [SideView; 4],
	/// uv of the candidate at the current orientation (strip cell 1+bits).
	cand_uv: TexRect,
	main_uv: TexRect,
	presses: Vec<CrossPress>,
	rect: Rect,
}

impl CrossView {
	pub fn new(tex: TextureId, cell: f32) -> Self {
		Self {
			id: wgpu_ui::interact::next_id(),
			tex,
			cell,
			sides: [SideView::default(); 4],
			cand_uv: TexRect::FULL,
			main_uv: TexRect::FULL,
			presses: Vec::new(),
			rect: Rect::ZERO,
		}
	}

	pub fn id(&self) -> WidgetId {
		self.id
	}

	pub fn set_cell(&mut self, cell: f32) {
		self.cell = cell;
	}

	pub fn set_state(&mut self, sides: [SideView; 4], main_uv: TexRect, cand_uv: TexRect) {
		self.sides = sides;
		self.main_uv = main_uv;
		self.cand_uv = cand_uv;
	}

	pub fn take_presses(&mut self) -> Vec<CrossPress> {
		std::mem::take(&mut self.presses)
	}

	fn cell_rect(&self, row: usize, col: usize) -> Rect {
		Rect::new(self.rect.x + col as f32 * self.cell, self.rect.y + row as f32 * self.cell, self.cell, self.cell)
	}

	fn side_rect(&self, dir: usize) -> Rect {
		match dir {
			0 => self.cell_rect(0, 1),
			1 => self.cell_rect(1, 2),
			2 => self.cell_rect(2, 1),
			_ => self.cell_rect(1, 0),
		}
	}
}

/// The matched-side highlight: a 2px accent band on the three outward edges
/// (the edge facing the centre stays open, so the pair reads as touching).
fn external_highlight(dl: &mut DrawList, r: Rect, dir: usize) {
	let t = 2.0;
	let green = rgba(theme::ACCENT);
	let edges: [(Rect, bool); 4] = [
		(Rect::new(r.x, r.y, r.w, t), dir != 2),
		(Rect::new(r.x, r.y + r.h - t, r.w, t), dir != 0),
		(Rect::new(r.x, r.y, t, r.h), dir != 1),
		(Rect::new(r.x + r.w - t, r.y, t, r.h), dir != 3),
	];
	for (rect, show) in edges {
		if show {
			dl.fill_rect(rect, green);
		}
	}
}

impl Widget for CrossView {
	fn measure(&mut self, _avail: Size, _ctx: &mut LayoutCtx) -> Size {
		Size::new(3.0 * self.cell, 3.0 * self.cell)
	}

	fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
		self.rect = rect;
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		let center = self.cell_rect(1, 1);
		dl.image(self.tex, center, self.main_uv, Rgba::WHITE);
		for dir in 0..4 {
			let r = self.side_rect(dir);
			match self.sides[dir].wildcard {
				Some(water) => {
					dl.fill_rect(r, rgba(if water { theme::MATCH_WATER } else { theme::MATCH_LAND }));
					ctx.theme.text_colored(
						dl,
						ctx.fonts,
						Vec2::new(r.x + 3.0, r.y + 13.0),
						if water { "WTR" } else { "LND" },
						TextRole::Small,
						Emboss::Engraved,
						rgba(theme::INK),
					);
				}
				None => {
					dl.image(self.tex, r, self.cand_uv, Rgba::WHITE);
					if self.sides[dir].matched {
						external_highlight(dl, r, dir);
					}
				}
			}
		}
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		if let Event::PointerButton { button, pressed: true, pos, .. } = ev {
			let primary = matches!(button, PointerButton::Primary);
			if (primary || matches!(button, PointerButton::Secondary)) && ctx.is_target(self.id) {
				for dir in 0..4 {
					if self.side_rect(dir).contains(*pos) {
						self.presses.push(CrossPress { dir, primary });
						ctx.fire(self.id, None);
						break;
					}
				}
				ctx.consume_pointer();
				return true;
			}
		}
		false
	}

	fn rect(&self) -> Rect {
		self.rect
	}

	fn id(&self) -> WidgetId {
		self.id
	}
}

// ----- OrientPicker ---------------------------------------------------------------

/// The eight mini-cross orientation previews (2 rows × 4). Each shows the main
/// tile centred with the candidate at that orientation on the non-wildcard
/// sides, matched sides highlighted; the picked orientation is ring-marked.
/// A click fires and reports the picked index.
pub(crate) struct OrientPicker {
	id: WidgetId,
	tex: TextureId,
	sel: usize,
	main_uv: TexRect,
	/// Per orientation `k`, per side: `None` = wildcarded (hidden), else
	/// `(uv of cand@k, matched)`.
	sides: [[Option<(TexRect, bool)>; 4]; 8],
	picked: Option<usize>,
	rect: Rect,
}

const CELL: f32 = MINI * 3.0;
const PITCH: f32 = CELL + 10.0;

impl OrientPicker {
	pub fn new(tex: TextureId) -> Self {
		Self {
			id: wgpu_ui::interact::next_id(),
			tex,
			sel: 0,
			main_uv: TexRect::FULL,
			sides: [[None; 4]; 8],
			picked: None,
			rect: Rect::ZERO,
		}
	}

	pub fn id(&self) -> WidgetId {
		self.id
	}

	pub fn set_state(&mut self, sel: usize, main_uv: TexRect, sides: [[Option<(TexRect, bool)>; 4]; 8]) {
		self.sel = sel.min(7);
		self.main_uv = main_uv;
		self.sides = sides;
	}

	pub fn take_picked(&mut self) -> Option<usize> {
		self.picked.take()
	}

	fn preview_rect(&self, k: usize) -> Rect {
		let (row, col) = (k / 4, k % 4);
		Rect::new(self.rect.x + col as f32 * PITCH, self.rect.y + row as f32 * (PITCH + 4.0), CELL, CELL)
	}
}

impl Widget for OrientPicker {
	fn measure(&mut self, _avail: Size, _ctx: &mut LayoutCtx) -> Size {
		// Row 1 starts at PITCH+4; both rows are CELL tall.
		Size::new(4.0 * PITCH - 10.0, PITCH + 4.0 + CELL)
	}

	fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
		self.rect = rect;
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		for k in 0..8 {
			let r = self.preview_rect(k);
			ctx.theme.well(dl, r, wgpu_ui::WidgetState::default());
			let cell = |row: usize, col: usize| Rect::new(r.x + col as f32 * MINI, r.y + row as f32 * MINI, MINI, MINI);
			dl.image(self.tex, cell(1, 1), self.main_uv, Rgba::WHITE);
			for dir in 0..4 {
				let Some((uv, matched)) = self.sides[k][dir] else { continue };
				let cr = match dir {
					0 => cell(0, 1),
					1 => cell(1, 2),
					2 => cell(2, 1),
					_ => cell(1, 0),
				};
				dl.image(self.tex, cr, uv, Rgba::WHITE);
				if matched {
					external_highlight(dl, cr, dir);
				}
			}
			if k == self.sel {
				let green = rgba(theme::ACCENT);
				dl.stroke_rect(r, 1.0, green);
				dl.stroke_rect(Rect::new(r.x + 1.0, r.y + 1.0, r.w - 2.0, r.h - 2.0), 1.0, green);
			}
		}
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		if let Event::PointerButton { button: PointerButton::Primary, pressed: true, pos, .. } = ev {
			if ctx.is_target(self.id) {
				for k in 0..8 {
					if self.preview_rect(k).contains(*pos) {
						self.picked = Some(k);
						self.sel = k;
						ctx.fire(self.id, None);
						break;
					}
				}
				ctx.consume_pointer();
				return true;
			}
		}
		false
	}

	fn rect(&self) -> Rect {
		self.rect
	}

	fn id(&self) -> WidgetId {
		self.id
	}
}
