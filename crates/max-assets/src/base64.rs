//! Dependency-free base64 (RFC 4648, standard alphabet, padded).
//!
//! The editor embeds a raw `.DTA` save image inside its JSON project file
//! (`map_core::project`), and JSON strings must be valid UTF-8 — so the binary
//! is base64-encoded on write and decoded on load. House rule: no new
//! dependencies, so this is hand-rolled alongside [`crate::sha256`].

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encodes bytes to a padded base64 string (standard alphabet, no line breaks).
pub fn encode(data: &[u8]) -> String {
	let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
	for chunk in data.chunks(3) {
		let b0 = chunk[0] as u32;
		let b1 = *chunk.get(1).unwrap_or(&0) as u32;
		let b2 = *chunk.get(2).unwrap_or(&0) as u32;
		let n = (b0 << 16) | (b1 << 8) | b2;
		out.push(ALPHABET[(n >> 18) as usize & 0x3f] as char);
		out.push(ALPHABET[(n >> 12) as usize & 0x3f] as char);
		out.push(if chunk.len() > 1 { ALPHABET[(n >> 6) as usize & 0x3f] as char } else { '=' });
		out.push(if chunk.len() > 2 { ALPHABET[n as usize & 0x3f] as char } else { '=' });
	}
	out
}

/// Reverse of [`encode`]. Skips ASCII whitespace (so re-wrapped strings still
/// decode); any other stray byte, or bad padding, is an error.
pub fn decode(text: &str) -> Result<Vec<u8>, String> {
	// Sextet value per input byte: 0..=63, or 255 for the `=` pad terminator.
	fn sextet(c: u8) -> Result<u8, String> {
		Ok(match c {
			b'A'..=b'Z' => c - b'A',
			b'a'..=b'z' => c - b'a' + 26,
			b'0'..=b'9' => c - b'0' + 52,
			b'+' => 62,
			b'/' => 63,
			b'=' => 255,
			_ => return Err(format!("base64: invalid byte {c:#04x}")),
		})
	}

	let mut out = Vec::with_capacity(text.len() / 4 * 3);
	let mut quad = [0u8; 4];
	let mut n = 0usize; // filled slots in the current 4-symbol group
	let mut pads = 0usize; // `=` seen in the current group
	let mut done = false; // a padded (final) group has been consumed
	for &c in text.as_bytes() {
		if c.is_ascii_whitespace() {
			continue;
		}
		if done {
			return Err("base64: data after padding".into());
		}
		let v = sextet(c)?;
		if v == 255 {
			pads += 1;
			quad[n] = 0;
		} else if pads > 0 {
			return Err("base64: data after padding".into());
		} else {
			quad[n] = v;
		}
		n += 1;
		if n == 4 {
			if pads > 2 {
				return Err("base64: invalid padding".into());
			}
			out.push((quad[0] << 2) | (quad[1] >> 4));
			if pads < 2 {
				out.push((quad[1] << 4) | (quad[2] >> 2));
			}
			if pads < 1 {
				out.push((quad[2] << 6) | quad[3]);
			}
			done = pads > 0; // padding may only appear in the final group
			n = 0;
			pads = 0;
		}
	}
	if n != 0 {
		return Err(format!("base64: truncated input ({n} trailing symbols)"));
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	// RFC 4648 §10 test vectors.
	#[test]
	fn rfc4648_vectors() {
		for (bytes, want) in [
			(&b""[..], ""),
			(b"f", "Zg=="),
			(b"fo", "Zm8="),
			(b"foo", "Zm9v"),
			(b"foob", "Zm9vYg=="),
			(b"fooba", "Zm9vYmE="),
			(b"foobar", "Zm9vYmFy"),
		] {
			assert_eq!(encode(bytes), want, "encode {bytes:?}");
			assert_eq!(decode(want).unwrap(), bytes, "decode {want}");
		}
	}

	#[test]
	fn roundtrips_all_byte_values_and_lengths() {
		for len in 0..=64usize {
			let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
			assert_eq!(decode(&encode(&data)).unwrap(), data, "len {len}");
		}
	}

	#[test]
	fn decode_skips_whitespace() {
		assert_eq!(decode("Zm9v\nYmFy").unwrap(), b"foobar");
		assert_eq!(decode("  Zg ==  ").unwrap(), b"f");
	}

	#[test]
	fn decode_rejects_garbage_and_truncation() {
		assert!(decode("Zm9v*bad").is_err(), "stray symbol");
		assert!(decode("Zm9").is_err(), "unpadded truncation");
		assert!(decode("Zg==Zg==").is_err(), "data after padding");
	}
}
