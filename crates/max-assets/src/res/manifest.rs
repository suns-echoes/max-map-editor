//! Loader for `apps/game/assets/config/res.ini` - the per-tag manifest
//! describing which MAX.RES entries the game should load and (for
//! simple-images) what palette index to treat as transparent.
//!
//! The file is hand-curated under the `[simple_image]` section and
//! sourced from `sandbox/resources/analysis.xlsx`. Format per line:
//!
//! ```text
//! TAG = <0..255>          ; palette index treated as transparent
//! TAG = opaque            ; render solid (no transparency)
//! TAG = skip              ; intentionally not loaded
//! ```
//!
//! Tags absent from the section are treated as `skip`. Invalid values
//! produce a warning on load and fall back to `skip`.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::image::SimpleImageTransparency;

/// What the manifest says about one simple-image tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimpleImageEntry {
	/// Load the image and apply this transparency policy.
	Load(SimpleImageTransparency),
	/// Present in MAX.RES but intentionally excluded (saves atlas space
	/// and avoids decoding artwork the game never uses).
	Skip,
}

/// Per-tag metadata extracted from `res.ini`.
#[derive(Default, Debug, Clone)]
pub struct ResManifest {
	pub simple_image: HashMap<String, SimpleImageEntry>,
}

impl ResManifest {
	/// Returns the policy for `tag`, or `Skip` when the tag isn't listed.
	pub fn simple(&self, tag: &str) -> SimpleImageEntry {
		self.simple_image.get(tag).copied().unwrap_or(SimpleImageEntry::Skip)
	}
}

/// Reads `res.ini` from disk and returns just the `[simple_image]`
/// section parsed into `ResManifest`. Other sections are ignored - they
/// remain inventory documentation for now.
pub fn load_res_manifest(path: &Path) -> std::io::Result<ResManifest> {
	let text = fs::read_to_string(path)?;
	Ok(parse_res_manifest(&text))
}

/// Pure-text parser. Exposed separately so tests don't need a tempfile.
pub fn parse_res_manifest(text: &str) -> ResManifest {
	let mut out = ResManifest::default();
	let mut in_simple = false;
	for raw_line in text.lines() {
		let line = strip_comment(raw_line).trim();
		if line.is_empty() {
			continue;
		}
		if let Some(name) = section_name(line) {
			in_simple = name == "simple_image";
			continue;
		}
		if !in_simple {
			continue;
		}
		let Some((tag, val)) = split_key_value(line) else { continue };
		let entry = match val {
			"skip" => SimpleImageEntry::Skip,
			"opaque" => SimpleImageEntry::Load(SimpleImageTransparency::Opaque),
			s => match s.parse::<u32>() {
				Ok(idx) if idx <= 255 => SimpleImageEntry::Load(SimpleImageTransparency::Index(idx as u8)),
				_ => {
					log::warn!("res.ini: tag {tag:?} has unrecognized value {val:?}; treating as skip",);
					SimpleImageEntry::Skip
				}
			},
		};
		out.simple_image.insert(tag.to_string(), entry);
	}
	out
}

fn strip_comment(line: &str) -> &str {
	match line.find(';') {
		Some(i) => &line[..i],
		None => line,
	}
}

fn section_name(line: &str) -> Option<&str> {
	let t = line.trim();
	if t.starts_with('[') && t.ends_with(']') && t.len() >= 2 { Some(&t[1..t.len() - 1]) } else { None }
}

fn split_key_value(line: &str) -> Option<(&str, &str)> {
	let eq = line.find('=')?;
	let key = line[..eq].trim();
	let val = line[eq + 1..].trim();
	if key.is_empty() {
		return None;
	}
	Some((key, val))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_index_opaque_skip() {
		let text = r#"
[simple_image]
ZOOMPTR = 16
ZOOMPNL1 = 0
SOMEBG = opaque
ACTVT_OF = skip
[other]
IGNORED = whatever
"#;
		let m = parse_res_manifest(text);
		assert_eq!(m.simple("ZOOMPTR"), SimpleImageEntry::Load(SimpleImageTransparency::Index(16)));
		assert_eq!(m.simple("ZOOMPNL1"), SimpleImageEntry::Load(SimpleImageTransparency::Index(0)));
		assert_eq!(m.simple("SOMEBG"), SimpleImageEntry::Load(SimpleImageTransparency::Opaque));
		assert_eq!(m.simple("ACTVT_OF"), SimpleImageEntry::Skip);
		// Tag in another section is not picked up.
		assert_eq!(m.simple("IGNORED"), SimpleImageEntry::Skip);
		// Missing tag defaults to skip.
		assert_eq!(m.simple("NEVER_HEARD_OF_IT"), SimpleImageEntry::Skip);
	}

	#[test]
	fn comments_and_whitespace_tolerated() {
		let text = r#"
; leading comment
[simple_image]
  TAG_A = 5     ; trailing comment
  TAG_B = opaque
"#;
		let m = parse_res_manifest(text);
		assert_eq!(m.simple("TAG_A"), SimpleImageEntry::Load(SimpleImageTransparency::Index(5)));
		assert_eq!(m.simple("TAG_B"), SimpleImageEntry::Load(SimpleImageTransparency::Opaque));
	}

	#[test]
	fn invalid_value_logs_and_skips() {
		let text = "[simple_image]\nFOO = potato\n";
		let m = parse_res_manifest(text);
		assert_eq!(m.simple("FOO"), SimpleImageEntry::Skip);
	}
}
