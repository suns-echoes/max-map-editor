//! UI primitives: screen-space rects, the two-batch quad
//! collector, and draw-from-rect widget helpers. Immediate mode — every frame
//! the workspace rebuilds its quads from layout; nothing here is retained.
//! Pattern from world-editor `ui.rs`/`widget.rs`, re-skinned for M.A.X.

use crate::text::{self, TextVertex};
use crate::theme;

/// Default UI text size (px) — primary labels, titles, buttons. Both
/// constants are baked sizes in [`crate::font::SIZES`].
pub const FONT_BODY: f32 = 16.0;
/// Small UI text size (px) — dense panels, hints, secondary labels.
pub const FONT_SMALL: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
	pub x: f32,
	pub y: f32,
	pub w: f32,
	pub h: f32,
}

impl Rect {
	pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
		Self { x, y, w, h }
	}

	pub fn contains(&self, px: f32, py: f32) -> bool {
		px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
	}

	/// A `w`×`h` box centered in a `screen_w`×`screen_h` viewport — every modal
	/// dialog rect.
	pub fn centered(screen_w: f32, screen_h: f32, w: f32, h: f32) -> Self {
		Self::new((screen_w - w) / 2.0, (screen_h - h) / 2.0, w, h)
	}

	/// The full-width `h`-tall strip at the top of this rect — a panel header
	/// band / sub-toolbar.
	pub fn strip_top(&self, h: f32) -> Self {
		Self::new(self.x, self.y, self.w, h)
	}

	/// This rect shifted by `(dx, dy)` — used to apply a draggable modal's offset.
	pub fn translate(&self, dx: f32, dy: f32) -> Self {
		Self::new(self.x + dx, self.y + dy, self.w, self.h)
	}
}

/// The live pointer snapshot the shell hands every view, so widgets can render
/// hover and pressed states. One source of truth: the shell updates it from
/// winit events; views only read it. The headless/screenshot path keeps the
/// inert default (no cursor, no press), so captures always show the rest state.
///
/// A surface that's covered (panels under a modal, everything under an open
/// menu dropdown) is handed [`Hot::NONE`] so nothing beneath highlights.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Hot {
	/// Cursor position, when the window has one.
	pub cursor: Option<(f32, f32)>,
	/// Where the primary button went down, while it is still held.
	pub down: Option<(f32, f32)>,
}

impl Hot {
	/// The inert pointer — for covered surfaces and headless rendering.
	pub const NONE: Hot = Hot { cursor: None, down: None };

	pub fn hover(&self, r: Rect) -> bool {
		self.cursor.is_some_and(|(x, y)| r.contains(x, y))
	}

	/// Held down on this widget: the press began inside `r` and the cursor is
	/// still there — the classic armed-button look.
	pub fn pressed(&self, r: Rect) -> bool {
		self.hover(r) && self.down.is_some_and(|(x, y)| r.contains(x, y))
	}
}

/// Max scroll offset so `content` px can reach the bottom of a `view`-tall
/// area: `(content - view)` clamped at zero. Callers clamp the live offset to
/// `0..=scroll_max(..)`.
pub fn scroll_max(content: f32, view: f32) -> f32 {
	(content - view).max(0.0)
}

/// Width of a vertical scrollbar gutter. Scrollable content should reserve
/// this much on its right so the bar never overlaps content.
pub const SCROLLBAR_W: f32 = 8.0;

/// Which atlas/sheet a vertex run samples (and thus which pipeline draws it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Batch {
	/// Solid fills + Hack mono text (the console atlas).
	Shape,
	/// MAX proportional labels at a baked size (px) — picks that size's atlas.
	Label(u32),
	/// Tinted brushed-steel chrome fills (the steel sheet).
	Steel,
}

/// How a [`UiQuads`] maps screen rects onto the steel sheet.
#[derive(Clone, Copy, Default)]
pub enum SteelMap {
	/// The sheet stretched once to fill the whole viewport — the main shell
	/// (background, docked panels, menu) is cut from one continuous plate.
	#[default]
	Stretch,
	/// A crop of the sheet anchored to a window's local space, so the grain
	/// travels with the window (no swimming as it moves). Used for modals +
	/// floating panels.
	Anchored { anchor: (f32, f32), crop: (f32, f32), scale: f32 },
}

impl SteelMap {
	/// Anchor sampling to `window`: fit the **whole** window into the sheet
	/// (one non-repeating copy — the larger side spans the full sheet, the other
	/// is centered) and move that crop with the window. Scaling to the window
	/// means the grain never wraps, whatever the size.
	pub fn anchored(window: Rect) -> Self {
		// Larger side → 1.0 of the sheet; keeps the crop inside [0,1] (no repeat).
		let scale = 1.0 / window.w.max(window.h).max(1.0);
		let crop = (0.5 - window.w * 0.5 * scale, 0.5 - window.h * 0.5 * scale);
		SteelMap::Anchored { anchor: (window.x, window.y), crop, scale }
	}

	/// The steel uv corners `[u0, v0, u1, v1]` for screen rect `r` in a
	/// `vw`×`vh` viewport.
	fn uv(self, r: Rect, vw: f32, vh: f32) -> [f32; 4] {
		match self {
			SteelMap::Stretch => [r.x / vw, r.y / vh, (r.x + r.w) / vw, (r.y + r.h) / vh],
			SteelMap::Anchored { anchor, crop, scale } => {
				let u = |x: f32| crop.0 + (x - anchor.0) * scale;
				let v = |y: f32| crop.1 + (y - anchor.1) * scale;
				[u(r.x), v(r.y), u(r.x + r.w), v(r.y + r.h)]
			}
		}
	}
}

/// One UI frame's geometry: a single vertex list plus the run structure —
/// consecutive same-atlas pushes coalesce into one run, and `draw_ui` plays
/// the runs **in push order**. That keeps z-order honest: a panel drawn
/// later covers an earlier panel's labels (a global shapes-then-labels split
/// would float every label above every body).
#[derive(Default)]
pub struct UiQuads {
	pub(crate) verts: Vec<TextVertex>,
	/// `(atlas, end index)` per run; a run starts where the previous ended.
	pub(crate) runs: Vec<(Batch, usize)>,
	/// How steel fills sample the sheet (default: stretched to the viewport).
	steel_map: SteelMap,
}

impl UiQuads {
	/// An empty frame whose steel fills sample via `map` (e.g. an anchored crop
	/// for a modal or floating panel).
	pub fn with_steel_map(map: SteelMap) -> Self {
		Self { steel_map: map, ..Default::default() }
	}

	/// Close the bookkeeping after pushing vertices of `kind`.
	fn mark(&mut self, kind: Batch) {
		match self.runs.last_mut() {
			Some((k, end)) if *k == kind => *end = self.verts.len(),
			_ => self.runs.push((kind, self.verts.len())),
		}
	}

	/// Append pre-built shape vertices (console overlay path).
	pub fn raw_shapes(&mut self, v: &[TextVertex]) {
		self.verts.extend_from_slice(v);
		self.mark(Batch::Shape);
	}

	/// Solid fill.
	pub fn rect(&mut self, r: Rect, w: f32, h: f32, color: [f32; 4]) {
		text::push_rect(&mut self.verts, r.x, r.y, r.x + r.w, r.y + r.h, w, h, color);
		self.mark(Batch::Shape);
	}

	/// 1-px border (four thin fills).
	pub fn border(&mut self, r: Rect, w: f32, h: f32, color: [f32; 4]) {
		let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
		text::push_rect(&mut self.verts, x0, y0, x1, y0 + 1.0, w, h, color);
		text::push_rect(&mut self.verts, x0, y1 - 1.0, x1, y1, w, h, color);
		text::push_rect(&mut self.verts, x0, y0, x0 + 1.0, y1, w, h, color);
		text::push_rect(&mut self.verts, x1 - 1.0, y0, x1, y1, w, h, color);
		self.mark(Batch::Shape);
	}

	// ----- steel skin: textured fills + directional bevels --------------------

	/// Tinted brushed-steel fill: maps `r` onto the sheet via this frame's
	/// [`SteelMap`] (stretched to the viewport, or an anchored window crop), so
	/// neighbours share one continuous grain. `tint` multiplies the texel
	/// (`tint.a` = how strongly the steel shows over whatever is beneath).
	pub fn steel(&mut self, r: Rect, w: f32, h: f32, tint: [f32; 4]) {
		let uv = self.steel_map.uv(r, w, h);
		text::push_textured(&mut self.verts, r.x, r.y, r.x + r.w, r.y + r.h, uv, w, h, tint);
		self.mark(Batch::Steel);
	}

	/// A material fill: flat base tone + the steel grain over it.
	pub fn material(&mut self, r: Rect, w: f32, h: f32, m: theme::Material) {
		self.rect(r, w, h, m.base);
		self.steel(r, w, h, m.grain);
	}

	/// A solid triangle from three screen-space points — chrome accents
	/// (e.g. the floating resize-handle grip).
	pub fn tri(&mut self, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), w: f32, h: f32, color: [f32; 4]) {
		text::push_tri(&mut self.verts, p0, p1, p2, w, h, color);
		self.mark(Batch::Shape);
	}

	/// A solid four-point polygon (two triangles) — for non-axis-aligned chrome
	/// like the mitered bevel trapezoids.
	fn quad4(&mut self, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), p3: (f32, f32), w: f32, h: f32, c: [f32; 4]) {
		text::push_tri(&mut self.verts, p0, p1, p2, w, h, c);
		text::push_tri(&mut self.verts, p0, p2, p3, w, h, c);
		self.mark(Batch::Shape);
	}

	/// A directional bevel: a `size`-px border ring, lit from the top-left. The
	/// four edges are trapezoids that meet at 45° corner diagonals (CSS-border
	/// miter, no overlap); `raised` lights top+left, inset swaps to bottom+right.
	/// Drawn over the fill, and — with borders-as-margin — sitting in the inset
	/// the content leaves clear.
	pub fn bevel(&mut self, r: Rect, w: f32, h: f32, size: f32, raised: bool) {
		let b = theme::BEVEL;
		let (top, left, bottom, right) =
			if raised { (b.top, b.left, b.bottom, b.right) } else { (b.bottom, b.right, b.top, b.left) };
		let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
		let (ix0, iy0, ix1, iy1) = (x0 + size, y0 + size, x1 - size, y1 - size);
		self.quad4((x0, y0), (x1, y0), (ix1, iy0), (ix0, iy0), w, h, top);
		self.quad4((x0, y1), (x1, y1), (ix1, iy1), (ix0, iy1), w, h, bottom);
		self.quad4((x0, y0), (x0, y1), (ix0, iy1), (ix0, iy0), w, h, left);
		self.quad4((x1, y0), (x1, y1), (ix1, iy1), (ix1, iy0), w, h, right);
	}

	/// A raised steel control (panel / button): material + a lit bevel.
	pub fn raised(&mut self, r: Rect, w: f32, h: f32, m: theme::Material, size: f32) {
		self.material(r, w, h, m);
		self.bevel(r, w, h, size, true);
	}

	/// A sunken steel well (text field / list): material + an inset bevel.
	pub fn inset(&mut self, r: Rect, w: f32, h: f32, m: theme::Material, size: f32) {
		self.material(r, w, h, m);
		self.bevel(r, w, h, size, false);
	}

	/// The state-aware face every button shares: raised at rest, a darkening
	/// wash under the cursor, and an inset bevel + stronger wash while held —
	/// the key visibly sinks. Inert with [`Hot::NONE`], so headless captures
	/// always render the rest state.
	pub fn button_face(&mut self, r: Rect, w: f32, h: f32, m: theme::Material, hot: Hot) {
		self.material(r, w, h, m);
		let pressed = hot.pressed(r);
		self.bevel(r, w, h, 1.0, !pressed);
		if pressed {
			self.rect(r, w, h, theme::PRESS);
		} else if hot.hover(r) {
			self.rect(r, w, h, theme::HOVER);
		}
	}

	/// A plain clickable button (1-px raised bevel; hover/pressed via `hot`).
	pub fn button(&mut self, r: Rect, w: f32, h: f32, hot: Hot) {
		self.button_face(r, w, h, theme::BUTTON, hot);
	}

	/// The primary action button (warm amber steel, raised).
	pub fn button_primary(&mut self, r: Rect, w: f32, h: f32, hot: Hot) {
		self.button_face(r, w, h, theme::BUTTON_PRIMARY, hot);
	}

	/// A toggled-on control (cool highlight, raised) — `on` falls back to a
	/// plain button so callers can `q.button_active(r, w, h, selected, hot)`.
	pub fn button_active(&mut self, r: Rect, w: f32, h: f32, on: bool, hot: Hot) {
		self.button_face(r, w, h, if on { theme::BUTTON_ACTIVE } else { theme::BUTTON }, hot);
	}

	/// A button that can't be pressed right now (settings locked mid-run): a
	/// muted face that never reacts to the pointer. Pair with INK_DIM labels.
	pub fn button_disabled(&mut self, r: Rect, w: f32, h: f32) {
		self.button_face(r, w, h, theme::BUTTON_DISABLED, Hot::NONE);
	}

	/// A text field / list well (dark, low-contrast, inset).
	pub fn field(&mut self, r: Rect, w: f32, h: f32) {
		self.inset(r, w, h, theme::TEXTAREA, 1.0);
	}

	/// The shared progress bar: an inset well filling with the accent green,
	/// optionally a centered embossed label over it (e.g. "42%"). Every live
	/// run (generate, convert, …) draws this one widget, so progress reads the
	/// same everywhere.
	pub fn progress_bar(&mut self, r: Rect, frac: f32, label: Option<&str>, px: f32, w: f32, h: f32) {
		self.field(r, w, h);
		let fill = (r.w - 2.0) * frac.clamp(0.0, 1.0);
		if fill >= 1.0 {
			self.rect(Rect::new(r.x + 1.0, r.y + 1.0, fill, r.h - 2.0), w, h, theme::ACCENT);
		}
		if let Some(s) = label {
			let x = r.x + (r.w - text::label_width(s, px)) / 2.0;
			self.label_emboss(s, x, r.y + (r.h - px) / 2.0, px, w, h, theme::INK);
		}
	}

	/// A toggleable button: the active bevel, a menu-style checkbox in the left
	/// gutter, and the label in ACCENT (#44FF00) when checked — so on/off buttons
	/// read like the menu's toggle items. `enabled: false` draws the muted
	/// locked face (state still readable from the checkbox) and ignores the
	/// pointer — for settings frozen during a live run.
	#[allow(clippy::too_many_arguments)]
	pub fn toggle_button(&mut self, r: Rect, label: &str, on: bool, enabled: bool, px: f32, w: f32, h: f32, hot: Hot) {
		if enabled {
			self.button_active(r, w, h, on, hot);
		} else {
			self.button_disabled(r, w, h);
		}
		let bx = Rect::new(r.x + 5.0, r.y + (r.h - 11.0) / 2.0, 11.0, 11.0);
		self.field(bx, w, h);
		if on {
			self.rect(
				Rect::new(bx.x + 2.0, bx.y + 2.0, bx.w - 4.0, bx.h - 4.0),
				w,
				h,
				if enabled { theme::ACCENT } else { theme::INK_DIM },
			);
		}
		let ink = if !enabled {
			theme::INK_DIM
		} else if on {
			theme::ACCENT
		} else {
			theme::INK
		};
		self.label_fit(label, r, 20.0, px, w, h, ink);
	}

	/// MAX-font label, top-left at `(x, y)`; returns the advance width.
	pub fn label(&mut self, s: &str, x: f32, y: f32, px: f32, w: f32, h: f32, color: [f32; 4]) -> f32 {
		let size = crate::font::snap(px);
		let advance = text::push_label(&mut self.verts, s, x, y, size, w, h, color);
		self.mark(Batch::Label(size));
		advance
	}

	/// MAX-font label vertically centered in `r`, left-padded by `pad`. Embossed
	/// (a top-left highlight + bottom-right shadow) — the chrome-text path
	/// for buttons, menu items, titles, and captions.
	pub fn label_in(&mut self, s: &str, r: Rect, pad: f32, px: f32, w: f32, h: f32, color: [f32; 4]) {
		let y = r.y + (r.h - px) / 2.0;
		self.label_emboss(s, r.x + pad, y, px, w, h, color);
	}

	/// [`Self::label_in`], but ellipsis-truncated to fit `r` (left pad `pad`,
	/// 4-px right margin) — the path for **dynamic** text in fixed containers
	/// (file names, status lines, tab titles), which must never escape the box.
	pub fn label_fit(&mut self, s: &str, r: Rect, pad: f32, px: f32, w: f32, h: f32, color: [f32; 4]) {
		let fitted = text::fit_label(s, px, (r.w - pad - 4.0).max(0.0));
		self.label_in(&fitted, r, pad, px, w, h, color);
	}

	/// A label drawn three times — shadow (bottom-right), highlight (top-left),
	/// then the ink — for a bevelled, lit-from-top-left look.
	pub fn label_emboss(&mut self, s: &str, x: f32, y: f32, px: f32, w: f32, h: f32, color: [f32; 4]) {
		let size = crate::font::snap(px);
		text::push_label(&mut self.verts, s, x + 1.0, y + 1.0, size, w, h, theme::TEXT_SHADOW);
		text::push_label(&mut self.verts, s, x - 1.0, y - 1.0, size, w, h, theme::TEXT_HILITE);
		text::push_label(&mut self.verts, s, x, y, size, w, h, color);
		self.mark(Batch::Label(size));
	}

	/// Draw a vertical scrollbar in the right [`SCROLLBAR_W`] px of `region` (a
	/// scroll viewport): an inset track + a thumb sized to the visible fraction
	/// and positioned by `scroll`. The thumb brightens under the cursor and
	/// while dragged — "dragged" = the press began inside the track, which
	/// stays true even when the cursor drifts off it mid-drag. A no-op when the
	/// content fits.
	#[allow(clippy::too_many_arguments)]
	pub fn scrollbar(&mut self, region: Rect, content_h: f32, scroll: f32, w: f32, h: f32, hot: Hot) {
		let max = scroll_max(content_h, region.h);
		if max <= 0.0 || region.h <= 0.0 {
			return;
		}
		let track = Rect::new(region.x + region.w - SCROLLBAR_W, region.y, SCROLLBAR_W, region.h);
		self.rect(track, w, h, theme::SCROLL_TRACK);
		let thumb_h = (region.h * (region.h / content_h)).clamp(16.0f32.min(region.h), region.h);
		let t = (scroll / max).clamp(0.0, 1.0);
		let thumb_y = track.y + t * (track.h - thumb_h);
		let thumb = Rect::new(track.x + 1.0, thumb_y + 1.0, SCROLLBAR_W - 2.0, thumb_h - 2.0);
		let dragging = hot.down.is_some_and(|(x, y)| track.contains(x, y));
		let color = if dragging {
			theme::SCROLL_THUMB_DRAG
		} else if hot.hover(thumb) {
			theme::SCROLL_THUMB_HOVER
		} else {
			theme::SCROLL_THUMB
		};
		self.rect(thumb, w, h, color);
	}

	/// Word-wrapped embossed label filling `r` from the top-left (padded by
	/// `pad`, wrapping within `r.w - 2·pad`). Returns the height drawn. Use where
	/// a container can be too narrow for one line — text breaks instead of
	/// overflowing.
	pub fn label_wrapped(&mut self, s: &str, r: Rect, pad: f32, px: f32, w: f32, h: f32, color: [f32; 4]) -> f32 {
		let line_h = px + 4.0;
		let lines = text::wrap_lines(s, px, r.w - 2.0 * pad);
		for (i, line) in lines.iter().enumerate() {
			self.label_emboss(line, r.x + pad, r.y + pad + i as f32 * line_h, px, w, h, color);
		}
		lines.len() as f32 * line_h
	}
}

/// A titlebar'd panel (the dockable-window chrome): a thin `frame`-px raised,
/// mitered bevel (2 px) that also **margins the content** — the titlebar +
/// body sit *inside* the ring, nothing on the border. Returns the close-button
/// hit rect.
#[allow(clippy::too_many_arguments)]
pub fn panel(q: &mut UiQuads, r: Rect, title: &str, dragging: bool, frame: f32, w: f32, h: f32, hot: Hot) -> Rect {
	q.material(r, w, h, theme::PANEL);
	let bar = titlebar_band(r, frame);
	q.material(bar, w, h, if dragging { theme::TITLE_DRAG } else { theme::TITLE });
	q.rect(Rect::new(bar.x, bar.y + TITLEBAR_H - 1.0, bar.w, 1.0), w, h, theme::BEVEL.bottom);
	q.bevel(r, w, h, frame, true);
	q.label_fit(title, titlebar_rect(r, frame), 6.0, FONT_BODY, w, h, theme::SILVER);

	// Close glyph: an "x" label right-aligned in the inset bar, washed under
	// the cursor so it reads as a live control.
	let close = close_rect(r, frame);
	if hot.hover(close) {
		q.rect(close, w, h, if hot.pressed(close) { theme::PRESS } else { theme::HOVER });
	}
	q.label_in("x", close, 6.0, FONT_BODY, w, h, theme::CLOSE_INK);
	close
}

/// Modal frame border width — the thin ring that also margins the content.
pub const MODAL_FRAME: f32 = 2.0;

/// A modal dialog frame: the same chrome as [`panel`] with a 2-px frame
/// and no close glyph (modals dismiss via their own buttons). Title sits inside
/// the ring (borders-as-margin).
pub fn modal_frame(q: &mut UiQuads, r: Rect, title: &str, title_h: f32, w: f32, h: f32) {
	q.material(r, w, h, theme::PANEL);
	let bar = content_box(r, MODAL_FRAME).strip_top(title_h);
	q.material(bar, w, h, theme::TITLE);
	q.rect(Rect::new(bar.x, bar.y + title_h - 1.0, bar.w, 1.0), w, h, theme::BEVEL.bottom);
	q.bevel(r, w, h, MODAL_FRAME, true);
	q.label_in(title, bar, 8.0, FONT_BODY, w, h, theme::ACCENT);
}

/// The full-screen backdrop drawn behind a modal — a 50% dark veil that dims
/// the scene (and blocks interaction with it) while keeping context visible.
pub fn modal_scrim(q: &mut UiQuads, w: f32, h: f32) {
	q.rect(Rect::new(0.0, 0.0, w, h), w, h, [0.0, 0.0, 0.0, 0.5]);
}

pub const TITLEBAR_H: f32 = 22.0;

/// The content box inside a panel's `frame`-px border ring — the area the
/// border *margins off*. Titlebar + body live here; nothing is drawn on the
/// border itself.
pub fn content_box(r: Rect, frame: f32) -> Rect {
	Rect::new(r.x + frame, r.y + frame, (r.w - 2.0 * frame).max(0.0), (r.h - 2.0 * frame).max(0.0))
}

/// The full titlebar band (drag handle + close) inside the border ring.
pub fn titlebar_band(r: Rect, frame: f32) -> Rect {
	content_box(r, frame).strip_top(TITLEBAR_H)
}

/// A panel's content area: inside the border, below the titlebar.
pub fn body_rect(r: Rect, frame: f32) -> Rect {
	let c = content_box(r, frame);
	Rect::new(c.x, c.y + TITLEBAR_H, c.w, (c.h - TITLEBAR_H).max(0.0))
}

/// The titlebar close-button hit area — the right `TITLEBAR_H` square of the
/// inset band.
pub fn close_rect(r: Rect, frame: f32) -> Rect {
	let bar = titlebar_band(r, frame);
	Rect::new(bar.x + bar.w - TITLEBAR_H, bar.y, TITLEBAR_H, TITLEBAR_H)
}

/// The titlebar drag handle (the inset band minus the close square).
pub fn titlebar_rect(r: Rect, frame: f32) -> Rect {
	let bar = titlebar_band(r, frame);
	Rect::new(bar.x, bar.y, (bar.w - TITLEBAR_H).max(0.0), bar.h)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rect_contains_is_half_open() {
		let r = Rect::new(10.0, 10.0, 20.0, 20.0);
		assert!(r.contains(10.0, 10.0));
		assert!(r.contains(29.9, 29.9));
		assert!(!r.contains(30.0, 30.0));
		assert!(!r.contains(9.9, 15.0));
	}

	#[test]
	fn quads_interleave_runs_in_push_order() {
		// Z-order honesty: a panel pushed later must cover an earlier
		// panel's labels, so shape→label→shape→label stays four runs —
		// while consecutive same-atlas pushes coalesce.
		let mut q = UiQuads::default();
		let r = Rect::new(0.0, 0.0, 10.0, 10.0);
		q.rect(r, 100.0, 100.0, [1.0; 4]);
		q.rect(r, 100.0, 100.0, [1.0; 4]); // coalesces with the previous
		q.label("a", 0.0, 0.0, 16.0, 100.0, 100.0, [1.0; 4]);
		q.rect(r, 100.0, 100.0, [1.0; 4]); // a later panel's body
		q.label("b", 0.0, 0.0, 16.0, 100.0, 100.0, [1.0; 4]);
		let kinds: Vec<Batch> = q.runs.iter().map(|(k, _)| *k).collect();
		assert_eq!(kinds, vec![Batch::Shape, Batch::Label(16), Batch::Shape, Batch::Label(16)]);
		// Runs partition the vertex list exactly.
		assert_eq!(q.runs.last().unwrap().1, q.verts.len());
		assert!(q.runs.windows(2).all(|w| w[0].1 < w[1].1));
	}

	#[test]
	fn hot_hover_and_pressed_track_the_pointer() {
		let r = Rect::new(10.0, 10.0, 40.0, 20.0);
		// Inert (headless): nothing highlights.
		assert!(!Hot::NONE.hover(r) && !Hot::NONE.pressed(r));
		// Hover: cursor inside, no press.
		let hover = Hot { cursor: Some((20.0, 15.0)), down: None };
		assert!(hover.hover(r) && !hover.pressed(r));
		// Pressed: the press began inside and the cursor is still inside.
		let pressed = Hot { cursor: Some((20.0, 15.0)), down: Some((12.0, 12.0)) };
		assert!(pressed.pressed(r));
		// Press began elsewhere → hover only (no false "armed" look).
		let stranger = Hot { cursor: Some((20.0, 15.0)), down: Some((500.0, 500.0)) };
		assert!(stranger.hover(r) && !stranger.pressed(r));
		// Dragged off the widget while held → no highlight at all.
		let off = Hot { cursor: Some((500.0, 500.0)), down: Some((12.0, 12.0)) };
		assert!(!off.hover(r) && !off.pressed(r));
	}

	#[test]
	fn titlebar_and_close_partition_the_bar() {
		let r = Rect::new(100.0, 50.0, 200.0, 150.0);
		let frame = 8.0;
		let bar = titlebar_rect(r, frame);
		let close = close_rect(r, frame);
		assert_eq!(bar.h, TITLEBAR_H);
		// Both sit inside the border-as-margin ring.
		assert_eq!(bar.x, r.x + frame, "drag handle inset by the frame");
		assert_eq!(close.x, bar.x + bar.w, "close starts where the drag handle ends");
		assert_eq!(close.x + close.w, r.x + r.w - frame, "close ends at the inner edge");
	}
}
