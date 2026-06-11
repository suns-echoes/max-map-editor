use std::{
	collections::HashSet,
	fs::File,
	io::{BufRead, BufReader},
	path::{Path, PathBuf},
};

use crate::{INI, INISection};

fn create_error_message(message: &str, file_path: Option<&Path>, line_number: usize) -> String {
	if let Some(file_path) = file_path {
		format!("{}\n    error at {}:{}", message, file_path.display(), line_number)
	} else {
		format!("{}\n    error at <string>:{}", message, line_number)
	}
}

/// Cap on `[[mod ]]` include nesting. Each level opens another file and
/// recurses, so without a bound a deep (or generated) include chain could
/// overflow the stack. Real mod trees nest only a few levels deep.
const MAX_INCLUDE_DEPTH: u32 = 32;

fn recursive_parse_ini(
	ini: &mut INI,
	ini_lines: &mut dyn Iterator<Item = Result<String, String>>, // Accept a boxed trait object
	file_path: Option<&Path>,
	parsed_files: &mut Option<&mut HashSet<PathBuf>>,
	depth: u32,
) -> Result<(), String> {
	let mut current_section_name: Option<String> = None;

	for (line_number, line) in ini_lines.enumerate() {
		let line = line.map_err(|_| create_error_message("Failed to read line", file_path, line_number))?;
		let trimmed_line = line.trim();

		if trimmed_line.is_empty() || trimmed_line.starts_with(';') || trimmed_line.starts_with('#') {
			continue;
		}

		if trimmed_line.starts_with("[[mod ") && trimmed_line.ends_with("]]") {
			if let Some(file_path_unpacked) = file_path {
				if depth >= MAX_INCLUDE_DEPTH {
					return Err(create_error_message(
						&format!("Module inclusion nested deeper than {MAX_INCLUDE_DEPTH} levels"),
						file_path,
						line_number + 1,
					));
				}
				let mod_identifier = trimmed_line[6..trimmed_line.len() - 2].trim();

				if mod_identifier.is_empty() {
					return Err(create_error_message("Module identifier cannot be empty", file_path, line_number + 1));
				}

				// Check for non-ASCII printable characters in mod_identifier
				if !mod_identifier.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
					return Err(create_error_message(
						"Module identifier cannot contain non-ASCII or non-printable characters",
						file_path,
						line_number + 1,
					));
				}

				let mut mod_path = file_path_unpacked
					.parent()
					.unwrap_or_else(|| Path::new(""))
					.join(format!("{}.ini", mod_identifier));
				if !mod_path.exists() {
					let dir_path = file_path_unpacked.parent().unwrap_or_else(|| Path::new("")).join(mod_identifier);
					let mod_ini_path = dir_path.join("mod.ini");
					if mod_ini_path.exists() {
						mod_path = mod_ini_path;
					} else {
						return Err(create_error_message(
							&format!(
								"Included module not found: tried '{}' and '{}'",
								mod_path.display(),
								mod_ini_path.display()
							),
							file_path,
							line_number + 1,
						));
					}
				}

				if let Some(ref mut parsed_files) = parsed_files.as_deref_mut() {
					if parsed_files.contains(&mod_path) {
						return Err(create_error_message(
							"Circular module inclusion",
							Some(&mod_path),
							line_number + 1,
						));
					}
					parsed_files.insert(mod_path.to_path_buf());
				}

				let included_file = File::open(&mod_path).map_err(|e| {
					create_error_message(
						&format!("Failed to open included module file '{}': {}", mod_path.display(), e),
						file_path,
						line_number + 1,
					)
				})?;

				let reader = BufReader::new(included_file);
				let mut included_lines: Box<dyn Iterator<Item = Result<String, String>>> =
					Box::new(reader.lines().map(|l| l.map_err(|e| e.to_string())));

				recursive_parse_ini(ini, &mut *included_lines, Some(&mod_path), parsed_files, depth + 1)?;
				continue;
			} else {
				return Err(create_error_message(
					"Module inclusion not allowed in string input",
					None,
					line_number + 1,
				));
			}
		}

		if trimmed_line.starts_with('[') && trimmed_line.ends_with(']') {
			let section_name = trimmed_line[1..trimmed_line.len() - 1].trim().to_string();

			if section_name.is_empty() {
				return Err(create_error_message("Section name cannot be empty", file_path, line_number + 1));
			}

			// Re-opening an existing section is allowed and merges into it:
			// later `key=value` lines append new entries and override any
			// duplicate keys. This is what lets mod modules layer on top of
			// a base config — matches the contract in `assets/config/main.ini`.
			if !ini.has_section(&section_name) {
				ini.insert_section(section_name.clone(), INISection::new());
			}
			current_section_name = Some(section_name);
			continue;
		}

		if let Some((key, value)) = trimmed_line.split_once('=') {
			let key = key.trim().to_string();
			let value = value.trim();

			if key.is_empty() {
				return Err(create_error_message("Key name cannot be empty", file_path, line_number + 1));
			}

			let current_section = if let Some(section_name) = &current_section_name {
				ini.get_section_mut(section_name)
					.ok_or_else(|| format!("Internal error: Section '{}' not found", section_name))?
			} else {
				return Err(create_error_message(
					"Key-value pair cannot be defined outside of a section",
					file_path,
					line_number + 1,
				));
			};

			if let Ok(number) = value.parse::<i64>() {
				current_section.set_entry(key, number)?;
			} else if let Ok(f) = value.parse::<f64>() {
				// Must come after the i64 attempt: `"5"` parses as both,
				// and callers that read back via `get_entry::<i64>` rely on
				// bare integers staying classified as Integer.
				current_section.set_entry(key, f)?;
			} else if value == "false" || value == "no" {
				current_section.set_entry(key, false)?;
			} else if value == "true" || value == "yes" {
				current_section.set_entry(key, true)?;
			} else {
				let value_str = value.trim_matches('"').to_string();
				current_section.set_entry(key, value_str)?;
			}
		}
	}

	//// parsed_files.remove(file_path);
	Ok(())
}

pub fn parse_ini_file(file_path: &Path) -> Result<INI, String> {
	let file = File::open(file_path).map_err(|e| format!("Failed to open file '{}': {}", file_path.display(), e))?;
	let reader = BufReader::new(file);
	let mut lines = reader.lines().map(|l| l.map_err(|e| e.to_string()));

	let mut root = INI::new();
	let mut parsed_files = HashSet::new();

	let mut parsed_files_opt = Some(&mut parsed_files);
	recursive_parse_ini(&mut root, &mut lines, Some(file_path), &mut parsed_files_opt, 0)?;

	Ok(root)
}

pub fn parse_ini_str(ini_content: &str) -> Result<INI, String> {
	let mut lines = ini_content.lines().map(|line| Ok::<String, String>(line.to_string()));
	let mut root = INI::new();
	let mut none_files: Option<&mut HashSet<PathBuf>> = None;
	recursive_parse_ini(&mut root, &mut lines, None, &mut none_files, 0)?;

	Ok(root)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_empty_ini() {
		// Act
		let result = parse_ini_str("");

		// Assert
		assert!(result.is_ok());
		let ini = result.unwrap();
		assert!(ini.is_empty());
	}

	#[test]
	fn test_parse_ini_string() {
		// Arrange
		let ini_str = r"
			; This is a comment

			[section1]
			key1 = value
			key2 = 42

			[section2]
		";

		// Act
		let result = parse_ini_str(ini_str);

		// Assert
		assert!(result.is_ok());
		let ini = result.unwrap();
		let section1 = ini.get_section("section1").unwrap();

		let value = section1.get_entry::<String>("key1").unwrap();
		assert_eq!(value, "value");
		let number: i64 = section1.get_entry("key2").unwrap();
		assert_eq!(number, 42);

		let section2 = ini.get_section("section2").unwrap();
		assert!(section2.is_empty());
	}

	#[test]
	fn test_parse_classifies_numeric_values() {
		let ini = parse_ini_str("[s]\n int = 42\n neg = -3\n f = 5.7\n exact = 5.0\n").unwrap();
		let s = ini.get_section("s").unwrap();
		// Bare integer — must stay an i64 so callers reading i64 keep working.
		assert_eq!(s.get_entry::<i64>("int"), Some(42));
		assert_eq!(s.get_entry::<f64>("int"), None);
		assert_eq!(s.get_entry::<i64>("neg"), Some(-3));
		// Values with a decimal point classify as f64 even when the fraction
		// is zero — that's what lets `5.0` round-trip through `Display`.
		assert_eq!(s.get_entry::<f64>("f"), Some(5.7));
		assert_eq!(s.get_entry::<f64>("exact"), Some(5.0));
		assert_eq!(s.get_entry::<i64>("f"), None);
	}

	#[test]
	fn test_parse_preserves_semicolon_inside_value() {
		// `;` only starts a comment at the beginning of a line; inside a
		// value it's a regular character. The game's save format uses
		// `;`-separated waypoints (e.g. `path = 6,5;7,5;8,5`), which
		// relies on this.
		let ini = parse_ini_str("[s]\n path = 6,5;7,5;8,5\n").unwrap();
		let s = ini.get_section("s").unwrap();
		assert_eq!(s.get_entry::<String>("path"), Some("6,5;7,5;8,5".to_string()),);
	}

	#[test]
	fn test_parse_trims_quotes_from_string_values() {
		// Arrange
		let ini_str = r#"
            [section]
            quoted = "hello world"
            unquoted = hello
        "#;

		// Act
		let result = parse_ini_str(ini_str);

		// Assert
		assert!(result.is_ok());
		let ini = result.unwrap();
		let section = ini.get_section("section").unwrap();
		assert_eq!(section.get_entry::<String>("quoted").unwrap(), "hello world");
		assert_eq!(section.get_entry::<String>("unquoted").unwrap(), "hello");
	}

	#[test]
	fn test_parse_handles_whitespace_around_keys_and_values() {
		// Arrange
		let ini_str = r"
			[section]
			key1    =    value1
			key2=42
		";

		// Act
		let result = parse_ini_str(ini_str);

		// Assert
		assert!(result.is_ok());
		let ini = result.unwrap();
		let section = ini.get_section("section").unwrap();
		assert_eq!(section.get_entry::<String>("key1").unwrap(), "value1");
		assert_eq!(section.get_entry::<i64>("key2").unwrap(), 42);
	}

	#[test]
	fn test_parse_empty_section_name_returns_error() {
		// Arrange
		let ini_str = r"[]";

		// Act
		let result = parse_ini_str(ini_str);

		// Assert
		assert!(result.is_err());
		assert_eq!(result.unwrap_err(), "Section name cannot be empty\n    error at <string>:1".to_string());
	}

	#[test]
	fn test_parse_empty_mod_identifier_returns_error() {
		// Act
		let result = parse_ini_file(Path::new("test-files/mod_null_id.ini"));

		// Assert
		assert!(result.is_err());
		assert_eq!(
			result.unwrap_err(),
			"Module identifier cannot be empty\n    error at test-files/mod_null_id.ini:3".to_string()
		);
	}

	#[test]
	fn test_parse_non_ascii_mod_identifier_returns_error() {
		// Act
		let result = parse_ini_file(Path::new("test-files/mod_non_ascii.ini"));

		// Assert
		assert!(result.is_err());
		assert_eq!(
			result.unwrap_err(),
			"Module identifier cannot contain non-ASCII or non-printable characters\n    error at test-files/mod_non_ascii.ini:3".to_string()
		);
	}

	#[test]
	fn test_parse_reopened_section_merges_keys() {
		// Reopening an existing section should append new keys and override
		// duplicates — the foundation of layered mod modules.
		let ini_str = r"
			[section1]
			keep = original
			override = first
			[section1]
			override = second
			new_key = added
		";

		let result = parse_ini_str(ini_str);

		assert!(result.is_ok(), "{:?}", result.err());
		let ini = result.unwrap();
		let section = ini.get_section("section1").unwrap();
		assert_eq!(section.get_entry::<String>("keep").unwrap(), "original");
		assert_eq!(section.get_entry::<String>("override").unwrap(), "second");
		assert_eq!(section.get_entry::<String>("new_key").unwrap(), "added");
	}

	#[test]
	fn test_parse_key_value_outside_section_returns_error() {
		// Arrange
		let ini_str = r"
			key = value
			[section]
		";

		// Act
		let result = parse_ini_str(ini_str);

		// Assert
		assert!(result.is_err());
		assert_eq!(
			result.unwrap_err(),
			"Key-value pair cannot be defined outside of a section\n    error at <string>:2".to_string()
		);
	}

	#[test]
	fn test_parse_empty_key_returns_error() {
		// Arrange
		let ini_str = r"
			[section]
			= value
		";

		// Act
		let result = parse_ini_str(ini_str);

		// Assert
		assert!(result.is_err());
		assert_eq!(result.unwrap_err(), "Key name cannot be empty\n    error at <string>:3".to_string());
	}

	#[test]
	fn test_mod_inclusion() {
		// Arrange
		let root_mod_path = Path::new("test-files/mod_root.ini");

		// Act
		let result = parse_ini_file(root_mod_path);

		// Assert
		assert!(result.is_ok());
		let ini = result.unwrap();

		let mod_a = ini.get_section("mod_a").unwrap();
		let name_a: String = mod_a.get_entry("name").unwrap();
		assert_eq!(name_a, "mod-a");

		let mod_b = ini.get_section("mod_b").unwrap();
		let name_b: String = mod_b.get_entry("name").unwrap();
		assert_eq!(name_b, "mod-b");

		let mod_c = ini.get_section("mod_c").unwrap();
		let name_c: String = mod_c.get_entry("name").unwrap();
		assert_eq!(name_c, "mod-c");
	}

	#[test]
	fn test_mod_inclusion_depth_is_bounded() {
		// A chain of includes where every file is distinct (so the
		// circular-inclusion guard never trips) must still terminate with an
		// error instead of recursing until the stack overflows. Build
		// link_0 → link_1 → … past the limit and parse the head.
		let dir = std::env::temp_dir().join(format!("mme-ini-depth-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let links = MAX_INCLUDE_DEPTH + 5;
		for i in 0..links {
			let body = if i + 1 < links {
				format!("[link{i}]\nn = {i}\n[[mod link_{}]]\n", i + 1)
			} else {
				format!("[link{i}]\nn = {i}\n")
			};
			std::fs::write(dir.join(format!("link_{i}.ini")), body).unwrap();
		}
		let result = parse_ini_file(&dir.join("link_0.ini"));
		let _ = std::fs::remove_dir_all(&dir);
		let err = result.unwrap_err();
		assert!(err.contains("nested deeper than"), "{err}");
	}

	#[test]
	fn test_mod_inclusion_within_depth_succeeds() {
		// A short distinct chain (well under the limit) still parses fully.
		let dir = std::env::temp_dir().join(format!("mme-ini-shallow-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		for i in 0..4 {
			let body = if i < 3 {
				format!("[link{i}]\nn = {i}\n[[mod link_{}]]\n", i + 1)
			} else {
				format!("[link{i}]\nn = {i}\n")
			};
			std::fs::write(dir.join(format!("link_{i}.ini")), body).unwrap();
		}
		let result = parse_ini_file(&dir.join("link_0.ini"));
		let _ = std::fs::remove_dir_all(&dir);
		let ini = result.unwrap();
		assert_eq!(ini.get_section("link3").unwrap().get_entry::<i64>("n"), Some(3));
	}

	#[test]
	fn test_mod_circular_inclusion_returns_error() {
		// Arrange
		let circular_mod_path = Path::new("test-files/mod_circular.ini");

		// Act
		let result = parse_ini_file(circular_mod_path);

		// Assert
		assert!(result.is_err());
		let err_msg = result.unwrap_err();
		assert_eq!(err_msg, "Circular module inclusion\n    error at test-files/mod_circular_a.ini:1");
	}
}
