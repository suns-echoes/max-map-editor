//! Export every shipped template to a PNG - the offline step that turns
//! `resources/assets/templates/<PACK>/*.json` into
//! `private/sources/scenery/<PACK>/*.png`, as reference art for
//! hand-authoring high-quality scenery (`SCENERY.md` stage H).
//!
//! Usage:
//! ```text
//! cargo run -p map-core --example export_template_pngs              # every pack
//! cargo run -p map-core --example export_template_pngs GREEN DESERT # named packs
//! ```
//!
//! The render is the template exactly as it sits on a map: every part of a cell
//! spec painted in order (water under, ground over), transforms applied, indices
//! resolved through the pack's own palette with the game's static slots baked in
//! ([`apply_game_statics`]) so the animated ranges read as they do in-game.
//!
//! RGBA8, and the alpha carries exactly one bit of meaning: a cell the template
//! leaves **empty** is fully transparent, everything a tile covers is fully
//! opaque. No cut-out, no shadow inference - that judgement is
//! `bake_scenery`'s, and the point of this export is to hand the artist the
//! untouched source. A template's hole is a hole, though, and painting it black
//! would lie about the footprint.

use std::path::{Path, PathBuf};

use map_core::{Template, TilePack, Transform, apply_game_statics, transform_tile};
use max_assets::wrl::TILE_DATA_SIZE;

const PACKS: [&str; 5] = ["CRATER", "DESERT", "GREEN", "SNOW", "SNOW_DARK"];
const TILE: usize = 64;

/// Where the hand-authored art lives - the cut-outs and the traced lines, one
/// directory per pack. Bake input only, and deliberately outside `resources/`:
/// it is worked on by hand and never shipped.
const SOURCE_ART: &str = "private/sources/scenery";

/// The art directory for one pack.
fn source_art(pack: &str) -> PathBuf {
	Path::new(SOURCE_ART).join(pack)
}

/// Every template's `use` list names its own pack plus this one - the shared
/// sea art, which owns no palette of its own and rides the ground pack's.
const WATER: &str = "WATER";

fn main() {
	let wanted: Vec<String> = std::env::args().skip(1).collect();
	let packs: Vec<String> = if wanted.is_empty() { PACKS.iter().map(|p| p.to_string()).collect() } else { wanted };

	let assets = Path::new("resources/assets");
	if !assets.join("tilepacks").is_dir() {
		eprintln!("run from the repo root (no resources/assets/tilepacks here)");
		std::process::exit(2);
	}
	let water = match TilePack::load(&assets.join("tilepacks"), WATER) {
		Ok(pack) => pack,
		Err(e) => {
			eprintln!("{WATER}: {e}");
			std::process::exit(1);
		}
	};

	let mut failed = false;
	let mut written = 0usize;
	for pack in &packs {
		match export_pack(assets, pack, &water) {
			Ok(n) => written += n,
			Err(e) => {
				eprintln!("{pack}: {e}");
				failed = true;
			}
		}
	}
	println!("{written} PNGs written");
	if failed {
		std::process::exit(1);
	}
}

fn export_pack(assets: &Path, name: &str, water: &TilePack) -> Result<usize, String> {
	let pack = TilePack::load(&assets.join("tilepacks"), name)?;
	let mut palette = pack.palette.clone().ok_or("pack owns no palette")?;
	apply_game_statics(&mut palette);

	let dir = assets.join("templates").join(name);
	let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
		.map_err(|e| format!("read {}: {e}", dir.display()))?
		.filter_map(|e| e.ok().map(|e| e.path()))
		.filter(|p| p.extension().is_some_and(|x| x == "json"))
		.collect();
	files.sort();

	let out = source_art(name);
	std::fs::create_dir_all(&out).map_err(|e| format!("create {}: {e}", out.display()))?;

	let (mut holes, mut unresolved) = (0usize, Vec::new());
	for path in &files {
		let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
		let template = Template::load(path)?;
		let shot = render(&template, &pack, water, &palette);
		holes += shot.holes;
		for missing in shot.unresolved {
			unresolved.push(format!("{id}:{missing}"));
		}
		write_png(&out.join(format!("{id}.png")), shot.width, shot.height, &shot.rgba)?;
	}

	println!(
		"{name}: {} PNGs -> {}{}",
		files.len(),
		out.display(),
		if holes > 0 { format!(" ({holes} empty cells left transparent)") } else { String::new() },
	);
	if !unresolved.is_empty() {
		// Not fatal - the cell just renders transparent - but it means a
		// template references art no installed pack carries.
		unresolved.sort();
		unresolved.dedup();
		println!("  warning: {} unresolved tile id(s): {}", unresolved.len(), unresolved.join(", "));
	}
	Ok(files.len())
}

struct Shot {
	width: usize,
	height: usize,
	rgba: Vec<u8>,
	/// Cells no tile covered - transparent in the output.
	holes: usize,
	unresolved: Vec<String>,
}

/// Flatten a template into one RGBA box, `64 px` per cell.
fn render(template: &Template, pack: &TilePack, water: &TilePack, palette: &[u8]) -> Shot {
	let (cols, rows) = (template.width as usize, template.height as usize);
	let (width, height) = (cols * TILE, rows * TILE);
	let mut rgba = vec![0u8; width * height * 4];
	let (mut holes, mut unresolved) = (0usize, Vec::new());

	for cy in 0..rows {
		for cx in 0..cols {
			let spec = &template.cells[cy * cols + cx];
			let mut painted = false;
			for part in spec.split(',').filter(|p| !p.is_empty()) {
				let (id, transform) = match part.split_once(':') {
					Some((id, t)) => (id, Transform::parse(t).unwrap_or_default()),
					None => (part, Transform::default()),
				};
				let source = match pack.index_of.get(id) {
					Some(&tile) => Some((pack, tile)),
					None => water.index_of.get(id).map(|&tile| (water, tile)),
				};
				let Some((source, tile)) = source else {
					unresolved.push(id.to_string());
					continue;
				};
				let mut pixels = [0u8; TILE_DATA_SIZE];
				pixels.copy_from_slice(source.tile_pixels(tile));
				let pixels = transform_tile(&pixels, transform);
				for y in 0..TILE {
					for x in 0..TILE {
						let index = pixels[y * TILE + x] as usize * 3;
						let at = ((cy * TILE + y) * width + cx * TILE + x) * 4;
						rgba[at..at + 3].copy_from_slice(&palette[index..index + 3]);
						rgba[at + 3] = 0xff;
					}
				}
				painted = true;
			}
			if !painted {
				holes += 1;
			}
		}
	}
	Shot { width, height, rgba, holes, unresolved }
}

fn write_png(path: &Path, width: usize, height: usize, rgba: &[u8]) -> Result<(), String> {
	let file = std::fs::File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
	let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
	encoder.set_color(png::ColorType::Rgba);
	encoder.set_depth(png::BitDepth::Eight);
	let mut writer = encoder.write_header().map_err(|e| format!("write {}: {e}", path.display()))?;
	writer.write_image_data(rgba).map_err(|e| format!("write {}: {e}", path.display()))
}
