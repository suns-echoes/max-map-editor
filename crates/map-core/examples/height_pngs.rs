//! Write a **height map beside every source PNG** - the eyeball pass on the
//! inferred relief, before any of it becomes a shipped data layer.
//!
//! ```text
//! cargo run -p map-core --example height_pngs               # every pack
//! cargo run -p map-core --example height_pngs CRATER        # named packs
//! cargo run -p map-core --example height_pngs --clean       # delete them again
//! ```
//!
//! For `private/sources/scenery/<PACK>/<id>.png` it writes `<id>.height.png`: **the same
//! size and the same frame**, greyscale, black where the object is not and
//! `1..=255` where it is, scaled so that white is the peak the object stands
//! at. The two flick back and forth in any viewer with the shape lined up
//! pixel for pixel.
//!
//! **The relief is read off the baked cut-out, not off the PNG.** The PNGs are
//! not one thing: CRATER's and DESERT's are hand-cut art with a real alpha
//! channel, and the other three packs' are untouched template exports - a box of
//! ground with the object somewhere in it, opaque throughout. Only
//! `resources/assets/scenery/<PACK>/`
//! knows which pixels are the object, because that is where `cut` took the
//! ground out. So the field comes from the baked `body` plane through the pack's
//! palette, and is then placed back into the source PNG's frame: at the sprite's
//! own origin for a box-sized PNG, at (0,0) for one that is already the crop.
//!
//! What it draws is `SceneryPiece::height_field` - the very call the renderer
//! and the WRL bake make, so judge what you see here and the judgement carries.
//! For a piece that ships a `<id>.hgt` that is the **authored** height map read
//! straight off disk; for one that does not it is the inference. The two are
//! deliberately indistinguishable here, because to everything downstream they
//! are the same thing: how high the object stands.
//!
//! **These files are QA output, not assets.** They are gitignored; `--clean`
//! removes them.

use std::path::{Path, PathBuf};

use map_core::{SceneryPack, SceneryPiece, TilePack, apply_game_statics, brightness_table};

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

/// What a height map is written as - `<id>.height.png` beside `<id>.png`.
const SUFFIX: &str = ".height.png";

/// A traced rim crest (`bake_scenery`'s `<id>-X.png`) is a bake input, not a
/// piece: it has no relief of its own to draw and is not a template the bake
/// dropped either, so it is skipped outright rather than counted as one.
const MARKER: &str = "-X.png";

fn main() {
	let mut clean = false;
	let mut wanted: Vec<String> = Vec::new();
	for arg in std::env::args().skip(1) {
		match arg.as_str() {
			"--clean" => clean = true,
			other => wanted.push(other.to_string()),
		}
	}
	let packs: Vec<String> = if wanted.is_empty() { PACKS.iter().map(|p| p.to_string()).collect() } else { wanted };

	let assets = Path::new("resources/assets");
	if !assets.join("templates").is_dir() {
		eprintln!("run from the repo root (no resources/assets/templates here)");
		std::process::exit(2);
	}
	let (mut total, mut failed) = (0usize, false);
	for pack in &packs {
		match run(assets, pack, clean) {
			Ok((n, skipped)) => {
				total += n;
				let what = if clean { "removed" } else { "height maps" };
				let note =
					if skipped == 0 { String::new() } else { format!(" ({skipped} PNG(s) the bake has no piece for)") };
				println!("{pack}: {n} {what}{note}");
			}
			Err(e) => {
				eprintln!("{pack}: {e}");
				failed = true;
			}
		}
	}
	println!("{total} file(s) {}", if clean { "removed" } else { "written" });
	if failed {
		std::process::exit(1);
	}
}

/// Returns `(written, skipped)`.
fn run(assets: &Path, name: &str, clean: bool) -> Result<(usize, usize), String> {
	let dir = source_art(name);
	let mut sources: Vec<PathBuf> = std::fs::read_dir(&dir)
		.map_err(|e| format!("read {}: {e}", dir.display()))?
		.filter_map(|e| e.ok().map(|e| e.path()))
		.filter(|p| p.to_string_lossy().ends_with(".png"))
		.collect();
	sources.sort();

	if clean {
		let mut n = 0;
		for path in sources.iter().filter(|p| p.to_string_lossy().ends_with(SUFFIX)) {
			std::fs::remove_file(path).map_err(|e| format!("remove {}: {e}", path.display()))?;
			n += 1;
		}
		return Ok((n, 0));
	}

	let pack = TilePack::load(&assets.join("tilepacks"), name)?;
	let mut palette = pack.palette.clone().ok_or("pack owns no palette")?;
	apply_game_statics(&mut palette);
	let brightness = brightness_table(&palette);
	let library = SceneryPack::load(assets, name)?;

	let (mut n, mut skipped) = (0, 0);
	let art = |p: &&PathBuf| {
		let name = p.to_string_lossy().to_string();
		!name.ends_with(SUFFIX) && !name.ends_with(MARKER)
	};
	for path in sources.iter().filter(art) {
		let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
		// A template the bake dropped (no OBSTRUCTION tile - a meadow, a dune)
		// exports a PNG but is not scenery, so there is no relief to show.
		let Some(piece) = library.pieces.iter().find(|p| p.id == stem) else {
			skipped += 1;
			continue;
		};
		let (w, h) = png_size(path)?;
		let grey = height_map(piece, &brightness, w, h);
		let out = path.with_file_name(format!("{stem}{SUFFIX}"));
		std::fs::write(&out, grey_png(w, h, &grey)).map_err(|e| format!("write {}: {e}", out.display()))?;
		n += 1;
	}
	Ok((n, skipped))
}

/// One piece's height map, in a `w` x `h` frame, as the picture
/// `map_core::height_to_grey` makes of it: white is the peak the piece stands
/// at.
///
/// The same picture the dialog's Heightmap tab shows and reads back, so what is
/// written here can be painted on and imported - there is one convention for
/// what a grey means, not one for looking and another for editing.
///
/// The frame is the **source PNG's**, and which one that is follows from its
/// size: a full template box takes the sprite at its crop origin, anything else
/// is treated as the crop itself and pinned at (0,0). Either way what comes out
/// overlays its input.
fn height_map(piece: &SceneryPiece, brightness: &[u8; 256], w: usize, h: usize) -> Vec<u8> {
	let field = map_core::height_to_grey(&piece.height_field(brightness), piece.height_opts().peak);
	let (sw, sh) = (piece.sprite.width as usize, piece.sprite.height as usize);
	let box_sized = w == piece.cells_w as usize * TILE && h == piece.cells_h as usize * TILE;
	let (ox, oy) = if box_sized { (piece.sprite.origin_x as usize, piece.sprite.origin_y as usize) } else { (0, 0) };

	let mut grey = vec![0u8; w * h];
	for y in 0..sh {
		for x in 0..sw {
			let (fx, fy) = (x + ox, y + oy);
			if fx >= w || fy >= h {
				continue;
			}
			grey[fy * w + fx] = field[y * sw + x];
		}
	}
	grey
}

/// A PNG's dimensions, straight out of its IHDR - the pixels are never needed,
/// only the frame the height map has to match.
fn png_size(path: &Path) -> Result<(usize, usize), String> {
	let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
	let reader =
		png::Decoder::new(std::io::BufReader::new(file)).read_info().map_err(|e| format!("{}: {e}", path.display()))?;
	let info = reader.info();
	Ok((info.width as usize, info.height as usize))
}

/// An 8-bit greyscale PNG. The `png` crate is already this crate's dev
/// dependency, and a height map is one channel by definition.
fn grey_png(width: usize, height: usize, grey: &[u8]) -> Vec<u8> {
	let mut out = Vec::new();
	let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
	encoder.set_color(png::ColorType::Grayscale);
	encoder.set_depth(png::BitDepth::Eight);
	let mut writer = encoder.write_header().expect("a greyscale header");
	writer.write_image_data(grey).expect("the rows fit the header");
	writer.finish().expect("finished");
	out
}
