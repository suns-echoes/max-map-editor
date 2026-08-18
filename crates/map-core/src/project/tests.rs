use super::*;

fn assets_root() -> std::path::PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets/tilepacks")
}

/// A GREEN project with one object placed, plus the piece it resolves to.
fn with_scenery(x: i32, y: i32) -> (Project, String) {
	let mut p = Project::new(8, 8, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	let piece = p.scenery_packs.iter().find(|s| s.pack == "GREEN").expect("GREEN scenery loads").pieces[0].id.clone();
	p.place_scenery(ScenerySpot { pack: "GREEN".into(), piece: piece.clone(), x, y, blend: Default::default() });
	(p, piece)
}

/// `Project::new` and a reloaded project both resolve their placements, and
/// place / move / remove each undo and redo as one unit.
#[test]
fn scenery_places_moves_and_removes_undoably() {
	let (mut p, piece) = with_scenery(64, 64);
	assert_eq!(p.scenery.len(), 1);
	assert!(p.scenery_piece(&p.scenery[0]).is_some(), "the placement resolves to a library piece");

	assert!(p.move_scenery_to(0, 128, 96));
	assert_eq!((p.scenery[0].x, p.scenery[0].y), (128, 96));
	assert!(!p.move_scenery_to(0, 128, 96), "a no-op move journals nothing");
	assert!(!p.move_scenery_to(9, 0, 0), "an out-of-range index journals nothing");

	assert!(p.undo(), "the move undoes");
	assert_eq!((p.scenery[0].x, p.scenery[0].y), (64, 64));
	assert!(p.undo(), "the placement undoes");
	assert!(p.scenery.is_empty());
	assert!(p.redo() && p.redo(), "and both redo");
	assert_eq!((p.scenery[0].x, p.scenery[0].y), (128, 96));

	assert!(p.remove_scenery(0));
	assert!(p.scenery.is_empty());
	assert!(!p.remove_scenery(0), "removing nothing journals nothing");
	assert!(p.undo(), "the removal undoes");
	assert_eq!(p.scenery.len(), 1);
	assert_eq!(p.scenery[0].piece, piece);
}

/// A placement round-trips through the project file, keeps its exact pixel
/// position, and re-resolves against the libraries on load. A scenery-free
/// project writes no block at all.
#[test]
fn scenery_round_trips_through_the_project_file() {
	let (p, piece) = with_scenery(-13, 250);
	let text = p.save_string();
	assert!(text.contains("\"scenery\""), "the block is written");
	let back = Project::from_str(&text, &assets_root()).expect("reloads");
	assert_eq!(back.scenery.len(), 1);
	assert_eq!(back.scenery[0].pack, "GREEN");
	assert_eq!(back.scenery[0].piece, piece);
	assert_eq!((back.scenery[0].x, back.scenery[0].y), (-13, 250), "a negative origin survives");
	assert!(back.scenery_piece(&back.scenery[0]).is_some(), "and re-resolves after the round-trip");

	let plain = Project::new(4, 4, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	assert!(!plain.save_string().contains("\"scenery\""), "a scenery-free project writes no block");
}

/// A placement naming a piece no library has is inert but not lost - it
/// draws nothing, blocks nothing, and is written back verbatim, so the map
/// survives a checkout without the baked assets.
#[test]
fn an_unresolved_placement_is_inert_and_preserved() {
	let mut p = Project::new(4, 4, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	p.place_scenery(ScenerySpot {
		pack: "GREEN".into(),
		piece: "no-such-piece".into(),
		x: 0,
		y: 0,
		blend: Default::default(),
	});
	assert!(p.scenery_piece(&p.scenery[0]).is_none());
	assert_eq!(p.scenery_at(0, 0), None, "it picks up nothing");
	assert_eq!(p.scenery_pass_at(0, 0), None, "and blocks nothing");
	let back = Project::from_str(&p.save_string(), &assets_root()).expect("reloads");
	assert_eq!(back.scenery, p.scenery, "the placement is written back verbatim");
}

/// Scenery reaches the exported pixels and the exported pass, and the cell
/// it covers stops matching the plain ground tile it started as.
#[test]
fn scenery_reaches_the_bake() {
	// Land under the object, so the composed cell has something to cover.
	let mut p = Project::new(8, 8, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	let green = p.packs.iter().position(|k| k.name == "GREEN").unwrap() as u8;
	let land = p.packs[green as usize].index_of["GLa000"];
	let edits: Vec<_> = (0..8u16)
		.flat_map(|y| {
			(0..8u16).map(move |x| {
				(x, y, LAYER_GROUND, Some(TileRef { pack: green, tile: land, transform: Transform::default() }))
			})
		})
		.collect();
	p.place_many(&edits);

	// The biggest GREEN piece, dropped so it covers the middle of the map.
	let piece = {
		let lib = p.scenery_packs.iter().find(|s| s.pack == "GREEN").unwrap();
		lib.pieces.iter().max_by_key(|p| p.sprite.covered()).unwrap().id.clone()
	};
	let before = crate::bake::bake(&p).expect("bakes without scenery");
	p.place_scenery(ScenerySpot { pack: "GREEN".into(), piece, x: 128, y: 128, blend: Default::default() });
	let after = crate::bake::bake(&p).expect("bakes with scenery");

	assert_ne!(before.tiles, after.tiles, "the exported pixels changed");
	assert!(after.tile_count > before.tile_count, "covered cells mint their own tiles");
	// Some cell under the object now reads blocked, and `pass_at` (the
	// overlay) agrees with the pass the export wrote.
	let blocked = (0..8u16)
		.flat_map(|y| (0..8u16).map(move |x| (x, y)))
		.filter(|&(x, y)| p.scenery_pass_at(x, y).is_some())
		.collect::<Vec<_>>();
	assert!(!blocked.is_empty(), "the object blocks at least one cell");
	for (x, y) in blocked {
		let cell = y as usize * 8 + x as usize;
		assert_eq!(
			after.pass_table[after.bigmap[cell] as usize],
			p.pass_at(x, y).unwrap(),
			"cell ({x},{y}): the exported pass matches the overlay's"
		);
	}
}

/// The pass rule keys on *body* coverage: a placement whose shadow alone
/// falls on a cell leaves it walkable, because a shadow is cast on ground
/// the object does not stand on.
#[test]
fn a_shadow_alone_does_not_block_a_cell() {
	let (p, _) = with_scenery(0, 0);
	let spot = &p.scenery[0];
	let piece = p.scenery_piece(spot).unwrap();
	let (ox, oy) = piece.sprite_origin(spot);
	// A cell whose pixels the piece only shades - if the art has one.
	let shaded_only = (0..8u16).flat_map(|y| (0..8u16).map(move |x| (x, y))).find(|&(x, y)| {
		let (cx, cy) = (x as i32 * 64, y as i32 * 64);
		let mut body = 0;
		let mut shade = 0;
		for py in cy..cy + 64 {
			for px in cx..cx + 64 {
				let (b, s) = piece.texel(px - ox, py - oy);
				body += usize::from(b != 0);
				shade += usize::from(s != 0);
			}
		}
		shade > 0 && body < SCENERY_PASS_COVERAGE
	});
	if let Some((x, y)) = shaded_only {
		assert_eq!(p.scenery_pass_at(x, y), None, "cell ({x},{y}) is shaded, not stood on");
	}
}

#[test]
fn render_dirty_tracks_only_the_edited_regions() {
	let mut p = Project::new(8, 6, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	let green = 1u8; // pack 0 = WATER, 1 = GREEN
	let tile = TileRef { pack: green, tile: 0, transform: Transform::default() };

	// A freshly built document has nothing pending (its cells were filled by
	// the constructor, not through an edit).
	assert_eq!(p.take_render_dirty(), RenderDirty::default());

	// A tile edit dirties that cell in *both* textures (the cell stack and its
	// derived pass value), and `take` drains it.
	assert!(p.place(2, 3, LAYER_GROUND, Some(tile)));
	assert_eq!(p.take_render_dirty(), RenderDirty { cells: Some((2, 3, 2, 3)), pass: Some((2, 3, 2, 3)) });
	assert_eq!(p.take_render_dirty(), RenderDirty::default(), "take drained it");

	// Several tile edits union into one bbox.
	assert!(p.place(0, 0, LAYER_GROUND, Some(tile)));
	assert!(p.place(5, 1, LAYER_GROUND, Some(tile)));
	assert_eq!(p.take_render_dirty().cells, Some((0, 0, 5, 1)), "the cell bbox is the union of edits");

	// A per-cell pass override dirties only the pass texture, not the cells.
	assert!(p.set_pass_override(4, 4, Some(3)));
	assert_eq!(p.take_render_dirty(), RenderDirty { cells: None, pass: Some((4, 4, 4, 4)) });

	// A per-tile pass retint touches every cell using that tile, so it dirties
	// the full pass extent (GREEN ships a pass table, so the edit lands).
	assert!(p.set_tile_pass_at(2, 3, 3), "GREEN has a pass table");
	assert_eq!(p.take_render_dirty(), RenderDirty { cells: None, pass: Some((0, 0, 7, 5)) });

	// Undo re-dirties the region it reverts (here, the full-map pass retint).
	assert!(p.undo());
	assert_eq!(p.take_render_dirty().pass, Some((0, 0, 7, 5)), "undo re-uploads the reverted region");

	// `clear_render_dirty` drops anything pending (the caller rebuilt the
	// renderer from scratch).
	assert!(p.place(1, 1, LAYER_GROUND, Some(tile)));
	p.clear_render_dirty();
	assert_eq!(p.take_render_dirty(), RenderDirty::default(), "clear drops pending dirty");
}

#[test]
fn render_dirty_batches_a_stroke_into_its_footprint() {
	let mut p = Project::new(8, 6, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	let tile = TileRef { pack: 1, tile: 0, transform: Transform::default() };
	// A stroke (one undo unit) still dirties every cell it paints, live.
	p.begin_stroke();
	assert!(p.place(1, 1, LAYER_GROUND, Some(tile)));
	assert!(p.place(3, 2, LAYER_GROUND, Some(tile)));
	p.end_stroke();
	assert_eq!(p.take_render_dirty().cells, Some((1, 1, 3, 2)), "the stroke's whole footprint is dirty");
	// Rolling back a stroke re-dirties the cells it reverts.
	p.begin_stroke();
	assert!(p.place(6, 4, LAYER_GROUND, Some(tile)));
	let _ = p.take_render_dirty(); // drain the live edit
	assert!(p.rollback_stroke());
	assert_eq!(p.take_render_dirty().cells, Some((6, 4, 6, 4)), "rollback re-uploads the reverted cell");
}

#[test]
fn undo_history_labels_steps_and_seq() {
	let mut p = Project::new(8, 6, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	let tile = TileRef { pack: 1, tile: 0, transform: Transform::default() };
	let seq0 = p.undo_seq();
	// An app-labelled edit and an unlabelled one (which derives its label).
	p.label_next_undo("Fill");
	assert!(p.place(0, 0, LAYER_GROUND, Some(tile)));
	assert!(p.place(1, 0, LAYER_GROUND, Some(tile))); // no label → derived
	assert_ne!(p.undo_seq(), seq0, "committing patches bumps the sequence");
	// Newest first: the derived one, then the labelled one.
	assert_eq!(p.undo_labels(10), vec!["Paint 1 cell".to_string(), "Fill".to_string()]);
	// The label survives undo/redo.
	assert!(p.undo());
	assert!(p.redo());
	assert_eq!(p.undo_labels(1), vec!["Paint 1 cell".to_string()]);
	// `undo_steps` jumps back multiple entries at once.
	assert_eq!(p.undo_steps(5), 2, "only two patches exist to undo");
	assert!(p.undo_labels(10).is_empty(), "history emptied");
	assert_eq!(p.cell(0, 0).unwrap()[LAYER_GROUND], None, "both edits reverted");
}

#[test]
fn delete_tile_shifts_indices_and_refuses_in_use() {
	let mut p = Project::new(8, 6, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	let green = 1u8; // pack 0 = WATER, 1 = GREEN
	let before = p.packs[green as usize].tile_count();
	let third_id = p.packs[green as usize].ids[2].clone();
	// Paint tile 5 onto a cell, then deleting tile 2 must shift it to 4.
	let t5 = TileRef { pack: green, tile: 5, transform: Transform::default() };
	assert!(p.place_many(&[(0, 0, LAYER_GROUND, Some(t5))]));
	// In-use tile can't be deleted.
	assert!(p.delete_tile(green, 5).is_err(), "painted tile is protected");
	// Deleting an earlier, unused tile shifts the painted ref down by one.
	p.delete_tile(green, 2).unwrap();
	assert_eq!(p.packs[green as usize].tile_count(), before - 1);
	assert!(!p.packs[green as usize].index_of.contains_key(&third_id), "deleted id is gone");
	assert_eq!(p.cell(0, 0).unwrap()[LAYER_GROUND].unwrap().tile, 4, "painted ref shifted 5->4");
}

#[test]
fn load_palette_touches_only_editable_slots_in_one_stroke() {
	let mut p = Project::new(4, 4, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	let before = p.palette.clone();
	// A full 256-colour palette of solid red.
	let red = vec![[0xffu8, 0, 0]; 256].concat();
	let n = p.load_palette(&red).unwrap();
	assert!(n > 0 && n <= 96, "only the 96 dynamic slots can change");
	// Dynamic slot 64 took the load; static slot 0 + 200 are untouched.
	assert_eq!(&p.palette[64 * 3..64 * 3 + 3], &[0xff, 0, 0]);
	assert_eq!(&p.palette[0..3], &before[0..3]);
	assert_eq!(&p.palette[200 * 3..200 * 3 + 3], &before[200 * 3..200 * 3 + 3]);
	// One undo unit reverts the whole load.
	p.undo();
	assert_eq!(p.palette, before);
}

#[test]
fn variants_load_and_random_stays_in_family() {
	let p = Project::new(4, 4, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	// GSa ships eight look-variants (tiles.variants.json).
	let (tile, _) = p.resolve_ref("GSa000").unwrap();
	let group = p.packs[tile.pack as usize].variants_of(tile.tile).to_vec();
	assert!(group.len() >= 2, "GSa is a multi-variant family");
	let mut rng = Rng::new(7);
	for _ in 0..32 {
		let v = p.random_variant(tile, &mut rng);
		assert_eq!(v.pack, tile.pack, "same pack");
		assert_eq!(v.transform, tile.transform, "transform preserved");
		assert!(group.contains(&v.tile), "variant stays within the family");
	}
}

#[test]
fn flood_fill_covers_the_connected_region() {
	let mut p = Project::new(4, 4, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	let (tile, layer) = p.resolve_ref("GSa000").unwrap();
	assert_eq!(layer, LAYER_GROUND);
	// Ground starts empty everywhere → the fill floods all 16 cells.
	assert!(p.cell(0, 0).unwrap()[LAYER_GROUND].is_none());
	let mut rng = Rng::new(0);
	assert!(p.fill(0, 0, tile, layer, false, &mut rng));
	for y in 0..4 {
		for x in 0..4 {
			assert_eq!(p.cell(x, y).unwrap()[LAYER_GROUND], Some(tile));
		}
	}
	// Re-filling the same uniform tile changes nothing.
	assert!(!p.fill(0, 0, tile, layer, false, &mut rng));
	// One undo reverts the whole fill (it was a single transaction).
	assert!(p.undo());
	assert!(p.cell(2, 2).unwrap()[LAYER_GROUND].is_none());
}

/// `from_wrl` is lossless: every cell composes back to the source tile,
/// bigmap indexing is honoured, and per-cell pass comes from the WRL.
#[test]
fn from_wrl_composes_back_to_source_pixels() {
	// 2×1 map, two distinct tiles; cell 0 → tile 1, cell 1 → tile 0.
	let mut tiles = vec![0u8; 2 * TILE_DATA_SIZE];
	tiles[..TILE_DATA_SIZE].fill(7);
	tiles[TILE_DATA_SIZE..].fill(42);
	let wrl = WrlFile {
		header: vec![0; 5],
		width: 2,
		height: 1,
		minimap: vec![42, 7],
		bigmap: vec![1, 0],
		tile_count: 2,
		tiles: tiles.clone(),
		palette: vec![0; 768],
		pass_table: vec![1, 2],
	};

	let p = Project::from_wrl(&wrl, "TEST");
	assert_eq!((p.width, p.height), (2, 1));
	// Cell 0 holds tile 1 (the 42s), cell 1 holds tile 0 (the 7s).
	assert_eq!(&p.compose_cell(0, 0)[..], &tiles[TILE_DATA_SIZE..]);
	assert_eq!(&p.compose_cell(1, 0)[..], &tiles[..TILE_DATA_SIZE]);
	// Pass derives from the synthetic pack: pass_table[bigmap[cell]].
	assert_eq!(p.pass_at(0, 0), Some(2)); // tile 1
	assert_eq!(p.pass_at(1, 0), Some(1)); // tile 0
	// The map decomposes by passability: cell 0 (tile 1, pass 2 = shore)
	// lands on the ground layer; cell 1 (tile 0, pass 1 = water) on the base.
	let c0 = p.cell(0, 0).unwrap();
	assert_eq!(c0[LAYER_GROUND].map(|t| t.tile), Some(1));
	assert!(c0[LAYER_WATER].is_none());
	let c1 = p.cell(1, 0).unwrap();
	assert_eq!(c1[LAYER_WATER].map(|t| t.tile), Some(0));
	assert!(c1[LAYER_GROUND].is_none());
	// Tile ids follow the XXXY### scheme (name TEST → consonants TST).
	assert_eq!(p.packs[0].ids[1], "TSTS000", "tile 1 is shore #0");
	assert_eq!(p.packs[0].ids[0], "TSTW000", "tile 0 is water #0");
	// A fresh import is clean.
	assert!(!p.dirty());
}

/// An imported WRL saved as a project dumps its synthetic pack to a
/// sibling folder and reloads from it (the persistence path the user
/// asked for): tiles, pass, and palette survive the round trip.
#[test]
fn wrl_import_dumps_and_reloads_via_sibling_pack() {
	let mut tiles = vec![0u8; 2 * TILE_DATA_SIZE];
	tiles[..TILE_DATA_SIZE].fill(7);
	tiles[TILE_DATA_SIZE..].fill(42);
	let wrl = WrlFile {
		header: vec![0; 5],
		width: 2,
		height: 1,
		minimap: vec![42, 7],
		bigmap: vec![1, 0],
		tile_count: 2,
		tiles,
		palette: vec![5; 768],
		pass_table: vec![2, 3],
	};
	let project = Project::from_wrl(&wrl, "WRLTEST");

	// Dump the synthetic pack next to a would-be `.json`.
	let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/maptest-wrl-dump");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	project.packs[0].dump(&dir.join("WRLTEST")).unwrap();
	let json = project.save_string();

	// Reload with an assets root that lacks the pack - only the sibling
	// fallback (`dir`) has it.
	let empty = dir.join("no-assets");
	std::fs::create_dir_all(&empty).unwrap();
	let reloaded = Project::from_str_in(&json, &empty, Some(&dir)).unwrap();

	assert_eq!((reloaded.width, reloaded.height), (2, 1));
	assert_eq!(reloaded.compose_cell(0, 0), project.compose_cell(0, 0));
	assert_eq!(reloaded.compose_cell(1, 0), project.compose_cell(1, 0));
	assert_eq!(reloaded.pass_at(0, 0), Some(3)); // cell 0 → tile 1
	assert_eq!(reloaded.pass_at(1, 0), Some(2)); // cell 1 → tile 0
	assert_eq!(reloaded.palette, project.palette);

	std::fs::remove_dir_all(&dir).ok();
}

/// A save-editor session round-trips: an opened `.DTA` embedded in a project
/// survives Save (base64 in the `.json`) and reload byte-for-byte, and
/// re-decodes to the same unit inventory. Gated on the V71 fixture (local
/// only - see `testdata/saves/README.md`).
#[test]
fn embedded_save_survives_project_round_trip() {
	let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/saves");
	let wrl_path = fixtures.join("GREEN_3-50x50.WRL");
	let save_path = fixtures.join("save11-green3-50x50.dta");
	if !wrl_path.is_file() || !save_path.is_file() {
		eprintln!("skipping embedded_save_survives_project_round_trip: fixtures absent");
		return;
	}

	let wrl = max_assets::wrl::read_wrl_file(&wrl_path).unwrap();
	let mut project = Project::from_wrl(&wrl, "GREEN3SAVE");
	let save_bytes = std::fs::read(&save_path).unwrap();
	project.attach_save(save_bytes.clone()).unwrap();

	// The fixture is a populated GREEN_3 save (>1000 units per SAVE-EDITOR.md
	// S0.3b); the decoded inventory is derived purely from `raw` + dims, so
	// byte-exact retention (below) is the real persistence invariant.
	let embedded = project.save.as_ref().unwrap();
	assert_eq!(embedded.raw, save_bytes, "raw bytes retained verbatim");
	let units = embedded.file.units().count();
	let seed_cargo = embedded.file.cargo_map.clone(); // the pristine resource seed (S5)
	assert!(units > 1000, "fixture decodes to a populated inventory (got {units})");
	// attach_save seeds the editable object model (S2.1): every on-map,
	// non-particle unit, with its gameplay props.
	assert!(!project.objects.is_empty(), "objects seeded from the save");
	assert!(project.objects.iter().any(|o| o.props.source_id.is_some()), "seeded objects carry their source id");
	// object_base_values seeds max stats from the save on the fly for an
	// unedited unit (no override yet): some seeded unit resolves a real max-HP
	// cap (not None, not a zero placeholder), proving the seed branch (S4.5).
	assert!(
		(0..project.objects.len()).filter_map(|i| project.object_base_values(i)).any(|v| v.hits > 0),
		"some seeded unit resolves a real max-HP cap from the save",
	);

	// Dump the synthetic pack next to a would-be `.json`, then reload from it.
	let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/maptest-embedded-save");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	project.packs[0].dump(&dir.join("GREEN3SAVE")).unwrap();
	let json = project.save_string();

	let empty = dir.join("no-assets");
	std::fs::create_dir_all(&empty).unwrap();
	let reloaded = Project::from_str_in(&json, &empty, Some(&dir)).unwrap();

	let got = reloaded.save.as_ref().expect("embedded save survives reload");
	assert_eq!(got.raw, save_bytes, "raw bytes identical after Save + reload");
	assert_eq!(got.file.units().count(), units, "inventory identical after reload");
	// The seeded object model persists via the `"objects"` block and reloads
	// identically (props included) - not re-seeded from the save.
	assert_eq!(reloaded.objects, project.objects, "objects identical after reload");
	// The resource map is seeded from the save and matches its cargo map (S5).
	assert_eq!(reloaded.cargo_map(), &seed_cargo[..], "cargo map seeded from the save");

	// A resource edit survives Save + reload via the compact `"resources"` diff.
	let mut edited = project;
	let painted =
		max_assets::save::cargo_compose(edited.cargo_at(1, 1).unwrap(), Some(max_assets::save::CargoMaterial::Raw), 15);
	assert!(edited.set_cargo(1, 1, painted));
	let json2 = edited.save_string();
	assert!(json2.contains("\"resources\""), "the resource diff is persisted");
	let reloaded2 = Project::from_str_in(&json2, &empty, Some(&dir)).unwrap();
	assert_eq!(reloaded2.cargo_at(1, 1), Some(painted), "the resource edit reloads");
	// Only the one changed cell is in the diff (the rest re-seed from the save).
	let changed = reloaded2.cargo_map().iter().zip(&seed_cargo).filter(|(a, b)| a != b).count();
	assert_eq!(changed, 1, "exactly the edited cell diverges from the seed");

	std::fs::remove_dir_all(&dir).ok();
}

/// Edit Save Data (S7.2): an applied settings block reaches the export
/// bytes, rebases the raw anchor without tripping the S6.6 guard, and
/// undo/redo swap it byte-exactly. Gated on the V71 fixture (local only).
#[test]
fn save_settings_edit_is_undoable_and_rebases_the_anchor() {
	let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/saves");
	let wrl_path = fixtures.join("GREEN_3-50x50.WRL");
	let save_path = fixtures.join("save11-green3-50x50.dta");
	if !wrl_path.is_file() || !save_path.is_file() {
		eprintln!("skipping save_settings_edit_is_undoable_and_rebases_the_anchor: fixtures absent");
		return;
	}

	let wrl = max_assets::wrl::read_wrl_file(&wrl_path).unwrap();
	let mut project = Project::from_wrl(&wrl, "GREEN3SAVE");
	assert!(project.save_settings().is_none(), "no settings before a save is attached");
	let mut edited = SaveSettings::extract(&read_save_bytes(&std::fs::read(&save_path).unwrap(), (50, 50)).unwrap());
	assert!(project.apply_save_settings(&edited).is_err(), "applying with no save attached refuses");

	let original = std::fs::read(&save_path).unwrap();
	project.attach_save(original.clone()).unwrap();
	let base = project.save_settings().expect("settings of the attached save");
	assert_eq!(base, edited, "extract agrees with a direct decode");

	edited.save_name = "SETTINGS".into();
	edited.options.start_gold = 999;
	edited.teams[0].team_points += 7;
	edited.team_gold[1] += 100;
	project.label_next_undo("Edit Save Data");
	project.apply_save_settings(&edited).unwrap();

	assert_ne!(project.save.as_ref().unwrap().raw, original, "the anchor was rebased");
	assert!(project.save_exports_losslessly(), "the rebased anchor keeps the write-safety guard green");
	assert_eq!(project.save_settings().unwrap(), edited, "the live settings carry the edit");
	assert_eq!(project.undo_labels(1), vec!["Edit Save Data".to_string()]);

	// The export path emits the edit.
	let out = project.export_save().unwrap();
	let decoded = read_save_bytes(&out, (50, 50)).unwrap();
	assert_eq!(decoded.header.save_name, "SETTINGS");
	assert_eq!(decoded.header.options.start_gold, 999);
	assert_eq!(decoded.teams[0].team_points, edited.teams[0].team_points);
	assert_eq!(decoded.team_units[1].gold, edited.team_gold[1]);

	// Undo restores the original bytes exactly; redo re-applies the edit.
	assert!(project.undo());
	assert_eq!(project.save.as_ref().unwrap().raw, original, "undo restores the original anchor byte-for-byte");
	assert_eq!(project.save_settings().unwrap(), base);
	assert!(project.redo());
	assert_eq!(project.save_settings().unwrap(), edited);

	// Applying the current settings again is a no-op: no new undo entry.
	let depth = project.undo_labels(10).len();
	project.apply_save_settings(&edited).unwrap();
	assert_eq!(project.undo_labels(10).len(), depth, "an unchanged block records no patch");
}

/// The tail's own object references survive a graph-structural export
/// (S6.2). A save's message logs and AI state name units by on-disk index,
/// and adding a unit, deleting one, or installing a per-unit stat override
/// all renumber that index space — so every one of them has to move the tail
/// with it. Checked by what those references *resolve to*: the same unit ids
/// before and after, whatever their indices became.
#[test]
fn graph_edits_keep_the_tails_references_on_their_own_units() {
	let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/saves");
	let wrl_path = fixtures.join("GREEN_3-50x50.WRL");
	let save_path = fixtures.join("save11-green3-50x50.dta");
	if !wrl_path.is_file() || !save_path.is_file() {
		eprintln!("skipping graph_edits_keep_the_tails_references_on_their_own_units: fixtures absent");
		return;
	}
	let wrl = max_assets::wrl::read_wrl_file(&wrl_path).unwrap();
	let dims = (50u16, 50u16);

	let open = || {
		let mut p = Project::from_wrl(&wrl, "GREEN3SAVE");
		p.attach_save(std::fs::read(&save_path).unwrap()).unwrap();
		p
	};
	let named = |p: &Project| {
		max_assets::save::referenced_units(&p.save.as_ref().unwrap().file).expect("the fixture's tail walks")
	};

	let before = named(&open());
	assert!(before.iter().any(Option::is_some), "the fixture's tail really does name units");

	// (1) Add a unit. It is appended to the model but first-seen partway
	// through the serializer's walk, so every object after it is renumbered.
	let mut p = open();
	let template = p.objects.first().cloned().expect("the save seeded objects");
	p.place_object(MapObject { x: 3, y: 3, props: ObjectProps::default(), ..template.clone() });
	let out = p.export_save().expect("export");
	let after = max_assets::save::referenced_units(&read_save_bytes(&out, dims).unwrap()).expect("walks");
	assert_eq!(after, before, "an added unit must not re-point the tail");

	// (2) Delete a seeded unit — its slot goes and everything above shifts
	// down, and any reference *to* it has to be let go of.
	let mut p = open();
	let victim = p.objects.iter().position(|o| o.props.source_id.is_some()).expect("a seeded unit");
	let (vx, vy) = (p.objects[victim].x, p.objects[victim].y);
	// The cell may hold a stack (a building on its slab); all of it goes.
	let victims: Vec<u16> =
		p.objects.iter().filter(|o| (o.x, o.y) == (vx, vy)).filter_map(|o| o.props.source_id).collect();
	p.remove_object_at(vx, vy);
	let out = p.export_save().expect("export");
	let after = max_assets::save::referenced_units(&read_save_bytes(&out, dims).unwrap()).expect("walks");
	// Every reference the deleted unit had is let go of - a log line keeps its
	// text and loses its unit, a spotted unit is dropped outright - and every
	// other reference still names exactly the unit it named before.
	let kept =
		|v: &[Option<u16>]| -> Vec<u16> { v.iter().flatten().copied().filter(|id| !victims.contains(id)).collect() };
	assert_eq!(kept(&after), kept(&before), "the survivors are untouched");
	assert!(!after.iter().flatten().any(|id| victims.contains(id)), "and nothing still names a deleted unit");

	// (3) A per-unit stat override inserts an inline `UnitValues` low in the
	// graph, shifting most of it.
	let mut p = open();
	let idx = p.objects.iter().position(|o| o.props.source_id.is_some()).expect("a seeded unit");
	let mut values = p.object_base_values(idx).expect("the save seeds max stats");
	values.hits += 5;
	let obj = p.objects[idx].clone();
	let mut props = obj.props.clone();
	props.base_values = Some(values);
	p.set_object_state(idx, obj.team, props);
	let out = p.export_save().expect("export");
	let after = max_assets::save::referenced_units(&read_save_bytes(&out, dims).unwrap()).expect("walks");
	assert_eq!(after, before, "a stat override must not re-point the tail either");
}

/// The S7.2 extension fields — team types and per-team master unit
/// upgrades — flow through [`Project::apply_save_settings`], reach the
/// export, and undo back, with the anchor staying consistent throughout
/// (the upgrades path may insert a fresh master object, so undo is
/// settings-exact, not necessarily byte-exact).
#[test]
fn team_type_and_upgrade_edits_flow_through_project_undo() {
	let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/saves");
	let wrl_path = fixtures.join("GREEN_3-50x50.WRL");
	let save_path = fixtures.join("save11-green3-50x50.dta");
	if !wrl_path.is_file() || !save_path.is_file() {
		eprintln!("skipping team_type_and_upgrade_edits_flow_through_project_undo: fixtures absent");
		return;
	}

	let wrl = max_assets::wrl::read_wrl_file(&wrl_path).unwrap();
	let mut project = Project::from_wrl(&wrl, "GREEN3SAVE");
	project.attach_save(std::fs::read(&save_path).unwrap()).unwrap();
	let base = project.save_settings().expect("settings of the attached save");

	let mut edited = base.clone();
	let slot = edited.team_types.iter().position(|&t| t != 0).expect("an active slot");
	edited.team_types[slot] = if edited.team_types[slot] == 4 { 1 } else { 4 };
	let ut = edited.team_upgrades[0].iter().position(Option::is_some).expect("table 0 holds current values");
	let mut vals = edited.team_upgrades[0][ut].unwrap();
	vals[0] += 3; // attack
	edited.team_upgrades[0][ut] = Some(vals);

	project.label_next_undo("Edit Save Data");
	project.apply_save_settings(&edited).unwrap();
	assert!(project.save_exports_losslessly(), "the rebased anchor keeps the write-safety guard green");
	assert_eq!(project.save_settings().unwrap(), edited, "the live settings carry both edits");

	let out = project.export_save().unwrap();
	let decoded = read_save_bytes(&out, (50, 50)).unwrap();
	assert_eq!(decoded.header.team_type[slot], edited.team_types[slot]);
	assert_eq!(SaveSettings::extract(&decoded).team_upgrades[0][ut], Some(vals));

	assert!(project.undo());
	assert_eq!(project.save_settings().unwrap(), base, "undo restores the settings block");
	assert!(project.save_exports_losslessly(), "the undone anchor stays consistent");
	assert!(project.redo());
	assert_eq!(project.save_settings().unwrap(), edited);
}

/// Export Save File (S6.1): a scalar prop edit + a resource edit round-trip
/// through [`Project::export_save`] into loadable `.DTA` bytes, an unedited
/// export is byte-identical to the original, and graph-touching edits are
/// reported (not silently dropped). Gated on the real SAVE10 autosave + its
/// pristine SNOW_1 world (both local-only).
#[test]
fn export_save_reflects_scalar_and_resource_edits() {
	let Some(home) = std::env::var_os("HOME") else { return };
	let save_path = Path::new(&home).join("MAX/SAVE10.DTA");
	let wrl_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals/SNOW_1.WRL");
	if !save_path.is_file() || !wrl_path.is_file() {
		eprintln!("skipping export_save_reflects_scalar_and_resource_edits: fixtures absent");
		return;
	}

	let wrl = max_assets::wrl::read_wrl_file(&wrl_path).unwrap();
	let mut project = Project::from_wrl(&wrl, "SNOW1SAVE");
	let raw = std::fs::read(&save_path).unwrap();
	project.attach_save(raw.clone()).unwrap();
	let dims = (project.width, project.height);

	// An unedited export reproduces the original file byte-for-byte, and there
	// is nothing to report.
	assert_eq!(project.export_save().unwrap(), raw, "unedited export == original .DTA bytes");
	assert!(project.unexported_edits().is_empty(), "no edits ⇒ empty report");

	// Edit a scalar prop (angle) of a seeded unit.
	let idx = project.objects.iter().position(|o| o.props.source_id.is_some()).expect("a seeded unit");
	let src_id = project.objects[idx].props.source_id.unwrap();
	let team = project.objects[idx].team;
	let mut props = project.objects[idx].props.clone();
	let new_angle = props.angle ^ 0x04;
	props.angle = new_angle;
	assert!(project.set_object_state(idx, team, props));

	// Edit a resource cell (S5) — proves the cargo map flushes into the export.
	let painted = max_assets::save::cargo_compose(
		project.cargo_at(2, 2).unwrap(),
		Some(max_assets::save::CargoMaterial::Gold),
		20,
	);
	assert!(project.set_cargo(2, 2, painted));

	// Export → re-decode → both edits are present, inventory unchanged.
	let bytes = project.export_save().unwrap();
	assert_ne!(bytes, raw, "an edited export differs from the original");
	let out = read_save_bytes(&bytes, dims).unwrap();
	let u = out.units().find(|u| u.id == src_id).expect("edited unit still present");
	assert_eq!(u.angle, new_angle, "exported save carries the angle edit");
	assert_eq!(out.cargo_map[2 * project.width as usize + 2], painted, "exported save carries the resource edit");
	assert_eq!(out.units().count(), project.save.as_ref().unwrap().file.units().count(), "inventory preserved");

	// A placement of a type NOT in the save (no body template) is the one edit
	// export can't represent — reported so it isn't dropped silently. TANK (0x33)
	// is absent from SAVE10.
	assert!(max_assets::save::unit_type_id("TANK").is_some());
	project.place_object(MapObject { unit_type: 0x33, x: 5, y: 5, team: 0, props: ObjectProps::default() });
	let report = project.unexported_edits();
	assert_eq!(report.added, ["TANK at 5,5"], "a placement with no same-type template is reported by name");

	// Write-safety guard (S6.6): SAVE10 round-trips losslessly, so export is
	// allowed; corrupt the retained raw and the guard refuses instead of
	// emitting a save that wouldn't match its byte-exact anchor.
	assert!(project.save_exports_losslessly(), "SAVE10 round-trips (S0.4)");
	project.save.as_mut().unwrap().raw.push(0xFF);
	assert!(!project.save_exports_losslessly(), "a diverged raw fails the guard");
	assert!(project.export_save().is_err(), "a non-lossless save is refused");
}

/// Export Save File (S6.2 moves): moving a seeded unit re-keys `Hash_MapHash`
/// — the exported save re-decodes with the unit at its new grid, present in the
/// new cell's bucket and absent from the old, and a move no longer counts as an
/// un-exported edit. Gated on SAVE10 + its pristine SNOW_1 world.
#[test]
fn export_save_moves_a_unit_and_rekeys_the_map_hash() {
	use max_assets::save::SaveObject;
	let Some(home) = std::env::var_os("HOME") else { return };
	let save_path = Path::new(&home).join("MAX/SAVE10.DTA");
	let wrl_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals/SNOW_1.WRL");
	if !save_path.is_file() || !wrl_path.is_file() {
		eprintln!("skipping export_save_moves_a_unit_and_rekeys_the_map_hash: fixtures absent");
		return;
	}

	let wrl = max_assets::wrl::read_wrl_file(&wrl_path).unwrap();
	let mut project = Project::from_wrl(&wrl, "SNOW1SAVE");
	project.attach_save(std::fs::read(&save_path).unwrap()).unwrap();
	let dims = (project.width, project.height);

	// Move the (mobile) engineer to a far, empty cell.
	let idx = project
		.objects
		.iter()
		.position(|o| max_assets::save::unit_type_name(o.unit_type) == Some("ENGINEER"))
		.expect("SAVE10 has an engineer");
	let id = project.objects[idx].props.source_id.unwrap();
	let (ox, oy) = (project.objects[idx].x, project.objects[idx].y);
	let (nx, ny) = (60u16, 60u16);
	assert_ne!((ox, oy), (nx, ny));
	assert!(project.move_object_to(idx, nx, ny));
	assert!(project.unexported_edits().is_empty(), "a move is fully exported (not reported)");

	let bytes = project.export_save().unwrap();
	let out = read_save_bytes(&bytes, dims).unwrap();
	let u = out.units().find(|u| u.id == id).expect("moved unit present");
	assert_eq!((u.grid_x, u.grid_y), (nx as i16, ny as i16), "grid moved");
	assert_eq!((u.pixel_x, u.pixel_y), (nx * 64 + 32, ny * 64 + 32), "pixel re-centred (mobile unit)");

	// The map hash lists the unit at the new cell and nowhere else.
	let slot = out.objects.iter().position(|o| matches!(o, SaveObject::Unit(uu) if uu.id == id)).expect("unit slot");
	let cells: Vec<(u16, u16)> =
		out.map_hash.buckets.iter().flatten().filter(|c| c.units.contains(&slot)).map(|c| (c.x, c.y)).collect();
	assert_eq!(cells, vec![(nx, ny)], "unit is hashed at the new cell only");
}

/// Export Save File (S6.2 stat override): raising a unit's max HP (a per-unit
/// `base_values` override, S4.5) inserts an inline `UnitValues` into the graph
/// that the exported save re-decodes with the new cap, leaving the inventory
/// intact. Gated on SAVE10 + its pristine SNOW_1 world.
#[test]
fn export_save_applies_a_stat_override() {
	let Some(home) = std::env::var_os("HOME") else { return };
	let save_path = Path::new(&home).join("MAX/SAVE10.DTA");
	let wrl_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals/SNOW_1.WRL");
	if !save_path.is_file() || !wrl_path.is_file() {
		eprintln!("skipping export_save_applies_a_stat_override: fixtures absent");
		return;
	}

	let wrl = max_assets::wrl::read_wrl_file(&wrl_path).unwrap();
	let mut project = Project::from_wrl(&wrl, "SNOW1SAVE");
	project.attach_save(std::fs::read(&save_path).unwrap()).unwrap();
	let dims = (project.width, project.height);

	// A seeded unit that resolves a real max-HP cap; raise it by 50.
	let idx = (0..project.objects.len()).find(|&i| project.object_base_values(i).is_some()).expect("a unit with stats");
	let id = project.objects[idx].props.source_id.unwrap();
	let mut values = project.object_base_values(idx).unwrap();
	let new_hits = values.hits + 50;
	values.hits = new_hits;
	let team = project.objects[idx].team;
	let mut props = project.objects[idx].props.clone();
	props.base_values = Some(values);
	assert!(project.set_object_state(idx, team, props));
	assert!(project.unexported_edits().is_empty(), "a stat override is now fully exported");

	let out = read_save_bytes(&project.export_save().unwrap(), dims).unwrap();
	let u = out.units().find(|u| u.id == id).expect("unit present");
	let vals = out.values(u.base_values.expect("has base_values")).expect("resolves stats");
	assert_eq!(vals.hits, new_hits, "the exported save carries the raised max HP");
	assert_eq!(out.units().count(), project.save.as_ref().unwrap().file.units().count(), "inventory preserved",);
}

/// Export Save File (S6.2 delete): erasing units exports a save that re-decodes
/// without them. Deletes SAVE10's crowded start cell (19,19) — which holds the
/// cyclic SMLTAPE→ENGINEER→ADUMP cluster — exercising the hard graph case
/// end-to-end. Gated on SAVE10 + its pristine SNOW_1 world.
#[test]
fn export_save_deletes_units() {
	let Some(home) = std::env::var_os("HOME") else { return };
	let save_path = Path::new(&home).join("MAX/SAVE10.DTA");
	let wrl_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals/SNOW_1.WRL");
	if !save_path.is_file() || !wrl_path.is_file() {
		eprintln!("skipping export_save_deletes_units: fixtures absent");
		return;
	}

	let wrl = max_assets::wrl::read_wrl_file(&wrl_path).unwrap();
	let mut project = Project::from_wrl(&wrl, "SNOW1SAVE");
	project.attach_save(std::fs::read(&save_path).unwrap()).unwrap();
	let dims = (project.width, project.height);
	let before = project.save.as_ref().unwrap().file.units().count();

	// Erase everything on the engineer's start cell (a multi-unit cell).
	let idx = project
		.objects
		.iter()
		.position(|o| max_assets::save::unit_type_name(o.unit_type) == Some("ENGINEER"))
		.expect("SAVE10 has an engineer");
	let (x, y) = (project.objects[idx].x, project.objects[idx].y);
	let removed: Vec<u16> =
		project.objects.iter().filter(|o| (o.x, o.y) == (x, y)).filter_map(|o| o.props.source_id).collect();
	assert!(removed.len() >= 2, "the start cell holds several units");
	assert!(project.remove_object_at(x, y));
	assert!(project.unexported_edits().is_empty(), "deletions are fully exported");

	let out = read_save_bytes(&project.export_save().unwrap(), dims).unwrap();
	assert_eq!(out.units().count(), before - removed.len(), "exactly the deleted units are gone");
	for id in removed {
		assert!(out.units().all(|u| u.id != id), "deleted unit {id} absent");
	}
}

/// Export Save File (S6.2 add): placing a unit whose type is already in the save
/// exports a save that re-decodes with one more unit at the placed cell. Gated
/// on SAVE10 + its pristine SNOW_1 world.
#[test]
fn export_save_adds_a_placed_unit() {
	let Some(home) = std::env::var_os("HOME") else { return };
	let save_path = Path::new(&home).join("MAX/SAVE10.DTA");
	let wrl_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals/SNOW_1.WRL");
	if !save_path.is_file() || !wrl_path.is_file() {
		eprintln!("skipping export_save_adds_a_placed_unit: fixtures absent");
		return;
	}

	let wrl = max_assets::wrl::read_wrl_file(&wrl_path).unwrap();
	let mut project = Project::from_wrl(&wrl, "SNOW1SAVE");
	project.attach_save(std::fs::read(&save_path).unwrap()).unwrap();
	let dims = (project.width, project.height);
	let before = project.save.as_ref().unwrap().file.units().count();

	// Place another engineer (type present in the save) at an empty cell.
	let engineer = max_assets::save::unit_type_id("ENGINEER").unwrap();
	project.place_object(MapObject { unit_type: engineer, x: 45, y: 45, team: 0, props: ObjectProps::default() });
	assert!(project.unexported_edits().is_empty(), "the placement has a template, so it exports");

	let out = read_save_bytes(&project.export_save().unwrap(), dims).unwrap();
	assert_eq!(out.units().count(), before + 1, "one unit added");
	let placed: Vec<_> = out.units().filter(|u| u.unit_type == engineer && (u.grid_x, u.grid_y) == (45, 45)).collect();
	assert_eq!(placed.len(), 1, "the placed engineer is present at its cell");
	assert_eq!(placed[0].team, 0, "owned by the chosen team");
}

/// Export Save File complex pass (HANDOFF 2026-08-02 Finding 1): two placed
/// adjacent buildings — auto-connected like the app does on placement —
/// export sharing ONE engine-valid `Complex`, and the whole exported save
/// passes the complex-invariant check. Gated on SAVE10 + SNOW_1.
#[test]
fn export_save_gives_placed_buildings_a_valid_complex() {
	let Some(home) = std::env::var_os("HOME") else { return };
	let save_path = Path::new(&home).join("MAX/SAVE10.DTA");
	let wrl_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals/SNOW_1.WRL");
	if !save_path.is_file() || !wrl_path.is_file() {
		eprintln!("skipping export_save_gives_placed_buildings_a_valid_complex: fixtures absent");
		return;
	}

	let wrl = max_assets::wrl::read_wrl_file(&wrl_path).unwrap();
	let mut project = Project::from_wrl(&wrl, "SNOW1SAVE");
	project.attach_save(std::fs::read(&save_path).unwrap()).unwrap();
	let dims = (project.width, project.height);

	// Two adjacent mining stations (2x2, a type present in SAVE10), connected
	// the way the app connects them on placement.
	let miningst = max_assets::save::unit_type_id("MININGST").unwrap();
	project.place_object(MapObject { unit_type: miningst, x: 40, y: 40, team: 0, props: ObjectProps::default() });
	project.place_object(MapObject { unit_type: miningst, x: 42, y: 40, team: 0, props: ObjectProps::default() });
	assert!(project.auto_connect_buildings(), "adjacent stations connect");

	let out = read_save_bytes(&project.export_save().unwrap(), dims).unwrap();
	let issues = max_assets::save::check_complexes(&out);
	assert!(issues.is_empty(), "exported save violates the complex invariant:\n  {}", issues.join("\n  "));
	let placed: Vec<_> = out.units().filter(|u| u.unit_type == miningst && u.grid_y == 40).collect();
	assert_eq!(placed.len(), 2, "both stations exported");
	let c = placed[0].complex.expect("a placed building has a complex");
	assert_eq!(placed[1].complex, Some(c), "adjacent connected stations share one complex");
	assert!(out.team_units[0].complexes.contains(&c), "the complex is listed by the owning team");
}

/// Export Save File mining pass (HANDOFF 2026-08-02 Finding 3): painting
/// resources under an existing mining station re-derives its stored
/// production on export; painting elsewhere leaves its bytes untouched; and
/// a placed station exports powered on (the deploy order the app seeds into
/// its props) with production derived from its own ground. Gated on SAVE10 +
/// its pristine SNOW_1 world.
#[test]
fn export_save_rederives_a_repainted_stations_production() {
	let Some(home) = std::env::var_os("HOME") else { return };
	let save_path = Path::new(&home).join("MAX/SAVE10.DTA");
	let wrl_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals/SNOW_1.WRL");
	if !save_path.is_file() || !wrl_path.is_file() {
		eprintln!("skipping export_save_rederives_a_repainted_stations_production: fixtures absent");
		return;
	}
	let wrl = max_assets::wrl::read_wrl_file(&wrl_path).unwrap();
	let open = |raw: &[u8]| {
		let mut p = Project::from_wrl(&wrl, "SNOW1SAVE");
		p.attach_save(raw.to_vec()).unwrap();
		p
	};
	let raw = std::fs::read(&save_path).unwrap();
	let project = open(&raw);
	let dims = (project.width, project.height);
	let stored = |file: &max_assets::save::SaveFile, id: u16| -> [u8; 7] {
		let slot =
			file.objects.iter().position(|o| matches!(o, max_assets::save::SaveObject::Unit(u) if u.id == id)).unwrap();
		let l = file.object_meta[slot].unit_layout.as_ref().unwrap();
		file.object_meta[slot].body_raw[l.build_time + 1..l.build_time + 8].try_into().unwrap()
	};
	let miningst = max_assets::save::MININGST;
	let mine = project.save.as_ref().unwrap().file.units().find(|u| u.unit_type == miningst).unwrap().clone();
	let before = stored(&project.save.as_ref().unwrap().file, mine.id);

	// Paint far from the station: its production bytes export unchanged.
	let mut away = open(&raw);
	away.set_cargo(2, 2, max_assets::save::cargo_surveyed(max_assets::save::CARGO_GOLD | 10));
	let out = read_save_bytes(&away.export_save().unwrap(), dims).unwrap();
	assert_eq!(stored(&out, mine.id), before, "ground elsewhere: the station keeps its bytes");

	// Paint under its footprint: the export re-derives production from the
	// new ground, deploy-style.
	let mut under = open(&raw);
	let (mx, my) = (mine.grid_x as u16, mine.grid_y as u16);
	under.set_cargo(mx, my, max_assets::save::cargo_surveyed(max_assets::save::CARGO_GOLD | 13));
	let out = read_save_bytes(&under.export_save().unwrap(), dims).unwrap();
	let (raw_c, gold_c, fuel_c) =
		max_assets::save::derive_mining(&out.cargo_map, &out.surface_map, dims, mx as i32, my as i32);
	assert!(gold_c >= 13, "the painted gold is in the derived ceiling");
	let expect = max_assets::save::mining_bytes(raw_c, gold_c, fuel_c);
	assert_eq!(stored(&out, mine.id), expect, "production re-derived off the repainted ground");
	assert_ne!(stored(&out, mine.id), before, "and it actually changed");

	// A placed station (props seeded with the deploy order, as the app does)
	// exports powered on, producing from its own ground.
	let mut placed = open(&raw);
	placed.set_cargo(45, 45, max_assets::save::cargo_surveyed(max_assets::save::CARGO_RAW | 11));
	placed.place_object(MapObject {
		unit_type: miningst,
		x: 45,
		y: 45,
		team: 0,
		props: ObjectProps { orders: max_assets::save::deploy_orders(miningst), ..Default::default() },
	});
	let out = read_save_bytes(&placed.export_save().unwrap(), dims).unwrap();
	let new = out.units().find(|u| u.unit_type == miningst && (u.grid_x, u.grid_y) == (45, 45)).unwrap();
	assert_eq!(new.orders, max_assets::save::ORDER_POWER_ON, "a placed station starts powered on");
	let (raw_c, gold_c, fuel_c) = max_assets::save::derive_mining(&out.cargo_map, &out.surface_map, dims, 45, 45);
	assert!(raw_c >= 11, "the painted raw is under it");
	assert_eq!(
		stored(&out, new.id),
		max_assets::save::mining_bytes(raw_c, gold_c, fuel_c),
		"placed production derived from its own ground"
	);
}

/// `export_onto_base` (save a normal map with no attached save): the caller's
/// base save is kept whole and each placed unit is added by cloning a same-type
/// template; a type absent from the base is counted as skipped, not dropped. Uses
/// the bundled GREEN_3 50×50 world + its save fixture.
#[test]
fn export_onto_base_adds_placed_units_to_a_chosen_save() {
	let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/saves");
	let (wrl_path, save_path) = (dir.join("GREEN_3-50x50.WRL"), dir.join("save11-green3-50x50.dta"));
	if !wrl_path.is_file() || !save_path.is_file() {
		eprintln!("skipping export_onto_base_adds_placed_units_to_a_chosen_save: fixtures absent");
		return;
	}
	let wrl = max_assets::wrl::read_wrl_file(&wrl_path).unwrap();
	let base_raw = std::fs::read(&save_path).unwrap();
	let mut project = Project::from_wrl(&wrl, "GREEN3");
	let dims = (project.width, project.height);

	let base = read_save_bytes(&base_raw, dims).unwrap();
	let base_count = base.units().count();
	let present = base.units().next().expect("the base save has units").unit_type;

	// No units placed: exporting onto the base re-emits it unchanged (no adds).
	let (bytes0, skipped0) = project.export_onto_base(&base_raw, None).unwrap();
	assert!(skipped0.is_empty());
	assert_eq!(read_save_bytes(&bytes0, dims).unwrap().units().count(), base_count, "empty project adds nothing");

	// A present type clones a template → the exported count grows by exactly one.
	project.place_object(MapObject { unit_type: present, x: 5, y: 5, team: 0, props: ObjectProps::default() });
	let (bytes, skipped) = project.export_onto_base(&base_raw, None).unwrap();
	assert!(skipped.is_empty(), "a present type is not skipped");
	let out = read_save_bytes(&bytes, dims).unwrap();
	assert_eq!(out.units().count(), base_count + 1, "the placement was added onto the base's units");
	assert!(
		out.units().any(|u| u.unit_type == present && (u.grid_x, u.grid_y) == (5, 5)),
		"the placed unit sits at its cell",
	);

	// A type the base has no template for is skipped (reported), not dropped silently.
	let present_types: std::collections::HashSet<u16> = base.units().map(|u| u.unit_type).collect();
	if let Some(absent) =
		(0u16..0x40).find(|t| !present_types.contains(t) && max_assets::save::unit_type_name(*t).is_some())
	{
		let mut p2 = Project::from_wrl(&wrl, "GREEN3");
		p2.place_object(MapObject { unit_type: absent, x: 7, y: 7, team: 0, props: ObjectProps::default() });
		let (bytes2, skipped2) = p2.export_onto_base(&base_raw, None).unwrap();
		assert_eq!(skipped2.len(), 1, "a type with no template in the base is skipped (no fresh-body ctx)");
		assert_eq!(
			read_save_bytes(&bytes2, dims).unwrap().units().count(),
			base_count,
			"a skipped placement adds nothing",
		);
	}
}

/// The internal palette keeps the WRL's own bytes (statics included) with
/// live dynamic edits merged in; conversion rewrites tiles + palette to
/// the compatible form and converges the two - and undoes as one unit.
#[test]
fn wrl_palette_conversion_remaps_tiles_and_converges_palettes() {
	let mut tiles = vec![0u8; TILE_DATA_SIZE];
	tiles.fill(40); // every pixel on a fixed game-ramp slot…
	let mut palette = crate::GAME_PALETTE.to_vec();
	palette[40 * 3..40 * 3 + 3].copy_from_slice(&[0xff, 0x00, 0xee]); // …claiming hot pink
	let wrl = WrlFile {
		header: vec![0; 5],
		width: 1,
		height: 1,
		minimap: vec![0],
		bigmap: vec![0],
		tile_count: 1,
		tiles,
		palette,
		pass_table: vec![0],
	};
	let mut p = Project::from_wrl(&wrl, "CONV");
	assert!(p.is_wrl_import());
	// The working palette resolved slot 40 to the game color, the
	// internal palette still says pink.
	assert_eq!(p.palette[40 * 3..40 * 3 + 3], crate::GAME_PALETTE[40 * 3..40 * 3 + 3]);
	assert_eq!(p.internal_palette()[40 * 3..40 * 3 + 3], [0xff, 0x00, 0xee]);
	// Dynamic edits show through the internal palette too.
	assert!(p.set_color(64, [9, 9, 9]).unwrap());
	assert_eq!(p.internal_palette()[64 * 3..64 * 3 + 3], [9, 9, 9]);

	let opts = crate::palette_convert::ConvertOptions::default();
	let structure = p.structure_revision();
	let report = p.convert_to_compatible_palette(opts).expect("off-spec static slot");
	assert_eq!((report.exact, report.approximated), (1, 0));
	// The pink moved to an (unused) free dynamic slot - pixels follow, exactly.
	let to = p.packs[0].tiles[0];
	assert!(DYNAMIC_SLOTS.contains(&to), "pixels remapped into a free dynamic slot, got {to}");
	assert!(p.packs[0].tiles.iter().all(|&b| b == to));
	assert_eq!(p.palette[to as usize * 3..to as usize * 3 + 3], [0xff, 0x00, 0xee]);
	// Palette and internal palette agree now (compatible), the doc is
	// dirty + structurally changed, and a re-run is a no-op.
	assert_eq!(p.internal_palette(), p.palette);
	assert!(p.dirty());
	assert_ne!(p.structure_revision(), structure);
	assert!(p.convert_to_compatible_palette(opts).is_none());

	// One Ctrl+Z brings the whole document back: tiles, palettes,
	// internal palette - and redo replays it byte-identically.
	let converted_tiles = p.packs[0].tiles.clone();
	let converted_palette = p.palette.clone();
	assert!(p.undo());
	assert!(p.packs[0].tiles.iter().all(|&b| b == 40), "tiles restored");
	assert_eq!(p.internal_palette()[40 * 3..40 * 3 + 3], [0xff, 0x00, 0xee], "internal palette restored");
	// The earlier set_color is still the next undo step (journal intact).
	assert!(p.undo());
	assert_ne!(p.internal_palette()[64 * 3..64 * 3 + 3], [9, 9, 9]);
	assert!(p.redo() && p.redo());
	assert_eq!(p.packs[0].tiles, converted_tiles);
	assert_eq!(p.palette, converted_palette);
}

/// The rasterize-and-reimport method rebuilds the tile table from the
/// composed pixels; pinned water keeps its cycle slots and colors, and
/// per-cell pass survives as overrides. Undoes as one unit.
#[test]
fn wrl_palette_conversion_by_reimport_pins_water_and_keeps_pass() {
	// Tile 0: all water-cycle slot 100; tile 1: all off-spec static 40.
	let mut tiles = vec![0u8; 2 * TILE_DATA_SIZE];
	tiles[..TILE_DATA_SIZE].fill(100);
	tiles[TILE_DATA_SIZE..].fill(40);
	let mut palette = crate::GAME_PALETTE.to_vec();
	palette[100 * 3..100 * 3 + 3].copy_from_slice(&[12, 34, 56]);
	palette[40 * 3..40 * 3 + 3].copy_from_slice(&[0xff, 0x00, 0xee]);
	let wrl = WrlFile {
		header: vec![0; 5],
		width: 2,
		height: 1,
		minimap: vec![100, 40],
		bigmap: vec![0, 1],
		tile_count: 2,
		tiles,
		palette,
		pass_table: vec![1, 0],
	};
	let mut p = Project::from_wrl(&wrl, "RAST");
	let tile_count = p.convert_palette_by_reimport(true, crate::image_import::Dedupe::Strict, 0.0).expect("reimport");
	assert!(tile_count >= 2);
	// Water pixels stay pinned to slot 100, with the map's color.
	assert_eq!(p.compose_cell(0, 0)[..], vec![100u8; TILE_DATA_SIZE][..]);
	assert_eq!(p.palette[100 * 3..100 * 3 + 3], [12, 34, 56]);
	// The pink tile re-quantized into stable (non-animated) slots close
	// to pink; statics are the game's.
	let cell1 = p.compose_cell(1, 0);
	assert!(cell1.iter().all(|&b| !(9..=31).contains(&b) && !(96..=127).contains(&b)));
	assert_eq!(p.palette[32 * 3..32 * 3 + 3], crate::GAME_PALETTE[32 * 3..32 * 3 + 3]);
	// Pass survived as per-cell overrides.
	assert_eq!(p.pass_at(0, 0), Some(1));
	assert_eq!(p.pass_at(1, 0), Some(0));
	// One undo restores the original document byte-for-byte.
	assert!(p.undo());
	assert_eq!(p.compose_cell(0, 0)[..], vec![100u8; TILE_DATA_SIZE][..]);
	assert_eq!(p.compose_cell(1, 0)[..], vec![40u8; TILE_DATA_SIZE][..]);
	assert_eq!(p.internal_palette()[40 * 3..40 * 3 + 3], [0xff, 0x00, 0xee]);
	assert_eq!(p.pass_at(0, 0), Some(1));
}

/// Golden splitmix64 vectors (seed 0) - pins the algorithm forever:
/// generated maps must replay identically from their seed.
#[test]
fn rng_matches_splitmix64_reference() {
	let mut rng = Rng::new(0);
	assert_eq!(rng.next_u64(), 0xe220_a839_7b1d_cdaf);
	assert_eq!(rng.next_u64(), 0x6e78_9e6a_a1b9_65f4);
	assert_eq!(rng.next_u64(), 0x06c4_5d18_8009_454f);
}

#[test]
fn new_project_fills_water_deterministically() {
	let root = assets_root();
	let p = Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap();
	assert_eq!((p.width, p.height), (8, 6));
	assert!(!p.dirty());

	// WATER implied at index 0; GREEN owns the palette.
	assert_eq!(p.uses[0].name, "WATER");
	assert!(!p.uses[0].palette);
	assert_eq!(p.uses[1].name, "GREEN");
	assert!(p.uses[1].palette);
	assert_eq!(p.water_pack, Some(0));

	let water_tiles = p.packs[0].tile_count();
	for stack in &p.cells {
		let water = stack[LAYER_WATER].expect("bottom layer fully covered");
		assert_eq!(water.pack, 0);
		assert!(water.tile < water_tiles);
		assert_eq!(water.transform, Transform::default(), "WATER is sync - identity");
		assert_eq!(stack[LAYER_GROUND], None);
	}

	// Same seed → same map; different seed → different map.
	let again = Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap();
	assert_eq!(p.hash(), again.hash());
	let other = Project::new(8, 6, &["GREEN".to_string()], &root, 43).unwrap();
	assert_ne!(p.hash(), other.hash());

	// Listing WATER explicitly must not duplicate it.
	let explicit = Project::new(8, 6, &["WATER".to_string(), "GREEN".to_string()], &root, 42).unwrap();
	assert_eq!(explicit.packs.len(), 2);
	assert_eq!(p.hash(), explicit.hash());
}

#[test]
fn new_project_round_trips_through_save() {
	let root = assets_root();
	let p = Project::new(5, 4, &["DESERT".to_string()], &root, 7).unwrap();
	let reloaded = Project::from_str(&p.save_string(), &root).unwrap();
	assert_eq!(p.hash(), reloaded.hash());
	assert_eq!(reloaded.uses.len(), 2);
	assert_eq!(reloaded.uses[1].name, "DESERT");
}

#[test]
fn stacked_same_layer_tiles_load_without_a_duplicate_error() {
	// Regression: an opened WRL becomes a project whose base pack is *not*
	// named WATER. Painting over the base then yields a cell with two
	// tiles, neither recognized as water - the old per-pack loader put both
	// on the ground layer and rejected the file ("duplicate ground layer").
	// Layers are advisory, so the loader reconstructs the stack positionally.
	let root = assets_root();
	let p = Project::new(2, 1, &["GREEN".to_string()], &root, 1).unwrap();
	assert!(p.packs[1].tile_count() >= 2, "GREEN has at least two tiles");
	let (a, b) = (p.packs[1].ids[0].clone(), p.packs[1].ids[1].clone());
	// A WATER-less project (GREEN owns the palette): both ids resolve to
	// GREEN, the case that used to collide on the ground layer.
	let json = format!(
		"{{\"version\":\"1\",\"name\":\"t\",\"description\":\"\",\"width\":2,\"height\":1,\
		 \"use\":[{{\"name\":\"GREEN\",\"tileset\":true,\"palette\":true,\"version\":\"1\"}}],\
		 \"map\":[[\"{a},{b}\",\"\"]]}}"
	);
	let loaded = Project::from_str(&json, &root).expect("stacked cell loads without error");
	let stack = loaded.cell(0, 0).unwrap();
	assert_eq!(stack[LAYER_WATER].map(|t| t.tile), Some(0), "first tile -> base layer");
	assert_eq!(stack[LAYER_GROUND].map(|t| t.tile), Some(1), "second tile -> ground layer");
}

#[test]
fn project_file_version_guards_on_major_and_migrates() {
	let root = assets_root();
	let p = Project::new(4, 4, &["GREEN".to_string()], &root, 1).unwrap();
	let text = p.save_string();
	// New saves carry the scheme'd top-level key/value.
	let current = format!("\"mme_project_file_version\": \"{PROJECT_VERSION}\"");
	assert!(text.contains(&current), "{text}");
	assert_eq!(Project::from_str(&text, &root).unwrap().version, PROJECT_VERSION);

	let swap = |to: &str| text.replace(&current, to);
	// A pre-scheme `version: "1"` file still opens, migrated to the current.
	let legacy = swap("\"version\": \"1\"");
	assert_eq!(Project::from_str(&legacy, &root).expect("legacy migrates").version, PROJECT_VERSION);
	// An older MINOR within the same MAJOR opens - which is what keeps every
	// map saved before the scenery block (2.0) readable.
	assert!(Project::from_str(&swap("\"mme_project_file_version\": \"2.0\""), &root).is_ok());
	// So does a newer one.
	assert!(Project::from_str(&swap("\"mme_project_file_version\": \"2.7\""), &root).is_ok());
	// A different MAJOR is a hard break; malformed versions are rejected.
	match Project::from_str(&swap("\"mme_project_file_version\": \"3.0\""), &root) {
		Ok(_) => panic!("a different MAJOR must be rejected"),
		Err(e) => assert!(e.contains("unsupported"), "{e}"),
	}
	assert!(Project::from_str(&swap("\"mme_project_file_version\": \"banana\""), &root).is_err());
}

#[test]
fn load_rejects_malformed_headers() {
	let root = assets_root();
	let err = |json: &str| match Project::from_str(json, &root) {
		Ok(_) => panic!("expected a load error for: {json}"),
		Err(e) => e,
	};
	// Missing required top-level fields (version is checked first; name/
	// description are read before the dimensions).
	assert!(err("{}").contains("mme_project_file_version"), "no version key");
	assert!(
		err(r#"{"mme_project_file_version": "2.0", "name": "t", "description": "", "height": 4}"#)
			.contains("missing field 'width'"),
		"no width"
	);
	// Bad / non-numeric dimensions - caught before any map parsing.
	let dims = |w: &str, h: &str| {
		format!(r#"{{"mme_project_file_version": "2.0", "name": "t", "description": "", "width": {w}, "height": {h}}}"#)
	};
	assert!(err(&dims("0", "4")).contains("bad map size"), "zero width");
	assert!(err(&dims("4", "0")).contains("bad map size"), "zero height");
	assert!(err(&dims("2000", "4")).contains("bad map size"), "width > 1024");
	assert!(err(&dims(r#""x""#, "4")).contains("width not a number"), "non-numeric width");
}

#[test]
fn load_rejects_malformed_body() {
	let root = assets_root();
	// A valid 2×1 project (GREEN owns the palette, empty cells); `map`/`extra`
	// are spliced in so each case isolates one malformation.
	let base = |map: &str, extra: &str| {
		format!(
			r#"{{"version":"1","name":"t","description":"","width":2,"height":1,"use":[{{"name":"GREEN","tileset":true,"palette":true,"version":"1"}}]{extra},"map":{map}}}"#
		)
	};
	let err = |json: String| match Project::from_str(&json, &root) {
		Ok(_) => panic!("expected a load error for: {json}"),
		Err(e) => e,
	};
	// Sanity: the unmutated base loads.
	Project::from_str(&base(r#"[["",""]]"#, ""), &root).expect("the base project loads");

	// Map shape: wrong row count, wrong cell count per row.
	assert!(err(base("[]", "")).contains("map has 0 rows"), "row count");
	assert!(err(base(r#"[[""]]"#, "")).contains("row 0 has 1 cells"), "cell count");
	// Cell typing: a non-string/array cell, and a non-string inside the array form.
	assert!(err(base(r#"[[123,""]]"#, "")).contains("not a string or array"), "scalar cell");
	assert!(err(base(r#"[[[123],""]]"#, "")).contains("non-string entry"), "array cell entry");
	// Pass overlay (array form): wrong row count and wrong row length.
	assert!(err(base(r#"[["",""]]"#, r#","pass":[]"#)).contains("pass has 0 rows"), "pass rows");
	assert!(err(base(r#"[["",""]]"#, r#","pass":["0"]"#)).contains("pass row 0 has 1 cells"), "pass row len");
	// Units: a coordinate outside the map.
	assert!(err(base(r#"[["",""]]"#, r#","units":["T 5 0 0"]"#)).contains("out of range"), "unit OOR");
	// Exactly one palette owner is required.
	let no_owner = r#"{"version":"1","name":"t","description":"","width":2,"height":1,"use":[{"name":"GREEN","tileset":true,"palette":false,"version":"1"}],"map":[["",""]]}"#;
	assert!(err(no_owner.to_string()).contains("palette owner"), "palette owner count");
}

#[test]
fn load_accepts_legacy_sparse_pass_and_positional_overstack() {
	let root = assets_root();
	let p = Project::new(2, 1, &["GREEN".to_string()], &root, 1).unwrap();
	let a = p.packs[1].ids[0].clone();
	// A cell with more refs than layers (3 > MAX_LAYERS) is reconstructed
	// positionally rather than rejected: first → base, the rest stack upward.
	let three = format!(
		r#"{{"version":"1","name":"t","description":"","width":2,"height":1,"use":[{{"name":"GREEN","tileset":true,"palette":true,"version":"1"}}],"map":[["{a},{a},{a}",""]]}}"#
	);
	let loaded = Project::from_str(&three, &root).expect("3-ref overstack loads via positional fallback");
	let stack = loaded.cell(0, 0).unwrap();
	assert!(stack[0].is_some() && stack[1].is_some(), "both layers filled from the overstack");

	// Legacy sparse pass form `{ "x,y": value }` still loads; out-of-range rejects.
	let pass = |v: &str| {
		format!(
			r#"{{"version":"1","name":"t","description":"","width":2,"height":1,"use":[{{"name":"GREEN","tileset":true,"palette":true,"version":"1"}}],"map":[["",""]],"pass":{{"0,0":{v}}}}}"#
		)
	};
	Project::from_str(&pass("2"), &root).expect("legacy sparse pass loads");
	assert!(Project::from_str(&pass("9"), &root).err().unwrap().contains("out of range"), "sparse pass OOR");
}

#[test]
fn map_metadata_round_trips_and_stays_optional() {
	let root = assets_root();
	let mut p = Project::new(4, 4, &["GREEN".to_string()], &root, 1).unwrap();
	// A metadata-free map writes none of the metadata keys.
	let bare = p.save_string();
	for key in ["\"players\"", "\"date\"", "\"map_version\"", "\"author\""] {
		assert!(!bare.contains(key), "bare save should omit {key}");
	}
	// Set them (description keeps newlines, strips CR; players clamps 2..=4).
	p.set_info(
		"Twin Peaks".into(),
		Some(9),
		"line one\r\nline two".into(),
		"2026".into(),
		"1.2".into(),
		"Aneta".into(),
	);
	assert_eq!(p.players, Some(4), "players clamps to 4");
	assert_eq!(p.description, "line one\nline two", "CR stripped, newline kept");
	assert!(p.dirty());
	let saved = p.save_string();
	assert!(saved.contains("\"players\": \"2-4\""), "players saved as its label, not a number");
	let reloaded = Project::from_str(&saved, &root).unwrap();
	assert_eq!(reloaded.name, "Twin Peaks");
	assert_eq!(reloaded.players, Some(4), "label round-trips back to the count");
	assert_eq!(reloaded.description, "line one\nline two", "newline survives the JSON round-trip");
	assert_eq!(reloaded.date, "2026");
	assert_eq!(reloaded.map_version, "1.2");
	assert_eq!(reloaded.author, "Aneta");
	// The other counts map to their labels; legacy bare-number saves still load.
	for (count, label) in [(2u8, "\"2\""), (3, "\"2-3\"")] {
		p.set_info(String::new(), Some(count), String::new(), String::new(), String::new(), String::new());
		assert!(p.save_string().contains(&format!("\"players\": {label}")), "count {count} -> {label}");
	}
	let legacy = saved.replace("\"players\": \"2-4\"", "\"players\": 3");
	assert_eq!(Project::from_str(&legacy, &root).unwrap().players, Some(3), "legacy numeric players loads");
}

#[test]
fn info_json_carries_set_metadata_only() {
	let root = assets_root();
	let mut p = Project::new(4, 4, &["GREEN".to_string()], &root, 1).unwrap();
	// Only the format tag + the default name: still worth appending.
	let named = p.info_json().expect("a named map has metadata");
	assert!(named.contains("\"mme_map_metadata\": 1"), "format tag present: {named}");
	assert!(named.contains("\"name\": \"Untitled\""), "{named}");
	assert!(!named.contains("\"players\""), "unset fields stay out: {named}");
	// Nothing set at all → no blob.
	p.set_info(String::new(), None, String::new(), String::new(), String::new(), String::new());
	assert_eq!(p.info_json(), None, "empty metadata appends nothing");
	// Full set uses the project-file keys and players label.
	p.set_info("Twin Peaks".into(), Some(4), "desc".into(), "2026".into(), "1.2".into(), "Aneta".into());
	let full = p.info_json().unwrap();
	for key in
		["\"players\": \"2-4\"", "\"description\": \"desc\"", "\"map_version\": \"1.2\"", "\"author\": \"Aneta\""]
	{
		assert!(full.contains(key), "{key} in {full}");
	}
}

#[test]
fn new_project_without_palette_owner_fails() {
	let Err(err) = Project::new(4, 4, &[], &assets_root(), 0) else {
		panic!("expected an error");
	};
	assert!(err.contains("palette"), "{err}");
}

#[test]
fn pixel_at_matches_full_compose() {
	let root = assets_root();
	let mut p = Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap();
	// A transformed shore over water exercises layering + transforms.
	let (tile, layer) = p.resolve_ref("GSa000:!N").unwrap();
	assert!(p.place(3, 2, layer, Some(tile)));

	for &(x, y) in &[(3u16, 2u16), (0, 0), (7, 5)] {
		let composed = p.compose_cell(x, y);
		for &(sx, sy) in &[(0usize, 0usize), (32, 32), (63, 63), (17, 48)] {
			assert_eq!(p.pixel_at(x, y, (sx, sy)), composed[sy * 64 + sx], "cell ({x},{y}) sub ({sx},{sy})",);
		}
		assert_eq!(p.minimap_pixel(x, y), composed[32 * 64 + 32]);
	}
}

#[test]
fn tile_pass_edits_retint_every_shared_cell_and_round_trip() {
	let root = assets_root();
	let mut p = Project::new(4, 1, &["GREEN".to_string()], &root, 7).unwrap();
	// The same land tile under two cells - they share one tile id.
	let (land, layer) = p.resolve_ref("GLa000").unwrap();
	assert!(p.place(0, 0, layer, Some(land)));
	assert!(p.place(1, 0, layer, Some(land)));
	let before = p.pass_at(0, 0);
	assert_eq!(p.pass_at(1, 0), before, "same tile, same pass");

	// Editing the tile pass at one cell retints the other (tile-dependent).
	assert!(p.set_tile_pass_at(0, 0, 3));
	assert_eq!(p.pass_at(0, 0), Some(3));
	assert_eq!(p.pass_at(1, 0), Some(3), "shared tile id retints together");
	assert_eq!(p.pass_override(0, 0), None, "it's tile pass, not a cell override");

	// One undo unit restores both cells.
	assert!(p.undo());
	assert_eq!(p.pass_at(0, 0), before);
	assert_eq!(p.pass_at(1, 0), before);
	p.redo();
	assert_eq!(p.pass_at(1, 0), Some(3), "redo replays the tile edit");

	// Per-tile pass persists through save/load (the `tilepass` block).
	let text = p.save_string();
	assert!(text.contains("\"tilepass\""), "tile pass is persisted");
	let reloaded = Project::from_str(&text, &root).unwrap();
	assert_eq!(reloaded.pass_at(0, 0), Some(3));
	assert_eq!(reloaded.pass_at(1, 0), Some(3));
}

#[test]
fn reset_tile_pass_reverts_to_the_supplied_canonical_pass() {
	let root = assets_root();
	let mut p = Project::new(2, 1, &["GREEN".to_string()], &root, 7).unwrap();
	let (land, layer) = p.resolve_ref("GLa000").unwrap();
	assert!(p.place(0, 0, layer, Some(land)));
	// The canonical (tileset) pass = a snapshot of every pack's current pass,
	// taken before any edit.
	let canonical: Vec<Option<Vec<u8>>> = p.packs.iter().map(|pk| pk.pass.clone()).collect();

	// Edit the land tile's pass away from its tileset value.
	let before = p.pass_at(0, 0).unwrap();
	let edited = if before == 3 { 0 } else { 3 };
	assert!(p.set_tile_pass_at(0, 0, edited));
	assert_eq!(p.pass_at(0, 0), Some(edited));

	// Reset to canonical reverts it, as one undo unit.
	assert!(p.reset_tile_pass(&canonical), "a change was applied");
	assert_eq!(p.pass_at(0, 0), Some(before), "back to the tileset value");
	assert!(p.undo(), "reset is undoable");
	assert_eq!(p.pass_at(0, 0), Some(edited), "undo brings the edit back");

	// Already-canonical → no-op (nothing to undo).
	p.redo();
	assert!(!p.reset_tile_pass(&canonical), "no change when already canonical");
	// A `None` entry leaves that pack untouched even when it differs.
	assert!(p.set_tile_pass_at(0, 0, edited));
	let skip: Vec<Option<Vec<u8>>> = vec![None; p.packs.len()];
	assert!(!p.reset_tile_pass(&skip), "None per pack skips it");
	assert_eq!(p.pass_at(0, 0), Some(edited), "skipped pack keeps its edit");
}

#[test]
fn pass_overrides_round_trip_through_the_dense_grid() {
	let root = assets_root();
	let mut p = Project::new(5, 3, &["GREEN".to_string()], &root, 1).unwrap();
	assert!(p.set_pass(2, 1, 3));
	assert!(p.set_pass(4, 2, 2));
	let text = p.save_string();
	// The block is a dense array of digit-rows, not a sparse object.
	assert!(text.contains("\"pass\""));
	assert!(text.contains("\"--3--\""), "row 1 carries the blocked override:\n{text}");
	let reloaded = Project::from_str(&text, &root).unwrap();
	assert_eq!(reloaded.pass_override(2, 1), Some(3));
	assert_eq!(reloaded.pass_override(4, 2), Some(2));
	assert_eq!(reloaded.pass_override(0, 0), None);
	assert_eq!(reloaded.hash(), p.hash(), "overrides survive the dense round-trip");
}

#[test]
fn pass_at_reads_the_stack_top() {
	let root = assets_root();
	let mut p = Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap();
	assert_eq!(p.pass_at(2, 2), Some(1), "fresh map is water");
	let (tile, layer) = p.resolve_ref("GLa000").unwrap();
	assert!(p.place(2, 2, layer, Some(tile)));
	assert_eq!(p.pass_at(2, 2), Some(0), "land tile on top");
	assert_eq!(p.pass_at(99, 99), None, "out of range");
}

#[test]
fn pass_override_paints_undoes_saves_and_bakes() {
	let root = assets_root();
	let mut p = Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap();
	// Fresh water cell derives pass 1; override it to blocked (3).
	assert_eq!(p.pass_at(2, 2), Some(1));
	assert!(p.set_pass(2, 2, 3));
	assert_eq!(p.pass_at(2, 2), Some(3), "override wins over derived");
	assert_eq!(p.pass_override(2, 2), Some(3));
	// The bake reads the override (a fresh water map is all pass 1, so a
	// blocked tile in the baked per-tile passtab can only come from it).
	let wrl = crate::bake(&p).unwrap();
	assert!(wrl.pass_table.contains(&3), "override flows into the bake");

	// Undoable, one unit; round-trips through save.
	let with = p.hash();
	assert!(p.undo());
	assert_eq!(p.pass_at(2, 2), Some(1), "undo restores the derived pass");
	assert_eq!(p.pass_override(2, 2), None);
	p.redo();
	assert_eq!(p.hash(), with, "redo replays the override");

	let text = p.save_string();
	assert!(text.contains("\"pass\""), "the override is persisted");
	let reloaded = Project::from_str(&text, &root).unwrap();
	assert_eq!(reloaded.pass_at(2, 2), Some(3), "and reloads");
	assert_eq!(reloaded.hash(), p.hash());
}

fn obj(tag: &str, x: u16, y: u16, team: u8) -> MapObject {
	let unit_type = max_assets::save::unit_type_id(tag).unwrap();
	MapObject { unit_type, x, y, team, props: ObjectProps::default() }
}

#[test]
fn objects_round_trip_through_save() {
	let root = assets_root();
	let mut p = Project::new(4, 4, &["GREEN".to_string()], &root, 42).unwrap();
	assert!(!p.dirty());

	p.place_object(obj("TANK", 1, 2, 3));
	p.place_object(obj("SCOUT", 0, 0, 0));
	// Restamping a cell replaces, not stacks.
	p.place_object(obj("AWAC", 1, 2, 1));
	assert!(p.dirty(), "objects persist, so they dirty the doc");
	assert_eq!(p.objects.len(), 2);

	let text = p.save_string();
	assert!(text.contains("\"objects\""), "the objects are persisted");
	let reloaded = Project::from_str(&text, &root).unwrap();
	assert_eq!(reloaded.objects, p.objects, "objects reload identically");

	assert!(p.remove_object_at(1, 2));
	assert!(!p.remove_object_at(1, 2), "already gone");
	assert_eq!(p.clear_objects(), 1);
	// An object-free project saves without the block at all.
	assert!(!p.save_string().contains("\"objects\""));
}

#[test]
fn object_edits_undo_and_redo() {
	let root = assets_root();
	let mut p = Project::new(4, 4, &["GREEN".to_string()], &root, 1).unwrap();

	p.place_object(obj("TANK", 1, 1, 0));
	p.place_object(obj("SCOUT", 2, 2, 1));
	assert_eq!(p.objects.len(), 2);

	// Undo peels off the placements one at a time, redo restores them.
	assert!(p.undo());
	assert_eq!(p.objects.len(), 1);
	assert_eq!(p.objects[0].x, 1, "the tank remains");
	assert!(p.undo());
	assert!(p.objects.is_empty());
	assert!(p.redo());
	assert!(p.redo());
	assert_eq!(p.objects.len(), 2);

	// Delete + clear are undoable too.
	assert!(p.remove_object_at(1, 1));
	assert_eq!(p.objects.len(), 1);
	assert!(p.undo(), "the delete undoes");
	assert_eq!(p.objects.len(), 2);
	assert_eq!(p.clear_objects(), 2);
	assert!(p.undo(), "the clear undoes");
	assert_eq!(p.objects.len(), 2);
}

#[test]
fn move_object_is_undoable_and_stroke_coalesces() {
	let root = assets_root();
	let mut p = Project::new(8, 8, &["GREEN".to_string()], &root, 1).unwrap();
	p.place_object(obj("TANK", 1, 1, 0));

	// A single move undoes to the original cell.
	assert!(p.move_object_to(0, 5, 5));
	assert_eq!((p.objects[0].x, p.objects[0].y), (5, 5));
	assert!(!p.move_object_to(0, 5, 5), "no-op when already there");
	assert!(p.undo());
	assert_eq!((p.objects[0].x, p.objects[0].y), (1, 1), "undo restores the cell");

	// A drag (stroke) of several steps is one undo unit.
	p.begin_stroke();
	p.move_object_to(0, 2, 1);
	p.move_object_to(0, 3, 1);
	p.move_object_to(0, 4, 1);
	p.end_stroke();
	assert_eq!((p.objects[0].x, p.objects[0].y), (4, 1));
	assert!(p.undo(), "one undo reverses the whole drag");
	assert_eq!((p.objects[0].x, p.objects[0].y), (1, 1));

	assert!(!p.move_object_to(9, 0, 0), "out-of-range index is a no-op");
}

#[test]
fn set_object_state_edits_team_and_props_undoably() {
	let root = assets_root();
	let mut p = Project::new(4, 4, &["GREEN".to_string()], &root, 1).unwrap();
	p.place_object(obj("TANK", 1, 1, 0));

	// One call rewrites team + gameplay props together, undoably.
	let mut props = p.objects[0].props.clone();
	props.name = "Rex".to_string();
	props.angle = 3;
	props.hits = 42;
	props.orders = 0x0C;
	assert!(p.set_object_state(0, 2, props.clone()));
	assert_eq!(p.objects[0].team, 2);
	assert_eq!(p.objects[0].props.name, "Rex");
	assert_eq!((p.objects[0].props.angle, p.objects[0].props.hits), (3, 42));

	// Re-applying the identical state records no patch.
	assert!(!p.set_object_state(0, 2, props), "unchanged state is a no-op");

	// One undo restores the whole pre-edit state; redo re-applies it.
	assert!(p.undo());
	assert_eq!(p.objects[0].team, 0);
	assert_eq!(p.objects[0].props, ObjectProps::default(), "props revert wholesale");
	assert!(p.redo());
	assert_eq!(p.objects[0].props.hits, 42);

	assert!(!p.set_object_state(9, 1, ObjectProps::default()), "out-of-range index is a no-op");
}

/// The per-unit max-stats override (S4.5): [`Project::object_base_values`]
/// returns the override when set (else `None` with no save seed), the edit is
/// undoable through `set_object_state`, and the override survives Save + reload
/// (the `"values"` block) byte-for-byte.
#[test]
fn object_base_values_override_edits_and_round_trips() {
	let root = assets_root();
	let mut p = Project::new(4, 4, &["GREEN".to_string()], &root, 1).unwrap();
	p.place_object(obj("TANK", 1, 1, 0));

	// A fresh placement (no opened save) has neither a seed nor an override.
	assert_eq!(p.object_base_values(0), None, "no save, no override -> unknown stats");

	// Editing installs a per-unit override (clone-on-edit), undoably.
	let values = UnitValues {
		turns: 3,
		hits: 99,
		armor: 8,
		attack: 16,
		speed: 16,
		range: 6,
		rounds: 1,
		move_and_fire: 0,
		scan: 6,
		storage: 0,
		ammo: 8,
		attack_radius: 0,
		agent_adjust: 0,
		version: 2,
		in_use: true,
	};
	let mut props = p.objects[0].props.clone();
	props.base_values = Some(values.clone());
	assert!(p.set_object_state(0, 0, props));
	assert_eq!(p.object_base_values(0), Some(values.clone()), "the override wins over any seed");

	// It persists through Save + reload (a nested `"values"` object).
	let text = p.save_string();
	assert!(text.contains("\"values\""), "the override is persisted");
	let reloaded = Project::from_str(&text, &root).unwrap();
	assert_eq!(reloaded.objects[0].props.base_values, Some(values), "override round-trips exactly");

	// Undo drops the override back to the inherited (here: absent) seed.
	assert!(p.undo());
	assert_eq!(p.object_base_values(0), None, "undo clears the override");
}

/// The resource (cargo) map (S5): `set_cargo` edits a cell undoably, is a no-op
/// without a save / out of range / on an unchanged value, and a drag stroke
/// collapses to one undo unit.
#[test]
fn set_cargo_edits_cells_undoably() {
	let root = assets_root();
	let mut p = Project::new(4, 4, &["GREEN".to_string()], &root, 1).unwrap();

	// Stage D: no save attached → the resource map materializes on the
	// first paint (resources are placeable on any project, and save
	// synthesis carries them into a real `.DTA`).
	assert_eq!(p.cargo_at(1, 1), None, "no cargo map until the first edit");
	assert!(p.set_cargo(1, 1, max_assets::save::CARGO_RAW | 12), "first paint materializes the map");
	assert_eq!(p.cargo_at(1, 1), Some(0x8C));
	assert_eq!(p.cargo_map.len(), 16, "sized to the project");
	// Reset for the undo/redo sequence below.
	assert!(p.undo(), "undo the materializing paint");
	assert_eq!(p.cargo_at(1, 1), Some(0), "map stays, cell back to zero");

	// Paint a raw-materials cell (amount 12); it's undoable and idempotent.
	assert!(p.set_cargo(1, 1, max_assets::save::CARGO_RAW | 12));
	assert_eq!(p.cargo_at(1, 1), Some(0x8C));
	assert!(!p.set_cargo(1, 1, 0x8C), "unchanged value is a no-op");
	assert!(!p.set_cargo(9, 9, 1), "out-of-range is a no-op");
	assert!(p.undo(), "undo restores the previous value");
	assert_eq!(p.cargo_at(1, 1), Some(0));
	assert!(p.redo());
	assert_eq!(p.cargo_at(1, 1), Some(0x8C));

	// A drag stroke over several cells is one undo unit.
	p.begin_stroke();
	p.set_cargo(0, 0, max_assets::save::CARGO_FUEL | 4);
	p.set_cargo(0, 1, max_assets::save::CARGO_FUEL | 5);
	p.set_cargo(0, 2, max_assets::save::CARGO_FUEL | 6);
	p.end_stroke();
	assert_eq!(p.cargo_at(0, 2), Some(0x26));
	assert!(p.undo(), "one undo reverses the whole drag");
	assert_eq!((p.cargo_at(0, 0), p.cargo_at(0, 1), p.cargo_at(0, 2)), (Some(0), Some(0), Some(0)));
}

#[test]
fn object_stroke_is_one_undo_unit() {
	let root = assets_root();
	let mut p = Project::new(4, 4, &["GREEN".to_string()], &root, 1).unwrap();

	// A whole stroke of placements collapses to a single undo step.
	p.begin_stroke();
	p.place_object(obj("TANK", 0, 0, 0));
	p.place_object(obj("SCOUT", 1, 0, 0));
	p.place_object(obj("AWAC", 2, 0, 0));
	p.end_stroke();
	assert_eq!(p.objects.len(), 3);
	assert!(p.undo(), "one undo reverses the whole stroke");
	assert!(p.objects.is_empty());
	assert!(!p.undo(), "nothing before it");
}

#[test]
fn auto_connect_links_adjacent_same_team_buildings() {
	let root = assets_root();
	let mut p = Project::new(6, 4, &["GREEN".to_string()], &root, 1).unwrap();
	// Two 2×2 buildings side by side (COMMTWR covers 0-1, POWERSTN 2-3), same team.
	p.place_object(obj("COMMTWR", 0, 0, 0));
	p.place_object(obj("POWERSTN", 2, 0, 0));
	assert!(p.auto_connect_buildings(), "adjacent same-team buildings connect");
	assert_eq!(p.objects[0].props.connectors, 0x0C, "COMMTWR east edge -> ET|EB");
	assert_eq!(p.objects[1].props.connectors, 0xC0, "POWERSTN west edge -> WT|WB");
	// Idempotent (add-only): a second pass changes nothing / records no patch.
	assert!(!p.auto_connect_buildings(), "already connected -> no-op");
	// One undo reverses the whole connect.
	assert!(p.undo());
	assert_eq!(p.objects[0].props.connectors, 0);
	assert_eq!(p.objects[1].props.connectors, 0);
}

#[test]
fn auto_connect_respects_team_bridges_and_preserves_existing() {
	let root = assets_root();
	let mut p = Project::new(6, 4, &["GREEN".to_string()], &root, 1).unwrap();
	// Different teams never connect.
	p.place_object(obj("COMMTWR", 0, 0, 0));
	p.place_object(obj("POWERSTN", 2, 0, 1));
	assert!(!p.auto_connect_buildings(), "different teams stay disconnected");
	assert_eq!(p.objects[0].props.connectors, 0);

	// A 1×1 connector bridging a building on the same team: single reciprocal
	// bit (building ET ↔ connector WT). Add-only preserves a pre-set bit.
	let mut p = Project::new(6, 4, &["GREEN".to_string()], &root, 1).unwrap();
	p.place_object(obj("COMMTWR", 0, 0, 0)); // 2×2
	p.place_object(obj("CNCT_4W", 2, 0, 0)); // 1×1 connector, east of the top-right cell
	p.objects[0].props.connectors = 0x01; // a pre-existing NL link (must survive)
	assert!(p.auto_connect_buildings());
	assert_eq!(p.objects[0].props.connectors, 0x05, "COMMTWR keeps NL, gains ET");
	assert_eq!(p.objects[1].props.connectors, 0x40, "connector gains WT toward the building");
}

#[test]
fn resize_places_old_map_and_fills_water() {
	let root = assets_root();
	let mut p = Project::new(4, 4, &["GREEN".to_string()], &root, 42).unwrap();
	let (land, layer) = p.resolve_ref("GLa000").unwrap();
	p.place(0, 0, layer, Some(land)); // a marker in the top-left
	p.set_pass(0, 0, 3);

	// Enlarge to 8×8 with the old map centered (offset 2,2).
	p.resize(8, 8, 2, 2).unwrap();
	assert_eq!((p.width, p.height), (8, 8));
	// The marker moved to (2,2); its pass override rode along.
	let top = p.cell(2, 2).unwrap()[layer].unwrap();
	assert_eq!(p.packs[top.pack as usize].ids[top.tile as usize], "GLa000");
	assert_eq!(p.pass_override(2, 2), Some(3));
	// New territory is water.
	assert_eq!(p.pass_at(0, 0), Some(1), "new corner is water");

	// Shrink/crop back: offset -2,-2 recovers the original window.
	p.resize(4, 4, -2, -2).unwrap();
	let top = p.cell(0, 0).unwrap()[layer].unwrap();
	assert_eq!(p.packs[top.pack as usize].ids[top.tile as usize], "GLa000");
	assert_eq!(p.pass_override(0, 0), Some(3));

	assert!(p.resize(0, 8, 0, 0).is_err(), "rejects zero dimension");
}

#[test]
fn pass_paint_drag_is_one_undo_unit() {
	let root = assets_root();
	let mut p = Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap();
	let before = p.hash();
	p.begin_stroke();
	p.set_pass(0, 0, 2);
	p.set_pass(1, 0, 2);
	p.set_pass(2, 0, 2);
	p.end_stroke();
	assert!(p.undo(), "the whole drag undoes at once");
	assert_eq!(p.hash(), before);
}

#[test]
fn bake_accepts_rectangular_maps() {
	// Any rectangle is a valid WRL (confirmed 2026-06) - width/height
	// are independent throughout.
	let p = Project::new(8, 6, &["GREEN".to_string()], &assets_root(), 42).unwrap();
	let wrl = crate::bake(&p).unwrap();
	assert_eq!((wrl.width, wrl.height), (8, 6));
}

#[test]
fn palette_edits_are_undoable_and_round_trip_through_save() {
	let root = assets_root();
	let mut p = Project::new(8, 8, &["GREEN".to_string()], &root, 42).unwrap();
	let before = p.hash();

	// Static slots refuse edits; dynamic accept and change the hash.
	assert!(p.set_color(32, [1, 2, 3]).is_err());
	assert!(p.set_color(200, [1, 2, 3]).is_err());
	assert!(p.set_color(100, [10, 20, 30]).unwrap());
	assert!(p.dirty());
	assert_ne!(p.hash(), before, "palette is document state");
	assert!(!p.set_color(100, [10, 20, 30]).unwrap(), "no-op edit");

	// Saved as a sparse override block; reload reproduces the palette.
	// (`"palette": {` is the override block - `"palette": true` in the
	// `use` entries is the unrelated owner flag.)
	let text = p.save_string();
	assert!(text.contains("\"palette\": {"), "{text}");
	assert!(text.contains("\"100\": \"#0a141e\""), "{text}");
	let reloaded = Project::from_str(&text, &root).unwrap();
	assert_eq!(reloaded.palette[300..303], [10, 20, 30]);
	assert_eq!(reloaded.hash(), p.hash());

	// Undo restores the pack color - and the override block disappears.
	assert!(p.undo());
	assert_eq!(p.hash(), before);
	assert!(!p.save_string().contains("\"palette\": {"));
	assert!(p.redo());
	assert_eq!(p.palette[300..303], [10, 20, 30]);

	// Overrides outside the dynamic range are rejected at load.
	let bad = text.replace("\"100\"", "\"32\"");
	assert!(Project::from_str(&bad, &root).is_err());
}

#[test]
fn static_slots_resolve_to_the_in_game_palette() {
	let root = assets_root();
	let p = Project::new(8, 8, &["GREEN".to_string()], &root, 42).unwrap();
	// Every static slot carries the game value (pack bytes there are
	// converter leftovers the engine would ignore anyway).
	for slot in 0..256usize {
		if (64..=159).contains(&slot) {
			continue;
		}
		assert_eq!(p.palette[slot * 3..slot * 3 + 3], crate::GAME_PALETTE[slot * 3..slot * 3 + 3], "slot {slot}",);
	}
	// Dynamic slots stay pack-owned (not the FF00FF placeholders).
	assert_ne!(p.palette[64 * 3..64 * 3 + 3], [0xff, 0x00, 0xff]);
	// Statics never count as overrides in the save.
	assert!(!p.save_string().contains("\"palette\": {"));
}

#[test]
fn hsl_block_shift_retints_one_water_cycle() {
	let root = assets_root();
	let mut p = Project::new(8, 8, &["GREEN".to_string()], &root, 42).unwrap();
	let before = p.hash();
	let snapshot = p.palette.clone();

	assert!(p.hsl_shift_block(110, 40.0, 0.0, 0.1).unwrap());
	// Only the 110–116 block changed.
	for slot in 0..256usize {
		let same = p.palette[slot * 3..slot * 3 + 3] == snapshot[slot * 3..slot * 3 + 3];
		if (110..=116).contains(&slot) {
			assert!(
				!same || {
					// A grey could map to itself; tolerate but don't expect.
					true
				}
			);
		} else {
			assert!(same, "slot {slot} must be untouched");
		}
	}
	assert_ne!(p.hash(), before);

	// The whole block re-tint is ONE undo step.
	assert!(p.undo());
	assert_eq!(p.hash(), before);
	assert_eq!(p.palette, snapshot);

	// Non-water slots refuse the block tool.
	assert!(p.hsl_shift_block(70, 10.0, 0.0, 0.0).is_err());
	assert!(p.hsl_shift_block(9, 10.0, 0.0, 0.0).is_err(), "game animated is fixed");
}

#[test]
fn transform_ops_match_pixel_operations() {
	// A recognizable asymmetric 64×64 test tile.
	let mut src = [0u8; TILE_DATA_SIZE];
	for y in 0..64usize {
		for x in 0..64usize {
			src[y * 64 + x] = ((x * 7 + y * 13) % 251) as u8;
		}
	}
	let rot_cw = |p: &[u8; TILE_DATA_SIZE]| {
		let mut out = [0u8; TILE_DATA_SIZE];
		for y in 0..64usize {
			for x in 0..64usize {
				out[y * 64 + x] = p[(63 - x) * 64 + y];
			}
		}
		out
	};
	let flip_h = |p: &[u8; TILE_DATA_SIZE]| {
		let mut out = [0u8; TILE_DATA_SIZE];
		for y in 0..64usize {
			for x in 0..64usize {
				out[y * 64 + x] = p[y * 64 + (63 - x)];
			}
		}
		out
	};
	let flip_v = |p: &[u8; TILE_DATA_SIZE]| {
		let mut out = [0u8; TILE_DATA_SIZE];
		for y in 0..64usize {
			for x in 0..64usize {
				out[y * 64 + x] = p[(63 - y) * 64 + x];
			}
		}
		out
	};

	for rot in 0..4u8 {
		for mirror in [false, true] {
			let t = Transform { rot, mirror };
			let base = transform_tile(&src, t);
			assert_eq!(transform_tile(&src, t.rotated_cw()), rot_cw(&base), "{t:?} cw");
			assert_eq!(transform_tile(&src, t.rotated_cw().rotated_ccw()), base, "{t:?} cw∘ccw = id",);
			assert_eq!(transform_tile(&src, t.flipped_h()), flip_h(&base), "{t:?} flip h");
			assert_eq!(transform_tile(&src, t.flipped_v()), flip_v(&base), "{t:?} flip v");
		}
	}
}

/// `compose` is exactly transform-then-transform on pixels, for all 64
/// pairs.
#[test]
fn compose_matches_pixel_chaining() {
	let mut src = [0u8; TILE_DATA_SIZE];
	for y in 0..64usize {
		for x in 0..64usize {
			src[y * 64 + x] = ((x * 7 + y * 13) % 251) as u8;
		}
	}
	for ra in 0..4u8 {
		for ma in [false, true] {
			for rb in 0..4u8 {
				for mb in [false, true] {
					let outer = Transform { rot: ra, mirror: ma };
					let inner = Transform { rot: rb, mirror: mb };
					let chained = transform_tile(&transform_tile(&src, inner), outer);
					assert_eq!(transform_tile(&src, outer.compose(inner)), chained, "{outer:?} ∘ {inner:?}",);
				}
			}
		}
	}
}

/// `inverse` undoes a transform from both sides, and `screen_to_base` is a
/// permutation of the 4 directions consistent with `compose`.
#[test]
fn transform_inverse_and_screen_to_base() {
	for rot in 0..4u8 {
		for mirror in [false, true] {
			let t = Transform { rot, mirror };
			let inv = t.inverse();
			assert_eq!(t.compose(inv), Transform::default(), "{t:?} ∘ inv");
			assert_eq!(inv.compose(t), Transform::default(), "inv ∘ {t:?}");
			// screen_to_base is a bijection over {N,E,S,W}.
			let mapped: Vec<usize> = (0..4).map(|d| t.screen_to_base(d)).collect();
			let mut seen = [false; 4];
			for &m in &mapped {
				assert!(!seen[m], "{t:?} screen_to_base not a permutation");
				seen[m] = true;
			}
			// Inverse direction map round-trips: base_to_screen ∘ screen_to_base = id.
			for d in 0..4 {
				assert_eq!(inv.screen_to_base(t.screen_to_base(d)), d, "{t:?} dir round-trip");
			}
		}
	}
}

#[test]
fn pack_prefix_tops_up_short_names_with_vowels_then_x() {
	assert_eq!(pack_prefix("GREEN_1"), "GRN", "three consonants win");
	assert_eq!(pack_prefix("GO"), "GOX", "vowels top up, X pads the rest");
	assert_eq!(pack_prefix("A"), "AXX");
	assert_eq!(pack_prefix("42"), "XXX", "no letters at all -> all X");
}

#[test]
fn transform_parse_rejects_unknown_suffixes() {
	let err = Transform::parse("Q").unwrap_err();
	assert!(err.contains("bad transform 'Q'"), "{err}");
	assert!(Transform::parse("!Q").is_err(), "the mirror prefix doesn't rescue a bad letter");
}

/// Write a minimal loadable pack: `tiles` zero-filled tiles with
/// `{prefix}NNN` ids, plus a bare-array palette when the pack owns one.
fn write_min_pack(dir: &Path, prefix: &str, tiles: usize, with_palette: bool) {
	std::fs::create_dir_all(dir).unwrap();
	std::fs::write(dir.join("tiles-data.bin"), vec![0u8; TILE_DATA_SIZE * tiles]).unwrap();
	let ids: Vec<String> = (0..tiles).map(|i| format!("\"{prefix}{i:03}\"")).collect();
	std::fs::write(dir.join("tiles-data.json"), format!("[{}]", ids.join(","))).unwrap();
	if with_palette {
		let colors: Vec<String> = (0..256).map(|i| format!("\"#{i:02x}0010\"")).collect();
		std::fs::write(dir.join("palette.json"), format!("[{}]", colors.join(","))).unwrap();
	}
}

/// A `user/tilepacks/<NAME>` pack mirroring a loaded stock pack joins the
/// roster after the stock packs, flagged `user`, and its ids resolve.
#[test]
fn user_packs_matching_a_stock_pack_join_the_roster() {
	let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/mc-cov-userpacks");
	let _ = std::fs::remove_dir_all(&root);
	let assets = root.join("assets/tilepacks");
	write_min_pack(&assets.join("WATER"), "WTR", 2, false);
	write_min_pack(&assets.join("OWNER"), "OLA", 1, true);
	write_min_pack(&root.join("user/tilepacks/OWNER"), "OLB", 1, false);
	// An unrelated user pack (no matching stock pack) must NOT join.
	write_min_pack(&root.join("user/tilepacks/STRAY"), "SLA", 1, false);

	let p = Project::new(2, 2, &["OWNER".to_string()], &assets, 1).unwrap();
	assert_eq!(p.packs.len(), 3, "stock WATER + OWNER + the user OWNER");
	assert!(p.packs[2].user && p.packs[2].name == "OWNER", "the user pack is appended, flagged user");
	let (tref, _) = p.resolve_ref("OLB000").expect("user-pack ids resolve");
	assert_eq!(tref.pack, 2);
	assert!(!p.packs.iter().any(|pk| pk.name == "STRAY"), "a stray user pack stays out");
	std::fs::remove_dir_all(&root).ok();
}

/// An assets root with no grandparent directory (a bare one-component path)
/// simply has no user-pack location - the project still loads.
#[test]
fn rootless_assets_dir_skips_the_user_pack_scan() {
	// Relative single-component path: parent is "", which has no parent.
	let root = Path::new(".mc-cov-noparent");
	let _ = std::fs::remove_dir_all(root);
	write_min_pack(&root.join("WATER"), "WTR", 1, true);
	let p = Project::new(1, 1, &[], root, 1).expect("loads without a user-pack root");
	assert_eq!(p.packs.len(), 1, "no user packs joined");
	std::fs::remove_dir_all(root).ok();
}

#[test]
fn new_rejects_a_tileless_water_pack() {
	let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/mc-cov-emptywater");
	let _ = std::fs::remove_dir_all(&root);
	write_min_pack(&root.join("WATER"), "WTR", 0, true);
	let err = Project::new(2, 2, &[], &root, 1).err().unwrap();
	assert!(err.contains("WATER pack has no tiles"), "{err}");
	std::fs::remove_dir_all(&root).ok();
}

#[test]
fn place_many_skips_offmap_and_unresolvable_edits() {
	let root = assets_root();
	let mut p = Project::new(2, 1, &["GREEN".to_string()], &root, 1).unwrap();
	let rev = p.revision();
	let t = |pack, tile| Some(TileRef { pack, tile, transform: Transform::default() });
	let changed = p.place_many(&[
		(5, 0, LAYER_GROUND, t(1, 0)),     // off the map
		(0, 0, LAYER_GROUND, t(200, 0)),   // no such pack
		(0, 0, LAYER_GROUND, t(1, 60000)), // no such tile
	]);
	assert!(!changed, "every edit was skipped");
	assert_eq!(p.revision(), rev, "a skipped batch bumps nothing");
}

#[test]
fn load_palette_rejects_wrong_lengths() {
	let mut p = Project::new(2, 1, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	let err = p.load_palette(&[0u8; 10]).unwrap_err();
	assert!(err.contains("10 bytes, want 768"), "{err}");
}

/// `rollback_stroke` reverts the open stroke without leaving an undo entry;
/// an empty stroke rolls back to nothing.
#[test]
fn rollback_stroke_reverts_without_journalling() {
	let root = assets_root();
	let mut p = Project::new(4, 4, &["GREEN".to_string()], &root, 1).unwrap();
	p.begin_stroke();
	assert!(!p.rollback_stroke(), "an empty stroke is a no-op rollback");
	let before = p.hash();
	let (tile, layer) = p.resolve_ref("GLa000").unwrap();
	p.begin_stroke();
	assert!(p.place(1, 1, layer, Some(tile)));
	assert!(p.rollback_stroke(), "the open stroke was reverted");
	assert_eq!(p.hash(), before, "the edit never happened");
	assert!(!p.undo(), "nothing landed on the undo stack");
}

#[test]
fn random_variant_leaves_groupless_tiles_alone() {
	// A WRL-import pack ships no variant groups at all.
	let p = Project::empty();
	let t = TileRef { pack: 0, tile: 0, transform: Transform::default() };
	let mut rng = Rng::new(1);
	assert_eq!(p.random_variant(t, &mut rng), t, "no variant group -> the tile itself");
}

#[test]
fn fill_off_the_map_is_a_noop() {
	let mut p = Project::new(2, 1, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	let (tile, layer) = p.resolve_ref("GLa000").unwrap();
	let mut rng = Rng::new(0);
	assert!(!p.fill(9, 9, tile, layer, false, &mut rng));
}

#[test]
fn set_pass_many_skips_invalid_and_redundant_edits() {
	let mut p = Project::new(2, 1, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	assert!(!p.set_pass_many(&[(5, 0, 2), (0, 0, 9)]), "off-map and out-of-range values are skipped");
	assert!(p.set_pass(0, 0, 2));
	assert!(!p.set_pass(0, 0, 2), "an already-set override is a no-op");
}

#[test]
fn set_pass_override_validates_clears_and_joins_strokes() {
	let mut p = Project::new(2, 1, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	assert!(!p.set_pass_override(9, 0, Some(1)), "off the map");
	assert!(!p.set_pass_override(0, 0, Some(9)), "pass values stop at 3");
	assert!(!p.set_pass_override(0, 0, None), "clearing an unset override is a no-op");
	assert!(p.set_pass_override(0, 0, Some(2)));
	assert_eq!(p.pass_override(0, 0), Some(2));
	// Within a stroke the edit joins the open unit: one undo clears both.
	p.begin_stroke();
	assert!(p.set_pass_override(0, 0, None), "explicit clear");
	assert!(p.set_pass_override(1, 0, Some(3)));
	p.end_stroke();
	assert!(p.undo());
	assert_eq!(p.pass_override(0, 0), Some(2), "the stroke undid as one unit");
	assert_eq!(p.pass_override(1, 0), None);
}

/// The pass tally counts what the map *is*: every cell through `pass_at`, so
/// an override is counted as the value it forces, not as the tile's own.
#[test]
fn pass_counts_tally_every_cell_and_its_overrides() {
	let mut p = Project::new(4, 2, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	let (before, overrides) = p.pass_counts();
	assert_eq!(before.iter().sum::<u32>(), 8, "every cell of a 4x2 map is tallied");
	assert_eq!(overrides, 0, "a fresh map carries no overrides");
	// Whatever the two cells read as before - a fresh map's fill is the pack's,
	// not this test's business.
	let (was_a, was_b) = (p.pass_at(0, 0).unwrap(), p.pass_at(1, 0).unwrap());

	assert!(p.set_pass_override(0, 0, Some(1)));
	assert!(p.set_pass_override(1, 0, Some(3)));
	let (after, overrides) = p.pass_counts();
	assert_eq!(overrides, 2, "both overridden cells are counted as such");
	assert_eq!(after.iter().sum::<u32>(), 8, "the tally still covers the whole map");
	let moved = |v: u8| after[v as usize] as i64 - before[v as usize] as i64;
	let expect = |v: u8| i64::from(v == 1) + i64::from(v == 3) - i64::from(was_a == v) - i64::from(was_b == v);
	for v in 0..4u8 {
		assert_eq!(moved(v), expect(v), "pass {v}: the overrides moved both cells to the value they force");
	}
}

#[test]
fn clear_pass_overrides_drops_every_override_as_one_unit() {
	let mut p = Project::new(3, 1, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	assert!(!p.clear_pass_overrides(), "nothing set -> nothing cleared");
	assert!(p.set_pass(0, 0, 2) && p.set_pass(2, 0, 3));
	assert!(p.clear_pass_overrides());
	assert_eq!((p.pass_override(0, 0), p.pass_override(2, 0)), (None, None));
	assert!(p.undo(), "the wipe is one undo unit");
	assert_eq!((p.pass_override(0, 0), p.pass_override(2, 0)), (Some(2), Some(3)));
	// Inside a stroke the wipe joins the open unit.
	p.begin_stroke();
	assert!(p.clear_pass_overrides());
	assert!(p.set_pass(1, 0, 1));
	p.end_stroke();
	assert!(p.undo());
	assert_eq!(p.pass_override(0, 0), Some(2), "stroke-joined wipe restored");
	assert_eq!(p.pass_override(1, 0), None);
}

/// A stroke records a tile's original pass only once, however many times
/// the drag repaints it - undo restores the pre-stroke value, not an
/// intermediate one.
#[test]
fn tile_pass_stroke_keeps_the_first_previous_value() {
	let mut p = Project::new(2, 1, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	let (land, layer) = p.resolve_ref("GLa000").unwrap();
	assert!(p.place(0, 0, layer, Some(land)));
	let original = p.pass_at(0, 0).unwrap();
	assert!(!p.set_tile_pass_at(0, 0, 9), "pass values stop at 3");
	p.begin_stroke();
	assert!(p.set_tile_pass_at(0, 0, 3));
	assert!(p.set_tile_pass_at(0, 0, 2), "the same tile repainted within the stroke");
	p.end_stroke();
	assert_eq!(p.pass_at(0, 0), Some(2));
	assert!(p.undo());
	assert_eq!(p.pass_at(0, 0), Some(original), "undo lands on the pre-stroke pass");
}

#[test]
fn set_base_tile_validates_range_and_position() {
	let mut p = Project::empty(); // 1×1, one tile
	assert!(!p.set_base_tile(5, 5, 0), "off the map");
	assert!(!p.set_base_tile(0, 0, 9), "tile index past the base pack");
	assert!(p.set_base_tile(0, 0, 0), "a valid write lands on the base layer");
	assert_eq!(p.base_tile(0, 0), Some(0));
	assert!(!p.set_base_tile(0, 0, 0), "rewriting the same tile is a no-op");
}

#[test]
fn delete_tile_rejects_bad_pack_and_tile_indices() {
	let mut p = Project::new(2, 1, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	assert!(p.delete_tile(99, 0).unwrap_err().contains("no pack"));
	assert!(p.delete_tile(1, 60000).unwrap_err().contains("out of range"));
}

#[test]
fn offmap_reads_are_none_or_zero() {
	let p = Project::new(2, 1, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	assert!(p.cell(9, 9).is_none());
	assert_eq!(p.compose_cell(9, 9), [0u8; TILE_DATA_SIZE], "off-map cells compose to zeroes");
	assert_eq!(p.pass_override(9, 9), None);
}

/// A true hole (both layers empty) reads as pixel 0 and land pass 0.
#[test]
fn empty_stacks_read_as_zero_pixel_and_land_pass() {
	let mut p = Project::new(2, 1, &["GREEN".to_string()], &assets_root(), 1).unwrap();
	assert!(p.place(0, 0, LAYER_WATER, None), "drop the water base");
	assert_eq!(p.pixel_at(0, 0, (32, 32)), 0, "an empty stack has no pixels");
	assert_eq!(p.pass_at(0, 0), Some(0), "empty stacks read as land");
}

#[test]
fn stroke_groups_edits_into_one_undo_unit() {
	let root = assets_root();
	let mut p = Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap();
	let before = p.hash();
	let (tile, layer) = p.resolve_ref("GLa000").unwrap();

	p.begin_stroke();
	assert!(p.place(2, 2, layer, Some(tile)));
	assert!(p.place(3, 2, layer, Some(tile)));
	assert!(p.place(4, 2, layer, Some(tile)));
	p.end_stroke();
	let painted = p.hash();
	assert_ne!(before, painted);

	assert!(p.undo(), "stroke undoes as one unit");
	assert_eq!(p.hash(), before);
	assert!(!p.undo(), "nothing left to undo");

	assert!(p.redo());
	assert_eq!(p.hash(), painted);

	// An empty stroke leaves no undo entry behind.
	p.begin_stroke();
	p.end_stroke();
	assert!(p.undo());
	assert_eq!(p.hash(), before);
}

/// Stage C3/D: resources paint onto a save-less project (the cargo map
/// materializes), and `synthesize_save` builds + attaches a real V71 save
/// from the project alone — carrying the painted resources, deriving the
/// mining station's initial mining from the cells under its footprint, and
/// leaving the project export-ready.
#[test]
fn synthesize_save_from_scratch_carries_resources_and_mining() {
	use max_assets::attribs::{CargoType, FrameInfo, UnitAttributes, UnitMeta, UnitStatsDb};
	use max_assets::save::{UNIT_END, cargo_compose};

	let mut p = Project::empty();
	p.resize(16, 12, 0, 0).unwrap();
	// Stage D: painting resources needs no attached save.
	let raw8 = cargo_compose(0, Some(max_assets::save::CargoMaterial::Raw), 8);
	let fuel5 = cargo_compose(0, Some(max_assets::save::CargoMaterial::Fuel), 5);
	assert!(p.set_cargo(3, 3, raw8), "cargo map materializes on first paint");
	assert!(p.set_cargo(4, 3, fuel5));
	assert_eq!(p.cargo_at(3, 3), Some(raw8));

	// A mining station over the resources + a tank, both team 0 (RED).
	p.place_object(MapObject {
		unit_type: max_assets::save::unit_type_id("MININGST").unwrap(),
		x: 3,
		y: 3,
		team: 0,
		props: ObjectProps { connectors: 0xFF, ..ObjectProps::default() },
	});
	p.place_object(MapObject {
		unit_type: max_assets::save::unit_type_id("TANK").unwrap(),
		x: 8,
		y: 8,
		team: 0,
		props: ObjectProps::default(),
	});

	// A synthetic unit database + frame table (no game files).
	let mut meta = [UnitMeta::default(); UNIT_END];
	meta[max_assets::save::unit_type_id("MININGST").unwrap() as usize] =
		UnitMeta { flags: 0x10 | 0x200, cargo_type: CargoType::Raw, ..Default::default() };
	meta[max_assets::save::unit_type_id("TANK").unwrap() as usize] =
		UnitMeta { flags: 0x100, cargo_type: CargoType::None, ..Default::default() };
	let db = UnitStatsDb {
		base: std::array::from_fn(|_| {
			max_assets::attribs::unit_values_from_attributes(&UnitAttributes {
				hit_points: 12,
				scan_range: 3,
				..Default::default()
			})
		}),
		clans: Default::default(),
		meta,
		source: std::path::PathBuf::from("synthetic"),
	};
	let frames = std::array::from_fn(|_| Some(FrameInfo { image_count: 8, ..Default::default() }));

	let opts = SynthesizeSaveOptions {
		save_name: "ScratchSave".into(),
		world_index: 12,
		team_clans: [1, 2, 3, 4, 0],
		start_gold: 150,
		rng_seed: 7,
	};
	let summary = p.synthesize_save(&opts, &db, &frames).expect("synthesis succeeds");
	assert_eq!((summary.units, summary.teams), (2, 1));

	let embedded = p.save.as_ref().expect("the synthesized save is attached");
	let file = &embedded.file;
	assert_eq!(file.header.save_name, "ScratchSave");
	assert_eq!(file.header.team_type, [1, 0, 0, 0, 0], "only RED plays");
	assert_eq!(file.cargo_map[3 * 16 + 3], raw8, "painted resources carried into the save");
	// The station's initial mining derives from its footprint (raw 8 +
	// fuel 5), exactly like `UnitsManager_SetInitialMining`: fuel_mining
	// 2 (the POWGEN rate), raw_mining 8, then the remaining 3 fuel →
	// fuel_mining 5, gold 0; total = 8 + 5 + 0 = 13. Caps: raw 8 / gold 0
	// / fuel 5.
	let station_slot = file.stationary[0];
	let layout = file.object_meta[station_slot].unit_layout.as_ref().unwrap();
	let mining = &file.object_meta[station_slot].body_raw[layout.build_time + 1..layout.build_time + 8];
	assert_eq!(mining, &[13, 8, 5, 0, 8, 0, 5], "total/raw/fuel/gold mining + raw/gold/fuel caps");
	// Export-ready: the attached synthesized save passes the S6.6 guard.
	assert!(p.save_exports_losslessly(), "synthesized bytes re-serialize byte-exact");

	// Attaching re-seeded `objects` wholesale, so the placement patches that
	// got us here describe a different document. They must not still be on the
	// journal: object patches swap the whole vector, so one undo would restore
	// the pre-attach list, stripping the `source_id`s the save seeded - and
	// `export_save`'s delete pass would then drop every seeded unit from the
	// file as "deleted by the user".
	let seeded: Vec<Option<u16>> = p.objects.iter().map(|o| o.props.source_id).collect();
	assert!(seeded.iter().all(Option::is_some), "every object is anchored to a save unit");
	assert!(!p.undo(), "the journal does not survive attaching a save");
	assert_eq!(
		p.objects.iter().map(|o| o.props.source_id).collect::<Vec<_>>(),
		seeded,
		"undo across the attach boundary must not restore the pre-attach objects"
	);
}

/// Security regression: `check_name_component` is the one gate between a name
/// a *document* supplies and a path component the editor joins onto a base
/// directory. Both escapes it must stop: `..` walks out of the base, and an
/// absolute name replaces it outright (`Path::join` discards the base when
/// handed one), so an imported file could pick the write target.
///
/// It deliberately checks path shape, not a charset - a WRL-import pack is
/// named after the file stem, so spaces and mixed case have to survive.
#[test]
fn check_name_component_refuses_only_what_escapes_the_directory() {
	use super::check_name_component;

	for bad in [
		"..",
		".",
		"",
		"a/b",
		"a\\b",
		"/etc",
		"/home/u/.config/autostart",
		"../../../../tmp/ESCAPED",
		"./x",
		"x/",
		"a\0b",
	] {
		let e = check_name_component("use entry", bad).expect_err(&format!("{bad:?} must be refused"));
		assert!(e.contains("illegal name"), "{bad:?}: {e}");
	}

	for ok in ["GREEN", "SNOW_DARK", "WATER", "My Map", "import_extra", "GREEN+DESERT", "a.b", "..x", "x..", "-x"] {
		check_name_component("use entry", ok).unwrap_or_else(|e| panic!("{ok:?} must be accepted: {e}"));
	}
}
