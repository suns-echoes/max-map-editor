//! Minimal JSON parser + serializer — no dependencies.
//!
//! Parser extended from re-MAX's `help.rs` scanner (objects/strings/numbers)
//! with arrays, booleans and null, which the map-editor data files need.
//! Object keys keep insertion order (Vec, not HashMap) so round-trip saves
//! diff cleanly against files written by `JSON.stringify(data, null, '\t')`.

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
	Null,
	Bool(bool),
	Number(f64),
	String(String),
	Array(Vec<JsonValue>),
	Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
	pub fn as_object(&self) -> Option<&[(String, JsonValue)]> {
		if let JsonValue::Object(v) = self { Some(v) } else { None }
	}
	pub fn as_array(&self) -> Option<&[JsonValue]> {
		if let JsonValue::Array(v) = self { Some(v) } else { None }
	}
	pub fn as_str(&self) -> Option<&str> {
		if let JsonValue::String(s) = self { Some(s.as_str()) } else { None }
	}
	pub fn as_f64(&self) -> Option<f64> {
		if let JsonValue::Number(n) = self { Some(*n) } else { None }
	}
	pub fn as_bool(&self) -> Option<bool> {
		if let JsonValue::Bool(b) = self { Some(*b) } else { None }
	}
	/// Object field lookup (insertion-ordered scan).
	pub fn get(&self, key: &str) -> Option<&JsonValue> {
		self.as_object()?.iter().find(|(k, _)| k == key).map(|(_, v)| v)
	}
}

/// Nesting limit for arrays/objects. The parser is recursive-descent, so an
/// adversarial deeply-nested document (`[[[[…]]]]`) would otherwise overflow
/// the stack — an uncatchable `SIGABRT`, not a recoverable error. Project
/// files nest only a handful of levels; 128 is comfortably generous.
const MAX_DEPTH: u32 = 128;

pub fn parse(s: &str) -> Result<JsonValue, String> {
	let mut sc = Scanner { src: s.as_bytes(), pos: 0, depth: 0 };
	let v = sc.parse_value()?;
	sc.skip_ws();
	if sc.pos != sc.src.len() {
		return Err(format!("trailing input at byte {}", sc.pos));
	}
	Ok(v)
}

struct Scanner<'a> {
	src: &'a [u8],
	pos: usize,
	/// Current array/object nesting depth (guarded by [`MAX_DEPTH`]).
	depth: u32,
}

impl<'a> Scanner<'a> {
	fn skip_ws(&mut self) {
		while let Some(&b) = self.src.get(self.pos) {
			if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
				self.pos += 1;
			} else {
				break;
			}
		}
	}

	fn peek(&self) -> Option<u8> {
		self.src.get(self.pos).copied()
	}

	fn bump(&mut self) -> Option<u8> {
		let b = self.peek()?;
		self.pos += 1;
		Some(b)
	}

	fn expect(&mut self, b: u8) -> Result<(), String> {
		self.skip_ws();
		match self.bump() {
			Some(c) if c == b => Ok(()),
			Some(c) => Err(format!("expected '{}' got '{}' at byte {}", b as char, c as char, self.pos)),
			None => Err(format!("expected '{}' at EOF", b as char)),
		}
	}

	fn eat_keyword(&mut self, word: &str) -> bool {
		if self.src[self.pos..].starts_with(word.as_bytes()) {
			self.pos += word.len();
			true
		} else {
			false
		}
	}

	fn parse_value(&mut self) -> Result<JsonValue, String> {
		self.skip_ws();
		match self.peek() {
			Some(b'"') => Ok(JsonValue::String(self.parse_string()?)),
			Some(b'{') => self.parse_object(),
			Some(b'[') => self.parse_array(),
			Some(b'-') | Some(b'0'..=b'9') => Ok(JsonValue::Number(self.parse_number()?)),
			Some(b't') if self.eat_keyword("true") => Ok(JsonValue::Bool(true)),
			Some(b'f') if self.eat_keyword("false") => Ok(JsonValue::Bool(false)),
			Some(b'n') if self.eat_keyword("null") => Ok(JsonValue::Null),
			Some(c) => Err(format!("unexpected '{}' at byte {}", c as char, self.pos)),
			None => Err("unexpected EOF".to_string()),
		}
	}

	/// Enter a nested container, erroring past [`MAX_DEPTH`] (the recursion
	/// guard). Paired with [`Scanner::leave`] on the way out.
	fn enter(&mut self) -> Result<(), String> {
		self.depth += 1;
		if self.depth > MAX_DEPTH {
			return Err(format!("nesting deeper than {MAX_DEPTH} levels at byte {}", self.pos));
		}
		Ok(())
	}

	fn leave(&mut self) {
		self.depth -= 1;
	}

	fn parse_object(&mut self) -> Result<JsonValue, String> {
		self.expect(b'{')?;
		self.enter()?;
		let mut items = Vec::new();
		self.skip_ws();
		if self.peek() == Some(b'}') {
			self.bump();
			self.leave();
			return Ok(JsonValue::Object(items));
		}
		loop {
			let key = self.parse_string()?;
			self.expect(b':')?;
			let value = self.parse_value()?;
			items.push((key, value));
			self.skip_ws();
			match self.bump() {
				Some(b',') => continue,
				Some(b'}') => {
					self.leave();
					return Ok(JsonValue::Object(items));
				}
				other => return Err(format!("expected ',' or '}}' got {:?}", other.map(|b| b as char))),
			}
		}
	}

	fn parse_array(&mut self) -> Result<JsonValue, String> {
		self.expect(b'[')?;
		self.enter()?;
		let mut items = Vec::new();
		self.skip_ws();
		if self.peek() == Some(b']') {
			self.bump();
			self.leave();
			return Ok(JsonValue::Array(items));
		}
		loop {
			items.push(self.parse_value()?);
			self.skip_ws();
			match self.bump() {
				Some(b',') => continue,
				Some(b']') => {
					self.leave();
					return Ok(JsonValue::Array(items));
				}
				other => return Err(format!("expected ',' or ']' got {:?}", other.map(|b| b as char))),
			}
		}
	}

	fn parse_string(&mut self) -> Result<String, String> {
		self.skip_ws();
		self.expect(b'"')?;
		let mut out = String::new();
		while let Some(b) = self.bump() {
			match b {
				b'"' => return Ok(out),
				b'\\' => match self.bump() {
					Some(b'"') => out.push('"'),
					Some(b'\\') => out.push('\\'),
					Some(b'/') => out.push('/'),
					Some(b'n') => out.push('\n'),
					Some(b't') => out.push('\t'),
					Some(b'r') => out.push('\r'),
					Some(b'b') => out.push('\u{0008}'),
					Some(b'f') => out.push('\u{000C}'),
					Some(b'u') => {
						let mut cp: u32 = 0;
						for _ in 0..4 {
							let d = self.bump().ok_or("unterminated \\u")?;
							cp = (cp << 4) | hex_digit(d)? as u32;
						}
						out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
					}
					_ => return Err("bad escape".to_string()),
				},
				_ => {
					// Multibyte UTF-8 — the source &str is already valid.
					let start = self.pos - 1;
					while let Some(&n) = self.src.get(self.pos) {
						if n & 0xC0 == 0x80 {
							self.pos += 1;
						} else {
							break;
						}
					}
					out.push_str(
						std::str::from_utf8(&self.src[start..self.pos]).map_err(|e| format!("bad utf8: {e}"))?,
					);
				}
			}
		}
		Err("unterminated string".to_string())
	}

	fn parse_number(&mut self) -> Result<f64, String> {
		self.skip_ws();
		let start = self.pos;
		while let Some(b) = self.peek() {
			match b {
				b'-' | b'+' | b'0'..=b'9' | b'.' | b'e' | b'E' => self.pos += 1,
				_ => break,
			}
		}
		let s = std::str::from_utf8(&self.src[start..self.pos]).map_err(|e| e.to_string())?;
		s.parse::<f64>().map_err(|e| e.to_string())
	}
}

fn hex_digit(b: u8) -> Result<u8, String> {
	match b {
		b'0'..=b'9' => Ok(b - b'0'),
		b'a'..=b'f' => Ok(b - b'a' + 10),
		b'A'..=b'F' => Ok(b - b'A' + 10),
		_ => Err(format!("bad hex digit {}", b as char)),
	}
}

// ----- Serializer -----------------------------------------------------------

impl JsonValue {
	/// Pretty-print with tab indentation — byte-compatible with
	/// `JSON.stringify(data, null, '\t')` for the value shapes we use.
	pub fn to_pretty(&self) -> String {
		let mut out = String::new();
		self.write(&mut out, 0);
		out
	}

	fn write(&self, out: &mut String, depth: usize) {
		match self {
			JsonValue::Null => out.push_str("null"),
			JsonValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
			JsonValue::Number(n) => write_number(out, *n),
			JsonValue::String(s) => write_string(out, s),
			JsonValue::Array(items) => {
				if items.is_empty() {
					out.push_str("[]");
					return;
				}
				out.push('[');
				for (i, item) in items.iter().enumerate() {
					if i > 0 {
						out.push(',');
					}
					out.push('\n');
					indent(out, depth + 1);
					item.write(out, depth + 1);
				}
				out.push('\n');
				indent(out, depth);
				out.push(']');
			}
			JsonValue::Object(items) => {
				if items.is_empty() {
					out.push_str("{}");
					return;
				}
				out.push('{');
				for (i, (key, value)) in items.iter().enumerate() {
					if i > 0 {
						out.push(',');
					}
					out.push('\n');
					indent(out, depth + 1);
					write_string(out, key);
					out.push_str(": ");
					value.write(out, depth + 1);
				}
				out.push('\n');
				indent(out, depth);
				out.push('}');
			}
		}
	}
}

fn indent(out: &mut String, depth: usize) {
	for _ in 0..depth {
		out.push('\t');
	}
}

fn write_string(out: &mut String, s: &str) {
	out.push('"');
	for ch in s.chars() {
		match ch {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\t' => out.push_str("\\t"),
			'\r' => out.push_str("\\r"),
			c if (c as u32) < 0x20 => {
				out.push_str(&format!("\\u{:04x}", c as u32));
			}
			c => out.push(c),
		}
	}
	out.push('"');
}

fn write_number(out: &mut String, n: f64) {
	if n.fract() == 0.0 && n.abs() < 1e15 {
		out.push_str(&format!("{}", n as i64));
	} else {
		out.push_str(&format!("{n}"));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_all_value_kinds() {
		let v = parse(r#"{"a": [1, -2.5, "x", true, false, null], "b": {"c": "żółć"}}"#).unwrap();
		assert_eq!(v.get("a").unwrap().as_array().unwrap().len(), 6);
		assert_eq!(v.get("a").unwrap().as_array().unwrap()[1].as_f64(), Some(-2.5));
		assert_eq!(v.get("a").unwrap().as_array().unwrap()[3].as_bool(), Some(true));
		assert_eq!(v.get("b").unwrap().get("c").unwrap().as_str(), Some("żółć"));
	}

	#[test]
	fn rejects_garbage() {
		assert!(parse("{").is_err());
		assert!(parse("[1,]").is_err());
		assert!(parse("{} extra").is_err());
		assert!(parse("nope").is_err());
	}

	#[test]
	fn pretty_round_trips_stringify_style() {
		// Mirrors JSON.stringify(data, null, '\t') output shape.
		let src = "{\n\t\"version\": \"1\",\n\t\"size\": 112,\n\t\"use\": [\n\t\t{\n\t\t\t\"name\": \"WATER\",\n\t\t\t\"tileset\": true\n\t\t}\n\t],\n\t\"empty\": []\n}";
		let v = parse(src).unwrap();
		assert_eq!(v.to_pretty(), src);
	}

	#[test]
	fn object_key_order_is_preserved() {
		let v = parse(r#"{"z": 1, "a": 2, "m": 3}"#).unwrap();
		let keys: Vec<&str> = v.as_object().unwrap().iter().map(|(k, _)| k.as_str()).collect();
		assert_eq!(keys, ["z", "a", "m"]);
	}

	#[test]
	fn deep_nesting_errors_instead_of_overflowing_the_stack() {
		// SEV-3 regression: a crafted document nested past MAX_DEPTH must
		// return an error, not recurse until the stack aborts (SIGABRT).
		let deep = "[".repeat(200_000) + &"]".repeat(200_000);
		let err = parse(&deep).unwrap_err();
		assert!(err.contains("nesting deeper than"), "{err}");
		// Mixed arrays/objects count toward the same limit.
		let mixed = "[{\"a\":".repeat(200_000);
		assert!(parse(&mixed).is_err());
		// Right at the limit still parses (128 levels), so real files are safe.
		let ok = "[".repeat(MAX_DEPTH as usize) + &"]".repeat(MAX_DEPTH as usize);
		assert!(parse(&ok).is_ok());
		// One past the limit is rejected.
		let over = "[".repeat(MAX_DEPTH as usize + 1) + &"]".repeat(MAX_DEPTH as usize + 1);
		assert!(parse(&over).unwrap_err().contains("nesting deeper than"));
	}
}
