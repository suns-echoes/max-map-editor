//! **DEV > UI Tests**: the chrome font's raster under a magnifying glass.
//!
//! A content widget over a domain no toolkit can know about - *the rasterizer
//! itself*. It draws the same string many times over a black ground and puts
//! the arithmetic next to each specimen, so a "the text looks resized" report
//! can be settled by looking instead of by argument.
//!
//! # What it shows, and why each part is there
//!
//! - **The roles.** Every [`TextRole`] the chrome uses, each drawn three times:
//!   [`Emboss::Raised`], [`Emboss::Engraved`] and [`Emboss::Flat`]. `Flat` is
//!   the control - one rasterization, no second or third pass offset over it -
//!   so a smear that survives it is the raster's, and a smear that only appears
//!   on the other two is the engraving's.
//! - **The raster ladder.** The same line at one *physical* em size per row.
//!   The widget asks for `px = physical / scale`, which is exactly the inverse
//!   of the bucket the renderer rounds to (`round(px * scale)`), so a row
//!   labelled `24 px` really is rasterized at 24.
//! - **A one-physical-pixel ruler**, for scale: whatever a stem looks like, the
//!   ruler is what a single device pixel looks like beside it.
//!
//! # What it was built to settle, and what it found
//!
//! The editor shipped the wrong chrome face for a long time: `max_square.ttf`,
//! a **pixel-art trace** of the MAX display font. 6770 of its 6830 outline
//! points sat on an exact multiple of 256 font units on a 4096-unit em - a
//! 16-cell grid, every edge axis-aligned, not one off-curve point in the face.
//! A design cell was therefore `raster_px / 16` device pixels, and it was only
//! a *whole* pixel at a raster em divisible by 16. At any other size the
//! rasterizer did the one thing always asked of it and always wrong for pixel
//! art: it resampled, with anti-aliasing, onto a grid that did not line up. On
//! this ladder that showed as hard-edged rows at 16 and 32 and mush at 17, 19,
//! 20, 21, 24 and 26 - and body text buckets to 17px at 100%, 21px at 125% and
//! 26px at 150%, none of them exact. That is what "the text looks like a
//! resized 16px bitmap" was.
//!
//! The fix was to ship the face that was meant to ship:
//! `MAX_Redesign_Square.ttf`, the same design on a 64-unit em with **real
//! curves**. It is metrically identical to the unit - same advances, same
//! 0.9375-em design cell - so nothing moved; it simply rasterizes cleanly at
//! every size. The ladder is what shows that, which is why it stays.
//!
//! Nothing here is chrome and nothing here is reusable: it is an instrument,
//! and it is DEV-only.

use wgpu_ui::text::Fonts;
use wgpu_ui::{DrawCtx, DrawList, Emboss, FontId, LayoutCtx, Rect, Rgba, Size, TextRole, Theme, Vec2, Widget};

use crate::theme as ed;
use crate::uikit_theme::rgba;

/// The role specimens' line: capitals, lower case, digits and the four
/// narrowest shapes in the face (where an uneven raster shows first).
const SAMPLE: &str = "HAMBURGEFONSTIV hamburgefonstiv 0123 |I1l";
/// The ladder's line - shorter, because its rows run up to a 32px em.
const LADDER_SAMPLE: &str = "Hamburgefonstiv 018";

/// The physical em sizes the ladder sweeps: every bucket the three shipped UI
/// scales ask for (13/17 at 100%, 16/21 at 125%, 19/26 at 150%, plus the
/// console's 16/20/24), bracketed by 16 and 32. Under the retired pixel-art
/// face only those two came out crisp; the point of the sweep is that they no
/// longer stand out.
const LADDER: [u32; 9] = [13, 16, 17, 19, 20, 21, 24, 26, 32];

/// The probe's inner padding.
const PAD: f32 = 8.0;
/// Vertical gap between rows, and the wider one between blocks.
const ROW_GAP: f32 = 1.0;
const BLOCK_GAP: f32 = 7.0;
/// Gap between the caption column and the specimen.
const CAPTION_GAP: f32 = 8.0;

/// One laid-out line of the probe.
enum Kind {
	/// A section heading / explanation line: caption text only, full width.
	Head(String),
	/// A caption in the left column and one rendering of a sample beside it.
	Sample {
		caption: String,
		caption_ink: Rgba,
		/// The face to draw the specimen in (the chrome one, or the mono one
		/// for the console role).
		font: FontId,
		/// Em size in **logical** px - what the theme and the renderer take.
		px: f32,
		emboss: Emboss,
		text: &'static str,
		/// Lay the right half of the specimen on [`ed::PROBE_LIT`] instead of
		/// black, so the engraving's shadow has something to fall on.
		lit_half: bool,
	},
	/// Alternating single-*physical*-pixel bars, as a scale reference.
	Ruler,
}

struct Row {
	kind: Kind,
	/// Row height in logical px, gap included.
	h: f32,
	/// Baseline offset from the row's top (shared by the caption and the
	/// specimen, so they sit on one line however different their sizes).
	baseline: f32,
}

/// The whole probe, laid out. Derived from nothing but the fonts, the theme and
/// the scale, so `measure` and `draw` build the same thing and cannot drift.
struct Sheet {
	rows: Vec<Row>,
	/// Width of the left caption column.
	caption_w: f32,
	size: Size,
}

/// The font-size diagnostic. A leaf: it owns no controls, takes no input, and
/// its whole domain is what the rasterizer does with a size.
pub struct FontProbe {
	rect: Rect,
	size: Size,
}

impl Default for FontProbe {
	fn default() -> Self {
		Self::new()
	}
}

impl FontProbe {
	pub fn new() -> Self {
		Self { rect: Rect::ZERO, size: Size::ZERO }
	}

	/// The physical raster size the renderer will bucket `px` to at `scale` -
	/// the same `round(px * scale)` the glyph cache keys on.
	fn raster_px(px: f32, scale: f32) -> f32 {
		(px * scale).round().max(1.0)
	}

	fn role_name(role: TextRole) -> &'static str {
		match role {
			TextRole::Small => "Small",
			TextRole::Body => "Body",
			TextRole::Title => "Title",
			TextRole::Mono => "Mono",
		}
	}

	/// Builds the sheet. `avail_w` only widens it (the rows never wrap; a
	/// specimen is meaningless cut in half), so the probe reports its natural
	/// width and the dialog scrolls if it must.
	fn sheet(&self, fonts: &Fonts, theme: &dyn Theme, scale: f32) -> Sheet {
		let chrome = theme.font();
		let cap_px = theme.font_px(TextRole::Small);
		let cap_m = fonts.metrics(chrome, cap_px);
		let mut rows: Vec<Row> = Vec::new();

		// A caption-only line.
		let head = |rows: &mut Vec<Row>, text: String, gap: f32| {
			rows.push(Row {
				kind: Kind::Head(text),
				h: fonts.line_height(chrome, cap_px) + gap,
				baseline: cap_m.ascent,
			});
		};

		// The size the chrome's body text is actually rasterized at right now -
		// the ladder marks that row, so the sweep stays tied to what is on
		// screen rather than being an abstract sampler.
		let live = Self::raster_px(theme.font_px(TextRole::Body), scale) as u32;
		head(
			&mut rows,
			format!("UI scale {scale:.2}x. Body text is rasterized at {live} px; the ladder marks it."),
			ROW_GAP,
		);
		head(
			&mut rows,
			"Every row below is one rasterization - no size is resampled from another.".to_string(),
			BLOCK_GAP,
		);

		// --- the ruler, up top: it is the reference the rest is read against --
		head(&mut rows, "One physical pixel, on and off:".to_string(), ROW_GAP);
		rows.push(Row { kind: Kind::Ruler, h: fonts.line_height(chrome, cap_px) + BLOCK_GAP, baseline: cap_m.ascent });

		// --- the raster ladder ------------------------------------------------
		head(&mut rows, "Raster ladder - one row per PHYSICAL em px, flat (one pass):".to_string(), ROW_GAP);
		for (i, phys) in LADDER.into_iter().enumerate() {
			// The inverse of the renderer's bucket, so this row really is
			// rasterized at `phys`.
			let px = phys as f32 / scale.max(1e-4);
			let m = fonts.metrics(chrome, px);
			let last = i + 1 == LADDER.len();
			rows.push(Row {
				h: fonts.line_height(chrome, px).max(fonts.line_height(chrome, cap_px))
					+ if last { BLOCK_GAP } else { ROW_GAP },
				baseline: m.ascent.max(cap_m.ascent),
				kind: Kind::Sample {
					caption: if phys == live { format!("{phys} px  <- body") } else { format!("{phys} px") },
					caption_ink: rgba(if phys == live { ed::PROBE_LIVE } else { ed::PROBE_NOTE }),
					font: chrome,
					px,
					emboss: Emboss::Flat,
					text: LADDER_SAMPLE,
					lit_half: false,
				},
			});
		}

		// --- the chrome roles, each rendered three ways -----------------------
		head(&mut rows, "The chrome roles, each raised / engraved / flat:".to_string(), ROW_GAP);
		for role in [TextRole::Small, TextRole::Body, TextRole::Title, TextRole::Mono] {
			let font = theme.font_for(role);
			let px = theme.font_px(role);
			let raster = Self::raster_px(px, scale);
			let note = if font == chrome { "MAX_Redesign_Square" } else { "Hack-Regular" };
			head(
				&mut rows,
				format!("{}  {px:.2} logical -> {raster:.0} px raster   {note}", Self::role_name(role)),
				ROW_GAP,
			);
			for (i, emboss) in [Emboss::Raised, Emboss::Engraved, Emboss::Flat].into_iter().enumerate() {
				let caption = match emboss {
					Emboss::Raised => "raised",
					Emboss::Engraved => "engraved",
					Emboss::Flat => "flat",
				};
				let m = fonts.metrics(font, px);
				let last = i == 2;
				rows.push(Row {
					h: fonts.line_height(font, px).max(fonts.line_height(chrome, cap_px))
						+ if last { BLOCK_GAP } else { ROW_GAP },
					baseline: m.ascent.max(cap_m.ascent),
					kind: Kind::Sample {
						caption: caption.to_string(),
						caption_ink: rgba(ed::PROBE_NOTE),
						font,
						px,
						emboss,
						text: SAMPLE,
						lit_half: true,
					},
				});
			}
		}

		// The caption column is as wide as its widest entry; specimens start
		// after it so every sample shares one left edge.
		let mut caption_w: f32 = 0.0;
		let mut body_w: f32 = 0.0;
		for row in &rows {
			match &row.kind {
				Kind::Head(text) => body_w = body_w.max(fonts.measure(chrome, text, cap_px)),
				Kind::Sample { caption, font, px, text, .. } => {
					caption_w = caption_w.max(fonts.measure(chrome, caption, cap_px));
					body_w = body_w.max(fonts.measure(*font, text, *px) + caption_w + CAPTION_GAP);
				}
				Kind::Ruler => {}
			}
		}
		// `body_w` folded in the caption column as it grew, so a caption found
		// late could have under-measured an earlier specimen; settle it once.
		for row in &rows {
			if let Kind::Sample { font, px, text, .. } = &row.kind {
				body_w = body_w.max(fonts.measure(*font, text, *px) + caption_w + CAPTION_GAP);
			}
		}
		let h: f32 = rows.iter().map(|r| r.h).sum();
		Sheet { rows, caption_w, size: Size::new(body_w + 2.0 * PAD, h + 2.0 * PAD) }
	}
}

impl Widget for FontProbe {
	fn measure(&mut self, _avail: Size, ctx: &mut LayoutCtx) -> Size {
		self.size = self.sheet(ctx.fonts, ctx.theme, ctx.scale).size;
		self.size
	}

	fn arrange(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
		self.rect = rect;
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		let sheet = self.sheet(ctx.fonts, ctx.theme, ctx.scale);
		dl.fill_rect(self.rect, rgba(ed::PROBE_GROUND));
		let chrome = ctx.theme.font();
		let cap_px = ctx.theme.font_px(TextRole::Small);
		let note = rgba(ed::PROBE_NOTE);
		let ink = rgba(ed::PROBE_INK);
		// One physical pixel in logical units - the ruler's bar width.
		let one = 1.0 / ctx.scale.max(1e-4);
		let mut y = self.rect.y + PAD;
		for row in &sheet.rows {
			let base = Vec2::new(self.rect.x + PAD, y + row.baseline);
			match &row.kind {
				Kind::Head(text) => {
					ctx.theme.text_run(dl, ctx.fonts, chrome, base, text, cap_px, Emboss::Flat, note);
				}
				Kind::Sample { caption, caption_ink, font, px, emboss, text, lit_half } => {
					ctx.theme.text_run(dl, ctx.fonts, chrome, base, caption, cap_px, Emboss::Flat, *caption_ink);
					let at = Vec2::new(base.x + sheet.caption_w + CAPTION_GAP, base.y);
					if *lit_half {
						// The half-ground starts mid-specimen, so one line shows
						// the same glyphs on black and on a lit surface.
						let split = at.x + (self.rect.right() - PAD - at.x) * 0.5;
						dl.fill_rect(
							Rect::new(split, y, (self.rect.right() - PAD - split).max(0.0), row.h),
							rgba(ed::PROBE_LIT),
						);
					}
					ctx.theme.text_run(dl, ctx.fonts, *font, at, text, *px, *emboss, ink);
				}
				Kind::Ruler => {
					// Bars laid out in *physical* space and mapped back, so each
					// is exactly one device pixel wide however the scale rounds.
					let left = (self.rect.x + PAD) * ctx.scale;
					let top = (y + row.baseline * 0.25) * ctx.scale;
					let h = (row.baseline * 0.6 * ctx.scale).round().max(1.0);
					let n = ((self.rect.w - 2.0 * PAD) * ctx.scale / 2.0).floor().max(0.0) as u32;
					for i in 0..n {
						let x = (left + (i * 2) as f32).round();
						dl.fill_rect(Rect::new(x / ctx.scale, top.round() / ctx.scale, one, h / ctx.scale), ink);
					}
				}
			}
			y += row.h;
		}
	}

	fn rect(&self) -> Rect {
		self.rect
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The ladder asks for a logical size that buckets back to exactly the
	/// physical size the row is labelled with - at every shipped UI scale. If
	/// this inverse ever breaks, every ladder row is a lie.
	#[test]
	fn ladder_rows_land_on_the_physical_size_they_claim() {
		for scale in [1.0f32, 1.25, 1.5] {
			for phys in LADDER {
				let px = phys as f32 / scale;
				assert_eq!(FontProbe::raster_px(px, scale) as u32, phys, "{phys}px row at {scale}x buckets elsewhere");
			}
		}
	}

	/// Body text's raster size at each shipped scale is a row the ladder
	/// actually sweeps - otherwise the "<- body" marker never appears and the
	/// sheet stops being tied to what is on screen. These are the three sizes
	/// the old pixel-art face could not draw cleanly.
	#[test]
	fn the_ladder_covers_the_size_body_text_lands_on() {
		// SteelTheme's Body nominal (16) through its design-cell correction.
		let body = 16.0 * 64.0 / 60.0;
		for (scale, want) in [(1.0f32, 17u32), (1.25, 21), (1.5, 26)] {
			assert_eq!(FontProbe::raster_px(body, scale) as u32, want, "body at {scale}x");
			assert!(LADDER.contains(&want), "the ladder skips {want}px, which body text uses at {scale}x");
		}
	}
}
