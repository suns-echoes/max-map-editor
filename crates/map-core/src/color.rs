//! RGB ↔ HSL (hand-rolled, no deps) - the palette editor's block re-tint
//! shifts whole water cycle classes in HSL space.
//! H in degrees 0..360, S/L in 0..1.

pub fn rgb_to_hsl([r, g, b]: [u8; 3]) -> (f32, f32, f32) {
	let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
	let max = r.max(g).max(b);
	let min = r.min(g).min(b);
	let l = (max + min) / 2.0;
	if max == min {
		return (0.0, 0.0, l);
	}
	let d = max - min;
	let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
	let h = if max == r {
		(g - b) / d + if g < b { 6.0 } else { 0.0 }
	} else if max == g {
		(b - r) / d + 2.0
	} else {
		(r - g) / d + 4.0
	};
	(h * 60.0, s, l)
}

pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [u8; 3] {
	let h = h.rem_euclid(360.0) / 360.0;
	let (s, l) = (s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
	if s == 0.0 {
		let v = (l * 255.0).round() as u8;
		return [v, v, v];
	}
	let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
	let p = 2.0 * l - q;
	let channel = |mut t: f32| {
		t = t.rem_euclid(1.0);
		let v = if t < 1.0 / 6.0 {
			p + (q - p) * 6.0 * t
		} else if t < 0.5 {
			q
		} else if t < 2.0 / 3.0 {
			p + (q - p) * (2.0 / 3.0 - t) * 6.0
		} else {
			p
		};
		(v * 255.0).round() as u8
	};
	[channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0)]
}

/// Parse the leading `rrggbb` of a hex colour (no `#`; 6 or 8 hex digits, the
/// optional alpha ignored) into RGB. `None` on a bad length or non-hex digit.
pub fn parse_hex_rgb(hex: &str) -> Option<[u8; 3]> {
	if hex.len() != 6 && hex.len() != 8 {
		return None;
	}
	let mut rgb = [0u8; 3];
	for (i, c) in rgb.iter_mut().enumerate() {
		*c = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
	}
	Some(rgb)
}

/// Encode RGB as a `#rrggbb` string.
pub fn rgb_to_hex([r, g, b]: [u8; 3]) -> String {
	format!("#{r:02x}{g:02x}{b:02x}")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn hex_round_trips_and_rejects_bad_input() {
		assert_eq!(parse_hex_rgb("ff8800"), Some([255, 136, 0]));
		assert_eq!(parse_hex_rgb("ff8800cc"), Some([255, 136, 0]), "8-digit alpha ignored");
		assert_eq!(parse_hex_rgb("fff"), None, "wrong length");
		assert_eq!(parse_hex_rgb("gg0000"), None, "non-hex");
		assert_eq!(rgb_to_hex([255, 136, 0]), "#ff8800");
		assert_eq!(parse_hex_rgb(rgb_to_hex([12, 34, 56]).strip_prefix('#').unwrap()), Some([12, 34, 56]));
	}

	#[test]
	fn known_colors_round_trip() {
		for &rgb in &[
			[0u8, 0, 0],
			[255, 255, 255],
			[255, 0, 0],
			[0, 255, 0],
			[0, 0, 255],
			[128, 64, 32],
			[90, 51, 170],
			[1, 254, 127],
		] {
			let (h, s, l) = rgb_to_hsl(rgb);
			let back = hsl_to_rgb(h, s, l);
			for c in 0..3 {
				assert!((back[c] as i16 - rgb[c] as i16).abs() <= 1, "{rgb:?} -> ({h},{s},{l}) -> {back:?}",);
			}
		}
	}

	#[test]
	fn hue_shift_wraps_and_sl_clamp() {
		// Pure red shifted +120° lands on pure green.
		let (h, s, l) = rgb_to_hsl([255, 0, 0]);
		assert_eq!(hsl_to_rgb(h + 120.0, s, l), [0, 255, 0]);
		assert_eq!(hsl_to_rgb(h - 240.0, s, l), [0, 255, 0], "negative wraps");
		// Saturation/lightness clamp instead of overflowing.
		assert_eq!(hsl_to_rgb(0.0, 5.0, 0.5), [255, 0, 0]);
		assert_eq!(hsl_to_rgb(0.0, 1.0, 9.0), [255, 255, 255]);
	}
}
