use std::{any::Any, collections::HashMap, fmt, path::Path};

use crate::{
	INISection,
	parse_ini::{parse_ini_file, parse_ini_str},
};

#[derive(Debug)]
pub struct INI(HashMap<String, INISection>);

impl Default for INI {
	fn default() -> Self {
		Self::new()
	}
}

impl INI {
	pub fn new() -> Self {
		Self(HashMap::new())
	}

	pub fn from_file(file_path: &Path) -> Result<Self, String> {
		parse_ini_file(file_path)
	}

	// `from_str` is the crate's established constructor (the app and the
	// pillar-world upstream both call `INI::from_str`), so it stays an
	// inherent method rather than the `FromStr` trait - which would force
	// callers through `.parse()` for no gain.
	#[allow(clippy::should_implement_trait)]
	pub fn from_str(ini_content: &str) -> Result<Self, String> {
		parse_ini_str(ini_content)
	}

	/// Serialize back to INI text and write it to `file_path`. Round-trips
	/// through `from_file` / `from_str`. Sections + keys are sorted for
	/// stable, diff-friendly output.
	pub fn to_file(&self, file_path: &Path) -> Result<(), String> {
		std::fs::write(file_path, self.to_string())
			.map_err(|e| format!("Failed to write file '{}': {}", file_path.display(), e))
	}

	pub fn get_section(&self, section_name: &str) -> Option<&INISection> {
		self.0.get(section_name)
	}

	pub fn get_section_mut(&mut self, section_name: &str) -> Option<&mut INISection> {
		self.0.get_mut(section_name)
	}

	pub fn get_entry<T: Any + Clone>(&self, section: &str, key: &str) -> Option<T> {
		let ini_section = self.0.get(section)?;
		ini_section.get_entry::<T>(key)
	}

	pub fn has_section(&self, section_name: &str) -> bool {
		self.0.contains_key(section_name)
	}

	pub fn insert_section(&mut self, section_name: String, section: INISection) {
		self.0.insert(section_name, section);
	}

	/// Overlay `other` onto self: merge every section's entries, overriding any
	/// duplicate key and adding new sections. The programmatic equivalent of
	/// concatenating the two files with `other` last (later wins) - used to layer
	/// a user override config over the shipped defaults.
	pub fn overlay(&mut self, other: INI) {
		for (name, section) in other.0 {
			match self.0.get_mut(&name) {
				Some(existing) => existing.overlay(section),
				None => {
					self.0.insert(name, section);
				}
			}
		}
	}

	pub fn delete_section(&mut self, section_name: &str) {
		self.0.remove(section_name);
	}

	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}

	/// Iterate all section names in this INI. Order is unspecified (HashMap
	/// iteration order). Lets callers discover sections by pattern (e.g. all
	/// names ending in `Weapon`) when there's no explicit roster section.
	pub fn section_names(&self) -> impl Iterator<Item = &str> {
		self.0.keys().map(|s| s.as_str())
	}
}

// Serialize to INI text: `[section]` headers with `key=value` lines, sections
// and keys sorted for stable output, a blank line between sections. Parses
// back via INI::from_str / from_file.
impl fmt::Display for INI {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let mut names: Vec<&String> = self.0.keys().collect();
		names.sort();
		for (idx, name) in names.iter().enumerate() {
			if idx > 0 {
				writeln!(f)?;
			}
			writeln!(f, "[{name}]")?;
			write!(f, "{}", self.0[*name])?;
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_to_string_round_trip() {
		let mut section = INISection::new();
		section.set_entry::<String>("name".to_string(), "alice".to_string()).unwrap();
		section.set_entry::<i64>("count".to_string(), 42).unwrap();
		section.set_entry::<f64>("ratio".to_string(), 12.0).unwrap();
		section.set_entry::<f64>("pi".to_string(), 3.5).unwrap();
		section.set_entry::<bool>("on".to_string(), true).unwrap();
		let mut ini = INI::new();
		ini.insert_section("S".to_string(), section);

		let text = ini.to_string();
		let re = INI::from_str(&text).expect("round-trip parse");
		assert_eq!(re.get_entry::<String>("S", "name"), Some("alice".to_string()));
		assert_eq!(re.get_entry::<i64>("S", "count"), Some(42));
		// whole-number float must stay Float (emitted as "12.0", not "12")
		assert_eq!(re.get_entry::<f64>("S", "ratio"), Some(12.0));
		assert_eq!(re.get_entry::<f64>("S", "pi"), Some(3.5));
		assert_eq!(re.get_entry::<bool>("S", "on"), Some(true));
	}

	fn create_sample_ini() -> INI {
		let mut section = INISection::new();
		section.set_entry::<String>("key1".to_string(), "value1".to_string()).ok();
		section.set_entry::<i64>("key2".to_string(), 42).ok();
		let mut ini = INI::new();
		ini.insert_section("Section1".to_string(), section);
		ini
	}

	#[test]
	fn overlay_overrides_keys_and_adds_sections() {
		// Base (shipped) defaults.
		let mut base = INI::from_str("[A]\nx=1\ny=2\n[B]\nk=base\n").unwrap();
		// Override (user): change one key, add a key, add a whole section.
		let over = INI::from_str("[A]\ny=20\nz=3\n[C]\nnew=yes\n").unwrap();
		base.overlay(over);
		assert_eq!(base.get_entry::<i64>("A", "x"), Some(1), "untouched key survives");
		assert_eq!(base.get_entry::<i64>("A", "y"), Some(20), "duplicate key is overridden");
		assert_eq!(base.get_entry::<i64>("A", "z"), Some(3), "new key in an existing section is added");
		assert_eq!(base.get_entry::<String>("B", "k"), Some("base".to_string()), "untouched section survives");
		assert_eq!(base.get_entry::<bool>("C", "new"), Some(true), "a new section is added wholesale");
	}

	#[test]
	fn test_new_ini_is_empty() {
		// Arrange & Act
		let ini = INI::new();

		// Assert
		assert!(ini.is_empty());
	}

	#[test]
	fn test_insert_and_get_section() {
		// Arrange
		let mut ini = INI::new();
		let section = INISection::new();

		// Act
		ini.insert_section("TestSection".to_string(), section);

		// Assert
		assert!(ini.get_section("TestSection").is_some());
	}

	#[test]
	fn test_delete_section() {
		// Arrange
		let mut ini = create_sample_ini();

		// Act
		ini.delete_section("Section1");

		// Assert
		assert!(!ini.has_section("Section1"));
	}

	#[test]
	fn test_get_entry_existing() {
		// Arrange
		let ini = create_sample_ini();

		// Act
		let value = ini.get_entry::<String>("Section1", "key1");

		// Assert
		assert_eq!(value, Some("value1".to_string()));
	}

	#[test]
	fn test_get_entry_wrong_type() {
		// Arrange
		let ini = create_sample_ini();

		// Act
		let value = ini.get_entry::<String>("Section1", "key2");

		// Assert
		assert!(value.is_none());
	}

	#[test]
	fn test_get_entry_unsupported_type() {
		// Arrange
		let ini = create_sample_ini();

		// Act
		let value = ini.get_entry::<i8>("Section1", "key2");

		// Assert
		assert!(value.is_none());
	}

	#[test]
	fn test_get_entry_nonexistent_key() {
		// Arrange
		let ini = create_sample_ini();

		// Act
		let value: Option<String> = ini.get_entry("Section1", "nonexistent");

		// Assert
		assert!(value.is_none());
	}

	#[test]
	fn test_has_section() {
		// Arrange
		let ini = create_sample_ini();

		// Act & Assert
		assert!(ini.has_section("Section1"));
		assert!(!ini.has_section("NoSection"));
	}

	#[test]
	fn test_get_section_mut() {
		// Arrange
		let mut ini = create_sample_ini();

		// Act
		if let Some(section) = ini.get_section_mut("Section1") {
			section.set_entry::<i64>("new_key".to_string(), 123).ok();
		}

		// Assert
		let value: Option<i64> = ini.get_entry("Section1", "new_key");
		assert_eq!(value, Some(123));
	}

	#[test]
	fn test_from_str() {
		// Arrange
		let ini_str = "[SectionA]\nfoo=bar\n";

		// Act
		let ini = INI::from_str(ini_str);

		// Assert
		assert!(ini.is_ok());
		let ini = ini.unwrap();
		assert!(ini.has_section("SectionA"));
	}

	#[test]
	fn test_from_file() {
		// Arrange - a unique path under the system temp dir (no stray dirs in
		// the working tree, no collision with a parallel test run).
		let ini_content = "[SectionB]\nkey=value\n";
		let ini_file_path = std::env::temp_dir().join(format!("mme-ini-from-file-{}.ini", std::process::id()));
		std::fs::write(&ini_file_path, ini_content).unwrap();

		// Act
		let ini = INI::from_file(&ini_file_path);

		// Assert
		assert!(ini.is_ok());
		let ini = ini.unwrap();
		assert!(ini.has_section("SectionB"));

		// Cleanup
		let _ = std::fs::remove_file(&ini_file_path);
	}

	#[test]
	fn test_from_file_not_found() {
		// Arrange
		let path = Path::new("nonexistent_file.ini");

		// Act
		let result = INI::from_file(path);

		// Assert
		assert!(result.is_err());
	}
}
