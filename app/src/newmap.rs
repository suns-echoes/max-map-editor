//! Shared New Map data: the map-size presets, the per-pack tile-preview
//! atlas, and the palette-selector choices. The New Map dialog itself is the
//! wgpu-ui overlay (`uikit_overlay`); this module only provides what it needs.
//!
//! The previews are CPU-composed RGBA strips - 5 tiles per pack picked to be
//! representative (the `?S[aA]000` shore tile and the `?L[aA]000` land tile
//! when the pack has them, then random tiles from other families) - coloured
//! with the pack's palette (or the dialog's custom palette choice), game
//! statics applied either way, and masked (shore) pixels showing the chosen
//! water tileset's first tile beneath, as they render in game. Registered as
//! a texture the overlay shows per pack via a `wgpu_ui::Image` uv and rebuilt
//! when the palette / water choice changes.

use std::path::{Path, PathBuf};

use map_core::{GAME_PALETTE, Rng, TilePack, apply_game_statics, family_of};

use crate::packlist::PackEntry;

/// Deterministic "random" previews - stable screenshots, still varied.
const PREVIEW_SEED: u64 = 0xC0FFEE;

/// Shared map-size presets (the overlay New Map + Resize dialogs).
pub const SIZE_PRESETS: [(&str, u16, u16); 3] =
	[("Classic 112x112", 112, 112), ("Mega 224x224", 224, 224), ("Giga 448x448", 448, 448)];

/// One row of the New Map palette selector: row 0 is "from selected tileset"
/// (no path - the owner radio's pack palette); the tileset palettes and the
/// user palettes follow, each with the file it loads from.
#[derive(Clone, PartialEq)]
pub struct PaletteChoice {
	pub label: String,
	pub path: Option<PathBuf>,
}

/// The palette selector's rows: "from selected tileset", the installed
/// tileset palettes (pack-list order, display names), then the user palettes
/// from `user_dir` (file order, display names). Returns the choices and the
/// tileset-palette count - the selector draws its separator after option
/// index `1 + count - 1 = count` when user palettes follow.
pub fn palette_choices(packs: &[PackEntry], assets_root: &Path, user_dir: &Path) -> (Vec<PaletteChoice>, usize) {
	let mut out = vec![PaletteChoice { label: "from selected tileset".to_string(), path: None }];
	for p in packs.iter().filter(|p| p.has_palette) {
		out.push(PaletteChoice {
			label: p.palette_name.clone().unwrap_or_else(|| p.title.clone()),
			path: Some(assets_root.join(&p.name).join("palette.json")),
		});
	}
	let tilesets = out.len() - 1;
	let mut files: Vec<PathBuf> = std::fs::read_dir(user_dir)
		.map(|entries| {
			entries
				.filter_map(|e| e.ok())
				.map(|e| e.path())
				.filter(|p| p.extension().is_some_and(|e| e == "json"))
				.collect()
		})
		.unwrap_or_default();
	files.sort();
	for f in files {
		let label = crate::packlist::json_name(&f)
			.map(|n| crate::packlist::trim_suffix(&n, "palette"))
			.filter(|s| !s.is_empty())
			.or_else(|| f.file_stem().map(|s| s.to_string_lossy().into_owned()))
			.unwrap_or_default();
		out.push(PaletteChoice { label, path: Some(f) });
	}
	(out, tilesets)
}

/// Tiles per preview strip: the `?S[aA]000` shore + `?L[aA]000` land picks
/// and three random-family fills.
pub const PREVIEW_TILES: usize = 5;

/// The [`PREVIEW_TILES`] preview tiles of a pack: the `?S[aA]000` shore tile
/// and the `?L[aA]000` land tile when present, filled with random tiles from
/// families not already shown (relaxing to any unpicked tile, then any tile,
/// so tiny packs still fill their strip).
pub fn preview_picks(pack: &TilePack, rng: &mut Rng) -> [u16; PREVIEW_TILES] {
	let count = pack.tile_count() as u32;
	if count == 0 {
		return [0; PREVIEW_TILES];
	}
	let find = |c: u8| {
		pack.ids
			.iter()
			.position(|id| {
				let b = id.as_bytes();
				b.len() == 6 && b[1] == c && (b[2] == b'a' || b[2] == b'A') && &b[3..] == b"000"
			})
			.map(|i| i as u16)
	};
	let mut picks: Vec<u16> = [find(b'S'), find(b'L')].into_iter().flatten().collect();
	let mut families: Vec<String> = picks.iter().map(|&t| family_of(&pack.ids[t as usize]).to_string()).collect();
	let mut attempt = 0u32;
	while picks.len() < PREVIEW_TILES {
		let t = rng.below(count) as u16;
		let fam = family_of(&pack.ids[t as usize]);
		attempt += 1;
		let fresh_family = attempt < 32 && !families.contains(&fam.to_string()) && !picks.contains(&t);
		let fresh_tile = (32..96).contains(&attempt) && !picks.contains(&t);
		if fresh_family || fresh_tile || attempt >= 96 {
			families.push(fam.to_string());
			picks.push(t);
		}
	}
	let mut out = [0u16; PREVIEW_TILES];
	out.copy_from_slice(&picks[..PREVIEW_TILES]);
	out
}

/// Compose the preview atlas: `PREVIEW_TILES*64` wide × `packs.len()*64` tall
/// RGBA, row `i` = pack `i`'s tile strip ([`preview_picks`]). Colours come from
/// `override_palette` when given (the selector's custom choice), else each
/// pack's own palette (palette-less packs borrow the first owner's) - game
/// statics applied either way, exactly as the game renders. Masked (shore)
/// pixels show `water_pack`'s first tile beneath; without loadable water art
/// they fall back to a dim checker. `rows` names each row's pack (empty
/// string for a pack that failed to load).
pub fn build_rgba(
	packs: &[PackEntry],
	assets_root: &Path,
	override_palette: Option<&[u8]>,
	water_pack: &str,
) -> (Vec<u8>, Vec<String>) {
	let loaded: Vec<Option<TilePack>> = packs.iter().map(|p| TilePack::load(assets_root, &p.name).ok()).collect();
	// The borrowed palette for palette-less packs (WATER): GREEN's if installed
	// (the canonical planet colors), else the first owner.
	let fallback: Vec<u8> = loaded
		.iter()
		.flatten()
		.find(|p| p.name == "GREEN")
		.and_then(|p| p.palette.clone())
		.or_else(|| loaded.iter().flatten().find_map(|p| p.palette.clone()))
		.unwrap_or_else(|| GAME_PALETTE.to_vec());
	// The chosen water tileset's first tile - the underlay masked (shore)
	// pixels reveal. Index-level: in game every pack renders through the one
	// project palette, so the underlay shares the row's palette.
	let water_pixels: Option<&[u8]> =
		loaded.iter().flatten().find(|p| p.name == water_pack && p.tile_count() > 0).map(|p| p.tile_pixels(0));

	let n = packs.len().max(1);
	let (tw, th) = (PREVIEW_TILES * 64, n * 64);
	let mut rgba = vec![0u8; tw * th * 4];
	let mut rows = Vec::with_capacity(packs.len());
	for (row, pack) in loaded.iter().enumerate() {
		let Some(pack) = pack else {
			rows.push(String::new());
			continue;
		};
		rows.push(packs[row].name.clone());
		if pack.tile_count() == 0 {
			continue;
		}
		let mut palette =
			override_palette.map(<[u8]>::to_vec).or_else(|| pack.palette.clone()).unwrap_or_else(|| fallback.clone());
		apply_game_statics(&mut palette);
		let mut rng = Rng::new(PREVIEW_SEED + row as u64);
		for (slot, &tile) in preview_picks(pack, &mut rng).iter().enumerate() {
			let pixels = pack.tile_pixels(tile);
			let mask = pack.tile_mask(tile);
			for y in 0..64usize {
				for x in 0..64usize {
					let mut p = pixels[y * 64 + x];
					let at = ((row * 64 + y) * tw + slot * 64 + x) * 4;
					if Some(p) == mask {
						match water_pixels {
							Some(w) => p = w[y * 64 + x],
							None => {
								// No water art to show through: dim checker.
								let dim = if (x / 8 + y / 8) % 2 == 0 { 26 } else { 34 };
								rgba[at..at + 4].copy_from_slice(&[dim, dim, dim, 255]);
								continue;
							}
						}
					}
					let p = p as usize;
					rgba[at..at + 3].copy_from_slice(&palette[p * 3..p * 3 + 3]);
					rgba[at + 3] = 255;
				}
			}
		}
	}
	(rgba, rows)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn assets_root() -> std::path::PathBuf {
		Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/assets/tilepacks")
	}

	#[test]
	fn build_rgba_makes_a_stable_row_per_pack() {
		let packs = crate::packlist::scan(&assets_root());
		assert!(!packs.is_empty(), "stock packs present");
		let (rgba, rows) = build_rgba(&packs, &assets_root(), None, "WATER");
		// PREVIEW_TILES tiles wide (64px each), one 64px row per pack, RGBA.
		assert_eq!(rgba.len(), PREVIEW_TILES * 64 * packs.len() * 64 * 4);
		assert_eq!(rows.len(), packs.len());
		assert!(rows.iter().any(|r| !r.is_empty()), "at least one pack loaded: {rows:?}");
		// Seeded: a second build is byte-identical (stable screenshots).
		let (rgba2, _) = build_rgba(&packs, &assets_root(), None, "WATER");
		assert_eq!(rgba, rgba2, "seeded previews are deterministic");
	}

	#[test]
	fn picks_lead_with_the_shore_and_land_families() {
		let pack = TilePack::load(&assets_root(), "GREEN").expect("GREEN loads");
		let mut rng = Rng::new(PREVIEW_SEED);
		let picks = preview_picks(&pack, &mut rng);
		assert_eq!(pack.ids[picks[0] as usize], "GSa000", "the ?S[aA]000 shore tile leads");
		assert_eq!(pack.ids[picks[1] as usize], "GLa000", "the ?L[aA]000 land tile follows");
		let fams: Vec<&str> = picks.iter().map(|&t| family_of(&pack.ids[t as usize])).collect();
		assert!(!fams[2..].contains(&"GSa") && !fams[2..].contains(&"GLa"), "the random picks vary: {fams:?}");
	}

	#[test]
	fn masked_pixels_show_the_water_tile_beneath() {
		let packs = crate::packlist::scan(&assets_root());
		let green_row = packs.iter().position(|p| p.name == "GREEN").expect("GREEN installed");
		let green = TilePack::load(&assets_root(), "GREEN").expect("GREEN loads");
		let water = TilePack::load(&assets_root(), "WATER").expect("WATER loads");
		let gsa = green.index_of["GSa000"];
		let mask = green.tile_mask(gsa).expect("GSa is a masked shore family");
		let masked_at = green.tile_pixels(gsa).iter().position(|&p| p == mask).expect("GSa000 has masked pixels");
		let (x, y) = (masked_at % 64, masked_at / 64);

		let (rgba, _) = build_rgba(&packs, &assets_root(), None, "WATER");
		// GSa000 is slot 0 of GREEN's row; the masked pixel must hold the
		// water tile's colour under GREEN's palette (game statics applied).
		let mut palette = green.palette.clone().expect("GREEN owns a palette");
		apply_game_statics(&mut palette);
		let w = water.tile_pixels(0)[y * 64 + x] as usize;
		let at = ((green_row * 64 + y) * (PREVIEW_TILES * 64) + x) * 4;
		assert_eq!(&rgba[at..at + 3], &palette[w * 3..w * 3 + 3], "water shows through the shore mask");
	}

	#[test]
	fn palette_choices_list_tilesets_then_users() {
		let packs = crate::packlist::scan(&assets_root());
		let (choices, tilesets) = palette_choices(&packs, &assets_root(), Path::new("/nonexistent"));
		assert_eq!(choices[0].label, "from selected tileset");
		assert!(choices[0].path.is_none());
		assert_eq!(tilesets, packs.iter().filter(|p| p.has_palette).count());
		assert_eq!(choices.len(), tilesets + 1, "no user palettes from a missing dir");
		let green = choices.iter().find(|c| c.label == "Green").expect("Green palette listed by display name");
		assert!(green.path.as_ref().is_some_and(|p| p.ends_with("GREEN/palette.json")));
	}

	/// A fresh empty scratch dir under the project `temp/`, unique per `tag` so
	/// tests don't collide (the palette_io/settings_io pattern).
	fn scratch(tag: &str) -> PathBuf {
		let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("temp").join(format!("newmap_{tag}"));
		let _ = std::fs::remove_dir_all(&d);
		std::fs::create_dir_all(&d).unwrap();
		d
	}

	/// A minimal picker entry for [`build_rgba`] (which only reads `name`).
	fn pack_entry(name: &str) -> PackEntry {
		PackEntry {
			name: name.to_string(),
			title: name.to_string(),
			selected: true,
			has_palette: false,
			palette_name: None,
			water: false,
		}
	}

	#[test]
	fn user_palettes_follow_with_display_names_or_stems() {
		let dir = scratch("userpals");
		// A named palette, a name-less JSON, a name that trims to nothing, and
		// a non-JSON file that must not be listed at all.
		std::fs::write(dir.join("a_sunset.json"), r#"{"name":"Sunset Palette"}"#).unwrap();
		std::fs::write(dir.join("b_bare.json"), "{}").unwrap();
		std::fs::write(dir.join("c_blank.json"), r#"{"name":""}"#).unwrap();
		std::fs::write(dir.join("notes.txt"), "not a palette").unwrap();
		let (choices, tilesets) = palette_choices(&[], Path::new("/nonexistent-assets"), &dir);
		assert_eq!(tilesets, 0, "no packs, no tileset palettes");
		let labels: Vec<&str> = choices[1..].iter().map(|c| c.label.as_str()).collect();
		assert_eq!(
			labels,
			["Sunset", "b_bare", "c_blank"],
			"the JSON name (' Palette' trimmed) when usable, else the file stem; sorted; non-json skipped"
		);
		assert!(choices[1].path.as_ref().is_some_and(|p| *p == dir.join("a_sunset.json")), "each row keeps its file");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn empty_and_missing_packs_leave_blank_rows() {
		// A valid pack with zero tiles: empty bin + empty id list.
		let root = scratch("emptypack");
		std::fs::create_dir_all(root.join("EMPTY")).unwrap();
		std::fs::write(root.join("EMPTY/tiles-data.bin"), b"").unwrap();
		std::fs::write(root.join("EMPTY/tiles-data.json"), "[]").unwrap();
		let empty = TilePack::load(&root, "EMPTY").expect("a zero-tile pack loads");
		let mut rng = Rng::new(PREVIEW_SEED);
		assert_eq!(preview_picks(&empty, &mut rng), [0; PREVIEW_TILES], "no tiles: the picks stay all zero");

		let (rgba, rows) = build_rgba(&[pack_entry("EMPTY"), pack_entry("MISSING")], &root, None, "WATER");
		assert_eq!(rows, ["EMPTY", ""], "a loaded pack keeps its name; a failed load leaves an empty row name");
		assert_eq!(rgba.len(), PREVIEW_TILES * 64 * 2 * 64 * 4, "the atlas still spans both rows");
		assert!(rgba.iter().all(|&b| b == 0), "neither row paints a pixel");
		let _ = std::fs::remove_dir_all(&root);
	}

	#[test]
	fn masked_pixels_fall_back_to_the_checker_without_water_art() {
		let assets = assets_root();
		let green = TilePack::load(&assets, "GREEN").expect("GREEN loads");
		let gsa = green.index_of["GSa000"];
		let mask = green.tile_mask(gsa).expect("GSa is a masked shore family");
		let masked_at = green.tile_pixels(gsa).iter().position(|&p| p == mask).expect("GSa000 has masked pixels");
		let (x, y) = (masked_at % 64, masked_at / 64);

		// No loadable water pack: the shore mask shows the dim 8px checker.
		let (rgba, rows) = build_rgba(&[pack_entry("GREEN")], &assets, None, "NO_SUCH_WATER");
		assert_eq!(rows, ["GREEN"]);
		let at = (y * (PREVIEW_TILES * 64) + x) * 4;
		let dim = if (x / 8 + y / 8) % 2 == 0 { 26 } else { 34 };
		assert_eq!(&rgba[at..at + 4], &[dim, dim, dim, 255], "masked pixel shows the checker, opaque");
	}
}
