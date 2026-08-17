//! UI primitives shared by the shell: screen-space rects (re-exported from
//! `wgpu-ui` so chrome and widgets speak one `Rect`), scroll math, and the
//! window-frame geometry. The bespoke quad collector is gone, and so is the
//! `kit::` shim that replaced it (U6.3) - drawing goes through `wgpu-ui`
//! `DrawList`s and the `Theme` trait.
//!
//! The shell's own hover/press snapshot (`Hot`) is gone too (U6.2): every
//! control's hover is its `Ui`'s, the panel frame's is the `Workspace`'s, and
//! what is left of the pointer is the **map's** - `EditorState::cursor`, which
//! is not a widget's business.
//!
//! And there are no font-size constants left (U7.1): a text size is
//! `Theme::font_px(role)`, asked of the theme the surface draws with, so nothing
//! app-side has an opinion about how tall or how wide a label is.

// The screen-space rectangle is `wgpu-ui`'s, so chrome and widgets speak one
// `Rect` with no conversion at the draw boundary.
pub use wgpu_ui::Rect;

/// The full-width `h`-tall strip at the top of `r` - a panel header band /
/// sub-toolbar. (`wgpu_ui::Rect` has no `strip_top`, so this is a free helper.)
pub fn strip_top(r: Rect, h: f32) -> Rect {
	Rect::new(r.x, r.y, r.w, h)
}

/// Max scroll offset so `content` px can reach the bottom of a `view`-tall
/// area: `(content - view)` clamped at zero. Callers clamp the live offset to
/// `0..=scroll_max(..)`.
pub fn scroll_max(content: f32, view: f32) -> f32 {
	(content - view).max(0.0)
}

/// How the steel sheet is sampled by [`crate::uikit_theme::SteelTheme`].
#[derive(Clone, Copy, Default)]
pub enum SteelMap {
	/// The sheet stretched once to fill the whole viewport - the main shell
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
	/// (one non-repeating copy - the larger side spans the full sheet, the other
	/// is centered) and move that crop with the window. Scaling to the window
	/// means the grain never wraps, whatever the size.
	pub fn anchored(window: Rect) -> Self {
		// Larger side → 1.0 of the sheet; keeps the crop inside [0,1] (no repeat).
		let scale = 1.0 / window.w.max(window.h).max(1.0);
		let crop = (0.5 - window.w * 0.5 * scale, 0.5 - window.h * 0.5 * scale);
		SteelMap::Anchored { anchor: (window.x, window.y), crop, scale }
	}

	/// The steel uv corners `[u0, v0, u1, v1]` for screen rect `r` in a
	/// `vw`×`vh` viewport. Public so the `wgpu-ui` [`SteelTheme`] can sample the
	/// sheet through the *same* mapping the native renderer uses - so migrated
	/// panel chrome shares one continuous grain with its still-native content.
	///
	/// [`SteelTheme`]: crate::uikit_theme::SteelTheme
	pub fn uv(self, r: Rect, vw: f32, vh: f32) -> [f32; 4] {
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

pub const TITLEBAR_H: f32 = 22.0;

/// Left padding of a titlebar's title text (docs/ui/theme.md §6.1): 12px left,
/// vertically centred (≈4px top/bottom in the band), for every window/modal.
pub const TITLE_PAD: f32 = 12.0;

/// The content box inside a panel's `frame`-px border ring - the area the
/// border *margins off*. Titlebar + body live here; nothing is drawn on the
/// border itself.
pub fn content_box(r: Rect, frame: f32) -> Rect {
	Rect::new(r.x + frame, r.y + frame, (r.w - 2.0 * frame).max(0.0), (r.h - 2.0 * frame).max(0.0))
}

/// The full titlebar band (drag handle + close) inside the border ring.
pub fn titlebar_band(r: Rect, frame: f32) -> Rect {
	strip_top(content_box(r, frame), TITLEBAR_H)
}

/// A panel's content area: inside the border, below the titlebar.
pub fn body_rect(r: Rect, frame: f32) -> Rect {
	let c = content_box(r, frame);
	Rect::new(c.x, c.y + TITLEBAR_H, c.w, (c.h - TITLEBAR_H).max(0.0))
}

/// The titlebar close-button hit area - the right `TITLEBAR_H` square of the
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
	fn body_rect_sits_below_the_titlebar_inside_the_frame() {
		let r = Rect::new(100.0, 50.0, 200.0, 150.0);
		let frame = 8.0;
		let c = content_box(r, frame);
		let bar = titlebar_band(r, frame);
		let body = body_rect(r, frame);
		assert_eq!(body.y, bar.y + bar.h, "the body starts where the titlebar band ends");
		assert_eq!((body.x, body.w), (c.x, c.w), "the body spans the content box");
		assert_eq!(body.y + body.h, c.y + c.h, "the body reaches the content-box bottom");
		// A window no taller than its titlebar clamps to an empty body, never negative.
		let tiny = body_rect(Rect::new(0.0, 0.0, 40.0, TITLEBAR_H), 4.0);
		assert_eq!(tiny.h, 0.0, "height clamps at zero");
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
