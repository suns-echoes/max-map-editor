//! A reusable dropdown/select control: a closed value box with a caret
//! triangle, and - when open - a popup option list dropped below (or above)
//! it. Pure geometry, hit-testing, and drawing; each host owns the `open` flag
//! and the selected value, and decides what a pick does (set a modal field,
//! run a command). The MAX font has no arrow glyph, so the caret is a filled
//! `q.tri` triangle.
//!
//! The popup floats: draw [`draw_box`] inline with the rest of a host's chrome,
//! and [`draw_popup`] *last* so the list sits on top of everything beneath it.
//! `up` drops the list above the box when there isn't room below; the host
//! computes it once and passes the same value to [`hit`] and [`draw_popup`].

use crate::theme;
use crate::ui::{FONT_SMALL, Hot, Rect, UiQuads};

/// Height of one option row in the open popup.
pub const ROW_H: f32 = 18.0;
/// Right-gutter width reserved for the caret (keeps long labels off it).
const CARET_GUTTER: f32 = 16.0;

/// What a click resolved to against a select control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
	/// The closed value box - toggle the popup open/closed.
	Box,
	/// Option `i` in the open popup - pick it (and close).
	Option(usize),
}

/// The popup list rect for `n` options, dropped below `box_rect` (or above when
/// `up`).
pub fn popup_rect(box_rect: Rect, n: usize, up: bool) -> Rect {
	let ph = n as f32 * ROW_H;
	let y = if up { box_rect.y - ph } else { box_rect.y + box_rect.h };
	Rect::new(box_rect.x, y, box_rect.w, ph)
}

/// The rect of option `i` within the open popup.
pub fn option_rect(box_rect: Rect, i: usize, n: usize, up: bool) -> Rect {
	let pr = popup_rect(box_rect, n, up);
	Rect::new(pr.x, pr.y + i as f32 * ROW_H, pr.w, ROW_H)
}

/// Hit-test a click. When closed only the box can hit; when `open` the popup's
/// options also hit. `None` = the click missed (the host closes the popup).
pub fn hit(box_rect: Rect, open: bool, n: usize, up: bool, x: f32, y: f32) -> Option<Hit> {
	if box_rect.contains(x, y) {
		return Some(Hit::Box);
	}
	if open {
		for i in 0..n {
			if option_rect(box_rect, i, n, up).contains(x, y) {
				return Some(Hit::Option(i));
			}
		}
	}
	None
}

/// The closed value box: a button face, `label` (ellipsized to fit, clear of
/// the caret gutter), and a caret triangle - down when closed, up when open.
pub fn draw_box(q: &mut UiQuads, r: Rect, label: &str, open: bool, w: f32, h: f32, hot: Hot) {
	q.button(r, w, h, hot);
	let text_r = Rect::new(r.x, r.y, (r.w - CARET_GUTTER).max(0.0), r.h);
	q.label_fit(label, text_r, 6.0, FONT_SMALL, w, h, theme::INK);
	draw_caret(q, r, open, w, h);
}

/// The caret triangle in the box's right gutter.
fn draw_caret(q: &mut UiQuads, r: Rect, open: bool, w: f32, h: f32) {
	let cx = r.x + r.w - 10.0;
	let cy = r.y + r.h / 2.0;
	let (hw, hh) = (3.5, 2.5);
	if open {
		// Points up (the list is expanded).
		q.tri((cx - hw, cy + hh), (cx + hw, cy + hh), (cx, cy - hh), w, h, theme::INK);
	} else {
		// Points down.
		q.tri((cx - hw, cy - hh), (cx + hw, cy - hh), (cx, cy + hh), w, h, theme::INK);
	}
}

/// The open popup option list. Draw this *after* the host's other chrome so it
/// floats on top. `selected` is the current value's option index (lit in
/// ACCENT); the row under the cursor gets a hover wash.
pub fn draw_popup<S: AsRef<str>>(
	q: &mut UiQuads,
	box_rect: Rect,
	labels: &[S],
	selected: Option<usize>,
	up: bool,
	w: f32,
	h: f32,
	hot: Hot,
) {
	let n = labels.len();
	let pr = popup_rect(box_rect, n, up);
	q.field(pr, w, h);
	for (i, label) in labels.iter().enumerate() {
		let r = option_rect(box_rect, i, n, up);
		if hot.hover(r) {
			q.rect(r, w, h, theme::HOVER);
		}
		let ink = if Some(i) == selected { theme::ACCENT } else { theme::INK };
		q.label_fit(label.as_ref(), r, 6.0, FONT_SMALL, w, h, ink);
	}
	q.border(pr, w, h, theme::INK_DIM);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn hit_resolves_box_then_options_when_open() {
		let r = Rect::new(100.0, 50.0, 80.0, 20.0);
		// Closed: only the box hits; options are inert.
		assert_eq!(hit(r, false, 3, false, 110.0, 55.0), Some(Hit::Box));
		assert_eq!(hit(r, false, 3, false, 110.0, 90.0), None);
		// Open: the box still toggles, and each option row hits.
		assert_eq!(hit(r, true, 3, false, 110.0, 55.0), Some(Hit::Box));
		for i in 0..3 {
			let o = option_rect(r, i, 3, false);
			assert_eq!(hit(r, true, 3, false, o.x + 2.0, o.y + 2.0), Some(Hit::Option(i)));
		}
		// Below the last option: a miss (host closes the popup).
		let below = popup_rect(r, 3, false);
		assert_eq!(hit(r, true, 3, false, below.x + 2.0, below.y + below.h + 1.0), None);
	}

	#[test]
	fn up_flips_the_popup_above_the_box() {
		let r = Rect::new(100.0, 200.0, 80.0, 20.0);
		let down = popup_rect(r, 4, false);
		let up = popup_rect(r, 4, true);
		assert_eq!(down.y, r.y + r.h, "down hangs below the box");
		assert_eq!(up.y + up.h, r.y, "up sits flush above the box");
		// Options stay top-to-bottom in both directions.
		assert!(option_rect(r, 0, 4, true).y < option_rect(r, 3, 4, true).y);
	}
}
