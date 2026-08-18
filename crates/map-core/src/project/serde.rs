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
		// Optional Map Metadata (all default to empty / unspecified).
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
			// Joined onto `assets_root` / the project dir to find the pack, and
			// onto the save target again by `write_project` - so it has to be a
			// plain directory name, not a path out of the tree.
			super::check_name_component("use entry", name)?;
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
		// ids resolve (they live in resources/user/tilepacks, not in `use`).
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
				// Cells appear as "WTR000,CSd001" or ["WTR000", "CSd001"]
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

		// Optional `"scenery"` block: cut-out objects placed by pixel. Loading the
		// libraries is best-effort - a checkout without the baked assets opens the
		// map with inert placements rather than refusing it, and they persist
		// untouched so the assets can come back.
		let mut scenery = Vec::new();
		if let Some(list) = root.get("scenery") {
			for (i, entry) in list.as_array().ok_or("'scenery' not an array")?.iter().enumerate() {
				scenery.push(read_scenery(entry, i)?);
			}
		}
		let scenery_packs = super::load_scenery_packs(assets_root, &uses);

		// Optional `"objects"` block: first-class map objects (units / slabs /
		// rubble / preview annotations) as JSON objects. Superseded the old
		// compact `"units"` string block (still read below for migration).
		let mut objects = Vec::new();
		if let Some(list) = root.get("objects") {
			for (i, entry) in list.as_array().ok_or("'objects' not an array")?.iter().enumerate() {
				objects.push(read_object(entry, i, width, height)?);
			}
		} else if let Some(list) = root.get("units") {
			// Legacy `"units"`: `"TAG x y team"` strings (pre-S2.1 preview notes).
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
				let unit_type = max_assets::save::unit_type_id(tag)
					.ok_or_else(|| format!("units[{i}] '{text}': unknown unit tag '{tag}'"))?;
				objects.push(MapObject { unit_type, x, y, team, props: ObjectProps::default() });
			}
		}

		// Optional `"save"` block: an embedded M.A.X. save (`.DTA`) as base64,
		// re-decoded at the map's dimensions (a save-editor session, per D1). The
		// `.DTA` has no stored dimensions, so it must be decoded at this project's
		// width×height — the world the save was opened onto.
		let save = match root.get("save") {
			Some(v) => {
				let b64 = v.as_str().ok_or("'save' not a string")?;
				let raw = max_assets::base64::decode(b64).map_err(|e| format!("save: {e}"))?;
				let file = read_save_bytes(&raw, (width, height)).map_err(|e| format!("decode embedded save: {e}"))?;
				Some(EmbeddedSave { raw, file })
			}
			None => None,
		};

		// A pre-S2.1 save session embedded the `.DTA` but had no object model;
		// seed it from the save so its units still overlay (later saves persist
		// the edited `"objects"` block instead, so this only fires once).
		if objects.is_empty() {
			if let Some(save) = &save {
				objects = objects_from_save(&save.file);
			}
		}

		// The editable resource map (S5): seed from the embedded save's pristine
		// cargo map, then apply the persisted edit diff (`"resources"`: a flat list
		// of `[x, y, value]` triples for the cells the user changed).
		let mut cargo_map = save.as_ref().map(|s| s.file.cargo_map.clone()).unwrap_or_default();
		if let Some(res) = root.get("resources") {
			let arr = res.as_array().ok_or("'resources' not an array")?;
			for (i, entry) in arr.iter().enumerate() {
				let t =
					entry.as_array().filter(|t| t.len() == 3).ok_or(format!("resources[{i}]: expected [x, y, v]"))?;
				let n = |k: usize| t[k].as_f64().ok_or(format!("resources[{i}]: non-numeric field"));
				let (x, y, v) = (n(0)? as usize, n(1)? as usize, n(2)? as u16);
				if x >= width as usize || y >= height as usize {
					return Err(format!("resources[{i}]: ({x},{y}) outside the {width}x{height} map"));
				}
				let cell = y * width as usize + x;
				*cargo_map.get_mut(cell).ok_or(format!("resources[{i}]: no cargo map (missing save)"))? = v;
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
			objects,
			scenery,
			scenery_packs,
			save,
			cargo_map,
			dirty: false,
			revision: 0,
			structure: 0,
			pending_label: None,
			undo_seq: 0,
			undo_stack: Vec::new(),
			redo_stack: Vec::new(),
			stroke: None,
			render_dirty_cells: None,
			render_dirty_pass: None,
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
		// Optional Map Metadata - written only when set, so a map
		// without metadata stays byte-identical.
		if let Some(p) = self.players {
			// Saved as its label, not a bare number.
			fields.push(("players".to_string(), J::String(players_label(p).to_string())));
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
		// Map objects (preview annotations / save units) as JSON objects -
		// only when present, so object-free projects stay byte-identical. Props
		// are omitted when default, so a preview annotation stays compact.
		if !self.objects.is_empty() {
			let list: Vec<J> = self.objects.iter().map(write_object).collect();
			fields.push(("objects".to_string(), J::Array(list)));
		}
		// Scenery placements, only when present so scenery-free projects stay
		// byte-identical. Unresolved placements are written back verbatim.
		if !self.scenery.is_empty() {
			let list: Vec<J> = self
				.scenery
				.iter()
				.map(|s| {
					let mut fields = vec![
						("pack".to_string(), J::String(s.pack.clone())),
						("piece".to_string(), J::String(s.piece.clone())),
						("x".to_string(), J::Number(s.x as f64)),
						("y".to_string(), J::Number(s.y as f64)),
					];
					// Only when it is not the default, so a project of plain
					// placements stays byte-identical to one written before
					// blending existed.
					if s.blend != crate::scenery::SceneryBlend::Normal {
						fields.push(("blend".to_string(), J::String(s.blend.name().to_string())));
					}
					J::Object(fields)
				})
				.collect();
			fields.push(("scenery".to_string(), J::Array(list)));
		}
		// An opened M.A.X. save (`.DTA`) as base64 - the byte-exact export anchor
		// for a save-editor session (D1). Only present when a save is embedded, so
		// ordinary map projects stay byte-identical.
		if let Some(save) = &self.save {
			fields.push(("save".to_string(), J::String(max_assets::base64::encode(&save.raw))));
			// Resource (cargo) edits (S5): only the cells whose value diverges from
			// the save's pristine seed, as `[x, y, value]` triples — compact (the
			// seed already round-trips inside the base64 `.DTA`), and absent entirely
			// when nothing was painted.
			let seed = &save.file.cargo_map;
			let w = self.width as usize;
			let diff: Vec<J> = self
				.cargo_map
				.iter()
				.enumerate()
				.filter(|(i, v)| seed.get(*i) != Some(*v))
				.map(|(i, &v)| {
					J::Array(vec![J::Number((i % w) as f64), J::Number((i / w) as f64), J::Number(v as f64)])
				})
				.collect();
			if !diff.is_empty() {
				fields.push(("resources".to_string(), J::Array(diff)));
			}
		}
		fields.push(("map".to_string(), J::Array(rows)));
		J::Object(fields).to_pretty()
	}

	/// The Map Metadata as a standalone JSON object, or `None` when every
	/// field is unset. `export` appends it to the baked WRL after the binary
	/// payload - a tail both the game and `read_wrl_file` ignore - so the
	/// metadata travels with the exported map. Keys and the `players` label
	/// match the project file's; `mme_map_metadata` tags the blob's format.
	pub fn info_json(&self) -> Option<String> {
		use json::JsonValue as J;
		let mut fields = vec![("mme_map_metadata".to_string(), J::Number(1.0))];
		if !self.name.is_empty() {
			fields.push(("name".to_string(), J::String(self.name.clone())));
		}
		if let Some(p) = self.players {
			fields.push(("players".to_string(), J::String(players_label(p).to_string())));
		}
		for (key, value) in [
			("description", &self.description),
			("date", &self.date),
			("map_version", &self.map_version),
			("author", &self.author),
		] {
			if !value.is_empty() {
				fields.push((key.to_string(), J::String(value.clone())));
			}
		}
		(fields.len() > 1).then(|| J::Object(fields).to_pretty())
	}
}

/// Serialize one [`MapObject`] to a JSON object: the type name (`"t"`), cell,
/// and team, then any non-default [`ObjectProps`]. Props are omitted at their
/// default so a preview annotation stays compact (`{"t":"TANK","x":..}`); the
/// type is written by name (`unit_type_name`), stable across `ResourceID` shifts.
fn write_object(obj: &MapObject) -> json::JsonValue {
	use json::JsonValue as J;
	let name = max_assets::save::unit_type_name(obj.unit_type).unwrap_or("");
	let mut fields = vec![
		("t".to_string(), J::String(name.to_string())),
		("x".to_string(), J::Number(obj.x as f64)),
		("y".to_string(), J::Number(obj.y as f64)),
		("team".to_string(), J::Number(obj.team as f64)),
	];
	let p = &obj.props;
	if !p.name.is_empty() {
		fields.push(("name".to_string(), J::String(p.name.clone())));
	}
	for (key, value) in [
		("angle", p.angle as f64),
		("turret", p.turret_angle as f64),
		("hits", p.hits as f64),
		("ammo", p.ammo as f64),
		("orders", p.orders as f64),
		("disabled", p.disabled_turns as f64),
		("storage", p.storage as f64),
		("connectors", p.connectors as f64),
	] {
		if value != 0.0 {
			fields.push((key.to_string(), J::Number(value)));
		}
	}
	if let Some(id) = p.source_id {
		fields.push(("id".to_string(), J::Number(id as f64)));
	}
	// The per-unit max-stats override (S4.5), only when this unit was edited off
	// its shared seed. Written as a nested object of every `UnitValues` field so
	// the clone round-trips exactly (a future byte-aware export re-emits it).
	if let Some(v) = &p.base_values {
		fields.push(("values".to_string(), write_unit_values(v)));
	}
	J::Object(fields)
}

/// Serialize a [`UnitValues`] as a flat JSON object of all its numeric fields
/// (`in_use` as 0/1), the inverse of [`read_unit_values`]. Every field is written
/// so the per-unit override clone round-trips byte-for-byte.
fn write_unit_values(v: &UnitValues) -> json::JsonValue {
	use json::JsonValue as J;
	J::Object(
		[
			("turns", v.turns as f64),
			("hits", v.hits as f64),
			("armor", v.armor as f64),
			("attack", v.attack as f64),
			("speed", v.speed as f64),
			("range", v.range as f64),
			("rounds", v.rounds as f64),
			("move_and_fire", v.move_and_fire as f64),
			("scan", v.scan as f64),
			("storage", v.storage as f64),
			("ammo", v.ammo as f64),
			("attack_radius", v.attack_radius as f64),
			("agent_adjust", v.agent_adjust as f64),
			("version", v.version as f64),
			("in_use", v.in_use as u8 as f64),
		]
		.into_iter()
		.map(|(k, n)| (k.to_string(), J::Number(n)))
		.collect(),
	)
}

/// Parse one [`MapObject`] from a JSON object written by [`write_object`]:
/// required `t` / `x` / `y` / `team`, optional props (missing = default). `i` is
/// the array index for error messages; `width`/`height` bound the cell.
fn read_object(entry: &json::JsonValue, i: usize, width: u16, height: u16) -> Result<MapObject, String> {
	let obj = entry.as_object().ok_or(format!("objects[{i}]: not an object"))?;
	let get = |key: &str| entry.get(key);
	let num = |key: &str| -> Result<f64, String> {
		get(key).and_then(|v| v.as_f64()).ok_or(format!("objects[{i}]: missing/invalid '{key}'"))
	};
	// Optional numeric prop, defaulting to 0 when absent (present-but-invalid errors).
	let opt = |key: &str| -> Result<f64, String> {
		match get(key) {
			None => Ok(0.0),
			Some(v) => v.as_f64().ok_or(format!("objects[{i}]: invalid '{key}'")),
		}
	};
	let tag = get("t").and_then(|v| v.as_str()).ok_or(format!("objects[{i}]: missing 't'"))?;
	let unit_type = max_assets::save::unit_type_id(tag).ok_or(format!("objects[{i}]: unknown unit type '{tag}'"))?;
	let x = num("x")? as u16;
	let y = num("y")? as u16;
	let team = num("team")? as u8;
	if x >= width || y >= height {
		return Err(format!("objects[{i}] '{tag}': ({x},{y}) outside the {width}x{height} map"));
	}
	// Same bound the legacy `units` loader enforces: four players plus the alien
	// slot. Downstream clamps rather than checks, so a hand-edited file is caught
	// here or not at all.
	if team > 4 {
		return Err(format!("objects[{i}] '{tag}': team {team} outside 0..=4"));
	}
	let props = ObjectProps {
		name: get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
		angle: opt("angle")? as u8,
		turret_angle: opt("turret")? as u8,
		hits: opt("hits")? as u16,
		ammo: opt("ammo")? as u8,
		orders: opt("orders")? as u8,
		disabled_turns: opt("disabled")? as u8,
		storage: opt("storage")? as i16,
		connectors: opt("connectors")? as u16,
		source_id: obj.iter().any(|(k, _)| k == "id").then(|| opt("id").map(|v| v as u16)).transpose()?,
		base_values: get("values").map(|v| read_unit_values(v, i)).transpose()?,
	};
	Ok(MapObject { unit_type, x, y, team, props })
}

/// Parse a [`UnitValues`] override from the `"values"` block written by
/// [`write_unit_values`]. Every field is optional (absent = 0 / `in_use` false),
/// so a hand-written or partial block still loads; `i` names the object for
/// errors. Present-but-non-numeric fields error rather than silently zeroing.
fn read_unit_values(entry: &json::JsonValue, i: usize) -> Result<UnitValues, String> {
	let field = |key: &str| -> Result<u16, String> {
		match entry.get(key) {
			None => Ok(0),
			Some(v) => v.as_f64().map(|n| n as u16).ok_or(format!("objects[{i}].values: invalid '{key}'")),
		}
	};
	Ok(UnitValues {
		turns: field("turns")?,
		hits: field("hits")?,
		armor: field("armor")?,
		attack: field("attack")?,
		speed: field("speed")?,
		range: field("range")?,
		rounds: field("rounds")?,
		move_and_fire: field("move_and_fire")? as u8,
		scan: field("scan")?,
		storage: field("storage")?,
		ammo: field("ammo")?,
		attack_radius: field("attack_radius")?,
		agent_adjust: field("agent_adjust")?,
		version: field("version")?,
		in_use: field("in_use")? != 0,
	})
}

/// A player count's on-disk label ("2-3" = two to three players). Two stays
/// the legacy bare "2" so existing saves and readers keep matching; the Map
/// Metadata dialog shows the same ranges (with "2-2" for two).
fn players_label(p: u8) -> &'static str {
	match p {
		2 => "2",
		3 => "2-3",
		_ => "2-4",
	}
}

/// One `"scenery"` entry. The position is unclamped on purpose: an object may
/// legitimately hang off the map's left or top edge, and a placement whose
/// piece is missing must survive the round-trip unaltered.
fn read_scenery(entry: &json::JsonValue, i: usize) -> Result<ScenerySpot, String> {
	let text = |key: &str| {
		entry.get(key).and_then(|v| v.as_str()).map(str::to_string).ok_or(format!("scenery[{i}]: missing '{key}'"))
	};
	let coord = |key: &str| {
		entry
			.get(key)
			.and_then(|v| v.as_f64())
			.filter(|f| f.is_finite() && f.abs() <= i32::MAX as f64)
			.map(|f| f as i32)
			.ok_or(format!("scenery[{i}]: missing/invalid '{key}'"))
	};
	let blend = match entry.get("blend").and_then(|v| v.as_str()) {
		None => crate::scenery::SceneryBlend::default(),
		Some(name) => crate::scenery::SceneryBlend::parse(name)
			.ok_or(format!("scenery[{i}]: unknown blend '{name}' (normal|brighter|darker|higher)"))?,
	};
	Ok(ScenerySpot { pack: text("pack")?, piece: text("piece")?, x: coord("x")?, y: coord("y")?, blend })
}

#[cfg(test)]
mod tests {
	use super::*;

	fn assets_root() -> std::path::PathBuf {
		Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets/tilepacks")
	}

	/// A loadable 2×1 GREEN project with `extra` top-level fields spliced in
	/// (same shape as the `project/mod.rs` malformed-body harness).
	fn base(map: &str, extra: &str) -> String {
		format!(
			r#"{{"version":"1","name":"t","description":"","width":2,"height":1,"use":[{{"name":"GREEN","tileset":true,"palette":true,"version":"1"}}]{extra},"map":{map}}}"#
		)
	}

	fn err(json: String) -> String {
		match Project::from_str(&json, &assets_root()) {
			Ok(_) => panic!("expected a load error for: {json}"),
			Err(e) => e,
		}
	}

	/// A placement's blend mode round-trips, an absent one is `normal`, and a
	/// bad one is a load error rather than a silent default.
	#[test]
	fn a_scenery_blend_mode_round_trips() {
		let root = assets_root();
		let spots = r#","scenery":[{"pack":"GREEN","piece":"mountain-1","x":1,"y":2},
			{"pack":"GREEN","piece":"mountain-2","x":3,"y":4,"blend":"darker"}]"#;
		let p = Project::from_str(&base(r#"[["",""]]"#, spots), &root).expect("loads");
		assert_eq!(p.scenery[0].blend, crate::scenery::SceneryBlend::Normal, "absent means normal");
		assert_eq!(p.scenery[1].blend, crate::scenery::SceneryBlend::Darker);
		let text = p.save_string();
		assert!(text.contains(r#""blend": "darker""#), "the mode is written back: {text}");
		assert_eq!(text.matches("\"blend\"").count(), 1, "...and a normal placement writes nothing");
		let back = Project::from_str(&text, &root).expect("re-loads");
		assert_eq!(back.scenery[1].blend, crate::scenery::SceneryBlend::Darker);

		let bad = base(r#"[["",""]]"#, r#","scenery":[{"pack":"G","piece":"p","x":0,"y":0,"blend":"lighter"}]"#);
		assert!(err(bad).contains("unknown blend"), "a typo is a load error");
	}

	#[test]
	fn legacy_version_other_than_1_is_rejected() {
		let json = base(r#"[["",""]]"#, "").replace(r#""version":"1""#, r#""version":"7""#);
		assert!(err(json).contains("unsupported legacy project version '7'"));
	}

	/// A bare-number-in-a-string `players` (neither a label nor a JSON number)
	/// still parses, clamped into 2..=4.
	#[test]
	fn players_accepts_stringified_numbers_with_clamping() {
		let root = assets_root();
		let three = Project::from_str(&base(r#"[["",""]]"#, r#","players":"3""#), &root).unwrap();
		assert_eq!(three.players, Some(3), "a stringified count parses");
		let nine = Project::from_str(&base(r#"[["",""]]"#, r#","players":"9""#), &root).unwrap();
		assert_eq!(nine.players, Some(4), "out-of-range counts clamp to 4");
		let junk = Project::from_str(&base(r#"[["",""]]"#, r#","players":"lots""#), &root).unwrap();
		assert_eq!(junk.players, None, "an unparseable string is treated as unset");
	}

	#[test]
	fn tilepass_values_over_3_are_rejected() {
		let e = err(base(r#"[["",""]]"#, r#","tilepass":{"GLa000":9}"#));
		assert!(e.contains("tilepass GLa000: value out of range"), "{e}");
	}

	#[test]
	fn unknown_cell_ids_are_rejected_with_the_cell_position() {
		let e = err(base(r#"[["","ZZZ999"]]"#, ""));
		assert!(e.contains("cell 1,0") && e.contains("unknown tile id 'ZZZ999'"), "{e}");
	}

	#[test]
	fn dense_pass_rows_reject_bad_cell_characters() {
		let e = err(base(r#"[["",""]]"#, r#","pass":["4-"]"#));
		assert!(e.contains("pass 0,0: bad cell '4'"), "digits above 3 are invalid: {e}");
		let e = err(base(r#"[["",""]]"#, r#","pass":["-x"]"#));
		assert!(e.contains("pass 1,0: bad cell 'x'"), "{e}");
	}

	#[test]
	fn pass_block_must_be_rows_or_an_xy_object() {
		let e = err(base(r#"[["",""]]"#, r#","pass":"nope""#));
		assert!(e.contains("'pass' must be an array of rows or an x,y object"), "{e}");
	}

	#[test]
	fn unit_lines_must_have_four_fields() {
		let e = err(base(r#"[["",""]]"#, r#","units":["TANK 1 0"]"#));
		assert!(e.contains(r#"want "TAG x y team""#), "{e}");
		assert!(e.contains("TANK 1 0"), "the offending line is quoted: {e}");
	}

	#[test]
	fn an_object_team_outside_the_slots_is_rejected() {
		// The `objects` block gets the same bound the legacy `units` lines have:
		// downstream clamps instead of checking, so this is the only gate.
		let e = err(base(r#"[["",""]]"#, r#","objects":[{"t":"TANK","x":0,"y":0,"team":200}]"#));
		assert!(e.contains("team 200 outside 0..=4"), "{e}");
		assert!(
			Project::from_str(
				&base(r#"[["",""]]"#, r#","objects":[{"t":"TANK","x":0,"y":0,"team":4}]"#),
				&assets_root()
			)
			.is_ok(),
			"the alien slot stays legal"
		);
	}
}
