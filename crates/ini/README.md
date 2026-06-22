# ini

INI parser with two re-MAX extensions on top of the usual `[section]` + `key=value` syntax:

- `[[mod X]]` - include another module, either `X.ini` next to the current file or `X/mod.ini` in a subfolder. Parsed recursively into the same flat namespace, with cycle detection.
- Re-opening an existing `[section]` is *merge*, not an error: later `key=value` lines append new keys and override duplicates. This is what lets layered mods extend a base config. (Matches the contract documented in `apps/game/assets/config/main.ini`.)

## Usage

```rust
use ini::INI;

let ini = INI::from_file(Path::new("main.ini"))?;
let sprites = ini.get_section("Sprites").unwrap();
let count: i64 = sprites.get_entry("count").unwrap();
```

Values auto-type into `i64`, `bool` (`yes`/`no`/`true`/`false`), or `String` at parse time; `get_entry::<T>` returns `None` if the stored type doesn't match `T`.

## Public surface

```rust
pub use ini::INI;
pub use ini_section::INISection;
pub use parse_ini::parse_ini_file;
```

## Errors

Parser errors carry the offending line number + file path (or `<string>` for in-memory input):

```
Module identifier cannot contain non-ASCII or non-printable characters
    error at assets/config/v0.1/mod.ini:2
```

Structural violations caught: empty section name, empty key, key-value outside any section, circular module inclusion, non-ASCII module identifier.
