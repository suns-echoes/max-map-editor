//! Project tab strip: the row of open-project tabs below the menu
//! bar. One project is active at a time; click a tab to switch, click its `x`
//! to close (hidden when only one tab is open - the last stays). Pure
//! geometry + draw; the shell (`main.rs`) routes clicks and the document model
//! (`state.rs`) owns the project list. Steel-themed like the rest of the
//! chrome (active tab raised + amber, others dim; dirty marked with `*`).

use crate::text;
use crate::theme;
use crate::ui::{Hot, Rect, UiQuads};

/// Tab strip height (px). Sits in `[menu::BAR_H, menu::BAR_H + BAR_H)`.
pub const BAR_H: f32 = 22.0;
const FONT: f32 = crate::ui::FONT_SMALL; // project tabs → the 12px tier
const PAD: f32 = 8.0;
const CLOSE_W: f32 = 13.0;
const MIN_W: f32 = 70.0;
const MAX_W: f32 = 200.0;
const GAP: f32 = 2.0;
/// The floor a tab can compress to when the strip overflows the window -
/// keeps the close `x` and a few label glyphs usable.
const MIN_COMPRESSED: f32 = 44.0;

/// What a press on the strip resolved to.
#[derive(Debug, PartialEq, Eq)]
pub enum Hit {
	None,
	Select(usize),
	Close(usize),
}

/// A tab's label - dirty projects get a trailing `*` (title-bar parity).
fn label(name: &str, dirty: bool) -> String {
	if dirty { format!("{name}*") } else { name.to_string() }
}

/// Per-tab widths: each fits its label (clamped MIN_W..MAX_W), then the whole
/// strip compresses equally toward [`MIN_COMPRESSED`] when it would overflow
/// the `vw`-wide window - labels ellipsize instead of tabs clipping off-screen.
fn tab_widths(tabs: &[(String, bool)], vw: f32) -> Vec<f32> {
	let natural: Vec<f32> = tabs
		.iter()
		.map(|(name, dirty)| {
			let lw = text::label_width(&label(name, *dirty), FONT);
			(PAD + lw + 6.0 + CLOSE_W + PAD).clamp(MIN_W, MAX_W)
		})
		.collect();
	let gaps = (tabs.len() as f32 + 1.0) * GAP;
	let total: f32 = natural.iter().sum();
	if total + gaps <= vw {
		return natural;
	}
	let scale = ((vw - gaps).max(0.0) / total.max(1.0)).min(1.0);
	natural.iter().map(|w| (w * scale).max(MIN_COMPRESSED)).collect()
}

/// Tab `i`'s rect in a strip starting at `top`, in a `vw`-wide window.
fn tab_rect(tabs: &[(String, bool)], top: f32, i: usize, vw: f32) -> Rect {
	let widths = tab_widths(tabs, vw);
	let x = GAP + widths[..i.min(widths.len())].iter().map(|w| w + GAP).sum::<f32>();
	Rect::new(x, top, widths.get(i).copied().unwrap_or(MIN_W), BAR_H - 1.0)
}

/// The close-`x` hit area inside a tab rect.
fn close_rect(r: Rect) -> Rect {
	Rect::new(r.x + r.w - CLOSE_W - 4.0, r.y + (r.h - CLOSE_W) / 2.0, CLOSE_W, CLOSE_W)
}

/// Hit-test a press at `(x, y)` in a `vw`-wide window: a tab's `x` closes it,
/// else the tab body selects it. `closable` is false for the lone blank
/// scratch tab (no `x`).
pub fn hit(tabs: &[(String, bool)], closable: bool, top: f32, x: f32, y: f32, vw: f32) -> Hit {
	if y < top || y >= top + BAR_H {
		return Hit::None;
	}
	for i in 0..tabs.len() {
		let r = tab_rect(tabs, top, i, vw);
		if r.contains(x, y) {
			if closable && close_rect(r).contains(x, y) {
				return Hit::Close(i);
			}
			return Hit::Select(i);
		}
	}
	Hit::None
}

/// Draw the strip: a steel band, then each tab (active raised + amber, others
/// dim). The close `x` shows on every tab except the lone blank scratch
/// (`closable` false - there's nothing to close).
pub fn draw(tabs: &[(String, bool)], active: usize, closable: bool, top: f32, w: f32, h: f32, hot: Hot) -> UiQuads {
	let mut q = UiQuads::default();
	let strip = Rect::new(0.0, top, w, BAR_H);
	q.material(strip, w, h, theme::PANEL);
	q.rect(Rect::new(0.0, top + BAR_H - 1.0, w, 1.0), w, h, theme::BEVEL.bottom);
	for (i, (name, dirty)) in tabs.iter().enumerate() {
		let r = tab_rect(tabs, top, i, w);
		q.button_active(r, w, h, i == active, hot);
		let ink = if i == active { theme::ACCENT } else { theme::INK_DIM };
		// Long project names ellipsize inside the tab instead of spilling over
		// the close glyph / the neighbour tab.
		let avail = Rect::new(r.x, r.y, r.w - if closable { CLOSE_W + 4.0 } else { 0.0 }, r.h);
		q.label_fit(&label(name, *dirty), avail, PAD, FONT, w, h, ink);
		if closable {
			let c = close_rect(r);
			if hot.hover(c) {
				q.rect(c, w, h, if hot.pressed(c) { theme::PRESS } else { theme::HOVER });
			}
			q.label_in("x", c, 3.0, FONT, w, h, theme::CLOSE_INK);
		}
	}
	q
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tabs() -> Vec<(String, bool)> {
		vec![("alpha".into(), false), ("beta".into(), true), ("gamma".into(), false)]
	}

	#[test]
	fn body_selects_and_x_closes() {
		let t = tabs();
		let (top, vw) = (24.0, 1280.0);
		// Above/below the strip: nothing.
		assert_eq!(hit(&t, true, top, 10.0, top - 1.0, vw), Hit::None);
		// Tab 1's body selects it; its `x` closes it.
		let r = tab_rect(&t, top, 1, vw);
		assert_eq!(hit(&t, true, top, r.x + 4.0, top + 4.0, vw), Hit::Select(1));
		let c = close_rect(r);
		assert_eq!(hit(&t, true, top, c.x + 2.0, c.y + 2.0, vw), Hit::Close(1));
	}

	#[test]
	fn non_closable_strip_only_selects() {
		// The lone blank scratch tab has no `x` - its corner just selects.
		let one = vec![("empty".to_string(), false)];
		let (top, vw) = (24.0, 1280.0);
		let c = close_rect(tab_rect(&one, top, 0, vw));
		assert_eq!(hit(&one, false, top, c.x + 2.0, c.y + 2.0, vw), Hit::Select(0));
	}

	#[test]
	fn many_tabs_compress_to_fit_the_window() {
		// 12 long-named tabs in a narrow window: every tab stays on-screen
		// (compressed equally), and hit-testing matches the drawn rects.
		let many: Vec<(String, bool)> =
			(0..12).map(|i| (format!("a-rather-long-project-name-{i}.json"), i % 2 == 0)).collect();
		let (top, vw) = (24.0, 800.0);
		for i in 0..many.len() {
			let r = tab_rect(&many, top, i, vw);
			assert!(r.x + r.w <= vw + 0.5, "tab {i} overflows: ends at {}", r.x + r.w);
			assert!(r.w >= MIN_COMPRESSED - 0.5, "tab {i} unusably narrow: {}", r.w);
			assert_eq!(hit(&many, true, top, r.x + 2.0, top + 4.0, vw), Hit::Select(i));
		}
		// A roomy window keeps natural widths (no needless compression).
		let r = tab_rect(&many, top, 0, 10_000.0);
		assert!(r.w > MIN_COMPRESSED + 10.0);
	}
}
