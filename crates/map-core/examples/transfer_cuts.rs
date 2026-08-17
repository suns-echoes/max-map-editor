//! Carry one pack's **hand cuts** onto a sibling pack that draws the same
//! shapes in a different tone.
//!
//! ```text
//! cargo run -p map-core --example transfer_cuts -- SNOW SNOW_DARK
//! cargo run -p map-core --example transfer_cuts -- SNOW SNOW_DARK --dry
//! ```
//!
//! SNOW_DARK is SNOW a few shades down: the same templates, the same grids, the
//! same empty cells, `SMb023` against `SMB018`. Cutting all forty-six of its
//! objects out by hand a second time would be redrawing a decision somebody
//! already made, so this takes the decision - **which pixels are the object** -
//! and leaves the paint alone.
//!
//! For each target template it finds the source *piece* of the same name, crops
//! the target's own untouched box export at that piece's crop origin, and stamps
//! the piece's alpha on it: opaque where the source has body, [`SHADOW_ALPHA`]
//! where it has drawn shade, clear elsewhere. The colours are the target's
//! throughout - nothing is recoloured, tinted or copied across - so what comes
//! out is the target's own art wearing the source's silhouette, which is what a
//! person cutting it out by hand would have produced.
//!
//! **The two packs place their art at the same offset**, which is why the crop
//! origin can be carried straight across: sliding the source mask over the target
//! and looking for the best agreement puts every one of the thirty-three
//! transferable SNOW pieces at `(0,0)`, give or take a pixel of dither.
//!
//! **A name match is not proof**, so every transfer is measured before it is
//! written - against the source cut itself, which is the only reference that
//! means anything here. Asking "how much plain ground is inside this mask" has no
//! absolute answer: these packs paint snow on snow, so an object and its ground
//! share inks and a perfectly good hand cut scores anywhere from 6% to 95%. What
//! is diagnostic is the **difference**. Score the mask over the source art, score
//! it again over the target art, and if the two agree then the mask fits the
//! target exactly as well as it fits the pack it was drawn for - which is
//! precisely the claim "the same shapes in a darker tone" makes.
//!
//! A piece whose scores disagree is **reported and skipped**, because a bad cut is
//! worse than no cut: the flood is still there for anything this cannot do, and it
//! is what the pack had before.
//!
//! Three things it deliberately does not do. It will not touch a template whose
//! **box is a different size** from the source piece's footprint - that is a
//! different drawing that happens to share a name. It will not write over a
//! target PNG that is **already a hand cut** (anything but the full `cells x 64`
//! box), because that is somebody's work. And it carries no `-X.png`: a traced
//! line is drawn on one pack's art and means nothing on another's.
//!
//! A trailing `a`/`b` on the target name is read as a **variant of the source
//! piece it hangs off** - SNOW_DARK splits seven of SNOW's templates into two
//! alternates over the same footprint - and the same cut serves both, subject to
//! the same measurement.

use std::path::{Path, PathBuf};

use map_core::{GroundInk, SHADOW_ALPHA, SceneryPack, Sprite, Template, TilePack, apply_game_statics};

const TILE: usize = 64;

/// Where the hand-authored art lives - the cut-outs and the traced lines, one
/// directory per pack. Bake input only, and deliberately outside `resources/`:
/// it is worked on by hand and never shipped.
const SOURCE_ART: &str = "private/sources/scenery";

/// The art directory for one pack.
fn source_art(pack: &str) -> PathBuf {
	Path::new(SOURCE_ART).join(pack)
}

/// How far the target's ground share may drift from the source's before the mask
/// is judged not to fit, in percentage points.
///
/// Five. The SNOW pieces that transfer cleanly all land within a single point of
/// their source, so this is five times the observed spread and still far under
/// what a genuinely different drawing costs. The first point is bought by the
/// dither: the two packs quantize their tones slightly differently, so the same
/// pixel of the same shape can be a ground ink in one pack and not in the other.
const MAX_GROUND_DRIFT: f64 = 5.0;

fn main() {
	let mut dry = false;
	let mut names: Vec<String> = Vec::new();
	for arg in std::env::args().skip(1) {
		match arg.as_str() {
			"--dry" => dry = true,
			other => names.push(other.to_string()),
		}
	}
	let [source, target] = match names.as_slice() {
		[a, b] => [a.clone(), b.clone()],
		_ => {
			eprintln!("usage: transfer_cuts -- <SOURCE> <TARGET> [--dry]");
			std::process::exit(2);
		}
	};
	let assets = Path::new("resources/assets");
	if !assets.join("templates").is_dir() {
		eprintln!("run from the repo root (no resources/assets/templates here)");
		std::process::exit(2);
	}
	if let Err(e) = run(assets, &source, &target, dry) {
		eprintln!("{source} -> {target}: {e}");
		std::process::exit(1);
	}
}

fn run(assets: &Path, source: &str, target: &str, dry: bool) -> Result<(), String> {
	let library = SceneryPack::load(assets, source)?;
	let source_ground = ground_colours(assets, source)?;
	let target_ground = ground_colours(assets, target)?;
	let source_art_dir = source_art(source);
	let dir = assets.join("templates").join(target);
	let art = source_art(target);

	let mut ids: Vec<String> = std::fs::read_dir(&dir)
		.map_err(|e| format!("read {}: {e}", dir.display()))?
		.filter_map(|e| e.ok().map(|e| e.path()))
		.filter(|p| p.extension().is_some_and(|x| x == "json"))
		.filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_string))
		.collect();
	ids.sort();

	let (mut done, mut skipped) = (0usize, Vec::new());
	for id in &ids {
		let carry = Carry {
			dir: &dir,
			art: &art,
			source_art_dir: &source_art_dir,
			source_ground: &source_ground,
			target_ground: &target_ground,
		};
		match carry.transfer(id, &library, dry) {
			Ok(Some(note)) => {
				done += 1;
				println!("  {id}: {note}");
			}
			Ok(None) => {}
			Err(why) => skipped.push(format!("{id}: {why}")),
		}
	}
	println!("{source} -> {target}: {done} cut(s) {}", if dry { "would be written" } else { "written" });
	if !skipped.is_empty() {
		println!("  {} left to the flood:", skipped.len());
		for line in &skipped {
			println!("    {line}");
		}
	}
	Ok(())
}

/// What one transfer needs that does not change between pieces.
struct Carry<'a> {
	dir: &'a Path,
	art: &'a Path,
	source_art_dir: &'a Path,
	source_ground: &'a [[u8; 3]],
	target_ground: &'a [[u8; 3]],
}

impl Carry<'_> {
	/// `Ok(Some(note))` when a cut was written, `Ok(None)` when there is nothing
	/// to carry, `Err(why)` when there was and it did not suit the art.
	fn transfer(&self, id: &str, library: &SceneryPack, dry: bool) -> Result<Option<String>, String> {
		// The piece of the same name, or the one a trailing `a`/`b` hangs off.
		let stem = id.strip_suffix(['a', 'b']).unwrap_or(id);
		let Some(piece) =
			library.pieces.iter().find(|p| p.id == id).or_else(|| library.pieces.iter().find(|p| p.id == stem))
		else {
			return Ok(None);
		};
		let template = Template::load(&self.dir.join(format!("{id}.json")))?;
		if (template.width, template.height) != (piece.cells_w, piece.cells_h) {
			return Err(format!(
				"{}x{} box against the source's {}x{} - a different drawing with the same name",
				template.width, template.height, piece.cells_w, piece.cells_h
			));
		}
		let path = self.art.join(format!("{id}.png"));
		let Some(target_art) = read_png(&path)? else { return Ok(None) };
		let (bw, bh) = (template.width as usize * TILE, template.height as usize * TILE);
		if (target_art.width, target_art.height) != (bw, bh) {
			// Already cut, by this tool or by a person. Either way not ours to
			// overwrite.
			return Ok(None);
		}
		// The reference: the same mask over the art it was drawn for. The source
		// PNG is normally the crop itself, so the mask sits at (0,0) there; the odd
		// one that is still box-sized takes the origin, exactly as `height_pngs`
		// decides it.
		let source_path = self.source_art_dir.join(format!("{}.png", piece.id));
		let Some(source_art) = read_png(&source_path)? else {
			return Err(format!("no {} to measure the cut against", source_path.display()));
		};
		let source_at = match (source_art.width, source_art.height) == (bw, bh) {
			true => (piece.sprite.origin_x as usize, piece.sprite.origin_y as usize),
			false => (0, 0),
		};
		let want = ground_share(&source_art, &piece.sprite, source_at, self.source_ground);
		let got = ground_share(
			&target_art,
			&piece.sprite,
			(piece.sprite.origin_x as usize, piece.sprite.origin_y as usize),
			self.target_ground,
		);
		let drift = (got - want).abs();
		if drift > MAX_GROUND_DRIFT {
			return Err(format!(
				"{got:.0}% ground under the mask against {want:.0}% at home - a {drift:.0}-point drift, so this is not the same shape"
			));
		}
		let (sw, sh) = (piece.sprite.width as usize, piece.sprite.height as usize);
		let note = format!(
			"{sw}x{sh} at ({},{}) off {} - {got:.1}% ground under the mask against {want:.1}% at home",
			piece.sprite.origin_x, piece.sprite.origin_y, piece.id
		);
		if !dry {
			let cut = stamp(&target_art, &piece.sprite);
			std::fs::write(&path, rgba_png(sw, sh, &cut)).map_err(|e| format!("write {}: {e}", path.display()))?;
		}
		Ok(Some(note))
	}
}

/// The target's own box, cropped to the source sprite's frame and wearing its
/// alpha. Colour is never touched: this is the target's paint throughout.
fn stamp(art: &Image, sprite: &Sprite) -> Vec<u8> {
	let (sw, sh) = (sprite.width as usize, sprite.height as usize);
	let (ox, oy) = (sprite.origin_x as usize, sprite.origin_y as usize);
	let mut out = vec![0u8; sw * sh * 4];
	for y in 0..sh {
		for x in 0..sw {
			let i = y * sw + x;
			// The alpha rule, backwards: what `band_of` will read out of this pixel
			// is what the source piece recorded at it.
			let alpha = match (sprite.body[i], sprite.shade[i]) {
				(0, 0) => 0,
				(0, _) => SHADOW_ALPHA,
				_ => 255,
			};
			if alpha == 0 {
				continue;
			}
			let Some(px) = art.at(x + ox, y + oy) else { continue };
			out[i * 4..i * 4 + 3].copy_from_slice(px);
			out[i * 4 + 3] = alpha;
		}
	}
	out
}

/// What share of the pixels **inside the mask** are the pack's plain ground, as a
/// percentage - the number whose *drift* between source and target says whether
/// the mask still fits. Meaningless on its own; see the module docs.
///
/// Shade is skipped: a cast shadow falls on ground by definition, so counting it
/// would drown the signal in agreement.
fn ground_share(art: &Image, sprite: &Sprite, (ox, oy): (usize, usize), ground: &[[u8; 3]]) -> f64 {
	let (sw, sh) = (sprite.width as usize, sprite.height as usize);
	let (mut n, mut g) = (0usize, 0usize);
	for y in 0..sh {
		for x in 0..sw {
			if sprite.body[y * sw + x] == 0 {
				continue;
			}
			let Some(px) = art.at(x + ox, y + oy) else { continue };
			n += 1;
			g += usize::from(ground.iter().any(|c| c == px));
		}
	}
	100.0 * g as f64 / n.max(1) as f64
}

/// A pack's plain-ground inks, as the exported PNGs paint them.
fn ground_colours(assets: &Path, name: &str) -> Result<Vec<[u8; 3]>, String> {
	let pack = TilePack::load(&assets.join("tilepacks"), name)?;
	let mut palette = pack.palette.clone().ok_or(format!("{name} owns no palette"))?;
	apply_game_statics(&mut palette);
	let ground = GroundInk::of_pack(&pack);
	if ground.is_empty() {
		return Err(format!("{name} has no plain-ground family to measure a cut against"));
	}
	Ok((0..=255u8)
		.filter(|&i| ground.contains(i))
		.map(|i| [palette[i as usize * 3], palette[i as usize * 3 + 1], palette[i as usize * 3 + 2]])
		.collect())
}

/// One decoded PNG, always as RGB triples however it was stored.
struct Image {
	width: usize,
	height: usize,
	rgb: Vec<[u8; 3]>,
}

impl Image {
	fn at(&self, x: usize, y: usize) -> Option<&[u8; 3]> {
		(x < self.width && y < self.height).then(|| &self.rgb[y * self.width + x])
	}
}

fn read_png(path: &Path) -> Result<Option<Image>, String> {
	let file = match std::fs::File::open(path) {
		Ok(f) => f,
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
		Err(e) => return Err(format!("{}: {e}", path.display())),
	};
	let mut reader =
		png::Decoder::new(std::io::BufReader::new(file)).read_info().map_err(|e| format!("{}: {e}", path.display()))?;
	let mut buf = vec![0; reader.output_buffer_size().ok_or("png: image too large")?];
	let info = reader.next_frame(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
	let src = &buf[..info.buffer_size()];
	let rgb: Vec<[u8; 3]> = match info.color_type {
		png::ColorType::Rgba => src.chunks_exact(4).map(|p| [p[0], p[1], p[2]]).collect(),
		png::ColorType::Rgb => src.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect(),
		other => return Err(format!("{}: {other:?} PNG - re-export as RGB or RGBA", path.display())),
	};
	Ok(Some(Image { width: info.width as usize, height: info.height as usize, rgb }))
}

fn rgba_png(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
	let mut out = Vec::new();
	let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
	encoder.set_color(png::ColorType::Rgba);
	encoder.set_depth(png::BitDepth::Eight);
	let mut writer = encoder.write_header().expect("an RGBA header");
	writer.write_image_data(rgba).expect("the rows fit the header");
	writer.finish().expect("finished");
	out
}
