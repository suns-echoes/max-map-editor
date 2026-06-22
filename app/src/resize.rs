//! Resize Map modal (design: features.drawio "Map Resize
//! Modal"). GIMP-style canvas resize: new W×H text fields plus a 3×3
//! alignment anchor that places the existing map within the new bounds -
//! enlarging fills the rest with water, shrinking crops to the anchored
//! window. Resolves to a `resize W H OFFX OFFY` command (the same path
//! scripts use); the offset is derived from the anchor.
//!
//! Pure state/geometry, drawn through the shared [`UiQuads`].

use crate::newmap::SIZE_PRESETS;
use crate::textinput::{Charset, TextInput};
use crate::theme;
use crate::ui::{self, Hot, Rect, UiQuads};

const W: f32 = 300.0;
const TITLE_H: f32 = 22.0;
const ROW_H: f32 = 24.0;
const FIELD_W: f32 = 56.0;
const BTN_H: f32 = 22.0;
const ANCHOR_CELL: f32 = 26.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
	Width,
	Height,
}

pub struct Resize {
	pub width: TextInput,
	pub height: TextInput,
	/// Current map size (for the offset math + preview).
	old_w: u16,
	old_h: u16,
	/// Anchor column/row in 0..3 (1,1 = center).
	col: u8,
	row: u8,
	/// The size-preset dropdown's open state.
	preset_open: bool,
	focus: Option<Field>,
	/// The field being mouse-drag-selected (press..release).
	drag_field: Option<Field>,
	/// A command button held down, waiting for release-inside
	/// - dragging off cancels.
	armed: Option<ArmedBtn>,
	/// Drag offset from centered (draggable by the titlebar).
	pub(crate) drag_offset: (f32, f32),
}

/// The deferred command buttons (anchor/fields stay press-fired).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArmedBtn {
	Abort,
	Confirm,
}

#[derive(Debug, PartialEq)]
pub enum Press {
	Consumed,
	Abort,
	/// Validated `resize …` command line.
	Resize(String),
	Invalid(String),
}

impl Resize {
	pub fn new(old_w: u16, old_h: u16) -> Self {
		Self {
			width: TextInput::new(&old_w.to_string(), 4).charset(Charset::Digits),
			height: TextInput::new(&old_h.to_string(), 4).charset(Charset::Digits),
			old_w,
			old_h,
			col: 1,
			row: 1,
			preset_open: false,
			focus: None,
			drag_field: None,
			armed: None,
			drag_offset: (0.0, 0.0),
		}
	}

	fn field_mut(&mut self, f: Field) -> &mut TextInput {
		match f {
			Field::Width => &mut self.width,
			Field::Height => &mut self.height,
		}
	}

	fn field_ref(&self, f: Field) -> &TextInput {
		match f {
			Field::Width => &self.width,
			Field::Height => &self.height,
		}
	}

	fn parsed(&self) -> Option<(u16, u16)> {
		Some((self.width.text().parse().ok()?, self.height.text().parse().ok()?))
	}

	/// Offset of the old map inside the new bounds from the 3×3 anchor:
	/// col/row 0 = top/left edge, 1 = centered, 2 = bottom/right edge.
	fn offset(&self, new_w: u16, new_h: u16) -> (i32, i32) {
		let ox = self.col as i32 * (new_w as i32 - self.old_w as i32) / 2;
		let oy = self.row as i32 * (new_h as i32 - self.old_h as i32) / 2;
		(ox, oy)
	}

	pub fn command(&self) -> Result<String, String> {
		let (w, h) = self.parsed().ok_or("size is not a number")?;
		if !(1..=1024).contains(&w) || !(1..=1024).contains(&h) {
			return Err(format!("bad size {w}×{h} (1..=1024)"));
		}
		let (ox, oy) = self.offset(w, h);
		Ok(format!("resize {w} {h} {ox} {oy}"))
	}

	/// The preset whose dimensions match the current W×H fields, if any.
	fn preset_match(&self) -> Option<usize> {
		let (w, h) = self.parsed()?;
		SIZE_PRESETS.iter().position(|&(_, pw, ph)| w == pw && h == ph)
	}

	/// The dropdown's value label: the matching preset's name, else `Custom`.
	fn preset_label(&self) -> &'static str {
		self.preset_match().map(|i| SIZE_PRESETS[i].0).unwrap_or("Custom")
	}

	// ----- geometry ----------------------------------------------------------

	fn height(&self) -> f32 {
		TITLE_H + 8.0 + 2.0 * (ROW_H + 8.0) + 3.0 * ANCHOR_CELL + 16.0 + BTN_H + 12.0
	}

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		let dh = self.height();
		Rect::centered(w, h, W, dh).translate(self.drag_offset.0, self.drag_offset.1)
	}

	/// Size-preset cycle button (row 0), above the size fields.
	fn preset_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + 70.0, d.y + TITLE_H + 8.0, 170.0, BTN_H)
	}

	fn field_rect(&self, d: Rect, f: Field) -> Rect {
		let y = d.y + TITLE_H + 8.0 + (ROW_H + 8.0);
		match f {
			Field::Width => Rect::new(d.x + 70.0, y, FIELD_W, BTN_H),
			Field::Height => Rect::new(d.x + 70.0 + FIELD_W + 26.0, y, FIELD_W, BTN_H),
		}
	}

	fn anchor_origin(&self, d: Rect) -> (f32, f32) {
		(d.x + 70.0, d.y + TITLE_H + 8.0 + 2.0 * (ROW_H + 8.0))
	}

	fn anchor_cell(&self, d: Rect, col: u8, row: u8) -> Rect {
		let (ox, oy) = self.anchor_origin(d);
		Rect::new(ox + col as f32 * ANCHOR_CELL, oy + row as f32 * ANCHOR_CELL, ANCHOR_CELL - 2.0, ANCHOR_CELL - 2.0)
	}

	fn abort_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + 10.0, d.y + d.h - BTN_H - 10.0, 90.0, BTN_H)
	}

	fn confirm_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + d.w - 100.0, d.y + d.h - BTN_H - 10.0, 90.0, BTN_H)
	}

	// ----- events -------------------------------------------------------------

	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		// Size-preset dropdown: the box toggles; an option rewrites the fields.
		match crate::select::hit(self.preset_rect(d), self.preset_open, SIZE_PRESETS.len(), false, x, y) {
			Some(crate::select::Hit::Box) => {
				self.preset_open = !self.preset_open;
				return Press::Consumed;
			}
			Some(crate::select::Hit::Option(i)) => {
				let (_, pw, ph) = SIZE_PRESETS[i];
				self.width.set_text(&pw.to_string());
				self.height.set_text(&ph.to_string());
				self.preset_open = false;
				return Press::Consumed;
			}
			None if self.preset_open => {
				// A click off an open list closes it (and is consumed).
				self.preset_open = false;
				return Press::Consumed;
			}
			None => {}
		}
		for f in [Field::Width, Field::Height] {
			let r = self.field_rect(d, f);
			if r.contains(x, y) {
				self.focus = Some(f);
				self.drag_field = Some(f);
				self.field_mut(f).on_press(x, y, r);
				return Press::Consumed;
			}
		}
		for row in 0..3 {
			for col in 0..3 {
				if self.anchor_cell(d, col, row).contains(x, y) {
					self.col = col;
					self.row = row;
					return Press::Consumed;
				}
			}
		}
		// Abort/Resize arm and fire on release-inside.
		if self.abort_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Abort);
			return Press::Consumed;
		}
		if self.confirm_rect(d).contains(x, y) {
			self.armed = Some(ArmedBtn::Confirm);
			return Press::Consumed;
		}
		if !d.contains(x, y) {
			return Press::Abort;
		}
		self.focus = None;
		Press::Consumed
	}

	/// Fire the armed command button if the release is still on it;
	/// a release anywhere else just disarms.
	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		self.drag_field = None;
		let d = self.dialog_rect(w, h);
		match self.armed.take() {
			Some(ArmedBtn::Abort) if self.abort_rect(d).contains(x, y) => Press::Abort,
			Some(ArmedBtn::Confirm) if self.confirm_rect(d).contains(x, y) => self.confirm(),
			_ => Press::Consumed,
		}
	}

	/// Mouse drag extends the active field's selection (after a press on it).
	pub fn on_drag(&mut self, x: f32, y: f32, w: f32, h: f32) {
		if let Some(f) = self.drag_field {
			let r = self.field_rect(self.dialog_rect(w, h), f);
			self.field_mut(f).on_drag(x, y, r);
		}
	}

	/// The focused W/H field's edit state.
	pub fn edit_context(&self) -> Option<crate::modal::EditContext> {
		let f = self.field_ref(self.focus?);
		Some(f.edit_context())
	}

	/// Route an editing key to the focused field.
	pub fn key(&mut self, key: &crate::modal::ModalKey) {
		if let Some(f) = self.focus {
			self.field_mut(f).on_key(key);
		}
	}

	/// Tab toggles focus between the width and height fields.
	pub fn focus_next(&mut self) {
		self.focus = Some(match self.focus {
			Some(Field::Width) => Field::Height,
			_ => Field::Width,
		});
	}

	pub fn confirm(&self) -> Press {
		match self.command() {
			Ok(line) => Press::Resize(line),
			Err(e) => Press::Invalid(e),
		}
	}

	// ----- drawing -------------------------------------------------------------

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		ui::modal_scrim(&mut q, w, h);
		ui::modal_frame(&mut q, d, "Resize Map", TITLE_H, w, h);

		// Size-preset dropdown (Classic / Mega / Giga); the popup is drawn last.
		let pr = self.preset_rect(d);
		q.label("preset", d.x + 10.0, pr.y + 5.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		crate::select::draw_box(&mut q, pr, self.preset_label(), self.preset_open, w, h, hot);

		// Size fields.
		q.label(
			"size",
			d.x + 10.0,
			self.field_rect(d, Field::Width).y + 5.0,
			crate::ui::FONT_SMALL,
			w,
			h,
			theme::INK_DIM,
		);
		for f in [Field::Width, Field::Height] {
			let r = self.field_rect(d, f);
			q.field(r, w, h);
			if self.focus == Some(f) {
				q.border(r, w, h, theme::INK);
			}
		}
		let xr = self.field_rect(d, Field::Width);
		q.label_in(
			"x",
			Rect::new(xr.x + FIELD_W + 8.0, xr.y, 12.0, BTN_H),
			0.0,
			crate::ui::FONT_SMALL,
			w,
			h,
			theme::INK_DIM,
		);

		// 3×3 anchor grid: the selected cell is bright; the offset note
		// summarizes what fills or crops.
		q.label(
			"align",
			d.x + 10.0,
			self.anchor_origin(d).1 + ANCHOR_CELL,
			crate::ui::FONT_SMALL,
			w,
			h,
			theme::INK_DIM,
		);
		for row in 0..3 {
			for col in 0..3 {
				let r = self.anchor_cell(d, col, row);
				let on = col == self.col && row == self.row;
				q.button_active(r, w, h, on, hot);
				if on {
					q.rect(Rect::new(r.x + r.w / 2.0 - 3.0, r.y + r.h / 2.0 - 3.0, 6.0, 6.0), w, h, theme::INK);
				}
			}
		}
		if let Some((nw, nh)) = self.parsed() {
			// Fixed, short, ASCII-only lines (the MAX font has no em-dash) right of
			// the anchor grid - never word-wrapped, so the note can't overflow.
			let (verb, desc) = if nw >= self.old_w && nh >= self.old_h {
				("enlarge", "fills with water")
			} else if nw <= self.old_w && nh <= self.old_h {
				("shrink", "crops to the anchor")
			} else {
				("resize", "fills and crop")
			};
			let (ox, oy) = self.offset(nw.max(1), nh.max(1));
			let (ax, ay) = self.anchor_origin(d);
			let cx = ax + 3.0 * ANCHOR_CELL + 10.0;
			let lines = [
				verb.to_string(),
				desc.to_string(),
				format!("from {} x {}", self.old_w, self.old_h),
				format!("at {ox} - {oy}"),
			];
			for (i, line) in lines.iter().enumerate() {
				q.label(line, cx, ay + i as f32 * 16.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
			}
		}

		q.button(self.abort_rect(d), w, h, hot);
		q.label_in("Abort", self.abort_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		q.button_primary(self.confirm_rect(d), w, h, hot);
		q.label_in("Resize", self.confirm_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK);
		q
	}

	/// The open size-preset dropdown, as its own layer. The shell draws this
	/// *after* `field_contents`, so the floating list (opaque well + border)
	/// sits above the W/H field text - which is painted last and would otherwise
	/// bleed through it. `None` when the dropdown is closed.
	pub fn popup(&self, w: f32, h: f32, hot: Hot) -> Option<UiQuads> {
		if !self.preset_open {
			return None;
		}
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(ui::SteelMap::anchored(d));
		let labels: Vec<&str> = SIZE_PRESETS.iter().map(|&(name, _, _)| name).collect();
		crate::select::draw_popup(&mut q, self.preset_rect(d), &labels, self.preset_match(), false, w, h, hot);
		Some(q)
	}

	/// Per-field text/caret/selection with the clip rect the shell scissors it to.
	pub fn field_contents(&self, w: f32, h: f32) -> Vec<(UiQuads, Rect)> {
		let d = self.dialog_rect(w, h);
		[Field::Width, Field::Height]
			.into_iter()
			.map(|f| {
				let r = self.field_rect(d, f);
				(self.field_ref(f).content_quads(r, self.focus == Some(f), w, h), r)
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::modal::ModalKey;

	#[test]
	fn drag_offset_shifts_the_dialog_rect() {
		// a titlebar drag accumulates into drag_offset, moving the whole
		// dialog (and thus every field/button computed from it).
		let mut m = Resize::new(4, 4);
		let before = m.dialog_rect(1280.0, 800.0);
		m.drag_offset = (37.0, -12.0);
		let after = m.dialog_rect(1280.0, 800.0);
		assert_eq!((after.x - before.x, after.y - before.y), (37.0, -12.0));
		assert_eq!((after.w, after.h), (before.w, before.h));
	}

	#[test]
	fn center_anchor_centers_the_old_map() {
		let mut m = Resize::new(4, 4);
		m.width.set_text("8");
		m.height.set_text("8");
		// Center (1,1): offset = (8-4)/2 = 2.
		assert_eq!(m.command().unwrap(), "resize 8 8 2 2");
		// Top-left anchor (0,0): offset 0.
		m.col = 0;
		m.row = 0;
		assert_eq!(m.command().unwrap(), "resize 8 8 0 0");
		// Bottom-right (2,2): offset = 2*(8-4)/2 = 4.
		m.col = 2;
		m.row = 2;
		assert_eq!(m.command().unwrap(), "resize 8 8 4 4");
	}

	#[test]
	fn shrink_crops_with_anchor() {
		let mut m = Resize::new(8, 8);
		m.width.set_text("4");
		m.height.set_text("4");
		// Center crop: offset = 1*(4-8)/2 = -2.
		assert_eq!(m.command().unwrap(), "resize 4 4 -2 -2");
	}

	#[test]
	fn validates_size() {
		let mut m = Resize::new(8, 8);
		m.width.set_text("");
		assert!(m.command().is_err());
		m.width.set_text("2000");
		m.height.set_text("8");
		assert!(m.command().is_err());
	}

	#[test]
	fn key_edits_focused_field_and_tab_toggles() {
		let mut m = Resize::new(4, 4);
		m.focus = Some(Field::Width);
		m.key(&ModalKey::SelectAll);
		for c in "256x".chars() {
			m.key(&ModalKey::Char(c)); // non-digit ignored by the Digits charset
		}
		assert_eq!(m.width.text(), "256");
		m.key(&ModalKey::Backspace);
		assert_eq!(m.width.text(), "25");
		m.focus_next();
		assert_eq!(m.focus, Some(Field::Height));
	}

	#[test]
	fn press_anchor_and_fields() {
		let mut m = Resize::new(4, 4);
		let (w, h) = (1280.0, 800.0);
		let d = m.dialog_rect(w, h);
		let f = m.field_rect(d, Field::Width);
		assert_eq!(m.on_press(f.x + 2.0, f.y + 2.0, w, h), Press::Consumed);
		assert_eq!(m.focus, Some(Field::Width));
		let a = m.anchor_cell(d, 0, 0);
		m.on_press(a.x + 2.0, a.y + 2.0, w, h);
		assert_eq!((m.col, m.row), (0, 0));
		// Resize fires on release-inside (press only arms); drag-off cancels.
		let c = m.confirm_rect(d);
		assert_eq!(m.on_press(c.x + 2.0, c.y + 2.0, w, h), Press::Consumed);
		assert!(matches!(m.on_release(c.x + 2.0, c.y + 2.0, w, h), Press::Resize(_)));
		m.on_press(c.x + 2.0, c.y + 2.0, w, h);
		assert_eq!(m.on_release(2.0, 2.0, w, h), Press::Consumed, "drag-off cancels");
	}
}
