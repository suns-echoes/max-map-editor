use std::{
    collections::BTreeMap,
    fmt::{self, Display},
    fs,
    path::Path,
    str,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

impl Value {
    fn to_ini(&self) -> String {
        match self {
            Value::Bool(v) => v.to_string(),
            Value::Int(v) => v.to_string(),
            Value::Float(v) => {
                if v.fract() == 0.0 {
                    format!("{:.1}", v)
                } else {
                    v.to_string()
                }
            }
            Value::Str(v) => stringify_string(v),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Section {
    entries: BTreeMap<String, Value>,
}

impl Section {
    fn insert(&mut self, key: String, value: Value) {
        self.entries.insert(key, value);
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.entries.iter()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct XINI {
    sections: BTreeMap<String, Section>,
}

impl XINI {
	pub fn new() -> Self {
		Self::default()
	}

    pub fn parse(input: &str) -> Result<Self, IniError> {
        let mut ini = XINI::default();
        let mut current_section = String::new();

        for (idx, raw_line) in input.lines().enumerate() {
            let line_number = idx + 1;
            let line = sanitize_line(raw_line);

            if line.trim().is_empty() {
                continue;
            }

            if line.trim_start().starts_with('[') {
                let section = parse_section_header(&line, line_number)?;
                current_section = section.clone();
                ini.sections.entry(section).or_default();
                continue;
            }

            let (key, value) = parse_entry(&line, line_number)?;
            ini.sections
                .entry(current_section.clone())
                .or_default()
                .insert(key, value);
        }

        Ok(ini)
    }

    pub fn parse_bytes(bytes: &[u8]) -> Result<Self, IniError> {
        let text = str::from_utf8(bytes).map_err(|_| IniError::InvalidUtf8)?;
        Self::parse(text)
    }

    pub fn load_from_file(path: &Path) -> Result<Self, IniError> {
        let data = fs::read(path)?;
        Self::parse_bytes(&data)
    }

    pub fn save_to_file(&self, path: &Path) -> Result<(), std::io::Error> {
        fs::write(path, self.to_string())
    }

    pub fn to_string(&self) -> String {
        let mut output = String::new();
		let mut first_section = true;

        if let Some(section) = self.sections.get("") {
            write_section_entries("", section, &mut output);
        }

        for (name, section) in self.sections.iter().filter(|(k, _)| !k.is_empty()) {
			if !first_section {
				output.push('\n');
			} else {
				first_section = false;
			}
            output.push_str(&format!("[{}]\n", name));
            write_section_entries(name, section, &mut output);
        }

        output
    }

    pub fn get_section(&self, name: &str) -> Option<&Section> {
        self.sections.get(name)
    }

    pub fn get(&self, section: &str, key: &str) -> ValueAccessor<'_> {
        ValueAccessor::new(self.get_section(section).and_then(|sec| sec.get(key)))
    }

    pub fn get_value(&self, section: &str, key: &str) -> Option<&Value> {
        self.get_section(section).and_then(|sec| sec.get(key))
    }

	pub fn get_bool(&self, section: &str, key: &str) -> Option<bool> {
		self.get(section, key).into_option()
	}

    pub fn get_bool_or(&self, section: &str, key: &str, default: bool) -> bool {
        self.get(section, key).into_or(default)
    }

	pub fn get_int(&self, section: &str, key: &str) -> Option<i64> {
		self.get(section, key).into_option()
	}

    pub fn get_int_or(&self, section: &str, key: &str, default: i64) -> i64 {
        self.get(section, key).into_or(default)
    }

	pub fn get_float(&self, section: &str, key: &str) -> Option<f64> {
		self.get(section, key).into_option()
	}

    pub fn get_float_or(&self, section: &str, key: &str, default: f64) -> f64 {
        self.get(section, key).into_or(default)
    }

	pub fn get_string(&self, section: &str, key: &str) -> Option<String> {
		self.get(section, key).into_option()
	}

    pub fn get_string_or(&self, section: &str, key: &str, default: &str) -> String {
        self.get(section, key).into_or(default.to_string())
    }

    pub fn set_value(&mut self, section: &str, key: impl Into<String>, value: Value) {
        self.sections
            .entry(section.to_string())
            .or_default()
            .insert(key.into(), value);
    }

	pub fn set_bool(&mut self, section: &str, key: impl Into<String>, value: bool) {
		self.set_value(section, key, Value::Bool(value));
	}

	pub fn set_int(&mut self, section: &str, key: impl Into<String>, value: i64) {
		self.set_value(section, key, Value::Int(value));
	}

	pub fn set_float(&mut self, section: &str, key: impl Into<String>, value: f64) {
		self.set_value(section, key, Value::Float(value));
	}

	pub fn set_string(&mut self, section: &str, key: impl Into<String>, value: impl Into<String>) {
		self.set_value(section, key, Value::Str(value.into()));
	}
}

impl Display for XINI {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string())
    }
}

// -----------------------------------------------------------------------------

fn sanitize_line(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut in_quotes = false;

    for ch in line.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                result.push(ch);
            }
            ';' if !in_quotes => break,
            _ => result.push(ch),
        }
    }

    result.trim_end().to_string()
}

fn parse_section_header(line: &str, line_number: usize) -> Result<String, IniError> {
    let line = line.trim();
    if !line.ends_with(']') {
        return Err(IniError::MalformedSection {
            line: line_number,
            line_content: line.to_string(),
        });
    }
    let trimmed = &line[1..line.len() - 1];
    if trimmed.is_empty() {
        return Err(IniError::EmptySection { line: line_number });
    }
    Ok(trimmed.to_string())
}

fn parse_entry(line: &str, line_number: usize) -> Result<(String, Value), IniError> {
    let Some((key, value)) = line.split_once('=') else {
        return Err(IniError::MalformedEntry {
            line: line_number,
            line_content: line.to_string(),
        });
    };

    let key = key.trim();
    if key.is_empty() {
        return Err(IniError::EmptyKey { line: line_number });
    }

    let value = value.trim();
    let parsed_value = parse_value(value)?;

    Ok((key.to_string(), parsed_value))
}

fn parse_value(raw: &str) -> Result<Value, IniError> {
    if let Some(stripped) = strip_quotes(raw) {
        return Ok(Value::Str(stripped));
    }

    let lower = raw.to_ascii_lowercase();
    if lower == "true" {
        return Ok(Value::Bool(true));
    }
    if lower == "false" {
        return Ok(Value::Bool(false));
    }

    if let Ok(int) = raw.parse::<i64>() {
        return Ok(Value::Int(int));
    }

    if let Ok(float) = raw.parse::<f64>() {
        return Ok(Value::Float(float));
    }

    Ok(Value::Str(raw.to_string()))
}

fn strip_quotes(value: &str) -> Option<String> {
    let trimmed = value.trim();

    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        let inner = &trimmed[1..trimmed.len() - 1];
        return Some(inner.to_string());
    }

    None
}

fn stringify_string(value: &str) -> String {
    if value.is_empty() || needs_quotes(value) {
        let escaped = value.replace('"', "\\\"");
        format!("\"{}\"", escaped)
    } else {
        value.to_string()
    }
}

fn needs_quotes(value: &str) -> bool {
    value.bytes().any(|b| matches!(b, b' ' | b'\t' | b';' | b'=' | b'[' | b']'))
}

fn write_section_entries(_name: &str, section: &Section, output: &mut String) {
    for (key, value) in section.iter() {
        output.push_str(key);
        output.push('=');
        output.push_str(&value.to_ini());
        output.push('\n');
    }
}

// -----------------------------------------------------------------------------

#[derive(Debug)]
pub enum IniError {
    MalformedSection { line: usize, line_content: String },
    MalformedEntry { line: usize, line_content: String },
    EmptyKey { line: usize },
    EmptySection { line: usize },
    InvalidUtf8,
    Io(std::io::Error),
}

impl fmt::Display for IniError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IniError::MalformedSection { line, line_content } => {
                write!(f, "Malformed section header on line {}: {}", line, line_content)
            }
            IniError::MalformedEntry { line, line_content } => {
                write!(f, "Malformed entry on line {}: {}", line, line_content)
            }
            IniError::EmptyKey { line } => write!(f, "Empty key on line {}", line),
            IniError::EmptySection { line } => write!(f, "Empty section on line {}", line),
            IniError::InvalidUtf8 => write!(f, "Invalid UTF-8 sequence"),
            IniError::Io(err) => write!(f, "I/O error: {}", err),
        }
    }
}

impl std::error::Error for IniError {}

impl From<std::io::Error> for IniError {
    fn from(err: std::io::Error) -> Self {
        IniError::Io(err)
    }
}

// -----------------------------------------------------------------------------

pub trait FromValue<'a>: Sized {
    fn from_value(value: Option<&'a Value>) -> Option<Self>;
}

impl<'a> FromValue<'a> for bool {
	fn from_value(value: Option<&'a Value>) -> Option<Self> {
		match value {
			Some(Value::Bool(v)) => Some(*v),
			_ => None,
		}
	}
}

impl<'a> FromValue<'a> for i64 {
	fn from_value(value: Option<&'a Value>) -> Option<Self> {
		match value {
			Some(Value::Int(v)) => Some(*v),
			_ => None,
		}
	}
}

impl<'a> FromValue<'a> for f64 {
	fn from_value(value: Option<&'a Value>) -> Option<Self> {
		match value {
			Some(Value::Float(v)) => Some(*v),
			_ => None,
		}
	}
}

impl<'a> FromValue<'a> for String {
	fn from_value(value: Option<&'a Value>) -> Option<Self> {
		match value {
			Some(Value::Str(v)) => Some(v.clone()),
			_ => None,
		}
	}
}

pub struct ValueAccessor<'a> {
	value: Option<&'a Value>,
}

impl<'a> ValueAccessor<'a> {
	fn new(value: Option<&'a Value>) -> Self {
		Self { value }
	}

	pub fn into<T>(self) -> T
	where
		T: FromValue<'a>,
	{
		self.into_option().expect("XINI::get(): missing or incompatible value")
	}

	pub fn into_or<T>(self, default: T) -> T
	where
		T: FromValue<'a>,
	{
		self.into_option().unwrap_or(default)
	}

	pub fn into_option<T>(self) -> Option<T>
	where
		T: FromValue<'a>,
	{
		T::from_value(self.value)
	}
}

// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialization() {
        // Arrange
        let mut ini = XINI::default();

        // Act
        ini.set_value("", "enabled", Value::Bool(true));
        ini.set_value("display", "width", Value::Int(1920));
        let text = ini.to_string();

        // Assert
        assert!(text.contains("enabled=true"));
        assert!(text.contains("[display]"));
        assert!(text.contains("width=1920"));
    }

    #[test]
    fn test_deserialization() {
        // Arrange
        let input = "[video]\nfps=60";

        // Act
        let ini = XINI::parse(input).unwrap();

        // Assert
        assert_eq!(ini.get_int("video", "fps"), Some(60));
    }

    #[test]
    fn test_typed_getters() {
        // Arrange
        let ini = XINI::parse("flag=true\ncount=7\nratio=3.5\nname=\"alpha\"").unwrap();

        // Assert
        assert_eq!(ini.get_bool("", "flag"), Some(true));
        assert_eq!(ini.get_int("", "count"), Some(7));
        assert_eq!(ini.get_float("", "ratio"), Some(3.5));
        assert_eq!(ini.get_string("", "name"), Some("alpha".to_string()));
    }

    #[test]
    fn test_defaults() {
        // Arrange
        let ini = XINI::default();

        // Assert
        assert_eq!(ini.get_bool_or("missing", "flag", true), true);
        assert_eq!(ini.get_int_or("", "count", 5), 5);
        assert_eq!(ini.get_float_or("", "ratio", 1.25), 1.25);
        assert_eq!(ini.get_string_or("", "name", "fallback"), "fallback");
    }

    #[test]
    fn test_wrong_type_defaults() {
        // Arrange
        let ini = XINI::parse("flag=\"true\"\ncount=\"7\"\nratio=\"3.5\"\nname=0").unwrap();

        // Assert
        assert_eq!(ini.get_bool_or("", "flag", false), false);
        assert_eq!(ini.get_int_or("", "count", 5), 5);
        assert_eq!(ini.get_float_or("", "ratio", 1.25), 1.25);
        assert_eq!(ini.get_string_or("", "name", "fallback"), "fallback");
    }

    #[test]
    fn test_quoted_strings() {
        // Arrange
        let mut ini = XINI::default();
        ini.set_value("", "path", Value::Str("Program Files".into()));

        // Act
        let serialized = ini.to_string();
        let reparsed = XINI::parse(&serialized).unwrap();

        // Assert
        assert!(serialized.contains("\"Program Files\""));
        assert_eq!(reparsed.get_string_or("", "path", ""), "Program Files");
    }

    #[test]
    fn test_malformed_entry_rejection() {
        // Act
        let err = XINI::parse("invalid line");

        // Assert
        assert!(matches!(err, Err(IniError::MalformedEntry { .. })));
    }

    #[test]
    fn test_empty_section_rejection() {
        // Act
        let err = XINI::parse("[]");

        // Assert
        assert!(matches!(err, Err(IniError::EmptySection { .. })));
    }

    #[test]
    fn test_empty_key_rejection() {
        // Act
        let err = XINI::parse("=value");

        // Assert
        assert!(matches!(err, Err(IniError::EmptyKey { .. })));
    }

    #[test]
    fn test_invalid_utf8_rejection() {
        // Act
        let err = XINI::parse_bytes(b"\xFF\xFF");

        // Assert
        assert!(matches!(err, Err(IniError::InvalidUtf8)));
    }

    #[test]
    fn test_into_accessors() {
        // Arrange
        let ini = XINI::parse("[section]\nenabled=true\nwidth=640\nfps=59.99\nname=\"demo\"").unwrap();

        // Act
		let enabled: bool = ini.get("section", "enabled").into();
		let width: i64 = ini.get("section", "width").into();
		let fps: f64 = ini.get("section", "fps").into();
		let title: String = ini.get("section", "name").into();

		// Assert
		assert_eq!(enabled, true);
		assert_eq!(width, 640);
		assert_eq!(fps, 59.99);
		assert_eq!(title, "demo");
    }

    #[test]
    fn test_into_or_accessors() {
        // Arrange
        let ini = XINI::parse("[section]").unwrap();

        // Act
		let enabled: bool = ini.get("section", "enabled").into_or(true);
		let width: i64 = ini.get("section", "width").into_or(640);
		let fps: f64 = ini.get("section", "fps").into_or(59.99);
		let title: String = ini.get("section", "name").into_or("demo".to_string());

		// Assert
		assert_eq!(enabled, true);
		assert_eq!(width, 640);
		assert_eq!(fps, 59.99);
		assert_eq!(title, "demo");
    }

	#[test]
	fn test_into_or_accessor_on_wrong_type() {
		// Arrange
		let ini = XINI::parse("[section]\nenabled=\"yes\"\nwidth=\"wide\",fps=\"fast\",name=0").unwrap();

        // Act
		let enabled: bool = ini.get("section", "enabled").into_or(true);
		let width: i64 = ini.get("section", "width").into_or(640);
		let fps: f64 = ini.get("section", "fps").into_or(59.99);
		let title: String = ini.get("section", "name").into_or("demo".to_string());

		// Assert
		assert_eq!(enabled, true);
		assert_eq!(width, 640);
		assert_eq!(fps, 59.99);
		assert_eq!(title, "demo");
	}
}
