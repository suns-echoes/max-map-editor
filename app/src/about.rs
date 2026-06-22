//! About dialog: the editor's name, version (from Cargo), author, a one-line
//! tagline, and buttons that open the website / project GitHub. Button-only,
//! like the other confirm modals - no text field.

use crate::theme;
use crate::ui::{self, Hot, Rect, SteelMap, UiQuads};

/// The project's public homepage + source, shared with the Help menu items.
pub const WEBSITE: &str = "https://suns-echoes.github.io/max-map-editor/";
pub const GITHUB: &str = "https://github.com/suns-echoes/max-map-editor";

const W: f32 = 340.0;
const TITLE_H: f32 = 22.0;
const BTN_H: f32 = 24.0;
const PAD: f32 = 14.0;
const LINE_H: f32 = 18.0;
/// The credits text sits in a darker inset: this is the gap around the inset
/// (from the dialog content + the buttons) and the gap from the inset to its text.
const MARGIN: f32 = 4.0;
const PADDING: f32 = 4.0;
/// Rendered text rows (blank rows included), driving the inset's height.
const LINES: usize = 8;

pub struct About {
	/// Held button: 0 Website, 1 GitHub, 2 Close - dragging off cancels.
	armed: Option<usize>,
	pub(crate) drag_offset: (f32, f32),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Press {
	Consumed,
	Close,
	Website,
	GitHub,
}

impl About {
	pub fn new() -> Self {
		Self { armed: None, drag_offset: (0.0, 0.0) }
	}

	// ----- geometry ------------------------------------------------------------

	/// The darker credits inset (4px margin from the dialog content).
	fn inset_h() -> f32 {
		LINES as f32 * LINE_H + 2.0 * PADDING
	}

	fn height() -> f32 {
		TITLE_H + MARGIN + Self::inset_h() + MARGIN + BTN_H + PAD
	}

	pub fn dialog_rect(&self, w: f32, h: f32) -> Rect {
		Rect::centered(w, h, W, Self::height()).translate(self.drag_offset.0, self.drag_offset.1)
	}

	/// The three buttons share the bottom row: [Website] [GitHub] ...... [Close].
	fn website_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + PAD, d.y + d.h - PAD - BTN_H, 78.0, BTN_H)
	}

	fn github_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + PAD + 78.0 + 6.0, d.y + d.h - PAD - BTN_H, 70.0, BTN_H)
	}

	fn close_rect(&self, d: Rect) -> Rect {
		Rect::new(d.x + W - PAD - 78.0, d.y + d.h - PAD - BTN_H, 78.0, BTN_H)
	}

	// ----- events --------------------------------------------------------------

	pub fn on_press(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		if self.website_rect(d).contains(x, y) {
			self.armed = Some(0);
			return Press::Consumed;
		}
		if self.github_rect(d).contains(x, y) {
			self.armed = Some(1);
			return Press::Consumed;
		}
		if self.close_rect(d).contains(x, y) {
			self.armed = Some(2);
			return Press::Consumed;
		}
		if !d.contains(x, y) {
			return Press::Close; // click-out dismisses
		}
		Press::Consumed
	}

	pub fn on_release(&mut self, x: f32, y: f32, w: f32, h: f32) -> Press {
		let d = self.dialog_rect(w, h);
		match self.armed.take() {
			Some(0) if self.website_rect(d).contains(x, y) => Press::Website,
			Some(1) if self.github_rect(d).contains(x, y) => Press::GitHub,
			Some(2) if self.close_rect(d).contains(x, y) => Press::Close,
			_ => Press::Consumed,
		}
	}

	// ----- drawing -------------------------------------------------------------

	pub fn view(&self, w: f32, h: f32, hot: Hot) -> UiQuads {
		let d = self.dialog_rect(w, h);
		let mut q = UiQuads::with_steel_map(SteelMap::anchored(d));
		ui::modal_scrim(&mut q, w, h);
		ui::modal_frame(&mut q, d, "About", TITLE_H, w, h);
		let f = ui::FONT_SMALL;

		// Credits in a darker, textured inset (4px margin around it, 4px padding
		// to the text). The title line keeps its brighter INK; the rest is dim.
		let inset = Rect::new(d.x + MARGIN, d.y + TITLE_H + MARGIN, W - 2.0 * MARGIN, Self::inset_h());
		q.field(inset, w, h);
		let title = format!("M.A.X. Map Editor v{}", env!("CARGO_PKG_VERSION"));
		let lines: [(&str, [f32; 4]); LINES] = [
			(&title, theme::INK),
			("", theme::INK_DIM),
			("The map `Utility` for M.A.X.:", theme::INK_DIM),
			("Mechanized Assault & Exploration", theme::INK_DIM),
			("", theme::INK_DIM),
			("by MAX Commander for MAX Commanders", theme::INK_DIM),
			("", theme::INK_DIM),
			("(c) Aneta Suns", theme::INK_DIM),
		];
		let tx = inset.x + PADDING;
		for (i, (text, color)) in lines.into_iter().enumerate() {
			if !text.is_empty() {
				q.label(text, tx, inset.y + PADDING + i as f32 * LINE_H, f, w, h, color);
			}
		}

		q.button(self.website_rect(d), w, h, hot);
		q.label_in("Website", self.website_rect(d), 8.0, f, w, h, theme::INK_DIM);
		q.button(self.github_rect(d), w, h, hot);
		q.label_in("GitHub", self.github_rect(d), 8.0, f, w, h, theme::INK_DIM);
		q.button_primary(self.close_rect(d), w, h, hot);
		q.label_in("Close", self.close_rect(d), 8.0, f, w, h, theme::INK);
		q
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn buttons_arm_and_fire_on_release_inside() {
		let (w, h) = (1280.0, 800.0);
		let mut m = About::new();
		let d = m.dialog_rect(w, h);
		for (rect, want) in
			[(m.website_rect(d), Press::Website), (m.github_rect(d), Press::GitHub), (m.close_rect(d), Press::Close)]
		{
			assert_eq!(m.on_press(rect.x + 2.0, rect.y + 2.0, w, h), Press::Consumed);
			assert_eq!(m.on_release(rect.x + 2.0, rect.y + 2.0, w, h), want);
		}
		// Drag-off cancels; click-out dismisses.
		let c = m.close_rect(d);
		m.on_press(c.x + 2.0, c.y + 2.0, w, h);
		assert_eq!(m.on_release(2.0, 2.0, w, h), Press::Consumed);
		assert_eq!(m.on_press(2.0, 2.0, w, h), Press::Close);
	}
}
