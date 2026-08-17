use super::*;

fn resources() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources")
}

#[test]
fn shape_land_mask_reads_blue_as_water() {
	// 2×1: blue pixel → water, green pixel → land.
	let rgba = [0, 0, 255, 255, 0, 200, 0, 255];
	assert_eq!(shape_land_mask(&rgba, 2, 1, 2, 1), vec![false, true]);
	// Black and white are not "blue" → land (only blue means water).
	let bw = [0, 0, 0, 255, 255, 255, 255, 255];
	assert_eq!(shape_land_mask(&bw, 2, 1, 2, 1), vec![true, true]);
	// Downsample 4×1 (3 blue + 1 green) into one tile → water majority.
	let mixed = [0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 200, 0, 255];
	assert_eq!(shape_land_mask(&mixed, 4, 1, 1, 1), vec![false]);
	// A 50/50 tie defaults to water (the new map's base fill).
	let tie = [0, 0, 255, 255, 0, 200, 0, 255];
	assert_eq!(shape_land_mask(&tie, 2, 1, 1, 1), vec![false]);
	// Upsample: a 1×1 green image fills a 2×2 land map (every tile sampled).
	assert_eq!(shape_land_mask(&[0, 200, 0, 255], 1, 1, 2, 2), vec![true, true, true, true]);
}

#[test]
fn nearest_palette_index_matches_closest_and_skips_slot_0() {
	// Palette: slot 0 black (transparent slot), 1 red, 2 green, 3 blue.
	let mut pal = vec![0u8; 768];
	pal[3..6].copy_from_slice(&[255, 0, 0]);
	pal[6..9].copy_from_slice(&[0, 255, 0]);
	pal[9..12].copy_from_slice(&[0, 0, 255]);
	assert_eq!(nearest_palette_index(&pal, 250, 10, 10), 1, "near-red -> red");
	assert_eq!(nearest_palette_index(&pal, 10, 240, 5), 2, "near-green -> green");
	// Pure black is closest to slot 0, but slot 0 is skipped, so it falls to
	// the next-nearest real color rather than mapping to "transparent".
	assert_ne!(nearest_palette_index(&pal, 0, 0, 0), 0);
}

#[test]
fn template_map_entries_label_name_no_filename_note() {
	let entries = template_map_entries(&resources().join("assets/maps"));
	assert!(!entries.is_empty(), "the shipped stock maps");
	// Map name as the label (GREEN_1.json is "New Luzon"); no right-aligned
	// file name - the Template Maps submenu groups by terrain column instead.
	let green = entries.iter().find(|e| e.path.file_stem().is_some_and(|s| s == "GREEN_1")).expect("GREEN_1");
	assert_eq!(green.label, "New Luzon");
	assert_eq!(green.note, None);
}

fn editor() -> EditorState {
	let resources = resources();
	let project = Project::new(8, 8, &["GREEN".to_string()], &resources.join("assets/tilepacks"), 1).unwrap();
	EditorState::new(project, (800, 600), None, resources)
}

/// The whole authoring round-trip, with the writes pointed at `temp/` rather
/// than the real install: author from an image, then export, rename, delete
/// and re-import the piece - and check the shipped library is never touched
/// and that the user's half is the only thing written back.
#[test]
fn a_cut_out_can_be_authored_shared_renamed_and_deleted() {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/scenery_authoring");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let real = resources();
	let project = Project::new(8, 8, &["GREEN".to_string()], &real.join("assets/tilepacks"), 1).unwrap();
	// A real project, but a scratch resources root - so every write below
	// lands under temp/ and the shipped bake is out of reach.
	let mut e = EditorState::new(project, (800, 600), None, dir.clone());
	let user_lib = dir.join("user/scenery/GREEN");

	// A 96x96 image: a 40x40 opaque block (one cell's worth of ink) and a
	// band of half-alpha shadow beside it.
	let (w, h) = (96usize, 96usize);
	let mut rgba = vec![0u8; w * h * 4];
	for y in 8..48 {
		for x in 8..48 {
			rgba[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[40, 160, 40, 255]);
		}
	}
	for y in 60..70 {
		for x in 60..70 {
			rgba[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[0, 0, 0, 150]);
		}
	}
	let (sprite, pass, cw, ch) = map_core::rasterize(&rgba, w, h, &e.project.palette, &map_core::RasterOpts::default())
		.expect("the block survives the thresholds");
	assert_eq!((cw, ch), (2, 2), "the footprint is the source image in cells");
	assert!(sprite.body.iter().any(|&b| b != 0) && sprite.shade.iter().any(|&s| s != 0), "both planes carry ink");

	assert!(matches!(
		e.scenery_commit("GREEN".into(), "oak-stand".into(), "Oak Stand".into(), sprite, pass, (cw, ch), None, None),
		Outcome::DocReplaced
	));
	let armed = e.armed_scenery().expect("the new piece is armed for placing");
	assert_eq!(armed, ("GREEN".to_string(), "oak-stand".to_string()));
	let lib = map_core::SceneryPack::load(&dir.join("user"), "GREEN").expect("the user library was written");
	assert_eq!(lib.pieces.len(), 1);
	// A piece is its own files, under its own id - the layout a user adds a
	// cut-out to by dropping files in.
	assert!(user_lib.join("oak-stand.scn").is_file(), "the piece");
	assert!(user_lib.join("oak-stand.json").is_file(), "and its hand-editable meta");
	assert!(!dir.join("assets/scenery").exists(), "the shipped bake is never written");

	// `Stands: auto` means the relief stays inferred - nothing is written into
	// the meta, no height map is written at all, and the piece measures itself
	// off its own art.
	assert!(!user_lib.join("oak-stand.hgt").exists(), "an inferred relief is no file");
	assert_eq!((lib.pieces[0].peak, lib.pieces[0].sunken), (None, None));
	assert_eq!(lib.pieces[0].height_opts().peak, lib.pieces[0].sprite.default_peak());

	// **A painted height map, all the way through**: a picture in, fitted to the
	// art, filed as the piece's own `.hgt`, and read back as the relief.
	let (sprite, pass, cw, ch) = map_core::rasterize(&rgba, w, h, &e.project.palette, &map_core::RasterOpts::default())
		.expect("the block survives the thresholds");
	let height_png = dir.join("oak.height.png");
	{
		// A ramp over the sprite's own frame, so every row is a different height.
		let (sw, sh) = (sprite.width as usize, sprite.height as usize);
		let grey: Vec<u8> = (0..sw * sh).map(|i| ((i / sw) * 255 / sh.max(1)) as u8).collect();
		let file = std::fs::File::create(&height_png).unwrap();
		let mut enc = png::Encoder::new(std::io::BufWriter::new(file), sw as u32, sh as u32);
		enc.set_color(png::ColorType::Grayscale);
		enc.set_depth(png::BitDepth::Eight);
		enc.write_header().unwrap().write_image_data(&grey).unwrap();
	}
	// The dialog has to be open: a height map is read against the art being
	// authored, and there is no art without a run.
	assert!(matches!(
		e.execute(Command::SceneryHeightImport { path: Some(height_png.clone()) }),
		Outcome::Failed(m) if m.contains("open New/Clone/Edit")
	));
	assert!(matches!(e.execute(Command::SceneryClone), Outcome::OpenDialog(_)), "the armed piece clones");
	assert!(matches!(e.execute(Command::SceneryHeightImport { path: Some(height_png) }), Outcome::Redraw));
	let drawn = e.fit_scenery_height(&sprite, (cw, ch), None).expect("the sprite's own frame fits");
	assert!(matches!(
		e.scenery_commit(
			"GREEN".into(),
			"oak-hill".into(),
			"Oak Hill".into(),
			sprite,
			pass,
			(cw, ch),
			None,
			Some(drawn)
		),
		Outcome::DocReplaced
	));
	assert!(user_lib.join("oak-hill.hgt").is_file(), "a drawn relief is a file of its own");
	let lib = map_core::SceneryPack::load(&dir.join("user"), "GREEN").expect("the user library reads back");
	let hill = lib.piece("oak-hill").expect("the piece is in it");
	assert!(hill.height_authored(), "and it comes back as authored, not inferred");
	// It really is the ramp: the object's top row stands lower than its bottom.
	let field = hill.height_field(&[0; 256]);
	let w_px = hill.sprite.width as usize;
	let body = |y: usize| (0..w_px).filter(|&x| hill.sprite.body[y * w_px + x] != 0).map(|x| field[y * w_px + x]).max();
	// Between the object's own first and last rows - the sprite is taller than
	// the body, because the cast shadow is part of the box and carries no relief.
	let rows: Vec<usize> = (0..hill.sprite.height as usize).filter(|&y| body(y).is_some()).collect();
	let (first, last) = (rows[0], rows[rows.len() - 1]);
	let (top, bottom) = (body(first).unwrap_or(0), body(last).unwrap_or(0));
	assert!(top < bottom, "the painted ramp survived the trip: row {first} = {top}, row {last} = {bottom}");
	e.scenerypaint = None;
	e.active_scenery = crate::scenery::index_of(&e.project, "GREEN", "oak-stand");

	// Share it, and read it back: a `.scn` is the piece, whole.
	let scn = dir.join("oak.scn");
	assert!(matches!(e.execute(Command::SceneryExport { path: Some(scn.clone()) }), Outcome::Redraw));
	let (shared, hint) = map_core::read_scn(&std::fs::read(&scn).unwrap()).expect("a readable .scn");
	assert_eq!((shared.id.as_str(), hint.as_str()), ("oak-stand", "GREEN"));

	// Rename moves the display name and leaves the id (what a placement
	// stores) exactly where it was.
	assert!(matches!(e.execute(Command::SceneryRename { name: Some("Grand Oak".into()) }), Outcome::DocReplaced));
	let (_, piece) = crate::scenery::piece_at(&e.project, e.active_scenery.unwrap()).unwrap();
	assert_eq!((piece.name.as_str(), piece.id.as_str()), ("Grand Oak", "oak-stand"));
	assert!(piece.user, "an authored piece is the user's");

	// The bare verb only asks; the forced one does it.
	assert!(matches!(
		e.execute(Command::SceneryDelete { force: false }),
		Outcome::OpenDialog(DialogRequest::DeleteScenery { .. })
	));
	assert_eq!(crate::scenery::piece_count(&e.project), 2, "asking deleted nothing");
	assert!(matches!(e.execute(Command::SceneryDelete { force: true }), Outcome::DocReplaced));
	assert_eq!(crate::scenery::piece_count(&e.project), 1, "the drawn-relief piece is still there");
	assert!(e.active_scenery.is_none(), "the armed piece went with it");

	// And the shared file brings it back.
	assert!(matches!(e.execute(Command::SceneryImport { path: Some(scn) }), Outcome::DocReplaced));
	assert_eq!(crate::scenery::piece_count(&e.project), 2);
	let piece = e.project.scenery_packs[0].piece("oak-stand").expect("the shared file brought it back").clone();
	assert_eq!(piece.id, "oak-stand");
	assert_eq!(piece.name, "Oak Stand", "the .scn carried the name it was exported with");
	let _ = std::fs::remove_dir_all(&dir);
}

/// Shipped cut-outs are read-only without `--dev`, so the destructive verbs
/// and the in-place edit all refuse rather than rewriting the bake - and
/// **clone** is the one that does not, because it is the way in.
#[test]
fn shipped_scenery_refuses_rename_delete_and_edit_outside_dev() {
	let mut e = editor();
	assert!(crate::scenery::piece_count(&e.project) > 0, "the GREEN bake loaded");
	e.active_scenery = Some(0);
	assert!(matches!(e.execute(Command::SceneryDelete { force: true }), Outcome::Failed(m) if m.contains("--dev")));
	assert!(
		matches!(e.execute(Command::SceneryRename { name: Some("X".into()) }), Outcome::Failed(m) if m.contains("--dev"))
	);
	assert!(
		matches!(e.execute(Command::SceneryEdit), Outcome::Failed(m) if m.contains("clone it instead")),
		"a shipped piece is not editable in place",
	);
	assert!(
		matches!(e.execute(Command::SceneryClone), Outcome::OpenDialog(DialogRequest::SceneryNew)),
		"but it always clones",
	);
	let source_id = crate::scenery::piece_at(&e.project, 0).unwrap().1.id.clone();
	let run = e.scenerypaint.as_ref().expect("the clone opened a run");
	assert_eq!(run.mode, crate::scenerypaint::Mode::Clone);
	assert!(run.piece.is_some(), "the clone carries the art it copies");
	assert_ne!(run.id_text, source_id, "and a fresh id: two pieces cannot answer to one name");
	assert!(run.id_text.starts_with(&source_id), "and one that still says what it came from: {}", run.id_text);
	assert!(!run.uses_image(), "there is no image behind it, so the alpha bands do not apply");

	// --dev unlocks the in-place edit, on the piece itself.
	e.dev_mode = true;
	assert!(matches!(e.execute(Command::SceneryEdit), Outcome::OpenDialog(DialogRequest::SceneryNew)));
	let run = e.scenerypaint.as_ref().expect("the edit opened a run");
	assert_eq!(run.mode, crate::scenerypaint::Mode::Edit);
	assert_eq!(run.id_text, source_id, "an edit keeps the id placements point at");
	assert_eq!(run.from.as_ref().map(|f| f.2), Some(false), "and remembers it is shipped art");

	// With nothing armed, every armed-piece verb says so rather than acting
	// on piece 0.
	e.dev_mode = false;
	e.active_scenery = None;
	assert!(
		matches!(e.execute(Command::SceneryDelete { force: true }), Outcome::Failed(m) if m.contains("arm a piece"))
	);
	assert!(matches!(e.execute(Command::SceneryClone), Outcome::Failed(m) if m.contains("arm a piece")));
	assert!(matches!(e.execute(Command::SceneryEdit), Outcome::Failed(m) if m.contains("arm a piece")));
	assert!(matches!(e.execute(Command::SceneryExport { path: Some("x.scn".into()) }), Outcome::Failed(_)));
}

/// A commit that would file a **user** piece under a shipped id is refused:
/// the user root layers over the bake, so that is editing shipped art by the
/// back door, and the clone key hands you a free id instead.
#[test]
fn a_clone_may_not_take_a_shipped_id() {
	let mut e = editor();
	let (_, piece) = crate::scenery::piece_at(&e.project, 0).expect("the GREEN bake loaded");
	let (id, sprite, pass, cw, ch) =
		(piece.id.clone(), piece.sprite.clone(), piece.pass.clone(), piece.cells_w, piece.cells_h);
	let out = e.scenery_commit("GREEN".into(), id.clone(), "Mine".into(), sprite, pass, (cw, ch), None, None);
	assert!(matches!(out, Outcome::Failed(m) if m.contains("shipped piece")), "a shipped id is refused");
	assert!(crate::scenery::piece_at(&e.project, 0).unwrap().1.name != "Mine", "and nothing was written");
}

/// The New Tile dialog offers the map's own tilesets, in its order, without
/// WATER - and without the duplicate a user sidecar pack would produce.
/// Scenery offers those first and then every other installed tileset,
/// because a cut-out is loose art rather than a tile of one map's set.
#[test]
fn the_authoring_pack_list_is_the_maps_tilesets() {
	let e = editor();
	assert_eq!(e.authoring_pack_names(), vec!["GREEN".to_string()]);
	assert!(e.project.uses.iter().any(|u| u.name == "WATER"), "WATER is used, and deliberately not offered");

	let scenery = e.scenery_pack_names();
	assert_eq!(scenery.first().map(String::as_str), Some("GREEN"), "the map's own comes first, and prefills");
	assert!(scenery.len() > 1, "and the rest of the install is offered too: {scenery:?}");
	assert!(scenery.iter().any(|p| p == "SNOW"), "{scenery:?}");
	assert!(!scenery.iter().any(|p| p.starts_with("WATER")), "no water pack holds authorable art");
	let mut sorted = scenery.clone();
	sorted.sort();
	sorted.dedup();
	assert_eq!(sorted.len(), scenery.len(), "each pack once: {scenery:?}");
}

#[test]
fn open_save_anyway_commits_the_pending_project_and_clears_it() {
	let mut e = editor();
	// Park a ready project (as the swapped-map open does when it falls back to
	// the stock world) and confirm "Open Anyway" opens it as a new tab.
	let resources = resources();
	let project = Project::new(6, 4, &["GREEN".to_string()], &resources.join("assets/tilepacks"), 7).unwrap();
	e.pending_save_open = Some(PendingSaveOpen { project, summary: "opened test save".into() });
	let out = e.execute(Command::OpenSaveAnyway);
	assert!(matches!(out, Outcome::DocReplaced), "Open Anyway opens the parked project");
	assert!(e.pending_save_open.is_none(), "the pending open is consumed");
	assert_eq!((e.project.width, e.project.height), (6, 4), "the parked project's dimensions are live");
}

#[test]
fn apply_preferences_persists_paths_and_resets_libraries() {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp").join("prefs_state");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let ini_path = dir.join("mme.ini");
	let mut e = editor();
	e.settings_path = Some(ini_path.clone());
	// Stand in for loaded libraries + a pending "required" prompt.
	e.units_loaded = true;
	e.markers_loaded = true;
	e.paths_prompt_reason = Some("needed".into());
	e.apply_preferences("/games/MAX".into(), "/games/max-port".into(), "/games/max-port/assets".into(), true);
	assert_eq!(e.max_path.as_deref(), Some(Path::new("/games/MAX")));
	assert_eq!(e.max_port_path.as_deref(), Some(Path::new("/games/max-port")));
	assert!(e.skip_path_prompt, "don't-ask-again is applied");
	assert!(e.paths_prompt_reason.is_none(), "the prompt reason clears once paths are provided");
	// MaxPath changed → the game-data libraries drop so they reload.
	assert!(!e.units_loaded && e.units.is_none(), "units reload from the new folder");
	assert!(!e.markers_loaded && e.markers.is_none(), "markers reload from the new folder");
	// The [Paths] section round-trips through the ini.
	let back = ini::INI::from_file(&ini_path).unwrap();
	assert_eq!(back.get_entry::<String>("Paths", "MaxPath").as_deref(), Some("/games/MAX"));
	assert_eq!(back.get_entry::<String>("Paths", "MaxPortPath").as_deref(), Some("/games/max-port"));
	assert_eq!(back.get_entry::<String>("Paths", "MaxPortDataPath").as_deref(), Some("/games/max-port/assets"));
	assert_eq!(back.get_entry::<bool>("Paths", "SkipPathPrompt"), Some(true));
}

#[test]
fn blank_preference_paths_unset_the_folders() {
	let mut e = editor();
	e.max_path = Some(PathBuf::from("/old"));
	e.apply_preferences("   ".into(), String::new(), String::new(), false);
	assert!(e.max_path.is_none(), "a blank path unsets the folder");
	assert!(e.max_port_path.is_none());
	assert!(e.max_port_data_path.is_none());
}

#[test]
fn prompt_paths_opens_a_required_preferences_dialog() {
	let mut e = editor();
	let out = e.prompt_paths("needs a folder");
	assert!(matches!(out, Outcome::OpenDialog(DialogRequest::EditorPreferences)));
	assert_eq!(e.paths_prompt_reason.as_deref(), Some("needs a folder"), "the reason marks it required");
	// A manual (menu) open clears the required marker.
	assert!(matches!(e.execute(Command::PreferencesModal), Outcome::OpenDialog(DialogRequest::EditorPreferences)));
	assert!(e.paths_prompt_reason.is_none(), "a menu open is never required");
}

#[test]
fn open_save_anyway_without_a_pending_open_is_a_noop() {
	let mut e = editor();
	assert!(e.pending_save_open.is_none());
	assert!(matches!(e.execute(Command::OpenSaveAnyway), Outcome::Ok), "no parked open -> nothing happens");
}

/// Open Save File warns before the picker: the menu action / `open-save-warn`
/// resolves to the experimental-warning dialog, not straight to the file
/// dialog. The dialog's confirm command (`file-dialog open-save`) is the shell's.
#[test]
fn open_save_warn_opens_the_experimental_confirm() {
	let mut e = editor();
	assert!(matches!(
		e.execute(Command::OpenSaveWarn),
		Outcome::OpenDialog(DialogRequest::ConfirmExperimentalOpenSave)
	));
	// The menu item and any keybinding fire this command (not `file-dialog
	// open-save` directly), so the warning can't be bypassed.
	assert_eq!(crate::input::action_command("OpenSaveFile"), "open-save-warn");
}

/// Edit Save Data guards its open: no attached save fails with an
/// explanation instead of opening an empty form, and the menu item routes
/// through the action registry.
#[test]
fn edit_save_data_requires_an_open_save() {
	let mut e = editor();
	match e.execute(Command::EditSaveData) {
		Outcome::Failed(m) => assert!(m.contains("no save open"), "{m}"),
		_ => panic!("no attached save must refuse to open the dialog"),
	}
	assert_eq!(crate::input::action_command("EditSaveData"), "edit-save-data");
}

/// Edit Save Data on an attached save: the dialog request carries the
/// extracted settings plus the display context (team types, world,
/// category, clan names). Gated on the V71 fixture (local only).
#[test]
fn edit_save_data_extracts_the_embedded_saves_settings() {
	let saves = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("testdata/saves");
	let (wrl_path, save_path) = (saves.join("GREEN_3-50x50.WRL"), saves.join("save11-green3-50x50.dta"));
	if !wrl_path.is_file() || !save_path.is_file() {
		eprintln!("skipping edit_save_data_extracts_the_embedded_saves_settings: fixtures absent");
		return;
	}
	let wrl = read_wrl_file(&wrl_path).unwrap();
	let project = Project::from_wrl(&wrl, "GREEN3");
	let mut e = EditorState::new(project, (800, 600), None, resources());
	e.project.attach_save(std::fs::read(&save_path).unwrap()).unwrap();

	let Outcome::OpenDialog(DialogRequest::EditSaveData(init)) = e.execute(Command::EditSaveData) else {
		panic!("an attached save opens the dialog");
	};
	assert_eq!(init.settings.save_name, "WIP", "the fixture's save title");
	assert_eq!(init.world, "GREEN_3.WRL");
	assert_eq!(init.game_state, 8);
	assert_eq!(init.clan_names.len(), 9, "Random + eight clans");
	assert_eq!(init.clan_names[0], "Random");
	assert!(init.settings.team_types.iter().any(|&t| t != 0), "the fixture has active teams");

	// Applying an edited block through the shell path updates the project
	// and is undoable under its menu label.
	let mut settings = init.settings.clone();
	settings.options.start_gold = 777;
	let line = e.apply_save_data(&settings).expect("a valid block applies");
	assert!(line.contains("save data updated"), "{line}");
	assert_eq!(e.project.save_settings().unwrap().options.start_gold, 777);
	assert_eq!(e.project.undo_labels(1), vec!["Edit Save Data".to_string()]);
}

/// `export-save-onto` saves a normal map (no attached save) as a `.DTA` by
/// adding its placed units onto a chosen base save, writing the file and
/// reporting the count. Uses the bundled GREEN_3 50×50 world + save fixture.
#[test]
fn export_save_onto_base_writes_a_dta_from_a_normal_map() {
	let saves = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("testdata/saves");
	let (wrl_path, base_path) = (saves.join("GREEN_3-50x50.WRL"), saves.join("save11-green3-50x50.dta"));
	if !wrl_path.is_file() || !base_path.is_file() {
		eprintln!("skipping export_save_onto_base_writes_a_dta_from_a_normal_map: fixtures absent");
		return;
	}
	let wrl = read_wrl_file(&wrl_path).unwrap();
	let project = Project::from_wrl(&wrl, "GREEN3");
	let mut e = EditorState::new(project, (800, 600), None, resources());
	assert!(e.project.save.is_none(), "a from_wrl project has no attached save");
	let dims = (e.project.width, e.project.height);

	// Place one unit of a type the base already carries (so it clones a template).
	let base_raw = std::fs::read(&base_path).unwrap();
	let base = max_assets::save::read_save_bytes(&base_raw, dims).unwrap();
	let (present, base_count) = (base.units().next().unwrap().unit_type, base.units().count());
	e.project.place_object(map_core::MapObject {
		unit_type: present,
		x: 6,
		y: 6,
		team: 0,
		props: map_core::ObjectProps::default(),
	});

	let dir = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp");
	std::fs::create_dir_all(&dir).unwrap();
	let out = dir.join("export_onto_base_test.dta");
	let _ = std::fs::remove_file(&out);
	assert!(matches!(
		e.execute(Command::ExportSaveOnBase { base: base_path.clone(), out: out.clone() }),
		Outcome::Redraw
	));
	assert!(out.is_file(), "the .DTA was written");
	let written = max_assets::save::read_save_bytes(&std::fs::read(&out).unwrap(), dims).unwrap();
	assert_eq!(written.units().count(), base_count + 1, "the placed unit was added onto the base");
	assert!(e.console.log().last().unwrap().contains("added onto"), "the console reports the export");
	let _ = std::fs::remove_file(&out);
}

/// The installed-map-first open path (the swapped-map fix): with a valid map
/// installed at the save's slot whose dimensions match the save, the save
/// opens directly on it. Uses the bundled V71 GREEN_3 50×50 save + its paired
/// swapped world as the "installed" map (in-repo, so this always runs).
#[test]
fn open_save_opens_on_the_installed_map_at_the_slot() {
	let saves = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("testdata/saves");
	let (save, green3) = (saves.join("save11-green3-50x50.dta"), saves.join("GREEN_3-50x50.WRL"));
	if !save.is_file() || !green3.is_file() {
		eprintln!("skipping open_save_opens_on_the_installed_map_at_the_slot: fixtures absent");
		return;
	}
	// A temp "M.A.X. folder" holding the swapped 50×50 GREEN_3 as the slot's
	// installed map — the open must resolve save11's GREEN_3 slot to it.
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp").join("open_save_installed");
	let _ = std::fs::create_dir_all(&dir);
	std::fs::copy(&green3, dir.join("GREEN_3.WRL")).unwrap();
	let mut e = editor();
	e.max_path = Some(dir);
	let out = e.execute(Command::OpenSave { path: save });
	assert!(matches!(out, Outcome::DocReplaced), "the save opens on the installed GREEN_3");
	assert_eq!((e.project.width, e.project.height), (50, 50), "opened at the installed map's dimensions");
	assert!(e.project.save.is_some(), "the save is attached");
	assert!(e.pending_save_open.is_none(), "a clean open needs no Open-Anyway confirm");
}

/// The save editor only edits V71 (M.A.X. Port v0.7.X) saves: an unrecognized
/// version, and a real V70 stock DOS save, both refuse to open with an
/// explanatory Open-Save error dialog rather than loading.
#[test]
fn open_save_refuses_incompatible_versions() {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp").join("open_save_reject");
	let _ = std::fs::create_dir_all(&dir);

	// A file whose leading version word is neither 70 nor 71 → decoded as an
	// unsupported version, surfaced as the incompatible-format dialog.
	let bogus = dir.join("bogus-version.dta");
	std::fs::write(&bogus, 99u16.to_le_bytes()).unwrap();
	let mut e = editor();
	let out = e.execute(Command::OpenSave { path: bogus });
	let Outcome::OpenDialog(DialogRequest::OpenSaveError { message }) = out else {
		panic!("an unsupported version must open the error dialog");
	};
	assert!(message.contains("version 71"), "names the required version: {message}");
	assert!(message.contains("M.A.X. Port v0.7.X"), "cites the format's source: {message}");
	// The garbage "version" a non-save file carries must never be quoted, and
	// only the base name is shown — not the containing directory.
	assert!(!message.contains("version 99"), "must not quote the file's nonsense version: {message}");
	assert!(message.contains("bogus-version.dta"), "names the file: {message}");
	assert!(!message.contains("open_save_reject"), "shows the file name only, not its path: {message}");
	assert!(e.project.save.is_none(), "nothing was loaded");

	// A real V70 stock DOS save (parses fine, but must not be edited here).
	if let Some(home) = std::env::var_os("HOME") {
		let v70 = Path::new(&home).join("MAX/SAVE10.DTA");
		if v70.is_file() {
			let mut e = editor();
			let out = e.execute(Command::OpenSave { path: v70 });
			let Outcome::OpenDialog(DialogRequest::OpenSaveError { message }) = out else {
				panic!("a V70 save must open the error dialog");
			};
			assert!(!message.contains("version 70"), "must not quote the save's version: {message}");
			assert!(message.contains("SAVE10.DTA") && !message.contains("MAX/SAVE10"), "file name only: {message}");
			assert!(e.project.save.is_none(), "the V70 save was not loaded");
		}
	}
}

#[test]
fn quick_load_persists_to_ini_immediately_deduped_top() {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp").join("quickload_state");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let ini_path = dir.join("mme.ini");

	let mut e = editor();
	e.settings_path = Some(ini_path.clone());
	// Opening writes the [QuickLoad] section right away (not only on quit).
	e.remember_recent(Path::new("/maps/alpha.json"));
	let read0 = |p: &Path| ini::INI::from_file(p).unwrap().get_section("QuickLoad").unwrap().get_entry::<String>("0");
	assert_eq!(read0(&ini_path).as_deref(), Some("/maps/alpha.json"), "persisted on open");

	// Dedup + move-to-top: re-opening alpha (after beta) puts it back at key 0.
	e.remember_recent(Path::new("/maps/beta.json"));
	e.remember_recent(Path::new("/maps/alpha.json"));
	let ini = ini::INI::from_file(&ini_path).unwrap();
	let qs = ini.get_section("QuickLoad").unwrap();
	assert_eq!(qs.get_entry::<String>("0").as_deref(), Some("/maps/alpha.json"), "re-open moves to top");
	assert_eq!(qs.get_entry::<String>("1").as_deref(), Some("/maps/beta.json"));
	assert_eq!(qs.get_entry::<String>("2"), None, "deduped, not appended");
	drop(ini);
	let _ = std::fs::remove_dir_all(&dir);
}

fn new_tab(e: &mut EditorState, seed: u64) -> Outcome {
	e.execute(Command::New { width: 8, height: 8, packs: vec!["GREEN".into()], seed: Some(seed) })
}

/// Routing safety net: every toolbox run-button command must parse AND
/// execute without tripping an `unreachable!` (mis-routed-variant) panic.
/// Toolbox commands are side-effect-free (tool/brush/shape/layer/transform/
/// pass/select) - no IO or dialogs - so running them on a scratch editor is
/// safe.
#[test]
fn toolbox_commands_route_without_panicking() {
	for group in crate::toolbox::GROUPS {
		for button in group.buttons {
			let cmd = button.cmd;
			let parsed = crate::command::parse_line(cmd)
				.unwrap_or_else(|e| panic!("{cmd}: parse error: {e}"))
				.unwrap_or_else(|| panic!("{cmd}: empty command"));
			// A mis-routed variant trips `unreachable!` in execute and fails here.
			let mut e = editor();
			let _ = e.execute(parsed);
		}
	}
}

#[test]
fn filename_sanitization_lowercases_and_strips() {
	assert_eq!(sanitize_filename("My Cool Oasis"), "my-cool-oasis");
	assert_eq!(sanitize_filename("a/b:c*?"), "abc", "special chars dropped");
	assert_eq!(sanitize_filename("  spaced  out  "), "spaced-out", "edges trimmed, runs collapsed");
	assert_eq!(sanitize_filename("Lake-2"), "lake-2");
	assert_eq!(sanitize_filename("***"), "template", "empty result falls back");
	assert_eq!(sanitize_filename("под"), "template", "non-ascii dropped -> fallback");
}

#[test]
fn natural_sort_orders_numbers_by_value() {
	use std::cmp::Ordering::Less;
	assert_eq!(natural_cmp("template-3", "template-20"), Less, "3 < 20");
	assert_eq!(natural_cmp("template-20", "template-100"), Less, "20 < 100");
	let mut v = ["template-100", "template-3", "template-20", "template-2", "template-1"];
	v.sort_by(|a, b| natural_cmp(a, b));
	assert_eq!(v, ["template-1", "template-2", "template-3", "template-20", "template-100"]);
	// Leading zeros tie by value; plain text is case-insensitive.
	assert_eq!(natural_cmp("a007", "a7"), std::cmp::Ordering::Equal);
	assert_eq!(natural_cmp("Crater", "desert"), Less);
}

#[test]
fn dedupe_finds_only_removable_exact_duplicates() {
	let mut e = editor();
	// All-hole templates resolve in any project, so every one is "visible".
	let mk = |w: u16, h: u16| Template {
		name: String::new(),
		width: w,
		height: h,
		uses: Vec::new(),
		cells: vec![String::new(); (w * h) as usize],
	};
	let entry = |name: &str, stock: bool, t: Template| TemplateEntry {
		name: name.to_string(),
		path: PathBuf::from(format!("{name}.json")),
		stock,
		template: t,
	};
	// A stock template, two user copies of it, then a differently-sized one.
	e.templates.entries = vec![
		entry("stock", true, mk(2, 1)),
		entry("copy-a", false, mk(2, 1)),
		entry("copy-b", false, mk(2, 1)),
		entry("other", false, mk(1, 1)),
	];
	// Both user copies are removable duplicates of the (kept) earlier original;
	// the stock entry and the odd-sized one are left alone.
	assert_eq!(e.duplicate_template_indices(), vec![1, 2]);
}

#[test]
fn context_menu_opens_with_state_dependent_items() {
	let mut e = editor();
	e.apply_shortcut_hints(vec![("copy".into(), "Ctrl+C".into())]);
	// Nothing selected, empty clipboard: the lean menu.
	assert!(matches!(e.execute(Command::ContextMenu { at: Some((400.0, 300.0)) }), Outcome::Redraw));
	let lean = e.context_menu.as_ref().expect("open").items.len();
	// Select something: the cut/copy/delete block appears, the menu grows.
	e.execute(Command::SelectRect { x0: 1, y0: 1, x1: 2, y1: 2, mode: SelectMode::Replace });
	e.execute(Command::ContextMenu { at: Some((400.0, 300.0)) });
	let full = e.context_menu.as_ref().expect("open").items.len();
	assert!(full > lean, "selection adds cut/copy/delete rows");
	// The snapshot builds cleanly into widget items + acts (the view side).
	let (built, acts) = menu::build_context(&e.context_menu.as_ref().unwrap().items);
	assert_eq!(built.len(), full);
	assert!(!acts.is_empty(), "the menu carries runnable acts");
	// `off` closes.
	e.execute(Command::ContextMenu { at: None });
	assert!(e.context_menu.is_none());
}

#[test]
fn menu_hint_resolves_binding_alias_and_fixed_shortcut() {
	let mut e = editor();
	e.apply_shortcut_hints(vec![("undo".into(), "Ctrl+Z".into()), ("quit".into(), "Esc".into())]);
	// Exact config binding.
	assert_eq!(e.menu_hint("undo").as_deref(), Some("Ctrl+Z"));
	// Alias: Exit runs `quit-request` but shows the `quit` chord, and follows
	// a rebind (here the table maps `quit` to Esc).
	assert_eq!(e.menu_hint("quit-request").as_deref(), Some("Esc"));
	// Fixed shell shortcuts (not in the config table).
	assert_eq!(e.menu_hint("stamp cancel").as_deref(), Some("Esc"));
	// Unbound commands stay clean.
	assert_eq!(e.menu_hint("map-metadata"), None);
	// The bar bakes the alias too: File ▸ Exit gets the quit chord.
	let exit = e.menu_tree.menus.iter().flat_map(|m| &m.items).find_map(|i| match i {
		menu::Item::Action { command, hint, .. } if command == "quit-request" => Some(hint.clone()),
		_ => None,
	});
	assert_eq!(exit, Some(Some("Esc".into())), "Exit row baked with the quit chord");
}

#[test]
fn palette_manager_save_rename_delete_round_trip() {
	let mut e = editor();
	let dir = e.user_palettes_dir();
	let path = dir.join("__test_pal__.json");
	let renamed = dir.join("__test_pal2__.json");
	// Clean slate (a leftover from a previously-failed run must not confuse us).
	let _ = std::fs::remove_file(&path);
	let _ = std::fs::remove_file(&renamed);

	// Save the working palette under a name → file written, rescanned, selected.
	assert!(matches!(e.execute(Command::PaletteSaveAs { name: "__test_pal__".into() }), Outcome::Redraw));
	assert!(path.is_file(), "saved file exists");
	assert!(e.palettes.files.contains(&path), "rescanned + present");
	assert_eq!(e.selected_palette(), Some(&path), "the new palette is selected");
	assert!(e.selected_palette_is_user(), "a user palette is editable");

	// Rename it.
	e.execute(Command::PaletteRename { from: path.clone(), to: "__test_pal2__".into() });
	assert!(!path.is_file() && renamed.is_file(), "renamed on disk");
	assert_eq!(e.selected_palette(), Some(&renamed));

	// Delete it.
	e.execute(Command::PaletteDelete { path: renamed.clone() });
	assert!(!renamed.is_file(), "deleted on disk");
	assert!(e.palettes.sel.is_none(), "selection cleared");
}

#[test]
fn map_palette_toggle_reseeds_the_cycler_from_the_internal_palette() {
	let mut e = editor();
	// The toggle reveals a map whose *stored* palette diverges from the game's
	// at a game-owned (static) slot - e.g. a WRL import. (Stock packs are baked
	// to the game palette, so their static slots match and the toggle is a
	// no-op there; slot 40 is static + non-animated, so the game resolves it to
	// GAME_PALETTE while the WRL keeps its own byte.)
	let mut tiles = vec![0u8; max_assets::wrl::TILE_DATA_SIZE];
	tiles.fill(40);
	let mut palette = map_core::GAME_PALETTE.to_vec();
	palette[40 * 3..40 * 3 + 3].copy_from_slice(&[0xff, 0x00, 0xee]);
	let wrl = max_assets::wrl::WrlFile {
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
	e.add_doc(Project::from_wrl(&wrl, "CONV"), None, None);

	let game = [e.project.palette[40 * 3], e.project.palette[40 * 3 + 1], e.project.palette[40 * 3 + 2]];
	let internal = e.project.internal_palette();
	let raw = [internal[40 * 3], internal[40 * 3 + 1], internal[40 * 3 + 2]];
	assert_ne!(game, raw, "the WRL's stored static slot 40 differs from the game palette");
	assert_eq!(raw, [0xff, 0x00, 0xee], "internal keeps the map's own bytes");

	assert!(matches!(e.execute(Command::MapPalette { on: None }), Outcome::Redraw));
	assert!(e.debug_map_palette);
	assert_eq!(&e.cycler.rgba()[40 * 4..40 * 4 + 3], &raw, "cycler reseeded from the internal palette");
	e.execute(Command::MapPalette { on: Some(false) });
	assert!(!e.debug_map_palette);
	assert_eq!(&e.cycler.rgba()[40 * 4..40 * 4 + 3], &game, "back to the game-resolved palette");
	// A `window wrlpalette` toggle reaches the (hidden-by-default) panel.
	assert!(!e.workspace.is_visible("wrlpalette"));
	assert!(matches!(e.execute(Command::Window { id: "wrlpalette".into(), on: Some(true) }), Outcome::Redraw));
	assert!(e.workspace.is_visible("wrlpalette"));
}

#[test]
fn apply_shape_image_carves_land_and_opens_fix_shore() {
	let mut e = editor(); // 8×8 GREEN, all open water
	// A tiny 8×8 PNG: left half blue (→ water), right half green (→ land).
	let mut rgba = Vec::with_capacity(8 * 8 * 4);
	for _y in 0..8 {
		for x in 0..8 {
			rgba.extend_from_slice(if x < 4 { &[0, 0, 255, 255] } else { &[0, 200, 0, 255] });
		}
	}
	let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../temp");
	std::fs::create_dir_all(&dir).unwrap();
	let path = dir.join("apply-shape-test.png");
	{
		let file = std::fs::File::create(&path).unwrap();
		let mut enc = png::Encoder::new(std::io::BufWriter::new(file), 8, 8);
		enc.set_color(png::ColorType::Rgba);
		enc.set_depth(png::BitDepth::Eight);
		enc.write_header().unwrap().write_image_data(&rgba).unwrap();
	}

	let out = e.apply_shape_image(&path);
	// Fix Shore opened for the user to choose a method (it does not auto-run).
	assert!(matches!(out, Outcome::OpenDialog(DialogRequest::AutoFix)));
	assert!(e.autofix_open(), "Fix Shore run state opened");
	// Right half carries land on the ground layer; left half stayed open water.
	let ground = |e: &EditorState, x, y| e.project.cell(x, y).unwrap()[LAYER_GROUND].is_some();
	for y in 0..8u16 {
		for x in 0..8u16 {
			assert_eq!(ground(&e, x, y), x >= 4, "land mask at ({x},{y})");
		}
	}
	let _ = std::fs::remove_file(&path);
}

/// `tool default` resolves to the **active mode's** select tool, and every
/// alias echoes the tool it actually armed (not the word that was typed).
#[test]
fn tool_default_is_the_modes_own_select_tool() {
	let mut e = editor();
	e.execute(Command::ToolSelect { name: "default".into() });
	assert_eq!(e.tool, Tool::Select, "the map editor selects cells");
	assert_eq!(e.console.log().last().map(String::as_str), Some("tool: select"));

	e.execute(Command::Mode { name: "save".into() });
	e.execute(Command::ToolSelect { name: "default".into() });
	assert_eq!(e.tool, Tool::ObjSelect, "the save editor selects objects");
	assert_eq!(e.console.log().last().map(String::as_str), Some("tool: obj-select"));

	// An alias echoes the canonical slug, so a resolved `default` and a typed
	// alias read the same in the console.
	e.execute(Command::Mode { name: "map".into() });
	e.execute(Command::ToolSelect { name: "paint-water".into() });
	assert_eq!(e.console.log().last().map(String::as_str), Some("tool: paint-mask"));
	assert!(matches!(e.execute(Command::ToolSelect { name: "nope".into() }), Outcome::Failed(_)));
}

/// Switching mode with a tool the incoming mode does not offer leaves **no**
/// tool selected - nothing lit in any of its toolboxes - so the mode falls
/// back to its own select tool. A tool both modes own (the place/erase pair)
/// survives the switch.
#[test]
fn a_mode_switch_reverts_a_tool_that_mode_does_not_own() {
	let mut e = editor();
	e.execute(Command::ToolSelect { name: "obj-move".into() });
	assert_eq!(e.tool, Tool::ObjMove, "an object tool is armable from anywhere");
	e.execute(Command::Mode { name: "pass".into() });
	assert_eq!(e.tool, Tool::Select, "the pass editor has no object tools");

	e.execute(Command::ToolSelect { name: "pencil".into() });
	e.execute(Command::Mode { name: "save".into() });
	assert_eq!(e.tool, Tool::ObjSelect, "the save toolbox has no pencil");

	e.execute(Command::ToolSelect { name: "obj-place".into() });
	e.execute(Command::Mode { name: "map".into() });
	assert_eq!(e.tool, Tool::Unit, "place is shared with the Units panel - it stays armed");

	e.execute(Command::ToolSelect { name: "select-rect".into() });
	e.execute(Command::Mode { name: "localpass".into() });
	assert_eq!(e.tool, Tool::SelectRect, "and the pass editors keep the map's own set");
}

/// Entering either pass editor brings up the Pass Types Palette - the panel
/// that holds the swatches those modes paint with. Leaving restores the mode
/// you went back to, whose own layout says nothing about this panel.
#[test]
fn a_pass_mode_reveals_the_pass_types_palette() {
	let mut e = editor();
	assert!(!e.workspace.is_visible("passtools"), "hidden by default - the map editor has no use for it");
	e.execute(Command::Mode { name: "pass".into() });
	assert!(e.workspace.is_visible("passtools"), "the pass editor needs its swatches");
	e.execute(Command::Window { id: "passtools".into(), on: Some(false) });
	assert!(!e.workspace.is_visible("passtools"), "and the user can still close it");
	e.execute(Command::Mode { name: "localpass".into() });
	assert!(!e.workspace.is_visible("passtools"), "flipping between the two pass editors leaves it closed");
	e.execute(Command::Mode { name: "map".into() });
	e.execute(Command::Mode { name: "pass".into() });
	assert!(e.workspace.is_visible("passtools"), "arriving in the pass group again reveals it");
}

#[test]
fn status_bar_toggles_and_reserves_the_bottom_strip() {
	let mut e = editor();
	assert!(e.status_bar);
	assert_eq!(e.workspace.bottom, crate::statusbar::BAR_H);
	e.execute(Command::StatusBar { on: Some(false) });
	assert!(!e.status_bar);
	assert_eq!(e.workspace.bottom, 0.0, "hidden bar releases the strip");
	e.execute(Command::StatusBar { on: None });
	assert!(e.status_bar);
	// The hint follows the active tool / mode.
	e.execute(Command::ToolSelect { name: "eraser".into() });
	assert!(e.status_hint().contains("Eraser"), "{}", e.status_hint());
	e.execute(Command::Mode { name: "localpass".into() });
	assert!(e.status_hint().contains("Override"), "{}", e.status_hint());
}

#[test]
fn brush_size_paints_a_centered_square() {
	let mut e = editor(); // 8×8
	e.execute(Command::Tile { spec: Some("GLa000".into()) });
	e.execute(Command::BrushSize { size: 3 });
	assert_eq!(e.brush_size, 3);
	e.execute(Command::Paint { x: 4, y: 4 });
	let ground = |e: &EditorState, x, y| e.project.cell(x, y).unwrap()[LAYER_GROUND].is_some();
	for dy in -1..=1i32 {
		for dx in -1..=1i32 {
			assert!(ground(&e, (4 + dx) as u16, (4 + dy) as u16), "({},{}) painted", 4 + dx, 4 + dy);
		}
	}
	assert!(!ground(&e, 6, 4), "outside the 3x3 footprint untouched");
	// Even sizes snap odd so the square stays centred.
	e.execute(Command::BrushSize { size: 4 });
	assert_eq!(e.brush_size, 5);
}

#[test]
fn circle_brush_drops_the_far_corners() {
	let mut e = editor(); // 8×8
	e.execute(Command::BrushSize { size: 5 });
	e.execute(Command::BrushShape { shape: "circle".into() });
	let cells = e.brush_cells(4, 4);
	assert!(!cells.contains(&(2, 2)) && !cells.contains(&(6, 6)), "circle drops the far corners");
	assert!(cells.contains(&(4, 2)) && cells.contains(&(2, 4)), "axis cells kept");
	e.execute(Command::BrushShape { shape: "square".into() });
	assert!(e.brush_cells(4, 4).contains(&(2, 2)), "square keeps corners");
}

#[test]
fn terrain_brush_paints_a_land_water_mask_and_grows_the_coast() {
	let mut e = editor(); // 8×8 GREEN
	let ground = |e: &EditorState, x, y| e.project.cell(x, y).unwrap()[LAYER_GROUND];
	let water = |e: &EditorState, x, y| e.project.cell(x, y).unwrap()[LAYER_WATER];

	// "water" button: arms the terrain brush AND picks the water material.
	e.execute(Command::ToolSelect { name: "paint-water".into() });
	assert_eq!(e.tool, Tool::PaintMask);
	assert!(e.mask_water, "the water button paints water");
	// One huge dab floods the whole 8×8 to open ocean (no active tile needed).
	e.execute(Command::BrushShape { shape: "square".into() });
	e.execute(Command::BrushSize { size: 15 });
	e.execute(Command::PaintMask { x: 4, y: 4 });
	for y in 0..8u16 {
		for x in 0..8u16 {
			assert!(ground(&e, x, y).is_none(), "({x},{y}) ground cleared for water");
			assert!(water(&e, x, y).is_some(), "({x},{y}) water-variant beneath");
		}
	}
	e.take_mask_region(); // discard the flood's bounds

	// "land" button flips the material; paint a 3×3 island into the ocean.
	e.execute(Command::ToolSelect { name: "paint-land".into() });
	assert!(!e.mask_water, "the land button paints land");
	e.execute(Command::BrushSize { size: 3 });
	e.execute(Command::PaintMask { x: 4, y: 4 });
	for dy in -1..=1i32 {
		for dx in -1..=1i32 {
			assert!(ground(&e, (4 + dx) as u16, (4 + dy) as u16).is_some(), "land at ({},{})", 4 + dx, 4 + dy);
		}
	}

	// The stroke recorded its painted bounds, grown by one and clamped.
	let region = e.take_mask_region().expect("the stroke painted something");
	assert_eq!(region, (2, 2, 6, 6), "3x3 footprint at (4,4), grown by one");
	assert!(e.take_mask_region().is_none(), "the bounds are consumed once");

	// Shoring that region (what release does) tiles the new coast: the water
	// ring around the island becomes shore on the ground layer.
	let (changed, _unresolved) = e.project.auto_shore(Some(region));
	assert!(changed > 0, "the land/water boundary grew shore tiles");
	assert!(ground(&e, 2, 4).is_some(), "a shore tile landed on the island's coast");
}

#[test]
fn auto_shore_command_sets_the_brush_coast_mode() {
	let mut e = editor();
	assert_eq!(e.brush_shore, BrushShore::Sweep, "default is sweep");
	e.execute(Command::AutoShore { mode: "loop-walk".into() });
	assert_eq!(e.brush_shore, BrushShore::LoopWalk);
	e.execute(Command::AutoShore { mode: "off".into() });
	assert_eq!(e.brush_shore, BrushShore::Off);
	assert!(matches!(e.execute(Command::AutoShore { mode: "bogus".into() }), Outcome::Failed(_)));
}

#[test]
fn fix_shore_modal_improves_the_coast_without_destroying_terrain_then_undoes() {
	// A 24x24 GREEN map with a RAW scattered-noise mask (auto-shore off):
	// dense enough that a single placement pass leaves broken seams the GREEN
	// set can't tile without reshaping - so the accurate tier must escalate.
	let res = resources();
	let project = Project::new(24, 24, &["GREEN".to_string()], &res.join("assets/tilepacks"), 1).unwrap();
	let mut e = EditorState::new(project, (800, 600), None, res);
	e.execute(Command::AutoShore { mode: "off".into() });
	e.execute(Command::ToolSelect { name: "paint-water".into() });
	e.execute(Command::BrushShape { shape: "square".into() });
	e.execute(Command::BrushSize { size: 99 });
	e.execute(Command::PaintMask { x: 12, y: 12 });
	e.execute(Command::ToolSelect { name: "paint-land".into() });
	e.execute(Command::BrushSize { size: 1 });
	for y in 0..24u16 {
		for x in 0..24u16 {
			if (x.wrapping_mul(2654) ^ y.wrapping_mul(40503)) % 5 < 2 {
				e.execute(Command::PaintMask { x, y });
			}
		}
	}
	// Ground cells whose top layer is painted - the run must never erase any
	// (the fix re-tiles shore-band cells only; placement adds coast, never
	// floods land), so this count can only grow.
	let ground = |p: &Project| {
		(0..p.height)
			.flat_map(|y| (0..p.width).map(move |x| (x, y)))
			.filter(|&(x, y)| p.cell(x, y).unwrap()[LAYER_GROUND].is_some())
			.count()
	};
	let raw = e.project.shore_defects(None);
	let raw_strict = e.project.shore_defect_cells(None).len();
	let ground_before = ground(&e.project);
	assert!(raw > 0 && raw_strict > 0, "the noise mask has defects");

	// Open the dialog and drive its per-frame loop to completion. The fix is
	// shore-band only (no terrain destruction), so it converges to the best
	// coast the tileset allows - it must terminate and report a result.
	e.execute(Command::AutoFixModal { autostart: true });
	assert!(e.autofix_running(), "the run auto-starts");
	let mut guard = 0;
	while e.autofix_running() {
		e.autofix_tick(0.0, false);
		guard += 1;
		assert!(guard < 10_000, "the fix loop must terminate");
	}
	let af = e.autofix.as_ref().unwrap();
	assert!(af.applied.is_some(), "the run finished and reports a result (so Undo is offered)");
	assert_eq!(af.remaining, e.project.shore_defect_cells(None).len(), "remaining = the live strict defect count");

	// It improved the coast and never destroyed terrain.
	assert!(e.project.shore_defect_cells(None).len() < raw_strict, "the run cut the broken/missing shore");
	assert!(ground(&e.project) >= ground_before, "no land was flooded - shore-band cells only");

	// Undo reverts the whole run (placement + every pass) in one step.
	e.project.undo();
	assert_eq!(e.project.shore_defects(None), raw, "undo restores the raw coast");
	assert_eq!(ground(&e.project), ground_before, "and its terrain");
}

#[test]
fn shore_ladder_lays_missing_coast_and_fixes_seams() {
	use map_core::FixStrength;
	// Paint a RAW land/water mask (terrain brush, auto-shore off) so there is
	// no coast yet - the case the old fix modes couldn't handle.
	let paint_raw = |e: &mut EditorState| {
		e.execute(Command::AutoShore { mode: "off".into() });
		e.execute(Command::ToolSelect { name: "paint-water".into() });
		e.execute(Command::BrushShape { shape: "square".into() });
		e.execute(Command::BrushSize { size: 15 });
		e.execute(Command::PaintMask { x: 4, y: 4 }); // flood to ocean
		e.execute(Command::ToolSelect { name: "paint-land".into() });
		e.execute(Command::BrushSize { size: 3 });
		e.execute(Command::PaintMask { x: 4, y: 4 }); // a 3x3 island
	};
	let ground = |e: &EditorState, x, y| e.project.cell(x, y).unwrap()[LAYER_GROUND].is_some();

	// Full (Destructive): every seam closed and the coast is stable - a
	// fresh auto_shore is a no-op. This is the 100% guarantee.
	let mut e = editor();
	paint_raw(&mut e);
	assert!(!ground(&e, 2, 4), "raw: no coast laid beside the island");
	e.execute(Command::Shore { region: None, mode: ShoreMode::Full });
	assert_eq!(e.project.fix_session(None, FixStrength::Shore).found(), 0, "Full closes every seam");
	assert_eq!(e.project.auto_shore(None), (0, 0), "Full's coast is complete and idempotent");

	// Sweep + Fix (Aggressive): lays the missing coast - the water ring
	// around the island becomes shore on the ground layer.
	let mut e = editor();
	paint_raw(&mut e);
	e.execute(Command::Shore { region: None, mode: ShoreMode::SweepFix });
	assert!(ground(&e, 2, 4), "Sweep + Fix laid the coast where it was missing");
}

#[test]
fn fill_with_active_selection_fills_only_the_selection() {
	let mut e = editor();
	e.execute(Command::Tile { spec: Some("GLa000".into()) });
	e.execute(Command::SelectRect { x0: 1, y0: 1, x1: 2, y1: 2, mode: SelectMode::Replace });
	assert_eq!(e.selection.count(), 4);
	// Fill: the click cell (6,6) is ignored when a selection is active.
	assert!(matches!(e.execute(Command::Fill { x: 6, y: 6 }), Outcome::Redraw));
	let ground = |e: &EditorState, x, y| e.project.cell(x, y).unwrap()[LAYER_GROUND].map(|t| t.tile);
	let want = ground(&e, 1, 1);
	assert!(want.is_some(), "selected cell filled");
	assert_eq!(ground(&e, 2, 2), want, "whole selection filled");
	assert_eq!(ground(&e, 6, 6), None, "outside the selection untouched");
}

#[test]
fn pass_editors_split_tile_passability_from_per_cell_overrides() {
	let mut e = editor();
	// Local Pass Override Editor: the pass overlay turns on; painting sets a
	// per-cell override, the eraser-driven clear lifts it.
	assert!(matches!(e.execute(Command::Mode { name: "localpass".into() }), Outcome::Redraw));
	assert_eq!(e.mode, EditorMode::LocalPass);
	assert!(e.show_pass_overlay);
	e.execute(Command::PassPaint { x: 1, y: 1, value: 3 });
	assert_eq!(e.project.pass_override(1, 1), Some(3));
	e.execute(Command::PassClear { x: 1, y: 1 });
	assert_eq!(e.project.pass_override(1, 1), None, "clear lifts the override");
	// Pass Table Editor: tile passability is tile-dependent - no per-cell
	// override is created, but the cell reads the new value.
	assert!(matches!(e.execute(Command::Mode { name: "pass".into() }), Outcome::Redraw));
	e.execute(Command::TilePass { x: 2, y: 2, value: 2 });
	assert_eq!(e.project.pass_override(2, 2), None, "tile pass is not a cell override");
	assert_eq!(e.project.pass_at(2, 2), Some(2), "the cell reads the tile's new pass");
}

#[test]
fn each_mode_group_keeps_its_own_dock_layout() {
	use crate::workspace::Place;
	let mut e = editor(); // opens in Map (the Main layout group)
	// Distinguish each group by the minimap's floating position - all well
	// within the 800x600 viewport, so nothing clamps and each round-trips
	// exactly. (Dock sizes clamp to per-side minimums, so they're a poor
	// probe here.)
	let minimap = |e: &EditorState| e.workspace.panels[e.workspace.find("minimap").unwrap()].place;
	let float_to = |e: &mut EditorState, x: f32, y: f32| e.workspace.dock_to("minimap", Place::Floating(x, y)).unwrap();

	float_to(&mut e, 100.0, 100.0); // Map (Main)
	e.execute(Command::Mode { name: "pass".into() });
	float_to(&mut e, 300.0, 120.0); // Pass
	// The two pass editors share ONE layout - switching between them must not
	// reset or swap it.
	e.execute(Command::Mode { name: "localpass".into() });
	assert_eq!(minimap(&e), Place::Floating(300.0, 120.0), "Pass & Local Pass Override share one layout");
	e.execute(Command::Mode { name: "save".into() });
	float_to(&mut e, 150.0, 300.0); // Save

	// Each group restores its own layout, revisited in any order.
	e.execute(Command::Mode { name: "map".into() });
	assert_eq!(minimap(&e), Place::Floating(100.0, 100.0), "Map layout restored");
	e.execute(Command::Mode { name: "save".into() });
	assert_eq!(minimap(&e), Place::Floating(150.0, 300.0), "Save layout restored");
	e.execute(Command::Mode { name: "pass".into() });
	assert_eq!(minimap(&e), Place::Floating(300.0, 120.0), "Pass layout restored");
}

#[test]
fn seed_mode_layouts_loads_saved_sections_and_defaults_to_main() {
	use crate::workspace::Place;
	let mut e = editor();
	let minimap = |e: &EditorState| e.workspace.panels[e.workspace.find("minimap").unwrap()].place;
	// A distinctive live layout, as if `[Workspace]` was just applied.
	e.workspace.dock_to("minimap", Place::Floating(140.0, 140.0)).unwrap();

	// A settings INI carrying only a Pass layout; the Save section is absent.
	let mut ini = ini::INI::new();
	let mut pass = ini::INISection::new();
	let _ = pass.set_entry("Minimap".to_string(), "Float 300 120 260 220 234".to_string());
	ini.insert_section("Workspace.Pass".to_string(), pass);
	e.seed_mode_layouts(&ini, 800.0, 600.0);

	// Pass loads its saved section; Save (absent) defaults to the main layout.
	e.execute(Command::Mode { name: "pass".into() });
	assert_eq!(minimap(&e), Place::Floating(300.0, 120.0), "Pass loads its [Workspace.Pass] section");
	e.execute(Command::Mode { name: "save".into() });
	assert_eq!(minimap(&e), Place::Floating(140.0, 140.0), "Save (no section) defaults to the main layout");
	e.execute(Command::Mode { name: "map".into() });
	assert_eq!(minimap(&e), Place::Floating(140.0, 140.0), "Main keeps the applied layout");
}

#[test]
fn pass_table_edit_queues_the_stock_pack_for_bake_only_in_dev() {
	let mut e = editor();
	// A stock GREEN tile under the cell, so the pass edit lands in GREEN's table.
	e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });

	// Without --dev the pass still edits in memory, but nothing is queued for
	// Bake (so it could never reach the shipped tiles.pass.json).
	e.execute(Command::TilePass { x: 1, y: 1, value: 3 });
	assert!(!e.tile_ops.dirty_packs.contains("GREEN"), "no --dev: pack not queued for bake");

	// With --dev, editing a stock tile's pass queues its pack - Bake then writes
	// tiles.pass.json (this was the missing link; the edit was lost before).
	e.dev_mode = true;
	assert!(matches!(e.execute(Command::TilePass { x: 1, y: 1, value: 1 }), Outcome::Redraw));
	assert!(e.tile_ops.dirty_packs.contains("GREEN"), "--dev: the affected pack is queued for bake");
	assert_eq!(e.project.pass_at(1, 1), Some(1), "the in-memory pass reflects the edit");

	// A no-op edit (same value) does not spuriously queue anything new.
	let mut e2 = editor();
	e2.dev_mode = true;
	e2.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });
	let current = e2.project.pass_at(1, 1).unwrap();
	e2.execute(Command::TilePass { x: 1, y: 1, value: current });
	assert!(!e2.tile_ops.dirty_packs.contains("GREEN"), "unchanged pass does not queue a bake");
}

#[test]
fn reset_tile_pass_restores_the_tileset_values_and_undoes() {
	let mut e = editor();
	e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });
	// The tileset's canonical pass for GLa000 (a fresh load of GREEN).
	let fresh = map_core::TilePack::load(&e.assets_root, "GREEN").unwrap();
	let canonical = fresh.pass.as_ref().unwrap()[fresh.index_of["GLa000"] as usize];
	let edited = if canonical == 3 { 0 } else { 3 };

	// Edit the tile pass away from the tileset value.
	e.execute(Command::TilePass { x: 1, y: 1, value: edited });
	assert_eq!(e.project.pass_at(1, 1), Some(edited), "the edit took");

	// Reset restores the tileset value...
	assert!(matches!(e.execute(Command::ResetTilePass), Outcome::Redraw));
	assert_eq!(e.project.pass_at(1, 1), Some(canonical), "reset to the tileset pass");
	// ...as one undo unit (the edit comes back).
	assert!(e.project.undo(), "reset is undoable");
	assert_eq!(e.project.pass_at(1, 1), Some(edited), "undo restored the edit");

	// Resetting when already canonical is a quiet no-op.
	e.execute(Command::ResetTilePass);
	assert!(matches!(e.execute(Command::ResetTilePass), Outcome::Ok), "no-op when already at tileset");
	// Per-cell overrides are untouched by the reset.
	e.execute(Command::Mode { name: "localpass".into() });
	e.execute(Command::PassPaint { x: 1, y: 1, value: 2 });
	e.execute(Command::ResetTilePass);
	assert_eq!(e.project.pass_override(1, 1), Some(2), "reset leaves per-cell overrides alone");
}

#[test]
fn opening_a_stock_map_keeps_its_origin_path_less() {
	let mut e = editor();
	let stock = e.resources_root.join("assets/maps/GREEN_1.json");
	assert!(stock.is_file(), "the shipped GREEN_1 map exists");
	e.execute(Command::Open { path: stock.clone() });
	// A shipped map loads path-less (so Save can't overwrite it) but keeps its
	// origin, so DEV ▸ Update Map can still write back to it.
	assert_eq!(e.path, None, "stock map is path-less (Save -> Save As)");
	assert_eq!(e.origin.as_deref(), Some(stock.as_path()), "its origin is remembered");
}

#[test]
fn first_save_prompts_map_metadata_before_save_as() {
	// A never-saved map's Save routes to Save-As, which prompts for the
	// map's metadata first (the dialog's Save resumes via the one-shot).
	let mut e = editor();
	assert_eq!(e.path, None, "a fresh map is unsaved");
	assert!(
		matches!(e.execute(Command::SaveProject), Outcome::OpenDialog(DialogRequest::Metadata { save_after: true })),
		"first save prompts Map Metadata instead of the file dialog"
	);
	assert!(!e.first_save_meta, "the one-shot is only ever set by the shell");
}

#[test]
fn template_born_maps_report_doc_from_template() {
	let mut e = editor();
	assert!(!e.doc_from_template(), "a fresh New Map is not template-born");
	let stock = e.resources_root.join("assets/maps/GREEN_1.json");
	e.execute(Command::Open { path: stock });
	assert!(e.doc_from_template(), "a shipped template opened path-less is");
	// Once saved somewhere it stops counting (metadata blanking is a
	// first-save-only affair).
	e.path = Some(PathBuf::from("/tmp/somewhere.json"));
	assert!(!e.doc_from_template(), "a saved map is not template-born");
}

/// Backup rotation (S6.5): the first write leaves no backup; each overwrite
/// keeps the prior bytes as `.bak1` and shifts older ones up, never exceeding
/// `keep` backups (the oldest is dropped on the `keep+1`-th overwrite).
#[test]
fn rotate_backups_keeps_five_newest() {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/rotate-backups-test");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let target = dir.join("save.dta");
	let bak = |n: usize| dir.join(format!("save.dta.bak{n}"));

	// Simulate seven successive writes, versions "v0".."v6".
	for v in 0..7 {
		let backed_up = rotate_backups(&target, SAVE_BACKUP_KEEP).unwrap();
		assert_eq!(backed_up, v != 0, "a backup is made iff the file already existed (v{v})");
		std::fs::write(&target, format!("v{v}")).unwrap();
	}

	// After 7 writes: current is v6, and bak1..bak5 are v5..v1 (v0 dropped).
	assert_eq!(std::fs::read_to_string(&target).unwrap(), "v6", "current file is the newest");
	for n in 1..=SAVE_BACKUP_KEEP {
		assert_eq!(
			std::fs::read_to_string(bak(n)).unwrap(),
			format!("v{}", 6 - n),
			"bak{n} holds the n-th prior version"
		);
	}
	assert!(!bak(SAVE_BACKUP_KEEP + 1).exists(), "no 6th backup is kept");
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_appends_metadata_json_the_wrl_reader_ignores() {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/export-meta-test");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let target = dir.join("out.wrl");

	let mut e = editor();
	e.project.set_info("Isle".into(), Some(3), "d".into(), "2026".into(), "1.0".into(), "A".into());
	assert!(matches!(e.execute(Command::Export { path: Some(target.clone()) }), Outcome::Redraw));

	let bytes = std::fs::read(&target).unwrap();
	let text = String::from_utf8_lossy(&bytes);
	assert!(text.contains("\"mme_map_metadata\": 1"), "metadata tail appended");
	assert!(text.contains("\"players\": \"2-3\""), "players label in the tail");
	// The reader consumes the WRL by its structured field sizes, so the
	// tail must not break a re-import of our own export.
	let wrl = read_wrl_file(&target).expect("the JSON tail doesn't break the WRL reader");
	assert_eq!((wrl.width, wrl.height), (8, 8), "payload intact under the tail");
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wrl_export_path_forces_one_uppercase_ext() {
	let p = |s: &str| wrl_export_path(PathBuf::from(s));
	// No extension → append `.WRL`.
	assert_eq!(p("/maps/atoll"), PathBuf::from("/maps/atoll.WRL"));
	// A user-typed `.WRL` is kept as-is (not doubled).
	assert_eq!(p("/maps/atoll.WRL"), PathBuf::from("/maps/atoll.WRL"));
	// A lowercase `.wrl` is upper-cased.
	assert_eq!(p("/maps/atoll.wrl"), PathBuf::from("/maps/atoll.WRL"));
	// A non-WRL extension is preserved, then `.WRL` appended.
	assert_eq!(p("/maps/atoll.backup"), PathBuf::from("/maps/atoll.backup.WRL"));
}

#[test]
fn update_map_overwrites_the_origin_only_in_dev() {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/update-map-test");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let target = dir.join("stock.json");

	// Simulate a stock map: an origin to write back to, but path-less (Save off).
	let mut e = editor();
	e.origin = Some(target.clone());
	e.path = None;
	e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });

	// Without --dev it's refused and nothing is written.
	assert!(matches!(e.execute(Command::UpdateMap), Outcome::Failed(_)), "update-map needs --dev");
	assert!(!target.exists(), "nothing written without --dev");

	// With --dev it overwrites the origin and marks the project saved, without
	// adopting a save path (so plain Save stays protected).
	e.dev_mode = true;
	assert!(!matches!(e.execute(Command::UpdateMap), Outcome::Failed(_)), "update-map writes in --dev");
	assert!(target.is_file(), "the original file was written");
	assert!(!e.dirty(), "update-map marks the project saved");
	assert_eq!(e.path, None, "it does not adopt the path");

	// A map with no original file at all (New / WRL / image) is refused, even in --dev.
	let mut fresh = editor();
	fresh.dev_mode = true;
	assert!(matches!(fresh.execute(Command::UpdateMap), Outcome::Failed(_)), "no origin/path -> refused");
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn show_only_selected_layer_masks_the_view_to_the_active_layer() {
	let mut e = editor();
	// Off: every layer composites (all bits set).
	assert!(!e.show_only_layer);
	assert_eq!(e.layer_mask(), (1 << map_core::MAX_LAYERS) - 1);
	// On with the default active layer (ground) → ground only.
	assert!(matches!(e.execute(Command::ShowOnlyLayer { on: None }), Outcome::Redraw));
	assert!(e.show_only_layer);
	assert_eq!(e.layer_mask(), 1 << LAYER_GROUND);
	// Switching the active layer re-targets the filter, no extra toggle.
	e.execute(Command::Layer { name: "water".into() });
	assert_eq!(e.layer_mask(), 1 << LAYER_WATER);
	// Off restores the full mask.
	e.execute(Command::ShowOnlyLayer { on: Some(false) });
	assert_eq!(e.layer_mask(), (1 << map_core::MAX_LAYERS) - 1);
	// The Scenery layer is not a tile layer, so "show only" it drops every
	// tile bit: the terrain goes, the cut-outs (their own pass, over the
	// composed map) stay.
	e.execute(Command::Layer { name: "scenery".into() });
	e.execute(Command::ShowOnlyLayer { on: Some(true) });
	assert_eq!(e.layer_mask() & ((1 << map_core::MAX_LAYERS) - 1), 0);
}

/// The Scenery layer adds no tools - it **re-points** the three the toolbox
/// already has, and hands them back on the way out. Naming a scenery tool
/// outright implies the layer, so the menu tick, the toolbox key and the
/// armed tool can never disagree.
#[test]
fn the_scenery_layer_re_points_the_pencil_eraser_and_arrow() {
	let mut e = editor();
	assert_eq!(e.active_layer_name(), "ground");

	for (name, terrain, scenery) in [
		("pencil", Tool::Pencil, Tool::Scenery),
		("eraser", Tool::Eraser, Tool::SceneryEraser),
		("select", Tool::Select, Tool::SceneryMove),
	] {
		e.execute(Command::Layer { name: "ground".into() });
		e.execute(Command::ToolSelect { name: name.into() });
		assert_eq!(e.tool, terrain, "{name} on the ground layer");
		// Switching to Scenery re-points what is already armed...
		e.execute(Command::Layer { name: "scenery".into() });
		assert_eq!(e.tool, scenery, "{name} follows the layer");
		assert_eq!(e.active_layer_name(), "scenery");
		// ...and picking the same key again while there stays on the twin.
		e.execute(Command::ToolSelect { name: name.into() });
		assert_eq!(e.tool, scenery, "{name} resolves to its twin while Scenery is live");
		// Leaving hands the terrain tool back.
		e.execute(Command::Layer { name: "water".into() });
		assert_eq!(e.tool, terrain, "{name} comes back on the way out");
	}

	// A tool with no scenery meaning is left exactly as it is.
	e.execute(Command::Layer { name: "scenery".into() });
	e.execute(Command::ToolSelect { name: "fill".into() });
	assert_eq!(e.tool, Tool::Fill, "fill has no scenery twin");

	// Naming a scenery tool outright arms the layer with it.
	e.execute(Command::Layer { name: "ground".into() });
	e.execute(Command::ToolSelect { name: "scenery-move".into() });
	assert_eq!((e.tool, e.active_layer_name()), (Tool::SceneryMove, "scenery"));

	// A cell edit never addresses the Scenery layer: it is `MAX_LAYERS`, not
	// a tile-layer index, so every tile path reads `tile_layer` instead.
	assert_eq!(e.tile_layer(), LAYER_GROUND);
	assert_eq!(e.tile_layer_name(), "ground");
	const { assert!(LAYER_SCENERY >= map_core::MAX_LAYERS, "the selector value is not a tile-layer index") };
	assert!(matches!(e.execute(Command::Layer { name: "objects".into() }), Outcome::Failed(_)));
}

#[test]
fn ctrl_click_builds_a_palette_multi_selection() {
	let mut e = editor();
	e.execute(Command::ColorToggle { index: 64 });
	e.execute(Command::ColorToggle { index: 70 });
	assert_eq!(e.palettes.multi, vec![64, 70]);
	assert_eq!(e.active_color, Some(70), "last toggled stays the focus");
	// Re-toggling removes a slot.
	e.execute(Command::ColorToggle { index: 64 });
	assert_eq!(e.palettes.multi, vec![70]);
	// A plain select clears the multi set; a shift-range too.
	e.execute(Command::Color { index: 100 });
	assert!(e.palettes.multi.is_empty());
	e.execute(Command::ColorToggle { index: 80 });
	e.execute(Command::ColorTo { index: 90 });
	assert!(e.palettes.multi.is_empty(), "shift-range clears multi");
}

#[test]
fn convert_palette_guards_projects_and_converts_wrl_imports() {
	let convert = || Command::ConvertPalette { rasterize: false, water: true, relaxed: false, threshold: 0.05 };
	// A .json project doesn't own its tiles - loud refusal; the modal
	// opener refuses identically.
	let mut e = editor();
	assert!(matches!(e.execute(convert()), Outcome::Failed(_)));
	assert!(matches!(e.execute(Command::ConvertPaletteModal), Outcome::Failed(_)));

	// A WRL import with an off-spec static slot converts (DocReplaced -
	// the tile atlas must rebuild) and the cycler follows the new palette.
	let mut tiles = vec![0u8; max_assets::wrl::TILE_DATA_SIZE];
	tiles.fill(40);
	let mut palette = map_core::GAME_PALETTE.to_vec();
	palette[40 * 3..40 * 3 + 3].copy_from_slice(&[0xff, 0x00, 0xee]);
	let wrl = max_assets::wrl::WrlFile {
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
	e.add_doc(Project::from_wrl(&wrl, "CONV"), None, None);
	assert!(matches!(e.execute(Command::ConvertPaletteModal), Outcome::OpenDialog(DialogRequest::ConvertPalette)));
	assert!(matches!(e.execute(convert()), Outcome::DocReplaced));
	let to = e.project.packs[0].tiles[0] as usize;
	assert_eq!(&e.cycler.rgba()[to * 4..to * 4 + 3], &[0xff, 0x00, 0xee]);
	// Already compatible now - the second run is a no-op.
	assert!(matches!(e.execute(convert()), Outcome::Redraw));
	// Undo restores the document structurally (atlas rebuild) and the
	// cycler follows the restored (game-resolved) palette; redo too.
	assert!(matches!(e.execute(Command::Undo), Outcome::DocReplaced));
	assert!(e.project.packs[0].tiles.iter().all(|&b| b == 40));
	assert_eq!(&e.cycler.rgba()[40 * 4..40 * 4 + 3], &map_core::GAME_PALETTE[40 * 3..40 * 3 + 3]);
	assert!(matches!(e.execute(Command::Redo), Outcome::DocReplaced));
	assert_eq!(&e.cycler.rgba()[to * 4..to * 4 + 3], &[0xff, 0x00, 0xee]);
	// The rasterize method works through the same command (tiny map -
	// the synchronous re-import is instant here).
	let rast = Command::ConvertPalette { rasterize: true, water: true, relaxed: false, threshold: 0.05 };
	assert!(matches!(e.execute(rast), Outcome::DocReplaced));
	assert!(matches!(e.execute(Command::Undo), Outcome::DocReplaced));
}

#[test]
fn rasterize_conversion_runs_stepped_with_progress_and_abort() {
	// The interactive path: state-owned run → start → per-frame ticks →
	// completion swaps the document (DocReplaced) and drops the run.
	let mut e = editor();
	let mut tiles = vec![0u8; max_assets::wrl::TILE_DATA_SIZE];
	tiles.fill(40);
	let mut palette = map_core::GAME_PALETTE.to_vec();
	palette[40 * 3..40 * 3 + 3].copy_from_slice(&[0xff, 0x00, 0xee]);
	let wrl = max_assets::wrl::WrlFile {
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
	e.add_doc(Project::from_wrl(&wrl, "STEP"), None, None);
	e.execute(Command::ConvertPaletteModal);

	// An abort mid-run returns to idle with the session dropped (rasterize
	// options: water=keep, strict, threshold irrelevant).
	assert!(matches!(e.palette_convert_start(true, false, 0.0), Outcome::Redraw));
	assert!(e.palette_converting());
	assert!(matches!(e.palette_convert_tick(0.1, true), Outcome::Redraw));
	let m = e.pconvert.as_ref().unwrap();
	assert!(!m.running && m.session.is_none() && m.stage == "Aborted");
	assert!(e.project.packs[0].tiles.iter().all(|&b| b == 40), "abort leaves the document untouched");

	// A full run: bounded ticks make visible progress, completion swaps
	// the document as one undo unit and drops the run.
	assert!(matches!(e.palette_convert_start(true, false, 0.0), Outcome::Redraw));
	let mut ticks = 0;
	let outcome = loop {
		ticks += 1;
		assert!(ticks < 10_000, "conversion never finished");
		match e.palette_convert_tick(ticks as f32 * 0.01, false) {
			Outcome::Redraw => continue,
			other => break other,
		}
	};
	assert!(matches!(outcome, Outcome::DocReplaced));
	assert!(e.pconvert.is_none(), "the run drops on completion (the dialog closes)");
	assert!(!e.project.packs[0].tiles.contains(&40), "pink re-quantized off the static slot");
	assert!(matches!(e.execute(Command::Undo), Outcome::DocReplaced), "one undo restores the document");
	assert!(e.project.packs[0].tiles.iter().all(|&b| b == 40));
}

#[test]
fn delete_clears_selected_ground_without_clipboard() {
	let mut e = editor();
	e.execute(Command::Place { x: 1, y: 1, spec: "GSa000".into() });
	// Nothing selected → a loud no-op.
	assert!(matches!(e.execute(Command::Delete), Outcome::Failed(_)));
	e.execute(Command::SelectRect { x0: 1, y0: 1, x1: 1, y1: 1, mode: SelectMode::Replace });
	assert!(matches!(e.execute(Command::Delete), Outcome::Redraw));
	assert!(e.clipboard.is_none(), "delete is not cut");
	let spec = e.project.cell_spec(1, 1).unwrap_or_default();
	assert!(!spec.contains("GSa000"), "ground cleared: {spec}");
}

#[test]
fn delete_clears_the_active_layer_delete_all_clears_both() {
	let mut e = editor();
	let has = |e: &EditorState, layer: usize| e.project.cell(1, 1).unwrap()[layer].is_some();
	// A cell with the water base + ground on top.
	e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });
	assert!(has(&e, LAYER_WATER) && has(&e, LAYER_GROUND), "starts with water + ground");
	e.execute(Command::SelectRect { x0: 1, y0: 1, x1: 1, y1: 1, mode: SelectMode::Replace });

	// On the ground layer (default), Delete lifts ground and keeps the water.
	assert!(matches!(e.execute(Command::Delete), Outcome::Redraw));
	assert!(!has(&e, LAYER_GROUND) && has(&e, LAYER_WATER), "ground gone, water base kept");

	// On the water layer, the same Delete drops the water - no land/water split.
	e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });
	e.execute(Command::Layer { name: "water".into() });
	assert!(matches!(e.execute(Command::Delete), Outcome::Redraw));
	assert!(has(&e, LAYER_GROUND) && !has(&e, LAYER_WATER), "water gone, ground kept");

	// Delete All empties every layer regardless of which one is active.
	assert!(matches!(e.execute(Command::DeleteAll), Outcome::Redraw));
	assert!(!has(&e, LAYER_GROUND) && !has(&e, LAYER_WATER), "all layers cleared -> a true hole");

	// Both refuse an empty selection.
	e.execute(Command::SelectOp { op: "clear".into() });
	assert!(matches!(e.execute(Command::DeleteAll), Outcome::Failed(_)), "delete-all needs a selection");
}

#[test]
fn quit_request_guards_unsaved_work() {
	let mut e = editor();
	// A fresh map is clean: a quit request goes straight through.
	assert!(!e.dirty());
	assert!(matches!(e.execute(Command::QuitRequest), Outcome::Quit));
	// Dirtying it makes the quit request raise the confirm instead of quitting.
	e.execute(Command::Place { x: 0, y: 0, spec: "GLa000".into() });
	assert!(e.dirty());
	// The quit guard travels as a dialog request (quit=true picks the
	// quit!/save-and-quit command pair), not the tab-close one.
	assert!(
		matches!(e.execute(Command::QuitRequest), Outcome::OpenDialog(DialogRequest::ConfirmClose { quit: true, .. })),
		"quit raises the Save/Discard/Cancel guard"
	);
}

#[test]
fn tabs_stack_switch_and_close() {
	let mut e = editor();
	assert_eq!(e.tab_infos().len(), 1);
	// The first new replaces the bootstrap scratch tab (no stacking).
	new_tab(&mut e, 2);
	assert_eq!(e.tab_infos().len(), 1);
	// Subsequent new/open stack as tabs and activate the newest.
	new_tab(&mut e, 3);
	new_tab(&mut e, 4);
	assert_eq!(e.tab_infos().len(), 3);
	assert_eq!(e.active_tab(), 2);
	// Switching activates another tab; switching to the active one is a no-op.
	assert!(matches!(e.execute(Command::Tab { index: 0 }), Outcome::DocReplaced));
	assert_eq!(e.active_tab(), 0);
	assert!(matches!(e.execute(Command::Tab { index: 0 }), Outcome::Ok));
	// Closing drops a tab (these are clean new maps, so no prompt).
	e.execute(Command::CloseProject { force: false });
	assert_eq!(e.tab_infos().len(), 2);
	e.execute(Command::CloseProject { force: false });
	assert_eq!(e.tab_infos().len(), 1);
	// Closing the last project is allowed - it resets to a blank scratch
	// (one tab, replaceable by the next open/new), app stays open.
	assert!(matches!(e.execute(Command::CloseProject { force: false }), Outcome::DocReplaced));
	assert_eq!(e.tab_infos().len(), 1);
	assert!(e.tabs.replace_scratch);
}

#[test]
fn nav_pan_and_zoom_move_the_view() {
	let mut e = editor();
	let pan0 = e.view.pan;
	e.execute(Command::Pan { dx: 3.0, dy: 2.0 });
	assert_eq!(e.view.pan[0] - pan0[0], 3.0 * TILE_PX as f32, "pan dx = 3 tiles");
	assert_eq!(e.view.pan[1] - pan0[1], 2.0 * TILE_PX as f32, "pan dy = 2 tiles");
	let z = e.view.zoom;
	e.execute(Command::Zoom { factor: 2.0 });
	assert!(e.view.zoom > z, "zoom in grows the zoom");
	e.execute(Command::Zoom { factor: 0.25 });
	assert!(e.view.zoom < 2.0 * z, "zoom out shrinks it back");
	e.execute(Command::Fit);
	assert!((ZOOM_MIN..=ZOOM_MAX).contains(&e.view.zoom), "fit stays in range");
}

#[test]
fn overlay_flags_toggle_through_execute() {
	let mut e = editor();
	// on/off are explicit; a bare/None argument toggles (the unified flag rule).
	e.execute(Command::Grid { on: Some(true) });
	assert!(e.show_grid, "grid on");
	e.execute(Command::Grid { on: Some(false) });
	assert!(!e.show_grid, "grid off");
	e.execute(Command::Grid { on: None });
	assert!(e.show_grid, "grid toggle flips off -> on");
	// The resource overlay toggle (S5) follows the same on/off/toggle rule.
	e.execute(Command::Resources { on: Some(true) });
	assert!(e.show_resources, "resources on");
	e.execute(Command::Resources { on: None });
	assert!(!e.show_resources, "resources toggle flips on -> off");
	let animate = e.animate;
	e.execute(Command::Animate { on: None });
	assert_eq!(e.animate, !animate, "animate toggles");
	e.execute(Command::Crt { on: Some(true) });
	assert!(e.crt, "crt on");
	e.execute(Command::PassOverlay { on: Some(true) });
	assert!(e.show_pass_overlay, "pass overlay on");
}

#[test]
fn select_ops_set_the_mask() {
	let mut e = editor(); // 8x8 = 64 cells
	e.execute(Command::SelectOp { op: "all".into() });
	assert_eq!(e.selection.count(), 64, "select all");
	e.execute(Command::SelectOp { op: "clear".into() });
	assert_eq!(e.selection.count(), 0, "clear");
	e.execute(Command::SelectOp { op: "invert".into() });
	assert_eq!(e.selection.count(), 64, "invert of empty = all");
	e.execute(Command::SelectOp { op: "invert".into() });
	assert_eq!(e.selection.count(), 0, "invert of all = empty");
	assert!(matches!(e.execute(Command::SelectOp { op: "bogus".into() }), Outcome::Failed(_)), "unknown op fails");
}

#[test]
fn set_color_writes_a_dynamic_slot_and_rejects_static() {
	let mut e = editor();
	assert!(matches!(e.execute(Command::SetColor { slot: 100, rgb: [0xaa, 0xbb, 0xcc] }), Outcome::Redraw));
	let at = 100 * 3;
	assert_eq!(&e.project.palette[at..at + 3], &[0xaa, 0xbb, 0xcc], "dynamic slot 100 written");
	// A game-static slot (outside the dynamic 64..=159 range) is refused.
	let out = e.execute(Command::SetColor { slot: 0, rgb: [1, 2, 3] });
	assert!(matches!(out, Outcome::Failed(_)), "static slot refused");
}

#[test]
fn erase_clears_a_painted_ground_cell() {
	let mut e = editor();
	e.execute(Command::Tile { spec: Some("GLa000".into()) });
	e.execute(Command::Paint { x: 3, y: 3 });
	assert!(e.project.cell(3, 3).unwrap()[LAYER_GROUND].is_some(), "painted");
	e.execute(Command::Erase { x: 3, y: 3, layer: None });
	assert!(e.project.cell(3, 3).unwrap()[LAYER_GROUND].is_none(), "erased");
}

#[test]
fn paint_fill_transform_and_hsl_drive_state() {
	let mut e = editor(); // 8×8 GREEN
	// Paint needs an active tile; with one it places onto the ground layer.
	assert!(matches!(e.execute(Command::Paint { x: 0, y: 0 }), Outcome::Failed(_)), "paint needs a tile");
	e.execute(Command::Tile { spec: Some("GLa000".into()) });
	assert!(matches!(e.execute(Command::Paint { x: 2, y: 2 }), Outcome::Redraw));
	assert!(e.project.cell(2, 2).unwrap()[LAYER_GROUND].is_some(), "paint placed the tile");

	// Fill floods the connected empty-ground region with the active tile.
	e.execute(Command::Fill { x: 0, y: 0 });
	let painted = (0..8u16)
		.flat_map(|y| (0..8u16).map(move |x| (x, y)))
		.filter(|&(x, y)| e.project.cell(x, y).unwrap()[LAYER_GROUND].is_some())
		.count();
	assert!(painted > 1, "fill spread to multiple cells (got {painted})");

	// Transform rotates the active paint tile; four cw turns are identity.
	e.execute(Command::Tile { spec: Some("GLa000".into()) });
	assert!(matches!(e.execute(Command::TransformTile { op: "cw".into() }), Outcome::Redraw));
	assert_ne!(e.active_tile.as_deref(), Some("GLa000"), "cw added a transform suffix");
	for _ in 0..3 {
		e.execute(Command::TransformTile { op: "cw".into() });
	}
	assert_eq!(e.active_tile.as_deref(), Some("GLa000"), "4x cw returns to identity");
	assert!(matches!(e.execute(Command::TransformTile { op: "bogus".into() }), Outcome::Failed(_)), "bad op fails");

	// HSL block shift darkens a dynamic slot; a game-static slot is refused.
	e.execute(Command::SetColor { slot: 100, rgb: [120, 120, 120] });
	let before = e.project.palette[100 * 3];
	assert!(matches!(e.execute(Command::HslBlock { slot: 100, dh: 0.0, ds: 0.0, dl: -40.0 }), Outcome::Redraw));
	assert!(e.project.palette[100 * 3] < before, "hsl-block -L darkened the slot");
	let static_shift = e.execute(Command::HslBlock { slot: 0, dh: 0.0, ds: 0.0, dl: 10.0 });
	assert!(matches!(static_shift, Outcome::Failed(_)), "static slot refused");
}

#[test]
fn free_stem_in_bumps_on_collision() {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/free-stem-test");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	// An empty dir: the base name is free.
	assert_eq!(free_stem_in(&dir, "map", None), "map");
	// With `map.json` present it bumps to `map-2`, then `map-3`.
	std::fs::write(dir.join("map.json"), "{}").unwrap();
	assert_eq!(free_stem_in(&dir, "map", None), "map-2");
	std::fs::write(dir.join("map-2.json"), "{}").unwrap();
	assert_eq!(free_stem_in(&dir, "map", None), "map-3");
	// Excluding the colliding file (a rename keeping its own name) frees the base.
	assert_eq!(free_stem_in(&dir, "map", Some(&dir.join("map.json"))), "map");
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn template_pack_joins_terrain_packs_and_excludes_water() {
	let pack = |uses: &str| {
		let json = format!(r#"{{"version":"1","name":"t","width":1,"height":1,"use":{uses},"map":[[""]]}}"#);
		template_pack(&Template::from_str(&json).unwrap())
	};
	assert_eq!(pack(r#"[{"name":"GREEN","version":"1"}]"#), "GREEN", "single pack -> its name");
	// WATER is the universal base layer - excluded from the dir name.
	assert_eq!(pack(r#"[{"name":"WATER","version":"1"},{"name":"CRATER","version":"1"}]"#), "CRATER");
	// Multiple terrain packs: sorted, joined with `+` (regardless of order).
	assert_eq!(pack(r#"[{"name":"GREEN","version":"1"},{"name":"DESERT","version":"1"}]"#), "DESERT+GREEN");
	assert_eq!(pack(r#"[{"name":"WATER","version":"1"}]"#), "WATER", "only WATER -> WATER");
	assert_eq!(pack("[]"), "MISC", "no packs -> MISC");
}

#[test]
fn template_rename_name_uniqueness_is_per_tileset() {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/rename-tileset-test");
	let _ = std::fs::remove_dir_all(&dir);
	let (a, b) = (dir.join("A"), dir.join("B"));
	std::fs::create_dir_all(&a).unwrap();
	std::fs::create_dir_all(&b).unwrap();
	let mk = |dir: &Path, name: &str, pack: &str| {
		let t = Template {
			name: name.to_string(),
			width: 1,
			height: 1,
			uses: vec![(pack.to_string(), "1".to_string())],
			cells: vec![String::new()],
		};
		t.save(&dir.join(format!("{name}.json"))).unwrap();
		TemplateEntry { name: t.name.clone(), path: dir.join(format!("{name}.json")), stock: false, template: t }
	};
	let mut e = editor();
	// Tileset A holds "Shared" + "Taken"; tileset B holds another "Shared".
	e.templates.entries = vec![mk(&a, "Shared", "A"), mk(&b, "Shared", "B"), mk(&a, "Taken", "A")];

	// Renaming A's "Shared" onto "Taken" (same tileset) is rejected...
	e.templates.sel = Some(0);
	assert!(
		matches!(e.execute(Command::TemplateRename { from: "Shared".into(), to: "Taken".into() }), Outcome::Failed(_)),
		"same-tileset name collision is rejected",
	);

	// ...but the *selected* duplicate is the one renamed (not the first by name),
	// and a target that only exists in another tileset is allowed. Rename B's
	// "Shared" (index 1) → "Taken": B has no "Taken", so it succeeds and touches
	// B, leaving A's "Shared" alone.
	e.templates.sel = Some(1);
	assert!(
		!matches!(e.execute(Command::TemplateRename { from: "Shared".into(), to: "Taken".into() }), Outcome::Failed(_)),
		"a name used only in another tileset is allowed",
	);
	assert!(b.join("taken.json").exists(), "B's Shared was renamed (sanitized filename)");
	assert!(!b.join("Shared.json").exists(), "B's old file is gone");
	assert!(a.join("Shared.json").exists(), "A's Shared was untouched - the selected dup was renamed");
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn template_pick_arms_the_selected_entry_not_the_first_by_name() {
	// Two templates share the display name "Shared" but belong to different
	// tilesets (packs A and B). Empty cells → both compatible with any map, so
	// resolution - not `missing_id` - is what's under test. The explorer arms
	// the exact entry clicked, so `template-pick` must honour the selection
	// rather than grabbing the first "Shared" by entry order.
	let mk = |name: &str, pack: &str| {
		let t = Template {
			name: name.to_string(),
			width: 1,
			height: 1,
			uses: vec![(pack.to_string(), "1".to_string())],
			cells: vec![String::new()],
		};
		TemplateEntry {
			name: t.name.clone(),
			path: PathBuf::from(format!("{pack}/{name}.json")),
			stock: false,
			template: t,
		}
	};
	let mut e = editor();
	e.templates.entries = vec![mk("Shared", "A"), mk("Shared", "B")];

	// Selecting B's "Shared" (index 1) arms B's template, not A's (index 0).
	e.templates.sel = Some(1);
	assert!(!matches!(e.execute(Command::TemplatePick { name: "Shared".into() }), Outcome::Failed(_)));
	assert_eq!(e.stamp.as_ref().unwrap().uses[0].0, "B", "the selected entry is armed");
	assert_eq!(e.templates.sel, Some(1), "selection stays on the picked entry");

	// With no matching selection, the scripted path falls back to first-by-name.
	e.stamp = None;
	e.templates.sel = None;
	assert!(!matches!(e.execute(Command::TemplatePick { name: "Shared".into() }), Outcome::Failed(_)));
	assert_eq!(e.stamp.as_ref().unwrap().uses[0].0, "A", "no selection -> first match (scripted path)");
}

#[test]
fn template_context_items_adapt_to_stock_vs_user() {
	let labels = |items: &[menu::Item]| -> Vec<String> {
		items
			.iter()
			.filter_map(|it| match it {
				menu::Item::Action { label, .. } => Some(label.clone()),
				_ => None,
			})
			.collect()
	};
	let mk = |name: &str, stock: bool| TemplateEntry {
		name: name.into(),
		path: PathBuf::from(format!("{name}.json")),
		stock,
		template: Template {
			name: name.into(),
			width: 1,
			height: 1,
			uses: vec![("GREEN".into(), String::new())],
			cells: vec!["GLa000".into()],
		},
	};
	let mut e = editor();
	e.templates.entries = vec![mk("mine", false), mk("shipped", true)];

	// A user template: Use, Rename, Duplicate, Delete, Export as PNG.
	e.templates.sel = Some(0);
	let user = labels(&e.template_context_items());
	for want in ["Use", "Rename", "Duplicate", "Delete", "Export as PNG"] {
		assert!(user.iter().any(|l| l == want), "user menu has {want}: {user:?}");
	}
	// A stock template is read-only: no Rename/Delete, but Duplicate + Export stay.
	e.templates.sel = Some(1);
	let stock = labels(&e.template_context_items());
	assert!(!stock.iter().any(|l| l == "Rename"), "stock can't be renamed");
	assert!(!stock.iter().any(|l| l == "Delete"), "stock can't be deleted");
	for want in ["Use", "Duplicate", "Export as PNG"] {
		assert!(stock.iter().any(|l| l == want), "stock menu has {want}: {stock:?}");
	}
	// --dev unlocks the stock template: Rename + Delete come back.
	e.dev_mode = true;
	let dev_stock = labels(&e.template_context_items());
	for want in ["Use", "Rename", "Duplicate", "Delete", "Export as PNG"] {
		assert!(dev_stock.iter().any(|l| l == want), "dev stock menu has {want}: {dev_stock:?}");
	}
}

#[test]
fn dev_mode_unlocks_stock_template_rename_and_delete() {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/dev-stock-template-test");
	let _ = std::fs::remove_dir_all(&dir);
	let pack_dir = dir.join("GREEN");
	std::fs::create_dir_all(&pack_dir).unwrap();
	let t = Template {
		name: "ridge".into(),
		width: 1,
		height: 1,
		uses: vec![("GREEN".into(), String::new())],
		cells: vec!["GLa000".into()],
	};
	let path = pack_dir.join("ridge.json");
	t.save(&path).unwrap();
	let entry = || TemplateEntry { name: "ridge".into(), path: path.clone(), stock: true, template: t.clone() };

	// Without --dev: the rename/delete modals and the delete itself are refused,
	// and the stock file is left on disk.
	let mut e = editor();
	e.templates.entries = vec![entry()];
	e.templates.sel = Some(0);
	assert!(matches!(e.execute(Command::TemplateRenameModal), Outcome::Failed(_)), "no --dev: rename refused");
	assert!(matches!(e.execute(Command::TemplateDeleteModal), Outcome::Failed(_)), "no --dev: delete refused");
	assert!(matches!(e.execute(Command::TemplateDelete { name: None }), Outcome::Failed(_)));
	assert!(path.exists(), "the stock file survives without --dev");

	// With --dev: the modal opener no longer refuses, and the delete removes the
	// stock file. (A fresh editor so the opened modal doesn't linger.)
	let mut e = editor();
	e.dev_mode = true;
	e.templates.entries = vec![entry()];
	e.templates.sel = Some(0);
	assert!(
		matches!(e.execute(Command::TemplateRenameModal), Outcome::OpenDialog(DialogRequest::RenameTemplate { .. })),
		"--dev: rename opens"
	);
	assert!(!matches!(e.execute(Command::TemplateDelete { name: None }), Outcome::Failed(_)), "--dev: delete runs");
	assert!(!path.exists(), "--dev removed the stock template file");
	let _ = std::fs::remove_dir_all(&dir);
}

/// The greyed header keys are never the only guard: with nothing selected,
/// the verbs behind them still refuse with a message - scripts, the console
/// and keybindings reach these commands without ever seeing a panel (the
/// disabled-dead convention's standing constraint, audit item 5).
#[test]
fn panel_verbs_refuse_loudly_with_nothing_selected() {
	let mut e = editor();
	assert!(e.active_tile.is_none() && e.templates.sel.is_none(), "the fixture starts unselected");
	assert!(matches!(e.execute(Command::TilePaintClone), Outcome::Failed(m) if m.contains("select a tile")));
	assert!(matches!(e.execute(Command::TilePaintEdit), Outcome::Failed(m) if m.contains("select a tile")));
	assert!(matches!(e.execute(Command::TileDelete), Outcome::Failed(m) if m.contains("select a tile")));
	assert!(
		matches!(e.execute(Command::TemplateRenameModal), Outcome::Failed(m) if m.contains("no template selected"))
	);
	assert!(
		matches!(e.execute(Command::TemplateDeleteModal), Outcome::Failed(m) if m.contains("no template selected"))
	);
}

#[test]
fn template_export_png_writes_one_image_cell_per_template_cell() {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/template-png-test");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let mut e = editor();
	// A real 2x1 template: ground over the project's actual water spec, then bare
	// ground - so every id resolves and the tiles rasterize.
	let water = e.project.cell_spec(0, 0).unwrap();
	let t = Template {
		name: "ridge".into(),
		width: 2,
		height: 1,
		uses: vec![("GREEN".into(), String::new()), ("WATER".into(), String::new())],
		cells: vec![format!("{water},GLa000"), "GLa001".into()],
	};
	e.templates.entries =
		vec![TemplateEntry { name: t.name.clone(), path: dir.join("ridge.json"), stock: false, template: t }];

	// No selection → refused; the bare command (no path) just opens the dialog,
	// which is unavailable headless.
	e.templates.sel = None;
	assert!(matches!(e.execute(Command::TemplateExportPng { path: None }), Outcome::Failed(_)));

	e.templates.sel = Some(0);
	let png = dir.join("ridge.png");
	assert!(matches!(e.execute(Command::TemplateExportPng { path: Some(png.clone()) }), Outcome::Redraw));
	let (rgba, w, h) = decode_png_rgba(&png).expect("decode the exported png");
	assert_eq!((w, h), (2 * 64, 64), "one 64px image cell per template cell");
	assert!(rgba.chunks_exact(4).any(|p| p[3] == 255), "the ground tiles rasterize opaque pixels");
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_then_open_round_trips_the_project_on_disk() {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/save-roundtrip-test");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let path = dir.join("m.json");

	let mut e = editor();
	e.execute(Command::Tile { spec: Some("GLa000".into()) });
	e.execute(Command::Paint { x: 1, y: 1 });
	e.execute(Command::SetColor { slot: 100, rgb: [0x12, 0x34, 0x56] }); // a palette override to carry
	let saved_hash = e.project.hash();
	assert!(matches!(e.execute(Command::Save { path: Some(path.clone()) }), Outcome::Ok | Outcome::Redraw));
	assert!(path.exists(), "the project file was written");
	assert!(!e.dirty(), "save cleared the dirty flag");

	// Reload into a fresh editor: the document hashes identically.
	let mut e2 = editor();
	e2.execute(Command::Open { path });
	assert_eq!(e2.project.hash(), saved_hash, "reloaded project matches what was saved");
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dirty_tab_guards_close_and_quit() {
	let mut e = editor();
	new_tab(&mut e, 2); // replaces scratch
	new_tab(&mut e, 3); // second tab, active
	// Dirty the active tab.
	e.execute(Command::Place { x: 0, y: 0, spec: "GSa000".into() });
	assert!(e.dirty());
	// Closing a dirty tab raises the Save/Discard/Cancel guard (as a dialog
	// request; quit=false picks the save-and-close/close-project! pair).
	assert!(matches!(
		e.execute(Command::CloseProject { force: false }),
		Outcome::OpenDialog(DialogRequest::ConfirmClose { quit: false, .. })
	));
	// Discard (`close-project!`) closes despite the unsaved changes.
	assert!(matches!(e.execute(Command::CloseProject { force: true }), Outcome::DocReplaced));
	// Quit guards on ANY open tab being dirty.
	e.execute(Command::Place { x: 1, y: 1, spec: "GSa000".into() });
	assert!(matches!(e.execute(Command::Quit { force: false }), Outcome::Failed(_)));
	assert!(matches!(e.execute(Command::Quit { force: true }), Outcome::Quit));
}

#[test]
fn reopening_a_path_switches_instead_of_stacking() {
	let mut e = editor();
	// Two distinct in-memory tabs first (path-less new maps never dedup).
	new_tab(&mut e, 2);
	new_tab(&mut e, 3);
	assert_eq!(e.tab_infos().len(), 2);
	// Switching keeps per-tab state independent: dirty one, switch away, back.
	e.execute(Command::Place { x: 0, y: 0, spec: "GSa000".into() });
	assert!(e.dirty());
	e.execute(Command::Tab { index: 0 });
	assert!(!e.dirty(), "tab 0 is its own clean document");
	e.execute(Command::Tab { index: 1 });
	assert!(e.dirty(), "tab 1's edit survived the switch");
}

#[test]
fn ui_scale_shrinks_the_logical_ui_size() {
	// `ui_screen` is what the chrome lays out in: physical / scale. (Set the
	// field directly rather than `set_ui_scale`, which also writes a process
	// global the parallel font tests read.)
	let mut e = editor();
	assert_eq!(e.ui_scale, 1.0);
	assert_eq!(e.ui_screen(), (800.0, 600.0)); // 1.0: logical == physical
	e.ui_scale = 1.25;
	assert_eq!(e.ui_screen(), (640.0, 480.0)); // 125% of an 800×600 target
	e.ui_scale = 1.5;
	let (lw, lh) = e.ui_screen();
	assert!((lw - 533.333).abs() < 0.01 && (lh - 400.0).abs() < 0.01, "150%: {lw}x{lh}");
}

/// An editor whose `resources_root` is a scratch dir under `temp/<tag>` -
/// user packs / templates / palettes land there, never in the repo's real
/// `resources/`. The project still loads the shipped GREEN pack
/// (read-only); with no `assets/tilepacks/<PACK>` dir under the scratch
/// root, no pack counts as "stock", so tile edits stay in memory.
fn temp_editor(tag: &str) -> (EditorState, PathBuf) {
	let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp").join(tag);
	let _ = std::fs::remove_dir_all(&root);
	std::fs::create_dir_all(&root).unwrap();
	let project = Project::new(8, 8, &["GREEN".to_string()], &resources().join("assets/tilepacks"), 1).unwrap();
	(EditorState::new(project, (800, 600), None, root.clone()), root)
}

/// A tiny 1×1 in-memory WRL whose single tile is filled with palette index
/// `fill` - the synthetic-pack (WRL-import) document several tests need. Its
/// one tile is passability class water (`pass = 1`) so it composes onto the
/// base/water layer that `base_tile`/`set-tile` read and write.
fn tiny_wrl(fill: u8) -> max_assets::wrl::WrlFile {
	max_assets::wrl::WrlFile {
		header: vec![0; 5],
		width: 1,
		height: 1,
		minimap: vec![0],
		bigmap: vec![0],
		tile_count: 1,
		tiles: vec![fill; max_assets::wrl::TILE_DATA_SIZE],
		palette: map_core::GAME_PALETTE.to_vec(),
		pass_table: vec![1],
	}
}

/// `decode_png_rgba` normalizes every 8-bit source the `png` crate emits
/// (RGB, grayscale, gray+alpha, indexed with/without transparency) to
/// RGBA, and rejects 16-bit files and out-of-palette indices with clear
/// errors instead of panicking.
#[test]
fn decode_png_rgba_normalizes_every_8bit_color_type() {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/decode-png-test");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	type Pal<'a> = Option<(&'a [u8], Option<&'a [u8]>)>;
	let write = |name: &str, color: png::ColorType, depth: png::BitDepth, pal: Pal, data: &[u8]| -> PathBuf {
		let path = dir.join(name);
		let file = std::fs::File::create(&path).unwrap();
		let mut enc = png::Encoder::new(std::io::BufWriter::new(file), 2, 1);
		enc.set_color(color);
		enc.set_depth(depth);
		if let Some((plte, trns)) = pal {
			enc.set_palette(plte.to_vec());
			if let Some(t) = trns {
				enc.set_trns(t.to_vec());
			}
		}
		enc.write_header().unwrap().write_image_data(data).unwrap();
		path
	};

	// RGB gains an opaque alpha channel.
	let p = write("rgb.png", png::ColorType::Rgb, png::BitDepth::Eight, None, &[1, 2, 3, 4, 5, 6]);
	assert_eq!(decode_png_rgba(&p).unwrap(), (vec![1, 2, 3, 255, 4, 5, 6, 255], 2, 1));
	// Grayscale replicates the value across RGB.
	let p = write("gray.png", png::ColorType::Grayscale, png::BitDepth::Eight, None, &[7, 8]);
	assert_eq!(decode_png_rgba(&p).unwrap(), (vec![7, 7, 7, 255, 8, 8, 8, 255], 2, 1));
	// Gray+alpha keeps its own alpha.
	let p = write("ga.png", png::ColorType::GrayscaleAlpha, png::BitDepth::Eight, None, &[9, 128, 10, 255]);
	assert_eq!(decode_png_rgba(&p).unwrap(), (vec![9, 9, 9, 128, 10, 10, 10, 255], 2, 1));
	// Indexed resolves through the PLTE palette; tRNS drives per-index
	// alpha (index 0 transparent here), missing entries default opaque.
	let plte = [10u8, 20, 30, 40, 50, 60];
	let p = write("idx.png", png::ColorType::Indexed, png::BitDepth::Eight, Some((&plte, Some(&[0]))), &[0, 1]);
	assert_eq!(decode_png_rgba(&p).unwrap(), (vec![10, 20, 30, 0, 40, 50, 60, 255], 2, 1));
	// An index past the palette is a crafted-file error, not a panic.
	let p = write("oob.png", png::ColorType::Indexed, png::BitDepth::Eight, Some((&plte[..3], None)), &[0, 5]);
	assert!(decode_png_rgba(&p).unwrap_err().contains("outside the palette"));
	// 16-bit depth is refused with re-export advice.
	let p = write("deep.png", png::ColorType::Grayscale, png::BitDepth::Sixteen, None, &[0, 1, 0, 2]);
	assert!(decode_png_rgba(&p).unwrap_err().contains("re-export as 8-bit"));
	let _ = std::fs::remove_dir_all(&dir);
}

/// A zero-sized or truncated shape image degrades to "all water" (the new
/// map's base fill) instead of indexing out of the pixel buffer.
#[test]
fn shape_land_mask_defends_short_input() {
	assert_eq!(shape_land_mask(&[], 0, 0, 2, 1), vec![false, false], "no image -> all water");
	assert_eq!(shape_land_mask(&[0, 200, 0, 255], 2, 2, 1, 1), vec![false], "truncated pixels -> all water");
}

/// The unmapped-tile review classifies pass bytes into terrain words;
/// anything unknown reads as land (the WRL default).
#[test]
fn class_name_maps_pass_bytes_to_terrain_words() {
	assert_eq!(class_name(0), "land");
	assert_eq!(class_name(1), "water");
	assert_eq!(class_name(2), "shore");
	assert_eq!(class_name(3), "blocked");
	assert_eq!(class_name(9), "land", "unknown bytes read as land");
}

/// The generate report (console) and status lines (modal) carry the seed,
/// the counts, the symmetry tag, and the leftover-seam warning - the seed
/// line first, since it's what gets copied to re-make a map.
#[test]
fn generate_reports_name_seed_counts_and_leftover_seams() {
	let mut p = map_core::GenParams::defaults(map_core::Generator::Islands);
	p.seed = 42;
	let clean = map_core::GenStats { water: 10, land: 20, obstructions: 1, decorations: 2, shore: 3, unresolved: 0 };
	let report = generate_report(&p, &clean);
	assert!(report.contains("seed 42") && report.contains("10 water / 20 land"), "{report}");
	assert!(!report.contains("seams left"), "clean runs don't warn: {report}");
	assert_eq!(generate_status_lines(&p, &clean).len(), 4, "seed/cells/features/shore rows");

	p.symmetry = map_core::Symmetry::LeftRight;
	let dirty = map_core::GenStats { unresolved: 5, ..clean };
	let report = generate_report(&p, &dirty);
	assert!(report.contains('[') && report.contains("5 seams left"), "{report}");
	let lines = generate_status_lines(&p, &dirty);
	assert_eq!(lines.len(), 5, "the seam warning gets its own row");
	assert!(lines[0].contains("seed 42"), "the seed line stays first: {:?}", lines);
}

/// The rename-cascade file scan collects `.json` files recursively and
/// nothing else; a missing directory is quietly empty.
#[test]
fn collect_json_files_recurses_and_keeps_only_json() {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/collect-json-test");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(dir.join("sub/deep")).unwrap();
	std::fs::write(dir.join("a.json"), "{}").unwrap();
	std::fs::write(dir.join("sub/deep/b.json"), "{}").unwrap();
	std::fs::write(dir.join("sub/readme.txt"), "x").unwrap();
	let mut out = Vec::new();
	collect_json_files(&dir, &mut out);
	out.sort();
	assert_eq!(out, vec![dir.join("a.json"), dir.join("sub/deep/b.json")]);
	let mut none = Vec::new();
	collect_json_files(&dir.join("nope"), &mut none);
	assert!(none.is_empty(), "a missing dir contributes nothing");
	let _ = std::fs::remove_dir_all(&dir);
}

/// Template-purpose dialogs without a known user-templates dir fall back
/// to the current directory rather than inventing a path.
#[test]
fn dialog_template_dir_without_a_home_falls_back_to_cwd() {
	use crate::command::FilePurpose::ImportTemplate;
	assert_eq!(dialog_default_dir(ImportTemplate, Path::new("/r"), None, None, None, None), PathBuf::from("."));
}

#[test]
fn sanitize_filename_trims_trailing_dashes() {
	assert_eq!(sanitize_filename("name-"), "name");
	assert_eq!(sanitize_filename("a - "), "a", "a dash left dangling by trimming is dropped");
}

#[test]
fn dialog_path_policy_follows_purpose() {
	use crate::command::FilePurpose::*;
	let tmp = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/dialog-policy-test");
	let _ = std::fs::remove_dir_all(&tmp);
	let res = tmp.join("resources");
	let doc = PathBuf::from("/maps/proj/forest.json");
	let user_templates = res.join("user/templates");

	// Palette purposes always land in (and create) user/palettes.
	let pal = dialog_default_dir(SavePalette, &res, None, None, None, None);
	assert_eq!(pal, res.join("user/palettes"));
	assert!(pal.is_dir(), "palette dir is created on first use");
	// Templates land in (and create) the user templates dir.
	assert_eq!(dialog_default_dir(ImportTemplate, &res, None, None, None, Some(&user_templates)), user_templates);
	assert!(user_templates.is_dir());
	// Maps: the open doc's folder wins; with no doc, Load falls back to
	// assets/maps (not created), Save to user/maps (created).
	assert_eq!(dialog_default_dir(Load, &res, Some(&doc), None, None, None), Path::new("/maps/proj"));
	assert_eq!(dialog_default_dir(Load, &res, None, None, None, None), res.join("assets/maps"));
	assert_eq!(dialog_default_dir(SaveAs, &res, None, None, None, None), res.join("user/maps"));
	assert!(res.join("user/maps").is_dir(), "save destination is created");
	assert!(!res.join("maps").exists(), "no stray maps dir at the resources root");
	// Saved games open in MaxPortPath when it exists, else fall back to
	// assets/maps (the `.DTA` picker's start dir; doc folder is ignored).
	let mport = tmp.join("maxport");
	std::fs::create_dir_all(&mport).unwrap();
	assert_eq!(dialog_default_dir(OpenSave, &res, Some(&doc), None, Some(&mport), None), mport);
	assert_eq!(dialog_default_dir(OpenSave, &res, None, None, None, None), res.join("assets/maps"));

	// Suggested names ensure a `.json` extension; only save-style purposes pre-fill.
	assert_eq!(dialog_suggested_name(SaveAs, Some(&doc), "Untitled").as_deref(), Some("forest.json"));
	assert_eq!(dialog_suggested_name(SaveCopy, None, "My Map").as_deref(), Some("My Map.json"));
	assert_eq!(dialog_suggested_name(SavePalette, None, "swamp").as_deref(), Some("swamp.json"));
	assert_eq!(dialog_suggested_name(Load, Some(&doc), "x"), None);
	// WRL export pre-fills an uppercase `.WRL` name (the doc's stem, else the
	// project name) and lands in the same save dir as a project save.
	assert_eq!(dialog_suggested_name(ExportWrl, Some(&doc), "Untitled").as_deref(), Some("forest.WRL"));
	assert_eq!(dialog_suggested_name(ExportWrl, None, "My Map").as_deref(), Some("My Map.WRL"));
	// A doc already ending in `.WRL` keeps a single extension (not `.WRL.WRL`).
	let wrl_doc = Path::new("/maps/atoll.WRL");
	assert_eq!(dialog_suggested_name(ExportWrl, Some(wrl_doc), "x").as_deref(), Some("atoll.WRL"));
	assert_eq!(dialog_default_dir(ExportWrl, &res, None, None, None, None), res.join("user/maps"));

	let _ = std::fs::remove_dir_all(&tmp);
}

/// `zoom-at` multiplies zoom keeping the world point under the given
/// screen point stationary (the wheel-zoom contract).
#[test]
fn zoom_at_anchors_the_point_under_the_cursor() {
	let mut e = editor();
	let anchor = e.cell_at(200.0, 150.0);
	assert!(anchor.is_some(), "the probe point sits on the fitted map");
	let z = e.view.zoom;
	assert!(matches!(e.execute(Command::ZoomAt { x: 200.0, y: 150.0, factor: 2.0 }), Outcome::Redraw));
	assert_eq!(e.view.zoom, (z * 2.0).clamp(ZOOM_MIN, ZOOM_MAX));
	assert_eq!(e.cell_at(200.0, 150.0), anchor, "the anchored cell stays under the cursor");
}

/// The script asserts fail loudly on a mismatch, naming both sides.
#[test]
fn script_asserts_fail_loudly_on_mismatch() {
	let mut e = editor();
	assert!(matches!(e.execute(Command::AssertDirty { dirty: true }), Outcome::Failed(_)), "a fresh doc is clean");
	assert!(matches!(e.execute(Command::AssertHash { hash: 0 }), Outcome::Failed(_)));
	e.add_doc(Project::from_wrl(&tiny_wrl(40), "FLAT"), None, None);
	assert!(matches!(e.execute(Command::AssertTile { x: 0, y: 0, tile: 7 }), Outcome::Failed(_)));
	assert!(matches!(e.execute(Command::AssertTile { x: 0, y: 0, tile: 0 }), Outcome::Ok));
}

/// `set-tile` no-ops quietly when the cell already shows the tile, and
/// `set-pass` is retired in favour of the Pass Table Editor.
#[test]
fn set_tile_noops_and_set_pass_is_retired() {
	let mut e = editor();
	e.add_doc(Project::from_wrl(&tiny_wrl(40), "FLAT"), None, None);
	assert!(matches!(e.execute(Command::SetTile { x: 0, y: 0, tile: 0 }), Outcome::Ok), "same tile -> no-op");
	let out = e.execute(Command::SetPass { tile: 0, value: 1 });
	assert!(matches!(out, Outcome::Failed(msg) if msg.contains("retired")), "set-pass points at pass-paint");
}

/// place / erase / assert-cell edge behavior: repeat placement no-ops, bad
/// specs and layers fail, the eraser falls back to the water layer, and
/// assert-cell reports the actual stack.
#[test]
fn place_erase_and_assert_cell_report_precise_errors() {
	let mut e = editor();
	assert!(matches!(e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() }), Outcome::Redraw));
	assert!(matches!(e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() }), Outcome::Ok), "no-op repeat");
	assert!(matches!(e.execute(Command::Place { x: 1, y: 1, spec: "ZZZ999".into() }), Outcome::Failed(_)));
	assert!(matches!(e.execute(Command::Erase { x: 1, y: 1, layer: Some("lava".into()) }), Outcome::Failed(_)));
	// No ground at (5,5): the unspecified-layer eraser drops the water base.
	assert!(matches!(e.execute(Command::Erase { x: 5, y: 5, layer: None }), Outcome::Redraw));
	assert!(e.project.cell(5, 5).unwrap()[LAYER_WATER].is_none(), "water base erased via the fallback");
	let out = e.execute(Command::AssertCell { x: 1, y: 1, spec: "WRONG".into() });
	assert!(matches!(out, Outcome::Failed(msg) if msg.contains("expected 'WRONG'")), "mismatch names both sides");
	assert!(matches!(e.execute(Command::AssertCell { x: 60, y: 0, spec: "-".into() }), Outcome::Failed(_)));
}

/// `new` rejects unknown packs loudly; without a seed it rolls a fresh one
/// and still opens the map (the interactive default).
#[test]
fn new_command_rolls_a_seed_and_rejects_unknown_packs() {
	let mut e = editor();
	assert!(matches!(
		e.execute(Command::New { width: 8, height: 8, packs: vec!["NOPE".into()], seed: Some(1) }),
		Outcome::Failed(_)
	));
	assert!(matches!(
		e.execute(Command::New { width: 8, height: 8, packs: vec!["GREEN".into()], seed: None }),
		Outcome::DocReplaced
	));
}

/// The `tile` command's three shapes: bare reports the brush, `-` clears
/// it, and an unresolvable spec is refused (the brush stays valid).
#[test]
fn tile_command_reports_clears_and_validates_the_brush() {
	let mut e = editor();
	e.execute(Command::Tile { spec: Some("GLa000".into()) });
	assert!(e.can_paint());
	assert!(matches!(e.execute(Command::Tile { spec: None }), Outcome::Redraw));
	assert!(e.console.log().last().unwrap().contains("GLa000"), "bare `tile` reports the active brush");
	assert!(matches!(e.execute(Command::Tile { spec: Some("-".into()) }), Outcome::Redraw));
	assert_eq!(e.active_tile(), None, "`tile -` clears the brush");
	assert!(!e.can_paint());
	assert!(matches!(e.execute(Command::Tile { spec: Some("ZZZ999".into()) }), Outcome::Failed(_)));
}

/// paint / fill guards: the unit tool intercepts paint, a stale brush spec
/// fails at resolve, and the randomize toggle picks group variants for
/// both paint and the selection-confined fill.
#[test]
fn paint_and_fill_guard_the_brush_and_randomize() {
	let mut e = editor();
	e.execute(Command::ToolSelect { name: "unit".into() });
	assert!(matches!(e.execute(Command::Paint { x: 1, y: 1 }), Outcome::Failed(_)), "unit tool, nothing armed");
	e.execute(Command::ToolSelect { name: "pencil".into() });
	// A brush spec left over from another document fails at resolve.
	e.active_tile = Some("ZZZ999".into());
	assert!(matches!(e.execute(Command::Paint { x: 1, y: 1 }), Outcome::Failed(_)));
	assert!(matches!(e.execute(Command::Fill { x: 1, y: 1 }), Outcome::Failed(_)));
	e.active_tile = None;
	assert!(matches!(e.execute(Command::Fill { x: 1, y: 1 }), Outcome::Failed(_)), "fill needs a brush");
	e.execute(Command::Tile { spec: Some("GLa000".into()) });
	e.execute(Command::Randomize { on: Some(true) });
	assert!(matches!(e.execute(Command::Paint { x: 4, y: 4 }), Outcome::Redraw));
	assert!(e.project.cell(4, 4).unwrap()[LAYER_GROUND].is_some(), "randomized paint still lands");
	e.execute(Command::SelectRect { x0: 0, y0: 0, x1: 1, y1: 1, mode: SelectMode::Replace });
	assert!(matches!(e.execute(Command::Fill { x: 6, y: 6 }), Outcome::Redraw));
	assert!(e.project.cell(0, 0).unwrap()[LAYER_GROUND].is_some(), "randomized selection fill painted");
}

/// The terrain brush needs a LAND / WATER variant group - a WRL import's
/// synthetic pack has neither, so both materials refuse with the group
/// named.
#[test]
fn terrain_brush_needs_a_variant_group() {
	let mut e = editor();
	e.add_doc(Project::from_wrl(&tiny_wrl(40), "FLAT"), None, None);
	e.execute(Command::ToolSelect { name: "paint-land".into() });
	let out = e.execute(Command::PaintMask { x: 0, y: 0 });
	assert!(matches!(out, Outcome::Failed(msg) if msg.contains("LAND variant group")));
	e.execute(Command::ToolSelect { name: "paint-water".into() });
	let out = e.execute(Command::PaintMask { x: 0, y: 0 });
	assert!(matches!(out, Outcome::Failed(msg) if msg.contains("WATER variant group")));
}

/// Every name-taking command refuses an unknown word instead of guessing.
#[test]
fn name_taking_commands_reject_unknown_words() {
	let mut e = editor();
	for (cmd, what) in [
		(Command::BrushShape { shape: "hex".into() }, "brush-shape"),
		(Command::Layer { name: "lava".into() }, "layer"),
		(Command::ToolSelect { name: "sprayer".into() }, "tool"),
		(Command::Mode { name: "3d".into() }, "mode"),
		(Command::MinimapMode { mode: "radar".into() }, "minimap"),
		(Command::PickerFilter { name: "sparkly".into() }, "picker filter"),
		(Command::PickerSize { size: "7".into() }, "picker size"),
		(Command::MenuOpen { name: "bogus".into() }, "menu"),
		(Command::Window { id: "bogus".into(), on: None }, "window"),
		(Command::DockTo { id: "minimap".into(), place: "diagonal".into(), at: None }, "dock"),
	] {
		assert!(matches!(e.execute(cmd), Outcome::Failed(_)), "{what} must reject the unknown word");
	}
}

/// Pass values are 0..=3 everywhere; clearing a cell that carries no
/// override is a quiet no-op.
#[test]
fn pass_values_clamp_to_the_editor_range() {
	let mut e = editor();
	assert!(matches!(e.execute(Command::PassPick { value: 9 }), Outcome::Failed(_)));
	assert!(matches!(e.execute(Command::PassPaint { x: 0, y: 0, value: 9 }), Outcome::Failed(_)));
	assert!(matches!(e.execute(Command::TilePass { x: 0, y: 0, value: 9 }), Outcome::Failed(_)));
	assert!(matches!(e.execute(Command::PassClear { x: 0, y: 0 }), Outcome::Ok), "no override -> no-op");
}

/// A `--dev` pass edit on a pack that is NOT stock (its folder isn't under
/// `assets_root`) queues nothing for Bake - only shipped packs bake.
#[test]
fn dev_pass_edit_on_a_non_stock_pack_queues_no_bake() {
	let (mut e, root) = temp_editor("tilepass-nonstock-test");
	e.dev_mode = true;
	e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });
	let cur = e.project.pass_at(1, 1).unwrap();
	let value = if cur == 3 { 0 } else { 3 };
	assert!(matches!(e.execute(Command::TilePass { x: 1, y: 1, value }), Outcome::Redraw));
	assert!(e.tile_ops.dirty_packs.is_empty(), "no stock dir under the scratch root -> nothing to bake");
	let _ = std::fs::remove_dir_all(&root);
}

/// With a stamp armed, `transform` turns the whole stamp (the footprint
/// swaps on quarter turns); without one, a corrupt active-tile suffix is
/// refused at parse.
#[test]
fn transform_turns_the_armed_stamp_not_the_brush() {
	let mut e = editor();
	e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });
	e.execute(Command::SelectRect { x0: 1, y0: 1, x1: 2, y1: 1, mode: SelectMode::Replace });
	e.execute(Command::Copy);
	e.execute(Command::Paste);
	let armed = e.stamp.as_ref().expect("paste arms the stamp");
	assert_eq!((armed.width, armed.height), (2, 1));
	assert!(matches!(e.execute(Command::TransformTile { op: "cw".into() }), Outcome::Redraw));
	let turned = e.stamp.as_ref().unwrap();
	assert_eq!((turned.width, turned.height), (1, 2), "cw turned the footprint");
	assert!(matches!(e.execute(Command::TransformTile { op: "bogus".into() }), Outcome::Failed(_)));
	// With no stamp, the transform falls to the single active tile (the base
	// drives the branch now, so clear it too).
	e.stamp = None;
	e.stamp_base = None;
	e.active_tile = Some("GLa000:??".into());
	assert!(matches!(e.execute(Command::TransformTile { op: "cw".into() }), Outcome::Failed(_)));
}

/// The 8-orientation grid's `Orient` command: it sets a single tile's
/// transform suffix (refusing an orientation the family forbids), and
/// re-derives an armed stamp from its unchanged base.
#[test]
fn orient_transforms_the_tile_or_stamp() {
	let mut e = editor();
	// Single tile (GLa = Free): any orientation is allowed and sets the suffix.
	e.execute(Command::Tile { spec: Some("GLa000".into()) });
	let t = map_core::Transform { rot: 1, mirror: true };
	assert!(e.orient_allowed(t));
	assert!(matches!(e.execute(Command::Orient { rot: 1, mirror: true }), Outcome::Redraw));
	assert_eq!(e.active_tile().unwrap(), format!("GLa000{}", t.suffix()));
	// A No-family tile (GLc) refuses any non-identity orientation.
	e.execute(Command::Tile { spec: Some("GLc000".into()) });
	assert!(!e.orient_allowed(map_core::Transform { rot: 1, mirror: false }));
	assert!(matches!(e.execute(Command::Orient { rot: 1, mirror: false }), Outcome::Failed(_)));

	// A stamp: orient re-derives it from the base (footprint swaps); the base
	// and the cached 8 orientations are unchanged.
	e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });
	e.execute(Command::Place { x: 2, y: 1, spec: "GLa001".into() });
	e.execute(Command::SelectRect { x0: 1, y0: 1, x1: 2, y1: 1, mode: SelectMode::Replace });
	e.execute(Command::Copy);
	e.execute(Command::Paste);
	let dims = |s: &Template| (s.width, s.height);
	assert_eq!(dims(e.stamp.as_ref().unwrap()), (2, 1));
	assert!(e.stamp_orients.iter().all(|o| o.is_some()), "a Free stamp allows every orientation");
	assert!(matches!(e.execute(Command::Orient { rot: 1, mirror: false }), Outcome::Redraw));
	assert_eq!(dims(e.stamp.as_ref().unwrap()), (1, 2), "oriented footprint swaps");
	assert_eq!(dims(e.stamp_base.as_ref().unwrap()), (2, 1), "base unchanged");
	assert_eq!(e.stamp_xform, map_core::Transform { rot: 1, mirror: false });
}

/// The eyedropper refuses out-of-range and truly empty cells; a valid pick
/// re-targets the Tile Explorer, falling back to the All filter when the
/// current filter would hide the picked tile.
#[test]
fn pick_validates_cells_and_reveals_through_the_filter() {
	let mut e = editor();
	assert!(matches!(e.execute(Command::Pick { x: 60, y: 60 }), Outcome::Failed(_)), "out of range");
	// Empty a cell completely, then pick it: loud refusal.
	e.execute(Command::SelectRect { x0: 2, y0: 2, x1: 2, y1: 2, mode: SelectMode::Replace });
	e.execute(Command::DeleteAll);
	assert!(matches!(e.execute(Command::Pick { x: 2, y: 2 }), Outcome::Failed(_)), "empty cell");
	// A land tile hidden by the water filter: pick falls back to All.
	e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });
	e.execute(Command::PickerFilter { name: "water".into() });
	assert!(matches!(e.execute(Command::Pick { x: 1, y: 1 }), Outcome::Redraw));
	assert_eq!(e.active_tile(), Some("GLa000"), "the stack top becomes the brush");
	assert!(matches!(e.picker.filter, picker::Filter::All), "the hiding filter fell back to All");
	assert_eq!(e.tool, Tool::Pencil, "the eyedropper hands back to the pencil");
	// A brush id that exists in no pack reveals nothing (and must not panic).
	e.active_tile = Some("BOGUS".into());
	e.reveal_active_tile_in_explorer();
}

/// `shore` validates its region against the map and reports the seams the
/// tileset cannot close; the loop-fix rung drives the loop-walk + Mangle
/// repair.
#[test]
fn shore_validates_regions_and_reports_unclosable_seams() {
	let mut e = editor();
	let out = e.execute(Command::Shore { region: Some((0, 0, 20, 20)), mode: ShoreMode::Sweep });
	assert!(matches!(out, Outcome::Failed(msg) if msg.contains("exceeds")), "region beyond the 8x8 map");
	// Paint a 1-cell land/water checkerboard, then sweep: the auto-shore
	// pass tiles the whole boundary and reports how many cells it changed.
	e.execute(Command::AutoShore { mode: "off".into() });
	e.execute(Command::ToolSelect { name: "paint-land".into() });
	for y in 0..8u16 {
		for x in 0..8u16 {
			if (x + y) % 2 == 0 {
				e.execute(Command::PaintMask { x, y });
			}
		}
	}
	assert!(matches!(e.execute(Command::Shore { region: None, mode: ShoreMode::Sweep }), Outcome::Redraw));
	assert!(
		e.console.log().last().unwrap().starts_with("auto-shore:"),
		"the sweep reports its result: {}",
		e.console.log().last().unwrap(),
	);
	// The loop-fix rung (loop-walk placement + Mangle) runs to completion.
	let mut e = editor();
	assert!(matches!(e.execute(Command::Shore { region: None, mode: ShoreMode::LoopFix }), Outcome::Redraw));
}

/// The Edit ▸ Undo History submenu mirrors the undo stack (newest first,
/// each an `undo-to N` command), and `UndoTo` jumps back multiple steps.
#[test]
fn undo_history_submenu_reflects_the_stack() {
	let mut e = editor();
	e.execute(Command::Place { x: 0, y: 0, spec: "GLa000".into() });
	e.execute(Command::Place { x: 1, y: 0, spec: "GLa000".into() });
	e.sync_undo_history();
	let edit = e.menu_tree.menus.iter().find(|m| m.title == "Edit").unwrap();
	let hist = edit
		.items
		.iter()
		.find_map(|it| match it {
			menu::Item::Sub { label, items } if label == "Undo History" => Some(items),
			_ => None,
		})
		.expect("Undo History submenu");
	assert_eq!(hist.len(), 2, "one entry per undo patch");
	assert!(matches!(&hist[0], menu::Item::Action { command, .. } if command == "undo-to 1"));
	assert!(matches!(&hist[1], menu::Item::Action { command, .. } if command == "undo-to 2"));
	// Jumping back two steps reverts both edits.
	e.execute(Command::UndoTo { steps: 2 });
	assert_eq!(e.project.cell(0, 0).unwrap()[map_core::LAYER_GROUND], None);
	assert_eq!(e.project.cell(1, 0).unwrap()[map_core::LAYER_GROUND], None);
}

/// The Show Shore Bugs / Show Problems overlays: turning a toggle on caches
/// the offending cells (a stray shore tile on open water is both), and
/// turning it off clears the cache.
#[test]
fn problem_overlays_toggle_and_cache() {
	let mut e = editor();
	// A lone shore tile on all-water: a floating coast (shore bug) whose land
	// edge faces the sea (a match violation).
	e.execute(Command::Place { x: 4, y: 4, spec: "GSa000:!E".into() });
	e.execute(Command::ShoreBugs { on: Some(true) });
	e.execute(Command::MatchProblems { on: Some(true) });
	e.refresh_problem_overlays();
	assert!(e.shore_bug_cells.contains(&(4, 4)), "the stray shore tile is a shore bug");
	assert!(e.match_problem_cells.contains(&(4, 4)), "and a match violation");
	// Toggling off clears the cached overlay cells.
	e.execute(Command::ShoreBugs { on: Some(false) });
	e.execute(Command::MatchProblems { on: Some(false) });
	e.refresh_problem_overlays();
	assert!(e.shore_bug_cells.is_empty() && e.match_problem_cells.is_empty(), "off clears the overlays");
}

/// With an active selection and no explicit region, `shore` confines itself
/// to the selection's bounds: a land block inside it grows a coast, an
/// identical block outside is left as open water.
#[test]
fn shore_confines_to_the_active_selection() {
	let mut e = editor();
	e.execute(Command::AutoShore { mode: "off".into() });
	// Two 2×2 land blocks: one in the top-left, one in the bottom-right.
	for (bx, by) in [(1u16, 1u16), (5, 5)] {
		for dy in 0..2 {
			for dx in 0..2 {
				e.execute(Command::Place { x: bx + dx, y: by + dy, spec: "GLa000".into() });
			}
		}
	}
	// Select only the top-left corner (bounds (0,0)..(3,3)).
	e.selection.apply_rect(0, 0, 3, 3, map_core::SelectMode::Add);
	assert!(matches!(e.execute(Command::Shore { region: None, mode: ShoreMode::Sweep }), Outcome::Redraw));
	let has_ground = |x: u16, y: u16| e.project.cell(x, y).unwrap()[map_core::LAYER_GROUND].is_some();
	assert!(has_ground(1, 0), "water above the selected block became shore");
	assert!(!has_ground(5, 4), "the block outside the selection kept its open water");
}

/// The synchronous `generate` command reports seed + counts to the console
/// and fails cleanly on a document with no LAND variant group.
#[test]
fn generate_command_runs_synchronously_and_validates_the_doc() {
	let res = resources();
	let project = Project::new(32, 32, &["GREEN".to_string()], &res.join("assets/tilepacks"), 1).unwrap();
	let mut e = EditorState::new(project, (800, 600), None, res);
	let params = map_core::GenParams::defaults(map_core::Generator::Islands);
	assert!(matches!(e.execute(Command::Generate { params, explicit_seed: Some(7) }), Outcome::Redraw));
	assert!(e.console.log().last().unwrap().contains("seed 7"), "the report names the seed");
	assert!(e.dirty(), "generation edits the document");

	let mut e = editor();
	e.add_doc(Project::from_wrl(&tiny_wrl(40), "FLAT"), None, None);
	let params = map_core::GenParams::defaults(map_core::Generator::Islands);
	assert!(matches!(e.execute(Command::Generate { params, explicit_seed: Some(7) }), Outcome::Failed(_)));
}

/// `pan-to` centers the view on a tile (the named cell lands under the screen
/// midpoint); `zoom-to` sets an absolute zoom level, clamped to the range.
#[test]
fn pan_to_centers_and_zoom_to_sets_absolute_zoom() {
	let mut e = editor(); // 8×8, an 800×600 screen
	assert!(matches!(e.execute(Command::PanTo { x: 2.0, y: 3.0 }), Outcome::Redraw));
	assert_eq!(e.cell_at(400.0, 300.0), Some((2, 3)), "the named tile sits under the screen center");
	// zoom-to lands on the requested level when it's in range...
	e.execute(Command::ZoomTo { level: 4.0 });
	assert!((e.view.zoom - 4.0).abs() < 1e-3, "zoom set to 4.0, got {}", e.view.zoom);
	// ...and a level past the ceiling clamps to ZOOM_MAX.
	e.execute(Command::ZoomTo { level: 999.0 });
	assert_eq!(e.view.zoom, ZOOM_MAX, "over-range zoom clamps to the ceiling");
}

/// `select-cell` adds or subtracts one cell; `select-move` translates the mask
/// (a zero move or an empty mask is a quiet no-op, cells shoved off are dropped).
#[test]
fn select_cell_adds_subtracts_and_move_translates() {
	let mut e = editor(); // 8×8
	e.execute(Command::SelectCell { x: 2, y: 2, mode: SelectMode::Add });
	e.execute(Command::SelectCell { x: 3, y: 3, mode: SelectMode::Add });
	assert_eq!(e.selection.count(), 2, "two cells added");
	e.execute(Command::SelectCell { x: 2, y: 2, mode: SelectMode::Subtract });
	assert_eq!(e.selection.count(), 1);
	assert!(e.selection.contains(3, 3) && !e.selection.contains(2, 2), "subtract lifted (2,2)");
	// A zero move changes nothing → Ok; a real move shifts the mask → Redraw.
	assert!(matches!(e.execute(Command::SelectMove { dx: 0, dy: 0 }), Outcome::Ok), "no-op move");
	assert!(matches!(e.execute(Command::SelectMove { dx: 2, dy: 0 }), Outcome::Redraw));
	assert!(e.selection.contains(5, 3) && !e.selection.contains(3, 3), "the cell moved right by two");
	// Pushing the whole selection off the map drops it (still a change → Redraw);
	// a following move then has nothing to translate → Ok.
	assert!(matches!(e.execute(Command::SelectMove { dx: 99, dy: 0 }), Outcome::Redraw));
	assert_eq!(e.selection.count(), 0, "shoved off the map");
	assert!(matches!(e.execute(Command::SelectMove { dx: 1, dy: 1 }), Outcome::Ok), "empty mask no-ops");
}

/// `cut` captures the selection to the clipboard and lifts only its ground
/// (keeping the water base, like the eraser); an empty selection is refused.
#[test]
fn cut_captures_to_the_clipboard_and_lifts_only_ground() {
	let mut e = editor();
	// Nothing selected: cut refuses and the clipboard stays empty.
	assert!(matches!(e.execute(Command::Cut), Outcome::Failed(_)), "cut needs a selection");
	assert!(e.clipboard.is_none());
	// A cell with the water base + ground on top, selected.
	e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });
	let has = |e: &EditorState, layer: usize| e.project.cell(1, 1).unwrap()[layer].is_some();
	assert!(has(&e, LAYER_WATER) && has(&e, LAYER_GROUND), "starts with water + ground");
	e.execute(Command::SelectRect { x0: 1, y0: 1, x1: 1, y1: 1, mode: SelectMode::Replace });
	assert!(matches!(e.execute(Command::Cut), Outcome::Redraw));
	assert!(e.clipboard.is_some(), "cut fills the clipboard");
	assert!(!has(&e, LAYER_GROUND) && has(&e, LAYER_WATER), "ground lifted, water base kept");
}

/// A ghost stamp: `stamp` refuses when nothing is armed, places the armed
/// clipboard at a cell and stays armed for repeats, and `stamp-cancel` disarms.
#[test]
fn stamp_places_the_armed_clipboard_and_cancel_disarms() {
	let mut e = editor();
	assert!(matches!(e.execute(Command::Stamp { x: 3, y: 3 }), Outcome::Failed(_)), "nothing armed");
	// Copy a 2×1 ground strip, then paste to arm the stamp.
	e.execute(Command::Place { x: 0, y: 0, spec: "GLa000".into() });
	e.execute(Command::Place { x: 1, y: 0, spec: "GLa000".into() });
	e.execute(Command::SelectRect { x0: 0, y0: 0, x1: 1, y1: 0, mode: SelectMode::Replace });
	e.execute(Command::Copy);
	e.execute(Command::Paste);
	assert!(e.stamp.is_some(), "paste arms the stamp");
	// Placing writes the strip and leaves the stamp armed (forests!). The
	// footprint is centred on the cell, so a 2-wide strip starts one left of it.
	let ground = |e: &EditorState, x, y| e.project.cell(x, y).unwrap()[LAYER_GROUND].is_some();
	assert!(matches!(e.execute(Command::Stamp { x: 4, y: 4 }), Outcome::Redraw));
	assert!(ground(&e, 3, 4) && ground(&e, 4, 4), "the 2x1 strip landed centred on the cell");
	assert!(!ground(&e, 5, 4), "and not from the cell rightwards");
	assert!(e.stamp.is_some(), "still armed for a repeat");
	// At the left edge the centred footprint would run off, so it saturates.
	assert!(matches!(e.execute(Command::Stamp { x: 0, y: 2 }), Outcome::Redraw));
	assert!(ground(&e, 0, 2) && ground(&e, 1, 2), "the strip clamps to the edge");
	// Cancel disarms; a following stamp refuses again.
	assert!(matches!(e.execute(Command::StampCancel), Outcome::Redraw));
	assert!(e.stamp.is_none());
	assert!(matches!(e.execute(Command::Stamp { x: 6, y: 6 }), Outcome::Failed(_)));
}

/// A plain `color` select focuses the slot, clears any multi/range selection,
/// and logs the slot's hex (from the project palette).
#[test]
fn color_select_focuses_and_logs_the_slot() {
	let mut e = editor();
	// Seed a multi-selection, then a plain select clears it.
	e.execute(Command::ColorToggle { index: 70 });
	e.execute(Command::ColorToggle { index: 80 });
	assert_eq!(e.palettes.multi.len(), 2);
	assert!(matches!(e.execute(Command::Color { index: 100 }), Outcome::Redraw));
	assert_eq!(e.active_color, Some(100));
	assert!(e.palettes.multi.is_empty() && e.palettes.sel_end.is_none(), "plain select clears multi + range");
	// The log carries the slot's hex from the project palette.
	let at = 100 * 3;
	let hex =
		format!("#{:02x}{:02x}{:02x}", e.project.palette[at], e.project.palette[at + 1], e.project.palette[at + 2]);
	assert!(e.console.log().last().unwrap().contains(&hex), "logged {}", e.console.log().last().unwrap());
}

/// `palette-save` writes the working palette to a path, `palette-load` reads a
/// dynamic-slot override back (a missing file fails), and `palette-import`
/// copies a validated file into the user dir and selects it.
#[test]
fn palette_save_load_and_import_round_trip_through_disk() {
	let (mut e, root) = temp_editor("palette-io-cmd-test");
	let (mut e2, root2) = temp_editor("palette-io-cmd-test-2");
	let at = 100 * 3; // a dynamic (editable) slot
	e.execute(Command::SetColor { slot: 100, rgb: [0x11, 0x22, 0x33] });
	let path = root.join("mypal.json");
	assert!(matches!(e.execute(Command::PaletteSave { path: path.clone() }), Outcome::Redraw));
	assert!(path.is_file(), "palette written to the explicit path");
	// A fresh editor loads it back: the dynamic slot is restored.
	assert_ne!(&e2.project.palette[at..at + 3], &[0x11, 0x22, 0x33], "slot 100 starts elsewhere");
	assert!(matches!(e2.execute(Command::PaletteLoad { path: path.clone() }), Outcome::Redraw));
	assert_eq!(&e2.project.palette[at..at + 3], &[0x11, 0x22, 0x33], "the override round-tripped");
	// A missing file is a clean failure, not a panic.
	assert!(matches!(e2.execute(Command::PaletteLoad { path: root2.join("absent.json") }), Outcome::Failed(_)));
	// Import copies the file into the user palettes dir and selects it.
	assert!(matches!(e.execute(Command::PaletteImport { path }), Outcome::Redraw));
	let dest = e.user_palettes_dir().join("mypal.json");
	assert!(dest.is_file(), "imported into user/palettes");
	assert_eq!(e.selected_palette(), Some(&dest), "the import is selected");
	let _ = std::fs::remove_dir_all(&root);
	let _ = std::fs::remove_dir_all(&root2);
}

/// `palette-tab` flips the panel between the grid and the saved list (scanning
/// on the way to saved); the three palette-manager modals open their dialogs.
#[test]
fn palette_tab_switches_view_and_manager_modals_open() {
	let mut e = editor();
	assert!(!e.palettes.show_saved, "starts on the grid");
	assert!(matches!(e.execute(Command::PaletteTab { saved: true }), Outcome::Redraw));
	assert!(e.palettes.show_saved, "switched to the saved list");
	assert!(!e.palettes.files.is_empty(), "the saved tab scanned the installed palette files");
	e.execute(Command::PaletteTab { saved: false });
	assert!(!e.palettes.show_saved);
	assert!(matches!(e.execute(Command::PaletteSaveModal), Outcome::OpenDialog(DialogRequest::PaletteSave)));
	assert!(matches!(e.execute(Command::PaletteRenameModal), Outcome::OpenDialog(DialogRequest::PaletteRename)));
	assert!(matches!(e.execute(Command::PaletteDeleteModal), Outcome::OpenDialog(DialogRequest::PaletteDelete)));
}

/// Undo / redo report `Ok` on an empty stack and `Redraw` for an ordinary
/// per-cell edit (the non-structural path); the edit round-trips.
#[test]
fn undo_redo_report_ok_when_empty_and_redraw_for_a_cell_edit() {
	let mut e = editor();
	// Nothing to undo/redo yet.
	assert!(matches!(e.execute(Command::Undo), Outcome::Ok), "empty undo stack");
	assert!(matches!(e.execute(Command::Redo), Outcome::Ok), "empty redo stack");
	e.execute(Command::Place { x: 2, y: 2, spec: "GLa000".into() });
	let painted = e.project.hash();
	assert!(matches!(e.execute(Command::Undo), Outcome::Redraw), "a cell edit undoes as a plain redraw");
	assert!(e.project.cell(2, 2).unwrap()[LAYER_GROUND].is_none(), "the placement was reverted");
	assert!(matches!(e.execute(Command::Redo), Outcome::Redraw));
	assert_eq!(e.project.hash(), painted, "redo restored the edit");
}

/// `resize` grows/crops the canvas (the old map shifts by the offset, new
/// territory fills with water, the doc is replaced) and refuses a zero side.
#[test]
fn resize_grows_crops_and_validates_dimensions() {
	let mut e = editor(); // 8×8
	e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() });
	// Grow to 12×10 at offset (2,2): the old map shifts, new territory is water.
	let out = e.execute(Command::Resize { width: 12, height: 10, off_x: 2, off_y: 2 });
	assert!(matches!(out, Outcome::DocReplaced), "a dimension change replaces the document");
	assert_eq!(e.map_size(), (12, 10));
	assert!(e.project.cell(3, 3).unwrap()[LAYER_GROUND].is_some(), "the placed tile shifted to (3,3)");
	assert!(e.project.cell(0, 0).unwrap()[LAYER_WATER].is_some(), "new territory filled with water");
	// A zero dimension is refused.
	assert!(matches!(e.execute(Command::Resize { width: 0, height: 4, off_x: 0, off_y: 0 }), Outcome::Failed(_)));
}

/// `save` + `save-copy` guard their paths: a missing path and a non-`.json`
/// extension both fail loudly, and a `.json` copy writes without adopting the
/// path or clearing the dirty flag.
#[test]
fn save_and_save_copy_guard_paths_and_extensions() {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp/save-guard-test");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let mut e = editor();
	e.execute(Command::Place { x: 1, y: 1, spec: "GLa000".into() }); // dirty it
	// A never-saved doc with no explicit path has nowhere to save.
	assert!(matches!(e.execute(Command::Save { path: None }), Outcome::Failed(_)), "no path to save to");
	// The wrong extension is refused (projects are .json; export writes WRL).
	let wrl = dir.join("m.wrl");
	assert!(matches!(e.execute(Command::Save { path: Some(wrl.clone()) }), Outcome::Failed(_)));
	assert!(matches!(e.execute(Command::SaveCopy { path: wrl }), Outcome::Failed(_)), "a copy is .json too");
	// A real copy writes the file but leaves the path unset and the doc dirty.
	let copy = dir.join("copy.json");
	assert!(matches!(e.execute(Command::SaveCopy { path: copy.clone() }), Outcome::Ok));
	assert!(copy.is_file(), "the copy was written");
	assert_eq!(e.path, None, "save-copy does not adopt the path");
	assert!(e.dirty(), "save-copy leaves the doc dirty");
	let _ = std::fs::remove_dir_all(&dir);
}

/// The remaining modal-opener commands return their dialog requests (the shell
/// routes them; headless runs drop them) and their `open_*` builders run clean.
#[test]
fn modal_openers_return_their_dialog_requests() {
	let mut e = editor();
	assert!(matches!(e.execute(Command::ResizeModal), Outcome::OpenDialog(DialogRequest::Resize)));
	assert!(matches!(e.execute(Command::GenerateModal), Outcome::OpenDialog(DialogRequest::Generate)));
	assert!(matches!(
		e.execute(Command::MetadataModal),
		Outcome::OpenDialog(DialogRequest::Metadata { save_after: false })
	));
	assert!(matches!(e.execute(Command::NewMapModal), Outcome::OpenDialog(DialogRequest::NewMap { shape: None })));
}

/// The unit-preview commands that need no sprite library: team select (unknown
/// refused), the visibility toggle, an empty erase + clear, and `unit off`
/// handing the tool back to the pencil.
#[test]
fn unit_team_visibility_and_clear_manage_previews_without_the_library() {
	let mut e = editor();
	assert!(matches!(e.execute(Command::UnitTeam { team: "blue".into() }), Outcome::Redraw));
	assert_eq!(e.unit_team, 2, "blue is team 2");
	assert!(matches!(e.execute(Command::UnitTeam { team: "octarine".into() }), Outcome::Failed(_)), "unknown team");
	// Visibility toggles (explicit off, then bare toggle back on).
	assert!(e.show_units);
	e.execute(Command::UnitsVisible { on: Some(false) });
	assert!(!e.show_units);
	e.execute(Command::UnitsVisible { on: None });
	assert!(e.show_units, "None toggles back on");
	// Nothing placed: erase is a quiet no-op, clear reports zero.
	assert!(matches!(e.execute(Command::UnitErase { x: 0, y: 0 }), Outcome::Ok), "no preview to erase");
	assert!(matches!(e.execute(Command::UnitClear), Outcome::Redraw));
	assert!(e.console.log().last().unwrap().contains("(0)"), "cleared zero previews");
	// `unit off` clears the active unit and returns the tool to the pencil.
	e.tool = Tool::Unit;
	assert!(matches!(e.execute(Command::UnitSelect { tag: None }), Outcome::Redraw));
	assert_eq!(e.tool, Tool::Pencil, "unit off hands back to the pencil");
}

/// The Clone tool is a clone stamp: a click on an object takes it as the
/// source — type, team **and** every per-unit property, which is what
/// separates it from the eyedropper — and a click on a bare cell stamps a
/// copy there. Nothing armed and nothing under the click is an error, not a
/// Changing MaxPath drops the unit library for a reload against the new
/// folder. An armed unit is an index into the roster being dropped, so a
/// click afterwards used to index `None`/a stale roster and panic.
#[test]
fn changing_max_path_disarms_the_unit_tool_instead_of_panicking() {
	use crate::units::{UnitEntry, UnitLibrary};
	let mut e = editor(); // 8x8
	e.units = Some(UnitLibrary::new(vec![UnitEntry {
		tag: "TANK".into(),
		frames: vec![],
		shadow: vec![],
		data: Default::default(),
		footprint: 1,
	}]));
	e.units_loaded = true;
	e.active_unit = Some(0);
	e.tool = Tool::Unit;

	e.apply_preferences("/nonexistent/max".into(), String::new(), String::new(), true);
	assert_eq!(e.active_unit, None, "the armed roster index does not survive the roster");

	// The tool is still selected, so a click still routes here - it must fail
	// with a message, not panic.
	assert!(matches!(e.execute(Command::Paint { x: 2, y: 2 }), Outcome::Failed(_)));
}

/// A stale index into a library that reloaded shorter is the same hazard
/// from the other direction: reported, never indexed.
#[test]
fn placing_a_unit_index_past_the_roster_fails_cleanly() {
	use crate::units::{UnitEntry, UnitLibrary};
	let mut e = editor();
	e.units = Some(UnitLibrary::new(vec![UnitEntry {
		tag: "TANK".into(),
		frames: vec![],
		shadow: vec![],
		data: Default::default(),
		footprint: 1,
	}]));
	e.units_loaded = true;
	e.active_unit = Some(7); // the roster holds one entry
	e.tool = Tool::Unit;

	match e.execute(Command::Paint { x: 2, y: 2 }) {
		Outcome::Failed(msg) => assert!(msg.contains("no unit #7"), "{msg}"),
		_ => panic!("expected a clean failure"),
	}
}

/// silent no-op.
#[test]
fn the_clone_tool_stamps_a_full_copy() {
	use crate::units::{UnitEntry, UnitLibrary};
	let entry = |tag: &str, footprint: u32| UnitEntry {
		tag: tag.into(),
		frames: vec![],
		shadow: vec![],
		data: Default::default(),
		footprint,
	};
	let mut e = editor(); // 8×8
	e.units = Some(UnitLibrary::new(vec![entry("TANK", 1)]));
	e.active_unit = Some(0);
	e.unit_team = 1;
	e.tool = Tool::Unit;
	e.execute(Command::Paint { x: 2, y: 2 });

	// Give the placed tank a life of its own, so a copy that only carried
	// the type and owner would be visibly wrong.
	e.selected_object = e.object_at(2, 2);
	e.execute(Command::ObjectEdit { field: "name".into(), value: "Ripper".into() });
	e.execute(Command::ObjectEdit { field: "hits".into(), value: "7".into() });
	let source = e.project.objects[e.object_at(2, 2).unwrap()].clone();

	// Nothing armed yet: a bare cell says so rather than doing nothing.
	e.tool = Tool::ObjClone;
	assert!(matches!(e.execute(Command::ObjectClone { x: 5, y: 5 }), Outcome::Failed(_)));

	// Click the tank to source it, then a bare cell to stamp it.
	e.execute(Command::ObjectClone { x: 2, y: 2 });
	assert_eq!(e.clone_source.as_ref().map(|o| o.unit_type), Some(source.unit_type), "the tank is the source");
	e.execute(Command::ObjectClone { x: 5, y: 5 });

	let copy = e.project.objects[e.object_at(5, 5).expect("a copy landed")].clone();
	assert_eq!(copy, map_core::MapObject { x: 5, y: 5, ..source.clone() }, "same everything but the cell");
	assert_eq!(e.project.objects.len(), 2, "and the original is untouched");

	// One undo takes the stamp back; the source stays armed for the next one.
	e.execute(Command::Undo);
	assert_eq!(e.project.objects.len(), 1, "the stamp was one undo unit");
	assert!(e.clone_source.is_some(), "the source is not part of the document");
}

/// A building lays its own foundation, the way the game's deploy does: the
/// slab its `REQUIRES_SLAB` flag calls for, on the same cell, for the same
/// team, *under* the structure — and both come back on one undo. A unit that
/// needs no slab lays none, and a restamp replaces only its own layer.
#[test]
fn a_building_lays_its_slab() {
	use crate::units::{UnitEntry, UnitLibrary};
	let entry = |tag: &str, footprint: u32| UnitEntry {
		tag: tag.into(),
		frames: vec![],
		shadow: vec![],
		data: Default::default(),
		footprint,
	};
	let id = |tag: &str| max_assets::save::unit_type_id(tag).expect("a real type");

	let mut e = editor(); // 8×8
	e.units =
		Some(UnitLibrary::new(vec![entry("COMMTWR", 2), entry("GUNTURRT", 1), entry("TANK", 1), entry("LRGSLAB", 2)]));
	e.unit_team = 2;
	e.tool = Tool::Unit;

	// A 2×2 building lays the large slab; the slab comes first, so the
	// structure is over it.
	e.active_unit = Some(0);
	e.execute(Command::Paint { x: 1, y: 1 });
	let laid: Vec<(u16, u16, u16, u8)> = e.project.objects.iter().map(|o| (o.unit_type, o.x, o.y, o.team)).collect();
	assert_eq!(laid, vec![(id("LRGSLAB"), 1, 1, 2), (id("COMMTWR"), 1, 1, 2)], "slab under the tower, same owner");
	let picked = |e: &EditorState, x, y| e.object_at(x, y).map(|i| e.project.objects[i].unit_type);
	assert_eq!(picked(&e, 1, 1), Some(id("COMMTWR")), "and the tower is what a click there selects");

	// A 1×1 fixture lays the small one.
	e.active_unit = Some(1);
	e.execute(Command::Paint { x: 4, y: 4 });
	assert!(e.project.objects.iter().any(|o| o.unit_type == id("SMLSLAB") && (o.x, o.y) == (4, 4)));

	// A mobile unit lays nothing.
	e.active_unit = Some(2);
	e.execute(Command::Paint { x: 6, y: 6 });
	assert_eq!(e.project.objects.iter().filter(|o| (o.x, o.y) == (6, 6)).count(), 1, "just the tank");

	// Restamping the tower's cell replaces the tower, not the slab under it.
	e.active_unit = Some(0);
	e.execute(Command::Paint { x: 1, y: 1 });
	fn at(e: &EditorState, x: u16, y: u16) -> usize {
		e.project.objects.iter().filter(|o| (o.x, o.y) == (x, y)).count()
	}
	assert_eq!(at(&e, 1, 1), 2, "still one slab and one tower");

	// And a hand-placed slab on that cell replaces the slab, not the tower.
	e.active_unit = Some(3);
	e.execute(Command::Paint { x: 1, y: 1 });
	assert_eq!(at(&e, 1, 1), 2, "the layers stay one deep each");
	assert_eq!(picked(&e, 1, 1), Some(id("COMMTWR")), "the tower is still what a click selects");
}

/// A place-tool drag lays one object per cell and commits as a single undo
/// unit (the shell opens the stroke; the tool stays armed for the whole drag).
/// Placing connector-host buildings mid-drag must NOT nest a fresh stroke —
/// that would split the drag — yet auto-connect still runs inside it.
#[test]
fn unit_place_drag_is_one_undo_unit() {
	use crate::units::{UnitEntry, UnitLibrary};
	let entry = |tag: &str, footprint: u32| UnitEntry {
		tag: tag.into(),
		frames: vec![],
		shadow: vec![],
		data: Default::default(),
		footprint,
	};
	// Mobile 1×1 units: three placed across one drag → one undo unit.
	let mut e = editor(); // 8×8
	e.units = Some(UnitLibrary::new(vec![entry("TANK", 1)]));
	e.active_unit = Some(0);
	e.tool = Tool::Unit;
	e.project.begin_stroke();
	for x in 1..=3 {
		e.execute(Command::Paint { x, y: 1 });
	}
	e.project.end_stroke();
	assert_eq!(e.project.objects.len(), 3, "the drag placed three tanks");
	e.execute(Command::Undo);
	assert_eq!(e.project.objects.len(), 0, "one undo removes the whole drag");

	// Connector-host buildings: the mid-drag `in_stroke` guard keeps the drag
	// whole (place + auto-connect both land inside the one stroke).
	let mut e = editor();
	e.units = Some(UnitLibrary::new(vec![entry("COMMTWR", 2)]));
	e.active_unit = Some(0);
	e.tool = Tool::Unit;
	e.project.begin_stroke();
	e.execute(Command::Paint { x: 0, y: 0 });
	assert!(e.project.in_stroke(), "placing a building mid-drag keeps the drag stroke open");
	e.execute(Command::Paint { x: 2, y: 0 });
	assert!(e.project.in_stroke(), "still open after the second building");
	e.project.end_stroke();
	// Two buildings, each on the slab it laid (see `a_building_lays_its_slab`).
	let commtwr = max_assets::save::unit_type_id("COMMTWR").expect("a real type");
	let towers = e.project.objects.iter().filter(|o| o.unit_type == commtwr).count();
	assert_eq!(towers, 2, "both buildings placed");
	assert_eq!(e.project.objects.len(), 4, "each on its own slab");
	assert!(
		e.project.objects.iter().any(|o| o.props.connectors != 0),
		"adjacent same-team buildings auto-connect during the drag",
	);
	e.execute(Command::Undo);
	assert_eq!(e.project.objects.len(), 0, "one undo removes the whole building drag");
}

/// A place-tool drag never overpaints: once the stroke has laid its first
/// object, a continuation cell whose footprint would overlap an existing
/// object's is skipped — a 2×2 dragged across a row tiles edge to edge, and
/// a 1×1 dragged back over its own trail adds nothing. A deliberate click
/// outside a drag keeps the restamp-on-click replace semantics.
#[test]
fn unit_place_drag_skips_overlapping_cells() {
	use crate::units::{UnitEntry, UnitLibrary};
	let entry = |tag: &str, footprint: u32| UnitEntry {
		tag: tag.into(),
		frames: vec![],
		shadow: vec![],
		data: Default::default(),
		footprint,
	};

	// A 2×2 building dragged across x = 0..=3: the press lands at 0, x=1
	// overlaps it (skipped), x=2 tiles cleanly, x=3 overlaps that (skipped).
	let mut e = editor(); // 8×8
	e.units = Some(UnitLibrary::new(vec![entry("COMMTWR", 2)]));
	e.active_unit = Some(0);
	e.tool = Tool::Unit;
	e.project.begin_stroke();
	for x in 0..=3 {
		e.execute(Command::Paint { x, y: 0 });
	}
	e.project.end_stroke();
	let commtwr = max_assets::save::unit_type_id("COMMTWR").expect("a real type");
	let placed: Vec<u16> = e.project.objects.iter().filter(|o| o.unit_type == commtwr).map(|o| o.x).collect();
	assert_eq!(placed, vec![0, 2], "the drag tiles 2x2s edge to edge instead of overlapping them");

	// A 1×1 dragged forward and back over its own trail adds nothing new.
	let mut e = editor();
	e.units = Some(UnitLibrary::new(vec![entry("TANK", 1)]));
	e.active_unit = Some(0);
	e.tool = Tool::Unit;
	e.project.begin_stroke();
	for x in [1, 2, 3, 2, 1] {
		e.execute(Command::Paint { x, y: 1 });
	}
	e.project.end_stroke();
	assert_eq!(e.project.objects.len(), 3, "revisited cells are skipped, not restamped");

	// Outside a drag, a click on an occupied cell still replaces (restamp).
	let mut e = editor();
	e.units = Some(UnitLibrary::new(vec![entry("TANK", 1)]));
	e.active_unit = Some(0);
	e.tool = Tool::Unit;
	e.execute(Command::Paint { x: 4, y: 4 });
	e.execute(Command::Paint { x: 4, y: 4 });
	assert_eq!(e.project.objects.len(), 1, "a deliberate click replaces the object on its cell");
}

/// The unit place / erase tools are cancellable from the map context menu
/// (they stay armed like a stamp); `tool default` disarms them to the mode's
/// own select tool.
#[test]
fn context_menu_cancels_the_unit_tools() {
	let has = |e: &EditorState, label: &str| {
		e.context_menu.as_ref().expect("open").items.iter().any(
			|it| matches!(it, menu::Item::Action { label: l, command, .. } if l == label && command == "tool default"),
		)
	};
	let mut e = editor();
	e.tool = Tool::Unit;
	e.execute(Command::ContextMenu { at: Some((400.0, 300.0)) });
	assert!(has(&e, "Cancel Placement"), "the place tool offers Cancel Placement");
	e.execute(Command::ContextMenu { at: None });
	e.tool = Tool::UnitEraser;
	e.execute(Command::ContextMenu { at: Some((400.0, 300.0)) });
	assert!(has(&e, "Cancel Erase"), "the eraser offers Cancel Erase");
}

/// Object hit-testing is footprint + z aware: any of a 2×2 building's four
/// cells selects it, an object drawn on top wins a shared cell, and the
/// `object-select` command drives `selected_object` / the highlight footprint.
#[test]
fn object_hit_test_is_footprint_and_z_aware() {
	use crate::units::{UnitEntry, UnitLibrary};
	let entry = |tag: &str, footprint: u32| UnitEntry {
		tag: tag.into(),
		frames: vec![],
		shadow: vec![],
		data: Default::default(),
		footprint,
	};
	let mut e = editor(); // 8×8
	// A minimal in-memory roster so footprints resolve (no MAX.RES needed).
	e.units = Some(UnitLibrary::new(vec![entry("COMMTWR", 2), entry("TANK", 1)]));
	let obj = |tag: &str, x, y| map_core::MapObject {
		unit_type: max_assets::save::unit_type_id(tag).unwrap(),
		x,
		y,
		team: 0,
		props: map_core::ObjectProps::default(),
	};
	e.project.place_object(obj("COMMTWR", 2, 2)); // 2×2: (2,2)(3,2)(2,3)(3,3)
	e.project.place_object(obj("TANK", 3, 3)); // 1×1 drawn on top at (3,3)

	assert_eq!(e.object_at(2, 2), Some(0), "top-left of the 2x2");
	assert_eq!(e.object_at(3, 2), Some(0), "top-right");
	assert_eq!(e.object_at(2, 3), Some(0), "bottom-left");
	assert_eq!(e.object_at(3, 3), Some(1), "the tank drawn on top wins the shared cell");
	assert_eq!(e.object_at(5, 5), None, "empty cell selects nothing");

	// The command picks from any covered cell and anchors the 2×2 footprint.
	assert!(matches!(e.execute(Command::ObjectSelect { x: 2, y: 3 }), Outcome::Redraw));
	assert_eq!(e.selected_object, Some(0), "picked the building from its bottom-left cell");
	assert_eq!(e.object_footprint_of(0), 2, "the selection is the 2x2 building");

	// Clicking empty clears; erasing an object clears a now-stale index.
	e.execute(Command::ObjectSelect { x: 6, y: 6 });
	assert_eq!(e.selected_object, None, "no object under the cursor -> cleared");
	e.execute(Command::ObjectSelect { x: 3, y: 3 });
	assert_eq!(e.selected_object, Some(1), "re-picked the tank");
	e.execute(Command::UnitErase { x: 3, y: 3 });
	assert_eq!(e.selected_object, None, "erase drops the selection");
}

/// Item 7: repeated clicks on a cell covered by several objects loop through
/// the whole stack (top-most first), then wrap.
#[test]
fn object_select_cycles_through_stacked_objects() {
	use crate::units::UnitLibrary;
	fn entry(tag: &str, footprint: u32) -> crate::units::UnitEntry {
		crate::units::UnitEntry { tag: tag.into(), frames: vec![], shadow: vec![], data: Default::default(), footprint }
	}
	let mut e = editor(); // 8×8
	e.units = Some(UnitLibrary::new(vec![entry("COMMTWR", 2), entry("TANK", 1)]));
	let obj = |tag: &str, x, y| map_core::MapObject {
		unit_type: max_assets::save::unit_type_id(tag).unwrap(),
		x,
		y,
		team: 0,
		props: map_core::ObjectProps::default(),
	};
	e.project.place_object(obj("COMMTWR", 2, 2)); // 2×2 building, index 0
	e.project.place_object(obj("TANK", 3, 3)); // 1×1 on the shared cell (3,3), index 1

	// (3,3) is covered by both; the tank draws on top, so the first click takes it.
	let stack = e.objects_at(3, 3);
	assert_eq!(stack, vec![1, 0], "top-most (tank) first, building beneath");
	e.execute(Command::ObjectSelect { x: 3, y: 3 });
	assert_eq!(e.selected_object, Some(1), "first click: top-most");
	e.execute(Command::ObjectSelect { x: 3, y: 3 });
	assert_eq!(e.selected_object, Some(0), "second click: the object beneath");
	e.execute(Command::ObjectSelect { x: 3, y: 3 });
	assert_eq!(e.selected_object, Some(1), "third click wraps back to the top");
	// A click on a cell the current selection doesn't cover just picks top-most.
	e.execute(Command::ObjectSelect { x: 2, y: 2 });
	assert_eq!(e.selected_object, Some(0), "different cell -> its top-most object");
}

/// The `auto-connect` command links adjacent same-team buildings via their
/// connector masks and reports; a second run is a quiet no-op (S4.4).
#[test]
fn auto_connect_command_links_adjacent_buildings() {
	let mut e = editor(); // 8×8
	let obj = |tag: &str, x, y, team| map_core::MapObject {
		unit_type: max_assets::save::unit_type_id(tag).unwrap(),
		x,
		y,
		team,
		props: map_core::ObjectProps::default(),
	};
	e.project.place_object(obj("COMMTWR", 0, 0, 0)); // 2×2 covers 0-1
	e.project.place_object(obj("POWERSTN", 2, 0, 0)); // 2×2 covers 2-3, east neighbour
	assert!(matches!(e.execute(Command::AutoConnect), Outcome::Redraw), "connects -> redraw");
	assert_eq!(e.project.objects[0].props.connectors, 0x0C, "COMMTWR east: ET|EB");
	assert_eq!(e.project.objects[1].props.connectors, 0xC0, "POWERSTN west: WT|WB");
	assert!(matches!(e.execute(Command::AutoConnect), Outcome::Ok), "already connected -> no-op");
}

/// `object-edit` sets each editable field on the selected object, parses
/// per-field, rejects bad input without corrupting the model, and each edit
/// is its own undo step (Unit Properties panel's command layer, S4.2).
#[test]
fn object_edit_sets_fields_parses_and_undoes() {
	let mut e = editor(); // 8×8
	let obj = |tag: &str, x, y| map_core::MapObject {
		unit_type: max_assets::save::unit_type_id(tag).unwrap(),
		x,
		y,
		team: 0,
		props: map_core::ObjectProps::default(),
	};
	e.project.place_object(obj("TANK", 3, 3));
	let edit = |field: &str, value: &str| Command::ObjectEdit { field: field.into(), value: value.into() };

	// Editing requires a selection.
	assert!(matches!(e.execute(edit("hits", "20")), Outcome::Failed(_)), "no selection -> fails");

	e.execute(Command::ObjectSelect { x: 3, y: 3 });
	assert_eq!(e.selected_object, Some(0));

	// Each field parses into its slot.
	assert!(matches!(e.execute(edit("team", "blue")), Outcome::Redraw));
	assert_eq!(e.project.objects[0].team, 2, "blue is team 2");
	assert!(matches!(e.execute(edit("name", "Rex")), Outcome::Redraw));
	assert_eq!(e.project.objects[0].props.name, "Rex");
	assert!(matches!(e.execute(edit("angle", "3")), Outcome::Redraw));
	assert_eq!(e.project.objects[0].props.angle, 3);
	assert!(matches!(e.execute(edit("hits", "42")), Outcome::Redraw));
	assert_eq!(e.project.objects[0].props.hits, 42);
	assert!(matches!(e.execute(edit("ammo", "5")), Outcome::Redraw));
	assert_eq!(e.project.objects[0].props.ammo, 5);
	// Turret heading is independent of the body angle.
	assert!(matches!(e.execute(edit("turret", "6")), Outcome::Redraw));
	assert_eq!(e.project.objects[0].props.turret_angle, 6);
	assert_eq!(e.project.objects[0].props.angle, 3, "editing the turret leaves the body angle");
	// Storage is signed (cargo / experience).
	assert!(matches!(e.execute(edit("storage", "12")), Outcome::Redraw));
	assert_eq!(e.project.objects[0].props.storage, 12);
	assert!(matches!(e.execute(edit("storage", "-3")), Outcome::Redraw));
	assert_eq!(e.project.objects[0].props.storage, -3, "storage accepts a negative value");
	// Connector adjacency bitmask (the panel toggles single bits).
	assert!(matches!(e.execute(edit("connectors", "12")), Outcome::Redraw));
	assert_eq!(e.project.objects[0].props.connectors, 0x0C, "connectors set to the raw mask");
	// Orders take a slug or a raw byte - but only one the type can hold at
	// rest (resting_orders); a runtime/garbage order is refused.
	assert!(matches!(e.execute(edit("orders", "sentry")), Outcome::Redraw));
	assert_eq!(e.project.objects[0].props.orders, 0x0C);
	assert!(matches!(e.execute(edit("orders", "200")), Outcome::Failed(_)), "not a resting order");
	assert!(matches!(e.execute(edit("orders", "idle")), Outcome::Failed(_)), "IDLE is container-only");
	assert_eq!(e.project.objects[0].props.orders, 0x0C, "a rejected order leaves the value intact");
	assert!(matches!(e.execute(edit("orders", "await")), Outcome::Redraw));
	assert_eq!(e.project.objects[0].props.orders, 0x00);

	// Bad field / value fail without touching the model.
	assert!(matches!(e.execute(edit("bogus", "1")), Outcome::Failed(_)), "unknown field");
	assert!(matches!(e.execute(edit("hits", "nope")), Outcome::Failed(_)), "non-numeric hits");
	assert!(matches!(e.execute(edit("team", "octarine")), Outcome::Failed(_)), "unknown team");
	assert_eq!(e.project.objects[0].props.hits, 42, "a rejected edit leaves the value intact");

	// A no-op edit (same value) commits nothing.
	assert!(matches!(e.execute(edit("hits", "42")), Outcome::Ok), "unchanged value is a quiet no-op");

	// Each applied edit is its own undo step.
	assert!(e.project.undo(), "undo the orders=await edit");
	assert_eq!(e.project.objects[0].props.orders, 0x0C, "back to sentry");
	assert!(e.project.undo(), "undo the sentry edit");
	assert_eq!(e.project.objects[0].props.orders, 0, "back to the default order");
}

/// `object-edit disabled N` couples the disable countdown with the order:
/// N>0 puts the unit on ORDER_DISABLE, N=0 lifts a disable back to await, and
/// the value clamps to the signed V70 disable-byte range.
#[test]
fn object_edit_disabled_couples_orders_and_countdown() {
	let mut e = editor();
	e.project.place_object(map_core::MapObject {
		unit_type: max_assets::save::unit_type_id("TANK").unwrap(),
		x: 3,
		y: 3,
		team: 0,
		props: map_core::ObjectProps::default(),
	});
	e.execute(Command::ObjectSelect { x: 3, y: 3 });
	let edit = |field: &str, value: &str| Command::ObjectEdit { field: field.into(), value: value.into() };

	assert!(matches!(e.execute(edit("disabled", "5")), Outcome::Redraw));
	assert_eq!(e.project.objects[0].props.disabled_turns, 5);
	assert_eq!(e.project.objects[0].props.orders, max_assets::save::ORDER_DISABLE, "a disable sets the order");

	assert!(matches!(e.execute(edit("disabled", "0")), Outcome::Redraw));
	assert_eq!(e.project.objects[0].props.disabled_turns, 0);
	assert_eq!(e.project.objects[0].props.orders, 0, "clearing the disable lifts the order to await");

	// Clamp to 0..=127 (the V70 disable byte is signed → ≥128 reads back as 0).
	assert!(matches!(e.execute(edit("disabled", "300")), Outcome::Redraw));
	assert_eq!(e.project.objects[0].props.disabled_turns, 127, "disable turns clamp to 127");
}

/// Stage B: with the stock unit database loaded, a fresh placement on a
/// save-less map resolves effective values from the database, and
/// `object-values` forks a per-unit override from that stock seed.
#[test]
fn object_values_seed_from_stock_db_without_a_save() {
	use max_assets::attribs::{UnitAttributes, UnitMeta, UnitStatsDb, unit_values_from_attributes};
	let mut e = editor(); // 8×8, no save
	let tank = max_assets::save::unit_type_id("TANK").unwrap();
	e.project.place_object(map_core::MapObject {
		unit_type: tank,
		x: 3,
		y: 3,
		team: 0,
		props: map_core::ObjectProps::default(),
	});
	assert!(e.object_effective_values(0).is_none(), "no save + no db -> no stats");

	// A synthetic stock database: every unit zeroed except the TANK.
	let mut base = std::array::from_fn(|_| unit_values_from_attributes(&UnitAttributes::default()));
	base[tank as usize] = unit_values_from_attributes(&UnitAttributes {
		turns_to_build: 4,
		hit_points: 24,
		armor_rating: 10,
		attack_rating: 16,
		movement_points: 6,
		attack_range: 4,
		shots_per_turn: 2,
		scan_range: 4,
		ammunition: 14,
		..Default::default()
	});
	e.unit_stats = Some(UnitStatsDb {
		base,
		clans: Default::default(),
		meta: [UnitMeta::default(); max_assets::save::UNIT_END],
		source: PathBuf::from("synthetic"),
	});

	let seed = e.object_effective_values(0).expect("db seeds stats save-lessly");
	assert_eq!((seed.hits, seed.attack, seed.speed), (24, 16, 6));
	assert_eq!(e.object_max_hits(0), Some(24), "hits cap comes from the db seed");

	// Editing forks a per-unit override off the stock seed.
	e.execute(Command::ObjectSelect { x: 3, y: 3 });
	let val = |attr: &str, value: u32| Command::ObjectValues { attr: attr.into(), value };
	assert!(matches!(e.execute(val("hits", 50)), Outcome::Redraw), "stock seed makes the edit possible");
	let forked = e.project.object_base_values(0).expect("override exists now");
	assert_eq!((forked.hits, forked.attack), (50, 16), "hits edited, the rest keeps the stock seed");

	// Applicability gating (S7.5): this TANK's meta says no cargo type, so
	// the storage editor is refused; the panel mask hides the same row.
	assert!(
		matches!(e.execute(val("storage", 5)), Outcome::Failed(_)),
		"storage is not applicable without a cargo type"
	);
	let mask = crate::unitprops::applicability_mask(&e, 0);
	let stat_pos = |attr: &str| crate::unitprops::VALUE_STATS.iter().position(|s| s.attr == attr).unwrap();
	assert!(mask[stat_pos("hits")], "hits always applicable");
	assert!(!mask[stat_pos("storage")], "storage masked out for a no-cargo type");
	assert!(!mask[stat_pos("agent-adjust")], "a tank is no agent");

	// The permissive escapes: a nonzero value in an inapplicable slot stays
	// editable (never trap data)…
	let mut props = e.project.objects[0].props.clone();
	let mut v = props.base_values.take().unwrap();
	v.storage = 7;
	props.base_values = Some(v);
	e.project.set_object_state(0, 0, props);
	assert!(e.object_stat_applicable(0, max_assets::attribs::StatKind::Storage), "nonzero value stays editable");
	// …and with no database at all nothing is restricted.
	e.unit_stats = None;
	assert!(e.object_stat_applicable(0, max_assets::attribs::StatKind::AgentAdjust));
}

/// `object-values` edits a unit's maximum stats into a per-unit override,
/// parses per attribute, preserves untouched fields, clamps over-range,
/// rejects bad input, and each edit is undoable (S4.5). Without a stats
/// block (no save seed, no override) it fails cleanly.
#[test]
fn object_values_edits_max_stats_and_undoes() {
	let mut e = editor(); // 8×8
	e.project.place_object(map_core::MapObject {
		unit_type: max_assets::save::unit_type_id("TANK").unwrap(),
		x: 3,
		y: 3,
		team: 0,
		props: map_core::ObjectProps::default(),
	});
	let val = |attr: &str, value: u32| Command::ObjectValues { attr: attr.into(), value };

	// Editing requires a selection.
	assert!(matches!(e.execute(val("hits", 50)), Outcome::Failed(_)), "no selection -> fails");
	e.execute(Command::ObjectSelect { x: 3, y: 3 });
	assert_eq!(e.selected_object, Some(0));

	// A fresh placement (no save, no override) has no stats block to edit.
	assert!(matches!(e.execute(val("hits", 50)), Outcome::Failed(_)), "no unit-values block -> fails");

	// Install a starting override so edits have a base to fork from (stands in
	// for a save seed in this save-less unit test).
	let mut props = e.project.objects[0].props.clone();
	props.base_values = Some(max_assets::save::UnitValues {
		turns: 3,
		hits: 40,
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
	});
	e.project.set_object_state(0, 0, props);

	// Each attribute lands in its slot; untouched fields are preserved.
	assert!(matches!(e.execute(val("hits", 120)), Outcome::Redraw));
	assert_eq!(e.project.object_base_values(0).unwrap().hits, 120);
	assert!(matches!(e.execute(val("attack", 30)), Outcome::Redraw));
	assert_eq!(e.project.object_base_values(0).unwrap().attack, 30);
	assert!(matches!(e.execute(val("move-and-fire", 1)), Outcome::Redraw));
	assert_eq!(e.project.object_base_values(0).unwrap().move_and_fire, 1);
	assert_eq!(e.project.object_base_values(0).unwrap().armor, 8, "an untouched stat is preserved");

	// A no-op (same value) commits nothing; an unknown attr fails; both leave
	// the model intact.
	assert!(matches!(e.execute(val("hits", 120)), Outcome::Ok), "unchanged value is a quiet no-op");
	assert!(matches!(e.execute(val("bogus", 1)), Outcome::Failed(_)), "unknown attribute");
	assert_eq!(e.project.object_base_values(0).unwrap().hits, 120, "a rejected edit leaves the value intact");

	// Over-range clamps to the field width instead of overflowing.
	assert!(matches!(e.execute(val("hits", 100_000)), Outcome::Redraw));
	assert_eq!(e.project.object_base_values(0).unwrap().hits, u16::MAX, "over-range clamps to u16::MAX");

	// Each applied edit is its own undo step.
	assert!(e.project.undo(), "undo the clamp edit");
	assert_eq!(e.project.object_base_values(0).unwrap().hits, 120);
	assert!(e.project.undo(), "undo move-and-fire");
	assert_eq!(e.project.object_base_values(0).unwrap().move_and_fire, 0);
}

/// Stage D: `resource-set` works on a plain map (no attached save) — the
/// cargo map materializes on the first paint and save synthesis carries it
/// into a real `.DTA`. The edit mechanics are covered by map-core's
/// `set_cargo` test.
#[test]
fn resource_set_works_without_an_open_save() {
	let mut e = editor(); // a plain map, no save
	assert!(e.project.cargo_at(1, 1).is_none(), "no cargo map until the first paint");
	let cmd = Command::ResourceSet { x: 1, y: 1, material: "raw".into(), amount: 15 };
	assert!(matches!(e.execute(cmd), Outcome::Redraw), "save-less paint succeeds");
	let v = e.project.cargo_at(1, 1).expect("cargo map materialized");
	assert_eq!(max_assets::save::cargo_amount(v), 15);
	assert_eq!(max_assets::save::cargo_material(v), Some(max_assets::save::CargoMaterial::Raw));
}

/// The resource brush (S5.3): `resource-brush` configures material/amount/mode,
/// arming `tool resource-brush` selects the tool, and `ResourceMode::apply`
/// combines the brush with a cell under each mode.
#[test]
fn resource_brush_config_and_apply_modes() {
	use max_assets::save::{CargoMaterial, cargo_amount, cargo_compose, cargo_material};
	let mut e = editor();
	// Config commands land in the brush state.
	e.execute(Command::ResourceBrush { field: "material".into(), value: "fuel".into() });
	assert_eq!(e.resource_material, Some(CargoMaterial::Fuel));
	e.execute(Command::ResourceBrush { field: "amount".into(), value: "40".into() });
	assert_eq!(e.resource_amount, 31, "amount clamps to 31");
	e.execute(Command::ResourceBrush { field: "mode".into(), value: "add".into() });
	assert_eq!(e.resource_mode, ResourceMode::Add);
	e.execute(Command::ResourceBrush { field: "material".into(), value: "none".into() });
	assert_eq!(e.resource_material, None, "none = erase");
	assert!(matches!(
		e.execute(Command::ResourceBrush { field: "mode".into(), value: "zzz".into() }),
		Outcome::Failed(_)
	));
	// The status-bar readout (S5.4) is gated to resource modes: outside one
	// it's None regardless of the map.
	e.execute(Command::ToolSelect { name: "pencil".into() });
	assert_eq!(e.resource_readout(1, 1), None, "not in a resource mode -> no readout");
	// Arm the tool; Stage D reads `empty` on a save-less map (no cargo yet).
	e.execute(Command::ToolSelect { name: "resource-brush".into() });
	assert_eq!(e.tool, Tool::ResourceBrush);
	assert_eq!(e.resource_readout(1, 1).as_deref(), Some("empty"), "in a resource mode -> reads empty");
	assert_eq!(e.resource_readout(9999, 1), None, "out of bounds -> None");

	// apply(): Set replaces; Add raises + sets material; Sub lowers, clearing at 0.
	let raw10 = cargo_compose(0, Some(CargoMaterial::Raw), 10);
	let set = ResourceMode::Set.apply(raw10, Some(CargoMaterial::Gold), 20);
	assert_eq!((cargo_material(set), cargo_amount(set)), (Some(CargoMaterial::Gold), 20));
	let added = ResourceMode::Add.apply(raw10, Some(CargoMaterial::Raw), 25);
	assert_eq!(cargo_amount(added), 31, "add caps at 31");
	let subbed = ResourceMode::Sub.apply(raw10, Some(CargoMaterial::Raw), 4);
	assert_eq!((cargo_material(subbed), cargo_amount(subbed)), (Some(CargoMaterial::Raw), 6));
	let cleared = ResourceMode::Sub.apply(raw10, Some(CargoMaterial::Raw), 10);
	assert_eq!(cargo_material(cleared), None, "sub to 0 clears the cell");
	let erased = ResourceMode::Set.apply(raw10, None, 30);
	assert_eq!(cargo_material(erased), None, "erase (material none) clears regardless of amount");

	// S5.5: a painted resource is surveyed by all players (usable in-game); an
	// erase / sub-to-empty adds none (the source cells here were unsurveyed).
	use max_assets::save::CARGO_SURVEY_ALL;
	assert_eq!(set & CARGO_SURVEY_ALL, CARGO_SURVEY_ALL, "Set marks all player survey bits");
	assert_eq!(added & CARGO_SURVEY_ALL, CARGO_SURVEY_ALL, "Add marks survey bits too");
	assert_eq!(erased & CARGO_SURVEY_ALL, 0, "erase adds no survey bits");
	assert_eq!(cleared & CARGO_SURVEY_ALL, 0, "sub-to-empty adds no survey bits");

	// The "exact..." toolbox key opens the amount modal (S5.4) via the shared
	// dialog-request path.
	assert!(matches!(e.execute(Command::ResourceAmountDialog), Outcome::OpenDialog(DialogRequest::ResourceAmount)));
}

/// A Select-tool pick reveals the Unit Properties panel (S4.3); a bare
/// scripted `object-select` (tool not armed) leaves the layout alone.
#[test]
fn select_tool_pick_reveals_unit_properties_panel() {
	let mut e = editor(); // 8×8
	e.project.place_object(map_core::MapObject {
		unit_type: max_assets::save::unit_type_id("TANK").unwrap(),
		x: 3,
		y: 3,
		team: 0,
		props: map_core::ObjectProps::default(),
	});
	assert!(!e.workspace.is_visible("unitprops"), "hidden by default");

	// A scripted select without the tool armed doesn't pop the panel.
	e.tool = Tool::Pencil;
	e.execute(Command::ObjectSelect { x: 3, y: 3 });
	assert_eq!(e.selected_object, Some(0));
	assert!(!e.workspace.is_visible("unitprops"), "a bare object-select leaves the layout alone");

	// The Select tool routes straight to editing: the panel appears.
	e.tool = Tool::ObjSelect;
	e.execute(Command::ObjectSelect { x: 3, y: 3 });
	assert!(e.workspace.is_visible("unitprops"), "the Select tool reveals the panel");

	// Missing (empty cell) with the tool armed clears the selection and does
	// not force the panel open again.
	e.execute(Command::Window { id: "unitprops".into(), on: Some(false) });
	e.execute(Command::ObjectSelect { x: 6, y: 6 });
	assert_eq!(e.selected_object, None);
	assert!(!e.workspace.is_visible("unitprops"), "an empty pick doesn't reopen it");
}

/// Move-tool collision: buildings (2×2) block and are blocked; footprint-1
/// objects (units, ground cover) stack freely.
#[test]
fn object_move_collision_blocks_only_buildings() {
	use crate::units::{UnitEntry, UnitLibrary};
	let entry = |tag: &str, footprint: u32| UnitEntry {
		tag: tag.into(),
		frames: vec![],
		shadow: vec![],
		data: Default::default(),
		footprint,
	};
	let mut e = editor(); // 8×8
	e.units = Some(UnitLibrary::new(vec![entry("COMMTWR", 2), entry("TANK", 1)]));
	let obj = |tag: &str, x, y| map_core::MapObject {
		unit_type: max_assets::save::unit_type_id(tag).unwrap(),
		x,
		y,
		team: 0,
		props: Default::default(),
	};
	e.project.place_object(obj("COMMTWR", 0, 0)); // idx 0: 2×2 at (0,0)
	e.project.place_object(obj("TANK", 4, 4)); // idx 1
	e.project.place_object(obj("TANK", 6, 6)); // idx 2

	assert!(!e.object_collides(1, 6, 6), "a unit stacks on another unit");
	assert!(e.object_collides(1, 1, 1), "a unit onto the building is blocked");
	assert!(e.object_collides(1, 0, 0), "...from any of the building's cells");
	assert!(e.object_collides(0, 3, 4), "the building onto a unit is blocked");
	assert!(!e.object_collides(0, 5, 0), "a clear cell doesn't collide");
}

/// The object tools are selectable by name, and Pick (eyedropper) arms the
/// type + team of the object under the cursor and switches to the Place tool.
#[test]
fn object_tools_select_and_pick() {
	use crate::units::{UnitEntry, UnitLibrary};
	let mut e = editor();
	e.units = Some(UnitLibrary::new(vec![UnitEntry {
		tag: "TANK".into(),
		frames: vec![],
		shadow: vec![],
		data: Default::default(),
		footprint: 1,
	}]));
	assert!(matches!(e.execute(Command::ToolSelect { name: "obj-select".into() }), Outcome::Redraw));
	assert_eq!(e.tool, Tool::ObjSelect);
	assert!(matches!(e.execute(Command::ToolSelect { name: "obj-pick".into() }), Outcome::Redraw));
	assert_eq!(e.tool, Tool::ObjPick);

	let tank = map_core::MapObject {
		unit_type: max_assets::save::unit_type_id("TANK").unwrap(),
		x: 2,
		y: 2,
		team: 3,
		props: Default::default(),
	};
	e.project.place_object(tank);
	assert!(matches!(e.execute(Command::ObjectPick { x: 2, y: 2 }), Outcome::Redraw));
	assert_eq!(e.active_unit, Some(0), "armed the tank sprite");
	assert_eq!(e.unit_team, 3, "picked its team");
	assert_eq!(e.tool, Tool::Unit, "switched to the Place tool");
	assert!(matches!(e.execute(Command::ObjectPick { x: 5, y: 5 }), Outcome::Ok), "picking empty is a quiet no-op");
}

/// `template-save` captures the selection into the user dir (and selects it),
/// `template-export` writes it to an explicit path, and `template-clone`
/// duplicates the selected entry; an empty selection refuses.
#[test]
fn template_save_export_and_clone_write_user_files() {
	let (mut e, root) = temp_editor("template-write-cmd-test");
	// No selection: save refuses.
	assert!(matches!(e.execute(Command::TemplateSave { name: None }), Outcome::Failed(_)), "save needs a selection");
	// Paint a 2×1 strip and select it.
	e.execute(Command::Place { x: 0, y: 0, spec: "GLa000".into() });
	e.execute(Command::Place { x: 1, y: 0, spec: "GLa000".into() });
	e.execute(Command::SelectRect { x0: 0, y0: 0, x1: 1, y1: 0, mode: SelectMode::Replace });
	assert!(matches!(e.execute(Command::TemplateSave { name: Some("ridge".into()) }), Outcome::Redraw));
	let saved = e.templates.sel.expect("save selects the new template");
	assert_eq!(e.templates.entries[saved].name, "ridge");
	assert!(e.templates.entries[saved].path.is_file(), "the template file exists under the user dir");
	// Export to an explicit path writes a loadable template.
	let out = root.join("exported.json");
	assert!(matches!(e.execute(Command::TemplateExport { path: out.clone() }), Outcome::Redraw));
	assert!(Template::load(&out).is_ok(), "the exported file is a valid template");
	// Clone the selected entry: a second file appears with a `-copy` name.
	let before = e.templates.entries.len();
	assert!(matches!(e.execute(Command::TemplateClone { name: None }), Outcome::Redraw));
	assert!(e.templates.entries.len() > before, "the clone was added to the library");
	assert!(e.templates.entries.iter().any(|t| t.name == "ridge-copy"), "clone names it <name>-copy");
	let _ = std::fs::remove_dir_all(&root);
}

/// `template-dedupe` deletes the removable duplicate file and rescans; the
/// modal reports the names it would remove (the first copy is always kept).
#[test]
fn template_dedupe_removes_the_duplicate_file_and_reports_it() {
	let (mut e, root) = temp_editor("template-dedupe-cmd-test");
	let dir = e.user_templates_dir();
	std::fs::create_dir_all(&dir).unwrap();
	let mk = |name: &str| -> TemplateEntry {
		// All-hole templates resolve in any project, so both stay visible.
		let t = Template { name: name.into(), width: 2, height: 1, uses: Vec::new(), cells: vec![String::new(); 2] };
		let path = dir.join(format!("{name}.json"));
		t.save(&path).unwrap();
		TemplateEntry { name: name.into(), path, stock: false, template: t }
	};
	e.templates.entries = vec![mk("first"), mk("second")];
	// The modal names the second copy (a removable duplicate of the first).
	let out = e.execute(Command::TemplateDedupeModal);
	assert!(
		matches!(out, Outcome::OpenDialog(DialogRequest::DedupeTemplates { names }) if names == vec!["second".to_string()]),
		"the dedupe modal lists the removable duplicate",
	);
	// Running it removes exactly that file (rescanning after).
	assert!(matches!(e.execute(Command::TemplateDedupe), Outcome::Redraw));
	assert!(dir.join("first.json").is_file(), "the first copy is kept");
	assert!(!dir.join("second.json").is_file(), "the duplicate file was removed");
	let _ = std::fs::remove_dir_all(&root);
}

/// `template-explore` in a headless run creates the user dir and logs a note
/// instead of shelling out to a file manager.
#[test]
fn template_explore_headless_creates_the_dir_and_notes_it() {
	let (mut e, root) = temp_editor("template-explore-test");
	e.headless = true;
	let dir = e.user_templates_dir();
	assert!(!dir.exists(), "the user templates dir starts absent");
	assert!(matches!(e.execute(Command::TemplateExplore), Outcome::Redraw));
	assert!(dir.is_dir(), "explore created the folder");
	assert!(e.console.log().last().unwrap().contains("headless"), "logged the headless note");
	let _ = std::fs::remove_dir_all(&root);
}

/// In-Game mode implies animation and can't coexist with a plain Animate
/// toggle; the console toggles; a tick advances the clock; `hash` logs.
#[test]
fn ingame_console_tick_and_hash_drive_state() {
	let mut e = editor();
	assert!(matches!(e.execute(Command::InGame { on: Some(true) }), Outcome::Redraw));
	assert!(e.ingame && e.animate, "In-Game turns on animation");
	// A plain Animate toggle leaves In-Game mode.
	e.execute(Command::Animate { on: None });
	assert!(!e.ingame, "Animate drops In-Game mode");
	// Console open toggles.
	let open0 = e.console.is_open();
	e.execute(Command::Console { on: None });
	assert_eq!(e.console.is_open(), !open0);
	// Tick advances the deterministic clock by dt.
	let c0 = e.clock;
	assert!(matches!(e.execute(Command::Tick { seconds: 0.5 }), Outcome::Redraw));
	assert!((e.clock - (c0 + 0.5)).abs() < 1e-6, "clock advanced by dt");
	// Hash logs the document hash and redraws.
	assert!(matches!(e.execute(Command::Hash), Outcome::Redraw));
	assert!(e.console.log().last().unwrap().contains("hash:"), "hash logged");
}

/// Panel view options apply and clamp: the minimap source, the picker filter /
/// size (named + `next`), the negative-clamped scroll positions, and the layout
/// reset.
#[test]
fn panel_view_options_apply_and_clamp() {
	let mut e = editor();
	assert!(matches!(e.execute(Command::MinimapMode { mode: "pass".into() }), Outcome::Redraw));
	assert_eq!(e.minimap_mode, minimap::Mode::Pass);
	// A named picker filter, then `next` advances to a different one.
	e.execute(Command::PickerFilter { name: "water".into() });
	assert_eq!(e.picker.filter, picker::Filter::Water);
	e.execute(Command::PickerFilter { name: "next".into() });
	assert_ne!(e.picker.filter, picker::Filter::Water, "next advanced the filter");
	// A numeric picker size sets the tile px; `next` cycles it to another size.
	e.execute(Command::PickerSize { size: "32".into() });
	assert_eq!(e.picker.tile_px, 32.0);
	e.execute(Command::PickerSize { size: "next".into() });
	assert_ne!(e.picker.tile_px, 32.0, "next cycled the size");
	// Scroll positions clamp negatives to zero. The picker's offset lives in
	// its panel widget since U2.4, so the command leaves a request instead.
	e.execute(Command::PickerScroll { to: -50.0 });
	assert_eq!(e.picker.scroll_request, Some(picker::ScrollRequest::To(0.0)));
	e.execute(Command::PaletteScroll { to: -5.0 });
	assert_eq!(e.palettes.scroll_request, Some(0.0));
	// Reset the layout back to defaults.
	assert!(matches!(e.execute(Command::ResetLayout), Outcome::Redraw));
}

/// `match-combos` gates on `--dev` and refuses before doing any work (named
/// pack or not) when it's off.
#[test]
fn match_combos_requires_dev_mode() {
	let mut e = editor();
	assert!(!e.dev_mode);
	assert!(matches!(e.execute(Command::MatchCombos { pack: None }), Outcome::Failed(_)), "needs --dev");
	assert!(matches!(e.execute(Command::MatchCombos { pack: Some("GREEN".into()) }), Outcome::Failed(_)));
}
