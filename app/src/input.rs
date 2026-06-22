//! Config-driven input bindings.
//!
//! `mme.ini` `[Bindings]` maps **command lines to key chords** -
//! `ACTION = CHORD [CHORD ...]`, e.g. `redo=Ctrl+Shift+Z Ctrl+Y`. Anything
//! the command parser accepts is bindable, arguments included - validated at
//! load so a typo warns at startup, not on keypress. An entry overrides the
//! default chords for that action; an empty value unbinds it. The legacy
//! inverted form (`Ctrl+S=save`) still applies, with a warning. `[Mouse]`
//! covers the pointer behaviors (pan buttons, paint button, wheel zoom
//! step). (While the console is open, its editing keys are fixed:
//! Esc/`/F1 close.)
//!
//! One chord may carry several actions - the shell dispatches the first one
//! whose *context* applies (e.g. `1` picks pass value 1 in the Pass Table
//! Editor and zooms to 100% in the map editor).

use ini::INI;
use winit::event::MouseButton;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::command::{self, Command};

/// A key chord: modifiers + one key.
#[derive(Debug, Clone, PartialEq)]
pub struct Chord {
	ctrl: bool,
	shift: bool,
	alt: bool,
	key: BindKey,
}

#[derive(Debug, Clone, PartialEq)]
enum BindKey {
	/// A printable key, stored lowercase (`"z"`, `"`"`, `"]"`).
	Char(String),
	Named(NamedKey),
}

impl Chord {
	/// Display label for menu hints: `Ctrl+Shift+Z`, `Del`, `F1`.
	pub fn label(&self) -> String {
		let mut s = String::new();
		if self.ctrl {
			s.push_str("Ctrl+");
		}
		if self.shift {
			s.push_str("Shift+");
		}
		if self.alt {
			s.push_str("Alt+");
		}
		match &self.key {
			BindKey::Char(c) => s.push_str(&c.to_uppercase()),
			BindKey::Named(named) => s.push_str(named_label(*named)),
		}
		s
	}
}

fn named_label(named: NamedKey) -> &'static str {
	match named {
		NamedKey::Escape => "Esc",
		NamedKey::Enter => "Enter",
		NamedKey::Space => "Space",
		NamedKey::Tab => "Tab",
		NamedKey::Backspace => "Backspace",
		NamedKey::Delete => "Del",
		NamedKey::Insert => "Ins",
		NamedKey::Home => "Home",
		NamedKey::End => "End",
		NamedKey::PageUp => "PgUp",
		NamedKey::PageDown => "PgDn",
		NamedKey::ArrowLeft => "Left",
		NamedKey::ArrowRight => "Right",
		NamedKey::ArrowUp => "Up",
		NamedKey::ArrowDown => "Down",
		NamedKey::F1 => "F1",
		NamedKey::F2 => "F2",
		NamedKey::F3 => "F3",
		NamedKey::F4 => "F4",
		NamedKey::F5 => "F5",
		NamedKey::F6 => "F6",
		NamedKey::F7 => "F7",
		NamedKey::F8 => "F8",
		NamedKey::F9 => "F9",
		NamedKey::F10 => "F10",
		NamedKey::F11 => "F11",
		NamedKey::F12 => "F12",
		_ => "?",
	}
}

/// Parse `"Ctrl+Shift+Z"` / `"F1"` / `"Backquote"` (case-insensitive).
pub fn parse_chord(s: &str) -> Result<Chord, String> {
	let (mut ctrl, mut shift, mut alt) = (false, false, false);
	let mut key = None;
	for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
		match part.to_ascii_lowercase().as_str() {
			"ctrl" | "control" => ctrl = true,
			"shift" => shift = true,
			"alt" => alt = true,
			lower => {
				if key.is_some() {
					return Err(format!("chord '{s}': more than one key"));
				}
				key = Some(parse_key(lower).ok_or_else(|| format!("chord '{s}': unknown key '{part}'"))?);
			}
		}
	}
	let key = key.ok_or_else(|| format!("chord '{s}': no key"))?;
	Ok(Chord { ctrl, shift, alt, key })
}

fn parse_key(lower: &str) -> Option<BindKey> {
	// Single printable character (letters, digits, punctuation).
	let mut chars = lower.chars();
	if let (Some(c), None) = (chars.next(), chars.next()) {
		if !c.is_control() {
			return Some(BindKey::Char(c.to_string()));
		}
	}
	let named = match lower {
		"backquote" | "grave" => return Some(BindKey::Char("`".into())),
		// `+` can't be written bare (it's the chord separator) and `=`/`-`
		// read awkwardly next to the INI `=` - names for all three.
		"plus" => return Some(BindKey::Char("+".into())),
		"minus" => return Some(BindKey::Char("-".into())),
		"equals" | "equal" => return Some(BindKey::Char("=".into())),
		"escape" | "esc" => NamedKey::Escape,
		"enter" | "return" => NamedKey::Enter,
		"space" => NamedKey::Space,
		"tab" => NamedKey::Tab,
		"backspace" => NamedKey::Backspace,
		"delete" | "del" => NamedKey::Delete,
		"insert" | "ins" => NamedKey::Insert,
		"home" => NamedKey::Home,
		"end" => NamedKey::End,
		"pageup" => NamedKey::PageUp,
		"pagedown" => NamedKey::PageDown,
		"arrowleft" | "left" => NamedKey::ArrowLeft,
		"arrowright" | "right" => NamedKey::ArrowRight,
		"arrowup" | "up" => NamedKey::ArrowUp,
		"arrowdown" | "down" => NamedKey::ArrowDown,
		"f1" => NamedKey::F1,
		"f2" => NamedKey::F2,
		"f3" => NamedKey::F3,
		"f4" => NamedKey::F4,
		"f5" => NamedKey::F5,
		"f6" => NamedKey::F6,
		"f7" => NamedKey::F7,
		"f8" => NamedKey::F8,
		"f9" => NamedKey::F9,
		"f10" => NamedKey::F10,
		"f11" => NamedKey::F11,
		"f12" => NamedKey::F12,
		_ => return None,
	};
	Some(BindKey::Named(named))
}

/// One bound action: the chord, the canonical command line it came from
/// (whitespace-normalized - the menu-hint lookup key), and the parsed command.
struct Binding {
	chord: Chord,
	line: String,
	command: Command,
}

pub struct Bindings {
	/// Table order is dispatch order among same-chord entries; user entries
	/// precede the surviving defaults so they win chord conflicts.
	keys: Vec<Binding>,
	pan_buttons: Vec<MouseButton>,
	paint_button: MouseButton,
	zoom_step: f32,
}

/// Every bindable action: its **`[Bindings]` INI key** (PascalCase), the
/// **command line** it runs, and its **default chord(s)**. The single source of
/// truth for both the compiled-in defaults and the name↔command mapping that
/// lets the config use clean CamelCase keys instead of raw command lines.
///
/// The key is usually the mechanical PascalCase of the command; the zoom actions
/// use readable aliases (`ZoomIn`/`ZoomOut`, `ZoomTo<percent>`). One chord may
/// serve several actions when their contexts are disjoint - the digit keys pick
/// pass values in the Pass Table Editor and zoom presets in the map editor; the
/// shell resolves by context, not table order.
const ACTIONS: &[(&str, &str, &str)] = &[
	// File
	("SaveProject", "save-project", "Ctrl+S"),
	("FileDialogSaveAs", "file-dialog save-as", "Ctrl+Shift+S"),
	("FileDialogLoad", "file-dialog load", "Ctrl+O"),
	("NewMap", "new-map", "Ctrl+N"),
	("CloseProject", "close-project", "Ctrl+W"),
	("Export", "export", "Ctrl+E"),
	// Edit
	("Undo", "undo", "Ctrl+Z"),
	("Redo", "redo", "Ctrl+Shift+Z Ctrl+Y"),
	("Cut", "cut", "Ctrl+X"),
	("Copy", "copy", "Ctrl+C"),
	("Paste", "paste", "Ctrl+V"),
	("Delete", "delete", "Delete"),
	("DeleteAll", "delete-all", "Shift+Delete"),
	// Select
	("SelectAll", "select all", "Ctrl+A"),
	("SelectClear", "select clear", "Ctrl+D"),
	("SelectInvert", "select invert", "Ctrl+I"),
	// Pass Table Editor: digits pick the active pass value.
	("PassPick0", "pass-pick 0", "0"),
	("PassPick1", "pass-pick 1", "1"),
	("PassPick2", "pass-pick 2", "2"),
	("PassPick3", "pass-pick 3", "3"),
	// View (map editor: digits double as zoom presets).
	("Fit", "fit", "F"),
	("ZoomTo100", "zoom-to 1", "1"),
	("ZoomTo50", "zoom-to 0.5", "2"),
	("ZoomTo25", "zoom-to 0.25", "3"),
	("ZoomIn", "zoom 1.25", "Plus Shift+Plus Equals"),
	("ZoomOut", "zoom 0.8", "Minus"),
	("GridToggle", "grid toggle", "G"),
	("PassOverlayToggle", "pass-overlay toggle", "O"),
	("UnitsToggle", "units toggle", "U"),
	// Tools (map editor only)
	("ToolPencil", "tool pencil", "B"),
	("ToolEraser", "tool eraser", "E"),
	("ToolPicker", "tool picker", "I"),
	("ToolFill", "tool fill", "K"),
	("ToolSelect", "tool select", "L"),
	("ToolSelectRect", "tool select-rect", "M"),
	// Templates (only when one is selected in the explorer).
	("TemplateRename", "template-rename", "F2"),
	// Misc
	("AnimateToggle", "animate toggle", "A"),
	("ConsoleToggle", "console toggle", "Backquote F1"),
	("Quit", "quit", "Escape"),
];

/// Resolve a `[Bindings]` INI key to the command line it runs: a PascalCase
/// action name (the documented form) maps via [`ACTIONS`]; anything else is
/// taken verbatim as a command line, so raw-command keys and old configs still
/// bind.
fn command_for_key(key: &str) -> &str {
	ACTIONS.iter().find(|(name, ..)| *name == key).map_or(key, |(_, command, _)| command)
}

/// Whitespace-normalize a command line so `select  all` and `select all`
/// name the same action (the replace/hint key).
fn normalize(line: &str) -> String {
	line.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Bindings {
	/// Compiled-in defaults, overridden by the `[Bindings]` / `[Mouse]`
	/// sections of the settings INI (the shipped + user `mme.ini`, when present).
	pub fn load(ini: Option<&INI>) -> Self {
		let mut bindings = Self {
			keys: ACTIONS
				.iter()
				.flat_map(|(_, line, chords)| {
					let command = command::parse_line(line).expect("default command").expect("non-empty");
					chords.split_whitespace().map(move |chord| Binding {
						chord: parse_chord(chord).expect("default chord"),
						line: normalize(line),
						command: command.clone(),
					})
				})
				.collect(),
			// LMB paints; RMB pans when held, opens the context menu when
			// clicked in place.
			pan_buttons: vec![MouseButton::Middle, MouseButton::Right],
			paint_button: MouseButton::Left,
			zoom_step: 1.15,
		};
		let Some(ini) = ini else { return bindings };
		bindings.apply_keyboard(ini);
		bindings.apply_mouse(ini);
		bindings
	}

	fn apply_keyboard(&mut self, ini: &INI) {
		let Some(section) = ini.get_section("Bindings") else {
			return; // no [Bindings] - defaults apply
		};
		// User entries collect in file order, then go *in front of* the
		// surviving defaults: dispatch takes the first same-chord match, so
		// rebinding a default's chord must shadow it.
		let mut user: Vec<Binding> = Vec::new();
		let mut applied = 0;
		for (action, value) in section {
			let value = value.to_string();
			// The INI key is a PascalCase action name (or a raw command line);
			// resolve it to the command before parsing the entry.
			let command_line = command_for_key(action);
			match parse_entry(command_line, &value) {
				Ok(entry) => {
					let line = entry.first().map(|b| b.line.clone()).unwrap_or_else(|| normalize(command_line));
					// The entry replaces every existing chord for its action
					// (an empty entry just unbinds it).
					self.keys.retain(|b| b.line != line);
					user.retain(|b| b.line != line);
					user.extend(entry);
					applied += 1;
				}
				Err(e) => eprintln!("config: [Bindings] '{action}': {e} - skipped"),
			}
		}
		user.append(&mut self.keys);
		self.keys = user;
		eprintln!("config: [Bindings] applied {applied} entries ({} bindings total)", self.keys.len());
	}

	fn apply_mouse(&mut self, ini: &INI) {
		let Some(section) = ini.get_section("Mouse") else {
			return; // no [Mouse] - defaults apply
		};
		if let Some(buttons) = section.get_entry::<String>("PanButtons") {
			let parsed: Vec<MouseButton> = buttons
				.split_whitespace()
				.filter_map(|name| match name.to_ascii_lowercase().as_str() {
					"left" => Some(MouseButton::Left),
					"middle" => Some(MouseButton::Middle),
					"right" => Some(MouseButton::Right),
					other => {
						eprintln!("config: [Mouse]: unknown button '{other}' - skipped");
						None
					}
				})
				.collect();
			if !parsed.is_empty() {
				self.pan_buttons = parsed;
			}
		}
		if let Some(button) = section.get_entry::<String>("PaintButton") {
			match button.to_ascii_lowercase().as_str() {
				"left" => self.paint_button = MouseButton::Left,
				"middle" => self.paint_button = MouseButton::Middle,
				"right" => self.paint_button = MouseButton::Right,
				other => eprintln!("config: [Mouse]: unknown paint button '{other}' - ignored"),
			}
		}
		if let Some(step) = section.get_entry::<f64>("ZoomStep") {
			if (1.01..=2.0).contains(&step) {
				self.zoom_step = step as f32;
			} else {
				eprintln!("config: [Mouse]: ZoomStep {step} out of range (1.01..=2.0) - ignored");
			}
		}
	}

	/// Every command bound to a pressed key under the given modifiers, in
	/// table order - the shell picks the first whose context applies.
	pub fn lookup_all(&self, mods: ModifiersState, key: &Key) -> Vec<Command> {
		let (ctrl, shift, alt) = (mods.control_key(), mods.shift_key(), mods.alt_key());
		self.keys
			.iter()
			.filter(|b| {
				b.chord.ctrl == ctrl
					&& b.chord.shift == shift
					&& b.chord.alt == alt
					&& match (&b.chord.key, key) {
						(BindKey::Char(bound), Key::Character(pressed)) => bound.as_str() == pressed.to_lowercase(),
						(BindKey::Named(bound), Key::Named(pressed)) => bound == pressed,
						_ => false,
					}
			})
			.map(|b| b.command.clone())
			.collect()
	}

	/// First command bound to a pressed key, context-blind (the tests' lens;
	/// the shell dispatches via `lookup_all` + its context filter).
	#[cfg(test)]
	pub fn lookup(&self, mods: ModifiersState, key: &Key) -> Option<Command> {
		self.lookup_all(mods, key).into_iter().next()
	}

	/// The menu-hint table: each bound action's first chord label,
	/// keyed by the normalized command line.
	pub fn hint_table(&self) -> Vec<(String, String)> {
		let mut hints: Vec<(String, String)> = Vec::new();
		for b in &self.keys {
			if !hints.iter().any(|(line, _)| *line == b.line) {
				hints.push((b.line.clone(), b.chord.label()));
			}
		}
		hints
	}

	pub fn is_pan_button(&self, button: MouseButton) -> bool {
		self.pan_buttons.contains(&button)
	}

	pub fn is_paint_button(&self, button: MouseButton) -> bool {
		button == self.paint_button
	}

	pub fn zoom_step(&self) -> f32 {
		self.zoom_step
	}
}

/// Parse one `[Bindings]` entry. New form first - `ACTION = CHORD [CHORD…]`
/// (empty value = unbind) - then the legacy inverted `CHORD = ACTION` with a
/// warning nudging toward the new form.
fn parse_entry(action: &str, value: &str) -> Result<Vec<Binding>, String> {
	if let Ok(Some(command)) = command::parse_line(action) {
		if value.trim().is_empty() {
			return Ok(Vec::new()); // explicit unbind
		}
		let chords: Result<Vec<Chord>, String> = value.split_whitespace().map(parse_chord).collect();
		if let Ok(chords) = chords {
			let line = normalize(action);
			return Ok(chords
				.into_iter()
				.map(|chord| Binding { chord, line: line.clone(), command: command.clone() })
				.collect());
		}
	}
	if let (Ok(chord), Ok(Some(command))) = (parse_chord(action), command::parse_line(value)) {
		eprintln!(
			"config: [Bindings] '{action}={value}' uses the legacy CHORD=ACTION form - flip it to '{value}={action}'"
		);
		return Ok(vec![Binding { chord, line: normalize(value), command }]);
	}
	Err("expected ACTION = CHORD [CHORD ...]".into())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn chords_parse() {
		assert_eq!(
			parse_chord("Ctrl+Shift+Z").unwrap(),
			Chord { ctrl: true, shift: true, alt: false, key: BindKey::Char("z".into()) },
		);
		assert_eq!(
			parse_chord("f1").unwrap(),
			Chord { ctrl: false, shift: false, alt: false, key: BindKey::Named(NamedKey::F1) },
		);
		assert_eq!(parse_chord("Backquote").unwrap().key, BindKey::Char("`".into()),);
		assert_eq!(parse_chord("Plus").unwrap().key, BindKey::Char("+".into()));
		assert_eq!(parse_chord("Del").unwrap().key, BindKey::Named(NamedKey::Delete));
		assert!(parse_chord("Ctrl+").is_err(), "no key");
		assert!(parse_chord("Ctrl+Q+W").is_err(), "two keys");
		assert!(parse_chord("Hyper+Q").is_err(), "unknown modifier is not a key");
	}

	#[test]
	fn chord_labels() {
		assert_eq!(parse_chord("ctrl+shift+z").unwrap().label(), "Ctrl+Shift+Z");
		assert_eq!(parse_chord("Delete").unwrap().label(), "Del");
		assert_eq!(parse_chord("Backquote").unwrap().label(), "`");
		assert_eq!(parse_chord("Ctrl+Equals").unwrap().label(), "Ctrl+=");
		assert_eq!(parse_chord("f1").unwrap().label(), "F1");
	}

	#[test]
	fn defaults_resolve_and_lookup_matches_modifiers() {
		let b = Bindings::load(None);
		let ctrl = ModifiersState::CONTROL;
		let none = ModifiersState::empty();

		let cmd = b.lookup(ctrl, &Key::Character("z".into()));
		assert_eq!(cmd, Some(Command::Undo));
		// Shifted logical key still matches the lowercase chord.
		let cmd = b.lookup(ctrl | ModifiersState::SHIFT, &Key::Character("Z".into()));
		assert_eq!(cmd, Some(Command::Redo));
		// The secondary redo chord works too.
		assert_eq!(b.lookup(ctrl, &Key::Character("y".into())), Some(Command::Redo));
		assert_eq!(b.lookup(none, &Key::Character("z".into())), None);
		assert_eq!(b.lookup(none, &Key::Named(NamedKey::Escape)), Some(Command::Quit { force: false }),);
		// Ctrl+S is Save Project (the dialog-aware save), not bare `save`.
		assert_eq!(b.lookup(ctrl, &Key::Character("s".into())), Some(Command::SaveProject));
		assert_eq!(b.lookup(none, &Key::Named(NamedKey::Delete)), Some(Command::Delete));
	}

	#[test]
	fn shared_chords_list_every_context() {
		// `1` belongs to both pass-pick (Pass mode) and the 100% zoom preset
		// (map mode) - lookup_all surfaces both for the shell to pick from.
		let b = Bindings::load(None);
		let all = b.lookup_all(ModifiersState::empty(), &Key::Character("1".into()));
		assert!(all.contains(&Command::PassPick { value: 1 }), "{all:?}");
		assert!(all.contains(&Command::ZoomTo { level: 1.0 }), "{all:?}");
	}

	#[test]
	fn ini_overrides_extend_and_unbind() {
		let mut b = Bindings::load(None);
		let ini = INI::from_str("[Bindings]\nredo=Ctrl+Z\nzoom-to 1=J\nfit=\nconsole toggle=F2 F3\n").unwrap();
		b.apply_keyboard(&ini);

		let ctrl = ModifiersState::CONTROL;
		let none = ModifiersState::empty();
		// `redo=Ctrl+Z` replaces redo's chords AND shadows undo's default.
		assert_eq!(b.lookup(ctrl, &Key::Character("z".into())), Some(Command::Redo));
		assert_eq!(b.lookup(ctrl, &Key::Character("y".into())), None, "old redo chord replaced");
		// New chord for an argumented action.
		assert_eq!(b.lookup(none, &Key::Character("j".into())), Some(Command::ZoomTo { level: 1.0 }));
		// `fit=` unbinds.
		assert_eq!(b.lookup(none, &Key::Character("f".into())), None);
		// Multi-chord values bind every chord.
		assert_eq!(b.lookup(none, &Key::Named(NamedKey::F2)), Some(Command::Console { on: None }));
		assert_eq!(b.lookup(none, &Key::Named(NamedKey::F3)), Some(Command::Console { on: None }));
	}

	#[test]
	fn legacy_chord_first_entries_still_apply() {
		let mut b = Bindings::load(None);
		let ini = INI::from_str("[Bindings]\nCtrl+G = zoom-to 2\nnonsense = nonsense\n").unwrap();
		b.apply_keyboard(&ini);
		assert_eq!(
			b.lookup(ModifiersState::CONTROL, &Key::Character("g".into())),
			Some(Command::ZoomTo { level: 2.0 })
		);
	}

	#[test]
	fn hint_table_maps_lines_to_first_chord() {
		let b = Bindings::load(None);
		let hints = b.hint_table();
		let hint = |line: &str| hints.iter().find(|(l, _)| l == line).map(|(_, c)| c.as_str());
		assert_eq!(hint("undo"), Some("Ctrl+Z"));
		assert_eq!(hint("redo"), Some("Ctrl+Shift+Z"), "first chord wins");
		assert_eq!(hint("select all"), Some("Ctrl+A"));
		assert_eq!(hint("delete"), Some("Del"));
		assert_eq!(hint("nonexistent"), None);
	}

	#[test]
	fn mouse_ini_applies_with_validation() {
		let mut b = Bindings::load(None);
		assert!(b.is_paint_button(MouseButton::Left), "default paint button");
		assert!(b.is_pan_button(MouseButton::Middle), "default pan button");

		let ini = INI::from_str("[Mouse]\nPanButtons = Right\nPaintButton = Middle\nZoomStep = 1.3\n").unwrap();
		b.apply_mouse(&ini);
		assert!(b.is_pan_button(MouseButton::Right));
		assert!(!b.is_pan_button(MouseButton::Left));
		assert!(b.is_paint_button(MouseButton::Middle));
		assert_eq!(b.zoom_step(), 1.3);

		let bad = INI::from_str("[Mouse]\nZoomStep = 9.0\n").unwrap();
		b.apply_mouse(&bad);
		assert_eq!(b.zoom_step(), 1.3, "out-of-range step ignored");
	}

	/// The shipped config file must always parse cleanly (new form only -
	/// the legacy fallback is for user files, not ours).
	#[test]
	fn shipped_config_file_is_valid() {
		let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("resources/config");
		assert!(dir.is_dir(), "resources/config/ missing at workspace root");

		let ini = INI::from_file(&dir.join("mme.ini")).unwrap();
		let section = ini.get_section("Bindings").expect("[Bindings]");
		for (action, value) in section {
			// The shipped config uses the documented PascalCase action keys, each
			// of which resolves to a command line that parses.
			assert!(ACTIONS.iter().any(|(name, ..)| name == action), "mme.ini '{action}': unknown action name");
			command::parse_line(command_for_key(action))
				.unwrap_or_else(|e| panic!("mme.ini '{action}': {e}"))
				.unwrap_or_else(|| panic!("mme.ini '{action}': empty action"));
			let value = value.to_string();
			for chord in value.split_whitespace() {
				parse_chord(chord).unwrap_or_else(|e| panic!("mme.ini '{action}': {e}"));
			}
		}

		assert!(ini.get_section("Mouse").is_some(), "[Mouse] missing");
		let paths = ini.get_section("Paths").expect("[Paths]");
		assert!(paths.has_entry("MaxPath"), "MaxPath key missing");
	}

	#[test]
	fn actions_table_is_valid() {
		// Every action's command line + default chord(s) must parse (a typo here
		// would ship a dead key), and its INI key must be a unique PascalCase name
		// that resolves back to the command.
		let mut names = std::collections::HashSet::new();
		for &(name, action, chords) in ACTIONS {
			command::parse_line(action)
				.unwrap_or_else(|e| panic!("ACTIONS '{name}': {e}"))
				.unwrap_or_else(|| panic!("ACTIONS '{name}': empty action"));
			for chord in chords.split_whitespace() {
				parse_chord(chord).unwrap_or_else(|e| panic!("ACTIONS '{name}' chord '{chord}': {e}"));
			}
			assert!(names.insert(name), "ACTIONS: duplicate key '{name}'");
			let first_upper = name.chars().next().is_some_and(|c| c.is_ascii_uppercase());
			assert!(first_upper && !name.contains(' ') && !name.contains('-'), "ACTIONS: '{name}' is not PascalCase");
			assert_eq!(command_for_key(name), action, "ACTIONS: '{name}' must resolve to its command");
		}
	}
}
