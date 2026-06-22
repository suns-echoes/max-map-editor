use std::{
	any::{Any, TypeId},
	collections::HashMap,
	fmt,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum INIValueType {
	String,
	Integer,
	Float,
	Boolean,
}

#[derive(Debug)]
pub struct INIValue(INIValueType, Box<dyn Any>);

pub trait IntoINIValue<T> {
	fn into_val(self) -> Option<T>;
}

impl IntoINIValue<String> for Option<&INIValue> {
	fn into_val(self) -> Option<String> {
		self.and_then(|ini_value| {
			if ini_value.0 == INIValueType::String { ini_value.1.downcast_ref::<String>().cloned() } else { None }
		})
	}
}

impl IntoINIValue<i64> for Option<&INIValue> {
	fn into_val(self) -> Option<i64> {
		self.and_then(|ini_value| {
			if ini_value.0 == INIValueType::Integer { ini_value.1.downcast_ref::<i64>().cloned() } else { None }
		})
	}
}

#[derive(Debug)]
pub struct INISection(HashMap<String, INIValue>);

impl Default for INISection {
	fn default() -> Self {
		Self::new()
	}
}

impl INISection {
	pub fn new() -> Self {
		Self(HashMap::new())
	}

	pub fn get_entry<T: Any + Clone>(&self, key: &str) -> Option<T> {
		self.0.get(key).and_then(|ini_value| {
			let target_type_id = TypeId::of::<T>();

			let required_ini_type = match target_type_id {
				_ if target_type_id == TypeId::of::<String>() => INIValueType::String,
				_ if target_type_id == TypeId::of::<i64>() => INIValueType::Integer,
				_ if target_type_id == TypeId::of::<f64>() => INIValueType::Float,
				_ if target_type_id == TypeId::of::<bool>() => INIValueType::Boolean,
				_ => return None,
			};

			if ini_value.0 == required_ini_type { ini_value.1.downcast_ref::<T>().cloned() } else { None }
		})
	}

	pub fn has_entry(&self, key: &str) -> bool {
		self.0.contains_key(key)
	}
	pub fn set_entry<T: Any>(&mut self, key: String, value: T) -> Result<(), String> {
		match TypeId::of::<T>() {
			_ if TypeId::of::<String>() == TypeId::of::<T>() => {
				let ini_value = INIValue(INIValueType::String, Box::new(value));
				self.0.insert(key, ini_value);
			}
			_ if TypeId::of::<i8>() == TypeId::of::<T>() => {
				let i8_value = Box::new(value) as Box<dyn Any>;
				let i8_value_ref = i8_value.downcast_ref::<i8>().unwrap();
				let i32_value = *i8_value_ref as i32;
				self.0.insert(key, INIValue(INIValueType::Integer, Box::new(i32_value)));
			}
			_ if TypeId::of::<u8>() == TypeId::of::<T>() => {
				let u8_value = Box::new(value) as Box<dyn Any>;
				let u8_value_ref = u8_value.downcast_ref::<u8>().unwrap();
				let i32_value = *u8_value_ref as i32;
				self.0.insert(key, INIValue(INIValueType::Integer, Box::new(i32_value)));
			}
			_ if TypeId::of::<i16>() == TypeId::of::<T>() => {
				let i16_value = Box::new(value) as Box<dyn Any>;
				let i16_value_ref = i16_value.downcast_ref::<i16>().unwrap();
				let i32_value = *i16_value_ref as i32;
				self.0.insert(key, INIValue(INIValueType::Integer, Box::new(i32_value)));
			}
			_ if TypeId::of::<u16>() == TypeId::of::<T>() => {
				let u16_value = Box::new(value) as Box<dyn Any>;
				let u16_value_ref = u16_value.downcast_ref::<u16>().unwrap();
				let i32_value = *u16_value_ref as i32;
				self.0.insert(key, INIValue(INIValueType::Integer, Box::new(i32_value)));
			}
			_ if TypeId::of::<i32>() == TypeId::of::<T>() => {
				let i32_value = Box::new(value) as Box<dyn Any>;
				let i32_value_ref = i32_value.downcast_ref::<i32>().unwrap();
				let i64_value = *i32_value_ref as i64;
				self.0.insert(key, INIValue(INIValueType::Integer, Box::new(i64_value)));
			}
			_ if TypeId::of::<u32>() == TypeId::of::<T>() => {
				let u32_value = Box::new(value) as Box<dyn Any>;
				let u32_value_ref = u32_value.downcast_ref::<u32>().unwrap();
				let i64_value = *u32_value_ref as i64;
				self.0.insert(key, INIValue(INIValueType::Integer, Box::new(i64_value)));
			}
			_ if TypeId::of::<i64>() == TypeId::of::<T>() => {
				let ini_value = INIValue(INIValueType::Integer, Box::new(value));
				self.0.insert(key, ini_value);
			}
			_ if TypeId::of::<u64>() == TypeId::of::<T>() => {
				let u64_value = Box::new(value) as Box<dyn Any>;
				let u64_value_ref = u64_value.downcast_ref::<u64>().unwrap();
				let i64_value = *u64_value_ref as i64;
				self.0.insert(key, INIValue(INIValueType::Integer, Box::new(i64_value)));
			}
			_ if TypeId::of::<f64>() == TypeId::of::<T>() => {
				let ini_value = INIValue(INIValueType::Float, Box::new(value));
				self.0.insert(key, ini_value);
			}
			_ if TypeId::of::<bool>() == TypeId::of::<T>() => {
				let ini_value = INIValue(INIValueType::Boolean, Box::new(value));
				self.0.insert(key, ini_value);
			}
			_ => {
				return Err(format!("Unsupported type for key '{}'. Value not set.", key));
			}
		};

		Ok(())
	}

	pub fn delete_entry(&mut self, key: &str) {
		self.0.remove(key);
	}

	/// Overlay `other`'s entries onto this section, overriding any duplicate key
	/// (the same last-wins layering that re-opening a `[section]` provides). Used
	/// to merge a user config over shipped defaults.
	pub fn overlay(&mut self, other: INISection) {
		self.0.extend(other.0);
	}

	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}
}

// --- Serialization (stringifier) -----------------------------------------
// Emits the `key=value` form that parse_ini reads back. Lives here (not a
// separate module) so it can read INIValue's private type tag + boxed value.

impl fmt::Display for INIValue {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match &self.0 {
			INIValueType::String => {
				if let Some(s) = self.1.downcast_ref::<String>() {
					return f.write_str(s);
				}
			}
			INIValueType::Integer => {
				// Boxed as i64 (i32/i64/u32/u64 inputs) or i32 (i8/i16/u8/u16).
				if let Some(v) = self.1.downcast_ref::<i64>() {
					return write!(f, "{v}");
				}
				if let Some(v) = self.1.downcast_ref::<i32>() {
					return write!(f, "{v}");
				}
			}
			INIValueType::Float => {
				if let Some(v) = self.1.downcast_ref::<f64>() {
					let s = v.to_string();
					// parse_ini tries i64 before f64, so a whole-number float
					// ("12") would re-parse as Integer - force a decimal point.
					if s.contains('.') || s.contains('e') || s.contains('E') || s.contains("inf") || s.contains("NaN") {
						return f.write_str(&s);
					}
					return write!(f, "{s}.0");
				}
			}
			INIValueType::Boolean => {
				if let Some(v) = self.1.downcast_ref::<bool>() {
					return f.write_str(if *v { "true" } else { "false" });
				}
			}
		}
		Ok(())
	}
}

impl fmt::Display for INISection {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let mut keys: Vec<&String> = self.0.keys().collect();
		keys.sort();
		for k in keys {
			writeln!(f, "{k}={}", self.0[k])?;
		}
		Ok(())
	}
}

impl<'a> IntoIterator for &'a INISection {
	type Item = (&'a String, &'a INIValue);
	type IntoIter = std::collections::hash_map::Iter<'a, String, INIValue>;

	fn into_iter(self) -> Self::IntoIter {
		self.0.iter()
	}
}

impl IntoIterator for INISection {
	type Item = (String, INIValue);
	type IntoIter = std::collections::hash_map::IntoIter<String, INIValue>;

	fn into_iter(self) -> Self::IntoIter {
		self.0.into_iter()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_set_and_get_string_entry() {
		// Arrange
		let mut section = INISection::new();
		let key = "username".to_string();
		let value = "alice".to_string();

		// Act
		section.set_entry(key.clone(), value.clone()).unwrap();
		let result = section.get_entry::<String>(&key);

		// Assert
		assert_eq!(result, Some(value));
	}

	#[test]
	fn test_cast_assign_string_entry() {
		// Arrange
		let mut section = INISection::new();
		let key = "username".to_string();
		let value = "alice".to_string();
		section.set_entry(key.clone(), value.clone()).unwrap();

		// Act
		let result: String = section.get_entry(&key).unwrap();

		// Assert
		assert_eq!(result, "alice");
	}

	#[test]
	fn test_set_and_get_integer_entry() {
		// Arrange
		let mut section = INISection::new();
		let key = "age".to_string();
		let value = 42;

		// Act
		section.set_entry(key.clone(), value).unwrap();
		let result = section.get_entry::<i64>(&key);

		// Assert
		assert_eq!(result, Some(value));
	}

	#[test]
	fn test_cast_assign_integer_entry() {
		// Arrange
		let mut section = INISection::new();
		let key = "age".to_string();
		let value = 42;
		section.set_entry(key.clone(), value).unwrap();

		// Act
		let result: i64 = section.get_entry(&key).unwrap();

		// Assert
		assert_eq!(result, 42);
	}

	#[test]
	fn test_set_and_get_float_entry() {
		// Arrange
		let mut section = INISection::new();
		let key = "ratio".to_string();
		let value = 2.5f64;

		// Act
		section.set_entry(key.clone(), value).unwrap();
		let result = section.get_entry::<f64>(&key);

		// Assert
		assert_eq!(result, Some(value));
	}

	#[test]
	fn test_cast_assign_float_entry() {
		// Arrange
		let mut section = INISection::new();
		let key = "ratio".to_string();
		let value = 2.5f64;
		section.set_entry(key.clone(), value).unwrap();

		// Act
		let result: f64 = section.get_entry(&key).unwrap();

		// Assert
		assert_eq!(result, 2.5);
	}

	#[test]
	fn test_set_and_get_boolean_entry() {
		// Arrange
		let mut section = INISection::new();
		let key = "enabled".to_string();
		let value = true;

		// Act
		section.set_entry(key.clone(), value).unwrap();
		let result = section.get_entry::<bool>(&key);

		// Assert
		assert_eq!(result, Some(value));
	}

	#[test]
	fn test_cast_assign_boolean_entry() {
		// Arrange
		let mut section = INISection::new();
		let key = "enabled".to_string();
		let value = true;
		section.set_entry(key.clone(), value).unwrap();

		// Act
		let result: bool = section.get_entry(&key).unwrap();

		// Assert
		assert!(result);
	}

	#[test]
	fn test_get_entry_wrong_type_returns_none() {
		// Arrange
		let mut section = INISection::new();
		section.set_entry("number".to_string(), 123).unwrap();

		// Act
		let result: Option<String> = section.get_entry("number");

		// Assert
		assert!(result.is_none());
	}

	#[test]
	fn test_cast_assign_wrong_type_returns_none() {
		// Arrange
		let mut section = INISection::new();
		section.set_entry("number".to_string(), 123).unwrap();

		// Act
		let result: Option<String> = section.get_entry("number");

		// Assert
		assert!(result.is_none());
	}

	#[test]
	fn test_has_entry() {
		// Arrange
		let mut section = INISection::new();
		section.set_entry("exists".to_string(), true).unwrap();

		// Act & Assert
		assert!(section.has_entry("exists"));
		assert!(!section.has_entry("missing"));
	}

	#[test]
	fn test_delete_entry() {
		// Arrange
		let mut section = INISection::new();
		section.set_entry("to_delete".to_string(), 1).unwrap();

		// Act
		section.delete_entry("to_delete");

		// Assert
		assert!(!section.has_entry("to_delete"));
	}

	#[test]
	fn test_is_empty() {
		// Arrange
		let mut section = INISection::new();

		// Act & Assert
		assert!(section.is_empty());
		section.set_entry("foo".to_string(), "bar".to_string()).unwrap();
		assert!(!section.is_empty());
	}

	#[test]
	fn test_set_entry_unsupported_type() {
		// Arrange
		let mut section = INISection::new();

		// Act
		let result = section.set_entry("unsupported".to_string(), vec![1, 2, 3]);

		// Assert
		assert!(result.is_err());
	}

	#[test]
	fn test_cast_assign_unsupported_type() {
		// Arrange
		let mut section = INISection::new();
		section.set_entry("number".to_string(), 123).unwrap();

		// Act
		let result: Option<Vec<i32>> = section.get_entry("number");

		// Assert
		assert!(result.is_none());
	}

	#[test]
	fn test_iterate_section() {
		// Arrange
		let mut section = INISection::new();
		section.set_entry("a".to_string(), 1i64).unwrap();
		section.set_entry("b".to_string(), 2i64).unwrap();

		// Act
		let keys: Vec<_> = (&section).into_iter().map(|(k, _)| k.clone()).collect();

		// Assert
		assert!(keys.contains(&"a".to_string()));
		assert!(keys.contains(&"b".to_string()));
		assert_eq!(keys.len(), 2);
	}
}
