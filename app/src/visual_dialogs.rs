//! Visual-regression snapshots for the editor's modals / dialogs. Each test
//! opens a dialog on an [`Overlay`](crate::uikit_overlay::Overlay) and renders
//! it through `Overlay::render` (see [`crate::visual_test`]). Regenerate with
//! `UPDATE_SNAPSHOTS=1`.

use crate::uikit_menu::MenuChrome;
use crate::uikit_overlay::{MetadataValues, Overlay};
use crate::visual_test::{chrome_fixture, snapshot_overlay};

/// Open a dialog on a fresh overlay + steel chrome fixture, then snapshot the
/// rendered frame at `w`×`h`. The `open` closure receives the overlay and the
/// chrome (dialogs that compose preview/atlas textures register them on the
/// latter); dialogs that need neither simply ignore it.
fn snap(name: &str, w: u32, h: u32, open: impl FnOnce(&mut Overlay, &mut MenuChrome)) {
	snap_at(name, 1.0, w, h, open);
}

/// [`snap`] at an explicit UI scale, `w`×`h` being the **physical** target. The
/// UI Tests probe is its one caller: what it exists to show only happens at a
/// fractional scale, so it is snapped at all three shipped ones.
fn snap_at(name: &str, scale: f64, w: u32, h: u32, open: impl FnOnce(&mut Overlay, &mut MenuChrome)) {
	let (device, queue, mut chrome) = chrome_fixture();
	let mut overlay = Overlay::new(scale);
	open(&mut overlay, &mut chrome);
	snapshot_overlay(&device, &queue, &mut overlay, &mut chrome, name, w, h);
}

/// The editor's bundled tilepack asset root — the deterministic source for the
/// New Map / Import WRL preview strips and the Match editor's tile atlas.
fn assets() -> std::path::PathBuf {
	std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/assets/tilepacks")
}

/// A 128×64 solid-magenta stand-in for a composed template thumbnail (pixels the
/// steel + green UI never makes), shared by the Rename / Delete template tests.
fn template_preview() -> (Vec<u8>, u32, u32) {
	let (tw, th) = (128u32, 64u32);
	(vec![[255u8, 0, 255, 255]; (tw * th) as usize].concat(), tw, th)
}

/// Map Metadata: a fully-filled `MetadataValues` whose description overflows the
/// field (word-wrap + scrollbar) and carries a token wider than the field.
#[test]
fn dialog_metadata() {
	snap("dialog_metadata", 620, 800, |o, _c| {
		o.open_metadata(
			MetadataValues {
				name: "New Luzon".into(),
				players: Some(4),
				description: "A test map set among the islands of a drowned range. \
					Landings are contested from the first turn; hold the straits and the \
					supersupersupercalifragilisticexpialidocious-long-token wraps mid-word. \
					Expect naval skirmishes, tight beachheads, and late-game air superiority \
					fights over the central atoll."
					.into(),
				date: "1996".into(),
				version: "1.0".into(),
				author: "MAX".into(),
			},
			false,
		);
	});
}

/// Resize: the current 112×112 bounds pre-filled, ready for new dimensions.
#[test]
fn dialog_resize() {
	snap("dialog_resize", 620, 800, |o, _c| o.open_resize(112, 112));
}

/// Generate: the (non-blocking) knob form for the default generator memory over
/// a 64×64 map.
#[test]
fn dialog_generate() {
	snap("dialog_generate", 620, 800, |o, _c| o.open_generate(&crate::genform::GenMemory::default(), 64, 64));
}

/// Convert Palette: the options stage (re-import settings), pre-run.
#[test]
fn dialog_convert_palette() {
	snap("dialog_convert_palette", 620, 800, |o, _c| o.open_convert_palette());
}

/// New from Image: the quantize/settings stage for a 64×48 source, pre-run.
#[test]
fn dialog_new_from_image() {
	snap("dialog_new_from_image", 620, 800, |o, _c| o.open_new_image(64, 48));
}

/// Fix Shore: the non-blocking float showing its stat rows for 42 found tiles.
#[test]
fn dialog_fix_shore() {
	snap("dialog_fix_shore", 620, 800, |o, _c| o.open_autofix(42));
}

/// Object Field: the shared one-field editor at its resource-brush Amount face —
/// a right-aligned digits field (max 31) over the OK/Cancel pair. The audit
/// found this dialog with no golden at all: its predecessor modal's snapshot
/// died with the unitprops in-place rework (`30ca0ac`) and was never replaced.
#[test]
fn dialog_object_field() {
	snap("dialog_object_field", 620, 800, |o, _c| o.open_resource_amount("12"));
}

/// Edit Save Data: the Game Setup tab — save name, the five team rows (type
/// select where the tail allows it, clan select, name field) and the twelve
/// game options, under the flush framed tab bar.
#[test]
fn dialog_save_data() {
	snap("dialog_save_data", 720, 800, |o, _c| o.open_save_data(crate::savedata::tests::init()));
}

/// Edit Save Data: the Stats tab — the all-players score/build-counter table
/// with the per-team column washes.
#[test]
fn dialog_save_data_stats() {
	snap("dialog_save_data_stats", 720, 800, |o, _c| {
		o.open_save_data(crate::savedata::tests::init());
		o.show_save_data_tab_for_test(1);
	});
}

/// Edit Save Data: the Research tab — the eight topic levels per player.
#[test]
fn dialog_save_data_research() {
	snap("dialog_save_data_research", 720, 800, |o, _c| {
		o.open_save_data(crate::savedata::tests::init());
		o.show_save_data_tab_for_test(2);
	});
}

/// Edit Save Data: the Upgrades tab — the unit-type select over the
/// purchased-upgrade (master current values) table per player.
#[test]
fn dialog_save_data_upgrades() {
	snap("dialog_save_data_upgrades", 720, 800, |o, _c| {
		o.open_save_data(crate::savedata::tests::init());
		o.show_save_data_tab_for_test(3);
	});
}

/// Edit Save Data: the Advanced tab — game scalars, team selects, the cheater
/// pair, and the seven in-game preference settings.
#[test]
fn dialog_save_data_advanced() {
	snap("dialog_save_data_advanced", 720, 800, |o, _c| {
		o.open_save_data(crate::savedata::tests::init());
		o.show_save_data_tab_for_test(4);
	});
}

/// Invalid Save Data: the validation list (field, value, valid range per row)
/// with Back / Auto Fix.
#[test]
fn dialog_save_data_issues() {
	use crate::savedata::{Issue, Target};
	snap("dialog_save_data_issues", 720, 800, |o, _c| {
		o.open_save_data(crate::savedata::tests::init());
		o.open_save_data_issues_for_test(vec![
			Issue {
				field: "Game Setup / Start gold".into(),
				message: "is 700000 - enter 0 to 9999".into(),
				target: Target::StartGold,
				fixed: "9999".into(),
			},
			Issue {
				field: "Advanced / Turn counter".into(),
				message: "is 0 - enter 1 to 999999".into(),
				target: Target::TurnCounter,
				fixed: "1".into(),
			},
		]);
	});
}

/// New Map: the size preset / palette selector plus the per-pack tile preview
/// strips composed from the bundled tilepacks.
#[test]
fn dialog_new_map() {
	snap("dialog_new_map", 620, 800, |o, c| {
		let assets = assets();
		let packs = crate::packlist::scan(&assets);
		let (palettes, tilesets) =
			crate::newmap::palette_choices(&packs, &assets, std::path::Path::new("/nonexistent"));
		o.open_newmap(c, packs, &assets, palettes, tilesets, true, None);
	});
}

/// Import WRL: the pack picker for a SNOW1 import (112×112, 400 cells), with the
/// original-colour preview strips.
#[test]
fn dialog_import_wrl() {
	snap("dialog_import_wrl", 620, 800, |o, c| {
		let assets = assets();
		o.open_import_wrl(c, crate::packlist::scan(&assets), &assets, "SNOW1", (112, 112, 400));
	});
}

/// Rename Template: the frozen thumbnail + a name field seeded with "Ridge" and
/// a 2×1 footprint (a "Bluff" name already exists — the clash source).
#[test]
fn dialog_rename() {
	snap("dialog_rename", 620, 800, |o, c| {
		let (preview, tw, th) = template_preview();
		o.open_rename_template(c, "Ridge", (2, 1), vec!["Bluff".into()], &preview, tw, th);
	});
}

/// Delete Template: the frozen thumbnail + name/footprint over a danger confirm.
#[test]
fn dialog_delete_template() {
	snap("dialog_delete_template", 620, 800, |o, c| {
		let (preview, tw, th) = template_preview();
		o.open_delete_template(c, "Ridge", (2, 1), &preview, tw, th);
	});
}

/// A Confirm (Delete Palette): title, prompt, note, and a danger action button.
#[test]
fn dialog_confirm_delete() {
	snap("dialog_confirm_delete", 620, 800, |o, _c| {
		o.open_confirm(
			"Delete Palette",
			"Delete \"swamp\"?",
			"This cannot be undone.",
			"Delete",
			"palette-delete \"/u/swamp.json\"".into(),
		);
	});
}

/// A Confirm-Save (unsaved-changes guard): Cancel / Discard / Save (primary).
#[test]
fn dialog_confirm_save() {
	snap("dialog_confirm_save", 620, 800, |o, _c| {
		o.open_confirm_save(
			"Unsaved Changes",
			"\"scratch\" has unsaved changes.",
			"Save",
			"save-and-close".into(),
			"Discard",
			"close-project!".into(),
		);
	});
}

/// Dedupe: the duplicate template names in a scrolling well over a danger
/// confirm (the populated path, not the empty acknowledgement).
#[test]
fn dialog_dedupe() {
	snap("dialog_dedupe", 620, 800, |o, _c| {
		o.open_dedupe(&["ridge-a".into(), "ridge-b".into(), "coast-1".into(), "coast-2".into()]);
	});
}

/// Error: the word-wrapped message + OK dismiss.
#[test]
fn dialog_error() {
	snap("dialog_error", 620, 800, |o, _c| {
		o.open_error("Could not read \"/maps/atoll.wrl\": unexpected end of file.");
	});
}

/// Notice: a dismiss-only acknowledgement with a custom button label.
#[test]
fn dialog_notice() {
	snap("dialog_notice", 620, 800, |o, _c| {
		o.open_notice("Export Complete", "Close", "Wrote map to \"/maps/atoll.wrl\" (112x112, 400 cells).");
	});
}

/// Editor Preferences: the three game-folder path fields (with Browse) + the
/// "Don't ask again" toggle, seeded with example paths.
#[test]
fn dialog_preferences() {
	snap("dialog_preferences", 640, 800, |o, _c| {
		o.open_preferences("/home/you/MAX", "/home/you/.local/share/max-port", "/opt/max-port", false, false);
	});
}

/// Save-open confirm (swapped map): Abort / Open Anyway with a multi-line body,
/// shown when the installed map at the slot didn't fit but the original did.
#[test]
fn dialog_confirm_open_save() {
	snap("dialog_confirm_open_save", 620, 800, |o, _c| {
		o.open_confirm_labeled(
			"Open Save",
			"The GREEN_3 map installed in your M.A.X. folder is 50x50, which doesn't fit this save.\n\
			 The original GREEN_3 (112x112) does - its dimensions match.\n\n\
			 Open the save on the original GREEN_3?",
			"Abort",
			"Open Anyway",
			"open-save-anyway".into(),
		);
	});
}

/// Experimental-feature warning shown before the Open Save File picker: Cancel /
/// I Understand over the multi-line "this can break your saves" body, with a red
/// "don't report game bugs on modified saves" warning line.
#[test]
fn dialog_confirm_experimental_open_save() {
	snap("dialog_confirm_experimental_open_save", 620, 800, |o, _c| {
		o.open_confirm_warned(
			"Experimental Feature",
			"The Save File editor is EXPERIMENTAL. It may not work, and it can corrupt or destroy real saved games.\n\n\
			 Back up your save files manually before you touch anything here.\n\n\
			 Misuse may result in unforeseen consequences, like: world destruction or -kzzt- your -kzzt- cat.",
			"/!\\ DO NOT REPORT GAME BUGS IF YOU PLAY ON MODIFIED SAVE FILES",
			"Cancel",
			"I Understand",
			"file-dialog open-save".into(),
		);
	});
}

/// Palette name (Save): a name field seeded with "forest", the inline alert
/// slot, and Cancel / Save (a "swamp" palette already exists — the clash set).
#[test]
fn dialog_palette_name() {
	snap("dialog_palette_name", 620, 800, |o, _c| {
		o.open_palette_name("Save Palette", "forest", None, vec!["swamp".into()]);
	});
}

/// Tile Painter: the blocking editor modal with its composed canvas (a magenta
/// palette slot filling the canvas) + swatch strip, at its natural larger size.
#[test]
fn dialog_tile_paint() {
	snap("dialog_tile_paint", 900, 700, |o, c| {
		let mut pal: Vec<u8> = (0..256u16).flat_map(|i| [i as u8, i as u8, i as u8, 255]).collect();
		pal[5 * 4..5 * 4 + 4].copy_from_slice(&[255, 0, 255, 255]); // slot 5 = magenta
		let run = crate::tilepaint::TilePaintRun {
			mode: crate::tilepaint::Mode::Edit,
			tile_id: "GLa000".into(),
			pack_name: "GREEN".into(),
			mask: None,
			canvas: vec![5u8; crate::tilepaint::TILE * crate::tilepaint::TILE],
			canvas_rev: 0,
			pass: 1,
			id_text: "GLa000".into(),
			packs: Vec::new(),
		};
		o.open_tile_paint(c, &run, &pal, false, None);
	});
}

/// An authoring run over the two-pack list every scenery golden uses: an image
/// (New/Clone from a PNG) or a finished piece (Clone/Edit of a library entry).
fn scenery_run(
	mode: crate::scenerypaint::Mode,
	src: Vec<u8>,
	piece: Option<map_core::SceneryPiece>,
) -> crate::scenerypaint::SceneryPaintRun {
	let (name, id) = match &piece {
		Some(p) if mode.in_place() => (p.name.clone(), p.id.clone()),
		Some(p) => (format!("{} Copy", p.name), format!("{}-copy", p.id)),
		None => ("Oak Stand 3".to_string(), "oak-stand-3".to_string()),
	};
	let from = piece.as_ref().map(|p| ("GREEN".to_string(), p.id.clone(), p.user));
	crate::scenerypaint::SceneryPaintRun {
		mode,
		packs: vec!["GREEN".into(), "SNOW".into()],
		grounds: vec![[84, 116, 60], [180, 186, 196]],
		pack_sel: 0,
		src_w: if src.is_empty() { 0 } else { 128 },
		src_h: if src.is_empty() { 0 } else { 128 },
		src,
		piece,
		from,
		name_text: name,
		id_text: id,
		rev: 1,
		hgt_src: Vec::new(),
		hgt_w: 0,
		hgt_h: 0,
		hgt_rev: 0,
		hgt_out: Vec::new(),
		hgt_out_w: 0,
		hgt_out_h: 0,
	}
}

/// New Scenery: the blocking editor modal over a real imported image - a green
/// canopy in three tones on a brown trunk, with a half-alpha shadow falling
/// down-left of it, on a clear field.
///
/// The shot is the one place the whole chain is visible at once: the alpha
/// bands landing in the two planes, the crop, the footprint readout, the
/// sub-palette the object quantized onto, and the shadow over the transparency
/// checkerboard - the one backdrop that shows *how* see-through it is.
#[test]
fn dialog_scenery_new() {
	snap("dialog_scenery_new", 880, 700, |o, c| {
		let assets = assets();
		let project = map_core::Project::new(16, 16, &["GREEN".to_string()], &assets, 7).expect("GREEN project");
		let run = scenery_run(crate::scenerypaint::Mode::New, scenery_source(), None);
		let rgba: Vec<u8> = project.palette.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect();
		o.open_scenery_new(c, &run, &project.palette, &rgba);
	});
}

/// A 128x128 RGBA cut-out source: a three-tone canopy over a trunk (opaque),
/// an ellipse of half-alpha shadow offset down-left (the middle band), and
/// clear everywhere else.
fn scenery_source() -> Vec<u8> {
	let (w, h) = (128usize, 128usize);
	let mut px = vec![0u8; w * h * 4];
	let mut set = |x: usize, y: usize, rgba: [u8; 4]| px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&rgba);
	for y in 0..h {
		for x in 0..w {
			let (dx, dy) = ((x as f32 - 48.0) / 34.0, (y as f32 - 96.0) / 14.0);
			if dx * dx + dy * dy <= 1.0 {
				set(x, y, [0, 0, 0, 128]);
			}
		}
	}
	for y in 70..104 {
		for x in 60..70 {
			set(x, y, [90, 60, 30, 255]);
		}
	}
	for y in 0..h {
		for x in 0..w {
			let (dx, dy) = ((x as f32 - 64.0) / 36.0, (y as f32 - 46.0) / 34.0);
			let d = dx * dx + dy * dy;
			if d > 1.0 {
				continue;
			}
			set(
				x,
				y,
				if d < 0.28 {
					[120, 200, 100, 255]
				} else if d < 0.7 {
					[60, 140, 60, 255]
				} else {
					[30, 80, 40, 255]
				},
			);
		}
	}
	px
}

/// New Scenery, recoloured: the same import with every used colour selected and
/// dropped onto GREEN's sand ramp in **Ramp** mode.
///
/// This is the evidence for the whole recolour feature - the object comes back
/// sandy with its three-tone shading *intact* (a Flat remap would leave a
/// silhouette), the sub-palette strip shows where the colours now point, and
/// the shadow is unmoved, because a shadow is an alpha and not an ink.
#[test]
fn dialog_scenery_recolor() {
	snap("dialog_scenery_recolor", 880, 700, |o, c| {
		let assets = assets();
		let project = map_core::Project::new(16, 16, &["GREEN".to_string()], &assets, 7).expect("GREEN project");
		let run = scenery_run(crate::scenerypaint::Mode::New, scenery_source(), None);
		let rgba: Vec<u8> = project.palette.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect();
		o.open_scenery_new(c, &run, &project.palette, &rgba);
		o.scenery_recolor_for_test(c, 168, crate::scenerypaint::RemapMode::Ramp);
	});
}

/// New Scenery, **Heightmap tab**: how high the piece stands, as the picture
/// somebody paints on.
///
/// The shot is the fallback made visible - nothing has been drawn for this
/// image, so the well shows the relief *inferred* from the art (the canopy
/// domes, the trunk is a low ridge) and the note says so rather than passing a
/// guess off as a measurement. The two file keys are the editing loop, Clear is
/// the way back to the inference, and `Stands:` is here because it is what a
/// grey means: white is the peak.
#[test]
fn dialog_scenery_heightmap() {
	snap("dialog_scenery_heightmap", 880, 700, |o, c| {
		let assets = assets();
		let project = map_core::Project::new(16, 16, &["GREEN".to_string()], &assets, 7).expect("GREEN project");
		let run = scenery_run(crate::scenerypaint::Mode::New, scenery_source(), None);
		let rgba: Vec<u8> = project.palette.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect();
		o.open_scenery_new(c, &run, &project.palette, &rgba);
		o.scenery_show_heightmap_for_test();
	});
}

/// Edit Scenery: the same dialog opened on a piece out of the shipped GREEN
/// library instead of on an image.
///
/// Three things the New shot cannot show: the alpha thresholds are **gone**
/// (a cut piece has no bands left to move - the `Reveal` is closed, and the
/// import key says "Replace art..."), the id field and the pack dropdown are
/// disabled because a placement points at both, and the sub-palette is the
/// object's real quantized ramp rather than one derived from a test image.
#[test]
fn dialog_scenery_edit() {
	snap("dialog_scenery_edit", 880, 780, |o, c| {
		let assets = assets();
		let project = map_core::Project::new(16, 16, &["GREEN".to_string()], &assets, 7).expect("GREEN project");
		let piece = crate::scenery::piece_at(&project, 0).expect("the GREEN library ships pieces").1.clone();
		let run = scenery_run(crate::scenerypaint::Mode::Edit, Vec::new(), Some(piece));
		let rgba: Vec<u8> = project.palette.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect();
		o.open_scenery_new(c, &run, &project.palette, &rgba);
	});
}

/// Rename Scenery: the one-field name prompt, with the id it will not touch.
#[test]
fn dialog_rename_scenery() {
	snap("dialog_rename_scenery", 620, 800, |o, _c| o.open_rename_scenery("GREEN", "mountain-3", "Mountain 3"));
}

/// Delete Scenery: the confirm, naming how many placements it makes inert.
#[test]
fn dialog_delete_scenery() {
	snap("dialog_delete_scenery", 620, 800, |o, _c| o.open_delete_scenery("GREEN", "oak-stand-3", "Oak Stand 3", 4));
}

/// Match editor: the blocking editor modal over a real GREEN project, its list
/// thumbnails from the composed tile atlas and a magenta cross/orientation
/// strip, at its natural larger size.
#[test]
fn dialog_match_edit() {
	snap("dialog_match_edit", 900, 700, |o, c| {
		let assets = assets();
		let project = map_core::Project::new(16, 16, &["GREEN".to_string()], &assets, 7).expect("GREEN project");
		let me = crate::matcheditor::MatchEditor::new(&project, None).expect("match rules");
		let lut = crate::tile_atlas::rest_lut(&project.palette);
		let (argba, aw, ah, acount) = crate::tile_atlas::compose(&project, &lut);
		let atlas_tex = c.register_texture(&argba, aw, ah);
		let strip = vec![[255u8, 0, 255, 255]; 9 * 64 * 64].concat();
		o.open_match_edit(c, me, &strip, (atlas_tex, acount, 0));
	});
}

/// UI Tests (DEV): the font/raster probe at each shipped UI scale. These three
/// are the evidence for the "text looks resized at 125%/150%" report and the
/// regression net under it - a change to the rasterizer, the glyph bucket, or
/// the engraving offset shows up here first, and at the two fractional scales
/// it shows up *only* here.
#[test]
fn dialog_ui_tests_100() {
	snap_at("dialog_ui_tests_100", 1.0, 620, 640, |o, _c| o.open_ui_tests());
}

#[test]
fn dialog_ui_tests_125() {
	snap_at("dialog_ui_tests_125", 1.25, 780, 800, |o, _c| o.open_ui_tests());
}

#[test]
fn dialog_ui_tests_150() {
	snap_at("dialog_ui_tests_150", 1.5, 940, 960, |o, _c| o.open_ui_tests());
}
