//! Remove Duplicate Templates modal: reports the exact-duplicate user
//! templates found in the explorer's visible list (a scrollable list of all of
//! them) and asks to confirm their removal. Remove emits `template-dedupe!`
//! (the shell deletes the files and rescans); when nothing is duplicated it's
//! just an acknowledgement.
//!
//! Pure state/geometry - the duplicate set is computed by the shell when the
//! modal opens and passed in as the names to be removed. The scrolling name
//! list is drawn clipped by the shell (like the other scrolling wells).

use crate::theme;
use crate::ui::{self, Hot, Rect, SteelMap, UiQuads};

const W: f32 = 380.0;
const TITLE_H: f32 = 22.0;
const LINE_H: f32 = 15.0;
const BTN_H: f32 = 24.0;
const PAD: f32 = 12.0;
const GAP: f32 = 8.0;
/// The list well shows at most this many rows before it scrolls.
const VISIBLE_ROWS: usize = 8;

pub struct DedupeTemplates {
	/// The exact-duplicate user-template names that would be removed.
	names: Vec<String>,
	/// Scroll offset (px) into the name list.
	scroll: f32,
	/// Held button: 0 Cancel/Close, 1 Remove - dragging off cancels.
	armed: Option<usize>,
	pub(crate) drag_offset: (f32, f32),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Press {
	Consumed,
	Cancel,
	Remove,
}

impl DedupeTemplates {
	pub fn new(names: Vec<String>) -> Self {
		Self { names, scroll: 0.0, armed: None, drag_offset: (0.0, 0.0) }
	}

	fn has_dupes(&self) -> bool {
		!self.names.is_empty()
	}

	/// Rows shown in the well (capped); the rest scroll into view.
	fn shown_rows(&self) -> usize {
		self.names.len().min(VISIBLE_ROWS)
	}

	/// Total list content height vs the well height.
	fn content_h(&self) -> f32 {
		self.names.len() as f32 * LINE_H
	}

	fn well_h(&self) -> f32 {
		self.shown_rows() as f32 * LINE_H
	}

	pub fn max_scroll(&self) -> f32 {
		(self.content_h() - self.well_h()).max(0.0)
	}

	// ----- geometry -----------------------------------------------------------

	fn height(&self) -> f32 {
		if self.has_dupes() {
			TITLE_H + PAD + LINE_H + 4.0 + self.well_h() + GAP + BTN_H + PAD
		} else {
			TITLE_H + PAD + LINE_H + GAP + BTN_H + PAD
		}
	}

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		Rect::centered(w, h, W, self.height()).translate(self.drag_offset.0, self.drag_offset.1)
	}

	/// The scrolling name-list well, below the heading.
	fn list_well(&self, d: Rect) -> Rect {
		let y = d.y + TITLE_H + PAD + LINE_H + 4.0;
		Rect::new(d.x + PAD, y, W - 2.0 * PAD, self.well_h())
	}

	fn cancel_rect(&self, d: Rect) -> Rect {
		crate::ui::button_pair(d, W, PAD, BTN_H).0
	}

	fn remove_rect(&self, d: Rect) -> Rect {
		crate::ui::button_pair(d, W, PAD, BTN_H).1
	}

	// ----- events -------------------------------------------------------------

	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		if self.cancel_rect(d).contains(x, y) {
			self.armed = Some(0);
			return Press::Consumed;
		}
		if self.has_dupes() && self.remove_rect(d).contains(x, y) {
			self.armed = Some(1);
			return Press::Consumed;
		}
		if !d.contains(x, y) {
			return Press::Cancel;
		}
		Press::Consumed
	}

	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		match self.armed.take() {
			Some(0) if self.cancel_rect(d).contains(x, y) => Press::Cancel,
			Some(1) if self.has_dupes() && self.remove_rect(d).contains(x, y) => Press::Remove,
			_ => Press::Consumed,
		}
	}

	/// Wheel scrolls the name list (one row per notch).
	pub fn scroll_by(&mut self, steps: f32) {
		self.scroll = (self.scroll - steps * LINE_H).clamp(0.0, self.max_scroll());
	}

	// ----- drawing ------------------------------------------------------------

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(SteelMap::anchored(d));
		ui::modal_scrim(&mut q, w, h);
		ui::modal_frame(&mut q, d, "Remove Duplicates", TITLE_H, w, h);

		let x = d.x + PAD;
		let y = d.y + TITLE_H + PAD;
		if self.has_dupes() {
			let n = self.names.len();
			let heading = format!("Found {n} exact-duplicate template{}:", if n == 1 { "" } else { "s" });
			q.label(&heading, x, y, crate::ui::FONT_SMALL, w, h, theme::INK);
			// The list well (its names are drawn clipped by the shell) + a
			// scrollbar when the list overflows.
			let well = self.list_well(d);
			q.field(well, w, h);
			q.scrollbar(well, self.content_h(), self.scroll, w, h, hot);
			q.button(self.cancel_rect(d), w, h, hot);
			q.label_in("Cancel", self.cancel_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
			q.button_primary(self.remove_rect(d), w, h, hot);
			q.label_in("Remove", self.remove_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK);
		} else {
			q.label("No duplicate templates found.", x, y, crate::ui::FONT_SMALL, w, h, theme::INK);
			// A single acknowledgement button (the "Cancel" slot reads "Close").
			q.button_primary(self.cancel_rect(d), w, h, hot);
			q.label_in("Close", self.cancel_rect(d), 8.0, crate::ui::FONT_SMALL, w, h, theme::INK);
		}
		q
	}

	/// The scrolling name rows + the clip rect the shell scissors them to.
	pub fn list_content(&self, w: f32, h: f32) -> (UiQuads, Rect) {
		let well = self.list_well(self.dialog_rect(w, h));
		let mut q = UiQuads::default();
		if !self.has_dupes() {
			return (q, well);
		}
		let row_w = well.w - crate::ui::SCROLLBAR_W - 4.0;
		for (i, name) in self.names.iter().enumerate() {
			let ry = well.y + i as f32 * LINE_H - self.scroll;
			if ry + LINE_H < well.y || ry > well.y + well.h {
				continue;
			}
			q.label_fit(
				name,
				Rect::new(well.x + 4.0, ry, row_w, LINE_H),
				0.0,
				crate::ui::FONT_SMALL,
				w,
				h,
				theme::INK_DIM,
			);
		}
		(q, well)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn center(r: Rect) -> (f32, f32) {
		(r.x + r.w / 2.0, r.y + r.h / 2.0)
	}

	#[test]
	fn remove_is_live_only_when_there_are_dupes() {
		let (w, h) = (800.0, 600.0);
		// With dupes: the Remove button arms on press and fires on release.
		let mut dupes = DedupeTemplates::new(vec!["a".into(), "b".into()]);
		assert!(dupes.has_dupes());
		let d = dupes.dialog_rect(w, h);
		let (rx, ry) = center(dupes.remove_rect(d));
		assert_eq!(dupes.on_press(rx, ry, w, h), Press::Consumed);
		assert_eq!(dupes.on_release(rx, ry, w, h), Press::Remove);

		// No dupes: shorter dialog (no list well), and the Remove spot is dead -
		// pressing it never arms Remove, so release can't fire it.
		let mut empty = DedupeTemplates::new(vec![]);
		assert!(!empty.has_dupes());
		assert!(empty.dialog_rect(w, h).h < d.h, "the no-dupes dialog drops the list well");
		let dd = empty.dialog_rect(w, h);
		let (rx, ry) = center(empty.remove_rect(dd));
		empty.on_press(rx, ry, w, h);
		assert_eq!(empty.on_release(rx, ry, w, h), Press::Consumed, "no Remove without dupes");
	}

	#[test]
	fn cancel_arms_and_clickout_cancels() {
		let (w, h) = (800.0, 600.0);
		let mut m = DedupeTemplates::new(vec!["x".into()]);
		let d = m.dialog_rect(w, h);
		let (cx, cy) = center(m.cancel_rect(d));
		assert_eq!(m.on_press(cx, cy, w, h), Press::Consumed);
		assert_eq!(m.on_release(cx, cy, w, h), Press::Cancel);
		// A press outside the dialog cancels immediately.
		assert_eq!(m.on_press(1.0, 1.0, w, h), Press::Cancel);
	}
}
