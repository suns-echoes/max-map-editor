//! One-shot data fix for the shipped land tilepacks. For each pack it
//!
//!   1. bakes the in-game (game-overwritten) colours into `palette.json` -
//!      every static slot becomes its [`apply_game_statics`] value, so the
//!      shipped palette matches what the editor renders (no `#d707ff`
//!      placeholders / stale bytes in the animated slots); and
//!   2. de-animates every LAND / OBSTRUCTION tile - any pixel on a cycled slot
//!      (effects 9..=31, water 96..=127) is remapped to the nearest static
//!      look-alike via [`map_core::deanimate_pixels`], the same mapping the WRL
//!      importer uses. SHORE / WATER tiles are left untouched.
//!
//! Idempotent. Run from the repo root:
//!     cargo run -p map-core --example deanimate_packs

use std::path::Path;

use map_core::{TileKind, TilePack, apply_game_statics, deanimate_remap, deanimate_with, family_of, write_palette};

const PACKS: &[&str] = &["CRATER", "DESERT", "GREEN", "SNOW", "SNOW_DARK"];

/// Write `bytes` only when they differ from what is on disk; report whether a
/// write happened (so the summary counts real changes only).
fn write_if_changed(path: &Path, bytes: &[u8]) -> bool {
	if std::fs::read(path).is_ok_and(|cur| cur == bytes) {
		return false;
	}
	std::fs::write(path, bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
	true
}

fn main() {
	let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/assets/tilepacks");

	let (mut total_tiles, mut total_pixels) = (0usize, 0usize);
	for &name in PACKS {
		let mut pack = TilePack::load(&assets, name).unwrap_or_else(|e| panic!("load {name}: {e}"));
		let dir = assets.join(name);

		// 1. Bake the game statics into the palette.
		let mut baked = pack.palette.clone().unwrap_or_else(|| panic!("{name}: no palette.json"));
		apply_game_statics(&mut baked);
		let pal_name = pack.palette_name.clone().unwrap_or_else(|| format!("{name} Palette"));
		let pal_changed = write_if_changed(&dir.join("palette.json"), write_palette(&baked, &pal_name).as_bytes());

		// 2. De-animate LAND / OBSTRUCTION tiles against that baked palette.
		let remap = deanimate_remap(&baked);
		let (mut tiles, mut pixels) = (0usize, 0usize);
		for t in 0..pack.tile_count() {
			let kind = pack.props.get(family_of(&pack.ids[t as usize])).and_then(|p| p.kind);
			if !matches!(kind, Some(TileKind::Land | TileKind::Obstruction)) {
				continue;
			}
			let mut px = pack.tile_pixels(t).to_vec();
			let changed = deanimate_with(&mut px, &remap);
			if changed > 0 {
				pack.set_tile_pixels(t, &px);
				tiles += 1;
				pixels += changed;
			}
		}
		let bin_changed = write_if_changed(&dir.join("tiles-data.bin"), &pack.tiles);

		total_tiles += tiles;
		total_pixels += pixels;
		println!(
			"{name}: palette {}, tiles de-animated={tiles}, pixels={pixels}, bin {}",
			if pal_changed { "baked" } else { "unchanged" },
			if bin_changed { "rewritten" } else { "unchanged" },
		);
	}
	println!("\nTOTAL: {total_tiles} tiles, {total_pixels} pixels de-animated across {} packs", PACKS.len());
}
