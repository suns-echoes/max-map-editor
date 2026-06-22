//! Project (de)serialization: the JSON `from_str_in` reader and the
//! `save_string` writer, split out of the document model in `super`.

use super::*;
use crate::palette::{set_slot_rgb, slot_rgb};

impl Project {
	/// As `from_str`, but referenced packs are looked up in `assets_root`
	/// first, then in `project_dir` (the saved `.json`'s folder) - that's
	/// where a project saved from an imported WRL dumps its synthetic pack.
	pub fn from_str_in(text: &str, assets_root: &Path, project_dir: Option<&Path>) -> Result<Self, String> {
		let root = json::parse(text)?;
		let field = |key: &str| root.get(key).ok_or(format!("missing field '{key}'"));

		// Version guard. The current scheme stores `mme_project_file_version` =
		// "MAJOR.MINOR": a matching MAJOR opens (and is migrated up to this
		// editor's MINOR); a different MAJOR is a hard break. A pre-scheme
		// `"version": "1"` is grandfathered in and migrated to the current form.
		let current_major: u32 =
			PROJECT_VERSION.split('.').next().and_then(|m| m.parse().ok()).expect("PROJECT_VERSION is MAJOR.MINOR");
		if let Some(raw) = root.get("mme_project_file_version") {
			let raw = raw.as_str().ok_or("mme_project_file_version not a string")?;
			let (maj, min) =
				raw.split_once('.').ok_or(format!("bad mme_project_file_version '{raw}' (want MAJOR.MINOR)"))?;
			let major: u32 = maj.parse().map_err(|_| format!("bad mme_project_file_version '{raw}'"))?;
			min.parse::<u32>().map_err(|_| format!("bad mme_project_file_version '{raw}'"))?;
			if major != current_major {
				return Err(format!("project version {raw} is unsupported - this editor reads {current_major}.x"));
			}
			// Same MAJOR: open. (Future MINOR migrations would run here.)
		} else if let Some(legacy) = root.get("version").and_then(|v| v.as_str()) {
			if legacy != "1" {
				return Err(format!("unsupported legacy project version '{legacy}'"));
			}
		} else {
			return Err("missing field 'mme_project_file_version'".into());
		}
		// Every opened document migrates to the version this editor writes.
		let version = PROJECT_VERSION.to_string();
		let name = field("name")?.as_str().unwrap_or("").to_string();
		let description = field("description")?.as_str().unwrap_or("").to_string();
		// Optional Map Preferences metadata (all default to empty / unspecified).
		let str_field = |key: &str| root.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
		// `players` is the max count, saved as its preferences label ("2"/"2-3"/
		// "2-4"); a bare number is also accepted (legacy saves).
		let players = root.get("players").and_then(|v| match v.as_str() {
			Some("2") => Some(2),
			Some("2-3") => Some(3),
			Some("2-4") => Some(4),
			Some(other) => other.parse::<u8>().ok().map(|n| n.clamp(2, 4)),
			None => v.as_f64().map(|n| (n as u8).clamp(2, 4)),
		});
		let date = str_field("date");
		let map_version = str_field("map_version");
		let author = str_field("author");
		let width = field("width")?.as_f64().ok_or("width not a number")? as u16;
		let height = field("height")?.as_f64().ok_or("height not a number")? as u16;
		check_map_size(width, height)?;

		// `use` - load referenced packs; exactly one owns the palette.
		let mut uses = Vec::new();
		let mut packs = Vec::new();
		for entry in field("use")?.as_array().ok_or("'use' not an array")? {
			let name = entry.get("name").and_then(|v| v.as_str()).ok_or("use entry: no name")?;
			let use_entry = UseEntry {
				name: name.to_string(),
				tileset: entry.get("tileset").and_then(|v| v.as_bool()).unwrap_or(false),
				palette: entry.get("palette").and_then(|v| v.as_bool()).unwrap_or(false),
				version: entry.get("version").and_then(|v| v.as_str()).unwrap_or("1").to_string(),
			};
			// assets_root first, then the project's own folder (imported-WRL packs).
			let pack = if !assets_root.join(name).is_dir() && project_dir.is_some_and(|d| d.join(name).is_dir()) {
				TilePack::load(project_dir.unwrap(), name)?
			} else {
				TilePack::load(assets_root, name)?
			};
			packs.push(pack);
			uses.push(use_entry);
		}
		// User-owned packs join before cells parse, so a saved map's custom-tile
		// ids resolve (they live in resources/user/assets, not in `use`).
		append_user_packs(&mut packs, assets_root);
		let palette_owners: Vec<usize> = uses.iter().enumerate().filter(|(_, u)| u.palette).map(|(i, _)| i).collect();
		let [owner] = palette_owners[..] else {
			return Err(format!("expected exactly one palette owner, got {}", palette_owners.len()));
		};
		let mut pack_palette = packs[owner]
			.palette
			.clone()
			.ok_or_else(|| format!("palette owner '{}' has no palette.json", uses[owner].name))?;
		// The file's own bytes, kept for debug rendering / inspection.
		let source_palette = pack_palette.clone();
		// Static slots belong to the game (contract §1) - the engine
		// replaces them at runtime, so the editor resolves them to the
		// in-game values too (pack bytes there are converter leftovers).
		crate::game_palette::apply_game_statics(&mut pack_palette);
		// Optional `"palette"` block: this map's dynamic-slot overrides
		// (`{ "96": "#aabbcc", … }`) over the owner pack's palette.
		let mut palette = pack_palette.clone();
		if let Some(overrides) = root.get("palette") {
			let entries = overrides.as_object().ok_or("'palette' not an object")?;
			for (key, value) in entries {
				let slot: u8 = key.parse().map_err(|_| format!("palette override: bad slot '{key}'"))?;
				if !DYNAMIC_SLOTS.contains(&slot) {
					return Err(format!("palette override slot {slot} outside the dynamic range 64..=159",));
				}
				let hex = value
					.as_str()
					.and_then(|s| s.strip_prefix('#'))
					.filter(|h| h.len() == 6)
					.ok_or(format!("palette override {slot}: expected \"#rrggbb\""))?;
				let rgb = crate::color::parse_hex_rgb(hex)
					.ok_or_else(|| format!("palette override {slot}: bad hex '#{hex}'"))?;
				set_slot_rgb(&mut palette, slot, rgb);
			}
		}

		// Optional `"tilepass"` block: persisted per-tile passability,
		// `{ "TSTW000": 1, … }`. Applied onto the packs *before* cells are
		// decomposed, so a WRL import recovers its layer split from the project's
		// own pass. (Must run before the immutable `resolve` borrow below.)
		if let Some(tp) = root.get("tilepass") {
			let entries = tp.as_object().ok_or("'tilepass' not an object")?;
			for (id, value) in entries {
				let v = value.as_f64().ok_or(format!("tilepass {id}: not a number"))? as u8;
				if v > 3 {
					return Err(format!("tilepass {id}: value out of range (0..=3)"));
				}
				let pack = packs
					.iter_mut()
					.find(|p| p.index_of.contains_key(id.as_str()))
					.ok_or(format!("tilepass: unknown tile id '{id}'"))?;
				let tile = pack.index_of[id.as_str()];
				if let Some(pass) = pack.pass.as_mut() {
					pass[tile as usize] = v;
				}
			}
		}

		// Tile id → (pack, index) across all used packs.
		let resolve = |id: &str| -> Result<(u8, u16), String> {
			for (pack_index, pack) in packs.iter().enumerate() {
				if let Some(&tile) = pack.index_of.get(id) {
					return Ok((pack_index as u8, tile));
				}
			}
			Err(format!("unknown tile id '{id}'"))
		};
		// v1 heuristic: the WATER pack fills the water layer; everything
		// else is ground. v2 will declare layers explicitly.
		let water_pack = uses.iter().position(|u| u.name == "WATER").map(|i| i as u8);
		// A WRL import has no "WATER" pack, so the heuristic can't find the base
		// layer - recover the split by passability instead, mirroring `from_wrl`.
		let wrl_import = !uses.is_empty() && uses.iter().all(|u| u.version == "wrl");

		let rows = field("map")?.as_array().ok_or("'map' not an array")?;
		if rows.len() != height as usize {
			return Err(format!("map has {} rows, want {height}", rows.len()));
		}
		let mut cells = Vec::with_capacity(width as usize * height as usize);
		for (y, row) in rows.iter().enumerate() {
			let row = row.as_array().ok_or(format!("row {y} not an array"))?;
			if row.len() != width as usize {
				return Err(format!("row {y} has {} cells, want {width}", row.len()));
			}
			for (x, cell) in row.iter().enumerate() {
				// Cells appear as "WATR00,CSd001" or ["WATR00", "CSd001"]
				// in the v1 corpus - accept both, save normalizes to the
				// comma-string form.
				let parts: Vec<&str> = if let Some(text) = cell.as_str() {
					text.split(',').filter(|p| !p.is_empty()).collect()
				} else if let Some(list) = cell.as_array() {
					list.iter()
						.map(|v| v.as_str().ok_or(format!("cell {x},{y}: non-string entry")))
						.collect::<Result<_, _>>()?
				} else {
					return Err(format!("cell {x},{y} not a string or array"));
				};
				// Resolve every tile, with its v1 *preferred* layer (WATER pack →
				// base). Layers are advisory, not strict - a convenience, not a
				// constraint - so we never reject a stack.
				let mut refs: Vec<(usize, TileRef)> = Vec::with_capacity(parts.len());
				for part in &parts {
					let (id, transform) = match part.split_once(':') {
						Some((id, t)) => (id, Transform::parse(t).map_err(|e| format!("cell {x},{y}: {e}"))?),
						None => (*part, Transform::default()),
					};
					let (pack, tile) = resolve(id).map_err(|e| format!("cell {x},{y}: {e}"))?;
					let layer = if wrl_import {
						let pass = packs[pack as usize].pass.as_ref().map(|p| p[tile as usize]).unwrap_or(0);
						pass_layer(pass)
					} else if Some(pack) == water_pack {
						LAYER_WATER
					} else {
						LAYER_GROUND
					};
					refs.push((layer, TileRef { pack, tile, transform }));
				}
				// The heuristic places each tile on its preferred layer - but an
				// opened WRL's synthetic pack is no longer recognized as WATER, so
				// its base tile and a painted tile would both want the ground layer
				// and collide. When that happens, fall back to a positional
				// reconstruction (`save_string` writes the stack bottom-up: first
				// part → base layer, each subsequent one up), which loads cleanly
				// instead of erroring. Collision-free stacks keep the v1 layout
				// byte-for-byte.
				let mut seen = 0u32;
				let collides = refs.len() > MAX_LAYERS
					|| refs.iter().any(|&(layer, _)| {
						let hit = seen & (1 << layer) != 0;
						seen |= 1 << layer;
						hit
					});
				let mut stack: [Option<TileRef>; MAX_LAYERS] = [None; MAX_LAYERS];
				for (i, (layer, tref)) in refs.into_iter().enumerate() {
					let slot = if collides { i.min(MAX_LAYERS - 1) } else { layer };
					stack[slot] = Some(tref);
				}
				cells.push(stack);
			}
		}

		// Optional `"pass"` block - per-cell pass overrides (0 land / 1 water /
		// 2 shore / 3 blocked). New form: a dense grid of digit-rows, `'-'` = no
		// override. Old form (still accepted): a sparse `{ "x,y": value }` object.
		let mut pass_overrides = vec![None; width as usize * height as usize];
		if let Some(po) = root.get("pass") {
			if let Some(rows) = po.as_array() {
				if rows.len() != height as usize {
					return Err(format!("pass has {} rows, want {height}", rows.len()));
				}
				for (y, row) in rows.iter().enumerate() {
					let row = row.as_str().ok_or(format!("pass row {y}: not a string"))?;
					if row.chars().count() != width as usize {
						return Err(format!("pass row {y} has {} cells, want {width}", row.chars().count()));
					}
					for (x, c) in row.chars().enumerate() {
						pass_overrides[y * width as usize + x] = match c {
							'-' => None,
							'0'..='3' => Some(c as u8 - b'0'),
							other => return Err(format!("pass {x},{y}: bad cell '{other}' (-|0|1|2|3)")),
						};
					}
				}
			} else if let Some(entries) = po.as_object() {
				for (key, value) in entries {
					let (xs, ys) = key.split_once(',').ok_or(format!("pass key '{key}': want x,y"))?;
					let x: u16 = xs.trim().parse().map_err(|_| format!("pass key '{key}': bad x"))?;
					let y: u16 = ys.trim().parse().map_err(|_| format!("pass key '{key}': bad y"))?;
					let v = value.as_f64().ok_or(format!("pass {key}: not a number"))? as u8;
					if x >= width || y >= height || v > 3 {
						return Err(format!("pass {key}: out of range"));
					}
					pass_overrides[y as usize * width as usize + x as usize] = Some(v);
				}
			} else {
				return Err("'pass' must be an array of rows or an x,y object".into());
			}
		}

		// Optional `"units"` block: unit-preview annotations as compact
		// `"TAG x y team"` strings (editor aid - never baked into the WRL).
		let mut units = Vec::new();
		if let Some(list) = root.get("units") {
			for (i, entry) in list.as_array().ok_or("'units' not an array")?.iter().enumerate() {
				let text = entry.as_str().ok_or(format!("units[{i}]: not a string"))?;
				let parts: Vec<&str> = text.split_whitespace().collect();
				let [tag, xs, ys, ts] = parts[..] else {
					return Err(format!("units[{i}] '{text}': want \"TAG x y team\""));
				};
				let x: u16 = xs.parse().map_err(|_| format!("units[{i}]: bad x"))?;
				let y: u16 = ys.parse().map_err(|_| format!("units[{i}]: bad y"))?;
				let team: u8 = ts.parse().map_err(|_| format!("units[{i}]: bad team"))?;
				if x >= width || y >= height || team > 4 {
					return Err(format!("units[{i}] '{text}': out of range"));
				}
				units.push(UnitNote { tag: tag.to_string(), x, y, team });
			}
		}

		Ok(Self {
			version,
			name,
			description,
			players,
			date,
			map_version,
			author,
			width,
			height,
			uses,
			packs,
			cells,
			pass_overrides,
			palette,
			pack_palette,
			source_palette,
			water_pack,
			units,
			dirty: false,
			revision: 0,
			structure: 0,
			undo_stack: Vec::new(),
			redo_stack: Vec::new(),
			stroke: None,
		})
	}

	/// Serialize back to the v1 JSON format (round-trip stable).
	pub fn save_string(&self) -> String {
		use json::JsonValue as J;
		let use_entries: Vec<J> = self
			.uses
			.iter()
			.map(|u| {
				let mut fields = vec![("name".to_string(), J::String(u.name.clone()))];
				if u.tileset {
					fields.push(("tileset".to_string(), J::Bool(true)));
				}
				if u.palette {
					fields.push(("palette".to_string(), J::Bool(true)));
				}
				fields.push(("version".to_string(), J::String(u.version.clone())));
				J::Object(fields)
			})
			.collect();

		let rows = encode_cell_grid(self.width as usize, self.height as usize, |x, y| {
			let stack = &self.cells[y * self.width as usize + x];
			let mut text = String::new();
			for layer in stack.iter().flatten() {
				if !text.is_empty() {
					text.push(',');
				}
				text.push_str(&self.packs[layer.pack as usize].ids[layer.tile as usize]);
				text.push_str(&layer.transform.suffix());
			}
			text
		});

		// The map's palette overrides: dynamic slots differing from the
		// owner pack's palette, as a sparse `{ "96": "#aabbcc" }` block.
		let mut overrides = Vec::new();
		for slot in DYNAMIC_SLOTS {
			let rgb = slot_rgb(&self.palette, slot);
			if rgb != slot_rgb(&self.pack_palette, slot) {
				overrides.push((slot.to_string(), J::String(crate::color::rgb_to_hex(rgb))));
			}
		}

		let mut fields = vec![
			("mme_project_file_version".to_string(), J::String(self.version.clone())),
			("name".to_string(), J::String(self.name.clone())),
			("description".to_string(), J::String(self.description.clone())),
			("width".to_string(), J::Number(self.width as f64)),
			("height".to_string(), J::Number(self.height as f64)),
			("use".to_string(), J::Array(use_entries)),
		];
		// Optional Map Preferences metadata - written only when set, so a map
		// without preferences stays byte-identical.
		if let Some(p) = self.players {
			// Saved as the preferences label, not a bare number.
			let label = match p {
				2 => "2",
				3 => "2-3",
				_ => "2-4",
			};
			fields.push(("players".to_string(), J::String(label.to_string())));
		}
		for (key, value) in [("date", &self.date), ("map_version", &self.map_version), ("author", &self.author)] {
			if !value.is_empty() {
				fields.push((key.to_string(), J::String(value.clone())));
			}
		}
		if !overrides.is_empty() {
			fields.push(("palette".to_string(), J::Object(overrides)));
		}
		// Per-tile passability of every tile in use (Pass Table Editor state),
		// `{ "TSTW000": 1, … }`. Passability is tile-dependent: the pack holds
		// the live value, and this persists it at the project level so a reload
		// restores edits even for shared, read-only packs.
		let mut seen = std::collections::HashSet::new();
		let mut tilepass: Vec<(String, J)> = Vec::new();
		for stack in &self.cells {
			for layer in stack.iter().flatten() {
				if seen.insert((layer.pack, layer.tile)) {
					if let Some(pass) = self.packs[layer.pack as usize].pass.as_ref() {
						let id = self.packs[layer.pack as usize].ids[layer.tile as usize].clone();
						tilepass.push((id, J::Number(pass[layer.tile as usize] as f64)));
					}
				}
			}
		}
		tilepass.sort_by(|a, b| a.0.cmp(&b.0));
		if !tilepass.is_empty() {
			fields.push(("tilepass".to_string(), J::Object(tilepass)));
		}
		// Per-cell pass overrides as a dense grid of digit-rows - `'-'` = no
		// override, `'0'..'3'` = a local override (Local Pass Override Editor).
		// Written only when an override exists, so derived-pass maps stay
		// block-free.
		if self.pass_overrides.iter().any(Option::is_some) {
			let rows: Vec<J> = (0..self.height as usize)
				.map(|y| {
					let mut row = String::with_capacity(self.width as usize);
					for x in 0..self.width as usize {
						row.push(match self.pass_overrides[y * self.width as usize + x] {
							Some(v) => (b'0' + v) as char,
							None => '-',
						});
					}
					J::String(row)
				})
				.collect();
			fields.push(("pass".to_string(), J::Array(rows)));
		}
		// Unit-preview annotations as compact `"TAG x y team"` strings -
		// only when present, so unit-free projects stay byte-identical.
		if !self.units.is_empty() {
			let list: Vec<J> =
				self.units.iter().map(|u| J::String(format!("{} {} {} {}", u.tag, u.x, u.y, u.team))).collect();
			fields.push(("units".to_string(), J::Array(list)));
		}
		fields.push(("map".to_string(), J::Array(rows)));
		J::Object(fields).to_pretty()
	}
}
