//! Palette file IO: read/write a 256-colour palette as JSON.
//!
//! Two read forms are accepted - the bare `["#rrggbb", …×256]` array (the
//! tileset `palette.json` format, see [`crate::TilePack`]) and the richer
//! `{ "name", "version", "date", "author", "colors": ["#rrggbb", …×256] }`
//! object. Writing always emits the object form (carrying a name + version);
//! extra fields on read are ignored.

/// Parse a palette JSON document into 768 RGB bytes (256 × `[r, g, b]`).
pub fn parse_palette(text: &str) -> Result<Vec<u8>, String> {
	let value = json::parse(text).map_err(|e| format!("palette: {e}"))?;
	// A bare array, or an object carrying a `colors` array.
	let colors = value
		.as_array()
		.or_else(|| value.get("colors").and_then(|c| c.as_array()))
		.ok_or("palette: expected an array of 256 colours, or { \"colors\": [...] }")?;
	if colors.len() != 256 {
		return Err(format!("palette: {} colours, want 256", colors.len()));
	}
	let mut rgb = Vec::with_capacity(768);
	for color in colors {
		let hex =
			color.as_str().and_then(|s| s.strip_prefix('#')).ok_or("palette: bad colour entry (want \"#rrggbb\")")?;
		let parsed = crate::color::parse_hex_rgb(hex)
			.ok_or_else(|| format!("palette: bad colour '#{hex}' (want 6 or 8 hex digits)"))?;
		rgb.extend_from_slice(&parsed);
	}
	Ok(rgb)
}

/// Serialize 768 RGB bytes as the object form, named `name`.
pub fn write_palette(palette: &[u8], name: &str) -> String {
	let name = name.replace('\\', "\\\\").replace('"', "\\\"");
	let mut s = String::with_capacity(256 * 12 + 96);
	s.push_str("{\n");
	s.push_str(&format!("\t\"name\": \"{name}\",\n"));
	s.push_str("\t\"version\": \"1.0.0\",\n");
	s.push_str("\t\"colors\": [\n");
	for i in 0..256 {
		let at = i * 3;
		let (r, g, b) = (palette[at], palette[at + 1], palette[at + 2]);
		let comma = if i < 255 { "," } else { "" };
		s.push_str(&format!("\t\t\"{}\"{comma}\n", crate::color::rgb_to_hex([r, g, b])));
	}
	s.push_str("\t]\n}\n");
	s
}

/// Read slot `slot`'s RGB triple from a 768-byte (256 × RGB) palette.
#[inline]
pub fn slot_rgb(palette: &[u8], slot: u8) -> [u8; 3] {
	let at = slot as usize * 3;
	[palette[at], palette[at + 1], palette[at + 2]]
}

/// Overwrite slot `slot`'s RGB triple in a 768-byte (256 × RGB) palette.
#[inline]
pub fn set_slot_rgb(palette: &mut [u8], slot: u8, rgb: [u8; 3]) {
	let at = slot as usize * 3;
	palette[at..at + 3].copy_from_slice(&rgb);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn round_trips_through_the_object_form() {
		let mut pal = vec![0u8; 768];
		for (i, b) in pal.iter_mut().enumerate() {
			*b = (i % 256) as u8;
		}
		let text = write_palette(&pal, "test");
		assert!(text.contains("\"name\": \"test\""));
		assert_eq!(parse_palette(&text).unwrap(), pal);
	}

	#[test]
	fn reads_the_bare_array_form_too() {
		let mut arr = String::from("[\n");
		for i in 0..256 {
			arr.push_str(&format!("\t\"#{i:02x}0000\"{}\n", if i < 255 { "," } else { "" }));
		}
		arr.push(']');
		let rgb = parse_palette(&arr).unwrap();
		assert_eq!(rgb.len(), 768);
		assert_eq!(rgb[3 * 5], 5, "slot 5 red = 0x05");
		assert_eq!(rgb[3 * 5 + 1], 0);
	}

	#[test]
	fn rejects_wrong_length_and_bad_hex() {
		assert!(parse_palette("[\"#ffffff\"]").is_err(), "too few");
		assert!(parse_palette("{\"colors\": []}").is_err(), "empty object");
		assert!(parse_palette("\"nope\"").is_err(), "not array/object");
	}
}
